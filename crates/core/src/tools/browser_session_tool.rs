use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use headless_chrome::browser::tab::RequestPausedDecision;
use headless_chrome::browser::Tab;
use headless_chrome::protocol::cdp::types::Event;
use headless_chrome::protocol::cdp::Fetch::{
    events::RequestPausedEvent, FailRequest, RequestPattern, RequestStage,
};
use headless_chrome::protocol::cdp::{Network, Page};
use serde::{Deserialize, Serialize};

use crate::activity::{ActivityEventKind, ActivitySpec, ActivityState, ActivitySurface};
use crate::browser_runtime::{BrowserObservationCoverage, BrowserObservationOptions};
use crate::error::CoreError;

use super::fetch_url_tool::{
    browser_request_allowed, is_loopback_url, launch_browser_for_capture,
    validate_url_for_browser_capture,
};
use super::{
    Tool, ToolCategory, ToolExecutionContext, ToolOutput, ToolOutputAttachment, ToolResult,
};

const INTERACTIVE_SELECTOR: &str =
    "a[href],button,input:not([type=hidden i]),textarea,select,[contenteditable=true],[role=button],[role=link],[role=textbox],[role=checkbox],[role=radio],[role=switch],[role=combobox],[tabindex]";
const INTERACTIVE_ELEMENTS_SCRIPT: &str = r#"
(selector) => Array.from(document.querySelectorAll(selector)).map((el,index) => {
  const r = el.getBoundingClientRect();
  const labelledBy = (el.getAttribute('aria-labelledby') || '').split(/\s+/).filter(Boolean).map(id => document.getElementById(id)?.textContent || '').join(' ').replace(/\s+/g, ' ').trim();
  const role = el.getAttribute('role') || (el.tagName === 'INPUT' && ({checkbox:'checkbox',radio:'radio',range:'slider',button:'button',submit:'button',reset:'button'}[el.type])) || ({A:'link',BUTTON:'button',INPUT:'textbox',TEXTAREA:'textbox',SELECT:'combobox'}[el.tagName] || '');
  const name = labelledBy || el.getAttribute('aria-label') || Array.from(el.labels || []).map(label => label.innerText).join(' ') || el.innerText || el.getAttribute('alt') || (['button','submit','reset'].includes(el.type) ? el.value : '') || el.getAttribute('placeholder') || el.getAttribute('title') || el.getAttribute('name') || '';
  const privateValue = (el.tagName === 'INPUT' && ['password','hidden','file','checkbox','radio','button','submit','reset','image'].includes(el.type)) || ['current-password','new-password'].includes(el.autocomplete);
  const value = privateValue ? null : ['INPUT','TEXTAREA'].includes(el.tagName) ? String(el.value) : el.isContentEditable ? el.innerText || '' : null;
  return {ref:`e_${index+1}`,index,tag:el.tagName.toLowerCase(),role,name:String(name).trim().slice(0,240),inputType:el.type || null,value:value === null ? null : value.slice(0,2000),valueTruncated:value === null ? null : value.length > 2000,enabled:!el.matches(':disabled') && el.getAttribute('aria-disabled') !== 'true',visible:r.width>0 && r.height>0 && getComputedStyle(el).visibility!=='hidden',bounds:[r.x,r.y,r.width,r.height]};
})
"#;
const FRAME_LIMITATIONS_SCRIPT: &str = r#"
(() => {
  const frames = [];
  let unavailableCount = 0;
  for (const frame of document.querySelectorAll('iframe')) {
    let sameOrigin = false;
    try { sameOrigin = Boolean(frame.contentDocument?.documentElement); } catch (_) {}
    unavailableCount += 1;
    if (frames.length >= 32) continue;
    const rect = frame.getBoundingClientRect();
    let url = null;
    try { const parsed = new URL(frame.getAttribute('src') || '', document.baseURI); if (['http:','https:'].includes(parsed.protocol)) url = `${parsed.origin}${parsed.pathname}`.slice(0,2048); } catch (_) {}
    frames.push({reason:sameOrigin?'frame_context_unavailable':'cross_origin_or_sandboxed_frame',url,title:(frame.title || '').slice(0,240),visible:rect.width>0 && rect.height>0 && getComputedStyle(frame).visibility!=='hidden',bounds:{x:rect.x,y:rect.y,width:rect.width,height:rect.height}});
  }
  return {unavailableCount,detailsOmitted:unavailableCount>frames.length,frames};
})()
"#;
const MAX_OBSERVATIONS: usize = 64;
const MAX_WAIT_MS: u64 = 120_000;
const OBSERVE_QUANTUM_MS: u64 = 2_500;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BrowserArgs {
    action: String,
    session_id: Option<String>,
    tab_id: Option<String>,
    url: Option<String>,
    observation_id: Option<String>,
    target_ref: Option<String>,
    text: Option<String>,
    value: Option<String>,
    key: Option<String>,
    scroll_x: Option<i64>,
    scroll_y: Option<i64>,
    condition: Option<serde_json::Value>,
    timeout_ms: Option<u64>,
    after_diagnostic_cursor: Option<u64>,
    query: Option<String>,
    offset: Option<usize>,
}

struct BrowserTab {
    tab: Arc<Tab>,
    allow_loopback: Arc<AtomicBool>,
    diagnostics: Arc<Mutex<BrowserDiagnostics>>,
    blocked_requests: Arc<AtomicUsize>,
}

#[derive(Default)]
struct BrowserDiagnostics {
    entries: VecDeque<(u64, serde_json::Value)>,
    cursor: u64,
}

impl BrowserDiagnostics {
    fn push(&mut self, entry: serde_json::Value) {
        self.cursor = self.cursor.saturating_add(1);
        self.entries.push_back((self.cursor, entry));
        while self.entries.len() > 500 {
            self.entries.pop_front();
        }
    }

    fn after(&self, cursor: u64) -> Vec<serde_json::Value> {
        self.entries
            .iter()
            .filter(|(seq, _)| *seq > cursor)
            .map(|(seq, event)| serde_json::json!({ "seq": seq, "event": event }))
            .collect()
    }
}

struct BrowserSession {
    // Attached over its own CDP connection; this handle does not own the
    // process. Closing the separate owner can interrupt even new_tab calls.
    browser: headless_chrome::Browser,
    tabs: HashMap<String, BrowserTab>,
    active_tab_id: String,
    observations: HashMap<String, BrowserObservation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ObservedElement {
    #[serde(rename = "ref")]
    element_ref: String,
    index: usize,
    tag: String,
    role: String,
    name: String,
    enabled: bool,
    visible: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    input_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    value_truncated: Option<bool>,
    bounds: [f64; 4],
}

#[derive(Debug, Clone)]
struct BrowserObservation {
    created_at: Instant,
    tab_id: String,
    url: String,
    content_hash: String,
    elements: Vec<ObservedElement>,
    options: BrowserObservationOptions,
}

struct ObservationCapture {
    data: serde_json::Value,
    screenshot: Vec<u8>,
}

fn oldest_observation_id(observations: &HashMap<String, BrowserObservation>) -> Option<String> {
    observations
        .iter()
        .min_by_key(|(_, observation)| observation.created_at)
        .map(|(observation_id, _)| observation_id.clone())
}

/// The resource owns admission and lifetime separately from the transport lock.
/// Close revokes queued work before waiting for the current operation and drops
/// the owned browser before acknowledging closure, even when wait tasks hold Arcs.
struct SessionResource<T, R = ()> {
    conversation_id: Option<String>,
    closing: AtomicBool,
    close_gate: Mutex<()>,
    transport: Mutex<Option<R>>,
    value: Mutex<Option<T>>,
}

struct SessionGuard<'a, T>(std::sync::MutexGuard<'a, Option<T>>);
impl<T> std::ops::Deref for SessionGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.0.as_ref().expect("active resource")
    }
}
impl<T> std::ops::DerefMut for SessionGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        self.0.as_mut().expect("active resource")
    }
}
impl<T, R> SessionResource<T, R> {
    fn new(value: T, conversation_id: Option<String>, transport: R) -> Self {
        Self {
            conversation_id,
            closing: AtomicBool::new(false),
            close_gate: Mutex::new(()),
            transport: Mutex::new(Some(transport)),
            value: Mutex::new(Some(value)),
        }
    }
    fn lock(&self) -> Result<SessionGuard<'_, T>, String> {
        if self.closing.load(Ordering::Acquire) {
            return Err("browser session is closing or closed".into());
        }
        let value = self
            .value
            .lock()
            .map_err(|_| "browser session is unavailable")?;
        if self.closing.load(Ordering::Acquire) || value.is_none() {
            return Err("browser session is closing or closed".into());
        }
        Ok(SessionGuard(value))
    }
    fn close(&self) -> Result<(), String> {
        self.closing.store(true, Ordering::Release);
        let _closing = self
            .close_gate
            .lock()
            .map_err(|_| "browser close is unavailable; resource remains registered")?;
        // Closing the transport first releases pending CDP calls. A worker may
        // still own the operation lock, so draining before shutdown can stall.
        let transport = self
            .transport
            .lock()
            .map_err(|_| "browser transport cleanup failed; resource remains registered")?
            .take();
        drop(transport);
        let owned = self
            .value
            .lock()
            .map_err(|_| "browser cleanup failed; resource remains registered")?
            .take();
        drop(owned);
        Ok(())
    }
}

