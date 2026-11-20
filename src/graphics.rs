//! ---- forum-crash ----
//! Graphics start-up failures. When the window or its OpenGL context can't be created (an old GPU,
//! a broken driver), the user gets a plain dialog with one way forward, "Start in compatibility mode",
//! instead of an OS error or a white window.
//!
//! Compatibility mode:
//! - the preview renders on the CPU compositor (the GPU preview's shaders are what old drivers trip on),
//! - vsync is off (some drivers hang in the swap),
//! - a software OpenGL (Mesa's `opengl32.dll`, MIT) is used when one sits in a `compat-gl` folder next
//!   to the exe or in `%LOCALAPPDATA%\SimpleEditor\compat-gl`: that folder is searched before
//!   System32, so the whole window renders on the CPU. None is bundled.
//!
//! It turns on with `--compat-graphics`, `SE_COMPAT_GRAPHICS=1`, Settings ▸ Performance ▸ Compatibility
//! graphics, or by answering Yes after a failed start (that also ticks the setting). A run that died
//! before its first frame (`crashguard::Kind::Graphics`) asks the same question on the next start.
//! `SE_FORCE_GL_FAIL=1` skips window creation outside compatibility mode, to test the dialog path.

use crate::settings::Settings;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

static COMPAT: AtomicBool = AtomicBool::new(false);

/// True when this run uses compatibility graphics.
pub fn compat() -> bool {
    COMPAT.load(Ordering::Relaxed) || std::env::var("SE_COMPAT_GRAPHICS").is_ok_and(|v| v == "1")
}

#[derive(Debug, PartialEq, Eq)]
pub enum Decision {
    Off,
    On,
    /// The last run died before its first frame: ask.
    Ask,
}

/// Pure: whether this start uses compatibility graphics.
pub fn decide(forced: bool, setting: bool, died_in_graphics: bool, quiet: bool) -> Decision {
    if forced || setting {
        Decision::On
    } else if died_in_graphics && !quiet {
        Decision::Ask
    } else {
        Decision::Off
    }
}

/// `main`, before the window: settles compatibility mode for this run (may ask, may tick the setting).
pub fn decide_compat(seed: &mut Settings) {
    let died = crate::crashguard::leftovers().entries.iter().any(|e| e.kind == crate::crashguard::Kind::Graphics);
    let on = match decide(compat(), seed.compat_graphics, died, crate::winpos::quiet()) {
        Decision::On => true,
        Decision::Off => false,
        Decision::Ask => {
            let yes = ask_yes_no(
                "Simple Editor closed before its window was ready last time.\n\n\
                 Start in compatibility mode? (Settings > Performance turns it off again.)",
            );
            if yes {
                seed.compat_graphics = true;
                seed.save();
            }
            yes
        }
    };
    COMPAT.store(on, Ordering::Relaxed);
    if on {
        use_software_gl_if_present();
    }
}

/// eframe options for this run.
pub fn native_options(mut o: eframe::NativeOptions) -> eframe::NativeOptions {
    if compat() {
        o.vsync = false;
        o.multisampling = 0;
        o.depth_buffer = 0;
        o.stencil_buffer = 0;
        o.hardware_acceleration = eframe::HardwareAcceleration::Preferred;
    }
    o
}

/// The test hook: pretend the window could not be created (never in compatibility mode).
pub fn forced_failure() -> bool {
    !compat() && std::env::var("SE_FORCE_GL_FAIL").is_ok_and(|v| v == "1")
}

/// Folders searched for a software `opengl32.dll`.
pub fn software_gl_dirs() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(d) = std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.join("compat-gl"))) {
        v.push(d);
    }
    if let Some(l) = std::env::var_os("LOCALAPPDATA") {
        v.push(PathBuf::from(l).join(crate::settings::APP_DIR).join("compat-gl"));
    }
    v
}

