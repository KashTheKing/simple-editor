//! The Windows side: the three windows (install, update, uninstall) on top of `setup.rs`.

mod http;
mod reg;
mod shell;
mod ui;

use crate::addons::{self, ADDONS, FFMPEG};
use crate::paths::{Paths, APP_EXE};
use crate::report::{self, mb, Report};
use crate::setup::{self, AppSource, Options, Shell};
use crate::Role;
use http::WinNet;
use reg::WinReg;
use shell::WinShell;
use std::path::{Path, PathBuf};
use ui::{Btn, Check, Flow, Next, Page};

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn paths() -> Paths {
    Paths::from_env(shell::desktop())
}

fn has(args: &[String], flag: &str) -> bool {
    args.iter().any(|a| a.eq_ignore_ascii_case(flag))
}

fn value<'a>(args: &'a [String], key: &str) -> Option<&'a str> {
    args.iter().find_map(|a| a.strip_prefix(key).and_then(|v| v.strip_prefix('=')))
}

pub fn run() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let me = std::env::current_exe().unwrap_or_default();
    let code = match crate::role(&args, &me) {
        Role::Install => install_main(&args, &me),
        Role::Update => update_main(&args),
        Role::Uninstall => uninstall_main(&args, &me),
    };
    std::process::exit(code);
}

fn open_app(p: &Paths) {
    let _ = std::process::Command::new(p.app_exe()).current_dir(&p.install).spawn();
}

/// Wait (polling) until no process runs `exe`, or Cancel.
fn wait_closed(exe: &Path, rep: &Report) -> Result<(), String> {
    while WinShell.running(exe) {
        rep.check()?;
        std::thread::sleep(std::time::Duration::from_millis(400));
    }
    Ok(())
}

fn failed_page(title: &str, e: &str, retry: &str) -> Page {
    Page { title: title.into(), body: e.into(), primary: Some(retry.into()), secondary: Some("Close".into()), ..Default::default() }
}

// ------------------------------------------------------------------------------------------------
// install

struct Install {
    p: Paths,
    me: PathBuf,
    ffmpeg_missing: bool,
    /// (add-on index, already installed)
    addons: Vec<(usize, bool)>,
    opts: Options,
    installed: bool,
}

impl Install {
    fn new(me: PathBuf) -> Install {
        let p = paths();
        let ffmpeg_missing = !addons::installed(&FFMPEG, &p);
        let addons = ADDONS.iter().enumerate().map(|(i, a)| (i, addons::installed(a, &p))).collect();
        let rec = setup::Record::load(&p);
        let opts = Options {
            ffmpeg: true,
            addons: Vec::new(),
            desktop: rec.as_ref().map(|r| r.desktop).unwrap_or(false),
            context_menu: rec.as_ref().map(|r| r.context_menu).unwrap_or(true),
        };
        Install { p, me, ffmpeg_missing, addons, opts, installed: false }
    }

    fn page(&self) -> Page {
        let ffmpeg_size = crate::json::parse(addons::FFMPEG_JSON).ok().and_then(|j| j.num("size")).map(|n| mb(n as u64)).unwrap_or_default();
        let current = setup::installed_version(&self.p, &WinShell);
        let mut body = format!("For you only, no administrator needed. It goes in\n{}", self.p.install.display());
        if self.p.app_exe().is_file() {
            body.push_str(&format!(
                "\n\nSimple Editor{} is already here: this replaces it and keeps your settings.",
                if current.is_empty() { String::new() } else { format!(" {current}") }
            ));
        }
        if setup::local_app(&self.me).is_none() {
            body.push_str("\n\nThe latest Simple Editor is downloaded from GitHub.");
        }
        let mut checks = Vec::new();
        if self.ffmpeg_missing {
            body.push_str("\n\nFFmpeg isn't on this PC. Simple Editor needs it to export, convert and open some files.");
            checks.push(Check::new("Install FFmpeg", format!("Downloads FFmpeg 7.1.1 ({ffmpeg_size}) and checks it against its published checksum."), true));
        } else {
            checks.push(Check { enabled: false, ..Check::new("FFmpeg - Installed", "FFmpeg is already on this PC.", true) });
        }
        for &(i, have) in &self.addons {
            let a = &ADDONS[i];
            let mut c = if have {
                Check { enabled: false, ..Check::new(format!("{} (Installed)", a.label), "Already on this PC.", true) }
            } else {
                Check::new(a.label, "Optional. Downloaded and checksum-verified; you can also add it later from inside Simple Editor.", false)
            };
            if i == 0 {
                c.heading = Some("Add-ons".into());
            }
            checks.push(c);
        }
        let mut desk = Check::new("Desktop shortcut", "A Simple Editor icon on the desktop. The Start menu always gets one.", self.opts.desktop);
        desk.heading = Some("Options".into());
        checks.push(desk);
        checks.push(Check::new("Right-click menu", "Adds \"Edit with Simple Editor\" to video files in File Explorer (Show more options on Windows 11).", self.opts.context_menu));
        Page { title: "Install Simple Editor".into(), body, checks, primary: Some("Install".into()), secondary: Some("Cancel".into()), busy: false }
    }

