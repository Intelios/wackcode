//! Viewing copies: what a picture becomes before any model is sent it.
//!
//! Every picture fits inside limits each provider accepts, whatever the source: at most
//! [`MAX_EDGE`] pixels on its long edge (Anthropic's own cap; past it the API downscales a user's
//! picture and rejects a tool's) and at most [`MAX_PICTURE_BYTES`] of base64. A picture already
//! inside both passes through untouched, so a browser screenshot keeps the exact pixels its
//! coordinates refer to. Anything else is scaled down and re-encoded: PNG when the source was
//! lossless and PNG fits (text and UI stay crisp), JPEG otherwise, stepping the quality down and
//! then the size until it fits. A GIF becomes its first frame as a PNG, since not every provider
//! takes an animated one.

use std::io::Cursor;

use base64::Engine as _;
use image::{DynamicImage, ImageFormat, ImageReader, RgbImage, imageops::FilterType};
use wack_llm::Image;

/// The longest edge a viewing copy has.
pub const MAX_EDGE: u32 = 1568;
/// The most base64 one viewing copy has.
pub const MAX_PICTURE_BYTES: usize = 1_500_000;

/// A picture ready for a model.
#[derive(Debug)]
pub struct Viewing {
    pub image: Image,
    /// The original's size in pixels.
    pub original: (u32, u32),
    /// The copy's size in pixels.
    pub shown: (u32, u32),
}

impl Viewing {
    /// A line for the model when the copy is smaller than the original, so it can map
    /// coordinates back.
    pub fn note(&self) -> Option<String> {
        if self.original == self.shown {
            return None;
        }
        let (width, height) = self.original;
        let (shown_width, shown_height) = self.shown;
        let factor = f64::from(width) / f64::from(shown_width.max(1));
        Some(format!(
            "[Picture resized: original {width} × {height}, viewing copy {shown_width} × {shown_height}. Multiply the copy's coordinates by {factor:.2} for the original's.]"
        ))
    }
}

/// The picture types every provider takes, recognised by their first bytes.
pub fn media_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

/// A picture's width and height, read from its header without decoding it.
pub fn dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    ImageReader::new(Cursor::new(bytes)).with_guessed_format().ok()?.into_dimensions().ok()
}

pub fn encode(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

pub fn decode(data: &str) -> Result<Vec<u8>, String> {
    base64::engine::general_purpose::STANDARD.decode(data).map_err(|_| "The picture's data is damaged.".to_string())
}

/// The viewing copy of a picture's bytes. Slow for a large picture: call it off the async
/// threads.
pub fn viewing_copy(bytes: &[u8]) -> Result<Viewing, String> {
    let media = media_type(bytes).ok_or("It is not a picture the model can view (PNG, JPEG, GIF or WebP).")?;
    let original = dimensions(bytes).ok_or("The picture could not be read: it may be damaged.")?;
    let base64_len = bytes.len().div_ceil(3) * 4;
    if media != "image/gif" && original.0.max(original.1) <= MAX_EDGE && base64_len <= MAX_PICTURE_BYTES {
        return Ok(Viewing { image: Image::new(media, encode(bytes)), original, shown: original });
    }
    let format = ImageFormat::from_mime_type(media).ok_or("The picture could not be read.")?;
    let picture = image::load_from_memory_with_format(bytes, format).map_err(|error| format!("The picture could not be read: {error}."))?;
    let lossless = matches!(media, "image/png" | "image/gif");
    let mut edge = MAX_EDGE.min(original.0.max(original.1));
    loop {
        let scaled = if picture.width().max(picture.height()) > edge { picture.resize(edge, edge, FilterType::Lanczos3) } else { picture.clone() };
        let shown = (scaled.width(), scaled.height());
        if lossless && let Some(png) = png(&scaled).filter(|png| png.len().div_ceil(3) * 4 <= MAX_PICTURE_BYTES) {
            return Ok(Viewing { image: Image::new("image/png", encode(&png)), original, shown });
        }
        let flat = flatten(&scaled);
        for quality in [85, 70, 55, 40] {
            let jpeg = jpeg(&flat, quality)?;
            if jpeg.len().div_ceil(3) * 4 <= MAX_PICTURE_BYTES {
                return Ok(Viewing { image: Image::new("image/jpeg", encode(&jpeg)), original, shown });
            }
        }
        if edge <= 256 {
            return Err("The picture could not be made small enough to send.".to_string());
        }
        edge = edge * 3 / 4;
    }
}

fn png(picture: &DynamicImage) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    picture.write_to(&mut Cursor::new(&mut out), ImageFormat::Png).ok()?;
    Some(out)
}

