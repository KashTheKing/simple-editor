//! Add-on installs run as background jobs started from Settings ▸ Add-ons or an action's "install it?"
//! offer. A failed install says so in a toast, wherever the user is (the Add-ons row keeps the reason too).

use super::*;
use crate::engine::{bgremove, deepfilter, demucs, enhance, fast_whisper, sam2, tts_addon, vlm};

thread_local! {
    /// The failures already shown, per add-on (so each one toasts once).
    static SHOWN: std::cell::RefCell<Vec<Option<String>>> = Default::default();
}

/// FRAME_HOOK: toast a newly failed install.
pub(super) fn tick(app: &mut App, _ctx: &egui::Context) {
    let now = [
        ("Stem Separation", demucs::install_error()),
        ("Speech denoise", deepfilter::install_error()),
        ("Object masks", sam2::install_error()),
        ("Background removal", bgremove::install_error()),
        ("AI enhancer", enhance::install_error()),
        ("Transcription", fast_whisper::install_error()),
        ("Image understanding", vlm::install_error()),
        ("Kokoro voices", tts_addon::install_error(tts_addon::Addon::Kokoro)),
        ("Chatterbox voices", tts_addon::install_error(tts_addon::Addon::Chatterbox)),
    ];
    let fresh: Vec<String> = SHOWN.with(|s| {
        let mut s = s.borrow_mut();
        s.resize(now.len(), None);
        let mut out = Vec::new();
        for (i, (name, e)) in now.iter().enumerate() {
            if e.is_some() && *e != s[i] {
                out.push(format!("{name} install failed: {}", e.as_deref().unwrap_or_default()));
            }
            s[i] = e.clone();
        }
        out
    });
    for msg in fresh {
        app.push_toast(feedback::Toast::new(msg).kind(feedback::ToastKind::Error).action("Add-ons", Action::Settings));
    }
}
