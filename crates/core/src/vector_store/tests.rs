use super::adapter::RemoteStore;
use super::*;
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    net::TcpListener,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};
static TEST_LOCK: Mutex<()> = Mutex::new(());

#[derive(Default)]
struct RemoteState {
    collection: Option<String>,
    points: BTreeMap<String, Value>,
    requests: Vec<(String, Value)>,
    fail_upsert: bool,
    stale: bool,
}
struct MockStore {
    url: String,
    state: Arc<Mutex<RemoteState>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl MockStore {
    fn new(provider: VectorStoreProvider) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let state = Arc::new(Mutex::new(RemoteState::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let (server_state, server_stop) = (state.clone(), stop.clone());
        let thread = std::thread::spawn(move || {
            while !server_stop.load(Ordering::SeqCst) {
                let Ok((mut socket, _)) = listener.accept() else {
                    std::thread::sleep(std::time::Duration::from_millis(2));
                    continue;
                };
                socket.set_nonblocking(false).unwrap();
                socket
                    .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                    .unwrap();
                let mut received = Vec::new();
                let (headers, body) = loop {
                    let mut buffer = [0; 8192];
                    let read = socket.read(&mut buffer).unwrap();
                    assert!(read > 0);
                    received.extend_from_slice(&buffer[..read]);
                    if let Some(end) = received.windows(4).position(|part| part == b"\r\n\r\n") {
                        let headers = String::from_utf8(received[..end].to_vec()).unwrap();
                        let length: usize = headers
                            .lines()
                            .find_map(|line| {
                                line.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|v| v.trim().parse().unwrap())
                            })
                            .unwrap_or(0);
                        if received.len() >= end + 4 + length {
                            let body = if length == 0 {
                                Value::Null
                            } else {
                                serde_json::from_slice(&received[end + 4..end + 4 + length])
                                    .unwrap()
                            };
                            break (headers, body);
                        }
                    }
                };
                let mut state = server_state.lock().unwrap();
                state.requests.push((headers.clone(), body.clone()));
                let path = headers
                    .lines()
                    .next()
                    .unwrap()
                    .split_whitespace()
                    .nth(1)
                    .unwrap();
                let response = reply(provider, path, &headers, &body, &mut state).to_string();
                write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",response.len()).unwrap();
            }
        });
        Self {
            url,
            state,
            stop,
            thread: Some(thread),
        }
    }
}
impl Drop for MockStore {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.thread.take().unwrap().join().unwrap();
    }
}

