//! The WackCode app: opens the window and hands it to `wack-ui`.

use std::{cell::RefCell, rc::Rc};

use gpui::{
    App, KeyBinding, Menu, MenuItem, SystemMenuType, TitlebarOptions, WeakEntity, WindowOptions, actions,
    point, prelude::*, px,
};
use wack_ui::{
    actions::{
        AddFolder, ArchiveCurrentChat, CloseTab, CopyScratchpadPath, EnterChatArea, EnterCodeArea, NewChat, NewTerminal, NextTab, OpenSettings, PreviousTab, RenameChat,
        RevealScratchpad, ToggleChanges, ToggleSidebar, ToggleTerminal,
    },
    app::AppState,
    shell::{Shell, TRAFFIC_LIGHTS},
    text_field, window_state,
};

actions!(wackcode, [Quit]);

fn main() {
    // Before any other thread: loading Chromium's library swaps the process's allocator (it
    // starts nothing; the browser starts on first use).
    wack_browser::cef::load_library();
    // Opened from the Dock, the app has launchd's bare PATH: read the user's login environment
    // now, for `bash` and MCP servers (`npx`, `uvx`).
    wack_core::shell_env::warm();
    gpui_platform::application().with_assets(wack_ui::icon::Assets).run(|cx: &mut App| {
        wack_ui::init(cx);

        let shell_ref: Rc<RefCell<Option<WeakEntity<Shell>>>> = Rc::new(RefCell::new(None));
        let shutdown_shell = |shell_ref: &Rc<RefCell<Option<WeakEntity<Shell>>>>, cx: &mut App| {
            if let Some(shell) = shell_ref.borrow().as_ref().and_then(WeakEntity::upgrade) {
                shell.update(cx, |shell, cx| shell.shutdown(cx));
            }
        };

        cx.on_action({
            let shell_ref = shell_ref.clone();
            move |_: &Quit, cx| {
                shutdown_shell(&shell_ref, cx);
                cx.quit();
            }
        });
        cx.bind_keys([KeyBinding::new("cmd-q", Quit, None)]);
        cx.set_menus([
            Menu::new("WackCode").items([
                MenuItem::action("Settings…", OpenSettings),
                MenuItem::separator(),
                MenuItem::os_submenu("Services", SystemMenuType::Services),
                MenuItem::separator(),
                MenuItem::action("Quit WackCode", Quit),
            ]),
            Menu::new("File").items([
                MenuItem::action("New Chat", NewChat),
                MenuItem::action("Add Folder…", AddFolder),
                MenuItem::separator(),
                MenuItem::action("Rename Chat…", RenameChat),
                MenuItem::action("Archive Chat", ArchiveCurrentChat),
                MenuItem::separator(),
                // A Chat chat's own folder; nothing in a Code chat.
                MenuItem::action("Reveal Scratchpad in Finder", RevealScratchpad),
                MenuItem::action("Copy Scratchpad Path", CopyScratchpadPath),
                MenuItem::separator(),
                MenuItem::action("Close Tab", CloseTab),
            ]),
            Menu::new("Edit").items([
                MenuItem::action("Undo", text_field::Undo),
                MenuItem::action("Redo", text_field::Redo),
                MenuItem::separator(),
                MenuItem::action("Cut", text_field::Cut),
                MenuItem::action("Copy", text_field::Copy),
                MenuItem::action("Paste", text_field::Paste),
                MenuItem::action("Select All", text_field::SelectAll),
            ]),
            Menu::new("View").items([
                MenuItem::action("Code", EnterCodeArea),
                MenuItem::action("Chat", EnterChatArea),
                MenuItem::separator(),
                MenuItem::action("Sidebar", ToggleSidebar),
                MenuItem::action("Changes", ToggleChanges),
                MenuItem::action("Terminal", ToggleTerminal),
                MenuItem::action("New Terminal", NewTerminal),
                MenuItem::separator(),
                MenuItem::action("Next Tab", NextTab),
                MenuItem::action("Previous Tab", PreviousTab),
            ]),
        ]);
        cx.on_window_closed({
            let shell_ref = shell_ref.clone();
            move |cx, _| {
                if cx.windows().is_empty() {
                    shutdown_shell(&shell_ref, cx);
                    cx.quit();
                }
            }
        })
        .detach();

        let opened = AppState::open_store();
        let saved_window = opened.as_ref().ok().and_then(|store| store.ui().window.as_ref());
        let (display_id, window_bounds) = window_state::restore(saved_window, cx);
        let options = WindowOptions {
            display_id,
            window_bounds: Some(window_bounds),
            titlebar: Some(TitlebarOptions {
                title: Some("WackCode".into()),
                appears_transparent: true,
                traffic_light_position: Some(point(px(TRAFFIC_LIGHTS.0), px(TRAFFIC_LIGHTS.1))),
            }),
            // The shell draws the title strip and moves the window itself (`wack_ui::shell`).
            app_owns_titlebar_drag: true,
            // The solver keeps the transcript readable down to this size (`panes::solve`),
            // and the frame's gutters go round that.
            window_min_size: Some(window_state::min_size()),
            ..Default::default()
        };
        if let Err(error) = cx.open_window(options, |window, cx| {
            wack_ui::theme::follow_system_appearance(window);
            let shell = cx.new(|cx| Shell::with_store(opened, window, cx));
            *shell_ref.borrow_mut() = Some(shell.downgrade());
            shell
        }) {
            eprintln!("WackCode could not open its window: {error}");
            cx.quit();
            return;
        }
        cx.activate(true);

        #[cfg(feature = "automation")]
        wack_ui::automation::start(cx);
    });
}
