//! ---- ws:music-burn ----
//! Burn to Disc. Everything before the drive is shared by a real burn and the "Write disc image" target:
//!
//! * **Audio CD (Red Book)**: each track is converted by ffmpeg to 44.1 kHz 16-bit stereo PCM, padded to
//!   whole 2352-byte sectors (and to the 4 s minimum), laid out with the chosen gaps and checked against
//!   the disc (74 / 80 min). The image target writes those sectors as one `.bin` plus a `.cue` (with
//!   CD-Text TITLE / PERFORMER lines); a drive gets the same track files through IMAPI2 track-at-once
//!   (`imapi::burn_audio`).
//! * **Data disc**: files, folders and playlists (their songs plus an `.m3u8`) go into an IMAPI2
//!   file-system image (ISO 9660 + Joliet + UDF, `imapi::build_image`); its stream is either written to a
//!   `.iso` or burned.
//! * **Video DVD**: not offered. A player-compatible DVD needs VIDEO_TS authoring (IFO/BUP menus, VOB
//!   cells), which ffmpeg alone does not write; without a new dependency the honest answer is a data
//!   disc holding the video files.

pub mod imapi;

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// Bytes in one audio CD sector (1/75 s of 44.1 kHz 16-bit stereo).
pub const AUDIO_SECTOR: u64 = 2352;
pub const SECTORS_PER_SEC: u64 = 75;
/// The 2 s before track 1 every audio CD has.
pub const LEAD_PREGAP: u64 = 150;
/// Red Book: a track is at least 4 s.
pub const MIN_TRACK: u64 = 4 * SECTORS_PER_SEC;
pub const MAX_TRACKS: usize = 99;
/// Data sector size (ISO 9660 / UDF logical block).
pub const DATA_BLOCK: u64 = 2048;

/// Recordable disc sizes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum DiscSize {
    Cd74,
    Cd80,
    Dvd,
    DvdDl,
    Bd25,
}

impl DiscSize {
    pub const ALL: [DiscSize; 5] = [DiscSize::Cd80, DiscSize::Cd74, DiscSize::Dvd, DiscSize::DvdDl, DiscSize::Bd25];
    pub fn label(self) -> &'static str {
        match self {
            DiscSize::Cd74 => "CD 74 min / 650 MB",
            DiscSize::Cd80 => "CD 80 min / 700 MB",
            DiscSize::Dvd => "DVD 4.7 GB",
            DiscSize::DvdDl => "DVD DL 8.5 GB",
            DiscSize::Bd25 => "Blu-ray 25 GB",
        }
    }
    pub fn is_cd(self) -> bool {
        matches!(self, DiscSize::Cd74 | DiscSize::Cd80)
    }
    /// Audio sectors (CDs only).
    pub fn audio_sectors(self) -> u64 {
        match self {
            DiscSize::Cd74 => 74 * 60 * SECTORS_PER_SEC,
            DiscSize::Cd80 => 80 * 60 * SECTORS_PER_SEC,
            _ => 0,
        }
    }
    /// 2048-byte data blocks.
    pub fn data_blocks(self) -> u64 {
        match self {
            DiscSize::Cd74 => 333_000,
            DiscSize::Cd80 => 359_846,
            DiscSize::Dvd => 2_295_104,
            DiscSize::DvdDl => 4_173_824,
            DiscSize::Bd25 => 12_219_392,
        }
    }
    pub fn from_name(s: &str) -> Option<DiscSize> {
        Some(match s.to_ascii_lowercase().replace([' ', '-', '_'], "").as_str() {
            "cd74" | "cd650" => DiscSize::Cd74,
            "cd" | "cd80" | "cd700" => DiscSize::Cd80,
            "dvd" | "dvd47" => DiscSize::Dvd,
            "dvddl" | "dvd85" => DiscSize::DvdDl,
            "bd" | "bd25" | "bluray" => DiscSize::Bd25,
            _ => return None,
        })
    }
}

/// `mm:ss:ff` (75 frames a second): the cue sheet's time format.
pub fn msf(sectors: u64) -> String {
    let f = sectors % SECTORS_PER_SEC;
    let s = sectors / SECTORS_PER_SEC;
    format!("{:02}:{:02}:{:02}", s / 60, s % 60, f)
}

