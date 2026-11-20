//! What installing, updating and uninstalling DO, independent of any window: files, registry keys and
//! shortcuts, through the `Reg` / `Net` / `Shell` traits so tests run it against a scratch folder and an
//! in-memory registry.

use crate::addons::{self, AddOn, ADDONS, FFMPEG};
use crate::json::{self, Json};
use crate::net::{self, Net, Release};
use crate::paths::{Paths, APP_EXE, UNINSTALL_EXE, UPDATE_EXE};
use crate::reg::{Reg, Val};
use crate::report::{mb, Report};
use crate::shell_keys as keys;
use crate::version;
use std::path::{Path, PathBuf};

/// The parts of Windows that are not files or registry values.
pub trait Shell {
    /// Write a .lnk to `target` (with the app's taskbar identity).
    fn shortcut(&self, lnk: &Path, target: &Path, desc: &str) -> Result<(), String>;
    /// Tell Explorer that associations changed.
    fn notify_assoc(&self) {}
    /// The exe's file version ("1.4.0"), if it has one.
    fn file_version(&self, _exe: &Path) -> Option<String> {
        None
    }
    /// Is a process running from this exe?
    fn running(&self, _exe: &Path) -> bool {
        false
    }
    /// Remove a Windows Credential Manager entry (the Assistant's API key).
    fn delete_credential(&self, _target: &str) {}
}

// ------------------------------------------------------------------------------------------------
// the record of what we installed

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Record {
    pub version: String,
    /// We installed FFmpeg (so the uninstaller removes it).
    pub ffmpeg: bool,
    /// Add-on ids we installed.
    pub addons: Vec<String>,
    pub desktop: bool,
    pub context_menu: bool,
}

impl Record {
    pub fn load(p: &Paths) -> Option<Record> {
        let j = json::parse(&std::fs::read_to_string(p.record()).ok()?).ok()?;
        Some(Record {
            version: j.str("version").unwrap_or("").to_string(),
            ffmpeg: j.bool("ffmpeg").unwrap_or(false),
            addons: j.arr("addons").iter().filter_map(|a| a.as_str().map(str::to_string)).collect(),
            desktop: j.bool("desktop").unwrap_or(false),
            context_menu: j.bool("context_menu").unwrap_or(false),
        })
    }
    pub fn save(&self, p: &Paths) -> Result<(), String> {
        let j = json::obj([
            ("version", Json::Str(self.version.clone())),
            ("ffmpeg", Json::Bool(self.ffmpeg)),
            ("addons", Json::Arr(self.addons.iter().map(|a| Json::Str(a.clone())).collect())),
            ("desktop", Json::Bool(self.desktop)),
            ("context_menu", Json::Bool(self.context_menu)),
        ]);
        std::fs::write(p.record(), j.dump()).map_err(|e| format!("{}: {e}", p.record().display()))
    }
}

// ------------------------------------------------------------------------------------------------
// files

/// Put `new` in place of `target` atomically enough: the old file is renamed aside first (a running exe
/// can be renamed, not overwritten), and renamed back if the new one cannot take its place.
pub fn swap_in(new: &Path, target: &Path) -> Result<(), String> {
    // an `.old` from last time may still be running (it can't be deleted then): use the next free name
    let old = (0..20)
        .map(|i| PathBuf::from(if i == 0 { format!("{}.old", target.display()) } else { format!("{}.old{i}", target.display()) }))
        .find(|o| !o.exists() || std::fs::remove_file(o).is_ok())
        .ok_or_else(|| format!("{}: too many old copies still running", target.display()))?;
    let had = target.exists();
    if had {
        std::fs::rename(target, &old).map_err(|e| format!("{}: {e}", target.display()))?;
    }
    if let Err(e) = std::fs::rename(new, target) {
        if had {
            let _ = std::fs::rename(&old, target); // rollback
        }
        return Err(format!("{}: {e}", target.display()));
    }
    // still running (the old exe): it stays until the next install or update clears it
    let _ = std::fs::remove_file(&old);
    Ok(())
}

fn is_project(p: &Path) -> bool {
    p.extension().map(|e| e.eq_ignore_ascii_case("sedit")).unwrap_or(false)
}

/// Delete a folder and everything in it, except project files (.sedit), which stay with their folders.
/// Returns whether anything was kept (or could not be removed).
pub fn remove_tree_keep_projects(dir: &Path) -> bool {
    let Ok(rd) = std::fs::read_dir(dir) else { return false };
    let mut kept = false;
    for e in rd.flatten() {
        let p = e.path();
        let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
        if is_dir {
            kept |= remove_tree_keep_projects(&p);
        } else if is_project(&p) {
            kept = true;
        } else if std::fs::remove_file(&p).is_err() {
            kept = true;
        }
    }
    if !kept {
        kept = std::fs::remove_dir(dir).is_err();
    }
    kept
}

