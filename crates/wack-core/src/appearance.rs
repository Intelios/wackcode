//! How the app looks, as the user chose it: light or dark, the theme colour, the accent, and an
//! image behind the window. The UI derives every colour from these (`wack-ui/src/theme.rs`); this
//! module only keeps the choices and prepares the image.
//!
//! Invariants:
//! - Colours are kept as `#rrggbb` or not at all (`None` is the default). Anything else is
//!   refused here, so the theme never has to guess.
//! - A backdrop is always WackCode's own copy in `backgrounds/`, never the user's original: the
//!   original can move or be deleted without the window losing its picture. The copy is a JPEG
//!   no larger than [`MAX_BACKDROP_EDGE`] on its long side, so a huge photo costs no more to draw
//!   than a screenful.
//! - Copies no setting refers to are deleted when the setting changes.

use std::{
    fs,
    hash::{Hash, Hasher},
    io::Cursor,
    path::Path,
};

use image::{ImageReader, Limits, imageops::FilterType};
use serde::{Deserialize, Serialize};

/// The long side of a stored backdrop, in pixels: a 5K display's width.
pub const MAX_BACKDROP_EDGE: u32 = 5120;
/// The largest picture WackCode will open, on either side.
const MAX_SOURCE_EDGE: u32 = 20_000;
/// The most a backdrop's veil can tone the picture, in percent.
pub const MAX_VEIL: u8 = 90;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
    /// Light or dark as macOS is.
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Backdrop {
    /// The copy's file name inside `backgrounds/`.
    pub file: String,
    pub width: u32,
    pub height: u32,
    /// The point of the picture kept in view when the window's shape crops it: 0 to 1 across and
    /// down, the middle by default.
    #[serde(default = "middle")]
    pub focus_x: f32,
    #[serde(default = "middle")]
    pub focus_y: f32,
    /// How far the picture is toned towards the window's colour, 0 to [`MAX_VEIL`] percent.
    #[serde(default)]
    pub veil: u8,
}

fn middle() -> f32 {
    0.5
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Appearance {
    #[serde(default)]
    pub mode: Mode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme_colour: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backdrop: Option<Backdrop>,
}

impl Appearance {
    /// The same choices, checked: a colour that is not `#rrggbb` is refused, and the backdrop's
    /// focus and veil are kept in range.
    pub fn checked(mut self) -> Result<Self, String> {
        for colour in [&mut self.theme_colour, &mut self.accent].into_iter().flatten() {
            *colour = hex_colour(colour).ok_or_else(|| format!("“{colour}” is not a colour. Write it as #rrggbb."))?;
        }
        if let Some(backdrop) = &mut self.backdrop {
            let unit = |value: f32| if value.is_finite() { value.clamp(0., 1.) } else { 0.5 };
            backdrop.focus_x = unit(backdrop.focus_x);
            backdrop.focus_y = unit(backdrop.focus_y);
            backdrop.veil = backdrop.veil.min(MAX_VEIL);
            if backdrop.file.contains('/') || backdrop.file.starts_with('.') {
                return Err("That backdrop is not one of WackCode’s.".into());
            }
        }
        Ok(self)
    }
}

/// `#rrggbb` in lower case for a colour written as `#RGB`, `#rrggbb` or without the `#`.
pub fn hex_colour(text: &str) -> Option<String> {
    let digits = text.trim().trim_start_matches('#');
    let digits: String = match digits.len() {
        3 => digits.chars().flat_map(|digit| [digit, digit]).collect(),
        6 => digits.to_string(),
        _ => return None,
    };
    digits.chars().all(|digit| digit.is_ascii_hexdigit()).then(|| format!("#{}", digits.to_ascii_lowercase()))
}

/// Reads a picture, scales it down to [`MAX_BACKDROP_EDGE`] if larger, and writes WackCode's copy
/// into `backgrounds`. Slow for a large photo: run it off the UI thread.
pub fn import_backdrop(source: &Path, backgrounds: &Path) -> Result<Backdrop, String> {
    let name = source.file_name().map(|name| name.to_string_lossy().to_string()).unwrap_or_default();
    let bytes = fs::read(source).map_err(|error| format!("Could not read {name}: {error}."))?;
    let mut reader = ImageReader::new(Cursor::new(&bytes))
        .with_guessed_format()
        .map_err(|error| format!("Could not read {name}: {error}."))?;
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_SOURCE_EDGE);
    limits.max_image_height = Some(MAX_SOURCE_EDGE);
    reader.limits(limits);
    if reader.format().is_none() {
        return Err(format!("{name} is not a picture WackCode can open. Use a PNG, JPEG, WebP or GIF."));
    }
    let picture = reader.decode().map_err(|error| format!("Could not open {name} as a picture: {error}."))?;
    let picture = if picture.width().max(picture.height()) > MAX_BACKDROP_EDGE {
        picture.resize(MAX_BACKDROP_EDGE, MAX_BACKDROP_EDGE, FilterType::Lanczos3)
    } else {
        picture
    };
    let picture = picture.to_rgb8();

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hasher);
    let file = format!("{:016x}.jpg", hasher.finish());
    fs::create_dir_all(backgrounds).map_err(|error| format!("Could not create the backgrounds folder: {error}."))?;
    let path = backgrounds.join(&file);
    let mut encoded = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut encoded, 92)
        .encode_image(&picture)
        .map_err(|error| format!("Could not save the backdrop: {error}."))?;
    let temporary = path.with_extension("jpg.tmp");
    fs::write(&temporary, encoded)
        .and_then(|_| fs::rename(&temporary, &path))
        .map_err(|error| format!("Could not save the backdrop: {error}."))?;
    Ok(Backdrop { file, width: picture.width(), height: picture.height(), focus_x: 0.5, focus_y: 0.5, veil: 0 })
}

