//! Read-only phone workspace previews. Canonical containment is shared by data
//! and metadata requests; executable HTML is rendered by the isolated UI host.
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};
use std::path::Path;
use tokio::io::AsyncReadExt;

pub async fn read(
    root: &Path,
    requested: &str,
    data: bool,
    text_citation: bool,
) -> Result<Value, String> {
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
    let (kind, mime) = match extension.as_str() {
        "png" => ("image", "image/png"),
        "jpg" | "jpeg" => ("image", "image/jpeg"),
        "webp" => ("image", "image/webp"),
        "gif" => ("image", "image/gif"),
        "html" | "htm" => ("code", "text/html"),
        "md" | "markdown" => ("markdown", "text/markdown"),
        "txt" | "csv" | "json" | "log" => ("text", "text/plain"),
        "pdf" => ("document", "application/pdf"),
        _ if text_citation || nexa_core::parse::detect_mime_type(&path).starts_with("text/") => {
            ("text", "text/plain")
        }
        _ => ("binary", "application/octet-stream"),
    };
    let limit = if data { 8 * 1024 * 1024 } else { 512 * 1024 };
    let mut bytes = Vec::new();
    if data || matches!(kind, "code" | "markdown" | "text") {
        if metadata.len() > limit {
            return Err("File exceeds the inline preview size; open it from Files.".into());
        }
        file.take(limit + 1)
            .read_to_end(&mut bytes)
            .await
            .map_err(|error| error.to_string())?;
        if bytes.len() as u64 > limit {
            return Err("File grew beyond the inline preview size.".into());
        }
    }
    if data {
        return Ok(json!(format!(
            "data:{mime};base64,{}",
            STANDARD.encode(bytes)
        )));
    }
    let content = if matches!(kind, "code" | "markdown" | "text") {
        Some(nexa_core::parse::decode_text_bytes(&bytes, &path).map_err(|err| err.to_string())?)
    } else {
        None
    };
    let hash = content
        .as_ref()
        .map(|_| blake3::hash(&bytes).to_hex().to_string())
        .unwrap_or_default();
    let line_count = content.as_deref().map_or(0, |text| text.lines().count());
    Ok(
        json!({"path":path,"displayName":path.file_name().unwrap().to_string_lossy(),
        "sourceId":null,"sourceName":"Phone","extension":format!(".{extension}"),"mimeType":mime,"kind":kind,
        "content":content,"editable":false,"agentEditAllowed":false,"sizeBytes":metadata.len(),"hash":hash,
        "lineCount":line_count,"truncated":false}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn previews_workspace_files_without_reading_outside_or_oversized_files() {
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
        let preview = read(&root, "timeline.html", false, true).await.unwrap();
        assert_eq!(preview["content"], "<h1>Timeline</h1>");
        assert_eq!(preview["lineCount"], 1);
        assert_eq!(
            preview["hash"],
            blake3::hash(b"<h1>Timeline</h1>").to_hex().as_str()
        );
        assert_eq!(
            read(&root, "image.png", true, false).await.unwrap(),
            "data:image/png;base64,AQID"
        );
        assert!(read(&root, "../private.txt", true, false).await.is_err());
        assert!(read(
            &root,
            temporary.join("private.txt").to_str().unwrap(),
            true,
            false
        )
        .await
        .is_err());
        assert!(read(&root, ".", true, false).await.is_err());
        let large = tokio::fs::File::create(root.join("large.png"))
            .await
            .unwrap();
        large.set_len(8 * 1024 * 1024 + 1).await.unwrap();
        assert!(read(&root, "large.png", true, false).await.is_err());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(temporary.join("private.txt"), root.join("link.txt"))
                .unwrap();
            assert!(read(&root, "link.txt", true, false).await.is_err());
        }
        tokio::fs::remove_dir_all(temporary).await.unwrap();
    }
}