type SharedSession = Arc<SessionResource<BrowserSession, headless_chrome::Browser>>;
type BrowserSessionRegistry = Arc<Mutex<HashMap<String, SharedSession>>>;

fn owned_session_resources<T, R>(
    sessions: &HashMap<String, Arc<SessionResource<T, R>>>,
    conversation_id: Option<&str>,
) -> Vec<(String, Arc<SessionResource<T, R>>)> {
    let mut owned = sessions
        .iter()
        .filter(|(_, resource)| {
            belongs_to_conversation(&resource.conversation_id, conversation_id)
                && !resource.closing.load(Ordering::Acquire)
        })
        .map(|(id, resource)| (id.clone(), Arc::clone(resource)))
        .collect::<Vec<_>>();
    owned.sort_by(|left, right| left.0.cmp(&right.0));
    owned
}

/// Opening has already succeeded. A failed capture must not invite the model to
/// create a duplicate tab/session or replay navigation to a page with side effects.
pub fn browser_open_observation_failure(
    call_id: &str,
    action: &str,
    session_id: &str,
    tab_id: &str,
    error: impl std::fmt::Display,
) -> ToolResult {
    let artifacts = serde_json::json!({
        "kind": "browserOpenReceipt",
        "action": action,
        "sessionId": session_id,
        "tabId": tab_id,
        "opened": true,
        "observationPending": true,
        "retrySafe": false,
        "sideEffect": "may_have_occurred",
        "recovery": { "action": "observe", "sessionId": session_id, "tabId": tab_id },
    });
    ToolResult {
        call_id: call_id.into(),
        content: format!(
            "Browser tab was opened, but its initial observation failed: {error}. Do not repeat {action}; use browser_session observe with sessionId={session_id} and tabId={tab_id} to inspect the existing page.\n{artifacts}"
        ),
        is_error: true,
        artifacts: Some(artifacts),
    }
}

/// Bind a completed opening capture to the same observation used for input.
/// The workflow accepts this receipt only for create_session/open_tab, never
/// a plain mutation acknowledgement or an arbitrary screenshot attachment.
pub fn mark_browser_open_observation(result: ToolResult, action: &str) -> ToolResult {
    let mut output = result.output_channels();
    if !result.is_error && matches!(action, "create_session" | "open_tab") {
        if let (Some(data), Some(artifacts)) = (&output.data, &mut output.artifacts) {
            artifacts["openingObservation"] = serde_json::json!({
                "action": action,
                "sessionId": data.get("sessionId"),
                "tabId": data.get("tabId"),
                "observationId": data.get("observationId"),
            });
        }
    }
    ToolResult::from_output(result.call_id, result.is_error, output)
}

fn belongs_to_conversation(owner: &Option<String>, conversation_id: Option<&str>) -> bool {
    owner.as_deref() == conversation_id
}

fn invalid(message: impl Into<String>) -> CoreError {
    CoreError::InvalidInput(message.into())
}

fn required<'a>(value: Option<&'a str>, field: &str) -> Result<&'a str, CoreError> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| invalid(format!("browser_session requires {field}")))
}

fn session_by_id(
    sessions: &BrowserSessionRegistry,
    session_id: &str,
    conversation_id: Option<&str>,
) -> Result<SharedSession, CoreError> {
    let session = sessions
        .lock()
        .map_err(|_| CoreError::Internal("browser session registry is unavailable".to_string()))?
        .get(session_id)
        .cloned()
        .ok_or_else(|| invalid(format!("Unknown browser session '{session_id}'")))?;
    let owned_by_conversation = belongs_to_conversation(&session.conversation_id, conversation_id);
    if !owned_by_conversation {
        return Err(invalid(
            "Browser session belongs to a different conversation. Create or list a session in the current conversation.",
        ));
    }
    Ok(session)
}

fn configure_tab(tab: Arc<Tab>) -> Result<BrowserTab, String> {
    let allow_loopback = Arc::new(AtomicBool::new(false));
    let diagnostics = Arc::new(Mutex::new(BrowserDiagnostics::default()));
    let blocked_requests = Arc::new(AtomicUsize::new(0));
    tab.set_default_timeout(Duration::from_secs(10));
    tab.enable_log()
        .and_then(|tab| tab.enable_runtime())
        .map_err(|error| format!("failed to enable browser diagnostics: {error}"))?;
    tab.call_method(Network::Enable {
        max_total_buffer_size: None,
        max_resource_buffer_size: None,
        max_post_data_size: None,
        report_direct_socket_traffic: None,
        enable_durable_messages: None,
    })
    .map_err(|error| format!("failed to enable browser network events: {error}"))?;
    let diagnostics_for_listener = Arc::clone(&diagnostics);
    tab.add_event_listener(Arc::new(move |event: &Event| {
        let entry = match event {
            Event::LogEntryAdded(event) => Some(serde_json::json!({
                "kind": "console",
                "message": event.params.entry.text,
            })),
            Event::RuntimeExceptionThrown(event) => Some(serde_json::json!({
                "kind": "pageError",
                "message": event.params.exception_details.text,
            })),
            Event::NetworkLoadingFailed(event) => Some(serde_json::json!({
                "kind": "networkFailure",
                "message": event.params.error_text,
            })),
            Event::NetworkResponseReceived(event) if event.params.response.status >= 400 => {
                Some(serde_json::json!({
                    "kind": "httpError",
                    "status": event.params.response.status,
                    "url": event.params.response.url,
                }))
            }
            _ => None,
        };
        if let (Some(entry), Ok(mut diagnostics)) = (entry, diagnostics_for_listener.lock()) {
            diagnostics.push(entry);
        }
    }))
    .map_err(|error| format!("failed to install browser event listener: {error}"))?;

    tab.enable_fetch(
        Some(&[RequestPattern {
            url_pattern: None,
            resource_Type: None,
            request_stage: Some(RequestStage::Request),
        }]),
        None,
    )
    .map_err(|error| format!("failed to enable browser request validation: {error}"))?;
    let request_cache = Arc::new(Mutex::new(HashMap::<String, bool>::new()));
    let loopback_for_interceptor = Arc::clone(&allow_loopback);
    let blocked_for_interceptor = Arc::clone(&blocked_requests);
    tab.enable_request_interception(Arc::new(
        move |_transport, _session_id, intercepted: RequestPausedEvent| {
            if browser_request_allowed(
                &intercepted.params.request.url,
                request_cache.as_ref(),
                loopback_for_interceptor.load(Ordering::Relaxed),
            ) {
                RequestPausedDecision::Continue(None)
            } else {
                blocked_for_interceptor.fetch_add(1, Ordering::Relaxed);
                RequestPausedDecision::Fail(FailRequest {
                    request_id: intercepted.params.request_id,
                    error_reason: Network::ErrorReason::BlockedByClient,
                })
            }
        },
    ))
    .map_err(|error| format!("failed to install browser request validator: {error}"))?;

    Ok(BrowserTab {
        tab,
        allow_loopback,
        diagnostics,
        blocked_requests,
    })
}

fn evaluate_json(tab: &Tab, expression: &str) -> Result<serde_json::Value, String> {
    // headless_chrome::Tab::evaluate uses returnByValue=false: objects/arrays
    // are remote handles, not RemoteObject.value. Serialize in the page so the
    // transport returns a primitive and no object handle is leaked.
    let result = tab
        .evaluate(&format!("JSON.stringify(({expression}))"), false)
        .map_err(|error| format!("failed to inspect browser data: {error}"))?;
    let encoded = result
        .value
        .as_ref()
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "browser observation did not return serialized data".to_string())?;
    serde_json::from_str(encoded)
        .map_err(|error| format!("invalid browser observation data: {error}"))
}

fn interactive_elements(tab: &Tab) -> Result<Vec<ObservedElement>, String> {
    let selector = serde_json::to_string(INTERACTIVE_SELECTOR).unwrap_or_default();
    let expression = format!("({INTERACTIVE_ELEMENTS_SCRIPT})({selector})");
    let value = evaluate_json(tab, &expression)?;
    serde_json::from_value(value)
        .map_err(|error| format!("failed to decode browser elements: {error}"))
}

