//! Content that fades out at its top and bottom edges, rather than being cut off.
//!
//! The fade is the GPU's (`Window::with_edge_fade`, from the zui fork): every glyph, quad and
//! image painted inside is faded by its position, so it works over a solid ground and over an
//! image backdrop alike. A gradient of the window's colour laid on top would paint a band across
//! a picture.

use gpui::{
    AnyElement, App, Bounds, EdgeFade, Element, ElementId, GlobalElementId, InspectorElementId, IntoElement, LayoutId,
    Pixels, Window,
};

/// `child`, fading over `top` points at its top edge and `bottom` points at its bottom edge.
pub fn faded(child: impl IntoElement, top: Pixels, bottom: Pixels) -> Faded {
    Faded { child: child.into_any_element(), top, bottom }
}

pub struct Faded {
    child: AnyElement,
    top: Pixels,
    bottom: Pixels,
}

impl IntoElement for Faded {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for Faded {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
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
        _request_layout: &mut (),
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
        _request_layout: &mut (),
        _prepaint: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        let fade = EdgeFade {
            bounds,
            band: self.top,
            band_top: Some(self.top),
            band_bottom: Some(self.bottom),
            band_left: None,
            band_right: None,
            top: true,
            bottom: true,
            left: false,
            right: false,
        };
        window.with_edge_fade(Some(fade), |window| self.child.paint(window, cx));
    }
}
