/// TF-IDF embedding module for local vector search.
///
/// Provides a trait-based pluggable embedder design with a concrete
/// TF-IDF implementation that requires no external services.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::db::Database;
use crate::error::CoreError;

// ── Embedder trait ──────────────────────────────────────────────────

/// Pluggable embedding interface.
pub trait Embedder: Send + Sync {
    /// Human-readable model identifier (e.g. `"tfidf-v1"`).
    fn model_name(&self) -> &str;

    /// Dimensionality of the output vectors.
    fn dimensions(&self) -> usize;

    /// Embed a single text into a dense vector.
    fn embed(&self, text: &str) -> Result<Vec<f32>, CoreError>;

    /// Embed a batch of texts.
    fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, CoreError>;
}

// ── TF-IDF Embedder ─────────────────────────────────────────────────

/// Maximum vocabulary size (and therefore vector dimensionality).
const MAX_DIMENSIONS: usize = 512;

/// Basic English stop words filtered during tokenization.
const STOP_WORDS: &[&str] = &[
    "a", "an", "and", "are", "as", "at", "be", "but", "by", "for", "from",
    "had", "has", "have", "he", "her", "his", "how", "if", "in", "into",
    "is", "it", "its", "me", "my", "no", "nor", "not", "of", "on", "or",
    "our", "out", "own", "she", "so", "than", "that", "the", "their",
    "them", "then", "there", "these", "they", "this", "to", "too", "up",
    "us", "very", "was", "we", "were", "what", "when", "where", "which",
    "who", "whom", "why", "will", "with", "would", "you", "your",
];

/// TF-IDF based embedder that builds a vocabulary from a corpus.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TfIdfEmbedder {
    pub vocabulary: HashMap<String, usize>,
    pub idf: Vec<f32>,
    pub dimensions: usize,
}

impl TfIdfEmbedder {
    /// Build a new embedder from a corpus of documents.
    ///
    /// 1. Tokenizes every document.
    /// 2. Selects the top `MAX_DIMENSIONS` most frequent terms as vocabulary.
    /// 3. Computes IDF for each term.
    pub fn build_from_corpus(documents: &[&str]) -> Self {
        let total_docs = documents.len() as f32;

        // Tokenize all documents.
        let tokenized: Vec<Vec<String>> = documents.iter().map(|d| tokenize(d)).collect();

        // Count total occurrences of each term across all documents.
        let mut global_freq: HashMap<String, usize> = HashMap::new();
        // Count how many documents contain each term (for IDF).
        let mut doc_freq: HashMap<String, usize> = HashMap::new();

        for tokens in &tokenized {
            for token in tokens {
                *global_freq.entry(token.clone()).or_insert(0) += 1;
            }
            // Unique terms in this document.
            let unique: HashSet<&String> = tokens.iter().collect();
            for term in unique {
                *doc_freq.entry(term.clone()).or_insert(0) += 1;
            }
        }

        // Sort by frequency descending, take top MAX_DIMENSIONS.
        let mut freq_list: Vec<(String, usize)> = global_freq.into_iter().collect();
        freq_list.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        freq_list.truncate(MAX_DIMENSIONS);

        let dimensions = freq_list.len();
        let mut vocabulary = HashMap::with_capacity(dimensions);
        let mut idf = Vec::with_capacity(dimensions);

        for (idx, (term, _)) in freq_list.iter().enumerate() {
            vocabulary.insert(term.clone(), idx);
            let df = *doc_freq.get(term).unwrap_or(&0) as f32;
            idf.push((total_docs / (1.0 + df)).ln());
        }

        Self {
            vocabulary,
            idf,
            dimensions,
        }
    }

    /// Reconstruct an embedder from a previously saved vocabulary and IDF.
    pub fn from_vocabulary(vocabulary: HashMap<String, usize>, idf: Vec<f32>) -> Self {
        let dimensions = vocabulary.len();
        Self {
            vocabulary,
            idf,
            dimensions,
        }
    }
}

impl Embedder for TfIdfEmbedder {
    fn model_name(&self) -> &str {
        "tfidf-v1"
    }

    fn dimensions(&self) -> usize {
        self.dimensions
    }

