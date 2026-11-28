//! The layout of a WackCode `.app` bundle, as plain data, and the code that writes it.
//!
//! CEF on macOS needs the app to be a real bundle:
//!
//! ```text
//! WackCode.app/Contents/
//!   Info.plist
//!   MacOS/WackCode                                   the app (`wackcode`)
//!   Resources/AppIcon.icns
//!   Resources/LICENSE.models.dev                     licence notices: the app's own data, and
//!   Resources/LICENSE.cef.txt                        with the browser CEF's BSD licence and
//!   Resources/CREDITS.chromium.html                  Chromium's credits
//!   Frameworks/Chromium Embedded Framework.framework
//!   Frameworks/WackCode Helper.app                   five helper bundles, one binary
//!   Frameworks/WackCode Helper (GPU).app             (`wackcode-helper`)
//!   …
//! ```
//!
//! Invariants:
//! - The main executable is named `WackCode` in every flavour: CEF finds its helpers as
//!   `<executable> Helper (<Kind>).app` beside the framework, so the two names must agree.
//! - The framework is copied only when the CEF distribution it came from changes (a stamp file
//!   beside the bundle records which), so `cargo dev` does not copy 300 MB on every run. The stamp
//!   stays outside the bundle: codesign treats every file in `Contents/Frameworks` as code. Symlinks inside it are
//!   kept as symlinks, as a framework's `Versions/Current` requires.
//! - The executables are copied on every run, so the bundle always holds the binaries just built.
//! - Licence notices ship in `Resources` (`Notice`): models.dev's data, and with the browser
//!   CEF's BSD licence and Chromium's credits, which distributing the framework requires. A
//!   release bundle is not written without the browser's two; a dev bundle warns and ships
//!   what it has. The licence text comes from `main.rs`, out of the archive the distribution
//!   arrived in (the extraction keeps the credits but deletes the licence).

use std::{
    fs, io,
    os::unix::fs::{PermissionsExt, symlink},
    path::{Path, PathBuf},
};

pub const EXECUTABLE: &str = "WackCode";
pub const FRAMEWORK: &str = "Chromium Embedded Framework.framework";

/// `…/WackCode Dev.app` → `…/WackCode Dev.cef-source`.
pub fn stamp_path(app_path: &Path) -> PathBuf {
    app_path.with_extension("cef-source")
}

/// The helper bundles CEF expects, by the suffix after "WackCode Helper".
pub const HELPERS: &[(&str, &str)] = &[
    ("", "helper"),
    (" (GPU)", "helper.gpu"),
    (" (Renderer)", "helper.renderer"),
    (" (Plugin)", "helper.plugin"),
    (" (Alerts)", "helper.alerts"),
];

/// Which app a bundle is: the dev app keeps its own name, identifier and icon, so it can never
/// be mistaken for the installed app.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flavour {
    Dev,
    Release,
}

impl Flavour {
    pub fn bundle_name(self) -> &'static str {
        match self {
            Flavour::Dev => "WackCode Dev",
            Flavour::Release => "WackCode",
        }
    }

    pub fn identifier(self) -> &'static str {
        match self {
            Flavour::Dev => "com.wackcode.desktop.dev",
            Flavour::Release => "com.wackcode.desktop",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Flavour::Dev => "icon-dev.icns",
            Flavour::Release => "icon.icns",
        }
    }
}

/// Everything the bundle is made from.
pub struct Sources {
    pub app: PathBuf,
    pub helper: PathBuf,
    pub icon: PathBuf,
    /// Licence notices, written into `Resources` under their names.
    pub notices: Vec<Notice>,
    /// The CEF distribution's folder (the one holding the framework).
    pub cef: PathBuf,
    /// `CFBundleShortVersionString`: numbers only, as macOS requires.
    pub version: String,
}

