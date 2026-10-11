//! Running apps, resolving what the model called an app, launching one in the background, and
//! its icon for the access card. Called from the engine thread; none of these AppKit calls are
//! main-thread-only.

use super::policy::AppIdentity;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use block2::RcBlock;
use objc2::{rc::Retained, AnyThread};
use objc2_app_kit::{
    NSApplicationActivationPolicy, NSBitmapImageFileType, NSBitmapImageRep, NSRunningApplication, NSWorkspace,
    NSWorkspaceOpenConfiguration,
};
use objc2_foundation::{NSBundle, NSDictionary, NSError, NSPoint, NSRect, NSSize, NSString, NSURL};
use std::path::{Path, PathBuf};
use std::time::Duration;

const LAUNCH_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_ICON_BYTES: usize = 48 * 1024;
const APPLICATION_FOLDERS: &[&str] = &["/Applications", "/Applications/Utilities", "/System/Applications", "/System/Applications/Utilities"];

#[derive(Debug, Clone)]
pub struct RunningApp {
    pub pid: i32,
    pub name: String,
    pub bundle_id: Option<String>,
    pub bundle_path: Option<PathBuf>,
    pub executable: Option<PathBuf>,
    pub frontmost: bool,
}

impl RunningApp {
    pub fn identity(&self) -> AppIdentity {
        let path = self.bundle_path.clone().or_else(|| self.executable.clone()).map(|path| path.canonicalize().unwrap_or(path));
        AppIdentity { pid: Some(self.pid), name: self.name.clone(), bundle_id: self.bundle_id.clone(), path }
    }
}

fn url_path(url: Option<Retained<NSURL>>) -> Option<PathBuf> {
    url.and_then(|url| url.path()).map(|path| PathBuf::from(path.to_string()))
}

fn describe(app: &NSRunningApplication) -> RunningApp {
    let bundle_path = url_path(app.bundleURL());
    let name = app
        .localizedName()
        .map(|name| name.to_string())
        .or_else(|| bundle_path.as_deref().and_then(Path::file_stem).map(|stem| stem.to_string_lossy().into_owned()))
        .unwrap_or_else(|| format!("pid {}", app.processIdentifier()));
    RunningApp {
        pid: app.processIdentifier(),
        name,
        bundle_id: app.bundleIdentifier().map(|id| id.to_string()),
        bundle_path,
        executable: url_path(app.executableURL()),
        frontmost: app.isActive(),
    }
}

/// Running apps that show in the Dock (regular activation policy).
pub fn running() -> Vec<RunningApp> {
    let workspace = NSWorkspace::sharedWorkspace();
    workspace
        .runningApplications()
        .iter()
        .filter(|app| app.activationPolicy() == NSApplicationActivationPolicy::Regular && !app.isTerminated())
        .map(|app| describe(&app))
        .collect()
}

/// Any running process's app name (for naming whoever covers a click point).
pub fn name_of(pid: i32) -> (String, Option<String>) {
    match NSRunningApplication::runningApplicationWithProcessIdentifier(pid) {
        Some(app) => (
            app.localizedName().map(|name| name.to_string()).unwrap_or_else(|| format!("pid {pid}")),
            app.bundleIdentifier().map(|id| id.to_string()),
        ),
        None => (format!("pid {pid}"), None),
    }
}

pub fn frontmost_pid() -> Option<i32> {
    NSWorkspace::sharedWorkspace().frontmostApplication().map(|app| app.processIdentifier())
}

/// An app that isn't running yet, found on disk.
#[derive(Debug, Clone)]
pub struct InstalledApp {
    pub path: PathBuf,
    pub name: String,
    pub bundle_id: Option<String>,
}

impl InstalledApp {
    pub fn identity(&self) -> AppIdentity {
        AppIdentity { pid: None, name: self.name.clone(), bundle_id: self.bundle_id.clone(), path: Some(self.path.clone()) }
    }
}

pub enum Resolved {
    Running(RunningApp),
    Installed(InstalledApp),
}

/// Reads a `.app` bundle's id and display name.
pub fn bundle_info(path: &Path) -> Option<InstalledApp> {
    let bundle = NSBundle::bundleWithPath(&NSString::from_str(&path.to_string_lossy()))?;
    let string_key = |key: &str| {
        bundle
            .objectForInfoDictionaryKey(&NSString::from_str(key))
            .and_then(|value| value.downcast::<NSString>().ok())
            .map(|value| value.to_string())
            .filter(|value| !value.trim().is_empty())
    };
    let name = string_key("CFBundleDisplayName")
        .or_else(|| string_key("CFBundleName"))
        .or_else(|| path.file_stem().map(|stem| stem.to_string_lossy().into_owned()))?;
    Some(InstalledApp { path: path.to_path_buf(), name, bundle_id: bundle.bundleIdentifier().map(|id| id.to_string()) })
}

