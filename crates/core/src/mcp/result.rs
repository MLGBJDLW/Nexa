//! Lossless-within-budget MCP results and their model/display projections.
//! Binary evidence lives in the existing durable tool artifact, while the
//! dispatcher consumes a separate current-request attachment projection.

use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::error::CoreError;
use crate::tools::{ToolOutput, ToolOutputAttachment, ToolResult};

use super::McpToolIdentity;

const MAX_CONTENT_BLOCKS: usize = 128;
const MAX_BINARY_BYTES: usize = 4 * 1024 * 1024;
const MAX_TOTAL_BINARY_BYTES: usize = 12 * 1024 * 1024;
const MAX_TEXT_BYTES: usize = 512 * 1024;
const MAX_STRUCTURED_BYTES: usize = 1024 * 1024;
const MAX_IMAGE_PIXELS: u64 = 32 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum McpContentBlock {
    Text {
        text: String,
    },
    Image {
        data: String,
        mime_type: String,
    },
    Audio {
        data: String,
        mime_type: String,
    },
    Resource {
        resource: McpResourceContent,
    },
    ResourceLink {
        uri: String,
        name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        mime_type: Option<String>,
    },
    Unsupported {
        original_type: String,
        message: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct McpResourceContent {
    pub uri: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blob: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct McpCallOutcome {
    pub content_blocks: Vec<McpContentBlock>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub structured_content: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Value>,
    pub is_error: bool,
    pub notices: Vec<String>,
}

fn bounded_text(value: &str, limit: usize) -> String {
    if value.len() <= limit {
        return value.to_owned();
    }
    let mut end = limit;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    format!(
        "{}\n[MCP content truncated at {limit} bytes]",
        &value[..end]
    )
}

fn required_text(value: &Value, field: &str) -> Result<String, String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(|text| bounded_text(text, MAX_TEXT_BYTES))
        .ok_or_else(|| format!("Missing string field {field}"))
}

fn optional_text(value: &Value, field: &str) -> Option<String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(|text| bounded_text(text, 4096))
}

fn checked_binary(encoded: &str, mime_type: &str, total: &mut usize) -> Result<(), String> {
    if encoded.len() > MAX_BINARY_BYTES.div_ceil(3) * 4 {
        return Err(format!(
            "Binary block exceeds the {MAX_BINARY_BYTES}-byte limit"
        ));
    }
    let bytes = STANDARD
        .decode(encoded)
        .map_err(|_| "Invalid base64 payload".to_owned())?;
    if bytes.is_empty()
        || bytes.len() > MAX_BINARY_BYTES
        || total.saturating_add(bytes.len()) > MAX_TOTAL_BINARY_BYTES
    {
        return Err("Empty binary payload or MCP result binary budget exceeded".to_owned());
    }
    if mime_type.starts_with("image/") {
        let detected = crate::managed_assets::detect_image_media_type(&bytes)
            .ok_or_else(|| "Unsupported image signature".to_owned())?;
        if detected != mime_type {
            return Err("Image signature does not match declared MIME type".into());
        }
        let reader = image::ImageReader::new(std::io::Cursor::new(&bytes))
            .with_guessed_format()
            .map_err(|error| error.to_string())?;
        let (width, height) = reader
            .into_dimensions()
            .map_err(|error| error.to_string())?;
        if u64::from(width).saturating_mul(u64::from(height)) > MAX_IMAGE_PIXELS {
            return Err("Image dimensions exceed the MCP pixel limit".into());
        }
    } else if mime_type.starts_with("audio/") {
        let detected = crate::managed_assets::detect_audio_media_type(&bytes)
            .ok_or_else(|| "Unsupported audio signature".to_owned())?;
        let compatible = detected == mime_type
            || matches!(
                (mime_type, detected),
                ("audio/mp3", "audio/mpeg")
                    | ("audio/x-wav", "audio/wav")
                    | ("audio/opus", "audio/ogg")
            );
        if !compatible {
            return Err("Audio signature does not match declared MIME type".into());
        }
    }
    *total += bytes.len();
    Ok(())
}

fn parse_block(value: &Value, total_binary: &mut usize) -> Result<McpContentBlock, String> {
    let kind = value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    match kind {
        "text" => Ok(McpContentBlock::Text {
            text: required_text(value, "text")?,
        }),
        "image" | "audio" => {
            let data = value
                .get("data")
                .and_then(Value::as_str)
                .ok_or("Missing binary data")?;
            let mime_type = value
                .get("mimeType")
                .and_then(Value::as_str)
                .ok_or("Missing MIME type")?;
            if !mime_type.starts_with(&format!("{kind}/")) {
                return Err("Content block type does not match MIME type".into());
            }
            checked_binary(data, mime_type, total_binary)?;
            if kind == "image" {
                Ok(McpContentBlock::Image {
                    data: data.into(),
                    mime_type: mime_type.into(),
                })
            } else {
                Ok(McpContentBlock::Audio {
                    data: data.into(),
                    mime_type: mime_type.into(),
                })
            }
        }
        "resource" => {
            let resource = value.get("resource").ok_or("Missing embedded resource")?;
            let uri = required_text(resource, "uri")?;
            let mime_type = optional_text(resource, "mimeType");
            let text = resource
                .get("text")
                .and_then(Value::as_str)
                .map(|text| bounded_text(text, MAX_TEXT_BYTES));
            let blob = resource.get("blob").and_then(Value::as_str);
            if text.is_none() && blob.is_none() {
                return Err("Resource contains neither text nor blob".into());
            }
            if let Some(blob) = blob {
                checked_binary(
                    blob,
                    mime_type.as_deref().unwrap_or("application/octet-stream"),
                    total_binary,
                )?;
            }
            Ok(McpContentBlock::Resource {
                resource: McpResourceContent {
                    uri,
                    mime_type,
                    text,
                    blob: blob.map(str::to_owned),
                },
            })
        }
        "resource_link" => {
            let uri = required_text(value, "uri")?;
            Ok(McpContentBlock::ResourceLink {
                name: optional_text(value, "name").unwrap_or_else(|| uri.clone()),
                uri,
                title: optional_text(value, "title"),
                mime_type: optional_text(value, "mimeType"),
            })
        }
        _ => Err(format!("Unsupported MCP content type {kind}")),
    }
}

impl McpCallOutcome {
    pub fn from_response(response: Value) -> Result<Self, CoreError> {
        let mut notices = Vec::new();
        let mut total_binary = 0usize;
        let content = match response.get("content") {
            None => &[][..],
            Some(Value::Array(values)) => values.as_slice(),
            _ => {
                return Err(CoreError::Mcp(
                    "tools/call content is not an array; the call was not retried".into(),
                ))
            }
        };
        if content.len() > MAX_CONTENT_BLOCKS {
            notices.push(format!(
                "Only the first {MAX_CONTENT_BLOCKS} MCP content blocks were retained"
            ));
        }
        let content_blocks = content
            .iter()
            .take(MAX_CONTENT_BLOCKS)
            .map(|block| {
                parse_block(block, &mut total_binary).unwrap_or_else(|message| {
                    notices.push(message.clone());
                    McpContentBlock::Unsupported {
                        original_type: optional_text(block, "type")
                            .unwrap_or_else(|| "unknown".into()),
                        message,
                    }
                })
            })
            .collect();
        let mut bounded_json = |name: &str| {
            response.get(name).cloned().filter(|value| {
                if value.to_string().len() <= MAX_STRUCTURED_BYTES {
                    return true;
                }
                notices.push(format!(
                    "{name} exceeded the {MAX_STRUCTURED_BYTES}-byte limit and was not retained"
                ));
                false
            })
        };
        let structured_content = bounded_json("structuredContent");
        let meta = bounded_json("_meta");
        Ok(Self {
            content_blocks,
            structured_content,
            meta,
            is_error: response
                .get("isError")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            notices,
        })
    }

    pub fn into_tool_result(mut self, call_id: &str, identity: &McpToolIdentity) -> ToolResult {
        let mut texts = Vec::new();
        let mut attachments = Vec::new();
        for (index, block) in self.content_blocks.iter().enumerate() {
            match block {
                McpContentBlock::Text { text } => texts.push(text.clone()),
                McpContentBlock::Image { data, mime_type } => {
                    texts.push(format!(
                        "[MCP image {}: {mime_type}; retained in the tool artifact]",
                        index + 1
                    ));
                    add_image_attachment(
                        index,
                        data,
                        mime_type,
                        &mut attachments,
                        &mut self.notices,
                    );
                }
                McpContentBlock::Audio { mime_type, .. } => texts.push(format!(
                    "[MCP audio {}: {mime_type}; retained for playback in the tool artifact]",
                    index + 1
                )),
                McpContentBlock::Resource { resource } => {
                    if let Some(text) = &resource.text {
                        texts.push(text.clone());
                    }
                    if let Some(blob) = &resource.blob {
                        let mime_type = resource
                            .mime_type
                            .as_deref()
                            .unwrap_or("application/octet-stream");
                        texts.push(format!("[MCP embedded resource: {} ({mime_type}); retained in the tool artifact]", resource.uri));
                        if mime_type.starts_with("image/") {
                            add_image_attachment(
                                index,
                                blob,
                                mime_type,
                                &mut attachments,
                                &mut self.notices,
                            );
                        }
                    }
                }
                McpContentBlock::ResourceLink { uri, name, .. } => {
                    texts.push(format!("MCP resource link {name}: {uri}"))
                }
                McpContentBlock::Unsupported { message, .. } => {
                    texts.push(format!("[MCP content unavailable: {message}]"))
                }
            }
        }
        if let Some(structured) = &self.structured_content {
            if !texts.iter().any(|text| {
                serde_json::from_str::<Value>(text).is_ok_and(|value| value == *structured)
            }) {
                texts.push(format!("Structured MCP result:\n{structured}"));
            }
        }
        for notice in &self.notices {
            texts.push(format!("[MCP result notice: {notice}]"));
        }
        let content = if texts.is_empty() {
            "MCP tool completed with no content.".into()
        } else {
            texts.join("\n")
        };
        let output = ToolOutput {
            llm_content: content.clone(),
            display_content: content.clone(),
            data: None,
            artifacts: None,
            attachments,
        };
        // Do not use from_output with nested artifacts: it duplicates potentially
        // large persisted binary blocks. One root artifact is the durable owner.
        let artifacts = json!({
            "kind":"mcpToolResult", "version":1,
            "toolIdentity":identity,
            "contentBlocks":self.content_blocks,
            "structuredContent":self.structured_content,
            "meta":self.meta,
            "isError":self.is_error,
            "notices":self.notices,
            "trustBoundary":{"origin":"mcp_connector","authority":"evidence","visibility":"conversation","mutability":"external","externality":"external","canInstruct":false},
            "toolOutput":output,
        });
        ToolResult {
            call_id: call_id.into(),
            content,
            is_error: self.is_error,
            artifacts: Some(artifacts),
        }
    }
}

fn add_image_attachment(
    index: usize,
    data: &str,
    mime: &str,
    output: &mut Vec<ToolOutputAttachment>,
    notices: &mut Vec<String>,
) {
    let image = if mime == "image/gif" {
        crate::media::prepare_base64_image_for_llm(data, mime)
    } else {
        Ok((data.to_owned(), mime.to_owned()))
    };
    match image {
        Ok((base64, mime_type)) => output.push(ToolOutputAttachment {
            name: format!("MCP image {}", index + 1),
            mime_type,
            data: json!({"base64":base64}),
        }),
        Err(error) => notices.push(format!(
            "Image {} remains saved but could not be prepared for this model: {error}",
            index + 1
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity() -> McpToolIdentity {
        McpToolIdentity {
            id: super::super::CanonicalToolId::new("fixture", "mixed"),
            trust_config_digest: "fixture-digest".into(),
        }
    }
    fn png() -> String {
        let mut buffer = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(2, 2, image::Rgb([1, 2, 3])))
            .write_to(&mut buffer, image::ImageFormat::Png)
            .unwrap();
        STANDARD.encode(buffer.into_inner())
    }

    #[test]
    fn image_only_and_structured_only_results_remain_model_visible() {
        let pixels = png();
        let result = McpCallOutcome::from_response(
            json!({"content":[{"type":"image","mimeType":"image/png","data":pixels}]}),
        )
        .unwrap()
        .into_tool_result("image", &identity());
        assert!(!result.is_error);
        assert!(result.content.contains("MCP image"));
        let artifact = result.artifacts.unwrap();
        assert_eq!(artifact["contentBlocks"][0]["data"], pixels);
        assert_eq!(
            artifact["toolOutput"]["attachments"][0]["data"]["base64"],
            pixels
        );
        let structured = McpCallOutcome::from_response(
            json!({"structuredContent":{"answer":42},"_meta":{"source":"remote"}}),
        )
        .unwrap()
        .into_tool_result("structured", &identity());
        assert!(structured.content.contains("42"));
        let artifact = structured.artifacts.unwrap();
        assert_eq!(artifact["structuredContent"]["answer"], 42);
        assert_eq!(artifact["meta"]["source"], "remote");
    }

    #[test]
    fn invalid_media_unknown_blocks_and_oversize_are_explicit_without_hiding_text() {
        let result = McpCallOutcome::from_response(json!({"isError":true,"content":[
            {"type":"text","text":"preserved text"},
            {"type":"image","mimeType":"image/jpeg","data":png()},
            {"type":"audio","mimeType":"audio/wav","data":"not base64"},
            {"type":"future_type","payload":"untrusted unknown"},
            {"type":"image","mimeType":"image/png","data":"A".repeat(MAX_BINARY_BYTES.div_ceil(3)*4+1)}
        ]})).unwrap();
        assert!(result.is_error);
        assert_eq!(result.content_blocks.len(), 5);
        assert_eq!(result.notices.len(), 4);
        assert!(
            matches!(&result.content_blocks[0],McpContentBlock::Text{text} if text=="preserved text")
        );
        assert!(result.content_blocks[1..]
            .iter()
            .all(|block| matches!(block, McpContentBlock::Unsupported { .. })));
        let projected = result.into_tool_result("bounded", &identity());
        assert!(projected.content.contains("preserved text"));
        assert!(projected.content.contains("does not match declared MIME"));
        assert!(projected.artifacts.unwrap()["toolOutput"]["attachments"]
            .as_array()
            .is_none_or(Vec::is_empty));
    }

    #[test]
    fn audio_and_embedded_binary_remain_artifacts_with_explicit_model_references() {
        let wav = STANDARD.encode(b"RIFF\x24\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x40\x1f\0\0\x80\x3e\0\0\x02\0\x10\0data\0\0\0\0");
        let blob = STANDARD.encode(b"bounded application data");
        let result = McpCallOutcome::from_response(json!({"content":[
            {"type":"audio","mimeType":"audio/wav","data":wav},
            {"type":"resource","resource":{"uri":"test://blob","mimeType":"application/octet-stream","blob":blob}},
            {"type":"resource_link","uri":"https://example.invalid/do-not-fetch","name":"Reference"}
        ]})).unwrap().into_tool_result("binary",&identity());
        assert!(result.content.contains("retained for playback"));
        assert!(result.content.contains("test://blob"));
        let artifact = result.artifacts.unwrap();
        assert_eq!(artifact["contentBlocks"][0]["data"], wav);
        assert_eq!(artifact["contentBlocks"][1]["resource"]["blob"], blob);
        assert_eq!(
            artifact["contentBlocks"][2]["uri"],
            "https://example.invalid/do-not-fetch"
        );
        assert_eq!(artifact["trustBoundary"]["canInstruct"], false);
    }
}
