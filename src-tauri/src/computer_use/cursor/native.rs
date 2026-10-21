//! AppKit objects live exclusively in main-thread TLS. The panel is inert both to input and
//! Accessibility, and its exact CG window number is the sole input hit-test exemption.
//! The timer exists only while a marker exists; window-list reads are capped at 10 Hz.
//!
//! The panel is ordered directly above its target window, on that window's layer, so other
//! windows cover it exactly where they cover the target. Activating the target app raises its
//! windows over the panel, so every window-list read checks the stacking and restores it. If
//! the window server won't take the order after a few tries, the marker falls back to status
//! level, shown only while nothing else covers its point.

use super::{
    appkit_point, layout, placement, stacked_above, uncovered, Cursor, CursorAppearance, Kind,
    Marker, Placement, HEIGHT, WIDTH,
};
use crate::computer_use::geometry::{Rect, WindowEntry};
use crate::computer_use::input;
use block2::RcBlock;
use objc2::{
    define_class, msg_send,
    rc::Retained,
    runtime::{AnyObject, ProtocolObject},
    DefinedClass, MainThreadMarker, MainThreadOnly,
};
use objc2_app_kit::{
    NSBackingStoreType, NSBezierPath, NSColor, NSFont, NSFontAttributeName, NSFontWeightRegular,
    NSFontWeightSemibold, NSForegroundColorAttributeName, NSGraphicsContext, NSLineJoinStyle,
    NSPanel, NSScreen, NSShadow, NSStringDrawing, NSView, NSWindowCollectionBehavior,
    NSWindowOrderingMode, NSWindowSharingType, NSWindowStyleMask, NSWorkspace,
};
use objc2_foundation::{
    NSArray, NSDictionary, NSNotification, NSNotificationCenter, NSObjectProtocol,
    NSOperationQueue, NSPoint, NSRect, NSRunLoop, NSRunLoopCommonModes, NSSize, NSString, NSTimer,
};
use std::cell::RefCell;
use std::f64::consts::{FRAC_PI_2, FRAC_PI_4, PI, TAU};
use std::ptr::NonNull;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

static WINDOW_NUMBER: AtomicU32 = AtomicU32::new(0);
pub(super) fn window_number() -> Option<u32> {
    match WINDOW_NUMBER.load(Ordering::SeqCst) {
        0 => None,
        n => Some(n),
    }
}

/// NSStatusWindowLevel: above app windows and the menu bar, below system shields.
const STATUS_LEVEL: isize = 25;
/// Failed stacking attempts before a marker falls back to status level.
const STACK_TRIES: u32 = 3;
const APPEAR: f64 = 0.18;
/// Seconds after landing: beads collapse into the tip, then the ping ring spreads.
const COLLAPSE: f64 = 0.12;
const PING: f64 = 0.5;
const TRAIL: usize = 4;
const TRAIL_STEP: f64 = 0.035;
const TRAIL_FADE: f64 = 0.1;

#[derive(Clone, PartialEq)]
struct Drawing {
    appearance: CursorAppearance,
    tip: (f64, f64),
    mirror_x: bool,
    mirror_y: bool,
    label: &'static str,
    resting: bool,
    /// Fade-in, 0–1.
    opacity: f64,
    /// Arrow scale: the fade-in's grow and a press's dip.
    scale: f64,
    /// A scroll's downward nudge, in points.
    nudge: f64,
    /// Trail beads as offsets from the tip, with their fade.
    trail: Vec<(f64, f64, f64)>,
    /// Click beads converging on the tip, 0–1; negative when none.
    collapse: f64,
    /// Click ping ring, 0–1; negative when none.
    ping: f64,
    /// Reduced motion's still ring for a click.
    flash: bool,
    /// Phase of the pill's orbiting beads; `None` draws a still dot.
    orbit: Option<f64>,
    /// Debug frames only: a backdrop to judge contrast against.
    backdrop: Option<String>,
}

define_class!(
    #[unsafe(super = NSView)]
    #[thread_kind = MainThreadOnly]
    #[name = "WackCodeAgentCursorView"]
    #[ivars = RefCell<Option<Drawing>>]
    struct CursorView;

    impl CursorView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool { true }
        #[unsafe(method(isAccessibilityElement))]
        fn is_accessibility_element(&self) -> bool { false }
        #[unsafe(method(accessibilityHitTest:))]
        fn accessibility_hit_test(&self, _point: NSPoint) -> *mut AnyObject { std::ptr::null_mut() }
        #[unsafe(method_id(accessibilityChildren))]
        fn accessibility_children(&self) -> Retained<NSArray<AnyObject>> { NSArray::new() }
        #[unsafe(method(drawRect:))]
        fn draw(&self, _rect: NSRect) {
            if let Some(drawing) = self.ivars().borrow().as_ref() { draw(drawing); }
        }
    }
);

