//! Colour maths for the theme: sRGB, OKLCH and WCAG contrast.
//!
//! Ported from version 2's `src/theme.ts`. Lightness is Ottosson's toe-corrected `Lr` rather than
//! raw OKLab `L`: raw `L` is compressed near black, so a step of 0.02 above a dark background
//! would be invisible while the same step above a light one would be loud. With `Lr`, equal steps
//! look equal in both appearances, which is what lets one recipe drive light and dark.
//!
//! Pure functions only; `theme.rs` turns the results into GPUI colours.

/// An sRGB colour with channels in 0–1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Srgb {
    pub r: f64,
    pub g: f64,
    pub b: f64,
}

/// OKLCH with toe-corrected lightness (`l` 0–1), chroma (`c`, about 0–0.37) and hue in degrees.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lch {
    pub l: f64,
    pub c: f64,
    pub h: f64,
}

impl Srgb {
    /// `#rrggbb` or `#rgb`, with or without the `#`. `None` for anything else.
    pub fn from_hex(hex: &str) -> Option<Srgb> {
        let digits = hex.trim().trim_start_matches('#');
        let expanded: String = match digits.len() {
            3 => digits.chars().flat_map(|digit| [digit, digit]).collect(),
            6 => digits.to_string(),
            _ => return None,
        };
        let channel = |at: usize| {
            u8::from_str_radix(expanded.get(at..at + 2)?, 16)
                .ok()
                .map(|value| value as f64 / 255.)
        };
        Some(Srgb { r: channel(0)?, g: channel(2)?, b: channel(4)? })
    }

    pub fn to_hex(self) -> String {
        let byte = |value: f64| (value.clamp(0., 1.) * 255.).round() as u8;
        format!("#{:02x}{:02x}{:02x}", byte(self.r), byte(self.g), byte(self.b))
    }

    pub fn to_lch(self) -> Lch {
        let [l, a, b] = linear_to_oklab([to_linear(self.r), to_linear(self.g), to_linear(self.b)]);
        Lch { l: toe(l), c: a.hypot(b), h: b.atan2(a).to_degrees().rem_euclid(360.) }
    }

    /// WCAG 2 relative luminance.
    pub fn luminance(self) -> f64 {
        0.2126 * to_linear(self.r) + 0.7152 * to_linear(self.g) + 0.0722 * to_linear(self.b)
    }
}

impl Lch {
    pub fn new(l: f64, c: f64, h: f64) -> Lch {
        Lch { l, c, h }
    }

    /// The nearest sRGB colour: chroma is lowered, never lightness or hue, until it fits.
    pub fn to_srgb(self) -> Srgb {
        let lightness = untoe(self.l.clamp(0., 1.));
        let (sin, cos) = self.h.to_radians().sin_cos();
        let mut chroma = self.c.max(0.);
        for _ in 0..24 {
            let linear = oklab_to_linear([lightness, chroma * cos, chroma * sin]);
            if linear.iter().all(|value| (-0.0005..=1.0005).contains(value)) {
                return from_linear_rgb(linear);
            }
            chroma *= 0.85;
        }
        from_linear_rgb(oklab_to_linear([lightness, 0., 0.]))
    }

    pub fn with_l(self, l: f64) -> Lch {
        Lch { l, ..self }
    }

    pub fn with_c(self, c: f64) -> Lch {
        Lch { c, ..self }
    }
}

