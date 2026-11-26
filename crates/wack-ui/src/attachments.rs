//! Pictures and text files the user attaches to a message: pasted, dropped on the chat, chosen
//! with the attach button, or sent from the Terminal pane.
//!
//! Pictures travel as `Image` blocks alongside the prompt (the agent makes the viewing copy and
//! saves the original for the chat when the message is sent). Text files exist inside the message
//! text itself via [`compose`] / [`split`] (`<attached-files>`), using exact line counts and
//! attribute-escaped names so arbitrary file names and content containing lookalike tags round-trip
//! untouched.
//!
//! The limits are version 2's:
//! - Pictures: at most [`MAX_ATTACHMENTS`] (8), each at most [`MAX_ATTACHMENT_BYTES`] (20 MB),
//!   PNG, JPEG, GIF or WebP (anything else the `image` crate reads, such as a clipboard TIFF, is
//!   converted to PNG).
//! - Text files (picked, dropped, ≥32 KB pastes, and terminal selections share one budget): at
//!   most [`MAX_FILES`] (8), each at most [`MAX_FILE_BYTES`] (256 KB UTF-8), combined at most
//!   [`MAX_FILE_TOTAL_BYTES`] (512 KB UTF-8).
//!
//! Whatever is refused says why. Reading and thumbnailing are slow, so they run off the main
//! thread. Both lists live in the model that owns the composer (`ChatSession`, `AppState`'s draft),
//! so switching chats keeps them.

use std::{
    collections::HashSet,
    io::{Cursor, Read},
    path::Path,
    sync::Arc,
};

use gpui::{RenderImage, SharedString};
use wack_core::{images, llm::Image};

/// The most pictures one message carries.
pub const MAX_ATTACHMENTS: usize = 8;
/// The largest picture one can attach.
pub const MAX_ATTACHMENT_BYTES: usize = 20 * 1024 * 1024;
/// A thumbnail's long edge in pixels: the tray draws it at a third of that, sharp on Retina.
const THUMB_EDGE: u32 = 192;

/// The most text files one message carries (picked, dropped, pasted and terminal combined).
pub const MAX_FILES: usize = 8;
/// The largest text file one can attach (UTF-8 bytes).
pub const MAX_FILE_BYTES: usize = 256 * 1024;
/// The combined UTF-8 bytes of all text files on one message.
pub const MAX_FILE_TOTAL_BYTES: usize = 512 * 1024;
/// Pastes this long or longer become a text file attached to the message, as in WackCode 2.
pub const PASTE_FILE_BYTES: usize = 32 * 1024;

/// Where a text file chip came from, for the composer tray's icon and actions only.
/// Never serialized: round-tripping through [`compose`]/[`split`] yields [`FileSource::File`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FileSource {
    /// Picked from the attach button, dropped from Finder, or restored from a message.
    #[default]
    File,
    /// Pasted text ≥32 KB (offers "Insert as text" in the composer tray).
    Paste,
    /// Sent from a Terminal selection (wears the terminal icon in the composer tray).
    Terminal,
}

/// A text file attached to a message, folded into the message text at send time ([`compose`]) and
/// split back out for the transcript, queued bubbles, and restores ([`split`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileAttachment {
    /// The file's name ("notes.md", "Pasted text 1.txt", "Terminal output.txt").
    pub name: SharedString,
    /// The file's full UTF-8 text.
    pub text: Arc<str>,
    /// Presentation origin in the composer tray; never serialized.
    pub source: FileSource,
}

impl FileAttachment {
    pub fn file(name: impl Into<SharedString>, text: impl Into<Arc<str>>) -> Self {
        Self { name: name.into(), text: text.into(), source: FileSource::File }
    }

    /// A long paste as an attached text file, held to the shared text-file caps.
    pub fn from_paste(text: &str, existing: &[FileAttachment]) -> Result<Self, String> {
        if let Some(problem) = text_file_problem("That paste", text.len(), existing) {
            return Err(problem);
        }
        Ok(Self { name: pasted_text_name(existing).into(), text: text.into(), source: FileSource::Paste })
    }

