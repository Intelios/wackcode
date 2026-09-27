//! ZIPs are data, never extensions: validate every entry before the normal skill scanner sees
//! the extracted tree. Staging stays private and hidden from live skill discovery until copied.
use std::{
    collections::HashMap,
    fs::{self, File, OpenOptions},
    io::{self, Cursor, Read, Write},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};
use tempfile::TempDir;
use unicode_normalization::UnicodeNormalization;
use zip::{CompressionMethod, ZipArchive};

#[derive(Clone, Copy)]
struct Limits { compressed: u64, extracted: u64, entries: usize }
const LIMITS: Limits = Limits { compressed: 100 * 1024 * 1024, extracted: 250 * 1024 * 1024, entries: 20_000 };
const TOO_LARGE: &str = "That ZIP is too large. Use at most 100 MiB compressed, 250 MiB extracted and 20,000 entries.";

pub fn extract(home: &Path, path: &Path) -> Result<TempDir, String> {
    extract_with_limits(home, path, LIMITS)
}

fn invalid(error: impl std::fmt::Display) -> String {
    format!("Could not import that ZIP: {error}. Choose a valid, unencrypted ZIP using Stored or Deflate compression.")
}

// Bound the entry count before the ZIP library allocates its central-directory index. ZIP64
// and split archives aren't needed within our import limits and are intentionally unsupported.
fn check_directory(bytes: &[u8], limit: usize) -> Result<usize, String> {
    let start = bytes.len().saturating_sub(65_535 + 22);
    let end = bytes.len().checked_sub(22).ok_or_else(|| invalid("the archive is incomplete"))?;
    for offset in (start..=end).rev() {
        let tail = &bytes[offset..];
        if !tail.starts_with(b"PK\x05\x06") { continue; }
        let word = |i| u16::from_le_bytes([tail[i], tail[i + 1]]);
        if offset + 22 + usize::from(word(20)) != bytes.len() { continue; }
        if word(4) != 0 || word(6) != 0 || word(8) != word(10)
            || word(10) == u16::MAX || tail[12..16] == [255; 4] || tail[16..20] == [255; 4]
            || (offset >= 20 && &bytes[offset - 20..offset - 16] == b"PK\x06\x07")
        {
            return Err(invalid("split and ZIP64 archives are not supported"));
        }
        if usize::from(word(10)) > limit { return Err(TOO_LARGE.into()); }
        return Ok(usize::from(word(10)));
    }
    Err(invalid("the archive's directory is missing or damaged"))
}

#[derive(Debug)]
struct Entry { path: PathBuf, directory: bool, skip: bool, mode: u32 }

fn entry_path(name: &str) -> Result<PathBuf, String> {
    let trimmed = name.strip_suffix('/').unwrap_or(name);
    if trimmed.is_empty() || name.len() > 4_096 || trimmed.split('/').count() > 128 || name.contains(['\\', ':', '\0'])
        || trimmed.split('/').any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(invalid("the archive contains an unsafe path"));
    }
    Ok(PathBuf::from(trimmed))
}

fn ignored(path: &Path) -> bool {
    path.iter().any(|part| {
        let name = part.to_string_lossy();
        matches!(name.as_ref(), "__MACOSX" | ".DS_Store" | ".git" | "node_modules") || name.starts_with("._")
    })
}

// Track implicit directories too: `Assets/a` and `assets/b` must not silently merge on macOS.
fn register_path(paths: &mut HashMap<String, (PathBuf, bool, bool)>, path: &Path, directory: bool) -> Result<(), String> {
    let mut partial = PathBuf::new();
    for part in path.iter() {
        partial.push(part);
        let explicit = partial == path;
        let is_dir = !explicit || directory;
        let key: String = partial.to_string_lossy().nfd().flat_map(char::to_lowercase).collect();
        if let Some((spelling, was_dir, was_explicit)) = paths.get_mut(&key) {
            if *spelling != partial || *was_dir != is_dir || (explicit && *was_explicit) {
                return Err(invalid("the archive contains duplicate or conflicting paths"));
            }
            *was_explicit |= explicit;
        } else {
            paths.insert(key, (partial.clone(), is_dir, explicit));
        }
    }
    Ok(())
}

