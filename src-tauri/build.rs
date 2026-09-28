fn main() {
    // ScreenCaptureKit first shipped in macOS 12.3, but the app supports 12.0. Weak-link it so
    // launch never fails there; computer use checks for `SCScreenshotManager` (macOS 14) at
    // runtime before touching it (`computer_use::permissions::is_supported`).
    println!("cargo:rustc-link-arg=-Wl,-weak_framework,ScreenCaptureKit");
    tauri_build::build()
}
