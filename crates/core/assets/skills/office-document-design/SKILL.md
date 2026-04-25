---
name: office-document-design
description: Activate when generating DOCX, XLSX, or PPTX files via Python-backed Office workflows (`doc-script-editor`, python-docx, openpyxl, python-pptx) or native fallback tools. Trigger on requests to create reports, spreadsheets, presentations, slides, decks, workbooks, or any polished office document output.
---

## Trigger
When creating DOCX, XLSX, or PPTX files via `doc-script-editor` / Python Office libraries first, or `generate_docx` / `generate_xlsx` / `ppt_generate` as fallback.

## Tool Preference
1. Prefer Python-backed creation through `doc-script-editor` for real Office output, validation, templates, charts, formulas, speaker notes, or follow-up edits.
2. Use `generate_docx`, `generate_xlsx`, or `ppt_generate` only as compatibility fallback when Python/LibreOffice is unavailable or the file is very simple.
3. Always validate generated files when possible: `edit_doc.py --path <file> validate`.
4. For PPTX and layout-sensitive DOCX/XLSX, render pages/slides with `edit_doc.py --path <file> render --outdir <dir>` when LibreOffice/Poppler are available.
5. For template adaptation or precise existing-file surgery, use `unpack` → OOXML/media/relationship edits → `pack` → `validate` instead of rebuilding the file from scratch.
6. For XLSX formulas, run `recalc_xlsx` after writing formulas and treat any reported formula error as a blocking issue.

## Rules

### DOCX — Professional Documents
1. ALWAYS include: theme colors, title font, body font
2. Start with a cover page (title, subtitle, date/author note)
3. Use section rhythm: heading → 1-2 paragraphs → callout or table → next section
4. Insert callout boxes for key takeaways (tone: info for facts, warning for risks, success for wins)
5. Tables: use for any data with 3+ items. Always include header row
6. Bullet lists: max 7 items per list. Prefer grouped bullets with sub-headings
7. Existing templates override generic styling. Preserve margins, heading rhythm, headers/footers, and table conventions unless the user explicitly asks for redesign.

### XLSX — Data Workbooks
1. Sheet 1 = Summary dashboard (title banner, KPIs, key metrics)
2. Sheet 2+ = Detail data (raw data, calculations)
3. ALWAYS add charts when showing trends, comparisons, or distributions
4. Use formulas for derived values — never hardcode calculated numbers
5. Freeze header rows. Enable auto-filter. Set column widths explicitly
6. Use color coding: green for positive, red for negative, blue for neutral
7. For financial or scenario models, assumptions must live in input cells and formulas must reference those cells.
8. Do not save a workbook opened with `data_only=True`; it can destroy formulas.

### PPTX — Presentations
1. Max 6 bullets per slide. One message per slide
2. Storyboard: Title slide → Agenda → Content (3-7 slides) → Summary → Q&A
3. Use section divider slides between major topics
4. Comparison layout for pros/cons, before/after, option A vs B
5. Every data claim needs a source citation on the slide
6. Speaker notes: include detailed talking points (2-3 sentences per slide)
7. Do at least one QA pass for overlap, overflow, and low contrast before declaring completion.
8. Use visual elements on content slides: chart, image, icon, timeline, comparison, or stat callout. Avoid text-only slide runs.
9. For template decks, remove unused placeholders/shapes rather than leaving blank text boxes.

## Common Rules (All Formats)
- Choose colors that match the topic: blue for corporate, green for nature/health, orange for energy/startup
- Never use default black-and-white. Always set a theme
- Information hierarchy: most important info first, details second
- If user doesn't specify design, use professional blue theme: primary #2B579A, accent #217346
