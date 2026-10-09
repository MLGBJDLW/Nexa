# Native document input

Nexa prepares original-file evidence for PDF, DOCX and PPTX attachments and for
authorized `read_file` / `read_files` calls. The physical provider request selects
native input when its provider, endpoint, API, catalog model and document type are
verified. Otherwise the existing local extraction is sent. No provider Files API
upload or persistent remote file ID is created.

| Actual route | Original files | What the model receives |
| --- | --- | --- |
| Official OpenAI Responses | PDF, DOCX, PPTX | PDF text and page images; Office text only |
| Official OpenAI Chat Completions | PDF | Text and page images |
| Official Anthropic Messages | PDF, up to 100 pages | Text and page images |
| Official Gemini generateContent | PDF, up to 1,000 pages | Text and page images |
| Private/compatible endpoints, unlisted models, subscription runtimes | Local extraction | Text, including available local visual descriptions |

These are input semantics, not a claim that a model has inspected or understood
every page. The attachment's **Automatic file reading** label explains this policy;
its tooltip distinguishes PDF pages from Office text. It is not a completion receipt.
OpenAI's existing route selection still applies: direct GPT-6 models use Responses;
the document feature does not silently switch an established model's API or account.
Copilot, Codex and ACP keep extracted input until their particular runtime/model's
file protocol is verified. Image support alone is not document support.

Native Office input does **not** include embedded images or charts. Use an explicit
PDF export or rendered-page evidence for visual layout work. XLSX/XLS remain on the
existing structured extraction and Office tools, which can inspect complete ranges,
formulas and recalculation instead of relying on a provider's partial spreadsheet view.

## Privacy and scope

Nexa's enabled privacy redaction policy uses local extraction. Original binary
documents cannot be treated as redacted text. The default privacy configuration is
enabled; this feature does not disable it. Existing privacy revocation cancels
in-flight work and is rechecked before each physical request and tool release.

File tools first resolve the existing workspace/source permission boundary. With
no explicit line parameters, a supported document can provide its whole original
file alongside a bounded local preview. Explicit `start_line`, `max_lines`, or
`max_lines_per_file` selects extraction only; it never uploads the whole document.
Text files keep their ordinary paging behavior. Tool extraction and native bytes
come from the same authorized file snapshot, so a later edit cannot mix two revisions.

Tool document bytes are removed before tool cards, receipts, traces and database
writes. The current-turn evidence follows **all** results from a parallel tool batch.
Durable tool history retains extraction for replay. The typed Document part omits
binary data from serde and Debug; a restored part automatically falls back to text.
User-uploaded attachment storage retains its existing ownership and privacy rules.

## Limits and fallback

The existing 10 MiB per-file product bound applies. A physical request includes at
most 16 MiB of original documents, with a lower bound when existing text/images
approach the inline request envelope. Native projection also checks remaining catalog
context capacity when that catalog entry supplies a window; the executor keeps
its resolved budget for entries without window metadata. Invalid containers, encrypted PDFs, excessive page counts, unknown
routes, restored history without bytes and privacy mode use extraction or the existing
explicit extraction error. UTF-8 text fallback is bounded and identifies truncation.

Native document estimates use extracted text and PDF page count, never base64 as
ordinary text and never zero cost. Content digests and byte availability participate
in prompt fingerprints. Actual provider usage still feeds the existing context calibration.

An explicit HTTP input-format/size rejection before streaming can retry extraction
once. Authentication, authorization, rate limits, network errors and server failures
keep their normal handling. A route fallback prepares again from the original
candidate rather than reusing a primary provider's wire representation.

## Verification

Offline tests cover exact endpoint/model/API gating, DOCX/PPTX and PDF wire formats,
container/MIME validation, aggregate limits, privacy, restored history, one-time format
fallback, file-tool scope/ranges, subscription preparation and parallel tool ordering.
They prove request and lifecycle contracts; provider answer quality on complex real
documents still requires a live semantic evaluation against the selected account/model.

Protocol references checked 2026-10-09:
[OpenAI file inputs](https://developers.openai.com/api/docs/guides/file-inputs),
[Anthropic PDF support](https://platform.claude.com/docs/en/build-with-claude/pdf-support),
[Gemini generateContent documents](https://ai.google.dev/gemini-api/docs/generate-content/document-processing).
