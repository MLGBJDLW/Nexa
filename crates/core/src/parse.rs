//! Parser module — extracts structure and content from documents.
//!
//! Reads local files, detects type, computes a blake3 content hash,
//! and splits the content into heading-aware (Markdown) or
//! paragraph-aware (plain text / log) chunks.

use std::path::Path;

use crate::error::CoreError;

// ---------------------------------------------------------------------------
// Public result types
// ---------------------------------------------------------------------------

/// The result of parsing a single file into indexable chunks.
#[derive(Debug, Clone)]
pub struct ParsedDocument {
    pub file_path: String,
    pub file_name: String,
    pub mime_type: String,
    pub file_size: i64,
    pub content_hash: String,
    pub chunks: Vec<ParsedChunk>,
}

/// A single chunk extracted from a document.
#[derive(Debug, Clone)]
pub struct ParsedChunk {
    pub content: String,
    pub chunk_index: i32,
    pub start_offset: i64,
    pub end_offset: i64,
    pub heading_context: Option<String>,
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Chunks larger than this (in chars) will be sub-split.
const MAX_CHUNK_CHARS: usize = 2000;

/// Chunks smaller than this are discarded.
const MIN_CHUNK_CHARS: usize = 50;

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Parse a file at `path` into a [`ParsedDocument`].
///
/// Reads the file, detects the MIME type from its extension, computes a
/// blake3 content hash, and splits the content into chunks using the
/// appropriate strategy (markdown-aware or plain-text).
pub fn parse_file(path: &Path) -> Result<ParsedDocument, CoreError> {
    let mime_type = detect_mime_type(path);

    // PDF files are binary — use a dedicated extractor.
    if mime_type == "application/pdf" {
        return parse_pdf(path);
    }

    let content = std::fs::read_to_string(path)?;
    let metadata = std::fs::metadata(path)?;

    let file_path = path.to_string_lossy().to_string();
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let file_size = metadata.len() as i64;
    let content_hash = blake3::hash(content.as_bytes()).to_hex().to_string();

    let chunks = match mime_type.as_str() {
        "text/markdown" => chunk_markdown(&content),
        _ => chunk_plaintext(&content),
    };

    Ok(ParsedDocument {
        file_path,
        file_name,
        mime_type,
        file_size,
        content_hash,
        chunks,
    })
}

/// Parse a PDF file by extracting its text content.
///
/// Reads the raw bytes, extracts text with `pdf_extract`, computes a blake3
/// hash over the original bytes, then chunks the extracted text using the
/// plain-text strategy.
pub fn parse_pdf(path: &Path) -> Result<ParsedDocument, CoreError> {
    let bytes = std::fs::read(path)?;
    let file_size = bytes.len() as i64;
    let content_hash = blake3::hash(&bytes).to_hex().to_string();

    let text = pdf_extract::extract_text_from_mem(&bytes)
        .map_err(|e| CoreError::Parse(format!("PDF extraction failed for {}: {}", path.display(), e)))?;

    // Normalize: replace \r\n with \n, collapse excessive blank lines.
    let text = text.replace("\r\n", "\n");

    let chunks = chunk_plaintext(&text);

    Ok(ParsedDocument {
        file_path: path.to_string_lossy().to_string(),
        file_name: path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default(),
        mime_type: "application/pdf".to_string(),
        file_size,
        content_hash,
        chunks,
    })
}

/// Detect MIME type from file extension.
pub fn detect_mime_type(path: &Path) -> String {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("md" | "markdown") => "text/markdown".to_string(),
        Some("txt") => "text/plain".to_string(),
        Some("log") => "text/x-log".to_string(),
        Some("pdf") => "application/pdf".to_string(),
        _ => "application/octet-stream".to_string(),
    }
}

// ---------------------------------------------------------------------------
// Markdown chunker
// ---------------------------------------------------------------------------