    fn read(&mut self, checks: &[bool]) {
        let n = ADDONS.len();
        if checks.len() < n + 3 {
            return;
        }
        self.opts.ffmpeg = self.ffmpeg_missing && checks[0];
        self.opts.addons = self.addons.iter().filter(|&&(i, have)| !have && checks[1 + i]).map(|&(i, _)| ADDONS[i].id.to_string()).collect();
        self.opts.desktop = checks[n + 1];
        self.opts.context_menu = checks[n + 2];
    }

    fn job(&self) -> ui::Job {
        let (p, me, opts) = (self.p.clone(), self.me.clone(), self.opts.clone());
        Box::new(move |rep| run_install(&p, &me, &opts, rep))
    }
}

fn run_install(p: &Paths, me: &Path, opts: &Options, rep: &Report) -> Result<String, String> {
    let src = setup::local_app(me).map(AppSource::Local).unwrap_or(AppSource::Latest);
    setup::install(p, &mut WinReg::hkcu(), &WinNet, &WinShell, &src, me, opts, rep)
}

impl Flow for Install {
    fn first(&mut self) -> Next {
        Next::Show(self.page())
    }
    fn button(&mut self, b: Btn, checks: &[bool]) -> Next {
        match b {
            Btn::Primary if self.installed => {
                open_app(&self.p);
                Next::Close
            }
            Btn::Primary => {
                if !checks.is_empty() {
                    self.read(checks);
                }
                let mut busy = self.page();
                busy.title = "Installing Simple Editor".into();
                busy.primary = None;
                busy.busy = true;
                Next::Run(busy, self.job())
            }
            Btn::Secondary => Next::Close,
        }
    }
    fn done(&mut self, r: Result<String, String>) -> Next {
        match r {
            Ok(v) => {
                self.installed = true;
                Next::Show(Page {
                    title: "Simple Editor is installed".into(),
                    body: format!("Version {v}. It's in the Start menu{}.", if self.opts.desktop { " and on the desktop" } else { "" }),
                    primary: Some("Open Simple Editor".into()),
                    secondary: Some("Close".into()),
                    ..Default::default()
                })
            }
            Err(e) if e.starts_with("Simple Editor is installed") => {
                self.installed = true;
                Next::Show(Page { title: "Installed, with a problem".into(), body: e, primary: Some("Open Simple Editor".into()), secondary: Some("Close".into()), ..Default::default() })
            }
            Err(e) if e == report::CANCELLED => Next::Show(self.page()),
            Err(e) => Next::Show(failed_page("Simple Editor couldn't be installed", &e, "Try Again")),
        }
    }
}

fn install_main(args: &[String], me: &Path) -> i32 {
    if has(args, "--silent") {
        let p = paths();
        let opts = Options {
            ffmpeg: !has(args, "--no-ffmpeg"),
            addons: value(args, "--addons").map(|v| v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()).unwrap_or_default(),
            desktop: has(args, "--desktop"),
            context_menu: !has(args, "--no-context-menu"),
        };
        return match report::quiet(|r| run_install(&p, me, &opts, r)) {
            Ok(_) => 0,
            Err(_) => 1,
        };
    }
    ui::run("Simple Editor Setup", Box::new(Install::new(me.to_path_buf())));
    0
}

// ------------------------------------------------------------------------------------------------
// update

struct Update {
    p: Paths,
    current: String,
    release: Option<crate::net::Release>,
    updated: bool,
}

