//! The wire protocol between Simple Editor and `se-html`, shared by both (the app includes this file with
//! `#[path]`, so there is one definition). Length-prefixed messages over the helper's stdin / stdout:
//!
//! ```text
//! message = u32 LE total | u32 LE head_len | head (UTF-8 JSON object) | body (raw bytes, may be empty)
//! request head  = {"cmd": "load" | "frame" | ..., ...}              (see `main.rs` for each command)
//! response head = {"ok": true, ...}  or  {"ok": false, "error": "...", "panic": true|false}
//! ```
//! A response always follows its request, in order. The body of a `frame` response is the picture
//! (PNG or raw straight-alpha RGBA, as asked). Nothing else is ever written to stdout.

use serde_json::Value;
use std::io::{self, Read, Write};

/// Bumped when a command or field changes meaning; the app refuses a helper with another one.
pub const PROTOCOL: u32 = 1;
/// Largest message accepted (a 4K RGBA frame is 33 MB).
pub const MAX_MESSAGE: usize = 256 << 20;

pub fn write_msg(w: &mut impl Write, head: &Value, body: &[u8]) -> io::Result<()> {
    let h = head.to_string();
    let total = 4 + h.len() + body.len();
    let mut buf = Vec::with_capacity(4 + total.min(1 << 16));
    buf.extend_from_slice(&(total as u32).to_le_bytes());
    buf.extend_from_slice(&(h.len() as u32).to_le_bytes());
    buf.extend_from_slice(h.as_bytes());
    w.write_all(&buf)?;
    w.write_all(body)?;
    w.flush()
}

/// None at a clean end of input.
pub fn read_msg(r: &mut impl Read) -> io::Result<Option<(Value, Vec<u8>)>> {
    let mut len = [0u8; 4];
    match r.read_exact(&mut len) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let total = u32::from_le_bytes(len) as usize;
    if !(4..=MAX_MESSAGE).contains(&total) {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "bad message length"));
    }
    let mut buf = vec![0u8; total];
    r.read_exact(&mut buf)?;
    let hl = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
    if 4 + hl > total {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "bad head length"));
    }
    let head: Value = serde_json::from_slice(&buf[4..4 + hl]).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let body = buf.split_off(4 + hl);
    Ok(Some((head, body)))
}
