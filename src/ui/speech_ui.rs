//! ---- ws:addon-voice ----
//! The Speech section of the Voiceover pane: engine (Windows / Kokoro / Chatterbox), voice with a Play
//! preview, speed, pitch and Chatterbox's intensity, voice cloning for Chatterbox, a script box, and
//! Speak at playhead / Speak selection. App-free: it returns what was asked for and `app::speech_ctl`
//! does it.

use crate::engine::tts::{Engine, Voice};
use crate::settings::Settings;
use eframe::egui::{self, Button, Slider, TextEdit};

/// Panel-local state (the persistent choices live in `Settings`).
#[derive(Default)]
pub struct SpeechUi {
    pub script: String,
    /// Name for the next cloned voice.
    pub clone_name: String,
}

/// What the app knows that the panel shows.
#[derive(Default)]
pub struct SpeechView<'a> {
    /// The current engine's voices (None = still being listed).
    pub voices: Option<&'a [Voice]>,
    /// Kokoro / Chatterbox installed.
    pub installed: [bool; 2],
    /// The current engine's add-on install: (fraction, status).
    pub installing: Option<(f32, String)>,
    pub install_error: Option<String>,
    /// The current engine's download size in bytes.
    pub download: u64,
    /// A preview is being synthesised.
    pub previewing: bool,
    /// The Main bus is muted: no preview.
    pub muted: bool,
    /// How many clips "Speak selection" would make (selected text clips, or the selected transcript words).
    pub selection: usize,
    /// What a clone would copy (the selected Library item, a selected audio clip or the last take).
    pub clone_source: Option<String>,
}

#[derive(Default, Debug, PartialEq)]
pub struct SpeechResponse {
    pub speak_script: bool,
    pub speak_selection: bool,
    pub preview: bool,
    pub install: bool,
    pub clone: bool,
    pub remove_clone: Option<String>,
    /// A setting changed: save.
    pub changed: bool,
}

fn installed(view: &SpeechView, e: Engine) -> bool {
    match e {
        Engine::Windows => true,
        Engine::Kokoro => view.installed[0],
        Engine::Chatterbox => view.installed[1],
    }
}

pub fn show(ui: &mut egui::Ui, st: &mut SpeechUi, s: &mut Settings, view: &SpeechView) -> SpeechResponse {
    let mut r = SpeechResponse::default();
    let engine = s.tts_engine;
    let ready = installed(view, engine);
    egui::Grid::new("speech_opts").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
        ui.label("Engine");
        ui.horizontal(|ui| {
            for e in Engine::ALL {
                let tip = match e {
                    Engine::Windows => "The voices built into Windows",
                    Engine::Kokoro => "Natural voices that run on the processor (add-on)",
                    Engine::Chatterbox => "Expressive voices and voice cloning, fastest with a graphics card (add-on)",
                };
                if ui.selectable_label(engine == e, e.label()).on_hover_text(tip).clicked() && engine != e {
                    s.tts_engine = e;
                    r.changed = true;
                }
            }
        });
        ui.end_row();

        if !ready {
            ui.label("");
            ui.horizontal(|ui| match &view.installing {
                Some((f, status)) => {
                    ui.add(egui::Spinner::new());
                    ui.weak(format!("{:.0}% · {status}", f * 100.0));
                }
                None => {
                    let mb = view.download as f64 / 1048576.0;
                    if ui
                        .button("Install")
                        .on_hover_text(format!("Downloads {mb:.0} MB once; until then Speak uses a Windows voice"))
                        .clicked()
                    {
                        r.install = true;
                    }
                    match &view.install_error {
                        Some(e) => ui.colored_label(ui.visuals().error_fg_color, e),
                        None => ui.weak(format!("{mb:.0} MB")),
                    };
                }
            });
            ui.end_row();
        }

        ui.label("Voice");
        ui.horizontal(|ui| {
            let key = engine.id().to_string();
            let mut cur = s.tts_voices.get(&key).cloned().unwrap_or_default();
            let shown = view
                .voices
                .and_then(|v| v.iter().find(|x| x.id == cur).map(|x| x.label.clone()))
                .unwrap_or_else(|| if cur.is_empty() { "Default".into() } else { cur.clone() });
            egui::ComboBox::from_id_salt("speech_voice").selected_text(shown).width(180.0).show_ui(ui, |ui| {
                if engine != Engine::Chatterbox {
                    ui.selectable_value(&mut cur, String::new(), "Default");
                }
                match view.voices {
                    Some(v) => {
                        for x in v {
                            ui.selectable_value(&mut cur, x.id.clone(), &x.label);
                        }
                    }
                    None => {
                        ui.weak("Listing voices…");
                    }
                }
            });
            if s.tts_voices.get(&key).map(String::as_str).unwrap_or("") != cur {
                s.tts_voices.insert(key, cur.clone());
                r.changed = true;
            }
            let play = ui.add_enabled(!view.previewing && !view.muted, Button::new(if view.previewing { "Playing…" } else { "Play" }));
            let play = if view.muted { play.on_disabled_hover_text("The Main bus is muted") } else { play };
            if play.on_hover_text("Speaks a short sample with this voice").clicked() {
                r.preview = true;
            }
            if engine == Engine::Chatterbox && !cur.is_empty() && ui.small_button("×").on_hover_text("Delete this cloned voice").clicked() {
                r.remove_clone = Some(cur);
            }
        });
        ui.end_row();

        ui.label("Speed");
        r.changed |= ui.add(Slider::new(&mut s.tts_speed, 0.5..=2.0).step_by(0.05).suffix("×")).changed();
        ui.end_row();
        ui.label("Pitch");
        r.changed |= ui
            .add(Slider::new(&mut s.tts_pitch, -12.0..=12.0).step_by(0.5).suffix(" st"))
            .on_hover_text("Semitones up or down")
            .changed();
        ui.end_row();
        if engine == Engine::Chatterbox {
            ui.label("Intensity");
            r.changed |= ui
                .add(Slider::new(&mut s.tts_intensity, 0.25..=2.0).step_by(0.05))
                .on_hover_text("How much emotion goes into the delivery; 0.5 is neutral, higher also speaks a little faster")
                .changed();
            ui.end_row();

            ui.label("Clone");
            ui.horizontal(|ui| {
                ui.add(TextEdit::singleline(&mut st.clone_name).desired_width(110.0).hint_text("Name"));
                let can = view.clone_source.is_some() && !st.clone_name.trim().is_empty();
                let b = ui.add_enabled(can, Button::new("Clone"));
                let b = match &view.clone_source {
                    Some(src) => b.on_hover_text(format!("Makes a voice from 5 to 15 s of {src}")),
                    None => b.on_disabled_hover_text("Select a clip of the voice in the Library or on the timeline, or record a take"),
                };
                if b.clicked() {
                    r.clone = true;
                }
            });
            ui.end_row();
            ui.label("");
            ui.weak("Only clone voices you have permission to use");
            ui.end_row();
        }
    });
    ui.add_space(4.0);
    ui.add(
        TextEdit::multiline(&mut st.script)
            .desired_rows(3)
            .desired_width(f32::INFINITY)
            .hint_text("Script: one clip per line; start a line with 0:12 to place it there"),
    );
    ui.horizontal(|ui| {
        let has = st.script.chars().any(|c| c.is_alphanumeric());
        if ui
            .add_enabled(has, Button::new("Speak at playhead"))
            .on_hover_text("Each line becomes a clip on the Voiceover track, one after another from the playhead")
            .clicked()
        {
            r.speak_script = true;
        }
        if ui
            .add_enabled(view.selection > 0, Button::new("Speak selection"))
            .on_hover_text("Speaks the selected text clips at their start, or the selected transcript words where they are")
            .on_disabled_hover_text("Select text clips or transcript words")
            .clicked()
        {
            r.speak_selection = true;
        }
    });
    r
}

