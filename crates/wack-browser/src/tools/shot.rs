//! `browser_screenshot`'s pictures, and turning a point in one back into the page's coordinates.
//!
//! A screenshot is the page's viewport as JPEG, at most `MAX_WIDTH` pixels wide (a model reads
//! detail no finer, and every pixel costs tokens). Its frame (`Frame`: the page's size and the
//! image's) is kept, so `browser_act` can map `x, y` in the image back to the page. A zoom shows
//! part of the page larger and keeps the full screenshot's frame: coordinates always refer to the
//! last full screenshot.

use super::Frame;

pub const MAX_WIDTH: f64 = 1280.;

/// The size of a JPEG, read from its frame header.
pub fn jpeg_size(bytes: &[u8]) -> Option<(u32, u32)> {
    let mut at = 2;
    while at + 9 < bytes.len() {
        if bytes[at] != 0xFF {
            at += 1;
            continue;
        }
        let marker = bytes[at + 1];
        let length = u16::from_be_bytes([bytes[at + 2], bytes[at + 3]]) as usize;
        // Start of frame (baseline, progressive and the rest), not the DHT/JPG/DAC markers.
        if (0xC0..=0xCF).contains(&marker) && ![0xC4, 0xC8, 0xCC].contains(&marker) {
            let height = u16::from_be_bytes([bytes[at + 5], bytes[at + 6]]) as u32;
            let width = u16::from_be_bytes([bytes[at + 7], bytes[at + 8]]) as u32;
            return Some((width, height));
        }
        at += 2 + length;
    }
    None
}

/// The scale to ask for so a viewport `width` CSS pixels wide comes back at most `MAX_WIDTH`
/// pixels wide, at a device pixel ratio of `ratio` (DevTools multiplies by it).
pub fn capture_scale(width: f64, ratio: f64) -> f64 {
    let ratio = if ratio > 0. { ratio } else { 1. };
    (MAX_WIDTH / width.max(1.)).min(1.) / ratio
}

/// A point in the screenshot, as a point in the page.
pub fn to_page(frame: Frame, x: f64, y: f64) -> Result<(f64, f64), String> {
    let (width, height) = frame.image;
    if x < 0. || y < 0. || x > width || y > height {
        return Err(format!("({x}, {y}) is outside the screenshot, which is {width} × {height}."));
    }
    Ok((x * frame.css.0 / width, y * frame.css.1 / height))
}
