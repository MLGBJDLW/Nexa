use super::*;

#[derive(Debug)]
pub(crate) struct SubscriptionImage {
    pub(crate) encoded: String,
    pub(crate) revised_prompt: Option<String>,
}

pub(crate) async fn generate_subscription_image(
    prompt: &str,
    requested_model: Option<&str>,
    cancellation: &nexa_core::agent::CancellationToken,
) -> Result<SubscriptionImage, CoreError> {
    if cancellation.is_cancelled() {
        return Err(CoreError::Cancelled(
            "Subscription image generation cancelled".into(),
        ));
    }
    let work = async {
        let workspace = tempfile::Builder::new()
            .prefix("nexa-codex-image-")
            .tempdir()
            .map_err(protocol_error)?;
        let cwd = workspace.path().to_string_lossy().to_string();
        let mut wire = Wire::start_for_images(true).await?;
        let account = wire
            .request("account/read", json!({"refreshToken":false}))
            .await?;
        if account.pointer("/account/type").and_then(Value::as_str) != Some("chatgpt") {
            return Err(protocol_error("Sign in with a ChatGPT subscription in AI provider settings first. No API-key fallback was attempted."));
        }
        let capabilities = wire
            .request("modelProvider/capabilities/read", json!({}))
            .await?;
        if capabilities["imageGeneration"] != true {
            return Err(protocol_error("This Codex runtime does not expose subscription image generation. Update Codex or explicitly select API Key."));
        }
        let model_id = match requested_model.filter(|id| !id.trim().is_empty()) {
            Some(id) => {
                model(&mut wire, id).await?;
                id.to_string()
            }
            None => {
                let catalog = wire
                    .request("model/list", json!({"limit":100,"includeHidden":false}))
                    .await?;
                catalog["data"]
                    .as_array()
                    .and_then(|models| models.iter().find(|m| m["isDefault"] == true))
                    .and_then(|model| model["model"].as_str())
                    .ok_or_else(|| {
                        protocol_error(
                            "The Codex account has no default model; refresh its model catalog",
                        )
                    })?
                    .to_string()
            }
        };
        let config = wire
            .request("config/read", json!({"includeLayers":false,"cwd":cwd}))
            .await?;
        let skills = wire
            .request("skills/list", json!({"cwds":[cwd],"forceReload":true}))
            .await?;
        let mut overrides = disable_ambient(&config, &skills, &cwd)?;
        overrides.insert("features.image_generation".into(), json!(true));
        // Models may expose image_gen through the official code-mode wrapper.
        // Its host must remain available even when other native tools are disabled.
        overrides.insert("features.code_mode_host".into(), json!(true));
        overrides.insert(
            "features.omit_app_server_notification_media".into(),
            json!(false),
        );
        drop(config);
        drop(skills);
        let response = wire.request("thread/start", json!({"model":model_id,"modelProvider":"openai","allowProviderModelFallback":false,"cwd":cwd,"config":overrides,"developerInstructions":"Generate exactly one image with the available image generation capability. If it is exposed through functions.exec or another code-mode wrapper, use that wrapper to call image_gen.imagegen and display its result with generatedImage. Preserve the user's requested prompt. Do not run shell commands, read workspace files, browse the web, or use unrelated capabilities. Stop after image generation completes.","approvalPolicy":"never","sandbox":"read-only","ephemeral":true})).await?;
        if response["model"] != model_id
            || response["modelProvider"] != "openai"
            || response["approvalPolicy"] != "never"
            || response.pointer("/sandbox/type").and_then(Value::as_str) != Some("readOnly")
        {
            return Err(protocol_error(
                "Codex did not accept the subscription image execution policy",
            ));
        }
        let thread_id = response
            .pointer("/thread/id")
            .and_then(Value::as_str)
            .ok_or_else(|| protocol_error("Codex created no image thread"))?
            .to_string();
        let response = wire.request("turn/start", json!({"threadId":thread_id,"input":user_input(prompt,&[]),"approvalPolicy":"never","sandboxPolicy":{"type":"readOnly","networkAccess":false}})).await?;
        let turn_id = response
            .pointer("/turn/id")
            .and_then(Value::as_str)
            .ok_or_else(|| protocol_error("Codex created no image turn"))?
            .to_string();
        let mut image = None;
        let mut explanation = String::new();
        let mut requests = 0;
        loop {
            let message = wire.receive().await?;
            let method = message["method"].as_str().unwrap_or_default();
            let params = &message["params"];
            if message.get("id").is_some() && !method.is_empty() {
                requests += 1;
                if method == "currentTime/read"
                    && requests <= 64
                    && params["threadId"].as_str() == Some(&thread_id)
                {
                    wire.write(json!({"id":message["id"],"result":{"currentTimeAt":chrono::Utc::now().timestamp()}})).await?;
                    continue;
                }
                wire.reject(&message).await?;
                return Err(protocol_error(
                    "Codex image generation requested an unrelated native capability",
                ));
            }
            if params["threadId"]
                .as_str()
                .is_some_and(|id| id != thread_id)
                || params["turnId"].as_str().is_some_and(|id| id != turn_id)
            {
                continue;
            }
            if method == "item/completed" && params["item"]["type"] == "imageGeneration" {
                image = Some(completed_image(&params["item"])?);
            } else if method == "item/completed" && params["item"]["type"] == "agentMessage" {
                explanation = params["item"]["text"]
                    .as_str()
                    .unwrap_or_default()
                    .chars()
                    .take(1500)
                    .collect();
            } else if method == "turn/completed" {
                if params["turn"]["status"] != "completed" {
                    return Err(protocol_error(
                        "Codex subscription image turn failed. No API-key fallback was attempted.",
                    ));
                }
                return image.ok_or_else(|| protocol_error(format!("The signed-in Codex runtime did not produce an image. No API-key fallback was attempted. {explanation}")));
            } else if method == "error" && params["willRetry"] != true {
                return Err(protocol_error("Codex subscription image generation failed. Check the account's image quota and runtime support; no API-key fallback was attempted."));
            }
        }
    };
    tokio::select! {
        _ = cancellation.cancelled() => Err(CoreError::Cancelled("Subscription image generation cancelled".into())),
        result = tokio::time::timeout(Duration::from_secs(300), work) => result.map_err(|_| protocol_error("Codex subscription image generation timed out; no API-key fallback was attempted"))?,
    }
}

