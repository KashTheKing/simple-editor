//! ---- ws:parity-editing ----
//! When a media file was made, for the Library's "Date Created" and "Date Recorded" sorts. Read once per
//! file on one worker thread, never on the caller's: a Library sort asks for every row each frame, and a
//! stat or header read on a sleeping disk takes tens of milliseconds.
//!
//! - created: the file's creation time on disk (when it was copied onto this PC).
//! - recorded: the time the camera wrote into the file - an MP4/MOV `mvhd` creation time, a JPEG's EXIF
//!   date - else the file's modified time, which cameras and phones keep as the shot time.
use std::collections::BTreeMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::Mutex;
use std::time::UNIX_EPOCH;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Dates {
    /// Unix seconds; None = unknown.
    pub created: Option<i64>,
    pub recorded: Option<i64>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// The dates of `path`, or None until the worker has read them (the first call queues the read).
pub fn dates(path: &str) -> Option<Dates> {
    use std::sync::mpsc::{channel, Sender};
    static KNOWN: Mutex<BTreeMap<String, Option<Dates>>> = Mutex::new(BTreeMap::new());
    static WORKER: std::sync::OnceLock<Mutex<Sender<String>>> = std::sync::OnceLock::new();
    let mut known = lock(&KNOWN);
    if let Some(d) = known.get(path) {
        return *d;
    }
    known.insert(path.to_string(), None);
    drop(known);
    let tx = WORKER.get_or_init(|| {
        let (tx, rx) = channel::<String>();
        let _ = std::thread::Builder::new().name("media-dates".into()).spawn(move || {
            for p in rx {
                let d = read_dates(Path::new(&p));
                lock(&KNOWN).insert(p, Some(d));
            }
        });
        Mutex::new(tx)
    });
    let _ = lock(tx).send(path.to_string());
    None
}

/// Read both dates now (blocking - the worker's job, and tests').
pub fn read_dates(path: &Path) -> Dates {
    let meta = std::fs::metadata(path).ok();
    let secs = |t: std::io::Result<std::time::SystemTime>| {
        t.ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64)
    };
    let created = meta.as_ref().and_then(|m| secs(m.created()));
    let modified = meta.as_ref().and_then(|m| secs(m.modified()));
    let ext = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    let embedded = match ext.as_str() {
        "mp4" | "mov" | "m4v" | "3gp" | "m4a" => std::fs::File::open(path).ok().and_then(|mut f| mvhd_time(&mut f)),
        "jpg" | "jpeg" => std::fs::File::open(path).ok().and_then(|f| {
            let mut head = Vec::new();
            f.take(256 * 1024).read_to_end(&mut head).ok()?;
            exif_time(&head)
        }),
        _ => None,
    };
    Dates { created: created.or(modified), recorded: embedded.or(modified) }
}

/// Seconds from 1904-01-01 (QuickTime's epoch) to 1970-01-01.
const QT_EPOCH: i64 = 2_082_844_800;

/// The `moov/mvhd` creation time of an MP4/MOV, as unix seconds (None when absent or zero).
pub fn mvhd_time<R: Read + Seek>(r: &mut R) -> Option<i64> {
    let end = r.seek(SeekFrom::End(0)).ok()?;
    let moov = find_box(r, 0, end, b"moov")?;
    let (start, _) = find_box(r, moov.0, moov.1, b"mvhd")?;
    r.seek(SeekFrom::Start(start)).ok()?;
    let mut vf = [0u8; 4];
    r.read_exact(&mut vf).ok()?;
    let t = if vf[0] == 1 {
        let mut b = [0u8; 8];
        r.read_exact(&mut b).ok()?;
        u64::from_be_bytes(b) as i64
    } else {
        let mut b = [0u8; 4];
        r.read_exact(&mut b).ok()?;
        u32::from_be_bytes(b) as i64
    };
    (t > QT_EPOCH).then_some(t - QT_EPOCH)
}

