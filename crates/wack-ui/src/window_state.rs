//! Remembers the main window's geometry across launches, clamped to live monitors.
//!
//! Everything is kept in logical points relative to the window's display's top-left corner,
//! matching GPUI's `Window::bounds`, `PlatformDisplay::visible_bounds` and `WindowOptions` on
//! macOS, so moving between a Retina screen and a 1x external display restores at the same
//! apparent size.
//!
//! Invariants:
//! - Fullscreen geometry is never recorded: exiting or relaunching always uses the last normal
//!   windowed size and position.
//! - A window whose display is temporarily missing (sleep, display reconfiguration) is ignored
//!   rather than recording GPUI's fallback default bounds.
//! - On launch, saved dimensions are clamped between the app's minimum window size and the
//!   target display's visible area (excluding menu bar and Dock), and the top-left corner is
//!   clamped inside that visible area. A disconnected monitor falls back to the primary display.

use std::rc::Rc;

use gpui::{App, Bounds, DisplayId, Pixels, PlatformDisplay, Size, Window, WindowBounds, point, px, size};
use wack_core::WindowState;

use crate::{
    panes::solve::{CONTENT_MIN_HEIGHT, WINDOW_MIN_WIDTH},
    shell::{FRAME_GUTTER, TITLE_STRIP_HEIGHT},
};

/// The default window size on a first launch (before the user has moved or resized the window).
pub const DEFAULT_WIDTH: f32 = 1180.;
pub const DEFAULT_HEIGHT: f32 = 760.;

/// The smallest window the app allows: the pane solver's minimum plus the frame's gutters.
pub fn min_size() -> Size<Pixels> {
    size(
        px(WINDOW_MIN_WIDTH) + FRAME_GUTTER * 2.,
        px(CONTENT_MIN_HEIGHT) + TITLE_STRIP_HEIGHT + FRAME_GUTTER,
    )
}

/// A display-local rectangle in logical points (top-left origin).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Rect {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

/// A connected monitor's identity and usable area (excluding menu bar and Dock).
#[derive(Clone, Debug, PartialEq)]
struct Monitor {
    id: DisplayId,
    uuid: Option<String>,
    visible: Rect,
}

/// Where and how large to open the window, in GPUI's display-local top-left coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Placement {
    display_id: Option<DisplayId>,
    width: f32,
    height: f32,
    origin: Option<(f32, f32)>,
}

/// Computes the `display_id` and `WindowBounds` to hand to `WindowOptions` on launch.
///
/// Reuses the saved display (matched by persistent UUID) and position while clamping size and
/// origin into that display's visible area. When there is no saved state, opens at the default
/// size centred on the primary display (still clamped if the display is smaller).
pub fn restore(saved: Option<&WindowState>, cx: &App) -> (Option<DisplayId>, WindowBounds) {
    let monitors = live_monitors(cx);
    let min = min_size();
    let placement = fit(
        saved,
        &monitors,
        (DEFAULT_WIDTH, DEFAULT_HEIGHT),
        (f32::from(min.width), f32::from(min.height)),
    );
    let window_size = size(px(placement.width), px(placement.height));
    let bounds = match placement.origin {
        Some((x, y)) => Bounds::new(point(px(x), px(y)), window_size),
        None => Bounds::centered(placement.display_id, window_size, cx),
    };
    (placement.display_id, WindowBounds::Windowed(bounds))
}

/// Reads the window's current normal-mode geometry in whole logical points.
///
/// Returns `None` while fullscreen, while AppKit reports no screen (sleep or display
/// reconfiguration), or if the reported size is non-positive.
pub fn snapshot(window: &Window, cx: &App) -> Option<WindowState> {
    if window.is_fullscreen() {
        return None;
    }
    let display = window.display(cx)?;
    let bounds = window.bounds();
    let width = whole(f32::from(bounds.size.width));
    let height = whole(f32::from(bounds.size.height));
    if !width.is_finite() || !height.is_finite() || width <= 0. || height <= 0. {
        return None;
    }
    let x = whole(f32::from(bounds.origin.x));
    let y = whole(f32::from(bounds.origin.y));
    let (x, y) = if x.is_finite() && y.is_finite() { (Some(x), Some(y)) } else { (None, None) };
    Some(WindowState {
        width,
        height,
        x,
        y,
        display: display.uuid().ok().map(|uuid| uuid.to_string()),
    })
}

/// Normalises a logical coordinate to a whole point without `-0.0`.
fn whole(value: f32) -> f32 {
    value.round() + 0.0
}

