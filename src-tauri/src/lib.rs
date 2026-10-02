mod backgrounds;
mod browser;
mod checkpoints;
mod commands;
mod computer_use;
mod files;
mod git;
mod glass;
mod history;
mod mcp;
mod memory;
mod menu_bar;
mod models;
mod pty_output;
mod run_command;
mod secrets;
mod shell_env;
mod skill_archive;
mod skills;
mod slash_commands;
mod storage;
mod subagents;
mod subscriptions;
mod terminal;
mod window_state;
mod worker;
mod usage;

use storage::MetadataState;
use tauri::Manager;
use worker::WorkerState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(computer_use::shortcut_plugin())
        .manage(WorkerState::default())
        .manage(usage::UsageState::default())
        .manage(browser::BrowserManager::default())
        .manage(computer_use::ComputerUseManager::default())
        .manage(worker::SelectedTask::default())
        .manage(worker::WorkerActivity::default())
        .manage(commands::TaskLocks::default())
        .manage(commands::GitLocks::default())
        .manage(commands::GitNetworkLocks::default())
        .manage(subscriptions::SubscriptionState::default())
        .manage(worker::ManagerState::default())
        .manage(terminal::TerminalState::default())
        .manage(run_command::RunState::default())
        .manage(menu_bar::MenuBarState::default())
        .setup(|app| {
            let state = MetadataState::load(&app.handle())?;
            let (appearance, saved_window) = state
                .data
                .lock()
                .map_err(|_| "Metadata lock was poisoned".to_string())
                .map(|data| (data.appearance.clone(), data.window))?;
            app.manage(state);
            usage::start_retry(app.handle().clone());
            menu_bar::setup(app)?;
            // The window is created hidden and transparent: paint it before it first appears.
            glass::apply(
                app.handle(),
                glass::NativeBackdrop::from_config(&appearance),
                true,
            )?;
            if let Some(window) = app.get_webview_window("main") {
                if let Some(saved_window) = saved_window {
                    window_state::restore(&window, saved_window);
                }
                window.show()?;
            }
            // Read the login-shell environment in the background, so the first chat doesn't wait.
            tauri::async_runtime::spawn(shell_env::warm());
            // Stops idle chat workers so clicking through old chats does not pile up processes.
            let reaper = worker::start_idle_reaper(app.handle().clone());
            app.manage(worker::ReaperHandle::default());
            app.state::<worker::ReaperHandle>().install(reaper);
            Ok(())
        })
        .on_window_event(|window, event| {
            window_state::record(window, event);
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
                return;
            }
            // Liquid Glass shows only while focused; unfocused, the window is painted opaque.
            if let tauri::WindowEvent::Focused(focused) = event {
                let app = window.app_handle();
                let Ok(data) = app
                    .state::<MetadataState>()
                    .data
                    .lock()
                    .map(|data| data.appearance.clone())
                else {
                    return;
                };
                if data.backdrop == models::BackdropMode::Glass {
                    let _ = glass::apply(app, glass::NativeBackdrop::from_config(&data), *focused);
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::bootstrap,
            commands::app_info,
            usage::usage_status,
            usage::set_usage_recording,
            commands::save_provider,
            commands::set_provider_enabled,
            commands::set_model_favorite,
            commands::delete_provider,
            commands::discover_models,
            commands::list_builtin_models,
            subscriptions::list_subscription_providers,
            subscriptions::start_subscription_login,
            subscriptions::respond_subscription_login,
            subscriptions::cancel_subscription_login,
            subscriptions::sign_out_subscription,
            subscriptions::open_subscription_auth_url,
            subscriptions::refresh_subscription_models,
            commands::set_tool_config,
            commands::set_appearance_config,
            commands::choose_background_image,
            commands::remove_background_image,
            commands::set_subagent_config,
            commands::set_auto_title_config,
            commands::set_prompt_config,
            commands::save_mcp_server,
            commands::delete_mcp_server,
            commands::set_mcp_server_enabled,
            commands::set_mcp_server_tools,
            commands::test_mcp_server,
            commands::list_skills,
            commands::read_skill,
            commands::save_skill,
            commands::delete_skill,
            commands::set_skill_enabled,
            commands::set_skill_folder_enabled,
            commands::add_skill_folder,
            commands::remove_skill_folder,
            commands::import_skill,
            commands::copy_skill_to_library,
            commands::search_skill_packages,
            commands::list_slash_commands,
            commands::read_slash_command,
            commands::save_slash_command,
            commands::delete_slash_command,
            commands::set_slash_command_enabled,
            commands::list_memories,
            commands::read_memory,
            commands::save_memory,
            commands::delete_memory,
            commands::remove_memory_project,
            commands::set_memory_config,
            commands::set_project_memory_enabled,
            commands::find_memory_in_finder,
            commands::list_packages,
            commands::refresh_packages,
            commands::install_package,
            commands::trust_package,
            commands::remove_package,
            commands::update_packages,
            commands::set_package_resources,
            commands::respond_extension_ui,
            browser::browser_state,
            browser::browser_present,
            browser::browser_open,
            browser::browser_navigation,
            browser::browser_set_control,
            browser::browser_reset,
            browser::browser_return_from_popup,
            commands::search_packages,
            commands::package_details,
            commands::add_project,
            commands::create_task,
            commands::configure_task,
            commands::open_task,
            commands::watch_subagent,
            commands::list_draft_commands,
            commands::list_commands,
            commands::execute_command,
            commands::init_agents,
            commands::compact_task,
            commands::goal_control,
            commands::prompt,
            commands::queue_message,
            commands::dequeue_messages,
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
            commands::clear_task_error,
            commands::delete_task,
            commands::convert_task_to_worktree,
            commands::remove_project,
            commands::git_changes,
            commands::git_change_action,
            commands::git_commit,
            commands::set_diff_comments,
            commands::git_publish_info,
            commands::git_push,
            commands::git_branches,
            commands::git_checkout,
            commands::git_pr_prepare,
            commands::git_pr_create,
            commands::git_generate_message,
            commands::git_sync_status,
            commands::git_remote_url,
            commands::git_fetch,
            commands::git_pull,
            commands::git_log,
            commands::git_commit_files,
            commands::git_commit_diff,
            commands::git_undo_commit,
            commands::git_revert_commit,
            commands::list_workspace_files,
            commands::reveal_task,
            commands::reveal_path,
            commands::list_editors,
            commands::open_in_editor,
            terminal::open_terminal,
            terminal::write_terminal,
            terminal::resize_terminal,
            terminal::detach_terminal,
            terminal::restart_terminal,
            terminal::close_terminal,
            run_command::save_project_run_command,
            run_command::start_run,
            run_command::stop_run,
            run_command::get_run,
            run_command::attach_run,
            run_command::detach_run,
            run_command::write_run,
            run_command::resize_run,
            menu_bar::take_menu_navigation,
            commands::tool_image,
            commands::message_image,
            computer_use::computer_use_status,
            computer_use::computer_use_request_permission,
            computer_use::computer_use_open_settings,
            computer_use::computer_use_reset_permissions,
            computer_use::computer_use_relaunch,
            computer_use::set_computer_use_config,
            computer_use::computer_use_respond_access,
            computer_use::computer_use_list_apps,
        ])
        .build(tauri::generate_context!())
        .expect("error while building WackCode");

    app.run(|app, event| {
        if matches!(event, tauri::RunEvent::Reopen { .. }) {
            menu_bar::show_main_window(app);
        }
        if matches!(
            event,
            tauri::RunEvent::Exit | tauri::RunEvent::ExitRequested { .. }
        ) {
            cleanup_before_exit(app);
        }
    });
}

/// Everything that must happen before the process exits: on quit, and before a relaunch.
pub(crate) fn cleanup_before_exit(app: &tauri::AppHandle) {
    // Flushes the window geometry `window_state::record` only kept in memory.
    let _ = app.state::<MetadataState>().save();
    app.state::<worker::ReaperHandle>().stop();
    app.state::<WorkerState>().terminate_all();
    app.state::<browser::BrowserManager>().dispose_all();
    computer_use::dispose_all(app);
    app.state::<terminal::TerminalState>().terminate_all();
    app.state::<run_command::RunState>().terminate_all();
    app.state::<subscriptions::SubscriptionState>()
        .terminate_all();
}
