//! ---- feat/plugins ----
//! Where a plugin's zip comes from: a pasted GitHub link (a repo, a release, a tag, or a folder on a
//! branch) is turned into the GitHub API call that finds the latest release and, when that release has
//! no zip asset, the codeload archive of the default branch (or the named ref). Pure functions over
//! strings, so every case is tested without the network; `fetch` does the downloads with `curl.exe`.

use serde_json::Value;

/// What a pasted link points at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Source {
    pub owner: String,
    pub repo: String,
    pub at: At,
    /// A folder inside the repository that holds `plugin.json` (`tree/<ref>/<path>` links), "" = the root.
    pub path: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum At {
    /// The latest release, else the default branch.
    Latest,
    /// A release by its tag (`releases/tag/v1.2`).
    Release(String),
    /// A branch, tag or commit (`tree/<ref>`, `archive/<ref>.zip`): no release lookup.
    Ref(String),
}

impl Source {
    pub fn repo_url(&self) -> String {
        format!("https://github.com/{}/{}", self.owner, self.repo)
    }
    /// The API call that lists the release to install, if this source uses releases.
    pub fn release_api(&self) -> Option<String> {
        let base = format!("https://api.github.com/repos/{}/{}/releases", self.owner, self.repo);
        match &self.at {
            At::Latest => Some(format!("{base}/latest")),
            At::Release(tag) => Some(format!("{base}/tags/{tag}")),
            At::Ref(_) => None,
        }
    }
    /// The source archive of `reference` (`HEAD` = the default branch) from codeload.
    pub fn archive_url(&self, reference: &str) -> String {
        format!("https://codeload.github.com/{}/{}/zip/{}", self.owner, self.repo, reference)
    }
    /// The archive used when there is no release zip: the named ref, the release's tag, or the default branch.
    pub fn fallback_archive(&self) -> String {
        match &self.at {
            At::Latest => self.archive_url("HEAD"),
            At::Release(r) | At::Ref(r) => self.archive_url(r),
        }
    }
}

fn clean(seg: &str) -> Option<String> {
    let ok = !seg.is_empty() && seg != "." && seg != ".." && seg.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c));
    ok.then(|| seg.to_string())
}

/// Read a pasted GitHub link: `https://github.com/owner/repo`, `github.com/owner/repo.git`, `owner/repo`,
/// `.../releases/latest`, `.../releases/tag/<tag>`, `.../tree/<ref>/<folder>`, `.../archive/<ref>.zip`.
/// Anything else (another host, a missing repo name) is an error that says what was expected.
pub fn parse(url: &str) -> Result<Source, String> {
    let bad = || format!("'{url}' is not a GitHub repository link (expected https://github.com/owner/repo)");
    let mut s = url.trim().trim_end_matches('/');
    for p in ["https://", "http://"] {
        if let Some(r) = s.strip_prefix(p) {
            s = r;
        }
    }
    let s = s.strip_prefix("www.").unwrap_or(s);
    let rest = match s.strip_prefix("github.com/") {
        Some(r) => r,
        None if !s.contains('.') || s.split('/').count() == 2 && !s.contains(':') && !s.starts_with("github.com") => s,
        None => return Err(bad()),
    };
    let rest = rest.split(['?', '#']).next().unwrap_or("");
    let segs: Vec<&str> = rest.split('/').filter(|x| !x.is_empty()).collect();
    if segs.len() < 2 {
        return Err(bad());
    }
    let owner = clean(segs[0]).ok_or_else(bad)?;
    let repo = clean(segs[1].trim_end_matches(".git")).ok_or_else(bad)?;
    let mut src = Source { owner, repo, at: At::Latest, path: String::new() };
    match segs.get(2..).unwrap_or(&[]) {
        [] | ["releases"] | ["releases", "latest"] => {}
        ["releases", "tag", tag, ..] => src.at = At::Release(tag.to_string()),
        ["tree" | "blob", reference, path @ ..] => {
            src.at = At::Ref(reference.to_string());
            let path: Vec<&str> = path.iter().copied().filter(|p| *p != "plugin.json").collect();
            if path.iter().any(|p| *p == "..") {
                return Err(bad());
            }
            src.path = path.join("/");
        }
        ["archive", file] | ["archive", "refs", "heads" | "tags", file] => {
            src.at = At::Ref(file.trim_end_matches(".zip").to_string());
        }
        _ => return Err(bad()),
    }
    Ok(src)
}

/// The release a GitHub API reply describes: its tag and the download URL of its first `.zip` asset
/// (None = a release without a zip; install from the tag's source archive then).
pub fn release_zip(api_reply: &str) -> Result<(String, Option<String>), String> {
    let v: Value = serde_json::from_str(api_reply).map_err(|e| format!("GitHub's reply could not be read: {e}"))?;
    if let Some(msg) = v.get("message").and_then(Value::as_str) {
        if v.get("tag_name").is_none() {
            return Err(format!("GitHub: {msg}"));
        }
    }
    let tag = v.get("tag_name").and_then(Value::as_str).unwrap_or_default().to_string();
    let zip = v
        .get("assets")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|a| a.get("name").and_then(Value::as_str).is_some_and(|n| n.to_ascii_lowercase().ends_with(".zip")))
        .and_then(|a| a.get("browser_download_url").and_then(Value::as_str))
        .map(str::to_string);
    Ok((tag, zip))
}

