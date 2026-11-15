//! Frosted glass: a pane that blurs whatever was painted beneath it, then lays its fill over the
//! blur. The sidebar and the stage are glass over an image backdrop.
//!
//! Invariants:
//! - Only the blur and the pane's own ground share one scene layer. The pane's content is painted
//!   after the layer, in the ordinary order, so a menu or card inside it still stacks as drawn.
//!   (Inside one layer every primitive shares a draw order and is grouped by kind, so a later
//!   quad could fall under an earlier image.)
//! - The blur only sees what was painted before the pane: the picture, never the pane's content.
//!
//! Adapted from Zeron's `frosted` (MIT, github.com/zeronsh/zeron, `crates/ui/src/frost.rs`).

use gpui::{
    AnyElement, App, Bounds, Corners, Element, GlobalElementId, Hsla, InspectorElementId, IntoElement, LayoutId, Pixels,
    Window, px, quad,
};

/// How far the glass blurs what lies under it, in points.
pub const GLASS_BLUR: f32 = 30.;

/// `child` on frosted glass: the picture beneath is blurred inside `radius`-rounded corners, then
/// `fill` and a one-point `edge` are laid over it, and `child` is painted on top. `blur` of zero
/// skips the blur (a solid pane needs none).
pub fn glass(radius: f32, blur: f32, fill: Hsla, edge: Hsla, child: impl IntoElement) -> Glass {
    Glass { radius, blur, fill, edge, child: child.into_any_element() }
}

pub struct Glass {
    radius: f32,
    blur: f32,
    fill: Hsla,
    edge: Hsla,
    child: AnyElement,
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
        window.paint_layer(bounds, |window| {
            if self.blur > 0. {
                window.paint_backdrop_blur(bounds, corners, px(self.blur));
            }
            window.paint_quad(quad(bounds, corners, self.fill, px(1.), self.edge, Default::default()));
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
