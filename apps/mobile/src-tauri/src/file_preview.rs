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
    verify_hash: bool,
    start_line: Option<u32>,
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
        _ if start_line.is_some()
            || nexa_core::parse::detect_mime_type(&path).starts_with("text/") =>
        {
            ("text", "text/plain")
        }
        _ => ("binary", "application/octet-stream"),
    };
    let mut first_line = 1;
    let mut hash = String::new();
    let mut truncated = false;
    if data {
        let limit = 8 * 1024 * 1024;
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
        return Ok(json!(format!(
            "data:{mime};base64,{}",
            STANDARD.encode(bytes)
        )));
    }
    let content = if matches!(kind, "code" | "markdown" | "text") {
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
        "content":content,"editable":false,"agentEditAllowed":false,"sizeBytes":metadata.len(),"hash":hash,
        "lineCount":line_count,"firstLine":first_line,"truncated":truncated}),
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
        let preview = read(&root, "timeline.html", false, true, None)
            .await
            .unwrap();
        assert_eq!(preview["content"], "<h1>Timeline</h1>");
        assert_eq!(preview["lineCount"], 1);
        assert_eq!(
            preview["hash"],
            blake3::hash(b"<h1>Timeline</h1>").to_hex().as_str()
        );
        assert_eq!(
            read(&root, "image.png", true, false, None).await.unwrap(),
            "data:image/png;base64,AQID"
        );
        assert!(read(&root, "../private.txt", true, false, None)
            .await
            .is_err());
        assert!(read(
            &root,
            temporary.join("private.txt").to_str().unwrap(),
            true,
            false,
            None
        )
        .await
        .is_err());
        assert!(read(&root, ".", true, false, None).await.is_err());
        let log = "line\n".repeat(110_000) + "last line";
        tokio::fs::write(root.join("large.log"), log.as_bytes())
            .await
            .unwrap();
        let tail = read(&root, "large.log", false, true, Some(110_001))
            .await
            .unwrap();
        assert_eq!(tail["content"], "last line");
        assert_eq!(tail["firstLine"], 110_001);
        assert_eq!(tail["hash"], blake3::hash(log.as_bytes()).to_hex().as_str());
        let large = tokio::fs::File::create(root.join("large.png"))
            .await
            .unwrap();
        large.set_len(8 * 1024 * 1024 + 1).await.unwrap();
        assert!(read(&root, "large.png", true, false, None).await.is_err());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(temporary.join("private.txt"), root.join("link.txt"))
                .unwrap();
            assert!(read(&root, "link.txt", true, false, None).await.is_err());
        }
        tokio::fs::remove_dir_all(temporary).await.unwrap();
    }
}
