//! Provider-specific request adapters behind one validated embedding boundary.
use super::Embedder;
use crate::embedding_provider_catalog::{
    find_embedding_model, find_embedding_provider, normalize_base_url, EmbeddingApiStyle,
};
use crate::error::CoreError;
use serde::Deserialize;
use serde_json::{json, Value};
use std::io::Read;

const MAX_RESPONSE_BYTES: u64 = 16 * 1024 * 1024;

pub struct ApiEmbedder {
    client: reqwest::blocking::Client,
    api_key: String,
    base_url: String,
    model: String,
    dimensions: usize,
    request_dimensions: bool,
    dimension_parameter: Option<String>,
    style: EmbeddingApiStyle,
    batch_size: usize,
    space_id: String,
}

struct ApiFailure {
    error: CoreError,
    retryable: bool,
}

impl From<CoreError> for ApiFailure {
    fn from(error: CoreError) -> Self {
        Self {
            error,
            retryable: false,
        }
    }
}

impl ApiEmbedder {
    fn space_identity(
        base_url: &str,
        model: &str,
        dimensions: usize,
        style: EmbeddingApiStyle,
    ) -> String {
        let identity = json!(["api-v2", base_url, model, dimensions, style]);
        format!(
            "api-v2:{model}:{}",
            blake3::hash(identity.to_string().as_bytes()).to_hex()
        )
    }

    /// Resolve the same identity without an HTTP client or credentials. Used by
    /// index readiness before a provider call or while editing unsaved settings.
    pub fn configured_space_id(config: &super::EmbedderConfig) -> String {
        let base = normalize_base_url(if config.api_base_url.is_empty() {
            "https://api.openai.com/v1"
        } else {
            &config.api_base_url
        });
        let model = if config.api_model.is_empty() {
            "text-embedding-3-small"
        } else {
            config.api_model.trim()
        };
        let dimensions = if config.vector_dimensions > 0 {
            config.vector_dimensions as usize
        } else {
            find_embedding_model(&base, model)
                .map(|model| model.dimensions)
                .unwrap_or(1536)
        };
        Self::space_identity(
            &base,
            model,
            dimensions,
            find_embedding_provider(&base)
                .map(|preset| preset.api_style)
                .unwrap_or_default(),
        )
    }

