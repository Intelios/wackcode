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
pub use recorded::{TagBounds, frame_started, start_frames, stop_frames, tags};

/// Marks the start of a frame, for the automation socket's frame log. Without the `automation`
/// feature it does nothing.
#[cfg(not(feature = "automation"))]
pub fn frame_started() {}

#[cfg(feature = "automation")]
mod recorded {
    use std::{
        cell::RefCell,
        collections::HashMap,
        panic::Location,
        time::{Duration, Instant},
    };

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

    /// While the socket measures frames (`{"cmd":"frames"}`): each frame drawn (when the shell
    /// began rendering it, and how long the main thread spent until the root's paint), and each
    /// tick of the display the window took.
    #[derive(Default)]
    struct FrameLog {
        on: bool,
        started: Option<Instant>,
        draws: Vec<(Instant, Duration)>,
        ticks: Vec<Instant>,
    }

    thread_local! {
        static FRAMES: RefCell<FrameLog> = RefCell::new(FrameLog::default());
    }

    /// The shell has begun rendering a frame.
    pub fn frame_started() {
        FRAMES.with(|log| {
            let mut log = log.borrow_mut();
            if log.on {
                log.started = Some(Instant::now());
            }
        });
    }

    /// Starts logging frames, following the display from `window`.
    pub fn start_frames(window: &mut Window) {
        FRAMES.with(|log| *log.borrow_mut() = FrameLog { on: true, ..FrameLog::default() });
        window.on_next_display_frame(tick);
    }

    /// Stops logging, and hands back the frames drawn and the display's ticks.
    pub fn stop_frames() -> (Vec<(Instant, Duration)>, Vec<Instant>) {
        FRAMES.with(|log| {
            let log = std::mem::take(&mut *log.borrow_mut());
            (log.draws, log.ticks)
        })
    }

    /// A tick of the display, at the start of the window's frame: asks for the next while logging.
    fn tick(window: &mut Window, _: &mut App) {
        let on = FRAMES.with(|log| {
            let mut log = log.borrow_mut();
            if log.on {
                log.ticks.push(Instant::now());
            }
            log.on
        });
        if on {
            window.on_next_display_frame(tick);
        }
    }

    fn frame_finished() {
        FRAMES.with(|log| {
            let mut log = log.borrow_mut();
            if let Some(started) = log.started.take() {
                log.draws.push((started, started.elapsed()));
            }
        });
    }

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
            if self.name.is_none() {
                frame_finished();
            }
        }
    }
}
