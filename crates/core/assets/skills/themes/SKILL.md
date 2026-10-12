---
name: themes
description: Customize Nexa's own appearance, switch or restore a theme, or change interface colors, fonts, and backgrounds. Use for Nexa 主题、换肤、外观 requests; website and document styling belong to their own workflows.
---

# Themes

Use the existing `appearance` tool for Nexa's interface. Settings and chat share this registry. If the tool is deferred, discover `appearance` with `tool_search`.

- Start with `action: "list"` to read the active theme, built-in ids, and installed themes. Use short display names when talking to the user. `activate` and `remove` accept an exact id or unique custom name through `themeId`; resolve duplicate names from the list before changing anything.
- Switch an existing theme with `activate`; restore the previous theme with `rollback`. Keep the host's normal permission flow and the user's existing authorization.
- To create or edit a theme, reuse an installed definition where appropriate. Give new themes a short, human-readable `name` and a distinct stable `id`; keep the id when editing. Read [authoring.md](references/authoring.md) for a minimal draft and supported customization.
- Validate with `draft`, then pass the returned normalized `plugin` to `apply` when the user requested applying the change. A request only for a preview ends at the draft. Keep all changes in this tool's declarative format so rollback and Settings remain consistent.
- Read `list` after a mutation to confirm the active theme. Report the theme name and what changed; only claim visual inspection when an actual rendered view was inspected.
