//! Main-thread-only native presentation and picker interception. AppKit scales a container,
//! never the WKWebView's logical viewport. Remote pages receive no event/IPC capability.
use block2::RcBlock;
use objc2::{
    define_class, msg_send, rc::Retained, runtime::AnyObject, DefinedClass, MainThreadMarker,
    MainThreadOnly,
};
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSBezierPath, NSColor, NSEvent, NSEventMask, NSEventType, NSFont,
    NSFontAttributeName, NSForegroundColorAttributeName, NSStringDrawing, NSView,
};
use objc2_foundation::{NSArray, NSDictionary, NSPoint, NSRect, NSSize, NSString};
use objc2_web_kit::WKWebView;
use std::{cell::RefCell, collections::HashMap, ptr::NonNull};
use tauri::{AppHandle, Manager, Webview};

struct Highlight {
    target: crate::models::BrowserContextTarget,
    accent: String,
    outline: String,
    scale: f64,
}
define_class!(
    #[unsafe(super = NSView)]
    #[thread_kind = MainThreadOnly]
    #[name = "WackCodeBrowserHighlight"]
    #[ivars = RefCell<Option<Highlight>>]
    struct HighlightView;
    impl HighlightView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool { true }
        #[unsafe(method(hitTest:))]
        fn hit_test(&self, _point: NSPoint) -> *mut NSView { std::ptr::null_mut() }
        #[unsafe(method(isAccessibilityElement))]
        fn is_accessibility_element(&self) -> bool { false }
        #[unsafe(method_id(accessibilityChildren))]
        fn accessibility_children(&self) -> Retained<NSArray<AnyObject>> { NSArray::new() }
        #[unsafe(method(drawRect:))]
        fn draw(&self, _rect: NSRect) {
            let drawing = self.ivars().borrow();
            let Some(drawing) = drawing.as_ref() else { return; };
            let r = &drawing.target.bounds;
            let scale = drawing.scale.max(0.05);
            let rect = NSRect::new(NSPoint::new(r.x, r.y), NSSize::new(r.width, r.height));
            let border = NSBezierPath::bezierPathWithRect(rect);
            colour(&drawing.outline).setStroke(); border.setLineWidth(6.0 / scale); border.stroke();
            colour(&drawing.accent).setStroke(); border.setLineWidth(3.0 / scale); border.stroke();
            let text = NSString::from_str(&drawing.target.label);
            let font: Retained<AnyObject> = NSFont::systemFontOfSize(12.0 / scale).into();
            let foreground: Retained<AnyObject> = colour(&drawing.accent).into();
            unsafe {
                let attrs = NSDictionary::from_slices(&[NSFontAttributeName, NSForegroundColorAttributeName], &[&*font, &*foreground]);
                let width = (text.sizeWithAttributes(Some(&attrs)).width + 12.0 / scale).min(self.bounds().size.width);
                let x = r.x.max(0.0).min((self.bounds().size.width - width).max(0.0));
                let y = (r.y - 24.0 / scale).max(0.0);
                colour(&drawing.outline).setFill();
                NSBezierPath::bezierPathWithRect(NSRect::new(NSPoint::new(x,y), NSSize::new(width,22.0/scale))).fill();
                text.drawAtPoint_withAttributes(NSPoint::new(x + 6.0/scale,y + 3.0/scale), Some(&attrs));
            }
        }
    }
);

fn colour(hex: &str) -> Retained<NSColor> {
    let (r, g, b) = crate::glass::parse_hex(hex).unwrap_or((0, 0, 0));
    NSColor::colorWithSRGBRed_green_blue_alpha(
        r as f64 / 255.0,
        g as f64 / 255.0,
        b as f64 / 255.0,
        1.0,
    )
}

