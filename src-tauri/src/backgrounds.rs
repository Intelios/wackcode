//! Background images (Settings › Appearance › Backdrop). The user picks a file in a native
//! dialog opened from Rust, so the renderer never names a path. A validated copy is stored as
//! `<app data>/backgrounds/<uuid>.<ext>`, the only folder the asset protocol may serve
//! (`tauri.conf.json`). The user's original is never touched.
use std::{
    fs,
    path::{Path, PathBuf},
};

/// Anything bigger is almost certainly not meant as a window background.
pub const MAX_IMAGE_BYTES: u64 = 25 * 1024 * 1024;

pub fn directory(app_data: &Path) -> PathBuf {
    app_data.join("backgrounds")
}

/// The image kind from the file's first bytes; the extension is not trusted.
fn sniff(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]) {
        Some("png")
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("jpg")
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("webp")
    } else {
        None
    }
}

/// Copies `source` into `folder` under a fresh name and returns that name. Nothing else in
/// `folder` changes; call `prune` once the new name is saved.
pub fn import(source: &Path, folder: &Path) -> Result<String, String> {
    let size = fs::metadata(source).map_err(|_| "That image could not be read.".to_string())?.len();
    if size > MAX_IMAGE_BYTES {
        return Err(format!("Background images can be at most {} MB.", MAX_IMAGE_BYTES / 1024 / 1024));
    }
    let bytes = fs::read(source).map_err(|_| "That image could not be read.".to_string())?;
    let extension = sniff(&bytes).ok_or_else(|| "Choose a PNG, JPEG, or WebP image.".to_string())?;
    fs::create_dir_all(folder).map_err(|error| format!("Could not create the backgrounds folder: {error}"))?;
    let name = format!("{}.{extension}", uuid::Uuid::new_v4());
    let temporary = folder.join(format!(".{name}.tmp"));
    fs::write(&temporary, &bytes).map_err(|error| format!("Could not save the image: {error}"))?;
    fs::rename(&temporary, folder.join(&name)).map_err(|error| format!("Could not save the image: {error}"))?;
    Ok(name)
}

/// Deletes every file in `folder` except `keep`: replaced images and abandoned temporaries.
pub fn prune(folder: &Path, keep: Option<&str>) {
    let Ok(entries) = fs::read_dir(folder) else { return };
    for entry in entries.flatten() {
        if keep.is_some_and(|keep| entry.file_name() == keep) {
            continue;
        }
        if entry.file_type().is_ok_and(|kind| kind.is_file()) {
            let _ = fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG: &[u8] = &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13];

    #[test]
    fn imports_by_content_not_extension() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("holiday.txt");
        fs::write(&source, PNG).unwrap();
        let folder = directory.path().join("backgrounds");
        let name = import(&source, &folder).unwrap();
        assert!(name.ends_with(".png"), "{name}");
        assert_eq!(fs::read(folder.join(&name)).unwrap(), PNG);
        assert!(source.exists(), "the original is never touched");

        let text = directory.path().join("notes.png");
        fs::write(&text, b"not an image at all").unwrap();
        assert!(import(&text, &folder).unwrap_err().contains("PNG, JPEG, or WebP"));
    }

    #[test]
    fn recognises_jpeg_and_webp() {
        assert_eq!(sniff(&[0xff, 0xd8, 0xff, 0xe0]), Some("jpg"));
        assert_eq!(sniff(b"RIFF\x10\x00\x00\x00WEBPVP8 "), Some("webp"));
        assert_eq!(sniff(b"RIFF\x10\x00\x00\x00WAVEfmt "), None);
        assert_eq!(sniff(b"GIF89a"), None);
    }

    #[test]
    fn refuses_huge_files() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("huge.png");
        let file = fs::File::create(&source).unwrap();
        file.set_len(MAX_IMAGE_BYTES + 1).unwrap();
        assert!(import(&source, &directory.path().join("backgrounds")).unwrap_err().contains("25 MB"));
    }

    #[test]
    fn prune_keeps_only_the_current_image() {
        let directory = tempfile::tempdir().unwrap();
        let folder = directory.path().join("backgrounds");
        let source = directory.path().join("a.png");
        fs::write(&source, PNG).unwrap();
        let old = import(&source, &folder).unwrap();
        let new = import(&source, &folder).unwrap();
        fs::write(folder.join(".stale.tmp"), b"x").unwrap();
        prune(&folder, Some(&new));
        assert!(folder.join(&new).exists());
        assert!(!folder.join(&old).exists());
        assert!(!folder.join(".stale.tmp").exists());
        prune(&folder, None);
        assert_eq!(fs::read_dir(&folder).unwrap().count(), 0);
    }
}
