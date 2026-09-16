use super::*;

#[tauri::command]
pub fn list_personas_cmd(state: tauri::State<'_, AppState>) -> Result<Vec<PersonaProfile>, String> {
    nexa_core::persona::list_personas(&state.db).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn save_persona_cmd(
    state: tauri::State<'_, AppState>,
    input: SavePersonaInput,
) -> Result<PersonaProfile, String> {
    state
        .db_executor
        .write(move |db| db.save_persona(&input))
        .await
        .map(|execution| execution.value)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn delete_persona_cmd(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    state
        .db_executor
        .write(move |db| db.delete_persona(&id))
        .await
        .map(|execution| execution.value)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn toggle_persona_cmd(
    state: tauri::State<'_, AppState>,
    id: String,
    enabled: bool,
) -> Result<(), String> {
    state
        .db_executor
        .write(move |db| db.toggle_persona(&id, enabled))
        .await
        .map(|execution| execution.value)
        .map_err(|error| error.to_string())
}
