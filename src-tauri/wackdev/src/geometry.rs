//! Screenshot sizing, screenshot-pixel ↔ screen-point mapping and the foreground hit-test
//! decision. Pure, so it is unit-tested. Screen coordinates are global points with a top-left
//! origin, as Quartz window lists and Accessibility report them.

pub const MAX_LONG_EDGE: u32 = 1280;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    pub fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.width && y < self.y + self.height
    }

    pub fn center(&self) -> (f64, f64) {
        (self.x + self.width / 2.0, self.y + self.height / 2.0)
    }

    /// Same frame within a point, allowing for rounding between APIs.
    pub fn approx_eq(&self, other: &Rect) -> bool {
        (self.x - other.x).abs() <= 1.0
            && (self.y - other.y).abs() <= 1.0
            && (self.width - other.width).abs() <= 1.0
            && (self.height - other.height).abs() <= 1.0
    }
}

/// The capture's pixel size: the window at its display's scale, scaled down (never up) so the
/// long edge is at most `max_long_edge`.
pub fn output_size(frame: &Rect, scale: f64, max_long_edge: u32) -> (u32, u32) {
    let scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
    let width = (frame.width * scale).max(1.0);
    let height = (frame.height * scale).max(1.0);
    let long = width.max(height);
    let factor = if long > max_long_edge as f64 { max_long_edge as f64 / long } else { 1.0 };
    (((width * factor).round() as u32).max(1), ((height * factor).round() as u32).max(1))
}

/// Screenshot pixels per screen point.
pub fn coordinate_scale(frame: &Rect, output_width: u32) -> f64 {
    if frame.width <= 0.0 { 1.0 } else { output_width as f64 / frame.width }
}

/// Maps a pixel of a window screenshot to a global screen point, or `None` when it lies
/// outside the image.
pub fn pixel_to_point(frame: &Rect, output: (u32, u32), x: f64, y: f64) -> Option<(f64, f64)> {
    let (width, height) = (output.0 as f64, output.1 as f64);
    if !x.is_finite() || !y.is_finite() || x < 0.0 || y < 0.0 || x >= width || y >= height {
        return None;
    }
    Some((frame.x + x / width * frame.width, frame.y + y / height * frame.height))
}

