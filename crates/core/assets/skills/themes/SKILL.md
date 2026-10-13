---
name: themes
description: Customize Nexa's own appearance, switch or restore a theme, or change interface colors, fonts, and backgrounds. Use for Nexa 主题、换肤、外观 requests; website and document styling belong to their own workflows.
---

# Themes

Use the existing `appearance` tool for Nexa's interface. Settings and chat share this registry. If the tool is deferred, discover `appearance` with `tool_search`.

- Start with `action: "list"` to read the active theme and installed names. Use `name` for `activate` and `remove`. If multiple themes share that name, use the exact `themeId` from the list.
- Switch an existing theme with `activate`; restore the previous theme with `rollback`. Keep the host's normal permission flow and the user's existing authorization.
- Create with a human-readable `name` and the requested `theme` overrides; Nexa supplies the internal identity and defaults. When editing, reuse the installed definition and preserve its id, especially when renaming. Read [authoring.md](references/authoring.md) for the minimal format and available fields.
- Use `apply` directly when the user requested a change. Use `draft` for a preview or to inspect normalization; it is optional before applying. A preview-only request ends at the draft. Keep changes in the declarative format so rollback and Settings remain consistent.
- Read `list` after a mutation to confirm the active theme. Report the theme name and what changed; only claim visual inspection when an actual rendered view was inspected.
