//! Picture sizes, coordinates and the hit test. Pure, so it is unit-tested. Ported from version
//! 2's `wackdev/src/geometry.rs`, with the picture budget taken from Claude Desktop.
//!
//! Invariants:
//! - Screen coordinates are global points with a top-left origin, the space Quartz window lists
//!   and Accessibility both report.
//! - A picture's **frame** is the size the model's coordinates refer to: [`budget`] of the window's
//!   size in points. A `scale` below 1 makes the picture smaller, never the frame, so coordinates
//!   mean the same thing at any scale.
//! - Every frame fits [`MAX_EDGE`] and [`MAX_PATCHES`], which keeps a picture inside what providers
//!   take without resizing it (and inside `wack_agent::images::view`'s untouched test): a picture
//!   resized after its frame was recorded is what made version 2's clicks drift.
//! - Coordinates are window-local: the window may move between a picture and an action, but not
//!   change size.

/// The longest edge a picture may have.
pub const MAX_EDGE: u32 = 1568;
/// The side of one image patch a model reads, in pixels.
pub const PATCH: u32 = 28;
/// The most patches one picture may cover (about 1.23 megapixels).
pub const MAX_PATCHES: u32 = 1568;

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    pub fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self { x, y, width, height }
    }

    pub fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.width && y < self.y + self.height
    }

    pub fn center(&self) -> (f64, f64) {
        (self.x + self.width / 2.0, self.y + self.height / 2.0)
    }

    /// Finite, with a positive area: a rect SkyLight can use. A window mid-teardown (or one
    /// that is minimized or off every Space) can report CGRectNull, NaN or zero instead, and
    /// SkyLight asserts, aborting the process, on those.
    pub fn has_area(&self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.width.is_finite() && self.height.is_finite() && self.width > 0.0 && self.height > 0.0
    }

    /// Same frame within a point, allowing for rounding between APIs.
    pub fn approx_eq(&self, other: &Rect) -> bool {
        (self.x - other.x).abs() <= 1.0 && (self.y - other.y).abs() <= 1.0 && self.same_size(other)
    }

    /// Same size within a point; the position may differ.
    pub fn same_size(&self, other: &Rect) -> bool {
        (self.width - other.width).abs() <= 1.0 && (self.height - other.height).abs() <= 1.0
    }

    pub fn intersects(&self, other: &Rect) -> bool {
        self.x < other.x + other.width && other.x < self.x + self.width && self.y < other.y + other.height && other.y < self.y + self.height
    }
}

fn patches(width: u32, height: u32) -> u32 {
    width.div_ceil(PATCH) * height.div_ceil(PATCH)
}

/// The largest picture size with the aspect ratio of `width × height` whose long edge is at most
/// [`MAX_EDGE`] and which covers at most [`MAX_PATCHES`]; never larger than the source.
pub fn budget(width: f64, height: f64) -> (u32, u32) {
    let (width, height) = (if width.is_finite() { width.max(1.0) } else { 1.0 }, if height.is_finite() { height.max(1.0) } else { 1.0 });
    let height_for = |w: u32| ((w as f64) * height / width).round().max(1.0) as u32;
    let fits = |w: u32| {
        let h = height_for(w);
        w.max(h) <= MAX_EDGE && patches(w, h) <= MAX_PATCHES
    };
    let full = width.round().max(1.0) as u32;
    if fits(full) {
        return (full, height_for(full));
    }
    // Patches and the long edge both grow with the width, so the largest width that fits is a
    // binary search away.
    let (mut low, mut high) = (1u32, full);
    while low < high {
        let middle = (low + high).div_ceil(2);
        if fits(middle) { low = middle } else { high = middle - 1 }
    }
    (low, height_for(low))
}

/// The size of a picture taken at `scale` of a `frame`-sized frame: at least one pixel each way.
pub fn scaled(frame: (u32, u32), scale: f64) -> (u32, u32) {
    let scale = if scale.is_finite() { scale.clamp(0.1, 1.0) } else { 1.0 };
    (((frame.0 as f64 * scale).round() as u32).max(1), ((frame.1 as f64 * scale).round() as u32).max(1))
}