fn completed_image(item: &Value) -> Result<SubscriptionImage, CoreError> {
    if item["status"] != "completed" {
        if item.pointer("/failure/type").and_then(Value::as_str) == Some("usageLimitExceeded") {
            return Err(protocol_error("ChatGPT subscription image quota is exhausted. Wait for it to reset or explicitly select API Key; no fallback was attempted."));
        }
        return Err(protocol_error(
            "Codex subscription image generation failed; no API-key fallback was attempted",
        ));
    }
    let encoded = required_string(item, "result")?;
    if encoded.len() > 32usize.saturating_mul(1024 * 1024).div_ceil(3) * 4 {
        return Err(protocol_error("Codex image exceeds the preview size limit"));
    }
    Ok(SubscriptionImage {
        encoded,
        revised_prompt: item["revisedPrompt"]
            .as_str()
            .filter(|v| !v.trim().is_empty())
            .map(str::to_owned),
    })
}

#[cfg(test)]
mod image_tests {
    use super::*;
    #[test]
    fn subscription_image_completion_requires_success_and_never_uses_saved_paths() {
        assert_eq!(
            completed_image(
                &json!({"status":"completed","result":"aW1hZ2U=","savedPath":"C:/private/secret"})
            )
            .unwrap()
            .encoded,
            "aW1hZ2U="
        );
        assert!(completed_image(
            &json!({"status":"failed","result":"aW1hZ2U=","failure":{"type":"usageLimitExceeded"}})
        )
        .unwrap_err()
        .to_string()
        .contains("quota"));
        assert!(
            completed_image(&json!({"status":"completed","savedPath":"C:/private/secret"}))
                .is_err()
        );
        let image = completed_image(
            &json!({"status":"completed","result":"aW1hZ2U=","revisedPrompt":"A blue circle"}),
        )
        .unwrap();
        assert_eq!(image.revised_prompt.as_deref(), Some("A blue circle"));
    }

    #[tokio::test]
    async fn cancelled_subscription_image_does_not_start_a_runtime() {
        let cancellation = nexa_core::agent::CancellationToken::new();
        cancellation.cancel();
        assert!(matches!(
            generate_subscription_image("test", None, &cancellation).await,
            Err(CoreError::Cancelled(_))
        ));
    }
    #[tokio::test]
    #[ignore = "uses the signed-in ChatGPT subscription for one image generation"]
    async fn native_codex_subscription_image_smoke() {
        let result = generate_subscription_image(
            "Generate one simple teal circle centered on a white background. No text.",
            None,
            &nexa_core::agent::CancellationToken::new(),
        )
        .await
        .unwrap();
        let artifact = nexa_core::tools::image_generation_tool::codex_subscription_image_result(
            "smoke",
            "teal circle",
            &result.encoded,
            result.revised_prompt.as_deref(),
            Some("subscription-smoke.png"),
        )
        .unwrap();
        let path = artifact.artifacts.unwrap()["path"]
            .as_str()
            .unwrap()
            .to_string();
        println!("Subscription image preview: {path}");
        assert!(std::fs::metadata(&path).unwrap().len() > 100);
    }
}
