//! The browser engine's helper processes: Chromium's renderer, GPU and utility processes.
//!
//! CEF on macOS runs each kind from its own helper bundle inside the app
//! (`Contents/Frameworks/WackCode Helper (Renderer).app` and so on); `cargo xtask` copies this
//! binary into all of them. It never starts on its own: the browser process launches it with
//! `--type=…`.
//!
//! Invariants:
//! - The sandbox is entered before anything else runs, as CEF requires.
//! - The framework is loaded from the bundle (`../../..` from this executable), never from a
//!   path in the environment.

use cef::{args::Args, library_loader::LibraryLoader};

fn main() {
    let args = Args::new();

    let mut sandbox = cef::sandbox::Sandbox::new();
    sandbox.initialize(args.as_main_args());

    let Ok(exe) = std::env::current_exe() else {
        eprintln!("WackCode Helper: cannot find its own executable");
        std::process::exit(1);
    };
    let loader = LibraryLoader::new(&exe, true);
    if !loader.load() {
        eprintln!("WackCode Helper: cannot load the Chromium Embedded Framework");
        std::process::exit(1);
    }

    let _ = cef::api_hash(cef::sys::CEF_API_VERSION_LAST, 0);
    let code = cef::execute_process(Some(args.as_main_args()), None::<&mut cef::App>, std::ptr::null_mut());
    drop(loader);
    std::process::exit(code.max(0));
}
