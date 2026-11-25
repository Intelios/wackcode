//! The agent's cursor on screen: a borderless panel that draws the marker. Ported from
//! version 2's `computer_use/cursor/native.rs`.
//!
//! Invariants:
//! - AppKit objects live only in main-thread storage. The panel takes no input, is invisible
//!   to Accessibility, and is never captured (its sharing type is none); its window number is
//!   the one exemption hit tests make ([`window_number`]).
//! - The panel is ordered directly above its target window, on that window's layer, so other
//!   windows cover it exactly where they cover the target. An app coming forward raises its
//!   windows over the panel, so every window-list read checks the stacking and restores it. If
//!   the window server won't take the order after a few tries, the marker falls back to status
//!   level, shown only while nothing else covers its point.
//! - The timer exists only while a marker does; window-list reads are capped at 10 a second.

use std::{
    cell::RefCell,
    f64::consts::{FRAC_PI_2, FRAC_PI_4, PI, TAU},
    ptr::NonNull,
    sync::atomic::{AtomicBool, AtomicU32, Ordering},
    time::{Duration, Instant},
};

use block2::RcBlock;
use objc2::{
    DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send,
    rc::Retained,
    runtime::{AnyObject, ProtocolObject},
};
use objc2_app_kit::{
    NSBackingStoreType, NSBezierPath, NSColor, NSFont, NSFontAttributeName, NSFontWeightRegular, NSFontWeightSemibold,
    NSForegroundColorAttributeName, NSGraphicsContext, NSLineJoinStyle, NSPanel, NSScreen, NSShadow, NSStringDrawing, NSView,
    NSWindowCollectionBehavior, NSWindowOrderingMode, NSWindowSharingType, NSWindowStyleMask, NSWorkspace,
};
use objc2_foundation::{
    NSArray, NSDictionary, NSNotification, NSNotificationCenter, NSObjectProtocol, NSOperationQueue, NSPoint, NSRect,
    NSRunLoop, NSRunLoopCommonModes, NSSize, NSString, NSTimer,
};

use super::input;
use crate::{
    cursor::{self, Appearance, Cursor, HEIGHT, Kind, Marker, Placement, Rgb, WIDTH},
    geometry::{Rect, WindowEntry},
};

static WINDOW_NUMBER: AtomicU32 = AtomicU32::new(0);

/// The panel's window number while it exists: hit tests look through it.
pub fn window_number() -> Option<u32> {
    match WINDOW_NUMBER.load(Ordering::SeqCst) {
        0 => None,
        number => Some(number),
    }
}

