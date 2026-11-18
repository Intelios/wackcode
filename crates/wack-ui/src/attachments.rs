//! Pictures the user attaches to a message: pasted, dropped on the composer or chosen with its
//! attach button.
//!
//! An attachment keeps the original's bytes (the agent makes the viewing copy and saves the
//! original for the chat when the message is sent) and a thumbnail for the composer's tray.
//! The limits are version 2's: at most [`MAX_ATTACHMENTS`] to a message, each at most
//! [`MAX_ATTACHMENT_BYTES`], and PNG, JPEG, GIF or WebP; anything else the `image` crate can
//! read (the TIFF a macOS screenshot puts on the clipboard) is converted to PNG. Whatever is
//! refused says why. Reading and thumbnailing are slow, so they run off the main thread.
//!
//! The list lives in the model that owns the composer (`ChatSession::attachments`, the app's
//! `draft_attachments` for the new chat), so switching chats keeps it.

use std::{io::Cursor, path::Path, sync::Arc};

use gpui::{RenderImage, SharedString};
use wack_core::{images, llm::Image};

/// The most pictures one message carries.
pub const MAX_ATTACHMENTS: usize = 8;
/// The largest picture one can attach.
pub const MAX_ATTACHMENT_BYTES: usize = 20 * 1024 * 1024;
/// A thumbnail's long edge in pixels: the tray draws it at a third of that, sharp on Retina.
const THUMB_EDGE: u32 = 192;

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

    pub fn from_path(path: &Path) -> Result<Self, String> {
        let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string());
        let size = std::fs::metadata(path).map_err(|error| format!("Could not open {name}: {error}."))?.len();
        if size as usize > MAX_ATTACHMENT_BYTES {
            return Err(format!("{name} is larger than {} MB.", MAX_ATTACHMENT_BYTES / 1024 / 1024));
        }
        let bytes = std::fs::read(path).map_err(|error| format!("Could not open {name}: {error}."))?;
        Self::from_bytes(bytes, &name)
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

/// Adds what was read to `list` within the limit. The refusals, and anything past the limit,
/// come back as one sentence for the user, or `None` when everything fitted.
pub fn add(list: &mut Vec<Attachment>, read: Vec<Result<Attachment, String>>) -> Option<String> {
    let mut problems = Vec::new();
    let mut over = 0;
    for item in read {
        match item {
            Ok(attachment) if list.len() < MAX_ATTACHMENTS => list.push(attachment),
            Ok(_) => over += 1,
            Err(problem) => problems.push(problem),
        }
    }
    if over > 0 {
        let noun = if over == 1 { "picture was" } else { "pictures were" };
        problems.push(format!("A message carries at most {MAX_ATTACHMENTS} pictures: {over} {noun} left out."));
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
    fn a_png_is_kept_as_it_is_with_a_small_thumbnail() {
        let png = encoded(image::ImageFormat::Png, 800, 400);
        let attachment = Attachment::from_bytes(png.clone(), "shot.png").unwrap();
        assert_eq!((attachment.media_type, attachment.size), ("image/png", (800, 400)));
        assert_eq!(*attachment.bytes, png);
        let thumb = attachment.thumb.size(0);
        assert_eq!((thumb.width.0, thumb.height.0), (192, 96));
        assert_eq!(attachment.image().label.as_deref(), Some("shot.png"));
    }

    #[test]
    fn a_tiff_from_the_clipboard_becomes_a_png() {
        let attachment = Attachment::from_bytes(encoded(image::ImageFormat::Tiff, 4, 4), "Pasted picture").unwrap();
        assert_eq!(attachment.media_type, "image/png");
        assert_eq!(images::media_type(&attachment.bytes), Some("image/png"));
    }

    #[test]
    fn what_is_not_a_picture_or_too_many_says_why() {
        assert!(Attachment::from_bytes(b"hello".to_vec(), "notes.txt").unwrap_err().contains("notes.txt isn't a picture"));
        let one = || Attachment::from_bytes(encoded(image::ImageFormat::Png, 2, 2), "a.png");
        let mut list = Vec::new();
        assert_eq!(add(&mut list, (0..MAX_ATTACHMENTS).map(|_| one()).collect()), None);
        let said = add(&mut list, vec![one(), one(), Err("b.txt isn't a picture.".into())]).unwrap();
        assert_eq!(list.len(), MAX_ATTACHMENTS);
        assert_eq!(said, "b.txt isn't a picture. A message carries at most 8 pictures: 2 pictures were left out.");
    }
}
