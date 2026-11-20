//! ws:parity-io: Contour ShuttlePRO / ShuttlePRO v2 / ShuttleXpress jog-shuttle controllers.
//!
//! The device is a plain USB HID with vendor id 0x0B33. Windows lets any process open it for reading (it is
//! not a keyboard or mouse, so the OS does not hold it exclusively), so the reader is `std::fs::File` on the
//! HID interface path plus one `cfgmgr32` call to list those paths: no new crate, no window-message hook.
//! The reader thread blocks in `read`, so it costs nothing while the dial is still; with no device plugged
//! in it looks again every few seconds.
//!
//! Report layout (all three models, after Windows' leading report-id byte 0): `[shuttle: i8 -7..=7,
//! jog: u8 counter, 0, buttons 1-8, buttons 9-15]`. The jog is an absolute counter that wraps, so a turn is
//! the wrapping difference from the last report. `Decoder` turns reports into edge events; `ui::app::io_ctl`
//! maps them to playback (shuttle = rate, jog = frame steps) and to `Settings.shuttle_buttons` actions.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub const VENDOR: u16 = 0x0B33;

/// One input report.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Report {
    /// -7 (full left) ..= 7 (full right), 0 = centred.
    pub shuttle: i8,
    /// Absolute jog counter (wraps at 256).
    pub jog: u8,
    /// Bit n = button n+1 held.
    pub buttons: u16,
}

/// What changed between two reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// The shuttle ring moved to this position (-7..=7; 0 = let go).
    Shuttle(i8),
    /// The jog wheel turned this many detents (negative = left).
    Jog(i32),
    /// Button 1..=15 went down (true) or up (false).
    Button(u8, bool),
}

/// A raw read: 5 bytes, or 6 with Windows' report-id 0 in front. None = not a ShuttlePRO report.
pub fn parse(buf: &[u8]) -> Option<Report> {
    let b = match buf.len() {
        6.. if buf[0] == 0 => &buf[1..],
        5.. => buf,
        _ => return None,
    };
    let shuttle = (b[0] as i8).clamp(-7, 7);
    Some(Report { shuttle, jog: b[1], buttons: b[3] as u16 | (b[4] as u16) << 8 })
}

/// Reports in, edge events out. The first report only sets the baseline (a jog counter is not a turn).
#[derive(Default)]
pub struct Decoder {
    last: Option<Report>,
}

impl Decoder {
    pub fn feed(&mut self, r: Report) -> Vec<Event> {
        let mut out = Vec::new();
        let last = self.last.replace(r);
        let prev = last.unwrap_or(Report { jog: r.jog, ..Default::default() });
        if r.shuttle != prev.shuttle {
            out.push(Event::Shuttle(r.shuttle));
        }
        let turn = r.jog.wrapping_sub(prev.jog) as i8 as i32;
        if turn != 0 {
            out.push(Event::Jog(turn));
        }
        let changed = r.buttons ^ prev.buttons;
        for bit in 0..15u8 {
            if changed & (1 << bit) != 0 {
                out.push(Event::Button(bit + 1, r.buttons & (1 << bit) != 0));
            }
        }
        out
    }
}

/// Playback rate for a shuttle position: a slow crawl near the centre, up to 16x at the end stop.
pub fn rate(pos: i8) -> f64 {
    const LADDER: [f64; 7] = [0.25, 0.5, 1.0, 2.0, 4.0, 8.0, 16.0];
    match pos {
        0 => 0.0,
        p => LADDER[(p.unsigned_abs().min(7) - 1) as usize] * p.signum() as f64,
    }
}

/// The model's name from its product id (for Settings and the toast on connect).
pub fn model(pid: u16) -> &'static str {
    match pid {
        0x0010 => "ShuttlePRO",
        0x0020 => "ShuttleXpress",
        0x0030 => "ShuttlePRO v2",
        _ => "Contour Shuttle",
    }
}

static STATUS: std::sync::Mutex<Option<&'static str>> = std::sync::Mutex::new(None);