#[cfg(windows)]
fn use_software_gl_if_present() {
    use windows::core::HSTRING;
    use windows::Win32::System::LibraryLoader::SetDllDirectoryW;
    if let Some(dir) = software_gl_dirs().into_iter().find(|d| d.join("opengl32.dll").is_file()) {
        // searched after the exe's folder and before System32: glutin's LoadLibrary("opengl32.dll") finds it
        unsafe {
            let _ = SetDllDirectoryW(&HSTRING::from(dir.as_os_str()));
        }
    }
}

#[cfg(not(windows))]
fn use_software_gl_if_present() {}

/// The args a compatibility relaunch gets: the same ones plus `--compat-graphics` (once).
pub fn relaunch_args(args: &[String]) -> Vec<String> {
    let mut v: Vec<String> = args.to_vec();
    if !v.iter().any(|a| a == "--compat-graphics") {
        v.push("--compat-graphics".into());
    }
    v
}

/// `run_native` failed: say so plainly and offer compatibility mode (a fresh process, so nothing of
/// the failed context is left), or, already in it, what else to try.
pub fn start_failed(err: &str) {
    if crate::winpos::quiet() {
        return; // automation: the exit code and stderr say it
    }
    if compat() {
        let dirs = software_gl_dirs();
        let folder = dirs.last().map(|d| d.display().to_string()).unwrap_or_default();
        message(&format!(
            "Simple Editor couldn't start its graphics, even in compatibility mode.\n\n\
             Update the graphics driver, or put a software OpenGL (Mesa's opengl32.dll) in\n{folder}\n\n({err})"
        ));
        return;
    }
    if !ask_yes_no(&format!("Simple Editor couldn't start its graphics.\n\nStart in compatibility mode?\n\n({err})")) {
        return;
    }
    let mut s = Settings::load();
    s.compat_graphics = true;
    s.save();
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::process::Command::new(exe).args(relaunch_args(&args)).spawn();
    }
}

#[cfg(windows)]
fn ask_yes_no(text: &str) -> bool {
    use windows::core::HSTRING;
    use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, IDYES, MB_ICONWARNING, MB_YESNO};
    unsafe { MessageBoxW(None, &HSTRING::from(text), &HSTRING::from("Simple Editor"), MB_YESNO | MB_ICONWARNING) == IDYES }
}

#[cfg(windows)]
fn message(text: &str) {
    use windows::core::HSTRING;
    use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};
    unsafe {
        MessageBoxW(None, &HSTRING::from(text), &HSTRING::from("Simple Editor"), MB_OK | MB_ICONERROR);
    }
}

#[cfg(not(windows))]
fn ask_yes_no(text: &str) -> bool {
    eprintln!("{text}");
    false
}

#[cfg(not(windows))]
fn message(text: &str) {
    eprintln!("{text}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compat_mode_decision() {
        assert_eq!(decide(true, false, false, false), Decision::On, "--compat-graphics");
        assert_eq!(decide(false, true, false, false), Decision::On, "the setting");
        assert_eq!(decide(false, false, true, false), Decision::Ask, "died before the first frame last time");
        assert_eq!(decide(false, false, true, true), Decision::Off, "a quiet (automation) start never asks");
        assert_eq!(decide(false, false, false, false), Decision::Off);
    }

    #[test]
    fn relaunch_keeps_args_and_adds_the_flag_once() {
        let a = vec!["C:\\p\\film.sedit".to_string()];
        assert_eq!(relaunch_args(&a), vec!["C:\\p\\film.sedit".to_string(), "--compat-graphics".into()]);
        assert_eq!(relaunch_args(&relaunch_args(&a)).len(), 2);
    }

    #[test]
    fn compat_options_drop_vsync_and_extra_buffers() {
        COMPAT.store(true, Ordering::Relaxed);
        let o = native_options(eframe::NativeOptions { multisampling: 4, depth_buffer: 24, ..Default::default() });
        COMPAT.store(false, Ordering::Relaxed);
        assert!(!o.vsync);
        assert_eq!((o.multisampling, o.depth_buffer, o.stencil_buffer), (0, 0, 0));
    }
}