    fn embed(&self, text: &str) -> Result<Vec<f32>, CoreError> {
        if self.dimensions == 0 {
            return Ok(vec![]);
        }

        let tokens = tokenize(text);
        if tokens.is_empty() {
            return Ok(vec![0.0; self.dimensions]);
        }

        // Compute term frequency for this text.
        let mut tf_counts: HashMap<&str, f32> = HashMap::new();
        let total_tokens = tokens.len() as f32;
        for tok in &tokens {
            *tf_counts.entry(tok.as_str()).or_insert(0.0) += 1.0;
        }

        // Build TF-IDF vector.
        let mut vector = vec![0.0f32; self.dimensions];
        for (term, &count) in &tf_counts {
            if let Some(&idx) = self.vocabulary.get(*term) {
                let tf = count / total_tokens;
                vector[idx] = tf * self.idf[idx];
            }
        }

        // L2 normalize.
        l2_normalize(&mut vector);
        Ok(vector)
    }

    fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, CoreError> {
        texts.iter().map(|t| self.embed(t)).collect()
    }
}

// ── Tokenization ────────────────────────────────────────────────────

/// Tokenize text into lowercase terms.
///
/// - Splits on non-alphanumeric boundaries.
/// - Keeps individual CJK characters as tokens.
/// - Removes English tokens shorter than 2 characters.
/// - Filters common English stop words.
fn tokenize(text: &str) -> Vec<String> {
    let stop: HashSet<&str> = STOP_WORDS.iter().copied().collect();
    let lower = text.to_lowercase();
    let mut tokens = Vec::new();
    let mut current = String::new();

    for ch in lower.chars() {
        if is_cjk(ch) {
            // Flush any accumulated ASCII word.
            if !current.is_empty() {
                push_if_valid(&mut tokens, &current, &stop);
                current.clear();
            }
            // Each CJK character is its own token.
            tokens.push(ch.to_string());
        } else if ch.is_alphanumeric() {
            current.push(ch);
        } else {
            // Separator — flush current word.
            if !current.is_empty() {
                push_if_valid(&mut tokens, &current, &stop);
                current.clear();
            }
        }
    }
    // Flush trailing word.
    if !current.is_empty() {
        push_if_valid(&mut tokens, &current, &stop);
    }

    tokens
}

/// Push a token if it passes length and stop-word filters.
fn push_if_valid(tokens: &mut Vec<String>, word: &str, stop: &HashSet<&str>) {
    if word.len() >= 2 && !stop.contains(word) {
        tokens.push(word.to_owned());
    }
}

/// Returns `true` if the character is in the CJK Unified Ideographs block.
fn is_cjk(ch: char) -> bool {
    matches!(ch as u32,
        0x4E00..=0x9FFF   // CJK Unified Ideographs
        | 0x3400..=0x4DBF // Extension A
        | 0xF900..=0xFAFF // Compatibility Ideographs
    )
}

// ── Vector utilities ────────────────────────────────────────────────

/// L2 (Euclidean) normalize a vector in place.
fn l2_normalize(v: &mut [f32]) {
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in v.iter_mut() {
            *x /= norm;
        }
    }
}

/// Cosine similarity between two vectors.
///
/// Returns 0.0 if either vector has zero magnitude.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    debug_assert_eq!(a.len(), b.len(), "vectors must have equal length");
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    dot / (norm_a * norm_b)
}

