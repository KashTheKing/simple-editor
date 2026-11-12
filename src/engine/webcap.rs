//! ---- ws:html-layer ----
//! Web pages saved from a browser, as HTML layers. A page saved as a single `.mhtml` file (Brave /
//! Chrome / Edge: "Webpage, Single File") holds the HTML and every stylesheet, image and font in one
//! MIME archive; `mhtml_to_folder` unpacks it into `index.html` + `files/` with the links rewritten, so
//! the page renders from disk like any linked HTML (`engine::html`: `<base>`, linked CSS and images, a
//! full-page capture). Scripts are removed: the saved page is what the browser had drawn, not a program.
//! A page saved as "Webpage, Complete" (an `.html` file + a `_files` folder) needs no conversion.

use std::path::{Path, PathBuf};

/// One resource of the archive.
#[derive(Clone, Debug)]
pub struct Part {
    pub location: String,
    pub mime: String,
    pub data: Vec<u8>,
}

fn find(h: &[u8], n: &[u8], from: usize) -> Option<usize> {
    if n.is_empty() || h.len() < n.len() || from > h.len() - n.len() {
        return None;
    }
    h[from..].windows(n.len()).position(|w| w == n).map(|p| p + from)
}

/// (headers, offset of the body) of a block that starts at `at`: unfolded `name: value` lines, up to a
/// blank line.
fn headers(data: &[u8], at: usize) -> (Vec<(String, String)>, usize) {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut i = at;
    loop {
        let eol = data[i..].iter().position(|&b| b == b'\n').map_or(data.len(), |p| i + p);
        let line = String::from_utf8_lossy(&data[i..eol]).trim_end_matches('\r').to_string();
        i = (eol + 1).min(data.len());
        if line.is_empty() {
            return (out, i);
        }
        if line.starts_with([' ', '\t']) {
            if let Some(last) = out.last_mut() {
                last.1.push(' ');
                last.1.push_str(line.trim());
            }
        } else if let Some((k, v)) = line.split_once(':') {
            out.push((k.trim().to_ascii_lowercase(), v.trim().to_string()));
        }
        if i >= data.len() {
            return (out, i);
        }
    }
}

fn header<'a>(h: &'a [(String, String)], name: &str) -> Option<&'a str> {
    h.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
}

fn qp_decode(b: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'=' {
            if b.get(i + 1) == Some(&b'\r') && b.get(i + 2) == Some(&b'\n') {
                i += 3;
                continue;
            }
            if b.get(i + 1) == Some(&b'\n') {
                i += 2;
                continue;
            }
            if let (Some(h), Some(l)) = (b.get(i + 1), b.get(i + 2)) {
                if let Ok(v) = u8::from_str_radix(&String::from_utf8_lossy(&[*h, *l]), 16) {
                    out.push(v);
                    i += 3;
                    continue;
                }
            }
        }
        out.push(b[i]);
        i += 1;
    }
    out
}

pub(crate) fn b64_decode(s: &[u8]) -> Vec<u8> {
    let (mut out, mut acc, mut bits) = (Vec::with_capacity(s.len() / 4 * 3), 0u32, 0);
    for &c in s {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => continue,
        };
        acc = acc << 6 | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    out
}

fn b64_encode(d: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(d.len() / 3 * 4 + 4);
    for c in d.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for k in 0..4 {
            if k <= c.len() {
                s.push(T[(n >> (18 - 6 * k) & 63) as usize] as char);
            } else {
                s.push('=');
            }
        }
    }
    s
}

/// The parts of an MHTML archive, in order (the first is the page).
pub fn parse_mhtml(data: &[u8]) -> Result<Vec<Part>, String> {
    let (top, body_at) = headers(data, 0);
    let ct = header(&top, "content-type").unwrap_or("");
    let boundary = ct
        .split(';')
        .find_map(|p| p.trim().strip_prefix("boundary=").map(|b| b.trim_matches('"').to_string()))
        .ok_or("not an MHTML file (no multipart boundary)")?;
    let delim = format!("--{boundary}").into_bytes();
    let mut parts = Vec::new();
    let mut pos = find(data, &delim, body_at).ok_or("not an MHTML file (no parts)")?;
    loop {
        // the delimiter line
        let line_end = data[pos..].iter().position(|&b| b == b'\n').map_or(data.len(), |p| pos + p + 1);
        if data[pos + delim.len()..].starts_with(b"--") {
            break; // the closing delimiter
        }
        let (h, start) = headers(data, line_end);
        let next = {
            let mut probe = start;
            loop {
                match find(data, &delim, probe) {
                    Some(p) if p == 0 || data[p - 1] == b'\n' => break Some(p),
                    Some(p) => probe = p + 1,
                    None => break None,
                }
            }
        };
        let end = next.unwrap_or(data.len());
        let mut body = &data[start.min(end)..end];
        // the newline before the next delimiter belongs to the delimiter
        if body.ends_with(b"\r\n") {
            body = &body[..body.len() - 2];
        } else if body.ends_with(b"\n") {
            body = &body[..body.len() - 1];
        }
        let bytes = match header(&h, "content-transfer-encoding").map(str::to_ascii_lowercase).as_deref() {
            Some("base64") => b64_decode(body),
            Some("quoted-printable") => qp_decode(body),
            _ => body.to_vec(),
        };
        parts.push(Part {
            location: header(&h, "content-location").unwrap_or("").to_string(),
            mime: header(&h, "content-type").unwrap_or("").split(';').next().unwrap_or("").trim().to_ascii_lowercase(),
            data: bytes,
        });
        match next {
            Some(p) => pos = p,
            None => break,
        }
    }
    if parts.is_empty() {
        return Err("the MHTML file has no parts".into());
    }
    Ok(parts)
}