fn is_bundle_dir(path: &Path) -> bool {
    path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("app")) && path.join("Contents/Info.plist").is_file()
}

/// Resolves the model's `app`: an absolute `.app` path, then a running app by bundle id or
/// name, then an installed app by bundle id or name.
pub fn resolve(query: &str, home: Option<&Path>) -> Result<Resolved, String> {
    let query = query.trim();
    let running = running();
    if query.starts_with('/') {
        let path = Path::new(query).canonicalize().map_err(|_| format!("There is no app at {query}."))?;
        if !is_bundle_dir(&path) {
            return Err(format!("{} isn't a .app bundle.", path.display()));
        }
        if let Some(app) = running.into_iter().find(|app| app.bundle_path.as_deref().and_then(|bundle| bundle.canonicalize().ok()).as_deref() == Some(path.as_path())) {
            return Ok(Resolved::Running(app));
        }
        return bundle_info(&path).map(Resolved::Installed).ok_or_else(|| format!("{} can't be read as an app bundle.", path.display()));
    }
    let lower = query.to_lowercase();
    let pick = |matches: Vec<RunningApp>| matches.iter().find(|app| app.frontmost).cloned().or_else(|| matches.into_iter().next());
    let by_id = running.iter().filter(|app| app.bundle_id.as_deref().is_some_and(|id| id.eq_ignore_ascii_case(query))).cloned().collect::<Vec<_>>();
    if let Some(app) = pick(by_id) {
        return Ok(Resolved::Running(app));
    }
    let by_name = running.iter().filter(|app| app.name.to_lowercase() == lower).cloned().collect::<Vec<_>>();
    if let Some(app) = pick(by_name) {
        return Ok(Resolved::Running(app));
    }
    if query.contains('.') {
        if let Some(url) = NSWorkspace::sharedWorkspace().URLForApplicationWithBundleIdentifier(&NSString::from_str(query)) {
            if let Some(app) = url_path(Some(url)).and_then(|path| bundle_info(&path)) {
                return Ok(Resolved::Installed(app));
            }
        }
    }
    let file_name = if lower.ends_with(".app") { query.to_string() } else { format!("{query}.app") };
    let mut folders = APPLICATION_FOLDERS.iter().map(PathBuf::from).collect::<Vec<_>>();
    if let Some(home) = home {
        folders.insert(1, home.join("Applications"));
    }
    for folder in folders {
        let Ok(entries) = std::fs::read_dir(&folder) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            let matches = path.file_name().is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case(&file_name));
            if matches && is_bundle_dir(&path) {
                if let Some(app) = bundle_info(&path.canonicalize().unwrap_or(path)) {
                    return Ok(Resolved::Installed(app));
                }
            }
        }
    }
    Err(format!("No app called “{query}” is running or installed. Use computer_apps to see running apps, or pass the absolute path of a .app."))
}

/// Launches the app at `path` without activating it and returns its pid.
pub fn launch(path: &Path) -> Result<i32, String> {
    let workspace = NSWorkspace::sharedWorkspace();
    let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
    let configuration = NSWorkspaceOpenConfiguration::configuration();
    configuration.setActivates(false);
    configuration.setAddsToRecentItems(false);
    configuration.setPromptsUserIfNeeded(false);
    let (sender, receiver) = std::sync::mpsc::channel::<Result<i32, String>>();
    let handler = RcBlock::new(move |app: *mut NSRunningApplication, error: *mut NSError| {
        let result = if app.is_null() {
            let reason = if error.is_null() { "macOS didn't say why".to_string() } else { unsafe { (*error).localizedDescription() }.to_string() };
            Err(format!("The app couldn't be launched: {reason}"))
        } else {
            Ok(unsafe { (*app).processIdentifier() })
        };
        let _ = sender.send(result);
    });
    workspace.openApplicationAtURL_configuration_completionHandler(&url, &configuration, Some(&handler));
    receiver.recv_timeout(LAUNCH_TIMEOUT).map_err(|_| "The app took too long to launch.".to_string())?
}

/// The app's icon as a small PNG data URL for the access card, if one can be made cheaply.
pub fn icon_data_url(path: &Path) -> Option<String> {
    let image = NSWorkspace::sharedWorkspace().iconForFile(&NSString::from_str(&path.to_string_lossy()));
    let mut rect = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(64.0, 64.0));
    let cg_image = unsafe { image.CGImageForProposedRect_context_hints(&mut rect, None, None) }?;
    let bitmap = NSBitmapImageRep::initWithCGImage(NSBitmapImageRep::alloc(), &cg_image);
    let data = unsafe { bitmap.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::dictionary()) }?;
    let bytes = data.to_vec();
    (bytes.len() <= MAX_ICON_BYTES).then(|| format!("data:image/png;base64,{}", BASE64.encode(bytes)))
}
