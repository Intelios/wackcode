//! Window and display pictures with ScreenCaptureKit. Ported from version 2's
//! `wackdev/src/capture.rs`.
//!
//! Invariants:
//! - A window is captured through a desktop-independent-window filter, so it is pictured even
//!   while other windows cover it, and nothing else (no other app, notification or WackCode
//!   pixel) can be in it. Its child windows (sheets, attached panels) come with it.
//! - A full picture's frame is `geometry::budget` of the window's size in **points**; `scale`
//!   shrinks the picture, never the frame. A zoom (`region`) is taken at the display's full
//!   pixel density and carries no frame of its own: coordinates keep referring to the last full
//!   picture.
//! - A display picture includes only the windows of the apps it is given (full-screen control's
//!   allowed apps); WackCode is never among them.
//! - A frame SkyLight would abort on (CGRectNull, NaN, zero) is refused before any filter is
//!   made (`Rect::has_area`): a window whose app just quit reports exactly that.
//! - JPEG is encoded inside the completion handler, so no image object crosses threads, and
//!   kept under the base64 size a picture may have.

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use block2::RcBlock;
use objc2::{AnyThread, rc::Retained, runtime::AnyObject};
use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSImageCompressionFactor};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_core_graphics::CGImage;
use objc2_foundation::{NSArray, NSDictionary, NSError, NSNumber};
use objc2_screen_capture_kit::{SCContentFilter, SCScreenshotManager, SCShareableContent, SCStreamConfiguration};
use tokio::sync::oneshot;

use crate::geometry::{self, Rect};

const CAPTURE_TIMEOUT: Duration = Duration::from_secs(15);
/// The most base64 one picture may have (`wack_agent::images::view::MAX_PICTURE_BYTES`).
const MAX_BASE64: usize = 1_500_000;
const QUALITIES: [f64; 4] = [0.8, 0.65, 0.5, 0.35];

#[derive(Debug, Clone)]
pub struct Shot {
    pub jpeg: Vec<u8>,
    /// The picture's size in pixels.
    pub picture: (u32, u32),
    /// The size the model's coordinates refer to (a full picture's frame); for a zoom, the
    /// picture's own size.
    pub frame: (u32, u32),
    /// The window's (or display's) frame in global points when it was taken: what the picture
    /// covers, so picture coordinates map through this. ScreenCaptureKit's window frame includes
    /// the border, a point wider on each side than the AX and window-list frames.
    pub bounds: Rect,
}

/// What to take of a window.
#[derive(Debug, Clone, Copy)]
pub struct WindowShot {
    pub number: u32,
    /// In (0.1, 1]: the picture's size against its frame.
    pub scale: f64,
    pub region: Option<Zoom>,
}

/// A zoom into part of a window.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Zoom {
    /// `[x0, y0, x1, y1]` in the frame of the last full picture.
    pub rect: [f64; 4],
    /// That picture's frame, in pixels.
    pub frame: (u32, u32),
    /// The window's size in points when it was taken.
    pub size: (f64, f64),
}

type Reply = Arc<Mutex<Option<oneshot::Sender<Result<Shot, String>>>>>;

fn reply(sender: &Reply, result: Result<Shot, String>) {
    if let Some(sender) = sender.lock().ok().and_then(|mut sender| sender.take()) {
        let _ = sender.send(result);
    }
}

fn error_text(error: *mut NSError) -> String {
    if error.is_null() { "macOS didn't say why".into() } else { unsafe { (*error).localizedDescription() }.to_string() }
}

fn jpeg(image: &CGImage) -> Option<(Vec<u8>, u32, u32)> {
    let bitmap = NSBitmapImageRep::initWithCGImage(NSBitmapImageRep::alloc(), image);
    let (width, height) = (bitmap.pixelsWide().max(0) as u32, bitmap.pixelsHigh().max(0) as u32);
    let mut last = None;
    for quality in QUALITIES {
        let quality: Retained<AnyObject> = NSNumber::numberWithDouble(quality).into();
        let properties = NSDictionary::from_slices(&[unsafe { NSImageCompressionFactor }], &[&*quality]);
        let data = unsafe { bitmap.representationUsingType_properties(NSBitmapImageFileType::JPEG, &properties) }?.to_vec();
        if data.len().div_ceil(3) * 4 <= MAX_BASE64 {
            return Some((data, width, height));
        }
        last = Some(data);
    }
    last.map(|data| (data, width, height))
}

