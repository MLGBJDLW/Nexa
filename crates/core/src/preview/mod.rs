mod docx;
pub mod model;
mod pdf;
mod xlsx;

use std::path::{Path, PathBuf};

pub use model::{PreviewCapabilities, StructuredPreview};

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
}

pub fn build_structured_preview(
    path: &Path,
    mime_type: &str,
    content_hash: &str,
    options: &PreviewBuildOptions,
) -> Result<Option<StructuredPreview>, String> {
    match mime_type {
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => {
            docx::preview_docx(path, content_hash, options).map(Some)
        }
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" => {
            if path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("xlsx"))
            {
                xlsx::preview_xlsx(path).map(Some)
            } else {
                Ok(None)
            }
        }
        "application/pdf" => pdf::preview_pdf(path),
        _ => Ok(None),
    }
}
