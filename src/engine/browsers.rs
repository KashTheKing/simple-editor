//! The user's web browser: the Windows default (the https `UserChoice` ProgId -> its open command) or
//! the one picked in Settings ▸ General ▸ Web browser. Used wherever the app sends the user to a
//! browser: links, the snapshot extension's install steps, the folder the extension's snapshots land in.
//! (The headless page renderer in `engine::html` is not this: it is part of the app, not the user's.)

use std::path::{Path, PathBuf};
use std::sync::RwLock;
use winreg::enums::{HKEY_CLASSES_ROOT, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
use winreg::RegKey;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    /// Chrome, Edge, Brave, Vivaldi, Opera, Arc, Chromium: the snapshot extension works as is.
    Chromium,
    /// Firefox: no "save as one file" (MHTML), so no extension; Ctrl+S ▸ Web Page, complete instead.
    Firefox,
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Browser {
    pub name: String,
    pub exe: PathBuf,
}

/// Known browsers by exe file name: (exe stem, name, family, extensions page, profile folder under
/// %LOCALAPPDATA% or, for Opera, %APPDATA%; "" = unknown).
const KNOWN: &[(&str, &str, Family, &str, &str)] = &[
    ("chrome", "Google Chrome", Family::Chromium, "chrome://extensions", r"L:Google\Chrome\User Data\Default"),
    ("msedge", "Microsoft Edge", Family::Chromium, "edge://extensions", r"L:Microsoft\Edge\User Data\Default"),
    ("brave", "Brave", Family::Chromium, "brave://extensions", r"L:BraveSoftware\Brave-Browser\User Data\Default"),
    ("vivaldi", "Vivaldi", Family::Chromium, "vivaldi://extensions", r"L:Vivaldi\User Data\Default"),
    ("opera", "Opera", Family::Chromium, "opera://extensions", r"R:Opera Software\Opera Stable"),
    ("arc", "Arc", Family::Chromium, "chrome://extensions", ""),
    ("chromium", "Chromium", Family::Chromium, "chrome://extensions", r"L:Chromium\User Data\Default"),
    ("firefox", "Firefox", Family::Firefox, "about:debugging#/runtime/this-firefox", ""),
];

impl Browser {
    fn known(&self) -> Option<&'static (&'static str, &'static str, Family, &'static str, &'static str)> {
        let stem = self.exe.file_stem()?.to_string_lossy().to_ascii_lowercase();
        // Opera's own launcher is `launcher.exe` inside its folder
        let stem = if stem == "launcher" && self.exe.to_string_lossy().to_ascii_lowercase().contains("opera") { "opera".into() } else { stem };
        KNOWN.iter().find(|k| k.0 == stem)
    }
    /// A browser for an exe path, named from the table (or the file name).
    pub fn from_exe(exe: &Path) -> Self {
        let mut b = Self { name: String::new(), exe: exe.to_path_buf() };
        b.name = match b.known() {
            Some(k) if k.0 == "opera" && exe.to_string_lossy().to_ascii_lowercase().contains("opera gx") => "Opera GX".into(),
            Some(k) => k.1.into(),
            None => exe.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Browser".into()),
        };
        b
    }
    pub fn family(&self) -> Family {
        self.known().map_or(Family::Other, |k| k.2)
    }
    /// The address of the page that loads an unpacked extension, when known.
    pub fn extensions_page(&self) -> Option<&'static str> {
        self.known().map(|k| k.3)
    }
    /// The browser's own profile folder (where its `Preferences` live), when known.
    fn profile(&self) -> Option<PathBuf> {
        let k = self.known()?;
        let gx = self.name == "Opera GX";
        let rel = if gx { r"R:Opera Software\Opera GX Stable" } else { k.4 };
        let (var, rel) = rel.split_once(':')?;
        let base = std::env::var_os(if var == "L" { "LOCALAPPDATA" } else { "APPDATA" })?;
        Some(PathBuf::from(base).join(rel))
    }
    /// Where this browser saves downloads: its own setting when it has changed it, else the Windows
    /// Downloads folder. ponytail: reads the Default profile only; other profiles use the Windows folder.
    pub fn download_dirs(&self) -> Vec<PathBuf> {
        let mut v = vec![windows_downloads()];
        let custom = self.profile().and_then(|p| std::fs::read_to_string(p.join("Preferences")).ok()).and_then(|t| chromium_download_dir(&t));
        if let Some(d) = custom.filter(|d| !v.contains(d)) {
            v.insert(0, d);
        }
        v
    }
    /// The steps that load the snapshot extension in this browser, one string per step.
    pub fn extension_steps(&self) -> Vec<String> {
        match (self.family(), self.extensions_page()) {
            (Family::Firefox, _) => vec![
                "Firefox cannot save a page as one file (.mhtml), so the extension does not work there.".into(),
                "Instead press Ctrl+S on the page, pick \"Web Page, complete\", then use \"Import a saved page…\" below.".into(),
            ],
            (_, page) => vec![
                format!(
                    "In {} open {}, turn on Developer mode, click Load unpacked and pick that folder.",
                    self.name,
                    page.unwrap_or("the extensions page")
                ),
                "On any page click the extension's button (or press Alt+Shift+S). The page appears here.".into(),
            ],
        }
    }
    /// Open `url` in this browser.
    pub fn open(&self, url: &str) -> Result<(), String> {
        std::process::Command::new(&self.exe).arg(url).spawn().map(|_| ()).map_err(|e| format!("{}: {e}", self.name))
    }
}

