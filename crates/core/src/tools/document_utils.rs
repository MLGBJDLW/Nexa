use std::path::Path;

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

/// Read already-authorized bytes once. Extraction and native evidence use the
/// same snapshot even when the source file is subsequently changed.
pub(crate) fn read_file_evidence(
    path: &Path,
    native_candidate: bool,
) -> Result<(String, Option<NativeFileEvidence>), CoreError> {
    use crate::llm::document::{DocumentInput, MAX_DOCUMENT_BYTES};
    use std::io::{Read, Write};
    let mime = crate::parse::detect_mime_type(path);
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
        return Ok((text, Some(NativeFileEvidence::Image(attachment, length))));
    }
    if !native_candidate || !crate::llm::document::supported_mime(&mime) {
        return read_supported_file_content(path).map(|mut text| {
            if crate::media::is_supported_image(&mime) {
                text.push_str("\n[Pixels were not included in this text-only read. For visual inspection, use read_file with image_mode=\"native\" and no line parameters; raw-image consent is required. Use extract_image_text only for text extraction.]");
            }
            (text, None)
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
    if DocumentInput::from_bytes(name, &mime, &bytes, "").is_none() {
        return read_supported_file_content(path).map(|text| (text, None));
    }
    let suffix = path.extension().and_then(|v| v.to_str()).unwrap_or("bin");
    let mut snapshot = tempfile::Builder::new()
        .prefix("nexa-document-")
        .suffix(&format!(".{suffix}"))
        .tempfile()?;
    snapshot.write_all(&bytes)?;
    let text = read_supported_file_content(snapshot.path()).unwrap_or_else(|error| {
        format!("[Local extraction failed: {error}. The original file may still be read by a supported native document route.]")
    });
    let candidate = DocumentInput::from_bytes(name, &mime, &bytes, &text);
    Ok((text, candidate.map(NativeFileEvidence::Document)))
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

pub(crate) fn is_binary_file_error(err: &CoreError) -> bool {
    matches!(err, CoreError::Parse(msg) if msg.starts_with("File appears to be binary:"))
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

pub(crate) fn flatten_parsed_document_text(parsed: &crate::parse::ParsedDocument) -> String {
    let mut out = String::new();
    for chunk in &parsed.chunks {
        let visible = chunk
            .content
            .get(chunk.overlap_start..)
            .unwrap_or(chunk.content.as_str());
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(visible);
    }
    for artifact in &parsed.visual_artifacts {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&artifact.to_chunk_content());
    }

    if out.trim().is_empty() {
        format!(
            "[No extractable text found in document: {}]",
            parsed.file_name
        )
    } else {
        out
    }
}

pub(crate) fn read_supported_file_content(path: &Path) -> Result<String, CoreError> {
    match crate::parse::read_text_file(path) {
        Ok(raw) => Ok(raw),
        Err(err) if is_binary_file_error(&err) && supports_document_fallback(path) => {
            let parsed = crate::parse::parse_file(
                path,
                None,
                #[cfg(feature = "video")]
                None,
                None,
                None,
                None,
            )?;
            Ok(flatten_parsed_document_text(&parsed))
        }
        Err(err) => Err(err),
    }
}