/// A cursor that draws on screen. Its updates come from any thread and are drawn on the main one.
pub fn cursor() -> Cursor {
    Cursor::with_wake(|cursor| {
        let cursor = cursor.clone();
        // The callback reads the current state, not this moment's, so a stop wins even when
        // drawing was already queued.
        super::main_thread::post(move || {
            debug::render_frames_once(&cursor);
            refresh(&cursor);
        });
    })
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
/// The name on the label.
const NAME: &str = "WackCode";

#[derive(Clone, PartialEq)]
struct Drawing {
    appearance: Appearance,
    tip: (f64, f64),
    mirror_x: bool,
    mirror_y: bool,
    label: String,
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
    /// Phase of the label's orbiting beads; `None` draws a still dot.
    orbit: Option<f64>,
    /// Debug frames only: a ground to judge contrast against.
    backdrop: Option<Rgb>,
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
        fn draw_rect(&self, _rect: NSRect) {
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
    // Notification blocks can't hold AppKit objects (they must be Send): they reach back
    // through main-thread storage.
    observers: Vec<Observer>,
    windows: Vec<WindowEntry>,
    last_check: Option<Instant>,
    suspended: bool,
    visibility_changed: bool,
    stacking: Stacking,
    reason: String,
}

type Observer = (Retained<NSNotificationCenter>, Retained<ProtocolObject<dyn NSObjectProtocol>>);

thread_local! { static NATIVE: RefCell<Option<Native>> = const { RefCell::new(None) }; }

fn panel(mtm: MainThreadMarker) -> Option<(Retained<CursorPanel>, Retained<CursorView>)> {
    let size = NSSize::new(WIDTH, HEIGHT);
    // Initializers may return nil. Losing the decoration must never lose an action.
    let panel: Option<Retained<CursorPanel>> = unsafe {
        msg_send![CursorPanel::alloc(mtm), initWithContentRect: NSRect::new(NSPoint::new(0.0, 0.0), size),
            styleMask: NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel,
            backing: NSBackingStoreType::Buffered, defer: false]
    };
    let panel = panel?;
    unsafe { panel.setReleasedWhenClosed(false) };
    panel.setOpaque(false);
    panel.setBackgroundColor(Some(&NSColor::clearColor()));
    panel.setHasShadow(false);
    panel.setIgnoresMouseEvents(true);
    panel.setHidesOnDeactivate(false);
    panel.setCollectionBehavior(NSWindowCollectionBehavior::CanJoinAllSpaces | NSWindowCollectionBehavior::FullScreenAuxiliary | NSWindowCollectionBehavior::IgnoresCycle);
    panel.setSharingType(if debug::capturable() { NSWindowSharingType::ReadOnly } else { NSWindowSharingType::None });
    let view = CursorView::alloc(mtm).set_ivars(RefCell::new(None));
    let view: Option<Retained<CursorView>> = unsafe { msg_send![super(view), initWithFrame: NSRect::new(NSPoint::new(0.0, 0.0), size)] };
    let view = view?;
    panel.setContentView(Some(&view));
    Some((panel, view))
}

impl Native {
    fn new(mtm: MainThreadMarker, cursor: &Cursor) -> Option<Self> {
        let (panel, view) = panel(mtm)?;
        WINDOW_NUMBER.store(u32::try_from(panel.windowNumber()).unwrap_or(0), Ordering::SeqCst);
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
        let workspace = NSWorkspace::sharedWorkspace().notificationCenter();
        let default = NSNotificationCenter::defaultCenter();
        for (center, name, suspend) in [
            (&workspace, "NSWorkspaceActiveSpaceDidChangeNotification", None),
            (&workspace, "NSWorkspaceSessionDidResignActiveNotification", Some(true)),
            (&workspace, "NSWorkspaceSessionDidBecomeActiveNotification", Some(false)),
            (&default, "NSApplicationDidChangeScreenParametersNotification", None),
            (&workspace, "NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification", None),
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
                    }
                });
                refresh(&cursor);
            });
            let observer = unsafe { center.addObserverForName_object_queue_usingBlock(Some(&NSString::from_str(name)), None, Some(&NSOperationQueue::mainQueue()), &block) };
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
        unsafe { NSRunLoop::mainRunLoop().addTimer_forMode(&timer, NSRunLoopCommonModes) };
        self.timer = Some(timer);
    }

    /// Puts the panel directly above `window`. `fresh` means `self.windows` was just read, so
    /// the stacking it shows is current. False once the marker should fall back.
    fn stack(&mut self, window: u32, layer: i32, pid: i32, fresh: bool) -> bool {
        if self.stacking.window != Some(window) {
            self.stacking = Stacking { window: Some(window), ..Stacking::default() };
        }
        if self.stacking.fallback {
            return false;
        }
        let visible = self.panel.isVisible();
        let stacked = window_number().is_some_and(|panel| cursor::stacked_above(&self.windows, panel, window, pid));
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
        self.panel.orderWindow_relativeTo(NSWindowOrderingMode::Above, window as isize);
        self.store_number();
        self.log(&format!("shown: above window {window} on layer {layer}"));
        true
    }

    /// Status level, for markers whose stacking failed.
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
        WINDOW_NUMBER.store(u32::try_from(self.panel.windowNumber()).unwrap_or(0), Ordering::SeqCst);
    }
}