/// The exe in a shell open command: `"C:\…\chrome.exe" --single-argument %1` or an unquoted path.
pub fn exe_from_command(cmd: &str) -> Option<PathBuf> {
    let cmd = cmd.trim();
    let exe = if let Some(rest) = cmd.strip_prefix('"') {
        rest.split('"').next()?
    } else {
        let lower = cmd.to_ascii_lowercase();
        &cmd[..lower.find(".exe").map(|i| i + 4).unwrap_or(cmd.len())]
    };
    let exe = exe.trim();
    (!exe.is_empty()).then(|| PathBuf::from(exe))
}

/// `download.default_directory` in a Chromium `Preferences` file.
pub fn chromium_download_dir(prefs: &str) -> Option<PathBuf> {
    let v: serde_json::Value = serde_json::from_str(prefs).ok()?;
    let d = v.get("download")?.get("default_directory")?.as_str()?;
    (!d.trim().is_empty()).then(|| PathBuf::from(d))
}

/// The Windows Downloads folder (moved or not).
pub fn windows_downloads() -> PathBuf {
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Explorer\Shell Folders")
        .and_then(|k| k.get_value::<String, _>("{374DE290-123F-4565-9164-39C4925E467B}"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::var_os("USERPROFILE").map(|h| PathBuf::from(h).join("Downloads")).unwrap_or_default())
}

/// The default browser's ProgId for https (Windows 11 24H2 keeps it in `UserChoiceLatest`).
fn default_prog_id() -> Option<String> {
    let base = r"Software\Microsoft\Windows\Shell\Associations\UrlAssociations";
    let hk = RegKey::predef(HKEY_CURRENT_USER);
    ["https", "http"].iter().find_map(|scheme| {
        [format!(r"{base}\{scheme}\UserChoiceLatest\ProgId"), format!(r"{base}\{scheme}\UserChoice")]
            .iter()
            .find_map(|k| hk.open_subkey(k).and_then(|k| k.get_value::<String, _>("ProgId")).ok())
            .filter(|p| !p.is_empty())
    })
}

/// The Windows default browser: its ProgId's open command, named by `Application\ApplicationName`.
pub fn default_browser() -> Option<Browser> {
    let id = default_prog_id()?;
    let cls = RegKey::predef(HKEY_CLASSES_ROOT);
    let cmd: String = cls.open_subkey(format!(r"{id}\shell\open\command")).and_then(|k| k.get_value("")).ok()?;
    let mut b = Browser::from_exe(&exe_from_command(&cmd)?);
    if b.known().is_none() {
        if let Ok(n) = cls.open_subkey(format!(r"{id}\Application")).and_then(|k| k.get_value::<String, _>("ApplicationName")) {
            if !n.starts_with('@') {
                b.name = n;
            }
        }
    }
    Some(b)
}

/// Every browser registered with Windows (`Clients\StartMenuInternet`, machine and user), by name.
pub fn installed() -> Vec<Browser> {
    let mut v: Vec<Browser> = Vec::new();
    for hk in [HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER] {
        let Ok(root) = RegKey::predef(hk).open_subkey(r"SOFTWARE\Clients\StartMenuInternet") else { continue };
        for name in root.enum_keys().flatten() {
            let cmd = root.open_subkey(format!(r"{name}\shell\open\command")).and_then(|k| k.get_value::<String, _>(""));
            let Some(exe) = cmd.ok().and_then(|c| exe_from_command(&c)) else { continue };
            if name.eq_ignore_ascii_case("IEXPLORE.EXE") || !exe.is_file() || v.iter().any(|b| b.exe == exe) {
                continue;
            }
            v.push(Browser::from_exe(&exe));
        }
    }
    v.sort_by(|a, b| a.name.cmp(&b.name));
    v
}

/// `Settings.web_browser` as last applied ("" = the Windows default).
static CHOICE: RwLock<String> = RwLock::new(String::new());

/// Remember the browser picked in Settings (an exe path, "" = the Windows default).
pub fn set_choice(exe: &str) {
    *CHOICE.write().unwrap_or_else(|e| e.into_inner()) = exe.trim().to_string();
}

/// The browser to send the user to: the one picked in Settings, else the Windows default.
pub fn current() -> Option<Browser> {
    let c = CHOICE.read().unwrap_or_else(|e| e.into_inner()).clone();
    if c.is_empty() {
        default_browser()
    } else {
        Some(Browser::from_exe(Path::new(&c)))
    }
}

/// Open a web address in the user's browser (a file or folder: Explorer, as before).
pub fn open_url(url: &str) {
    let web = url.starts_with("http://") || url.starts_with("https://");
    let picked = !CHOICE.read().unwrap_or_else(|e| e.into_inner()).is_empty();
    if web && picked {
        if let Some(b) = current() {
            if b.open(url).is_ok() {
                return;
            }
        }
    }
    let _ = std::process::Command::new("explorer").arg(url).spawn();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_exe_is_read_from_quoted_and_bare_open_commands() {
        let c = r#""C:\Program Files\BraveSoftware\Brave-Browser\Application\brave.exe" --single-argument %1"#;
        assert_eq!(exe_from_command(c), Some(PathBuf::from(r"C:\Program Files\BraveSoftware\Brave-Browser\Application\brave.exe")));
        assert_eq!(exe_from_command(r"C:\Firefox\firefox.exe -osint -url %1"), Some(PathBuf::from(r"C:\Firefox\firefox.exe")));
        assert_eq!(exe_from_command("  "), None);
    }

    #[test]
    fn browsers_are_named_and_sorted_into_families_by_exe() {
        let b = |p: &str| Browser::from_exe(Path::new(p));
        let cases = [
            (r"C:\Program Files\Google\Chrome\Application\chrome.exe", "Google Chrome", Family::Chromium, Some("chrome://extensions")),
            (r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe", "Microsoft Edge", Family::Chromium, Some("edge://extensions")),
            (r"C:\Users\u\AppData\Local\Vivaldi\Application\vivaldi.exe", "Vivaldi", Family::Chromium, Some("vivaldi://extensions")),
            (r"C:\Users\u\AppData\Local\Programs\Opera GX\launcher.exe", "Opera GX", Family::Chromium, Some("opera://extensions")),
            (r"C:\Program Files\Mozilla Firefox\firefox.exe", "Firefox", Family::Firefox, Some("about:debugging#/runtime/this-firefox")),
            (r"C:\Tools\SomeBrowser.exe", "SomeBrowser", Family::Other, None),
        ];
        for (exe, name, fam, page) in cases {
            let x = b(exe);
            assert_eq!((x.name.as_str(), x.family(), x.extensions_page()), (name, fam, page), "{exe}");
        }
    }

    #[test]
    fn the_extension_steps_name_the_chosen_browser_and_its_extensions_page() {
        let chrome = Browser::from_exe(Path::new(r"C:\x\chrome.exe")).extension_steps().join(" ");
        assert!(chrome.contains("Google Chrome") && chrome.contains("chrome://extensions"));
        let edge = Browser::from_exe(Path::new(r"C:\x\msedge.exe")).extension_steps().join(" ");
        assert!(edge.contains("edge://extensions") && !edge.contains("chrome://"));
        let ff = Browser::from_exe(Path::new(r"C:\x\firefox.exe")).extension_steps().join(" ");
        assert!(ff.contains("Web Page, complete") && !ff.contains("Load unpacked"), "Firefox gets the saved-page route");
        let other = Browser::from_exe(Path::new(r"C:\x\web.exe")).extension_steps().join(" ");
        assert!(other.contains("the extensions page"));
    }

    #[test]
    fn a_chromium_download_folder_is_read_from_its_preferences() {
        assert_eq!(chromium_download_dir(r#"{"download":{"default_directory":"D:\\Dl"}}"#), Some(PathBuf::from(r"D:\Dl")));
        assert_eq!(chromium_download_dir(r#"{"download":{"prompt_for_download":true}}"#), None);
        assert_eq!(chromium_download_dir("not json"), None);
    }
}
