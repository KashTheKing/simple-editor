//! ---- feat/plugins ----
//! A small zip reader for plugin archives: stored and deflated entries (inflated by `miniz_oxide`, which
//! the build already links through png / flate2), CRC-checked, with every name checked so an archive can
//! never write outside the folder it is unpacked into. Zip64, encryption and multi-disk archives are refused.

use std::path::{Component, Path, PathBuf};

/// The most an archive may unpack to (all entries together).
pub const MAX_TOTAL: u64 = 256 << 20;
/// The most entries an archive may hold.
pub const MAX_ENTRIES: usize = 20_000;

/// One file of an archive (folders are implied by the names).
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    /// The name inside the archive, with `/` separators (`repo-main/plugin.json`).
    pub name: String,
    pub data: Vec<u8>,
}

fn u16_at(b: &[u8], i: usize) -> Result<u16, String> {
    b.get(i..i + 2).map(|s| u16::from_le_bytes([s[0], s[1]])).ok_or_else(|| "the zip file is cut short".into())
}

fn u32_at(b: &[u8], i: usize) -> Result<u32, String> {
    b.get(i..i + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]])).ok_or_else(|| "the zip file is cut short".into())
}

/// The standard CRC-32 (IEEE, as zip uses it).
pub fn crc32(data: &[u8]) -> u32 {
    static TABLE: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
    let t = TABLE.get_or_init(|| {
        let mut t = [0u32; 256];
        for (i, slot) in t.iter_mut().enumerate() {
            let mut c = i as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            }
            *slot = c;
        }
        t
    });
    let mut c = !0u32;
    for &b in data {
        c = t[((c ^ b as u32) & 0xFF) as usize] ^ (c >> 8);
    }
    !c
}

/// A name that stays inside the folder it is unpacked into: relative, no `..`, no drive or root.
pub fn safe_name(name: &str) -> Option<PathBuf> {
    let name = name.replace('\\', "/");
    if name.starts_with('/') || name.contains(':') || name.contains('\0') {
        return None;
    }
    let mut out = PathBuf::new();
    for c in Path::new(&name).components() {
        match c {
            Component::Normal(s) => out.push(s),
            Component::CurDir => {}
            _ => return None,
        }
    }
    (!out.as_os_str().is_empty()).then_some(out)
}

/// Every file in `bytes` (folders skipped), in archive order.
pub fn read(bytes: &[u8]) -> Result<Vec<Entry>, String> {
    // the end-of-central-directory record: 22 bytes plus a comment of up to 64 KB
    let lo = bytes.len().saturating_sub(22 + 0xFFFF);
    let eocd = (lo..=bytes.len().saturating_sub(22))
        .rev()
        .find(|&i| bytes[i..i + 4] == [0x50, 0x4b, 0x05, 0x06])
        .ok_or("not a zip file")?;
    if u16_at(bytes, eocd + 4)? != 0 || u16_at(bytes, eocd + 6)? != 0 {
        return Err("multi-part zip files are not supported".into());
    }
    let count = u16_at(bytes, eocd + 10)? as usize;
    let cd_size = u32_at(bytes, eocd + 12)? as usize;
    let cd_off = u32_at(bytes, eocd + 16)? as usize;
    if count == 0xFFFF || cd_off == 0xFFFF_FFFF {
        return Err("zip64 archives are not supported".into());
    }
    if count > MAX_ENTRIES {
        return Err(format!("the zip has more than {MAX_ENTRIES} files"));
    }
    if cd_off.checked_add(cd_size).is_none_or(|end| end > bytes.len()) {
        return Err("the zip's directory is damaged".into());
    }
    let mut out = Vec::new();
    let mut total = 0u64;
    let mut p = cd_off;
    for _ in 0..count {
        if u32_at(bytes, p)? != 0x0201_4b50 {
            return Err("the zip's directory is damaged".into());
        }
        let flags = u16_at(bytes, p + 8)?;
        let method = u16_at(bytes, p + 10)?;
        let crc = u32_at(bytes, p + 16)?;
        let csize = u32_at(bytes, p + 20)? as usize;
        let usize_ = u32_at(bytes, p + 24)? as u64;
        let (nlen, xlen, clen) = (u16_at(bytes, p + 28)? as usize, u16_at(bytes, p + 30)? as usize, u16_at(bytes, p + 32)? as usize);
        let local = u32_at(bytes, p + 42)? as usize;
        let name_bytes = bytes.get(p + 46..p + 46 + nlen).ok_or("the zip's directory is damaged")?;
        let name = String::from_utf8_lossy(name_bytes).replace('\\', "/");
        p += 46 + nlen + xlen + clen;
        if name.ends_with('/') {
            continue; // a folder
        }
        if flags & 1 != 0 {
            return Err(format!("{name}: encrypted zip entries are not supported"));
        }
        if safe_name(&name).is_none() {
            return Err(format!("{name}: a file name that points outside the plugin folder"));
        }
        total += usize_;
        if total > MAX_TOTAL {
            return Err(format!("the zip unpacks to more than {} MB", MAX_TOTAL >> 20));
        }
        if u32_at(bytes, local)? != 0x0403_4b50 {
            return Err(format!("{name}: damaged entry"));
        }
        let start = local + 30 + u16_at(bytes, local + 26)? as usize + u16_at(bytes, local + 28)? as usize;
        let raw = bytes.get(start..start + csize).ok_or_else(|| format!("{name}: cut short"))?;
        let data = match method {
            0 => raw.to_vec(),
            8 => miniz_oxide::inflate::decompress_to_vec_with_limit(raw, usize_ as usize)
                .map_err(|e| format!("{name}: could not inflate ({:?})", e.status))?,
            m => return Err(format!("{name}: compression method {m} is not supported")),
        };
        if data.len() as u64 != usize_ || crc32(&data) != crc {
            return Err(format!("{name}: damaged (checksum mismatch)"));
        }
        out.push(Entry { name, data });
    }
    Ok(out)
}

