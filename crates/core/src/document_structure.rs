//! Native Office text blocks with stable paragraph, table-row, slide and cell
//! anchors. Extraction preserves formula text and cached values separately.
use crate::{
    error::CoreError,
    evidence::EvidenceLocator,
    parse::{chunk_plaintext, ParsedChunk},
};
use quick_xml::{
    events::{BytesStart, Event},
    Reader, XmlVersion,
};
use std::{
    collections::BTreeMap,
    io::{Cursor, Read},
};

const MAX_XML_BYTES: u64 = 64 * 1024 * 1024;

pub(crate) fn office_package_error(bytes: &[u8], error: impl std::fmt::Display) -> CoreError {
    let recovery = if bytes.starts_with(&[0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1]) {
        "This is an encrypted Office package or an older binary document. Open it in Office, remove password protection if appropriate, and save a new unencrypted copy in the matching format."
    } else {
        "The file is incomplete, damaged, or does not match its extension. Finish copying/downloading it, or open it in Office and save a new copy before rescanning."
    };
    CoreError::Parse(format!(
        "Invalid Office document package. {recovery} Detail: {error}"
    ))
}

fn xml_part(archive: &mut zip::ZipArchive<Cursor<&[u8]>>, name: &str) -> Result<String, CoreError> {
    let mut file = archive
        .by_name(name)
        .map_err(|error| CoreError::Parse(format!("Missing Office part {name}: {error}")))?;
    if file.size() > MAX_XML_BYTES {
        return Err(CoreError::Parse(format!(
            "Office part exceeds 64 MiB: {name}"
        )));
    }
    let mut xml = String::new();
    file.read_to_string(&mut xml)?;
    Ok(xml)
}

fn attribute(event: &BytesStart<'_>, name: &[u8], reader: &Reader<&[u8]>) -> Option<String> {
    event
        .attributes()
        .flatten()
        .find(|attribute| attribute.key.local_name().as_ref() == name)
        .and_then(|attribute| {
            attribute
                .decoded_and_normalized_value(XmlVersion::Implicit1_0, reader.decoder())
                .ok()
                .map(|value| value.into_owned())
        })
}

fn append_block(
    chunks: &mut Vec<ParsedChunk>,
    text: &str,
    heading: Option<String>,
    locator: EvidenceLocator,
    max_chars: usize,
) {
    for mut chunk in chunk_plaintext(text, max_chars) {
        chunk.chunk_index = chunks.len() as i32;
        chunk.heading_context = heading.clone();
        chunk.locator = locator.clone();
        chunks.push(chunk);
    }
}

#[derive(Default)]
struct WordTable {
    id: u32,
    row: u32,
    paragraph: u32,
    cells: Vec<String>,
    rows: Vec<(u32, u32, Vec<String>)>,
}