/// One on-screen window, as the window server lists them (front to back).
#[derive(Debug, Clone, PartialEq)]
pub struct WindowEntry {
    pub number: u32,
    pub pid: i32,
    pub bounds: Rect,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HitOwner {
    /// The target app's own window is topmost there.
    Target,
    /// WackCode's own window covers the point.
    Own,
    /// A blocked app's window covers the point.
    Blocked(String),
    /// Another app's window covers the point.
    Other(String),
    /// No window is there (desktop, or off every display).
    Nothing,
}

/// Who owns the topmost window at `point`. `skip` lists window numbers to look through (our own
/// click-through overlays); `describe` names another pid's app and says whether it is blocked.
pub fn topmost_owner(
    windows_front_to_back: &[WindowEntry],
    point: (f64, f64),
    target_pid: i32,
    own_pid: i32,
    skip: &[u32],
    describe: impl Fn(i32) -> (String, bool),
) -> HitOwner {
    let hit = windows_front_to_back.iter().find(|window| {
        !skip.contains(&window.number)
            && window.bounds.width > 0.0
            && window.bounds.height > 0.0
            && window.bounds.contains(point.0, point.1)
    });
    match hit {
        None => HitOwner::Nothing,
        Some(window) if window.pid == own_pid => HitOwner::Own,
        Some(window) if window.pid == target_pid => HitOwner::Target,
        Some(window) => {
            let (name, blocked) = describe(window.pid);
            if blocked { HitOwner::Blocked(name) } else { HitOwner::Other(name) }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f64, y: f64, width: f64, height: f64) -> Rect {
        Rect { x, y, width, height }
    }

    #[test]
    fn output_is_scaled_down_to_the_long_edge_and_never_up() {
        assert_eq!(output_size(&rect(0.0, 0.0, 1600.0, 1000.0), 2.0, 1280), (1280, 800));
        assert_eq!(output_size(&rect(0.0, 0.0, 400.0, 300.0), 2.0, 1280), (800, 600));
        assert_eq!(output_size(&rect(0.0, 0.0, 300.0, 2000.0), 1.0, 1280), (192, 1280));
        assert_eq!(output_size(&rect(0.0, 0.0, 0.0, 0.0), 2.0, 1280), (1, 1));
        assert_eq!(output_size(&rect(0.0, 0.0, 100.0, 50.0), f64::NAN, 1280), (100, 50));
    }

    #[test]
    fn pixels_map_back_to_screen_points() {
        let frame = rect(100.0, 50.0, 800.0, 600.0);
        let output = (1280, 960);
        assert_eq!(pixel_to_point(&frame, output, 0.0, 0.0), Some((100.0, 50.0)));
        assert_eq!(pixel_to_point(&frame, output, 640.0, 480.0), Some((500.0, 350.0)));
        assert_eq!(pixel_to_point(&frame, output, 1280.0, 10.0), None);
        assert_eq!(pixel_to_point(&frame, output, -1.0, 10.0), None);
        assert_eq!(pixel_to_point(&frame, output, f64::NAN, 10.0), None);
        assert!((coordinate_scale(&frame, 1280) - 1.6).abs() < 1e-9);
    }

    #[test]
    fn hit_test_names_the_topmost_owner() {
        let describe = |pid: i32| match pid {
            7 => ("Terminal".to_string(), true),
            _ => ("Notes".to_string(), false),
        };
        let target = WindowEntry { number: 1, pid: 42, bounds: rect(0.0, 0.0, 800.0, 600.0) };
        let popover = WindowEntry { number: 2, pid: 42, bounds: rect(100.0, 100.0, 50.0, 50.0) };
        let wackcode = WindowEntry { number: 3, pid: 100, bounds: rect(500.0, 0.0, 400.0, 400.0) };
        let terminal = WindowEntry { number: 4, pid: 7, bounds: rect(0.0, 500.0, 200.0, 200.0) };
        let notes = WindowEntry { number: 5, pid: 9, bounds: rect(300.0, 300.0, 100.0, 100.0) };
        let overlay = WindowEntry { number: 6, pid: 100, bounds: rect(0.0, 0.0, 800.0, 600.0) };
        let windows = vec![overlay.clone(), popover, wackcode, terminal, notes, target];
        let hit = |x: f64, y: f64| topmost_owner(&windows, (x, y), 42, 100, &[6], describe);
        assert_eq!(hit(10.0, 10.0), HitOwner::Target);
        assert_eq!(hit(120.0, 120.0), HitOwner::Target, "the target's own popover");
        assert_eq!(hit(600.0, 100.0), HitOwner::Own);
        assert_eq!(hit(50.0, 550.0), HitOwner::Blocked("Terminal".into()));
        assert_eq!(hit(350.0, 350.0), HitOwner::Other("Notes".into()));
        assert_eq!(hit(2000.0, 2000.0), HitOwner::Nothing);
        // Without skipping it, our own overlay would cover everything.
        assert_eq!(topmost_owner(&windows, (10.0, 10.0), 42, 100, &[], describe), HitOwner::Own);
    }

    #[test]
    fn overlay_exemption_is_a_window_number_not_an_app_exemption() {
        let bounds = rect(0.0, 0.0, 800.0, 600.0);
        let overlay = WindowEntry { number: 99, pid: 100, bounds };
        let own_window = WindowEntry { number: 3, pid: 100, bounds };
        let blocked = WindowEntry { number: 4, pid: 7, bounds };
        let target = WindowEntry { number: 1, pid: 42, bounds };
        let describe = |_| ("Terminal".to_string(), true);
        assert_eq!(topmost_owner(&[overlay.clone(), own_window, target.clone()], (10.0, 10.0), 42, 100, &[99], describe), HitOwner::Own);
        assert_eq!(topmost_owner(&[overlay.clone(), blocked, target.clone()], (10.0, 10.0), 42, 100, &[99], describe), HitOwner::Blocked("Terminal".into()));
        assert_eq!(topmost_owner(&[overlay, target], (10.0, 10.0), 42, 100, &[99], describe), HitOwner::Target);
    }
}