/// The first box named `name` among the boxes in `[from, to)`: its body as `(start, end)`.
fn find_box<R: Read + Seek>(r: &mut R, mut at: u64, to: u64, name: &[u8; 4]) -> Option<(u64, u64)> {
    while at + 8 <= to {
        r.seek(SeekFrom::Start(at)).ok()?;
        let mut h = [0u8; 8];
        r.read_exact(&mut h).ok()?;
        let mut size = u32::from_be_bytes([h[0], h[1], h[2], h[3]]) as u64;
        let mut body = at + 8;
        if size == 1 {
            let mut b = [0u8; 8];
            r.read_exact(&mut b).ok()?;
            size = u64::from_be_bytes(b);
            body += 8;
        } else if size == 0 {
            size = to - at; // to the end
        }
        if size < body - at {
            return None; // corrupt
        }
        if &h[4..8] == name {
            return Some((body, (at + size).min(to)));
        }
        at += size;
    }
    None
}

/// A JPEG's EXIF date (`YYYY:MM:DD HH:MM:SS`, the camera's local time read as UTC) in the first bytes.
/// Takes the first such stamp after the `Exif` header: DateTime and DateTimeOriginal agree for a photo
/// straight off a camera.
pub fn exif_time(head: &[u8]) -> Option<i64> {
    let start = head.windows(6).position(|w| w == b"Exif\0\0")?;
    head[start..].windows(19).find_map(|w| {
        let s = std::str::from_utf8(w).ok()?;
        let b = s.as_bytes();
        let shape = b.iter().enumerate().all(|(i, c)| match i {
            4 | 7 | 13 | 16 => *c == b':',
            10 => *c == b' ',
            _ => c.is_ascii_digit(),
        });
        if !shape {
            return None;
        }
        let n = |a: usize, b: usize| s[a..b].parse::<i64>().ok();
        let (y, mo, d, h, mi, se) = (n(0, 4)?, n(5, 7)?, n(8, 10)?, n(11, 13)?, n(14, 16)?, n(17, 19)?);
        if y < 1900 || !(1..=12).contains(&mo) || !(1..=31).contains(&d) {
            return None;
        }
        Some(days_from_civil(y, mo, d) * 86_400 + h * 3600 + mi * 60 + se)
    })
}

/// Days from 1970-01-01 to a proleptic Gregorian date (Howard Hinnant's algorithm).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn bx(name: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut v = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        v.extend_from_slice(name);
        v.extend_from_slice(body);
        v
    }

    #[test]
    fn mvhd_creation_time_is_read_through_moov() {
        // 2024-03-05 12:00:00 UTC
        let unix = days_from_civil(2024, 3, 5) * 86_400 + 12 * 3600;
        let mut mvhd = vec![0, 0, 0, 0];
        mvhd.extend_from_slice(&((unix + QT_EPOCH) as u32).to_be_bytes());
        mvhd.extend_from_slice(&[0; 16]);
        let mut file = bx(b"ftyp", b"isom0000");
        file.extend(bx(b"mdat", &[7; 40]));
        file.extend(bx(b"moov", &[bx(b"trak", &[1; 12]), bx(b"mvhd", &mvhd)].concat()));
        assert_eq!(mvhd_time(&mut Cursor::new(file)), Some(unix));
        // a zero time (many encoders write none) is unknown, not 1904
        let zero = bx(b"moov", &bx(b"mvhd", &[0; 24]));
        assert_eq!(mvhd_time(&mut Cursor::new(zero)), None);
        assert_eq!(mvhd_time(&mut Cursor::new(b"not a movie".to_vec())), None);
    }

    #[test]
    fn exif_date_is_found_after_the_exif_header() {
        let mut jpg = vec![0xFF, 0xD8, 0xFF, 0xE1, 0, 0];
        jpg.extend_from_slice(b"Exif\0\0MM\0*junk2023:12:31 23:59:58\0more");
        let want = days_from_civil(2023, 12, 31) * 86_400 + 23 * 3600 + 59 * 60 + 58;
        assert_eq!(exif_time(&jpg), Some(want));
        assert_eq!(exif_time(b"2023:12:31 23:59:58 without an exif header"), None);
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2000, 3, 1), 11_017);
    }
}
