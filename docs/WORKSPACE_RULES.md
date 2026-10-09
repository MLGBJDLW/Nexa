# Workspace file rules

The chat's `/rules` inspector follows its managed worktree when one is attached.
The project settings preview continues to inspect the project's source folders.
Narrowed workers retain the read-only rule acknowledgement tool whenever they
receive scoped file or process tools; this adds no file mutation permission.

Choose folders in a project's workspace settings. Nexa reads `AGENTS.md` in
those folders at the start of each native or host-tool agent turn. A nonempty
`AGENTS.override.md` in the same directory replaces `AGENTS.md`. Rules apply only
to their directory and descendants; more specific directories take precedence.
Explicit user instructions and the project's instructions remain authoritative.

For projects that keep Nexa contracts together, `.nexa/AGENTS.md` is a fallback
when neither standard rule file supplies instructions in that directory. It has
the enclosing project/directory scope, not just the `.nexa` subfolder. Reading a
tool manifest inside `.nexa/tools` does not load the same file a second time.
No existing rules are moved or rewritten. See [local storage](LOCAL_STORAGE.md).

Open **File rules** in the project menu, command palette, or `/rules`. The
inspector shows the applicable files, directory scopes, content revisions and
loading diagnostics. Enter a target file or directory to inspect its ancestor
chain. This is a current disk preview; tool receipts retain the rules actually
loaded during a conversation. Refreshing the inspector does not change a draft
or send a message to a model.

## Runtime behavior

The host loads child-directory rules lazily when a file tool names a path, or a
process/terminal tool supplies a working directory. It does not scan every child
directory or interpret shell source as a list of file accesses. Agents must read
the appropriate rules before using a shell command to change files elsewhere.

Read operations include newly encountered rules in their result. A write or
execution with unacknowledged rules returns those rules without performing the
operation. After inspecting them, the agent calls `workspace_rules` with
`action: "acknowledge"`, the same target paths and exact snapshot revision, then
retries. An acknowledgement is scoped to the conversation and turn; changes on
disk invalidate it. Acknowledgements neither approve tools nor weaken execution
permissions. Filtered registries share the same state, while another
conversation or a resumed turn must read its own current rules.

Only explicitly selected workspace folders provide instruction authority.
Knowledge sources are not searched for instructions. Discovery does not walk
above the selected roots, follow rule-file symlinks, or read a rule that resolves
outside its root. The combined content limit is 32 KiB across up to 64 rule
files. Truncated, invalid UTF-8, or inaccessible rules are displayed as
diagnostics; affected writes wait until the rules can be loaded completely.

External ACP agents retain their native rule discovery and permission behavior.
Nexa tells the runtime to use its native rules instead of injecting a duplicate
copy. This distinction also applies to native tools that run outside Nexa's
host-tool registry.

Implementation: [discovery and revisions](../crates/core/src/workspace_rules.rs),
[tool runtime](../crates/core/src/tools/workspace_rules_tool.rs),
[desktop context](../apps/desktop/src-tauri/src/desktop_agent_session/turn_config.rs),
and [inspector](../apps/desktop/src/components/chat/WorkspaceRulesPanel.tsx).
