//! Reading .zip files (stored and deflate entries, streamed through miniz_oxide, CRC-checked). Enough for
//! the FFmpeg build and the add-on zips; no zip64 (none of them needs it).

use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};

#[derive(Clone, Debug)]
pub struct Entry {
    /// The path inside the zip, with `/` separators.
    pub name: String,
    method: u16,
    crc: u32,
    csize: u64,
    pub size: u64,
    local: u64,
}

impl Entry {
    pub fn is_dir(&self) -> bool {
        self.name.ends_with('/')
    }
    /// The last path component ("ffmpeg.exe" for "ffmpeg-7.1/bin/ffmpeg.exe").
    pub fn file_name(&self) -> &str {
        self.name.trim_end_matches('/').rsplit('/').next().unwrap_or("")
    }
}

fn u16_at(b: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([b[i], b[i + 1]])
}
fn u32_at(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

/// The central directory.
pub fn entries(f: &mut File) -> Result<Vec<Entry>, String> {
    let len = f.seek(SeekFrom::End(0)).map_err(|e| e.to_string())?;
    let tail = len.min(22 + 65535);
    f.seek(SeekFrom::Start(len - tail)).map_err(|e| e.to_string())?;
    let mut t = vec![0u8; tail as usize];
    f.read_exact(&mut t).map_err(|e| e.to_string())?;
    let eocd = (0..t.len().saturating_sub(21)).rev().find(|&i| u32_at(&t, i) == 0x0605_4b50).ok_or("not a zip file")?;
    let count = u16_at(&t, eocd + 10) as usize;
    let cd_size = u32_at(&t, eocd + 12) as u64;
    let cd_off = u32_at(&t, eocd + 16) as u64;
    if cd_off == 0xFFFF_FFFF || cd_off + cd_size > len {
        return Err("unsupported zip (zip64 or damaged)".into());
    }
    f.seek(SeekFrom::Start(cd_off)).map_err(|e| e.to_string())?;
    let mut cd = vec![0u8; cd_size as usize];
    f.read_exact(&mut cd).map_err(|e| e.to_string())?;
    let mut out = Vec::with_capacity(count);
    let mut i = 0;
    for _ in 0..count {
        if i + 46 > cd.len() || u32_at(&cd, i) != 0x0201_4b50 {
            return Err("damaged zip directory".into());
        }
        let flags = u16_at(&cd, i + 8);
        let method = u16_at(&cd, i + 10);
        let crc = u32_at(&cd, i + 16);
        let csize = u32_at(&cd, i + 20) as u64;
        let size = u32_at(&cd, i + 24) as u64;
        let (nl, xl, cl) = (u16_at(&cd, i + 28) as usize, u16_at(&cd, i + 30) as usize, u16_at(&cd, i + 32) as usize);
        let local = u32_at(&cd, i + 42) as u64;
        let raw = cd.get(i + 46..i + 46 + nl).ok_or("damaged zip directory")?;
        // bit 11: UTF-8 names; otherwise CP437, which for the ASCII names we care about is the same
        let name = if flags & 0x800 != 0 { String::from_utf8_lossy(raw).into_owned() } else { raw.iter().map(|&b| b as char).collect() };
        out.push(Entry { name: name.replace('\\', "/"), method, crc, csize, size, local });
        i += 46 + nl + xl + cl;
    }
    Ok(out)
}

/// A zip entry name as a safe relative path: no `..`, no root, no drive (zip-slip).
pub fn safe_relative(name: &str) -> Option<PathBuf> {
    let p = Path::new(name);
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::Normal(s) => {
                let s = s.to_string_lossy();
                if s.contains(':') {
                    return None;
                }
                out.push(&*s);
            }
            Component::CurDir => {}
            _ => return None,
        }
    }
    (!out.as_os_str().is_empty()).then_some(out)
}

