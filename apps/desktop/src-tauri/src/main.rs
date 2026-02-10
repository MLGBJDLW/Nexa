#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;

use ask_core::db::Database;
use commands::AppState;
use tauri::Manager;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .setup(|app| {
            let data_dir = app
                .path()
                .app_data_dir()
                .expect("failed to resolve app data directory");
            std::fs::create_dir_all(&data_dir)
                .expect("failed to create app data directory");

            let db_path = data_dir.join("ask-myself.db");
            let db =
                Database::new(&db_path).expect("failed to initialize database");

            app.manage(AppState { db });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // Sources
            commands::add_source,
            commands::list_sources,
            commands::get_source,
            commands::update_source,
            commands::delete_source,
            // Ingest
            commands::scan_source,
            commands::scan_all_sources,
            // Search
            commands::search,
            commands::get_evidence_card,
            // Index
            commands::get_index_stats,
            commands::rebuild_index,
            // Playbooks
            commands::create_playbook,
            commands::list_playbooks,
            commands::get_playbook,
            commands::update_playbook,
            commands::delete_playbook,
            // Citations
            commands::add_citation,
            commands::list_citations,
            commands::remove_citation,
            // Query log
            commands::get_recent_queries,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