impl Update {
    fn checking(&self) -> Next {
        let current = self.current.clone();
        let page = Page { title: "Simple Editor updates".into(), body: "Checking for a newer version…".into(), secondary: Some("Cancel".into()), busy: true, ..Default::default() };
        Next::Run(
            page,
            Box::new(move |rep| {
                rep.say(0.3, "Asking GitHub for the latest release…");
                match setup::check_update(&WinNet, &current)? {
                    Some(r) => Ok(format!("new:{}", r.tag)),
                    None => Ok("same".into()),
                }
            }),
        )
    }
}

impl Flow for Update {
    fn first(&mut self) -> Next {
        if !self.p.app_exe().is_file() {
            return Next::Show(failed_page("Simple Editor isn't installed", &format!("There is no {APP_EXE} in\n{}", self.p.install.display()), "Close"));
        }
        self.checking()
    }
    fn button(&mut self, b: Btn, _: &[bool]) -> Next {
        match (b, &self.release) {
            (Btn::Primary, _) if self.updated => {
                open_app(&self.p);
                Next::Close
            }
            (Btn::Primary, Some(rel)) => {
                let (p, rel) = (self.p.clone(), rel.clone());
                let page = Page {
                    title: format!("Updating to {}", crate::version::plain(&rel.tag)),
                    body: "Your settings and projects stay as they are.".into(),
                    secondary: Some("Cancel".into()),
                    busy: true,
                    ..Default::default()
                };
                Next::Run(
                    page,
                    Box::new(move |rep| {
                        let exe = p.app_exe();
                        let wait = || {
                            rep.say(0.88, "Waiting for Simple Editor to close… save your work and close it.");
                            wait_closed(&exe, rep)
                        };
                        setup::apply_update(&p, &mut WinReg::hkcu(), &WinNet, &WinShell, &rel, &wait, rep)
                    }),
                )
            }
            (Btn::Primary, None) if !self.p.app_exe().is_file() => Next::Close,
            (Btn::Primary, None) => self.checking(),
            (Btn::Secondary, _) => Next::Close,
        }
    }
    fn done(&mut self, r: Result<String, String>) -> Next {
        match r {
            Ok(s) if s == "same" => Next::Show(Page {
                title: "Simple Editor is up to date".into(),
                body: format!("Version {} is the latest.", if self.current.is_empty() { "here" } else { &self.current }),
                secondary: Some("Close".into()),
                ..Default::default()
            }),
            Ok(s) if s.starts_with("new:") => {
                // look the release up again in the job's result shape: keep it simple, ask once more
                let tag = s[4..].to_string();
                match crate::net::latest_release(&WinNet, crate::net::REPO) {
                    Ok(r) if r.tag == tag => {
                        self.release = Some(r);
                        Next::Show(Page {
                            title: format!("Simple Editor {} is available", crate::version::plain(&tag)),
                            body: format!(
                                "You have {}. Updating keeps your settings and projects; Simple Editor closes for a moment if it's open.",
                                if self.current.is_empty() { "an older version".to_string() } else { self.current.clone() }
                            ),
                            primary: Some("Update".into()),
                            secondary: Some("Not Now".into()),
                            ..Default::default()
                        })
                    }
                    Ok(_) => self.checking(),
                    Err(e) => Next::Show(failed_page("Couldn't check for updates", &e, "Try Again")),
                }
            }
            Ok(v) => {
                self.updated = true;
                self.current = v.clone();
                Next::Show(Page {
                    title: "Simple Editor is updated".into(),
                    body: format!("Version {v}."),
                    primary: Some("Open Simple Editor".into()),
                    secondary: Some("Close".into()),
                    ..Default::default()
                })
            }
            Err(e) if e == report::CANCELLED => Next::Close,
            Err(e) if self.release.is_some() => Next::Show(failed_page("The update didn't finish", &format!("{e}\n\nSimple Editor was left as it was."), "Try Again")),
            Err(e) => Next::Show(failed_page("Couldn't check for updates", &e, "Try Again")),
        }
    }
}

fn update_main(args: &[String]) -> i32 {
    let p = paths();
    let current = setup::installed_version(&p, &WinShell);
    if has(args, "--silent") {
        // check, and when newer: download, wait for the app to close, swap
        let r = report::quiet(|rep| -> Result<(), String> {
            if let Some(rel) = setup::check_update(&WinNet, &current)? {
                let exe = p.app_exe();
                setup::apply_update(&p, &mut WinReg::hkcu(), &WinNet, &WinShell, &rel, &|| wait_closed(&exe, rep), rep)?;
            }
            Ok(())
        });
        return if r.is_ok() { 0 } else { 1 };
    }
    ui::run("Simple Editor Update", Box::new(Update { p, current, release: None, updated: false }));
    0
}

