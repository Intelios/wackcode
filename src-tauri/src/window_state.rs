//! Remembers the main window's geometry across launches. Resize and move events update
//! `AppData.window` in memory — writing `wackcode.json` on every drag step would churn the
//! disk, so it goes out with the next `MetadataState::mutate`, and always on exit — and
//! `restore` applies it in `setup` while the window is still hidden, before `show()`.
//!
//! Everything is kept in logical points, so a window moved between a Retina display and a 1x
//! external one restores at the same apparent size.

use tauri::{LogicalPosition, LogicalSize, Manager, WebviewWindow, Window, WindowEvent};

use crate::models::WindowState;
use crate::storage::MetadataState;

/// A monitor's bounds in logical points.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Rect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

impl Rect {
    fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x && x < self.x + self.width && y >= self.y && y < self.y + self.height
    }
}

/// Applies `saved` to the still-hidden window: the size clamped to the monitor it lands on,
/// and the saved position reused only while it keeps the window on a connected monitor — a
/// display can be unplugged between launches.
pub fn restore(window: &WebviewWindow, saved: WindowState) {
    let monitors = window
        .available_monitors()
        .unwrap_or_default()
        .iter()
        .map(|monitor| {
            let scale = monitor.scale_factor();
            let origin = monitor.position().to_logical::<f64>(scale);
            let size = monitor.size().to_logical::<f64>(scale);
            Rect { x: origin.x, y: origin.y, width: size.width, height: size.height }
        })
        .collect::<Vec<_>>();
    let (size, position) = fit(saved, &monitors);
    let _ = window.set_size(size);
    if let Some(position) = position {
        let _ = window.set_position(position);
    }
}

/// Folds a resize or move event into `AppData.window`. Skipped while fullscreen — that
/// geometry is the screen's, not the user's.
pub fn record(window: &Window, event: &WindowEvent) {
    let (size, position) = match event {
        WindowEvent::Resized(size) => (Some(*size), None),
        WindowEvent::Moved(position) => (None, Some(*position)),
        _ => return,
    };
    if window.is_fullscreen().unwrap_or(false) {
        return;
    }
    // The hidden window already emits events during `setup`, before the state is managed.
    let Some(state) = window.app_handle().try_state::<MetadataState>() else {
        return;
    };
    let Ok(mut data) = state.data.lock() else {
        return;
    };
    let scale = window.scale_factor().unwrap_or(1.0);
    let entry = data.window.get_or_insert_with(|| snapshot(window, scale));
    if let Some(size) = size {
        let logical = size.to_logical::<f64>(scale);
        if logical.width > 0.0 && logical.height > 0.0 {
            entry.width = logical.width;
            entry.height = logical.height;
        }
    }
    if let Some(position) = position {
        let logical = position.to_logical::<f64>(scale);
        entry.x = Some(logical.x.round() as i32);
        entry.y = Some(logical.y.round() as i32);
    }
}

/// The window's current geometry, for whichever of `Resized`/`Moved` happens to arrive first.
fn snapshot(window: &Window, scale: f64) -> WindowState {
    let size = window
        .inner_size()
        .map(|size| size.to_logical::<f64>(scale))
        .unwrap_or_else(|_| LogicalSize::new(0.0, 0.0));
    let position = window
        .outer_position()
        .map(|position| position.to_logical::<f64>(scale))
        .unwrap_or_else(|_| LogicalPosition::new(0.0, 0.0));
    WindowState {
        width: size.width,
        height: size.height,
        x: Some(position.x.round() as i32),
        y: Some(position.y.round() as i32),
    }
}

