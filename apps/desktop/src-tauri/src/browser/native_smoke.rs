//! Opt-in real WebView2 acceptance. Uses a non-activating, isolated fixture window and
//! profile, never the user's active browser or installed Nexa database.
use super::*;

#[test]
#[ignore = "requires an interactive Windows desktop and installed WebView2"]
fn native_dialog_and_download_complete_the_original_trusted_gesture() {
    let root = tempfile::tempdir().unwrap();
    let profile = root.path().join("profile");
    let destination = root.path().join("export.txt");
    let (sender, receiver) = std::sync::mpsc::channel();
    let mut context = tauri::generate_context!();
    context.config_mut().app.windows.clear();
    context.config_mut().identifier = "com.nexa.browser-fixture".into();
    let app = tauri::Builder::default().any_thread().setup(move |app| {
        let window = tauri::window::WindowBuilder::new(app, "main")
            .title("Nexa Browser regression fixture").visible(true).focused(false)
            .inner_size(640.0, 480.0).build()?;
        let webview = window.add_child(
            WebviewBuilder::new("fixture", WebviewUrl::External(Url::parse("about:blank")?))
                .data_directory(profile)
                .initialization_script(super::super::scripts::browser_init_script("fixture-pick"))
                .initialization_script(super::super::scripts::browser_takeover_script("fixture-input"))
                .on_navigation(|url| url.scheme() != "nexa-user-input")
                ,
            tauri::LogicalPosition::new(0.0, 0.0), tauri::LogicalSize::new(640.0, 480.0),
        )?;
        let handle = app.handle().clone();
        tauri::async_runtime::spawn(async move {
            let result = tokio::time::timeout(std::time::Duration::from_secs(30), async {
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.map_err(|error| error.to_string())?;
                let fixture_url = format!("http://{}/", listener.local_addr().map_err(|error| error.to_string())?);
                let server = tauri::async_runtime::spawn(async move {
                    let router = axum::Router::new().route("/", axum::routing::get(|| async { axum::response::Html("<!doctype html><html><body></body></html>") }));
                    let _ = axum::serve(listener, router).await;
                });
                webview.navigate(Url::parse(&fixture_url).map_err(|error| error.to_string())?).map_err(|error| error.to_string())?;
                for _ in 0..100 {
                    if eval_json(&webview, "({url: location.href, ready: document.readyState, runtime: Boolean(window.__NEXA_BROWSER_RUNTIME__)})").await?.get("runtime").and_then(serde_json::Value::as_bool) == Some(true) { break; }
                    tokio::time::sleep(std::time::Duration::from_millis(25)).await;
                }
                server.abort();
                native_form_competence(&webview).await?;
                native_shadow_component_competence(&webview).await?;
                let restricted = Arc::new(AtomicBool::new(true));
                let dialogs = Arc::new(super::super::dialogs::DialogPolicy::default());
                super::super::dialogs::install(&webview, dialogs.clone(), restricted.clone(), |_| {}).await?;
                let downloads = Arc::new(super::super::downloads::DownloadGate::default());
                let blocked = Arc::new(std::sync::atomic::AtomicUsize::new(0));
                let blocked_event = blocked.clone();
                super::super::downloads::install(&webview, downloads.clone(), restricted, move |_| { blocked_event.fetch_add(1, std::sync::atomic::Ordering::AcqRel); }).await?;
                eval_json(&webview, "(() => { const a = document.createElement('a'); a.href = URL.createObjectURL(new Blob(['blocked fixture'])); a.download = 'blocked.txt'; a.click(); return true; })()").await?;
                for _ in 0..80 { if blocked.load(std::sync::atomic::Ordering::Acquire) > 0 { break; } tokio::time::sleep(std::time::Duration::from_millis(25)).await; }
                if blocked.load(std::sync::atomic::Ordering::Acquire) != 1 || downloads.blocked_count() != 1 { return Err("Unarmed native download did not emit a blocked notification".into()); }

                eval_json(&webview, r#"(() => {
                    document.body.innerHTML = '<button id="export">Export</button>';
                    window.events = [];
                    const button = document.getElementById('export');
                    button.onmousedown = () => { window.events.push('down'); window.accepted = confirm('Export this report?'); };
                    button.onmouseup = () => window.events.push('up');
                    button.onclick = () => {
                        window.events.push('click');
                        if (!window.accepted || prompt('Export name', '') !== 'report') return;
                        const link = document.createElement('a');
                        link.href = URL.createObjectURL(new Blob(['native download proof'], { type: 'text/plain' }));
                        link.download = 'export.txt'; link.click();
                    };
                    return true;
                })()"#).await?;
                let binding = eval_json(&webview, r#"(() => {
                    const bridge = window.__NEXA_BROWSER_RUNTIME__;
                    const element = document.getElementById('export');
                    const target = bridge.observe().elements.find(item => bridge.resolveTargetRef(item.ref) === element);
                    return { targetRef: target.ref, targetContext: bridge.targetContextFingerprint(element), x: target.bounds.x + target.bounds.width / 2, y: target.bounds.y + target.bounds.height / 2 };
                })()"#).await?;
                let responses: Vec<super::super::dialogs::DialogResponse> = serde_json::from_value(serde_json::json!([
                    { "kind": "confirm", "message": "Export this report?", "accept": true },
                    { "kind": "prompt", "message": "Export name", "accept": true, "promptText": "report" }
                ])).map_err(|error| error.to_string())?;
                let dialog_action = dialogs.arm(&fixture_url, &responses)?;
                let download = downloads.arm(&destination)?;
                let x = binding["x"].as_f64().ok_or("Missing x")?;
                let y = binding["y"].as_f64().ok_or("Missing y")?;
                let guard = BrowserTrustedInputGuard { webview: webview.clone(), token: Arc::from("fixture-input") };
                let armed = guard.arm(TrustedInputEventBudget::pointer_click(1, 0)?, TrustedInputMatch::Pointer { x, y, button: "left".into() }, binding["targetRef"].as_str().ok_or("Missing target")?, binding["targetContext"].as_str().ok_or("Missing context")?).await?;
                dispatch_trusted_pointer_click(&armed, x, y, "left", &[], 1).await?;
                armed.disarm().await?;
                let events = eval_json(&webview, "window.events").await?;
                if events != serde_json::json!(["down", "up", "click"]) { return Err(format!("Input was replayed or release was lost: {events}")); }
                let results = dialog_action.results();
                if results.len() != 2 || results.iter().any(|dialog| !dialog.accepted || !dialog.matched) { return Err(format!("Dialog responses did not match: {results:?}")); }
                let result = download.finish().await;
                if result.state != "completed" { return Err(format!("Download did not complete: {result:?}")); }
                let content = std::fs::read_to_string(&destination).map_err(|error| error.to_string())?;
                if content != "native download proof" || result.bytes != content.len() as u64 || result.content_hash.is_none() { return Err("Native download content verification failed".into()); }
                let binding = eval_json(&webview, r#"(() => {
                    const input = document.createElement('input'); input.type = 'file'; input.id = 'upload'; input.hidden = true;
                    document.body.appendChild(input);
                    const bridge = window.__NEXA_BROWSER_RUNTIME__;
                    const target = bridge.observe().elements.find(item => bridge.resolveTargetRef(item.ref) === input);
                    return { targetRef: target.ref, targetContext: bridge.targetContextFingerprint(input) };
                })()"#).await?;
                let upload = super::super::file_upload::PreparedUpload {
                    paths: vec![destination.canonicalize().map_err(|error| error.to_string())?.to_string_lossy().into()],
                    files: vec![nexa_core::browser_runtime::BrowserFileMetadata { name: "export.txt".into(), size: content.len() as u64 }],
                };
                let target_ref = binding["targetRef"].as_str().ok_or("Missing upload target")?;
                let target_context = binding["targetContext"].as_str().ok_or("Missing upload context")?;
                let armed = guard.arm(TrustedInputEventBudget::new(0, 0, 1)?, TrustedInputMatch::Files { files: upload.files.clone() }, target_ref, target_context).await?;
                set_trusted_files(&armed, target_ref, target_context, &upload).await?;
                armed.disarm().await?;
                let (upload_content, _) = call_devtools_protocol_method(&webview, "Runtime.evaluate", serde_json::json!({"expression":"document.getElementById('upload').files[0].text()", "awaitPromise":true, "returnByValue":true}), std::time::Duration::from_secs(5), None).await?;
                let upload_content: serde_json::Value = serde_json::from_str(&upload_content).map_err(|error| error.to_string())?;
                if upload_content["result"]["value"].as_str() != Some(content.as_str()) { return Err("Native upload file content did not match".into()); }
                // A later unsolicited dialog is dismissed immediately. It cannot
                // consume an old answer or leave page evaluation suspended.
                drop(dialog_action);
                if eval_json(&webview, "confirm('Export this report?')").await? != serde_json::json!(false) { return Err("A consumed dialog response was reused".into()); }
                let burst_policy = dialogs.arm(&fixture_url, &[])?;
                let burst = tokio::time::timeout(std::time::Duration::from_millis(500), eval_json(&webview, "(() => { for (let i = 0; i < 100; i++) confirm('burst'); return true; })()")).await;
                if burst.is_ok() || !dialogs.is_paused() || !burst_policy.results().iter().any(|dialog| dialog.dialog_limit_exceeded) { return Err("Dialog flood did not pause at its bound".into()); }
                let (ui_sender, ui_receiver) = tokio::sync::oneshot::channel();
                webview.app_handle().run_on_main_thread(move || { let _ = ui_sender.send(()); }).map_err(|error| error.to_string())?;
                tokio::time::timeout(std::time::Duration::from_secs(1), ui_receiver).await.map_err(|_| "Host UI froze during dialog flood")?.map_err(|_| "Host UI callback dropped")?;
                webview.close().map_err(|error| error.to_string())?;
                Ok::<_, String>(())
            }).await.unwrap_or_else(|_| Err("Native browser fixture exceeded 30 seconds".into()));
            let _ = sender.send(result);
            handle.exit(0);
        });
        Ok(())
    }).build(context).unwrap();
    app.run_return(|_, _| {});
    receiver
        .recv_timeout(std::time::Duration::from_secs(2))
        .unwrap()
        .unwrap();
}

