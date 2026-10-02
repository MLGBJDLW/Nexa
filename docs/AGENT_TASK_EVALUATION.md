# Agent task evaluation

Nexa maintains separate evidence for runtime contracts and model task quality. Passing a
replay or scripted transport test does not establish a live model's success rate. The
task runner uses the real `AgentExecutor`, provider adapters, approval callback and
scoped file tools in a fresh temporary workspace.

## Evaluation layers

| Layer | Entry point | What a pass establishes |
| --- | --- | --- |
| Deterministic contracts | `npm run eval:smoke`, `npm run eval:nightly` | Routing, evidence, orchestration and stored-trajectory invariants |
| Wire and recovery fixtures | Core provider/MCP tests; `cargo test -p nexa-agent-eval` | Request encoding, local HTTP execution, transport failures, independent task oracles and accounting |
| Scripted task transport | `npm run eval:tasks` | Real executor/tool/approval behavior on maintained tasks, using predetermined model responses |
| Live task quality | Explicit `--live` and route configuration | Whether that configured model completes the tasks within the selected budget |

[The maintained corpus](../eval/tasks.json) contains 20 cases covering code changes,
Unicode, parsing, preserving unrelated configuration, scoped output, read-only analysis,
approval continuation, recovery after an ambiguous edit, repository location,
enabled-tool discovery, long-turn compaction and two-image comparison. Required runtime
evidence (successful tool names, minimum tool calls, compaction and approvals)
accompanies file/answer oracles. Each acceptance program rejects the original task
state. Multimodal MCP and opaque-replay fixtures are additionally maintained at their
core runtime interfaces; the task score is not a complete desktop/browser competence
score.

## Run and inspect

From the repository root with Rust and Node.js 24:

```sh
cargo test -p nexa-agent-eval
cargo run -p nexa-agent-eval -- --output docs/local/task-eval.json
cargo run -p nexa-agent-eval -- --filter approval --output docs/local/approval-eval.json
```

Default mode makes no model-account request. Its report is labelled
`scripted_runtime_contract`, and synthetic token counters are not billing evidence. A
task passes only when execution and its independent oracle succeed. Approval tasks also
require an actual approval event.

Oracles and reference answers are outside the agent's authorized workspace. Oracle Node
processes receive no provider key or unrelated user environment, have filesystem read
permission only for the workspace and oracle inputs, and have a separate timeout.
Diagnostics are bounded. The trusted runner also requires completion after all
assertions, rejecting early process exits and replaced assertion exports. These are
quality-test guards; imported JavaScript shares the oracle VM and this is not an
adversarial-code sandbox. The agent receives only the selected production file tools,
without shell or unrestricted filesystem access.

Reports verify the compiled source SHA and content fingerprint against the checkout,
rejecting stale binaries or source changes during a suite. Matching dirty development
builds can run but cannot become version baselines. Reports record source SHA/dirty
state, corpus digest, Node version, model route, reasoning setting, repetitions,
time/token limits, oracle results and output evidence, provider invocations,
first-output/total time, reported usage, tool failures and approvals. Time remaining
after provider waits includes tools and scheduling; it is not a CPU profile. Repeated
cumulative usage chunks replace the prior observation for that provider invocation
instead of being added again.

## Opt-in live run

Store a non-secret route configuration outside tracked source, for example
`docs/local/eval-config.json`:

```json
{
  "provider": "openAi",
  "baseUrl": "https://api.openai.com/v1",
  "model": "gpt-5.6",
  "repetitions": 3,
  "maxTokensPerTask": 24000,
  "timeoutSeconds": 120,
  "reasoningEnabled": null
}
```

Set `NEXA_EVAL_API_KEY` through the local environment, then run:

```sh
cargo run -p nexa-agent-eval -- --live --config docs/local/eval-config.json --output docs/local/live-eval.json
```

Choose the model and budget intentionally. The runner does not read saved app
credentials. Local Ollama/LM Studio routes can omit the key; live runs still require an
explicit model and URL. Keys are not intentionally included in reports or oracle
processes. Provider error diagnostics should be reviewed before sharing reports.

Reported-usage costs remain `null` unless `prices` supplies input, output, cache-read
and cache-creation prices per million tokens, a currency and a source. Missing provider
usage also leaves cost unknown. These are estimates from supplied rates and
adapter-specific usage counters. Anthropic uncached input is disjoint from cache
counters; OpenAI-compatible and Gemini input includes cache reads. These estimates cover
only returned usage, not provider invoices. Adapter-internal HTTP retries are not
exposed at this boundary, so physical attempt counts remain unknown and reported-usage
costs may omit failed attempts. Model/pricing changes must not be presented as
implementation-only improvements.

## Compare versions and CI

```sh
node scripts/compare-task-eval.mjs docs/local/baseline.json docs/local/candidate.json
```

Comparison rejects different corpora, modes, model routes, reasoning settings, runtimes,
repetitions and budgets, and rejects dirty source as a version baseline. It reports lost
task successes even if a failing candidate is faster. Use repeated live runs to
distinguish model/network variability from code changes; retain per-task evidence and
immutable source revisions.

PR CI runs the scripted suite and a loopback HTTP fixture through the real live adapter
without paid credentials. The fixed per-request output limit is 2,048 tokens; task
input-window overrides and required runtime events are included in each report. The
existing nightly workflow uploads scripted task evidence too. Its manual `run_live`
input requires a `NEXA_EVAL_CONFIG` repository variable and dedicated
`NEXA_EVAL_API_KEY` secret. Scheduled runs never enable that step automatically.

Implementation: [task runner](../crates/agent-eval/src/lib.rs), [provider
metering](../crates/agent-eval/src/provider.rs), [local live-wire
test](../crates/agent-eval/tests/live_wire.rs), and
[comparison](../scripts/compare-task-eval.mjs).