define_class!(
    #[unsafe(super = NSPanel)]
    #[thread_kind = MainThreadOnly]
    #[name = "WackCodeAgentCursorPanel"]
    struct CursorPanel;
    impl CursorPanel {
        #[unsafe(method(canBecomeKeyWindow))]
        fn can_become_key(&self) -> bool { false }
        #[unsafe(method(canBecomeMainWindow))]
        fn can_become_main(&self) -> bool { false }
        #[unsafe(method(isAccessibilityElement))]
        fn is_accessibility_element(&self) -> bool { false }
        #[unsafe(method(accessibilityHitTest:))]
        fn accessibility_hit_test(&self, _point: NSPoint) -> *mut AnyObject { std::ptr::null_mut() }
        #[unsafe(method_id(accessibilityChildren))]
        fn accessibility_children(&self) -> Retained<NSArray<AnyObject>> { NSArray::new() }
    }
);

/// How the panel is being stacked for the current anchor window.
#[derive(Default)]
struct Stacking {
    window: Option<u32>,
    failures: u32,
    fallback: bool,
}

struct Native {
    panel: Retained<CursorPanel>,
    view: Retained<CursorView>,
    timer: Option<Retained<NSTimer>>,
    timer_interval: f64,
    // Notifications cannot retain an AppKit object in a Send block. They call back into TLS.
    observers: Vec<(
        Retained<NSNotificationCenter>,
        Retained<ProtocolObject<dyn NSObjectProtocol>>,
    )>,
    windows: Vec<WindowEntry>,
    last_check: Option<Instant>,
    suspended: bool,
    visibility_changed: bool,
    stacking: Stacking,
    reason: String,
}

thread_local! { static NATIVE: RefCell<Option<Native>> = const { RefCell::new(None) }; }

fn panel(mtm: MainThreadMarker) -> Option<(Retained<CursorPanel>, Retained<CursorView>)> {
    let size = NSSize::new(WIDTH, HEIGHT);
    // Initializers may return nil. Losing decoration must never lose a computer action.
    let panel: Option<Retained<CursorPanel>> = unsafe {
        msg_send![CursorPanel::alloc(mtm), initWithContentRect: NSRect::new(NSPoint::new(0.0, 0.0), size),
            styleMask: NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel,
            backing: NSBackingStoreType::Buffered, defer: false]
    };
    let panel = panel?;
    unsafe {
        panel.setReleasedWhenClosed(false);
    }
    panel.setOpaque(false);
    panel.setBackgroundColor(Some(&NSColor::clearColor()));
    panel.setHasShadow(false);
    panel.setIgnoresMouseEvents(true);
    panel.setHidesOnDeactivate(false);
    panel.setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::FullScreenAuxiliary
            | NSWindowCollectionBehavior::IgnoresCycle,
    );
    panel.setSharingType(if debug::capturable() {
        NSWindowSharingType::ReadOnly
    } else {
        NSWindowSharingType::None
    });
    let allocated_view = CursorView::alloc(mtm).set_ivars(RefCell::new(None));
    let view: Option<Retained<CursorView>> = unsafe {
        msg_send![super(allocated_view), initWithFrame: NSRect::new(NSPoint::new(0.0, 0.0), size)]
    };
    let view = view?;
    panel.setContentView(Some(&view));
    Some((panel, view))
}

