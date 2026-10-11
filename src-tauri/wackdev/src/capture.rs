//! Captures one window with ScreenCaptureKit (macOS 14+): a desktop-independent-window filter,
//! so the window is captured even while other windows cover it, and nothing else — no other
//! app, notification or WackCode pixel — can be in the image. Encoded as JPEG inside the
//! completion handler so no image object crosses threads.

use super::geometry::{self, Rect, MAX_LONG_EDGE};
use block2::RcBlock;
use objc2::{AnyThread, rc::Retained, runtime::AnyObject};
use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSImageCompressionFactor};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_core_graphics::CGImage;
use objc2_foundation::{NSDictionary, NSError, NSNumber};
use objc2_screen_capture_kit::{
    SCContentFilter, SCScreenshotManager, SCShareableContent, SCStreamConfiguration,
};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::oneshot;

const CAPTURE_TIMEOUT: Duration = Duration::from_secs(15);
const JPEG_QUALITY: f64 = 0.8;

pub struct Capture {
    pub jpeg: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// The window's frame in global screen points when it was captured.
    pub frame: Rect,
}

type Reply = Arc<Mutex<Option<oneshot::Sender<Result<Capture, String>>>>>;

fn reply(sender: &Reply, result: Result<Capture, String>) {
    if let Some(sender) = sender.lock().ok().and_then(|mut sender| sender.take()) {
        let _ = sender.send(result);
    }
}

fn error_text(error: *mut NSError) -> String {
    if error.is_null() { "macOS didn't say why".into() } else { unsafe { (*error).localizedDescription() }.to_string() }
}

fn encode(image: &CGImage) -> Option<(Vec<u8>, u32, u32)> {
    let bitmap = NSBitmapImageRep::initWithCGImage(NSBitmapImageRep::alloc(), image);
    let (width, height) = (bitmap.pixelsWide(), bitmap.pixelsHigh());
    let quality: Retained<AnyObject> = NSNumber::numberWithDouble(JPEG_QUALITY).into();
    let properties = NSDictionary::from_slices(&[unsafe { NSImageCompressionFactor }], &[&*quality]);
    let data = unsafe { bitmap.representationUsingType_properties(NSBitmapImageFileType::JPEG, &properties) }?;
    Some((data.to_vec(), width.max(0) as u32, height.max(0) as u32))
}

/// Captures window `number` (a window-server number), scaled so its long edge is at most
/// `MAX_LONG_EDGE` pixels. `crop` is a global screen rect (same space as AX frames); parts of
/// it outside the window clamp to the window's edge. `None` captures the whole window.
pub async fn window(number: u32, crop: Option<Rect>) -> Result<Capture, String> {
    let (sender, receiver) = oneshot::channel();
    start_window_capture(number, crop, Arc::new(Mutex::new(Some(sender))));
    tokio::time::timeout(CAPTURE_TIMEOUT, receiver)
        .await
        .map_err(|_| "The window capture timed out.".to_string())?
        .map_err(|_| "The window capture was cancelled.".to_string())?
}

/// Starts the capture; ScreenCaptureKit copies the handlers, so none outlives this call here
/// (and no block is held across an await).
fn start_window_capture(number: u32, crop: Option<Rect>, sender: Reply) {
    let content_reply = sender.clone();
    let content_handler = RcBlock::new(move |content: *mut SCShareableContent, error: *mut NSError| {
        if content.is_null() {
            let message = format!(
                "The window couldn't be captured ({}). Check that WackCode has Screen Recording permission in System Settings › Privacy & Security.",
                error_text(error)
            );
            return reply(&content_reply, Err(message));
        }
        let content = unsafe { &*content };
        let windows = unsafe { content.windows() };
        let Some(window) = windows.iter().find(|window| unsafe { window.windowID() } == number) else {
            return reply(&content_reply, Err("That window can't be captured right now: it may be closed, minimized or in another Space.".into()));
        };
        let raw = unsafe { window.frame() };
        let frame = Rect { x: raw.origin.x, y: raw.origin.y, width: raw.size.width, height: raw.size.height };
        let filter = unsafe { SCContentFilter::initWithDesktopIndependentWindow(SCContentFilter::alloc(), &window) };
        let scale = unsafe { SCShareableContent::infoForFilter(&filter).pointPixelScale() } as f64;
        // The capture's pixel size; `crop` (in points) maps into it with these ratios.
        let (width, height) = geometry::output_size(&frame, scale, MAX_LONG_EDGE);
        let configuration = unsafe { SCStreamConfiguration::new() };
        unsafe {
            configuration.setWidth(width as usize);
            configuration.setHeight(height as usize);
            configuration.setScalesToFit(true);
            configuration.setShowsCursor(false);
            configuration.setIgnoreShadowsSingleWindow(true);
        }
        let image_reply = content_reply.clone();
        let image_handler = RcBlock::new(move |image: *mut CGImage, error: *mut NSError| {
            if image.is_null() {
                return reply(&image_reply, Err(format!("The window couldn't be captured: {}.", error_text(error))));
            }
            let image = unsafe { &*image };
            // `crop` is global points; the image is `width`×`height` pixels of the window
            // frame, so shift by the frame origin and scale, clamped to the image.
            let cropped = crop.and_then(|rect| {
                let x = ((width as f64 / frame.width) * (rect.x - frame.x)).clamp(0.0, width as f64 - 1.0);
                let y = ((height as f64 / frame.height) * (rect.y - frame.y)).clamp(0.0, height as f64 - 1.0);
                let w = ((width as f64 / frame.width) * rect.width).clamp(1.0, width as f64 - x);
                let h = ((height as f64 / frame.height) * rect.height).clamp(1.0, height as f64 - y);
                CGImage::with_image_in_rect(Some(image), CGRect::new(CGPoint::new(x, y), CGSize::new(w, h)))
            });
            let result = encode(cropped.as_deref().unwrap_or(image))
                .map(|(jpeg, width, height)| Capture { jpeg, width, height, frame })
                .ok_or_else(|| "The captured window couldn't be encoded.".to_string());
            reply(&image_reply, result);
        });
        unsafe { SCScreenshotManager::captureImageWithFilter_configuration_completionHandler(&filter, &configuration, Some(&image_handler)) };
    });
    unsafe { SCShareableContent::getShareableContentExcludingDesktopWindows_onScreenWindowsOnly_completionHandler(true, false, &content_handler) };
}

/// Whether ScreenCaptureKit will actually hand out content: `CGPreflightScreenCaptureAccess`
/// can report a grant that no longer applies (for example after WackCode was rebuilt).
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