    pub fn new(
        api_key: String,
        base_url: Option<String>,
        model: Option<String>,
        dimensions: Option<usize>,
    ) -> Result<Self, CoreError> {
        let base_url =
            normalize_base_url(base_url.as_deref().unwrap_or("https://api.openai.com/v1"));
        let endpoint = url::Url::parse(&base_url)
            .map_err(|_| CoreError::Embedding("Invalid embedding API URL".into()))?;
        if !matches!(endpoint.scheme(), "http" | "https")
            || !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || endpoint.query().is_some()
            || endpoint.fragment().is_some()
        {
            return Err(CoreError::Embedding("Embedding API URL must be an HTTP(S) base URL without credentials, query or fragment".into()));
        }
        let loopback = endpoint.host_str().is_some_and(|host| {
            host == "localhost"
                || host
                    .trim_matches(['[', ']'])
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback())
        });
        if api_key.trim().is_empty() && !loopback {
            return Err(CoreError::Embedding(
                "API key is required for a remote embedding endpoint".into(),
            ));
        }
        let model = model
            .unwrap_or_else(|| "text-embedding-3-small".into())
            .trim()
            .to_string();
        if model.is_empty() {
            return Err(CoreError::Embedding("Embedding model is required".into()));
        }
        let preset = find_embedding_provider(&base_url);
        let catalog_model = find_embedding_model(&base_url, &model);
        let dimensions = dimensions
            .or_else(|| catalog_model.as_ref().map(|model| model.dimensions))
            .unwrap_or(1536);
        if dimensions == 0 || dimensions > 65536 {
            return Err(CoreError::Embedding(
                "Embedding dimensions must be between 1 and 65536".into(),
            ));
        }
        if let Some(model) = &catalog_model {
            let valid = if !model.supports_dimension_override {
                dimensions == model.dimensions
            } else if !model.allowed_dimensions.is_empty() {
                model.allowed_dimensions.contains(&dimensions)
            } else {
                dimensions >= model.min_dimensions.unwrap_or(1)
                    && dimensions <= model.max_dimensions.unwrap_or(model.dimensions)
            };
            if !valid {
                return Err(CoreError::Embedding(format!(
                    "Unsupported dimensions {dimensions} for {}",
                    model.id
                )));
            }
        }
        let request_dimensions = catalog_model
            .as_ref()
            .map(|model| model.supports_dimension_override)
            .unwrap_or(true);
        let dimension_parameter = catalog_model
            .as_ref()
            .and_then(|model| model.dimension_parameter.clone());
        let style = preset.map(|preset| preset.api_style).unwrap_or_default();
        let batch_size = catalog_model
            .as_ref()
            .and_then(|model| model.max_batch_size)
            .or_else(|| preset.map(|preset| preset.max_batch_size))
            .unwrap_or(100)
            .clamp(1, 100);
        // Query/document task semantics are versioned together. Credentials are
        // excluded; endpoints and dimensions are not interchangeable vector spaces.
        let space_id = Self::space_identity(&base_url, &model, dimensions, style);
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .map_err(|error| CoreError::Embedding(format!("HTTP client init: {error}")))?;
        Ok(Self {
            client,
            api_key,
            base_url,
            model,
            dimensions,
            request_dimensions,
            dimension_parameter,
            style,
            batch_size,
            space_id,
        })
    }

    fn request(&self, texts: &[&str], query: bool) -> (String, Value) {
        let (suffix, mut body) = match self.style {
            EmbeddingApiStyle::Cohere => (
                "/embed".into(),
                json!({"model": self.model, "texts": texts, "input_type": if query { "search_query" } else { "search_document" }, "embedding_types": ["float"]}),
            ),
            EmbeddingApiStyle::Gemini => {
                let model = format!("models/{}", self.model.trim_start_matches("models/"));
                let requests: Vec<_> = texts.iter().map(|text| {
                    let mut config = json!({"outputDimensionality": self.dimensions});
                    let text = if self.model.starts_with("gemini-embedding-2") {
                        if query { format!("task: search result | query: {text}") } else { format!("title: none | text: {text}") }
                    } else {
                        config["taskType"] = json!(if query { "RETRIEVAL_QUERY" } else { "RETRIEVAL_DOCUMENT" });
                        text.to_string()
                    };
                    json!({"model": model, "content": {"parts": [{"text": text}]}, "embedContentConfig": config})
                }).collect();
                (
                    format!("/{model}:batchEmbedContents"),
                    json!({"requests": requests}),
                )
            }
            _ => (
                "/embeddings".into(),
                json!({"input": texts, "model": self.model}),
            ),
        };
        match self.style {
            EmbeddingApiStyle::Voyage => {
                body["input_type"] = json!(if query { "query" } else { "document" });
            }
            EmbeddingApiStyle::Jina => {
                body["task"] = json!(if query {
                    "retrieval.query"
                } else {
                    "retrieval.passage"
                });
            }
            _ => {}
        }
        if self.request_dimensions && self.style != EmbeddingApiStyle::Gemini {
            let parameter = self.dimension_parameter.as_deref().unwrap_or(
                if matches!(
                    self.style,
                    EmbeddingApiStyle::Voyage | EmbeddingApiStyle::Cohere
                ) {
                    "output_dimension"
                } else {
                    "dimensions"
                },
            );
            body[parameter] = json!(self.dimensions);
        }
        (format!("{}{suffix}", self.base_url), body)
    }

    fn decode(&self, value: Value, expected: usize) -> Result<Vec<Vec<f32>>, CoreError> {
        let malformed =
            |message: &str| CoreError::Embedding(format!("Invalid embedding response: {message}"));
        let vectors: Vec<Vec<f32>> = match self.style {
            EmbeddingApiStyle::Cohere => {
                serde_json::from_value(value["embeddings"]["float"].clone())
                    .map_err(|_| malformed("missing float embeddings"))?
            }
            EmbeddingApiStyle::Gemini => {
                #[derive(Deserialize)]
                struct GeminiVector {
                    values: Vec<f32>,
                }
                let data: Vec<GeminiVector> = serde_json::from_value(value["embeddings"].clone())
                    .map_err(|_| malformed("missing embeddings"))?;
                data.into_iter().map(|entry| entry.values).collect()
            }
            _ => {
                #[derive(Deserialize)]
                struct IndexedVector {
                    index: usize,
                    embedding: Vec<f32>,
                }
                let mut data: Vec<IndexedVector> = serde_json::from_value(value["data"].clone())
                    .map_err(|_| malformed("missing indexed vectors"))?;
                data.sort_by_key(|entry| entry.index);
                if data
                    .iter()
                    .enumerate()
                    .any(|(index, entry)| entry.index != index)
                {
                    return Err(malformed("duplicate, missing or out-of-range input index"));
                }
                data.into_iter().map(|entry| entry.embedding).collect()
            }
        };
        if vectors.len() != expected {
            return Err(malformed("vector count does not match input count"));
        }
        if vectors.iter().any(|vector| {
            vector.len() != self.dimensions || vector.iter().any(|value| !value.is_finite())
        }) {
            return Err(malformed(&format!("expected {} finite dimensions per input; check the selected model and rebuild its index", self.dimensions)));
        }
        Ok(vectors)
    }

    fn call_api(&self, texts: &[&str], query: bool) -> Result<Vec<Vec<f32>>, ApiFailure> {
        let (url, body) = self.request(texts, query);
        let mut request = self.client.post(url).json(&body);
        if !self.api_key.trim().is_empty() {
            request = if self.style == EmbeddingApiStyle::Gemini {
                request.header("x-goog-api-key", &self.api_key)
            } else {
                request.bearer_auth(&self.api_key)
            };
        }
        let response = request.send().map_err(|error| ApiFailure {
            retryable: error.is_timeout() || error.is_connect(),
            error: CoreError::Embedding(format!("API request failed: {error}")),
        })?;
        let status = response.status();
        let mut bytes = Vec::new();
        response
            .take(MAX_RESPONSE_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| CoreError::Embedding(format!("API response read: {error}")))?;
        if bytes.len() as u64 > MAX_RESPONSE_BYTES {
            return Err(CoreError::Embedding("Embedding response exceeds 16 MiB".into()).into());
        }
        if !status.is_success() {
            let detail: String = String::from_utf8_lossy(&bytes).chars().take(1000).collect();
            return Err(ApiFailure {
                retryable: status.as_u16() == 429 || status.is_server_error(),
                error: CoreError::Embedding(format!("API returned HTTP {status}: {detail}")),
            });
        }
        let value = serde_json::from_slice(&bytes)
            .map_err(|error| CoreError::Embedding(format!("API response parse: {error}")))?;
        Ok(self.decode(value, texts.len())?)
    }

    fn call_api_with_retry(&self, texts: &[&str], query: bool) -> Result<Vec<Vec<f32>>, CoreError> {
        for attempt in 0..=3 {
            match self.call_api(texts, query) {
                Ok(result) => return Ok(result),
                Err(failure) if failure.retryable && attempt < 3 => {
                    std::thread::sleep(std::time::Duration::from_millis(200 * 2u64.pow(attempt)))
                }
                Err(failure) => return Err(failure.error),
            }
        }
        unreachable!()
    }
}

