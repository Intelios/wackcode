//! Frosted glass: a pane that softens whatever lies beneath it, then lays its fill over that. The
//! sidebar, the stage and the side panel are glass over an image backdrop.
//!
//! Over a picture the glass is thin and vibrant, as WackCode 2's was: the picture shows through
//! as colour and shape, tinted by the theme. Where the only thing beneath a pane is the picture,
//! the pane paints the picture's softened copy itself ([`Under`]): already blurred on import, its
//! colour boosted (`backdrop::vibrant`), cropped to the pane's rounded shape. That is what gives
//! the glass its colour (GPUI's blur cannot saturate), and it costs less than blurring live. Until
//! that copy is ready, the pane blurs what was painted beneath it instead.
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
    AnyElement, App, BoxShadow, Bounds, Corners, Element, GlobalElementId, Hsla, InspectorElementId, IntoElement, LayoutId,
    Pixels, RenderImage, Window, linear_color_stop, linear_gradient, point, px, quad,
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

/// What a pane lays under itself instead of blurring live: the picture's softened, vibrant copy,
/// fitted to the window exactly as the picture behind it is, and dimmed as the user chose.
#[derive(Clone)]
pub struct Under {
    pub image: Arc<RenderImage>,
    /// Where the whole picture lies, in window coordinates (`backdrop::cover`).
    pub fitted: Bounds<Pixels>,
    /// The user's dim: the window's colour at the veil.
    pub veil: Hsla,
    /// How much more to blur the copy live, for a picture the user made softer than the copy.
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
        if let Some(shadow) = self.lit.and_then(|lit| lit.shadow) {
            window.paint_drop_shadows(
                bounds,
                corners,
                &[BoxShadow { color: shadow, offset: point(px(0.), px(12.)), blur_radius: px(32.), spread_radius: px(-10.), inset: false }],
            );
        }
        if let Some(under) = &self.under {
            window.paint_layer(bounds, |window| {
                let _ = window.paint_image_fitted(bounds, under.fitted, corners, under.image.clone(), 0, false);
            });
        }
        window.paint_layer(bounds, |window| {
            match &self.under {
                Some(under) => {
                    window.paint_quad(quad(bounds, corners, under.veil, px(0.), clear, Default::default()));
                    if under.blur > 0.5 {
                        window.paint_backdrop_blur(bounds, corners, px(under.blur));
                    }
                }
                None if self.blur > 0. => window.paint_backdrop_blur(bounds, corners, px(self.blur)),
                None => {}
            }
            window.paint_quad(quad(bounds, corners, self.fill, px(0.), clear, Default::default()));
            if let Some(lit) = self.lit {
                let sheen = linear_gradient(180., linear_color_stop(lit.sheen, 0.), linear_color_stop(Hsla { a: 0., ..lit.sheen }, 0.36));
                window.paint_quad(quad(bounds, corners, sheen, px(0.), clear, Default::default()));
            }
        });
        window.paint_layer(bounds, |window| {
            if let Some(lit) = self.lit {
                window.paint_inset_shadows(
                    bounds,
                    corners,
                    &[BoxShadow { color: lit.highlight, offset: point(px(0.), px(1.)), blur_radius: px(0.), spread_radius: px(0.), inset: true }],
                );
            }
            window.paint_quad(quad(bounds, corners, clear, px(1.), self.edge, Default::default()));
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