async fn native_shadow_component_competence(webview: &Webview) -> Result<(), String> {
    eval_json(webview, r#"(() => {
        document.body.innerHTML = '<section id="component"></section><output id="receipt"></output>';
        const root = document.getElementById('component').attachShadow({mode:'open'});
        root.innerHTML = '<section id="nested"></section>';
        const nested = root.querySelector('section').attachShadow({mode:'open'});
        nested.innerHTML = '<input aria-label="Component editor" value="Before"><button>Save component</button>';
        nested.querySelector('button').onclick = () => document.getElementById('receipt').textContent = 'Saved ' + nested.querySelector('input').value;
        return true;
    })()"#).await?;
    let guard = BrowserTrustedInputGuard {
        webview: webview.clone(),
        token: Arc::from("fixture-input"),
    };
    for (name, action) in [("Component editor", "type"), ("Save component", "click")] {
        let before = fixture_observe(webview, Some(name)).await?;
        let target = &before["elements"][0];
        if target["name"] != name {
            return Err("Native observation missed a nested web component control".into());
        }
        let input = serde_json::json!({"action":action,"targetRef":target["ref"],"expected":target,"userEpoch":before["userEpoch"],"interactionFingerprint":before["interactionFingerprint"]});
        let method = if action == "type" {
            "prepareTrustedText"
        } else {
            "prepareNativePointer"
        };
        let prepared = eval_json(
            webview,
            &format!("window.__NEXA_BROWSER_RUNTIME__.{method}({input})"),
        )
        .await?;
        let (budget, expected) = if action == "type" {
            if prepared["focused"] != true {
                return Err("Native web component editor could not receive focus".into());
            }
            (
                TrustedInputEventBudget::text_insert(),
                TrustedInputMatch::Text {
                    data: "Reviewed component".into(),
                },
            )
        } else {
            let bounds = &prepared["bounds"];
            let x = bounds["x"].as_f64().ok_or("Missing component x")?
                + bounds["width"].as_f64().ok_or("Missing component width")? / 2.0;
            let y = bounds["y"].as_f64().ok_or("Missing component y")?
                + bounds["height"]
                    .as_f64()
                    .ok_or("Missing component height")?
                    / 2.0;
            (
                TrustedInputEventBudget::pointer_click(1, 0)?,
                TrustedInputMatch::Pointer {
                    x,
                    y,
                    button: "left".into(),
                },
            )
        };
        let armed = guard
            .arm(
                budget,
                expected.clone(),
                prepared["targetRef"]
                    .as_str()
                    .ok_or("Missing component target")?,
                prepared["targetContext"]
                    .as_str()
                    .ok_or("Missing component context")?,
            )
            .await?;
        match expected {
            TrustedInputMatch::Text { data } => insert_trusted_text(&armed, &data).await?,
            TrustedInputMatch::Pointer { x, y, button } => {
                dispatch_trusted_pointer_click(&armed, x, y, &button, &[], 1).await?
            }
            _ => unreachable!(),
        }
        armed.disarm().await?;
        let after = fixture_observe(webview, Some(name)).await?;
        if before["userEpoch"] != after["userEpoch"] {
            return Err(
                "Trusted native component input incorrectly triggered user takeover".into(),
            );
        }
        if action == "type" && after["elements"][0]["value"] != "Reviewed component" {
            return Err("Native component text did not reach the editor".into());
        }
    }
    if eval_json(webview, "document.getElementById('receipt').textContent").await?
        != "Saved Reviewed component"
    {
        return Err("Native web component click did not produce its visible receipt".into());
    }
    Ok(())
}