fn dir_size_kb(dir: &Path) -> u64 {
    fn walk(d: &Path) -> u64 {
        std::fs::read_dir(d)
            .map(|rd| rd.flatten().map(|e| if e.path().is_dir() { walk(&e.path()) } else { e.metadata().map(|m| m.len()).unwrap_or(0) }).sum())
            .unwrap_or(0)
    }
    walk(dir) / 1024
}

// ------------------------------------------------------------------------------------------------
// registry

/// Every key the installer writes. `context_menu` adds the same keys as the app's `contextmenu::install`.
pub fn register(reg: &mut dyn Reg, p: &Paths, ver: &str, context_menu: bool) -> Result<(), String> {
    let exe = p.app_exe().display().to_string();
    let dir = p.install.display().to_string();
    let quoted = format!("\"{exe}\"");
    let u = keys::UNINSTALL_KEY;
    let uninstall = p.install.join(UNINSTALL_EXE).display().to_string();
    reg.set_sz(u, "DisplayName", "Simple Editor")?;
    reg.set_sz(u, "DisplayVersion", ver)?;
    reg.set_sz(u, "Publisher", "Simple Editor")?;
    reg.set_sz(u, "DisplayIcon", &format!("{exe},0"))?;
    reg.set_sz(u, "InstallLocation", &dir)?;
    reg.set_sz(u, "UninstallString", &format!("\"{uninstall}\""))?;
    reg.set_sz(u, "QuietUninstallString", &format!("\"{uninstall}\" --silent"))?;
    reg.set_sz(u, "URLInfoAbout", "https://github.com/KashTheKing/simple-editor")?;
    reg.set_sz(u, "URLUpdateInfo", "https://github.com/KashTheKing/simple-editor/releases")?;
    reg.set(u, "NoModify", Val::Dword(1))?;
    reg.set(u, "NoRepair", Val::Dword(1))?;
    reg.set(u, "EstimatedSize", Val::Dword(dir_size_kb(&p.install).min(u32::MAX as u64) as u32))?;

    reg.set_sz(keys::APP_PATHS_KEY, "", &exe)?;
    reg.set_sz(keys::APP_PATHS_KEY, "Path", &dir)?;

    reg.set_sz(keys::SEDIT_KEY, "", keys::PROG_ID)?;
    reg.set_sz(&format!(r"{}\OpenWithProgids", keys::SEDIT_KEY), keys::PROG_ID, "")?;
    reg.set_sz(keys::PROG_ID_KEY, "", "Simple Editor Project")?;
    reg.set_sz(&format!(r"{}\DefaultIcon", keys::PROG_ID_KEY), "", &format!("{quoted},0"))?;
    reg.set_sz(&format!(r"{}\shell\open\command", keys::PROG_ID_KEY), "", &format!("{quoted} \"%1\""))?;

    reg.delete_tree(keys::LEGACY_MENU_KEY)?;
    if context_menu {
        for ext in keys::VIDEO_EXTS {
            let k = keys::menu_key(ext);
            reg.set_sz(&k, "", keys::MENU_LABEL)?;
            reg.set_sz(&k, "Icon", &format!("{quoted},0"))?;
            reg.set_sz(&format!(r"{k}\command"), "", &format!("{quoted} \"%1\""))?;
        }
    }
    Ok(())
}

/// Remove every key the installer or the app ever wrote (legacy keys included). Keys of other programs
/// stay: a `.sedit` association someone else took over is left to them.
pub fn unregister(reg: &mut dyn Reg) -> Result<(), String> {
    for k in [keys::UNINSTALL_KEY, keys::APP_PATHS_KEY, keys::PROG_ID_KEY, keys::APPLICATIONS_KEY, keys::LEGACY_MENU_KEY] {
        reg.delete_tree(k)?;
    }
    for ext in keys::VIDEO_EXTS {
        reg.delete_tree(&keys::menu_key(ext))?;
    }
    let owp = format!(r"{}\OpenWithProgids", keys::SEDIT_KEY);
    reg.delete_value(&owp, keys::PROG_ID)?;
    if reg.exists(&owp) && reg.is_empty(&owp) {
        reg.delete_tree(&owp)?;
    }
    if reg.get_sz(keys::SEDIT_KEY, "").as_deref() == Some(keys::PROG_ID) {
        reg.delete_value(keys::SEDIT_KEY, "")?;
    }
    if reg.exists(keys::SEDIT_KEY) && reg.is_empty(keys::SEDIT_KEY) {
        reg.delete_tree(keys::SEDIT_KEY)?;
    }
    Ok(())
}

// ------------------------------------------------------------------------------------------------
// install

#[derive(Clone, Debug)]
pub struct Options {
    pub ffmpeg: bool,
    pub addons: Vec<String>,
    pub desktop: bool,
    pub context_menu: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options { ffmpeg: true, addons: Vec::new(), desktop: false, context_menu: true }
    }
}

