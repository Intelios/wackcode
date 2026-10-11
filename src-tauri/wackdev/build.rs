fn main() {
    // ScreenCaptureKit first shipped in macOS 12.3; weak-link it so binaries that may run on an
    // older system launch anyway, and check `SCScreenshotManager` (macOS 14) at runtime before
    // capturing (`permissions::is_supported`).
    println!("cargo:rustc-link-arg=-Wl,-weak_framework,ScreenCaptureKit");
}
