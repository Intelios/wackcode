//! A chat's saved pictures: the original of every picture a tool returned or the user attached,
//! so `read_image` and the transcript can reopen one at full size after a request elided it.
//!
//! One folder per chat, `images/` in the folder its sub-agents keep their sessions in (so deleting
//! the chat deletes them, and its sub-agents share it). Files are named by their SHA-256, so a
//! picture saved twice is stored once, and are private to the user (0600 in a 0700 folder).
//! The model names a picture `img:` and the first [`SHORT`] characters of its id.

use std::{
    fs,
    io::Write as _,
    os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _},
    path::{Path, PathBuf},
};

use sha2::{Digest as _, Sha256};

/// How many of an id's characters the model sees.
pub const SHORT: usize = 12;

/// Where a chat keeps its pictures. `none()` keeps nothing, for a run with no chat behind it.
#[derive(Clone, Debug, Default)]
pub struct Images {
    folder: Option<PathBuf>,
}

impl Images {
    pub fn none() -> Self {
        Self { folder: None }
    }

    pub fn at(folder: impl Into<PathBuf>) -> Self {
        Self { folder: Some(folder.into()) }
    }

    /// The folder for the chat whose own session is at `session`.
    pub fn for_session(session: &Path) -> Self {
        Self::at(crate::agents::children_folder(session).join("images"))
    }

    pub fn folder(&self) -> Option<&Path> {
        self.folder.as_deref()
    }

    /// Saves `bytes` and returns its id; a picture already saved is not written again.
    pub fn save(&self, bytes: &[u8], media_type: &str) -> Result<String, String> {
        let folder = self.folder.as_ref().ok_or("This run keeps no pictures.")?;
        let id = hex(&Sha256::digest(bytes));
        let path = folder.join(format!("{id}.{}", extension(media_type)));
        if path.exists() {
            return Ok(id);
        }
        fs::DirBuilder::new().recursive(true).mode(0o700).create(folder).map_err(|error| format!("Could not save the picture: {error}."))?;
        let partial = folder.join(format!(".{id}.partial"));
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&partial)
            .map_err(|error| format!("Could not save the picture: {error}."))?;
        file.write_all(bytes).map_err(|error| format!("Could not save the picture: {error}."))?;
        fs::rename(&partial, &path).map_err(|error| format!("Could not save the picture: {error}."))?;
        Ok(id)
    }

    /// The saved file for an id, a short one or `img:` and one: at least six characters, and only
    /// when exactly one picture starts with them.
    pub fn find(&self, reference: &str) -> Result<PathBuf, String> {
        let prefix = reference.trim().trim_start_matches("img:").to_ascii_lowercase();
        if prefix.len() < 6 || !prefix.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(format!("\"{reference}\" is not a picture's reference: they look like img:3f2a9c01b4d7."));
        }
        let folder = self.folder.as_ref().ok_or("This run keeps no pictures.")?;
        let found: Vec<PathBuf> = fs::read_dir(folder)
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.file_name().and_then(|name| name.to_str()).is_some_and(|name| name.starts_with(&prefix)))
            .collect();
        match found.as_slice() {
            [path] => Ok(path.clone()),
            [] => Err(format!("No picture in this chat is {reference}. list_images lists them.")),
            _ => Err(format!("{reference} matches several pictures: give more of its reference.")),
        }
    }
}

/// The reference the model knows a picture by.
pub fn reference(id: &str) -> String {
    format!("img:{}", &id[..id.len().min(SHORT)])
}

fn extension(media_type: &str) -> &'static str {
    match media_type {
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        _ => "png",
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt as _;

    use super::*;

    #[test]
    fn saves_once_privately_and_finds_by_a_short_reference() {
        let folder = tempfile::tempdir().unwrap();
        let images = Images::at(folder.path().join("images"));
        let id = images.save(b"picture", "image/png").unwrap();
        assert_eq!(images.save(b"picture", "image/png").unwrap(), id);
        let path = images.find(&reference(&id)).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"picture");
        assert_eq!(path.extension().unwrap(), "png");
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(fs::metadata(folder.path().join("images")).unwrap().permissions().mode() & 0o777, 0o700);
        assert_eq!(fs::read_dir(folder.path().join("images")).unwrap().count(), 1);
    }

    #[test]
    fn a_reference_must_be_long_enough_and_known() {
        let folder = tempfile::tempdir().unwrap();
        let images = Images::at(folder.path());
        assert!(images.find("img:ab").unwrap_err().contains("not a picture's reference"));
        assert!(images.find("img:abcdef12").unwrap_err().contains("No picture"));
        assert!(Images::none().save(b"x", "image/png").is_err());
    }

    #[test]
    fn a_chats_pictures_sit_beside_its_sub_agents() {
        let images = Images::for_session(Path::new("/data/sessions/chat-1.jsonl"));
        assert_eq!(images.folder(), Some(Path::new("/data/sessions/chat-1/images")));
    }
}