fn extract_with_limits(home: &Path, path: &Path, limits: Limits) -> Result<TempDir, String> {
    let file = File::open(path).map_err(invalid)?;
    if !file.metadata().map_err(invalid)?.is_file() { return Err(invalid("that is not a file")); }
    if file.metadata().map_err(invalid)?.len() > limits.compressed { return Err(TOO_LARGE.into()); }
    // A bounded snapshot prevents a changed source file from invalidating our preflight checks.
    let mut bytes = Vec::new();
    file.take(limits.compressed + 1).read_to_end(&mut bytes).map_err(invalid)?;
    if bytes.len() as u64 > limits.compressed { return Err(TOO_LARGE.into()); }
    let declared_entries = check_directory(&bytes, limits.entries)?;
    let mut archive = ZipArchive::new(Cursor::new(bytes)).map_err(invalid)?;
    // The library indexes by name and collapses exact duplicates; compare against the original
    // count so none can bypass path validation (or hide corrupt data in a discarded entry).
    if archive.len() != declared_entries { return Err(invalid("the archive contains duplicate paths")); }
    if archive.len() > limits.entries { return Err(TOO_LARGE.into()); }
    let mut paths = HashMap::new();
    let mut entries = Vec::new();
    let mut declared = 0u64;
    for index in 0..archive.len() {
        let entry = archive.by_index(index).map_err(invalid)?;
        let path = entry_path(entry.name())?;
        let directory = entry.is_dir();
        let mode = entry.unix_mode().unwrap_or(0);
        let kind = mode & 0o170000;
        if !matches!(kind, 0 | 0o040000 | 0o100000) || (kind == 0o040000 && !directory) || (kind == 0o100000 && directory) {
            return Err(invalid("links and special files are not supported"));
        }
        if entry.encrypted() || !matches!(entry.compression(), CompressionMethod::Stored | CompressionMethod::Deflated) {
            return Err(invalid("the archive uses encryption or unsupported compression"));
        }
        if directory && entry.size() != 0 { return Err(invalid("a directory contains file data")); }
        declared = declared.checked_add(entry.size()).ok_or(TOO_LARGE)?;
        if declared > limits.extracted { return Err(TOO_LARGE.into()); }
        register_path(&mut paths, &path, directory)?;
        if paths.len() > limits.entries { return Err(TOO_LARGE.into()); }
        entries.push(Entry { skip: ignored(&path), path, directory, mode });
    }
    let library = crate::skills::library_dir(home);
    fs::create_dir_all(&library).map_err(invalid)?;
    let staging = tempfile::Builder::new().prefix(".wackcode-zip-").permissions(fs::Permissions::from_mode(0o700)).tempdir_in(&library).map_err(invalid)?;
    let mut actual = 0u64;
    for (index, entry) in entries.iter().enumerate() {
        let mut source = archive.by_index(index).map_err(invalid)?;
        let destination = staging.path().join(&entry.path);
        if !entry.skip && entry.directory { fs::create_dir_all(&destination).map_err(invalid)?; }
        let mut output = if !entry.skip && !entry.directory {
            fs::create_dir_all(destination.parent().unwrap()).map_err(invalid)?;
            Some(OpenOptions::new().write(true).create_new(true).mode(0o600).open(&destination).map_err(invalid)?)
        } else { None };
        let expected = source.size();
        let mut sink = io::sink();
        let writer: &mut dyn Write = match output.as_mut() { Some(file) => file, None => &mut sink };
        // Read ignored entries too, so corrupt data and bombs can't hide in metadata folders.
        let count = io::copy(&mut (&mut source).take(limits.extracted - actual + 1), writer).map_err(invalid)?;
        actual += count;
        if actual > limits.extracted { return Err(TOO_LARGE.into()); }
        if count != expected { return Err(invalid("an entry's size does not match its contents")); }
        if let Some(file) = output {
            // Keep scripts executable; never carry setuid/setgid/sticky or writable-by-others bits.
            file.set_permissions(fs::Permissions::from_mode(0o600 | (entry.mode & 0o155))).map_err(invalid)?;
        }
    }
    Ok(staging)
}

