# UX Quality Bar

## Goal

This document defines the front-end quality bar for Nexa as a consumer-facing desktop assistant.

Every major UI change should be judged against this bar before it ships.

## Product Experience Standard

The interface should feel:

- calm
- clear
- grounded
- useful
- predictable

It should not feel:

- noisy
- over-technical
- agent-debug-heavy
- visually fragmented
- like it was built only for programmers

## Core UX Rules

### Chat editing and command access

Long user messages initially show roughly five lines of rendered content. The
disclosure expands the complete message and suspends automatic scrolling while
the user reads. Markdown is rendered after sending; the source toggle, copying,
and edit/resend retain the original text. User Markdown does not execute HTML or
diagrams, and remote image syntax does not initiate image requests.

The composer remains a plain-text editor for stable IME, selection, paste, and
dictation behavior. Its optional Markdown preview uses the same renderer and
never changes the draft. Plan/Normal stays above the editor. Compact provider,
model/reasoning selection sits in the bottom toolbar with attachments, voice,
sharing, and sending. Workflows, collaboration,
quality, Nexus, compaction, checkpoints, and emoji are grouped under More options.

Ctrl/Cmd+Shift+P opens the command palette; Ctrl/Cmd+K remains an alias. The palette
searches navigation, available chat actions, and conversation titles, and can
submit a query to document search. Arrow keys select, Enter runs, and Escape
returns focus. Customize the palette shortcut from the palette or Appearance
settings. `/model`, `/preview`, `/options`, `/attach`, and `/commands` are local
composer actions and are never sent to the LLM.

### 1. Show the user what matters, not what the model felt

Default UI should prioritize:

- what task is being worked on
- what sources are in scope
- what evidence supports the answer
- what the user can do next

Default UI should de-prioritize:

- raw chain-of-thought dumps
- verbose internal state labels
- agent implementation details that do not help user decisions

### 2. Streaming and completed states must match

- Streaming should not present a structure that collapses into a different structure after completion.
- Tool, phase, and reply order must remain stable.
- Users should feel that the run matured, not that it was reinterpreted.

### 3. Scope must always be visible

If the assistant is restricted by:

- source filters
- collection context
- file path constraints
- provider limits

the user should be able to see that clearly.

### 4. Evidence must be legible

- Answers should reveal support, not hide it.
- Evidence counts, citation chips, and confidence level should be easy to scan.
- “No evidence” and “weak evidence” should be explicit, never implied.

### 5. Consumer language first

Prefer:

- “working set” over “playbook state payload”
- “source scope” over “conversation_sources binding”
- “evidence strength” over “retrieval confidence artifact”

Internal naming can stay technical; user-facing copy should not.

### 6. Pages should form a workflow

Search, Collections, and Chat should feel like connected stages of one job:

- find
- review
- collect
- continue asking

The user should not need to mentally rebuild context after moving between pages.

### 7. Office and document tasks should feel natural

For non-technical users:

- document creation should feel intentional and guided
- file operations should feel safe
- generated outputs should look polished
- recovery and retry should be understandable

## Visual Quality Rules

### Layout

- Strong visual hierarchy
- Clear primary action
- Few simultaneous focal points
- No accidental duplication of the same context in multiple places

### Density

- Compact enough for desktop power use
- Spacious enough for ordinary users
- No “wall of pills” or “wall of debug panels” effect

### Motion

- Motion should explain state changes, not decorate them
- Streaming updates should feel continuous, not jumpy
- Expansion/collapse should preserve orientation

### Copy

- Short labels
- One meaning per chip/panel
- Status text should imply next action when relevant

## Review Checklist

Before shipping a UI change, check:

- Does the interface reduce or increase jargon?
- Is source scope obvious?
- Is evidence support visible?
- Do streaming and finished states feel like the same run?
- Can a non-technical user understand the page without prior explanation?
- Does the page help the user finish a real task faster?
- Are keyboard focus, reduced motion, long translations, and narrow phone
  layouts usable on the changed surface?
- Do manual composer corrections survive later speech hypotheses and retries?
- Do remote connection, permission, and expired-session states show an
  actionable next step while preserving the user's draft?
- Do closing a browser workspace and leaving Live release their owned
  sessions/resources, including asynchronous startup races?
- Does document preview distinguish successful opening from actual validation
  and visual inspection?

If the answer to multiple questions is “no”, the design is not ready.

## Acceptance evidence

Use focused [browser regressions](../apps/desktop/e2e) for layout and interaction
contracts, then verify the real native surface where the change depends on
WebView2, a microphone/camera, a phone, terminal input, or Office. Keep the tested
platform and remaining device/provider gaps explicit. See
[Contributing](../CONTRIBUTING.md#verification) for commands and evidence limits.
