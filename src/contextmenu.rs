//! Explorer context menu: "Edit with Simple Editor" for video files (HKCU, no admin needed).
//! Appears in the classic menu ("Show more options" on Windows 11). Registered per extension because
//! several video extensions (.mov/.m4v/.flv/.ts/.mts/.mpg) have no PerceivedType=video.

use std::io::ErrorKind;
use winreg::enums::HKEY_CURRENT_USER;
use winreg::RegKey;

pub(crate) const VIDEO_EXTS: &[&str] = &[
    "mp4", "mov", "mkv", "webm", "avi", "m4v", "wmv", "ts", "m2ts", "mts", "flv", "3gp", "mpg", "mpeg", "gif", "ogv",
    "vob", "divx", "asf", "f4v", "dv", "mxf",
];
/// Written by older builds; removed on uninstall.
const LEGACY_KEY: &str = r"Software\Classes\SystemFileAssociations\video\shell\SimpleEditor";

fn key(ext: &str) -> String {
    format!(r"Software\Classes\SystemFileAssociations\.{ext}\shell\SimpleEditor")
}

fn exe() -> String {
    std::env::current_exe().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default()
}

/// True if the menu entry exists and points at this executable.
pub fn is_installed() -> bool {
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(format!(r"{}\command", key("mp4")))
        .ok()
        .and_then(|k| k.get_value::<String, _>("").ok())
        .map(|v| v.to_ascii_lowercase().contains(&exe().to_ascii_lowercase()))
        .unwrap_or(false)
}

pub fn install() -> std::io::Result<()> {
    let hk = RegKey::predef(HKEY_CURRENT_USER);
    let _ = hk.delete_subkey_all(LEGACY_KEY); // would duplicate the entry for .mp4 etc.
    let exe = exe();
    for ext in VIDEO_EXTS {
        let key = key(ext);
        let (k, _) = hk.create_subkey(&key)?;
        k.set_value("", &"Edit with Simple Editor")?;
        k.set_value("Icon", &format!("\"{exe}\",0"))?;
        let (c, _) = hk.create_subkey(format!(r"{key}\command"))?;
        c.set_value("", &format!("\"{exe}\" \"%1\""))?;
    }
    notify();
    Ok(())
}

pub fn uninstall() -> std::io::Result<()> {
    let hk = RegKey::predef(HKEY_CURRENT_USER);
    let _ = hk.delete_subkey_all(LEGACY_KEY);
    let mut r = Ok(());
    for ext in VIDEO_EXTS {
        match hk.delete_subkey_all(key(ext)) {
            Err(e) if e.kind() != ErrorKind::NotFound => r = Err(e),
            _ => {}
        }
    }
    notify();
    r
}

fn notify() {
    use windows::Win32::UI::Shell::{SHChangeNotify, SHCNE_ASSOCCHANGED, SHCNF_IDLIST};
    unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None) };
}

/// Show `path` in File Explorer: its folder opens with the file selected (a folder path opens itself;
/// a missing file opens the nearest existing parent). The one place every "Reveal / Open file location"
/// goes through: explorer only understands `/select,"C:\dir\file"` as ONE raw argument with backslashes -
/// std's `.arg()` quotes the whole `/select,...` when the path has spaces (explorer then opens Documents)
/// and asset paths are stored with forward slashes (no match, nothing selected).
pub(crate) fn reveal(path: &std::path::Path) {
    let win = path.to_string_lossy().replace('/', "\\");
    let p = std::path::Path::new(&win);
    let mut cmd = std::process::Command::new("explorer.exe");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        if p.is_file() {
            cmd.raw_arg(format!("/select,\"{win}\""));
        } else if p.is_dir() {
            cmd.raw_arg(format!("\"{win}\""));
        } else if let Some(d) = p.ancestors().skip(1).find(|a| a.is_dir()) {
            cmd.raw_arg(format!("\"{}\"", d.display()));
        }
    }
    let _ = cmd.spawn();
}

/// The argument `reveal` hands explorer for a file (pure, for the test).
#[cfg(test)]
fn select_arg(path: &str) -> String {
    format!("/select,\"{}\"", path.replace('/', "\\"))
}

#[cfg(test)]
mod reveal_tests {
    #[test]
    fn select_is_one_quoted_backslash_argument() {
        assert_eq!(super::select_arg("C:/My Videos/a b.mp4"), r#"/select,"C:\My Videos\a b.mp4""#);
    }
}