/// Serialize an f32 vector to a little-endian byte blob (for SQLite BLOB).
pub fn vector_to_blob(vec: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(vec.len() * 4);
    for &v in vec {
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    bytes
}

/// Deserialize a little-endian byte blob back into an f32 vector.
pub fn blob_to_vector(blob: &[u8]) -> Vec<f32> {
    blob.chunks_exact(4)
        .map(|chunk| {
            let arr: [u8; 4] = chunk.try_into().expect("chunk is exactly 4 bytes");
            f32::from_le_bytes(arr)
        })
        .collect()
}

// ── Database operations ─────────────────────────────────────────────

impl Database {
    /// Store (upsert) a vector embedding for a chunk.
    pub fn store_embedding(
        &self,
        chunk_id: &str,
        model: &str,
        vector: &[f32],
    ) -> Result<(), CoreError> {
        let blob = vector_to_blob(vector);
        let id = uuid::Uuid::new_v4().to_string();
        let conn = self.conn();
        conn.execute(
            "INSERT INTO embeddings (id, chunk_id, model, vector, dimensions)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(chunk_id, model) DO UPDATE SET
                vector = excluded.vector,
                dimensions = excluded.dimensions,
                created_at = datetime('now')",
            rusqlite::params![id, chunk_id, model, blob, vector.len() as i64],
        )?;
        Ok(())
    }

    /// Store multiple embeddings in a single transaction for bulk operations.
    ///
    /// Much faster than calling `store_embedding` in a loop, as SQLite
    /// transactions are expensive per-call.
    pub fn batch_store_embeddings(
        &self,
        embeddings: &[(String, String, Vec<f32>)],
    ) -> Result<(), CoreError> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        for (chunk_id, model, vector) in embeddings {
            let blob = vector_to_blob(vector);
            let id = uuid::Uuid::new_v4().to_string();
            tx.execute(
                "INSERT INTO embeddings (id, chunk_id, model, vector, dimensions)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(chunk_id, model) DO UPDATE SET
                    vector = excluded.vector,
                    dimensions = excluded.dimensions,
                    created_at = datetime('now')",
                rusqlite::params![id, chunk_id, model, blob, vector.len() as i64],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Retrieve the embedding vector for a specific chunk + model.
    pub fn get_embedding(
        &self,
        chunk_id: &str,
        model: &str,
    ) -> Result<Option<Vec<f32>>, CoreError> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT vector FROM embeddings WHERE chunk_id = ?1 AND model = ?2",
        )?;
        let mut rows = stmt.query(rusqlite::params![chunk_id, model])?;
        match rows.next()? {
            Some(row) => {
                let blob: Vec<u8> = row.get(0)?;
                Ok(Some(blob_to_vector(&blob)))
            }
            None => Ok(None),
        }
    }

    /// Retrieve all embeddings for a given model as `(chunk_id, vector)` pairs.
    pub fn get_all_embeddings(
        &self,
        model: &str,
    ) -> Result<Vec<(String, Vec<f32>)>, CoreError> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT chunk_id, vector FROM embeddings WHERE model = ?1",
        )?;
        let rows = stmt.query_map(rusqlite::params![model], |row| {
            let chunk_id: String = row.get(0)?;
            let blob: Vec<u8> = row.get(1)?;
            Ok((chunk_id, blob))
        })?;

        let mut results = Vec::new();
        for row in rows {
            let (chunk_id, blob) = row?;
            results.push((chunk_id, blob_to_vector(&blob)));
        }
        Ok(results)
    }

    /// Retrieve embeddings in batches using LIMIT/OFFSET for streaming large datasets.
    ///
    /// Avoids loading all embeddings into memory at once — essential for 1M+ chunks.
    pub fn get_embeddings_batched(
        &self,
        model: &str,
        batch_size: usize,
        offset: usize,
    ) -> Result<Vec<(String, Vec<f32>)>, CoreError> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT chunk_id, vector FROM embeddings WHERE model = ?1
             ORDER BY chunk_id LIMIT ?2 OFFSET ?3",
        )?;
        let rows = stmt.query_map(
            rusqlite::params![model, batch_size as i64, offset as i64],
            |row| {
                let chunk_id: String = row.get(0)?;
                let blob: Vec<u8> = row.get(1)?;
                Ok((chunk_id, blob))
            },
        )?;
        let mut results = Vec::new();
        for row in rows {
            let (chunk_id, blob) = row?;
            results.push((chunk_id, blob_to_vector(&blob)));
        }
        Ok(results)
    }

    /// Delete all embeddings belonging to chunks of a given document.
    pub fn delete_embeddings_for_document(
        &self,
        document_id: &str,
    ) -> Result<(), CoreError> {
        let conn = self.conn();
        conn.execute(
            "DELETE FROM embeddings WHERE chunk_id IN (
                SELECT id FROM chunks WHERE document_id = ?1
            )",
            rusqlite::params![document_id],
        )?;
        Ok(())
    }

    /// Persist the embedder's vocabulary and IDF so it can be restored later.
    ///
    /// Uses a lightweight `model_state` table (auto-created on first call).
    pub fn save_embedder_state(
        &self,
        model: &str,
        vocabulary: &HashMap<String, usize>,
        idf: &[f32],
    ) -> Result<(), CoreError> {
        let conn = self.conn();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS model_state (
                model TEXT PRIMARY KEY,
                vocab_json TEXT NOT NULL,
                idf_json TEXT NOT NULL,
                updated_at TEXT NOT NULL DEFAULT (datetime('now'))
            )",
        )?;
        let vocab_json = serde_json::to_string(vocabulary)?;
        let idf_json = serde_json::to_string(idf)?;
        conn.execute(
            "INSERT INTO model_state (model, vocab_json, idf_json)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(model) DO UPDATE SET
                vocab_json = excluded.vocab_json,
                idf_json = excluded.idf_json,
                updated_at = datetime('now')",
            rusqlite::params![model, vocab_json, idf_json],
        )?;
        Ok(())
    }

    /// Load a previously saved embedder state.
    ///
    /// Returns `None` if the model has never been saved.
    pub fn load_embedder_state(
        &self,
        model: &str,
    ) -> Result<Option<(HashMap<String, usize>, Vec<f32>)>, CoreError> {
        let conn = self.conn();
        // Table might not exist yet.
        let table_exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='model_state')",
            [],
            |row| row.get(0),
        )?;
        if !table_exists {
            return Ok(None);
        }

        let mut stmt = conn.prepare(
            "SELECT vocab_json, idf_json FROM model_state WHERE model = ?1",
        )?;
        let mut rows = stmt.query(rusqlite::params![model])?;
        match rows.next()? {
            Some(row) => {
                let vocab_json: String = row.get(0)?;
                let idf_json: String = row.get(1)?;
                let vocabulary: HashMap<String, usize> = serde_json::from_str(&vocab_json)?;
                let idf: Vec<f32> = serde_json::from_str(&idf_json)?;
                Ok(Some((vocabulary, idf)))
            }
            None => Ok(None),
        }
    }
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── tokenization ────────────────────────────────────────────────

    #[test]
    fn test_tokenize_basic_english() {
        let tokens = tokenize("Hello, World! This is a test.");
        assert!(tokens.contains(&"hello".to_string()));
        assert!(tokens.contains(&"world".to_string()));
        assert!(tokens.contains(&"test".to_string()));
        // Stop words removed.
        assert!(!tokens.contains(&"this".to_string()));
        assert!(!tokens.contains(&"is".to_string()));
        assert!(!tokens.contains(&"a".to_string()));
    }

    #[test]
    fn test_tokenize_removes_short_tokens() {
        let tokens = tokenize("I am a go-to x y z person");
        // "i", "x", "y", "z" are single chars → removed.
        assert!(!tokens.contains(&"i".to_string()));
        assert!(!tokens.contains(&"x".to_string()));
        // "am" is a stop word? No, but it's 2 chars and not a stop word — kept.
        assert!(tokens.contains(&"go".to_string()));
        assert!(tokens.contains(&"person".to_string()));
    }

    #[test]
    fn test_tokenize_cjk_characters() {
        let tokens = tokenize("Hello 你好世界 test");
        assert!(tokens.contains(&"hello".to_string()));
        assert!(tokens.contains(&"你".to_string()));
        assert!(tokens.contains(&"好".to_string()));
        assert!(tokens.contains(&"世".to_string()));
        assert!(tokens.contains(&"界".to_string()));
        assert!(tokens.contains(&"test".to_string()));
    }

    #[test]
    fn test_tokenize_mixed_punctuation() {
        let tokens = tokenize("email@example.com http://foo.bar/baz?q=1");
        // Should split on @ . : / ? =
        assert!(tokens.contains(&"email".to_string()));
        assert!(tokens.contains(&"example".to_string()));
        assert!(tokens.contains(&"com".to_string()));
        assert!(tokens.contains(&"http".to_string()));
        assert!(tokens.contains(&"foo".to_string()));
        assert!(tokens.contains(&"bar".to_string()));
        assert!(tokens.contains(&"baz".to_string()));
    }

    // ── TF-IDF construction ─────────────────────────────────────────

    #[test]
    fn test_build_from_corpus() {
        let docs = vec![
            "the cat sat on the mat",
            "the dog chased the cat",
            "the bird flew over the mat",
        ];
        let embedder = TfIdfEmbedder::build_from_corpus(&docs);
        assert!(embedder.dimensions > 0);
        assert!(embedder.dimensions <= MAX_DIMENSIONS);
        assert_eq!(embedder.vocabulary.len(), embedder.dimensions);
        assert_eq!(embedder.idf.len(), embedder.dimensions);
    }

    #[test]
    fn test_build_from_empty_corpus() {
        let embedder = TfIdfEmbedder::build_from_corpus(&[]);
        assert_eq!(embedder.dimensions, 0);
    }

    #[test]
    fn test_from_vocabulary_roundtrip() {
        let docs = vec!["rust is fast", "python is easy"];
        let original = TfIdfEmbedder::build_from_corpus(&docs);
        let restored =
            TfIdfEmbedder::from_vocabulary(original.vocabulary.clone(), original.idf.clone());
        assert_eq!(original.dimensions, restored.dimensions);

        let v1 = original.embed("rust is great").unwrap();
        let v2 = restored.embed("rust is great").unwrap();
        assert_eq!(v1, v2);
    }

    // ── embed ───────────────────────────────────────────────────────

    #[test]
    fn test_embed_produces_normalized_vector() {
        let docs = vec![
            "machine learning algorithms",
            "deep learning neural networks",
            "natural language processing",
        ];
        let embedder = TfIdfEmbedder::build_from_corpus(&docs);
        let vec = embedder.embed("machine learning").unwrap();

        // Non-zero vector.
        assert!(vec.iter().any(|&v| v != 0.0));

        // L2 norm ≈ 1.0
        let norm: f32 = vec.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!(
            (norm - 1.0).abs() < 1e-5,
            "expected unit vector, got norm={}",
            norm
        );
    }

    #[test]
    fn test_embed_empty_text() {
        let embedder = TfIdfEmbedder::build_from_corpus(&["hello world"]);
        let vec = embedder.embed("").unwrap();
        assert!(vec.iter().all(|&v| v == 0.0));
    }

    #[test]
    fn test_embed_batch() {
        let docs = vec!["alpha beta", "gamma delta"];
        let embedder = TfIdfEmbedder::build_from_corpus(&docs);
        let results = embedder.embed_batch(&["alpha", "gamma"]).unwrap();
        assert_eq!(results.len(), 2);
        for v in &results {
            assert_eq!(v.len(), embedder.dimensions);
        }
    }

    // ── cosine similarity ───────────────────────────────────────────

    #[test]
    fn test_cosine_similarity_identical() {
        let a = vec![1.0, 2.0, 3.0];
        let sim = cosine_similarity(&a, &a);
        assert!((sim - 1.0).abs() < 1e-5);
    }

    #[test]
    fn test_cosine_similarity_orthogonal() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![0.0, 1.0, 0.0];
        let sim = cosine_similarity(&a, &b);
        assert!(sim.abs() < 1e-5);
    }

    #[test]
    fn test_cosine_similarity_opposite() {
        let a = vec![1.0, 2.0, 3.0];
        let b = vec![-1.0, -2.0, -3.0];
        let sim = cosine_similarity(&a, &b);
        assert!((sim + 1.0).abs() < 1e-5);
    }

    #[test]
    fn test_cosine_similarity_zero_vector() {
        let a = vec![1.0, 2.0];
        let zero = vec![0.0, 0.0];
        assert_eq!(cosine_similarity(&a, &zero), 0.0);
        assert_eq!(cosine_similarity(&zero, &a), 0.0);
    }

    #[test]
    fn test_cosine_similar_documents() {
        // Larger corpus so IDF values distinguish topics properly.
        let docs = vec![
            "rust compiler systems performance memory safety",
            "cpp compiler systems performance memory management",
            "python scripting dynamic typing interpreted language",
            "javascript scripting dynamic typing web browser",
            "cooking recipes food kitchen ingredients preparation",
            "baking pastry dessert oven flour sugar",
        ];
        let embedder = TfIdfEmbedder::build_from_corpus(&docs);

        let v_rust = embedder.embed("rust compiler systems performance").unwrap();
        let v_cpp = embedder.embed("cpp compiler systems performance").unwrap();
        let v_cooking = embedder.embed("cooking recipes food kitchen").unwrap();

        let sim_related = cosine_similarity(&v_rust, &v_cpp);
        let sim_unrelated = cosine_similarity(&v_rust, &v_cooking);

        assert!(
            sim_related > sim_unrelated,
            "related topics should be more similar: related={} vs unrelated={}",
            sim_related,
            sim_unrelated
        );
    }

    // ── vector serialization ────────────────────────────────────────

    #[test]
    fn test_vector_blob_roundtrip() {
        let original = vec![1.0f32, -2.5, 3.14, 0.0, f32::MAX, f32::MIN];
        let blob = vector_to_blob(&original);
        assert_eq!(blob.len(), original.len() * 4);
        let restored = blob_to_vector(&blob);
        assert_eq!(original, restored);
    }

    #[test]
    fn test_vector_blob_empty() {
        let empty: Vec<f32> = vec![];
        let blob = vector_to_blob(&empty);
        assert!(blob.is_empty());
        let restored = blob_to_vector(&blob);
        assert!(restored.is_empty());
    }

    // ── DB operations ───────────────────────────────────────────────

    fn setup_db_with_chunk() -> (Database, String, String) {
        let db = Database::open_memory().unwrap();
        let (source_id, doc_id, chunk_id) = {
            let conn = db.conn();
            let source_id = uuid::Uuid::new_v4().to_string();
            conn.execute(
                "INSERT INTO sources (id, kind, root_path) VALUES (?1, 'local_folder', '/tmp/test')",
                rusqlite::params![&source_id],
            )
            .unwrap();

            let doc_id = uuid::Uuid::new_v4().to_string();
            conn.execute(
                "INSERT INTO documents (id, source_id, path, title, mime_type, file_size, modified_at, content_hash)
                 VALUES (?1, ?2, '/tmp/test.md', 'Test', 'text/plain', 100, datetime('now'), 'hash')",
                rusqlite::params![&doc_id, &source_id],
            )
            .unwrap();

            let chunk_id = uuid::Uuid::new_v4().to_string();
            conn.execute(
                "INSERT INTO chunks (id, document_id, chunk_index, kind, content, start_offset, end_offset, line_start, line_end, content_hash)
                 VALUES (?1, ?2, 0, 'text', 'hello world', 0, 11, 1, 1, 'chash')",
                rusqlite::params![&chunk_id, &doc_id],
            )
            .unwrap();

            (source_id, doc_id, chunk_id)
        };
        let _ = source_id;
        (db, doc_id, chunk_id)
    }

    #[test]
    fn test_store_and_get_embedding() {
        let (db, _doc_id, chunk_id) = setup_db_with_chunk();
        let vector = vec![0.1, 0.2, 0.3, 0.4];

        db.store_embedding(&chunk_id, "tfidf-v1", &vector).unwrap();
        let result = db.get_embedding(&chunk_id, "tfidf-v1").unwrap();

        assert!(result.is_some());
        assert_eq!(result.unwrap(), vector);
    }

    #[test]
    fn test_get_embedding_not_found() {
        let (db, _doc_id, _chunk_id) = setup_db_with_chunk();
        let result = db.get_embedding("nonexistent", "tfidf-v1").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_store_embedding_upsert() {
        let (db, _doc_id, chunk_id) = setup_db_with_chunk();

        db.store_embedding(&chunk_id, "tfidf-v1", &[1.0, 2.0])
            .unwrap();
        db.store_embedding(&chunk_id, "tfidf-v1", &[3.0, 4.0])
            .unwrap();

        let result = db.get_embedding(&chunk_id, "tfidf-v1").unwrap().unwrap();
        assert_eq!(result, vec![3.0, 4.0]);
    }

    #[test]
    fn test_get_all_embeddings() {
        let (db, _doc_id, chunk_id) = setup_db_with_chunk();
        db.store_embedding(&chunk_id, "tfidf-v1", &[1.0, 2.0])
            .unwrap();

        let all = db.get_all_embeddings("tfidf-v1").unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].0, chunk_id);
        assert_eq!(all[0].1, vec![1.0, 2.0]);
    }

    #[test]
    fn test_delete_embeddings_for_document() {
        let (db, doc_id, chunk_id) = setup_db_with_chunk();
        db.store_embedding(&chunk_id, "tfidf-v1", &[1.0]).unwrap();

        db.delete_embeddings_for_document(&doc_id).unwrap();

        let result = db.get_embedding(&chunk_id, "tfidf-v1").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_save_and_load_embedder_state() {
        let db = Database::open_memory().unwrap();

        let mut vocab = HashMap::new();
        vocab.insert("hello".to_string(), 0);
        vocab.insert("world".to_string(), 1);
        let idf = vec![1.5, 0.8];

        db.save_embedder_state("tfidf-v1", &vocab, &idf).unwrap();
        let loaded = db.load_embedder_state("tfidf-v1").unwrap();

        assert!(loaded.is_some());
        let (loaded_vocab, loaded_idf) = loaded.unwrap();
        assert_eq!(loaded_vocab, vocab);
        assert_eq!(loaded_idf, idf);
    }

    #[test]
    fn test_load_embedder_state_not_found() {
        let db = Database::open_memory().unwrap();
        let result = db.load_embedder_state("nonexistent").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_save_embedder_state_upsert() {
        let db = Database::open_memory().unwrap();

        let mut vocab1 = HashMap::new();
        vocab1.insert("old".to_string(), 0);
        db.save_embedder_state("tfidf-v1", &vocab1, &[1.0]).unwrap();

        let mut vocab2 = HashMap::new();
        vocab2.insert("new".to_string(), 0);
        db.save_embedder_state("tfidf-v1", &vocab2, &[2.0]).unwrap();

        let (loaded_vocab, loaded_idf) = db.load_embedder_state("tfidf-v1").unwrap().unwrap();
        assert_eq!(loaded_vocab, vocab2);
        assert_eq!(loaded_idf, vec![2.0]);
    }

    #[test]
    fn test_batch_store_embeddings() {
        let (db, doc_id, chunk_id) = setup_db_with_chunk();

        // Add a second chunk.
        let chunk_id_2 = {
            let conn = db.conn();
            let id = uuid::Uuid::new_v4().to_string();
            conn.execute(
                "INSERT INTO chunks (id, document_id, chunk_index, kind, content, \
                 start_offset, end_offset, line_start, line_end, content_hash) \
                 VALUES (?1, ?2, 1, 'text', 'second chunk content', 0, 20, 1, 1, 'chash2')",
                rusqlite::params![&id, &doc_id],
            )
            .unwrap();
            id
        };

        let batch = vec![
            (chunk_id.clone(), "tfidf-v1".to_string(), vec![0.1, 0.2, 0.3]),
            (
                chunk_id_2.clone(),
                "tfidf-v1".to_string(),
                vec![0.4, 0.5, 0.6],
            ),
        ];

        db.batch_store_embeddings(&batch).unwrap();

        let v1 = db.get_embedding(&chunk_id, "tfidf-v1").unwrap().unwrap();
        assert_eq!(v1, vec![0.1, 0.2, 0.3]);

        let v2 = db
            .get_embedding(&chunk_id_2, "tfidf-v1")
            .unwrap()
            .unwrap();
        assert_eq!(v2, vec![0.4, 0.5, 0.6]);
    }

    #[test]
    fn test_get_embeddings_batched() {
        let (db, _doc_id, chunk_id) = setup_db_with_chunk();

        db.store_embedding(&chunk_id, "tfidf-v1", &[1.0, 2.0, 3.0])
            .unwrap();

        let batch = db.get_embeddings_batched("tfidf-v1", 10, 0).unwrap();
        assert_eq!(batch.len(), 1);
        assert_eq!(batch[0].0, chunk_id);
        assert_eq!(batch[0].1, vec![1.0, 2.0, 3.0]);

        // Offset past all results returns empty.
        let empty = db.get_embeddings_batched("tfidf-v1", 10, 100).unwrap();
        assert!(empty.is_empty());
    }
}
