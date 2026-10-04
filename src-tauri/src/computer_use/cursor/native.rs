//! AppKit objects live exclusively in main-thread TLS. The panel is inert both to input and
//! Accessibility, and its exact CG window number is the sole input hit-test exemption.
//! The timer exists only while a marker exists; window-list reads are capped at 10 Hz.

use super::{
    appkit_point, layout, visibility, Cursor, CursorAppearance, Kind, Visibility, HEIGHT, WIDTH,
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
    NSBackingStoreType, NSBezierPath, NSColor, NSFont, NSFontAttributeName,
    NSForegroundColorAttributeName, NSPanel, NSScreen, NSStringDrawing, NSView,
    NSWindowCollectionBehavior, NSWindowSharingType, NSWindowStyleMask, NSWorkspace,
};
use objc2_foundation::{
    NSArray, NSDictionary, NSNotification, NSNotificationCenter, NSObjectProtocol,
    NSOperationQueue, NSPoint, NSRect, NSRunLoop, NSRunLoopCommonModes, NSSize, NSString, NSTimer,
};
use std::cell::RefCell;
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

#[derive(Clone, PartialEq)]
struct Drawing {
    appearance: CursorAppearance,
    tip: (f64, f64),
    mirror_x: bool,
    mirror_y: bool,
    label: &'static str,
    resting: bool,
    pulse: f64,
    press: bool,
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
            if let Some(drawing) = self.ivars().borrow().as_ref() { draw(drawing, self.mtm()); }
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
}

thread_local! { static NATIVE: RefCell<Option<Native>> = const { RefCell::new(None) }; }

impl Native {
    fn new(mtm: MainThreadMarker, cursor: &Cursor) -> Option<Self> {
        // Initializers may return nil. Losing decoration must never lose a computer action.
        let panel: Option<Retained<CursorPanel>> = unsafe {
            msg_send![CursorPanel::alloc(mtm), initWithContentRect: NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(WIDTH, HEIGHT)),
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
        panel.setFloatingPanel(true);
        panel.setLevel(25); // NSStatusWindowLevel: above app windows, below system shields.
        panel.setCollectionBehavior(
            NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::FullScreenAuxiliary
                | NSWindowCollectionBehavior::IgnoresCycle,
        );
        panel.setSharingType(NSWindowSharingType::None);
        let allocated_view = CursorView::alloc(mtm).set_ivars(RefCell::new(None));
        let view: Option<Retained<CursorView>> = unsafe {
            msg_send![super(allocated_view), initWithFrame: NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(WIDTH, HEIGHT))]
        };
        let view = view?;
        panel.setContentView(Some(&view));
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
        Some((state.marker.clone()?, state.appearance.clone()?))
    });
    NATIVE.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some((marker, appearance)) = snapshot else {
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
        if native
            .last_check
            .is_none_or(|last| now.duration_since(last) >= Duration::from_millis(100))
        {
            native.windows = input::window_list();
            native.last_check = Some(now);
            native.visibility_changed = false;
        }
        let visible = visibility(marker.target, &native.windows, &super::overlay_windows());
        if visible == Visibility::Invalid {
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
            native.hide();
            return;
        }
        if visible != Visibility::Visible
            || native.visibility_changed
            || native.suspended
            || input::screen_locked()
        {
            native.ensure_timer(cursor, 0.1);
            native.panel.orderOut(None);
            return;
        }
        let screens = NSScreen::screens(mtm);
        let Some(primary) = screens.firstObject() else {
            native.panel.orderOut(None);
            return;
        };
        let primary = primary.frame();
        let point = appkit_point(marker.target.point, primary.origin.y + primary.size.height);
        let Some(screen) = screens.iter().map(|screen| screen.frame()).find(|frame| {
            point.0 >= frame.origin.x
                && point.0 < frame.origin.x + frame.size.width
                && point.1 >= frame.origin.y
                && point.1 < frame.origin.y + frame.size.height
        }) else {
            native.panel.orderOut(None);
            return;
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
        let reduced = NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion();
        let elapsed = now.duration_since(marker.started).as_secs_f64();
        let resting = marker
            .completed
            .is_some_and(|end| now.duration_since(end) >= Duration::from_millis(600));
        let pulse = if !reduced && marker.kind == Kind::Clicking && elapsed < 0.3 {
            elapsed / 0.3
        } else {
            -1.0
        };
        let press = !reduced && marker.kind == Kind::Pressing && elapsed < 0.16;
        native.ensure_timer(
            cursor,
            if pulse >= 0.0 || press {
                1.0 / 60.0
            } else {
                0.1
            },
        );
        let drawing = Drawing {
            appearance,
            tip,
            mirror_x,
            mirror_y,
            label: marker.kind.label(resting),
            resting,
            pulse,
            press,
        };
        let frame = NSRect::new(NSPoint::new(frame.x, frame.y), NSSize::new(WIDTH, HEIGHT));
        if native.panel.frame() != frame {
            native.panel.setFrame_display(frame, false);
        }
        let changed = native.view.ivars().borrow().as_ref() != Some(&drawing);
        if changed {
            *native.view.ivars().borrow_mut() = Some(drawing);
            native.view.setNeedsDisplay(true);
        }
        if !native.panel.isVisible() {
            native.panel.orderFrontRegardless();
            WINDOW_NUMBER.store(
                u32::try_from(native.panel.windowNumber()).unwrap_or(0),
                Ordering::SeqCst,
            );
        }
    });
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

