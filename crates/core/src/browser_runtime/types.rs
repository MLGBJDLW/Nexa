use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum BrowserControlOwner {
    #[default]
    None,
    User,
    Agent {
        call_id: String,
    },
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserBounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl BrowserBounds {
    pub fn sanitized(self) -> Self {
        Self {
            x: self.x.max(0.0),
            y: self.y.max(0.0),
            width: self.width.clamp(1.0, 16_384.0),
            height: self.height.clamp(1.0, 16_384.0),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserElementBounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserLocatorFingerprint {
    pub tag: Option<String>,
    pub id: Option<String>,
    pub test_id: Option<String>,
    pub name: Option<String>,
    pub href: Option<String>,
    pub css_path: Option<String>,
    pub text_hash: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserElement {
    #[serde(rename = "ref")]
    pub element_ref: String,
    pub tag: String,
    pub role: String,
    pub name: String,
    pub href: Option<String>,
    pub input_type: Option<String>,
    pub enabled: bool,
    pub visible: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value_truncated: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checked: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<Vec<BrowserSelectOption>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub option_count: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_values: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub files: Option<Vec<BrowserFileMetadata>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_count: Option<usize>,
    pub bounds: BrowserElementBounds,
    pub locator_fingerprint: BrowserLocatorFingerprint,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct BrowserFileMetadata {
    pub name: String,
    pub size: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserSelectOption {
    pub value: String,
    pub label: String,
    pub selected: bool,
    pub enabled: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserScreenshot {
    pub mime_type: String,
    pub content_hash: String,
    pub width: u32,
    pub height: u32,
    pub byte_length: usize,
    #[serde(skip, default)]
    pub image_bytes: Vec<u8>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserObservation {
    pub observation_id: String,
    pub session_id: String,
    pub tab_id: String,
    pub url: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ready_state: Option<String>,
    pub text: String,
    pub viewport: serde_json::Value,
    pub content_hash: String,
    pub elements: Vec<BrowserElement>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observation_coverage: Option<BrowserObservationCoverage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame_limitations: Option<BrowserFrameLimitations>,
    pub accessibility_tree: Vec<BrowserElement>,
    pub control_owner: BrowserControlOwner,
    pub screenshot: Option<BrowserScreenshot>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserObservationOptions {
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub offset: usize,
}

impl BrowserObservationOptions {
    pub fn validate(&self) -> Result<(), String> {
        if self
            .query
            .as_ref()
            .is_some_and(|query| query.chars().count() > 240)
        {
            return Err("Browser observation query must contain at most 240 characters".into());
        }
        if self.offset as u128 > 9_007_199_254_740_991 {
            return Err("Browser observation offset exceeds the supported integer range".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserObservationCoverage {
    pub query: String,
    pub offset: usize,
    pub returned: usize,
    pub total_matches: usize,
    pub has_more: bool,
    pub next_offset: Option<usize>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserFrameLimitations {
    pub unavailable_count: usize,
    pub details_omitted: bool,
    pub frames: Vec<BrowserUnavailableFrame>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserUnavailableFrame {
    pub reason: String,
    pub url: Option<String>,
    pub title: String,
    pub visible: bool,
    pub bounds: BrowserElementBounds,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserTab {
    pub id: String,
    pub session_id: String,
    pub url: String,
    pub title: String,
    pub active: bool,
    pub loading: bool,
    pub status: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserSession {
    pub id: String,
    pub conversation_id: Option<String>,
    pub profile_id: String,
    pub active_tab_id: Option<String>,
    pub tabs: Vec<BrowserTab>,
    pub control_owner: BrowserControlOwner,
    pub workspace_visible: bool,
    #[serde(default)]
    pub cleanup_pending: bool,
    #[serde(default)]
    pub visibility_revision: u64,
    #[serde(default)]
    pub visibility_requested: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visibility_request_revision: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::BrowserSession;

    #[test]
    fn legacy_browser_session_json_defaults_cleanup_state_to_active() {
        let session: BrowserSession = serde_json::from_value(serde_json::json!({
            "id": "browser-1",
            "conversationId": "conversation-1",
            "profileId": "profile-1",
            "activeTabId": null,
            "tabs": [],
            "controlOwner": { "type": "none" },
            "workspaceVisible": false
        }))
        .unwrap();

        assert!(!session.cleanup_pending);
    }
}
