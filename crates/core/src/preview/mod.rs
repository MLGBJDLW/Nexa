mod docx;
pub mod model;
#[cfg(feature = "document-processing")]
mod pdf;
mod pptx;
#[cfg(feature = "document-processing")]
mod xlsx;

use std::io::{BufReader, Read, Seek};
use std::path::{Path, PathBuf};

pub use model::{PreviewCapabilities, StructuredPreview};

/// Maximum on-disk input for an interactive document preview.
pub const DOCUMENT_PREVIEW_BYTES_LIMIT: u64 = 25 * 1024 * 1024;

/// Media rendered directly by the desktop webview instead of document parsing.
pub fn direct_media_kind(mime_type: &str) -> Option<&'static str> {
    if mime_type.starts_with("image/") {
        Some("image")
    } else if mime_type.starts_with("audio/") {
        Some("audio")
    } else if mime_type.starts_with("video/") {
        Some("video")
    } else {
        None
    }
}

pub fn prefers_internal_preview(path: &Path) -> bool {
    let mime = crate::parse::detect_mime_type(path);
    if direct_media_kind(&mime).is_some()
        || mime.starts_with("text/")
        || mime == "application/pdf"
        || mime.contains("officedocument")
        || mime.starts_with("application/vnd.oasis.opendocument.")
        || matches!(
            mime.as_str(),
            "application/json"
                | "application/xml"
                | "application/javascript"
                | "application/msword"
                | "application/vnd.ms-excel"
                | "application/vnd.ms-powerpoint"
                | "application/epub+zip"
        )
    {
        return true;
    }
    use std::io::Read;
    let mut prefix = [0_u8; 8192];
    std::fs::File::open(path)
        .and_then(|mut file| file.read(&mut prefix))
        .is_ok_and(|length| !prefix[..length].contains(&0))
}

#[derive(Debug, Clone, Default)]
pub struct PreviewBuildOptions {
    pub asset_cache_dir: Option<PathBuf>,
    pub focus_slide: Option<u32>,
}

pub fn build_structured_preview(
    path: &Path,
    mime_type: &str,
    content_hash: &str,
    options: &PreviewBuildOptions,
) -> Result<Option<StructuredPreview>, String> {
    if mime_type == "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
        && !path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("xlsx"))
    {
        return Ok(None);
    }
    build_with_source(
        || {
            std::fs::File::open(path)
                .map(BufReader::new)
                .map_err(|e| e.to_string())
        },
        mime_type,
        content_hash,
        options,
    )
}

/// Render an already-opened source, including a bounded, content-hashed snapshot.
pub fn build_structured_preview_from_reader(
    reader: impl Read + Seek,
    mime_type: &str,
    content_hash: &str,
    options: &PreviewBuildOptions,
) -> Result<Option<StructuredPreview>, String> {
    build_with_source(|| Ok(reader), mime_type, content_hash, options)
}

fn build_with_source<R: Read + Seek>(
    open: impl FnOnce() -> Result<R, String>,
    mime_type: &str,
    content_hash: &str,
    options: &PreviewBuildOptions,
) -> Result<Option<StructuredPreview>, String> {
    match mime_type {
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => {
            docx::preview_docx(open()?, content_hash, options).map(Some)
        }
        "application/vnd.openxmlformats-officedocument.presentationml.presentation" => {
            pptx::preview_pptx(open()?, options.focus_slide).map(Some)
        }
        #[cfg(feature = "document-processing")]
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" => {
            xlsx::preview_xlsx(open()?).map(Some)
        }
        #[cfg(feature = "document-processing")]
        "application/pdf" => pdf::preview_pdf(open()?),
        #[cfg(not(feature = "document-processing"))]
        "application/pdf" | "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" => {
            Err("PDF and spreadsheet previews require the document-processing build feature".into())
        }
        _ => Ok(None),
    }
}
