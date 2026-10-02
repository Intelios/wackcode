fn main() {
    // ScreenCaptureKit first shipped in macOS 12.3, but the app supports 12.0. Weak-link it so
    // launch never fails there; computer use checks for `SCScreenshotManager` (macOS 14) at
    // runtime before touching it (`computer_use::permissions::is_supported`).
    println!("cargo:rustc-link-arg=-Wl,-weak_framework,ScreenCaptureKit");
    bake_runtime_versions();
    tauri_build::build()
}

/// Bake the bundled runtime's versions into the binary for Settings › About. `runtime-lock.json`
/// is the contract `prepare:runtime` enforces (Node is checksum-verified against it), and a vitest
/// guard (`src/version.test.ts`) keeps its Pi pin in step with `worker/package.json`, so these
/// numbers can't drift from what ships. Baking avoids dev/prod resource paths and subprocesses.
fn bake_runtime_versions() {
    let lock_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../runtime-lock.json");
    println!("cargo:rerun-if-changed={}", lock_path.display());
    let text = std::fs::read_to_string(&lock_path)
        .unwrap_or_else(|error| panic!("Could not read {}: {error}", lock_path.display()));
    let lock: serde_json::Value = serde_json::from_str(&text)
        .unwrap_or_else(|error| panic!("Could not parse {}: {error}", lock_path.display()));
    for (key, field) in [("WACKCODE_PI_VERSION", "pi"), ("WACKCODE_NODE_VERSION", "node")] {
        let version = lock[field]["version"]
            .as_str()
            .unwrap_or_else(|| panic!("{field}.version is missing from {}", lock_path.display()))
            .to_string();
        println!("cargo:rustc-env={key}={version}");
    }
}
