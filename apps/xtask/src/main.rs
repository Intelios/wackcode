//! `cargo xtask`: builds WackCode's `.app` bundles.
//!
//! - `cargo dev [--smooth] [args…]` (an alias for `cargo xtask dev`): builds the app with the
//!   automation socket and the CEF helper, writes `target/debug/WackCode Dev.app`, and runs it in
//!   place of this process, so the terminal, Ctrl-C and `WACKCODE_AUTOMATION` behave as `cargo run`
//!   did. The socket's screenshots need GPUI's test mode, which draws after every event instead of
//!   once a refresh; `--smooth` leaves it (and screenshots) out, so the app draws as a release
//!   build does. Use it to judge how scrolling and motion feel (docs/testing.md).
//! - `cargo xtask bundle`: a release build in `target/release/WackCode.app`, signed ad hoc
//!   (notarization is out of scope).
//!
//! The browser engine (CEF) only works from inside a bundle, which is why `cargo dev` no longer
//! runs the bare binary; `bundle.rs` holds the layout and its invariants.

mod bundle;

use std::{
    fs,
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::Command,
};

use bundle::{Flavour, Notice, Sources};

fn main() {
    let mut args = std::env::args().skip(1);
    let result = match args.next().as_deref() {
        Some("dev") => dev(args.collect()),
        Some("bundle") => release_bundle(),
        _ => Err("Usage: cargo xtask dev [--smooth] [app args…] | cargo xtask bundle".to_string()),
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
    let mut notices = vec![
        // The model catalogue's snapshot is models.dev's data (MIT).
        Notice::File { from: workspace_root().join("crates/wack-catalog/data/LICENSE.models.dev"), name: "LICENSE.models.dev" },
    ];
    notices.extend(cef_notices(&cef, flavour)?);
    Ok(Sources {
        app: profile_dir.join("wackcode"),
        helper: profile_dir.join("wackcode-helper"),
        icon: workspace_root().join("apps/wackcode/icons").join(flavour.icon()),
        notices,
        cef,
        version: bundle::plist_version(env!("CARGO_PKG_VERSION")),
    })
}

/// The browser's licence notices: CEF's BSD licence and Chromium's credits, which shipping the
/// framework in binary form requires. The credits are in the distribution; the licence exists
/// only inside the archive it arrived in (the extraction keeps the credits but deletes the
/// licence), so it is read back out of that. A release bundle is not written without them; a
/// dev bundle warns and ships what it has.
fn cef_notices(cef: &Path, flavour: Flavour) -> Result<Vec<Notice>, String> {
    let credits = cef.join("CREDITS.html");
    let mut notices = Vec::new();
    let mut missing = Vec::new();
    match cef_licence_text(cef) {
        Ok(contents) => notices.push(Notice::Text { name: "LICENSE.cef.txt", contents }),
        Err(message) => missing.push(message),
    }
    if credits.is_file() {
        notices.push(Notice::File { from: credits, name: "CREDITS.chromium.html" });
    } else {
        missing.push(format!(
            "Chromium's credits are missing: {} was not found, so the CEF distribution is incomplete.",
            credits.display()
        ));
    }
    if !missing.is_empty() {
        let message = missing.join(" ");
        match flavour {
            Flavour::Release => return Err(message),
            Flavour::Dev => eprintln!("xtask: {message}"),
        }
    }
    Ok(notices)
}

/// CEF's `LICENSE.txt`, read out of the archive the distribution arrived in: the archive sits
/// beside the extracted folder, and the directory inside it is named after the archive itself.
fn cef_licence_text(cef: &Path) -> Result<String, String> {
    let location = cef.parent().ok_or("The CEF distribution has no parent folder.")?;
    let mut archives: Vec<PathBuf> = fs::read_dir(location)
        .map_err(|e| format!("Could not read {}: {e}", location.display()))?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && path.file_name().is_some_and(|name| name.to_string_lossy().ends_with(".tar.bz2")))
        .collect();
    let archive = match archives.len() {
        1 => archives.remove(0),
        0 => {
            return Err(format!(
                "CEF's licence could not be read: the archive the distribution came from is not in {}. Delete {} and build once so CEF downloads again, then bundle.",
                location.display(),
                location.display()
            ))
        }
        _ => {
            return Err(format!(
                "CEF's licence could not be read: more than one archive is in {}.",
                location.display()
            ))
        }
    };
    let folder = archive
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_suffix(".tar.bz2"))
        .ok_or_else(|| format!("{} does not look like a CEF archive.", archive.display()))?;
    let member = format!("{folder}/LICENSE.txt");
    let output = Command::new("tar")
        .arg("-xOf")
        .arg(&archive)
        .arg(&member)
        .output()
        .map_err(|e| format!("Could not run tar to read {}: {e}", archive.display()))?;
    if !output.status.success() {
        return Err(format!(
            "tar could not read {member} from {}: {}",
            archive.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    String::from_utf8(output.stdout).map_err(|_| format!("{member} in {} is not UTF-8 text.", archive.display()))
}

fn dev(mut app_args: Vec<String>) -> Result<(), String> {
    let smooth = app_args.first().is_some_and(|arg| arg == "--smooth");
    if smooth {
        app_args.remove(0);
    }
    cargo_build(false, &[if smooth { "wackcode/automation" } else { "wackcode/screenshots" }])?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_cefs_licence_from_the_archive_beside_the_distribution() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("cef_binary_156.0.30_macosarm64_minimal");
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join("LICENSE.txt"), "CEF's BSD licence.\n").unwrap();
        let archive = dir.path().join("cef_binary_156.0.30_macosarm64_minimal.tar.bz2");
        let status = Command::new("tar")
            .arg("-cjf")
            .arg(&archive)
            .arg(folder.file_name().unwrap())
            .current_dir(dir.path())
            .status()
            .unwrap();
        assert!(status.success());
        let cef = dir.path().join("cef_macos_aarch64");
        fs::create_dir_all(&cef).unwrap();
        assert_eq!(cef_licence_text(&cef).unwrap(), "CEF's BSD licence.\n");
    }
}
