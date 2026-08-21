//! Explorer context menu: "Edit with Simple Editor" for video files (HKCU, no admin needed).
//! Appears in the classic menu ("Show more options" on Windows 11).

use winreg::enums::HKEY_CURRENT_USER;
use winreg::RegKey;

const KEY: &str = r"Software\Classes\SystemFileAssociations\video\shell\SimpleEditor";

fn exe() -> String {
    std::env::current_exe().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default()
}

/// True if the menu entry exists and points at this executable.
pub fn is_installed() -> bool {
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(format!(r"{KEY}\command"))
        .ok()
        .and_then(|k| k.get_value::<String, _>("").ok())
        .map(|v| v.to_ascii_lowercase().contains(&exe().to_ascii_lowercase()))
        .unwrap_or(false)
}

pub fn install() -> std::io::Result<()> {
    let hk = RegKey::predef(HKEY_CURRENT_USER);
    let (k, _) = hk.create_subkey(KEY)?;
    k.set_value("", &"Edit with Simple Editor")?;
    k.set_value("Icon", &format!("\"{}\",0", exe()))?;
    let (c, _) = hk.create_subkey(format!(r"{KEY}\command"))?;
    c.set_value("", &format!("\"{}\" \"%1\"", exe()))?;
    notify();
    Ok(())
}

pub fn uninstall() -> std::io::Result<()> {
    let r = RegKey::predef(HKEY_CURRENT_USER).delete_subkey_all(KEY);
    notify();
    r
}

fn notify() {
    use windows::Win32::UI::Shell::{SHChangeNotify, SHCNE_ASSOCCHANGED, SHCNF_IDLIST};
    unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None) };
}
