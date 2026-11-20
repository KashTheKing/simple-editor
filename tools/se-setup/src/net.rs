//! Downloads, behind a trait (WinHTTP on Windows, a table of canned answers in tests), and the GitHub
//! release API.

use crate::json::{self, Json};
use crate::sha256::Sha256;
use std::io::Write;
use std::path::Path;

pub const REPO: &str = "KashTheKing/simple-editor";

pub trait Net: Sync {
    /// GET a URL (API calls: small).
    fn get(&self, url: &str) -> Result<Vec<u8>, String>;
    /// GET a URL in pieces, each handed to `sink`; `total` is the Content-Length when known. `sink`
    /// returning false cancels.
    fn stream(&self, url: &str, sink: &mut dyn FnMut(&[u8], Option<u64>) -> bool) -> Result<(), String>;
}

/// Download `url` to `dest` (through `dest.part`), hashing as it arrives. Checks the size and the SHA-256
/// when given and deletes the file on any mismatch. Returns the hex digest.
pub fn download(
    net: &dyn Net,
    url: &str,
    dest: &Path,
    want_sha: Option<&str>,
    want_size: Option<u64>,
    progress: &mut dyn FnMut(u64, Option<u64>) -> bool,
) -> Result<String, String> {
    if let Some(d) = dest.parent() {
        std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    let part = std::path::PathBuf::from(format!("{}.part", dest.display()));
    let mut f = std::io::BufWriter::new(std::fs::File::create(&part).map_err(|e| format!("{}: {e}", part.display()))?);
    let mut h = Sha256::new();
    let mut got = 0u64;
    let mut werr = None;
    let r = net.stream(url, &mut |chunk, total| {
        if let Err(e) = f.write_all(chunk) {
            werr = Some(e.to_string());
            return false;
        }
        h.update(chunk);
        got += chunk.len() as u64;
        progress(got, total.or(want_size))
    });
    let flushed = f.flush().map_err(|e| e.to_string());
    drop(f);
    let fail = |e: String| {
        let _ = std::fs::remove_file(&part);
        Err(e)
    };
    if let Some(e) = werr {
        return fail(e);
    }
    if let Err(e) = r.and(flushed) {
        return fail(e);
    }
    if let Some(n) = want_size {
        if got != n {
            return fail(format!("the download is {got} bytes, expected {n}"));
        }
    }
    let hex = h.hex();
    if let Some(w) = want_sha {
        if !hex.eq_ignore_ascii_case(w.trim()) {
            return fail("the download does not match its checksum".into());
        }
    }
    let _ = std::fs::remove_file(dest);
    std::fs::rename(&part, dest).map_err(|e| format!("{}: {e}", dest.display()))?;
    Ok(hex)
}

#[derive(Clone, Debug, PartialEq)]
pub struct Asset {
    pub name: String,
    pub url: String,
    pub size: u64,
    /// The SHA-256 GitHub computed on upload ("digest": "sha256:<hex>"), when the API gives one.
    pub sha256: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    pub tag: String,
    pub assets: Vec<Asset>,
}

impl Release {
    pub fn asset(&self, name: &str) -> Option<&Asset> {
        self.assets.iter().find(|a| a.name.eq_ignore_ascii_case(name))
    }
}

pub fn parse_release(j: &Json) -> Result<Release, String> {
    let tag = j.str("tag_name").ok_or_else(|| j.str("message").map(|m| format!("GitHub: {m}")).unwrap_or_else(|| "no release found".into()))?;
    let assets = j
        .arr("assets")
        .iter()
        .filter_map(|a| {
            Some(Asset {
                name: a.str("name")?.to_string(),
                url: a.str("browser_download_url")?.to_string(),
                size: a.num("size").unwrap_or(0.0) as u64,
                sha256: a.str("digest").and_then(|d| d.strip_prefix("sha256:")).map(str::to_string),
            })
        })
        .collect();
    Ok(Release { tag: tag.to_string(), assets })
}

/// The latest (non-prerelease) release of `owner/repo`.
pub fn latest_release(net: &dyn Net, repo: &str) -> Result<Release, String> {
    let body = net.get(&format!("https://api.github.com/repos/{repo}/releases/latest"))?;
    let text = String::from_utf8(body).map_err(|_| "GitHub sent something that is not text")?;
    parse_release(&json::parse(&text)?)
}

/// Canned answers (tests).
#[cfg(test)]
pub struct FakeNet {
    pub pages: std::sync::Mutex<std::collections::HashMap<String, Vec<u8>>>,
    pub hits: std::sync::Mutex<Vec<String>>,
}

#[cfg(test)]
impl FakeNet {
    pub fn new(pages: &[(&str, &[u8])]) -> Self {
        FakeNet {
            pages: std::sync::Mutex::new(pages.iter().map(|(u, b)| (u.to_string(), b.to_vec())).collect()),
            hits: Default::default(),
        }
    }
    pub fn hit(&self, url: &str) -> bool {
        self.hits.lock().unwrap().iter().any(|h| h == url)
    }
}

#[cfg(test)]
impl Net for FakeNet {
    fn get(&self, url: &str) -> Result<Vec<u8>, String> {
        self.hits.lock().unwrap().push(url.to_string());
        self.pages.lock().unwrap().get(url).cloned().ok_or_else(|| format!("404 {url}"))
    }
    fn stream(&self, url: &str, sink: &mut dyn FnMut(&[u8], Option<u64>) -> bool) -> Result<(), String> {
        let body = self.get(url)?;
        for c in body.chunks(4096) {
            if !sink(c, Some(body.len() as u64)) {
                return Err("cancelled".into());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;

    #[test]
    fn download_checks_size_and_checksum() {
        let t = TempDir::new("dl");
        let net = FakeNet::new(&[("https://x/a", b"abc")]);
        let abc = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        let dest = t.path().join("a.bin");
        assert_eq!(download(&net, "https://x/a", &dest, Some(abc), Some(3), &mut |_, _| true).unwrap(), abc);
        assert_eq!(std::fs::read(&dest).unwrap(), b"abc");
        let bad = t.path().join("b.bin");
        assert!(download(&net, "https://x/a", &bad, Some(&abc.replace('b', "c")), None, &mut |_, _| true).is_err());
        assert!(!bad.exists() && !t.path().join("b.bin.part").exists());
        assert!(download(&net, "https://x/a", &bad, None, Some(4), &mut |_, _| true).is_err());
        assert!(download(&net, "https://x/missing", &bad, None, None, &mut |_, _| true).is_err());
    }

    #[test]
    fn release_digest_is_read() {
        let j = json::parse(r#"{"tag_name":"v1.5.0","assets":[{"name":"simple-editor.exe","size":5,"digest":"sha256:00ff","browser_download_url":"https://g/se.exe"},{"name":"x.zip","size":1,"digest":null,"browser_download_url":"https://g/x"}]}"#).unwrap();
        let r = parse_release(&j).unwrap();
        assert_eq!(r.asset("Simple-Editor.exe").unwrap().sha256.as_deref(), Some("00ff"));
        assert_eq!(r.asset("x.zip").unwrap().sha256, None);
        assert!(parse_release(&json::parse(r#"{"message":"Not Found"}"#).unwrap()).unwrap_err().contains("Not Found"));
    }
}