fn ext_of(mime: &str, location: &str) -> &'static str {
    match mime {
        "image/png" => "png",
        "image/jpeg" | "image/jpg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/svg+xml" => "svg",
        "image/avif" => "avif",
        "image/x-icon" | "image/vnd.microsoft.icon" => "ico",
        "text/css" => "css",
        "font/woff2" | "application/font-woff2" => "woff2",
        "font/woff" | "application/font-woff" => "woff",
        "font/ttf" | "application/x-font-ttf" => "ttf",
        "font/otf" => "otf",
        _ => {
            let p = location.split(['?', '#']).next().unwrap_or("").to_ascii_lowercase();
            ["png", "jpg", "jpeg", "gif", "webp", "svg", "css", "woff2", "woff", "ttf", "otf", "ico"]
                .into_iter()
                .find(|e| p.ends_with(&format!(".{e}")))
                .unwrap_or("bin")
        }
    }
}

fn is_font(mime: &str, location: &str) -> bool {
    mime.contains("font") || matches!(ext_of(mime, location), "woff2" | "woff" | "ttf" | "otf")
}

fn name_of(location: &str) -> String {
    let path = location.split(['?', '#']).next().unwrap_or("");
    let base = path.rsplit('/').next().unwrap_or("");
    let stem = base.rsplit_once('.').map_or(base, |(s, _)| s);
    let s: String = stem.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' }).take(40).collect();
    if s.is_empty() { "file".into() } else { s }
}