struct Surface {
    container: Retained<NSView>,
    monitor: Option<Retained<AnyObject>>,
    accepts_mouse: bool,
    highlight: Retained<HighlightView>,
    active_capture: Option<String>,
}
thread_local! { static SURFACES: RefCell<HashMap<String, Surface>> = RefCell::new(HashMap::new()); }

pub fn place(
    webview: &Webview,
    rect: (f64, f64, f64, f64),
    viewport: (f64, f64),
) -> Result<(), String> {
    let label = webview.label().to_owned();
    webview
        .with_webview(move |platform| unsafe {
            let view: &WKWebView = &*platform.inner().cast();
            let mtm = MainThreadMarker::new_unchecked();
            SURFACES.with_borrow_mut(|surfaces| {
                if !surfaces.contains_key(&label) {
                    let Some(parent) = view.superview() else {
                        return;
                    };
                    let container = NSView::initWithFrame(NSView::alloc(mtm), NSRect::ZERO);
                    parent.addSubview(&container);
                    container.addSubview(view);
                    view.setAutoresizingMask(NSAutoresizingMaskOptions::empty());
                    let allocated = HighlightView::alloc(mtm).set_ivars(RefCell::new(None));
                    let highlight: Retained<HighlightView> =
                        msg_send![super(allocated), initWithFrame: NSRect::ZERO];
                    container.addSubview(&highlight);
                    surfaces.insert(
                        label.clone(),
                        Surface {
                            container,
                            monitor: None,
                            accepts_mouse: false,
                            highlight,
                            active_capture: None,
                        },
                    );
                }
                let Some(surface) = surfaces.get(&label) else {
                    return;
                };
                let Some(parent) = surface.container.superview() else {
                    return;
                };
                let (x, y, width, height) = rect;
                let py = if parent.isFlipped() {
                    y
                } else {
                    parent.bounds().size.height - y - height
                };
                surface
                    .container
                    .setFrame(NSRect::new(NSPoint::new(x, py), NSSize::new(width, height)));
                surface
                    .container
                    .setBoundsSize(NSSize::new(viewport.0, viewport.1));
                view.setFrame(NSRect::new(
                    NSPoint::ZERO,
                    NSSize::new(viewport.0, viewport.1),
                ));
                surface.highlight.setFrame(view.frame());
            });
        })
        .map_err(|error| format!("Could not position the browser: {error}"))
}

pub fn cancel(webview: &Webview) {
    let label = webview.label().to_owned();
    let _ = webview.with_webview(move |platform| unsafe {
        let view: &WKWebView = &*platform.inner().cast();
        SURFACES.with_borrow_mut(|surfaces| {
            if let Some(surface) = surfaces.get_mut(&label) {
                surface.active_capture = None;
                *surface.highlight.ivars().borrow_mut() = None;
                surface.highlight.setNeedsDisplay(true);
                if let Some(monitor) = surface.monitor.take() {
                    NSEvent::removeMonitor(&monitor);
                    if let Some(window) = view.window() {
                        window.setAcceptsMouseMovedEvents(surface.accepts_mouse);
                    }
                }
            }
        });
    });
}

pub fn dispose(webview: &Webview) {
    cancel(webview);
    let label = webview.label().to_owned();
    let _ = webview.with_webview(move |_| {
        SURFACES.with_borrow_mut(|surfaces| {
            if let Some(surface) = surfaces.remove(&label) {
                surface.container.removeFromSuperview();
            }
        });
    });
}

/// Drawing a native sibling avoids touching the page DOM or triggering its observers.
pub fn highlight(
    webview: &Webview,
    capture_id: String,
    target: Option<crate::models::BrowserContextTarget>,
    accent: String,
    outline: String,
) {
    let label = webview.label().to_owned();
    let _ = webview.with_webview(move |_| {
        SURFACES.with_borrow(|surfaces| {
            if let Some(surface) = surfaces.get(&label) {
                if surface.active_capture.as_deref() != Some(&capture_id) {
                    return;
                }
                let scale =
                    surface.container.frame().size.width / surface.container.bounds().size.width;
                *surface.highlight.ivars().borrow_mut() = target.map(|target| Highlight {
                    target,
                    accent,
                    outline,
                    scale,
                });
                surface.highlight.setNeedsDisplay(true);
            }
        });
    });
}

