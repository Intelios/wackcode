mod checkpoints;
mod commands;
mod files;
mod git;
mod glass;
mod models;
mod secrets;
mod storage;
mod subagents;
mod subscriptions;
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
        .manage(commands::TaskLocks::default())
        .manage(subscriptions::SubscriptionState::default())
        .manage(worker::ManagerState::default())
        .setup(|app| {
            let state = MetadataState::load(&app.handle())?;
            let appearance = state.data.lock().map_err(|_| "Metadata lock was poisoned".to_string())?.appearance.clone();
            app.manage(state);
            // The window is created hidden and transparent: paint it before it first appears.
            glass::apply(app.handle(), glass::NativeBackdrop::from_config(&appearance), true)?;
            if let Some(window) = app.get_webview_window("main") {
                window.show()?;
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            // Liquid Glass shows only while focused; unfocused, the window is painted opaque.
            if let tauri::WindowEvent::Focused(focused) = event {
                let app = window.app_handle();
                let Ok(data) = app.state::<MetadataState>().data.lock().map(|data| data.appearance.clone()) else { return };
                if data.backdrop == models::BackdropMode::Glass {
                    let _ = glass::apply(app, glass::NativeBackdrop::from_config(&data), *focused);
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::bootstrap,
            commands::save_provider,
            commands::delete_provider,
            commands::discover_models,
            commands::list_builtin_models,
            subscriptions::list_subscription_providers,
            subscriptions::start_subscription_login,
            subscriptions::respond_subscription_login,
            subscriptions::cancel_subscription_login,
            subscriptions::sign_out_subscription,
            subscriptions::open_subscription_auth_url,
            commands::set_tool_config,
            commands::set_appearance_config,
            commands::set_subagent_config,
            commands::set_auto_title_config,
            commands::set_prompt_config,
            commands::list_packages,
            commands::refresh_packages,
            commands::install_package,
            commands::trust_package,
            commands::remove_package,
            commands::update_packages,
            commands::set_package_resources,
            commands::respond_extension_ui,
            commands::search_packages,
            commands::package_details,
            commands::add_project,
            commands::create_task,
            commands::configure_task,
            commands::open_task,
            commands::list_commands,
            commands::execute_command,
            commands::init_agents,
            commands::compact_task,
            commands::prompt,
            commands::resend_message,
            commands::navigate_task,
            commands::restore_checkpoint,
            commands::checkpoint_changes,
            commands::fork_task,
            commands::set_task_mode,
            commands::export_plan,
            commands::stop_task,
            commands::archive_task,
            commands::unarchive_task,
            commands::rename_task,
            commands::delete_task,
            commands::convert_task_to_worktree,
            commands::remove_project,
            commands::git_changes,
            commands::list_workspace_files,
            commands::reveal_task,
            commands::reveal_path,
        ])
        .build(tauri::generate_context!())
        .expect("error while building WackCode");

    app.run(|app, event| {
        if matches!(event, tauri::RunEvent::Exit | tauri::RunEvent::ExitRequested { .. }) {
            app.state::<WorkerState>().terminate_all();
            app.state::<subscriptions::SubscriptionState>().terminate_all();
        }
    });
}