/// Split markdown content by headings, then by paragraphs if a section is
/// too large. Each chunk records the heading it falls under.
pub fn chunk_markdown(content: &str) -> Vec<ParsedChunk> {
    // Collect (heading, section_text, byte_start) tuples.
    let mut sections: Vec<(Option<String>, String, usize)> = Vec::new();
    let mut current_heading: Option<String> = None;
    let mut current_text = String::new();
    let mut section_start: usize = 0;

    for line in content.lines() {
        if let Some(heading) = parse_heading(line) {
            // Flush previous section.
            if !current_text.is_empty() {
                sections.push((current_heading.clone(), current_text.clone(), section_start));
            }
            current_heading = Some(heading);
            current_text.clear();
            // The new section starts at the current byte offset.
            section_start = byte_offset_of_line(content, line);
        } else {
            if current_text.is_empty() && sections.is_empty() && current_heading.is_none() {
                section_start = byte_offset_of_line(content, line);
            }
            if !current_text.is_empty() {
                current_text.push('\n');
            }
            current_text.push_str(line);
        }
    }
    // Flush last section.
    if !current_text.is_empty() {
        sections.push((current_heading, current_text, section_start));
    }

    // Convert sections into chunks, splitting large ones by paragraph.
    let mut chunks = Vec::new();
    for (heading, text, start) in sections {
        let trimmed = text.trim();
        if trimmed.len() < MIN_CHUNK_CHARS {
            continue;
        }

        if trimmed.len() <= MAX_CHUNK_CHARS {
            let end = start + text.len();
            chunks.push(make_chunk(
                trimmed.to_string(),
                0, // index assigned later
                start as i64,
                end as i64,
                heading.clone(),
            ));
        } else {
            // Sub-split by paragraphs (double newline).
            let parts = split_by_paragraphs(trimmed, MAX_CHUNK_CHARS);
            let mut offset = start;
            for part in parts {
                let len = part.len();
                if len < MIN_CHUNK_CHARS {
                    offset += len;
                    continue;
                }
                chunks.push(make_chunk(
                    part.clone(),
                    0,
                    offset as i64,
                    (offset + len) as i64,
                    heading.clone(),
                ));
                offset += len;
            }
        }
    }

    // Assign sequential chunk indices.
    for (i, chunk) in chunks.iter_mut().enumerate() {
        chunk.chunk_index = i as i32;
    }

    chunks
}

// ---------------------------------------------------------------------------
// Plain-text / log chunker
// ---------------------------------------------------------------------------

