use std::{collections::HashMap, fmt::Write as _, path::Path};

use crate::evidence::EvidenceLocator;
use crate::privacy::PrivacyConfig;

use crate::error::CoreError;

/// Explicit raw-image egress has its own approval even when text redaction is
/// enabled. Ordinary file reads never silently bypass that privacy setting.
pub(crate) fn requests_native_image(args: &serde_json::Value) -> bool {
    args.get("image_mode").and_then(serde_json::Value::as_str) == Some("native")
        && !["start_line", "max_lines", "max_lines_per_file"]
            .iter()
            .any(|field| args.get(field).is_some())
        && args
            .get("path")
            .and_then(serde_json::Value::as_str)
            .into_iter()
            .chain(
                args.get("paths")
                    .and_then(serde_json::Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(serde_json::Value::as_str),
            )
            .any(|path| {
                crate::media::is_supported_image(&crate::parse::detect_mime_type(Path::new(path)))
            })
}

pub(crate) fn native_image_consent_message(args: &serde_json::Value) -> Option<String> {
    requests_native_image(args).then(|| "Inspect the selected local image(s) visually by sending their pixels to the configured model or vision interpreter. Text redaction cannot remove content from these pixels. File access remains limited to the current authorized scope.".to_string())
}

pub(crate) fn file_read_capabilities(
    tool: &dyn super::Tool,
    args: &serde_json::Value,
) -> super::ToolRunCapabilities {
    super::ToolRunCapabilities {
        input_streaming: tool.input_streaming(),
        render_kind: tool.render_kind(),
        read_only: true,
        destructive: false,
        concurrency_safe: true,
        interrupt_behavior: super::ToolInterruptBehavior::Cancel,
        resource_keys: tool.resource_keys(args),
    }
}

pub(crate) enum NativeFileEvidence {
    Document(crate::llm::document::DocumentInput),
    Image(super::ToolOutputAttachment, usize),
}

impl NativeFileEvidence {
    pub(crate) fn byte_length(&self) -> usize {
        match self {
            Self::Document(document) => document.byte_length,
            Self::Image(_, bytes) => *bytes,
        }
    }

    pub(crate) fn into_attachment(self, fallback: String) -> super::ToolOutputAttachment {
        match self {
            Self::Document(document) => document_attachment(document, fallback),
            Self::Image(attachment, _) => attachment,
        }
    }
}

pub(crate) struct FileEvidence {
    pub text: String,
    pub native: Option<NativeFileEvidence>,
    source: ReadSource,
}

enum ReadSource {
    Text(blake3::Hash),
    Document {
        hash: String,
        passages: Vec<LocatedPassage>,
    },
    Unlocated,
}

struct LocatedPassage {
    start: usize,
    end: usize,
    locator: EvidenceLocator,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PassageCitation {
    line_start: usize,
    line_end: usize,
    citation: String,
}

pub(crate) struct FileCitations {
    pub suggested: String,
    pub passages: Vec<PassageCitation>,
}

impl FileCitations {
    pub fn write_hint(&self, out: &mut String) {
        writeln!(out, "Suggested citation: {}", self.suggested).unwrap();
        if self
            .passages
            .first()
            .is_some_and(|p| p.citation != self.suggested)
        {
            out.push_str("Passage citations by line in this extraction:\n");
            for p in &self.passages {
                writeln!(out, "{}–{}: {}", p.line_start, p.line_end, p.citation).unwrap();
            }
        }
    }
}

impl LocatedPassage {
    fn window_locator(
        &self,
        start: usize,
        end: usize,
        mut may_show: impl FnMut(&str) -> bool,
    ) -> Option<(EvidenceLocator, String)> {
        let mut locator = self.locator.clone();
        let label = match &mut locator {
            EvidenceLocator::Pdf {
                page,
                extracted_lines,
                ..
            } => {
                // Overlap is removed from the flattened chunk's beginning; its
                // last visible line still ends on the parser's recorded end line.
                if let Some(lines) = extracted_lines {
                    if self.end - self.start <= lines.end.saturating_sub(lines.start) as usize {
                        lines.start = lines.end - (self.end - start) as u32;
                        lines.end -= (self.end - end) as u32;
                    } else {
                        *extracted_lines = None;
                    }
                }
                match extracted_lines {
                    Some(lines) => format!("p. {page} · L{}–{}", lines.start, lines.end),
                    None => format!("p. {page}"),
                }
            }
            EvidenceLocator::Document {
                part,
                paragraph,
                table,
                row,
                ..
            } if may_show(part) => match (table, row) {
                (Some(table), Some(row)) => format!("{part} · table {table}, row {row}"),
                _ => format!("{part} · ¶{paragraph}"),
            },
            EvidenceLocator::Sheet { sheet, range, .. } if may_show(sheet) => {
                format!("{sheet}!{range}")
            }
            EvidenceLocator::Slide { slide } => format!("slide {slide}"),
            _ => return None,
        };
        Some((locator, label))
    }
}

fn markdown_label(name: &str) -> String {
    name.replace('\\', "\\\\")
        .replace('[', "\\[")
        .replace(']', "\\]")
        .replace(['\r', '\n'], " ")
}

impl FileEvidence {
    fn extracted(text: String, native: Option<NativeFileEvidence>) -> Self {
        Self {
            text,
            native,
            source: ReadSource::Unlocated,
        }
    }

    pub fn citations(
        &self,
        path: &Path,
        start: usize,
        end: usize,
        privacy: &PrivacyConfig,
        same_projection: bool,
    ) -> FileCitations {
        let label = markdown_label(&path.file_name().unwrap_or_default().to_string_lossy());
        let mut result = FileCitations {
            suggested: label.clone(),
            passages: Vec::new(),
        };
        let Ok(mut url) = url::Url::from_file_path(path) else {
            return result;
        };
        result.suggested = format!("[{label}](<{url}>)");
        if start == 0 || end < start {
            return result;
        }
        match &self.source {
            ReadSource::Text(hash) => {
                url.set_fragment(Some(&format!("L{start}-L{end}@{hash}")));
                result.suggested = format!("[{label} · L{start}–{end}](<{url}>)");
            }
            ReadSource::Document { hash, passages } if same_projection => {
                // Never expose a masked sheet name through a percent-encoded link.
                let mut checked = HashMap::<String, bool>::new();
                let mut may_show = |value: &str| {
                    !privacy.enabled
                        || checked.get(value).copied().unwrap_or_else(|| {
                            let allowed =
                                crate::privacy::redact_content(value, &privacy.redact_patterns)
                                    == value;
                            checked.insert(value.to_owned(), allowed);
                            allowed
                        })
                };
                let first = passages.partition_point(|p| p.end < start);
                for p in passages[first..].iter().take_while(|p| p.start <= end) {
                    let line_start = p.start.max(start);
                    let line_end = p.end.min(end);
                    let Some((locator, location)) =
                        p.window_locator(line_start, line_end, &mut may_show)
                    else {
                        continue;
                    };
                    let fragment = url::form_urlencoded::Serializer::new(String::new())
                        .append_pair(
                            "loc",
                            &serde_json::to_string(&locator).expect("serialize source locator"),
                        )
                        .append_pair("rev", hash)
                        .finish();
                    url.set_fragment(Some(&fragment));
                    result.passages.push(PassageCitation {
                        line_start,
                        line_end,
                        citation: format!("[{label} · {}](<{url}>)", markdown_label(&location)),
                    });
                }
                if let [p] = result.passages.as_slice() {
                    if p.line_start == start && p.line_end == end {
                        result.suggested.clone_from(&p.citation);
                    }
                }
            }
            _ => (),
        }
        result
    }
}

/// Read already-authorized bytes once. Extraction and native evidence use the
/// same snapshot even when the source file is subsequently changed.
pub(crate) fn read_file_evidence(
    path: &Path,
    native_candidate: bool,
) -> Result<FileEvidence, CoreError> {
    use crate::llm::document::{DocumentInput, MAX_DOCUMENT_BYTES};
    use std::io::{Read, Write};
    let mime = crate::parse::detect_mime_type(path);
    if mime.starts_with("text/") || mime == "application/octet-stream" {
        let bytes = std::fs::read(path)?;
        let text = crate::parse::decode_text_bytes(&bytes, path)?;
        return Ok(FileEvidence {
            text,
            native: None,
            source: ReadSource::Text(blake3::hash(&bytes)),
        });
    }
    if native_candidate && crate::media::is_supported_image(&mime) {
        use base64::Engine;
        let mut bytes = Vec::new();
        std::fs::File::open(path)?
            .take((crate::media::MAX_IMAGE_SIZE + 1) as u64)
            .read_to_end(&mut bytes)?;
        let evidence = crate::media::finalize_local_image(bytes)?;
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("image");
        let text = format!(
            "[Image: {name}] Native visual evidence, {}x{} pixels.{} Inspect the attached pixels; OCR is only needed for explicit text extraction.",
            evidence.width, evidence.height,
            if mime == "image/gif" { " First animation frame." } else { "" }
        );
        let length = evidence.image_bytes.len();
        let attachment = super::ToolOutputAttachment {
            name: name.to_string(),
            mime_type: evidence.mime_type,
            data: serde_json::json!({
                "base64": base64::engine::general_purpose::STANDARD.encode(&evidence.image_bytes),
                "width": evidence.width, "height": evidence.height,
                "source": "local_file", "firstFrameOnly": mime == "image/gif",
                "sourcePath": path.to_string_lossy(),
            }),
        };
        return Ok(FileEvidence::extracted(
            text,
            Some(NativeFileEvidence::Image(attachment, length)),
        ));
    }
    if !native_candidate || !crate::llm::document::supported_mime(&mime) {
        return read_projection(path).map(|mut evidence| {
            if crate::media::is_supported_image(&mime) {
                evidence.text.push_str("\n[Pixels were not included in this text-only read. For visual inspection, use read_file with image_mode=\"native\" and no line parameters; raw-image consent is required. Use extract_image_text only for text extraction.]");
            }
            evidence
        });
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take((MAX_DOCUMENT_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    let name = path
        .file_name()
        .and_then(|v| v.to_str())
        .unwrap_or("document");
    if bytes.len() > MAX_DOCUMENT_BYTES {
        return read_projection(path);
    }
    let suffix = path.extension().and_then(|v| v.to_str()).unwrap_or("bin");
    let mut snapshot = tempfile::Builder::new()
        .prefix("nexa-document-")
        .suffix(&format!(".{suffix}"))
        .tempfile()?;
    snapshot.write_all(&bytes)?;
    match read_projection(snapshot.path()) {
        Ok(mut evidence) => {
            evidence.native = DocumentInput::from_bytes(name, &mime, &bytes, &evidence.text)
                .map(NativeFileEvidence::Document);
            Ok(evidence)
        }
        Err(error) => {
            let text = format!("[Local extraction failed: {error}. The original file may still be read by a supported native document route.]");
            let native = DocumentInput::from_bytes(name, &mime, &bytes, &text).ok_or(error)?;
            Ok(FileEvidence::extracted(
                text,
                Some(NativeFileEvidence::Document(native)),
            ))
        }
    }
}

fn document_attachment(
    mut document: crate::llm::document::DocumentInput,
    fallback: String,
) -> super::ToolOutputAttachment {
    document.fallback_text = fallback;
    let mut data = serde_json::to_value(&document).expect("document metadata serialization");
    data["base64"] = serde_json::Value::String(document.data().to_string());
    super::ToolOutputAttachment {
        name: document.name,
        mime_type: document.media_type,
        data,
    }
}

pub(crate) fn generated_document_mime(path: &Path) -> Option<&'static str> {
    let mime = crate::parse::detect_mime_type(path);
    match mime.as_str() {
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => Some("docx"),
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" => Some("xlsx"),
        "application/vnd.openxmlformats-officedocument.presentationml.presentation" => Some("pptx"),
        _ => None,
    }
}

pub(crate) fn supports_document_fallback(path: &Path) -> bool {
    let mime = crate::parse::detect_mime_type(path);
    matches!(
        mime.as_str(),
        "application/pdf"
            | "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
            | "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
            | "application/vnd.openxmlformats-officedocument.presentationml.presentation"
    ) || mime.starts_with("image/")
}

pub(crate) fn edit_guidance_for_path(path: &Path) -> Option<String> {
    if let Some(format) = generated_document_mime(path) {
        return Some(format!(
            "Office documents are not plain-text editable with edit_file/create_file. Use office_artifact for transactional DOCX/XLSX/PPTX creation, modification, validation, publication, and restore. Use run_shell + doc-script-editor for PDF, conversion/rendering, or low-level OOXML compatibility work. Pair it with docx-document-design, pptx-presentation-design, or xlsx-workbook-design. Detected format: '{}'.",
            format
        ));
    }

    let mime = crate::parse::detect_mime_type(path);
    if mime == "application/pdf" {
        return Some(
            "PDF files are not editable via edit_file/create_file. Use the 'doc-script-editor' skill: run_shell with `python <SKILL_DIR>/scripts/edit_doc.py --path <abs-path> <replace|extract|redact>` to modify, extract text, or redact."
                .to_string(),
        );
    }
    if mime.starts_with("image/") {
        return Some(
            "This file can be inspected with read_file, but it is not editable via edit_file/create_file."
                .to_string(),
        );
    }

    None
}

fn project_document(parsed: crate::parse::ParsedDocument) -> FileEvidence {
    let mut text = String::new();
    let mut passages = Vec::new();
    let mut line = 1;
    for chunk in parsed.chunks {
        let visible = chunk
            .content
            .get(chunk.overlap_start..)
            .unwrap_or(&chunk.content);
        if !text.is_empty() {
            text.push('\n');
            line += 1;
        }
        let start = line;
        line += visible.bytes().filter(|byte| *byte == b'\n').count();
        if !visible.is_empty() {
            passages.push(LocatedPassage {
                start,
                end: line - usize::from(visible.ends_with('\n')),
                locator: chunk.locator,
            });
        }
        text.push_str(visible);
    }
    for artifact in parsed.visual_artifacts {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&artifact.to_chunk_content());
    }
    if text.trim().is_empty() {
        text = format!(
            "[No extractable text found in document: {}]",
            parsed.file_name
        );
        passages.clear();
    }
    FileEvidence {
        text,
        native: None,
        source: ReadSource::Document {
            hash: parsed.content_hash,
            passages,
        },
    }
}

fn read_projection(path: &Path) -> Result<FileEvidence, CoreError> {
    // A valid PDF may contain no NUL bytes. Prefer its parsed pages to serialized PDF operators.
    let mime = crate::parse::detect_mime_type(path);
    if supports_document_fallback(path)
        && (mime == "application/pdf" || crate::parse::file_appears_binary(path)?)
    {
        match crate::parse::parse_file(
            path,
            None,
            #[cfg(feature = "video")]
            None,
            None,
            None,
            None,
        ) {
            Ok(parsed) => return Ok(project_document(parsed)),
            Err(error) if mime != "application/pdf" || crate::parse::file_appears_binary(path)? => {
                return Err(error)
            }
            Err(_) => (), // Preserve the existing text fallback for a mislabeled file.
        }
    }
    crate::parse::read_text_file(path).map(|text| FileEvidence::extracted(text, None))
}

pub(crate) fn read_supported_file_content(path: &Path) -> Result<String, CoreError> {
    read_projection(path).map(|evidence| evidence.text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evidence::ExtractedLineRange;

    #[test]
    fn direct_pdf_citation_clips_source_lines_after_overlap_is_removed() {
        let passage = LocatedPassage {
            start: 5,
            end: 8,
            locator: EvidenceLocator::Pdf {
                page: 2,
                bbox: None,
                extracted_lines: Some(ExtractedLineRange { start: 14, end: 22 }),
            },
        };
        let (locator, _) = passage.window_locator(6, 7, |_| true).unwrap();
        assert_eq!(
            serde_json::to_value(locator).unwrap(),
            serde_json::json!({
                "kind": "pdf", "page": 2, "extractedLineStart": 20, "extractedLineEnd": 21
            })
        );
    }
}