pub fn pick(
    app: &AppHandle,
    task_id: &str,
    webview: &Webview,
    capture_id: &str,
    accent: &str,
    outline: &str,
) -> Result<(), String> {
    cancel(webview);
    let label = webview.label().to_owned();
    let app = app.clone();
    let task = task_id.to_owned();
    let capture = capture_id.to_owned();
    let accent = accent.to_owned();
    let outline = outline.to_owned();
    webview
        .with_webview(move |platform| unsafe {
            let view: &WKWebView = &*platform.inner().cast();
            let view = Retained::retain(view as *const WKWebView as *mut WKWebView).unwrap();
            let Some(window) = view.window() else {
                return;
            };
            let accepts_mouse = window.acceptsMouseMovedEvents();
            window.setAcceptsMouseMovedEvents(true);
            let owner = capture.clone();
            let handler = RcBlock::new(move |event: NonNull<NSEvent>| -> *mut NSEvent {
                let event_ref = event.as_ref();
                if view
                    .window()
                    .is_none_or(|window| window.windowNumber() != event_ref.windowNumber())
                {
                    return event.as_ptr();
                }
                let manager = app.state::<super::BrowserManager>();
                let active = manager.sessions.lock().ok().is_some_and(|sessions| {
                    sessions.get(&task).is_some_and(|session| {
                        session.capture_id.as_deref() == Some(&capture) && session.visible
                    })
                });
                if !active {
                    return event.as_ptr();
                }
                if event_ref.r#type() == NSEventType::KeyDown && event_ref.keyCode() == 53 {
                    super::cancel_capture(&app, &task, Some(&capture));
                    return std::ptr::null_mut();
                }
                let point = view.convertPoint_fromView(event_ref.locationInWindow(), None);
                let y = if view.isFlipped() {
                    point.y
                } else {
                    view.bounds().size.height - point.y
                };
                if point.x < 0.0
                    || y < 0.0
                    || point.x >= view.bounds().size.width
                    || y >= view.bounds().size.height
                {
                    return event.as_ptr();
                }
                match event_ref.r#type() {
                    NSEventType::LeftMouseDown => {
                        if let Ok(mut sessions) = manager.sessions.lock() {
                            if let Some(sender) = sessions
                                .get_mut(&task)
                                .and_then(|session| session.pick_sender.take())
                            {
                                let _ = sender.send(Some((point.x, y)));
                            }
                        }
                        std::ptr::null_mut()
                    }
                    NSEventType::LeftMouseUp | NSEventType::LeftMouseDragged => {
                        std::ptr::null_mut()
                    }
                    NSEventType::MouseMoved => {
                        super::hover_pick(&app, &task, &capture, point.x, y, &accent, &outline);
                        std::ptr::null_mut()
                    }
                    _ => event.as_ptr(),
                }
            });
            let monitor = NSEvent::addLocalMonitorForEventsMatchingMask_handler(
                NSEventMask::MouseMoved
                    | NSEventMask::LeftMouseDown
                    | NSEventMask::LeftMouseUp
                    | NSEventMask::LeftMouseDragged
                    | NSEventMask::KeyDown,
                &handler,
            );
            SURFACES.with_borrow_mut(|surfaces| {
                if let Some(surface) = surfaces.get_mut(&label) {
                    *surface.highlight.ivars().borrow_mut() = None;
                    surface.highlight.setNeedsDisplay(true);
                    surface.active_capture = Some(owner);
                    surface.monitor = monitor;
                    surface.accepts_mouse = accepts_mouse;
                }
            });
        })
        .map_err(|error| format!("Could not start element picking: {error}"))
}