/// The connected model (None = nothing plugged in or the reader is off), for Settings.
pub fn status() -> Option<&'static str> {
    *STATUS.lock().unwrap_or_else(|e| e.into_inner())
}
pub fn set_status(model: Option<&'static str>) {
    *STATUS.lock().unwrap_or_else(|e| e.into_inner()) = model;
}

/// Number of buttons (bindings shown in Settings).
pub const BUTTONS: usize = 15;

/// What each button does out of the box (`Action` ids, button 1 first). The ShuttleXpress's five buttons
/// report as 5..=9, which land on the transport row here.
pub fn default_buttons() -> Vec<String> {
    [
        "mark_in",
        "mark_out",
        "clear_in_out",
        "add_marker",
        "go_start",
        "prev_cut",
        "play_pause",
        "next_cut",
        "go_end",
        "split",
        "ripple_delete",
        "undo",
        "redo",
        "zoom_out",
        "zoom_in",
    ]
    .map(String::from)
    .to_vec()
}

/// (vendor, product) from a HID interface path such as `\\?\hid#vid_0b33&pid_0030#7&...`.
pub fn ids_of(path: &str) -> Option<(u16, u16)> {
    let p = path.to_ascii_lowercase();
    let hex = |key: &str| {
        let at = p.find(key)? + key.len();
        u16::from_str_radix(p.get(at..at + 4)?, 16).ok()
    };
    Some((hex("vid_")?, hex("pid_")?))
}

