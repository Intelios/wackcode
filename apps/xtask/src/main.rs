//! `cargo xtask`: builds WackCode's `.app` bundles.
//!
//! - `cargo dev [args…]` (an alias for `cargo xtask dev`): builds the app with the automation
//!   socket and the CEF helper, writes `target/debug/WackCode Dev.app`, and runs it in place of
//!   this process, so the terminal, Ctrl-C and `WACKCODE_AUTOMATION` behave as `cargo run` did.
//! - `cargo xtask bundle`: a release build in `target/release/WackCode.app`, signed ad hoc
//!   (notarization is out of scope).
//!
//! The browser engine (CEF) only works from inside a bundle, which is why `cargo dev` no longer
//! runs the bare binary. See docs/cef.md.

mod bundle;

use std::{
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::Command,
};

use bundle::{Flavour, Sources};

fn main() {
    let mut args = std::env::args().skip(1);
    let result = match args.next().as_deref() {
        Some("dev") => dev(args.collect()),
        Some("bundle") => release_bundle(),
        _ => Err("Usage: cargo xtask dev [app args…] | cargo xtask bundle".to_string()),
    };
    if let Err(message) = result {
        eprintln!("xtask: {message}");
        std::process::exit(1);
    }
}

fn workspace_root() -> PathBuf {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    root.canonicalize().unwrap_or(root)
}

fn target_dir() -> PathBuf {
    std::env::var_os("CARGO_TARGET_DIR").map(PathBuf::from).unwrap_or_else(|| workspace_root().join("target"))
}

fn cargo_build(release: bool, features: &[&str]) -> Result<(), String> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let mut command = Command::new(cargo);
    command.current_dir(workspace_root()).args(["build", "-p", "wackcode", "-p", "wackcode-helper"]);
    if release {
        command.arg("--release");
    }
    if !features.is_empty() {
        command.args(["--features", &features.join(",")]);
    }
    let status = command.status().map_err(|e| format!("Could not run cargo: {e}"))?;
    if status.success() { Ok(()) } else { Err("The build failed.".into()) }
}

fn sources(profile_dir: &Path, flavour: Flavour) -> Result<Sources, String> {
    let cef = cef_dll_sys::get_cef_dir()
        .ok_or("The CEF distribution was not found. Build once with `cargo build` so it downloads, and check CEF_PATH in .cargo/config.toml.")?;
    Ok(Sources {
        app: profile_dir.join("wackcode"),
        helper: profile_dir.join("wackcode-helper"),
        icon: workspace_root().join("apps/wackcode/icons").join(flavour.icon()),
        // The model catalogue's snapshot is models.dev's data (MIT).
        notices: vec![workspace_root().join("crates/wack-catalog/data/LICENSE.models.dev")],
        cef,
        version: bundle::plist_version(env!("CARGO_PKG_VERSION")),
    })
}

fn dev(app_args: Vec<String>) -> Result<(), String> {
    cargo_build(false, &["wackcode/automation"])?;
    let profile_dir = target_dir().join("debug");
    let app = profile_dir.join(format!("{}.app", Flavour::Dev.bundle_name()));
    let executable = bundle::assemble(&app, Flavour::Dev, &sources(&profile_dir, Flavour::Dev)?)?;
    let error = Command::new(&executable).args(app_args).exec();
    Err(format!("Could not start {}: {error}", executable.display()))
}

fn release_bundle() -> Result<(), String> {
    cargo_build(true, &[])?;
    let profile_dir = target_dir().join("release");
    let app = profile_dir.join(format!("{}.app", Flavour::Release.bundle_name()));
    bundle::assemble(&app, Flavour::Release, &sources(&profile_dir, Flavour::Release)?)?;

    // Sign inside out: helpers and the framework first, the app last.
    let frameworks = app.join("Contents/Frameworks");
    let mut parts: Vec<PathBuf> = bundle::HELPERS
        .iter()
        .map(|(suffix, _)| frameworks.join(format!("{} Helper{suffix}.app", bundle::EXECUTABLE)))
        .collect();
    parts.push(frameworks.join(bundle::FRAMEWORK));
    parts.push(app.clone());
    for part in parts {
        let status = Command::new("codesign")
            .args(["--force", "--sign", "-"])
            .arg(&part)
            .status()
            .map_err(|e| format!("Could not run codesign: {e}"))?;
        if !status.success() {
            return Err(format!("codesign failed for {}", part.display()));
        }
    }
    println!("{}", app.display());
    Ok(())
}