    /// A terminal selection, tidied and held to the shared text-file caps.
    /// An empty or whitespace-only selection is refused, never attached.
    pub fn from_terminal(raw: &str, existing: &[FileAttachment]) -> Result<Self, String> {
        let text = normalize_terminal_selection(raw);
        if text.trim().is_empty() {
            return Err("The terminal selection is empty.".into());
        }
        if let Some(problem) = text_file_problem("That terminal selection", text.len(), existing) {
            return Err(problem);
        }
        Ok(Self { name: terminal_output_name(existing).into(), text: text.into(), source: FileSource::Terminal })
    }

    /// Exact number of lines as serialized (`text.split('\n').count()`, always at least 1).
    pub fn lines(&self) -> usize {
        self.text.split('\n').count()
    }

    /// UTF-8 byte length.
    pub fn bytes(&self) -> usize {
        self.text.len()
    }
}

/// The first free `Pasted text N.txt`, case-insensitive past the files already in the tray.
pub fn pasted_text_name(existing: &[FileAttachment]) -> String {
    let taken: HashSet<String> = existing.iter().map(|file| file.name.to_lowercase()).collect();
    let mut index = 1usize;
    while taken.contains(&format!("pasted text {index}.txt")) {
        index += 1;
    }
    format!("Pasted text {index}.txt")
}

/// The first free terminal chip name, macOS numbering past the files already in the tray:
/// `Terminal output.txt`, then `Terminal output 2.txt`, and so on.
pub fn terminal_output_name(existing: &[FileAttachment]) -> String {
    let taken: HashSet<String> = existing.iter().map(|file| file.name.to_lowercase()).collect();
    let mut index = 1usize;
    loop {
        let lower = if index == 1 { "terminal output.txt".to_string() } else { format!("terminal output {index}.txt") };
        if !taken.contains(&lower) {
            return if index == 1 { "Terminal output.txt".into() } else { format!("Terminal output {index}.txt") };
        }
        index += 1;
    }
}

/// A terminal selection tidied for the chip: CRLF/CR become LF, trailing spaces and tabs on each
/// line are trimmed, and leading/trailing blank lines picked up by a drag are dropped.
pub fn normalize_terminal_selection(raw: &str) -> String {
    let normalized = raw.replace("\r\n", "\n").replace('\r', "\n");
    let mut lines: Vec<&str> = normalized.split('\n').map(|line| line.trim_end_matches([' ', '\t'])).collect();
    while lines.first().copied() == Some("") {
        lines.remove(0);
    }
    while lines.last().copied() == Some("") {
        lines.pop();
    }
    lines.join("\n")
}

fn too_large_file(label: &str) -> String {
    format!("{label} is larger than {} KB.", MAX_FILE_BYTES / 1024)
}

fn too_many_files() -> String {
    format!("Attach at most {MAX_FILES} files to one message.")
}

fn too_much_text() -> String {
    format!("Attached text can add up to at most {} KB.", MAX_FILE_TOTAL_BYTES / 1024)
}

fn text_file_problem(label: &str, bytes: usize, existing: &[FileAttachment]) -> Option<String> {
    if bytes > MAX_FILE_BYTES {
        return Some(too_large_file(label));
    }
    if existing.len() >= MAX_FILES {
        return Some(too_many_files());
    }
    let total: usize = existing.iter().map(FileAttachment::bytes).sum();
    if total + bytes > MAX_FILE_TOTAL_BYTES {
        return Some(too_much_text());
    }
    None
}

/// Checks whether `files` currently exceeds the composer's caps (e.g. after restoring queued
/// messages onto a draft that already held files). Sending waits until it fits.
pub fn files_problem(files: &[FileAttachment]) -> Option<String> {
    if files.len() > MAX_FILES {
        return Some(format!("Attach at most {MAX_FILES} files to one message. Remove some files to send."));
    }
    let total: usize = files.iter().map(FileAttachment::bytes).sum();
    if total > MAX_FILE_TOTAL_BYTES {
        return Some(format!("Attached text is larger than {} KB. Remove some files to send.", MAX_FILE_TOTAL_BYTES / 1024));
    }
    None
}

/// Opens and closes the attached-files tail section.
pub const FILES_OPEN: &str = "<attached-files>";
pub const FILES_CLOSE: &str = "</attached-files>";
const FILE_CLOSE: &str = "</file>";

/// Legacy v3 tag written before general file attachments; parsed read-only so existing sessions
/// keep showing their pasted chips.
const LEGACY_FILES_OPEN: &str = "<pasted-files>";
const LEGACY_FILES_CLOSE: &str = "</pasted-files>";