fn reply(
    provider: VectorStoreProvider,
    path: &str,
    headers: &str,
    body: &Value,
    state: &mut RemoteState,
) -> Value {
    use VectorStoreProvider::*;
    let key = match provider {
        Qdrant => "api-key: vector-secret",
        Pinecone => "api-key: vector-secret",
        Dashvector => "dashvector-auth-token: vector-secret",
        Milvus => "authorization: bearer vector-secret",
        Tencent => "authorization: bearer account=root&api_key=vector-secret",
    };
    assert!(headers.to_ascii_lowercase().contains(key));
    if provider == Pinecone {
        assert!(headers
            .to_ascii_lowercase()
            .contains("x-pinecone-api-version: 2026-07"));
    }
    let list = state.collection.iter().cloned().collect::<Vec<_>>();
    match (provider, path) {
        (Qdrant, "/collections") => {
            return json!({"status":"ok","result":{"collections":list.iter().map(|name|json!({"name":name})).collect::<Vec<_>>()}})
        }
        (Pinecone, "/describe_index_stats") => return json!({"dimension":2,"namespaces":{}}),
        (Dashvector, "/v1/collections") if headers.starts_with("GET ") => {
            return json!({"code":0,"output":list})
        }
        (Milvus, "/v2/vectordb/collections/list") => return json!({"code":0,"data":list}),
        (Tencent, "/collection/list") => {
            return json!({"code":0,"collections":list.iter().map(|name|json!({"collection":name})).collect::<Vec<_>>()})
        }
        _ => {}
    }
    let create = match provider {
        Qdrant => headers.starts_with("PUT ") && body.get("vectors").is_some(),
        Dashvector => path == "/v1/collections",
        Milvus => path.ends_with("/collections/create"),
        Tencent => path == "/collection/create",
        Pinecone => false,
    };
    if create {
        let name = match provider {
            Qdrant => path.trim_start_matches("/collections/"),
            Dashvector => body["name"].as_str().unwrap(),
            Milvus => body["collectionName"].as_str().unwrap(),
            Tencent => body["collection"].as_str().unwrap(),
            _ => unreachable!(),
        };
        state.collection = Some(name.into());
        return if provider == Qdrant {
            json!({"status":"ok","result":true})
        } else {
            json!({"code":0,"output":{},"data":{}})
        };
    }
    if provider == Qdrant && path.ends_with("index?wait=true") {
        return json!({"status":"ok","result":{"status":"completed"}});
    }
    if provider == Qdrant && headers.starts_with("GET ") {
        return json!({"status":"ok","result":{"config":{"params":{"vectors":{"size":2,"distance":"Cosine"}}}}});
    }
    if provider == Dashvector && headers.starts_with("GET ") {
        return json!({"code":0,"output":{"dimension":2,"metric":"cosine"}});
    }
    if provider == Milvus && path.ends_with("/collections/describe") {
        return json!({"code":0,"data":{"autoId":false,"fields":[{"name":"vector","params":[{"key":"dim","value":"2"}]}],"indexes":[{"fieldName":"vector","metricType":"COSINE"}],"load":"LoadStateLoaded"}});
    }
    if provider == Milvus && path.ends_with("/collections/load") {
        return json!({"code":0,"data":{}});
    }
    if provider == Tencent && path == "/collection/describe" {
        return json!({"code":0,"collection":{"indexes":[{"fieldName":"vector","metricType":"COSINE","dimension":2}]}});
    }
    let upsert = path.contains("upsert")
        || (provider == Qdrant && headers.starts_with("PUT ") && path.contains("/points?"));
    if upsert {
        let rows = match provider {
            Qdrant => &body["points"],
            Pinecone => &body["vectors"],
            Dashvector => &body["docs"],
            Milvus => &body["data"],
            Tencent => &body["documents"],
        }
        .as_array()
        .unwrap();
        for point in rows {
            let metadata = match provider {
                Qdrant => point["payload"].clone(),
                Pinecone => point["metadata"].clone(),
                Dashvector => point["fields"].clone(),
                _ => point.clone(),
            };
            assert!(metadata.get("content").is_none());
            assert!(metadata.get("text").is_none());
            assert!(metadata["version"].as_str().is_some());
            state
                .points
                .insert(point["id"].as_str().unwrap().into(), metadata);
        }
        if state.fail_upsert {
            return match provider {
                Qdrant => json!({"status":{"error":"vector-secret simulated failure"}}),
                Pinecone => json!({"upsertedCount":0}),
                _ => json!({"code":99,"message":"vector-secret simulated failure"}),
            };
        }
        return match provider {
            Qdrant => json!({"status":"ok","result":{"status":"completed"}}),
            Pinecone => json!({"upsertedCount":rows.len()}),
            Dashvector => {
                json!({"code":0,"output":rows.iter().map(|p|json!({"id":p["id"],"code":0})).collect::<Vec<_>>()})
            }
            Milvus => json!({"code":0,"data":{"upsertCount":rows.len()}}),
            Tencent => json!({"code":0,"affectedCount":rows.len()}),
        };
    }
    let delete = path.contains("delete") || headers.starts_with("DELETE ");
    if delete {
        let ids = match provider {
            Qdrant => &body["points"],
            Pinecone | Dashvector => &body["ids"],
            Milvus => &body["exprParams"]["ids"],
            Tencent => &body["query"]["documentIds"],
        }
        .as_array()
        .unwrap();
        for id in ids {
            state.points.remove(id.as_str().unwrap());
        }
        return match provider {
            Qdrant => json!({"status":"ok","result":{"status":"completed"}}),
            Pinecone => json!({}),
            _ => json!({"code":0,"affectedCount":ids.len(),"output":[],"data":{}}),
        };
    }
    // Deliberately return all stored sources to exercise authoritative client
    // filtering even when a remote implementation ignores its filter.
    let rows = state
        .points
        .iter()
        .map(|(id, metadata)| {
            let mut metadata = metadata.clone();
            if state.stale {
                metadata["version"] = json!("stale");
            }
            match provider {
                Qdrant => json!({"id":id,"score":0.9,"payload":metadata}),
                Pinecone => json!({"id":id,"score":0.9,"metadata":metadata}),
                Dashvector => json!({"id":id,"score":0.1,"fields":metadata}),
                _ => {
                    metadata["id"] = json!(id);
                    metadata
                }
            }
        })
        .collect::<Vec<_>>();
    match provider {
        Qdrant => json!({"status":"ok","result":{"points":rows}}),
        Pinecone => json!({"matches":rows}),
        Dashvector => json!({"code":0,"output":rows}),
        Milvus => json!({"code":0,"data":rows}),
        Tencent => json!({"code":0,"documents":[rows]}),
    }
}