fn jpeg(picture: &RgbImage, quality: u8) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality)
        .encode_image(picture)
        .map_err(|error| format!("The picture could not be re-encoded: {error}."))?;
    Ok(out)
}

/// The picture on white, for JPEG, which has no transparency (black would hide dark text).
fn flatten(picture: &DynamicImage) -> RgbImage {
    let rgba = picture.to_rgba8();
    RgbImage::from_fn(rgba.width(), rgba.height(), |x, y| {
        let [r, g, b, a] = rgba.get_pixel(x, y).0;
        let over = |channel: u8| ((u16::from(channel) * u16::from(a) + 255 * (255 - u16::from(a))) / 255) as u8;
        image::Rgb([over(r), over(g), over(b)])
    })
}

#[cfg(test)]
mod tests {
    use image::{Rgb, Rgba, RgbaImage};

    use super::*;

    fn png_of(width: u32, height: u32) -> Vec<u8> {
        let picture = RgbaImage::from_fn(width, height, |x, y| Rgba([(x % 256) as u8, (y % 256) as u8, 128, 255]));
        let mut out = Vec::new();
        DynamicImage::ImageRgba8(picture).write_to(&mut Cursor::new(&mut out), ImageFormat::Png).unwrap();
        out
    }

    fn noise_png(width: u32, height: u32) -> Vec<u8> {
        let mut state: u32 = 7;
        let picture = image::RgbImage::from_fn(width, height, |_, _| {
            state = state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            let bytes = state.to_le_bytes();
            Rgb([bytes[0], bytes[1], bytes[2]])
        });
        let mut out = Vec::new();
        DynamicImage::ImageRgb8(picture).write_to(&mut Cursor::new(&mut out), ImageFormat::Png).unwrap();
        out
    }

    #[test]
    fn recognises_the_four_types_by_their_first_bytes() {
        assert_eq!(media_type(&png_of(2, 2)), Some("image/png"));
        assert_eq!(media_type(&[0xFF, 0xD8, 0xFF, 0xE0]), Some("image/jpeg"));
        assert_eq!(media_type(b"GIF89a..."), Some("image/gif"));
        assert_eq!(media_type(b"RIFF\0\0\0\0WEBPVP8 "), Some("image/webp"));
        assert_eq!(media_type(b"<svg"), None);
    }

    #[test]
    fn a_picture_inside_the_limits_passes_through_untouched() {
        let bytes = png_of(800, 600);
        let viewing = viewing_copy(&bytes).unwrap();
        assert_eq!(viewing.image.data, encode(&bytes));
        assert_eq!((viewing.original, viewing.shown), ((800, 600), (800, 600)));
        assert_eq!(viewing.note(), None);
    }

    #[test]
    fn a_large_picture_is_scaled_to_the_long_edge_and_says_so() {
        let viewing = viewing_copy(&png_of(3136, 1000)).unwrap();
        assert_eq!(viewing.shown, (1568, 500));
        assert_eq!(viewing.image.media_type, "image/png", "a simple PNG stays PNG");
        let note = viewing.note().unwrap();
        assert!(note.contains("original 3136 × 1000, viewing copy 1568 × 500"));
        assert!(note.contains("by 2.00"));
    }

    #[test]
    fn a_picture_too_heavy_for_png_becomes_a_jpeg_that_fits() {
        let viewing = viewing_copy(&noise_png(1500, 1500)).unwrap();
        assert_eq!(viewing.image.media_type, "image/jpeg");
        assert!(viewing.image.data.len() <= MAX_PICTURE_BYTES);
        assert_eq!(dimensions(&decode(&viewing.image.data).unwrap()), Some(viewing.shown));
    }

    #[test]
    fn something_that_is_not_a_picture_is_refused() {
        assert!(viewing_copy(b"hello").unwrap_err().contains("not a picture"));
        assert!(viewing_copy(b"\x89PNG\r\n\x1a\nbroken").is_err());
    }
}