fn filter_observed_elements(
    elements: Vec<ObservedElement>,
    options: &BrowserObservationOptions,
    viewport: &serde_json::Value,
) -> (Vec<ObservedElement>, BrowserObservationCoverage) {
    let query = options.query.as_deref().unwrap_or_default().trim();
    let normalized_query = query.to_lowercase();
    let mut matches: Vec<_> = elements
        .into_iter()
        .filter(|element| {
            (element.visible || element.input_type.as_deref() == Some("file"))
                && (query.is_empty()
                    || format!("{} {} {}", element.name, element.role, element.tag)
                        .to_lowercase()
                        .contains(&normalized_query))
        })
        .collect();
    let width = viewport["width"].as_f64().unwrap_or_default();
    let height = viewport["height"].as_f64().unwrap_or_default();
    matches.sort_by_key(|element| {
        let [x, y, w, h] = element.bounds;
        let in_viewport =
            w > 0.0 && h > 0.0 && x < width && y < height && x + w > 0.0 && y + h > 0.0;
        (!in_viewport, element.index)
    });
    let total_matches = matches.len();
    let elements: Vec<_> = matches.into_iter().skip(options.offset).take(300).collect();
    let next = options.offset.saturating_add(elements.len());
    let coverage = BrowserObservationCoverage {
        query: query.to_string(),
        offset: options.offset,
        returned: elements.len(),
        total_matches,
        has_more: next < total_matches,
        next_offset: (next < total_matches).then_some(next),
    };
    (elements, coverage)
}

fn observe_tab(
    session: &mut BrowserSession,
    tab_id: &str,
    after_diagnostic_cursor: u64,
    options: &BrowserObservationOptions,
) -> Result<ObservationCapture, String> {
    let browser_tab = session
        .tabs
        .get(tab_id)
        .ok_or_else(|| format!("Unknown browser tab '{tab_id}'"))?;
    let url = browser_tab.tab.get_url();
    let title = browser_tab.tab.get_title().unwrap_or_default();
    let html = browser_tab
        .tab
        .get_content()
        .map_err(|error| format!("failed to read browser DOM: {error}"))?;
    let screenshot = browser_tab
        .tab
        .capture_screenshot(Page::CaptureScreenshotFormatOption::Png, None, None, true)
        .map_err(|error| format!("failed to capture browser screenshot: {error}"))?;
    let elements = interactive_elements(&browser_tab.tab)?;
    let viewport = evaluate_json(
        &browser_tab.tab,
        "({width:innerWidth,height:innerHeight,deviceScaleFactor:devicePixelRatio})",
    )?;
    let (elements, coverage) = filter_observed_elements(elements, options, &viewport);
    let frame_limitations = evaluate_json(&browser_tab.tab, FRAME_LIMITATIONS_SCRIPT)?;
    let text = browser_tab
        .tab
        .evaluate(
            "document.body ? document.body.innerText.slice(0,20000) : ''",
            false,
        )
        .ok()
        .and_then(|result| result.value)
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_default();
    let content_hash = blake3::hash(format!("{url}\n{html}").as_bytes())
        .to_hex()
        .to_string();
    let screenshot_hash = blake3::hash(&screenshot).to_hex().to_string();
    let observation_id = format!("obs_{}", uuid::Uuid::new_v4());
    let (diagnostic_cursor, diagnostics) = browser_tab
        .diagnostics
        .lock()
        .map(|log| (log.cursor, log.after(after_diagnostic_cursor)))
        .unwrap_or_default();
    let data = serde_json::json!({
        "observationId": observation_id,
        "tabId": tab_id,
        "url": url,
        "title": title,
        "viewport": viewport,
        "contentHash": content_hash,
        "screenshotHash": screenshot_hash,
        "elements": elements,
        "observationCoverage": coverage,
        "frameLimitations": frame_limitations,
        "accessibilityTree": elements,
        "consoleAndNetworkCursor": diagnostic_cursor,
        "diagnostics": diagnostics,
        "blockedRequests": browser_tab.blocked_requests.load(Ordering::Relaxed),
        "text": text,
    });
    session.observations.insert(
        observation_id.clone(),
        BrowserObservation {
            created_at: Instant::now(),
            tab_id: tab_id.to_string(),
            url,
            content_hash,
            elements,
            options: options.clone(),
        },
    );
    if session.observations.len() > MAX_OBSERVATIONS {
        if let Some(oldest) = oldest_observation_id(&session.observations) {
            session.observations.remove(&oldest);
        }
    }
    Ok(ObservationCapture { data, screenshot })
}

fn bounds_stable(before: &[f64; 4], after: &[f64; 4]) -> bool {
    before
        .iter()
        .zip(after)
        .all(|(left, right)| (left - right).abs() <= 4.0)
}

fn validated_element(
    session: &BrowserSession,
    tab_id: &str,
    observation_id: &str,
    target_ref: &str,
) -> Result<(Arc<Tab>, usize), String> {
    let observation = session
        .observations
        .get(observation_id)
        .ok_or_else(|| "stale observation: observe the tab again".to_string())?;
    if observation.created_at.elapsed() > Duration::from_secs(120) {
        return Err("stale observation: observation expired".to_string());
    }
    if observation.tab_id != tab_id {
        return Err("stale observation: tab changed".to_string());
    }
    let browser_tab = session
        .tabs
        .get(tab_id)
        .ok_or_else(|| format!("Unknown browser tab '{tab_id}'"))?;
    if browser_tab.tab.get_url() != observation.url {
        return Err("stale observation: page navigated".to_string());
    }
    let expected = observation
        .elements
        .iter()
        .find(|element| element.element_ref == target_ref)
        .ok_or_else(|| format!("Unknown targetRef '{target_ref}'"))?;
    let current = interactive_elements(&browser_tab.tab)?;
    let actual = current
        .get(expected.index)
        .ok_or_else(|| "stale observation: target disappeared".to_string())?;
    if !actual.enabled
        || !actual.visible
        || actual.tag != expected.tag
        || actual.role != expected.role
        || actual.name != expected.name
        || !bounds_stable(&actual.bounds, &expected.bounds)
    {
        return Err("stale observation: target identity or bounds changed".to_string());
    }
    Ok((Arc::clone(&browser_tab.tab), expected.index))
}

fn validated_observation(
    session: &BrowserSession,
    tab_id: &str,
    observation_id: &str,
) -> Result<Arc<Tab>, String> {
    let observation = session
        .observations
        .get(observation_id)
        .ok_or_else(|| "stale observation: observe the tab again".to_string())?;
    if observation.created_at.elapsed() > Duration::from_secs(120) {
        return Err("stale observation: observation expired".to_string());
    }
    if observation.tab_id != tab_id {
        return Err("stale observation: tab changed".to_string());
    }
    let browser_tab = session
        .tabs
        .get(tab_id)
        .ok_or_else(|| format!("Unknown browser tab '{tab_id}'"))?;
    if browser_tab.tab.get_url() != observation.url {
        return Err("stale observation: page navigated".to_string());
    }
    let current_hash = blake3::hash(
        format!(
            "{}\n{}",
            browser_tab.tab.get_url(),
            browser_tab.tab.get_content().unwrap_or_default()
        )
        .as_bytes(),
    )
    .to_hex()
    .to_string();
    if current_hash != observation.content_hash {
        return Err("stale observation: page content changed".to_string());
    }
    Ok(Arc::clone(&browser_tab.tab))
}

fn check_condition(session: &BrowserSession, tab_id: &str, condition: &serde_json::Value) -> bool {
    let Some(browser_tab) = session.tabs.get(tab_id) else {
        return false;
    };
    let kind = condition
        .get("type")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    match kind {
        "page_loaded" => browser_tab
            .tab
            .evaluate("document.readyState === 'complete'", false)
            .ok()
            .and_then(|result| result.value)
            .and_then(|value| value.as_bool())
            .unwrap_or(false),
        "text_present" => condition
            .get("text")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|text| {
                let text = serde_json::to_string(text).unwrap_or_default();
                browser_tab
                    .tab
                    .evaluate(
                        &format!("document.body && document.body.innerText.includes({text})"),
                        false,
                    )
                    .ok()
                    .and_then(|result| result.value)
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false)
            }),
        "url_matches" => condition
            .get("pattern")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|pattern| browser_tab.tab.get_url().contains(pattern)),
        "selector_visible" | "selector_hidden" => condition
            .get("selector")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|selector| {
                let selector = serde_json::to_string(selector).unwrap_or_default();
                let visible = browser_tab
                    .tab
                    .evaluate(
                        &format!("(()=>{{const e=document.querySelector({selector});if(!e)return false;const r=e.getBoundingClientRect();return r.width>0&&r.height>0&&getComputedStyle(e).visibility!=='hidden'}})()"),
                        false,
                    )
                    .ok()
                    .and_then(|result| result.value)
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false);
                if kind == "selector_hidden" { !visible } else { visible }
            }),
        "console_error" => browser_tab.diagnostics.lock().ok().is_some_and(|log| {
            let after_cursor = condition
                .get("afterCursor")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            log.entries.iter().any(|(seq, entry)| {
                *seq > after_cursor &&
                matches!(
                    entry.get("kind").and_then(serde_json::Value::as_str),
                    Some("pageError" | "httpError" | "networkFailure")
                )
            })
        }),
        _ => false,
    }
}