const UNREACHABLE: &str = "That window can't be pictured right now: it may be closed, minimized or in another Space.";

fn permission_message(error: *mut NSError) -> String {
    format!("Windows couldn't be pictured ({}). Check that WackCode has Screen Recording in Settings › Computer use.", error_text(error))
}

/// Takes one window. See the header for what the frame is.
pub async fn window(shot: WindowShot) -> Result<Shot, String> {
    let (sender, receiver) = oneshot::channel();
    start_window(shot, Arc::new(Mutex::new(Some(sender))));
    tokio::time::timeout(CAPTURE_TIMEOUT, receiver).await.map_err(|_| "Picturing the window timed out.".to_string())?.map_err(|_| "Picturing the window was cancelled.".to_string())?
}

/// Starts the capture. ScreenCaptureKit copies the handlers, so none outlives this call here and
/// no block is held across an await.
fn start_window(shot: WindowShot, sender: Reply) {
    let content_reply = sender.clone();
    let content_handler = RcBlock::new(move |content: *mut SCShareableContent, error: *mut NSError| {
        if content.is_null() {
            return reply(&content_reply, Err(permission_message(error)));
        }
        let content = unsafe { &*content };
        let windows = unsafe { content.windows() };
        let Some(window) = windows.iter().find(|window| unsafe { window.windowID() } == shot.number) else {
            return reply(&content_reply, Err(UNREACHABLE.into()));
        };
        let raw = unsafe { window.frame() };
        let bounds = Rect::new(raw.origin.x, raw.origin.y, raw.size.width, raw.size.height);
        if !bounds.has_area() {
            return reply(&content_reply, Err(UNREACHABLE.into()));
        }
        let filter = unsafe { SCContentFilter::initWithDesktopIndependentWindow(SCContentFilter::alloc(), &window) };
        let density = (unsafe { SCShareableContent::infoForFilter(&filter).pointPixelScale() } as f64).max(1.0);
        let configuration = unsafe { SCStreamConfiguration::new() };
        let (picture, frame) = match shot.region {
            None => {
                let frame = geometry::budget(bounds.width, bounds.height);
                (geometry::scaled(frame, shot.scale), frame)
            }
            Some(Zoom { rect: [x0, y0, x1, y1], frame: full, size }) => {
                // The region in window points, clamped to the window.
                let to_points = |x: f64, y: f64| ((x / full.0 as f64 * size.0).clamp(0.0, bounds.width), (y / full.1 as f64 * size.1).clamp(0.0, bounds.height));
                let (left, top) = to_points(x0.min(x1), y0.min(y1));
                let (right, bottom) = to_points(x0.max(x1), y0.max(y1));
                if right - left < 1.0 || bottom - top < 1.0 {
                    return reply(&content_reply, Err("The region is empty: give [x0, y0, x1, y1] with x1 > x0 and y1 > y0, inside the last picture.".into()));
                }
                unsafe { configuration.setSourceRect(CGRect::new(CGPoint::new(left, top), CGSize::new(right - left, bottom - top))) };
                let zoom = geometry::budget((right - left) * density, (bottom - top) * density);
                let picture = geometry::scaled(zoom, shot.scale);
                (picture, picture)
            }
        };
        unsafe {
            configuration.setWidth(picture.0 as usize);
            configuration.setHeight(picture.1 as usize);
            configuration.setScalesToFit(true);
            configuration.setShowsCursor(false);
            configuration.setIgnoreShadowsSingleWindow(true);
            configuration.setIncludeChildWindows(true);
        }
        let image_reply = content_reply.clone();
        let image_handler = RcBlock::new(move |image: *mut CGImage, error: *mut NSError| {
            if image.is_null() {
                return reply(&image_reply, Err(format!("The window couldn't be pictured: {}.", error_text(error))));
            }
            let result = jpeg(unsafe { &*image })
                .map(|(jpeg, width, height)| Shot { jpeg, picture: (width, height), frame, bounds })
                .ok_or_else(|| "The window's picture couldn't be encoded.".to_string());
            reply(&image_reply, result);
        });
        unsafe { SCScreenshotManager::captureImageWithFilter_configuration_completionHandler(&filter, &configuration, Some(&image_handler)) };
    });
    unsafe { SCShareableContent::getShareableContentExcludingDesktopWindows_onScreenWindowsOnly_completionHandler(true, false, &content_handler) };
}