/// A licence notice the bundle ships in `Resources`. The two kinds exist because CEF's licence
/// arrives as text, read out of the archive the distribution came in (`main.rs`), while the
/// other notices are files.
pub enum Notice {
    /// A file on disk, copied in under `name`.
    File { from: PathBuf, name: &'static str },
    /// Text held in memory, written in under `name`.
    Text { name: &'static str, contents: String },
}

/// `3.0.0-dev.0` → `3.0.0`: Info.plist versions are dot-separated integers.
pub fn plist_version(cargo_version: &str) -> String {
    let core = cargo_version.split(['-', '+']).next().unwrap_or(cargo_version);
    let parts: Vec<&str> = core.split('.').filter(|p| p.chars().all(|c| c.is_ascii_digit()) && !p.is_empty()).collect();
    if parts.is_empty() { "0.0.0".into() } else { parts.join(".") }
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// An Info.plist with string and boolean keys, in the order given.
pub fn info_plist(entries: &[(&str, PlistValue)]) -> String {
    let mut out = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n<dict>\n",
    );
    for (key, value) in entries {
        out.push_str(&format!("\t<key>{}</key>\n", escape(key)));
        match value {
            PlistValue::Text(text) => out.push_str(&format!("\t<string>{}</string>\n", escape(text))),
            PlistValue::Bool(true) => out.push_str("\t<true/>\n"),
            PlistValue::Bool(false) => out.push_str("\t<false/>\n"),
        }
    }
    out.push_str("</dict>\n</plist>\n");
    out
}

pub enum PlistValue {
    Text(String),
    Bool(bool),
}

fn text(value: impl Into<String>) -> PlistValue {
    PlistValue::Text(value.into())
}

pub fn app_plist(flavour: Flavour, version: &str) -> String {
    info_plist(&[
        ("CFBundleDevelopmentRegion", text("en")),
        ("CFBundleExecutable", text(EXECUTABLE)),
        ("CFBundleIdentifier", text(flavour.identifier())),
        ("CFBundleName", text(flavour.bundle_name())),
        ("CFBundleDisplayName", text(flavour.bundle_name())),
        ("CFBundleIconFile", text("AppIcon")),
        ("CFBundleInfoDictionaryVersion", text("6.0")),
        ("CFBundlePackageType", text("APPL")),
        ("CFBundleShortVersionString", text(version)),
        ("CFBundleVersion", text(version)),
        ("LSApplicationCategoryType", text("public.app-category.developer-tools")),
        ("LSMinimumSystemVersion", text("26.0")),
        ("NSHighResolutionCapable", PlistValue::Bool(true)),
        ("NSSupportsAutomaticGraphicsSwitching", PlistValue::Bool(true)),
    ])
}

pub fn helper_plist(flavour: Flavour, suffix: &str, id_suffix: &str, version: &str) -> String {
    let name = format!("{EXECUTABLE} Helper{suffix}");
    info_plist(&[
        ("CFBundleDevelopmentRegion", text("en")),
        ("CFBundleExecutable", text(name.clone())),
        ("CFBundleIdentifier", text(format!("{}.{id_suffix}", flavour.identifier()))),
        ("CFBundleName", text(name.clone())),
        ("CFBundleDisplayName", text(name)),
        ("CFBundleInfoDictionaryVersion", text("6.0")),
        ("CFBundlePackageType", text("APPL")),
        ("CFBundleShortVersionString", text(version)),
        ("CFBundleVersion", text(version)),
        ("LSMinimumSystemVersion", text("26.0")),
        ("LSUIElement", PlistValue::Bool(true)),
        ("NSSupportsAutomaticGraphicsSwitching", PlistValue::Bool(true)),
    ])
}

fn io_error(what: &str, path: &Path, error: io::Error) -> String {
    format!("Could not {what} {}: {error}", path.display())
}

fn write(path: &Path, contents: &str) -> Result<(), String> {
    fs::write(path, contents).map_err(|e| io_error("write", path, e))
}

fn create_dir(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path).map_err(|e| io_error("create", path, e))
}

fn copy_executable(from: &Path, to: &Path) -> Result<(), String> {
    // Remove first: overwriting a running or signed binary in place can leave a stale signature
    // cached by the kernel.
    let _ = fs::remove_file(to);
    fs::copy(from, to).map_err(|e| io_error("copy", from, e))?;
    fs::set_permissions(to, fs::Permissions::from_mode(0o755)).map_err(|e| io_error("set permissions on", to, e))
}

/// Copies a directory tree, keeping symlinks as symlinks.
pub fn copy_tree(from: &Path, to: &Path) -> Result<(), String> {
    create_dir(to)?;
    let entries = fs::read_dir(from).map_err(|e| io_error("read", from, e))?;
    for entry in entries {
        let entry = entry.map_err(|e| io_error("read", from, e))?;
        let source = entry.path();
        let target = to.join(entry.file_name());
        let kind = entry.file_type().map_err(|e| io_error("inspect", &source, e))?;
        if kind.is_symlink() {
            let link = fs::read_link(&source).map_err(|e| io_error("read the link", &source, e))?;
            let _ = fs::remove_file(&target);
            symlink(&link, &target).map_err(|e| io_error("link", &target, e))?;
        } else if kind.is_dir() {
            copy_tree(&source, &target)?;
        } else {
            fs::copy(&source, &target).map_err(|e| io_error("copy", &source, e))?;
        }
    }
    Ok(())
}

/// Writes (or refreshes) the bundle at `app_path` and returns the main executable's path.
pub fn assemble(app_path: &Path, flavour: Flavour, sources: &Sources) -> Result<PathBuf, String> {
    let contents = app_path.join("Contents");
    let macos = contents.join("MacOS");
    let resources = contents.join("Resources");
    let frameworks = contents.join("Frameworks");
    for dir in [&macos, &resources, &frameworks] {
        create_dir(dir)?;
    }

    write(&contents.join("Info.plist"), &app_plist(flavour, &sources.version))?;
    fs::copy(&sources.icon, resources.join("AppIcon.icns")).map_err(|e| io_error("copy", &sources.icon, e))?;
    for notice in &sources.notices {
        match notice {
            Notice::File { from, name } => {
                fs::copy(from, resources.join(name)).map_err(|e| io_error("copy", from, e))?;
            }
            Notice::Text { name, contents } => {
                write(&resources.join(name), contents)?;
            }
        }
    }
    let executable = macos.join(EXECUTABLE);
    copy_executable(&sources.app, &executable)?;

    // The framework: copied only when its source changed.
    let stamp = stamp_path(app_path);
    let _ = fs::remove_file(frameworks.join(".cef-source")); // where earlier builds kept it
    let source_id = sources.cef.display().to_string();
    let framework = frameworks.join(FRAMEWORK);
    if fs::read_to_string(&stamp).ok().as_deref() != Some(source_id.as_str()) || !framework.exists() {
        if framework.exists() {
            fs::remove_dir_all(&framework).map_err(|e| io_error("remove", &framework, e))?;
        }
        copy_tree(&sources.cef.join(FRAMEWORK), &framework)?;
        write(&stamp, &source_id)?;
    }

    for (suffix, id_suffix) in HELPERS {
        let name = format!("{EXECUTABLE} Helper{suffix}");
        let helper = frameworks.join(format!("{name}.app")).join("Contents");
        create_dir(&helper.join("MacOS"))?;
        write(&helper.join("Info.plist"), &helper_plist(flavour, suffix, id_suffix, &sources.version))?;
        copy_executable(&sources.helper, &helper.join("MacOS").join(&name))?;
    }

    Ok(executable)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plist_versions_are_numbers_only() {
        assert_eq!(plist_version("3.0.0-dev.0"), "3.0.0");
        assert_eq!(plist_version("3.1.2"), "3.1.2");
        assert_eq!(plist_version("3.0.0+build.5"), "3.0.0");
        assert_eq!(plist_version("weird"), "0.0.0");
    }

    #[test]
    fn plists_escape_and_carry_the_identity() {
        let plist = app_plist(Flavour::Dev, "3.0.0");
        assert!(plist.contains("<string>com.wackcode.desktop.dev</string>"));
        assert!(plist.contains("<string>WackCode Dev</string>"));
        assert!(plist.contains("<key>CFBundleExecutable</key>\n\t<string>WackCode</string>"));
        let helper = helper_plist(Flavour::Release, " (GPU)", "helper.gpu", "3.0.0");
        assert!(helper.contains("<string>WackCode Helper (GPU)</string>"));
        assert!(helper.contains("<string>com.wackcode.desktop.helper.gpu</string>"));
        assert!(helper.contains("<key>LSUIElement</key>\n\t<true/>"));
        assert!(info_plist(&[("k", PlistValue::Text("a<b&c".into()))]).contains("a&lt;b&amp;c"));
    }

    #[test]
    fn assembles_a_bundle_and_copies_the_framework_once() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let cef = root.join("cef");
        let framework = cef.join(FRAMEWORK);
        fs::create_dir_all(framework.join("Versions/A/Resources")).unwrap();
        fs::write(framework.join("Versions/A/Resources/marker"), "one").unwrap();
        symlink("A", framework.join("Versions/Current")).unwrap();
        for (name, body) in [
            ("app", "app"),
            ("helper", "helper"),
            ("icon.icns", "icon"),
            ("LICENSE.data", "MIT"),
            ("credits.html", "credits"),
        ] {
            fs::write(root.join(name), body).unwrap();
        }
        let sources = Sources {
            app: root.join("app"),
            helper: root.join("helper"),
            icon: root.join("icon.icns"),
            notices: vec![
                Notice::File { from: root.join("LICENSE.data"), name: "LICENSE.data" },
                Notice::File { from: root.join("credits.html"), name: "CREDITS.chromium.html" },
                Notice::Text { name: "LICENSE.cef.txt", contents: "BSD-3-Clause".into() },
            ],
            cef: cef.clone(),
            version: "3.0.0".into(),
        };
        let app = root.join("WackCode Dev.app");

        let executable = assemble(&app, Flavour::Dev, &sources).unwrap();
        assert_eq!(executable, app.join("Contents/MacOS/WackCode"));
        assert!(stamp_path(&app).ends_with("WackCode Dev.cef-source") && stamp_path(&app).exists());
        assert_eq!(fs::read_dir(app.join("Contents/Frameworks")).unwrap().count(), 1 + HELPERS.len());
        assert_eq!(fs::read_to_string(&executable).unwrap(), "app");
        assert_eq!(fs::read_to_string(app.join("Contents/Resources/LICENSE.data")).unwrap(), "MIT");
        assert_eq!(fs::read_to_string(app.join("Contents/Resources/CREDITS.chromium.html")).unwrap(), "credits");
        assert_eq!(fs::read_to_string(app.join("Contents/Resources/LICENSE.cef.txt")).unwrap(), "BSD-3-Clause");
        let copied = app.join("Contents/Frameworks").join(FRAMEWORK);
        assert!(fs::symlink_metadata(copied.join("Versions/Current")).unwrap().file_type().is_symlink());
        for (suffix, _) in HELPERS {
            let name = format!("WackCode Helper{suffix}");
            let binary = app.join("Contents/Frameworks").join(format!("{name}.app/Contents/MacOS/{name}"));
            assert_eq!(fs::read_to_string(binary).unwrap(), "helper");
        }

        // A second run with the same CEF leaves the framework alone but refreshes the binaries.
        fs::write(copied.join("Versions/A/Resources/marker"), "untouched").unwrap();
        fs::write(root.join("app"), "app v2").unwrap();
        assemble(&app, Flavour::Dev, &sources).unwrap();
        assert_eq!(fs::read_to_string(copied.join("Versions/A/Resources/marker")).unwrap(), "untouched");
        assert_eq!(fs::read_to_string(&executable).unwrap(), "app v2");

        // A different CEF replaces it.
        let other = Sources { cef: root.join("cef2"), ..sources };
        fs::create_dir_all(other.cef.join(FRAMEWORK)).unwrap();
        fs::rename(&cef, root.join("unused")).unwrap();
        assemble(&app, Flavour::Dev, &other).unwrap();
        assert!(!copied.join("Versions").exists());
    }
}