/// Where the app exe comes from.
pub enum AppSource {
    /// A `simple-editor.exe` beside the setup exe (an offline install).
    Local(PathBuf),
    /// The latest GitHub release (verified against its digest).
    Latest,
}

/// The `simple-editor.exe` next to the setup program, if there is one (an offline install).
#[cfg_attr(not(windows), allow(dead_code))]
pub fn local_app(setup_exe: &Path) -> Option<PathBuf> {
    let p = setup_exe.parent()?.join(APP_EXE);
    (p.is_file() && p != setup_exe).then_some(p)
}

/// Install (or reinstall over) Simple Editor. Returns the installed version.
pub fn install(
    p: &Paths,
    reg: &mut dyn Reg,
    net: &dyn Net,
    sh: &dyn Shell,
    src: &AppSource,
    setup_exe: &Path,
    o: &Options,
    rep: &Report,
) -> Result<String, String> {
    std::fs::create_dir_all(&p.install).map_err(|e| format!("{}: {e}", p.install.display()))?;
    let mut rec = Record::load(p).unwrap_or_default();
    let target = p.app_exe();
    let new = PathBuf::from(format!("{}.new", target.display()));
    // the app (60% of the bar when it is downloaded)
    let ver = match src {
        AppSource::Local(exe) => {
            rep.say(0.02, "Copying Simple Editor…");
            std::fs::copy(exe, &new).map_err(|e| format!("{}: {e}", exe.display()))?;
            sh.file_version(exe).unwrap_or_default()
        }
        AppSource::Latest => {
            rep.say(0.02, "Looking up the latest Simple Editor…");
            let r = net::latest_release(net, net::REPO)?;
            let a = r.asset(APP_EXE).ok_or("the latest release has no simple-editor.exe")?;
            let sha = a.sha256.as_deref().ok_or("simple-editor.exe has no checksum on GitHub")?;
            net::download(net, &a.url, &new, Some(sha), Some(a.size).filter(|&s| s > 0), &mut |got, total| {
                let f = total.map(|t| got as f32 / t.max(1) as f32).unwrap_or(0.0);
                rep.say(0.02 + 0.3 * f, &format!("Downloading Simple Editor… {} of {}", mb(got), total.map(mb).unwrap_or_else(|| "?".into())));
                !rep.cancelled()
            })?;
            version::plain(&r.tag)
        }
    };
    rep.check().inspect_err(|_| {
        let _ = std::fs::remove_file(&new);
    })?;
    swap_in(&new, &target)?;
    rec.version = ver.clone();

    // the updater and the uninstaller: this same program under two names
    for name in [UPDATE_EXE, UNINSTALL_EXE] {
        let to = p.install.join(name);
        if to != setup_exe {
            let tmp = PathBuf::from(format!("{}.new", to.display()));
            std::fs::copy(setup_exe, &tmp).map_err(|e| format!("{}: {e}", to.display()))?;
            swap_in(&tmp, &to)?;
        }
    }
    rec.save(p)?; // from here on the uninstaller can clean up whatever follows

    // FFmpeg and the add-ons share 0.35..0.9
    let mut jobs: Vec<&AddOn> = Vec::new();
    if o.ffmpeg && !addons::installed(&FFMPEG, p) {
        jobs.push(&FFMPEG);
    }
    for a in ADDONS {
        if o.addons.iter().any(|id| id == a.id) && !addons::installed(a, p) {
            jobs.push(a);
        }
    }
    let mut failed: Vec<String> = Vec::new();
    let step = 0.55 / jobs.len().max(1) as f32;
    for (i, a) in jobs.iter().enumerate() {
        rep.check()?;
        let lo = 0.35 + step * i as f32;
        match addons::install(a, p, net, rep, lo, lo + step) {
            Ok(_) => {
                if a.id == "ffmpeg" {
                    rec.ffmpeg = true;
                } else if !rec.addons.iter().any(|x| x == a.id) {
                    rec.addons.push(a.id.to_string());
                }
                rec.save(p)?;
            }
            Err(e) if e == crate::report::CANCELLED => return Err(e),
            // an add-on that fails does not stop the app's install
            Err(e) => failed.push(format!("{}: {e}", a.label.split(" - ").next().unwrap_or(a.id).trim_start_matches("Install "))),
        }
    }

    rep.say(0.92, "Adding shortcuts…");
    let exe = p.app_exe();
    sh.shortcut(&p.start_shortcut(), &exe, "Edit videos")?;
    if o.desktop {
        sh.shortcut(&p.desktop_shortcut(), &exe, "Edit videos")?;
    } else if rec.desktop {
        let _ = std::fs::remove_file(p.desktop_shortcut());
    }
    rec.desktop = o.desktop;
    if !o.context_menu && rec.context_menu {
        for ext in keys::VIDEO_EXTS {
            reg.delete_tree(&keys::menu_key(ext))?;
        }
    }
    rec.context_menu = o.context_menu;
    register(reg, p, &ver, o.context_menu)?;
    sh.notify_assoc();
    rec.save(p)?;
    if !failed.is_empty() {
        return Err(format!("Simple Editor is installed, but these were not:\n{}", failed.join("\n")));
    }
    rep.say(1.0, "Simple Editor is installed.");
    Ok(ver)
}