/// Split plain text by double newlines (paragraphs). Large paragraphs are
/// further split by single newlines.
pub fn chunk_plaintext(content: &str) -> Vec<ParsedChunk> {
    let paragraphs = split_by_paragraphs(content, MAX_CHUNK_CHARS);

    let mut chunks = Vec::new();
    let mut offset: usize = 0;

    for para in &paragraphs {
        let trimmed = para.trim();
        let len = para.len();

        if trimmed.len() < MIN_CHUNK_CHARS {
            offset += len;
            continue;
        }

        if trimmed.len() <= MAX_CHUNK_CHARS {
            chunks.push(make_chunk(
                trimmed.to_string(),
                0,
                offset as i64,
                (offset + len) as i64,
                None,
            ));
        } else {
            // Sub-split by single newlines.
            let sub_parts = split_by_lines(trimmed, MAX_CHUNK_CHARS);
            let mut sub_offset = offset;
            for part in sub_parts {
                let plen = part.len();
                if part.trim().len() < MIN_CHUNK_CHARS {
                    sub_offset += plen;
                    continue;
                }
                chunks.push(make_chunk(
                    part.trim().to_string(),
                    0,
                    sub_offset as i64,
                    (sub_offset + plen) as i64,
                    None,
                ));
                sub_offset += plen;
            }
        }

        offset += len;
    }

    for (i, chunk) in chunks.iter_mut().enumerate() {
        chunk.chunk_index = i as i32;
    }

    chunks
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Detect a Markdown heading line and return the heading text (without `#` prefix).
fn parse_heading(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    if trimmed.starts_with('#') {
        let hashes = trimmed.chars().take_while(|&c| c == '#').count();
        if hashes <= 6 {
            let rest = trimmed[hashes..].trim();
            if !rest.is_empty() || hashes <= 6 {
                return Some(rest.to_string());
            }
        }
    }
    None
}

/// Return the byte offset of `line_ptr` within `content`.
///
/// Both `content` and `line_ptr` come from `str::lines()`, so `line_ptr` is a
/// sub-slice of `content`.
fn byte_offset_of_line(content: &str, line: &str) -> usize {
    let content_start = content.as_ptr() as usize;
    let line_start = line.as_ptr() as usize;
    line_start.saturating_sub(content_start)
}

/// Split text at double-newline boundaries. If any resulting piece exceeds
/// `max_chars`, it is kept as-is (the caller may sub-split further).
fn split_by_paragraphs(text: &str, _max_chars: usize) -> Vec<String> {
    text.split("\n\n")
        .map(|s| s.to_string())
        .collect()
}

/// Split text at single-newline boundaries, grouping lines until the
/// accumulated size would exceed `max_chars`.
fn split_by_lines(text: &str, max_chars: usize) -> Vec<String> {
    let mut result = Vec::new();
    let mut current = String::new();

    for line in text.lines() {
        if !current.is_empty() && current.len() + line.len() + 1 > max_chars {
            result.push(current.clone());
            current.clear();
        }
        if !current.is_empty() {
            current.push('\n');
        }
        current.push_str(line);
    }
    if !current.is_empty() {
        result.push(current);
    }
    result
}

/// Convenience builder for a [`ParsedChunk`].
fn make_chunk(
    content: String,
    chunk_index: i32,
    start_offset: i64,
    end_offset: i64,
    heading_context: Option<String>,
) -> ParsedChunk {
    ParsedChunk {
        content,
        chunk_index,
        start_offset,
        end_offset,
        heading_context,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    // -- MIME detection -----------------------------------------------------

    #[test]
    fn test_detect_mime_markdown() {
        assert_eq!(detect_mime_type(Path::new("notes.md")), "text/markdown");
        assert_eq!(
            detect_mime_type(Path::new("readme.markdown")),
            "text/markdown"
        );
        assert_eq!(detect_mime_type(Path::new("UPPER.MD")), "text/markdown");
    }

    #[test]
    fn test_detect_mime_plaintext() {
        assert_eq!(detect_mime_type(Path::new("todo.txt")), "text/plain");
    }

    #[test]
    fn test_detect_mime_log() {
        assert_eq!(detect_mime_type(Path::new("app.log")), "text/x-log");
    }

    #[test]
    fn test_detect_mime_pdf() {
        assert_eq!(detect_mime_type(Path::new("report.pdf")), "application/pdf");
        assert_eq!(detect_mime_type(Path::new("UPPER.PDF")), "application/pdf");
    }

    #[test]
    fn test_detect_mime_unknown() {
        assert_eq!(
            detect_mime_type(Path::new("image.png")),
            "application/octet-stream"
        );
    }

    // -- Content hash -------------------------------------------------------

    #[test]
    fn test_content_hash_deterministic() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(f, "Hello, world!").unwrap();
        f.flush().unwrap();

        let doc1 = parse_file(f.path()).unwrap();
        let doc2 = parse_file(f.path()).unwrap();
        assert_eq!(doc1.content_hash, doc2.content_hash);
        assert!(!doc1.content_hash.is_empty());
    }

    #[test]
    fn test_content_hash_differs() {
        let mut f1 = NamedTempFile::new().unwrap();
        writeln!(f1, "aaa").unwrap();
        f1.flush().unwrap();

        let mut f2 = NamedTempFile::new().unwrap();
        writeln!(f2, "bbb").unwrap();
        f2.flush().unwrap();

        let d1 = parse_file(f1.path()).unwrap();
        let d2 = parse_file(f2.path()).unwrap();
        assert_ne!(d1.content_hash, d2.content_hash);
    }

    // -- Markdown chunking --------------------------------------------------

    #[test]
    fn test_markdown_heading_chunks() {
        let md = "\
# Introduction
This is the intro section with enough text to pass the minimum chunk size threshold of fifty characters easily.

## Details
Here are details about the topic with enough text to pass the minimum chunk size threshold of fifty characters easily.

## Conclusion
Final thoughts go here with enough text to pass the minimum chunk size threshold of fifty characters easily.
";
        let chunks = chunk_markdown(md);

        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].heading_context.as_deref(), Some("Introduction"));
        assert_eq!(chunks[1].heading_context.as_deref(), Some("Details"));
        assert_eq!(chunks[2].heading_context.as_deref(), Some("Conclusion"));

        // Indices are sequential.
        for (i, c) in chunks.iter().enumerate() {
            assert_eq!(c.chunk_index, i as i32);
        }
    }

    #[test]
    fn test_markdown_large_section_split() {
        // Build a section larger than MAX_CHUNK_CHARS.
        let big_paragraph_a = "word ".repeat(300); // ~1500 chars
        let big_paragraph_b = "more ".repeat(300);
        let md = format!(
            "# Big Section\n{}\n\n{}",
            big_paragraph_a.trim(),
            big_paragraph_b.trim()
        );

        let chunks = chunk_markdown(&md);
        assert!(
            chunks.len() >= 2,
            "Expected large section to be split into ≥2 chunks, got {}",
            chunks.len()
        );
        for c in &chunks {
            assert_eq!(c.heading_context.as_deref(), Some("Big Section"));
        }
    }

    #[test]
    fn test_markdown_skips_tiny_chunks() {
        let md = "# Heading\nTiny.\n";
        let chunks = chunk_markdown(md);
        assert!(chunks.is_empty(), "Chunks < 50 chars should be skipped");
    }

    // -- Plain text chunking ------------------------------------------------

    #[test]
    fn test_plaintext_paragraphs() {
        let text = format!(
            "{}\n\n{}\n\n{}",
            "First paragraph. ".repeat(5).trim(),
            "Second paragraph. ".repeat(5).trim(),
            "Third paragraph. ".repeat(5).trim(),
        );

        let chunks = chunk_plaintext(&text);
        assert_eq!(chunks.len(), 3);
        assert!(chunks[0].heading_context.is_none());
    }

    #[test]
    fn test_plaintext_skips_small() {
        let text = "hi\n\nbye";
        let chunks = chunk_plaintext(text);
        assert!(chunks.is_empty());
    }

    #[test]
    fn test_plaintext_large_paragraph_split() {
        let big = "line of text that is reasonably long\n".repeat(100);
        let chunks = chunk_plaintext(&big);
        assert!(
            chunks.len() >= 1,
            "Large paragraph should produce chunks"
        );
        for c in &chunks {
            assert!(c.content.len() <= MAX_CHUNK_CHARS + 200);
        }
    }

    // -- parse_file integration ---------------------------------------------

    #[test]
    fn test_parse_file_md() {
        let mut f = NamedTempFile::with_suffix(".md").unwrap();
        let body = "# Hello\nSome content that is long enough to exceed the minimum chunk size threshold easily.\n";
        write!(f, "{}", body).unwrap();
        f.flush().unwrap();

        let doc = parse_file(f.path()).unwrap();
        assert_eq!(doc.mime_type, "text/markdown");
        assert_eq!(doc.file_name, f.path().file_name().unwrap().to_str().unwrap());
        assert_eq!(doc.file_size, body.len() as i64);
        assert!(!doc.content_hash.is_empty());
    }

    #[test]
    fn test_parse_file_txt() {
        let mut f = NamedTempFile::with_suffix(".txt").unwrap();
        let body = "A plain text paragraph with enough words to pass the minimum size filter of fifty characters.\n";
        write!(f, "{}", body).unwrap();
        f.flush().unwrap();

        let doc = parse_file(f.path()).unwrap();
        assert_eq!(doc.mime_type, "text/plain");
    }

    #[test]
    fn test_parse_file_not_found() {
        let result = parse_file(Path::new("/tmp/nonexistent_ask_core_test_file.txt"));
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_pdf_not_found() {
        let result = parse_pdf(Path::new("/tmp/nonexistent_report.pdf"));
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_pdf_invalid_bytes() {
        // Write non-PDF bytes to a .pdf file — should return a parse error.
        let mut f = NamedTempFile::with_suffix(".pdf").unwrap();
        f.write_all(b"this is not a real pdf").unwrap();
        f.flush().unwrap();

        let result = parse_file(f.path());
        assert!(result.is_err(), "Corrupt/fake PDF should produce an error");
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("PDF extraction failed"),
            "Error should mention PDF extraction, got: {}",
            err_msg
        );
    }
}