impl Embedder for ApiEmbedder {
    fn model_name(&self) -> &str {
        &self.model
    }
    fn vector_space_id(&self) -> &str {
        &self.space_id
    }
    fn dimensions(&self) -> usize {
        self.dimensions
    }
    fn embed(&self, text: &str) -> Result<Vec<f32>, CoreError> {
        self.embed_batch(&[text])?
            .into_iter()
            .next()
            .ok_or_else(|| CoreError::Embedding("Empty response from API".into()))
    }
    fn embed_query(&self, text: &str) -> Result<Vec<f32>, CoreError> {
        self.call_api_with_retry(&[text], true)?
            .into_iter()
            .next()
            .ok_or_else(|| CoreError::Embedding("Empty response from API".into()))
    }
    fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, CoreError> {
        let mut vectors = Vec::with_capacity(texts.len());
        for batch in texts.chunks(self.batch_size) {
            vectors.extend(self.call_api_with_retry(batch, false)?);
        }
        Ok(vectors)
    }
}

pub fn test_api_connection(
    api_key: &str,
    base_url: &str,
    model: &str,
    dimensions: u32,
) -> Result<bool, CoreError> {
    let embedder = ApiEmbedder::new(
        api_key.into(),
        Some(base_url.into()),
        Some(model.into()),
        (dimensions > 0).then_some(dimensions as usize),
    )?;
    embedder.embed_query("connection test")?;
    embedder.embed("connection test")?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::net::TcpListener;

    fn client(base: &str, model: &str, dimensions: usize) -> ApiEmbedder {
        ApiEmbedder::new(
            "test-key".into(),
            Some(base.into()),
            Some(model.into()),
            Some(dimensions),
        )
        .unwrap()
    }

    #[test]
    fn provider_requests_preserve_retrieval_tasks_and_dimension_dialects() {
        let voyage = client("https://api.voyageai.com/v1", "voyage-code-4", 512);
        let (_, body) = voyage.request(&["query"], true);
        assert_eq!(body["input_type"], "query");
        assert_eq!(body["output_dimension"], 512);
        assert!(body.get("dimensions").is_none());
        assert_eq!(
            voyage.request(&["document"], false).1["input_type"],
            "document"
        );
        let cohere = client("https://api.cohere.com/v2", "embed-v4.0", 256);
        let (url, body) = cohere.request(&["query"], true);
        assert_eq!(url, "https://api.cohere.com/v2/embed");
        assert_eq!(body["texts"], json!(["query"]));
        assert_eq!(body["input_type"], "search_query");
        assert_eq!(body["embedding_types"], json!(["float"]));
        let jina = client(
            "https://api.jina.ai/v1",
            "jina-embeddings-v5-text-small",
            256,
        );
        assert_eq!(
            jina.request(&["document"], false).1["task"],
            "retrieval.passage"
        );
        assert_eq!(jina.request(&["query"], true).1["task"], "retrieval.query");
        let mistral = client("https://api.mistral.ai/v1", "mistral-embed", 1024);
        assert!(mistral
            .request(&["document"], false)
            .1
            .get("dimensions")
            .is_none());
        let code = client("https://api.mistral.ai/v1", "codestral-embed", 3072);
        assert_eq!(code.request(&["code"], false).1["output_dimension"], 3072);
        let custom = client("https://api.voyageai.com/v1", "voyage-3.5", 512);
        assert_eq!(custom.request(&["code"], false).1["output_dimension"], 512);
    }

    #[test]
    fn gemini_batches_one_vector_per_input_and_uses_model_specific_tasks() {
        let gemini = client(
            "https://generativelanguage.googleapis.com/v1beta",
            "gemini-embedding-001",
            768,
        );
        let (url, body) = gemini.request(&["one", "two"], false);
        assert!(url.ends_with("/models/gemini-embedding-001:batchEmbedContents"));
        assert_eq!(body["requests"].as_array().unwrap().len(), 2);
        assert_eq!(body["requests"][1]["content"]["parts"][0]["text"], "two");
        assert_eq!(
            body["requests"][0]["embedContentConfig"]["taskType"],
            "RETRIEVAL_DOCUMENT"
        );
        assert_eq!(
            body["requests"][0]["embedContentConfig"]["outputDimensionality"],
            768
        );
        let gemini2 = client(
            "https://generativelanguage.googleapis.com/v1beta",
            "gemini-embedding-2",
            768,
        );
        let body = gemini2.request(&["find me"], true).1;
        assert!(body["requests"][0]["embedContentConfig"]
            .get("taskType")
            .is_none());
        assert_eq!(
            body["requests"][0]["content"]["parts"][0]["text"],
            "task: search result | query: find me"
        );
    }

    #[test]
    fn space_identity_changes_with_endpoint_model_or_dimensions_but_not_keys() {
        let a = client("https://api.openai.com/v1", "text-embedding-3-small", 512);
        let rotated = ApiEmbedder::new(
            "rotated".into(),
            Some("https://API.OPENAI.COM/v1/".into()),
            Some("text-embedding-3-small".into()),
            Some(512),
        )
        .unwrap();
        assert_eq!(a.vector_space_id(), rotated.vector_space_id());
        assert_eq!(a.model_name(), "text-embedding-3-small");
        for other in [
            client("https://api.openai.com/v1", "text-embedding-3-small", 256),
            client("https://api.openai.com/v1", "text-embedding-3-large", 512),
            client("https://other.example/v1", "text-embedding-3-small", 512),
        ] {
            assert_ne!(a.vector_space_id(), other.vector_space_id());
        }
        assert_ne!(
            a.vector_space_id(),
            a.model_name(),
            "legacy model-only rows must not be reused"
        );
    }

    #[test]
    fn fixed_and_provider_specific_dimensions_are_validated_before_requests() {
        for (base, model, dimension) in [
            ("https://api.mistral.ai/v1", "mistral-embed", 512),
            (
                "https://api.siliconflow.com/v1",
                "Qwen/Qwen3-Embedding-4B",
                2560,
            ),
            (
                "https://workspace.cn-beijing.maas.aliyuncs.com/compatible-mode/v1",
                "text-embedding-v4",
                123,
            ),
            ("https://api.jina.ai/v1", "jina-embeddings-v3", 16),
            (
                "https://generativelanguage.googleapis.com/v1beta",
                "gemini-embedding-2",
                64,
            ),
        ] {
            assert!(ApiEmbedder::new(
                "key".into(),
                Some(base.into()),
                Some(model.into()),
                Some(dimension)
            )
            .is_err());
        }
    }

    #[test]
    fn malformed_vectors_never_enter_the_index() {
        let c = client("http://localhost:1234/v1", "custom", 2);
        for response in [
            json!({"data":[]}),
            json!({"data":[{"index":1,"embedding":[1,2]}]}),
            json!({"data":[{"index":0,"embedding":[1]}]}),
            json!({"data":[{"index":0,"embedding":[1e39,2]}]}),
            json!({"data":[{"index":0,"embedding":[1,2]},{"index":0,"embedding":[3,4]}]}),
        ] {
            assert!(c.decode(response, 1).is_err());
        }
        assert_eq!(
            c.decode(
                json!({"data":[{"index":1,"embedding":[3,4]},{"index":0,"embedding":[1,2]}]}),
                2
            )
            .unwrap(),
            vec![vec![1., 2.], vec![3., 4.]]
        );
        let cohere = client("https://api.cohere.com/v2", "embed-v4.0", 256);
        assert!(cohere
            .decode(json!({"embeddings":{"float":[[1,2]]}}), 1)
            .is_err());
    }

    fn server(responses: Vec<Value>) -> (String, std::thread::JoinHandle<Vec<(String, Value)>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let thread = std::thread::spawn(move || {
            let mut requests = Vec::new();
            for response in responses {
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
                let mut socket = loop {
                    if let Ok((socket, _)) = listener.accept() {
                        break socket;
                    }
                    assert!(std::time::Instant::now() < deadline, "missing API request");
                    std::thread::sleep(std::time::Duration::from_millis(5));
                };
                socket.set_nonblocking(false).unwrap();
                socket
                    .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                    .unwrap();
                let mut bytes = Vec::new();
                let (header, body) = loop {
                    let mut buffer = [0; 4096];
                    let read = socket.read(&mut buffer).unwrap();
                    assert!(read > 0);
                    bytes.extend_from_slice(&buffer[..read]);
                    if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                        let header = String::from_utf8(bytes[..end].to_vec()).unwrap();
                        let length: usize = header
                            .lines()
                            .find_map(|line| {
                                line.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|value| value.trim().parse().unwrap())
                            })
                            .unwrap();
                        if bytes.len() >= end + 4 + length {
                            break (
                                header,
                                serde_json::from_slice(&bytes[end + 4..end + 4 + length]).unwrap(),
                            );
                        }
                    }
                };
                requests.push((header, body));
                let body = response.to_string();
                write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            }
            requests
        });
        (base, thread)
    }

    #[test]
    fn real_http_honors_qwen_batch_limit_and_input_order() {
        let response = |count| json!({"data":(0..count).rev().map(|index| json!({"index":index,"embedding":vec![index as f32;64]})).collect::<Vec<_>>()});
        let (base, server) = server(vec![response(10), response(1)]);
        let mut c = client(
            "https://dashscope.aliyuncs.com/compatible-mode/v1",
            "text-embedding-v4",
            64,
        );
        c.base_url = base;
        let vectors = c.embed_batch(&["text"; 11]).unwrap();
        assert_eq!(vectors.len(), 11);
        assert_eq!(vectors[9][0], 9.);
        let requests = server.join().unwrap();
        assert_eq!(requests[0].1["input"].as_array().unwrap().len(), 10);
        assert_eq!(requests[1].1["input"].as_array().unwrap().len(), 1);
        assert!(requests[0].0.starts_with("POST /embeddings "));
        assert!(requests[0]
            .0
            .to_ascii_lowercase()
            .contains("authorization: bearer test-key"));
    }

    #[test]
    fn real_http_supports_keyless_local_and_native_gemini_auth() {
        let (base, local) = server(vec![json!({"data":[{"index":0,"embedding":[1,2]}]})]);
        let c = ApiEmbedder::new(String::new(), Some(base), Some("local".into()), Some(2)).unwrap();
        assert_eq!(c.embed_query("query").unwrap(), vec![1., 2.]);
        assert!(!local.join().unwrap()[0]
            .0
            .to_ascii_lowercase()
            .contains("authorization:"));
        assert!(ApiEmbedder::new(
            String::new(),
            Some("https://example.com/v1".into()),
            None,
            None
        )
        .is_err());
        let (base, native) = server(vec![json!({"embeddings":[{"values":vec![1.;128]}]})]);
        let mut c = client(
            "https://generativelanguage.googleapis.com/v1beta",
            "gemini-embedding-001",
            128,
        );
        c.base_url = base;
        assert_eq!(c.embed_query("query").unwrap().len(), 128);
        let requests = native.join().unwrap();
        assert!(requests[0]
            .0
            .to_ascii_lowercase()
            .contains("x-goog-api-key: test-key"));
        assert!(!requests[0]
            .0
            .to_ascii_lowercase()
            .contains("authorization:"));
    }
}
