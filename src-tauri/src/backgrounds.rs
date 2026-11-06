//! Background images (Settings › Appearance › Backdrop). The user picks files in a native
//! dialog opened from Rust, so the renderer never names a path. Validated copies are stored as
//! `<app data>/backgrounds/<uuid>.<ext>`, the only folder the asset protocol may serve
//! (`tauri.conf.json`). The user's originals are never touched. Several images can be saved at
//! once; which of them is shown is Rust's decision (`choose`, `select`, shuffle), never the
//! renderer's.
use std::{
    fs,
    io::Read,
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};

use crate::models::{BackgroundImage, BackgroundImageMotion};

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

/// A renderer may inspect only a bare library name. Never follow a symlink or read more than
/// the import limit. Container structure tells us whether a frame can stand for the image;
/// decoding remains the renderer's job, and any uncertainty requires conservative contrast.
pub fn image_motion(folder: &Path, images: &[BackgroundImage], name: &str) -> Result<BackgroundImageMotion, String> {
    if name.is_empty()
        || matches!(name, "." | "..")
        || name.contains(['/', '\\', '\0'])
        || !images.iter().any(|image| image.name == name)
    {
        return Err("That background is not saved.".into());
    }
    let inspect = || -> Option<BackgroundImageMotion> {
        let file = fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(folder.join(name))
            .ok()?;
        let metadata = file.metadata().ok()?;
        if !metadata.is_file() || metadata.len() > MAX_IMAGE_BYTES {
            return None;
        }
        let mut bytes = Vec::new();
        file.take(MAX_IMAGE_BYTES + 1).read_to_end(&mut bytes).ok()?;
        if bytes.len() as u64 > MAX_IMAGE_BYTES {
            return None;
        }
        Some(container_motion(&bytes))
    };
    Ok(inspect().unwrap_or(BackgroundImageMotion::Unknown))
}

fn container_motion(bytes: &[u8]) -> BackgroundImageMotion {
    match sniff(bytes) {
        Some("png") => png_motion(bytes),
        Some("webp") => webp_motion(bytes),
        // JPEG cannot animate. An incomplete container still takes the conservative path.
        Some("jpg") if bytes.ends_with(&[0xff, 0xd9]) => BackgroundImageMotion::Static,
        _ => BackgroundImageMotion::Unknown,
    }
}

fn png_motion(bytes: &[u8]) -> BackgroundImageMotion {
    use BackgroundImageMotion::{Animated, Static, Unknown};
    let mut offset = 8usize;
    let mut header = false;
    let mut data_seen = false;
    let mut animated = false;
    let mut expected_frames = 0;
    let mut frames = 0;
    while let Some(chunk_header) = bytes.get(offset..).and_then(|tail| tail.get(..8)) {
        let length = u32::from_be_bytes(chunk_header[..4].try_into().unwrap()) as usize;
        let Some(end) = offset.checked_add(12).and_then(|start| start.checked_add(length)) else { return Unknown };
        let Some(chunk) = bytes.get(offset + 8..end.saturating_sub(4)) else { return Unknown };
        if end > bytes.len() || (!header && &chunk_header[4..8] != b"IHDR") {
            return Unknown;
        }
        match &chunk_header[4..8] {
            b"IHDR" => {
                if header || offset != 8 || length != 13 || chunk[..4] == [0; 4] || chunk[4..8] == [0; 4] {
                    return Unknown;
                }
                header = true;
            }
            b"acTL" => {
                if animated || data_seen || length != 8 || chunk[..4] == [0; 4] { return Unknown }
                animated = true;
                expected_frames = u32::from_be_bytes(chunk[..4].try_into().unwrap());
            }
            b"fcTL" => {
                if !animated || length != 26 || chunk[4..8] == [0; 4] || chunk[8..12] == [0; 4] { return Unknown }
                frames += 1;
            }
            b"fdAT" if !animated || length < 4 => return Unknown,
            b"IDAT" => data_seen = true,
            b"IEND" => return if length == 0 && data_seen && end == bytes.len() && (!animated || frames == expected_frames) {
                if animated { Animated } else { Static }
            } else { Unknown },
            _ => {}
        }
        offset = end;
    }
    Unknown
}

