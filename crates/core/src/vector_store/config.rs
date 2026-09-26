use crate::{
    db::Database,
    embed::{ApiEmbedder, EmbedderConfig},
    error::CoreError,
};
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VectorSearchMode {
    #[default]
    Local,
    Cloud,
    Hybrid,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VectorStoreProvider {
    #[default]
    Qdrant,
    Pinecone,
    Dashvector,
    Milvus,
    Tencent,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct VectorStoreConfig {
    pub mode: VectorSearchMode,
    pub provider: VectorStoreProvider,
    pub endpoint: String,
    pub api_key: String,
    pub collection_prefix: String,
    pub database: String,
    pub account: String,
}
impl Default for VectorStoreConfig {
    fn default() -> Self {
        Self {
            mode: VectorSearchMode::Local,
            provider: VectorStoreProvider::Qdrant,
            endpoint: String::new(),
            api_key: String::new(),
            collection_prefix: "nexa".into(),
            database: "default".into(),
            account: "root".into(),
        }
    }
}
impl VectorStoreConfig {
    pub fn validate(&self) -> Result<(), CoreError> {
        if self.mode == VectorSearchMode::Local {
            return Ok(());
        }
        let url = url::Url::parse(self.endpoint.trim())
            .map_err(|_| CoreError::InvalidInput("Invalid vector store endpoint".into()))?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(CoreError::InvalidInput("Vector store endpoint must be an HTTP(S) base URL without credentials, query or fragment".into()));
        }
        let local = url.host_str().is_some_and(|host| {
            host == "localhost"
                || host
                    .trim_matches(['[', ']'])
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback())
        });
        if self.api_key.is_empty()
            && !(local
                && matches!(
                    self.provider,
                    VectorStoreProvider::Qdrant | VectorStoreProvider::Milvus
                ))
        {
            return Err(CoreError::InvalidInput(
                "Vector store API key is required".into(),
            ));
        }
        if self.collection_prefix.is_empty()
            || self.collection_prefix.len() > 12
            || !self
                .collection_prefix
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
            || !self
                .collection_prefix
                .starts_with(|ch: char| ch.is_ascii_alphabetic())
        {
            return Err(CoreError::InvalidInput("Collection prefix must start with a letter and contain at most 12 letters, digits or underscores".into()));
        }
        if matches!(
            self.provider,
            VectorStoreProvider::Milvus | VectorStoreProvider::Tencent
        ) && (self.database.is_empty()
            || self.database.len() > 64
            || !self
                .database
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '_'))
        {
            return Err(CoreError::InvalidInput(
                "An existing database name is required (letters, digits and underscores)".into(),
            ));
        }
        if self.provider == VectorStoreProvider::Tencent
            && (self.account.is_empty() || self.account.contains(['&', '\r', '\n']))
        {
            return Err(CoreError::InvalidInput(
                "Tencent VectorDB account is required".into(),
            ));
        }
        Ok(())
    }
    pub fn store_id(&self) -> String {
        let endpoint = crate::embedding_provider_catalog::normalize_base_url(&self.endpoint);
        blake3::hash(
            serde_json::json!([
                self.provider,
                endpoint,
                self.collection_prefix,
                self.database,
                self.account
            ])
            .to_string()
            .as_bytes(),
        )
        .to_hex()
        .to_string()
    }
}

pub fn embedding_space(config: &EmbedderConfig) -> Result<(String, usize), CoreError> {
    match config.provider.as_str() {
        "api" => {
            let dimensions = if config.vector_dimensions > 0 {
                config.vector_dimensions as usize
            } else {
                crate::embedding_provider_catalog::find_embedding_model(
                    &config.api_base_url,
                    &config.api_model,
                )
                .map(|model| model.dimensions)
                .unwrap_or(1536)
            };
            Ok((ApiEmbedder::configured_space_id(config), dimensions))
        }
        "local" => {
            let model = config.local_embedding_model();
            Ok((model.model_name().into(), model.dimensions()))
        }
        _ => Err(CoreError::InvalidInput(
            "Cloud vector stores require a dense API or local embedding model; TF-IDF stays local"
                .into(),
        )),
    }
}

impl Database {
    pub fn vector_store_config(&self) -> Result<VectorStoreConfig, CoreError> {
        let json: Option<String> = self
            .conn()
            .query_row(
                "SELECT config_json FROM vector_store_config WHERE singleton=1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        let mut config: VectorStoreConfig = json
            .map(|json| serde_json::from_str(&json))
            .transpose()?
            .unwrap_or_default();
        if !config.api_key.is_empty() {
            config.api_key = crate::crypto::decrypt_api_key(&config.api_key)?;
        }
        Ok(config)
    }
    pub fn save_vector_store_config(&self, config: &VectorStoreConfig) -> Result<(), CoreError> {
        config.validate()?;
        let mut stored = config.clone();
        stored.api_key = crate::crypto::encrypt_api_key(&config.api_key)?;
        self.conn().execute("INSERT INTO vector_store_config(singleton,owner_id,config_json) VALUES(1,?1,?2) ON CONFLICT(singleton) DO UPDATE SET config_json=excluded.config_json,last_error=NULL",
            rusqlite::params![uuid::Uuid::new_v4().to_string(),serde_json::to_string(&stored)?])?;
        super::resume_sync();
        super::notify_sync();
        Ok(())
    }
    pub(super) fn vector_store_owner(&self) -> Result<String, CoreError> {
        self.conn()
            .query_row(
                "SELECT owner_id FROM vector_store_config WHERE singleton=1",
                [],
                |row| row.get(0),
            )
            .map_err(Into::into)
    }
}
