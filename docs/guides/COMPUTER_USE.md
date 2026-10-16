# Computer Use and local visual evidence

[Documentation](../README.md) / [Category overview](README.md)

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
including negative monitor origins and mixed scaling. Only `computer_control`
starts this indicator; standalone observation and Browser Workspace operations
never announce desktop takeover. It remains available across desktop readback,
and hides when the run switches to other tools, pauses, waits for user input,
fails, finishes or stops its desktop session. Stop closes only that turn's
Computer Use scope: it cancels pending observations/input, denies pending desktop
permissions and blocks later desktop calls in the same turn. The model receives a
tool receipt and can continue chatting or using other tools. A new user turn gets
a fresh desktop scope. Run identities prevent a stale Stop click or another task's terminal event from clearing
the current activity.

The banner enters with a short eased slide/scale and uses the shared Nexa logo
with a flowing orbit and soft accent light. Reduced motion removes those effects.
The window stays non-activating, with transparency around its rounded surface.

The native action worker owns a compact triangular Nexa pointer, without a tail or idle halo.
Its color comes from the current theme's resolved accent, including theme resource
plugins; the main theme provider projects updates into the existing native surface.
Foreground actions temporarily replace Windows' hardware cursor shapes so the
system arrow or text caret cannot draw above Nexa's pointer. One lazily started
native feedback host retains exact originals and restores them on worker release,
parent-process exit, or an abandoned action timeout. The user's saved cursor
settings are never modified. Background UI Automation uses an input-transparent
marker at the semantic target and does not move or replace the user's pointer.

Four narrow, input-transparent native surfaces illuminate the active monitor's
edges using the theme accent. They share the banner's ownership and lifecycle,
do not activate windows, and need no full-screen renderer or animation polling.
The independent surfaces remain outside the target window's capture surface, so
window screenshots remain clean evidence.

Foreground pointer actions travel along a smooth, distance-scaled path at about
60 Hz (100–360 ms), including movement to a drag's starting point. The pointer and
feedback advance together; semantic background actions only mark their target.
For semantic clicks, the feedback follows the same animated approach without
moving the user's physical pointer; the semantic target is re-resolved afterward.
Each movement frame checks focus, physical buttons, cursor takeover and Stop.
Stopping signals the existing worker, which retains input ownership until cleanup;
an interrupted drag still releases its injected button and records uncertain
effects. Empty gaps between offset monitors are skipped without clamped input.
Delivered pointer clicks and semantic invocations show a short expanding ripple
in the same theme color. One ripple follows the complete click sequence, preserving
double-click timing. Reduced-motion preferences disable travel animation and ripples.

Coordinate clicks, scrolling and drag preparation recheck the target patch after
approaching it. Hover-triggered content within the same window can therefore stop
the action before button or wheel input. This still reports an uncertain effect
when the cursor has already moved. Explicit theme invalidations also refresh the
hidden main window, keeping native colors current during tray/background use.

If the Stop banner covers an admitted pointer target, only that registered,
process-verified Nexa window moves temporarily to the opposite edge of the same
work area. Its original placement is restored on worker exit without reopening
a hidden banner. Other windows and overlays still fail the normal input ownership
check.

## Shared interaction facts and ownership

Desktop and Browser Workspace actions share `observed_interaction` for observation
consumption, the possible-effect boundary, result facts and presentation phases.
Their platform adapters retain separate permission, target identity, capture and
input mechanisms. The existing activity journal remains the durable receipt;
presentation updates never grant authority or trigger an action replay.

An action's `interaction` receipt separates three facts:

- `delivery`: `notCommitted`, `inputSent`, or `mayHaveOccurred`.
- `evidence`: `changed`, `unchanged`, `targetClosed`, or `unavailable`.
- `condition`: `notRequested`, `matched`, or `pending`.

Changed pixels or DOM content alone do not prove the user's goal. Browser form
actions report a matched condition only after checking the requested checkbox,
selection or uploaded-file state against the fresh observation. Explicit browser
waits still recheck their condition against their final visual observation.

On Windows, a dedicated input thread records physical-input revisions and Escape
without retaining key text. Window observations bind the revision from capture;
input during capture or before a later action requires fresh evidence. Foreground
actions also watch physical input anywhere on the desktop, while background UIA
actions watch the target window's conservative revision bucket. Injected input
does not advance physical revisions. Nexa pointer movement uses injected absolute
motion across the virtual desktop and verifies the resulting physical position.
Existing focus, target, cursor and button checks remain in force.

Physical Escape stops only the Computer Use turn whose worker currently owns
desktop input. Queued calls and other chats do not acquire a stop signal. The
worker retains ownership while releasing buttons and finishing cleanup. Status
phases come from the executing worker: moving, sending input and observing the
result; a heartbeat cannot overwrite that phase.

Browser presentation is a separate module. Every preview carries a host operation
identity and reports actual animation completion. Old or cancelled acknowledgements
cannot complete a newer preview. A stalled decoration is removed and the normal
host lease and target checks still run; animation is never a capability limit.
Capture scopes hide only Nexa-owned decoration, wait for a rendered hidden frame,
and restore it on success, failure or cancellation. Native window capture remains
cursor-free WGC window capture, not a crop of the composed monitor image.

The native Windows adapter and its interactive fixtures live separately from the
tool admission/receipt module. The browser interaction script is a standalone
source file loaded by both the production host and browser regressions.

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