/// Maps a point in a picture's frame (`frame` pixels over a `size`-point window) to a point in
/// the window, or `None` outside the picture.
pub fn to_window(size: (f64, f64), frame: (u32, u32), x: f64, y: f64) -> Option<(f64, f64)> {
    let (width, height) = (frame.0 as f64, frame.1 as f64);
    if !x.is_finite() || !y.is_finite() || x < 0.0 || y < 0.0 || x >= width || y >= height {
        return None;
    }
    Some((x / width * size.0, y / height * size.1))
}

/// Maps a point in the window (points) to the picture's frame, rounded to a pixel.
pub fn to_frame(size: (f64, f64), frame: (u32, u32), x: f64, y: f64) -> (i64, i64) {
    let sx = if size.0 > 0.0 { frame.0 as f64 / size.0 } else { 1.0 };
    let sy = if size.1 > 0.0 { frame.1 as f64 / size.1 } else { 1.0 };
    ((x * sx).round() as i64, (y * sy).round() as i64)
}

/// One on-screen window, as the window server lists them (front to back).
#[derive(Debug, Clone, PartialEq)]
pub struct WindowEntry {
    pub number: u32,
    pub pid: i32,
    pub bounds: Rect,
    /// The window server layer: 0 for ordinary app windows, higher for panels, menus, the Dock.
    pub layer: i32,
    /// 0 for windows that are on screen but fully transparent (some utilities keep these).
    pub alpha: f64,
}

impl WindowEntry {
    /// An opaque window on the normal layer.
    pub fn new(number: u32, pid: i32, bounds: Rect) -> Self {
        Self { number, pid, bounds, layer: 0, alpha: 1.0 }
    }
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
        !skip.contains(&window.number) && window.alpha > 0.0 && window.bounds.width > 0.0 && window.bounds.height > 0.0 && window.bounds.contains(point.0, point.1)
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
    fn budget_keeps_small_windows_at_their_size() {
        assert_eq!(budget(800.0, 600.0), (800, 600));
        assert_eq!(budget(0.0, 0.0), (1, 1));
        assert_eq!(budget(f64::NAN, 50.0), (1, 50));
    }

    #[test]
    fn budget_fits_the_edge_and_the_patches() {
        // A full-screen window on a 14-inch MacBook Pro, in points.
        assert_eq!(budget(1512.0, 982.0), (1372, 891));
        for (width, height) in [(1512.0, 982.0), (3024.0, 1964.0), (5120.0, 1440.0), (300.0, 4000.0), (1568.0, 1568.0), (2560.0, 1600.0)] {
            let (w, h) = budget(width, height);
            assert!(w.max(h) <= MAX_EDGE, "{width}×{height} → {w}×{h}");
            assert!(patches(w, h) <= MAX_PATCHES, "{width}×{height} → {w}×{h}");
            // The aspect ratio holds within a pixel of rounding.
            assert!(((w as f64 / h as f64) - width / height).abs() < 0.02 * (width / height).max(1.0), "{width}×{height} → {w}×{h}");
            // And it is the largest such width.
            let next = w + 1;
            let next_height = ((next as f64) * height / width).round() as u32;
            assert!(next.max(next_height) > MAX_EDGE || patches(next, next_height) > MAX_PATCHES || next as f64 > width.round());
        }
    }

    #[test]
    fn a_scaled_picture_never_moves_the_frame() {
        assert_eq!(scaled((1372, 891), 0.5), (686, 446));
        assert_eq!(scaled((1372, 891), 3.0), (1372, 891));
        assert_eq!(scaled((10, 10), 0.0), (1, 1));
        assert_eq!(scaled((100, 100), f64::NAN), (100, 100));
    }

    #[test]
    fn area_rejects_the_degenerate_frames_a_gone_window_reports() {
        assert!(rect(0.0, 0.0, 10.0, 10.0).has_area());
        for frame in [
            rect(f64::INFINITY, f64::INFINITY, f64::INFINITY, f64::INFINITY), // CGRectNull
            rect(f64::NAN, 0.0, 10.0, 10.0),
            rect(0.0, 0.0, 10.0, f64::INFINITY),
            rect(0.0, 0.0, 0.0, 10.0),
            rect(0.0, 0.0, 10.0, 0.0),
            rect(0.0, 0.0, -10.0, 10.0),
        ] {
            assert!(!frame.has_area(), "{frame:?}");
        }
    }

