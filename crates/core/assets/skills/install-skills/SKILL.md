---
name: install-skills
description: Install or update agent skills in Nexa from a copied npx skills add command, a skills.sh listing, a GitHub repository or a local SKILL.md bundle.
---

# Install skills in Nexa

Use `manage_skill` for both inspection and installation. Nexa stores the resulting skills in the current user's library. Desktop, paired phones and standalone mobile use the same package format.

1. For a public GitHub repository, pass the copied `npx skills add owner/repo --skill name` or `npx skills add owner/repo@name` as one string in `sources`. For a skills.sh listing at `https://skills.sh/owner/repo/name`, use `npx skills add owner/repo@name`. For a checked-out or CLI-installed skill, pass its accessible local directory, such as `.agents/skills/name`. Private repositories need an accessible authenticated checkout.
2. Call `inspect_install_sources`. Match the requested skill names, inspect resource lists and warnings, and retain each selected `skillFile` and `contentDigest`. If a collection contains several skills and the request is ambiguous, show the names. If the user requested the entire collection, select all returned bundles.
3. Call `install_sources` with the same sources, revision if specified, and exact selection. Use `replace_existing` for an explicitly requested update. Treat CLI flags such as `-y` as command syntax and honor the host approval settings. If content changed, inspect it again.
4. Report the installed names and IDs. If the user also asked to use the skill, call `activate_skill` with the returned ID.