/// What `restore` applies: the saved size clamped to the monitor holding the saved top-left
/// corner (or the first monitor when there's no position or it is off-screen now), and the
/// position nudged back inside that monitor. `None` leaves the platform's default placement.
fn fit(saved: WindowState, monitors: &[Rect]) -> (LogicalSize<f64>, Option<LogicalPosition<f64>>) {
    let point = saved.x.zip(saved.y).map(|(x, y)| (x as f64, y as f64));
    let monitor = point
        .and_then(|(x, y)| monitors.iter().find(|monitor| monitor.contains(x, y)))
        .or_else(|| monitors.first());
    let mut size = LogicalSize::new(saved.width.max(1.0), saved.height.max(1.0));
    if let Some(monitor) = monitor {
        size.width = size.width.min(monitor.width);
        size.height = size.height.min(monitor.height);
    }
    let position = match (monitor, point) {
        (Some(monitor), Some((x, y))) => Some(LogicalPosition::new(
            x.clamp(monitor.x, (monitor.x + monitor.width - size.width).max(monitor.x)),
            y.clamp(monitor.y, (monitor.y + monitor.height - size.height).max(monitor.y)),
        )),
        _ => None,
    };
    (size, position)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MONITOR: Rect = Rect { x: 0.0, y: 0.0, width: 1728.0, height: 1117.0 };

    fn saved(width: f64, height: f64, x: Option<i32>, y: Option<i32>) -> WindowState {
        WindowState { width, height, x, y }
    }

    #[test]
    fn geometry_restores_verbatim() {
        let (size, position) = fit(saved(1200.0, 800.0, Some(40), Some(60)), &[MONITOR]);
        assert_eq!((size.width, size.height), (1200.0, 800.0));
        assert_eq!(position.map(|point| (point.x, point.y)), Some((40.0, 60.0)));
    }

    #[test]
    fn position_on_a_disconnected_monitor_is_pulled_onto_a_live_one() {
        // Saved while a second display to the right was connected; only the laptop's remains.
        let (size, position) = fit(saved(1200.0, 800.0, Some(2500), Some(100)), &[MONITOR]);
        assert_eq!((size.width, size.height), (1200.0, 800.0));
        assert_eq!(position.map(|point| (point.x, point.y)), Some((1728.0 - 1200.0, 100.0)));
    }

    #[test]
    fn oversize_geometry_is_clamped_to_the_monitor() {
        let (size, position) = fit(saved(3000.0, 2000.0, Some(-500), Some(5000)), &[MONITOR]);
        assert_eq!((size.width, size.height), (1728.0, 1117.0));
        assert_eq!(position.map(|point| (point.x, point.y)), Some((0.0, 0.0)));
    }

    #[test]
    fn no_saved_position_keeps_the_default_placement() {
        let (size, position) = fit(saved(1000.0, 700.0, None, None), &[MONITOR]);
        assert_eq!((size.width, size.height), (1000.0, 700.0));
        assert!(position.is_none());
    }

    #[test]
    fn without_monitors_nothing_is_clamped_or_moved() {
        let (size, position) = fit(saved(1000.0, 700.0, Some(40), Some(60)), &[]);
        assert_eq!((size.width, size.height), (1000.0, 700.0));
        assert!(position.is_none());
    }

    #[test]
    fn a_second_monitor_is_found_by_its_own_origin() {
        let second = Rect { x: 1728.0, y: 0.0, width: 1920.0, height: 1080.0 };
        let (_, position) = fit(saved(1200.0, 800.0, Some(2000), Some(100)), &[MONITOR, second]);
        assert_eq!(position.map(|point| (point.x, point.y)), Some((2000.0, 100.0)));
        // 1200pt at x=2500 would spill past the second display's right edge (3648).
        let (_, position) = fit(saved(1200.0, 800.0, Some(2500), Some(100)), &[MONITOR, second]);
        assert_eq!(position.map(|point| (point.x, point.y)), Some((2448.0, 100.0)));
    }

    #[test]
    fn nonsensical_sizes_come_back_sane() {
        let (size, _) = fit(saved(0.0, -50.0, Some(40), Some(60)), &[MONITOR]);
        assert_eq!((size.width, size.height), (1.0, 1.0));
    }
}
