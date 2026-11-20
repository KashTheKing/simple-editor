//! Every registry key Simple Editor writes under HKCU, in one place. Shared verbatim by the app
//! (`contextmenu.rs` includes it) and the installer / uninstaller (`tools/se-setup`, by `#[path]`), so
//! the uninstaller removes exactly what either of them added. Plain data: no dependencies.
#![allow(dead_code)]

/// Video extensions that get "Edit with Simple Editor" in Explorer's right-click menu. Registered per
/// extension because several (.mov/.m4v/.flv/.ts/.mts/.mpg) have no PerceivedType=video.
pub const VIDEO_EXTS: &[&str] = &[
    "mp4", "mov", "mkv", "webm", "avi", "m4v", "wmv", "ts", "m2ts", "mts", "flv", "3gp", "mpg", "mpeg", "gif", "ogv",
    "vob", "divx", "asf", "f4v", "dv", "mxf",
];

/// The menu entry's text.
pub const MENU_LABEL: &str = "Edit with Simple Editor";

/// Written by older builds (one key for every PerceivedType=video file); removed on install and uninstall.
pub const LEGACY_MENU_KEY: &str = r"Software\Classes\SystemFileAssociations\video\shell\SimpleEditor";

/// The right-click menu key for one extension (its `command` subkey holds `"exe" "%1"`).
pub fn menu_key(ext: &str) -> String {
    format!(r"Software\Classes\SystemFileAssociations\.{ext}\shell\SimpleEditor")
}

/// Apps & Features entry (installer only).
pub const UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\SimpleEditor";
/// Lets Win+R / `start simple-editor` find the exe (installer only).
pub const APP_PATHS_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\App Paths\simple-editor.exe";
/// The `.sedit` project association.
pub const SEDIT_KEY: &str = r"Software\Classes\.sedit";
pub const PROG_ID: &str = "SimpleEditor.Project";
pub const PROG_ID_KEY: &str = r"Software\Classes\SimpleEditor.Project";
/// Explorer's "Open with" record for the exe; not written by us, but removed on uninstall when present.
pub const APPLICATIONS_KEY: &str = r"Software\Classes\Applications\simple-editor.exe";
/// The taskbar identity (`with_app_id` in main.rs); shortcuts carry it so pins group with the window.
pub const APP_USER_MODEL_ID: &str = "SimpleEditor";
