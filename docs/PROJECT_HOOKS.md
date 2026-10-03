# Project lifecycle checks

Open **Project checks** from the project menu, command palette, or `/hooks`.
Nexa discovers command manifests in `.nexa/tools` and `.agents/tools` inside
the selected workspace roots. Discovery does not execute or enable them.

For example, save `.nexa/tools/typecheck.json` in a Node project:

```json
{
  "name": "typecheck",
  "description": "Verify TypeScript before completing work",
  "command": { "program": "node", "args": ["node_modules/typescript/bin/tsc", "--noEmit"], "timeoutSecs": 120 },
  "access": { "read": true, "write": false, "execute": true, "network": false }
}
```

Review the command, choose **After file changes** or **Before completion**, supply
its JSON argument object, and enable it. Enabling explicitly authorizes that
manifest version and those arguments for future project runs. A changed manifest
requires a new explicit enablement. Disable or delete an entry to stop future runs.
There are no model-facing configuration or enable tools.

## Execution and completion

- Checks run through the project-tool execution environment with a bounded timeout,
  bounded displayed output, workspace cwd, and owned process lifetime. Task
  cancellation terminates the process tree. This lifecycle containment is not
  an operating-system filesystem or network sandbox.
- After-file checks run after a native tool batch settles. External agent writes
  are also checked before the host accepts the final answer. Coverage follows
  Nexa's tracked file-change receipts; unobserved changes in another application
  do not automatically trigger a check.
- Command stdout/stderr are observations. Failed checks do not turn a successful
  edit into a failed edit and must not cause the edit to be blindly replayed.
- Before completion, all enabled checks must pass for the current tracked change
  revision. The native agent receives failures for bounded repair attempts;
  external runtimes cannot persist a successful final answer while checks fail.
  Completion receipts also remember the file revision after their command ends.
  If a later hook changes files, an earlier completion check becomes a visible
  stale failure. Nexa does not replay successful mutating hooks to repair this;
  inspect the changes, fix the check configuration or make a new tracked repair.
  Put transformations before final validations in a single declared command when
  their order is required for a meaningful result.
- A check runs once per hook configuration, conversation, turn, and tracked agent
  change revision. Formatter writes are recorded in the file-change history and
  do not recursively trigger the same hook. New agent edits allow a fresh run.
  Interrupted or failed runs are not automatically replayed against the same
  revision. Repair the files or start a new turn to run again.
- Plan mode never runs hooks. Existing deny policies and unavailable project-tool
  permissions remain authoritative. Controller-isolated runs do not support
  project-tool commands, so enabled checks block completion with an explicit
  explanation; disable them or use a normal workspace run.
- The recent-run panel retains status and output across restart. Interrupted
  runs become cancelled on startup. Deleting configuration preserves its receipts.

Commands receive `NEXA_HOOK_EVENT_JSON` containing the event, conversation ID,
turn ID, file revision and observed files. This data is not shell-interpolated.
Do not place credentials in hook arguments or output.
