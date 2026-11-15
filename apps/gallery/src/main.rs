//! Renders UI scenes to PNG files without opening a window.
//!
//! This is how UI work is checked: change a view, render its scene, look at the image. Scenes are
//! defined in `wack_ui::scenes`. Rendering is headless (GPUI's test platform with the real Metal
//! renderer and the real macOS text system), so it works from any shell and steals no focus.
//!
//! ```text
//! cargo snap --list
//! cargo snap new-chat --theme dark --out /tmp/new-chat.png
//! cargo snap chat --colour '#2f5bd6' --accent '#ff8fb8'   # any theme colour and accent
//! cargo snap chat --tags                  # also print each tagged element's bounds
//! cargo snap --all --out-dir target/gallery
//! ```

use std::{path::PathBuf, process::ExitCode, sync::Arc};

use gpui::{AppContext, HeadlessAppContext};
use wack_ui::{
    icon::Assets,
    scenes::{self, Scene, SceneRoot},
    tags,
    theme::{Appearance, DEFAULT_ACCENT, DEFAULT_THEME_COLOUR, Theme, ThemeInput},
};

const USAGE: &str = "usage: gallery --list
       gallery <scene> [--theme light|dark] [--colour <#hex>] [--accent <#hex>] [--out <file.png>] [--tags]
       gallery --all [--colour <#hex>] [--accent <#hex>] [--out-dir <dir>] [--tags]";

fn main() -> ExitCode {
    match run(std::env::args().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: Vec<String>) -> Result<(), String> {
    let mut scene_name = None;
    let mut all = false;
    let mut list = false;
    let mut print_tags = false;
    let mut theme = Appearance::Dark;
    let mut theme_colour = DEFAULT_THEME_COLOUR.to_string();
    let mut accent = DEFAULT_ACCENT.to_string();
    let mut out = None;
    let mut out_dir = PathBuf::from("target/gallery");

    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        let mut value = |flag: &str| args.next().ok_or(format!("{flag} needs a value.\n{USAGE}"));
        match arg.as_str() {
            "--list" => list = true,
            "--all" => all = true,
            "--tags" => print_tags = true,
            "--theme" => {
                theme = match value("--theme")?.as_str() {
                    "light" => Appearance::Light,
                    "dark" => Appearance::Dark,
                    other => return Err(format!("Unknown theme \"{other}\": use light or dark.")),
                }
            }
            "--colour" => theme_colour = value("--colour")?,
            "--accent" => accent = value("--accent")?,
            "--out" => out = Some(PathBuf::from(value("--out")?)),
            "--out-dir" => out_dir = PathBuf::from(value("--out-dir")?),
            flag if flag.starts_with("--") => {
                return Err(format!("Unknown option {flag}.\n{USAGE}"));
            }
            name => scene_name = Some(name.to_string()),
        }
    }

    if list {
        for scene in scenes::all() {
            println!("{:<24} {}", scene.name, scene.description);
        }
        return Ok(());
    }

    let mut renders: Vec<(Scene, Appearance, PathBuf)> = Vec::new();
    if all {
        for appearance in [Appearance::Light, Appearance::Dark] {
            for scene in scenes::all() {
                let path = out_dir.join(file_name(&scene, appearance));
                renders.push((scene, appearance, path));
            }
        }
    } else {
        let name = scene_name.ok_or(USAGE)?;
        let scene = scenes::find(&name).ok_or(format!(
            "No scene named \"{name}\". Run with --list to see them."
        ))?;
        let path = out.unwrap_or_else(|| out_dir.join(file_name(&scene, theme)));
        renders.push((scene, theme, path));
    }

    // One headless app for every render: the Metal renderer and text system are costly to start.
    let text_system = gpui_platform::current_platform(true).text_system();
    let mut cx = HeadlessAppContext::with_platform(
        text_system,
        Arc::new(Assets),
        gpui_platform::current_headless_renderer,
    );
    cx.update(wack_ui::init);
    // A snapshot shows where every movement comes to rest, never a frame mid-way.
    cx.update(|cx| cx.set_reduce_motion(true));

    for (scene, appearance, path) in renders {
        let input = ThemeInput {
            appearance,
            theme_colour: theme_colour.clone(),
            accent: accent.clone(),
            ground: None,
        };
        cx.update(|cx| cx.set_global(Theme::derive(input)));
        let window = cx
            .open_window(scene.size, |window, cx| {
                let view = scene.build(window, cx);
                cx.new(|_| SceneRoot::new(view))
            })
            .map_err(|error| format!("Could not open a window for \"{}\": {error}", scene.name))?;
        // Pictures load while the first frame draws; once they have, draw again so they show.
        for _ in 0..2 {
            cx.run_until_parked();
            cx.update_window(window.into(), |_, window, cx| {
                window.refresh();
                window.draw(cx).clear();
            })
            .map_err(|error| format!("Could not draw \"{}\": {error}", scene.name))?;
        }
        let image = cx
            .capture_screenshot(window.into())
            .map_err(|error| format!("Could not render \"{}\": {error}", scene.name))?;
        let tagged = cx
            .update_window(window.into(), |_, window, cx| {
                let tagged = tags::tags(window, cx);
                window.remove_window();
                tagged
            })
            .map_err(|error| {
                format!("Could not close the window for \"{}\": {error}", scene.name)
            })?;

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("Could not create {}: {error}", parent.display()))?;
        }
        image
            .save(&path)
            .map_err(|error| format!("Could not write {}: {error}", path.display()))?;
        println!("{}", path.display());
        if print_tags {
            for tag in tagged {
                let (origin, size) = (tag.bounds.origin, tag.bounds.size);
                println!(
                    "  {:<24} x {:>6.1}  y {:>6.1}  {:>6.1} x {:<6.1}",
                    tag.name,
                    f32::from(origin.x),
                    f32::from(origin.y),
                    f32::from(size.width),
                    f32::from(size.height)
                );
            }
        }
    }
    Ok(())
}

fn file_name(scene: &Scene, appearance: Appearance) -> String {
    let theme = match appearance {
        Appearance::Light => "light",
        Appearance::Dark => "dark",
    };
    format!("{}-{theme}.png", scene.name)
}
