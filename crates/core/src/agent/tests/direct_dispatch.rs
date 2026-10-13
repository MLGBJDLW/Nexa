use super::*;
use crate::agent::turn_budget::TurnBudget;

fn direct_run() -> (Database, String, String, String) {
    let db = Database::open_memory().unwrap();
    let conversation = db
        .create_conversation(&CreateConversationInput {
            provider: "mock".into(),
            model: "mock-model".into(),
            system_prompt: None,
            collection_context: None,
            project_id: None,
            persona_id: None,
        })
        .unwrap();
    let message = ConversationMessage {
        id: Uuid::new_v4().to_string(),
        conversation_id: conversation.id.clone(),
        role: Role::User,
        content: "list sources".into(),
        tool_call_id: None,
        tool_calls: vec![],
        artifacts: None,
        token_count: 0,
        created_at: String::new(),
        sort_order: 0,
        thinking: None,
        image_attachments: None,
    };
    db.add_message(&message).unwrap();
    let turn = db
        .create_conversation_turn(&conversation.id, &message.id, None)
        .unwrap();
    let run = db
        .create_agent_task_run(
            &conversation.id,
            &turn.id,
            &message.id,
            "Direct",
            None,
            None,
        )
        .unwrap();
    (db, conversation.id, turn.id, run.id)
}

#[tokio::test]
async fn direct_dispatch_admission_precedes_events_and_restores_failed_execution() {
    for registered in [true, false] {
        let (db, conversation, turn, run) = direct_run();
        let executions = Arc::new(AtomicUsize::new(0));
        let captured = Arc::new(Mutex::new(Vec::new()));
        let mut tools = ToolRegistry::new();
        if registered {
            tools.register(Box::new(ErrorNamedMockTool {
                name: "list_sources",
                executions: executions.clone(),
            }));
        }
        let executor = AgentExecutor::new(
            Box::new(ToolSurfaceCapturingProvider {
                tool_names: captured.clone(),
                latest_user_texts: Arc::new(Mutex::new(Vec::new())),
            }),
            tools,
            AgentConfig {
                model: Some("mock-model".into()),
                max_iterations: 2,
                ..Default::default()
            },
        );
        let observer_db = db.clone();
        let observer_run = run.clone();
        let observer_turn = turn.clone();
        let (tx, mut rx) = mpsc::channel(128);
        let observer =
            tokio::spawn(async move {
                let mut started = false;
                let mut sequence = 0;
                while let Some(event) = rx.recv().await {
                    if matches!(event, AgentEvent::ToolRunStarted { .. }) {
                        assert!(observer_db
                            .get_agent_task_run_events(&observer_run)
                            .unwrap()
                            .iter()
                            .any(|receipt| receipt.event_type == "tool_round_admitted"));
                        started = true;
                    }
                    sequence += 1;
                    let event = crate::agent_run::AgentRunEvent::from_agent_event(&event)
                        .with_context(Some(&observer_run), Some(&observer_turn), Some(sequence));
                    observer_db.save_agent_run_event(&event).unwrap();
                }
                assert!(started);
            });
        let mut budget = TurnBudget::new(2);
        assert!(matches!(
            executor
                .try_direct_dispatch(
                    "list sources",
                    &db,
                    &[],
                    &tx,
                    Some(&conversation),
                    Some(&turn),
                    1,
                    &mut budget
                )
                .await
                .unwrap(),
            super::super::direct_dispatch_runner::DirectDispatchOutcome::ExecutedButFailed
        ));
        drop(tx);
        observer.await.unwrap();
        assert_eq!(budget.remaining_tool_rounds(), Some(1));
        let mut resumed = TurnBudget::new(20);
        resumed
            .restore_receipts(&db, Some(&conversation), Some(&turn))
            .unwrap();
        assert_eq!(resumed.remaining_tool_rounds(), Some(1));

        // Resume from the durable events without a later ordinary loop receipt.
        // The first direct dispatch must not be replayed automatically.
        let (tx, _rx) = mpsc::channel(128);
        let answer = executor
            .run(
                vec![],
                vec![ContentPart::Text {
                    text: "list sources".into(),
                }],
                &db,
                Some(&conversation),
                Some(&turn),
                tx,
                1,
            )
            .await
            .unwrap();
        assert_eq!(answer.text_content(), "done");
        assert_eq!(executions.load(Ordering::SeqCst), usize::from(registered));
        assert_eq!(captured.lock().unwrap().len(), 1);
    }
}

#[tokio::test]
async fn unmatched_or_unadmitted_direct_dispatch_cannot_execute_or_charge() {
    let (db, conversation, turn, run) = direct_run();
    let executions = Arc::new(AtomicUsize::new(0));
    let mut tools = ToolRegistry::new();
    tools.register(Box::new(ErrorNamedMockTool {
        name: "list_sources",
        executions: executions.clone(),
    }));
    let executor = AgentExecutor::new(
        Box::new(MockProvider {
            stream_calls: Arc::new(AtomicUsize::new(0)),
        }),
        tools,
        AgentConfig::default(),
    );
    let (tx, mut rx) = mpsc::channel(128);
    let mut budget = TurnBudget::new(2);
    assert!(matches!(
        executor
            .try_direct_dispatch(
                "hello",
                &db,
                &[],
                &tx,
                Some(&conversation),
                Some(&turn),
                1,
                &mut budget
            )
            .await
            .unwrap(),
        super::super::direct_dispatch_runner::DirectDispatchOutcome::NotMatched
    ));
    assert!(db.get_agent_task_run_events(&run).unwrap().is_empty());
    assert_eq!(budget.remaining_tool_rounds(), Some(2));
    db.conn()
        .execute_batch("DROP TABLE agent_task_run_events")
        .unwrap();
    assert!(executor
        .try_direct_dispatch(
            "list sources",
            &db,
            &[],
            &tx,
            Some(&conversation),
            Some(&turn),
            1,
            &mut budget
        )
        .await
        .is_err());
    assert_eq!(executions.load(Ordering::SeqCst), 0);
    assert_eq!(budget.remaining_tool_rounds(), Some(2));
    assert!(rx.try_recv().is_err());
}