impl Native {
    fn new(mtm: MainThreadMarker, cursor: &Cursor) -> Option<Self> {
        let (panel, view) = panel(mtm)?;
        WINDOW_NUMBER.store(
            u32::try_from(panel.windowNumber()).unwrap_or(0),
            Ordering::SeqCst,
        );

        let mut native = Self {
            panel,
            view,
            timer: None,
            timer_interval: 0.0,
            observers: Vec::new(),
            windows: Vec::new(),
            last_check: None,
            suspended: false,
            visibility_changed: false,
            stacking: Stacking::default(),
            reason: String::new(),
        };
        let workspace = NSWorkspace::sharedWorkspace();
        let workspace_center = workspace.notificationCenter();
        let default_center = NSNotificationCenter::defaultCenter();
        for (center, name, suspend) in [
            (
                &workspace_center,
                "NSWorkspaceActiveSpaceDidChangeNotification",
                None,
            ),
            (
                &workspace_center,
                "NSWorkspaceSessionDidResignActiveNotification",
                Some(true),
            ),
            (
                &workspace_center,
                "NSWorkspaceSessionDidBecomeActiveNotification",
                Some(false),
            ),
            (
                &default_center,
                "NSApplicationDidChangeScreenParametersNotification",
                None,
            ),
            (
                &workspace_center,
                "NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification",
                None,
            ),
        ] {
            let cursor = cursor.clone();
            let block = RcBlock::new(move |_notification: NonNull<NSNotification>| {
                NATIVE.with(|slot| {
                    if let Some(native) = slot.borrow_mut().as_mut() {
                        native.panel.orderOut(None);
                        native.visibility_changed = true;
                        if let Some(suspended) = suspend {
                            native.suspended = suspended;
                        }
                        // The next regular check refreshes Space/display visibility; keep the
                        // rate limit even when multiple notifications arrive together.
                    }
                });
                cursor.wake();
            });
            let observer = unsafe {
                center.addObserverForName_object_queue_usingBlock(
                    Some(&NSString::from_str(name)),
                    None,
                    Some(&NSOperationQueue::mainQueue()),
                    &block,
                )
            };
            native.observers.push((center.clone(), observer));
        }
        Some(native)
    }

    fn hide(&mut self) {
        self.panel.orderOut(None);
        if let Some(timer) = self.timer.take() {
            timer.invalidate();
        }
        *self.view.ivars().borrow_mut() = None;
        self.stacking = Stacking::default();
        self.log("hidden: no marker");
    }

    /// Hides the panel for now, keeping the marker and its timer.
    fn conceal(&mut self, cursor: &Cursor, reason: &str) {
        self.ensure_timer(cursor, 0.1);
        self.panel.orderOut(None);
        self.log(reason);
    }

    fn log(&mut self, reason: &str) {
        if self.reason != reason {
            self.reason = reason.to_string();
            debug::log(reason);
        }
    }

    fn ensure_timer(&mut self, cursor: &Cursor, interval: f64) {
        if self.timer.is_some() && self.timer_interval == interval {
            return;
        }
        if let Some(timer) = self.timer.take() {
            timer.invalidate();
        }
        self.timer_interval = interval;
        let cursor = cursor.clone();
        let block = RcBlock::new(move |_timer: NonNull<NSTimer>| refresh(&cursor));
        let timer = unsafe { NSTimer::timerWithTimeInterval_repeats_block(interval, true, &block) };
        unsafe {
            NSRunLoop::mainRunLoop().addTimer_forMode(&timer, NSRunLoopCommonModes);
        }
        self.timer = Some(timer);
    }

    /// Puts the panel directly above `window`. `fresh` means `self.windows` was just read, so
    /// the stacking it shows is current. Returns false once the marker should fall back.
    fn stack(&mut self, window: u32, layer: i32, pid: i32, fresh: bool) -> bool {
        if self.stacking.window != Some(window) {
            self.stacking = Stacking {
                window: Some(window),
                ..Stacking::default()
            };
        }
        if self.stacking.fallback {
            return false;
        }
        let visible = self.panel.isVisible();
        let stacked = window_number()
            .is_some_and(|panel| stacked_above(&self.windows, panel, window, pid));
        if visible && (!fresh || stacked) {
            if fresh {
                self.stacking.failures = 0;
            }
            return true;
        }
        if visible {
            self.stacking.failures += 1;
            if self.stacking.failures >= STACK_TRIES {
                self.stacking.fallback = true;
                self.log(&format!("fallback: couldn't stay above window {window}"));
                return false;
            }
        }
        self.panel.setLevel(layer as isize);
        self.panel
            .orderWindow_relativeTo(NSWindowOrderingMode::Above, window as isize);
        self.store_number();
        self.log(&format!("shown: above window {window} on layer {layer}"));
        true
    }

    /// Status level, for the menu bar and for markers whose stacking failed.
    fn float(&mut self, reason: &str) {
        if self.panel.level() != STATUS_LEVEL {
            self.panel.setLevel(STATUS_LEVEL);
        }
        if !self.panel.isVisible() {
            self.panel.orderFrontRegardless();
            self.store_number();
        }
        self.log(reason);
    }

