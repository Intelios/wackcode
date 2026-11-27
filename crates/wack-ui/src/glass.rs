//! Frosted glass: a pane that softens whatever lies beneath it, then lays its fill over that. The
//! sidebar, the stage and the side panel are glass over an image backdrop.
//!
//! Over a picture the glass is thin and vibrant, as WackCode 2's was: the picture shows through
//! as colour and shape, tinted by the theme, and only as soft as the user's Blur makes it. Where
//! the only thing beneath a pane is the picture, the pane paints a copy of the picture itself
//! ([`Under`]): its colour boosted, cropped to the pane's rounded shape, and blurred as the user
//! chose. That is what gives the glass its colour (GPUI's blur cannot saturate). The copy comes
//! already blurred (`backdrop::baked`), so a pane costs the GPU no blur of its own while it draws;
//! until that is ready, a boosted copy is blurred live (`backdrop::vibrant`), and before even that,
//! the pane blurs what was painted beneath it.
//!
//! A lit pane ([`Lit`]) catches the light: a sheen down from its top, a bright top edge, and a
//! soft shadow beneath.
//!
//! Invariants:
//! - The picture, the ground (dim, blur and fill) and the rim are each their own scene layer,
//!   painted in that order. Inside one layer every primitive shares a draw order and is grouped
//!   by kind, so a quad and an image, or a shadow and a quad, in the same layer could swap.
//! - The pane's content is painted after the layers, in the ordinary order, so a menu or card
//!   inside it still stacks as drawn.
//! - A live blur only sees what was painted before the pane: the picture, never the pane's
//!   content.
//!
//! Adapted from Zeron's `frosted` (MIT, github.com/zeronsh/zeron, `crates/ui/src/frost.rs`).

use std::sync::Arc;

use gpui::{
    AnyElement, App, BoxShadow, Bounds, ContentMask, Corners, Element, GlobalElementId, Hsla, InspectorElementId, IntoElement,
    LayoutId, Pixels, RenderImage, Rgba, Window, linear_color_stop, linear_gradient, point, px, quad,
};

use crate::theme::Theme;

/// How far a pane blurs what lies under it, in points, when it has no softened copy to paint: about
/// as soft as that copy, so a pane looks the same before and after it arrives.
pub const GLASS_BLUR: f32 = 16.;

/// `child` on frosted glass: the picture beneath is softened inside `radius`-rounded corners, then
/// `fill` and a one-point `edge` are laid over it, and `child` is painted on top. `blur` of zero
/// skips the blur (a solid pane needs none).
pub fn glass(radius: f32, blur: f32, fill: Hsla, edge: Hsla, child: impl IntoElement) -> Glass {
    Glass { radius, blur, fill, edge, under: None, lit: None, child: child.into_any_element() }
}

/// What a pane lays under itself instead of blurring what lies beneath: the picture's vibrant copy,
/// fitted to the window exactly as the picture behind it is, and dimmed as the user chose.
#[derive(Clone)]
pub struct Under {
    pub image: Arc<RenderImage>,
    /// Where the whole picture lies, in window coordinates (`backdrop::cover`).
    pub fitted: Bounds<Pixels>,
    /// The user's dim: the window's colour at the veil.
    pub veil: Hsla,
    /// How much to blur the copy live: the user's blur, less what the copy already has.
    pub blur: f32,
}

/// How a pane catches the light.
#[derive(Clone, Copy)]
pub struct Lit {
    /// The sheen's colour at the top; it fades out a third of the way down.
    pub sheen: Hsla,
    /// The one-point line along the top edge.
    pub highlight: Hsla,
    /// The shadow beneath, or none on a pane lying flat.
    pub shadow: Option<Hsla>,
}

impl Lit {
    /// The theme's light, with a shadow when the pane floats over a picture.
    pub fn of(theme: &Theme, floating: bool) -> Lit {
        Lit { sheen: theme.glass_sheen, highlight: theme.glass_highlight, shadow: floating.then_some(theme.glass_shadow) }
    }
}

pub struct Glass {
    radius: f32,
    blur: f32,
    fill: Hsla,
    edge: Hsla,
    under: Option<Under>,
    lit: Option<Lit>,
    child: AnyElement,
}

impl Glass {
    /// Paints `under` beneath the fill instead of blurring live, when there is one.
    pub fn under(mut self, under: Option<Under>) -> Self {
        self.under = under;
        self
    }

    pub fn lit(mut self, lit: Lit) -> Self {
        self.lit = Some(lit);
        self
    }
}

