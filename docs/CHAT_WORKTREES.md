# Chat worktrees

Managed background commands retain their chat's workspace ownership after the reply finishes. Stop them through the process controls or `run_shell` with `service_action=stop` before changing or archiving the worktree. Ending the foreground reply alone does not stop a persistent service.

Open **More options → Chat worktree**, the command palette, or `/worktree`.
A project with one Git workspace folder can create independent checkouts for
multiple chats. Choose a starting branch, tag, or commit; the default is `HEAD`.
Nexa creates a new `nexa/chat-…` branch. Source-folder edits are not copied.

The panel identifies the owning chat, folder, starting commit, branch, and
snapshot. File tools, Git views, new terminals, rules, previews, and agent
workers resolve the chat's workspace from the same durable binding. External
ACP runtimes start in that directory. Their completed sessions are not kept
alive for managed worktrees. Changing the project of a bound chat is rejected.

Stop the chat and close its terminals before changing the worktree. An active
worker retains its workspace lease through settlement, including after the
parent collector closes. Operations are owned by the host, so closing the
panel does not abandon a Git mutation. **Check / recover** reconciles an
interrupted operation without deleting files. Archived or incomplete bindings
block a new turn instead of silently using the source checkout.

## Archive and restore

Review and select the archive checkbox, then **Archive with snapshot**. Nexa
saves tracked and untracked working files in a Git commit and preserves the
original staging-area tree in a second parent commit. The snapshot is pinned
under `refs/nexa/chat-worktrees/<id>/snapshot` in the original repository.
Only after saving that reference and rechecking files, HEAD, staging, and
ownership does Nexa remove its checkout with `git worktree remove`.

Ignored files, submodules, and embedded Git repositories block archive; preserve
or remove them explicitly first. Concurrent edits by programs outside Nexa
cannot be locked by the app; close those programs before archiving.

**Restore snapshot** recreates the same path with detached HEAD at the snapshot.
The restored files are committed together; the old staging split is available
from the snapshot's second parent (`<snapshot>^2`), rather than reapplied to the
index. Original branches and previous commits remain in the source repository.
These are local recovery snapshots, not an off-device backup: deleting the
original Git repository also destroys its snapshots.

Archiving or deleting a chat never automatically deletes its checkout or Git
snapshots. Ownership records remain in the application database. Keep a chat
available if you want to use its restore controls. This workflow is separate
from Code Ultra's temporary isolation and does not relax its promotion checks.