    fn store_number(&self) {
        WINDOW_NUMBER.store(
            u32::try_from(self.panel.windowNumber()).unwrap_or(0),
            Ordering::SeqCst,
        );
    }
}

impl Drop for Native {
    fn drop(&mut self) {
        self.hide();
        for (center, observer) in &self.observers {
            unsafe {
                center.removeObserver((**observer).as_ref());
            }
        }
        self.panel.close();
        WINDOW_NUMBER.store(0, Ordering::SeqCst);
    }
}

/// Retires the marker if it is still the one this tick read.
fn retire(cursor: &Cursor, marker: &Marker) {
    if let Ok(mut state) = cursor.0.state.lock() {
        if state
            .marker
            .as_ref()
            .is_some_and(|current| current.sequence == marker.sequence)
        {
            state.marker = None;
            state.sequence += 1;
        }
    }
}

pub(super) fn refresh(cursor: &Cursor) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let snapshot = cursor.0.state.lock().ok().and_then(|mut state| {
        if state.marker.as_ref().is_some_and(|marker| {
            marker.token.is_stopped()
                || state.runs.get(&marker.task) != Some(&marker.generation)
                || state.sequence != marker.sequence
        }) {
            state.marker = None;
        }
        let appearance = state
            .appearance
            .clone()
            .unwrap_or_else(CursorAppearance::fallback);
        Some((state.marker.clone()?, appearance))
    });
    NATIVE.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some((mut marker, appearance)) = snapshot else {
            if let Some(native) = slot.as_mut() {
                native.hide();
            }
            return;
        };
        if slot.is_none() {
            *slot = Native::new(mtm, cursor);
        }
        let Some(native) = slot.as_mut() else { return };
        if native.timer.is_none() {
            native.ensure_timer(cursor, 0.1);
        }
        let now = Instant::now();
        let fresh = native
            .last_check
            .is_none_or(|last| now.duration_since(last) >= Duration::from_millis(100));
        if fresh {
            native.windows = input::window_list();
            native.last_check = Some(now);
            native.visibility_changed = false;
        }
        let mut place = placement(&marker.target, &native.windows);
        if let Placement::Moved { offset, frame } = place {
            if let Ok(mut state) = cursor.0.state.lock() {
                if let Some(current) = state
                    .marker
                    .as_mut()
                    .filter(|current| current.sequence == marker.sequence)
                {
                    current.shift(offset, frame);
                }
            }
            marker.shift(offset, frame);
            place = placement(&marker.target, &native.windows);
        }
        match place {
            Placement::Gone | Placement::Moved { .. } => {
                retire(cursor, &marker);
                native.hide();
                return;
            }
            Placement::Hidden => {
                let window = marker.target.anchor.map(|anchor| anchor.window).unwrap_or(0);
                return native.conceal(cursor, &format!("hidden: window {window} isn't on screen"));
            }
            _ => {}
        }
        if native.visibility_changed {
            return native.conceal(cursor, "hidden: Space or display changing");
        }
        if native.suspended || input::screen_locked() {
            return native.conceal(cursor, "hidden: session inactive or screen locked");
        }

        let screens = NSScreen::screens(mtm);
        let Some(primary) = screens.firstObject() else {
            return native.conceal(cursor, "hidden: no screens");
        };
        let primary = primary.frame();
        let reduced = NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion();
        let tip_global = marker.tip(now, reduced);
        let point = appkit_point(tip_global, primary.origin.y + primary.size.height);
        let Some(screen) = screens.iter().map(|screen| screen.frame()).find(|frame| {
            point.0 >= frame.origin.x
                && point.0 < frame.origin.x + frame.size.width
                && point.1 >= frame.origin.y
                && point.1 < frame.origin.y + frame.size.height
        }) else {
            return native.conceal(cursor, "hidden: point is off every display");
        };
        let (frame, tip, mirror_x, mirror_y) = layout(
            point,
            Rect {
                x: screen.origin.x,
                y: screen.origin.y,
                width: screen.size.width,
                height: screen.size.height,
            },
        );
        let (drawing, interval) = animate(&marker, appearance, now, reduced, tip, mirror_x, mirror_y);
        native.ensure_timer(cursor, interval);

        let frame = NSRect::new(NSPoint::new(frame.x, frame.y), NSSize::new(WIDTH, HEIGHT));
        if native.panel.frame() != frame {
            native.panel.setFrame_display(frame, false);
        }
        if native.view.ivars().borrow().as_ref() != Some(&drawing) {
            *native.view.ivars().borrow_mut() = Some(drawing);
            native.view.setNeedsDisplay(true);
        }

        match place {
            Placement::Window { number, layer } => {
                if !native.stack(number, layer, marker.target.pid, fresh) {
                    let skip = super::overlay_windows();
                    if uncovered(&marker.target, &native.windows, &skip) {
                        native.float("fallback: shown at status level, point uncovered");
                    } else {
                        native.conceal(cursor, "fallback: hidden, point covered");
                    }
                }
            }
            _ => native.float("shown: menu bar at status level"),
        }
    });
}

