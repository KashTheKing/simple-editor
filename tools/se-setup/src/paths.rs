//! Every folder the installer touches, in one struct, so tests point all of them at a scratch folder.

use crate::json::{self, Json};
use std::path::PathBuf;

pub const APP_EXE: &str = "simple-editor.exe";
pub const UPDATE_EXE: &str = "update.exe";
pub const UNINSTALL_EXE: &str = "uninstall.exe";
/// What this installer put where (read by the updater and the uninstaller).
pub const RECORD: &str = "install.json";
pub const SHORTCUT: &str = "Simple Editor.lnk";

#[derive(Clone, Debug)]
pub struct Paths {
    /// `%LOCALAPPDATA%\Programs\Simple Editor`
    pub install: PathBuf,
    /// `%APPDATA%\SimpleEditor` (settings, gallery, library, skills, add-ons under `bin`)
    pub app_data: PathBuf,
    /// `%LOCALAPPDATA%\SimpleEditor` (cache, autosave)
    pub local_data: PathBuf,
    /// The Start menu's Programs folder.
    pub start_menu: PathBuf,
    pub desktop: PathBuf,
    pub temp: PathBuf,
    /// PATH, split.
    pub path_dirs: Vec<PathBuf>,
}

impl Paths {
    /// Everything under one root (tests).
    #[cfg(test)]
    pub fn under(root: &std::path::Path) -> Paths {
        Paths {
            install: root.join("LocalAppData").join("Programs").join("Simple Editor"),
            app_data: root.join("AppData").join("SimpleEditor"),
            local_data: root.join("LocalAppData").join("SimpleEditor"),
            start_menu: root.join("AppData").join("Microsoft").join("Windows").join("Start Menu").join("Programs"),
            desktop: root.join("Desktop"),
            temp: root.join("Temp"),
            path_dirs: Vec::new(),
        }
    }

    /// The real folders, from the environment. `desktop` is the caller's (a known-folder lookup on Windows).
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn from_env(desktop: Option<PathBuf>) -> Paths {
        let var = |k: &str| std::env::var_os(k).map(PathBuf::from);
        let home = var("USERPROFILE").unwrap_or_else(|| PathBuf::from("."));
        let local = var("LOCALAPPDATA").unwrap_or_else(|| home.join("AppData").join("Local"));
        let roaming = var("APPDATA").unwrap_or_else(|| home.join("AppData").join("Roaming"));
        Paths {
            install: local.join("Programs").join("Simple Editor"),
            app_data: roaming.join("SimpleEditor"),
            local_data: local.join("SimpleEditor"),
            start_menu: roaming.join("Microsoft").join("Windows").join("Start Menu").join("Programs"),
            desktop: desktop.unwrap_or_else(|| home.join("Desktop")),
            temp: std::env::temp_dir(),
            path_dirs: var("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default(),
        }
    }

    pub fn app_exe(&self) -> PathBuf {
        self.install.join(APP_EXE)
    }
    pub fn record(&self) -> PathBuf {
        self.install.join(RECORD)
    }
    pub fn start_shortcut(&self) -> PathBuf {
        self.start_menu.join(SHORTCUT)
    }
    pub fn desktop_shortcut(&self) -> PathBuf {
        self.desktop.join(SHORTCUT)
    }
    /// Add-ons the app installs itself (`blitz::bin_dir`).
    pub fn bin(&self) -> PathBuf {
        self.app_data.join("bin")
    }

    /// The app's settings.json (Null when missing or unreadable).
    pub fn settings(&self) -> Json {
        std::fs::read_to_string(self.app_data.join("settings.json")).ok().and_then(|t| json::parse(&t).ok()).unwrap_or(Json::Null)
    }
    /// A folder setting ("" = unset).
    pub fn setting_dir(&self, key: &str) -> Option<PathBuf> {
        self.settings().str(key).filter(|s| !s.trim().is_empty()).map(PathBuf::from)
    }
    /// `Settings::cache_dir`: `cache_folder` when the user moved it, else `%LOCALAPPDATA%\SimpleEditor\cache`.
    pub fn cache(&self) -> PathBuf {
        self.setting_dir("cache_folder").unwrap_or_else(|| self.default_cache())
    }
    pub fn default_cache(&self) -> PathBuf {
        self.local_data.join("cache")
    }
    /// `name` on PATH.
    pub fn on_path(&self, name: &str) -> Option<PathBuf> {
        self.path_dirs.iter().map(|d| d.join(name)).find(|p| p.is_file())
    }

    /// The app's own lookup order for ffmpeg / yt-dlp (`ffpipe::find_all_exe`): the setting, beside the
    /// exe, its `ffmpeg` folder, then PATH.
    pub fn find_tool(&self, name: &str, setting: &str) -> Option<PathBuf> {
        let mut c: Vec<PathBuf> = Vec::new();
        if let Some(d) = self.setting_dir(setting) {
            c.push(d.join(name));
        }
        c.push(self.install.join(name));
        c.push(self.install.join("ffmpeg").join(name));
        c.into_iter().find(|p| p.is_file()).or_else(|| self.on_path(name))
    }
}