impl Drop for Native {
    fn drop(&mut self) {
        self.hide();
        for (center, observer) in &self.observers {
            unsafe { center.removeObserver((**observer).as_ref()) };
        }
        self.panel.close();
        WINDOW_NUMBER.store(0, Ordering::SeqCst);
    }
}

/// Retires the marker if it is still the one this tick read.
fn retire(cursor: &Cursor, marker: &Marker) {
    let mut state = cursor.state();
    if state.marker.as_ref().is_some_and(|current| current.sequence == marker.sequence) {
        state.marker = None;
        state.sequence += 1;
    }
}

/// Draws the cursor's current state. Main thread only; a no-op elsewhere.
pub fn refresh(cursor: &Cursor) {
    let Some(mtm) = MainThreadMarker::new() else { return };
    let snapshot = {
        let mut state = cursor.state();
        if state.marker.as_ref().is_some_and(|marker| !state.live(marker)) {
            state.marker = None;
        }
        state.marker.clone().map(|marker| (marker, state.appearance.clone()))
    };
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
        let fresh = native.last_check.is_none_or(|last| now.duration_since(last) >= Duration::from_millis(100));
        if fresh {
            native.windows = input::window_list();
            native.last_check = Some(now);
            native.visibility_changed = false;
        }
        let mut place = cursor::placement(&marker.target, &native.windows);
        if let Placement::Moved { offset, frame } = place {
            if let Some(current) = cursor.state().marker.as_mut().filter(|current| current.sequence == marker.sequence) {
                current.shift(offset, frame);
            }
            marker.shift(offset, frame);
            place = cursor::placement(&marker.target, &native.windows);
        }
        let (number, layer) = match place {
            Placement::Gone | Placement::Moved { .. } => {
                retire(cursor, &marker);
                native.hide();
                return;
            }
            Placement::Hidden => return native.conceal(cursor, &format!("hidden: window {} isn't on screen", marker.target.anchor.window)),
            Placement::Window { number, layer } => (number, layer),
        };
        if native.visibility_changed {
            return native.conceal(cursor, "hidden: a Space or display is changing");
        }
        if native.suspended || input::screen_locked() {
            return native.conceal(cursor, "hidden: the session is inactive or the screen locked");
        }

        let screens = NSScreen::screens(mtm);
        let Some(primary) = screens.firstObject() else {
            return native.conceal(cursor, "hidden: no screens");
        };
        let primary = primary.frame();
        let reduced = NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion();
        let point = cursor::appkit_point(marker.tip(now, reduced), primary.origin.y + primary.size.height);
        let Some(screen) = screens.iter().map(|screen| screen.frame()).find(|frame| {
            point.0 >= frame.origin.x && point.0 < frame.origin.x + frame.size.width && point.1 >= frame.origin.y && point.1 < frame.origin.y + frame.size.height
        }) else {
            return native.conceal(cursor, "hidden: the point is off every display");
        };
        let (frame, tip, mirror_x, mirror_y) = cursor::layout(point, Rect { x: screen.origin.x, y: screen.origin.y, width: screen.size.width, height: screen.size.height });
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
        if !native.stack(number, layer, marker.target.pid, fresh) {
            let skip: Vec<u32> = window_number().into_iter().collect();
            if cursor::uncovered(&marker.target, &native.windows, &skip) {
                native.float("fallback: shown at status level, the point uncovered");
            } else {
                native.conceal(cursor, "fallback: hidden, the point covered");
            }
        }
    });
}

