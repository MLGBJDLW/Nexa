# Local storage and project contracts

Open **Settings → Models & Embedding → Storage locations** to inspect the
global declaration home, application state directory and managed model folders.
The embedding provider selector appears above storage and downloads; **Sources →
Embedding Configuration** opens it directly. Online APIs do not need local model
downloads. See [embedding providers](EMBEDDING_PROVIDERS.md).

| Ownership | Location and contents |
| --- | --- |
| User declarations | `NEXA_HOME`, default `~/.nexa`: `skills`, `themes`, `workflows`, `capabilities`, `connectors/mcp.json` |
| Managed models | `NEXA_HOME/models`: local embedding model directories, `paddleocr`, `whisper`; an explicit model root or individual path remains supported |
| Project contracts | Each selected workspace: `.nexa/AGENTS.md`, `.nexa/tools`, `.nexa/tmp`; ordinary `AGENTS.override.md` and `AGENTS.md` retain precedence |
| Internal application state | The operating-system application directory: database, credential references, caches, logs and managed runtimes |

`NEXA_HOME` must be absolute and is read at startup. Project contracts are not
globalized or copied into the user home. Child rules keep directory scope, and
external agents retain their own native discovery. See [workspace rules](WORKSPACE_RULES.md).

## Existing downloads

Fresh downloads use the shared model home. Populated legacy model folders remain
readable, including the former Roaming embedding/OCR and Local Whisper locations
on Windows. A custom model path continues to win. Changing the default does not
silently invalidate a working installation or download a second model.

**Consolidate and use** copies existing configured embedding, OCR and Whisper
files (and other managed embedding variants) to the selected root. It verifies
file hashes before committing all model paths together in one SQLite transaction.
Unchanged destination files are reused. Different destination files, unfinished
downloads, links/reparse points, nested source/destination folders and settings
changed during copying stop the operation without switching paths. Originals are
retained. A failed copy or settings commit can leave verified copies at the
destination; the originals and previous configuration remain usable.

Save pending settings and finish active downloads before consolidating. After a
successful switch, download, readiness and delete controls use the configured
folders. The operation does not migrate the database, secrets or project rules,
and does not remove the old files automatically.

Implementation: [path resolution and verified consolidation](../crates/core/src/local_storage.rs),
[desktop command](../apps/desktop/src-tauri/src/commands/media.rs),
[settings](../apps/desktop/src/components/settings/LocalStorageSection.tsx).
