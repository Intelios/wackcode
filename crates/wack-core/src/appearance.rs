//! How the app looks, as the user chose it: light or dark, one of a fixed set of themes, and an
//! image behind the window. The UI derives every colour from these (`wack-ui/src/theme.rs`, which
//! holds each theme's colours); this module only keeps the choices and prepares the image.
//!
//! Invariants:
//! - A theme is chosen from [`ThemeName::ALL`], never made: no colour is ever stored. A theme
//!   name this build does not know (from a later build, or edited by hand) reads as the default,
//!   so it never stops the settings from loading.
//! - A backdrop is always WackCode's own copy in `backgrounds/`, never the user's original: the
//!   original can move or be deleted without the window losing its picture. The copy is a JPEG
//!   no larger than [`MAX_BACKDROP_EDGE`] on its long side, so a huge photo costs no more to draw
//!   than a screenful.
//! - Beside it is a softened copy ([`soft_name`]): small and blurred once, on import, so the
//!   picture behind a conversation costs nothing to soften while the window draws.
//! - A backdrop carries how dark and light its softened copy gets ([`shade`]), measured on
//!   import (or on the first launch after, for one imported before), so the theme can keep text
//!   readable over it without opening the picture.
//! - Pictures are a library ([`Appearance::pictures`]), at most one of them shown. Each picture
//!   remembers the theme last used with it (`docs/redesign.md`, phase 7): showing a picture brings
//!   its theme back, and choosing a theme while it shows is remembered on it. Change both only
//!   through [`Appearance::show`], [`Appearance::choose_theme`], [`Appearance::add`] and
//!   [`Appearance::remove`], which keep that promise. A theme never shows a picture.
//! - A settings file from before the library (one `backdrop`) opens as a library of that one,
//!   shown.
//! - Copies no picture in the library refers to are deleted when the setting changes.

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
/// The most a backdrop can be dimmed, in percent.
pub const MAX_DIM: u8 = 90;
/// How much a backdrop is dimmed unless the user chooses otherwise. Text sits on glass over the
/// picture, so the picture itself needs little.
pub const DEFAULT_DIM: u8 = 20;
/// The most a backdrop can be blurred, in points.
pub const MAX_BLUR: u8 = 40;
/// How much a backdrop is blurred unless the user chooses otherwise, in points.
pub const DEFAULT_BLUR: u8 = 8;
/// The softened copy's long side, in pixels: it is blurred, so it loses nothing by being small.
const SOFT_EDGE: u32 = 1024;
/// The softened copy's blur, in its own pixels: about 12 points across a laptop's window.
const SOFT_BLUR: f32 = 10.;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
    /// Light or dark as macOS is.
    #[default]
    System,
    Light,
    Dark,
}

/// How the picture is drawn on the new-chat screen: as it is, or through a print-like texture.
/// Behind a conversation it is always softened instead.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Texture {
    #[default]
    None,
    Dither,
    Ascii,
    Halftone,
    Scanlines,
}

impl Texture {
    pub const ALL: [Texture; 5] = [Texture::None, Texture::Dither, Texture::Ascii, Texture::Halftone, Texture::Scanlines];

    pub fn name(self) -> &'static str {
        match self {
            Texture::None => "None",
            Texture::Dither => "Dither",
            Texture::Ascii => "ASCII",
            Texture::Halftone => "Halftone",
            Texture::Scanlines => "Scanlines",
        }
    }
}

/// One of WackCode's themes (`docs/redesign.md`, phase 6): WackCode 2's set, by name. What each
/// looks like is the UI's business.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ThemeName {
    #[default]
    WackCode,
    Midnight,
    Lagoon,
    Verdant,
    Grape,
    Nebula,
    Rose,
    Solar,
    Ember,
    Mono,
}

impl ThemeName {
    pub const ALL: [ThemeName; 10] = [
        ThemeName::WackCode,
        ThemeName::Midnight,
        ThemeName::Lagoon,
        ThemeName::Verdant,
        ThemeName::Grape,
        ThemeName::Nebula,
        ThemeName::Rose,
        ThemeName::Solar,
        ThemeName::Ember,
        ThemeName::Mono,
    ];

