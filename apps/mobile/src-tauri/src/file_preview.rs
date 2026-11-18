//! Read-only phone workspace previews. Canonical containment is shared by data
//! and metadata requests; executable HTML is rendered by the isolated UI host.
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};
use std::io::Cursor;
use std::path::Path;

const INLINE_BYTES_LIMIT: u64 = 8 * 1024 * 1024;
const OFFICE_XML_BYTES_LIMIT: u64 = 32 * 1024 * 1024;
use tokio::io::AsyncReadExt;

fn check_office_xml_size(bytes: &[u8]) -> Result<(), String> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| e.to_string())?;
    let mut total = 0u64;
    for index in 0..archive.len() {
        let part = archive.by_index_raw(index).map_err(|e| e.to_string())?;
        let name = part.name().map_err(|e| e.to_string())?;
        if name.ends_with(".xml") || name.ends_with(".rels") {
            total = total.saturating_add(part.size());
            if total > OFFICE_XML_BYTES_LIMIT {
                return Err("Expanded document text exceeds the phone preview budget.".into());
            }
        }
    }
    Ok(())
}

#[derive(Default, Clone, Copy)]
pub struct ReadOptions<'a> {
    pub data: bool,
    pub verify_hash: bool,
    pub start_line: Option<u32>,
    pub focus_slide: Option<u32>,
    pub expected_hash: Option<&'a str>,
}