fn add_chunk(db: &Database, model: &str) -> (String, String) {
    let (source, document, chunk) = (
        uuid::Uuid::new_v4().to_string(),
        uuid::Uuid::new_v4().to_string(),
        uuid::Uuid::new_v4().to_string(),
    );
    {
        let conn = db.conn();
        conn.execute(
            "INSERT INTO sources(id,kind,root_path) VALUES(?1,'local_folder',?2)",
            rusqlite::params![source, format!("/fixture/{source}")],
        )
        .unwrap();
        conn.execute("INSERT INTO documents(id,source_id,path,title,mime_type,file_size,modified_at,content_hash) VALUES(?1,?2,?3,'Vector fixture','text/plain',10,datetime('now'),?1)",rusqlite::params![document,source,format!("/fixture/{document}.txt")]).unwrap();
        conn.execute("INSERT INTO chunks(id,document_id,chunk_index,kind,content,start_offset,end_offset,line_start,line_end,content_hash) VALUES(?1,?2,0,'text','private document text stays local',0,10,1,1,?1)",rusqlite::params![chunk,document]).unwrap();
    }
    db.store_embedding(&chunk, model, &[1., 0.]).unwrap();
    (source, chunk)
}

#[test]
fn all_cloud_adapters_sync_revision_fences_scope_and_durable_deletes() {
    let _guard = TEST_LOCK.lock().unwrap();
    for provider in [
        VectorStoreProvider::Qdrant,
        VectorStoreProvider::Pinecone,
        VectorStoreProvider::Dashvector,
        VectorStoreProvider::Milvus,
        VectorStoreProvider::Tencent,
    ] {
        resume_sync();
        let server = MockStore::new(provider);
        let db = Database::open_memory().unwrap();
        let embed = crate::embed::EmbedderConfig {
            provider: "api".into(),
            api_base_url: "https://api.openai.com/v1".into(),
            api_model: "text-embedding-3-small".into(),
            vector_dimensions: 2,
            ..Default::default()
        };
        db.save_embedder_config(&embed).unwrap();
        let space = crate::embed::ApiEmbedder::configured_space_id(&embed);
        let (source, chunk) = add_chunk(&db, &space);
        let (forbidden_source, forbidden) = add_chunk(&db, &space);
        let config = VectorStoreConfig {
            provider,
            mode: VectorSearchMode::Hybrid,
            endpoint: server.url.clone(),
            api_key: "vector-secret".into(),
            ..Default::default()
        };
        db.save_vector_store_config(&config).unwrap();
        let report = sync_vectors(&db, 4).unwrap();
        assert_eq!(report.uploaded, 2, "{provider:?}");
        assert_eq!(db.vector_sync_status().unwrap().pending_uploads, 0);
        let reads = server.state.lock().unwrap().requests.len();
        assert_eq!(sync_vectors(&db, 4).unwrap().uploaded, 0);
        assert_eq!(server.state.lock().unwrap().requests.len(), reads);
        let filters = SearchFilters {
            source_ids: vec![uuid::Uuid::parse_str(&source).unwrap()],
            ..Default::default()
        };
        let hits = cloud_candidates(&db, &space, &[1., 0.], &filters, 20).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].0, chunk);
        assert!(!hits.iter().any(|h| h.0 == forbidden));
        server.state.lock().unwrap().stale = true;
        let (hits, mode) = ranked_candidates(&db, &space, &[1., 0.], &filters, 20).unwrap();
        assert_eq!(mode, "hybrid+local-fallback");
        assert_eq!(hits[0].0, chunk);
        server.state.lock().unwrap().stale = false;
        db.store_embedding(&chunk, &space, &[0., 1.]).unwrap();
        assert_eq!(db.vector_sync_status().unwrap().pending_uploads, 1);
        assert!(cloud_candidates(&db, &space, &[1., 0.], &filters, 20)
            .unwrap()
            .is_empty());
        server.state.lock().unwrap().fail_upsert = true;
        let error = sync_vectors(&db, 4).unwrap_err().to_string();
        assert!(!error.contains("vector-secret"));
        assert_eq!(db.vector_sync_status().unwrap().pending_uploads, 1);
        server.state.lock().unwrap().fail_upsert = false;
        assert_eq!(sync_vectors(&db, 4).unwrap().uploaded, 1);
        assert_eq!(server.state.lock().unwrap().points.len(), 2);
        db.delete_source(&source).unwrap();
        assert_eq!(db.vector_sync_status().unwrap().pending_deletes, 1);
        assert_eq!(sync_vectors(&db, 4).unwrap().deleted, 1);
        assert_eq!(server.state.lock().unwrap().points.len(), 1);
        db.save_embedder_config(&crate::embed::EmbedderConfig {
            provider: "tfidf".into(),
            ..Default::default()
        })
        .unwrap();
        db.delete_source(&forbidden_source).unwrap();
        assert_eq!(db.vector_sync_status().unwrap().pending_deletes, 1);
        assert_eq!(sync_vectors(&db, 4).unwrap().deleted, 1);
        assert!(server.state.lock().unwrap().points.is_empty());
        let serialized = db
            .conn()
            .query_row("SELECT config_json FROM vector_store_config", [], |r| {
                r.get::<_, String>(0)
            })
            .unwrap();
        assert!(!serialized.contains("vector-secret"));
        let remote = RemoteStore::new(
            &config,
            &db.vector_store_owner().unwrap(),
            &space,
            2,
            std::time::Duration::from_secs(1),
        )
        .unwrap();
        let log = server.state.lock().unwrap();
        assert!(log
            .requests
            .iter()
            .all(|(_, body)| !body.to_string().contains("private document text")));
        assert!(remote.collection.len() <= 32);
    }
}

