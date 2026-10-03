//! Reproducible synthetic retrieval acceptance and exact-vector scaling probe.
//! cargo run -p nexa-core --example knowledge_eval --no-default-features --features host-tools -- report.json
use nexa_core::{
    db::Database,
    ingest,
    models::{SearchFilters, SearchQuery},
    search,
    sources::CreateSourceInput,
};
use serde_json::json;
use std::time::Instant;

fn query(text: &str) -> SearchQuery {
    SearchQuery {
        text: text.into(),
        filters: SearchFilters::default(),
        limit: 5,
        offset: 0,
    }
}
fn percentile(values: &mut [f64], fraction: f64) -> f64 {
    values.sort_by(f64::total_cmp);
    values[((values.len() - 1) as f64 * fraction).ceil() as usize]
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args()
        .nth(1)
        .ok_or("Provide an output JSON path")?;
    let workspace = tempfile::tempdir()?;
    let corpus = workspace.path().join("corpus");
    std::fs::create_dir(&corpus)?;
    let files=[
        ("zh-expenses.md","# 公司差旅\n差旅报销标准规定员工每日住宿上限为五百元。\n\n# 附录\n交通补贴为每天二十元。"),
        ("en-expenses.md","# Travel policy\nThe lodging allowance is 500 yuan per day.\n\n# Exception\nManager approval is required above the daily cap."),
        ("mixed-api.md","# API authentication\n使用PKCE保护OAuth登录。请求超时为30秒。错误代码AUTH-204表示令牌已过期。"),
        ("short-facts.md","# Facts\nOK.\n\n价税分离。\n\n42 kg."),
        ("long-appendix.md","# Overview\nResearch scope and methods.\n\n# Appendix Z\n复核截止日期为星期五。The audit deadline is Friday."),
        ("scope-other.md","# Private policy\n私人备用预算为九百万元。"),
    ];
    for (name, text) in files {
        std::fs::write(corpus.join(name), text)?;
    }
    let db = Database::new(workspace.path().join("knowledge.sqlite"))?;
    let source = db.add_source(CreateSourceInput {
        root_path: corpus.to_string_lossy().into(),
        include_globs: vec!["**/*.md".into()],
        exclude_globs: vec![],
        watch_enabled: false,
    })?;
    let ingest_start = Instant::now();
    let ingested = ingest::scan_source(&db, &source.id)?;
    let ingest_ms = ingest_start.elapsed().as_secs_f64() * 1000.;
    let cases = [
        ("zh-substring", "住宿上限", Some("zh-expenses.md")),
        ("zh-table-like-fact", "交通补贴", Some("zh-expenses.md")),
        ("en-fact", "lodging allowance", Some("en-expenses.md")),
        ("en-exception", "Manager approval", Some("en-expenses.md")),
        ("mixed-acronym", "PKCE", Some("mixed-api.md")),
        ("identifier", "AUTH-204", Some("mixed-api.md")),
        ("short-chinese", "价税分离", Some("short-facts.md")),
        ("short-number-unit", "42 kg", Some("short-facts.md")),
        ("appendix-chinese", "复核截止日期", Some("long-appendix.md")),
        (
            "appendix-english",
            "audit deadline",
            Some("long-appendix.md"),
        ),
        ("no-answer", "xylophonic zebrafish", None),
    ];
    let mut reports = Vec::new();
    let mut sum_rr = 0.0;
    let mut supported = 0;
    for (name, text, expected) in cases {
        let result = search::hybrid_search(&db, &query(text))?;
        let rank = expected
            .and_then(|expected| {
                result
                    .evidence_cards
                    .iter()
                    .position(|card| card.document_path.ends_with(expected))
            })
            .map(|position| position + 1);
        if expected.is_some() {
            supported += 1;
            sum_rr += rank.map_or(0.0, |rank| 1.0 / rank as f64);
        }
        let passed = if expected.is_some() {
            rank.is_some()
        } else {
            result.evidence_cards.is_empty()
        };
        reports.push(json!({"name":name,"query":text,"expected":expected,"rank":rank,"passed":passed,"locationsPresent":result.evidence_cards.iter().all(|card|card.evidence_ref.is_some())}));
    }
    let mut excluded = query("住宿上限");
    excluded.filters.source_ids = vec![uuid::Uuid::new_v4()];
    let scope_passed = search::hybrid_search(&db, &excluded)?
        .evidence_cards
        .is_empty();
    db.rebuild_fts_index()?;
    let rebuild_passed = db.integrity_check()?
        && !search::search(&db, &query("住宿上限"))?
            .evidence_cards
            .is_empty();
    // Cross-language paraphrase is deliberately diagnostic, not hidden in an aggregate.
    let cross = search::hybrid_search(&db, &query("How much is the transport subsidy?"))?;
    let cross_language = json!({"query":"How much is the transport subsidy?","expected":"zh-expenses.md","hit":cross.evidence_cards.iter().any(|card|card.document_path.ends_with("zh-expenses.md")),"neuralModelsEnabled":false});
    let passed =
        reports.iter().all(|case| case["passed"] == true) && scope_passed && rebuild_passed;
    let acceptance = json!({"fixture":"Nexa-authored synthetic bilingual text, no user documents","documents":ingested.files_added,"ingestMs":ingest_ms,"cases":reports,"meanReciprocalRank":sum_rr/supported as f64,"sourceExclusionPassed":scope_passed,"rebuildPassed":rebuild_passed,"crossLanguageDiagnostic":cross_language,"passed":passed});

    let mut scale = Vec::new();
    for count in [1_000usize, 10_000, 50_000] {
        let path = workspace.path().join(format!("scale-{count}.sqlite"));
        let db = Database::new(&path)?;
        let source = db.add_source(CreateSourceInput {
            root_path: workspace.path().to_string_lossy().into(),
            include_globs: vec![],
            exclude_globs: vec![],
            watch_enabled: false,
        })?;
        let start = Instant::now();
        let chunks = (0..count)
            .map(|index| {
                let mut chunk = nexa_core::parse::chunk_plaintext(
                    &format!("资料{index}的报销标准与预算 allowance budget item {index}"),
                    2000,
                )
                .remove(0);
                chunk.chunk_index = index as i32;
                chunk
            })
            .collect();
        db.insert_document(
            &source.id,
            &nexa_core::parse::ParsedDocument {
                file_path: workspace
                    .path()
                    .join("synthetic.md")
                    .to_string_lossy()
                    .into(),
                file_name: "synthetic.md".into(),
                title: "Synthetic".into(),
                mime_type: "text/markdown".into(),
                file_size: 0,
                content_hash: "synthetic".into(),
                chunks,
                visual_artifacts: vec![],
                metadata: Default::default(),
            },
        )?;
        let embeddings = db
            .get_all_chunks()?
            .into_iter()
            .enumerate()
            .map(|(index, (id, _))| {
                (
                    id,
                    "synthetic-64".to_string(),
                    (0..64)
                        .map(|dimension| ((index + dimension * 17) % 101) as f32 / 101.)
                        .collect(),
                )
            })
            .collect::<Vec<_>>();
        db.batch_store_embeddings(&embeddings)?;
        let index_ms = start.elapsed().as_secs_f64() * 1000.;
        let vector = vec![0.5f32; 64];
        let first = Instant::now();
        let hits = search::vector_search_top_k(&db, &vector, "synthetic-64", 10, None)?;
        let first_ms = first.elapsed().as_secs_f64() * 1000.;
        let mut warm = Vec::new();
        let mut lexical = Vec::new();
        for _ in 0..12 {
            let start = Instant::now();
            search::vector_search_top_k(&db, &vector, "synthetic-64", 10, None)?;
            warm.push(start.elapsed().as_secs_f64() * 1000.);
            let start = Instant::now();
            search::search(&db, &query("报销标准"))?;
            lexical.push(start.elapsed().as_secs_f64() * 1000.);
        }
        drop(db);
        scale.push(json!({"chunks":count,"dimensions":64,"indexMs":index_ms,"firstQueryMs":first_ms,"warmVectorP50Ms":percentile(&mut warm,0.5),"warmVectorP95Ms":percentile(&mut warm,0.95),"warmLexicalP50Ms":percentile(&mut lexical,0.5),"warmLexicalP95Ms":percentile(&mut lexical,0.95),"indexBytes":std::fs::metadata(path)?.len(),"returned":hits.len()}));
    }
    let report = json!({"generatedAt":chrono::Utc::now().to_rfc3339(),"profile":if cfg!(debug_assertions){"debug"}else{"release"},"platform":std::env::consts::OS,"limits":"Synthetic fixtures; 64-dimensional vectors; first query is not an OS cold-cache measurement. No neural model quality, peak memory or VRAM claim.","acceptance":acceptance,"scaling":scale});
    std::fs::write(&output, serde_json::to_string_pretty(&report)?)?;
    println!("Saved retrieval report to {output}; acceptance passed: {passed}");
    if !passed {
        std::process::exit(1);
    }
    Ok(())
}
