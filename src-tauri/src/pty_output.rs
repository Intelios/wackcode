//! Shared PTY transport helpers. Replay and streaming must use the caller's same lock.

use portable_pty::PtySize;
use std::{collections::VecDeque, io::Read};

pub(crate) const SCROLLBACK_BYTES: usize = 256 * 1024;

pub(crate) fn size(cols: u16, rows: u16) -> PtySize {
    PtySize { rows: rows.clamp(2, 500), cols: cols.clamp(2, 500), pixel_width: 0, pixel_height: 0 }
}

pub(crate) fn append_scrollback(bytes: &mut VecDeque<u8>, data: &[u8]) {
    bytes.extend(data);
    let excess = bytes.len().saturating_sub(SCROLLBACK_BYTES);
    bytes.drain(..excess);
    // Never replay half a UTF-8 character after trimming the ring.
    while bytes.front().is_some_and(|byte| byte & 0xc0 == 0x80) { bytes.pop_front(); }
}

pub(crate) fn split_utf8(buffer: &[u8]) -> (String, Vec<u8>) {
    match std::str::from_utf8(buffer) {
        Ok(text) => (text.to_string(), Vec::new()),
        Err(error) => {
            let boundary = error.valid_up_to();
            let mut text = String::from_utf8_lossy(&buffer[..boundary]).into_owned();
            match error.error_len() {
                None => (text, buffer[boundary..].to_vec()),
                Some(invalid) => {
                    text.push('\u{FFFD}');
                    let (rest, tail) = split_utf8(&buffer[boundary + invalid..]);
                    text.push_str(&rest);
                    (text, tail)
                }
            }
        }
    }
}

pub(crate) fn read_output(mut reader: Box<dyn Read + Send>, mut push: impl FnMut(&str)) {
    let mut chunk = [0u8; 16 * 1024];
    let mut tail = Vec::new();
    loop {
        let Ok(read) = reader.read(&mut chunk) else { break };
        if read == 0 { break; }
        let mut pending = std::mem::take(&mut tail);
        pending.extend_from_slice(&chunk[..read]);
        let (text, rest) = split_utf8(&pending);
        tail = rest;
        if !text.is_empty() { push(&text); }
    }
    if !tail.is_empty() { push(&String::from_utf8_lossy(&tail)); }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_trims_at_a_character_boundary() {
        let mut bytes = VecDeque::new();
        let data = format!("🦆{}", "x".repeat(SCROLLBACK_BYTES - 2));
        append_scrollback(&mut bytes, data.as_bytes());
        assert!(std::str::from_utf8(bytes.make_contiguous()).is_ok());
        assert!(bytes.len() <= SCROLLBACK_BYTES);
    }
}
