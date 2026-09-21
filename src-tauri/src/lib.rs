mod commands;
mod git;
mod models;
mod secrets;
mod storage;
mod worker;

use storage::MetadataState;
use tauri::Manager;
use worker::WorkerState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .manage(WorkerState::default())
        .setup(|app| {
            let state = MetadataState::load(&app.handle())?;
            app.manage(state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::bootstrap,
            commands::save_provider,
            commands::delete_provider,
            commands::discover_models,
            commands::add_project,
            commands::create_task,
            commands::configure_task,
            commands::open_task,
            commands::prompt,
            commands::stop_task,
            commands::archive_task,
            commands::rename_task,
            commands::delete_task,
            commands::convert_task_to_worktree,
            commands::remove_project,
            commands::git_changes,
            commands::reveal_task,
            commands::reveal_path,
        ])
        .build(tauri::generate_context!())
        .expect("error while building WackCode");

    app.run(|app, event| {
        if matches!(event, tauri::RunEvent::Exit | tauri::RunEvent::ExitRequested { .. }) {
            app.state::<WorkerState>().terminate_all();
        }
    });
}