pub(crate) fn docx_chunks(bytes: &[u8], max_chars: usize) -> Result<Vec<ParsedChunk>, CoreError> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|error| office_package_error(bytes, error))?;
    let mut parts = archive
        .file_names()
        .filter(|name| {
            name.ends_with(".xml")
                && (*name == "word/document.xml"
                    || name.starts_with("word/header")
                    || name.starts_with("word/footer")
                    || *name == "word/footnotes.xml"
                    || *name == "word/endnotes.xml")
        })
        .map(str::to_string)
        .collect::<Vec<_>>();
    if !parts.iter().any(|part| part == "word/document.xml") {
        return Err(CoreError::Parse("DOCX has no word/document.xml".into()));
    }
    parts.sort_by_key(|part| (part != "word/document.xml", part.clone()));
    let mut chunks = Vec::new();
    for part in parts {
        let xml = xml_part(&mut archive, &part)?;
        let mut reader = Reader::from_str(&xml);
        let mut paragraph = 0;
        let mut paragraph_text = String::new();
        let mut paragraph_style = None;
        let mut heading = None;
        let mut table_count = 0;
        let mut tables: Vec<WordTable> = Vec::new();
        loop {
            match reader.read_event() {
                Ok(Event::Start(event)) => match event.name().as_ref() {
                    b"w:tbl" => {
                        table_count += 1;
                        tables.push(WordTable {
                            id: table_count,
                            ..WordTable::default()
                        });
                    }
                    b"w:tr" => {
                        if let Some(table) = tables.last_mut() {
                            table.row += 1;
                            table.cells.clear();
                            table.paragraph = paragraph + 1;
                        }
                    }
                    b"w:tc" => {
                        if let Some(table) = tables.last_mut() {
                            table.cells.push(String::new());
                        }
                    }
                    b"w:p" => {
                        paragraph += 1;
                        paragraph_text.clear();
                        paragraph_style = None;
                    }
                    b"w:t" => {
                        let raw = reader
                            .read_text(event.name())
                            .map_err(|error| CoreError::Parse(error.to_string()))?;
                        let raw = raw
                            .decode()
                            .map_err(|error| CoreError::Parse(error.to_string()))?;
                        let text = quick_xml::escape::unescape(&raw)
                            .map_err(|error| CoreError::Parse(error.to_string()))?;
                        paragraph_text.push_str(&text);
                    }
                    _ => (),
                },
                Ok(Event::Empty(event)) => match event.name().as_ref() {
                    b"w:p" => paragraph += 1,
                    b"w:tbl" => table_count += 1,
                    b"w:tr" => {
                        if let Some(table) = tables.last_mut() {
                            table.row += 1;
                            table.rows.push((table.row, paragraph + 1, Vec::new()));
                        }
                    }
                    b"w:tc" => {
                        if let Some(table) = tables.last_mut() {
                            table.cells.push(String::new());
                        }
                    }
                    b"w:pStyle" => paragraph_style = attribute(&event, b"val", &reader),
                    b"w:tab" => paragraph_text.push('\t'),
                    b"w:br" | b"w:cr" => paragraph_text.push('\n'),
                    _ => (),
                },
                Ok(Event::End(event)) => match event.name().as_ref() {
                    b"w:p" => {
                        if paragraph_style.as_deref().is_some_and(|style| {
                            style.to_ascii_lowercase().starts_with("heading")
                                || style.starts_with("标题")
                        }) {
                            heading = Some(paragraph_text.clone());
                        }
                        if let Some(table) = tables.last_mut() {
                            if let Some(cell) = table.cells.last_mut() {
                                if !cell.is_empty() {
                                    cell.push('\n');
                                }
                                cell.push_str(&paragraph_text);
                            }
                        } else {
                            append_block(
                                &mut chunks,
                                &paragraph_text,
                                heading.clone(),
                                EvidenceLocator::Document {
                                    part: part.clone(),
                                    paragraph,
                                    table: None,
                                    row: None,
                                    context_row: None,
                                    column: None,
                                },
                                max_chars,
                            );
                        }
                    }
                    b"w:tr" => {
                        if let Some(table) = tables.last_mut() {
                            table.rows.push((
                                table.row,
                                table.paragraph,
                                std::mem::take(&mut table.cells),
                            ));
                        }
                    }
                    b"w:tbl" => {
                        if let Some(table) = tables.pop() {
                            let header = table
                                .rows
                                .first()
                                .map(|(_, _, cells)| cells.join(" | "))
                                .unwrap_or_default();
                            for (row, paragraph, cells) in table.rows {
                                let text = format!(
                                    "Table {} · Row {row}\n{}{}",
                                    table.id,
                                    if row > 1 {
                                        format!("First row: {header}\n")
                                    } else {
                                        String::new()
                                    },
                                    cells.join(" | ")
                                );
                                append_block(
                                    &mut chunks,
                                    &text,
                                    heading.clone(),
                                    EvidenceLocator::Document {
                                        part: part.clone(),
                                        paragraph,
                                        table: Some(table.id),
                                        row: Some(row),
                                        context_row: (row > 1 && !header.is_empty()).then_some(1),
                                        column: None,
                                    },
                                    max_chars,
                                );
                            }
                        }
                    }
                    _ => (),
                },
                Ok(Event::Eof) => break,
                Err(error) => return Err(CoreError::Parse(format!("Invalid {part}: {error}"))),
                _ => (),
            }
        }
    }
    Ok(chunks)
}

