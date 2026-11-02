## DOCX Playbook

### Generation
- Start with a cover or title block: title, subtitle or scope, author/date line, and a short executive summary when appropriate.
- Use a repeating rhythm for long reports: heading, short framing paragraph, evidence block or table, then implication or next action.
- Use `python-docx` for normal sections, headings, paragraphs, tables, images, headers, footers, and page breaks.
- Use a template document when the user provides one; copy the template and edit inside it instead of recreating styles.

### Editing Existing DOCX
- Take a `version` snapshot before destructive or broad edits.
- Use text replacement only for small formatting-preserving changes.
- Prefer the typed candidate lifecycle for comments, threaded replies, resolution state, tracked replacement, bookmarks, safe fields, content controls, protection, and template binding. Address a comment thread by its inspected `commentId`; do not guess IDs from visible order.
- Inspect `trackedChanges.unsupportedForAcceptReject` before accepting or rejecting revisions. Insertions and deletions are handled directly across Word story parts; moves, property/table revisions, conflicts, and custom-XML revisions must fail closed or use native Word automation.
- Use manual OOXML unpack/pack only for an explicitly inspected object that has no typed operation, and keep changed-part and relationship evidence.
- Preserve headers, footers, numbering, section breaks, margins, and named styles unless asked to redesign.

### Typography
- Reuse named paragraph styles for each role. Body paragraphs, list text and table text should share the body font and size; headings, title, subtitle and captions retain their intentional hierarchy. Configure header/footer styles too, including page fields.
- A Spec v2 `template` keeps its existing styles and first-section page geometry by default. Supplying `preset` or nonempty `tokens` explicitly applies the generated style system; supplying `page` explicitly changes the page geometry. Use the existing-document edit lifecycle for content-only modifications.
- Spec v2 supports `bodyFont`/`headingFont`, `bodyEastAsiaFont`/`headingEastAsiaFont`, `bodyComplexScriptFont`/`headingComplexScriptFont`, `bodySize`, `titleSize`, `subtitleSize` and `captionSize` in `tokens`. Sizes are points. Select fonts available in the target reader; font names do not embed or install a font. For Chinese documents set a Chinese font explicitly when the user supplies a preference.
- `python-docx`'s `font.name` sets only `w:ascii` and `w:hAnsi`. Set `w:eastAsia` and `w:cs` in `w:rFonts`, remove conflicting `*Theme` attributes on the styles you intentionally change, and set `w:szCs` to the same half-point value as `w:sz`. Otherwise mixed scripts can retain unrelated template fonts or sizes.
- When repairing existing documents, inspect direct run formatting and linked character/table styles as well as paragraph styles. Normalize only requested roles; preserve bold/italic emphasis, symbol fonts, equations, links and existing layout. Retain the original document and validate the candidate.
- Reopen the saved file and check effective fonts/sizes across title, body, lists, table cells, captions and story parts. Then render representative mixed-language pages and inspect line breaks and glyph fallback. A successful XML check alone does not prove that the reader has the requested font installed.

### Visual Quality
- Turn any comparable list with three or more rows into a table with a header row.
- Highlight decisions, risks, and recommendations with visually distinct callouts.
- Avoid dense pages with uninterrupted paragraphs; break them with headings, tables, or callouts.
- Validate the file, then render or convert to PDF for a layout pass when page fidelity matters.