impl Element for Glass {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<gpui::ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        (self.child.request_layout(window, cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.child.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let corners = Corners::all(px(self.radius));
        let clear = Hsla { a: 0., ..self.fill };
        let scale = window.scale_factor();
        // How far in from the pane's box its shape differs from a plain rectangle: the rounded
        // corners, and a pixel of softening.
        let rounding = self.radius + 1.;
        if let Some(shadow) = self.lit.and_then(|lit| lit.shadow) {
            let shadows = [BoxShadow { color: shadow, offset: point(px(0.), px(SHADOW_DROP)), blur_radius: px(SHADOW_BLUR), spread_radius: px(-10.), inset: false }];
            // Over a picture's copy the pane hides its own middle, so the shadow is only worked
            // out round the rim, where it shows.
            let bands = self.under.as_ref().and_then(|_| rim(bounds, rounding, 3. * SHADOW_BLUR + SHADOW_DROP, scale));
            in_bands(window, bands, |window| window.paint_drop_shadows(bounds, corners, &shadows));
        }
        if let Some(under) = &self.under {
            window.paint_layer(bounds, |window| {
                let _ = window.paint_image_fitted(bounds, under.fitted, corners, under.image.clone(), 0, false);
            });
        }
        window.paint_layer(bounds, |window| {
            let fill = |colour: Hsla| quad(bounds, corners, colour, px(0.), clear, Default::default());
            match &self.under {
                Some(under) if under.blur > 0.5 => {
                    window.paint_quad(fill(under.veil));
                    window.paint_backdrop_blur(bounds, corners, px(under.blur));
                    window.paint_quad(fill(self.fill));
                }
                // With nothing between them, the veil and the fill are painted as the one colour
                // the two make together.
                Some(under) => window.paint_quad(fill(laid_over(self.fill, under.veil))),
                None => {
                    if self.blur > 0. {
                        window.paint_backdrop_blur(bounds, corners, px(self.blur));
                    }
                    window.paint_quad(fill(self.fill));
                }
            }
            if let Some(lit) = self.lit {
                // The sheen is clear below SHEEN_DEPTH of the way down, so only the top is worked out.
                let sheen = linear_gradient(180., linear_color_stop(lit.sheen, 0.), linear_color_stop(Hsla { a: 0., ..lit.sheen }, SHEEN_DEPTH));
                let depth = f32::from(bounds.size.height) * SHEEN_DEPTH;
                window.with_content_mask(Some(ContentMask { bounds: top_band(bounds, depth, scale) }), |window| {
                    window.paint_quad(quad(bounds, corners, sheen, px(0.), clear, Default::default()));
                });
            }
        });
        window.paint_layer(bounds, |window| {
            if let Some(lit) = self.lit {
                // The highlight is a line along the top that bends down round the top corners.
                window.with_content_mask(Some(ContentMask { bounds: top_band(bounds, rounding + 1., scale) }), |window| {
                    window.paint_inset_shadows(
                        bounds,
                        corners,
                        &[BoxShadow { color: lit.highlight, offset: point(px(0.), px(1.)), blur_radius: px(0.), spread_radius: px(0.), inset: true }],
                    );
                });
            }
            in_bands(window, rim(bounds, rounding, 0., scale), |window| {
                window.paint_quad(quad(bounds, corners, clear, px(1.), self.edge, Default::default()));
            });
        });
        self.child.paint(window, cx);
    }
}

impl IntoElement for Glass {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

/// How far a lit pane's shadow falls below it, and how soft it is, in points.
const SHADOW_DROP: f32 = 12.;
const SHADOW_BLUR: f32 = 32.;

/// How far down a pane its sheen reaches, as a share of its height.
const SHEEN_DEPTH: f32 = 0.36;

/// `top` laid over `under`, as one colour: painting it once looks as painting `under` and then
/// `top` does, over any ground.
fn laid_over(top: Hsla, under: Hsla) -> Hsla {
    let (top_rgb, under_rgb) = (Rgba::from(top), Rgba::from(under));
    let under_share = under.a * (1. - top.a);
    let alpha = top.a + under_share;
    if alpha <= 0. {
        return Hsla { a: 0., ..top };
    }
    let mix = |over: f32, beneath: f32| (over * top.a + beneath * under_share) / alpha;
    Rgba { r: mix(top_rgb.r, under_rgb.r), g: mix(top_rgb.g, under_rgb.g), b: mix(top_rgb.b, under_rgb.b), a: alpha }.into()
}

/// The top `depth` points of `bounds`, its foot moved down to a whole device pixel.
fn top_band(bounds: Bounds<Pixels>, depth: f32, scale: f32) -> Bounds<Pixels> {
    let top = f32::from(bounds.top());
    let foot = (((top + depth) * scale).ceil() / scale).min(f32::from(bounds.bottom()));
    Bounds::from_corners(bounds.origin, point(bounds.right(), px(foot)))
}

/// The rim of `bounds`, reaching `inside` points in from its edges and `outside` points out, as
/// four bands that never overlap. Their edges lie on whole device pixels, so no pixel is in two.
/// `None` when the inner edges would meet: then the rim is the whole of it.
fn rim(bounds: Bounds<Pixels>, inside: f32, outside: f32, scale: f32) -> Option<[Bounds<Pixels>; 4]> {
    let snap = |value: f32| px((value * scale).round() / scale);
    let (left, top, right, bottom) = (f32::from(bounds.left()), f32::from(bounds.top()), f32::from(bounds.right()), f32::from(bounds.bottom()));
    if 2. * inside >= (right - left).min(bottom - top) {
        return None;
    }
    let (out_left, out_top, out_right, out_bottom) = (snap(left - outside), snap(top - outside), snap(right + outside), snap(bottom + outside));
    let (in_left, in_top, in_right, in_bottom) = (snap(left + inside), snap(top + inside), snap(right - inside), snap(bottom - inside));
    let band = |left, top, right, bottom| Bounds::from_corners(point(left, top), point(right, bottom));
    Some([
        band(out_left, out_top, out_right, in_top),
        band(out_left, in_bottom, out_right, out_bottom),
        band(out_left, in_top, in_left, in_bottom),
        band(in_right, in_top, out_right, in_bottom),
    ])
}

/// Runs `paint` once inside each of `bands`, or once unclipped when there are none.
fn in_bands(window: &mut Window, bands: Option<[Bounds<Pixels>; 4]>, mut paint: impl FnMut(&mut Window)) {
    match bands {
        Some(bands) => {
            for band in bands {
                window.with_content_mask(Some(ContentMask { bounds: band }), |window| paint(window));
            }
        }
        None => paint(window),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::size;

    /// `colour` painted over an opaque `ground`, as the GPU blends it.
    fn painted(colour: Hsla, ground: [f32; 3]) -> [f32; 3] {
        let rgb = Rgba::from(colour);
        let a = colour.a;
        [rgb.r * a + ground[0] * (1. - a), rgb.g * a + ground[1] * (1. - a), rgb.b * a + ground[2] * (1. - a)]
    }

    #[test]
    fn the_veil_and_fill_laid_together_paint_as_the_two_do() {
        let veils = [Hsla { h: 0.62, s: 0.2, l: 0.08, a: 0.15 }, Hsla { h: 0., s: 0., l: 0.97, a: 0.4 }];
        let fills = [Hsla { h: 0.6, s: 0.3, l: 0.12, a: 0.55 }, Hsla { h: 0.1, s: 0.9, l: 0.6, a: 0. }, Hsla { h: 0.3, s: 0.5, l: 0.5, a: 1. }];
        let grounds = [[0., 0., 0.], [1., 1., 1.], [0.8, 0.3, 0.1]];
        for veil in veils {
            for fill in fills {
                for ground in grounds {
                    let both = painted(fill, painted(veil, ground));
                    let once = painted(laid_over(fill, veil), ground);
                    for (a, b) in both.iter().zip(once) {
                        assert!((a - b).abs() < 1e-4, "{veil:?} then {fill:?} over {ground:?}: {both:?} against {once:?}");
                    }
                }
            }
        }
    }

    fn area(bounds: Bounds<Pixels>) -> f32 {
        f32::from(bounds.size.width).max(0.) * f32::from(bounds.size.height).max(0.)
    }

    #[test]
    fn the_rim_covers_the_edge_once_and_skips_the_middle() {
        let pane = Bounds::new(point(px(262.5), px(44.)), size(px(1412.), px(998.)));
        let bands = rim(pane, 15., 108., 2.).expect("a big pane has a middle");
        for band in bands {
            for edge in [band.left(), band.top(), band.right(), band.bottom()] {
                assert_eq!(f32::from(edge) * 2., (f32::from(edge) * 2.).round(), "{band:?} is off the pixel grid");
            }
        }
        for (index, band) in bands.iter().enumerate() {
            for other in &bands[index + 1..] {
                assert_eq!(area(band.intersect(other)), 0., "{band:?} overlaps {other:?}");
            }
        }
        let outer = Bounds::from_corners(bands[0].origin, bands[1].bottom_right());
        let middle = Bounds::from_corners(point(bands[2].right(), bands[0].bottom()), point(bands[3].left(), bands[1].top()));
        assert_eq!(bands.iter().map(|band| area(*band)).sum::<f32>() + area(middle), area(outer));
        assert!(middle.contains(&pane.center()));
        assert!(outer.contains(&pane.origin) && outer.contains(&pane.bottom_right()));
    }

    #[test]
    fn a_pill_is_all_rim() {
        assert!(rim(Bounds::new(point(px(0.), px(0.)), size(px(226.), px(34.))), 18., 0., 2.).is_none());
    }

    #[test]
    fn a_top_band_ends_on_a_pixel_inside_the_pane() {
        let pane = Bounds::new(point(px(10.), px(44.)), size(px(200.), px(100.)));
        let band = top_band(pane, 36.2, 2.);
        assert_eq!(band.top(), pane.top());
        assert_eq!(f32::from(band.bottom()), 80.5);
        assert_eq!(top_band(pane, 500., 2.).bottom(), pane.bottom());
    }
}
