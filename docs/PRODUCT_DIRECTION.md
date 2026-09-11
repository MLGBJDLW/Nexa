# Product direction

Nexa is a local-first desktop assistant with knowledge retrieval at its core.
It helps people find and explain information, create and revise documents,
organize evidence, and carry work across conversations, projects, and devices.

This document defines the durable product position. Implemented features and
ongoing priorities are tracked in the [roadmap](ROADMAP.md); setup belongs in
the [README](../README.md) and [user guides](README.md#use-nexa).

## Audience

Nexa serves office workers, students, researchers, operations teams, founders,
and other people working with personal files and knowledge. Developer tools
can support these tasks, but ordinary users should be able to understand the
main workflow without learning agent internals.

## Core pillars

| Pillar | Product requirement |
| --- | --- |
| Local-first ownership | Sources, indexes, collections, conversations, and durable task state belong to the desktop. Make each external service and disclosure boundary understandable. |
| Evidence-first answers | Show source scope and usable citations. Separate direct support, inference, and missing evidence. A graph relationship or generated summary is not a substitute for the underlying document. |
| Useful desktop assistance | Help users create, inspect, edit, and compare real artifacts. Show validation and recovery options alongside the result. |
| Clear interaction | Keep task progress, questions, approvals, stops, and failures understandable. Model/controller diagnostics belong in inspectable detail. |
| Reusable working sets | Search, collections, projects, memory, and conversations should preserve the user's working context across surfaces. |
| Continuity with control | Phone access, Live, and schedules reuse desktop ownership and permissions. Explain when the computer must remain running and which devices or services can access data. |

## Product principles

- Ground factual work in inspected evidence and show uncertainty when support is
  missing or contradictory.
- Make sources, model connections, and consequences visible before the user
  grants access or starts a costly operation.
- Preserve user edits and decisions across streaming, retry, navigation, and
  reconnection.
- Treat previews and validation reports as parts of the document workflow;
  opening a file is not a content-quality verdict.
- Put understandable defaults and language ahead of exposing every runtime knob.
- Keep optional network services, credentials, device pairing, and native
  integration trust separate from local storage.

## Product boundaries

Nexa is not intended to maximize autonomy at the cost of user control, make raw
reasoning traces the primary interface, or present unknown provider capability
as guaranteed support. A successful mock or compile check is not a claim that
every physical device, paid account, or Office host has been tested.

The phone client is a browser surface backed by the running desktop, not a
separate always-on cloud executor. Scheduled tasks also require that runtime to
be available. External extension declarations gain only the capabilities the
host actually implements and authorizes.

## Shipping heuristic

A feature is aligned when it improves trust, evidence quality, everyday
usefulness, or continuity without weakening local ownership and clear control.
Apply the [UX quality bar](UX_QUALITY_BAR.md) and
[internationalization rules](I18N_GUIDELINES.md) to every affected surface.
