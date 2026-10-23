//! A microphone recording outlives individual provider sockets. Unconfirmed
//! PCM is journaled on disk and replayed after reconnect; utterance commits
//! advance the audio checkpoint, never stop the microphone.
use super::*;
use tokio::io::AsyncWriteExt;
use tokio::time::Instant;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

type Socket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;
const OPENAI_ROTATE_AFTER: Duration = Duration::from_secs(55 * 60);
const JOURNAL_CHECKPOINT_BYTES: u64 = 1024 * 1024;
const WRITE_TIMEOUT: Duration = Duration::from_secs(15);

fn provider_diagnostic(
    config: &nexa_core::app_settings::SpeechToTextConfig,
    message: String,
) -> String {
    let key = config.api_key.trim();
    if key.is_empty() {
        return message;
    }
    let encoded = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("key", key)
        .finish();
    message
        .replace(encoded.trim_start_matches("key="), "[redacted]")
        .replace(key, "[redacted]")
}

fn gemini_endpoint(config: &nexa_core::app_settings::SpeechToTextConfig) -> Result<String, String> {
    let mut url = Url::parse(config.base_url.as_deref().unwrap_or_default())
        .map_err(|_| "Invalid Gemini transcription endpoint")?;
    let scheme = match url.scheme() {
        "https" | "wss" => "wss",
        "http" | "ws" => "ws",
        _ => return Err("Invalid Gemini transcription endpoint scheme".into()),
    };
    url.set_scheme(scheme)
        .map_err(|_| "Invalid Gemini transcription endpoint")?;
    if !url.path().ends_with(".BidiGenerateContent") {
        url.set_path(
            "/ws/google.ai.generativelanguage.v1beta.GenerativeService.BidiGenerateContent",
        );
    }
    url.set_query(None);
    url.query_pairs_mut()
        .append_pair("key", config.api_key.trim());
    Ok(url.to_string())
}

pub(super) async fn connect(
    config: &nexa_core::app_settings::SpeechToTextConfig,
    dialect: RealtimeDialect,
    task_id: &str,
) -> Result<Socket, String> {
    let endpoint = if dialect == RealtimeDialect::Gemini {
        gemini_endpoint(config)?
    } else if dialect == RealtimeDialect::DashScopeTask {
        nexa_core::dashscope_speech::task_endpoint(config.base_url.as_deref().unwrap_or_default())?
    } else {
        build_realtime_endpoint(
            config.base_url.as_deref().unwrap_or_default(),
            &config.model,
        )?
    };
    let mut request = endpoint
        .into_client_request()
        .map_err(|error| format!("Invalid Realtime WebSocket request: {error}"))?;
    if dialect != RealtimeDialect::Gemini {
        request.headers_mut().insert(
            "Authorization",
            format!("Bearer {}", config.api_key.trim())
                .parse()
                .map_err(|_| "Invalid realtime authorization header")?,
        );
    }
    request.headers_mut().insert(
        "User-Agent",
        nexa_core::USER_AGENT
            .parse()
            .map_err(|_| "Invalid User-Agent header")?,
    );
    let (mut socket, _) = tokio::time::timeout(
        SESSION_READY_TIMEOUT,
        tokio_tungstenite::connect_async(request),
    )
    .await
    .map_err(|_| "Timed out connecting to realtime transcription")?
    .map_err(|error| {
        provider_diagnostic(
            config,
            format!("Unable to connect to realtime transcription: {error}"),
        )
    })?;
    // gpt-live-transcribe requires client commits; server/semantic VAD is not
    // supported even when this session belongs to a Live meeting.
    socket
        .send(Message::Text(
            build_session_update(dialect, &config.model, config.language.as_deref(), task_id)
                .to_string()
                .into(),
        ))
        .await
        .map_err(|error| format!("Unable to configure realtime transcription: {error}"))?;
    wait_for_session_ready(&mut socket, dialect, task_id)
        .await
        .map_err(|error| provider_diagnostic(config, error))?;
    Ok(socket)
}

