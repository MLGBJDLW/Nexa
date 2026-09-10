//! Isolated, read-only origins for local web artifacts. Never serves the app IPC origin.
use axum::{
    body::Body,
    extract::State,
    http::{header, Request, StatusCode},
    response::{IntoResponse, Response},
    Router,
};
use nexa_core::tools::run_shell_tool::{ManagedLoopbackPermit, ManagedLoopbackPermitIssuer};
use serde::Serialize;
use std::{
    future::IntoFuture,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::sync::{oneshot, Semaphore};

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HtmlPreview {
    pub preview_id: String,
    pub path: String,
    pub url: String,
    pub title: String,
    pub conversation_id: String,
    pub reused: bool,
}
pub struct HtmlServer {
    pub preview: HtmlPreview,
    pub permit: ManagedLoopbackPermit,
    issuer: ManagedLoopbackPermitIssuer,
    stop: Option<oneshot::Sender<()>>,
}
impl Drop for HtmlServer {
    fn drop(&mut self) {
        self.issuer.revoke();
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
    }
}
#[derive(Clone)]
struct WebRoot {
    root: PathBuf,
    entry: String,
    host: String,
    token: String,
    capacity: Arc<Semaphore>,
}

pub async fn start(path: String, conversation_id: String) -> Result<HtmlServer, String> {
    let path = tokio::fs::canonicalize(&path)
        .await
        .map_err(|e| e.to_string())?;
    if !path.is_file()
        || !matches!(
            path.extension()
                .and_then(|e| e.to_str())
                .unwrap_or_default()
                .to_ascii_lowercase()
                .as_str(),
            "html" | "htm"
        )
    {
        return Err("Expected a local HTML file".into());
    }
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let id = uuid::Uuid::new_v4().simple().to_string();
    let host = format!("nexa-{id}.localhost");
    let origin = format!("http://{host}:{port}");
    let token = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let title = path.file_name().unwrap().to_string_lossy().into_owned();
    let entry = percent_encoding::utf8_percent_encode(&title, percent_encoding::NON_ALPHANUMERIC)
        .to_string();
    let root = WebRoot {
        root: path.parent().unwrap().into(),
        entry: format!("/{entry}"),
        host: format!("{host}:{port}"),
        token: token.clone(),
        capacity: Arc::new(Semaphore::new(16)),
    };
    let issuer = ManagedLoopbackPermitIssuer::new(format!("html-preview:{id}"), None);
    let permit = issuer.issue(&origin, &host, port);
    let (stop, mut stopped) = oneshot::channel();
    let lease = issuer.clone();
    let router = Router::new().fallback(serve).with_state(root);
    tokio::spawn(async move {
        let server = axum::serve(listener, router).into_future();
        tokio::pin!(server);
        let mut interval = tokio::time::interval(Duration::from_secs(1));
        loop {
            tokio::select! { _ = &mut stopped => break, _ = &mut server => break, _ = interval.tick() => lease.refresh() }
        }
        lease.revoke();
    });
    Ok(HtmlServer {
        preview: HtmlPreview {
            preview_id: id,
            path: path.to_string_lossy().into_owned(),
            url: format!("{origin}/__nexa_bootstrap/{token}"),
            title,
            conversation_id,
            reused: false,
        },
        permit,
        issuer,
        stop: Some(stop),
    })
}

async fn serve(State(root): State<WebRoot>, request: Request<Body>) -> Response {
    if request
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        != Some(&root.host)
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    if !matches!(
        *request.method(),
        axum::http::Method::GET | axum::http::Method::HEAD
    ) {
        return StatusCode::METHOD_NOT_ALLOWED.into_response();
    }
    let path = request.uri().path();
    if path == format!("/__nexa_bootstrap/{}", root.token) {
        return (
            StatusCode::SEE_OTHER,
            [
                (header::LOCATION, root.entry.clone()),
                (
                    header::SET_COOKIE,
                    format!(
                        "nexa_preview={}; Path=/; HttpOnly; SameSite=Strict",
                        root.token
                    ),
                ),
                (header::CACHE_CONTROL, "no-store".into()),
                (header::REFERRER_POLICY, "no-referrer".into()),
            ],
        )
            .into_response();
    }
    if !request
        .headers()
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|cookie| {
            cookie
                .split(';')
                .any(|value| value.trim() == format!("nexa_preview={}", root.token))
        })
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    if request
        .headers()
        .get("sec-fetch-site")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|site| !matches!(site, "same-origin" | "none"))
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    let Ok(_capacity) = root.capacity.try_acquire() else {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    };
    let Ok(decoded) = percent_encoding::percent_decode_str(path).decode_utf8() else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let Some(relative) = allowed_web_path(&decoded) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Ok(file) = tokio::fs::canonicalize(root.root.join(relative)).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if !file.starts_with(&root.root) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let Ok(metadata) = tokio::fs::metadata(&file).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if !metadata.is_file() || metadata.len() > 32 * 1024 * 1024 {
        return StatusCode::PAYLOAD_TOO_LARGE.into_response();
    }
    let mime = web_mime(&file).unwrap_or("application/octet-stream");
    let body = if request.method() == axum::http::Method::HEAD {
        Vec::new()
    } else {
        match tokio::fs::read(file).await {
            Ok(data) => data,
            Err(_) => return StatusCode::NOT_FOUND.into_response(),
        }
    };
    let mut response = ([(header::CONTENT_TYPE, mime), (header::CACHE_CONTROL, "no-store"), (header::REFERRER_POLICY, "no-referrer"), (header::X_CONTENT_TYPE_OPTIONS, "nosniff"), (header::CONTENT_SECURITY_POLICY, "default-src 'self' https: data: blob:; script-src 'self' https: 'unsafe-inline' 'unsafe-eval'; style-src 'self' https: 'unsafe-inline'; connect-src 'self' https:; frame-ancestors 'none'; object-src 'none'; base-uri 'self'; form-action 'none'")], body).into_response();
    response
        .headers_mut()
        .insert(header::CONTENT_LENGTH, metadata.len().into());
    response
}
fn allowed_web_path(path: &str) -> Option<&Path> {
    let relative = path.strip_prefix('/')?;
    if relative.is_empty()
        || relative.contains(['\\', ':', '\0'])
        || relative
            .split('/')
            .any(|part| part.is_empty() || part.starts_with('.'))
    {
        return None;
    }
    let path = Path::new(relative);
    web_mime(path)?;
    Some(path)
}
fn web_mime(path: &Path) -> Option<&'static str> {
    Some(
        match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
            "html" | "htm" => "text/html; charset=utf-8",
            "js" | "mjs" => "text/javascript; charset=utf-8",
            "css" => "text/css; charset=utf-8",
            "json" => "application/json",
            "csv" => "text/csv; charset=utf-8",
            "svg" => "image/svg+xml",
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "webp" => "image/webp",
            "gif" => "image/gif",
            "ico" => "image/x-icon",
            "woff" => "font/woff",
            "woff2" => "font/woff2",
            "ttf" => "font/ttf",
            "wasm" => "application/wasm",
            "mp4" => "video/mp4",
            "webm" => "video/webm",
            "mp3" => "audio/mpeg",
            "wav" => "audio/wav",
            "ogg" => "audio/ogg",
            _ => return None,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn local_html_requires_bootstrap_and_serves_relative_assets_until_closed() {
        let root = std::env::temp_dir().join(format!("nexa-html-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("assets")).unwrap();
        std::fs::write(
            root.join("index.html"),
            "<script type=module src=assets/main.js></script>",
        )
        .unwrap();
        std::fs::write(root.join("assets/main.js"), "document.title='loaded'").unwrap();
        let server = start(
            root.join("index.html").to_string_lossy().into(),
            "test".into(),
        )
        .await
        .unwrap();
        let parsed = url::Url::parse(&server.preview.url).unwrap();
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .resolve(
                parsed.host_str().unwrap(),
                std::net::SocketAddr::from(([127, 0, 0, 1], parsed.port().unwrap())),
            )
            .build()
            .unwrap();
        let origin = parsed.origin().ascii_serialization();
        assert_eq!(
            client
                .get(format!("{origin}/index.html"))
                .send()
                .await
                .unwrap()
                .status(),
            403
        );
        let response = client.get(&server.preview.url).send().await.unwrap();
        assert_eq!(response.status(), 303);
        let cookie = response.headers()[header::SET_COOKIE]
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap();
        let asset = client
            .get(format!("{origin}/assets/main.js"))
            .header(header::COOKIE, cookie)
            .send()
            .await
            .unwrap();
        assert_eq!(asset.text().await.unwrap(), "document.title='loaded'");
        assert_eq!(
            client
                .get(format!("{origin}/assets/main.js"))
                .header(header::COOKIE, cookie)
                .header("sec-fetch-site", "cross-site")
                .send()
                .await
                .unwrap()
                .status(),
            403
        );
        for path in [
            "/.env",
            "/../index.html",
            "/secret.key",
            "/C:/secret.json",
            "/assets/../index.html",
        ] {
            assert!(allowed_web_path(path).is_none());
        }
        let permit = server.permit.clone();
        drop(server);
        assert!(!permit.is_live());
        std::fs::remove_dir_all(root).unwrap();
    }
}