/// Unpack `bytes` into `dest` (created). Returns the files written, relative to `dest`.
pub fn extract(bytes: &[u8], dest: &Path) -> Result<Vec<PathBuf>, String> {
    let entries = read(bytes)?;
    std::fs::create_dir_all(dest).map_err(|e| format!("{}: {e}", dest.display()))?;
    let mut written = Vec::new();
    for e in entries {
        let rel = safe_name(&e.name).ok_or_else(|| format!("{}: unsafe name", e.name))?;
        let path = dest.join(&rel);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|err| format!("{}: {err}", dir.display()))?;
        }
        std::fs::write(&path, &e.data).map_err(|err| format!("{}: {err}", path.display()))?;
        written.push(rel);
    }
    Ok(written)
}

/// Build a zip of `files` (name, bytes): deflated when that is smaller, else stored. Used by the tests'
/// fixtures and by "Pack plugin" for authors; the archives it writes are what `read` accepts.
pub fn write(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut cd = Vec::new();
    for (name, data) in files {
        let crc = crc32(data);
        let deflated = miniz_oxide::deflate::compress_to_vec(data, 6);
        let (method, body): (u16, &[u8]) = if deflated.len() < data.len() { (8, &deflated) } else { (0, data) };
        let off = out.len() as u32;
        let head = |sig: u32, v: &mut Vec<u8>, central: bool| {
            v.extend_from_slice(&sig.to_le_bytes());
            if central {
                v.extend_from_slice(&20u16.to_le_bytes()); // made by
            }
            v.extend_from_slice(&20u16.to_le_bytes()); // needed
            v.extend_from_slice(&0x0800u16.to_le_bytes()); // utf-8 names
            v.extend_from_slice(&method.to_le_bytes());
            v.extend_from_slice(&[0, 0, 0x21, 0]); // time, date (1980-01-01)
            v.extend_from_slice(&crc.to_le_bytes());
            v.extend_from_slice(&(body.len() as u32).to_le_bytes());
            v.extend_from_slice(&(data.len() as u32).to_le_bytes());
            v.extend_from_slice(&(name.len() as u16).to_le_bytes());
            v.extend_from_slice(&0u16.to_le_bytes()); // extra
        };
        head(0x0403_4b50, &mut out, false);
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(body);
        head(0x0201_4b50, &mut cd, true);
        cd.extend_from_slice(&[0; 8]); // comment len, disk, internal attrs, external attrs (part)
        cd.extend_from_slice(&[0; 2]);
        cd.extend_from_slice(&off.to_le_bytes());
        cd.extend_from_slice(name.as_bytes());
    }
    let cd_off = out.len() as u32;
    out.extend_from_slice(&cd);
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&(cd.len() as u32).to_le_bytes());
    out.extend_from_slice(&cd_off.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zip_round_trips_stored_and_deflated_entries() {
        let big = "plugin ".repeat(500);
        let z = write(&[("a/plugin.json", b"{}"), ("a/big.txt", big.as_bytes())]);
        let e = read(&z).unwrap();
        assert_eq!(e.len(), 2);
        assert_eq!(e[0].name, "a/plugin.json");
        assert_eq!(e[0].data, b"{}");
        assert_eq!(e[1].data, big.as_bytes());
    }

    #[test]
    fn zip_refuses_names_outside_the_folder_and_damage() {
        for bad in ["../evil.txt", "/abs.txt", "C:/x.txt", "a/../../b"] {
            let z = write(&[(bad, b"x")]);
            assert!(read(&z).is_err(), "{bad} must be refused");
        }
        let mut z = write(&[("ok.txt", b"hello")]);
        let i = z.windows(5).position(|w| w == b"hello").unwrap();
        z[i] = b'j';
        assert!(read(&z).unwrap_err().contains("checksum"));
        assert!(read(b"not a zip").is_err());
    }

    #[test]
    fn crc32_matches_the_standard_check_value() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }
}
