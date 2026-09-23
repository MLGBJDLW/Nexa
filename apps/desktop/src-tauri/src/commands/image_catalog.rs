#[tauri::command]
pub async fn discover_openrouter_image_models_cmd(
) -> Result<Vec<nexa_core::image_provider_catalog::ImageModelPreset>, String> {
    nexa_core::openrouter_images::discover_models()
        .await
        .map_err(|error| error.to_string())
}
