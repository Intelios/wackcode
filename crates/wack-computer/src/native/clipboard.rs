//! The general pasteboard, for `computer_clipboard`.
//!
//! Invariant: an item a password manager marks concealed or transient (the nspasteboard.org
//! types) is never read: the read answers that the clipboard holds something private.

use objc2_app_kit::{NSPasteboard, NSPasteboardTypeString};
use objc2_foundation::NSString;

/// Types that mark a clipboard item as not for reading (nspasteboard.org).
const PRIVATE: [&str; 2] = ["org.nspasteboard.ConcealedType", "org.nspasteboard.TransientType"];

pub enum Read {
    Text(String),
    Empty,
    /// A concealed or transient item: never read.
    Private,
}

pub fn read() -> Read {
    let board = NSPasteboard::generalPasteboard();
    if let Some(types) = board.types() {
        let private = types.iter().any(|kind| PRIVATE.contains(&kind.to_string().as_str()));
        if private {
            return Read::Private;
        }
    }
    match board.stringForType(unsafe { NSPasteboardTypeString }) {
        Some(text) if !text.to_string().is_empty() => Read::Text(text.to_string()),
        _ => Read::Empty,
    }
}

pub fn write(text: &str) -> bool {
    let board = NSPasteboard::generalPasteboard();
    board.clearContents();
    board.setString_forType(&NSString::from_str(text), unsafe { NSPasteboardTypeString })
}
