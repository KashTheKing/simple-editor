//! ---- ws:voice-input ----
//! Dictation into the Assistant prompt. The contract with `ui::assistant_ui` (ws:assistant-pane):
//! its mic button toggles `AssistantState.mic_wanted`; this tick starts a recording when that turns
//! true and stops + transcribes when it turns false, appends the text to `prompt` (sets `send_now`
//! with `Settings.voice_autosend`) and owns `mic_status` ("" idle, "Listening…", "Transcribing…" or a
//! short error, cleared on the next start).
//!
//! Push to Talk: held longer than `TAP` it records until release; a quick tap (or the palette, no key
//! down) toggles - recording continues until the next press. Unbound by default - bind it in Settings ▸ Hotkeys. App exit drops the
//! `Recorder`, which stops the stream on its thread.
//! ponytail: the hotkey poll ignores keys while a text field has focus, so with the prompt box
//! focused use its mic button - polling the key ahead of `wants_keyboard_input` needs a hotkeys.rs change.

use super::*;
use crate::engine::{transcribe, voice};
use crate::mcp::tools::{ToolDef, ToolKind, ToolOutcome};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

/// Wake-up cadence while recording / transcribing (never when idle).
const POLL: Duration = Duration::from_millis(100);
/// Push to Talk released sooner than this after the press = a tap (toggle); later = hold-to-talk.
const TAP: Duration = Duration::from_millis(300);

fn is_hold(held_for: Duration) -> bool {
    held_for >= TAP
}

#[derive(Default)]
pub(super) struct VoiceState {
    rec: Option<voice::Recorder>,
    /// whisper running on a thread over the last take.
    job: Option<Receiver<Result<String, String>>>,
    /// `mic_wanted` as last seen - only its edges act.
    was_wanted: bool,
    /// Push to Talk's key while it may still be down: (key, when pressed). Watched until released.
    held: Option<(egui::Key, Instant)>,
    /// The last take's result, for `voice.status` / `voice.stop` callers.
    last: Option<Result<String, String>>,
}

impl VoiceState {
    /// Does the tick need to keep waking up? (Idle = no repaint at all.)
    fn busy(&self) -> bool {
        self.rec.is_some() || self.job.is_some() || self.held.is_some()
    }

    fn state(&self) -> &'static str {
        if self.rec.is_some() {
            "recording"
        } else if self.job.is_some() {
            "transcribing"
        } else {
            "idle"
        }
    }
}

fn start(app: &mut App) {
    app.assistant.mic_status.clear();
    let err = if transcribe::exe().is_none() {
        Some("Whisper is not installed — run Install in the Subtitles pane")
    } else if !transcribe::have_model(transcribe::MODELS[transcribe::default_model()].1) {
        Some("No whisper model — Get captions in the Subtitles pane")
    } else {
        None
    };
    if let Some(e) = err {
        app.assistant.mic_status = e.to_string();
        app.assistant.mic_wanted = false;
        app.voice.was_wanted = false;
        app.voice.held = None;
        return;
    }
    app.voice.rec = Some(voice::Recorder::start(&app.settings.capture_mic));
    app.assistant.mic_status = "Listening…".into();
}

/// A take's WAV landed (stopped, capped at 60 s, or failed): transcribe it on a thread.
fn on_wav(app: &mut App, r: Result<PathBuf, String>) {
    app.voice.rec = None;
    // a take that ended by itself also ends the "wanted" state, so the button pops back up
    app.assistant.mic_wanted = false;
    app.voice.was_wanted = false;
    app.voice.held = None;
    match r {
        Ok(wav) => {
            let (tx, rx) = mpsc::channel();
            std::thread::spawn(move || {
                let _ = tx.send(voice::transcribe_short(&wav));
            });
            app.voice.job = Some(rx);
            app.assistant.mic_status = "Transcribing…".into();
        }
        Err(e) => {
            app.assistant.mic_status = e.clone();
            app.voice.last = Some(Err(e));
        }
    }
}

fn apply(app: &mut App, r: Result<String, String>) {
    match &r {
        Ok(text) => {
            let p = &mut app.assistant.prompt;
            if !p.is_empty() && !p.ends_with(char::is_whitespace) {
                p.push(' ');
            }
            p.push_str(text);
            app.assistant.mic_status.clear();
            if app.settings.voice_autosend {
                app.assistant.send_now = true;
            }
        }
        Err(e) => app.assistant.mic_status = e.clone(),
    }
    app.voice.last = Some(r);
}