// ------------------------------------------------------------------------------------------------
// update

/// The installed version: the record, else the exe's own version resource.
pub fn installed_version(p: &Paths, sh: &dyn Shell) -> String {
    Record::load(p).map(|r| r.version).filter(|v| !v.is_empty()).or_else(|| sh.file_version(&p.app_exe())).unwrap_or_default()
}

/// The latest release when it is newer than `current`.
pub fn check_update(net: &dyn Net, current: &str) -> Result<Option<Release>, String> {
    let r = net::latest_release(net, net::REPO)?;
    Ok(version::is_newer(&r.tag, current).then_some(r))
}

/// Download `rel`'s exe, wait for the app to close (`wait` returns Err when the user gives up), and
/// swap it in; the old exe comes back if anything fails. Settings (in %APPDATA%) are never touched.
pub fn apply_update(p: &Paths, reg: &mut dyn Reg, net: &dyn Net, sh: &dyn Shell, rel: &Release, wait: &dyn Fn() -> Result<(), String>, rep: &Report) -> Result<String, String> {
    let a = rel.asset(APP_EXE).ok_or("the release has no simple-editor.exe")?;
    let sha = a.sha256.as_deref().ok_or("simple-editor.exe has no checksum on GitHub")?;
    let target = p.app_exe();
    let new = PathBuf::from(format!("{}.new", target.display()));
    let ver = version::plain(&rel.tag);
    net::download(net, &a.url, &new, Some(sha), Some(a.size).filter(|&s| s > 0), &mut |got, total| {
        let f = total.map(|t| got as f32 / t.max(1) as f32).unwrap_or(0.0);
        rep.say(0.85 * f, &format!("Downloading Simple Editor {ver}… {} of {}", mb(got), total.map(mb).unwrap_or_else(|| "?".into())));
        !rep.cancelled()
    })?;
    let cleanup = |e: String| {
        let _ = std::fs::remove_file(&new);
        Err(e)
    };
    // the file must say it is the version the release says
    if let Some(v) = sh.file_version(&new) {
        if version::parse(&v).map(|mut x| {
            x.truncate(3);
            x
        }) != version::parse(&ver)
        {
            return cleanup(format!("the download says it is {v}, not {ver}"));
        }
    }
    if sh.running(&target) {
        rep.say(0.88, "Waiting for Simple Editor to close…");
        if let Err(e) = wait() {
            return cleanup(e);
        }
    }
    rep.say(0.95, "Replacing Simple Editor…");
    if let Err(e) = swap_in(&new, &target) {
        return cleanup(e);
    }
    let mut rec = Record::load(p).unwrap_or_default();
    rec.version = ver.clone();
    rec.save(p)?;
    if reg.exists(keys::UNINSTALL_KEY) {
        reg.set_sz(keys::UNINSTALL_KEY, "DisplayVersion", &ver)?;
    }
    rep.say(1.0, &format!("Simple Editor is updated to {ver}."));
    Ok(ver)
}

// ------------------------------------------------------------------------------------------------
// uninstall

/// The Assistant's key in Windows Credential Manager (`engine::agent::CRED_TARGET`).
pub const CRED_TARGET: &str = "SimpleEditor/assistant";

