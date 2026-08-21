//! Explorer context menu: "Edit with Simple Editor" for video files (HKCU, no admin needed).
//! Appears in the classic menu ("Show more options" on Windows 11). Registered per extension because
//! several video extensions (.mov/.m4v/.flv/.ts/.mts/.mpg) have no PerceivedType=video.

use std::io::ErrorKind;
use winreg::enums::HKEY_CURRENT_USER;
use winreg::RegKey;

const VIDEO_EXTS: &[&str] = &[
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