pub fn parse_msf(s: &str) -> Option<u64> {
    let mut it = s.trim().split(':').map(|x| x.parse::<u64>().ok());
    let (m, sec, f) = (it.next()??, it.next()??, it.next()??);
    Some((m * 60 + sec) * SECTORS_PER_SEC + f)
}

/// `m:ss` for people.
pub fn clock(secs: f64) -> String {
    let s = secs.max(0.0).round() as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

// ------------------------------------------------------------------------------------------------ audio CD

/// One track to burn.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CdTrack {
    pub path: PathBuf,
    pub title: String,
    pub performer: String,
    /// Length in seconds as probed (before conversion) - for the capacity check up front.
    pub seconds: f64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AudioCd {
    pub title: String,
    pub performer: String,
    pub tracks: Vec<CdTrack>,
    /// Seconds of silence before each track after the first (0 = gapless).
    pub gap: f64,
}

/// Where each track lands, in sectors from the start of the program area (after the lead pregap).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TrackPlace {
    /// Start of the gap (INDEX 00); equals `start` when there is none.
    pub gap_start: u64,
    /// INDEX 01.
    pub start: u64,
    pub sectors: u64,
}

pub fn sectors_for_bytes(bytes: u64) -> u64 {
    bytes.div_ceil(AUDIO_SECTOR)
}

pub fn sectors_for_secs(secs: f64) -> u64 {
    (secs.max(0.0) * SECTORS_PER_SEC as f64).ceil() as u64
}

/// Lay tracks of `lens` sectors (each raised to the 4 s minimum) out with `gap` seconds between them.
pub fn layout(lens: &[u64], gap: f64) -> Vec<TrackPlace> {
    let gap = sectors_for_secs(gap);
    let mut at = 0;
    lens.iter()
        .enumerate()
        .map(|(i, &n)| {
            let gap_start = at;
            if i > 0 {
                at += gap;
            }
            let p = TrackPlace { gap_start, start: at, sectors: n.max(MIN_TRACK) };
            at += p.sectors;
            p
        })
        .collect()
}

/// Sectors the whole disc uses: the lead pregap plus the program.
pub fn total_sectors(places: &[TrackPlace]) -> u64 {
    LEAD_PREGAP + places.last().map_or(0, |p| p.start + p.sectors)
}

/// Will it fit? A clean message when it does not.
pub fn check_audio_capacity(places: &[TrackPlace], size: DiscSize) -> Result<(), String> {
    if places.is_empty() {
        return Err("the playlist is empty".into());
    }
    if places.len() > MAX_TRACKS {
        return Err(format!("an audio CD holds at most {MAX_TRACKS} tracks; this has {}", places.len()));
    }
    if !size.is_cd() {
        return Err("audio CDs are CDs: pick CD 74 or CD 80".into());
    }
    let need = total_sectors(places);
    let have = size.audio_sectors();
    if need > have {
        return Err(format!(
            "Too long for this disc: {} of music, a {} holds {}. Remove {} or use an 80 min CD.",
            clock(need as f64 / 75.0),
            size.label(),
            clock(have as f64 / 75.0),
            clock((need - have) as f64 / 75.0)
        ));
    }
    Ok(())
}

/// Check from probed lengths before anything is converted.
pub fn check_audio_estimate(cd: &AudioCd, size: DiscSize) -> Result<Vec<TrackPlace>, String> {
    let lens: Vec<u64> = cd.tracks.iter().map(|t| sectors_for_secs(t.seconds)).collect();
    let places = layout(&lens, cd.gap);
    check_audio_capacity(&places, size)?;
    Ok(places)
}

fn cue_text(s: &str) -> String {
    s.replace('"', "'").replace(['\r', '\n'], " ")
}

/// The cue sheet for a single `.bin` (CD-Text TITLE / PERFORMER at disc and track level).
pub fn cue_sheet(cd: &AudioCd, places: &[TrackPlace], bin_name: &str) -> String {
    let mut s = String::new();
    if !cd.performer.is_empty() {
        s += &format!("PERFORMER \"{}\"\n", cue_text(&cd.performer));
    }
    if !cd.title.is_empty() {
        s += &format!("TITLE \"{}\"\n", cue_text(&cd.title));
    }
    s += &format!("FILE \"{}\" BINARY\n", cue_text(bin_name));
    for (i, (t, p)) in cd.tracks.iter().zip(places).enumerate() {
        s += &format!("  TRACK {:02} AUDIO\n", i + 1);
        if !t.title.is_empty() {
            s += &format!("    TITLE \"{}\"\n", cue_text(&t.title));
        }
        if !t.performer.is_empty() {
            s += &format!("    PERFORMER \"{}\"\n", cue_text(&t.performer));
        }
        if p.gap_start < p.start {
            s += &format!("    INDEX 00 {}\n", msf(p.gap_start));
        }
        s += &format!("    INDEX 01 {}\n", msf(p.start));
    }
    s
}