/// The script's lines as (time, text): a line starting with `m:ss`, `m:ss.f` or `h:mm:ss` is placed at
/// that time, the others follow one after another. Blank lines are dropped.
pub fn script_lines(script: &str) -> Vec<(Option<f64>, String)> {
    script
        .lines()
        .filter_map(|l| {
            let l = l.trim();
            let (head, rest) = l.split_once(char::is_whitespace).unwrap_or((l, ""));
            let t = parse_time(head.trim_end_matches(['-', ':']));
            let text = if t.is_some() { rest.trim().trim_start_matches(['-', ':']).trim() } else { l };
            text.chars().any(|c| c.is_alphanumeric()).then(|| (t, text.to_string()))
        })
        .collect()
}

fn parse_time(s: &str) -> Option<f64> {
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() < 2 || parts.len() > 3 || parts.iter().any(|p| p.is_empty()) {
        return None;
    }
    let mut t = 0.0;
    for p in &parts {
        t = t * 60.0 + p.parse::<f64>().ok().filter(|v| *v >= 0.0)?;
    }
    Some(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_lines_take_an_optional_time() {
        let l = script_lines("Hello there.\n\n0:12 Second line\n1:02:03.5 - Third\n  12:x not a time\n:::\n");
        assert_eq!(
            l,
            vec![
                (None, "Hello there.".to_string()),
                (Some(12.0), "Second line".to_string()),
                (Some(3723.5), "Third".to_string()),
                (None, "12:x not a time".to_string()),
            ]
        );
    }

    fn run(view: &SpeechView, engine: Engine) -> (SpeechResponse, bool) {
        let ctx = egui::Context::default();
        let mut st = SpeechUi { script: "Hi".into(), ..Default::default() };
        let mut s = Settings::default();
        s.tts_engine = engine;
        let mut r = SpeechResponse::default();
        let mut repaint = false;
        for _ in 0..3 {
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| r = show(ui, &mut st, &mut s, view));
            });
            repaint = ctx.has_requested_repaint();
        }
        (r, repaint)
    }

    /// Each engine, installed or not, lays out without asking for anything or for a repaint.
    #[test]
    fn assert_no_idle_repaint_speech_panel() {
        let voices = vec![Voice { id: "af_heart".into(), label: "Heart (US, female)".into() }];
        for engine in Engine::ALL {
            for installed in [false, true] {
                let view = SpeechView {
                    voices: Some(&voices),
                    installed: [installed; 2],
                    download: 160 << 20,
                    clone_source: Some("me.wav".into()),
                    ..Default::default()
                };
                let (r, repaint) = run(&view, engine);
                assert_eq!(r, SpeechResponse::default(), "{engine:?}");
                assert!(!repaint, "idle Speech panel asked to repaint ({engine:?}, installed {installed})");
            }
        }
    }
}