    /// How the theme is written in `wackcode.json`.
    pub fn id(self) -> &'static str {
        match self {
            ThemeName::WackCode => "wackcode",
            ThemeName::Midnight => "midnight",
            ThemeName::Lagoon => "lagoon",
            ThemeName::Verdant => "verdant",
            ThemeName::Grape => "grape",
            ThemeName::Nebula => "nebula",
            ThemeName::Rose => "rose",
            ThemeName::Solar => "solar",
            ThemeName::Ember => "ember",
            ThemeName::Mono => "mono",
        }
    }

    /// How the theme is shown.
    pub fn name(self) -> &'static str {
        match self {
            ThemeName::WackCode => "WackCode",
            ThemeName::Midnight => "Midnight",
            ThemeName::Lagoon => "Lagoon",
            ThemeName::Verdant => "Verdant",
            ThemeName::Grape => "Grape",
            ThemeName::Nebula => "Nebula",
            ThemeName::Rose => "Rosé",
            ThemeName::Solar => "Solar",
            ThemeName::Ember => "Ember",
            ThemeName::Mono => "Mono",
        }
    }

    /// The theme written as `id`; the default for anything else.
    pub fn from_id(id: &str) -> ThemeName {
        ThemeName::ALL.into_iter().find(|theme| theme.id() == id).unwrap_or_default()
    }
}

impl Serialize for ThemeName {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.id())
    }
}

impl<'de> Deserialize<'de> for ThemeName {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // Anything but a known name, even something that is not a string, is the default.
        let value = serde_json::Value::deserialize(deserializer)?;
        Ok(value.as_str().map(ThemeName::from_id).unwrap_or_default())
    }
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
    /// How much of the window's colour covers the picture, 0 to [`MAX_DIM`] percent. Drawn
    /// exactly as chosen; the glass over it thickens on its own where text needs it.
    #[serde(default = "default_dim")]
    pub dim: u8,
    /// How soft the picture is, 0 to [`MAX_BLUR`] points.
    #[serde(default = "default_blur")]
    pub blur: u8,
    #[serde(default)]
    pub texture: Texture,
    /// How dark and how light the softened picture gets ([`shade`]), so the theme can keep text
    /// readable over all of it. Measured on import; `None` for a picture not yet measured.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shade: Option<[u8; 2]>,
    /// The theme last used with this picture, brought back when it is shown. `None` for one
    /// that has not been shown since the library began.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<ThemeName>,
}

fn middle() -> f32 {
    0.5
}

fn default_dim() -> u8 {
    DEFAULT_DIM
}

fn default_blur() -> u8 {
    DEFAULT_BLUR
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(from = "Stored")]
pub struct Appearance {
    pub mode: Mode,
    /// The theme in use.
    pub theme: ThemeName,
    /// Every picture the user has added, in the order added.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub pictures: Vec<Backdrop>,
    /// The file of the picture behind the window; `None` for none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shown: Option<String>,
}

/// `Appearance` as written, including the single `backdrop` of the files before the library.
#[derive(Deserialize)]
struct Stored {
    #[serde(default)]
    mode: Mode,
    #[serde(default)]
    theme: ThemeName,
    #[serde(default)]
    pictures: Vec<Backdrop>,
    #[serde(default)]
    shown: Option<String>,
    #[serde(default)]
    backdrop: Option<Backdrop>,
}

impl From<Stored> for Appearance {
    fn from(stored: Stored) -> Self {
        let mut appearance = Appearance { mode: stored.mode, theme: stored.theme, pictures: stored.pictures, shown: stored.shown };
        if let Some(backdrop) = stored.backdrop
            && appearance.pictures.is_empty()
        {
            appearance.shown = Some(backdrop.file.clone());
            appearance.pictures.push(Backdrop { theme: Some(appearance.theme), ..backdrop });
        }
        appearance
    }
}

impl Appearance {
    /// The picture behind the window, if any.
    pub fn backdrop(&self) -> Option<&Backdrop> {
        let shown = self.shown.as_deref()?;
        self.pictures.iter().find(|picture| picture.file == shown)
    }

    pub fn backdrop_mut(&mut self) -> Option<&mut Backdrop> {
        let shown = self.shown.clone()?;
        self.pictures.iter_mut().find(|picture| picture.file == shown)
    }

    /// Uses `theme`, and remembers it on the picture showing.
    pub fn choose_theme(&mut self, theme: ThemeName) {
        self.theme = theme;
        if let Some(picture) = self.backdrop_mut() {
            picture.theme = Some(theme);
        }
    }

    /// Shows the picture `file` (none for `None`), bringing back the theme it remembers, or
    /// remembering the one in use if it has none yet. A file not in the library shows nothing.
    pub fn show(&mut self, file: Option<&str>) {
        self.shown = file.filter(|file| self.pictures.iter().any(|picture| picture.file == *file)).map(str::to_string);
        let theme = self.theme;
        if let Some(picture) = self.backdrop_mut() {
            let remembered = *picture.theme.get_or_insert(theme);
            self.theme = remembered;
        }
    }