/// `text` with `files` after it in an `<attached-files>` tail section whose exact line counts and
/// attribute-escaped names let arbitrary content round-trip through [`split`] untouched.
pub fn compose(text: &str, files: &[FileAttachment]) -> String {
    if files.is_empty() {
        return text.to_string();
    }
    let mut out = String::new();
    if !text.is_empty() {
        out.push_str(text);
        out.push_str("\n\n");
    }
    out.push_str(FILES_OPEN);
    out.push('\n');
    for file in files {
        let count = file.lines();
        out.push_str("<file name=\"");
        out.push_str(&escape_attr(&file.name));
        out.push_str("\" lines=\"");
        out.push_str(&count.to_string());
        out.push_str("\">\n");
        out.push_str(&file.text);
        out.push('\n');
        out.push_str(FILE_CLOSE);
        out.push('\n');
    }
    out.push_str(FILES_CLOSE);
    out
}

/// A message's own words and its attached files — the exact inverse of [`compose`].
/// Only a tail that parses cleanly through to the very end of the message counts as a section;
/// lookalikes in the middle of the words stay plain text. Also reads legacy `<pasted-files>`
/// tails from earlier v3 sessions.
pub fn split(message: &str) -> (&str, Vec<FileAttachment>) {
    if let Some(found) = split_with(message, FILES_OPEN, FILES_CLOSE) {
        return found;
    }
    if let Some(found) = split_with(message, LEGACY_FILES_OPEN, LEGACY_FILES_CLOSE) {
        return found;
    }
    if let Some(found) = split_legacy_pasted(message) {
        return found;
    }
    (message, Vec::new())
}

fn split_with<'a>(message: &'a str, open_tag: &str, close_tag: &str) -> Option<(&'a str, Vec<FileAttachment>)> {
    let head = format!("\n\n{open_tag}\n");
    let mut end = message.len();
    while let Some(at) = message[..end].rfind(&head) {
        if let Some(files) = parse_section(message, at + 2, open_tag, close_tag) {
            return Some((&message[..at], files));
        }
        end = at + head.len() - 1;
    }
    let leading = format!("{open_tag}\n");
    if message.starts_with(&leading)
        && let Some(files) = parse_section(message, 0, open_tag, close_tag)
    {
        return Some(("", files));
    }
    None
}

fn parse_section(message: &str, start: usize, open_tag: &str, close_tag: &str) -> Option<Vec<FileAttachment>> {
    let lines: Vec<&str> = message[start..].split('\n').collect();
    if lines.first().copied() != Some(open_tag) {
        return None;
    }
    let mut files = Vec::new();
    let mut index = 1;
    while index < lines.len() && lines[index] != close_tag {
        let (name, count) = parse_file_open(lines[index])?;
        index += 1;
        if index + count >= lines.len() {
            return None;
        }
        let text = lines[index..index + count].join("\n");
        index += count;
        if lines[index] != FILE_CLOSE {
            return None;
        }
        index += 1;
        files.push(FileAttachment { name: name.into(), text: text.into(), source: FileSource::File });
    }
    if files.is_empty() || index != lines.len() - 1 || lines[index] != close_tag {
        return None;
    }
    Some(files)
}

/// Read-only fallback for pre-unification v3 `<pasted-files>` messages where `trim_end_matches('\n')`
/// could cause `lines` to differ from `split('\n').count()`.
fn split_legacy_pasted(message: &str) -> Option<(&str, Vec<FileAttachment>)> {
    let trimmed = message.trim_end();
    if !trimmed.ends_with(LEGACY_FILES_CLOSE) {
        return None;
    }
    let at = trimmed.rfind(LEGACY_FILES_OPEN)?;
    let section = &trimmed[at..];
    let mut files = Vec::new();
    let mut rest = section;
    while let Some(start) = rest.find("<file name=\"") {
        let after = &rest[start..];
        let head_end = after.find(">\n")?;
        let body = &after[head_end + 2..];
        let close = body.find("\n</file>")?;
        let name = after["<file name=\"".len()..].split('"').next().unwrap_or_default();
        files.push(FileAttachment { name: name.to_string().into(), text: body[..close].into(), source: FileSource::File });
        rest = &body[close + "\n</file>".len()..];
    }
    if files.is_empty() {
        return None;
    }
    Some((message[..at].trim_end(), files))
}