struct AudioJournal {
    file: tokio::fs::File,
    origin: u64,
    confirmed: u64,
    end: u64,
    sent: u64,
    // A finalized spool is read in place; its owner retains deletion authority.
    source_header_bytes: u64,
}
impl AudioJournal {
    fn new() -> Result<Self, String> {
        let file = tempfile::tempfile()
            .map_err(|error| format!("Unable to retain realtime audio: {error}"))?;
        Ok(Self {
            file: tokio::fs::File::from_std(file),
            origin: 0,
            confirmed: 0,
            end: 0,
            sent: 0,
            source_header_bytes: 0,
        })
    }
    async fn append(&mut self, bytes: &[u8]) -> Result<(), String> {
        if self.source_header_bytes != 0 {
            return Err("Cannot append to a finalized recording".into());
        }
        self.file
            .seek(SeekFrom::Start(self.end - self.origin))
            .await
            .map_err(|e| e.to_string())?;
        self.file
            .write_all(bytes)
            .await
            .map_err(|e| format!("Unable to retain realtime audio: {e}"))?;
        self.end += bytes.len() as u64;
        Ok(())
    }
    async fn next(&mut self, maximum: usize) -> Result<Vec<u8>, String> {
        let length = (self.end - self.sent).min(maximum as u64) as usize;
        let mut bytes = vec![0; length];
        self.file
            .seek(SeekFrom::Start(
                self.source_header_bytes + self.sent - self.origin,
            ))
            .await
            .map_err(|e| e.to_string())?;
        self.file
            .read_exact(&mut bytes)
            .await
            .map_err(|e| e.to_string())?;
        Ok(bytes)
    }
    async fn confirm(&mut self, end: u64) -> Result<(), String> {
        self.confirmed = end.min(self.sent);
        if self.source_header_bytes != 0 {
            return Ok(());
        }
        if self.confirmed == self.end {
            self.file.set_len(0).await.map_err(|e| e.to_string())?;
            self.origin = self.end;
        } else if self.confirmed - self.origin >= JOURNAL_CHECKPOINT_BYTES {
            // Reclaim only acknowledged PCM. The tail remains byte-identical
            // and the file handle's lifetime owns cleanup on cancel or exit.
            let replacement = tempfile::tempfile().map_err(|e| e.to_string())?;
            let mut replacement = tokio::fs::File::from_std(replacement);
            self.file
                .seek(SeekFrom::Start(self.confirmed - self.origin))
                .await
                .map_err(|e| e.to_string())?;
            tokio::io::copy(
                &mut (&mut self.file).take(self.end - self.confirmed),
                &mut replacement,
            )
            .await
            .map_err(|e| e.to_string())?;
            self.file = replacement;
            self.origin = self.confirmed;
        }
        Ok(())
    }
}

#[derive(Default)]
struct UtteranceBoundary {
    frames: u64,
    silent_frames: u64,
    speech: bool,
    nonzero: bool,
}
impl UtteranceBoundary {
    fn append(&mut self, pcm: &[u8], sample_rate: u32) -> bool {
        let frames = pcm.len() as u64 / 2;
        self.nonzero |= pcm.iter().any(|byte| *byte != 0);
        let speech = pcm
            .chunks_exact(2)
            .any(|sample| i16::from_le_bytes([sample[0], sample[1]]).unsigned_abs() >= 400);
        self.frames += frames;
        self.speech |= speech;
        self.silent_frames = if speech {
            0
        } else {
            self.silent_frames + frames
        };
        // A processing segment may end during continuous speech, but the
        // logical recording, microphone, journal and transcript stay alive.
        (self.speech && self.silent_frames >= u64::from(sample_rate) * 2 / 5)
            || self.frames >= u64::from(sample_rate) * 30
    }
}