// ------------------------------------------------------------------------------------------------
// uninstall

struct Uninstall {
    p: Paths,
    me: PathBuf,
    temp_copy: bool,
    done: bool,
}

impl Flow for Uninstall {
    fn first(&mut self) -> Next {
        Next::Show(Page {
            title: "Uninstall Simple Editor".into(),
            body: "Removes Simple Editor, its add-ons, the FFmpeg it installed, its shortcuts and its registry entries.\n\nYour projects are never touched.".into(),
            checks: vec![
                Check { radio: true, ..Check::new("Keep preferences", "Keeps your settings, themes, Gallery items, library and skills for a later install.", true) },
                Check {
                    radio: true,
                    ..Check::new("Remove preferences", "Also deletes settings, themes, Gallery items, library, skills and the saved Assistant key.", false)
                },
            ],
            primary: Some("Uninstall".into()),
            secondary: Some("Cancel".into()),
            busy: false,
        })
    }
    fn button(&mut self, b: Btn, checks: &[bool]) -> Next {
        match b {
            Btn::Primary if !self.done => {
                let keep = checks.first().copied().unwrap_or(true);
                let p = self.p.clone();
                let page = Page { title: "Uninstalling Simple Editor".into(), secondary: Some("Cancel".into()), busy: true, ..Default::default() };
                Next::Run(page, Box::new(move |rep| run_uninstall(&p, keep, rep)))
            }
            _ => Next::Close,
        }
    }
    fn done(&mut self, r: Result<String, String>) -> Next {
        match r {
            Ok(note) => {
                self.done = true;
                Next::Show(Page { title: "Simple Editor was removed".into(), body: note, secondary: Some("Close".into()), ..Default::default() })
            }
            Err(e) if e == report::CANCELLED => self.first(),
            Err(e) => Next::Show(failed_page("Simple Editor couldn't be removed", &e, "Try Again")),
        }
    }
    fn closing(&mut self) {
        if self.temp_copy {
            shell::delete_after_exit(&self.me);
        }
    }
}

fn run_uninstall(p: &Paths, keep_prefs: bool, rep: &Report) -> Result<String, String> {
    let exe = p.app_exe();
    if WinShell.running(&exe) {
        rep.say(0.05, "Waiting for Simple Editor to close… save your work and close it.");
        wait_closed(&exe, rep)?;
    }
    rep.check()?;
    let mut left = setup::uninstall(p, &mut WinReg::hkcu(), &WinShell, keep_prefs, rep)?;
    // the uninstall.exe this copy started from may still be exiting: try the program folder again
    for _ in 0..10 {
        if !left.contains(&p.install) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(300));
        if !setup::remove_tree_keep_projects(&p.install) {
            left.retain(|d| d != &p.install);
        }
    }
    let mut note = if keep_prefs { "Your preferences were kept for a later install." } else { "Your preferences were removed too." }.to_string();
    if !left.is_empty() {
        note.push_str("\n\nThese folders were kept because they hold projects or files in use:");
        for d in &left {
            note.push_str(&format!("\n{}", d.display()));
        }
    }
    Ok(note)
}

fn uninstall_main(args: &[String], me: &Path) -> i32 {
    let p = paths();
    let temp_copy = has(args, "--from-temp");
    // a program can't delete its own folder while it runs from it: carry on from a copy in %TEMP%
    if !temp_copy && me.starts_with(&p.install) {
        let tmp = std::env::temp_dir().join(format!("se-uninstall-{}.exe", std::process::id()));
        if std::fs::copy(me, &tmp).is_ok() {
            let mut a: Vec<&str> = vec!["--uninstall", "--from-temp"];
            if has(args, "--silent") {
                a.push("--silent");
            }
            if has(args, "--remove-preferences") {
                a.push("--remove-preferences");
            }
            if shell::spawn_hidden(&tmp, &a).is_ok() {
                return 0;
            }
        }
    }
    if has(args, "--silent") {
        let keep = !has(args, "--remove-preferences");
        let r = report::quiet(|rep| run_uninstall(&p, keep, rep));
        if temp_copy {
            shell::delete_after_exit(me);
        }
        return if r.is_ok() { 0 } else { 1 };
    }
    ui::run("Uninstall Simple Editor", Box::new(Uninstall { p, me: me.to_path_buf(), temp_copy, done: false }));
    0
}