/// Each RIFF chunk carries its own length and an optional pad byte. Bounds are checked before
/// reading fields, so malformed lengths cannot turn into indexing panics or partial certainty.
fn riff_chunk(bytes: &[u8], offset: usize) -> Option<(&[u8], &[u8], usize)> {
    let header = bytes.get(offset..)?.get(..8)?;
    let length = u32::from_le_bytes(header[4..8].try_into().ok()?) as usize;
    let start = offset.checked_add(8)?;
    let end = start.checked_add(length)?;
    let next = end.checked_add(length % 2)?;
    (next <= bytes.len()).then(|| (&header[..4], &bytes[start..end], next))
}

fn webp_frame_has_image(bytes: &[u8]) -> bool {
    let mut offset = 0;
    let mut image = false;
    while offset < bytes.len() {
        let Some((kind, data, next)) = riff_chunk(bytes, offset) else { return false };
        if matches!(kind, b"VP8 " | b"VP8L") {
            if image || data.is_empty() { return false }
            image = true;
        }
        offset = next;
    }
    image
}

fn webp_motion(bytes: &[u8]) -> BackgroundImageMotion {
    use BackgroundImageMotion::{Animated, Static, Unknown};
    let declared = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
    if declared.checked_add(8) != Some(bytes.len()) { return Unknown }
    let mut offset = 12;
    let mut extended = false;
    let mut animated = false;
    let mut animation_header = false;
    let mut frame = false;
    let mut image = false;
    while offset < bytes.len() {
        let Some((kind, data, next)) = riff_chunk(bytes, offset) else { return Unknown };
        match kind {
            b"VP8X" => {
                if extended || offset != 12 || data.len() != 10 { return Unknown }
                extended = true;
                animated = data[0] & 0x02 != 0;
            }
            b"ANIM" => {
                if !animated || animation_header || frame || data.len() != 6 { return Unknown }
                animation_header = true;
            }
            b"ANMF" => {
                if !animation_header || data.len() < 16 || !webp_frame_has_image(&data[16..]) { return Unknown }
                frame = true;
            }
            b"VP8 " | b"VP8L" => {
                if animated || image || data.is_empty() { return Unknown }
                image = true;
            }
            _ => {}
        }
        offset = next;
    }
    if animated && animation_header && frame { Animated }
    else if !animated && image { Static }
    else { Unknown }
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

    // These fixtures exercise container structure, not compression/CRC decoding. The renderer
    // separately requires image.decode() to succeed before trusting a static classification.
    fn png(chunks: &[(&[u8; 4], &[u8])]) -> Vec<u8> {
        let mut bytes = PNG[..8].to_vec();
        let mut header = [0; 13];
        header[3] = 1;
        header[7] = 1;
        for (kind, data) in std::iter::once((b"IHDR", header.as_slice())).chain(chunks.iter().copied()) {
            bytes.extend_from_slice(&(data.len() as u32).to_be_bytes());
            bytes.extend_from_slice(kind);
            bytes.extend_from_slice(data);
            bytes.extend_from_slice(&[0; 4]);
        }
        bytes
    }

    fn webp(chunks: &[(&[u8; 4], &[u8])]) -> Vec<u8> {
        let mut bytes = b"RIFF\0\0\0\0WEBP".to_vec();
        for (kind, data) in chunks {
            bytes.extend_from_slice(*kind);
            bytes.extend_from_slice(&(data.len() as u32).to_le_bytes());
            bytes.extend_from_slice(data);
            if data.len() % 2 != 0 { bytes.push(0); }
        }
        let length = bytes.len() as u32 - 8;
        bytes[4..8].copy_from_slice(&length.to_le_bytes());
        bytes
    }

    #[test]
    fn motion_distinguishes_static_images_and_animated_containers() {
        use BackgroundImageMotion::{Animated, Static};
        assert_eq!(container_motion(&png(&[(b"IDAT", b"acTL"), (b"IEND", b"")])), Static);
        let mut control = [0; 26];
        control[7] = 1;
        control[11] = 1;
        let apng = png(&[(b"acTL", &[0, 0, 0, 1, 0, 0, 0, 0]), (b"fcTL", &control), (b"IDAT", b"pixels"), (b"IEND", b"")]);
        assert_eq!(container_motion(&apng), Animated);
        assert_eq!(container_motion(&[0xff, 0xd8, 0xff, 0xe0, 0xff, 0xd9]), Static);
        assert_eq!(container_motion(&webp(&[(b"VP8 ", b"pixels")])), Static);
        assert_eq!(container_motion(&webp(&[(b"VP8X", &[0; 10]), (b"VP8L", b"pixels")])), Static);

        let mut frame = vec![0; 16];
        frame.extend_from_slice(&webp(&[(b"VP8 ", b"pixels")])[12..]);
        let mut extended = [0; 10];
        extended[0] = 0x02;
        let animation = webp(&[(b"VP8X", &extended), (b"ANIM", &[0; 6]), (b"ANMF", &frame)]);
        assert_eq!(container_motion(&animation), Animated);
    }

    #[test]
    fn malformed_motion_containers_remain_unknown_without_panicking() {
        use BackgroundImageMotion::Unknown;
        let malformed = [
            Vec::new(),
            PNG.to_vec(),
            vec![0xff, 0xd8, 0xff],
            png(&[(b"IDAT", b"pixels")]),
            png(&[(b"acTL", &[0; 8]), (b"IDAT", b"pixels"), (b"IEND", b"")]),
            png(&[(b"acTL", &[0, 0, 0, 1, 0, 0, 0, 0]), (b"IDAT", b"pixels"), (b"IEND", b"")]),
            png(&[(b"fdAT", &[0; 4]), (b"IDAT", b"pixels"), (b"IEND", b"")]),
            webp(&[]),
            webp(&[(b"ANIM", &[0; 6]), (b"VP8 ", b"pixels")]),
            webp(&[(b"VP8X", &[2, 0, 0, 0, 0, 0, 0, 0, 0, 0]), (b"ANIM", &[0; 6]), (b"ANMF", &[0; 16])]),
        ];
        for bytes in malformed {
            assert_eq!(container_motion(&bytes), Unknown, "{bytes:?}");
        }
        for source in [png(&[(b"IDAT", b"pixels"), (b"IEND", b"")]), webp(&[(b"VP8 ", b"pixels")])] {
            for end in 0..source.len() {
                assert_eq!(container_motion(&source[..end]), Unknown, "truncation at {end}");
            }
        }
        let mut oversized_png_chunk = PNG[..8].to_vec();
        oversized_png_chunk.extend_from_slice(&u32::MAX.to_be_bytes());
        oversized_png_chunk.extend_from_slice(b"IHDR");
        assert_eq!(container_motion(&oversized_png_chunk), Unknown);
        let mut oversized_webp_chunk = webp(&[(b"VP8 ", b"pixels")]);
        oversized_webp_chunk[16..20].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(container_motion(&oversized_webp_chunk), Unknown);
    }

    #[test]
    fn motion_reads_only_saved_bare_names_and_bounds_the_file() {
        use BackgroundImageMotion::{Static, Unknown};
        let directory = tempfile::tempdir().unwrap();
        let folder = directory.path().join("backgrounds");
        fs::create_dir(&folder).unwrap();
        fs::write(folder.join("a.png"), png(&[(b"IDAT", b"pixels"), (b"IEND", b"")])).unwrap();
        let mut images = vec![BackgroundImage::fresh("a.png".into()), BackgroundImage::fresh("missing.png".into())];
        assert_eq!(image_motion(&folder, &images, "a.png").unwrap(), Static);
        assert_eq!(image_motion(&folder, &images, "missing.png").unwrap(), Unknown);
        for name in ["unsaved.png", "", ".", "..", "../a.png", "a/b.png", "/a.png", "a\\b.png", "a\0.png"] {
            // Even a malformed persisted library entry cannot authorize a path.
            images.push(BackgroundImage::fresh(name.into()));
            if name == "unsaved.png" { images.pop(); }
            assert!(image_motion(&folder, &images, name).unwrap_err().contains("not saved"), "{name}");
        }
        let large = fs::File::create(folder.join("large.png")).unwrap();
        large.set_len(MAX_IMAGE_BYTES + 1).unwrap();
        images.push(BackgroundImage::fresh("large.png".into()));
        assert_eq!(image_motion(&folder, &images, "large.png").unwrap(), Unknown);
        std::os::unix::fs::symlink(folder.join("a.png"), folder.join("linked.png")).unwrap();
        images.push(BackgroundImage::fresh("linked.png".into()));
        assert_eq!(image_motion(&folder, &images, "linked.png").unwrap(), Unknown);
    }

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