/// Observe through the production JS, native screenshot finalizer and ToolResult
/// serializer. A second capture checks the same options before refs are used.
async fn fixture_observe(
    webview: &Webview,
    query: Option<&str>,
) -> Result<serde_json::Value, String> {
    use nexa_core::browser_runtime::{BrowserControlOwner, BrowserObservation, BrowserScreenshot};
    let options = serde_json::json!({"query": query, "offset": 0});
    let expression = format!("window.__NEXA_BROWSER_RUNTIME__.observe({options})");
    let snapshot = eval_json(webview, &expression).await?;
    let plan = BrowserCapturePlan::new(
        BrowserBounds {
            x: 0.0,
            y: 0.0,
            width: 640.0,
            height: 480.0,
        },
        1.0,
    )?;
    let capture = capture_webview_image(
        webview,
        plan,
        BrowserSurfaceGate::default().acquire().await?,
    )
    .await?;
    let confirmed = eval_json(webview, &expression).await?;
    if confirmed["interactionFingerprint"] != snapshot["interactionFingerprint"]
        || confirmed["userEpoch"] != snapshot["userEpoch"]
    {
        return Err("Native fixture changed during its screenshot capture".into());
    }
    let elements =
        serde_json::from_value(confirmed["elements"].clone()).map_err(|error| error.to_string())?;
    let observation = BrowserObservation {
        observation_id: "fixture-observation".into(),
        session_id: "fixture-session".into(),
        tab_id: "fixture-tab".into(),
        url: confirmed["url"]
            .as_str()
            .ok_or("Missing fixture URL")?
            .into(),
        title: confirmed["title"].as_str().unwrap_or_default().into(),
        ready_state: confirmed["readyState"].as_str().map(str::to_string),
        text: confirmed["text"].as_str().unwrap_or_default().into(),
        viewport: confirmed["viewport"].clone(),
        content_hash: confirmed["domFingerprint"]
            .as_str()
            .ok_or("Missing fixture fingerprint")?
            .into(),
        elements,
        observation_coverage: Some(
            serde_json::from_value(confirmed["observationCoverage"].clone())
                .map_err(|error| error.to_string())?,
        ),
        frame_limitations: Some(
            serde_json::from_value(confirmed["frameLimitations"].clone())
                .map_err(|error| error.to_string())?,
        ),
        accessibility_tree: Vec::new(),
        control_owner: BrowserControlOwner::Agent {
            call_id: "fixture".into(),
        },
        screenshot: Some(BrowserScreenshot {
            mime_type: capture.mime_type,
            content_hash: blake3::hash(&capture.image_bytes).to_hex().to_string(),
            width: capture.width,
            height: capture.height,
            byte_length: capture.image_bytes.len(),
            image_bytes: capture.image_bytes,
        }),
    };
    let result = super::super::agent_tool::observation_result("fixture", observation)
        .map_err(|error| error.to_string())?;
    let output = result.output_channels();
    if output.attachments.len() != 1
        || !output.llm_content.contains("fixture-observation")
        || output.llm_content.contains("private-native-password")
    {
        return Err("Native observation lost model evidence or exposed a password".into());
    }
    let typed_elements = output
        .data
        .as_ref()
        .and_then(|data| data.get("elements"))
        .ok_or("Missing typed model elements")?;
    for element in confirmed["elements"]
        .as_array()
        .ok_or("Missing fixture elements")?
    {
        if let Some(value) = element["value"].as_str() {
            let typed = typed_elements
                .as_array()
                .and_then(|elements| {
                    elements
                        .iter()
                        .find(|candidate| candidate["ref"] == element["ref"])
                })
                .ok_or("Model ref lost in serialization")?;
            if typed["value"].as_str() != Some(value) {
                return Err("Model form value lost in serialization".into());
            }
        }
    }
    Ok(confirmed)
}