/// The drawing for this tick, and the timer interval its motion needs.
fn animate(
    marker: &Marker,
    appearance: CursorAppearance,
    now: Instant,
    reduced: bool,
    tip: (f64, f64),
    mirror_x: bool,
    mirror_y: bool,
) -> (Drawing, f64) {
    let secs = |from: Instant| now.duration_since(from).as_secs_f64();
    let resting = marker
        .completed
        .is_some_and(|end| now.duration_since(end) >= Duration::from_millis(600));
    // Time since this action reached its point: the later of the landing and its start.
    let acted = marker.since_landing(now, reduced).min(secs(marker.started));
    let appear = if reduced { 1.0 } else { (secs(marker.shown) / APPEAR).min(1.0) };
    let opacity = ease_out(appear);
    let mut scale = 0.6 + 0.4 * ease_out(appear);
    let mut nudge = 0.0;
    let (mut collapse, mut ping, mut flash) = (-1.0, -1.0, false);
    let mut busy = appear < 1.0;
    if acted >= 0.0 {
        match marker.kind {
            Kind::Clicking if reduced => flash = acted < 0.3,
            Kind::Clicking => {
                if acted < COLLAPSE {
                    collapse = acted / COLLAPSE;
                }
                if (COLLAPSE * 0.6..PING).contains(&acted) {
                    ping = (acted - COLLAPSE * 0.6) / (PING - COLLAPSE * 0.6);
                }
                busy |= acted < PING;
            }
            Kind::Pressing if !reduced && acted < 0.2 => {
                scale *= 1.0 - 0.1 * (PI * acted / 0.2).sin();
                busy = true;
            }
            Kind::Scrolling if !reduced && acted < 0.24 => {
                nudge = 3.0 * (PI * acted / 0.24).sin();
                busy = true;
            }
            _ => {}
        }
    }
    let mut trail = Vec::new();
    let moved = secs(marker.moved);
    if let (Some(from), false) = (marker.from, reduced) {
        if moved < super::GLIDE + TRAIL_FADE {
            busy = true;
            let fade = 1.0 - ((moved - super::GLIDE) / TRAIL_FADE).clamp(0.0, 1.0);
            let head = super::glide(Some(from), marker.target.point, moved, false);
            for step in 1..=TRAIL {
                let at = moved - step as f64 * TRAIL_STEP;
                if at < 0.0 {
                    break;
                }
                let bead = super::glide(Some(from), marker.target.point, at, false);
                trail.push((bead.0 - head.0, bead.1 - head.1, fade));
            }
        }
    }
    let orbit = (!resting && !reduced).then(|| secs(marker.started) / 1.2 * TAU);
    let drawing = Drawing {
        appearance,
        tip,
        mirror_x,
        mirror_y,
        label: marker.kind.label(resting),
        resting,
        opacity,
        scale,
        nudge,
        trail,
        collapse,
        ping,
        flash,
        orbit,
        backdrop: None,
    };
    let interval = if busy {
        1.0 / 60.0
    } else if orbit.is_some() {
        1.0 / 30.0
    } else {
        0.1
    };
    (drawing, interval)
}

fn ease_out(t: f64) -> f64 {
    super::ease_out(t)
}

fn colour(hex: &str, alpha: f64) -> Retained<NSColor> {
    let (r, g, b) = crate::glass::parse_hex(hex).unwrap_or((0, 0, 0));
    NSColor::colorWithSRGBRed_green_blue_alpha(
        r as f64 / 255.0,
        g as f64 / 255.0,
        b as f64 / 255.0,
        alpha,
    )
}