fn parse_file_open(line: &str) -> Option<(String, usize)> {
    let rest = line.strip_prefix("<file name=\"")?;
    let (escaped_name, rest) = rest.split_once("\" lines=\"")?;
    let count_str = rest.strip_suffix("\">")?;
    if count_str.is_empty() || count_str.starts_with('0') || !count_str.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let count = count_str.parse::<usize>().ok()?;
    let name = unescape_attr(escaped_name)?;
    Some((name, count))
}

fn escape_attr(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '<' => out.push_str("&lt;"),
            '\r' => out.push_str("&#13;"),
            '\n' => out.push_str("&#10;"),
            other => out.push(other),
        }
    }
    out
}

fn unescape_attr(value: &str) -> Option<String> {
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(amp) = rest.find('&') {
        let before = &rest[..amp];
        if before.contains('"') {
            return None;
        }
        out.push_str(before);
        rest = &rest[amp + 1..];
        let semi = rest.find(';')?;
        let entity = &rest[..semi];
        match entity {
            "quot" => out.push('"'),
            "amp" => out.push('&'),
            "lt" => out.push('<'),
            "gt" => out.push('>'),
            "#10" => out.push('\n'),
            "#13" => out.push('\r'),
            _ => return None,
        }
        rest = &rest[semi + 1..];
    }
    if rest.contains('"') {
        return None;
    }
    out.push_str(rest);
    Some(out)
}

/// One file read off disk (or from the clipboard): either a picture or a UTF-8 text file.
#[derive(Clone, Debug)]
pub enum Loaded {
    Picture(Attachment),
    File(FileAttachment),
}

impl Loaded {
    /// Reads `path` as either a picture (PNG/JPEG/GIF/WebP, or a format the `image` crate can
    /// convert to PNG) or a UTF-8 text file (no NUL bytes, at most [`MAX_FILE_BYTES`]).
    /// `allow_mentions` is true in the Code area where `@` file mentions exist.
    pub fn from_path(path: &Path, allow_mentions: bool) -> Result<Self, String> {
        let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string());
        let meta = std::fs::metadata(path).map_err(|error| format!("Could not open {name}: {error}."))?;
        if meta.is_dir() {
            return Err(format!("{name} is a folder, not a file."));
        }
        let unsupported = || {
            if allow_mentions {
                format!("{name} isn't a text file or a PNG, JPEG, GIF, or WebP picture. Put it in the workspace and mention its path with @ instead.")
            } else {
                format!("{name} isn't a text file or a PNG, JPEG, GIF, or WebP picture.")
            }
        };
        let size = meta.len();
        if size as usize > MAX_ATTACHMENT_BYTES {
            let head = read_prefix(path, 4096).map_err(|error| format!("Could not open {name}: {error}."))?;
            if images::media_type(&head).is_some() {
                return Err(format!("{name} is larger than {} MB.", MAX_ATTACHMENT_BYTES / 1024 / 1024));
            }
            if looks_like_utf8_prefix(&head) {
                return Err(too_large_file(&name));
            }
            return Err(unsupported());
        }
        let bytes = std::fs::read(path).map_err(|error| format!("Could not open {name}: {error}."))?;
        if images::media_type(&bytes).is_some() {
            return Attachment::from_bytes(bytes, &name).map(Loaded::Picture);
        }
        if !bytes.contains(&0)
            && let Ok(text) = std::str::from_utf8(&bytes)
        {
            if bytes.len() > MAX_FILE_BYTES {
                return Err(too_large_file(&name));
            }
            return Ok(Loaded::File(FileAttachment::file(name, text)));
        }
        Attachment::from_bytes(bytes, &name).map(Loaded::Picture).map_err(|error| {
            if error.contains("larger than") {
                error
            } else {
                unsupported()
            }
        })
    }
}

fn read_prefix(path: &Path, max: usize) -> std::io::Result<Vec<u8>> {
    let file = std::fs::File::open(path)?;
    let mut buf = Vec::with_capacity(max);
    file.take(max as u64).read_to_end(&mut buf)?;
    Ok(buf)
}

fn looks_like_utf8_prefix(bytes: &[u8]) -> bool {
    if bytes.contains(&0) {
        return false;
    }
    match std::str::from_utf8(bytes) {
        Ok(_) => true,
        // Valid up to an incomplete trailing multi-byte char at the prefix cut.
        Err(error) => error.error_len().is_none(),
    }
}