/// Remove Simple Editor: the program, FFmpeg it installed, every add-on, every registry key and shortcut.
/// `keep_prefs` keeps `%APPDATA%\SimpleEditor` (settings, gallery, library, skills) apart from its add-ons.
/// Projects (.sedit files) and autosaved project backups are never deleted. Returns the folders that
/// could not be removed completely (a file in use, or projects someone kept there).
pub fn uninstall(p: &Paths, reg: &mut dyn Reg, sh: &dyn Shell, keep_prefs: bool, rep: &Report) -> Result<Vec<PathBuf>, String> {
    rep.say(0.1, "Removing shortcuts…");
    for s in [p.start_shortcut(), p.desktop_shortcut()] {
        let _ = std::fs::remove_file(s);
    }
    rep.say(0.2, "Removing registry entries…");
    unregister(reg)?;
    sh.notify_assoc();

    rep.say(0.35, "Removing add-ons…");
    // add-ons, whoever installed them (the app's own installs live here too)
    let _ = std::fs::remove_dir_all(p.bin());
    let cache = p.cache();
    for sub in ["whisper", "models"] {
        let _ = std::fs::remove_dir_all(cache.join(sub));
    }
    // the cache is not a preference; a cache the user moved elsewhere keeps its folder (it may hold other things)
    if cache == p.default_cache() {
        remove_tree_keep_projects(&cache);
    }

    rep.say(0.55, "Removing Simple Editor…");
    let mut left: Vec<PathBuf> = Vec::new();
    if p.install.exists() && remove_tree_keep_projects(&p.install) {
        left.push(p.install.clone());
    }

    if !keep_prefs {
        rep.say(0.8, "Removing preferences…");
        sh.delete_credential(CRED_TARGET);
        if p.app_data.exists() && remove_tree_keep_projects(&p.app_data) {
            left.push(p.app_data.clone());
        }
        // %LOCALAPPDATA%\SimpleEditor, apart from the autosaved project backups
        if let Ok(rd) = std::fs::read_dir(&p.local_data) {
            for e in rd.flatten() {
                if e.file_name().to_string_lossy().eq_ignore_ascii_case("autosave") {
                    continue;
                }
                let path = e.path();
                if path.is_dir() {
                    remove_tree_keep_projects(&path);
                } else if !is_project(&path) {
                    let _ = std::fs::remove_file(&path);
                }
            }
            let _ = std::fs::remove_dir(&p.local_data); // only when empty
        }
    }
    rep.say(1.0, "Simple Editor was removed.");
    Ok(left)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::FakeNet;
    use crate::reg::MemReg;
    use crate::report::quiet;
    use crate::sha256::Sha256;
    use crate::testutil::TempDir;
    use std::cell::RefCell;

    #[derive(Default)]
    struct FakeShell {
        running: RefCell<bool>,
        versions: Vec<(String, String)>,
        creds_deleted: RefCell<Vec<String>>,
    }
    impl Shell for FakeShell {
        fn shortcut(&self, lnk: &Path, target: &Path, _desc: &str) -> Result<(), String> {
            std::fs::create_dir_all(lnk.parent().unwrap()).unwrap();
            std::fs::write(lnk, target.display().to_string()).map_err(|e| e.to_string())
        }
        fn file_version(&self, exe: &Path) -> Option<String> {
            let body = std::fs::read_to_string(exe).ok()?;
            self.versions.iter().find(|(b, _)| *b == body).map(|(_, v)| v.clone())
        }
        fn running(&self, _exe: &Path) -> bool {
            *self.running.borrow()
        }
        fn delete_credential(&self, t: &str) {
            self.creds_deleted.borrow_mut().push(t.into());
        }
    }

    fn sha(b: &[u8]) -> String {
        let mut h = Sha256::new();
        h.update(b);
        h.hex()
    }

    fn release_json(tag: &str, body: &[u8]) -> String {
        format!(
            r#"{{"tag_name":"{tag}","assets":[{{"name":"simple-editor.exe","size":{},"digest":"sha256:{}","browser_download_url":"https://dl/{tag}/simple-editor.exe"}}]}}"#,
            body.len(),
            sha(body)
        )
    }

    struct World {
        _t: TempDir,
        p: Paths,
        setup: PathBuf,
        local: PathBuf,
    }

    fn world(name: &str) -> World {
        let t = TempDir::new(name);
        let p = Paths::under(t.path());
        let dl = t.path().join("Downloads");
        std::fs::create_dir_all(&dl).unwrap();
        let setup = dl.join("simple-editor-setup.exe");
        std::fs::write(&setup, b"setup program").unwrap();
        let local = dl.join("simple-editor.exe");
        std::fs::write(&local, b"app 1.4.0").unwrap();
        World { _t: t, p, setup, local }
    }

    fn opts() -> Options {
        Options { ffmpeg: false, addons: vec![], desktop: true, context_menu: true }
    }

    #[test]
    fn install_puts_files_keys_and_shortcuts_in_place() {
        let w = world("inst");
        let mut reg = MemReg::default();
        let sh = FakeShell { versions: vec![("app 1.4.0".into(), "1.4.0.0".into())], ..Default::default() };
        let net = FakeNet::new(&[]);
        let v = quiet(|r| install(&w.p, &mut reg, &net, &sh, &AppSource::Local(w.local.clone()), &w.setup, &opts(), r)).unwrap();
        assert_eq!(v, "1.4.0.0");
        assert_eq!(std::fs::read(w.p.app_exe()).unwrap(), b"app 1.4.0");
        assert_eq!(std::fs::read(w.p.install.join(UPDATE_EXE)).unwrap(), b"setup program");
        assert_eq!(std::fs::read(w.p.install.join(UNINSTALL_EXE)).unwrap(), b"setup program");
        assert!(w.p.start_shortcut().is_file() && w.p.desktop_shortcut().is_file());
        // per-user, under %LOCALAPPDATA%\Programs
        assert!(w.p.install.ends_with(Path::new("LocalAppData").join("Programs").join("Simple Editor")));
        // the uninstall entry
        assert_eq!(reg.get_sz(keys::UNINSTALL_KEY, "DisplayName").as_deref(), Some("Simple Editor"));
        assert!(reg.get_sz(keys::UNINSTALL_KEY, "UninstallString").unwrap().contains("uninstall.exe"));
        // .sedit
        assert_eq!(reg.get_sz(keys::SEDIT_KEY, "").as_deref(), Some(keys::PROG_ID));
        let open = reg.get_sz(&format!(r"{}\shell\open\command", keys::PROG_ID_KEY), "").unwrap();
        assert_eq!(open, format!("\"{}\" \"%1\"", w.p.app_exe().display()));
        // the same context-menu keys the app's contextmenu.rs writes, for every extension
        for ext in keys::VIDEO_EXTS {
            assert_eq!(reg.get_sz(&keys::menu_key(ext), "").as_deref(), Some("Edit with Simple Editor"));
            assert_eq!(reg.get_sz(&format!(r"{}\command", keys::menu_key(ext)), "").unwrap(), open);
        }
        assert!(!w.p.install.join("simple-editor.exe.new").exists());
        let rec = Record::load(&w.p).unwrap();
        assert!(rec.desktop && rec.context_menu && !rec.ffmpeg);
    }

    #[test]
    fn install_downloads_the_latest_release_when_there_is_no_local_exe() {
        let w = world("instnet");
        let body = b"app from github";
        let rel = release_json("v1.4.2", body);
        let net = FakeNet::new(&[("https://api.github.com/repos/KashTheKing/simple-editor/releases/latest", rel.as_bytes()), ("https://dl/v1.4.2/simple-editor.exe", body)]);
        let mut reg = MemReg::default();
        let v = quiet(|r| install(&w.p, &mut reg, &net, &FakeShell::default(), &AppSource::Latest, &w.setup, &opts(), r)).unwrap();
        assert_eq!(v, "1.4.2");
        assert_eq!(std::fs::read(w.p.app_exe()).unwrap(), body);
        assert_eq!(reg.get_sz(keys::UNINSTALL_KEY, "DisplayVersion").as_deref(), Some("1.4.2"));
    }

    #[test]
    fn install_offers_ffmpeg_with_a_checksum_and_skips_it_when_present() {
        let w = world("instff");
        // FFmpeg already on PATH: nothing is downloaded (the FakeNet has no pages, so a download would fail)
        let mut p = w.p.clone();
        let tools = w._t.path().join("tools");
        std::fs::create_dir_all(&tools).unwrap();
        std::fs::write(tools.join("ffmpeg.exe"), b"x").unwrap();
        std::fs::write(tools.join("ffprobe.exe"), b"x").unwrap();
        p.path_dirs.push(tools);
        let mut reg = MemReg::default();
        let o = Options { ffmpeg: true, ..opts() };
        quiet(|r| install(&p, &mut reg, &FakeNet::new(&[]), &FakeShell::default(), &AppSource::Local(w.local.clone()), &w.setup, &o, r)).unwrap();
        assert!(!Record::load(&p).unwrap().ffmpeg, "not ours to remove later");
        // missing: the pinned zip is fetched, and a wrong checksum fails the FFmpeg step only
        let net = FakeNet::new(&[]);
        let r = quiet(|r| install(&w.p, &mut MemReg::default(), &net, &FakeShell::default(), &AppSource::Local(w.local.clone()), &w.setup, &o, r));
        let e = r.unwrap_err();
        assert!(e.contains("installed, but") && e.contains("FFmpeg"), "{e}");
        assert!(net.hit(&json::parse(addons::FFMPEG_JSON).unwrap().str("url").unwrap().to_string()));
        assert!(w.p.app_exe().is_file(), "the app is installed anyway");
    }

    #[test]
    fn update_swaps_the_exe_and_keeps_settings() {
        let w = world("upd");
        let mut reg = MemReg::default();
        let sh = FakeShell { versions: vec![("app 1.4.0".into(), "1.4.0".into()), ("app 1.5.0".into(), "1.5.0.0".into())], ..Default::default() };
        quiet(|r| install(&w.p, &mut reg, &FakeNet::new(&[]), &sh, &AppSource::Local(w.local.clone()), &w.setup, &opts(), r)).unwrap();
        std::fs::create_dir_all(&w.p.app_data).unwrap();
        std::fs::write(w.p.app_data.join("settings.json"), b"{\"theme\":\"mine\"}").unwrap();
        assert_eq!(installed_version(&w.p, &sh), "1.4.0");

        let rel_text = release_json("v1.5.0", b"app 1.5.0");
        let net = FakeNet::new(&[("https://api.github.com/repos/KashTheKing/simple-editor/releases/latest", rel_text.as_bytes()), ("https://dl/v1.5.0/simple-editor.exe", b"app 1.5.0")]);
        let rel = check_update(&net, "1.4.0").unwrap().expect("newer");
        assert!(check_update(&net, "1.5.0").unwrap().is_none());
        *sh.running.borrow_mut() = true;
        let waited = std::cell::Cell::new(false);
        let v = quiet(|r| {
            apply_update(&w.p, &mut reg, &net, &sh, &rel, &|| {
                waited.set(true);
                Ok(())
            }, r)
        })
        .unwrap();
        assert!(waited.get(), "waits for the running app");
        assert_eq!(v, "1.5.0");
        assert_eq!(std::fs::read(w.p.app_exe()).unwrap(), b"app 1.5.0");
        assert_eq!(std::fs::read(w.p.app_data.join("settings.json")).unwrap(), b"{\"theme\":\"mine\"}");
        assert_eq!(reg.get_sz(keys::UNINSTALL_KEY, "DisplayVersion").as_deref(), Some("1.5.0"));
        assert_eq!(installed_version(&w.p, &sh), "1.5.0");
    }

    #[test]
    fn a_failed_update_leaves_the_old_exe_working() {
        let w = world("updfail");
        let mut reg = MemReg::default();
        let sh = FakeShell { versions: vec![("app 1.5.0".into(), "1.3.0".into())], ..Default::default() };
        quiet(|r| install(&w.p, &mut reg, &FakeNet::new(&[]), &sh, &AppSource::Local(w.local.clone()), &w.setup, &opts(), r)).unwrap();
        let rel_text = release_json("v1.5.0", b"app 1.5.0");
        let net = FakeNet::new(&[("https://api.github.com/repos/KashTheKing/simple-editor/releases/latest", rel_text.as_bytes()), ("https://dl/v1.5.0/simple-editor.exe", b"app 1.5.0")]);
        let rel = check_update(&net, "1.4.0").unwrap().unwrap();
        // the download says it is another version: refused
        assert!(quiet(|r| apply_update(&w.p, &mut reg, &net, &sh, &rel, &|| Ok(()), r)).is_err());
        assert_eq!(std::fs::read(w.p.app_exe()).unwrap(), b"app 1.4.0");
        // the user gives up waiting: nothing changes
        let sh = FakeShell { running: RefCell::new(true), ..Default::default() };
        assert!(quiet(|r| apply_update(&w.p, &mut reg, &net, &sh, &rel, &|| Err("Cancelled".into()), r)).is_err());
        assert_eq!(std::fs::read(w.p.app_exe()).unwrap(), b"app 1.4.0");
        assert!(!w.p.install.join("simple-editor.exe.new").exists());
        // a tampered download: refused
        let net = FakeNet::new(&[("https://api.github.com/repos/KashTheKing/simple-editor/releases/latest", rel_text.as_bytes()), ("https://dl/v1.5.0/simple-editor.exe", b"app 1.5.X")]);
        assert!(quiet(|r| apply_update(&w.p, &mut reg, &net, &FakeShell::default(), &rel, &|| Ok(()), r)).is_err());
        assert_eq!(std::fs::read(w.p.app_exe()).unwrap(), b"app 1.4.0");
    }

    #[test]
    fn swap_finds_a_free_name_when_an_old_copy_is_in_use() {
        let t = TempDir::new("swapold");
        let target = t.path().join("a.exe");
        std::fs::write(&target, b"v1").unwrap();
        // an `.old` that can't be removed (a directory stands in for a running exe)
        std::fs::create_dir_all(t.path().join("a.exe.old").join("x")).unwrap();
        std::fs::write(t.path().join("a.new"), b"v2").unwrap();
        swap_in(&t.path().join("a.new"), &target).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"v2");
    }

    #[test]
    fn swap_rolls_back_when_the_new_file_cannot_move_in() {
        let t = TempDir::new("swap");
        let target = t.path().join("a.exe");
        std::fs::write(&target, b"old").unwrap();
        assert!(swap_in(&t.path().join("missing.new"), &target).is_err());
        assert_eq!(std::fs::read(&target).unwrap(), b"old");
        assert!(!t.path().join("a.exe.old").exists());
        std::fs::write(t.path().join("a.new"), b"new").unwrap();
        swap_in(&t.path().join("a.new"), &target).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"new");
    }

    fn installed_world(name: &str) -> (World, MemReg) {
        let w = world(name);
        let mut reg = MemReg::default();
        quiet(|r| install(&w.p, &mut reg, &FakeNet::new(&[]), &FakeShell::default(), &AppSource::Local(w.local.clone()), &w.setup, &opts(), r)).unwrap();
        // what the app adds by itself: preferences, add-ons, cache, autosave, a legacy menu key
        let p = &w.p;
        for (f, body) in [
            (p.app_data.join("settings.json"), "{}"),
            (p.app_data.join("gallery").join("look").join("warm.json"), "{}"),
            (p.bin().join("se-html.exe"), "x"),
            (p.bin().join("enhancer").join("realesrgan-ncnn-vulkan.exe"), "x"),
            (p.default_cache().join("whisper").join("whisper-cli.exe"), "x"),
            (p.default_cache().join("models").join("ggml-base.en.bin"), "x"),
            (p.default_cache().join("frames").join("1.bin"), "x"),
            (p.local_data.join("autosave").join("My Film.sedit"), "{}"),
            (p.install.join("ffmpeg").join("ffmpeg.exe"), "x"),
            (p.install.join("yt-dlp.exe"), "x"),
        ] {
            std::fs::create_dir_all(f.parent().unwrap()).unwrap();
            std::fs::write(f, body).unwrap();
        }
        reg.set_sz(keys::LEGACY_MENU_KEY, "", "Edit with Simple Editor").unwrap();
        reg.set_sz(&format!(r"{}\command", keys::LEGACY_MENU_KEY), "", "x").unwrap();
        reg.set_sz(keys::APPLICATIONS_KEY, "FriendlyAppName", "Simple Editor").unwrap();
        // not ours: must survive
        reg.set_sz(r"Software\Classes\.mp4", "", "VLC.mp4").unwrap();
        reg.set_sz(r"Software\Classes\SystemFileAssociations\.mp4\shell\VLC", "", "Play with VLC").unwrap();
        (w, reg)
    }

    #[test]
    fn uninstall_keeping_preferences_removes_the_program_add_ons_keys_and_shortcuts() {
        let (w, mut reg) = installed_world("unkeep");
        let p = &w.p;
        let sh = FakeShell::default();
        quiet(|r| uninstall(p, &mut reg, &sh, true, r)).unwrap();
        assert!(!p.install.exists(), "program, FFmpeg and yt-dlp gone");
        assert!(!p.start_shortcut().exists() && !p.desktop_shortcut().exists());
        assert!(!p.bin().exists(), "add-ons gone");
        assert!(!p.default_cache().exists(), "cache gone");
        assert!(p.app_data.join("settings.json").is_file() && p.app_data.join("gallery").join("look").join("warm.json").is_file());
        assert!(p.local_data.join("autosave").join("My Film.sedit").is_file(), "project backups untouched");
        assert!(sh.creds_deleted.borrow().is_empty());
        let ours: Vec<String> = reg
            .keys
            .keys()
            .filter(|k| k.contains("simpleeditor") || k.contains("simple-editor") || k.contains(".sedit"))
            .cloned()
            .collect();
        assert!(ours.is_empty(), "left behind: {ours:?}");
        assert_eq!(reg.get_sz(r"Software\Classes\.mp4", "").as_deref(), Some("VLC.mp4"));
        assert!(reg.exists(r"Software\Classes\SystemFileAssociations\.mp4\shell\VLC"));
    }

    #[test]
    fn uninstall_removing_preferences_never_touches_projects() {
        let (w, mut reg) = installed_world("unrm");
        let p = &w.p;
        // a project someone saved inside the program folder, and one in Documents
        std::fs::write(p.install.join("Holiday.sedit"), b"{}").unwrap();
        let docs = w._t.path().join("Documents");
        std::fs::create_dir_all(&docs).unwrap();
        std::fs::write(docs.join("Wedding.sedit"), b"{}").unwrap();
        let sh = FakeShell::default();
        quiet(|r| uninstall(p, &mut reg, &sh, false, r)).unwrap();
        assert!(!p.app_data.exists(), "preferences gone");
        assert!(!p.default_cache().exists());
        assert_eq!(sh.creds_deleted.borrow().as_slice(), [CRED_TARGET.to_string()]);
        assert!(p.install.join("Holiday.sedit").is_file());
        assert!(!p.app_exe().exists() && !p.install.join("ffmpeg").exists());
        assert!(docs.join("Wedding.sedit").is_file());
        assert!(p.local_data.join("autosave").join("My Film.sedit").is_file());
    }

    #[test]
    fn a_foreign_sedit_association_is_left_alone() {
        let (w, mut reg) = installed_world("unforeign");
        reg.set_sz(keys::SEDIT_KEY, "", "OtherApp.Project").unwrap();
        quiet(|r| uninstall(&w.p, &mut reg, &FakeShell::default(), true, r)).unwrap();
        assert_eq!(reg.get_sz(keys::SEDIT_KEY, "").as_deref(), Some("OtherApp.Project"));
        assert!(reg.get_sz(&format!(r"{}\OpenWithProgids", keys::SEDIT_KEY), keys::PROG_ID).is_none());
    }

    #[test]
    fn reinstall_without_the_menu_removes_it() {
        let (w, mut reg) = installed_world("remenu");
        let o = Options { context_menu: false, desktop: false, ..opts() };
        quiet(|r| install(&w.p, &mut reg, &FakeNet::new(&[]), &FakeShell::default(), &AppSource::Local(w.local.clone()), &w.setup, &o, r)).unwrap();
        assert!(!reg.exists(&keys::menu_key("mp4")));
        assert!(!w.p.desktop_shortcut().exists());
        assert!(reg.exists(keys::UNINSTALL_KEY));
    }
}