async fn native_form_competence(webview: &Webview) -> Result<(), String> {
    eval_json(webview, r#"(() => {
        document.body.innerHTML = '<span id="name-label">Project name</span><input id="name" aria-labelledby="name-label" value="Before"><label>Approved<input id="approved" type="checkbox"></label><input type="password" value="private-native-password"><button id="save">Save project</button><p id="receipt"></p><section id="queue"></section>';
        document.getElementById('save').onclick = () => document.getElementById('receipt').textContent = document.getElementById('approved').checked ? 'Saved ' + document.getElementById('name').value : 'Approval missing';
        document.getElementById('queue').innerHTML = Array.from({length:340}, (_, i) => '<button>Earlier record '+i+'</button>').join('') + '<button>Export reviewed report</button>';
        return true;
    })()"#).await?;
    let before = fixture_observe(webview, Some("Project name")).await?;
    let target = before["elements"]
        .as_array()
        .and_then(|elements| elements.first())
        .ok_or("Native query did not find the labelled form field")?;
    if target["name"] != "Project name" || target["value"] != "Before" {
        return Err("Native form name/value evidence was incomplete".into());
    }
    let input = serde_json::json!({"action":"type", "userEpoch":before["userEpoch"], "interactionFingerprint":before["interactionFingerprint"], "targetRef":target["ref"], "expected":target, "text":"Reviewed"});
    let prepared = eval_json(
        webview,
        &format!("window.__NEXA_BROWSER_RUNTIME__.prepareTrustedText({input})"),
    )
    .await?;
    let guard = BrowserTrustedInputGuard {
        webview: webview.clone(),
        token: Arc::from("fixture-input"),
    };
    let armed = guard
        .arm(
            TrustedInputEventBudget::new(0, 0, 1)?,
            TrustedInputMatch::Text {
                data: "Reviewed".into(),
            },
            prepared["targetRef"]
                .as_str()
                .ok_or("Missing text target")?,
            prepared["targetContext"]
                .as_str()
                .ok_or("Missing text context")?,
        )
        .await?;
    insert_trusted_text(&armed, "Reviewed").await?;
    armed.disarm().await?;
    let after = fixture_observe(webview, Some("Project name")).await?;
    if after["elements"][0]["value"] != "Reviewed" {
        return Err("Trusted native text was not visible to the agent".into());
    }
    for (name, check_state) in [("Approved", true), ("Save project", false)] {
        let before = fixture_observe(webview, Some(name)).await?;
        let target = before["elements"]
            .as_array()
            .and_then(|elements| elements.iter().find(|element| element["name"] == name))
            .ok_or("Missing native click target")?;
        let input = serde_json::json!({"action": if check_state {"set_checked"} else {"click"}, "checked":true, "userEpoch":before["userEpoch"], "interactionFingerprint":before["interactionFingerprint"], "targetRef":target["ref"], "expected":target});
        let prepared = eval_json(
            webview,
            &format!("window.__NEXA_BROWSER_RUNTIME__.prepareNativePointer({input})"),
        )
        .await?;
        let bounds = &prepared["bounds"];
        let x = bounds["x"].as_f64().ok_or("Missing native x")?
            + bounds["width"].as_f64().ok_or("Missing native width")? / 2.0;
        let y = bounds["y"].as_f64().ok_or("Missing native y")?
            + bounds["height"].as_f64().ok_or("Missing native height")? / 2.0;
        let armed = guard
            .arm(
                TrustedInputEventBudget::pointer_click(1, if check_state { 1 } else { 0 })?,
                TrustedInputMatch::Pointer {
                    x,
                    y,
                    button: "left".into(),
                },
                prepared["targetRef"]
                    .as_str()
                    .ok_or("Missing click target")?,
                prepared["targetContext"]
                    .as_str()
                    .ok_or("Missing click context")?,
            )
            .await?;
        dispatch_trusted_pointer_click(&armed, x, y, "left", &[], 1).await?;
        armed.disarm().await?;
        let after = fixture_observe(webview, Some(name)).await?;
        if check_state && after["elements"][0]["checked"] != true {
            return Err("Native checkbox state was not observed".into());
        }
        if !check_state
            && !after["text"]
                .as_str()
                .unwrap_or_default()
                .contains("Saved Reviewed")
        {
            return Err("Native form commit lacked a visible receipt".into());
        }
    }
    let recovered = fixture_observe(webview, Some("Export reviewed report")).await?;
    if recovered["elements"].as_array().map(Vec::len) != Some(1)
        || recovered["observationCoverage"]["totalMatches"] != 1
    {
        return Err(
            "Native filtered observation could not recover a control past the default element page"
                .into(),
        );
    }
    let approved = fixture_observe(webview, Some("Approved")).await?;
    let target = &approved["elements"][0];
    let input = serde_json::json!({"action":"set_checked","checked":true,"targetRef":target["ref"],"expected":target,"userEpoch":approved["userEpoch"],"interactionFingerprint":approved["interactionFingerprint"]});
    let prepared = eval_json(
        webview,
        &format!("window.__NEXA_BROWSER_RUNTIME__.prepareNativePointer({input})"),
    )
    .await?;
    let after_noop = fixture_observe(webview, Some("Approved")).await?;
    if prepared["stateMatched"] != true
        || prepared["verificationBaseline"]["observationOptions"]["query"] != "Approved"
        || after_noop["domFingerprint"] != prepared["verificationBaseline"]["domFingerprint"]
    {
        return Err("Filtered native no-op changed its verification set".into());
    }
    eval_json(webview, "(() => { const label = document.createElement('label'); label.innerHTML = 'Queue mode<select><option value=\"pending\">Pending</option><option value=\"done\">Done</option></select>'; document.body.appendChild(label); return true; })()").await?;
    let before = fixture_observe(webview, Some("Queue mode")).await?;
    let target = &before["elements"][0];
    let input = serde_json::json!({"action":"select","value":"done","targetRef":target["ref"],"expected":target,"userEpoch":before["userEpoch"],"interactionFingerprint":before["interactionFingerprint"]});
    eval_json(
        webview,
        &format!("window.__NEXA_BROWSER_RUNTIME__.act({input})"),
    )
    .await?;
    let after = fixture_observe(webview, Some("Queue mode")).await?;
    if after["elements"][0]["selectedValues"] != serde_json::json!(["done"]) {
        return Err("Late-page select lost its queried post-action evidence".into());
    }
    Ok(())
}