/// A track parsed back from a cue sheet.
#[derive(Clone, Debug, PartialEq)]
pub struct CueTrack {
    pub number: u32,
    pub title: String,
    pub performer: String,
    pub index0: Option<u64>,
    pub index1: u64,
}

/// Read a cue sheet's tracks back (tests and the image check).
pub fn parse_cue(text: &str) -> Vec<CueTrack> {
    let mut out: Vec<CueTrack> = Vec::new();
    let unq = |s: &str| s.trim().trim_matches('"').to_string();
    for line in text.lines() {
        let l = line.trim();
        let (word, rest) = l.split_once(' ').unwrap_or((l, ""));
        match word {
            "TRACK" => {
                let n = rest.split_whitespace().next().and_then(|n| n.parse().ok()).unwrap_or(0);
                out.push(CueTrack { number: n, title: String::new(), performer: String::new(), index0: None, index1: 0 });
            }
            "TITLE" if !out.is_empty() => out.last_mut().unwrap().title = unq(rest),
            "PERFORMER" if !out.is_empty() => out.last_mut().unwrap().performer = unq(rest),
            "INDEX" => {
                let mut it = rest.split_whitespace();
                let (k, t) = (it.next().unwrap_or(""), it.next().and_then(parse_msf));
                if let (Some(tr), Some(t)) = (out.last_mut(), t) {
                    match k {
                        "00" => tr.index0 = Some(t),
                        "01" => tr.index1 = t,
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// Track lengths in sectors from a cue sheet and its bin's size (a track runs to the next one's gap).
pub fn cue_lengths(tracks: &[CueTrack], bin_bytes: u64) -> Vec<u64> {
    let end = bin_bytes / AUDIO_SECTOR;
    (0..tracks.len())
        .map(|i| {
            let next = tracks.get(i + 1).map_or(end, |n| n.index0.unwrap_or(n.index1));
            next.saturating_sub(tracks[i].index1)
        })
        .collect()
}

/// Convert one file to raw CD audio (`s16le`, 44.1 kHz, stereo), padded to whole sectors and to the
/// 4 s minimum. Returns the sector count. `cancel` is polled while ffmpeg runs.
pub fn convert_track(src: &Path, out: &Path, cancel: &dyn Fn() -> bool) -> Result<u64, String> {
    let exe = crate::media::ffpipe::ffmpeg_exe().ok_or("ffmpeg.exe not found")?;
    let mut child = crate::media::ffpipe::command(&exe)
        .args(["-nostdin", "-hide_banner", "-v", "error", "-y", "-i"])
        .arg(src)
        .args(["-vn", "-map", "0:a:0", "-ac", "2", "-ar", "44100", "-c:a", "pcm_s16le", "-f", "s16le"])
        .arg(out)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("ffmpeg: {e}"))?;
    loop {
        if let Some(st) = child.try_wait().map_err(|e| e.to_string())? {
            if !st.success() {
                let mut err = String::new();
                if let Some(mut e) = child.stderr.take() {
                    let _ = e.read_to_string(&mut err);
                }
                return Err(format!("can't convert {}: {}", src.display(), err.lines().last().unwrap_or("ffmpeg failed")));
            }
            break;
        }
        if cancel() {
            let _ = child.kill();
            let _ = child.wait();
            let _ = std::fs::remove_file(out);
            return Err(crate::engine::export::CANCELLED.into());
        }
        std::thread::sleep(std::time::Duration::from_millis(40));
    }
    pad_track(out)
}

/// Pad a raw track with silence to whole sectors and at least `MIN_TRACK`. Returns its sectors.
pub fn pad_track(path: &Path) -> Result<u64, String> {
    let len = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
    let sectors = sectors_for_bytes(len).max(MIN_TRACK);
    let want = sectors * AUDIO_SECTOR;
    if want > len {
        let mut f = std::fs::OpenOptions::new().append(true).open(path).map_err(|e| e.to_string())?;
        write_silence(&mut f, want - len)?;
    }
    Ok(sectors)
}

fn write_silence(w: &mut dyn Write, mut bytes: u64) -> Result<(), String> {
    let zero = [0u8; 64 * 1024];
    while bytes > 0 {
        let n = bytes.min(zero.len() as u64) as usize;
        w.write_all(&zero[..n]).map_err(|e| e.to_string())?;
        bytes -= n as u64;
    }
    Ok(())
}

/// Join converted tracks (each already whole sectors) into one `.bin`, with gap silence where `places`
/// put it, and write the `.cue` beside it.
pub fn write_bin_cue(
    cd: &AudioCd,
    raw: &[PathBuf],
    places: &[TrackPlace],
    bin: &Path,
    progress: &dyn Fn(f32),
) -> Result<PathBuf, String> {
    let mut out = std::io::BufWriter::new(std::fs::File::create(bin).map_err(|e| format!("{}: {e}", bin.display()))?);
    let total: u64 = places.last().map_or(1, |p| p.start + p.sectors).max(1);
    for (r, p) in raw.iter().zip(places) {
        write_silence(&mut out, (p.start - p.gap_start) * AUDIO_SECTOR)?;
        let mut f = std::fs::File::open(r).map_err(|e| e.to_string())?;
        let copied = std::io::copy(&mut f, &mut out).map_err(|e| e.to_string())?;
        // a raw file shorter than its place (should not happen after pad_track) is filled up
        write_silence(&mut out, (p.sectors * AUDIO_SECTOR).saturating_sub(copied))?;
        progress((p.start + p.sectors) as f32 / total as f32);
    }
    out.flush().map_err(|e| e.to_string())?;
    let cue = bin.with_extension("cue");
    let name = bin.file_name().and_then(|n| n.to_str()).unwrap_or("disc.bin");
    std::fs::write(&cue, cue_sheet(cd, places, name)).map_err(|e| e.to_string())?;
    Ok(cue)
}

// ------------------------------------------------------------------------------------------------ data disc

/// One thing to put on a data disc: a file or a folder (its whole tree), placed under `dest` ("" = root).
#[derive(Clone, Debug, PartialEq)]
pub struct DataItem {
    pub src: PathBuf,
    pub dest: String,
}

/// Bytes of the files under `p` (a file or a folder tree).
pub fn tree_bytes(p: &Path) -> u64 {
    match std::fs::metadata(p) {
        Ok(m) if m.is_file() => m.len(),
        Ok(m) if m.is_dir() => std::fs::read_dir(p).into_iter().flatten().flatten().map(|e| tree_bytes(&e.path())).sum(),
        _ => 0,
    }
}

/// Rough blocks a data disc needs: each file rounded up to a block, plus 1 % and 2 MB of file-system
/// overhead (ISO + Joliet + UDF structures). IMAPI's own count is exact; this is the early warning.
pub fn estimate_blocks(items: &[DataItem]) -> u64 {
    fn walk(p: &Path, n: &mut u64) {
        match std::fs::metadata(p) {
            Ok(m) if m.is_file() => *n += m.len().div_ceil(DATA_BLOCK).max(1),
            Ok(m) if m.is_dir() => {
                *n += 1;
                for e in std::fs::read_dir(p).into_iter().flatten().flatten() {
                    walk(&e.path(), n);
                }
            }
            _ => {}
        }
    }
    let mut n = 0;
    for it in items {
        walk(&it.src, &mut n);
    }
    n + n / 100 + 1024
}

pub fn check_data_capacity(blocks: u64, size: DiscSize) -> Result<(), String> {
    let have = size.data_blocks();
    if blocks > have {
        let mb = |b: u64| b * DATA_BLOCK / 1_000_000;
        return Err(format!(
            "Too big for this disc: about {} MB, a {} holds {} MB. Remove {} MB or pick a bigger disc.",
            mb(blocks),
            size.label(),
            mb(have),
            mb(blocks - have)
        ));
    }
    Ok(())
}

// ------------------------------------------------------------------------------------------------ ISO reader

/// Every file path on an ISO image (Joliet names when the image has them, else ISO 9660 names without
/// `;1`), folders as `dir/`. Reads only the volume descriptors and directory records.
pub fn iso_list(path: &Path) -> Result<Vec<String>, String> {
    use std::io::{Seek, SeekFrom};
    let mut f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut read_at = |lba: u64, len: usize| -> Result<Vec<u8>, String> {
        let mut b = vec![0u8; len];
        f.seek(SeekFrom::Start(lba * DATA_BLOCK)).map_err(|e| e.to_string())?;
        f.read_exact(&mut b).map_err(|e| e.to_string())?;
        Ok(b)
    };
    let (mut root, mut joliet) = (None, false);
    for lba in 16..64 {
        let d = read_at(lba, 2048)?;
        if &d[1..6] != b"CD001" {
            return Err("not an ISO 9660 image".into());
        }
        match d[0] {
            1 if root.is_none() => root = Some(dir_extent(&d[156..190])),
            // a Joliet supplementary descriptor: escape sequence %/@, %/C or %/E
            2 if d[88] == 0x25 && d[89] == 0x2f && matches!(d[90], 0x40 | 0x43 | 0x45) => {
                root = Some(dir_extent(&d[156..190]));
                joliet = true;
            }
            255 => break,
            _ => {}
        }
    }
    let (lba, len) = root.ok_or("no primary volume descriptor")?;
    let mut out = Vec::new();
    let mut stack = vec![(String::new(), lba, len)];
    let mut seen = 0;
    while let Some((prefix, lba, len)) = stack.pop() {
        seen += 1;
        if seen > 100_000 {
            return Err("directory loop".into());
        }
        let data = read_at(lba as u64, len as usize)?;
        let mut i = 0;
        while i < data.len() {
            let n = data[i] as usize;
            if n == 0 {
                // records never cross a block: skip to the next one
                i = (i / 2048 + 1) * 2048;
                continue;
            }
            let r = &data[i..(i + n).min(data.len())];
            i += n;
            if r.len() < 34 {
                break;
            }
            let id_len = r[32] as usize;
            let id = &r[33..(33 + id_len).min(r.len())];
            if id == [0] || id == [1] {
                continue; // . and ..
            }
            let name = if joliet {
                let u: Vec<u16> = id.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
                String::from_utf16_lossy(&u)
            } else {
                String::from_utf8_lossy(id).into_owned()
            };
            let name = name.split(';').next().unwrap_or("").trim_end_matches('.').to_string();
            let (elba, elen) = dir_extent(r);
            if r[25] & 2 != 0 {
                let p = format!("{prefix}{name}/");
                out.push(p.clone());
                stack.push((p, elba, elen));
            } else {
                out.push(format!("{prefix}{name}"));
            }
        }
    }
    out.sort();
    Ok(out)
}

/// (extent LBA, data length) of a directory record (both-endian fields, little-endian half).
fn dir_extent(r: &[u8]) -> (u32, u32) {
    let le = |o: usize| u32::from_le_bytes([r[o], r[o + 1], r[o + 2], r[o + 3]]);
    (le(2), le(10))
}

/// A file's bytes from an ISO 9660 image by its Joliet / ISO path (tests).
pub fn iso_read(path: &Path, want: &str) -> Result<Vec<u8>, String> {
    // ponytail: re-walks the tree for one file; only tests and the image check read files back
    use std::io::{Seek, SeekFrom};
    let mut f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut read_at = |lba: u64, len: usize| -> Result<Vec<u8>, String> {
        let mut b = vec![0u8; len];
        f.seek(SeekFrom::Start(lba * DATA_BLOCK)).map_err(|e| e.to_string())?;
        f.read_exact(&mut b).map_err(|e| e.to_string())?;
        Ok(b)
    };
    let (mut root, mut joliet) = (None, false);
    for lba in 16..64 {
        let d = read_at(lba, 2048)?;
        match d[0] {
            1 if root.is_none() => root = Some(dir_extent(&d[156..190])),
            2 if d[88] == 0x25 && d[89] == 0x2f => {
                root = Some(dir_extent(&d[156..190]));
                joliet = true;
            }
            255 => break,
            _ => {}
        }
    }
    let (mut lba, mut len) = root.ok_or("no volume descriptor")?;
    let parts: Vec<&str> = want.split('/').filter(|s| !s.is_empty()).collect();
    for (k, part) in parts.iter().enumerate() {
        let data = read_at(lba as u64, len as usize)?;
        let mut i = 0;
        let mut found = None;
        while i < data.len() {
            let n = data[i] as usize;
            if n == 0 {
                i = (i / 2048 + 1) * 2048;
                continue;
            }
            let r = &data[i..(i + n).min(data.len())];
            i += n;
            let id = &r[33..(33 + r[32] as usize).min(r.len())];
            let name = if joliet {
                let u: Vec<u16> = id.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
                String::from_utf16_lossy(&u)
            } else {
                String::from_utf8_lossy(id).into_owned()
            };
            if name.split(';').next().unwrap_or("").trim_end_matches('.').eq_ignore_ascii_case(part) {
                found = Some(dir_extent(r));
                break;
            }
        }
        let (l, n) = found.ok_or_else(|| format!("{want}: not on the image"))?;
        (lba, len) = (l, n);
        if k + 1 == parts.len() {
            return read_at(lba as u64, len as usize);
        }
    }
    Err("empty path".into())
}

// ------------------------------------------------------------------------------------------------ jobs

/// What to burn.
#[derive(Clone, Debug, PartialEq)]
pub enum BurnContent {
    Audio(AudioCd),
    Data { volume: String, items: Vec<DataItem> },
}

/// Where to: a drive (by IMAPI unique id) or an image file (`.iso`, or `.bin` + `.cue` for audio).
#[derive(Clone, Debug, PartialEq)]
pub enum BurnTarget {
    Drive { id: String, test_only: bool },
    Image(PathBuf),
}

#[derive(Clone, Debug, PartialEq)]
pub struct BurnOptions {
    pub size: DiscSize,
    /// Write speed in sectors per second; 0 = the drive's fastest.
    pub speed: i32,
    pub verify: bool,
    pub eject: bool,
    /// Close the disc (no more sessions can be added).
    pub close: bool,
}

impl Default for BurnOptions {
    fn default() -> Self {
        BurnOptions { size: DiscSize::Cd80, speed: 0, verify: true, eject: true, close: true }
    }
}

/// Run a burn (or an image write) on this thread, reporting into `prog`.
pub fn run(content: &BurnContent, target: &BurnTarget, opts: &BurnOptions, prog: &crate::engine::export::Progress) -> Result<String, String> {
    match content {
        BurnContent::Audio(cd) => run_audio(cd, target, opts, prog),
        BurnContent::Data { volume, items } => run_data(volume, items, target, opts, prog),
    }
}

fn work_dir() -> PathBuf {
    let d = crate::settings::Settings::cache_dir().join("burn").join(format!("{}-{}", std::process::id(), now_ms()));
    let _ = std::fs::create_dir_all(&d);
    d
}

fn now_ms() -> u128 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis())
}

fn run_audio(cd: &AudioCd, target: &BurnTarget, opts: &BurnOptions, prog: &crate::engine::export::Progress) -> Result<String, String> {
    check_audio_estimate(cd, opts.size)?;
    for t in &cd.tracks {
        if !t.path.is_file() {
            return Err(format!("missing file: {}", t.path.display()));
        }
    }
    if let BurnTarget::Drive { id, .. } = target {
        // fail on "no disc" before minutes of converting
        let m = imapi::media_info(id)?;
        m.check_audio()?;
    }
    let dir = work_dir();
    let cleanup = scopeguard(dir.clone());
    let n = cd.tracks.len();
    let mut raw = Vec::with_capacity(n);
    let mut lens = Vec::with_capacity(n);
    for (i, t) in cd.tracks.iter().enumerate() {
        prog.set(0.5 * i as f32 / n as f32, format!("Converting track {} of {n}", i + 1));
        let out = dir.join(format!("{:02}.raw", i + 1));
        lens.push(convert_track(&t.path, &out, &|| prog.is_cancelled())?);
        raw.push(out);
    }
    let places = layout(&lens, cd.gap);
    check_audio_capacity(&places, opts.size)?;
    let msg = match target {
        BurnTarget::Image(path) => {
            let bin = path.with_extension("bin");
            let cue = write_bin_cue(cd, &raw, &places, &bin, &|f| prog.set(0.5 + 0.5 * f, "Writing the image"))?;
            format!("Wrote {} and {} ({} tracks, {})", bin.display(), cue.display(), n, clock(total_sectors(&places) as f64 / 75.0))
        }
        BurnTarget::Drive { id, test_only } => {
            imapi::burn_audio(id, &raw, &places, opts, *test_only, prog)?;
            if *test_only {
                format!("Test passed: {n} tracks ({}) fit the disc in the drive; nothing was written", clock(total_sectors(&places) as f64 / 75.0))
            } else {
                format!("Burned {n} tracks")
            }
        }
    };
    drop(cleanup);
    Ok(msg)
}

fn run_data(volume: &str, items: &[DataItem], target: &BurnTarget, opts: &BurnOptions, prog: &crate::engine::export::Progress) -> Result<String, String> {
    if items.is_empty() {
        return Err("nothing to put on the disc".into());
    }
    for it in items {
        if !it.src.exists() {
            return Err(format!("missing: {}", it.src.display()));
        }
    }
    check_data_capacity(estimate_blocks(items), opts.size)?;
    prog.set(0.0, "Building the file system");
    match target {
        BurnTarget::Image(path) => {
            let blocks = imapi::write_iso(volume, items, opts.size, path, prog)?;
            Ok(format!("Wrote {} ({} MB)", path.display(), blocks * DATA_BLOCK / 1_000_000))
        }
        BurnTarget::Drive { id, test_only } => {
            let m = imapi::media_info(id)?;
            m.check_data()?;
            imapi::burn_data(id, volume, items, opts, *test_only, prog)
        }
    }
}

/// Deletes the work folder when dropped (also on an early `?` return).
struct Cleanup(PathBuf);
impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn scopeguard(p: PathBuf) -> Cleanup {
    Cleanup(p)
}

/// An `.m3u8` for a playlist copied onto a data disc: names relative to the playlist's folder.
pub fn disc_m3u8(entries: &[(String, f64, String)]) -> String {
    let mut s = String::from("#EXTM3U\n");
    for (title, secs, file) in entries {
        s += &format!("#EXTINF:{},{}\n{}\n", secs.round() as i64, title, file);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn have_ffmpeg() -> bool {
        crate::media::ffpipe::ffmpeg_exe().is_some()
    }

    fn tmpdir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("se-disc-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn msf_round_trips() {
        assert_eq!(msf(0), "00:00:00");
        assert_eq!(msf(75 * 61 + 7), "01:01:07");
        assert_eq!(parse_msf("01:01:07"), Some(75 * 61 + 7));
    }

    #[test]
    fn layout_puts_gaps_between_tracks_and_raises_short_tracks_to_4s() {
        let p = layout(&[750, 100, 1500], 2.0);
        assert_eq!(p[0], TrackPlace { gap_start: 0, start: 0, sectors: 750 });
        assert_eq!(p[1], TrackPlace { gap_start: 750, start: 900, sectors: MIN_TRACK });
        assert_eq!(p[2].gap_start, 900 + 300);
        assert_eq!(p[2].start, 1200 + 150);
        assert_eq!(total_sectors(&p), 150 + 1350 + 1500);
        let gapless = layout(&[750, 750], 0.0);
        assert_eq!(gapless[1].start, 750);
        assert_eq!(gapless[1].gap_start, gapless[1].start);
    }

    #[test]
    fn capacity_check_74_and_80_minutes() {
        // 77 minutes of music in one-minute tracks, gapless
        let p = layout(&vec![60 * 75; 77], 0.0);
        assert!(check_audio_capacity(&p, DiscSize::Cd80).is_ok());
        let e = check_audio_capacity(&p, DiscSize::Cd74).unwrap_err();
        assert!(e.contains("Too long") && e.contains("74:00") && e.contains("77:02"), "{e}");
        // exactly full: 80:00 including the 2 s lead pregap
        let p = layout(&[80 * 60 * 75 - LEAD_PREGAP], 0.0);
        assert!(check_audio_capacity(&p, DiscSize::Cd80).is_ok());
        let p = layout(&[80 * 60 * 75 - LEAD_PREGAP + 1], 0.0);
        assert!(check_audio_capacity(&p, DiscSize::Cd80).is_err());
        assert!(check_audio_capacity(&layout(&vec![300; 100], 0.0), DiscSize::Cd80).unwrap_err().contains("99"));
        assert!(check_audio_capacity(&[], DiscSize::Cd80).is_err());
        assert!(check_audio_capacity(&p, DiscSize::Dvd).is_err());
        // data
        assert!(check_data_capacity(359_846, DiscSize::Cd80).is_ok());
        let e = check_data_capacity(400_000, DiscSize::Cd80).unwrap_err();
        assert!(e.contains("Too big") && e.contains("819 MB") && e.contains("736 MB"), "{e}");
        assert!(check_data_capacity(400_000, DiscSize::Dvd).is_ok());
    }

    #[test]
    fn cue_sheet_round_trips_with_cd_text() {
        let cd = AudioCd {
            title: "Mix \"1\"".into(),
            performer: "Various".into(),
            gap: 2.0,
            tracks: vec![
                CdTrack { title: "One".into(), performer: "A".into(), ..Default::default() },
                CdTrack { title: "Two".into(), performer: "B".into(), ..Default::default() },
            ],
        };
        let places = layout(&[750, 1000], cd.gap);
        let cue = cue_sheet(&cd, &places, "mix.bin");
        assert!(cue.contains("TITLE \"Mix '1'\""), "{cue}");
        assert!(cue.contains("FILE \"mix.bin\" BINARY"));
        let t = parse_cue(&cue);
        assert_eq!(t.len(), 2);
        assert_eq!((t[0].number, t[0].title.as_str(), t[0].performer.as_str(), t[0].index1, t[0].index0), (1, "One", "A", 0, None));
        assert_eq!((t[1].index0, t[1].index1), (Some(750), 900));
        assert_eq!(cue_lengths(&t, (900 + 1000) * AUDIO_SECTOR), vec![750, 1000]);
    }

    /// Audio CD image end to end: three generated songs -> .bin + .cue; the cue's track count and the
    /// lengths it and the bin give match the songs.
    #[test]
    fn audio_cd_image_has_the_tracks_and_lengths() {
        if !have_ffmpeg() {
            eprintln!("skipped: no ffmpeg");
            return;
        }
        let d = tmpdir("cd");
        let mut tracks = Vec::new();
        for (i, secs) in [5.0, 7.5, 2.0].iter().enumerate() {
            let p = d.join(format!("s{i}.mp3"));
            let st = std::process::Command::new(crate::media::ffpipe::ffmpeg_exe().unwrap())
                .args(["-nostdin", "-v", "error", "-y", "-f", "lavfi", "-i", &format!("sine=f={}:d={secs}:sample_rate=48000", 300 + i * 100)])
                .arg(&p)
                .status()
                .unwrap();
            assert!(st.success());
            tracks.push(CdTrack { path: p, title: format!("Song {i}"), performer: "Gen".into(), seconds: *secs });
        }
        let cd = AudioCd { title: "Test".into(), performer: "Gen".into(), tracks, gap: 2.0 };
        let img = d.join("out.iso"); // any extension: audio writes .bin + .cue
        let prog = crate::engine::export::Progress::new();
        let msg = run(&BurnContent::Audio(cd), &BurnTarget::Image(img.clone()), &BurnOptions::default(), &prog).unwrap();
        assert!(msg.contains("3 tracks"), "{msg}");
        let bin = d.join("out.bin");
        let cue = std::fs::read_to_string(d.join("out.cue")).unwrap();
        let t = parse_cue(&cue);
        assert_eq!(t.len(), 3, "{cue}");
        assert_eq!(t.iter().map(|t| t.title.as_str()).collect::<Vec<_>>(), ["Song 0", "Song 1", "Song 2"]);
        let bytes = std::fs::metadata(&bin).unwrap().len();
        assert_eq!(bytes % AUDIO_SECTOR, 0);
        let lens = cue_lengths(&t, bytes);
        // mp3 encoder padding adds a few ms; the 2 s song is raised to the 4 s minimum
        let want = [5.0, 7.5, 4.0];
        for (got, w) in lens.iter().zip(want) {
            let secs = *got as f64 / 75.0;
            assert!((secs - w).abs() < 0.15, "track is {secs} s, want {w}: {lens:?}");
        }
        assert_eq!(t[1].index1 - t[1].index0.unwrap(), 150, "2 s gap");
        // the bin is 44.1 kHz stereo 16-bit: the first track's samples are not silence
        let pcm = std::fs::read(&bin).unwrap();
        assert!(pcm[..AUDIO_SECTOR as usize * 75].iter().any(|&b| b != 0));
        assert!(!d.join("burn").exists());
        let _ = std::fs::remove_dir_all(&d);
    }
}
