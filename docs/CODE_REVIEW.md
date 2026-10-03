# Local code review and GitHub PR observations

Open **More options → Code review**, choose it in the command palette, or enter `/review`. These entry points use the same local action and preserve the composer draft. A chat must belong to a project with one Git workspace folder and an existing commit. Managed chat worktrees are supported. A project subfolder restricts the diff to that subfolder.

The earlier generic review prompt remains available as `/review-prompt`.

Choose a comparison before starting:

| Comparison | Content and baseline |
| --- | --- |
| All local changes | Tracked changes against current HEAD, plus untracked files |
| Staged changes | Index against current HEAD |
| Unstaged changes | Working files against the index, plus untracked files; the index fingerprint is displayed |
| Branch against baseline | The selected reference's merge base with HEAD, pinned when the review starts; local dirty files are excluded |

The panel displays the workspace, comparison, baseline, HEAD and exact diff revision. **Refresh diff** observes current files. A new review retains earlier reviews in the saved-review picker. Reviews, immutable diff versions and findings persist in the application database and are removed when their owning chat is deleted. They do not create commits, stage files, or push branches.

Click an old/new line number, enter a priority, title and concrete explanation, and record a finding. The host validates the path and displayed hunk line against a fresh diff. Duplicate submissions of the same finding are idempotent. Binary files cannot receive line anchors. Renames appear as deletion/addition so old and new paths remain explicit.

Findings can be accepted, dismissed, or marked **Verified fixed** after inspecting the refreshed change. A changed diff or HEAD makes previous findings stale, including after an empty commit. Original anchors are immutable. A stale finding cannot be accepted or sent as a current repair instruction; record a replacement at the new line. Resolving or dismissing a stale finding records the user's explicit decision at the current revision, without rewriting the original evidence. This is a human disposition, not an automatic assertion that tests passed.

**Add review request to draft** prepares an Agent review request. **Add selected fixes to draft** includes only selected current open/accepted findings. Nothing is sent automatically. A stable packet marker prevents duplicate insertion into the same draft. The `code_review` tool can read the selected snapshot, page a file diff, and record findings. It cannot change user dispositions or access a different workspace. The Agent should inspect the current revision before fixing code and refresh after verification. External runtimes that do not expose Nexa host tools can still use the panel, but their native review output is not automatically imported.

Captures are bounded to 200 files, 256 KiB displayed per file and 4 MiB total diff text. Git output is bounded; untracked content is read only for regular files in scope. Content hashes include binary data and text omitted from a displayed patch. Omitted/oversized content is visibly marked, and incomplete snapshots cannot record decisions or generate repair feedback. Narrow the project/change or inspect the full change separately. This does not replace semantic review or test execution with `git diff --check`.

## Attach a GitHub PR

Enter a `https://github.com/owner/repository/pull/number` URL and choose **Read / refresh PR**. The connector uses the installed GitHub CLI and its existing authentication. If needed, install `gh` and sign in with `gh auth login` yourself. Nexa does not read or persist its token.

The read-only observation includes PR state, draft/review decision, current head/base SHA, the head commit's checks, unresolved review threads, and observation time. A local/remote HEAD mismatch is shown explicitly. Refresh after new commits or remote review activity; stored observations are not live checks. At most 100 check contexts and 100 review threads are returned, with a partial-data warning when more exist. Zero visible checks/threads or a partial response does not establish merge readiness. Each thread shows its latest comment and whether its anchor is outdated.

The connector supports GitHub.com and performs GraphQL queries only. PR publication, comments, approvals, resolutions and merges are not exposed through this panel or the `code_review` tool. Use a separately authorized GitHub workflow for those actions. GitHub Enterprise is not supported by this initial connector.

Protocol references: [GitHub CLI API input](https://cli.github.com/manual/gh_api) and [GitHub pull request GraphQL schema](https://docs.github.com/en/graphql/reference/pull-requests).