/// Strip a leading `v` and compare dotted numbers: "1.10.0" > "1.9.2". Non-numeric parts compare as 0.
pub fn newer(candidate: &str, installed: &str) -> bool {
    let nums = |s: &str| -> Vec<u64> {
        s.trim().trim_start_matches(['v', 'V']).split(['.', '-', '+']).take(4).map(|p| p.parse().unwrap_or(0)).collect()
    };
    let (a, b) = (nums(candidate), nums(installed));
    for i in 0..a.len().max(b.len()) {
        let (x, y) = (a.get(i).copied().unwrap_or(0), b.get(i).copied().unwrap_or(0));
        if x != y {
            return x > y;
        }
    }
    false
}

/// `curl.exe` `url` and return the body. The GitHub API wants a User-Agent; a generic one is sent (no
/// personal data). Blocking: background threads only.
pub fn get(url: &str, accept_json: bool) -> Result<Vec<u8>, String> {
    let curl = crate::media::ffpipe::find_exe("curl.exe", "").ok_or("curl.exe not found (it ships with Windows 10 1803+)")?;
    let mut cmd = crate::media::ffpipe::command(&curl);
    cmd.args(["-L", "--fail", "-sS", "--max-time", "120", "--max-filesize", "268435456", "-A", "SimpleEditor-plugins"]);
    if accept_json {
        cmd.args(["-H", "Accept: application/vnd.github+json"]);
    }
    let out = cmd
        .arg(url)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("curl: {e}"))?;
    if !out.status.success() {
        let msg = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(if msg.is_empty() { format!("download failed: {url}") } else { msg });
    }
    Ok(out.stdout)
}

/// The zip to install for `src`, and the version tag it came from ("" = a branch archive). A source
/// without a release (the API answers 404) falls back to the branch archive.
pub fn fetch(src: &Source) -> Result<(Vec<u8>, String), String> {
    if let Some(api) = src.release_api() {
        if let Ok(body) = get(&api, true) {
            if let Ok((tag, zip)) = release_zip(&String::from_utf8_lossy(&body)) {
                if let Some(z) = zip {
                    return Ok((get(&z, false)?, tag));
                }
                if !tag.is_empty() {
                    return Ok((get(&src.archive_url(&tag), false)?, tag));
                }
            }
        }
    }
    let tag = match &src.at {
        At::Release(t) => t.clone(),
        _ => String::new(),
    };
    Ok((get(&src.fallback_archive(), false)?, tag))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn github_links_parse_into_repo_release_ref_and_folder() {
        let s = parse("https://github.com/Kash/hello-panel").unwrap();
        assert_eq!((s.owner.as_str(), s.repo.as_str(), &s.at, s.path.as_str()), ("Kash", "hello-panel", &At::Latest, ""));
        assert_eq!(s.release_api().unwrap(), "https://api.github.com/repos/Kash/hello-panel/releases/latest");
        assert_eq!(s.fallback_archive(), "https://codeload.github.com/Kash/hello-panel/zip/HEAD");
        assert_eq!(parse("github.com/a/b.git/").unwrap().repo, "b");
        assert_eq!(parse("a/b").unwrap().owner, "a");
        let r = parse("https://github.com/a/b/releases/tag/v1.2.0").unwrap();
        assert_eq!(r.at, At::Release("v1.2.0".into()));
        assert_eq!(r.release_api().unwrap(), "https://api.github.com/repos/a/b/releases/tags/v1.2.0");
        let t = parse("https://github.com/a/b/tree/main/plugins/hello").unwrap();
        assert_eq!((t.at.clone(), t.path.as_str()), (At::Ref("main".into()), "plugins/hello"));
        assert!(t.release_api().is_none());
        assert_eq!(t.fallback_archive(), "https://codeload.github.com/a/b/zip/main");
        assert_eq!(parse("https://github.com/a/b/archive/refs/tags/v2.zip").unwrap().at, At::Ref("v2".into()));
        for bad in ["https://gitlab.com/a/b", "https://github.com/a", "", "https://github.com/a/b/issues/3", "https://github.com/../b"] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn release_reply_gives_tag_and_zip_asset() {
        let reply = r#"{"tag_name":"v1.3.0","assets":[
            {"name":"notes.txt","browser_download_url":"https://x/notes.txt"},
            {"name":"hello-panel.zip","browser_download_url":"https://github.com/a/b/releases/download/v1.3.0/hello-panel.zip"}]}"#;
        let (tag, zip) = release_zip(reply).unwrap();
        assert_eq!(tag, "v1.3.0");
        assert_eq!(zip.unwrap(), "https://github.com/a/b/releases/download/v1.3.0/hello-panel.zip");
        assert_eq!(release_zip(r#"{"tag_name":"v1","assets":[]}"#).unwrap(), ("v1".into(), None));
        assert!(release_zip(r#"{"message":"Not Found"}"#).unwrap_err().contains("Not Found"));
    }

    #[test]
    fn versions_compare_numerically() {
        assert!(newer("1.10.0", "1.9.9"));
        assert!(newer("v2", "1.99"));
        assert!(!newer("1.0.0", "1.0"));
        assert!(!newer("0.9", "1.0.0"));
    }
}