/// Takes the display `display` (a CGDirectDisplayID; 0 for the main display) showing only the
/// windows of `pids`. Full-screen control's picture: its frame is the budget of the display's
/// size in points.
pub async fn display(display: u32, pids: Vec<i32>, scale: f64) -> Result<Shot, String> {
    let (sender, receiver) = oneshot::channel();
    let sender: Reply = Arc::new(Mutex::new(Some(sender)));
    {
        let content_reply = sender.clone();
        let handler = RcBlock::new(move |content: *mut SCShareableContent, error: *mut NSError| {
            if content.is_null() {
                return reply(&content_reply, Err(permission_message(error)));
            }
            let content = unsafe { &*content };
            let displays = unsafe { content.displays() };
            let main = objc2_core_graphics::CGMainDisplayID();
            let wanted = if display == 0 { main } else { display };
            let Some(screen) = displays.iter().find(|candidate| unsafe { candidate.displayID() } == wanted) else {
                return reply(&content_reply, Err("That display isn't connected.".into()));
            };
            let apps = unsafe { content.applications() };
            let included: Vec<_> = apps.iter().filter(|app| pids.contains(&unsafe { app.processID() })).collect();
            let included = NSArray::from_retained_slice(&included);
            let filter = unsafe {
                SCContentFilter::initWithDisplay_includingApplications_exceptingWindows(SCContentFilter::alloc(), &screen, &included, &NSArray::new())
            };
            let raw = unsafe { screen.frame() };
            let bounds = Rect::new(raw.origin.x, raw.origin.y, raw.size.width, raw.size.height);
            let frame = geometry::budget(bounds.width, bounds.height);
            let picture = geometry::scaled(frame, scale);
            let configuration = unsafe { SCStreamConfiguration::new() };
            unsafe {
                configuration.setWidth(picture.0 as usize);
                configuration.setHeight(picture.1 as usize);
                configuration.setScalesToFit(true);
                configuration.setShowsCursor(true);
            }
            let image_reply = content_reply.clone();
            let image_handler = RcBlock::new(move |image: *mut CGImage, error: *mut NSError| {
                if image.is_null() {
                    return reply(&image_reply, Err(format!("The screen couldn't be pictured: {}.", error_text(error))));
                }
                let result = jpeg(unsafe { &*image })
                    .map(|(jpeg, width, height)| Shot { jpeg, picture: (width, height), frame, bounds })
                    .ok_or_else(|| "The screen's picture couldn't be encoded.".to_string());
                reply(&image_reply, result);
            });
            unsafe { SCScreenshotManager::captureImageWithFilter_configuration_completionHandler(&filter, &configuration, Some(&image_handler)) };
        });
        unsafe { SCShareableContent::getShareableContentExcludingDesktopWindows_onScreenWindowsOnly_completionHandler(true, true, &handler) };
    }
    tokio::time::timeout(CAPTURE_TIMEOUT, receiver).await.map_err(|_| "Picturing the screen timed out.".to_string())?.map_err(|_| "Picturing the screen was cancelled.".to_string())?
}

/// Whether ScreenCaptureKit will actually hand out content: `CGPreflightScreenCaptureAccess`
/// can report a grant that no longer applies (after an ad hoc rebuild, say).
pub async fn probe() -> bool {
    let (sender, receiver) = oneshot::channel::<bool>();
    {
        let sender = Arc::new(Mutex::new(Some(sender)));
        let handler = RcBlock::new(move |content: *mut SCShareableContent, _error: *mut NSError| {
            if let Some(sender) = sender.lock().ok().and_then(|mut sender| sender.take()) {
                let _ = sender.send(!content.is_null());
            }
        });
        unsafe { SCShareableContent::getShareableContentExcludingDesktopWindows_onScreenWindowsOnly_completionHandler(true, true, &handler) };
    }
    tokio::time::timeout(Duration::from_secs(5), receiver).await.ok().and_then(Result::ok).unwrap_or(false)
}
