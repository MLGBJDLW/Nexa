# Computer Use and local visual evidence

Computer Use is one observe–act–verify workflow. Consecutive native observation,
control and application-launch receipts appear in one expandable chat record.
The two executable contracts remain `computer_observe` and `computer_control`:
read-only observation and input keep distinct permission boundaries while sharing
the observation store, input arbiter, action journal and completion checks. There
is no third computer-use executor or duplicate set of model-facing aliases.

## Coordinates and recovery

Window screenshots and UI Automation use physical pixels. Native workers enter
a scoped per-monitor DPI context and restore the previous context on exit. The
window snapshot uses the compositor's visible frame bounds; `GetWindowRect` can
include invisible resize borders or DPI-virtualized dimensions and is not the
coordinate authority for a screenshot. Model pixels are mapped through the
captured image dimensions, without guessing a DPI multiplier. Semantic element
IDs remain scoped to the exact observation.

Arguments and captured-image bounds are checked before claiming the observation.
An out-of-bounds point therefore does not consume a valid token. Once a worker
claims an observation, the token stays consumed even if a subsequent precondition
fails. Typed failures distinguish coordinate bounds, transform mismatch, capture,
target identity, occlusion, focus, freshness and takeover. Diagnostics contain
fixed reason codes, never raw screen text, typed text or native error payloads.

An action that crossed its commit boundary returns `computer_action_uncertain`
with `sideEffect: may_have_occurred` and a diagnostic `causeCode`. Inspect fresh
state before any further input. Neither a retryable precondition nor a consumed
token authorizes replay of an uncertain click or keystroke. A successful control
result already contains a fresh observation: use it for the next step instead of
capturing the same window again.

## Visible control and Stop

The status window appears at the top center of the target monitor's work area,
including negative monitor origins and mixed scaling. It identifies observation,
control and the interval between actions; Stop remains available until the owning
run finishes. Run identities prevent another task's terminal event from clearing
the current activity.

The native action worker owns a compact purple-and-white Nexa pointer with a soft target
glow. It follows admitted pointer movement and marks semantic targets without
moving the user's pointer for UI Automation. The overlay is transparent to input,
does not activate a window, and is destroyed when that worker exits. It does not
change the Windows cursor scheme. Its independent window is outside the target
window's capture surface, so screenshots remain clean evidence.

If the Stop banner covers an admitted pointer target, only that registered,
process-verified Nexa window moves temporarily to the opposite edge of the same
work area. Its original placement is restored on worker exit without reopening
a hidden banner. Other windows and overlays still fail the normal input ownership
check.

## Inspecting a local image

Use `read_file` or `read_files` with `image_mode: "native"` and no line parameters.
This requests raw-image disclosure through the normal approval/full-access policy;
source scope and path validation still apply. Text redaction cannot redact pixels,
so an ordinary read under privacy redaction remains text-only unless this explicit
mode is admitted. `image_mode: "text"` always uses local extraction. OCR remains
available for requested text extraction and configured fallback processing.

Both file tools share one bounded decoder and the existing tool visual channel.
PNG/JPEG/WebP are decoded and normalized; GIF returns its first frame. Oversized,
corrupt and unsupported images are rejected before they can claim to be visual
evidence. Decoders run with bounded concurrency, and batch results identify native
evidence omitted by the request byte budget. Large transparent images are flattened
on white for JPEG output.

The exact model/endpoint capability and frozen vision routing permission jointly
decide whether the primary model receives pixels. OCR-only, local-only, disabled,
ask and auxiliary-vision selections remain authoritative, including when a worker
selects another model. Otherwise the configured visual interpreter handles a
text-only route. No second vision runtime is introduced. Pixel attachments are
removed before durable tool artifacts are persisted and are added to model context
only after the complete tool-result batch, preserving provider message ordering.

## Source references and verification

The implementation uses Nexa's existing contracts. Design references include
[pi's image-aware read tool](https://github.com/earendil-works/pi/blob/42a3497d03ad17e308a2299fa824727894f2c0ec/packages/coding-agent/src/core/tools/read.ts),
[Codex's view_image handler](https://github.com/openai/codex/blob/322bbf4d8486efd7dbbcf49598711a9e3fefc282/codex-rs/core/src/tools/handlers/view_image.rs),
and [Cua's Windows pointer overlay](https://github.com/trycua/cua/blob/5274342fbf66ca2998325be5e900299406ca8650/libs/cua-driver/rust/crates/platform-windows/src/overlay.rs).
The investigation also compared [Hermes' unified action tool and effect verdicts](https://github.com/NousResearch/hermes-agent/blob/5ba559c9e4b1c8397df7788e319ab634144ca728/tools/computer_use/tool.py#L519-L577);
the public tool count alone does not replace permission, recovery and receipt
integration.

Core regressions cover fresh-token retention, safe failure diagnostics, real local
image reads, native/interpreted routing and privacy. Interactive Windows helper
tests cover physical screenshot-coordinate clicks, semantic value/checkbox effects,
clean capture evidence, click-through pointer feedback and worker cleanup. Frontend
tests cover grouped live receipts, compact layout, waiting status and Stop.
