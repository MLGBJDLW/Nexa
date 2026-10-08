# Workflow Packages

Workflow packages are user-facing task templates. They compose tools, skills,
connectors, agent roles, prompts, and approval expectations into repeatable
workflows that non-technical users can understand.

Workflows are not native plugins. They should not contain host code. They are
product contracts that tell Nexa how to guide a task.

Status: the built-in catalog, editable saved workflows, recorded procedures,
and workflow execution are implemented. The
portable `workflow.yaml` format below is a design direction, not an implemented
general-purpose YAML importer or executor.

## Package Shape

The long-term portable shape is:

```text
<workflow-id>/
  workflow.yaml
  prompts/
  examples/
  tests/
```

Proposed workflow-specific metadata (distinct from the implemented
[generic capability manifest](CAPABILITY_PACKAGES.md)):

```yaml
id: document_compare
name: Document Compare
surface: workflow_package
description: Compare local documents for overlap, contradictions, and decision-relevant differences.
version: 1
requiredTools:
  - compare_documents
  - retrieve_evidence
optionalSkills:
  - evidence-first
approval:
  writesFiles: false
  usesNetwork: false
```

## Workflow Rules

Workflow packages should:

- use consumer-facing language
- make source scope and evidence expectations explicit
- list required tools and optional skills
- avoid raw chain-of-thought or model-debug wording
- describe review, cancellation, and failure states for long tasks
- declare whether file writes, network access, shell execution, or connector
  tools may be needed

## Built-In Status

Nexa's built-in workflow catalog is represented by the `builtin-workflows`
package manifest with surface `workflow_package`. Each catalog template maps to:

```text
.nexa/capabilities/builtin-workflows/workflows/<workflow-id>/workflow.yaml
```

This path is catalog metadata. Templates execute from
[workflow_catalog.rs](../crates/core/src/workflow_catalog.rs) through Nexa's
workflow runtime; the catalog path is not a directory the user must install.

Current built-in workflows include:

- Research + Verify
- Draft + Review
- Meeting Summary
- Document Compare
- Report Brief
- Connector + Background Task

## Authoring and running

The Workbench opens a configuration panel before running a built-in template.
Supply a goal, context, constraints, output requirements, and Source scope;
preview the backend-generated procedure, then save a copy or save and run it.
Saved workflows retain a versioned template snapshot and can be reopened,
edited, or copied. Additional instructions create a custom procedure. The
trigger and permissions action edits schedule/folder settings and the model
execution policy without flattening the saved procedure into a prompt.

Record & Replay saves ordered action, decision, check, and note steps together
with variables, preferences, success criteria, and safety notes. Reopening a
recording restores those fields. This is semantic replay by the agent, not a
recording of mouse coordinates. Long instructions are preserved in full; model
context capacity and explicit user budgets still govern execution.

A saved workflow can run with temporary inputs without changing its defaults.
Each launch freezes the definition revision, resolved inputs, compiled prompt,
and definition digest. Run history opens that snapshot, the original chat, or
the exact Task Center run, including runs outside its first page. Older runs
without a saved snapshot are identified explicitly.

Manual, scheduled, and folder launches share durable occurrence and approval
state. A manual launch awaiting approval appears in the Workbench; approval
then starts its original snapshot. Manual tools retain interactive approval
semantics. Unattended schedule/folder launches still require their configured
execution grants. Folder progress is tracked per definition revision using the
observed file cutoff, so waiting for approval does not consume later changes.

## Stage dependencies

Template tasks declare `dependsOn` IDs. Drafting precedes review, report
research precedes outlining and drafting, and extraction/comparison precedes
verification. Independent tasks can run concurrently. The existing subagent
batch scheduler checks duplicate IDs, missing dependencies, and cycles before
registering workers. A dependent worker waits before acquiring an execution
slot; an upstream failure blocks it with an explicit reason.

Completed upstream results are passed in full as task data with their worker
identities and content digests. They do not become system instructions. Parent
batch observation can return early while registered workers continue; worker
settlement still publishes cancellation, failure, or panic to their dependents.
This does not add persistent node-by-node replay after application restart;
recovery continues to use the existing task and agent checkpoints.

These workflows should remain product-facing. If a workflow needs new external
tools, it should declare connector dependencies instead of becoming a native
plugin.

Project-local workflow package manifests can use the same
`.nexa/capabilities/*/capability.yaml` discovery path as other capability
packages.

For actual execution, budgets, and checkpoints, see
[Orchestration runtime](ORCHESTRATION_RUNTIME.md). Recurrence, occurrence
identity, and unattended permissions belong to [Scheduled tasks](SCHEDULED_TASKS.md).
Package metadata and its tests live in
[skills/package.rs](../crates/core/src/skills/package.rs).