/// Unpack one entry to `dest` (written to `dest.part`, CRC-checked, then renamed). `progress` gets the
/// bytes written so far; returning false cancels.
pub fn extract(f: &mut File, e: &Entry, dest: &Path, progress: &mut dyn FnMut(u64) -> bool) -> Result<(), String> {
    use miniz_oxide::inflate::stream::{inflate, InflateState};
    use miniz_oxide::{DataFormat, MZError, MZFlush, MZStatus};
    f.seek(SeekFrom::Start(e.local)).map_err(|x| x.to_string())?;
    let mut h = [0u8; 30];
    f.read_exact(&mut h).map_err(|x| x.to_string())?;
    if u32_at(&h, 0) != 0x0403_4b50 {
        return Err(format!("{}: damaged entry", e.name));
    }
    let skip = u16_at(&h, 26) as i64 + u16_at(&h, 28) as i64;
    f.seek(SeekFrom::Current(skip)).map_err(|x| x.to_string())?;
    if let Some(d) = dest.parent() {
        std::fs::create_dir_all(d).map_err(|x| format!("{}: {x}", d.display()))?;
    }
    let part = PathBuf::from(format!("{}.part", dest.display()));
    let mut out = std::io::BufWriter::new(File::create(&part).map_err(|x| format!("{}: {x}", part.display()))?);
    let mut crc = Crc32::new();
    let mut written = 0u64;
    let mut left = e.csize;
    let mut inbuf = vec![0u8; 1 << 16];
    let r = (|| -> Result<(), String> {
        match e.method {
            0 => {
                while left > 0 {
                    let n = (left as usize).min(inbuf.len());
                    f.read_exact(&mut inbuf[..n]).map_err(|x| x.to_string())?;
                    left -= n as u64;
                    crc.update(&inbuf[..n]);
                    out.write_all(&inbuf[..n]).map_err(|x| x.to_string())?;
                    written += n as u64;
                    if !progress(written) {
                        return Err("cancelled".into());
                    }
                }
            }
            8 => {
                let mut st = InflateState::new_boxed(DataFormat::Raw);
                let mut outbuf = vec![0u8; 1 << 17];
                let (mut have, mut pos) = (0usize, 0usize);
                loop {
                    if pos == have && left > 0 {
                        let n = (left as usize).min(inbuf.len());
                        f.read_exact(&mut inbuf[..n]).map_err(|x| x.to_string())?;
                        left -= n as u64;
                        have = n;
                        pos = 0;
                    }
                    // never Finish: on a first call miniz then expects the whole output to fit in `outbuf`
                    let r = inflate(&mut st, &inbuf[pos..have], &mut outbuf, MZFlush::None);
                    pos += r.bytes_consumed;
                    crc.update(&outbuf[..r.bytes_written]);
                    out.write_all(&outbuf[..r.bytes_written]).map_err(|x| x.to_string())?;
                    written += r.bytes_written as u64;
                    if !progress(written) {
                        return Err("cancelled".into());
                    }
                    match r.status {
                        Ok(MZStatus::StreamEnd) => break,
                        Ok(_) => {}
                        // no progress possible with this input: more input is needed or the data is bad
                        Err(MZError::Buf) if left > 0 || pos < have || r.bytes_written > 0 => {}
                        Err(x) => return Err(format!("{}: damaged data ({x:?})", e.name)),
                    }
                    if r.bytes_consumed == 0 && r.bytes_written == 0 && pos == have && left == 0 {
                        return Err(format!("{}: truncated", e.name));
                    }
                }
            }
            m => return Err(format!("{}: unsupported compression {m}", e.name)),
        }
        out.flush().map_err(|x| x.to_string())?;
        if written != e.size || crc.value() != e.crc {
            return Err(format!("{}: damaged (size or CRC mismatch)", e.name));
        }
        Ok(())
    })();
    drop(out);
    if let Err(x) = r {
        let _ = std::fs::remove_file(&part);
        return Err(x);
    }
    let _ = std::fs::remove_file(dest);
    std::fs::rename(&part, dest).map_err(|x| format!("{}: {x}", dest.display()))
}

/// Unpack every file `map` names a destination for (a path relative to `dir`). Returns the files written.
pub fn extract_where(
    zip: &Path,
    dir: &Path,
    map: &dyn Fn(&Entry) -> Option<PathBuf>,
    progress: &mut dyn FnMut(u64, u64) -> bool,
) -> Result<Vec<PathBuf>, String> {
    let mut f = File::open(zip).map_err(|e| format!("{}: {e}", zip.display()))?;
    let list = entries(&mut f)?;
    let chosen: Vec<(Entry, PathBuf)> = list
        .into_iter()
        .filter(|e| !e.is_dir())
        .filter_map(|e| {
            let rel = map(&e)?;
            let rel = safe_relative(&rel.to_string_lossy())?;
            Some((e, dir.join(rel)))
        })
        .collect();
    let total: u64 = chosen.iter().map(|(e, _)| e.size).sum();
    let mut done = 0u64;
    let mut out = Vec::new();
    for (e, dest) in chosen {
        let base = done;
        extract(&mut f, &e, &dest, &mut |n| progress(base + n, total))?;
        done += e.size;
        out.push(dest);
    }
    Ok(out)
}

pub struct Crc32(u32);

impl Crc32 {
    pub fn new() -> Self {
        Crc32(!0)
    }
    pub fn update(&mut self, data: &[u8]) {
        static TABLE: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
        let t = TABLE.get_or_init(|| {
            let mut t = [0u32; 256];
            for (i, v) in t.iter_mut().enumerate() {
                let mut c = i as u32;
                for _ in 0..8 {
                    c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
                }
                *v = c;
            }
            t
        });
        let mut c = self.0;
        for &b in data {
            c = t[((c ^ b as u32) & 0xFF) as usize] ^ (c >> 8);
        }
        self.0 = c;
    }
    pub fn value(&self) -> u32 {
        !self.0
    }
}

