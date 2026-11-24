//! The user's keyboard layout, read through `UCKeyTranslate`: which key types which character,
//! so a shortcut like ⌘S lands on the key that types "s" there (`keys::Layout`).
//!
//! Invariant: the Text Input Sources calls must run on the main thread (macOS asserts otherwise),
//! so [`current`] is called there: when the app's computer is made, and whenever the window comes
//! forward.

use std::ffi::c_void;

use crate::keys::Layout;

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn TISCopyCurrentKeyboardLayoutInputSource() -> *mut c_void;
    fn TISGetInputSourceProperty(source: *mut c_void, key: *const c_void) -> *const c_void;
    static kTISPropertyUnicodeKeyLayoutData: *const c_void;
    fn LMGetKbdType() -> u8;
    fn UCKeyTranslate(
        layout: *const c_void,
        code: u16,
        action: u16,
        modifiers: u32,
        keyboard_type: u32,
        options: u32,
        dead_key_state: *mut u32,
        max_length: usize,
        actual_length: *mut usize,
        chars: *mut u16,
    ) -> i32;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFDataGetBytePtr(data: *const c_void) -> *const u8;
    fn CFRelease(object: *const c_void);
}

const KEY_ACTION_DOWN: u16 = 0;
const NO_DEAD_KEYS: u32 = 1;

/// The current layout's printable characters and the keys that type them, or ANSI when it can't
/// be read. Main thread only.
pub fn current() -> Layout {
    let mut pairs = Vec::new();
    unsafe {
        let source = TISCopyCurrentKeyboardLayoutInputSource();
        if source.is_null() {
            return Layout::ansi();
        }
        let data = TISGetInputSourceProperty(source, kTISPropertyUnicodeKeyLayoutData);
        if !data.is_null() {
            let layout = CFDataGetBytePtr(data) as *const c_void;
            let keyboard = LMGetKbdType() as u32;
            for code in 0u16..128 {
                let mut dead = 0u32;
                let mut length = 0usize;
                let mut chars = [0u16; 4];
                let status = UCKeyTranslate(layout, code, KEY_ACTION_DOWN, 0, keyboard, NO_DEAD_KEYS, &mut dead, chars.len(), &mut length, chars.as_mut_ptr());
                if status != 0 || length != 1 {
                    continue;
                }
                if let Some(ch) = char::from_u32(chars[0] as u32).filter(|ch| !ch.is_control()) {
                    pairs.push((ch, code));
                }
            }
        }
        CFRelease(source);
    }
    let layout = Layout::from_pairs(pairs);
    if layout.is_empty() { Layout::ansi() } else { layout }
}
