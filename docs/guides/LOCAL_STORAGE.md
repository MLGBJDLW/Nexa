# Local storage and project contracts

[Documentation](../README.md) / [Category overview](README.md)

Open **Settings → Data & Privacy → Storage locations** to inspect the
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

## Database storage

Settings → Data & Privacy breaks down storage used by conversations, knowledge,
local models, managed files, other database space and journal overhead. Stripes
mark the part of the database allocation SQLite can reuse for future writes.
Freed pages remain in the total while the file occupies disk space. New data
pages that still reside in the write-ahead log count toward their owning category
once. Use **Manage conversations** to inspect and act on selected conversations.

Storage scans use a separate read-only database connection and a blocking worker,
so opening this panel does not queue conversation or model reads behind a disk
scan. Desktop and phone reuse the same completed snapshot for 30 seconds and
share one pending refresh. A storage mutation invalidates that snapshot; if a
scan is already running, the panel performs one follow-up refresh. The previous
result stays visible during refreshes and recoverable errors.

Repeated knowledge-health scans update a stable finding identity instead of
inserting duplicates. The migration retains resolved state and one canonical
finding. Stream deltas are coalesced within durable commits; original live token
delivery remains unchanged. These changes reduce future row amplification, not
the size of every existing database immediately.

## Compacting an existing desktop database

In Settings → Data & Privacy, the reusable database space is already free for future inserts. To estimate the disk space a compacted copy would occupy, use SQLite's `VACUUM INTO` on the Nexa database. Choose a new file in a private directory with enough free space. For a replacement candidate, fully quit Nexa, including the tray, before making the copy and keep it closed during verification.

```sql
PRAGMA page_size;
PRAGMA freelist_count;
VACUUM INTO 'nexa-compact.db';
```

Multiply the first two values to obtain the bytes held in free pages. The copy contains the database's committed records, including archived chats and run history. Opening the source and writing to a different file leaves the original available for comparison. SQLite sidecar files belong to their own database. Use an integrity check and verify message and search results in the copy before using it for recovery. The application does not rebuild a large database at startup.

New file checkpoints use the existing lossless snapshot frame when it saves at least one eighth of the original size. File restore verifies the frame's length and digest before writing bytes back. Previously stored checkpoints remain readable.

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
An unfinished-consolidation marker keeps these copies out of automatic model
resolution until every copy and the settings transaction succeed. This also
applies when a destination is one of the legacy model roots.

Save pending settings and finish active downloads before consolidating. After a
successful switch, download, readiness and delete controls use the configured
folders. The operation does not migrate the database, secrets or project rules,
and does not remove the old files automatically.

Implementation: [path resolution and verified consolidation](../../crates/core/src/local_storage.rs),
[desktop command](../../apps/desktop/src-tauri/src/commands/media.rs),
[settings](../../apps/desktop/src/components/settings/LocalStorageSection.tsx).