fn text_attributes(hex: &str, alpha: f64, size: f64, semibold: bool) -> Retained<NSDictionary<NSString, AnyObject>> {
    let weight = unsafe {
        if semibold {
            NSFontWeightSemibold
        } else {
            NSFontWeightRegular
        }
    };
    let font: Retained<AnyObject> = NSFont::systemFontOfSize_weight(size, weight).into();
    let colour: Retained<AnyObject> = colour(hex, alpha).into();
    unsafe {
        NSDictionary::from_slices(
            &[NSFontAttributeName, NSForegroundColorAttributeName],
            &[&*font, &*colour],
        )
    }
}

fn circle(centre: (f64, f64), radius: f64) -> Retained<NSBezierPath> {
    NSBezierPath::bezierPathWithOvalInRect(NSRect::new(
        NSPoint::new(centre.0 - radius, centre.1 - radius),
        NSSize::new(radius * 2.0, radius * 2.0),
    ))
}

/// Draws `body` with a soft drop shadow, restoring the graphics state afterwards.
fn shadowed(blur: f64, alpha: f64, body: impl FnOnce()) {
    NSGraphicsContext::saveGraphicsState_class();
    let shadow = NSShadow::new();
    shadow.setShadowBlurRadius(blur);
    // Shadow offsets ignore the view's flip: negative y falls downward on screen.
    shadow.setShadowOffset(NSSize::new(0.0, -1.5));
    shadow.setShadowColor(Some(&NSColor::colorWithSRGBRed_green_blue_alpha(
        0.0, 0.0, 0.0, alpha,
    )));
    shadow.set();
    body();
    NSGraphicsContext::restoreGraphicsState_class();
}

/// The pointer outline, tip at the origin, pointing up and left as the system cursor does.
const ARROW_SCALE: f64 = 1.2;
const ARROW: [(f64, f64); 7] = [
    (0.0, 0.0),
    (0.0, 17.0),
    (4.5, 13.0),
    (7.9, 20.4),
    (11.0, 19.0),
    (7.7, 11.8),
    (13.6, 11.6),
];

fn draw(d: &Drawing) {
    let accent = &d.appearance.accent;
    if let Some(backdrop) = &d.backdrop {
        colour(backdrop, 1.0).setFill();
        NSBezierPath::fillRect(NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(WIDTH, HEIGHT)));
    }
    let fade = d.opacity * if d.resting { 0.8 } else { 1.0 };
    let (x, y) = d.tip;

    // The trail: the send comet's beads, shrinking and fading behind the tip.
    for (index, (dx, dy, bead_fade)) in d.trail.iter().enumerate() {
        let step = index as f64;
        colour(accent, fade * bead_fade * (0.75 - 0.15 * step)).setFill();
        circle((x + dx, y + dy), 3.4 - 0.6 * step).fill();
    }
    // A click: beads close in on the tip, then a ping ring spreads from it.
    if d.collapse >= 0.0 {
        let radius = 14.0 * (1.0 - d.collapse * d.collapse);
        colour(accent, fade * (0.9 - 0.4 * d.collapse)).setFill();
        for quarter in 0..4 {
            let angle = FRAC_PI_4 + quarter as f64 * FRAC_PI_2;
            circle((x + radius * angle.cos(), y + radius * angle.sin()), 2.4).fill();
        }
    }
    if d.ping >= 0.0 || d.flash {
        let (radius, width, alpha) = if d.flash {
            (12.0, 1.5, 0.6)
        } else {
            let p = ease_out(d.ping);
            (5.0 + 18.0 * p, 2.4 * (1.0 - d.ping) + 0.6, 0.85 * (1.0 - d.ping))
        };
        let ring = circle((x, y), radius);
        colour(accent, fade * alpha).setStroke();
        ring.setLineWidth(width);
        ring.stroke();
    }

    let sx = if d.mirror_x { -1.0 } else { 1.0 };
    let sy = if d.mirror_y { -1.0 } else { 1.0 };
    let arrow = NSBezierPath::bezierPath();
    for (i, (px, py)) in ARROW.iter().enumerate() {
        let (px, py) = (px * ARROW_SCALE * d.scale, py * ARROW_SCALE * d.scale);
        let point = NSPoint::new(x + px * sx, y + d.nudge + py * sy);
        if i == 0 {
            arrow.moveToPoint(point);
        } else {
            arrow.lineToPoint(point);
        }
    }
    arrow.closePath();
    arrow.setLineJoinStyle(NSLineJoinStyle::Round);
    // A contrasting edge and a soft shadow keep it readable on whatever the app paints.
    shadowed(4.0, 0.35 * fade, || {
        colour(&d.appearance.outline, fade).setStroke();
        arrow.setLineWidth(3.0);
        arrow.stroke();
    });
    colour(accent, fade).setFill();
    arrow.fill();

    draw_pill(d, fade);
}