pub async fn read(root: &Path, requested: &str, options: ReadOptions<'_>) -> Result<Value, String> {
    let ReadOptions {
        data,
        verify_hash,
        start_line,
        focus_slide,
        expected_hash,
    } = options;
    let root = tokio::fs::canonicalize(root)
        .await
        .map_err(|error| error.to_string())?;
    let path = tokio::fs::canonicalize(root.join(requested))
        .await
        .map_err(|error| error.to_string())?;
    if !path.starts_with(&root) {
        return Err("Choose a file in the phone workspace.".into());
    }
    let file = tokio::fs::File::open(&path)
        .await
        .map_err(|error| error.to_string())?;
    let metadata = file.metadata().await.map_err(|error| error.to_string())?;
    if !metadata.is_file() {
        return Err("Choose a regular file.".into());
    }
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let detected = nexa_core::parse::detect_mime_type(&path);
    let (kind, mime) = match extension.as_str() {
        "png" => ("image", "image/png"),
        "jpg" | "jpeg" => ("image", "image/jpeg"),
        "webp" => ("image", "image/webp"),
        "gif" => ("image", "image/gif"),
        "html" | "htm" => ("code", "text/html"),
        "md" | "markdown" => ("markdown", "text/markdown"),
        "txt" | "csv" | "json" | "log" => ("text", "text/plain"),
        "docx" | "xlsx" | "pptx" | "pdf" => ("document", detected.as_str()),
        _ if start_line.is_some() || detected.starts_with("text/") => ("text", "text/plain"),
        _ => ("binary", "application/octet-stream"),
    };
    let mut first_line = 1;
    let mut hash = String::new();
    let mut truncated = false;
    let mut structured_preview = None;
    let mut warning = None;
    let content = if data || kind == "document" {
        let limit = if data {
            INLINE_BYTES_LIMIT
        } else {
            nexa_core::preview::DOCUMENT_PREVIEW_BYTES_LIMIT
        };
        if metadata.len() > limit {
            return Err("File exceeds the inline preview size; open it from Files.".into());
        }
        let mut bytes = Vec::new();
        file.take(limit + 1)
            .read_to_end(&mut bytes)
            .await
            .map_err(|error| error.to_string())?;
        if bytes.len() as u64 > limit {
            return Err("File grew beyond the inline preview size.".into());
        }
        if data {
            let expected_hash = expected_hash.map(str::to_owned);
            let mime = mime.to_owned();
            return tokio::task::spawn_blocking(move || {
                if expected_hash
                    .is_some_and(|expected| blake3::hash(&bytes).to_hex().as_str() != expected)
                {
                    return Err(
                        "The file changed since its preview was opened. Reopen the citation."
                            .into(),
                    );
                }
                Ok(json!(format!(
                    "data:{mime};base64,{}",
                    STANDARD.encode(bytes)
                )))
            })
            .await
            .map_err(|error| error.to_string())?;
        }

        // Hash and parse the exact same bounded snapshot, even if another
        // writer replaces the source after its handle was opened.
        let format = mime.to_owned();
        let (content_hash, result) = tokio::task::spawn_blocking(move || {
            let content_hash = blake3::hash(&bytes).to_hex().to_string();
            let result = (|| {
                if format != "application/pdf" {
                    check_office_xml_size(&bytes)?;
                }
                nexa_core::preview::build_structured_preview_from_reader(
                    Cursor::new(bytes),
                    &format,
                    &content_hash,
                    &nexa_core::preview::PreviewBuildOptions {
                        focus_slide,
                        ..Default::default()
                    },
                )
            })();
            (content_hash, result)
        })
        .await
        .map_err(|error| error.to_string())?;
        hash = content_hash;
        match result {
            Ok(rendered) => structured_preview = rendered,
            Err(error) => warning = Some(error),
        }
        None
    } else if matches!(kind, "code" | "markdown" | "text") {
        let limit = 512 * 1024;
        if metadata.len() > limit {
            first_line = start_line.unwrap_or(1).max(1);
        }
        let file = file.into_std().await;
        let window = tokio::task::spawn_blocking(move || {
            nexa_core::text_window::read(file, first_line, limit as usize)
        })
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
        hash = window.hash;
        truncated = window.truncated;
        if window.binary {
            return Err("This file contains binary data.".into());
        }
        Some(
            nexa_core::parse::decode_text_bytes(&window.bytes, &path)
                .map_err(|err| err.to_string())?,
        )
    } else {
        if verify_hash {
            let file = file.into_std().await;
            hash = tokio::task::spawn_blocking(move || {
                let mut hasher = blake3::Hasher::new();
                hasher
                    .update_reader(file)
                    .map(|h| h.finalize().to_hex().to_string())
            })
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())?;
        }
        None
    };
    let line_count = content.as_deref().map_or(0, |text| text.lines().count());
    Ok(
        json!({"path":path,"displayName":path.file_name().unwrap().to_string_lossy(),
        "sourceId":null,"sourceName":"Phone","extension":format!(".{extension}"),"mimeType":mime,"kind":kind,
        "content":content,"structuredPreview":structured_preview,"warning":warning,"editable":false,"agentEditAllowed":false,"sizeBytes":metadata.len(),"hash":hash,
        "lineCount":line_count,"firstLine":first_line,"truncated":truncated}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn office_package(parts: &[(&str, &str)]) -> Vec<u8> {
        use std::io::Write;
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, xml) in parts {
            zip.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(xml.as_bytes()).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    fn text_pdf() -> Vec<u8> {
        use std::io::Write;
        let stream = "BT /F1 12 Tf 14 TL 10 60 Td (First) Tj T* (Second) Tj ET";
        let content = format!(
            "<< /Length {} >>\nstream\n{stream}\nendstream",
            stream.len()
        );
        let mut pdf = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for object in [
            "<< /Type /Catalog /Pages 2 0 R >>",
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>",
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
            &content,
        ] {
            offsets.push(pdf.len());
            writeln!(pdf, "{} 0 obj\n{object}\nendobj", offsets.len()).unwrap();
        }
        let xref = pdf.len();
        writeln!(pdf, "xref\n0 6\n0000000000 65535 f ").unwrap();
        for offset in offsets {
            writeln!(pdf, "{offset:010} 00000 n ").unwrap();
        }
        writeln!(
            pdf,
            "trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF"
        )
        .unwrap();
        pdf
    }

    #[tokio::test]
    async fn local_office_citations_open_the_hashed_file_with_source_coordinates() {
        let root =
            std::env::temp_dir().join(format!("nexa-office-preview-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir(&root).await.unwrap();
        let docx = office_package(&[(
            "word/document.xml",
            r#"<w:document xmlns:w="w"><w:body><w:p/><w:p><w:r><w:t>Decision</w:t></w:r></w:p></w:body></w:document>"#,
        )]);
        let xlsx = office_package(&[
            (
                "[Content_Types].xml",
                r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>"#,
            ),
            (
                "_rels/.rels",
                r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#,
            ),
            (
                "xl/_rels/workbook.xml.rels",
                r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#,
            ),
            (
                "xl/workbook.xml",
                r#"<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Budget" sheetId="1" r:id="rId1"/></sheets></workbook>"#,
            ),
            (
                "xl/worksheets/sheet1.xml",
                r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="3"><c r="B3"><f>1+2</f><v>3</v></c></row></sheetData></worksheet>"#,
            ),
        ]);
        let pptx = office_package(&[
            (
                "ppt/slides/slide1.xml",
                r#"<p:sld xmlns:p="p" xmlns:a="a"><a:p><a:r><a:t>Intro</a:t></a:r></a:p></p:sld>"#,
            ),
            (
                "ppt/slides/slide2.xml",
                r#"<p:sld xmlns:p="p" xmlns:a="a"><a:p><a:r><a:t>Decision</a:t></a:r></a:p></p:sld>"#,
            ),
        ]);
        for (name, bytes) in [
            ("agenda.docx", &docx),
            ("plan.xlsx", &xlsx),
            ("plan.pptx", &pptx),
        ] {
            tokio::fs::write(root.join(name), bytes).await.unwrap();
            let preview = read(
                &root,
                name,
                ReadOptions {
                    focus_slide: Some(2),
                    verify_hash: true,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
            assert_eq!(preview["kind"], "document");
            assert_eq!(preview["hash"], blake3::hash(bytes).to_hex().as_str());
            assert!(
                preview["structuredPreview"].is_object(),
                "{name}: {}",
                preview["warning"]
            );
            let structured = &preview["structuredPreview"];
            match name {
                "agenda.docx" => assert!(structured["blocks"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|block| block["sourceParagraph"] == 2
                        && block["runs"][0]["text"] == "Decision")),
                "plan.xlsx" => {
                    let sheet = &structured["sheets"][0];
                    assert_eq!(sheet["name"], "Budget");
                    assert_eq!(sheet["startRow"], 2);
                    assert_eq!(sheet["startColumn"], 1);
                    assert_eq!(sheet["cells"][0]["formula"], "1+2");
                }
                _ => {
                    assert_eq!(structured["blocks"][0]["number"], 2);
                    assert_eq!(structured["blocks"][0]["text"], "Decision");
                }
            }
        }
        let pdf = text_pdf();
        tokio::fs::write(root.join("notes.pdf"), &pdf)
            .await
            .unwrap();
        let preview = read(&root, "notes.pdf", ReadOptions::default())
            .await
            .unwrap();
        assert_eq!(preview["hash"], blake3::hash(&pdf).to_hex().as_str());
        assert!(preview["structuredPreview"]["blocks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|block| block["extractedLineStart"] == 1
                && block["extractedLineEnd"] == 2
                && block["runs"][0]["text"] == "First\nSecond"));
        tokio::fs::write(root.join("agenda.docx"), &pptx)
            .await
            .unwrap();
        let changed = read(&root, "agenda.docx", ReadOptions::default())
            .await
            .unwrap();
        assert_ne!(changed["hash"], blake3::hash(&docx).to_hex().as_str());
        assert!(changed["structuredPreview"].is_null() && changed["warning"].is_string());
        tokio::fs::remove_dir_all(root).await.unwrap();
    }

    #[tokio::test]
    async fn previews_workspace_files_without_reading_outside_or_oversized_files() {
        let details = ReadOptions {
            verify_hash: true,
            ..Default::default()
        };
        let data = ReadOptions {
            data: true,
            ..Default::default()
        };
        let temporary = std::env::temp_dir().join(format!("nexa-preview-{}", uuid::Uuid::new_v4()));
        let root = temporary.join("workspace");
        tokio::fs::create_dir_all(&root).await.unwrap();
        tokio::fs::write(root.join("timeline.html"), "<h1>Timeline</h1>")
            .await
            .unwrap();
        tokio::fs::write(root.join("image.png"), [1, 2, 3])
            .await
            .unwrap();
        tokio::fs::write(temporary.join("private.txt"), "private")
            .await
            .unwrap();
        let preview = read(&root, "timeline.html", details).await.unwrap();
        assert_eq!(preview["content"], "<h1>Timeline</h1>");
        assert_eq!(preview["lineCount"], 1);
        assert_eq!(
            preview["hash"],
            blake3::hash(b"<h1>Timeline</h1>").to_hex().as_str()
        );
        assert_eq!(
            read(&root, "image.png", data).await.unwrap(),
            "data:image/png;base64,AQID"
        );
        let expected_hash = blake3::hash(&[1, 2, 3]).to_hex();
        let verified = ReadOptions {
            expected_hash: Some(expected_hash.as_str()),
            ..data
        };
        assert_eq!(
            read(&root, "image.png", verified).await.unwrap(),
            "data:image/png;base64,AQID"
        );
        tokio::fs::write(root.join("image.png"), [4, 5, 6])
            .await
            .unwrap();
        assert!(read(&root, "image.png", verified)
            .await
            .unwrap_err()
            .contains("changed"));
        assert!(read(&root, "../private.txt", data).await.is_err());
        assert!(
            read(&root, temporary.join("private.txt").to_str().unwrap(), data)
                .await
                .is_err()
        );
        assert!(read(&root, ".", data).await.is_err());
        let log = "line\n".repeat(110_000) + "last line";
        tokio::fs::write(root.join("large.log"), log.as_bytes())
            .await
            .unwrap();
        let tail = read(
            &root,
            "large.log",
            ReadOptions {
                start_line: Some(110_001),
                ..details
            },
        )
        .await
        .unwrap();
        assert_eq!(tail["content"], "last line");
        assert_eq!(tail["firstLine"], 110_001);
        assert_eq!(tail["hash"], blake3::hash(log.as_bytes()).to_hex().as_str());
        let large = tokio::fs::File::create(root.join("large.png"))
            .await
            .unwrap();
        large.set_len(8 * 1024 * 1024 + 1).await.unwrap();
        assert!(read(&root, "large.png", data).await.is_err());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(temporary.join("private.txt"), root.join("link.txt"))
                .unwrap();
            assert!(read(&root, "link.txt", data).await.is_err());
        }
        tokio::fs::remove_dir_all(temporary).await.unwrap();
    }
}