/// The drawing for this tick, and the timer interval its motion needs.
fn animate(marker: &Marker, appearance: Appearance, now: Instant, reduced: bool, tip: (f64, f64), mirror_x: bool, mirror_y: bool) -> (Drawing, f64) {
    let secs = |from: Instant| now.duration_since(from).as_secs_f64();
    let resting = marker.completed.is_some_and(|end| now.duration_since(end) >= Duration::from_millis(600));
    // Time since this action reached its point: the later of the landing and its start.
    let acted = marker.since_landing(now, reduced).min(secs(marker.started));
    let appear = if reduced { 1.0 } else { (secs(marker.shown) / APPEAR).min(1.0) };
    let opacity = cursor::ease_out(appear);
    let mut scale = 0.6 + 0.4 * cursor::ease_out(appear);
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
    if let (Some(from), false) = (marker.from, reduced)
        && moved < cursor::GLIDE + TRAIL_FADE
    {
        busy = true;
        let fade = 1.0 - ((moved - cursor::GLIDE) / TRAIL_FADE).clamp(0.0, 1.0);
        let head = cursor::glide(Some(from), marker.target.point, moved, false);
        for step in 1..=TRAIL {
            let at = moved - step as f64 * TRAIL_STEP;
            if at < 0.0 {
                break;
            }
            let bead = cursor::glide(Some(from), marker.target.point, at, false);
            trail.push((bead.0 - head.0, bead.1 - head.1, fade));
        }
    }
    let orbit = (!resting && !reduced).then(|| secs(marker.started) / 1.2 * TAU);
    let drawing = Drawing {
        appearance,
        tip,
        mirror_x,
        mirror_y,
        label: marker.label(resting),
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

fn colour(rgb: Rgb, alpha: f64) -> Retained<NSColor> {
    NSColor::colorWithSRGBRed_green_blue_alpha(rgb[0], rgb[1], rgb[2], alpha)
}

fn text_attributes(rgb: Rgb, alpha: f64, size: f64, semibold: bool) -> Retained<NSDictionary<NSString, AnyObject>> {
    let weight = unsafe { if semibold { NSFontWeightSemibold } else { NSFontWeightRegular } };
    let font: Retained<AnyObject> = NSFont::systemFontOfSize_weight(size, weight).into();
    let colour: Retained<AnyObject> = colour(rgb, alpha).into();
    unsafe { NSDictionary::from_slices(&[NSFontAttributeName, NSForegroundColorAttributeName], &[&*font, &*colour]) }
}

fn circle(centre: (f64, f64), radius: f64) -> Retained<NSBezierPath> {
    NSBezierPath::bezierPathWithOvalInRect(NSRect::new(NSPoint::new(centre.0 - radius, centre.1 - radius), NSSize::new(radius * 2.0, radius * 2.0)))
}

/// Draws `body` with a soft drop shadow, restoring the graphics state afterwards.
fn shadowed(blur: f64, alpha: f64, body: impl FnOnce()) {
    NSGraphicsContext::saveGraphicsState_class();
    let shadow = NSShadow::new();
    shadow.setShadowBlurRadius(blur);
    // Shadow offsets ignore the view's flip: negative y falls downward on screen.
    shadow.setShadowOffset(NSSize::new(0.0, -1.5));
    shadow.setShadowColor(Some(&NSColor::colorWithSRGBRed_green_blue_alpha(0.0, 0.0, 0.0, alpha)));
    shadow.set();
    body();
    NSGraphicsContext::restoreGraphicsState_class();
}

/// The pointer outline, tip at the origin, pointing up and left as the system cursor does.
const ARROW_SCALE: f64 = 1.2;
const ARROW: [(f64, f64); 7] = [(0.0, 0.0), (0.0, 17.0), (4.5, 13.0), (7.9, 20.4), (11.0, 19.0), (7.7, 11.8), (13.6, 11.6)];

fn draw(d: &Drawing) {
    let accent = d.appearance.accent;
    if let Some(backdrop) = d.backdrop {
        colour(backdrop, 1.0).setFill();
        NSBezierPath::fillRect(NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(WIDTH, HEIGHT)));
    }
    let fade = d.opacity * if d.resting { 0.8 } else { 1.0 };
    let (x, y) = d.tip;

    // The trail: beads shrinking and fading behind the tip.
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
            let p = cursor::ease_out(d.ping);
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
        colour(d.appearance.outline, fade).setStroke();
        arrow.setLineWidth(3.0);
        arrow.stroke();
    });
    colour(accent, fade).setFill();
    arrow.fill();

    draw_label(d, fade);
}