fn text_attributes(hex: &str, size: f64) -> Retained<NSDictionary<NSString, AnyObject>> {
    let font: Retained<AnyObject> = NSFont::systemFontOfSize(size).into();
    let colour: Retained<AnyObject> = colour(hex, 1.0).into();
    unsafe {
        NSDictionary::from_slices(
            &[NSFontAttributeName, NSForegroundColorAttributeName],
            &[&*font, &*colour],
        )
    }
}

fn draw(d: &Drawing, _mtm: MainThreadMarker) {
    let alpha = if d.resting { 0.78 } else { 1.0 };
    let (x, y) = d.tip;
    if d.pulse >= 0.0 {
        let radius = 3.0 + d.pulse * 7.0;
        let ring = NSBezierPath::bezierPathWithOvalInRect(NSRect::new(
            NSPoint::new(x - radius, y - radius),
            NSSize::new(radius * 2.0, radius * 2.0),
        ));
        colour(&d.appearance.accent, (1.0 - d.pulse) * 0.4).setStroke();
        ring.setLineWidth(1.0);
        ring.stroke();
    }
    let sx = if d.mirror_x { -1.0 } else { 1.0 };
    let sy = if d.mirror_y { -1.0 } else { 1.0 };
    let scale = if d.press { 0.96 } else { 1.0 };
    let arrow = NSBezierPath::bezierPath();
    for (i, (px, py)) in [
        (0.0, 0.0),
        (0.0, 14.5),
        (3.8, 11.0),
        (6.8, 18.0),
        (9.6, 16.8),
        (6.5, 10.0),
        (13.2, 9.8),
    ]
    .iter()
    .enumerate()
    {
        let point = NSPoint::new(x + px * sx * scale, y + py * sy * scale);
        if i == 0 {
            arrow.moveToPoint(point);
        } else {
            arrow.lineToPoint(point);
        }
    }
    arrow.closePath();
    // A fine contrasting edge keeps the accent readable without turning it into a sticker.
    colour(&d.appearance.outline, alpha).setStroke();
    arrow.setLineWidth(1.4);
    arrow.stroke();
    colour(&d.appearance.accent, alpha).setFill();
    arrow.fill();

    let name_attrs = text_attributes(&d.appearance.text, 9.5);
    let action_attrs = text_attributes(&d.appearance.muted_text, 9.5);
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
    let width = name_size.width + action_size.width + 16.0;
    let px = (if d.mirror_x {
        x - width - 14.0
    } else {
        x + 14.0
    })
    .clamp(3.0, WIDTH - width - 3.0);
    let py = (if d.mirror_y { y - 26.0 } else { y + 6.0 }).clamp(2.0, HEIGHT - 24.0);
    let pill = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(
        NSRect::new(NSPoint::new(px, py), NSSize::new(width, 22.0)),
        5.0,
        5.0,
    );
    colour(&d.appearance.surface, 1.0).setFill();
    pill.fill();
    colour(&d.appearance.text, if d.resting { 0.1 } else { 0.16 }).setStroke();
    pill.setLineWidth(0.75);
    pill.stroke();
    unsafe {
        let baseline = py + (22.0 - name_size.height.max(action_size.height)) / 2.0;
        name.drawAtPoint_withAttributes(NSPoint::new(px + 8.0, baseline), Some(&name_attrs));
        label.drawAtPoint_withAttributes(
            NSPoint::new(px + 8.0 + name_size.width, baseline),
            Some(&action_attrs),
        );
    }
}
