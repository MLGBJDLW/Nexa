use std::io::Read;
use std::panic::{self, AssertUnwindSafe};

use lopdf::Document;

use super::model::{PreviewBlock, PreviewRun, StructuredPreview};

const PDF_PREVIEW_MAX_PAGES: usize = 80;
const PDF_PREVIEW_MAX_PARAGRAPHS_PER_PAGE: usize = 80;
const PDF_PREVIEW_MAX_PARAGRAPH_CHARS: usize = 4_000;

pub fn preview_pdf(reader: impl Read) -> Result<Option<StructuredPreview>, String> {
    panic::catch_unwind(AssertUnwindSafe(|| preview_pdf_inner(reader)))
        .map_err(|payload| format!("panic: {}", panic_payload_to_string(payload)))?
}

fn preview_pdf_inner(reader: impl Read) -> Result<Option<StructuredPreview>, String> {
    let doc = Document::load_from(reader).map_err(|e| format!("PDF load failed: {e}"))?;
    let pages = doc.get_pages();
    if pages.is_empty() {
        return Ok(None);
    }

    let page_count = pages.len();
    let mut blocks = Vec::new();
    let mut extracted_any_text = false;
    let mut first_error: Option<String> = None;

    for (index, page_number) in pages.keys().take(PDF_PREVIEW_MAX_PAGES).enumerate() {
        if index > 0 {
            blocks.push(PreviewBlock::PageBreak);
        }
        blocks.push(heading(format!("Page {}", index + 1), 2));

        let (text, error) = crate::parse::native_pdf_page_text(&doc, *page_number);
        if !text.trim().is_empty() {
            extracted_any_text = true;
            append_page_paragraphs(&mut blocks, &text, *page_number);
        } else if let Some(err) = error {
            first_error.get_or_insert_with(|| err.clone());
            blocks.push(PreviewBlock::Unsupported {
                message: format!("Could not extract text from this page: {err}"),
            });
        } else {
            blocks.push(PreviewBlock::Unsupported {
                message: "No extractable text layer was found on this page.".to_string(),
            });
        }
    }

    if page_count > PDF_PREVIEW_MAX_PAGES {
        blocks.push(PreviewBlock::Unsupported {
            message: format!(
                "PDF structured preview is limited to the first {} of {} pages.",
                PDF_PREVIEW_MAX_PAGES, page_count
            ),
        });
    }

    if !extracted_any_text {
        if let Some(err) = first_error {
            return Err(format!("Could not extract structured PDF text: {err}"));
        }
        return Ok(None);
    }

    Ok(Some(StructuredPreview::Document {
        blocks,
        assets: Vec::new(),
    }))
}

fn append_page_paragraphs(blocks: &mut Vec<PreviewBlock>, text: &str, page: u32) {
    let mut offset = 0;
    let mut start = None;
    let mut count = 0;
    // A sentinel flushes the last nonempty paragraph without a trailing newline.
    for (index, line) in text
        .split_inclusive('\n')
        .chain(std::iter::once(""))
        .enumerate()
    {
        if !line.trim().is_empty() {
            start.get_or_insert((offset, index as u32 + 1));
        } else if let Some((begin, line_number)) = start.take() {
            if count == PDF_PREVIEW_MAX_PARAGRAPHS_PER_PAGE {
                blocks.push(PreviewBlock::Unsupported {
                    message: format!(
                        "Page {page} preview is limited to the first {count} paragraphs."
                    ),
                });
                break;
            }
            blocks.push(paragraph_block(text[begin..offset].trim(), line_number));
            count += 1;
        }
        offset += line.len();
    }
}

fn heading(text: String, level: u8) -> PreviewBlock {
    PreviewBlock::Heading {
        source_paragraph: None,
        level,
        runs: vec![plain_run(text)],
        alignment: None,
    }
}

fn paragraph_block(text: &str, first_line: u32) -> PreviewBlock {
    let cut = text
        .char_indices()
        .nth(PDF_PREVIEW_MAX_PARAGRAPH_CHARS)
        .map(|(offset, _)| offset);
    let visible = text[..cut.unwrap_or(text.len())].trim_end_matches('\n');
    let last_line = first_line + visible.lines().count() as u32 - 1;
    let mut rendered = visible.to_owned();
    if cut.is_some() {
        rendered.push_str("\n[truncated]");
    }
    PreviewBlock::Paragraph {
        source_paragraph: None,
        extracted_lines: Some(crate::evidence::ExtractedLineRange {
            start: first_line,
            end: last_line,
        }),
        runs: vec![plain_run(rendered)],
        alignment: None,
    }
}