fn draw_pill(d: &Drawing, fade: f64) {
    const SIZE: f64 = 10.5;
    const PILL: f64 = 22.0;
    const MARK: f64 = 10.0;
    let (x, y) = d.tip;
    let name_attrs = text_attributes(&d.appearance.text, fade, SIZE, true);
    let action_attrs = text_attributes(&d.appearance.muted_text, fade, SIZE, false);
    let mut name = d.appearance.agent_name.chars().take(80).collect::<String>();
    if d.appearance.agent_name.chars().count() > 80 {
        name.push('…');
    }
    while unsafe { NSString::from_str(&name).sizeWithAttributes(Some(&name_attrs)) }.width > 90.0 {
        let mut chars = name.trim_end_matches('…').chars().collect::<Vec<_>>();
        if chars.is_empty() {
            break;
        }
        chars.pop();
        name = chars.into_iter().collect::<String>() + "…";
    }
    let name = NSString::from_str(&name);
    let label = NSString::from_str(&format!(" · {}", d.label));
    let name_size = unsafe { name.sizeWithAttributes(Some(&name_attrs)) };
    let action_size = unsafe { label.sizeWithAttributes(Some(&action_attrs)) };
    let width = 9.0 + MARK + 6.0 + name_size.width + action_size.width + 11.0;
    let px = (if d.mirror_x { x - width - 20.0 } else { x + 20.0 }).clamp(3.0, WIDTH - width - 3.0);
    let py = (if d.mirror_y { y - PILL - 14.0 } else { y + 14.0 }).clamp(3.0, HEIGHT - PILL - 5.0);
    let pill = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(
        NSRect::new(NSPoint::new(px, py), NSSize::new(width, PILL)),
        PILL / 2.0,
        PILL / 2.0,
    );
    // Content surface: solid, so the label reads over any app.
    shadowed(6.0, 0.28 * fade, || {
        colour(&d.appearance.surface, fade).setFill();
        pill.fill();
    });
    colour(&d.appearance.text, fade * if d.resting { 0.1 } else { 0.16 }).setStroke();
    pill.setLineWidth(0.75);
    pill.stroke();

    // The working mark: three beads orbiting while active, one still dot once settled.
    let centre = (px + 9.0 + MARK / 2.0, py + PILL / 2.0);
    match d.orbit {
        Some(phase) => {
            for bead in 0..3 {
                let angle = phase + bead as f64 * TAU / 3.0;
                colour(&d.appearance.accent, fade * (1.0 - 0.25 * bead as f64)).setFill();
                circle((centre.0 + 3.2 * angle.cos(), centre.1 + 3.2 * angle.sin()), 1.6).fill();
            }
        }
        None => {
            colour(&d.appearance.accent, fade * if d.resting { 0.7 } else { 1.0 }).setFill();
            circle(centre, 2.6).fill();
        }
    }
    unsafe {
        let text_x = px + 9.0 + MARK + 6.0;
        let baseline = py + (PILL - name_size.height.max(action_size.height)) / 2.0;
        name.drawAtPoint_withAttributes(NSPoint::new(text_x, baseline), Some(&name_attrs));
        label.drawAtPoint_withAttributes(
            NSPoint::new(text_x + name_size.width, baseline),
            Some(&action_attrs),
        );
    }
}

/// Development aids for a window screenshots can't see. All of it compiles to nothing in
/// release builds.
mod debug {
    #[cfg(debug_assertions)]
    fn flag(name: &str) -> Option<String> {
        std::env::var(name).ok().filter(|value| !value.is_empty())
    }

    /// `WACKCODE_CURSOR_CAPTURABLE=1`: let display screenshots include the panel.
    pub fn capturable() -> bool {
        #[cfg(debug_assertions)]
        return flag("WACKCODE_CURSOR_CAPTURABLE").is_some();
        #[cfg(not(debug_assertions))]
        false
    }

    /// `WACKCODE_CURSOR_DEBUG=1`: log each show/hide decision with its reason.
    pub fn log(_reason: &str) {
        #[cfg(debug_assertions)]
        if flag("WACKCODE_CURSOR_DEBUG").is_some() {
            eprintln!("[agent-cursor] {_reason}");
        }
    }

