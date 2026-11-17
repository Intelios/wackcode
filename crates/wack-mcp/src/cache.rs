//! The catalogue cache: each server's last tool list, so a run can offer its tools before the
//! server has started. Ported from version 2's `cache.ts`.
//!
//! Invariants:
//! - Declarations only (`ToolDecl`): never results, configuration or credentials. A list that
//!   contains any of the server's credential values is not written at all.
//! - A file is named by the connection's fingerprint, so a changed command, URL, header or
//!   variable is a cache miss.
//! - It is advisory: a call still checks the live tool before it runs. Missing, damaged,
//!   oversized, symlinked, expired or unwritable files are misses, never errors.
//! - Files are 0600 in a 0700 folder.

use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use crate::spec::{ServerSpec, ToolDecl};

const MAX_BYTES: u64 = 4 * 1024 * 1024;
const MAX_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);

pub fn path(folder: &Path, spec: &ServerSpec) -> PathBuf {
    folder.join(format!("{}.json", spec.fingerprint()))
}

pub fn read(path: &Path) -> Option<Vec<ToolDecl>> {
    let mut file = OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(path).ok()?;
    let metadata = file.metadata().ok()?;
    let age = SystemTime::now().duration_since(metadata.modified().ok()?).unwrap_or_default();
    if !metadata.is_file() || metadata.len() > MAX_BYTES || age > MAX_AGE {
        return None;
    }
    let mut bytes = Vec::new();
    Read::by_ref(&mut file).take(MAX_BYTES + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 > MAX_BYTES {
        return None;
    }
    let values: Vec<serde_json::Value> = serde_json::from_slice(&bytes).ok()?;
    values.iter().map(|value| value.get("inputSchema").filter(|schema| schema.is_object()).and(ToolDecl::parse(value))).collect()
}

pub fn write(folder: &Path, spec: &ServerSpec, tools: &[ToolDecl]) {
    let Ok(text) = serde_json::to_string(tools) else { return };
    // Even a server that echoes a credential into its schema must not have it kept here.
    if text.len() as u64 > MAX_BYTES || spec.secret_values().iter().any(|secret| text.contains(secret.as_str())) {
        return;
    }
    if fs::DirBuilder::new().recursive(true).mode(0o700).create(folder).is_err() {
        return;
    }
    let target = path(folder, spec);
    let temporary = folder.join(format!(".{}.{}.tmp", spec.fingerprint(), std::process::id()));
    let _ = fs::remove_file(&temporary);
    let written = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)
        .and_then(|mut file| file.write_all(text.as_bytes()))
        .and_then(|()| fs::rename(&temporary, &target));
    if written.is_err() {
        let _ = fs::remove_file(&temporary);
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use serde_json::json;

    use super::*;
    use crate::spec::test_spec;

    fn peek() -> ToolDecl {
        ToolDecl::parse(&json!({ "name": "peek", "inputSchema": { "type": "object" }, "annotations": { "readOnlyHint": true } })).unwrap()
    }

    #[test]
    fn a_catalogue_round_trips_privately() {
        let folder = tempfile::tempdir().unwrap();
        let cache = folder.path().join("mcp-cache");
        let spec = test_spec("Mock");
        write(&cache, &spec, &[peek()]);
        assert_eq!(read(&path(&cache, &spec)), Some(vec![peek()]));
        assert_eq!(fs::metadata(path(&cache, &spec)).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(fs::metadata(&cache).unwrap().permissions().mode() & 0o777, 0o700);
    }

    #[test]
    fn a_credential_in_the_list_or_a_changed_credential_misses() {
        let folder = tempfile::tempdir().unwrap();
        let mut spec = test_spec("Mock");
        spec.env.insert("TOKEN".into(), "secret-value-123".into());
        let mut leaky = peek();
        leaky.description = Some("Uses secret-value-123".into());
        write(folder.path(), &spec, &[leaky]);
        assert_eq!(read(&path(folder.path(), &spec)), None);
        write(folder.path(), &spec, &[peek()]);
        assert!(read(&path(folder.path(), &spec)).is_some());
        spec.env.insert("TOKEN".into(), "changed-secret-5678".into());
        assert_eq!(read(&path(folder.path(), &spec)), None);
    }

    #[test]
    fn damaged_symlinked_and_malformed_files_miss() {
        let folder = tempfile::tempdir().unwrap();
        let spec = test_spec("Mock");
        let file = path(folder.path(), &spec);
        fs::write(&file, "{").unwrap();
        assert_eq!(read(&file), None);
        fs::write(&file, json!([{ "name": "x", "inputSchema": [] }]).to_string()).unwrap();
        assert_eq!(read(&file), None);
        fs::remove_file(&file).unwrap();
        let real = folder.path().join("real.json");
        fs::write(&real, serde_json::to_string(&[peek()]).unwrap()).unwrap();
        std::os::unix::fs::symlink(&real, &file).unwrap();
        assert_eq!(read(&file), None);
    }
}