async fn blocking<T: Send + 'static>(
    operation: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, CoreError> {
    tokio::task::spawn_blocking(operation)
        .await
        .map_err(|error| CoreError::Internal(format!("browser worker failed: {error}")))?
        .map_err(invalid)
}

#[derive(Clone, Default)]
pub struct BrowserSessionTool {
    sessions: BrowserSessionRegistry,
}

pub fn browser_session_action_schema_variants(
    actions: &[&str],
    session_optional_actions: &[&str],
) -> serde_json::Value {
    let optional_actions = actions
        .iter()
        .copied()
        .filter(|action| session_optional_actions.contains(action))
        .collect::<Vec<_>>();
    let session_required_actions = actions
        .iter()
        .copied()
        .filter(|action| !session_optional_actions.contains(action) && *action != "close_tab")
        .collect::<Vec<_>>();
    serde_json::json!([
        {
            "type": "object",
            "properties": { "action": { "enum": optional_actions } }
        },
        {
            "type": "object",
            "properties": { "action": { "enum": session_required_actions } },
            "required": ["sessionId"]
        },
        {
            "type": "object",
            "properties": { "action": { "enum": ["close_tab"] } },
            "required": ["sessionId", "tabId"]
        }
    ])
}

#[async_trait]
impl Tool for BrowserSessionTool {
    fn name(&self) -> &str {
        "browser_session"
    }