/// What the reader thread reports.
pub enum Msg {
    /// A controller is open: its model name.
    Connected(&'static str),
    Disconnected,
    Event(Event),
}

/// The reader: a thread that finds a Contour device, reads it until it goes away, and looks again.
/// Dropping the handle (or `stop`) ends the thread once its current read or wait returns.
pub struct Reader {
    stop: Arc<AtomicBool>,
}

impl Reader {
    /// `send` runs on the reader thread for every message (it should queue and wake the UI).
    pub fn start(send: impl Fn(Msg) + Send + 'static) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let _ = std::thread::Builder::new().name("shuttle".into()).spawn(move || run(&flag, &send));
        Reader { stop }
    }
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl Drop for Reader {
    fn drop(&mut self) {
        self.stop();
    }
}

fn run(stop: &AtomicBool, send: &dyn Fn(Msg)) {
    use std::io::Read;
    while !stop.load(Ordering::Relaxed) {
        let found = hid_paths().into_iter().find_map(|p| {
            let (v, pid) = ids_of(&p)?;
            (v == VENDOR).then_some((p, pid))
        });
        let Some((path, pid)) = found else {
            sleep_unless(stop, 3.0);
            continue;
        };
        let Ok(mut f) = std::fs::OpenOptions::new().read(true).open(&path) else {
            sleep_unless(stop, 3.0);
            continue;
        };
        send(Msg::Connected(model(pid)));
        let (mut dec, mut buf) = (Decoder::default(), [0u8; 64]);
        loop {
            match f.read(&mut buf) {
                Ok(n) if n > 0 => {
                    if stop.load(Ordering::Relaxed) {
                        return;
                    }
                    if let Some(r) = parse(&buf[..n]) {
                        for e in dec.feed(r) {
                            send(Msg::Event(e));
                        }
                    }
                }
                _ => break, // unplugged
            }
        }
        send(Msg::Disconnected);
        sleep_unless(stop, 1.0);
    }
}

fn sleep_unless(stop: &AtomicBool, secs: f64) {
    let end = std::time::Instant::now() + std::time::Duration::from_secs_f64(secs);
    while !stop.load(Ordering::Relaxed) && std::time::Instant::now() < end {
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
}

windows::core::link!("cfgmgr32.dll" "system" fn CM_Get_Device_Interface_List_SizeW(len: *mut u32, guid: *const windows::core::GUID, device: *const u16, flags: u32) -> u32);
windows::core::link!("cfgmgr32.dll" "system" fn CM_Get_Device_Interface_ListW(guid: *const windows::core::GUID, device: *const u16, buffer: *mut u16, len: u32, flags: u32) -> u32);

/// Every present HID interface path (`GUID_DEVINTERFACE_HID`).
fn hid_paths() -> Vec<String> {
    const HID: windows::core::GUID = windows::core::GUID::from_u128(0x4d1e55b2_f16f_11cf_88cb_001111000030);
    let mut len = 0u32;
    // SAFETY: plain out-parameter calls; the buffer is sized by the first call.
    unsafe {
        if CM_Get_Device_Interface_List_SizeW(&mut len, &HID, std::ptr::null(), 0) != 0 || len == 0 {
            return Vec::new();
        }
        let mut buf = vec![0u16; len as usize];
        if CM_Get_Device_Interface_ListW(&HID, std::ptr::null(), buf.as_mut_ptr(), len, 0) != 0 {
            return Vec::new();
        }
        split_multi_sz(&buf)
    }
}

/// A double-NUL-terminated UTF-16 list.
fn split_multi_sz(buf: &[u16]) -> Vec<String> {
    buf.split(|c| *c == 0).filter(|s| !s.is_empty()).map(String::from_utf16_lossy).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_with_and_without_the_report_id() {
        let r = Report { shuttle: -3, jog: 200, buttons: 0x0105 };
        assert_eq!(parse(&[0xFD, 200, 0, 0x05, 0x01]), Some(r));
        assert_eq!(parse(&[0, 0xFD, 200, 0, 0x05, 0x01]), Some(r));
        assert_eq!(parse(&[1, 2, 3]), None);
        assert_eq!(parse(&[9, 0, 0, 0, 0]).unwrap().shuttle, 7, "clamped");
    }

    #[test]
    fn decoder_reports_edges_and_wrapping_jog() {
        let mut d = Decoder::default();
        // baseline: a jog counter of 250 is not a turn, a held button is a press
        assert_eq!(d.feed(Report { shuttle: 0, jog: 250, buttons: 0b10 }), vec![Event::Button(2, true)]);
        // 250 -> 3 is +9 across the wrap; button 2 released, 15 pressed
        let e = d.feed(Report { shuttle: 0, jog: 3, buttons: 1 << 14 });
        assert_eq!(e, vec![Event::Jog(9), Event::Button(2, false), Event::Button(15, true)]);
        // back the other way, and the shuttle moves
        assert_eq!(d.feed(Report { shuttle: -2, jog: 1, buttons: 1 << 14 }), vec![Event::Shuttle(-2), Event::Jog(-2)]);
        assert!(d.feed(Report { shuttle: -2, jog: 1, buttons: 1 << 14 }).is_empty(), "no change, no events");
        assert_eq!(d.feed(Report { shuttle: 0, jog: 1, buttons: 1 << 14 }), vec![Event::Shuttle(0)]);
    }

    #[test]
    fn shuttle_ladder_is_symmetric_and_stops_at_centre() {
        assert_eq!(rate(0), 0.0);
        assert_eq!((rate(1), rate(3), rate(7)), (0.25, 1.0, 16.0));
        assert_eq!((rate(-1), rate(-7)), (-0.25, -16.0));
        for p in 1..=7 {
            assert!(rate(p) > rate(p - 1), "monotonic");
        }
    }

    #[test]
    fn ids_and_paths() {
        let p = r"\\?\hid#vid_0b33&pid_0030#7&2f0b7c9a&0&0000#{4d1e55b2-f16f-11cf-88cb-001111000030}";
        assert_eq!(ids_of(p), Some((0x0B33, 0x0030)));
        assert_eq!(model(0x0030), "ShuttlePRO v2");
        assert_eq!(ids_of(r"\\?\hid#acpi0c50"), None);
        let list: Vec<u16> = "a\0bc\0\0".encode_utf16().collect();
        assert_eq!(split_multi_sz(&list), vec!["a".to_string(), "bc".to_string()]);
    }

    #[test]
    fn default_buttons_are_real_actions() {
        let b = default_buttons();
        assert_eq!(b.len(), BUTTONS);
        for id in &b {
            assert!(crate::hotkeys::Action::from_id(id).is_some(), "{id} is not an Action id");
        }
    }
}