#[cfg(test)]
mod tests {
    use super::*;
    use zip::{write::SimpleFileOptions, ZipWriter};

    fn fixture(home: &Path, entries: &[(&str, &[u8], u32)]) -> PathBuf {
        let path = home.join("fixture.zip");
        let mut writer = ZipWriter::new(File::create(&path).unwrap());
        for (name, bytes, mode) in entries {
            let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated).unix_permissions(*mode);
            if name.ends_with('/') { writer.add_directory(*name, options).unwrap(); }
            else { writer.start_file(*name, options).unwrap(); writer.write_all(bytes).unwrap(); }
        }
        writer.finish().unwrap();
        path
    }

    fn patch_central(path: &Path, field: usize, value: &[u8]) {
        let mut bytes = fs::read(path).unwrap();
        let offset = bytes.windows(4).position(|part| part == b"PK\x01\x02").unwrap();
        bytes[offset + field..offset + field + value.len()].copy_from_slice(value);
        fs::write(path, bytes).unwrap();
    }

    fn no_staging(home: &Path) {
        let library = crate::skills::library_dir(home);
        if library.exists() { assert_eq!(fs::read_dir(library).unwrap().count(), 0); }
    }

    #[test]
    fn preserves_root_wrapped_and_multiple_skill_trees_and_cleans_up() {
        for prefix in ["", "logo-design/", "bundle/logo-design/"] {
            let home = tempfile::tempdir().unwrap();
            let skill = format!("{prefix}SKILL.md");
            let script = format!("{prefix}scripts/run.sh");
            let asset = format!("{prefix}assets/logo.bin");
            let path = fixture(home.path(), &[
                (&skill, b"---\nname: logo-design\ndescription: Logos\n---\nOriginal text", 0o644),
                (&script, b"#!/bin/sh\necho hello", 0o755), (&asset, &[0, 255, 1, 128], 0o644),
                ("other/SKILL.md", b"---\nname: other\ndescription: Other\n---", 0o644),
            ]);
            let staging = extract(home.path(), &path).unwrap();
            assert_eq!(fs::metadata(staging.path()).unwrap().permissions().mode() & 0o777, 0o700);
            assert_eq!(fs::read(staging.path().join(&asset)).unwrap(), [0, 255, 1, 128]);
            assert_eq!(fs::metadata(staging.path().join(&script)).unwrap().permissions().mode() & 0o7777, 0o755);
            assert!(staging.path().join("other/SKILL.md").exists());
            let base = staging.path().join(prefix);
            let installed = crate::skills::copy_into_library(home.path(), "logo-design", &base.join("SKILL.md"), &base).unwrap();
            assert_eq!(fs::read(installed.parent().unwrap().join("assets/logo.bin")).unwrap(), [0, 255, 1, 128]);
            let temporary_path = staging.path().to_path_buf();
            drop(staging);
            assert!(!temporary_path.exists());
        }
    }

    #[test]
    fn ignores_metadata_and_dependencies_but_preserves_other_hidden_assets() {
        let home = tempfile::tempdir().unwrap();
        let path = fixture(home.path(), &[
            ("SKILL.md", b"skill", 0o644), ("__MACOSX/._SKILL.md", b"metadata", 0o644),
            ("assets/.DS_Store", b"metadata", 0o644), ("assets/._logo.svg", b"metadata", 0o644),
            (".git/config", b"git", 0o644), ("node_modules/a/index.js", b"dependency", 0o644),
            ("assets/.useful", b"keep", 0o644),
        ]);
        let staging = extract(home.path(), &path).unwrap();
        assert_eq!(fs::read_dir(staging.path()).unwrap().count(), 2);
        assert_eq!(fs::read_dir(staging.path().join("assets")).unwrap().count(), 1);
        assert_eq!(fs::read(staging.path().join("assets/.useful")).unwrap(), b"keep");
    }

    #[test]
    fn refuses_unsafe_paths_and_links_without_leaving_staging() {
        for name in ["../escape", "/absolute", "a/../../escape", "a\\..\\escape", "C:/escape", "a/./file", "a//file"] {
            let home = tempfile::tempdir().unwrap();
            let path = fixture(home.path(), &[(name, b"bad", 0o644)]);
            assert!(extract(home.path(), &path).is_err(), "accepted {name}");
            no_staging(home.path());
        }
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join("link.zip");
        let mut writer = ZipWriter::new(File::create(&path).unwrap());
        writer.add_symlink("link", "../outside", SimpleFileOptions::default()).unwrap();
        writer.finish().unwrap();
        assert!(extract(home.path(), &path).unwrap_err().contains("links"));
        no_staging(home.path());
    }

    #[test]
    fn refuses_duplicate_case_unicode_and_file_directory_collisions() {
        for (a, b) in [("Assets/a", "assets/b"), ("a", "a/b"), ("a/b", "a"), ("a/", "A/"), ("caf\u{e9}/a", "cafe\u{301}/b")] {
            let home = tempfile::tempdir().unwrap();
            let path = fixture(home.path(), &[(a, b"", 0o644), (b, b"", 0o644)]);
            assert!(extract(home.path(), &path).unwrap_err().contains("conflicting"), "accepted {a}, {b}");
            no_staging(home.path());
        }
        let home = tempfile::tempdir().unwrap();
        let path = fixture(home.path(), &[("a", b"", 0o644), ("b", b"", 0o644)]);
        let mut bytes = fs::read(&path).unwrap();
        let offsets: Vec<_> = bytes.windows(4).enumerate().filter_map(|(i, part)| (part == b"PK\x01\x02").then_some(i)).collect();
        bytes[offsets[1] + 46] = b'a';
        fs::write(&path, bytes).unwrap();
        assert!(extract(home.path(), &path).is_err());
        no_staging(home.path());
    }

    #[test]
    fn accepts_explicit_directories_after_their_children() {
        let home = tempfile::tempdir().unwrap();
        let path = fixture(home.path(), &[("assets/logo.svg", b"svg", 0o644), ("assets/", b"", 0o755)]);
        assert!(extract(home.path(), &path).unwrap().path().join("assets/logo.svg").exists());
    }

    #[test]
    fn rejects_corruption_encryption_special_files_and_unsupported_compression() {
        for (field, bytes) in [(16, vec![1, 2, 3, 4]), (8, vec![1, 0]), (10, vec![99, 0]), (38, (0o010644u32 << 16).to_le_bytes().to_vec())] {
            let home = tempfile::tempdir().unwrap();
            let path = fixture(home.path(), &[("SKILL.md", b"content", 0o644)]);
            patch_central(&path, field, &bytes);
            assert!(extract(home.path(), &path).is_err(), "accepted bad field {field}");
            no_staging(home.path());
        }
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join("invalid.zip");
        fs::write(&path, b"not a ZIP").unwrap();
        assert!(extract(home.path(), &path).is_err());
        no_staging(home.path());
    }

    #[test]
    fn strips_privileged_permissions() {
        let home = tempfile::tempdir().unwrap();
        let path = fixture(home.path(), &[("run.sh", b"echo hello", 0o755)]);
        patch_central(&path, 38, &(0o107777u32 << 16).to_le_bytes());
        let staging = extract(home.path(), &path).unwrap();
        assert_eq!(fs::metadata(staging.path().join("run.sh")).unwrap().permissions().mode() & 0o7777, 0o755);
    }

    #[test]
    fn bounds_compressed_declared_actual_and_entry_counts() {
        let home = tempfile::tempdir().unwrap();
        let path = fixture(home.path(), &[("SKILL.md", &[b'a'; 256], 0o644)]);
        for limits in [Limits { compressed: 1, ..LIMITS }, Limits { entries: 0, ..LIMITS }, Limits { extracted: 10, ..LIMITS }] {
            assert!(extract_with_limits(home.path(), &path, limits).unwrap_err().contains("too large"));
            no_staging(home.path());
        }
        // Lie about the size: streaming must fail rather than trusting the central directory.
        patch_central(&path, 24, &1u32.to_le_bytes());
        assert!(extract_with_limits(home.path(), &path, Limits { extracted: 10, ..LIMITS }).is_err());
        no_staging(home.path());
    }
}
