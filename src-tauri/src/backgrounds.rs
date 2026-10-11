//! Background images (Settings › Appearance › Backdrop). The user picks files in a native
//! dialog opened from Rust, so the renderer never names a path. Validated copies are stored as
//! `<app data>/backgrounds/<uuid>.<ext>`, the only folder the asset protocol may serve
//! (`tauri.conf.json`). The user's originals are never touched. Several images can be saved at
//! once; which of them is shown is Rust's decision (`choose`, `select`, shuffle), never the
//! renderer's.
use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::models::BackgroundImage;

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

/// Deletes every file in `folder` except the named library members: images the user removed
/// and abandoned temporaries.
pub fn prune(folder: &Path, keep: &[String]) {
    let Ok(entries) = fs::read_dir(folder) else { return };
    for entry in entries.flatten() {
        if keep.iter().any(|name| entry.file_name() == name.as_str()) {
            continue;
        }
        if entry.file_type().is_ok_and(|kind| kind.is_file()) {
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// Picks the image shuffle shows next: any saved image except the one currently shown, so an
/// open always brings a change. `None` when there is nothing to switch to.
pub fn pick_next(images: &[BackgroundImage], current: Option<&str>) -> Option<String> {
    let candidates: Vec<&str> = images
        .iter()
        .map(|image| image.name.as_str())
        .filter(|name| Some(*name) != current)
        .collect();
    // UUID v4 carries 122 random bits; a fresh one is enough entropy for choosing a picture,
    // without pulling a random-number crate in for one call.
    if candidates.is_empty() {
        return None;
    }
    candidates
        .get((uuid::Uuid::new_v4().as_u128() % candidates.len() as u128) as usize)
        .map(|name| name.to_string())
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
    fn prune_keeps_every_library_image_and_nothing_else() {
        let directory = tempfile::tempdir().unwrap();
        let folder = directory.path().join("backgrounds");
        let source = directory.path().join("a.png");
        fs::write(&source, PNG).unwrap();
        let first = import(&source, &folder).unwrap();
        let second = import(&source, &folder).unwrap();
        let dropped = import(&source, &folder).unwrap();
        fs::write(folder.join(".stale.tmp"), b"x").unwrap();
        prune(&folder, &[first.clone(), second.clone()]);
        assert!(folder.join(&first).exists());
        assert!(folder.join(&second).exists());
        assert!(!folder.join(&dropped).exists());
        assert!(!folder.join(".stale.tmp").exists());
        prune(&folder, &[]);
        assert_eq!(fs::read_dir(&folder).unwrap().count(), 0);
    }

    #[test]
    fn pick_next_never_repeats_the_current_image() {
        let images = vec![
            BackgroundImage::fresh("a.png".into()),
            BackgroundImage::fresh("b.png".into()),
            BackgroundImage::fresh("c.png".into()),
        ];
        // Every pick lands on a saved image that is not the current one.
        for _ in 0..64 {
            let picked = pick_next(&images, Some("b.png")).unwrap();
            assert!(picked == "a.png" || picked == "c.png");
        }
        // With no current image any saved one may show; with one image there is nothing to
        // switch to, and an empty library has nothing at all.
        assert!(pick_next(&images, None).is_some());
        assert_eq!(pick_next(&images[..1], Some("a.png")), None);
        assert_eq!(pick_next(&[], None), None);
    }

    #[test]
    fn pick_next_is_not_constant() {
        let images: Vec<_> = (0..4).map(|index| BackgroundImage::fresh(format!("{index}.png"))).collect();
        let picks: std::collections::HashSet<String> =
            (0..64).map(|_| pick_next(&images, None).unwrap()).collect();
        assert!(picks.len() > 1, "shuffle never varied across 64 picks");
    }
}