/// Deletes copies in `backgrounds` other than `keep`.
pub fn tidy_backgrounds(backgrounds: &Path, keep: Option<&str>) {
    let Ok(entries) = fs::read_dir(backgrounds) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if Some(name.as_str()) != keep {
            let _ = fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_are_kept_as_hex_or_refused() {
        assert_eq!(hex_colour("#C2EE4A").as_deref(), Some("#c2ee4a"));
        assert_eq!(hex_colour("c2ee4a").as_deref(), Some("#c2ee4a"));
        assert_eq!(hex_colour("#abc").as_deref(), Some("#aabbcc"));
        assert_eq!(hex_colour("olive"), None);
        assert_eq!(hex_colour("#12345"), None);
        let refused = Appearance { accent: Some("lime".into()), ..Appearance::default() }.checked();
        assert!(refused.is_err());
    }

    #[test]
    fn a_backdrop_is_scaled_down_and_kept_in_range() {
        let folder = tempfile::tempdir().unwrap();
        let source = folder.path().join("wide.png");
        image::RgbImage::from_fn(6000, 3000, |x, _| image::Rgb([(x % 256) as u8, 40, 90])).save(&source).unwrap();
        let backgrounds = folder.path().join("backgrounds");
        let backdrop = import_backdrop(&source, &backgrounds).unwrap();
        assert_eq!((backdrop.width, backdrop.height), (MAX_BACKDROP_EDGE, MAX_BACKDROP_EDGE / 2));
        assert!(backgrounds.join(&backdrop.file).exists());

        let wild = Appearance {
            backdrop: Some(Backdrop { focus_x: 7., focus_y: f32::NAN, veil: 200, ..backdrop.clone() }),
            ..Appearance::default()
        }
        .checked()
        .unwrap();
        let kept = wild.backdrop.unwrap();
        assert_eq!((kept.focus_x, kept.focus_y, kept.veil), (1., 0.5, MAX_VEIL));

        fs::write(backgrounds.join("old.jpg"), b"x").unwrap();
        tidy_backgrounds(&backgrounds, Some(&backdrop.file));
        assert!(!backgrounds.join("old.jpg").exists());
        assert!(backgrounds.join(&backdrop.file).exists());
    }

    #[test]
    fn something_that_is_not_a_picture_is_refused() {
        let folder = tempfile::tempdir().unwrap();
        let source = folder.path().join("notes.txt");
        fs::write(&source, "not a picture").unwrap();
        let problem = import_backdrop(&source, folder.path()).unwrap_err();
        assert!(problem.contains("notes.txt"), "{problem}");
    }
}