/// FRAME_HOOK.
pub(super) fn tick(app: &mut App, ctx: &egui::Context) {
    if let Some((key, pressed)) = app.voice.held {
        if !ctx.input(|i| i.key_down(key)) {
            // released after a hold = stop; a quick tap (or the palette) = toggle, keep recording
            app.voice.held = None;
            if is_hold(pressed.elapsed()) {
                app.assistant.mic_wanted = false;
            }
        }
    }
    let want = app.assistant.mic_wanted;
    if want != app.voice.was_wanted {
        app.voice.was_wanted = want;
        if want {
            start(app);
        } else if let Some(rec) = &app.voice.rec {
            rec.stop();
        }
    }
    if let Some(r) = app.voice.rec.as_ref().and_then(|rec| rec.try_finish()) {
        on_wav(app, r);
    }
    if let Some(rx) = &app.voice.job {
        let r = match rx.try_recv() {
            Ok(r) => Some(r),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err("transcription stopped unexpectedly".to_string())),
        };
        if let Some(r) = r {
            app.voice.job = None;
            apply(app, r);
            // one more frame: show the text, and let assistant_ctl::tick act on `send_now` (it ran before us)
            ctx.request_repaint();
        }
    }
    if app.voice.busy() {
        app.animate_until(ctx, Instant::now() + POLL);
    }
}

/// ACT_HANDLERS entry.
pub(super) fn act(app: &mut App, a: Action) -> bool {
    if a != Action::PushToTalk {
        return false;
    }
    if app.voice.held.is_some() {
        return true; // key repeat while held
    }
    app.surface(Pane::Assistant);
    if app.assistant.mic_wanted {
        app.assistant.mic_wanted = false;
    } else {
        app.assistant.mic_wanted = true;
        // the tick decides on release: a hold stops it, a tap / the palette (key not down) toggles
        app.voice.held = app.hotkeys.get(Action::PushToTalk).map(|k| (k.logical_key, Instant::now()));
    }
    true
}

fn status_json(app: &App) -> Value {
    let mut v = json!({"state": app.voice.state(), "status": app.assistant.mic_status});
    match &app.voice.last {
        Some(Ok(t)) => v["text"] = json!(t),
        Some(Err(e)) => v["error"] = json!(e),
        None => {}
    }
    v
}

pub const TOOLS: &[ToolDef] = &[
    ToolDef {
        name: "voice.start",
        desc: "Start dictating into the Assistant prompt from the microphone (stops by itself after 60 s).",
        args: &[],
        kind: ToolKind::Ui,
        run: |app, _| {
            if app.voice.rec.is_some() {
                return Err("already recording".into());
            }
            app.voice.last = None;
            app.assistant.mic_wanted = true;
            app.voice.was_wanted = true;
            start(app);
            Ok(ToolOutcome::Done(status_json(app)))
        },
    },
    ToolDef {
        name: "voice.stop",
        desc: "Stop dictating and transcribe. whisper takes a few seconds: poll voice.status until state is \
               'idle', then 'text' (also appended to the prompt) or 'error'.",
        args: &[],
        kind: ToolKind::Ui,
        run: |app, _| {
            let Some(rec) = &app.voice.rec else { return Err("not recording".into()) };
            rec.stop();
            app.assistant.mic_wanted = false;
            app.voice.was_wanted = false;
            app.voice.held = None;
            Ok(ToolOutcome::Done(json!({"state": "transcribing"})))
        },
    },
    ToolDef {
        name: "voice.status",
        desc: "Dictation state: 'recording', 'transcribing' or 'idle', the pane's status line, and the last \
               take's 'text' or 'error'.",
        args: &[],
        kind: ToolKind::Read,
        run: |app, _| Ok(ToolOutcome::Done(status_json(app))),
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    /// Idle dictation asks the tick for no wake-ups (the tick only calls `animate_until` when busy).
    #[test]
    fn assert_no_idle_repaint_voice() {
        let s = VoiceState::default();
        assert!(!s.busy());
        assert_eq!(s.state(), "idle");
        let s = VoiceState { held: Some((egui::Key::F9, Instant::now())), ..Default::default() };
        assert!(s.busy(), "a pending push-to-talk key is polled until it is released");
    }

    #[test]
    fn push_to_talk_tap_toggles_hold_stops() {
        assert!(!is_hold(Duration::from_millis(80)), "a tap keeps recording");
        assert!(is_hold(Duration::from_millis(900)), "a hold stops on release");
    }
}
