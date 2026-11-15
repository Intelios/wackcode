//! Renders UI scenes to PNG files without opening a window.
//!
//! This is how UI work is checked: change a view, render its scene, look at the image. Scenes are
//! defined in `wack_ui::scenes`. Rendering is headless (GPUI's test platform with the real Metal
//! renderer and the real macOS text system), so it works from any shell and steals no focus.
//!
//! ```text
//! cargo snap --list
//! cargo snap new-chat --theme dark --out /tmp/new-chat.png
//! cargo snap chat --look nebula          # any of the ten themes (WackCode by default)
//! cargo snap settings-appearance --look all --theme light   # the scene in every theme
//! cargo snap chat --tags                  # also print each tagged element's bounds
//! cargo snap --all --out-dir target/gallery
//! ```

use std::{path::PathBuf, process::ExitCode, sync::Arc};

use gpui::{AppContext, HeadlessAppContext};
use wack_ui::{
    icon::Assets,
    scenes::{self, Scene, SceneRoot},
    tags,
    theme::{Appearance, Theme, ThemeInput, ThemeName},
};

const USAGE: &str = "usage: gallery --list
       gallery <scene> [--theme light|dark] [--look <theme>|all] [--out <file.png>] [--tags]
       gallery --all [--look <theme>] [--out-dir <dir>] [--tags]
themes: wackcode midnight lagoon verdant grape nebula rose solar ember mono";

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
    // Which themes to draw in; more than one only for a single scene.
    let mut looks = vec![ThemeName::default()];
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
            "--look" => {
                looks = match value("--look")?.as_str() {
                    "all" => ThemeName::ALL.to_vec(),
                    id => match ThemeName::ALL.into_iter().find(|look| look.id() == id) {
                        Some(look) => vec![look],
                        None => return Err(format!("Unknown look \"{id}\".\n{USAGE}")),
                    },
                }
            }
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

    let mut renders: Vec<(Scene, Appearance, ThemeName, PathBuf)> = Vec::new();
    if all {
        if looks.len() > 1 {
            return Err(format!("--all draws in one look at a time.\n{USAGE}"));
        }
        for appearance in [Appearance::Light, Appearance::Dark] {
            for scene in scenes::all() {
                let path = out_dir.join(file_name(&scene, appearance, looks[0]));
                renders.push((scene, appearance, looks[0], path));
            }
        }
    } else {
        let name = scene_name.ok_or(USAGE)?;
        if out.is_some() && looks.len() > 1 {
            return Err(format!("--out names one file; use --out-dir with --look all.\n{USAGE}"));
        }
        for look in looks {
            let scene = scenes::find(&name).ok_or(format!(
                "No scene named \"{name}\". Run with --list to see them."
            ))?;
            let path = out.clone().unwrap_or_else(|| out_dir.join(file_name(&scene, theme, look)));
            renders.push((scene, theme, look, path));
        }
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

    for (scene, appearance, look, path) in renders {
        let input = ThemeInput { appearance, theme: look, ground: None };
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

/// `<scene>-<light|dark>.png`, with the look between them when it is not the default.
fn file_name(scene: &Scene, appearance: Appearance, look: ThemeName) -> String {
    let theme = match appearance {
        Appearance::Light => "light",
        Appearance::Dark => "dark",
    };
    if look == ThemeName::default() {
        format!("{}-{theme}.png", scene.name)
    } else {
        format!("{}-{}-{theme}.png", scene.name, look.id())
    }
}