/// The label beside the arrow: the working mark, the name, and what the agent is doing.
fn draw_label(d: &Drawing, fade: f64) {
    const SIZE: f64 = 10.5;
    const PILL: f64 = 22.0;
    const MARK: f64 = 10.0;
    let (x, y) = d.tip;
    let name_attrs = text_attributes(d.appearance.text, fade, SIZE, true);
    let action_attrs = text_attributes(d.appearance.muted, fade, SIZE, false);
    let name = NSString::from_str(NAME);
    let name_size = unsafe { name.sizeWithAttributes(Some(&name_attrs)) };
    // The label fits the panel beside the arrow (the tip sits `MARGIN` in from the panel's
    // edge, the label 20 beyond it): trim the summary until it does.
    let room = WIDTH - cursor::MARGIN - 20.0 - 3.0 - (9.0 + MARK + 6.0 + name_size.width + 11.0);
    let mut words = d.label.clone();
    let measure = |text: &str| unsafe { NSString::from_str(&format!(" · {text}")).sizeWithAttributes(Some(&action_attrs)) }.width;
    while measure(&words) > room {
        let mut chars: Vec<char> = words.trim_end_matches('…').chars().collect();
        if chars.pop().is_none() {
            break;
        }
        words = chars.into_iter().collect::<String>().trim_end().to_string() + "…";
    }
    let label = NSString::from_str(&format!(" · {words}"));
    let action_size = unsafe { label.sizeWithAttributes(Some(&action_attrs)) };
    let width = 9.0 + MARK + 6.0 + name_size.width + action_size.width + 11.0;
    let px = (if d.mirror_x { x - width - 20.0 } else { x + 20.0 }).clamp(3.0, (WIDTH - width - 3.0).max(3.0));
    let py = (if d.mirror_y { y - PILL - 14.0 } else { y + 14.0 }).clamp(3.0, HEIGHT - PILL - 5.0);
    let pill = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(NSRect::new(NSPoint::new(px, py), NSSize::new(width, PILL)), PILL / 2.0, PILL / 2.0);
    // A solid ground, so the label reads over any app.
    shadowed(6.0, 0.28 * fade, || {
        colour(d.appearance.surface, fade).setFill();
        pill.fill();
    });
    colour(d.appearance.text, fade * if d.resting { 0.1 } else { 0.16 }).setStroke();
    pill.setLineWidth(0.75);
    pill.stroke();

    // The working mark: three beads orbiting while active, one still dot once settled.
    let centre = (px + 9.0 + MARK / 2.0, py + PILL / 2.0);
    match d.orbit {
        Some(phase) => {
            for bead in 0..3 {
                let angle = phase + bead as f64 * TAU / 3.0;
                colour(d.appearance.accent, fade * (1.0 - 0.25 * bead as f64)).setFill();
                circle((centre.0 + 3.2 * angle.cos(), centre.1 + 3.2 * angle.sin()), 1.6).fill();
            }
        }
        None => {
            colour(d.appearance.accent, fade * if d.resting { 0.7 } else { 1.0 }).setFill();
            circle(centre, 2.6).fill();
        }
    }
    unsafe {
        let text_x = px + 9.0 + MARK + 6.0;
        let baseline = py + (PILL - name_size.height.max(action_size.height)) / 2.0;
        name.drawAtPoint_withAttributes(NSPoint::new(text_x, baseline), Some(&name_attrs));
        label.drawAtPoint_withAttributes(NSPoint::new(text_x + name_size.width, baseline), Some(&action_attrs));
    }
}

/// Development aids for a window screenshots can't see. All of it compiles to nothing in
/// release builds.
mod debug {
    use super::*;

