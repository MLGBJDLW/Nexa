//! Optional local inference services. SQLite remains the source of truth.
use crate::{db::Database, error::CoreError, models::EvidenceCard, parse::ParsedDocument};
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use std::{io::Read, path::Path, time::Duration};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct KnowledgeServicesConfig {
    pub reranker_url: String,
    pub parser_url: String,
    pub reranker_timeout_ms: u64,
    pub parser_timeout_seconds: u64,
    pub max_candidates: usize,
}
impl Default for KnowledgeServicesConfig {
    fn default() -> Self {
        Self {
            reranker_url: String::new(),
            parser_url: String::new(),
            reranker_timeout_ms: 5000,
            parser_timeout_seconds: 180,
            max_candidates: 64,
        }
    }
}

fn endpoint(value: &str) -> Result<reqwest::Url, CoreError> {
    let url = reqwest::Url::parse(value)
        .map_err(|_| CoreError::InvalidInput("Invalid local knowledge service URL".into()))?;
    if url.scheme() != "http"
        || !matches!(url.host_str(), Some("127.0.0.1" | "[::1]"))
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(CoreError::InvalidInput("Knowledge services require an HTTP loopback address (127.0.0.1 or [::1]) without credentials, query or fragment".into()));
    }
    Ok(url)
}
impl KnowledgeServicesConfig {
    pub fn validate(&self) -> Result<(), CoreError> {
        for value in [&self.reranker_url, &self.parser_url] {
            if !value.is_empty() {
                endpoint(value)?;
            }
        }
        if !(100..=15_000).contains(&self.reranker_timeout_ms)
            || !(1..=300).contains(&self.parser_timeout_seconds)
            || !(1..=128).contains(&self.max_candidates)
        {
            return Err(CoreError::InvalidInput(
                "Knowledge service budgets are out of range".into(),
            ));
        }
        Ok(())
    }
}
impl Database {
    pub fn knowledge_services_config(&self) -> Result<KnowledgeServicesConfig, CoreError> {
        let value: Option<String> = self
            .conn()
            .query_row(
                "SELECT value FROM knowledge_service_config WHERE key='knowledge_services'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        let config: KnowledgeServicesConfig = value
            .map(|value| serde_json::from_str(&value))
            .transpose()?
            .unwrap_or_default();
        config.validate()?;
        Ok(config)
    }
    pub fn save_knowledge_services_config(
        &self,
        config: &KnowledgeServicesConfig,
    ) -> Result<(), CoreError> {
        config.validate()?;
        self.conn().execute("INSERT INTO knowledge_service_config(key,value) VALUES('knowledge_services',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [serde_json::to_string(config)?])?;
        Ok(())
    }
}

fn post_json(
    url: &str,
    value: serde_json::Value,
    timeout: Duration,
    max_bytes: u64,
) -> Result<serde_json::Value, CoreError> {
    let url = endpoint(url)?;
    // reqwest's blocking runtime must never be constructed/dropped on a Tokio worker.
    std::thread::spawn(move || {
        let client = reqwest::blocking::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(timeout)
            .build()
            .map_err(|error| CoreError::InvalidInput(error.to_string()))?;
        let response = client.post(url).json(&value).send().map_err(|error| {
            CoreError::InvalidInput(format!("Local knowledge service unavailable: {error}"))
        })?;
        if !response.status().is_success() {
            return Err(CoreError::InvalidInput(format!(
                "Local knowledge service returned HTTP {}",
                response.status()
            )));
        }
        let mut bytes = Vec::new();
        response.take(max_bytes + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > max_bytes {
            return Err(CoreError::InvalidInput(
                "Local knowledge service response exceeds its budget".into(),
            ));
        }
        Ok(serde_json::from_slice(&bytes)?)
    })
    .join()
    .map_err(|_| CoreError::InvalidInput("Local knowledge service worker failed".into()))?
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RankingReport {
    pub method: String,
    pub candidates: usize,
    pub elapsed_ms: u64,
    pub fallback_reason: Option<String>,
}

/// TEI cross-encoder /rerank contract: scores for every input, indexed by position.
/// Reject partial, duplicate, non-finite and out-of-range responses atomically.
pub fn rerank(
    db: &Database,
    query: &str,
    cards: &mut [EvidenceCard],
) -> Result<RankingReport, CoreError> {
    let config = db.knowledge_services_config()?;
    let count = cards.len().min(config.max_candidates);
    let start = std::time::Instant::now();
    let mut report = RankingReport {
        method: "lexical_rules".into(),
        candidates: count,
        elapsed_ms: 0,
        fallback_reason: None,
    };
    if config.reranker_url.is_empty() || count == 0 {
        return Ok(report);
    }
    let result = (|| {
        let texts: Vec<String> = cards[..count]
            .iter()
            .map(|card| {
                crate::rag::build_contextual_search_text(card)
                    .chars()
                    .take(6000)
                    .collect()
            })
            .collect();
        let value = post_json(
            &config.reranker_url,
            serde_json::json!({"query": query.chars().take(2000).collect::<String>(), "texts":texts,"raw_scores":false,"return_text":false}),
            Duration::from_millis(config.reranker_timeout_ms),
            128 * 1024,
        )?;
        decode_scores(value, count)
    })();
    match result {
        Ok(scores) => {
            for (card, score) in cards[..count].iter_mut().zip(scores) {
                card.score = score;
            }
            // Only scored candidates compete with semantic scores, which use a different scale.
            for card in &mut cards[count..] {
                card.score = -1.0;
            }
            cards.sort_by(|a, b| b.score.total_cmp(&a.score));
            report.method = "semantic_cross_encoder".into();
        }
        Err(error) => report.fallback_reason = Some(error.to_string()),
    }
    report.elapsed_ms = start.elapsed().as_millis() as u64;
    Ok(report)
}
fn decode_scores(value: serde_json::Value, count: usize) -> Result<Vec<f64>, CoreError> {
    #[derive(Deserialize)]
    struct Score {
        index: usize,
        score: f64,
    }
    let values: Vec<Score> = serde_json::from_value(value)?;
    let mut scores = vec![None; count];
    for value in values {
        if value.index >= count
            || !value.score.is_finite()
            || !(0.0..=1.0).contains(&value.score)
            || scores[value.index].replace(value.score).is_some()
        {
            return Err(CoreError::InvalidInput(
                "Reranker returned invalid candidate scores".into(),
            ));
        }
    }
    scores
        .into_iter()
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| CoreError::InvalidInput("Reranker omitted candidates".into()))
}

/// Enhanced PDF parser contract implemented by scripts/knowledge/docling_service.py.
/// Failures retain the previous index and become source scan errors.
pub fn parse_pdf(
    config: &KnowledgeServicesConfig,
    path: &Path,
    max_chars: usize,
) -> Result<Option<ParsedDocument>, CoreError> {
    if config.parser_url.is_empty()
        || !path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("pdf"))
    {
        return Ok(None);
    }
    config.validate()?;
    let hash = crate::parse::hash_file_content(path)?;
    let absolute = std::fs::canonicalize(path)?;
    let value = post_json(
        &config.parser_url,
        serde_json::json!({"path":absolute.to_string_lossy()}),
        Duration::from_secs(config.parser_timeout_seconds),
        16 * 1024 * 1024,
    )?;
    if hash != crate::parse::hash_file_content(path)? {
        return Err(CoreError::Conflict(
            "File changed during enhanced parsing".into(),
        ));
    }
    let mut parsed = decode_document(value, path, max_chars)?;
    parsed.content_hash = hash;
    Ok(Some(parsed))
}
fn decode_document(
    value: serde_json::Value,
    path: &Path,
    max_chars: usize,
) -> Result<ParsedDocument, CoreError> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Block {
        text: String,
        page: Option<u32>,
        #[serde(default)]
        source_pages: Vec<u32>,
        section: Option<String>,
        #[serde(default)]
        bbox: Option<[f32; 4]>,
        #[serde(default)]
        heading: Option<String>,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Response {
        protocol: u32,
        parser_version: String,
        page_count: u32,
        blocks: Vec<Block>,
    }
    let response: Response = serde_json::from_value(value)?;
    if !matches!(response.protocol, 1 | 2)
        || response.page_count == 0
        || response.blocks.len() > 100_000
        || response.parser_version.is_empty()
    {
        return Err(CoreError::InvalidInput(
            "Enhanced parser returned an invalid document".into(),
        ));
    }
    let mut chunks = Vec::new();
    let mut covered = std::collections::HashSet::new();
    let mut unlocated = 0;
    for block in response.blocks {
        let valid_page = |page: u32| page > 0 && page <= response.page_count;
        let location_valid = match block.page {
            Some(page) => {
                valid_page(page) && block.source_pages.is_empty() && block.section.is_none()
            }
            None => {
                response.protocol == 2
                    && block.bbox.is_none()
                    && !block.source_pages.is_empty()
                    && block.source_pages.iter().all(|&page| valid_page(page))
                    && block
                        .section
                        .as_deref()
                        .is_some_and(|section| !section.trim().is_empty())
            }
        };
        if !location_valid
            || block
                .bbox
                .is_some_and(|bbox| bbox.iter().any(|value| !value.is_finite()))
        {
            return Err(CoreError::InvalidInput(
                "Enhanced parser returned an invalid page location".into(),
            ));
        }
        let located = match block.page {
            Some(page) => crate::evidence::EvidenceLocator::Pdf {
                page,
                bbox: block.bbox,
            },
            None => {
                unlocated += 1;
                crate::evidence::EvidenceLocator::Extracted {
                    section: block.section.unwrap_or_default(),
                }
            }
        };
        for mut chunk in crate::parse::chunk_plaintext(&block.text, max_chars.max(100)) {
            chunk.locator = located.clone();
            chunk.extraction_method = "docling_layout_ocr".into();
            chunk.heading_context = block.heading.clone();
            chunk.chunk_index = chunks.len() as i32;
            chunks.push(chunk);
            covered.extend(
                block
                    .page
                    .into_iter()
                    .chain(block.source_pages.iter().copied()),
            );
        }
    }
    if chunks.is_empty() {
        return Err(CoreError::InvalidInput(
            "Enhanced parser returned no searchable text".into(),
        ));
    }
    let mut metadata = std::collections::HashMap::from([
        (
            "parser_profile".into(),
            format!("docling-v{}:{}", response.protocol, response.parser_version),
        ),
        ("page_count".into(), response.page_count.to_string()),
        ("searchable_pages".into(), covered.len().to_string()),
    ]);
    let mut warnings = Vec::new();
    if covered.len() < response.page_count as usize {
        warnings.push(format!(
            "{} pages have no searchable text",
            response.page_count as usize - covered.len()
        ));
    }
    if unlocated > 0 {
        warnings.push(format!(
            "{unlocated} extracted blocks have no exact page location"
        ));
    }
    if !warnings.is_empty() {
        metadata.insert("parse_warnings".into(), warnings.join("; "));
    }
    let file_name = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    Ok(ParsedDocument {
        file_path: path.to_string_lossy().into(),
        file_name: file_name.clone(),
        title: file_name,
        mime_type: "application/pdf".into(),
        file_size: std::fs::metadata(path)?.len() as i64,
        content_hash: String::new(),
        chunks,
        visual_artifacts: Vec::new(),
        metadata,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    fn service(
        response: serde_json::Value,
    ) -> (String, std::thread::JoinHandle<serde_json::Value>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let thread = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut data = Vec::new();
            let mut buffer = [0; 8192];
            let body = loop {
                let size = stream.read(&mut buffer).unwrap();
                assert!(size > 0);
                data.extend_from_slice(&buffer[..size]);
                if let Some(end) = data.windows(4).position(|part| part == b"\r\n\r\n") {
                    let header = String::from_utf8_lossy(&data[..end]).to_ascii_lowercase();
                    let length: usize = header
                        .lines()
                        .find_map(|line| {
                            line.strip_prefix("content-length:")
                                .map(|value| value.trim().parse().unwrap())
                        })
                        .unwrap();
                    if data.len() >= end + 4 + length {
                        break serde_json::from_slice(&data[end + 4..end + 4 + length]).unwrap();
                    }
                }
            };
            let payload = response.to_string();
            write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",payload.len(),payload).unwrap();
            body
        });
        (format!("http://{address}/"), thread)
    }
    fn source(db: &Database, path: &Path, glob: &str) -> String {
        db.add_source(crate::sources::CreateSourceInput {
            root_path: path.to_string_lossy().into(),
            include_globs: vec![glob.into()],
            exclude_globs: vec![],
            watch_enabled: false,
        })
        .unwrap()
        .id
    }
    #[test]
    fn validates_local_routes_and_complete_scores() {
        assert!(endpoint("http://127.0.0.1:8080/rerank").is_ok());
        assert!(endpoint("http://[::1]:8080/rerank").is_ok());
        for url in [
            "https://example.com/rerank",
            "http://localhost/rerank",
            "http://user:pass@127.0.0.1/rerank",
        ] {
            assert!(endpoint(url).is_err());
        }
        assert_eq!(
            decode_scores(
                serde_json::json!([{"index":1,"score":0.9},{"index":0,"score":0.1}]),
                2
            )
            .unwrap(),
            vec![0.1, 0.9]
        );
        for value in [
            serde_json::json!([{"index":0,"score":0.9}]),
            serde_json::json!([{"index":0,"score":0.9},{"index":0,"score":0.1}]),
            serde_json::json!([{"index":2,"score":0.9}]),
        ] {
            assert!(decode_scores(value, 2).is_err());
        }
    }
    #[test]
    fn multi_page_extracted_blocks_retain_text_without_claiming_a_pdf_page() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let value = serde_json::json!({"protocol":2,"parserVersion":"nexa-adapter-2:test","pageCount":2,"blocks":[
            {"text":"| first-page | 10 |\n| later-page-only | 20 |","sourcePages":[1,2],"section":"Pages 1, 2"}
        ]});
        let parsed = decode_document(value.clone(), file.path(), 1000).unwrap();
        assert_eq!(parsed.chunks.len(), 1);
        assert!(parsed.chunks[0].content.contains("later-page-only"));
        assert!(matches!(
            parsed.chunks[0].locator,
            crate::evidence::EvidenceLocator::Extracted { .. }
        ));
        assert_eq!(parsed.metadata["searchable_pages"], "2");
        assert!(parsed.metadata["parse_warnings"].contains("exact page location"));
        for patch in [
            serde_json::json!({"bbox":[0.1,0.1,0.5,0.5]}),
            serde_json::json!({"sourcePages":[]}),
            serde_json::json!({"sourcePages":[0,3]}),
            serde_json::json!({"section":""}),
            serde_json::json!({"page":1}),
        ] {
            let mut invalid = value.clone();
            invalid["blocks"][0]
                .as_object_mut()
                .unwrap()
                .extend(patch.as_object().unwrap().clone());
            assert!(decode_document(invalid, file.path(), 1000).is_err());
        }
    }
    #[test]
    fn enhanced_parser_runs_through_ingestion_with_page_provenance() {
        let folder = tempfile::tempdir().unwrap();
        std::fs::write(
            folder.path().join("mixed.pdf"),
            b"%PDF fixture handled by the service",
        )
        .unwrap();
        let (url, request) = service(
            serde_json::json!({"protocol":1,"parserVersion":"fixture-layout","pageCount":2,"blocks":[{"page":1,"text":"原文报销标准为500元。","bbox":[0.1,0.1,0.5,0.2]},{"page":2,"text":"Appendix: audit deadline is Friday."}]}),
        );
        let db = Database::open_memory().unwrap();
        db.save_knowledge_services_config(&KnowledgeServicesConfig {
            parser_url: url,
            ..Default::default()
        })
        .unwrap();
        let source = source(&db, folder.path(), "**/*.pdf");
        let result = crate::ingest::scan_source(&db, &source).unwrap();
        assert_eq!(result.files_added, 1);
        assert_eq!(result.files_failed, 0);
        assert!(request.join().unwrap()["path"]
            .as_str()
            .unwrap()
            .ends_with("mixed.pdf"));
        let result = crate::search::search(
            &db,
            &crate::models::SearchQuery {
                text: "报销标准".into(),
                filters: Default::default(),
                limit: 5,
                offset: 0,
            },
        )
        .unwrap();
        let reference = result.evidence_cards[0].evidence_ref.as_ref().unwrap();
        assert!(matches!(
            reference.locator,
            crate::evidence::EvidenceLocator::Pdf { page: 1, .. }
        ));
        assert_eq!(reference.extraction_method, "docling_layout_ocr");
        // Unchanged PDF bytes must still replace old mappings after an adapter upgrade.
        let (url, request) = service(
            serde_json::json!({"protocol":2,"parserVersion":"nexa-adapter-2:fixture-layout","pageCount":2,"blocks":[
                {"text":"| original | 10 |\n| laterpagerow | 20 |","sourcePages":[1,2],"section":"Pages 1, 2"}
            ]}),
        );
        db.save_knowledge_services_config(&KnowledgeServicesConfig {
            parser_url: url,
            ..Default::default()
        })
        .unwrap();
        let upgraded = crate::ingest::scan_source(&db, &source).unwrap();
        assert_eq!(upgraded.files_updated, 1);
        assert_eq!(upgraded.files_failed, 0);
        request.join().unwrap();
        let matches = crate::search::search(
            &db,
            &crate::models::SearchQuery {
                text: "laterpagerow".into(),
                filters: Default::default(),
                limit: 10,
                offset: 0,
            },
        )
        .unwrap();
        assert_eq!(matches.evidence_cards.len(), 1);
        let updated = matches.evidence_cards[0].evidence_ref.as_ref().unwrap();
        assert_eq!(updated.document_id, reference.document_id);
        assert_ne!(updated.revision, reference.revision);
        assert!(matches!(
            updated.locator,
            crate::evidence::EvidenceLocator::Extracted { .. }
        ));
        assert!(db.integrity_check().unwrap());
    }
    #[test]
    fn semantic_reranking_sees_candidates_before_the_user_limit_and_fails_atomically() {
        let folder = tempfile::tempdir().unwrap();
        std::fs::write(
            folder.path().join("one.md"),
            "retrieval ranking first candidate",
        )
        .unwrap();
        std::fs::write(
            folder.path().join("two.md"),
            "retrieval ranking second candidate with additional context",
        )
        .unwrap();
        let db = Database::open_memory().unwrap();
        let source = source(&db, folder.path(), "**/*.md");
        crate::ingest::scan_source(&db, &source).unwrap();
        let query = crate::models::SearchQuery {
            text: "retrieval ranking".into(),
            filters: Default::default(),
            limit: 1,
            offset: 0,
        };
        let first = crate::search::search(&db, &query)
            .unwrap()
            .evidence_cards
            .remove(0);
        let (url, request) =
            service(serde_json::json!([{"index":0,"score":0.01},{"index":1,"score":0.99}]));
        db.save_knowledge_services_config(&KnowledgeServicesConfig {
            reranker_url: url,
            ..Default::default()
        })
        .unwrap();
        let ranked = crate::search::hybrid_search(&db, &query).unwrap();
        assert_eq!(
            request.join().unwrap()["texts"].as_array().unwrap().len(),
            2
        );
        assert_eq!(ranked.ranking.unwrap().method, "semantic_cross_encoder");
        assert_ne!(ranked.evidence_cards[0].chunk_id, first.chunk_id);
        let (url, request) =
            service(serde_json::json!([{"index":0,"score":0.9},{"index":0,"score":0.1}]));
        db.save_knowledge_services_config(&KnowledgeServicesConfig {
            reranker_url: url,
            ..Default::default()
        })
        .unwrap();
        let fallback = crate::search::search(&db, &query).unwrap();
        request.join().unwrap();
        assert!(fallback.ranking.unwrap().fallback_reason.is_some());
        assert_eq!(fallback.evidence_cards[0].chunk_id, first.chunk_id);
    }
}