    /// Adds a newly imported picture to the library and shows it, in the theme in use. It keeps
    /// the dim, blur and texture of the picture it replaces on screen.
    pub fn add(&mut self, mut picture: Backdrop) {
        if let Some(old) = self.backdrop() {
            (picture.dim, picture.blur, picture.texture) = (old.dim, old.blur, old.texture);
        }
        let file = picture.file.clone();
        picture.theme = None;
        self.pictures.retain(|kept| kept.file != file);
        self.pictures.push(picture);
        self.show(Some(&file));
    }

    /// Takes the picture `file` out of the library. If it was showing, none shows; the theme
    /// stays as it is.
    pub fn remove(&mut self, file: &str) {
        self.pictures.retain(|picture| picture.file != file);
        if self.shown.as_deref() == Some(file) {
            self.shown = None;
        }
    }

    /// The same choices, checked: each picture's focus, dim and blur are kept in range, and a
    /// shown picture not in the library shows nothing.
    pub fn checked(mut self) -> Result<Self, String> {
        if self.backdrop().is_none() {
            self.shown = None;
        }
        for backdrop in &mut self.pictures {
            let unit = |value: f32| if value.is_finite() { value.clamp(0., 1.) } else { 0.5 };
            backdrop.focus_x = unit(backdrop.focus_x);
            backdrop.focus_y = unit(backdrop.focus_y);
            backdrop.dim = backdrop.dim.min(MAX_DIM);
            backdrop.blur = backdrop.blur.min(MAX_BLUR);
            if backdrop.file.contains('/') || backdrop.file.starts_with('.') {
                return Err("That backdrop is not one of WackCode’s.".into());
            }
        }
        Ok(self)
    }
}

/// The softened copy's file name beside a backdrop's `file`.
pub fn soft_name(file: &str) -> String {
    let stem = file.rsplit_once('.').map_or(file, |(stem, _)| stem);
    format!("{stem}-soft.jpg")
}

/// Writes the softened copy of the picture at `path` beside it. Slow: run it off the UI thread.
pub fn soften(path: &Path) -> Result<std::path::PathBuf, String> {
    let picture = image::open(path).map_err(|error| format!("Could not open the backdrop: {error}."))?;
    let file = path.file_name().map(|name| name.to_string_lossy().to_string()).unwrap_or_default();
    let soft = path.with_file_name(soft_name(&file));
    write_jpeg(&soft_copy(&picture), &soft)?;
    Ok(soft)
}

fn soft_copy(picture: &image::DynamicImage) -> image::RgbImage {
    let small = picture.resize(SOFT_EDGE, SOFT_EDGE, FilterType::Triangle).to_rgb8();
    image::imageops::blur(&small, SOFT_BLUR)
}

/// How dark and how light a softened picture gets, as brightness from 0 to 255: the grey of the
/// same luminance below which 2%, and above which 2%, of its pixels lie. Text is kept readable
/// against that range rather than against a stray speck.
pub fn shade(picture: &image::RgbImage) -> [u8; 2] {
    let linear: Vec<f64> = (0..=255u8)
        .map(|value| {
            let value = value as f64 / 255.;
            if value <= 0.04045 { value / 12.92 } else { ((value + 0.055) / 1.055).powf(2.4) }
        })
        .collect();
    let mut luminance: Vec<f64> = picture
        .pixels()
        .map(|pixel| {
            let [r, g, b] = pixel.0;
            0.2126 * linear[r as usize] + 0.7152 * linear[g as usize] + 0.0722 * linear[b as usize]
        })
        .collect();
    if luminance.is_empty() {
        return [0, 255];
    }
    let cut = luminance.len() / 50;
    let last = luminance.len() - 1;
    let dark = *luminance.select_nth_unstable_by(cut, f64::total_cmp).1;
    let light = *luminance.select_nth_unstable_by(last - cut, f64::total_cmp).1;
    let encode = |y: f64| {
        let value = if y <= 0.0031308 { 12.92 * y } else { 1.055 * y.powf(1. / 2.4) - 0.055 };
        (value * 255.).round().clamp(0., 255.) as u8
    };
    [encode(dark), encode(light)]
}