fn relationships(xml: &str) -> BTreeMap<String, String> {
    let mut reader = Reader::from_str(xml);
    let mut result = BTreeMap::new();
    loop {
        match reader.read_event() {
            Ok(Event::Start(event) | Event::Empty(event))
                if event.local_name().as_ref() == b"Relationship" =>
            {
                if attribute(&event, b"TargetMode", &reader).as_deref() == Some("External") {
                    continue;
                }
                if let (Some(id), Some(target)) = (
                    attribute(&event, b"Id", &reader),
                    attribute(&event, b"Target", &reader),
                ) {
                    result.insert(id, target);
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => (),
        }
    }
    result
}

fn slide_text(xml: &str) -> Result<String, CoreError> {
    let mut reader = Reader::from_str(xml);
    let mut text = String::new();
    loop {
        match reader.read_event() {
            Ok(Event::Start(event)) if event.local_name().as_ref() == b"t" => {
                let raw = reader
                    .read_text(event.name())
                    .map_err(|error| CoreError::Parse(error.to_string()))?;
                let raw = raw
                    .decode()
                    .map_err(|error| CoreError::Parse(error.to_string()))?;
                text.push_str(
                    &quick_xml::escape::unescape(&raw)
                        .map_err(|error| CoreError::Parse(error.to_string()))?,
                );
            }
            Ok(Event::End(event)) if event.local_name().as_ref() == b"p" => text.push('\n'),
            Ok(Event::Empty(event)) if event.local_name().as_ref() == b"br" => text.push('\n'),
            Ok(Event::Eof) => break,
            Err(error) => return Err(CoreError::Parse(error.to_string())),
            _ => (),
        }
    }
    Ok(text)
}

pub(crate) fn pptx_chunks(bytes: &[u8], max_chars: usize) -> Result<Vec<ParsedChunk>, CoreError> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|error| office_package_error(bytes, error))?;
    let mut slides = Vec::new();
    if let (Ok(presentation), Ok(rels)) = (
        xml_part(&mut archive, "ppt/presentation.xml"),
        xml_part(&mut archive, "ppt/_rels/presentation.xml.rels"),
    ) {
        let relationships = relationships(&rels);
        let mut reader = Reader::from_str(&presentation);
        loop {
            match reader.read_event() {
                Ok(Event::Start(event) | Event::Empty(event))
                    if event.local_name().as_ref() == b"sldId" =>
                {
                    let id = event
                        .attributes()
                        .flatten()
                        .find(|attribute| attribute.key.as_ref() == b"r:id")
                        .and_then(|attribute| String::from_utf8(attribute.value.into_owned()).ok());
                    if let Some(target) = id.and_then(|id| relationships.get(&id)) {
                        slides.push(if target.starts_with('/') {
                            target.trim_start_matches('/').into()
                        } else {
                            format!("ppt/{target}")
                        });
                    }
                }
                Ok(Event::Eof) => break,
                Err(error) => return Err(CoreError::Parse(error.to_string())),
                _ => (),
            }
        }
    }
    if slides.is_empty() {
        slides = archive
            .file_names()
            .filter(|name| name.starts_with("ppt/slides/slide") && name.ends_with(".xml"))
            .map(str::to_string)
            .collect();
        slides.sort_by_key(|name| {
            name.strip_prefix("ppt/slides/slide")
                .and_then(|name| name.strip_suffix(".xml"))
                .and_then(|index| index.parse::<u32>().ok())
                .unwrap_or(u32::MAX)
        });
    }
    let mut chunks = Vec::new();
    for (index, part) in slides.into_iter().enumerate() {
        let slide = index as u32 + 1;
        append_block(
            &mut chunks,
            &slide_text(&xml_part(&mut archive, &part)?)?,
            Some(format!("Slide {slide}")),
            EvidenceLocator::Slide { slide },
            max_chars,
        );
        let basename = part.rsplit('/').next().unwrap_or_default();
        if let Ok(rels) = xml_part(&mut archive, &format!("ppt/slides/_rels/{basename}.rels")) {
            for target in relationships(&rels)
                .values()
                .filter(|target| target.contains("notesSlides/"))
            {
                let notes = format!("ppt/{}", target.trim_start_matches("../"));
                if let Ok(xml) = xml_part(&mut archive, &notes) {
                    append_block(
                        &mut chunks,
                        &slide_text(&xml)?,
                        Some(format!("Slide {slide} · Notes")),
                        EvidenceLocator::Slide { slide },
                        max_chars,
                    );
                }
            }
        }
    }
    Ok(chunks)
}

