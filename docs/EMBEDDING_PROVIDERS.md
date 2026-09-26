# Embedding providers and vector spaces

Use **Settings → Models → Embedding Configuration** to select local ONNX,
API embeddings, or TF-IDF. API presets share one catalog between the UI and
Rust runtime. Select a provider and model, choose supported dimensions, test
the connection, save, then rebuild embeddings. The custom model checkbox accepts
account-specific model IDs without losing the provider's endpoint or adapter.
Local ONNX downloads include Qwen3-Embedding-0.6B, multilingual MiniLM and E5.

## API and local-server choices

| Service | Included models | Adapter and constraints |
| --- | --- | --- |
| OpenAI | text-embedding-3-small, text-embedding-3-large | OpenAI envelope; dimensions can be shortened. [API](https://developers.openai.com/api/docs/guides/embeddings) |
| Alibaba Model Studio | text-embedding-v4, qwen3.7-text-embedding | Qwen dimension choices and 10/20-input limits. Use the URL from your workspace console; Beijing, Singapore and Hong Kong workspace URLs retain Qwen metadata. Existing DashScope URLs are preserved. [API](https://help.aliyun.com/zh/model-studio/text-embedding-synchronous-api) |
| SiliconFlow China and international | Qwen3-Embedding 0.6B/4B/8B; BGE large zh in China | Regional credentials stay separate; hosted dimension choices govern requests. The 4B preset uses explicit 1024 dimensions because the hosted API does not list native 2560. [API](https://docs.siliconflow.com/en/api-reference/embeddings/create-embeddings), [China BGE example](https://docs.siliconflow.cn/docs/usercases/use-siliconcloud-in-DB-GPT) |
| Zhipu / BigModel | embedding-3, embedding-2 | Up to 64 inputs; Embedding-3 has four dimension choices. [API](https://docs.bigmodel.cn/api-reference/模型-api/文本嵌入) |
| Baidu Qianfan | embedding-v1 | v2 endpoint, fixed 384 dimensions, batches of at most 16. [Official SDK](https://github.com/baidubce/bce-qianfan-sdk/blob/6cacf6002140a3249c1c8d557def462c29c129c5/python/qianfan/consts.py), [model dimensions](https://ai.baidu.com/ai-doc/WENXINWORKSHOP/Ultiovtgu) |
| Jina | v5 text small/nano, v3 | Retrieval query/passage tasks; supported dimension truncation. [API](https://jina.ai/embeddings/) |
| Mistral | mistral-embed, codestral-embed | Text embeddings use fixed 1024 dimensions; Codestral uses `output_dimension`. [Text](https://docs.mistral.ai/studio/knowledge-rag/embeddings/text_embeddings), [code](https://docs.mistral.ai/studio/knowledge-rag/embeddings/code_embeddings) |
| Voyage | voyage-4, large, lite, voyage-code-4 | Query/document input types and `output_dimension`. [API](https://docs.voyageai.com/docs/embeddings) |
| Cohere | embed-v4.0, multilingual/English v3 | Native `/v2/embed`, search query/document input types, float results. [API](https://docs.cohere.com/reference/embed) |
| Google Gemini | gemini-embedding-2, gemini-embedding-001 | Native batch requests with one vector per input. Gemini 2 uses retrieval text instructions; 001 uses task types. Each request uses the supported top-level `outputDimensionality` / `taskType` fields for compatibility with deployments predating `embedContentConfig`. [Guide](https://ai.google.dev/gemini-api/docs/embeddings), [request schema](https://ai.google.dev/api/embeddings) |
| OpenRouter | OpenAI embedding 3 small/large, Qwen3-Embedding-8B | Account/upstream availability applies. [API](https://openrouter.ai/docs/guides/overview/multimodal/embeddings) |
| Ollama | Qwen3-Embedding 0.6B/4B/8B, nomic-embed-text, mxbai-embed-large | Pull the selected model first. Loopback needs no key. [Compatibility](https://docs.ollama.com/api/openai-compatibility) |
| LM Studio / custom | User-entered installed or hosted model ID | OpenAI-compatible `/embeddings`; set the actual output dimensions. [LM Studio API](https://lmstudio.ai/docs/developer/openai-compat/embeddings) |

A preset describes a documented contract; it does not prove that an account has
access. Connection testing checks both query and document calls and reports the
provider's error. Tests cover the adapters using local HTTP servers; live paid
provider calls require credentials and are not part of automated verification.
No model is downloaded or installed just by selecting a preset.

## Storage, switching and upgrades

Vectors stay in Nexa's local SQLite database. Selecting a cloud embedding service
changes where text becomes vectors, not where the resulting index is stored.
An independently configured [optional cloud mirror](VECTOR_STORES.md) supports
several stores and local/cloud fusion. Nexa does not require Qdrant or silently
reduce dimensions/quantize vectors.

API indexes use a versioned identity of endpoint, model, dimensions and adapter
preprocessing. Rotating the key does not change the identity. Query and indexing
use the same identity; switching between equal-dimensional models cannot reuse
the other model's vectors. Old spaces remain stored until their source is removed.

**Rebuild existing API embeddings once after upgrading from model-name-only
indexes.** Their endpoint and preprocessing cannot be verified from the old rows,
so the new runtime does not mix them into a verified vector space. Keyword search
remains available and search skips the embedding API when the selected space
has no vectors. Settings shows indexed/total chunks for the selected space and
an actionable rebuild status, refreshed after rebuilding. Changing the API
endpoint, model or dimensions also requires
building the selected space. Local ONNX/TF-IDF identities are unchanged.

Roo Code is a useful reference for provider choice and indexing state, but its
Qdrant collection dimension check alone does not establish model compatibility.
Nexa separates the display model name from the vector-space identity.
[Roo indexing guide](https://roocodeinc.github.io/Roo-Code/features/codebase-indexing/),
[Roo vector store](https://github.com/RooCodeInc/Roo-Code/blob/b867ec9145750d0ae1ff7f02d35406e9bf2a0b16/src/services/code-index/vector-store/qdrant-client.ts)

## Runtime checks

The API boundary rejects missing/duplicate indices, unexpected vector counts,
wrong dimensions and non-finite values before storage. Batches respect provider
limits. Retries apply to transport failures, 429 and server errors, not malformed
successful results. Response bodies have a 16 MiB bound. Empty keys are accepted
only for loopback hosts; remote endpoints require credentials.

Implementation: [shared catalog](../shared/embedding-provider-presets.json),
[API adapters and HTTP tests](../crates/core/src/embed/api.rs),
[embedding jobs](../crates/core/src/embedding_job.rs),
[search](../crates/core/src/search.rs),
[settings](../apps/desktop/src/components/settings/EmbeddingConfigSection.tsx),
[browser tests](../apps/desktop/e2e/embedding-settings.spec.ts).