/// [`shade`] for a picture on disk: a backdrop's softened copy.
pub fn measure(path: &Path) -> Result<[u8; 2], String> {
    let picture = image::open(path).map_err(|error| format!("Could not open the backdrop: {error}."))?;
    let picture = if picture.width().max(picture.height()) > SOFT_EDGE {
        picture.resize(SOFT_EDGE, SOFT_EDGE, FilterType::Triangle)
    } else {
        picture
    };
    Ok(shade(&picture.to_rgb8()))
}

fn write_jpeg(picture: &image::RgbImage, path: &Path) -> Result<(), String> {
    let mut encoded = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut encoded, 92)
        .encode_image(picture)
        .map_err(|error| format!("Could not save the backdrop: {error}."))?;
    let temporary = path.with_extension("jpg.tmp");
    fs::write(&temporary, encoded)
        .and_then(|_| fs::rename(&temporary, path))
        .map_err(|error| format!("Could not save the backdrop: {error}."))
}

/// Reads a picture, scales it down to [`MAX_BACKDROP_EDGE`] if larger, and writes WackCode's copy
/// and its softened copy into `backgrounds`. Slow for a large photo: run it off the UI thread.
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
    let soft = soft_copy(&picture);
    let picture = picture.to_rgb8();

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hasher);
    let file = format!("{:016x}.jpg", hasher.finish());
    fs::create_dir_all(backgrounds).map_err(|error| format!("Could not create the backgrounds folder: {error}."))?;
    write_jpeg(&picture, &backgrounds.join(&file))?;
    write_jpeg(&soft, &backgrounds.join(soft_name(&file)))?;
    Ok(Backdrop {
        file,
        width: picture.width(),
        height: picture.height(),
        focus_x: 0.5,
        focus_y: 0.5,
        dim: DEFAULT_DIM,
        blur: DEFAULT_BLUR,
        texture: Texture::None,
        shade: Some(shade(&soft)),
        theme: None,
    })
}