fn join_transcript(prefix: &str, suffix: &str) -> String {
    match (prefix.trim().is_empty(), suffix.trim().is_empty()) {
        (true, _) => suffix.to_owned(),
        (_, true) => prefix.to_owned(),
        _ => format!("{} {}", prefix.trim_end(), suffix.trim_start()),
    }
}
fn permanent_provider_error(message: &str) -> bool {
    let lower = message.to_lowercase();
    [
        "unauthorized",
        "authentication",
        "api key",
        "permission",
        "quota exhausted",
        "insufficient_quota",
        "unsupported",
        "invalid model",
        "401 unauthorized",
        "403 forbidden",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}
fn should_rotate(dialect: RealtimeDialect, elapsed: Duration) -> bool {
    dialect == RealtimeDialect::OpenAi && elapsed >= OPENAI_ROTATE_AFTER
}

pub(super) async fn run(
    config: nexa_core::app_settings::SpeechToTextConfig,
    dialect: RealtimeDialect,
    task_id: String,
    socket: Socket,
    commands: mpsc::Receiver<RealtimeCommand>,
    events: &RealtimeEventSink,
    session_id: &str,
) -> Result<(), String> {
    run_with_audio(
        config,
        dialect,
        task_id,
        socket,
        commands,
        events,
        session_id,
        AudioJournal::new()?,
    )
    .await
}

pub(super) async fn replay(
    wav_path: &Path,
    config: &nexa_core::app_settings::SpeechToTextConfig,
) -> Result<String, String> {
    let dialect = RealtimeDialect::from_config(config)?;
    let mut file = tokio::fs::File::open(wav_path)
        .await
        .map_err(|error| error.to_string())?;
    let mut header = [0; 44];
    file.read_exact(&mut header)
        .await
        .map_err(|error| error.to_string())?;
    let end = u64::from(replay_wav_data_bytes(&header, dialect.sample_rate())?);
    if file
        .metadata()
        .await
        .map_err(|error| error.to_string())?
        .len()
        != end + 44
    {
        return Err("Managed realtime WAV length does not match its header".into());
    }
    let audio = AudioJournal {
        file,
        origin: 0,
        confirmed: 0,
        end,
        sent: 0,
        source_header_bytes: 44,
    };
    let task_id = Uuid::new_v4().simple().to_string();
    let socket = connect(config, dialect, &task_id).await?;
    let (sender, commands) = mpsc::channel(1);
    let (response, result) = oneshot::channel();
    sender
        .send(RealtimeCommand::Finish(response))
        .await
        .map_err(|error| error.to_string())?;
    let events = RealtimeEventSink {
        frontend: Arc::new(|_| {}),
        live: None,
    };
    run_with_audio(
        config.clone(),
        dialect,
        task_id,
        socket,
        commands,
        &events,
        "replay",
        audio,
    )
    .await?;
    drop(sender);
    result
        .await
        .map_err(|_| "Realtime replay ended without a transcript".to_string())?
}

async fn run_with_audio(
    config: nexa_core::app_settings::SpeechToTextConfig,
    dialect: RealtimeDialect,
    mut task_id: String,
    mut socket: Socket,
    mut commands: mpsc::Receiver<RealtimeCommand>,
    events: &RealtimeEventSink,
    session_id: &str,
    mut audio: AudioJournal,
) -> Result<(), String> {
    let mut prefix = String::new();
    let mut transcript = RealtimeTranscript::default();
    let mut sequence = 0;
    let mut pending_final = None;
    let mut final_progress_at: Option<Instant> = None;
    let mut pending_commit: Option<u64> = None;
    let mut expected_item: Option<String> = None;
    let mut committed_at: Option<Instant> = None;
    let mut acknowledged_items = std::collections::HashSet::new();
    let mut boundary = UtteranceBoundary::default();
    let mut input_started = false;
    let mut connected_at = Instant::now();
    let mut retry_delay = Duration::from_millis(250);
    // Qwen documents 100 ms PCM packets paced at their audio duration. A
    // recovered backlog stays on disk and does not flood a realtime endpoint.
    let mut tick = tokio::time::interval(Duration::from_millis(100));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let work: Result<(), String> = async {
        loop {
            let mut reconnect = false;
            let mut checkpoint = false;
            tokio::select! {
                command = commands.recv() => match command {
                    Some(RealtimeCommand::Append(bytes)) => audio.append(&bytes).await?,
                    Some(RealtimeCommand::Finish(response)) => {
                        if pending_final.is_some() { let _ = response.send(Err("Realtime transcription is already finishing".into())); }
                        else { pending_final = Some(response); final_progress_at = Some(Instant::now()); }
                    }
                    None => return Ok(()),
                },
                _ = tick.tick() => {
                    if final_progress_at.is_some_and(|at| at.elapsed() >= REPLAY_FINAL_TRANSCRIPT_TIMEOUT) {
                        return Err("Realtime service made no transcription progress; the recording remains saved for retry".into());
                    }
                    if pending_commit.is_none() && audio.sent < audio.end {
                        let bytes = audio.next(dialect.sample_rate() as usize / 5).await?; // <= 100 ms PCM16
                        let commit = boundary.append(&bytes, dialect.sample_rate());
                        if dialect == RealtimeDialect::Gemini && !input_started {
                            input_started = matches!(tokio::time::timeout(WRITE_TIMEOUT, socket.send(Message::Text(serde_json::json!({"realtimeInput":{"activityStart":{}}}).to_string().into()))).await, Ok(Ok(())));
                            reconnect = !input_started;
                        }
                        if !reconnect { match tokio::time::timeout(WRITE_TIMEOUT, socket.send(audio_message(dialect, &bytes))).await {
                            Ok(Ok(())) => {
                                audio.sent += bytes.len() as u64;
                                retry_delay = Duration::from_millis(250);
                                // Draining a long saved recording is progress,
                                // including silence before its first utterance.
                                if pending_final.is_some() { final_progress_at = Some(Instant::now()); }
                            }
                            _ => reconnect = true,
                        } }
                        if !reconnect && matches!(dialect, RealtimeDialect::OpenAi | RealtimeDialect::Gemini) && commit { pending_commit = Some(audio.sent); }
                    }
                    if !reconnect && pending_commit.is_none() && pending_final.is_some() && audio.sent == audio.end {
                        if audio.sent == audio.confirmed {
                            resolve_pending_final(&mut pending_final, Ok(prefix.clone()));
                            return Ok(());
                        }
                        pending_commit = Some(audio.sent);
                    }
                    if !reconnect && should_rotate(dialect, connected_at.elapsed()) && pending_commit.is_none() {
                        if audio.sent == audio.confirmed { reconnect = true; }
                        else { pending_commit = Some(audio.sent); }
                    }
                    // Commit exactly once at an explicit audio boundary. We
                    // retain later microphone audio on disk while finals arrive.
                    if !reconnect && pending_commit.is_some() && expected_item.is_none() {
                        expected_item = Some(String::new());
                        committed_at = Some(Instant::now());
                        for message in finish_messages(dialect, &task_id) {
                            if !matches!(tokio::time::timeout(WRITE_TIMEOUT, socket.send(Message::Text(message.to_string().into()))).await, Ok(Ok(()))) { reconnect = true; break; }
                        }
                    }
                    if committed_at.is_some_and(|at| at.elapsed() >= FINAL_TRANSCRIPT_TIMEOUT) {
                        reconnect = true;
                    }
                },
                incoming = socket.next() => match incoming {
                    Some(Ok(Message::Text(text))) => {
                        let Ok(event) = serde_json::from_str::<Value>(&text) else { continue; };
                        if dialect == RealtimeDialect::Gemini && pending_commit.is_some() && !boundary.nonzero
                            && event.pointer("/serverContent/turnComplete").and_then(Value::as_bool) == Some(true) {
                            checkpoint = true;
                        }
                        if dialect == RealtimeDialect::DashScopeTask && event.pointer("/header/task_id").and_then(Value::as_str) != Some(task_id.as_str()) { continue; }
                        if dialect == RealtimeDialect::OpenAi && pending_commit.is_some()
                            && event["type"] == "input_audio_buffer.committed" {
                            expected_item = event["item_id"].as_str().map(str::to_owned).or(expected_item);
                        }
                        match parse_server_event(dialect, &event) {
                            ParsedRealtimeServerEvent::Interim { utterance_id, text, update } => {
                                if utterance_id.as_ref().is_some_and(|id| acknowledged_items.contains(id)) { continue; }
                                transcript.update(utterance_id.as_deref(), &text, update == TranscriptUpdate::AppendDelta, false);
                                let current = transcript.snapshot();
                                let logical_id = format!("audio-{}", audio.confirmed);
                                emit_live_transcript(events, "interim", Some(&current), Some("replaceSnapshot"), Some(&logical_id));
                                emit_realtime_event(events, session_id, &mut sequence, "interim", Some(&join_transcript(&prefix, &current)), Some(TranscriptUpdate::ReplaceSnapshot), Some(&logical_id));
                            }
                            ParsedRealtimeServerEvent::Final { utterance_id, text } => {
                                if utterance_id.as_ref().is_some_and(|id| acknowledged_items.contains(id)) { continue; }
                                transcript.update(utterance_id.as_deref(), &text, false, true);
                                if matches!(dialect, RealtimeDialect::OpenAi | RealtimeDialect::Gemini) {
                                    checkpoint = pending_commit.is_some() && expected_item.as_deref().is_none_or(|id| id.is_empty() || Some(id) == utterance_id.as_deref());
                                    if checkpoint { if let Some(id) = utterance_id { acknowledged_items.insert(id); } }
                                } else if pending_commit.is_none() && audio.sent - audio.confirmed >= JOURNAL_CHECKPOINT_BYTES {
                                    // Qwen ASR has no recording duration cap.
                                    // A sentence boundary checkpoints the transport
                                    // journal so reconnection replays a short tail.
                                    pending_commit = Some(audio.sent);
                                }
                                if !checkpoint {
                                    let current = transcript.snapshot();
                                    let logical_id = format!("audio-{}", audio.confirmed);
                                    emit_live_transcript(events, "interim", Some(&current), Some("replaceSnapshot"), Some(&logical_id));
                                    emit_realtime_event(events, session_id, &mut sequence, "interim", Some(&join_transcript(&prefix, &current)), Some(TranscriptUpdate::ReplaceSnapshot), Some(&logical_id));
                                }
                            }
                            ParsedRealtimeServerEvent::SessionFinished => { checkpoint = pending_commit.is_some(); if !checkpoint { reconnect = true; } }
                            ParsedRealtimeServerEvent::Error(message) => {
                                if permanent_provider_error(&message) { return Err(message); }
                                reconnect = true;
                            }
                            ParsedRealtimeServerEvent::Other => {}
                        }
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        if !matches!(tokio::time::timeout(WRITE_TIMEOUT, socket.send(Message::Pong(payload))).await, Ok(Ok(()))) { reconnect = true; }
                    }
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => reconnect = true,
                    Some(Ok(_)) => {}
                }
            }
            if checkpoint {
                let text = transcript.finish()?;
                let logical_id = format!("audio-{}", audio.confirmed);
                let end = pending_commit.take().unwrap_or(audio.sent);
                prefix = join_transcript(&prefix, &text);
                audio.confirm(end).await?;
                if pending_final.is_some() { final_progress_at = Some(Instant::now()); }
                emit_live_transcript(events, "final", Some(&text), Some("replaceSnapshot"), Some(&logical_id));
                emit_realtime_event(events, session_id, &mut sequence, "final", Some(&prefix), Some(TranscriptUpdate::ReplaceSnapshot), Some(&logical_id));
                transcript = RealtimeTranscript::default();
                expected_item = None;
                committed_at = None;
                boundary = UtteranceBoundary::default();
                input_started = false;
                if pending_final.is_some() && audio.confirmed == audio.end {
                    resolve_pending_final(&mut pending_final, Ok(prefix.clone()));
                    return Ok(());
                }
                reconnect = dialect.waits_for_session_finished() || should_rotate(dialect, connected_at.elapsed());
            }
            if reconnect {
                audio.sent = audio.confirmed;
                pending_commit = None;
                expected_item = None;
                committed_at = None;
                acknowledged_items.clear();
                boundary = UtteranceBoundary::default();
                input_started = false;
                transcript = RealtimeTranscript::default();
                emit_realtime_event(events, session_id, &mut sequence, "reconnecting", None, None, None);
                loop {
                    if final_progress_at.is_some_and(|at| at.elapsed() >= REPLAY_FINAL_TRANSCRIPT_TIMEOUT) {
                        return Err("Realtime service is still unavailable; the recording remains saved for retry".into());
                    }
                    task_id = Uuid::new_v4().simple().to_string();
                    let delay = retry_delay;
                    let reconnect = async {
                        tokio::time::sleep(delay).await;
                        connect(&config, dialect, &task_id).await
                    };
                    tokio::pin!(reconnect);
                    let result = loop {
                        tokio::select! {
                            result = &mut reconnect => break result,
                            command = commands.recv() => match command {
                                Some(RealtimeCommand::Append(bytes)) => audio.append(&bytes).await?,
                                Some(RealtimeCommand::Finish(response)) => {
                                    if pending_final.is_none() { pending_final = Some(response); final_progress_at = Some(Instant::now()); }
                                    else { let _ = response.send(Err("Realtime transcription is already finishing".into())); }
                                }
                                None => return Ok(()),
                            }
                        }
                    };
                    match result {
                        Ok(connected) => {
                            socket = connected;
                            connected_at = Instant::now();
                            emit_realtime_event(events, session_id, &mut sequence, "connected", None, None, None);
                            break;
                        }
                        Err(message) if permanent_provider_error(&message) => return Err(message),
                        Err(_) => retry_delay = (retry_delay * 2).min(Duration::from_secs(15)),
                    }
                }
            }
        }
    }.await.map_err(|message| provider_diagnostic(&config, message));
    if let Err(message) = &work {
        resolve_pending_final(&mut pending_final, Err(message.clone()));
        emit_realtime_event(
            events,
            session_id,
            &mut sequence,
            "error",
            Some(message),
            None,
            None,
        );
    }
    let _ = tokio::time::timeout(Duration::from_secs(1), socket.close(None)).await;
    work
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn qwen_30_sentences_keep_streaming_after_the_first_final() {
        let mut transcript = RealtimeTranscript::default();
        for (begin_time, text, final_result) in [
            (170, "第一句", false),
            (170, "第一句。", true),
            (1400, "第二句正在", false),
            (1400, "第二句正在继续。", true),
        ] {
            // Qwen Audio 3.0 identifies sentences by begin_time; sentence_id
            // is present in newer task dialects but is not universal.
            let event = serde_json::json!({
                "header": {"event": "result-generated"},
                "payload": {"output": {"sentence": {
                    "begin_time": begin_time, "text": text,
                    "sentence_end": final_result
                }}}
            });
            match parse_server_event(RealtimeDialect::DashScopeTask, &event) {
                ParsedRealtimeServerEvent::Interim {
                    utterance_id, text, ..
                } => transcript.update(utterance_id.as_deref(), &text, false, false),
                ParsedRealtimeServerEvent::Final { utterance_id, text } => {
                    transcript.update(utterance_id.as_deref(), &text, false, true)
                }
                other => panic!("unexpected sentence event: {other:?}"),
            }
            if begin_time == 1400 {
                assert_eq!(transcript.snapshot(), format!("第一句。 {text}"));
            }
        }
        assert_eq!(transcript.finish().unwrap(), "第一句。 第二句正在继续。");
    }

    #[test]
    fn websocket_query_credentials_are_redacted_from_provider_diagnostics() {
        let config = nexa_core::app_settings::SpeechToTextConfig {
            api_key: "secret +value".into(),
            base_url: Some("https://generativelanguage.googleapis.com".into()),
            ..Default::default()
        };
        let url = gemini_endpoint(&config).unwrap();
        let error = provider_diagnostic(&config, format!("{url} {}", config.api_key));
        assert!(!error.contains("secret"));
        assert!(error.contains("[redacted]"));
    }
    #[tokio::test]
    async fn qwen_automatic_commit_does_not_block_explicit_session_finish() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            while let Some(Ok(Message::Text(text))) = socket.next().await {
                let event: Value = serde_json::from_str(&text).unwrap();
                let responses = match event["type"].as_str().unwrap() {
                    "session.update" => vec![serde_json::json!({"type":"session.updated"})],
                    "input_audio_buffer.append" => vec![
                        serde_json::json!({"type":"input_audio_buffer.committed", "item_id":"sentence"}),
                        serde_json::json!({"type":"conversation.item.input_audio_transcription.completed", "item_id":"sentence", "transcript":"Complete sentence."}),
                    ],
                    "session.finish" => vec![serde_json::json!({"type":"session.finished"})],
                    _ => vec![],
                };
                for response in responses {
                    socket
                        .send(Message::Text(response.to_string().into()))
                        .await
                        .unwrap();
                }
                if event["type"] == "session.finish" {
                    break;
                }
            }
        });
        let config = nexa_core::app_settings::SpeechToTextConfig {
            provider: "alibaba-qwen-realtime".into(),
            api_style: "dashscope_realtime_asr".into(),
            api_key: "fixture".into(),
            base_url: Some(format!("http://{address}/v1")),
            model: "qwen3-asr-flash-realtime".into(),
            ..Default::default()
        };
        let socket = connect(&config, RealtimeDialect::DashScope, "fixture")
            .await
            .unwrap();
        let (sender, receiver) = mpsc::channel(8);
        let seen = Arc::new(tokio::sync::Notify::new());
        let received = seen.clone();
        let events = RealtimeEventSink {
            frontend: Arc::new(move |event| {
                if event.kind == "interim" {
                    received.notify_one();
                }
            }),
            live: None,
        };
        let task = tokio::spawn(async move {
            run(
                config,
                RealtimeDialect::DashScope,
                "fixture".into(),
                socket,
                receiver,
                &events,
                "logical",
            )
            .await
        });
        sender
            .send(RealtimeCommand::Append(vec![0; 3_200]))
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(2), seen.notified())
            .await
            .unwrap();
        let (response, result) = oneshot::channel();
        sender
            .send(RealtimeCommand::Finish(response))
            .await
            .unwrap();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), result)
                .await
                .unwrap()
                .unwrap()
                .unwrap(),
            "Complete sentence."
        );
        task.await.unwrap().unwrap();
        server.await.unwrap();
    }

    #[tokio::test]
    async fn audio_checkpoint_replays_only_unconfirmed_pcm_after_reconnect() {
        let mut journal = AudioJournal::new().unwrap();
        journal.append(&[1, 0, 2, 0, 3, 0]).await.unwrap();
        journal.sent = 4;
        journal.confirm(4).await.unwrap();
        journal.append(&[4, 0]).await.unwrap();
        journal.sent = journal.confirmed;
        assert_eq!(journal.next(1024).await.unwrap(), [3, 0, 4, 0]);
        journal.sent = 8;
        journal.confirm(8).await.unwrap();
        journal.append(&[5, 0]).await.unwrap();
        assert_eq!(journal.next(1024).await.unwrap(), [5, 0]);
    }
    #[test]
    fn provider_session_rotation_is_not_a_recording_duration_limit() {
        assert!(should_rotate(
            RealtimeDialect::OpenAi,
            Duration::from_secs(60 * 60)
        ));
        assert!(!should_rotate(
            RealtimeDialect::DashScope,
            Duration::from_secs(24 * 60 * 60)
        ));
        assert!(!should_rotate(
            RealtimeDialect::DashScopeTask,
            Duration::from_secs(24 * 60 * 60)
        ));
        let mut vad = UtteranceBoundary::default();
        let speech: Vec<u8> = (0..2_400).flat_map(|_| 1000_i16.to_le_bytes()).collect();
        assert!(!vad.append(&speech, 24_000));
        assert!(!vad.append(&vec![0; 24_000 / 5], 24_000));
        assert!(vad.append(&vec![0; 24_000], 24_000));
    }
}
