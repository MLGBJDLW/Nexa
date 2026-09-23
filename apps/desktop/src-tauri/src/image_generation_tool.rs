use async_trait::async_trait;
use nexa_core::app_settings::ImageGenerationSource;
use nexa_core::error::CoreError;
use nexa_core::tools::image_generation_tool::{codex_subscription_image_result, GenerateImageTool};
use nexa_core::tools::{Tool, ToolExecutionContext, ToolRegistry, ToolResult};
use serde_json::{json, Value};

use crate::subscription_runtime::SubscriptionRuntimeKind;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ImageRoute {
    ApiKey,
    Codex,
    UnsupportedSubscription,
}

fn image_route(source: ImageGenerationSource, chat: Option<SubscriptionRuntimeKind>) -> ImageRoute {
    match (source, chat) {
        (ImageGenerationSource::ApiKey, _) | (ImageGenerationSource::Auto, None) => {
            ImageRoute::ApiKey
        }
        (ImageGenerationSource::Subscription, _)
        | (ImageGenerationSource::Auto, Some(SubscriptionRuntimeKind::Codex)) => ImageRoute::Codex,
        (ImageGenerationSource::Auto, Some(SubscriptionRuntimeKind::Copilot)) => {
            ImageRoute::UnsupportedSubscription
        }
    }
}

pub(crate) struct DesktopImageGenerationTool {
    route: ImageRoute,
    model: Option<String>,
}

pub(crate) fn install_desktop_image_tool(
    tools: ToolRegistry,
    source: ImageGenerationSource,
    chat: Option<SubscriptionRuntimeKind>,
    model: Option<String>,
) -> ToolRegistry {
    // Registration appends and lookup returns the first match. Replace the core
    // API tool so neither discovery nor execution can bypass the selected route.
    let mut tools = tools.without_names(&["generate_image"]);
    tools.register(Box::new(DesktopImageGenerationTool::new(
        source, chat, model,
    )));
    tools
}

impl DesktopImageGenerationTool {
    pub(crate) fn new(
        source: ImageGenerationSource,
        chat: Option<SubscriptionRuntimeKind>,
        model: Option<String>,
    ) -> Self {
        Self {
            route: image_route(source, chat),
            model: if chat == Some(SubscriptionRuntimeKind::Codex) {
                model
            } else {
                None
            },
        }
    }
}

#[async_trait]
impl Tool for DesktopImageGenerationTool {
    fn name(&self) -> &str {
        "generate_image"
    }
    fn description(&self) -> &str {
        match self.route {
            ImageRoute::ApiKey => GenerateImageTool.description(),
            ImageRoute::Codex => "Generate one image using the user's signed-in ChatGPT/Codex subscription and return a Nexa preview. This route never falls back to an API key. The subscription runtime manages its image model and output options.",
            ImageRoute::UnsupportedSubscription => "Image generation is unavailable through this chat's Copilot subscription. The user can select a signed-in Codex subscription or API Key in image settings.",
        }
    }
    fn parameters_schema(&self) -> Value {
        match self.route {
            ImageRoute::ApiKey => GenerateImageTool.parameters_schema(),
            _ => {
                json!({"type":"object","properties":{"prompt":{"type":"string","description":"The complete image prompt."},"filename":{"type":"string","description":"Optional suggested PNG filename."}},"required":["prompt"],"additionalProperties":false})
            }
        }
    }
    async fn execute(&self, context: ToolExecutionContext<'_>) -> Result<ToolResult, CoreError> {
        match self.route {
            ImageRoute::ApiKey => GenerateImageTool.execute(context).await,
            ImageRoute::UnsupportedSubscription => Err(CoreError::InvalidInput("The current Copilot subscription does not expose an image-generation adapter. Select Subscription to use your signed-in Codex account, or explicitly select API Key in image settings.".into())),
            ImageRoute::Codex => {
                let args: Value = serde_json::from_str(context.arguments).map_err(|e| CoreError::InvalidInput(e.to_string()))?;
                let prompt = args["prompt"].as_str().map(str::trim).filter(|v| !v.is_empty()).ok_or_else(|| CoreError::InvalidInput("Image prompt cannot be empty".into()))?;
                if prompt.chars().count() > 32_000 { return Err(CoreError::InvalidInput("Keep an image prompt under 32000 characters".into())); }
                let token = context.cancel_token.cloned().unwrap_or_default();
                let image = crate::subscription_runtime::codex::generate_subscription_image(prompt, self.model.as_deref(), &token).await?;
                codex_subscription_image_result(context.call_id, prompt, &image.encoded, image.revised_prompt.as_deref(), args["filename"].as_str())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn assembled_catalog_replaces_the_api_tool_for_subscription_chats() {
        let db = nexa_core::db::Database::open_memory().unwrap();
        let assembler =
            nexa_core::package_host::PackageRuntimeAssembler::database_builtin(&db).unwrap();
        for (source, chat, subscription) in [
            (
                ImageGenerationSource::Auto,
                Some(SubscriptionRuntimeKind::Codex),
                true,
            ),
            (ImageGenerationSource::Subscription, None, true),
            (
                ImageGenerationSource::ApiKey,
                Some(SubscriptionRuntimeKind::Codex),
                false,
            ),
        ] {
            let builtin = assembler.builtin_tool_registry();
            assert!(builtin.get("generate_image").is_some());
            let tools = install_desktop_image_tool(builtin, source, chat, None);
            let tools = assembler.assemble_tool_registry(tools).unwrap().tools;
            assert_eq!(
                tools
                    .definitions()
                    .iter()
                    .filter(|tool| tool.name == "generate_image")
                    .count(),
                1
            );
            let selected = tools.get("generate_image").unwrap();
            assert_eq!(
                selected
                    .description()
                    .contains("signed-in ChatGPT/Codex subscription"),
                subscription
            );
            assert_eq!(
                selected.parameters_schema()["properties"]
                    .get("provider_config_id")
                    .is_none(),
                subscription
            );
        }
    }
    #[test]
    fn image_source_choice_matches_chat_without_paid_fallback() {
        assert_eq!(
            image_route(
                ImageGenerationSource::Auto,
                Some(SubscriptionRuntimeKind::Codex)
            ),
            ImageRoute::Codex
        );
        assert_eq!(
            image_route(
                ImageGenerationSource::Auto,
                Some(SubscriptionRuntimeKind::Copilot)
            ),
            ImageRoute::UnsupportedSubscription
        );
        assert_eq!(
            image_route(ImageGenerationSource::Auto, None),
            ImageRoute::ApiKey
        );
        assert_eq!(
            image_route(ImageGenerationSource::Subscription, None),
            ImageRoute::Codex
        );
        assert_eq!(
            image_route(
                ImageGenerationSource::ApiKey,
                Some(SubscriptionRuntimeKind::Codex)
            ),
            ImageRoute::ApiKey
        );
    }
}