/// Unpack `data` (an MHTML archive) into `dir`: `index.html` and `files/`. Returns the path of
/// `index.html`. Links to resources in the archive point at the unpacked files (fonts are inlined as
/// `data:` URLs, which the page may load from a file), scripts and `<base>` tags are removed.
pub fn mhtml_to_folder(data: &[u8], dir: &Path) -> Result<PathBuf, String> {
    let parts = parse_mhtml(data)?;
    let main = parts.iter().position(|p| p.mime == "text/html").unwrap_or(0);
    let files = dir.join("files");
    std::fs::create_dir_all(&files).map_err(|e| format!("{}: {e}", files.display()))?;
    // (location, replacement from the page, replacement from a stylesheet next to the resources)
    let mut map: Vec<(String, String, String)> = Vec::new();
    let mut css: Vec<(usize, String)> = Vec::new();
    for (i, p) in parts.iter().enumerate() {
        // Blink names the stylesheets it embeds `cid:css-<uuid>@mhtml.blink` and links to them that way
        if i == main || p.location.is_empty() || !(p.location.starts_with("http") || p.location.starts_with("file:") || p.location.starts_with("cid:")) {
            continue;
        }
        if p.mime == "text/html" {
            continue; // a frame: not carried along
        }
        if is_font(&p.mime, &p.location) {
            let uri = format!("data:{};base64,{}", if p.mime.is_empty() { "font/woff2" } else { &p.mime }, b64_encode(&p.data));
            map.push((p.location.clone(), uri.clone(), uri));
            continue;
        }
        let file = format!("{i:03}-{}.{}", name_of(&p.location), ext_of(&p.mime, &p.location));
        if p.mime == "text/css" {
            css.push((i, file.clone()));
        } else {
            std::fs::write(files.join(&file), &p.data).map_err(|e| format!("{file}: {e}"))?;
        }
        map.push((p.location.clone(), format!("files/{file}"), file));
    }
    map.sort_by_key(|(l, _, _)| std::cmp::Reverse(l.len()));
    let rewrite = |mut s: String, from_css: bool| {
        for (loc, page, sheet) in &map {
            let to = if from_css { sheet } else { page };
            if s.contains(loc.as_str()) {
                s = s.replace(loc.as_str(), to);
            }
            let esc = loc.replace('&', "&amp;");
            if esc != *loc && s.contains(&esc) {
                s = s.replace(&esc, to);
            }
        }
        s
    };
    for (i, file) in &css {
        let text = rewrite(String::from_utf8_lossy(&parts[*i].data).into_owned(), true);
        std::fs::write(files.join(file), text).map_err(|e| format!("{file}: {e}"))?;
    }
    let html = rewrite(String::from_utf8_lossy(&parts[main].data).into_owned(), false);
    let html = crate::engine::html::strip_base_tags(&crate::engine::html::sanitize(&html));
    let index = dir.join("index.html");
    std::fs::write(&index, html).map_err(|e| format!("{}: {e}", index.display()))?;
    Ok(index)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn archive() -> Vec<u8> {
        let mut a = String::new();
        a += "From: <Saved by Blink>\r\nSubject: A page\r\nMIME-Version: 1.0\r\n";
        a += "Content-Type: multipart/related; type=\"text/html\"; boundary=\"----MultipartBoundary--abc123\"\r\n\r\n";
        a += "------MultipartBoundary--abc123\r\nContent-Type: text/html\r\nContent-ID: <frame-1@mhtml.blink>\r\n";
        a += "Content-Transfer-Encoding: quoted-printable\r\nContent-Location: https://site.example/page\r\n\r\n";
        a += "<!doctype html><html><head><title>Caf=C3=A9 page</title><base href=3D\"https://site.example/\">=\r\n";
        a += "<link rel=3D\"stylesheet\" href=3D\"https://cdn.example/s.css?v=2&amp;x=1\"><link rel=3D\"stylesheet\" type=3D\"text/css\" href=3D\"cid:css-9@mhtml.blink\" /><script>alert(1)</script></head>=\r\n";
        a += "<body onload=3D\"x()\"><img src=3D\"https://cdn.example/a/logo.png\"></body></html>\r\n";
        a += "------MultipartBoundary--abc123\r\nContent-Type: text/css\r\nContent-Transfer-Encoding: quoted-printable\r\n";
        a += "Content-Location: https://cdn.example/s.css?v=2&x=1\r\n\r\n";
        a += "@font-face{src:url(https://cdn.example/f.woff2)} body{background:url(https://cdn.example/a/logo.png)}\r\n";
        a += "------MultipartBoundary--abc123\r\nContent-Type: image/png\r\nContent-Transfer-Encoding: base64\r\n";
        a += "Content-Location: https://cdn.example/a/logo.png\r\n\r\niVBORw0KGgo=\r\n";
        a += "------MultipartBoundary--abc123\r\nContent-Type: font/woff2\r\nContent-Transfer-Encoding: base64\r\n";
        a += "Content-Location: https://cdn.example/f.woff2\r\n\r\nd09GMg==\r\n";
        a += "------MultipartBoundary--abc123\r\nContent-Type: text/css\r\nContent-Transfer-Encoding: quoted-printable\r\n";
        a += "Content-Location: cid:css-9@mhtml.blink\r\n\r\nh1{color:red}\r\n";
        a += "------MultipartBoundary--abc123--\r\n";
        a.into_bytes()
    }

    #[test]
    fn an_mhtml_archive_is_read_part_by_part() {
        let parts = parse_mhtml(&archive()).unwrap();
        assert_eq!(parts.len(), 5);
        assert_eq!(parts[0].mime, "text/html");
        assert!(String::from_utf8_lossy(&parts[0].data).contains("<title>Café page</title>"), "quoted-printable and soft breaks");
        assert_eq!(parts[2].data, [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A], "base64");
        assert_eq!(parts[2].location, "https://cdn.example/a/logo.png");
        assert!(parse_mhtml(b"hello").is_err());
    }

    #[test]
    fn an_archive_unpacks_into_a_page_that_renders_from_disk() {
        let dir = std::env::temp_dir().join(format!("se-mhtml-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let index = mhtml_to_folder(&archive(), &dir).unwrap();
        let html = std::fs::read_to_string(&index).unwrap();
        assert!(html.contains("href=\"files/001-s.css\""), "the stylesheet link points at the file: {html}");
        assert!(html.contains("src=\"files/002-logo.png\""), "{html}");
        assert!(html.contains("href=\"files/004-cid-css-9-mhtml.css\""), "a cid: stylesheet is unpacked too: {html}");
        assert_eq!(std::fs::read_to_string(dir.join("files/004-cid-css-9-mhtml.css")).unwrap().trim(), "h1{color:red}");
        assert!(!html.contains("<script") && !html.contains("onload") && !html.contains("<base"), "{html}");
        let css = std::fs::read_to_string(dir.join("files/001-s.css")).unwrap();
        assert!(css.contains("url(data:font/woff2;base64,d09GMg==)"), "fonts are inlined: {css}");
        assert!(css.contains("url(002-logo.png)"), "a stylesheet refers to its neighbours by name: {css}");
        assert_eq!(std::fs::read(dir.join("files/002-logo.png")).unwrap().len(), 8);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn base64_round_trips() {
        for d in [&b""[..], b"a", b"ab", b"abc", b"abcd", &[0u8, 255, 128, 7, 9]] {
            assert_eq!(b64_decode(b64_encode(d).as_bytes()), d);
        }
    }
}