    #[cfg(debug_assertions)]
    fn flag(name: &str) -> Option<String> {
        std::env::var(name).ok().filter(|value| !value.is_empty())
    }

    /// `WACKCODE_CURSOR_CAPTURABLE=1`: let screenshots include the panel.
    pub fn capturable() -> bool {
        #[cfg(debug_assertions)]
        return flag("WACKCODE_CURSOR_CAPTURABLE").is_some();
        #[cfg(not(debug_assertions))]
        false
    }

    /// `WACKCODE_CURSOR_DEBUG=1`: log each show and hide with its reason.
    pub fn log(_reason: &str) {
        #[cfg(debug_assertions)]
        if flag("WACKCODE_CURSOR_DEBUG").is_some() {
            eprintln!("[agent-cursor] {_reason}");
        }
    }

    static RENDERED: AtomicBool = AtomicBool::new(false);

    /// `WACKCODE_CURSOR_FRAMES=<dir>` (debug builds): the first time the app's theme arrives,
    /// draws every cursor state to PNGs over light and dark grounds, in that theme.
    pub fn render_frames_once(cursor: &Cursor) {
        #[cfg(debug_assertions)]
        {
            use objc2_app_kit::NSBitmapImageFileType;
            let Some(dir) = flag("WACKCODE_CURSOR_FRAMES").map(std::path::PathBuf::from) else { return };
            let appearance = cursor.appearance();
            if appearance == Appearance::default() || RENDERED.swap(true, Ordering::SeqCst) {
                return;
            }
            let Some(mtm) = MainThreadMarker::new() else { return };
            let Some((_panel, view)) = panel(mtm) else { return };
            let _ = std::fs::create_dir_all(&dir);
            let tip = (cursor::MARGIN, cursor::MARGIN);
            let base = Drawing {
                appearance,
                tip,
                mirror_x: false,
                mirror_y: false,
                label: "Press 7".into(),
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
            let trail = (1..=TRAIL).map(|step| (-7.0 * step as f64, -5.0 * step as f64, 1.0)).collect::<Vec<_>>();
            let states: Vec<(&str, Drawing)> = vec![
                ("active", base.clone()),
                ("appear", Drawing { opacity: 0.5, scale: 0.8, ..base.clone() }),
                ("glide", Drawing { trail, ..base.clone() }),
                ("collapse", Drawing { collapse: 0.5, ..base.clone() }),
                ("ping", Drawing { ping: 0.3, ..base.clone() }),
                ("long", Drawing { label: "Name the new document after the quarterly report".into(), ..base.clone() }),
                ("settled", Drawing { resting: true, orbit: None, ..base.clone() }),
                ("reduced-click", Drawing { flash: true, orbit: None, ..base.clone() }),
                ("mirrored", Drawing { tip: (WIDTH - cursor::MARGIN, HEIGHT - cursor::MARGIN), mirror_x: true, mirror_y: true, label: Kind::Menu.label(false).into(), ..base.clone() }),
            ];
            for (ground, rgb) in [("on-dark", [0.125, 0.129, 0.141]), ("on-light", [0.95, 0.95, 0.95])] {
                for (state, drawing) in &states {
                    *view.ivars().borrow_mut() = Some(Drawing { backdrop: Some(rgb), ..drawing.clone() });
                    let bounds = view.bounds();
                    let Some(bitmap) = view.bitmapImageRepForCachingDisplayInRect(bounds) else { continue };
                    view.cacheDisplayInRect_toBitmapImageRep(bounds, &bitmap);
                    if let Some(png) = unsafe { bitmap.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new()) } {
                        let _ = std::fs::write(dir.join(format!("{ground}-{state}.png")), png.to_vec());
                    }
                }
            }
            log(&format!("rendered frames to {}", dir.display()));
        }
        #[cfg(not(debug_assertions))]
        let _ = (cursor, &RENDERED);
    }
}