/// Connected displays with the primary display placed first so fallback always lands on it.
fn live_monitors(cx: &App) -> Vec<Monitor> {
    let mut ordered: Vec<Rc<dyn PlatformDisplay>> = Vec::new();
    if let Some(primary) = cx.primary_display() {
        ordered.push(primary);
    }
    for display in cx.displays() {
        if ordered.iter().all(|existing| existing.id() != display.id()) {
            ordered.push(display);
        }
    }
    ordered.into_iter().map(|display| monitor_from(display.as_ref())).collect()
}

fn monitor_from(display: &dyn PlatformDisplay) -> Monitor {
    let visible = display.visible_bounds();
    let usable = if f32::from(visible.size.width) > 0. && f32::from(visible.size.height) > 0. {
        visible
    } else {
        display.bounds()
    };
    Monitor {
        id: display.id(),
        uuid: display.uuid().ok().map(|uuid| uuid.to_string()),
        visible: Rect {
            x: f32::from(usable.origin.x),
            y: f32::from(usable.origin.y),
            width: f32::from(usable.size.width),
            height: f32::from(usable.size.height),
        },
    }
}

/// Pure placement solver: picks the monitor, clamps the size between `min` and the monitor's
/// visible area, and clamps the saved origin inside that area (or centres when no origin is
/// saved).
fn fit(
    saved: Option<&WindowState>,
    monitors: &[Monitor],
    default: (f32, f32),
    min: (f32, f32),
) -> Placement {
    let monitor = saved
        .and_then(|state| state.display.as_deref())
        .and_then(|uuid| monitors.iter().find(|monitor| monitor.uuid.as_deref() == Some(uuid)))
        .or_else(|| monitors.first());

    let raw_width = match saved {
        Some(state) if state.width.is_finite() => state.width,
        _ => default.0,
    };
    let raw_height = match saved {
        Some(state) if state.height.is_finite() => state.height,
        _ => default.1,
    };
    let mut width = raw_width.max(min.0);
    let mut height = raw_height.max(min.1);
    if let Some(monitor) = monitor {
        if monitor.visible.width > 0. {
            width = width.min(monitor.visible.width);
        }
        if monitor.visible.height > 0. {
            height = height.min(monitor.visible.height);
        }
    }

    let saved_origin = saved
        .and_then(|state| state.x.zip(state.y))
        .filter(|(x, y)| x.is_finite() && y.is_finite());

    let origin = match (monitor, saved_origin) {
        (Some(monitor), Some((x, y))) => {
            let max_x = (monitor.visible.x + monitor.visible.width - width).max(monitor.visible.x);
            let max_y = (monitor.visible.y + monitor.visible.height - height).max(monitor.visible.y);
            Some((x.clamp(monitor.visible.x, max_x), y.clamp(monitor.visible.y, max_y)))
        }
        (Some(monitor), None) => {
            let x = monitor.visible.x + ((monitor.visible.width - width) / 2.).max(0.);
            let y = monitor.visible.y + ((monitor.visible.height - height) / 2.).max(0.);
            Some((whole(x), whole(y)))
        }
        (None, _) => None,
    };

    Placement {
        display_id: monitor.map(|monitor| monitor.id),
        width,
        height,
        origin,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEFAULT: (f32, f32) = (DEFAULT_WIDTH, DEFAULT_HEIGHT);
    const MIN: (f32, f32) = (716., 492.);

    fn monitor(id: u64, uuid: &str, x: f32, y: f32, width: f32, height: f32) -> Monitor {
        Monitor {
            id: DisplayId::new(id),
            uuid: Some(uuid.to_string()),
            visible: Rect { x, y, width, height },
        }
    }

    fn saved(width: f32, height: f32, x: Option<f32>, y: Option<f32>, display: Option<&str>) -> WindowState {
        WindowState { width, height, x, y, display: display.map(str::to_string) }
    }

    #[test]
    fn first_launch_opens_at_default_size_centred_on_primary_monitor() {
        let laptop = monitor(1, "laptop", 0., 38., 1728., 1079.);
        let placement = fit(None, &[laptop], DEFAULT, MIN);
        assert_eq!(placement.display_id, Some(DisplayId::new(1)));
        assert_eq!((placement.width, placement.height), DEFAULT);
        assert_eq!(placement.origin, Some((274., 198.)));
    }

    #[test]
    fn first_launch_clamps_default_size_to_a_small_monitor() {
        let tiny = monitor(1, "tiny", 0., 25., 1024., 640.);
        let placement = fit(None, &[tiny], DEFAULT, MIN);
        assert_eq!((placement.width, placement.height), (1024., 640.));
        assert_eq!(placement.origin, Some((0., 25.)));
    }

    #[test]
    fn geometry_restores_verbatim_on_its_display() {
        let laptop = monitor(1, "laptop", 0., 38., 1728., 1079.);
        let state = saved(1200., 800., Some(40.), Some(60.), Some("laptop"));
        let placement = fit(Some(&state), &[laptop], DEFAULT, MIN);
        assert_eq!(placement.display_id, Some(DisplayId::new(1)));
        assert_eq!((placement.width, placement.height), (1200., 800.));
        assert_eq!(placement.origin, Some((40., 60.)));
    }

    #[test]
    fn second_monitor_is_matched_by_uuid_and_clamped_to_its_visible_area() {
        let laptop = monitor(1, "laptop", 0., 38., 1728., 1079.);
        let external = monitor(2, "external", 0., 25., 1920., 1055.);

        let inside = saved(1200., 800., Some(300.), Some(100.), Some("external"));
        let placement = fit(Some(&inside), &[laptop.clone(), external.clone()], DEFAULT, MIN);
        assert_eq!(placement.display_id, Some(DisplayId::new(2)));
        assert_eq!((placement.width, placement.height), (1200., 800.));
        assert_eq!(placement.origin, Some((300., 100.)));

        // Spilling past the external display's right edge (1920) and above its menu bar (25).
        let spilling = saved(1200., 800., Some(1100.), Some(0.), Some("external"));
        let placement = fit(Some(&spilling), &[laptop, external], DEFAULT, MIN);
        assert_eq!(placement.display_id, Some(DisplayId::new(2)));
        assert_eq!(placement.origin, Some((720., 25.)));
    }

    #[test]
    fn disconnected_monitor_falls_back_to_primary_and_pulls_position_on_screen() {
        let laptop = monitor(1, "laptop", 0., 38., 1728., 1079.);
        let state = saved(1200., 800., Some(2500.), Some(100.), Some("external-gone"));
        let placement = fit(Some(&state), &[laptop], DEFAULT, MIN);
        assert_eq!(placement.display_id, Some(DisplayId::new(1)));
        assert_eq!((placement.width, placement.height), (1200., 800.));
        assert_eq!(placement.origin, Some((1728. - 1200., 100.)));
    }

    #[test]
    fn oversize_geometry_is_clamped_to_the_monitor() {
        let laptop = monitor(1, "laptop", 60., 38., 1668., 1079.);
        let state = saved(3000., 2000., Some(-500.), Some(5000.), Some("laptop"));
        let placement = fit(Some(&state), &[laptop], DEFAULT, MIN);
        assert_eq!((placement.width, placement.height), (1668., 1079.));
        assert_eq!(placement.origin, Some((60., 38.)));
    }

    #[test]
    fn no_saved_position_centres_the_saved_size() {
        let laptop = monitor(1, "laptop", 0., 38., 1728., 1079.);
        let state = saved(1000., 700., None, None, Some("laptop"));
        let placement = fit(Some(&state), &[laptop], DEFAULT, MIN);
        assert_eq!((placement.width, placement.height), (1000., 700.));
        assert_eq!(placement.origin, Some((364., 228.)));
    }

    #[test]
    fn without_monitors_size_is_kept_and_origin_is_deferred() {
        let state = saved(1000., 700., Some(40.), Some(60.), Some("laptop"));
        let placement = fit(Some(&state), &[], DEFAULT, MIN);
        assert_eq!(placement.display_id, None);
        assert_eq!((placement.width, placement.height), (1000., 700.));
        assert_eq!(placement.origin, None);
    }

    #[test]
    fn nonsensical_and_non_finite_values_come_back_sane() {
        let laptop = monitor(1, "laptop", 0., 38., 1728., 1079.);

        let tiny = saved(0., -50., Some(40.), Some(60.), Some("laptop"));
        let placement = fit(Some(&tiny), std::slice::from_ref(&laptop), DEFAULT, MIN);
        assert_eq!((placement.width, placement.height), MIN);
        assert_eq!(placement.origin, Some((40., 60.)));

        let nan = saved(f32::NAN, f32::INFINITY, Some(f32::NAN), Some(100.), None);
        let placement = fit(Some(&nan), &[laptop], DEFAULT, MIN);
        assert_eq!((placement.width, placement.height), DEFAULT);
        assert_eq!(placement.origin, Some((274., 198.)));
    }
}
