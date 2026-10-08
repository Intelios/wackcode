//! Native menu accelerators reach the renderer even while a child WKWebView owns
//! keyboard focus. They are app-local (never global shortcuts). App owns navigation
//! and synchronizes availability; this module never creates or stops a worker.
use serde::Deserialize;
use std::sync::Mutex;
use tauri::{
    menu::{Menu, MenuItem, MenuItemKind, Submenu},
    AppHandle, Emitter, Manager, State,
};

const PREFIX: &str = "chat-tab:";

pub struct TabMenuState {
    items: Vec<(String, MenuItem<tauri::Wry>)>,
    availability: Mutex<TabMenuAvailability>,
}

#[derive(Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TabMenuAvailability {
    tabs_enabled: bool,
    enabled: bool,
    tab_count: usize,
    can_reopen: bool,
}

pub fn setup(app: &AppHandle) -> Result<(), String> {
    let menu = Menu::default(app).map_err(|e| format!("Could not create the app menu: {e}"))?;
    // The Tabs menu owns Cmd+W. Window close still hides the app through the existing
    // CloseRequested handler. Remove the default accelerator to avoid two owners.
    for item in menu.items().map_err(|e| e.to_string())? {
        if let MenuItemKind::Submenu(submenu) = item {
            for child in submenu.items().map_err(|e| e.to_string())? {
                if let MenuItemKind::Predefined(predefined) = &child {
                    if predefined.text().map_err(|e| e.to_string())? == "Close Window" {
                        submenu.remove(&child).map_err(|e| e.to_string())?;
                    }
                }
            }
        }
    }
    let submenu = Submenu::new(app, "Tabs", true).map_err(|e| e.to_string())?;
    let mut items = Vec::new();
    let commands = [
        ("new", "New Chat Tab", Some("Cmd+N")),
        ("close", "Close Window", Some("Cmd+W")),
        ("reopen", "Reopen Closed Tab", None),
        ("next", "Next Tab", Some("Ctrl+Tab")),
        ("previous", "Previous Tab", Some("Ctrl+Shift+Tab")),
    ];
    for (command, title, accelerator) in commands {
        let item = MenuItem::with_id(
            app,
            format!("{PREFIX}{command}"),
            title,
            command == "close",
            accelerator,
        )
        .map_err(|e| format!("Could not create the tab menu: {e}"))?;
        submenu.append(&item).map_err(|e| e.to_string())?;
        items.push((command.to_string(), item));
    }
    for index in 1..=9 {
        let command = format!("select-{index}");
        let item = MenuItem::with_id(
            app,
            format!("{PREFIX}{command}"),
            if index == 9 {
                "Last Tab".to_string()
            } else {
                format!("Tab {index}")
            },
            false,
            Some(format!("Cmd+{index}")),
        )
        .map_err(|e| e.to_string())?;
        submenu.append(&item).map_err(|e| e.to_string())?;
        items.push((command, item));
    }
    menu.append(&submenu).map_err(|e| e.to_string())?;
    app.set_menu(menu).map_err(|e| e.to_string())?;
    app.manage(TabMenuState {
        items,
        availability: Mutex::new(TabMenuAvailability::default()),
    });
    app.on_menu_event(|app, event| {
        let Some(command) = event.id().as_ref().strip_prefix(PREFIX) else {
            return;
        };
        let state = app.state::<TabMenuState>();
        let Ok(availability) = state.availability.lock() else {
            return;
        };
        if command == "close" && !availability.tabs_enabled {
            if let Some(window) = app.get_window("main") {
                let _ = window.hide();
            }
        } else if availability.enabled || (command == "new" && !availability.tabs_enabled) {
            let _ = app.emit("native-tab-action", command);
        }
    });
    Ok(())
}

fn action_enabled(command: &str, input: &TabMenuAvailability) -> bool {
    if (command == "close" || command == "new") && !input.tabs_enabled {
        return true;
    }
    if !input.enabled {
        return false;
    }
    match command {
        "reopen" => input.can_reopen,
        "next" | "previous" => input.tab_count > 1,
        _ if command.starts_with("select-") => command[7..].parse::<usize>().is_ok_and(|index| {
            if index == 9 {
                input.tab_count > 0
            } else {
                index <= input.tab_count
            }
        }),
        _ => true,
    }
}

#[tauri::command]
pub fn set_chat_tab_menu(
    state: State<'_, TabMenuState>,
    input: TabMenuAvailability,
) -> Result<(), String> {
    // Publish availability before scheduling AppKit changes. Do not hold this mutex
    // across main-thread work: an intervening menu event also reads it.
    *state
        .availability
        .lock()
        .map_err(|_| "Could not update the tab menu.".to_string())? = input.clone();
    for (command, item) in &state.items {
        if command == "close" {
            item.set_text(if input.tabs_enabled {
                "Close Tab"
            } else {
                "Close Window"
            })
            .map_err(|e| e.to_string())?;
        }
        if command == "new" {
            item.set_text(if input.tabs_enabled {
                "New Chat Tab"
            } else {
                "New Chat"
            })
            .map_err(|e| e.to_string())?;
        }
        item.set_enabled(action_enabled(command, &input))
            .map_err(|e| format!("Could not update the tab menu: {e}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shortcuts_follow_workspace_and_modal_availability() {
        let mut state = TabMenuAvailability::default();
        assert!(action_enabled("close", &state));
        assert!(action_enabled("new", &state));
        state.tabs_enabled = true;
        assert!(!action_enabled("close", &state));
        state.enabled = true;
        state.tab_count = 2;
        assert!(action_enabled("next", &state));
        assert!(action_enabled("select-2", &state));
        assert!(!action_enabled("select-3", &state));
        assert!(action_enabled("select-9", &state));
        assert!(!action_enabled("reopen", &state));
        state.can_reopen = true;
        assert!(action_enabled("reopen", &state));
        state.enabled = false;
        assert!(!action_enabled("reopen", &state));
    }
}