/// Write a zip (tests and fixtures): `(name, data, deflate?)`.
#[cfg(test)]
pub fn write_zip(path: &Path, files: &[(&str, &[u8], bool)]) {
    let mut body = Vec::new();
    let mut cd = Vec::new();
    for (name, data, deflate) in files {
        let mut c = Crc32::new();
        c.update(data);
        let crc = c.value();
        let comp = if *deflate { miniz_oxide::deflate::compress_to_vec(data, 6) } else { data.to_vec() };
        let method: u16 = if *deflate { 8 } else { 0 };
        let off = body.len() as u32;
        let mut h = Vec::new();
        h.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        h.extend_from_slice(&[20, 0, 0, 0]);
        h.extend_from_slice(&method.to_le_bytes());
        h.extend_from_slice(&[0, 0, 0, 0]);
        h.extend_from_slice(&crc.to_le_bytes());
        h.extend_from_slice(&(comp.len() as u32).to_le_bytes());
        h.extend_from_slice(&(data.len() as u32).to_le_bytes());
        h.extend_from_slice(&(name.len() as u16).to_le_bytes());
        h.extend_from_slice(&[0, 0]);
        h.extend_from_slice(name.as_bytes());
        body.extend_from_slice(&h);
        body.extend_from_slice(&comp);
        cd.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        cd.extend_from_slice(&[20, 0, 20, 0, 0, 0]);
        cd.extend_from_slice(&method.to_le_bytes());
        cd.extend_from_slice(&[0, 0, 0, 0]);
        cd.extend_from_slice(&crc.to_le_bytes());
        cd.extend_from_slice(&(comp.len() as u32).to_le_bytes());
        cd.extend_from_slice(&(data.len() as u32).to_le_bytes());
        cd.extend_from_slice(&(name.len() as u16).to_le_bytes());
        cd.extend_from_slice(&[0; 12]);
        cd.extend_from_slice(&off.to_le_bytes());
        cd.extend_from_slice(name.as_bytes());
    }
    let cd_off = body.len() as u32;
    body.extend_from_slice(&cd);
    body.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    body.extend_from_slice(&[0, 0, 0, 0]);
    body.extend_from_slice(&(files.len() as u16).to_le_bytes());
    body.extend_from_slice(&(files.len() as u16).to_le_bytes());
    body.extend_from_slice(&(cd.len() as u32).to_le_bytes());
    body.extend_from_slice(&cd_off.to_le_bytes());
    body.extend_from_slice(&[0, 0]);
    std::fs::write(path, body).unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;

    #[test]
    fn extracts_stored_and_deflated_entries() {
        let t = TempDir::new("zip");
        let big: Vec<u8> = (0..700_000u32).map(|i| (i % 97) as u8 ^ (i >> 9) as u8).collect();
        let zip = t.path().join("a.zip");
        write_zip(&zip, &[("pkg/bin/ffmpeg.exe", &big, true), ("pkg/bin/readme.txt", b"hello", false), ("pkg/doc/x.txt", b"x", true)]);
        let got = extract_where(&zip, &t.path().join("out"), &|e| (e.name.contains("/bin/")).then(|| PathBuf::from(e.file_name())), &mut |_, _| true).unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!(std::fs::read(t.path().join("out/ffmpeg.exe")).unwrap(), big);
        assert_eq!(std::fs::read(t.path().join("out/readme.txt")).unwrap(), b"hello");
        assert!(!t.path().join("out/x.txt").exists());
    }

    #[test]
    fn a_corrupted_entry_is_refused_and_leaves_nothing() {
        let t = TempDir::new("zipbad");
        let zip = t.path().join("a.zip");
        let data = vec![7u8; 50_000];
        write_zip(&zip, &[("a.bin", &data, false)]);
        let mut bytes = std::fs::read(&zip).unwrap();
        bytes[100] ^= 0xFF; // inside a.bin's data
        std::fs::write(&zip, bytes).unwrap();
        let r = extract_where(&zip, &t.path().join("out"), &|e| Some(PathBuf::from(&e.name)), &mut |_, _| true);
        assert!(r.is_err());
        assert!(!t.path().join("out/a.bin").exists() && !t.path().join("out/a.bin.part").exists());
    }

    #[test]
    fn zip_slip_names_are_dropped() {
        assert!(safe_relative("../evil.exe").is_none());
        assert!(safe_relative("a/../../evil.exe").is_none());
        assert!(safe_relative("/etc/x").is_none());
        assert!(safe_relative("C:/x").is_none());
        assert_eq!(safe_relative("a/./b.exe"), Some(PathBuf::from("a").join("b.exe")));
    }

    /// The real FFmpeg build the installer pins (SE_FFMPEG_ZIP=<path to the 92 MB zip>): both exes come out.
    #[test]
    fn real_ffmpeg_zip_when_given() {
        let Some(zip) = std::env::var_os("SE_FFMPEG_ZIP") else { return };
        let t = TempDir::new("ffzip");
        let got = extract_where(Path::new(&zip), t.path(), &crate::addons::ffmpeg_map, &mut |_, _| true).unwrap();
        assert_eq!(got.len(), 2);
        assert!(std::fs::metadata(t.path().join("ffmpeg.exe")).unwrap().len() > 10_000_000);
        assert!(std::fs::metadata(t.path().join("ffprobe.exe")).unwrap().len() > 10_000_000);
    }
}
