//! The WackCode app: opens the window and hands it to `wack-ui`.

use gpui::{
    App, Bounds, KeyBinding, Menu, MenuItem, SystemMenuType, TitlebarOptions, WindowBounds,
    WindowOptions, actions, point, prelude::*, px, size,
};
use wack_ui::shell::Shell;

actions!(wackcode, [Quit]);

fn main() {
    gpui_platform::application().run(|cx: &mut App| {
        wack_ui::init(cx);

        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.bind_keys([KeyBinding::new("cmd-q", Quit, None)]);
        cx.set_menus([Menu::new("WackCode").items([
            MenuItem::os_submenu("Services", SystemMenuType::Services),
            MenuItem::separator(),
            MenuItem::action("Quit WackCode", Quit),
        ])]);
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();

        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(1180.), px(760.)),
                cx,
            ))),
            titlebar: Some(TitlebarOptions {
                title: Some("WackCode".into()),
                appears_transparent: true,
                traffic_light_position: Some(point(px(14.), px(13.))),
            }),
            // The shell draws the title strip and moves the window itself (`wack_ui::shell`).
            app_owns_titlebar_drag: true,
            window_min_size: Some(size(px(720.), px(480.))),
            ..Default::default()
        };
        if let Err(error) = cx.open_window(options, |window, cx| {
            wack_ui::theme::follow_system_appearance(window);
            cx.new(|cx| Shell::new(window, cx))
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
