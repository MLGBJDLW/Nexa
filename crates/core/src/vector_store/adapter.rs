use super::config::{VectorStoreConfig, VectorStoreProvider as Provider};
use crate::error::CoreError;
use serde_json::{json, Value};
use std::io::Read;

pub(super) struct VectorRecord {
    pub chunk_id: String,
    pub source_id: String,
    pub embedding_id: String,
    pub revision: i64,
    pub vector: Vec<f32>,
}
impl VectorRecord {
    pub fn version(&self) -> String {
        format!("{}:{}", self.embedding_id, self.revision)
    }
}
#[derive(Clone)]
pub(crate) struct CloudHit {
    pub chunk_id: String,
    pub source_id: String,
    pub version: String,
}

pub(super) struct RemoteStore {
    pub config: VectorStoreConfig,
    pub space: String,
    pub collection: String,
    pub dimensions: usize,
    client: reqwest::blocking::Client,
}

impl RemoteStore {
    pub fn new(
        config: &VectorStoreConfig,
        owner: &str,
        model_space: &str,
        dimensions: usize,
        timeout: std::time::Duration,
    ) -> Result<Self, CoreError> {
        let mut validation = config.clone();
        validation.mode = super::VectorSearchMode::Cloud;
        validation.validate()?;
        if dimensions == 0
            || dimensions > 65536
            || (config.provider == Provider::Tencent && dimensions > 4096)
            || (config.provider == Provider::Dashvector && !(2..=20000).contains(&dimensions))
        {
            return Err(CoreError::InvalidInput(
                "Embedding dimensions are outside this vector store's supported range".into(),
            ));
        }
        let space = blake3::hash(format!("{owner}\0{model_space}").as_bytes())
            .to_hex()
            .to_string();
        let collection = format!("{}_{}", config.collection_prefix, &space[..16]);
        let client = reqwest::blocking::Client::builder()
            .timeout(timeout)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| CoreError::Embedding(e.to_string()))?;
        Ok(Self {
            config: config.clone(),
            space,
            collection,
            dimensions,
            client,
        })
    }
    pub fn point_id(&self, chunk: &str) -> String {
        let hash = blake3::hash(format!("{}\0{chunk}", self.space).as_bytes());
        uuid::Uuid::from_bytes(hash.as_bytes()[..16].try_into().expect("16 bytes")).to_string()
    }
    fn error(&self, message: impl Into<String>) -> CoreError {
        let message = message.into();
        let safe = if self.config.api_key.is_empty() {
            message
        } else {
            message.replace(&self.config.api_key, "[redacted]")
        };
        CoreError::Embedding(format!(
            "Vector store: {}",
            safe.chars().take(1000).collect::<String>()
        ))
    }
    fn request(&self, method: &str, path: &str, body: Option<Value>) -> Result<Value, CoreError> {
        let method = reqwest::Method::from_bytes(method.as_bytes())
            .map_err(|e| self.error(e.to_string()))?;
        let mut request = self.client.request(
            method,
            format!(
                "{}{}",
                self.config.endpoint.trim().trim_end_matches('/'),
                path
            ),
        );
        request = match self.config.provider {
            Provider::Qdrant => request.header("api-key", &self.config.api_key),
            Provider::Pinecone => request
                .header("Api-Key", &self.config.api_key)
                .header("X-Pinecone-Api-Version", "2026-07"),
            Provider::Dashvector => request.header("dashvector-auth-token", &self.config.api_key),
            Provider::Milvus => request.bearer_auth(&self.config.api_key),
            Provider::Tencent => request.header(
                "Authorization",
                format!(
                    "Bearer account={}&api_key={}",
                    self.config.account, self.config.api_key
                ),
            ),
        };
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request.send().map_err(|e| self.error(e.to_string()))?;
        let status = response.status();
        let mut bytes = Vec::new();
        response
            .take(16 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| self.error(e.to_string()))?;
        if bytes.len() > 16 * 1024 * 1024 {
            return Err(self.error("Response exceeded 16 MiB"));
        }
        if !status.is_success() {
            return Err(self.error(format!(
                "HTTP {status}: {}",
                String::from_utf8_lossy(&bytes)
            )));
        }
        let value: Value =
            serde_json::from_slice(&bytes).map_err(|e| self.error(format!("Invalid JSON: {e}")))?;
        if matches!(
            self.config.provider,
            Provider::Dashvector | Provider::Milvus | Provider::Tencent
        ) && value["code"].as_i64() != Some(0)
        {
            return Err(self.error(value.to_string()));
        }
        if self.config.provider == Provider::Qdrant && value["status"] != "ok" {
            return Err(self.error(value.to_string()));
        }
        Ok(value)
    }
    fn base(&self) -> Value {
        json!({"collectionName":self.collection,"dbName":self.config.database})
    }
    fn tencent_base(&self) -> Value {
        json!({"database":self.config.database,"collection":self.collection})
    }

    /// Read-only probe. Infrastructure/index creation is never a query side effect.
    pub fn probe(&self) -> Result<Value, CoreError> {
        match self.config.provider {
            Provider::Qdrant => self.request("GET", "/collections", None),
            Provider::Pinecone => self.request("POST", "/describe_index_stats", Some(json!({}))),
            Provider::Dashvector => self.request("GET", "/v1/collections", None),
            Provider::Milvus => self.request(
                "POST",
                "/v2/vectordb/collections/list",
                Some(json!({"dbName":self.config.database})),
            ),
            Provider::Tencent => self.request(
                "POST",
                "/collection/list",
                Some(json!({"database":self.config.database})),
            ),
        }
    }
    pub fn ensure_collection(&self) -> Result<(), CoreError> {
        let listed = self.probe()?;
        let exists = match self.config.provider {
            Provider::Pinecone => {
                if listed["dimension"].as_u64() != Some(self.dimensions as u64) {
                    return Err(self.error("Pinecone index dimension does not match. Configure an existing dense index with the selected embedding dimensions."));
                }
                return Ok(());
            }
            Provider::Qdrant => listed["result"]["collections"]
                .as_array()
                .ok_or_else(|| self.error("Invalid collection list"))?
                .iter()
                .any(|v| v["name"] == self.collection),
            Provider::Dashvector => listed["output"]
                .as_array()
                .ok_or_else(|| self.error("Invalid collection list"))?
                .iter()
                .any(|v| v.as_str() == Some(&self.collection)),
            Provider::Milvus => listed["data"]
                .as_array()
                .ok_or_else(|| self.error("Invalid collection list"))?
                .iter()
                .any(|v| v.as_str() == Some(&self.collection)),
            Provider::Tencent => listed["collections"]
                .as_array()
                .ok_or_else(|| self.error("Invalid collection list"))?
                .iter()
                .any(|v| v["collection"] == self.collection),
        };
        if !exists {
            match self.config.provider {
                Provider::Qdrant => {
                    self.request(
                        "PUT",
                        &format!("/collections/{}", self.collection),
                        Some(json!({"vectors":{"size":self.dimensions,"distance":"Cosine"}})),
                    )?;
                }
                Provider::Dashvector => {
                    self.request("POST","/v1/collections",Some(json!({"name":self.collection,"dimension":self.dimensions,"metric":"cosine","fields_schema":{"chunkId":"STRING","sourceId":"STRING","spaceId":"STRING","version":"STRING"}})))?;
                }
                Provider::Milvus => {
                    let mut body = self.base();
                    body["dimension"] = json!(self.dimensions);
                    body["metricType"] = json!("COSINE");
                    body["idType"] = json!("VarChar");
                    body["autoID"] = json!(false);
                    body["primaryFieldName"] = json!("id");
                    body["vectorFieldName"] = json!("vector");
                    body["params"] = json!({"max_length":128,"enableDynamicField":true});
                    self.request("POST", "/v2/vectordb/collections/create", Some(body))?;
                }
                Provider::Tencent => {
                    let mut body = self.tencent_base();
                    body["replicaNum"] = json!(1);
                    body["shardNum"] = json!(1);
                    body["indexes"] = json!([
                    {"fieldName":"id","fieldType":"string","indexType":"primaryKey"},
                    {"fieldName":"vector","fieldType":"vector","indexType":"HNSW","dimension":self.dimensions,"metricType":"COSINE","params":{"M":16,"efConstruction":200}},
                    {"fieldName":"sourceId","fieldType":"string","indexType":"filter"},{"fieldName":"spaceId","fieldType":"string","indexType":"filter"}]);
                    self.request("POST", "/collection/create", Some(body))?;
                }
                Provider::Pinecone => unreachable!(),
            }
        }
        let dimension = match self.config.provider {
            Provider::Qdrant => {
                let info =
                    self.request("GET", &format!("/collections/{}", self.collection), None)?;
                let vectors = &info["result"]["config"]["params"]["vectors"];
                if vectors["distance"] != "Cosine" {
                    return Err(self.error("Collection metric must be cosine"));
                }
                vectors["size"].as_u64()
            }
            Provider::Dashvector => {
                let info =
                    self.request("GET", &format!("/v1/collections/{}", self.collection), None)?;
                if info["output"]["metric"] != "cosine" {
                    return Err(self.error("Collection metric must be cosine"));
                }
                info["output"]["dimension"].as_u64()
            }
            Provider::Milvus => {
                let info = self.request(
                    "POST",
                    "/v2/vectordb/collections/describe",
                    Some(self.base()),
                )?;
                let data = &info["data"];
                if data["autoId"]
                    .as_bool()
                    .or_else(|| data["autoID"].as_bool())
                    != Some(false)
                {
                    return Err(self.error("Collection must use explicit string IDs"));
                }
                let metric = data["indexes"]
                    .as_array()
                    .and_then(|indexes| indexes.iter().find(|v| v["fieldName"] == "vector"))
                    .and_then(|v| v["metricType"].as_str());
                if metric != Some("COSINE") {
                    return Err(self.error("Collection metric must be COSINE"));
                }
                data["fields"]
                    .as_array()
                    .and_then(|fields| fields.iter().find(|v| v["name"] == "vector"))
                    .and_then(|field| field["params"].as_array())
                    .and_then(|params| params.iter().find(|v| v["key"] == "dim"))
                    .and_then(|v| {
                        v["value"]
                            .as_str()
                            .and_then(|s| s.parse().ok())
                            .or_else(|| v["value"].as_u64())
                    })
            }
            Provider::Tencent => {
                let info =
                    self.request("POST", "/collection/describe", Some(self.tencent_base()))?;
                if info["collection"]["embedding"]["status"] == "enabled" {
                    return Err(self
                        .error("Use a vector-only Tencent collection without managed embedding"));
                }
                let index = info["collection"]["indexes"]
                    .as_array()
                    .and_then(|v| v.iter().find(|v| v["fieldName"] == "vector"))
                    .ok_or_else(|| self.error("Missing vector index"))?;
                if index["metricType"] != "COSINE" {
                    return Err(self.error("Collection metric must be COSINE"));
                }
                index["dimension"].as_u64()
            }
            Provider::Pinecone => unreachable!(),
        };
        if dimension != Some(self.dimensions as u64) {
            return Err(
                self.error("Collection dimension does not match the selected embedding space")
            );
        }
        if self.config.provider == Provider::Qdrant {
            for field in ["sourceId", "spaceId"] {
                self.request(
                    "PUT",
                    &format!("/collections/{}/index?wait=true", self.collection),
                    Some(json!({"field_name":field,"field_schema":"keyword"})),
                )?;
            }
        }
        if self.config.provider == Provider::Milvus {
            self.request("POST", "/v2/vectordb/collections/load", Some(self.base()))?;
        }
        Ok(())
    }
    pub fn upsert(&self, records: &[VectorRecord]) -> Result<(), CoreError> {
        if records
            .iter()
            .any(|r| r.vector.len() != self.dimensions || r.vector.iter().any(|v| !v.is_finite()))
        {
            return Err(self.error("Invalid vector dimensions or values"));
        }
        let points=records.iter().map(|r| {
            let metadata=json!({"chunkId":r.chunk_id,"sourceId":r.source_id,"spaceId":self.space,"version":r.version()});
            match self.config.provider {
                Provider::Qdrant=>json!({"id":self.point_id(&r.chunk_id),"vector":r.vector,"payload":metadata}),
                Provider::Pinecone=>json!({"id":self.point_id(&r.chunk_id),"values":r.vector,"metadata":metadata}),
                Provider::Dashvector=>json!({"id":self.point_id(&r.chunk_id),"vector":r.vector,"fields":metadata}),
                _=>{let mut point=metadata;point["id"]=json!(self.point_id(&r.chunk_id));point["vector"]=json!(r.vector);point}
            }
        }).collect::<Vec<_>>();
        let result = match self.config.provider {
            Provider::Qdrant => self.request(
                "PUT",
                &format!("/collections/{}/points?wait=true", self.collection),
                Some(json!({"points":points})),
            )?,
            Provider::Pinecone => self.request(
                "POST",
                "/vectors/upsert",
                Some(json!({"namespace":self.collection,"vectors":points})),
            )?,
            Provider::Dashvector => self.request(
                "POST",
                &format!("/v1/collections/{}/docs/upsert", self.collection),
                Some(json!({"docs":points})),
            )?,
            Provider::Milvus => {
                let mut body = self.base();
                body["data"] = json!(points);
                self.request("POST", "/v2/vectordb/entities/upsert", Some(body))?
            }
            Provider::Tencent => {
                let mut body = self.tencent_base();
                body["documents"] = json!(points);
                self.request("POST", "/document/upsert", Some(body))?
            }
        };
        match self.config.provider {
            Provider::Qdrant if result["result"]["status"] != "completed" => {
                return Err(self.error("Upsert was not completed"))
            }
            Provider::Pinecone
                if result["upsertedCount"].as_u64() != Some(records.len() as u64) =>
            {
                return Err(self.error("Partial upsert"))
            }
            Provider::Milvus
                if result["data"]["upsertCount"].as_u64() != Some(records.len() as u64) =>
            {
                return Err(self.error("Partial upsert"))
            }
            Provider::Tencent if result["affectedCount"].as_u64() != Some(records.len() as u64) => {
                return Err(self.error("Partial upsert"))
            }
            Provider::Dashvector => {
                let output = result["output"]
                    .as_array()
                    .ok_or_else(|| self.error("Missing per-document upsert receipts"))?;
                if output.len() != records.len()
                    || output.iter().any(|v| v["code"].as_i64() != Some(0))
                {
                    return Err(self.error("Partial upsert"));
                }
            }
            _ => {}
        }
        Ok(())
    }
    pub fn delete(&self, chunks: &[String]) -> Result<(), CoreError> {
        let ids = chunks
            .iter()
            .map(|id| self.point_id(id))
            .collect::<Vec<_>>();
        let result = match self.config.provider {
            Provider::Qdrant => self.request(
                "POST",
                &format!("/collections/{}/points/delete?wait=true", self.collection),
                Some(json!({"points":ids})),
            )?,
            Provider::Pinecone => self.request(
                "POST",
                "/vectors/delete",
                Some(json!({"namespace":self.collection,"ids":ids})),
            )?,
            Provider::Dashvector => self.request(
                "DELETE",
                &format!("/v1/collections/{}/docs", self.collection),
                Some(json!({"ids":ids})),
            )?,
            Provider::Milvus => {
                let mut body = self.base();
                body["filter"] = json!("id in {ids}");
                body["exprParams"] = json!({"ids":ids});
                self.request("POST", "/v2/vectordb/entities/delete", Some(body))?
            }
            Provider::Tencent => {
                let mut body = self.tencent_base();
                body["query"] = json!({"documentIds":ids});
                self.request("POST", "/document/delete", Some(body))?
            }
        };
        if self.config.provider == Provider::Qdrant && result["result"]["status"] != "completed" {
            return Err(self.error("Delete was not completed"));
        }
        if self.config.provider == Provider::Dashvector
            && result["output"]
                .as_array()
                .is_some_and(|rows| rows.iter().any(|v| v["code"].as_i64() != Some(0)))
        {
            return Err(self.error("Partial delete"));
        }
        Ok(())
    }
    pub fn query(
        &self,
        vector: &[f32],
        sources: &[String],
        limit: usize,
    ) -> Result<Vec<CloudHit>, CoreError> {
        if sources.is_empty() {
            return Ok(vec![]);
        }
        if sources.len() > 256 || sources.iter().any(|id| uuid::Uuid::parse_str(id).is_err()) {
            return Err(self.error("Source scope exceeds cloud filter bounds; use local retrieval"));
        }
        let fields = json!(["chunkId", "sourceId", "spaceId", "version"]);
        let primary_fields = json!(["id", "chunkId", "sourceId", "spaceId", "version"]);
        let result=match self.config.provider {
            Provider::Qdrant=>self.request("POST",&format!("/collections/{}/points/query",self.collection),Some(json!({"query":vector,"limit":limit,"with_payload":true,"with_vector":false,"filter":{"must":[{"key":"spaceId","match":{"value":self.space}},{"key":"sourceId","match":{"any":sources}}]}})))?,
            Provider::Pinecone=>self.request("POST","/query",Some(json!({"namespace":self.collection,"vector":vector,"topK":limit,"includeMetadata":true,"includeValues":false,"filter":{"spaceId":{"$eq":self.space},"sourceId":{"$in":sources}}})))?,
            Provider::Dashvector=>{let filter=format!("spaceId = '{}' and ({})",self.space,sources.iter().map(|id|format!("sourceId = '{id}'")).collect::<Vec<_>>().join(" or "));self.request("POST",&format!("/v1/collections/{}/query",self.collection),Some(json!({"vector":vector,"topk":limit,"include_vector":false,"filter":filter,"output_fields":fields})))?}
            Provider::Milvus=>{let mut body=self.base();body["data"]=json!([vector]);body["annsField"]=json!("vector");body["limit"]=json!(limit);body["filter"]=json!("spaceId == {space} and sourceId in {sources}");body["exprParams"]=json!({"space":self.space,"sources":sources});body["outputFields"]=primary_fields.clone();body["searchParams"]=json!({"metricType":"COSINE"});self.request("POST","/v2/vectordb/entities/search",Some(body))?}
            Provider::Tencent=>{let mut body=self.tencent_base();body["search"]=json!({"vectors":[vector],"limit":limit,"params":{"ef":200},"retrieveVector":false,"outputFields":primary_fields,"filter":format!("spaceId = \"{}\" and sourceId in ({})",self.space,sources.iter().map(|id|format!("\"{id}\"")).collect::<Vec<_>>().join(","))});self.request("POST","/document/search",Some(body))?}
        };
        let rows = match self.config.provider {
            Provider::Qdrant => &result["result"]["points"],
            Provider::Pinecone => &result["matches"],
            Provider::Dashvector => &result["output"],
            Provider::Milvus => &result["data"],
            Provider::Tencent => &result["documents"][0],
        };
        let rows = rows
            .as_array()
            .ok_or_else(|| self.error("Missing vector search results"))?;
        let mut hits = Vec::new();
        for row in rows.iter().take(limit) {
            let metadata = match self.config.provider {
                Provider::Qdrant => &row["payload"],
                Provider::Pinecone => &row["metadata"],
                Provider::Dashvector => &row["fields"],
                Provider::Milvus => row.get("entity").unwrap_or(row),
                Provider::Tencent => row,
            };
            let chunk = metadata["chunkId"].as_str().unwrap_or_default();
            let source = metadata["sourceId"].as_str().unwrap_or_default();
            if metadata["spaceId"] != self.space
                || uuid::Uuid::parse_str(chunk).is_err()
                || !sources.iter().any(|id| id == source)
            {
                continue;
            }
            if row["id"].as_str() != Some(self.point_id(chunk).as_str()) {
                continue;
            }
            if let Some(version) = metadata["version"].as_str() {
                hits.push(CloudHit {
                    chunk_id: chunk.into(),
                    source_id: source.into(),
                    version: version.into(),
                });
            }
        }
        Ok(hits)
    }
}
