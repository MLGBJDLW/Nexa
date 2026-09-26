# Optional cloud vector storage

SQLite remains the default and authoritative store for documents, chunks and
vectors. Cloud storage is an optional mirror; it does not move document text out
of SQLite or turn the remote service into an authority for source access.

Open **Settings → Models & Embedding → Vector storage**. Choose:

- **Local only:** no cloud synchronization or cloud queries.
- **Cloud with local fallback:** use valid cloud candidates; fall back to the
  local index when the cloud fails, times out, or has no current candidates.
- **Local + cloud fusion:** combine local and cloud candidates once per chunk,
  then combine that semantic signal with keyword retrieval.

## Supported adapters

| Provider | Configure | Protocol |
| --- | --- | --- |
| Qdrant / Qdrant Cloud | Instance URL and API key; unauthenticated loopback is supported | Collections/points REST, cosine vectors and keyword payload indexes. [Reference](https://api.qdrant.tech/api-reference/points/upsert-points) |
| Pinecone | Existing dense **cosine** index data host and API key; dimension must match the embedding model | Vectors API `2026-07`, isolated namespace. Index provisioning remains in Pinecone. [Reference](https://docs.pinecone.io/reference/api/2026-07/data-plane/query) |
| Alibaba DashVector | Cluster endpoint and DashVector token, separate from a DashScope embedding key | Collections/docs REST, float vectors, typed metadata and scoped filters. [Reference](https://www.alibabacloud.com/help/en/vrs/latest/retrieve-doc-1) |
| Milvus / Zilliz Cloud | REST v2 cluster endpoint, token, existing database | Explicit string primary keys, dynamic metadata, cosine index and loaded collection. [Reference](https://docs.zilliz.com/reference/restful/search-v2) |
| Tencent VectorDB | Instance endpoint, API key, account and existing database | Native HTTP API with a vector-only collection and indexed scope fields. [Reference](https://cloud.tencent.com/document/product/1709/95123) |

Select and save a dense embedding model first; see
[Embedding providers](EMBEDDING_PROVIDERS.md). The connection test is read-only.
Saving an enabled cloud configuration starts background synchronization. Except
for Pinecone's existing index, Nexa creates its own logical collection on the
first upload, after a successful collection-list response proves it is missing.
It does not create a cloud account, cluster, subscription or paid instance.

Collection names/namespaces combine the configured prefix with a hash of the
local database owner and embedding space. Existing collection dimensions and
metrics are checked before reuse where the data-plane API exposes them.
Pinecone statistics expose dimensions but not metric; configure cosine when
creating its index. Credentials are encrypted at rest, never reused from model
providers, and HTTP redirects are not followed with service-specific key headers.

Weaviate is deliberately not presented as universally compatible: its newer
Cloud deployments disable GraphQL, and its experimental REST search does not
offer the same externally supplied vector contract. A future adapter must
verify that deployment's capabilities. Other services, such as OpenSearch and
PostgreSQL/pgvector deployments, need their own authentication/schema adapters;
their names are not advertised as working integrations.
[Weaviate API constraints](https://docs.weaviate.io/weaviate/api)

## Synchronization and recovery

Only vectors and opaque chunk/source/space IDs plus a revision token are
uploaded. Local indexing commits independently of the network. A single bounded
worker wakes for changes and retries on a 60-second timer; successful large
backlogs continue in bounded batches. It never holds a SQLite connection across
HTTP, and adaptive batch sizes stay below common request-size limits.

Receipts record the exact embedding ID and revision acknowledged by the remote
service. Updates that happen during an upload remain pending. Partial success,
transport failure and process restart cannot mark unconfirmed writes complete;
stable point IDs make replayed writes idempotent. Settings shows local/synced
counts, pending uploads/deletes, pause state and the latest error.

Deleting a source removes it locally immediately. Receipts survive the cascade
as durable deletion work, including older embedding spaces on the currently
configured store. Cleanup resumes when that cloud is enabled and reachable.
Pausing prevents additional batches until Sync now, Save or restart; an already
sent request can finish within its 20-second bound. Returning to Local only or
switching providers does not erase the previous cloud's collections. Re-enable
the previous configuration to process its queue, or manage its resources in the
provider console. This is a mirror, not bidirectional document synchronization.

## Retrieval and fusion

Cloud searches have a 1.5-second budget. Requests include the current namespace,
space and permitted source IDs. Returned IDs are checked against current local
chunks, source filters and embedding revisions before any content is hydrated.
Stale, unknown or out-of-scope cloud hits cannot introduce document content.
File/date filters and unusually large source/result sets use local retrieval.
The result badge distinguishes cloud, fusion and local fallback.

Fusion uses the best rank for duplicate local/cloud copies, not two votes for
the same vector. FTS and the combined semantic candidates then enter the normal
ranking path. Provider ANN recall can differ from exact local scans; choosing
Cloud/Fusion is optional and does not silently change the default local ranking.

The local vector path also applies source, MIME and date constraints before
top-k ranking. Final hydration checks filters again after graph expansion.
Unfiltered local scans keep the original batched fast path.

## Implementation and validation

- [Store interface and retrieval](../crates/core/src/vector_store/mod.rs)
- [Provider adapters](../crates/core/src/vector_store/adapter.rs)
- [Durable synchronization](../crates/core/src/vector_store/sync.rs)
- [Restart-safe schema](../crates/core/src/migrations/v133_vector_stores.sql)
- [HTTP, scope, revision and deletion tests](../crates/core/src/vector_store/tests.rs)
- [Settings regression tests](../apps/desktop/e2e/vector-store.spec.ts)

Tests use local HTTP fixtures matching the documented provider contracts. No
paid instances or live cloud credentials are required or used by these tests.