fn plain_run(text: String) -> PreviewRun {
    PreviewRun {
        text,
        ..PreviewRun::default()
    }
}

fn panic_payload_to_string(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::content::{Content, Operation};
    use lopdf::{dictionary, Object, Stream};
    use std::path::Path;

    #[test]
    fn preview_pdf_builds_document_blocks_per_page() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sample.pdf");
        write_test_pdf(
            &path,
            &[
                "Executive Summary\nFirst page details",
                "Second page details",
            ],
        );

        let preview = preview_pdf(std::fs::File::open(&path).unwrap())
            .expect("preview pdf")
            .expect("structured");
        let StructuredPreview::Document { blocks, assets } = preview else {
            panic!("expected document preview");
        };

        assert!(assets.is_empty());
        assert!(matches!(
            &blocks[0],
            PreviewBlock::Heading { runs, .. } if runs[0].text == "Page 1"
        ));
        assert!(blocks.iter().any(|block| matches!(
            block,
            PreviewBlock::Paragraph { runs, extracted_lines: Some(lines), .. }
                if runs[0].text == "Executive Summary\nFirst page details" && (lines.start, lines.end) == (1, 2)
        )));
        assert!(blocks
            .iter()
            .any(|block| matches!(block, PreviewBlock::PageBreak)));
        assert!(blocks.iter().any(|block| matches!(
            block,
            PreviewBlock::Paragraph { runs, extracted_lines: Some(lines), .. }
                if runs[0].text == "Second page details" && (lines.start, lines.end) == (1, 1)
        )));
        let parsed = crate::parse::parse_pdf(&path, &Default::default(), None, 2000).unwrap();
        let first = serde_json::to_value(&parsed.chunks[0].locator).unwrap();
        assert_eq!(
            first,
            serde_json::json!({
                "kind":"pdf", "page":1, "extractedLineStart":1, "extractedLineEnd":2
            })
        );
        assert_eq!(
            serde_json::from_value::<crate::evidence::EvidenceLocator>(first).unwrap(),
            parsed.chunks[0].locator
        );
        let old = serde_json::json!({"kind":"pdf","page":1});
        assert_eq!(
            serde_json::to_value(
                serde_json::from_value::<crate::evidence::EvidenceLocator>(old.clone()).unwrap()
            )
            .unwrap(),
            old
        );
        let shortened = paragraph_block(&"visible\n".repeat(1_000), 4);
        assert!(matches!(shortened,
            PreviewBlock::Paragraph { runs, extracted_lines: Some(lines), .. }
                if runs[0].text.ends_with("\n[truncated]") && (lines.start, lines.end) == (4, 503)));
    }

    fn write_test_pdf(path: &Path, page_texts: &[&str]) {
        let mut doc = Document::with_version("1.5");
        let pages_id = doc.new_object_id();
        let font_id = doc.add_object(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type1",
            "BaseFont" => "Helvetica",
        });
        let resources_id = doc.add_object(dictionary! {
            "Font" => dictionary! {
                "F1" => font_id,
            },
        });

        let mut kids = Vec::new();
        for text in page_texts {
            let mut operations = vec![
                Operation::new("BT", vec![]),
                Operation::new("Tf", vec!["F1".into(), 12.into()]),
                Operation::new("Td", vec![72.into(), 720.into()]),
            ];
            for (index, line) in text.lines().enumerate() {
                if index > 0 {
                    operations.push(Operation::new("T*", vec![]));
                }
                operations.push(Operation::new("Tj", vec![Object::string_literal(line)]));
            }
            operations.push(Operation::new("ET", vec![]));
            let content = Content { operations };
            let content_id = doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));
            let page_id = doc.add_object(dictionary! {
                "Type" => "Page",
                "Parent" => pages_id,
                "Contents" => content_id,
                "Resources" => resources_id,
                "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
            });
            kids.push(page_id.into());
        }

        let pages = dictionary! {
            "Type" => "Pages",
            "Kids" => kids,
            "Count" => page_texts.len() as i64,
        };
        doc.objects.insert(pages_id, Object::Dictionary(pages));
        let catalog_id = doc.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        });
        doc.trailer.set("Root", catalog_id);
        doc.compress();
        doc.save(path).unwrap();
    }
}
