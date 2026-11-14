//! Names for elements, so tests and agents can find them on screen.
//!
//! `.tag("composer")` at the end of an element's builder chain records where that element was
//! drawn. The gallery prints a scene's tags, and the dev app's automation socket uses them to aim
//! clicks. Without the `automation` feature a tag costs nothing: the element is returned as it is.
//!
//! Tags are recorded when an element is laid out for a frame, and each frame starts empty (see
//! [`root`]). A subtree GPUI reuses from a previous frame without laying it out again would lose
//! its tags; nothing in the app caches views that way today.

use gpui::IntoElement;

pub trait Tag: IntoElement + Sized {
    /// Names this element for tests. Call it last: the result takes no further styling.
    #[cfg(feature = "automation")]
    fn tag(self, name: &'static str) -> impl IntoElement {
        recorded::Tagged::new(Some(name), self.into_any_element())
    }

    /// Names this element for tests. Call it last: the result takes no further styling.
    #[cfg(not(feature = "automation"))]
    fn tag(self, _name: &'static str) -> impl IntoElement {
        self
    }
}

impl<E: IntoElement> Tag for E {}

/// Wraps a window's root element so every frame starts with an empty tag list.
#[cfg(feature = "automation")]
pub fn root(child: impl IntoElement) -> impl IntoElement {
    recorded::Tagged::new(None, child.into_any_element())
}

/// Wraps a window's root element so every frame starts with an empty tag list.
#[cfg(not(feature = "automation"))]
pub fn root(child: impl IntoElement) -> impl IntoElement {
    child
}

#[cfg(feature = "automation")]
pub use recorded::{TagBounds, tags};

#[cfg(feature = "automation")]
mod recorded {
    use std::{collections::HashMap, panic::Location};

    use gpui::{
        AnyElement, App, Bounds, Element, ElementId, Global, GlobalElementId, InspectorElementId,
        IntoElement, LayoutId, Pixels, Window, WindowId,
    };

    #[derive(Clone, Debug)]
    pub struct TagBounds {
        pub name: &'static str,
        pub bounds: Bounds<Pixels>,
    }

    #[derive(Default)]
    struct Registry(HashMap<WindowId, Vec<TagBounds>>);

    impl Global for Registry {}

    /// The tags drawn in `window`'s latest frame, in drawing order.
    pub fn tags(window: &Window, cx: &App) -> Vec<TagBounds> {
        cx.try_global::<Registry>()
            .and_then(|registry| registry.0.get(&window.window_handle().window_id()).cloned())
            .unwrap_or_default()
    }

    /// Draws `child` unchanged. With a name it records the child's bounds; without one it is the
    /// window's root and clears the previous frame's tags.
    pub struct Tagged {
        name: Option<&'static str>,
        child: AnyElement,
    }

    impl Tagged {
        pub fn new(name: Option<&'static str>, child: AnyElement) -> Self {
            Self { name, child }
        }
    }

    impl IntoElement for Tagged {
        type Element = Self;

        fn into_element(self) -> Self {
            self
        }
    }

    impl Element for Tagged {
        type RequestLayoutState = ();
        type PrepaintState = ();

        fn id(&self) -> Option<ElementId> {
            None
        }

        fn source_location(&self) -> Option<&'static Location<'static>> {
            None
        }

        fn request_layout(
            &mut self,
            _: Option<&GlobalElementId>,
            _: Option<&InspectorElementId>,
            window: &mut Window,
            cx: &mut App,
        ) -> (LayoutId, ()) {
            (self.child.request_layout(window, cx), ())
        }

        fn prepaint(
            &mut self,
            _: Option<&GlobalElementId>,
            _: Option<&InspectorElementId>,
            bounds: Bounds<Pixels>,
            _: &mut (),
            window: &mut Window,
            cx: &mut App,
        ) {
            let window_id = window.window_handle().window_id();
            match self.name {
                Some(name) => {
                    self.child.prepaint(window, cx);
                    let tags = cx
                        .default_global::<Registry>()
                        .0
                        .entry(window_id)
                        .or_default();
                    tags.push(TagBounds { name, bounds });
                }
                None => {
                    cx.default_global::<Registry>().0.remove(&window_id);
                    self.child.prepaint(window, cx);
                }
            }
        }

        fn paint(
            &mut self,
            _: Option<&GlobalElementId>,
            _: Option<&InspectorElementId>,
            _: Bounds<Pixels>,
            _: &mut (),
            _: &mut (),
            window: &mut Window,
            cx: &mut App,
        ) {
            self.child.paint(window, cx);
        }
    }
}