    fn description(&self) -> &str {
        "Operate a persistent, isolated Chromium session with stable session/tab IDs. Use list_sessions to recover existing sessions in this conversation. create_session or open_tab with url opens the page and returns its initial screenshot and observation-scoped refs in one call. observe atomically returns screenshot, text, URL, viewport, diagnostics, and semantic element refs. Interactions reject stale observations. wait_for is condition-based and becomes a browser Activity instead of blocking the agent."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        let actions = [
            "create_session",
            "list_sessions",
            "list_tabs",
            "open_tab",
            "activate_tab",
            "navigate",
            "observe",
            "click",
            "type",
            "select",
            "press",
            "scroll",
            "wait_for",
            "close_tab",
            "close_session",
        ];
        let action_variants =
            browser_session_action_schema_variants(&actions, &["create_session", "list_sessions"]);
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": actions },
                "sessionId": { "type": "string", "description": "Explicit session target. Required for close_session and close_tab so terminal receipts bind the exact requested target." },
                "tabId": { "type": "string", "description": "Explicit tab target. Required for close_tab so a final-tab receipt binds the exact requested target." },
                "url": { "type": "string", "description": "URL for navigate, create_session, or open_tab. Creating with a URL returns the initial page observation; use its returned sessionId, tabId and observationId directly." },
                "observationId": { "type": "string" },
                "targetRef": { "type": "string" },
                "query": { "type": "string", "maxLength": 240, "description": "observe only: case-insensitive substring filter on accessible name, role, or tag. Use this to locate controls omitted from a large observation; the result returns fresh refs and coverage." },
                "offset": { "type": "integer", "minimum": 0, "maximum": 9007199254740991_u64, "description": "observe only: start at this matching-control offset, using observationCoverage.nextOffset to continue. Keep the same query and page viewport while paging." },
                "text": { "type": "string" },
                "value": { "type": "string" },
                "key": { "type": "string" },
                "scrollX": { "type": "integer", "default": 0 },
                "scrollY": { "type": "integer", "default": 0 },
                "condition": { "type": "object", "description": "Condition type: page_loaded, text_present, url_matches, selector_visible, selector_hidden, or console_error." },
                "afterDiagnosticCursor": { "type": "integer", "minimum": 0, "description": "Return only console/network diagnostics newer than this cursor." },
                "timeoutMs": { "type": "integer", "minimum": 1, "maximum": MAX_WAIT_MS, "default": 15000 }
            },
            "required": ["action"],
            "oneOf": action_variants,
            "additionalProperties": false
        })
    }

    fn categories(&self) -> &'static [ToolCategory] {
        &[
            ToolCategory::BrowserRead,
            ToolCategory::VisualObservation,
            ToolCategory::BrowserInteract,
        ]
    }

    fn requires_confirmation(&self, args: &serde_json::Value) -> bool {
        args.get("action")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|action| {
                matches!(
                    action,
                    "click" | "type" | "select" | "press" | "close_tab" | "close_session"
                )
            })
    }

    fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool {
        // Observations also replace the session's action references.
        false
    }

    fn is_read_only(&self, args: &serde_json::Value) -> bool {
        !self.requires_confirmation(args)
    }

    async fn execute(&self, context: ToolExecutionContext<'_>) -> Result<ToolResult, CoreError> {
        let ToolExecutionContext {
            call_id,
            arguments,
            conversation_id,
            activity_runtime,
            ..
        } = context;
        let args: BrowserArgs = serde_json::from_str(arguments)
            .map_err(|error| invalid(format!("Invalid browser_session arguments: {error}")))?;
        let action = args.action.trim().to_ascii_lowercase();
        let observation_options = BrowserObservationOptions {
            query: args.query.clone(),
            offset: args.offset.unwrap_or(0),
        };
        observation_options.validate().map_err(invalid)?;
        if action != "observe" && (args.query.is_some() || args.offset.is_some()) {
            return Err(invalid(
                "query and offset are supported only by browser_session observe",
            ));
        }

        if action == "list_sessions" {
            let owned = owned_session_resources(
                &*self.sessions.lock().map_err(|_| {
                    CoreError::Internal("browser session registry is unavailable".into())
                })?,
                conversation_id,
            );
            // Discovery is also the recovery path for pending operations. Never
            // wait for a CDP operation lock merely to rediscover a session ID.
            let sessions = owned
                .into_iter()
                .filter_map(|(id, resource)| {
                    if resource.closing.load(Ordering::Acquire) {
                        return None;
                    }
                    let snapshot = match resource.value.try_lock() {
                        Ok(value) => value.as_ref().map(|session| {
                            serde_json::json!({
                                "id": id,
                                "activeTabId": session.active_tab_id,
                                "tabCount": session.tabs.len(),
                                "status": "ready",
                            })
                        }),
                        Err(std::sync::TryLockError::WouldBlock) => {
                            Some(serde_json::json!({ "id": id, "status": "busy" }))
                        }
                        Err(std::sync::TryLockError::Poisoned(_)) => {
                            Some(serde_json::json!({ "id": id, "status": "unavailable" }))
                        }
                    };
                    snapshot
                })
                .collect::<Vec<_>>();
            let artifacts = serde_json::json!({ "kind": "browserSessions", "sessions": sessions });
            return Ok(ToolResult {
                call_id: call_id.into(),
                content: serde_json::to_string_pretty(&artifacts)?,
                is_error: false,
                artifacts: Some(artifacts),
            });
        }

        if action == "create_session" {
            // Validate before allocating a browser; never silently discard a URL.
            let validated_url = match args.url.as_deref() {
                Some(url) => Some(
                    validate_url_for_browser_capture(url)
                        .await
                        .map_err(invalid)?,
                ),
                None => None,
            };
            let session_id = format!("browser_{}", uuid::Uuid::new_v4());
            let tab_id = format!("tab_{}", uuid::Uuid::new_v4());
            let session_id_for_worker = session_id.clone();
            let tab_id_for_worker = tab_id.clone();
            let conversation_id_for_worker = conversation_id.map(str::to_string);
            let (session, browser) = blocking(move || {
                let browser = launch_browser_for_capture()?;
                let operation_browser = headless_chrome::Browser::connect_with_timeout(
                    browser.get_ws_url(),
                    Duration::from_secs(30),
                )
                .map_err(|error| format!("failed to attach browser operations: {error}"))?;
                let tab = configure_tab(
                    operation_browser
                        .new_tab()
                        .map_err(|error| error.to_string())?,
                )?;
                let session = BrowserSession {
                    browser: operation_browser,
                    tabs: HashMap::from([(tab_id_for_worker.clone(), tab)]),
                    active_tab_id: tab_id_for_worker,
                    observations: HashMap::new(),
                };
                Ok((session, browser))
            })
            .await?;
            let resource = Arc::new(SessionResource::new(
                session,
                conversation_id_for_worker,
                browser,
            ));
            self.sessions
                .lock()
                .map_err(|_| CoreError::Internal("browser session registry is unavailable".into()))?
                .insert(session_id_for_worker, Arc::clone(&resource));
            if let Some(url) = validated_url {
                let tab_id_for_worker = tab_id.clone();
                let capture = blocking(move || {
                    let mut session = resource.lock()?;
                    let tab = &session.tabs[&tab_id_for_worker];
                    tab.allow_loopback
                        .store(is_loopback_url(&url), Ordering::Relaxed);
                    super::browser_navigation::navigate_to_document(
                        &tab.tab,
                        url.as_str(),
                        Duration::from_secs(10),
                    )?;
                    observe_tab(
                        &mut session,
                        &tab_id_for_worker,
                        0,
                        &BrowserObservationOptions::default(),
                    )
                })
                .await;
                return match capture {
                    Ok(capture) => observation_result(call_id, &session_id, &tab_id, capture)
                        .map(|result| mark_browser_open_observation(result, &action)),
                    Err(error) => Ok(browser_open_observation_failure(
                        call_id,
                        &action,
                        &session_id,
                        &tab_id,
                        error,
                    )),
                };
            }
            return Ok(ToolResult {
                call_id: call_id.to_string(),
                content: format!("Created browser session {session_id} with tab {tab_id}."),
                is_error: false,
                artifacts: Some(
                    serde_json::json!({ "kind": "browserSession", "sessionId": session_id, "tabId": tab_id }),
                ),
            });
        }

        let session_id = required(args.session_id.as_deref(), "sessionId")?.to_string();
        if action == "close_session" {
            let resource = session_by_id(&self.sessions, &session_id, conversation_id)?;
            // Revoke admission before queueing cleanup. No registry lock spans
            // transport shutdown or an in-flight input operation.
            resource.closing.store(true, Ordering::Release);
            blocking(move || resource.close()).await?;
            self.sessions
                .lock()
                .map_err(|_| CoreError::Internal("browser session registry is unavailable".into()))?
                .remove(&session_id);
            return Ok(ToolResult {
                call_id: call_id.to_string(),
                content: format!("Closed browser session {session_id}."),
                is_error: false,
                artifacts: Some(serde_json::json!({
                    "kind": "browserSessionClosed",
                    "sessionId": session_id,
                    "sessionClosed": true
                })),
            });
        }
        let session = session_by_id(&self.sessions, &session_id, conversation_id)?;

        if action == "list_tabs" {
            let session_for_worker = Arc::clone(&session);
            let tabs = blocking(move || {
                let session = session_for_worker.lock().map_err(|_| "browser session is unavailable".to_string())?;
                Ok(session.tabs.iter().map(|(tab_id, tab)| serde_json::json!({ "tabId": tab_id, "url": tab.tab.get_url(), "title": tab.tab.get_title().unwrap_or_default(), "active": tab_id == &session.active_tab_id })).collect::<Vec<_>>())
            }).await?;
            return Ok(ToolResult {
                call_id: call_id.to_string(),
                content: serde_json::to_string_pretty(&tabs)?,
                is_error: false,
                artifacts: Some(
                    serde_json::json!({ "kind": "browserTabs", "sessionId": session_id, "tabs": tabs }),
                ),
            });
        }

        if action == "open_tab" {
            let validated_url = match args.url.as_deref() {
                Some(url) => Some(
                    validate_url_for_browser_capture(url)
                        .await
                        .map_err(invalid)?,
                ),
                None => None,
            };
            let tab_id = format!("tab_{}", uuid::Uuid::new_v4());
            let tab_id_for_worker = tab_id.clone();
            let session_for_worker = Arc::clone(&session);
            blocking(move || {
                let mut session = session_for_worker
                    .lock()
                    .map_err(|_| "browser session is unavailable".to_string())?;
                let browser_tab = configure_tab(
                    session
                        .browser
                        .new_tab()
                        .map_err(|error| error.to_string())?,
                )?;
                session.tabs.insert(tab_id_for_worker.clone(), browser_tab);
                session.active_tab_id = tab_id_for_worker;
                Ok(())
            })
            .await?;
            if let Some(url) = validated_url {
                let resource = Arc::clone(&session);
                let tab_id_for_worker = tab_id.clone();
                let capture = blocking(move || {
                    let mut session = resource.lock()?;
                    let tab = &session.tabs[&tab_id_for_worker];
                    tab.allow_loopback
                        .store(is_loopback_url(&url), Ordering::Relaxed);
                    super::browser_navigation::navigate_to_document(
                        &tab.tab,
                        url.as_str(),
                        Duration::from_secs(10),
                    )?;
                    observe_tab(
                        &mut session,
                        &tab_id_for_worker,
                        0,
                        &BrowserObservationOptions::default(),
                    )
                })
                .await;
                return match capture {
                    Ok(capture) => observation_result(call_id, &session_id, &tab_id, capture)
                        .map(|result| mark_browser_open_observation(result, &action)),
                    Err(error) => Ok(browser_open_observation_failure(
                        call_id,
                        &action,
                        &session_id,
                        &tab_id,
                        error,
                    )),
                };
            }
            return Ok(ToolResult {
                call_id: call_id.to_string(),
                content: format!("Opened browser tab {tab_id}."),
                is_error: false,
                artifacts: Some(
                    serde_json::json!({ "kind": "browserTab", "sessionId": session_id, "tabId": tab_id }),
                ),
            });
        }

        let tab_id = if action == "close_tab" {
            required(args.tab_id.as_deref(), "tabId")?.to_string()
        } else if let Some(tab_id) = args.tab_id.clone() {
            tab_id
        } else {
            let resource = Arc::clone(&session);
            blocking(move || Ok(resource.lock()?.active_tab_id.clone())).await?
        };
        let tab_id = required(Some(&tab_id), "tabId")?.to_string();

        if action == "activate_tab" {
            let session_for_worker = Arc::clone(&session);
            let tab_id_for_worker = tab_id.clone();
            blocking(move || {
                let mut session = session_for_worker
                    .lock()
                    .map_err(|_| "browser session is unavailable".to_string())?;
                let tab = session
                    .tabs
                    .get(&tab_id_for_worker)
                    .ok_or_else(|| format!("Unknown browser tab '{tab_id_for_worker}'"))?;
                tab.tab
                    .bring_to_front()
                    .map_err(|error| error.to_string())?;
                session.active_tab_id = tab_id_for_worker;
                Ok(())
            })
            .await?;
            return Ok(ToolResult {
                call_id: call_id.to_string(),
                content: format!("Activated browser tab {tab_id}."),
                is_error: false,
                artifacts: None,
            });
        }

        if action == "navigate" {
            let url = validate_url_for_browser_capture(required(args.url.as_deref(), "url")?)
                .await
                .map_err(invalid)?;
            let session_for_worker = Arc::clone(&session);
            let tab_id_for_worker = tab_id.clone();
            blocking(move || {
                let session = session_for_worker
                    .lock()
                    .map_err(|_| "browser session is unavailable".to_string())?;
                let tab = session
                    .tabs
                    .get(&tab_id_for_worker)
                    .ok_or_else(|| format!("Unknown browser tab '{tab_id_for_worker}'"))?;
                tab.allow_loopback
                    .store(is_loopback_url(&url), Ordering::Relaxed);
                super::browser_navigation::navigate_to_document(
                    &tab.tab,
                    url.as_str(),
                    Duration::from_secs(10),
                )?;
                Ok(())
            })
            .await?;
        }

        if action == "wait_for" {
            let condition = args
                .condition
                .clone()
                .ok_or_else(|| invalid("browser_session wait_for requires condition"))?;
            let runtime = activity_runtime.ok_or_else(|| {
                CoreError::Internal("Activity Runtime is unavailable".to_string())
            })?;
            let mut spec = ActivitySpec::new(ActivitySurface::Browser, "browser_session")
                .with_activity_id(call_id)
                .with_session_id(&session_id);
            if let Some(conversation_id) = conversation_id {
                spec = spec.with_conversation_id(conversation_id);
            }
            let record = runtime.start(spec)?;
            let runtime_for_task = runtime.clone();
            let session_for_task = Arc::clone(&session);
            let activity_id = call_id.to_string();
            let tab_id_for_task = tab_id.clone();
            let timeout =
                Duration::from_millis(args.timeout_ms.unwrap_or(15_000).clamp(1, MAX_WAIT_MS));
            tokio::spawn(async move {
                let started = Instant::now();
                loop {
                    if session_for_task.closing.load(Ordering::Acquire) {
                        let _ = runtime_for_task.transition(
                            &activity_id,
                            ActivityState::Cancelled,
                            serde_json::json!({ "reason": "session_closed" }),
                        );
                        return;
                    }
                    let session_for_check = Arc::clone(&session_for_task);
                    let condition_for_check = condition.clone();
                    let tab_id_for_check = tab_id_for_task.clone();
                    let matched = tokio::task::spawn_blocking(move || {
                        session_for_check.lock().ok().is_some_and(|session| {
                            check_condition(&session, &tab_id_for_check, &condition_for_check)
                        })
                    })
                    .await
                    .unwrap_or(false);
                    if matched {
                        let _ = runtime_for_task.append(
                            &activity_id,
                            ActivityEventKind::BrowserObservation,
                            serde_json::json!({ "condition": condition, "matched": true }),
                        );
                        let _ = runtime_for_task.transition(
                            &activity_id,
                            ActivityState::Completed,
                            serde_json::json!({ "condition": condition }),
                        );
                        return;
                    }
                    if started.elapsed() >= timeout {
                        let _ = runtime_for_task.transition(&activity_id, ActivityState::Failed, serde_json::json!({ "reason": "condition_timeout", "condition": condition }));
                        return;
                    }
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            });
            let observation = runtime
                .observe(
                    call_id,
                    record.last_event_seq,
                    Duration::from_millis(OBSERVE_QUANTUM_MS),
                )
                .await?;
            return Ok(ToolResult {
                call_id: call_id.to_string(),
                content: serde_json::to_string_pretty(&observation)?,
                is_error: false,
                artifacts: Some(
                    serde_json::json!({ "kind": "browserActivity", "activity": observation }),
                ),
            });
        }

        if action == "close_tab" {
            let session_for_worker = Arc::clone(&session);
            let tab_id_for_worker = tab_id.clone();
            let remaining_tab_count = blocking(move || {
                let mut session = session_for_worker
                    .lock()
                    .map_err(|_| "browser session is unavailable".to_string())?;
                let tab = session
                    .tabs
                    .get(&tab_id_for_worker)
                    .ok_or_else(|| format!("Unknown browser tab '{tab_id_for_worker}'"))?;
                let closed = tab
                    .tab
                    .close_target()
                    .map_err(|error| format!("failed to close browser tab: {error}"))?;
                if !closed {
                    return Err(
                        "browser target rejected close; the tab remains registered".to_string()
                    );
                }
                session
                    .tabs
                    .remove(&tab_id_for_worker)
                    .expect("successfully closed browser tab must remain registered until commit");
                if session.active_tab_id == tab_id_for_worker {
                    session.active_tab_id = session.tabs.keys().next().cloned().unwrap_or_default();
                }
                Ok(session.tabs.len())
            })
            .await?;
            return Ok(ToolResult {
                call_id: call_id.to_string(),
                content: format!("Closed browser tab {tab_id}."),
                is_error: false,
                artifacts: Some(serde_json::json!({
                    "kind": "browserTabClosed",
                    "sessionId": session_id,
                    "tabId": tab_id,
                    "tabClosed": true,
                    "remainingTabCount": remaining_tab_count
                })),
            });
        }

        let after_diagnostic_cursor = args.after_diagnostic_cursor.unwrap_or(0);
        let capture_after_action = if matches!(
            action.as_str(),
            "click" | "type" | "select" | "press" | "scroll"
        ) {
            let observation_id = args.observation_id.clone();
            let target_ref = args.target_ref.clone();
            let text = args.text.clone();
            let value = args.value.clone();
            let key = args.key.clone();
            let scroll_x = args.scroll_x.unwrap_or(0);
            let scroll_y = args.scroll_y.unwrap_or(0);
            let action_for_worker = action.clone();
            let session_for_worker = Arc::clone(&session);
            let tab_id_for_worker = tab_id.clone();
            let (attempt, dispatched) = blocking(move || {
                let mut session = session_for_worker.lock().map_err(|_| "browser session is unavailable".to_string())?;
                let observation_id = required_string(observation_id.as_deref(), "observationId")?;
                let post_action_options = session.observations.get(observation_id).map(|observation| observation.options.clone()).unwrap_or_default();
                let mut dispatched = false;
                let attempt = (|| -> Result<ObservationCapture, String> {
                match action_for_worker.as_str() {
                    "press" => {
                        let tab = validated_observation(&session, &tab_id_for_worker, observation_id)?;
                        let key = required_string(key.as_deref(), "key")?;
                        session.observations.retain(|_, observation| observation.tab_id != tab_id_for_worker);
                        dispatched = true;
                        tab.press_key(key).map_err(|error| error.to_string())?;
                    }
                    "scroll" => {
                        let tab = validated_observation(&session, &tab_id_for_worker, observation_id)?;
                        session.observations.retain(|_, observation| observation.tab_id != tab_id_for_worker);
                        dispatched = true;
                        tab.evaluate(&format!("window.scrollBy({scroll_x},{scroll_y})"), false).map_err(|error| error.to_string())?;
                    }
                    _ => {
                        let target_ref = required_string(target_ref.as_deref(), "targetRef")?;
                        let (tab, index) = validated_element(&session, &tab_id_for_worker, observation_id, target_ref)?;
                        let elements = tab.find_elements(INTERACTIVE_SELECTOR).map_err(|error| error.to_string())?;
                        let element = elements.get(index).ok_or_else(|| "stale observation: target disappeared".to_string())?;
                        if action_for_worker == "type" { required_string(text.as_deref(), "text")?; }
                        if action_for_worker == "select" { required_string(value.as_deref(), "value")?; }
                        session.observations.retain(|_, observation| observation.tab_id != tab_id_for_worker);
                        dispatched = true;
                        match action_for_worker.as_str() {
                            "click" => { element.click().map_err(|error| error.to_string())?; }
                            "type" => { element.type_into(required_string(text.as_deref(), "text")?).map_err(|error| error.to_string())?; }
                            "select" => { element.call_js_fn("function(value){this.value=value;this.dispatchEvent(new Event('input',{bubbles:true}));this.dispatchEvent(new Event('change',{bubbles:true}));}", vec![serde_json::json!(required_string(value.as_deref(), "value")?)], false).map_err(|error| error.to_string())?; }
                            _ => {}
                        }
                    }
                }
                observe_tab(&mut session, &tab_id_for_worker, after_diagnostic_cursor, &post_action_options)
                })();
                Ok((attempt, dispatched))
            }).await?;
            match attempt {
                Ok(mut capture) => {
                    capture.data["actionReceipt"] = serde_json::json!({ "callId": call_id, "action": action, "status": "observed_after_action", "observationConsumed": true });
                    Some(capture)
                }
                Err(error) => {
                    let recovery = if dispatched {
                        "Input may have reached the page. Observe again before deciding whether any further action is needed; do not repeat this input automatically."
                    } else {
                        "Input was not dispatched. Observe again and use a current target."
                    };
                    return Ok(ToolResult {
                        call_id: call_id.into(),
                        is_error: true,
                        content: format!("{error}. {recovery}"),
                        artifacts: Some(
                            serde_json::json!({ "kind": "browserActionReceipt", "sessionId": session_id, "tabId": tab_id, "callId": call_id, "action": action, "status": if dispatched { "uncertain" } else { "not_dispatched" }, "observationConsumed": dispatched, "retrySafe": !dispatched }),
                        ),
                    });
                }
            }
        } else {
            None
        };

        if !matches!(
            action.as_str(),
            "observe" | "navigate" | "click" | "type" | "select" | "press" | "scroll"
        ) {
            return Err(invalid(format!(
                "Unsupported browser_session action '{action}'"
            )));
        }
        let capture = match capture_after_action {
            Some(capture) => capture,
            None => {
                let session_for_worker = Arc::clone(&session);
                let tab_id_for_worker = tab_id.clone();
                blocking(move || {
                    let mut session = session_for_worker
                        .lock()
                        .map_err(|_| "browser session is unavailable".to_string())?;
                    observe_tab(
                        &mut session,
                        &tab_id_for_worker,
                        after_diagnostic_cursor,
                        &observation_options,
                    )
                })
                .await?
            }
        };
        observation_result(call_id, &session_id, &tab_id, capture)
    }
}