/// Deletes copies in `backgrounds` other than those named in `keep` and their softened copies.
pub fn tidy_backgrounds(backgrounds: &Path, keep: &[&str]) {
    let Ok(entries) = fs::read_dir(backgrounds) else {
        return;
    };
    let soft: Vec<String> = keep.iter().map(|file| soft_name(file)).collect();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !keep.contains(&name.as_str()) && !soft.contains(&name) {
            let _ = fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A theme is stored by name; one this build does not know, and the colours earlier builds
    /// kept, still open as the default.
    #[test]
    fn themes_are_kept_by_name() {
        let chosen = Appearance { theme: ThemeName::Nebula, ..Appearance::default() };
        let written = serde_json::to_string(&chosen).unwrap();
        assert!(written.contains(r#""theme":"nebula""#), "{written}");
        assert_eq!(serde_json::from_str::<Appearance>(&written).unwrap(), chosen);
        let earlier: Appearance = serde_json::from_str(r##"{"mode":"dark","theme_colour":"#6f7a5c","accent":"#c2ee4a"}"##).unwrap();
        assert_eq!((earlier.mode, earlier.theme), (Mode::Dark, ThemeName::WackCode));
        for unknown in [r#"{"theme":"sepia"}"#, r#"{"theme":7}"#, r#"{"theme":null}"#] {
            assert_eq!(serde_json::from_str::<Appearance>(unknown).unwrap().theme, ThemeName::WackCode, "{unknown}");
        }
        for theme in ThemeName::ALL {
            assert_eq!(ThemeName::from_id(theme.id()), theme);
        }
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
        let soft = image::open(backgrounds.join(soft_name(&backdrop.file))).unwrap();
        assert_eq!((soft.width(), soft.height()), (SOFT_EDGE, SOFT_EDGE / 2), "the softened copy is small");
        assert_eq!((backdrop.dim, backdrop.blur), (DEFAULT_DIM, DEFAULT_BLUR));

        let wild = Appearance {
            pictures: vec![Backdrop { focus_x: 7., focus_y: f32::NAN, dim: 200, blur: 99, ..backdrop.clone() }],
            shown: Some(backdrop.file.clone()),
            ..Appearance::default()
        }
        .checked()
        .unwrap();
        let kept = wild.backdrop().unwrap();
        assert_eq!((kept.focus_x, kept.focus_y, kept.dim, kept.blur), (1., 0.5, MAX_DIM, MAX_BLUR));

        fs::write(backgrounds.join("old.jpg"), b"x").unwrap();
        tidy_backgrounds(&backgrounds, &[&backdrop.file]);
        assert!(!backgrounds.join("old.jpg").exists());
        assert!(backgrounds.join(&backdrop.file).exists());
        assert!(backgrounds.join(soft_name(&backdrop.file)).exists(), "the softened copy is kept with it");
    }

    fn picture(file: &str) -> Backdrop {
        Backdrop {
            file: file.into(),
            width: 10,
            height: 10,
            focus_x: 0.5,
            focus_y: 0.5,
            dim: DEFAULT_DIM,
            blur: DEFAULT_BLUR,
            texture: Texture::None,
            shade: None,
            theme: None,
        }
    }

    /// Showing a picture brings back the theme last used with it; choosing a theme while it
    /// shows is remembered on it; a theme never shows a picture.
    #[test]
    fn a_picture_remembers_its_theme() {
        let mut appearance = Appearance { theme: ThemeName::Lagoon, ..Appearance::default() };
        appearance.add(Backdrop { dim: 40, ..picture("a.jpg") });
        assert_eq!(appearance.backdrop().unwrap().theme, Some(ThemeName::Lagoon), "a new picture takes the theme in use");
        appearance.choose_theme(ThemeName::Solar);
        appearance.add(picture("b.jpg"));
        assert_eq!(appearance.backdrop().unwrap().dim, 40, "a new picture keeps the dim of the one it replaces");
        assert_eq!(appearance.theme, ThemeName::Solar);
        appearance.choose_theme(ThemeName::Mono);

        appearance.show(Some("a.jpg"));
        assert_eq!(appearance.theme, ThemeName::Solar, "a's theme comes back");
        appearance.show(Some("b.jpg"));
        assert_eq!(appearance.theme, ThemeName::Mono, "and b's");
        appearance.show(None);
        appearance.choose_theme(ThemeName::Grape);
        assert_eq!(appearance.shown, None, "a theme shows no picture");
        appearance.show(Some("b.jpg"));
        assert_eq!(appearance.theme, ThemeName::Mono, "choosing with no picture showing changes none");
        appearance.show(Some("gone.jpg"));
        assert_eq!((appearance.shown.as_deref(), appearance.theme), (None, ThemeName::Mono));

        appearance.show(Some("a.jpg"));
        appearance.remove("a.jpg");
        assert_eq!((appearance.shown.as_deref(), appearance.theme), (None, ThemeName::Solar), "removing keeps the theme");
        assert_eq!(appearance.pictures.iter().map(|picture| picture.file.as_str()).collect::<Vec<_>>(), ["b.jpg"]);
    }

    /// A file from before the library opens as a library of its one picture, shown, and is
    /// written back in the new shape.
    #[test]
    fn a_single_backdrop_becomes_a_library_of_one() {
        let earlier = r#"{"theme":"nebula","backdrop":{"file":"sky.jpg","width":10,"height":10,"dim":30}}"#;
        let appearance: Appearance = serde_json::from_str(earlier).unwrap();
        assert_eq!(appearance.shown.as_deref(), Some("sky.jpg"));
        let shown = appearance.backdrop().unwrap();
        assert_eq!((shown.dim, shown.theme), (30, Some(ThemeName::Nebula)));
        let written = serde_json::to_string(&appearance).unwrap();
        assert!(!written.contains("backdrop") && written.contains("pictures"), "{written}");
        assert_eq!(serde_json::from_str::<Appearance>(&written).unwrap(), appearance);
    }

    #[test]
    fn a_picture_is_measured_from_its_darkest_to_its_lightest() {
        // Half black, half white, with a speck of the other in each: the specks are ignored.
        let picture = image::RgbImage::from_fn(100, 100, |x, y| {
            let white = (x < 50) != (x == 10 && y == 10);
            image::Rgb(if white { [255; 3] } else { [0; 3] })
        });
        assert_eq!(shade(&picture), [0, 255]);
        let grey = image::RgbImage::from_pixel(10, 10, image::Rgb([119, 119, 119]));
        assert_eq!(shade(&grey), [119, 119], "a grey is measured as itself");
        // Pure green is far darker than pure white, and measured so.
        let green = image::RgbImage::from_pixel(10, 10, image::Rgb([0, 255, 0]));
        let [dark, light] = shade(&green);
        assert_eq!(dark, light);
        assert!((210..=225).contains(&dark), "{dark}");

        let folder = tempfile::tempdir().unwrap();
        let source = folder.path().join("dusk.png");
        image::RgbImage::from_fn(200, 100, |x, _| image::Rgb([x as u8, x as u8, x as u8])).save(&source).unwrap();
        let backdrop = import_backdrop(&source, folder.path()).unwrap();
        let [dark, light] = backdrop.shade.expect("an imported picture is measured");
        assert!(dark < 30 && light > 170, "{dark} to {light}");
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