    #[test]
    fn frame_points_map_to_window_points_and_back() {
        let size = (1512.0, 982.0);
        let frame = budget(size.0, size.1);
        assert_eq!(to_window(size, frame, 0.0, 0.0), Some((0.0, 0.0)));
        let (x, y) = to_window(size, frame, 686.0, 445.5).unwrap();
        assert!((x - 756.0).abs() < 0.01 && (y - 491.0).abs() < 0.6);
        assert_eq!(to_frame(size, frame, x, y), (686, 446));
        assert_eq!(to_window(size, frame, frame.0 as f64, 10.0), None);
        assert_eq!(to_window(size, frame, -1.0, 10.0), None);
        assert_eq!(to_window(size, frame, f64::NAN, 10.0), None);
    }

    #[test]
    fn a_moved_window_is_the_same_size() {
        let before = rect(100.0, 100.0, 800.0, 600.0);
        assert!(before.same_size(&rect(400.0, 50.0, 800.5, 599.5)));
        assert!(!before.approx_eq(&rect(400.0, 50.0, 800.0, 600.0)));
        assert!(!before.same_size(&rect(100.0, 100.0, 820.0, 600.0)));
    }

    #[test]
    fn hit_test_names_the_topmost_owner() {
        let describe = |pid: i32| match pid {
            7 => ("Terminal".to_string(), true),
            _ => ("Notes".to_string(), false),
        };
        let target = WindowEntry::new(1, 42, rect(0.0, 0.0, 800.0, 600.0));
        let popover = WindowEntry::new(2, 42, rect(100.0, 100.0, 50.0, 50.0));
        let wackcode = WindowEntry::new(3, 100, rect(500.0, 0.0, 400.0, 400.0));
        let terminal = WindowEntry::new(4, 7, rect(0.0, 500.0, 200.0, 200.0));
        let notes = WindowEntry::new(5, 9, rect(300.0, 300.0, 100.0, 100.0));
        let overlay = WindowEntry::new(6, 100, rect(0.0, 0.0, 800.0, 600.0));
        let mut invisible = WindowEntry::new(8, 9, rect(0.0, 0.0, 50.0, 50.0));
        invisible.alpha = 0.0;
        let windows = vec![overlay.clone(), invisible, popover, wackcode, terminal, notes, target];
        let hit = |x: f64, y: f64| topmost_owner(&windows, (x, y), 42, 100, &[6], describe);
        assert_eq!(hit(10.0, 10.0), HitOwner::Target, "a transparent window is looked through");
        assert_eq!(hit(120.0, 120.0), HitOwner::Target, "the target's own popover");
        assert_eq!(hit(600.0, 100.0), HitOwner::Own);
        assert_eq!(hit(50.0, 550.0), HitOwner::Blocked("Terminal".into()));
        assert_eq!(hit(350.0, 350.0), HitOwner::Other("Notes".into()));
        assert_eq!(hit(2000.0, 2000.0), HitOwner::Nothing);
        // Without skipping it, our own overlay would cover everything.
        assert_eq!(topmost_owner(&windows, (60.0, 60.0), 42, 100, &[], describe), HitOwner::Own);
    }

    #[test]
    fn overlay_exemption_is_a_window_number_not_an_app_exemption() {
        let bounds = rect(0.0, 0.0, 800.0, 600.0);
        let overlay = WindowEntry::new(99, 100, bounds);
        let own_window = WindowEntry::new(3, 100, bounds);
        let blocked = WindowEntry::new(4, 7, bounds);
        let target = WindowEntry::new(1, 42, bounds);
        let describe = |_| ("Terminal".to_string(), true);
        assert_eq!(topmost_owner(&[overlay.clone(), own_window, target.clone()], (10.0, 10.0), 42, 100, &[99], describe), HitOwner::Own);
        assert_eq!(topmost_owner(&[overlay.clone(), blocked, target.clone()], (10.0, 10.0), 42, 100, &[99], describe), HitOwner::Blocked("Terminal".into()));
        assert_eq!(topmost_owner(&[overlay, target], (10.0, 10.0), 42, 100, &[99], describe), HitOwner::Target);
    }
}