#[cfg(feature = "document-processing")]
fn cell_address(row: u32, column: u32) -> String {
    let mut column = column + 1;
    let mut letters = Vec::new();
    while column > 0 {
        column -= 1;
        letters.push((b'A' + (column % 26) as u8) as char);
        column /= 26;
    }
    format!(
        "{}{}",
        letters.into_iter().rev().collect::<String>(),
        row + 1
    )
}

#[cfg(feature = "document-processing")]
pub(crate) fn workbook_chunks(
    path: &std::path::Path,
    max_chars: usize,
) -> Result<Vec<ParsedChunk>, CoreError> {
    use calamine::{open_workbook_auto, Reader as WorkbookReader};
    let mut workbook = open_workbook_auto(path)
        .map_err(|error| CoreError::Parse(format!("Excel open failed: {error}")))?;
    let mut chunks = Vec::new();
    for name in workbook.sheet_names().to_vec() {
        let values = workbook
            .worksheet_range(&name)
            .map_err(|error| CoreError::Parse(format!("Sheet {name}: {error}")))?;
        let formulas = workbook
            .worksheet_formula(&name)
            .map_err(|error| CoreError::Parse(format!("Formulas in {name}: {error}")))?;
        let mut cells: BTreeMap<(u32, u32), (String, Option<String>)> = BTreeMap::new();
        let (row_start, column_start) = values.start().unwrap_or_default();
        for (row, column, value) in values.used_cells() {
            cells.insert(
                (row_start + row as u32, column_start + column as u32),
                (value.to_string(), None),
            );
        }
        let (row_start, column_start) = formulas.start().unwrap_or_default();
        for (row, column, formula) in formulas.used_cells() {
            cells
                .entry((row_start + row as u32, column_start + column as u32))
                .or_default()
                .1 = Some(formula.clone());
        }
        let mut rows: BTreeMap<u32, Vec<(u32, String)>> = BTreeMap::new();
        for ((row, column), (value, formula)) in cells {
            let address = cell_address(row, column);
            let text = if let Some(formula) = formula {
                format!(
                    "{address}: formula ={}; cached value = {value}",
                    formula.trim_start_matches('=')
                )
            } else {
                format!("{address}: {value}")
            };
            rows.entry(row).or_default().push((column, text));
        }
        let context = rows
            .values()
            .take(1)
            .flat_map(|cells| cells.iter().map(|(_, text)| text.as_str()))
            .collect::<Vec<_>>()
            .join(" | ");
        let context = context.chars().take(max_chars / 3).collect::<String>();
        let context_source = rows.first_key_value().map(|(row, cells)| {
            (
                *row,
                format!(
                    "{}:{}",
                    cell_address(*row, cells.first().unwrap().0),
                    cell_address(*row, cells.last().unwrap().0)
                ),
            )
        });
        for (row, cells) in rows {
            let range = format!(
                "{}:{}",
                cell_address(row, cells.first().unwrap().0),
                cell_address(row, cells.last().unwrap().0)
            );
            let prefix = format!("Sheet: {name}\nFirst row context: {context}\n");
            let text = cells
                .iter()
                .map(|(_, text)| text.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            for mut chunk in chunk_plaintext(
                &text,
                max_chars.saturating_sub(prefix.chars().count()).max(32),
            ) {
                chunk.chunk_index = chunks.len() as i32;
                chunk.locator = EvidenceLocator::Sheet {
                    sheet: name.clone(),
                    range: range.clone(),
                    context_range: context_source
                        .as_ref()
                        .filter(|(context_row, _)| *context_row != row && !context.is_empty())
                        .map(|(_, range)| range.clone()),
                };
                chunk.heading_context = Some(format!("{name}!{range}"));
                chunk.content.insert_str(0, &prefix);
                chunk.overlap_start += prefix.len();
                chunk.extraction_method = "native_cached_values".into();
                chunks.push(chunk);
            }
        }
    }
    Ok(chunks)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn package(parts: &[(&str, &str)]) -> Vec<u8> {
        let mut archive = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, text) in parts {
            archive
                .start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            archive.write_all(text.as_bytes()).unwrap();
        }
        archive.finish().unwrap().into_inner()
    }

    #[test]
    fn word_native_ordinals_count_self_closing_paragraphs_tables_and_rows() {
        let bytes = package(&[(
            "word/document.xml",
            r#"<w:document xmlns:w="word"><w:body>
          <w:p/><w:p><w:r><w:t>ANCHOR</w:t></w:r></w:p><w:tbl/>
          <w:tbl><w:tr/><w:tr><w:tc/><w:tc><w:p><w:r><w:t>VALUE</w:t></w:r></w:p></w:tc></w:tr></w:tbl>
          </w:body></w:document>"#,
        )]);
        let chunks = docx_chunks(&bytes, 2000).unwrap();
        assert!(chunks.iter().any(|chunk| chunk.content == "ANCHOR"
            && matches!(
                chunk.locator,
                EvidenceLocator::Document {
                    paragraph: 2,
                    table: None,
                    ..
                }
            )));
        assert!(chunks.iter().any(|chunk| chunk.content.contains("VALUE")
            && matches!(
                chunk.locator,
                EvidenceLocator::Document {
                    paragraph: 3,
                    table: Some(2),
                    row: Some(2),
                    ..
                }
            )));
    }

    #[test]
    fn word_keeps_short_paragraphs_table_context_and_native_anchors() {
        let bytes = package(&[(
            "word/document.xml",
            r#"<w:document xmlns:w="word"><w:body>
          <w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>预算 Budget</w:t></w:r></w:p>
          <w:p><w:r><w:t>批准。Approved &amp; signed.</w:t></w:r></w:p>
          <w:tbl><w:tr><w:tc><w:p><w:r><w:t>项目</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>金额(元)</w:t></w:r></w:p></w:tc></w:tr>
          <w:tr><w:tc><w:p><w:r><w:t>差旅</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>500</w:t></w:r></w:p></w:tc></w:tr></w:tbl>
          </w:body></w:document>"#,
        )]);
        let chunks = docx_chunks(&bytes, 2000).unwrap();
        assert!(chunks
            .iter()
            .any(|chunk| chunk.content.contains("Approved & signed.")
                && matches!(
                    &chunk.locator,
                    EvidenceLocator::Document {
                        paragraph: 2,
                        table: None,
                        ..
                    }
                )));
        assert!(chunks.iter().any(|chunk| chunk.content.contains("500")
            && chunk.content.contains("金额(元)")
            && matches!(
                &chunk.locator,
                EvidenceLocator::Document {
                    table: Some(1),
                    row: Some(2),
                    context_row: Some(1),
                    ..
                }
            )));
    }

    #[test]
    fn slides_follow_presentation_order_and_keep_notes() {
        let bytes = package(&[
            (
                "ppt/presentation.xml",
                r#"<p:presentation xmlns:p="p" xmlns:r="r"><p:sldIdLst><p:sldId r:id="r2"/><p:sldId r:id="r1"/></p:sldIdLst></p:presentation>"#,
            ),
            (
                "ppt/_rels/presentation.xml.rels",
                r#"<Relationships><Relationship Id="r1" Target="slides/slide1.xml"/><Relationship Id="r2" Target="slides/slide2.xml"/></Relationships>"#,
            ),
            (
                "ppt/slides/slide1.xml",
                r#"<a:p xmlns:a="a"><a:r><a:t>Second result</a:t></a:r></a:p>"#,
            ),
            (
                "ppt/slides/slide2.xml",
                r#"<a:p xmlns:a="a"><a:r><a:t>第一张</a:t></a:r></a:p>"#,
            ),
            (
                "ppt/slides/_rels/slide2.xml.rels",
                r#"<Relationships><Relationship Id="notes" Target="../notesSlides/notesSlide1.xml"/></Relationships>"#,
            ),
            (
                "ppt/notesSlides/notesSlide1.xml",
                r#"<a:p xmlns:a="a"><a:r><a:t>Speaker evidence</a:t></a:r></a:p>"#,
            ),
        ]);
        let chunks = pptx_chunks(&bytes, 2000).unwrap();
        assert_eq!(chunks[0].content, "第一张");
        assert_eq!(chunks[0].locator, EvidenceLocator::Slide { slide: 1 });
        assert!(chunks
            .iter()
            .any(|chunk| chunk.content == "Speaker evidence"
                && chunk.locator == EvidenceLocator::Slide { slide: 1 }));
        assert!(chunks.iter().any(|chunk| chunk.content == "Second result"
            && chunk.locator == EvidenceLocator::Slide { slide: 2 }));
    }

    #[cfg(feature = "document-processing")]
    #[test]
    fn workbook_preserves_real_cells_formulas_and_cached_values() {
        let bytes = package(&[
            (
                "[Content_Types].xml",
                r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/></Types>"#,
            ),
            (
                "_rels/.rels",
                r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#,
            ),
            (
                "xl/workbook.xml",
                r#"<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="预算" sheetId="1" r:id="rId1"/></sheets></workbook>"#,
            ),
            (
                "xl/_rels/workbook.xml.rels",
                r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#,
            ),
            (
                "xl/worksheets/sheet1.xml",
                r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><dimension ref="B3:C4"/><sheetData><row r="3"><c r="B3" t="inlineStr"><is><t>金额(元)</t></is></c><c r="C3"><v>500</v></c></row><row r="4"><c r="B4" t="inlineStr"><is><t>Total</t></is></c><c r="C4"><f>SUM(C3)</f><v>17</v></c></row></sheetData></worksheet>"#,
            ),
        ]);
        let file = tempfile::Builder::new().suffix(".xlsx").tempfile().unwrap();
        std::fs::write(file.path(), bytes).unwrap();
        let preview = crate::preview::build_structured_preview(
            file.path(),
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            "fixture",
            &crate::preview::PreviewBuildOptions::default(),
        )
        .unwrap()
        .unwrap();
        if let crate::preview::StructuredPreview::Workbook { sheets, .. } = preview {
            assert_eq!(sheets[0].start_row, 2);
            assert_eq!(sheets[0].start_column, 1);
        } else {
            panic!("expected workbook preview");
        }
        let chunks = workbook_chunks(file.path(), 2000).unwrap();
        let formula = chunks
            .iter()
            .find(|chunk| chunk.content.contains("formula =SUM(C3)"))
            .unwrap();
        assert!(formula.content.contains("cached value = 17"));
        assert!(formula.content.contains("金额(元)"));
        assert_eq!(
            formula.locator,
            EvidenceLocator::Sheet {
                sheet: "预算".into(),
                range: "B4:C4".into(),
                context_range: Some("B3:C3".into())
            }
        );
    }
}