fn observation_result(
    call_id: &str,
    session_id: &str,
    tab_id: &str,
    mut capture: ObservationCapture,
) -> Result<ToolResult, CoreError> {
    capture.data["sessionId"] = serde_json::json!(session_id);
    let output = ToolOutput {
        llm_content: serde_json::to_string_pretty(&capture.data)?,
        display_content: format!("Observed browser tab {tab_id}."),
        data: Some(capture.data.clone()),
        artifacts: Some(
            serde_json::json!({ "kind": "browserObservation", "sessionId": session_id, "observation": capture.data }),
        ),
        attachments: vec![ToolOutputAttachment {
            name: "browser-session.png".to_string(),
            mime_type: "image/png".to_string(),
            data: serde_json::json!({ "base64": STANDARD.encode(capture.screenshot) }),
        }],
    };
    Ok(ToolResult::from_output(call_id, false, output))
}

fn required_string<'a>(value: Option<&'a str>, field: &str) -> Result<&'a str, String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("browser_session requires {field}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_discovery_only_returns_current_owner_and_live_resources() {
        let own = Arc::new(SessionResource::new((), Some("current".into()), ()));
        let other = Arc::new(SessionResource::new((), Some("other".into()), ()));
        let detached = Arc::new(SessionResource::new((), None, ()));
        let closing = Arc::new(SessionResource::new((), Some("current".into()), ()));
        closing.closing.store(true, Ordering::Release);
        let sessions = HashMap::from([
            ("owned".into(), own),
            ("other".into(), other),
            ("detached".into(), detached),
            ("closing".into(), closing),
        ]);
        let owned = owned_session_resources(&sessions, Some("current"));
        assert_eq!(
            owned.iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>(),
            vec!["owned"]
        );
        assert_eq!(owned_session_resources(&sessions, None)[0].0, "detached");
        assert!(owned_session_resources(&sessions, Some("missing")).is_empty());
    }

    #[test]
    fn opened_page_capture_failure_preserves_target_and_prohibits_replay() {
        for action in ["create_session", "open_tab"] {
            let result =
                browser_open_observation_failure("call", action, "session", "tab", "capture lost");
            assert!(result.is_error);
            let receipt = result.artifacts.as_ref().unwrap();
            assert_eq!(receipt["opened"], true);
            assert_eq!(receipt["retrySafe"], false);
            assert_eq!(
                receipt["recovery"],
                serde_json::json!({"action":"observe","sessionId":"session","tabId":"tab"})
            );
            assert!(result.llm_context_content().contains("Do not repeat"));
        }
    }

    #[tokio::test]
    async fn startup_validates_url_before_launch_and_empty_discovery_needs_no_browser() {
        let db = crate::db::Database::open_memory().unwrap();
        let tool = BrowserSessionTool::default();
        for url in [
            "javascript:alert(1)",
            "file:///private.txt",
            "http://169.254.169.254/latest",
        ] {
            let args = serde_json::json!({"action":"create_session","url":url}).to_string();
            assert!(tool
                .execute(ToolExecutionContext::new("create", &args, &db, &[]))
                .await
                .is_err());
            assert!(tool.sessions.lock().unwrap().is_empty());
        }
        let listed = tool
            .execute(ToolExecutionContext::new(
                "list",
                r#"{"action":"list_sessions"}"#,
                &db,
                &[],
            ))
            .await
            .unwrap();
        assert_eq!(listed.artifacts.unwrap()["sessions"], serde_json::json!([]));
    }

    #[tokio::test]
    #[ignore = "requires an installed Chromium browser"]
    async fn browser_startup_url_returns_actionable_pixels_and_recoverable_session() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let page_loads = Arc::new(AtomicUsize::new(0));
        let loads = page_loads.clone();
        let server = tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let loads = loads.clone();
                tokio::spawn(async move {
                    let mut request = vec![0; 8192];
                    let count = stream.read(&mut request).await.unwrap_or(0);
                    let request = String::from_utf8_lossy(&request[..count]);
                    if request.starts_with("GET /first ") || request.starts_with("GET /second ") {
                        loads.fetch_add(1, Ordering::SeqCst);
                    }
                    let body = "<!doctype html><html><body><button onclick=\"document.getElementById('status').textContent='Saved once'\">Save fixture</button><p id='status'>Ready fixture</p></body></html>";
                    let response = format!("HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                    let _ = stream.write_all(response.as_bytes()).await;
                    let _ = stream.shutdown().await;
                });
            }
        });
        let db = crate::db::Database::open_memory().unwrap();
        let tool = BrowserSessionTool::default();
        let args = serde_json::json!({"action":"create_session","url":format!("{base_url}/first")})
            .to_string();
        let created = tool
            .execute(
                ToolExecutionContext::new("create", &args, &db, &[])
                    .with_conversation_id(Some("owner")),
            )
            .await
            .unwrap();
        assert!(!created.is_error, "{}", created.content);
        let output = created.output_channels();
        let observed = output.data.unwrap();
        assert_eq!(output.attachments.len(), 1);
        assert_eq!(observed["url"], format!("{base_url}/first"));
        assert!(observed["text"].as_str().unwrap().contains("Ready fixture"));
        let session_id = observed["sessionId"].as_str().unwrap();
        let tab_id = observed["tabId"].as_str().unwrap();
        let resource = session_by_id(&tool.sessions, session_id, Some("owner")).unwrap();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let busy = std::thread::spawn(move || {
            let _operation = resource.lock().unwrap();
            ready_tx.send(()).unwrap();
            release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        });
        ready_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let listed = tokio::time::timeout(
            Duration::from_secs(1),
            tool.execute(
                ToolExecutionContext::new(
                    "discover-busy",
                    r#"{"action":"list_sessions"}"#,
                    &db,
                    &[],
                )
                .with_conversation_id(Some("owner")),
            ),
        )
        .await;
        release_tx.send(()).unwrap();
        busy.join().unwrap();
        let busy_session = &listed
            .expect("session discovery must not wait for a browser operation")
            .unwrap()
            .artifacts
            .unwrap()["sessions"][0];
        assert_eq!(busy_session["id"], session_id);
        assert_eq!(busy_session["status"], "busy");
        let args = serde_json::json!({"action":"click","sessionId":session_id,"tabId":tab_id,"observationId":observed["observationId"],"targetRef":"e_1"}).to_string();
        let clicked = tool
            .execute(
                ToolExecutionContext::new("click", &args, &db, &[])
                    .with_conversation_id(Some("owner")),
            )
            .await
            .unwrap();
        assert!(!clicked.is_error, "{}", clicked.content);
        assert!(clicked.llm_context_content().contains("Saved once"));
        for (owner, expected) in [("owner", 1), ("different", 0)] {
            let listed = tool
                .execute(
                    ToolExecutionContext::new("list", r#"{"action":"list_sessions"}"#, &db, &[])
                        .with_conversation_id(Some(owner)),
                )
                .await
                .unwrap();
            assert_eq!(
                listed.artifacts.unwrap()["sessions"]
                    .as_array()
                    .unwrap()
                    .len(),
                expected
            );
        }
        let args = serde_json::json!({"action":"open_tab","sessionId":session_id,"url":format!("{base_url}/second")}).to_string();
        let opened = tool
            .execute(
                ToolExecutionContext::new("open", &args, &db, &[])
                    .with_conversation_id(Some("owner")),
            )
            .await
            .unwrap();
        assert!(!opened.is_error, "{}", opened.content);
        let output = opened.output_channels();
        assert_eq!(output.attachments.len(), 1);
        let second = output.data.unwrap();
        assert_eq!(second["url"], format!("{base_url}/second"));
        assert_ne!(second["tabId"], tab_id);
        assert_eq!(second["sessionId"], session_id);
        assert_eq!(
            page_loads.load(Ordering::SeqCst),
            2,
            "one navigation per opening"
        );
        let args = serde_json::json!({"action":"close_session","sessionId":session_id}).to_string();
        tool.execute(
            ToolExecutionContext::new("close", &args, &db, &[]).with_conversation_id(Some("owner")),
        )
        .await
        .unwrap();
        assert!(tool.sessions.lock().unwrap().is_empty());
        server.abort();
    }

    #[test]
    fn observation_filter_pages_original_refs_and_prioritizes_visible_controls() {
        let fixture: Vec<ObservedElement> = (0..341).map(|index| serde_json::from_value(serde_json::json!({
            "ref": format!("e_{}", index + 1), "index": index, "tag": "button", "role": "button",
            "name": if index == 340 { "Export reviewed report".to_string() } else { format!("Review record {index}") },
            "enabled": true, "visible": true,
            "bounds": [10, if index == 340 { 20 } else { 1000 + index * 30 }, 150, 24]
        })).unwrap()).collect();
        let viewport = serde_json::json!({"width":1360,"height":900});
        let (first, coverage) = filter_observed_elements(
            fixture.clone(),
            &BrowserObservationOptions::default(),
            &viewport,
        );
        assert_eq!(first[0].element_ref, "e_341");
        assert_eq!(
            first[0].index, 340,
            "action resolution must retain the original DOM selector index"
        );
        assert_eq!(coverage.total_matches, 341);
        assert_eq!(coverage.next_offset, Some(300));
        let (next, coverage) = filter_observed_elements(
            fixture.clone(),
            &BrowserObservationOptions {
                query: None,
                offset: 300,
            },
            &viewport,
        );
        assert_eq!(next.len(), 41);
        assert!(!coverage.has_more);
        assert!(next.iter().all(|item| !first
            .iter()
            .any(|previous| previous.element_ref == item.element_ref)));
        let (query, coverage) = filter_observed_elements(
            fixture,
            &BrowserObservationOptions {
                query: Some(" EXPORT REVIEWED ".into()),
                offset: 0,
            },
            &viewport,
        );
        assert_eq!(query.len(), 1);
        assert_eq!(query[0].index, 340);
        assert_eq!(coverage.query, "EXPORT REVIEWED");
        assert!(!coverage.has_more);
    }

    #[test]
    fn closing_resource_rejects_queued_work_and_drops_before_receipt() {
        struct Owned(Arc<AtomicBool>);
        impl Drop for Owned {
            fn drop(&mut self) {
                self.0.store(true, Ordering::Release);
            }
        }
        let dropped = Arc::new(AtomicBool::new(false));
        let resource = Arc::new(SessionResource::new(
            Owned(dropped.clone()),
            Some("owner".into()),
            (),
        ));
        let active = resource.lock().unwrap();
        let queued = resource.clone();
        let (ready, waiting) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            ready.send(()).unwrap();
            queued.lock().is_err()
        });
        waiting.recv().unwrap();
        resource.closing.store(true, Ordering::Release);
        assert!(resource.lock().is_err());
        drop(active);
        resource.close().unwrap();
        assert!(worker.join().unwrap());
        assert!(dropped.load(Ordering::Acquire));
        // Cloned handles cannot extend the transport's life after close.
        assert!(resource.value.lock().unwrap().is_none());
        resource.close().unwrap();
    }

    #[test]
    fn closing_transport_unblocks_a_worker_before_draining_it() {
        struct Transport(std::sync::mpsc::Sender<()>);
        impl Drop for Transport {
            fn drop(&mut self) {
                let _ = self.0.send(());
            }
        }
        let (closed_tx, closed_rx) = std::sync::mpsc::channel();
        let resource = Arc::new(SessionResource::new((), None, Transport(closed_tx)));
        let active = resource.lock().unwrap();
        let closer = resource.clone();
        let thread = std::thread::spawn(move || closer.close());
        // Keep the operation locked until transport shutdown releases it.
        closed_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        drop(active);
        thread.join().unwrap().unwrap();
        assert!(resource.lock().is_err());
    }

    #[tokio::test]
    #[ignore = "requires an installed Chromium browser"]
    async fn native_browser_consumes_input_observation_and_closes_owned_transport() {
        let db = crate::db::Database::open_memory().unwrap();
        let tool = BrowserSessionTool::default();
        let created = tool
            .execute(ToolExecutionContext::new(
                "create",
                r#"{"action":"create_session"}"#,
                &db,
                &[],
            ))
            .await
            .unwrap();
        let artifact = created.artifacts.unwrap();
        let session_id = artifact["sessionId"].as_str().unwrap();
        let tab_id = artifact["tabId"].as_str().unwrap();
        let resource = session_by_id(&tool.sessions, session_id, None).unwrap();
        {
            let session = resource.lock().unwrap();
            session.tabs[tab_id].tab.evaluate("window.nexaClicks=0;document.body.innerHTML='<button onclick=\"window.nexaClicks++\">Count</button>'", false).unwrap();
        }
        let observe = serde_json::json!({"action":"observe","sessionId":session_id,"tabId":tab_id})
            .to_string();
        let observed = tool
            .execute(ToolExecutionContext::new("observe", &observe, &db, &[]))
            .await
            .unwrap();
        let observation_id = observed.artifacts.as_ref().unwrap()["data"]["observationId"]
            .as_str()
            .unwrap();
        let click = serde_json::json!({"action":"click","sessionId":session_id,"tabId":tab_id,"observationId":observation_id,"targetRef":"e_1"}).to_string();
        let first = tool
            .execute(ToolExecutionContext::new("click1", &click, &db, &[]))
            .await
            .unwrap();
        assert!(!first.is_error, "{}", first.content);
        let second = tool
            .execute(ToolExecutionContext::new("click2", &click, &db, &[]))
            .await
            .unwrap();
        assert!(second.is_error);
        assert_eq!(
            second.artifacts.as_ref().unwrap()["status"],
            "not_dispatched"
        );
        let count = resource.lock().unwrap().tabs[tab_id]
            .tab
            .evaluate("window.nexaClicks", false)
            .unwrap()
            .value
            .unwrap();
        assert_eq!(count, 1);
        let open = serde_json::json!({"action":"open_tab","sessionId":session_id}).to_string();
        tool.execute(ToolExecutionContext::new("open", &open, &db, &[]))
            .await
            .unwrap();
        // Keep an operation handle alive through close. It must not prolong
        // process ownership or prevent the independent owner from closing CDP.
        let retained_operation_browser = resource.lock().unwrap().browser.clone();
        assert!(retained_operation_browser.get_version().is_ok());
        let pending_tab = resource.lock().unwrap().tabs[tab_id].tab.clone();
        let active_resource = resource.clone();
        let active_tab_id = tab_id.to_string();
        let pending = std::thread::spawn(move || {
            let session = active_resource.lock().unwrap();
            session.tabs[&active_tab_id]
                .tab
                .evaluate("window.nexaPending = true; new Promise(() => {})", true)
        });
        let deadline = Instant::now() + Duration::from_secs(5);
        while pending_tab
            .evaluate("window.nexaPending === true", false)
            .unwrap()
            .value
            != Some(serde_json::json!(true))
        {
            assert!(Instant::now() < deadline, "pending CDP call did not start");
            std::thread::sleep(Duration::from_millis(10));
        }
        let close =
            serde_json::json!({"action":"close_session","sessionId":session_id}).to_string();
        let closed = tool
            .execute(ToolExecutionContext::new("close", &close, &db, &[]))
            .await
            .unwrap();
        assert_eq!(closed.artifacts.unwrap()["sessionClosed"], true);
        assert!(resource.lock().is_err());
        assert!(resource.value.lock().unwrap().is_none());
        assert!(!tool.sessions.lock().unwrap().contains_key(session_id));
        assert!(retained_operation_browser.get_version().is_err());
        assert!(pending.join().unwrap().is_err());
    }

    #[test]
    fn schema_keeps_one_stable_browser_surface() {
        let schema = BrowserSessionTool::default().parameters_schema();
        let actions = schema["properties"]["action"]["enum"].as_array().unwrap();
        for action in [
            "create_session",
            "list_sessions",
            "observe",
            "click",
            "wait_for",
            "close_session",
        ] {
            assert!(actions.iter().any(|value| value == action));
        }
        assert_eq!(schema["properties"]["timeoutMs"]["maximum"], MAX_WAIT_MS);

        let variants = schema["oneOf"]
            .as_array()
            .expect("browser close targets must be represented in the provider schema");
        let close_session = variants
            .iter()
            .find(|variant| {
                variant["properties"]["action"]["enum"]
                    .as_array()
                    .is_some_and(|actions| actions.iter().any(|action| action == "close_session"))
            })
            .expect("close_session schema variant");
        assert_eq!(close_session["required"], serde_json::json!(["sessionId"]));
        assert!(close_session["properties"]["action"]["enum"]
            .as_array()
            .is_some_and(|actions| actions.iter().any(|action| action == "observe")));
        let close_tab = variants
            .iter()
            .find(|variant| {
                variant["properties"]["action"]["enum"] == serde_json::json!(["close_tab"])
            })
            .expect("close_tab schema variant");
        assert_eq!(
            close_tab["required"],
            serde_json::json!(["sessionId", "tabId"])
        );
        let create_session = variants
            .iter()
            .find(|variant| {
                variant["properties"]["action"]["enum"]
                    .as_array()
                    .is_some_and(|actions| actions.iter().any(|action| action == "create_session"))
            })
            .expect("create_session schema variant");
        assert!(create_session.get("required").is_none());
    }

    #[test]
    fn destructive_close_actions_require_confirmation() {
        let tool = BrowserSessionTool::default();

        assert!(tool.requires_confirmation(&serde_json::json!({ "action": "close_tab" })));
        assert!(tool.requires_confirmation(&serde_json::json!({ "action": "close_session" })));
        assert!(!tool.requires_confirmation(&serde_json::json!({ "action": "navigate" })));
    }

    #[test]
    fn target_bounds_reject_material_drift() {
        assert!(bounds_stable(
            &[1.0, 2.0, 30.0, 40.0],
            &[3.0, 2.0, 31.0, 39.0]
        ));
        assert!(!bounds_stable(
            &[1.0, 2.0, 30.0, 40.0],
            &[20.0, 2.0, 30.0, 40.0]
        ));
    }

    #[test]
    fn diagnostics_keep_a_bounded_monotonic_delta_cursor() {
        let mut diagnostics = BrowserDiagnostics::default();
        for index in 0..505 {
            diagnostics.push(serde_json::json!({ "message": index }));
        }

        assert_eq!(diagnostics.cursor, 505);
        assert_eq!(diagnostics.entries.len(), 500);
        let delta = diagnostics.after(503);
        assert_eq!(delta.len(), 2);
        assert_eq!(delta[0]["seq"], 504);
        assert_eq!(delta[1]["seq"], 505);
    }

    #[test]
    fn browser_session_ownership_is_conversation_scoped() {
        let owner = Some("conversation-1".to_string());
        assert!(belongs_to_conversation(&owner, Some("conversation-1")));
        assert!(!belongs_to_conversation(&owner, Some("conversation-2")));
        assert!(!belongs_to_conversation(&owner, None));
        assert!(belongs_to_conversation(&None, None));
    }

    #[test]
    fn browser_observation_eviction_selects_the_oldest_capture() {
        let newer = Instant::now();
        let older = newer - Duration::from_secs(1);
        let observation = |created_at| BrowserObservation {
            created_at,
            tab_id: "tab-1".to_string(),
            url: "https://example.com".to_string(),
            content_hash: "hash".to_string(),
            elements: Vec::new(),
            options: BrowserObservationOptions::default(),
        };
        let observations = HashMap::from([
            ("newest".to_string(), observation(newer)),
            ("oldest".to_string(), observation(older)),
        ]);

        assert_eq!(
            oldest_observation_id(&observations).as_deref(),
            Some("oldest")
        );
    }
}