#[derive(Clone)]
pub struct Attachment {
    /// The file's name, or "Pasted picture".
    pub name: SharedString,
    pub media_type: &'static str,
    pub bytes: Arc<Vec<u8>>,
    /// The original's size in pixels.
    pub size: (u32, u32),
    pub thumb: Arc<RenderImage>,
}

impl std::fmt::Debug for Attachment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Attachment").field("name", &self.name).field("media_type", &self.media_type).field("size", &self.size).finish()
    }
}

impl Attachment {
    /// A picture from its bytes, named `name` in the tray and to the model.
    pub fn from_bytes(bytes: Vec<u8>, name: &str) -> Result<Self, String> {
        if bytes.len() > MAX_ATTACHMENT_BYTES {
            return Err(format!("{name} is larger than {} MB.", MAX_ATTACHMENT_BYTES / 1024 / 1024));
        }
        let refused = || format!("{name} isn't a picture WackCode can send: PNG, JPEG, GIF or WebP.");
        let picture = image::load_from_memory(&bytes).map_err(|_| refused())?;
        let (media_type, bytes) = match images::media_type(&bytes) {
            Some(media_type) => (media_type, bytes),
            None => {
                let mut png = Vec::new();
                picture.write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png).map_err(|_| refused())?;
                if png.len() > MAX_ATTACHMENT_BYTES {
                    return Err(format!("{name} is larger than {} MB as a PNG.", MAX_ATTACHMENT_BYTES / 1024 / 1024));
                }
                ("image/png", png)
            }
        };
        Ok(Self { name: name.to_string().into(), media_type, size: (picture.width(), picture.height()), thumb: Arc::new(render_image(&picture, THUMB_EDGE)), bytes: Arc::new(bytes) })
    }

    /// The picture as the agent takes it: the original, labelled with its name.
    pub fn image(&self) -> Image {
        Image::new(self.media_type, images::encode(&self.bytes)).labelled(self.name.to_string())
    }
}

/// A picture as GPUI draws it, at most `edge` pixels on its long side.
pub fn render_image(picture: &image::DynamicImage, edge: u32) -> RenderImage {
    let picture = if picture.width().max(picture.height()) > edge { picture.thumbnail(edge, edge) } else { picture.clone() };
    let mut pixels = picture.to_rgba8();
    for pixel in pixels.pixels_mut() {
        pixel.0.swap(0, 2);
    }
    RenderImage::new([image::Frame::new(pixels)])
}