#[test]
fn fusion_does_not_double_count_the_same_vector_copy() {
    let one = vec![("a".into(), 0.9), ("b".into(), 0.8)];
    let two = vec![("a".into(), 99.), ("c".into(), 1.)];
    let result = fuse_dense_candidates(&one, &two, 10);
    assert_eq!(result.len(), 3);
    assert_eq!(result[0], ("a".into(), 1. / 60.));
}

#[test]
fn local_mode_never_contacts_cloud_and_failed_cloud_keeps_scoped_results() {
    let _guard = TEST_LOCK.lock().unwrap();
    resume_sync();
    let db = Database::open_memory().unwrap();
    let (source, chunk) = add_chunk(&db, "model");
    assert_eq!(sync_vectors(&db, 4).unwrap().uploaded, 0);
    let filters = SearchFilters {
        source_ids: vec![uuid::Uuid::parse_str(&source).unwrap()],
        ..Default::default()
    };
    let (hits, mode) = ranked_candidates(&db, "model", &[1., 0.], &filters, 10).unwrap();
    assert_eq!(mode, "hybrid");
    assert_eq!(hits[0].0, chunk);
    let config = VectorStoreConfig {
        mode: VectorSearchMode::Cloud,
        endpoint: "http://127.0.0.1:1".into(),
        ..Default::default()
    };
    db.save_vector_store_config(&config).unwrap();
    let (hits, mode) = ranked_candidates(&db, "model", &[1., 0.], &filters, 10).unwrap();
    assert_eq!(mode, "hybrid+local-fallback");
    assert_eq!(hits[0].0, chunk);
}

#[test]
fn cloud_config_and_space_isolation_survive_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("index.sqlite");
    let config = VectorStoreConfig {
        mode: VectorSearchMode::Cloud,
        provider: VectorStoreProvider::Dashvector,
        endpoint: "https://cluster.example".into(),
        api_key: "secret".into(),
        ..Default::default()
    };
    let owner = {
        let db = Database::new(&path).unwrap();
        db.save_vector_store_config(&config).unwrap();
        db.vector_store_owner().unwrap()
    };
    let db = Database::new(&path).unwrap();
    assert_eq!(db.vector_store_owner().unwrap(), owner);
    assert_eq!(db.vector_store_config().unwrap().api_key, "secret");
    let a = RemoteStore::new(
        &config,
        &owner,
        "model-a",
        2,
        std::time::Duration::from_secs(1),
    )
    .unwrap();
    let b = RemoteStore::new(
        &config,
        &owner,
        "model-b",
        2,
        std::time::Duration::from_secs(1),
    )
    .unwrap();
    assert_ne!(a.space, b.space);
    assert_ne!(a.point_id("chunk"), b.point_id("chunk"));
}

#[test]
fn partial_vector_schema_migration_repairs_tables_without_losing_vectors() {
    let db = Database::open_memory().unwrap();
    let (_, chunk) = add_chunk(&db, "model");
    db.conn()
        .execute_batch("DROP TABLE vector_store_receipts; DROP TABLE vector_store_config;")
        .unwrap();
    crate::migrations::run_migrations(&db.conn()).unwrap();
    assert!(db.get_embedding(&chunk, "model").unwrap().is_some());
    assert_eq!(
        db.vector_store_config().unwrap().mode,
        VectorSearchMode::Local
    );
}
