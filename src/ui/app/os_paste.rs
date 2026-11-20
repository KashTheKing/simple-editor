//! Ctrl+V with something from outside the app on the clipboard: a screenshot or any copied picture, or
//! files copied in Explorer, land in the Library and on the timeline at the playhead (one undo step via
//! the import). The app's own clip copy is text on the OS clipboard, so a picture or a file list there
//! is always newer than an internal copy and wins; otherwise Paste pastes the copied clips as before.
//! egui-winit only reports a paste when the clipboard holds text, so for pictures and files the key is
//! read from Windows (the same trick as the Assistant composer's paste).

use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

static V_DOWN: AtomicBool = AtomicBool::new(false);

/// What is on the OS clipboard that Paste can use, if anything.
enum Outside {
    Files(Vec<PathBuf>),
    Picture { rgba: Vec<u8>, w: u32, h: u32 },
}

fn outside() -> Option<Outside> {
    let mut cb = arboard::Clipboard::new().ok()?;
    if let Ok(files) = cb.get().file_list() {
        let files: Vec<PathBuf> = files.into_iter().filter(|p| p.exists()).collect();
        if !files.is_empty() {
            return Some(Outside::Files(files));
        }
    }
    let img = cb.get_image().ok()?;
    Some(Outside::Picture { rgba: img.bytes.into_owned(), w: img.width as u32, h: img.height as u32 })
}

/// Where a pasted picture is saved: beside the project (`Pasted\`) when it has a file, else the app's data.
fn pasted_dir(app: &App) -> PathBuf {
    match app.project_path.as_ref().and_then(|p| p.parent()) {
        Some(d) => d.join("Pasted"),
        None => crate::settings::Settings::dir().join("Pasted"),
    }
}

fn save_picture(dir: &std::path::Path, rgba: Vec<u8>, w: u32, h: u32) -> Result<PathBuf, String> {
    if w == 0 || h == 0 || rgba.len() < (w * h * 4) as usize {
        return Err("the picture on the clipboard is empty".into());
    }
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let stamp = chrono_like_stamp();
    let mut path = dir.join(format!("Pasted {stamp}.png"));
    let mut n = 2;
    while path.exists() {
        path = dir.join(format!("Pasted {stamp} ({n}).png"));
        n += 1;
    }
    let mut frame = crate::media::Frame::new(w, h);
    frame.rgba.copy_from_slice(&rgba[..(w * h * 4) as usize]);
    std::fs::write(&path, crate::mcp::png_encode(&frame)).map_err(|e| e.to_string())?;
    Ok(path)
}

/// `YYYY-MM-DD HHMMSS` in UTC from the system clock (no date crate for one file name).
fn chrono_like_stamp() -> String {
    let s = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let (days, rem) = (s / 86_400, s % 86_400);
    // civil-from-days (Howard Hinnant), valid for the Gregorian calendar
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02} {:02}{:02}{:02}", rem / 3600, rem / 60 % 60, rem % 60)
}

/// Paste from outside the app if the clipboard holds a picture or files; true when it handled the paste.
pub(super) fn try_paste(app: &mut App) -> bool {
    let Some(what) = outside() else { return false };
    let paths = match what {
        Outside::Files(f) => f,
        Outside::Picture { rgba, w, h } => match save_picture(&pasted_dir(app), rgba, w, h) {
            Ok(p) => vec![p],
            Err(e) => {
                app.toast(format!("Couldn't paste the picture: {e}"));
                return true;
            }
        },
    };
    let ids = app.import_files(&paths);
    if ids.is_empty() {
        return true; // a folder or project: import_files handled it
    }
    let mut at = app.playhead;
    let mut placed = Vec::new();
    for id in ids {
        let clips = app.project.insert_asset_clips(id, at, None);
        if let Some(end) = clips.iter().filter_map(|c| app.project.clip(*c)).map(|c| c.start + c.duration).reduce(f64::max) {
            at = end;
        }
        placed.extend(clips);
    }
    if !placed.is_empty() {
        app.selection = placed;
        app.after_edit();
    }
    let n = paths.len();
    app.toast(if n == 1 { format!("Pasted {}", paths[0].file_name().unwrap_or_default().to_string_lossy()) } else { format!("Pasted {n} files") });
    true
}

/// Per frame: Ctrl+V while no text field has focus and egui saw no text paste (a picture or a file list
/// on the clipboard) runs Paste.
pub(super) fn tick(app: &mut App, ctx: &egui::Context) {
    let armed = ctx.input(|i| i.focused && i.modifiers.command) && !ctx.wants_keyboard_input();
    // SAFETY: GetAsyncKeyState has no preconditions; the high bit is "down now".
    let v = armed && unsafe { windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState(i32::from(b'V')) } < 0;
    let pressed = v && !V_DOWN.swap(v, Ordering::Relaxed);
    if !v {
        V_DOWN.store(false, Ordering::Relaxed);
    }
    let text_paste = ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Paste(_))));
    if pressed && !text_paste {
        app.pending_actions.push(Action::PasteClips);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn stamp_is_a_plain_date_and_time() {
        let s = super::chrono_like_stamp();
        assert_eq!(s.len(), 17, "{s}");
        assert_eq!(&s[4..5], "-");
        assert_eq!(&s[10..11], " ");
    }
}