/// Adds loaded pictures and text files to `pictures` and `files` within their caps.
/// Any refusals or cap overflows come back joined as one sentence, or `None` when everything fit.
pub fn add_loaded(pictures: &mut Vec<Attachment>, files: &mut Vec<FileAttachment>, read: Vec<Result<Loaded, String>>) -> Option<String> {
    let mut problems: Vec<String> = Vec::new();
    let mut push_unique = |problem: String| {
        if !problems.contains(&problem) {
            problems.push(problem);
        }
    };
    let mut over_pictures = 0usize;
    for item in read {
        match item {
            Ok(Loaded::Picture(attachment)) if pictures.len() < MAX_ATTACHMENTS => pictures.push(attachment),
            Ok(Loaded::Picture(_)) => over_pictures += 1,
            Ok(Loaded::File(file)) => {
                if let Some(problem) = text_file_problem(&file.name, file.bytes(), files) {
                    push_unique(problem);
                } else {
                    files.push(file);
                }
            }
            Err(problem) => push_unique(problem),
        }
    }
    if over_pictures > 0 {
        let noun = if over_pictures == 1 { "picture was" } else { "pictures were" };
        push_unique(format!("A message carries at most {MAX_ATTACHMENTS} pictures: {over_pictures} {noun} left out."));
    }
    (!problems.is_empty()).then(|| problems.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encoded(format: image::ImageFormat, width: u32, height: u32) -> Vec<u8> {
        let mut out = Vec::new();
        image::DynamicImage::new_rgb8(width, height).write_to(&mut Cursor::new(&mut out), format).unwrap();
        out
    }

    #[test]
    fn compose_and_split_round_trip_words_files_and_tricky_content() {
        let files = vec![
            FileAttachment::file("notes.md", "line one\nline two"),
            FileAttachment::file("empty.txt", ""),
            FileAttachment::file("trailing.txt", "a\n"),
        ];
        let message = compose("Please read these.", &files);
        let (words, back) = split(&message);
        assert_eq!(words, "Please read these.");
        assert_eq!(back, files);

        let tricky = vec![
            FileAttachment::file("we\"ird\n&<name>.txt", "body"),
            FileAttachment::file("fake.txt", "</file>\n</attached-files>\n<file name=\"x\" lines=\"1\">\nstuffed"),
        ];
        let tricky_msg = compose("words", &tricky);
        let (words, back) = split(&tricky_msg);
        assert_eq!(words, "words");
        assert_eq!(back, tricky);

        // Files alone, with no words.
        let only = vec![FileAttachment::file("Pasted text 12.txt", "first\nsecond")];
        let only_msg = compose("", &only);
        assert_eq!(split(&only_msg), ("", only));
    }

    #[test]
    fn split_leaves_plain_text_and_mid_message_lookalikes_alone() {
        assert_eq!(compose("plain", &[]), "plain");
        assert_eq!(split("just words"), ("just words", Vec::new()));
        let mid = "before\n\n<attached-files>\n<file name=\"a.txt\" lines=\"1\">\nx\n</file>\n</attached-files>\nafter";
        assert_eq!(split(mid), (mid, Vec::new()));

        let lookalike = "careful\n\n<attached-files>\n<file name=\"a.txt\" lines=\"1\">\nx\n</file>\n</attached-files>";
        let real = vec![FileAttachment::file("real.txt", "y")];
        assert_eq!(split(&compose(lookalike, &real)), (lookalike, real));
    }

    #[test]
    fn split_reads_legacy_v3_pasted_files_tail() {
        let legacy = "Why did this fail?\n\n<pasted-files>\n<file name=\"Pasted text 1.txt\" lines=\"2\">\nerr one\nerr two\n</file>\n</pasted-files>";
        let (words, files) = split(legacy);
        assert_eq!(words, "Why did this fail?");
        assert_eq!(files, vec![FileAttachment::file("Pasted text 1.txt", "err one\nerr two")]);
    }

    #[test]
    fn terminal_selection_normalizes_and_names_macos_style() {
        assert_eq!(normalize_terminal_selection("  $ make   \n\nerror: boom \t\n\n\n"), "  $ make\n\nerror: boom");
        assert_eq!(normalize_terminal_selection("one\r\ntwo\r"), "one\ntwo");
        assert_eq!(normalize_terminal_selection("   \n\n"), "");

        assert_eq!(terminal_output_name(&[]), "Terminal output.txt");
        assert_eq!(terminal_output_name(&[FileAttachment::file("Terminal output.txt", "x")]), "Terminal output 2.txt");
        assert_eq!(
            terminal_output_name(&[FileAttachment::file("TERMINAL OUTPUT.TXT", "x"), FileAttachment::file("notes.md", "y")]),
            "Terminal output 2.txt"
        );
        assert_eq!(
            terminal_output_name(&[FileAttachment::file("Terminal output.txt", "x"), FileAttachment::file("Terminal output 2.txt", "y")]),
            "Terminal output 3.txt"
        );
    }

    #[test]
    fn terminal_and_paste_share_the_same_caps() {
        let chip = FileAttachment::from_terminal("  make: *** [all] Error 2  \n\n", &[]).unwrap();
        assert_eq!(chip.name.as_ref(), "Terminal output.txt");
        assert_eq!(chip.text.as_ref(), "  make: *** [all] Error 2");
        assert_eq!(chip.source, FileSource::Terminal);

        assert_eq!(FileAttachment::from_terminal("  \n\t ", &[]).unwrap_err(), "The terminal selection is empty.");
        assert_eq!(
            FileAttachment::from_terminal(&"x".repeat(MAX_FILE_BYTES + 1), &[]).unwrap_err(),
            "That terminal selection is larger than 256 KB."
        );
        assert_eq!(
            FileAttachment::from_paste(&"x".repeat(MAX_FILE_BYTES + 1), &[]).unwrap_err(),
            "That paste is larger than 256 KB."
        );

        let many: Vec<FileAttachment> = (0..MAX_FILES).map(|index| FileAttachment::file(format!("f{index}.txt"), "x")).collect();
        assert_eq!(FileAttachment::from_terminal("out", &many).unwrap_err(), "Attach at most 8 files to one message.");
        assert_eq!(FileAttachment::from_paste("out", &many).unwrap_err(), "Attach at most 8 files to one message.");

        let full = vec![
            FileAttachment::file("a.txt", "x".repeat(MAX_FILE_BYTES)),
            FileAttachment::file("b.txt", "y".repeat(MAX_FILE_BYTES)),
        ];
        assert_eq!(FileAttachment::from_terminal("x", &full).unwrap_err(), "Attached text can add up to at most 512 KB.");
        assert_eq!(FileAttachment::from_paste("x", &full).unwrap_err(), "Attached text can add up to at most 512 KB.");

        // Round-tripping drops `source` (presentation only).
        let msg = compose("Why did this fail?", &[chip]);
        let (words, back) = split(&msg);
        assert_eq!(words, "Why did this fail?");
        assert_eq!(back[0].source, FileSource::File);
        assert_eq!(back[0].name.as_ref(), "Terminal output.txt");
    }

    #[test]
    fn from_path_accepts_text_files_and_pictures_and_refuses_binaries_and_folders() {
        let dir = tempfile::tempdir().unwrap();

        let txt = dir.path().join("notes.rs");
        std::fs::write(&txt, "fn main() {}\n").unwrap();
        match Loaded::from_path(&txt, true).unwrap() {
            Loaded::File(file) => {
                assert_eq!(file.name.as_ref(), "notes.rs");
                assert_eq!(file.text.as_ref(), "fn main() {}\n");
                assert_eq!(file.source, FileSource::File);
            }
            Loaded::Picture(_) => panic!("expected text file"),
        }

        let big_txt = dir.path().join("huge.log");
        std::fs::write(&big_txt, "a".repeat(MAX_FILE_BYTES + 1)).unwrap();
        assert_eq!(Loaded::from_path(&big_txt, true).unwrap_err(), "huge.log is larger than 256 KB.");

        let bin = dir.path().join("archive.bin");
        std::fs::write(&bin, [0x00, 0x01, 0x02, 0xff]).unwrap();
        assert!(Loaded::from_path(&bin, true).unwrap_err().contains("mention its path with @"));
        assert!(!Loaded::from_path(&bin, false).unwrap_err().contains("@"));

        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        assert_eq!(Loaded::from_path(&sub, true).unwrap_err(), "sub is a folder, not a file.");

        let png = dir.path().join("shot.png");
        std::fs::write(&png, encoded(image::ImageFormat::Png, 800, 400)).unwrap();
        match Loaded::from_path(&png, true).unwrap() {
            Loaded::Picture(pic) => {
                assert_eq!((pic.media_type, pic.size), ("image/png", (800, 400)));
                assert_eq!(pic.image().label.as_deref(), Some("shot.png"));
            }
            Loaded::File(_) => panic!("expected picture"),
        }

        let tiff = dir.path().join("clip.tiff");
        std::fs::write(&tiff, encoded(image::ImageFormat::Tiff, 4, 4)).unwrap();
        match Loaded::from_path(&tiff, true).unwrap() {
            Loaded::Picture(pic) => assert_eq!(pic.media_type, "image/png"),
            Loaded::File(_) => panic!("expected converted picture"),
        }
    }

    #[test]
    fn add_loaded_enforces_both_budgets_independently() {
        let one_pic = || Attachment::from_bytes(encoded(image::ImageFormat::Png, 2, 2), "a.png").map(Loaded::Picture);
        let mut pictures = Vec::new();
        let mut files = Vec::new();
        assert_eq!(add_loaded(&mut pictures, &mut files, (0..MAX_ATTACHMENTS).map(|_| one_pic()).collect()), None);
        let mixed = vec![
            one_pic(),
            Ok(Loaded::File(FileAttachment::file("notes.md", "hello"))),
            Err("bad.bin isn't a text file.".into()),
        ];
        let said = add_loaded(&mut pictures, &mut files, mixed).unwrap();
        assert_eq!(pictures.len(), MAX_ATTACHMENTS);
        assert_eq!(files.len(), 1);
        assert!(said.contains("bad.bin isn't a text file.") && said.contains("1 picture was left out"), "{said}");
    }
}