    #[cfg(debug_assertions)]
    pub fn frames_dir() -> Option<std::path::PathBuf> {
        flag("WACKCODE_CURSOR_FRAMES").map(Into::into)
    }
}

/// `WACKCODE_CURSOR_FRAMES=<dir>` (debug builds): renders every cursor state to PNGs, over
/// light and dark backdrops, in the given appearance and two contrasting ones.
#[cfg(debug_assertions)]
pub(super) fn render_frames(appearance: CursorAppearance) {
    use objc2_app_kit::NSBitmapImageFileType;
    let Some(dir) = debug::frames_dir() else { return };
    let Some(mtm) = MainThreadMarker::new() else { return };
    let Some((_panel, view)) = panel(mtm) else { return };
    let _ = std::fs::create_dir_all(&dir);
    let light = CursorAppearance {
        agent_name: "Nova".into(),
        accent: "#2f6bff".into(),
        outline: "#ffffff".into(),
        surface: "#ffffff".into(),
        text: "#1a1c22".into(),
        muted_text: "#6a6f7a".into(),
    };
    let pink = CursorAppearance {
        agent_name: "A very long agent name indeed".into(),
        accent: "#ff5fa2".into(),
        outline: "#14090f".into(),
        surface: "#211a1e".into(),
        text: "#f4e9ef".into(),
        muted_text: "#a8949e".into(),
    };
    let tip = (super::MARGIN, super::MARGIN);
    let base = Drawing {
        appearance: appearance.clone(),
        tip,
        mirror_x: false,
        mirror_y: false,
        label: Kind::Clicking.label(false),
        resting: false,
        opacity: 1.0,
        scale: 1.0,
        nudge: 0.0,
        trail: Vec::new(),
        collapse: -1.0,
        ping: -1.0,
        flash: false,
        orbit: Some(0.6),
        backdrop: None,
    };
    let trail = (1..=TRAIL)
        .map(|step| (-7.0 * step as f64, -5.0 * step as f64, 1.0))
        .collect::<Vec<_>>();
    let states: Vec<(&str, Drawing)> = vec![
        ("active", base.clone()),
        ("appear", Drawing { opacity: 0.5, scale: 0.8, ..base.clone() }),
        ("glide", Drawing { trail, ..base.clone() }),
        ("collapse", Drawing { collapse: 0.5, ..base.clone() }),
        ("ping-early", Drawing { ping: 0.2, ..base.clone() }),
        ("ping-late", Drawing { ping: 0.65, ..base.clone() }),
        ("press", Drawing { label: Kind::Pressing.label(false), scale: 0.9, ..base.clone() }),
        ("typing", Drawing { label: Kind::Typing.label(false), orbit: Some(2.4), ..base.clone() }),
        ("settled", Drawing { label: Kind::Clicking.label(true), resting: true, orbit: None, ..base.clone() }),
        ("reduced-click", Drawing { flash: true, orbit: None, ..base.clone() }),
        (
            "mirrored",
            Drawing {
                tip: (WIDTH - super::MARGIN, HEIGHT - super::MARGIN),
                mirror_x: true,
                mirror_y: true,
                label: Kind::Menu.label(false),
                ..base.clone()
            },
        ),
    ];
    for (theme, appearance) in [("current", appearance), ("light", light), ("pink", pink)] {
        for (backdrop_name, backdrop) in [("on-dark", "#202124"), ("on-light", "#f2f2f2")] {
            for (state, drawing) in &states {
                let drawing = Drawing {
                    appearance: appearance.clone(),
                    backdrop: Some(backdrop.into()),
                    ..drawing.clone()
                };
                *view.ivars().borrow_mut() = Some(drawing);
                let bounds = view.bounds();
                let Some(bitmap) = view.bitmapImageRepForCachingDisplayInRect(bounds) else {
                    continue;
                };
                view.cacheDisplayInRect_toBitmapImageRep(bounds, &bitmap);
                let png = unsafe {
                    bitmap.representationUsingType_properties(
                        NSBitmapImageFileType::PNG,
                        &NSDictionary::new(),
                    )
                };
                if let Some(png) = png {
                    let path = dir.join(format!("{theme}-{backdrop_name}-{state}.png"));
                    let _ = std::fs::write(path, png.to_vec());
                }
            }
        }
    }
    debug::log(&format!("rendered frames to {}", dir.display()));
}