/// WCAG 2 contrast ratio between two opaque colours (1–21).
pub fn contrast(first: Srgb, second: Srgb) -> f64 {
    let (a, b) = (first.luminance(), second.luminance());
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

/// Moves `colour`'s lightness away from `background` until the pair reaches `ratio`, or the end
/// of the scale is reached. Hue and chroma are kept.
pub fn readable_on(colour: Lch, background: Srgb, ratio: f64) -> Lch {
    let darken = background.luminance() > 0.18;
    let mut candidate = colour;
    for _ in 0..100 {
        if contrast(candidate.to_srgb(), background) >= ratio {
            break;
        }
        let next = candidate.l + if darken { -0.01 } else { 0.01 };
        if !(0. ..=1.).contains(&next) {
            break;
        }
        candidate = candidate.with_l(next);
    }
    candidate
}

/// Shortest signed rotation from `from` to `to`, in degrees (−180 to 180).
pub fn angle_delta(from: f64, to: f64) -> f64 {
    (to - from + 540.).rem_euclid(360.) - 180.
}

fn to_linear(value: f64) -> f64 {
    if value <= 0.04045 { value / 12.92 } else { ((value + 0.055) / 1.055).powf(2.4) }
}

fn from_linear(value: f64) -> f64 {
    if value <= 0.0031308 { value * 12.92 } else { 1.055 * value.powf(1. / 2.4) - 0.055 }
}

fn from_linear_rgb([r, g, b]: [f64; 3]) -> Srgb {
    let channel = |value: f64| from_linear(value.clamp(0., 1.));
    Srgb { r: channel(r), g: channel(g), b: channel(b) }
}

fn linear_to_oklab([r, g, b]: [f64; 3]) -> [f64; 3] {
    let l = (0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b).cbrt();
    let m = (0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b).cbrt();
    let s = (0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b).cbrt();
    [
        0.2104542553 * l + 0.7936177850 * m - 0.0040720468 * s,
        1.9779984951 * l - 2.4285922050 * m + 0.4505937099 * s,
        0.0259040371 * l + 0.7827717662 * m - 0.8086757660 * s,
    ]
}

/// Unclamped linear sRGB: a channel outside 0–1 means the colour is out of gamut.
fn oklab_to_linear([lightness, a, b]: [f64; 3]) -> [f64; 3] {
    let l = (lightness + 0.3963377774 * a + 0.2158037573 * b).powi(3);
    let m = (lightness - 0.1055613458 * a - 0.0638541728 * b).powi(3);
    let s = (lightness - 0.0894841775 * a - 1.2914855480 * b).powi(3);
    [
        4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
        -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
        -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s,
    ]
}

const K1: f64 = 0.206;
const K2: f64 = 0.03;
const K3: f64 = (1. + K1) / (1. + K2);

fn toe(l: f64) -> f64 {
    0.5 * (K3 * l - K1 + ((K3 * l - K1).powi(2) + 4. * K2 * K3 * l).sqrt())
}

fn untoe(l: f64) -> f64 {
    (l * l + K1 * l) / (K3 * (l + K2))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trips() {
        for hex in ["#000000", "#ffffff", "#111310", "#c2ee4a", "#ff8fb8", "#6cc4ff"] {
            assert_eq!(Srgb::from_hex(hex).unwrap().to_hex(), hex);
        }
        assert_eq!(Srgb::from_hex("abc").unwrap().to_hex(), "#aabbcc");
        assert_eq!(Srgb::from_hex("#12345"), None);
        assert_eq!(Srgb::from_hex("#gggggg"), None);
    }

    #[test]
    fn oklch_round_trips_within_one_step() {
        for hex in ["#111310", "#c2ee4a", "#e8ece4", "#5dd7af", "#7b8275", "#ff9a5c", "#808080"] {
            let colour = Srgb::from_hex(hex).unwrap();
            let back = colour.to_lch().to_srgb();
            for (a, b) in [(colour.r, back.r), (colour.g, back.g), (colour.b, back.b)] {
                assert!((a - b).abs() * 255. <= 1., "{hex} came back as {}", back.to_hex());
            }
        }
    }

    #[test]
    fn grey_has_no_chroma_and_white_is_full_lightness() {
        let grey = Srgb::from_hex("#808080").unwrap().to_lch();
        assert!(grey.c < 1e-4);
        let white = Srgb::from_hex("#ffffff").unwrap().to_lch();
        assert!((white.l - 1.).abs() < 1e-3);
    }

    #[test]
    fn out_of_gamut_colours_keep_lightness_and_hue() {
        let wild = Lch::new(0.6, 0.5, 140.);
        let fitted = wild.to_srgb().to_lch();
        assert!((fitted.l - 0.6).abs() < 0.01);
        assert!(angle_delta(fitted.h, 140.).abs() < 2.);
        assert!(fitted.c < 0.5);
    }

    #[test]
    fn contrast_matches_wcag() {
        let black = Srgb::from_hex("#000").unwrap();
        let white = Srgb::from_hex("#fff").unwrap();
        assert!((contrast(black, white) - 21.).abs() < 1e-9);
        assert!((contrast(white, white) - 1.).abs() < 1e-9);
    }

    #[test]
    fn readable_on_moves_away_from_the_background() {
        let lime = Srgb::from_hex("#c2ee4a").unwrap().to_lch();
        let paper = Srgb::from_hex("#f7f8f3").unwrap();
        let ink = readable_on(lime, paper, 4.5);
        assert!(ink.l < lime.l);
        assert!(contrast(ink.to_srgb(), paper) >= 4.5);

        let night = Srgb::from_hex("#111310").unwrap();
        let dim = Lch::new(0.3, 0.01, 130.);
        let lifted = readable_on(dim, night, 4.5);
        assert!(lifted.l > dim.l);
        assert!(contrast(lifted.to_srgb(), night) >= 4.5);
    }

    #[test]
    fn angle_delta_takes_the_short_way() {
        assert_eq!(angle_delta(350., 10.), 20.);
        assert_eq!(angle_delta(10., 350.), -20.);
        assert_eq!(angle_delta(0., 180.), -180.);
    }
}
