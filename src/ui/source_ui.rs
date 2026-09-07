//! ---- ws:source-monitor ----
//! The Source monitor (`Pane::Source`): the old library preview (`App.lib_preview`, deleted) promoted
//! to a dockable two-up with its own `Player`, in/out marks with ticks on the shared scrub bar, the
//! three-point-edit / smart-edit button row and Source Tape. Pure UI — `App`-side glue (opening a file,
//! the per-frame texture upload, focus routing, the edits themselves) lives in `ui::app::source_pane`
//! and `ui::app::source_ctl`; button clicks bubble up through `SourceResponse`, never mutate `App`.

use crate::hotkeys::Action;
use crate::media::thumbs::ThumbCache;
use crate::media::waveform::WaveformCache;
use crate::model::{Asset, ClipKind, Id, Project, MIN_CLIP};
use crate::playback::Player;
use crate::settings::Settings;
use crate::theme::Palette;
use crate::ui::heartbeat::Heartbeat;
use crate::ui::library::PreviewFrame;
use crate::ui::tools::{self, Dir, Glyph};
use crate::ui::{markers_ui, preview};
use eframe::egui;
use std::path::PathBuf;

/// Source Tape: a synthetic sequence of the (filtered) bin laid end to end, played by the same
/// `Player` instead of `SourceState.path`. `offsets[i]` = where `assets[i]` starts on the tape (the cut
/// ticks); `assets` are the PARENT project's asset ids in tape order, so a three-point edit taken while
/// the tape is on resolves "the clip under the tape playhead" back to a library asset.
pub struct Tape {
    pub project: Project,
    pub offsets: Vec<f64>,
    pub assets: Vec<Id>,
}

/// The Source monitor's state: a file, its own decoder/audio pipeline, the duration/fps a transport
/// needs (the asset's own probed values — this player's project is a synthetic single-clip one), the
/// in/out marks and, while on, the Source Tape. Replaces `LibPreview` 1:1 (same fields) plus marks/tape.
pub struct SourceState {
    pub player: Player,
    /// The library asset `path` belongs to; None for a Global/Recent file never imported.
    pub asset: Option<Id>,
    pub path: PathBuf,
    pub duration: f64,
    pub fps: f64,
    pub has_video: bool,
    /// A still image: no transport, no timecode, no scrub bar — just the picture.
    pub is_image: bool,
    pub src_in: Option<f64>,
    pub src_out: Option<f64>,
    pub tape: Option<Tape>,
    heartbeat: Heartbeat,
}

impl SourceState {
    pub fn new(player: Player, asset: &Asset, path: PathBuf, asset_id: Option<Id>) -> Self {
        Self {
            player,
            asset: asset_id,
            path,
            duration: asset.duration.max(MIN_CLIP),
            fps: if asset.fps > 0.0 { asset.fps } else { 30.0 },
            has_video: asset.has_video(),
            is_image: asset.kind == ClipKind::Image,
            src_in: None,
            src_out: None,
            tape: None,
            heartbeat: Heartbeat::default(),
        }
    }

    /// The marked source range, if any mark is set (an unset in = 0, an unset out = the end). None
    /// when nothing is marked — the same as "the whole clip" to every ranged op.
    pub fn marks(&self) -> Option<(f64, f64)> {
        if self.src_in.is_none() && self.src_out.is_none() {
            return None;
        }
        let a = self.src_in.unwrap_or(0.0);
        let b = self.src_out.unwrap_or(self.duration);
        (b > a + 1e-9).then_some((a, b))
    }

    /// What a three-point edit places: the library asset and its source range. Off tape that is
    /// `asset` + `marks()`; on tape it is the clip under the tape playhead (or under the in mark),
    /// with the marks intersected with that clip and converted to its source time.
    pub fn three_point(&self, project: &Project) -> Option<(Id, Option<(f64, f64)>)> {
        let Some(tape) = &self.tape else { return Some((self.asset?, self.marks())) };
        let t = self.src_in.unwrap_or_else(|| self.player.time());
        let (i, c) = tape
            .project
            .all_clips()
            .filter(|(_, c)| c.contains(t) || (c.end() - t).abs() < 1e-6)
            .map(|(_, c)| c)
            .find_map(|c| tape.offsets.iter().position(|&o| (o - c.start).abs() < 1e-6).map(|i| (i, c)))?;
        let asset = *tape.assets.get(i)?;
        let (a, b) = self.marks().unwrap_or((c.start, c.end()));
        let (a, b) = (a.max(c.start), b.min(c.end()));
        if b <= a + 1e-9 {
            return None;
        }
        let range = (c.src_time(a), c.src_time(b));
        let _ = project;
        Some((asset, Some(range)))
    }

    /// Swap the player onto a tape (`Some`) or back onto `path`'s own single-clip project (`None`).
    pub fn set_tape(&mut self, tape: Option<Tape>, own: &Project) {
        match &tape {
            Some(t) => {
                self.player.set_project(&t.project);
                self.duration = t.project.duration().max(MIN_CLIP);
                self.has_video = t.project.assets.iter().any(|a| a.has_video());
                self.is_image = false;
            }
            None => {
                self.player.set_project(own);
                self.duration = own.duration().max(MIN_CLIP);
            }
        }
        self.src_in = None;
        self.src_out = None;
        self.player.pause();
        self.player.seek(0.0);
        self.tape = tape;
    }
}

/// Borrow-split inputs to `show` (mirrors `draw_lib_preview`'s field destructure): everything the
/// pane reads or edits besides `SourceState` itself.
pub struct SourceCtx<'a> {
    pub palette: &'a Palette,
    pub settings: &'a mut Settings,
    pub thumbs: Option<&'a mut ThumbCache>,
    pub waveforms: Option<&'a mut WaveformCache>,
    /// This update's uploaded frame (see `App::source_frame`).
    pub frame: Option<PreviewFrame>,
    /// Space/JKL/I/O currently route here (last-clicked transport) — painted as an accent outline.
    pub focused: bool,
    /// Signed seconds from the record playhead to the nearest timeline cut, within the UI threshold
    /// (`source_ctl::smart_indicator`); None = no cut nearby.
    pub smart: Option<f64>,
}

#[derive(Default)]
pub struct SourceResponse {
    /// Edit verbs the buttons dispatch (Splice/Overwrite/Append/Ripple Overwrite/Close Up/Place on
    /// Top/Source Tape) — the same `Action`s the hotkeys and palette fire, so every verb has one path.
    pub actions: Vec<Action>,
    pub settings_changed: bool,
    pub close: bool,
    pub seek: Option<f64>,
    pub toggle_play: bool,
    pub stop: bool,
    /// I / O / clear buttons — applied by the caller regardless of which transport has focus.
    pub mark_in: bool,
    pub mark_out: bool,
    pub clear_marks: bool,
    /// "Subclip" button: `Project::subclip_from_marks` on the current marks.
    pub subclip: bool,
    /// A primary press landed inside the pane this frame — the caller moves transport focus here.
    pub clicked: bool,
    /// The Source/Record button: flip transport focus (Source <-> timeline) instead.
    pub toggle_focus: bool,
    /// The player is refilling its read-ahead: the caller schedules a poll repaint (`animate_until`).
    pub buffering: bool,
}

/// Frame-step target: one frame of `fps` either way, clamped to `[0, duration]`.
pub(crate) fn step_time(playhead: f64, fps: f64, forward: bool, duration: f64) -> f64 {
    let dt = 1.0 / fps.max(1.0);
    if forward {
        (playhead + dt).min(duration)
    } else {
        (playhead - dt).max(0.0)
    }
}

/// x of source time `t` on a scrub bar of `duration`.
pub(crate) fn mark_x(bar: egui::Rect, duration: f64, t: f64) -> f32 {
    bar.left() + bar.width() * (t / duration.max(f64::MIN_POSITIVE)).clamp(0.0, 1.0) as f32
}

/// In/out marks (accent triangles + a shaded band between them) and tape cut ticks over the bar.
fn paint_marks(p: &egui::Painter, bar: egui::Rect, duration: f64, st: &SourceState, palette: &Palette) {
    if let Some(tape) = &st.tape {
        for &o in tape.offsets.iter().skip(1) {
            let x = mark_x(bar, duration, o);
            p.line_segment([egui::pos2(x, bar.top()), egui::pos2(x, bar.bottom())], egui::Stroke::new(1.0, palette.text));
        }
    }
    if let Some((a, b)) = st.marks() {
        let band = egui::Rect::from_min_max(
            egui::pos2(mark_x(bar, duration, a), bar.top()),
            egui::pos2(mark_x(bar, duration, b), bar.bottom()),
        );
        p.rect_filled(band, 0.0, palette.selection.gamma_multiply(0.35));
    }
    for (t, dir) in [(st.src_in, 1.0f32), (st.src_out, -1.0)] {
        let Some(t) = t else { continue };
        let x = mark_x(bar, duration, t);
        let top = egui::pos2(x, bar.top() - 1.0);
        let tri = vec![top, egui::pos2(x + dir * 5.0, top.y), egui::pos2(x, top.y + 5.0)];
        p.add(egui::Shape::convex_polygon(tri, palette.accent, egui::Stroke::NONE));
        p.line_segment([egui::pos2(x, bar.top()), egui::pos2(x, bar.bottom())], egui::Stroke::new(1.5, palette.accent));
    }
}

/// Draw the Source monitor: transport, scrub bar with mark ticks, the video, and the edit button row.
/// Ports `draw_lib_preview`'s transport/scrub/video body; marks, smart edits and tape are additive.
pub fn show(ui: &mut egui::Ui, st: &mut SourceState, c: SourceCtx<'_>) -> SourceResponse {
    let mut resp = SourceResponse::default();
    let pane_rect = ui.max_rect();
    resp.clicked = ui.input(|i| i.pointer.primary_pressed()) && ui.rect_contains_pointer(pane_rect);
    let name = match &st.tape {
        Some(t) => format!("Source Tape ({} clips)", t.assets.len()),
        None => st.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
    };
    let path = st.path.to_string_lossy().into_owned();
    let duration = st.duration;
    let fps = st.fps;
    let playhead = st.player.time();
    let playing = st.player.is_playing();
    let has_video = st.has_video;
    let is_image = st.is_image;
    resp.buffering = st.player.is_buffering();
    let palette = c.palette;
    let tc = crate::ui::timecode;
    let SourceCtx { settings, thumbs, waveforms, frame, focused, smart, .. } = c;

    ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
        // edit row (bottom): three-point verbs + smart edits + indicator + tape/subclip
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 3.0;
            let mut b = |ui: &mut egui::Ui, icon: Glyph, hint: &str, a: Action| {
                if tools::glyph_text_button(ui, icon, "").on_hover_text(hint).clicked() {
                    resp.actions.push(a);
                }
            };
            b(ui, Glyph::Indent(true), "Splice (insert) the marked range at the playhead (Shift+V)", Action::SpliceInsert);
            b(ui, Glyph::Indent(false), "Overwrite at the playhead (B)", Action::OverwriteAtPlayhead);
            ui.separator();
            b(ui, Glyph::Append, "Append at End", Action::AppendAtEnd);
            b(ui, Glyph::Skip(Dir::Right), "Ripple Overwrite: replace the clip under the playhead, ripple the rest", Action::RippleOverwrite);
            b(ui, Glyph::CloseUp, "Close Up: close the gap under the playhead on its track", Action::CloseUp);
            b(ui, Glyph::PlaceOnTop, "Place on Top: a new track above everything", Action::PlaceOnTop);
            match smart {
                Some(d) => {
                    ui.weak(format!("cut {d:+.2} s")).on_hover_text("Nearest timeline cut to the playhead");
                }
                None => {
                    ui.weak("no cut near");
                }
            }
            ui.separator();
            let tape_on = st.tape.is_some();
            let r = tools::glyph_text_button(ui, Glyph::Tape, "")
                .on_hover_text(if tape_on { "Source Tape: back to the single clip" } else { "Source Tape: play the bin end to end" });
            if tape_on {
                ui.painter().rect_stroke(r.rect, 2.0, egui::Stroke::new(1.0, palette.accent), egui::StrokeKind::Inside);
            }
            if r.clicked() {
                resp.actions.push(Action::SourceTape);
            }
            if !tape_on && st.asset.is_some() && ui.small_button("Subclip").on_hover_text("New library subclip from the marks").clicked() {
                resp.subclip = true;
            }
        });
        if is_image {
            ui.horizontal(|ui| {
                if markers_ui::x_button(ui).on_hover_text("Close").clicked() {
                    resp.close = true;
                }
                ui.weak(name);
            });
        } else {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                let b = |ui: &mut egui::Ui, icon: Glyph, label: &str| -> bool {
                    tools::glyph_text_button(ui, icon, "").on_hover_text(label).clicked()
                };
                if b(ui, Glyph::Jump(Dir::Left), "Go to start") {
                    resp.seek = Some(0.0);
                }
                if b(ui, Glyph::Tri(Dir::Left), "Step back one frame") {
                    resp.seek = Some(step_time(playhead, fps, false, duration));
                }
                let (pp_icon, pp_label) = if playing { (Glyph::Pause, "Pause") } else { (Glyph::Play, "Play") };
                if b(ui, pp_icon, pp_label) {
                    resp.toggle_play = true;
                }
                if b(ui, Glyph::Stop, "Stop") {
                    resp.stop = true;
                }
                if b(ui, Glyph::Tri(Dir::Right), "Step forward one frame") {
                    resp.seek = Some(step_time(playhead, fps, true, duration));
                }
                if b(ui, Glyph::Jump(Dir::Right), "Go to end") {
                    resp.seek = Some(duration);
                }
                ui.add_space(6.0);
                ui.monospace(format!("{} / {}", tc(playhead, fps), tc(duration, fps)));
                ui.add_space(6.0);
                if b(ui, Glyph::Letter('I'), "Mark In (I)") {
                    resp.mark_in = true;
                }
                if b(ui, Glyph::Letter('O'), "Mark Out (O)") {
                    resp.mark_out = true;
                }
                let marks = format!(
                    "{} – {}",
                    st.src_in.map(|t| tc(t, fps)).unwrap_or_else(|| "·".into()),
                    st.src_out.map(|t| tc(t, fps)).unwrap_or_else(|| "·".into())
                );
                ui.weak(marks);
                if st.marks().is_some() && markers_ui::x_button(ui).on_hover_text("Clear marks (Alt+X)").clicked() {
                    resp.clear_marks = true;
                }
                ui.add_space(6.0);
                let cur = preview::QUALITIES.iter().copied().find(|&q| q == settings.preview_quality).unwrap_or(100);
                egui::ComboBox::from_id_salt("source_quality").selected_text(format!("{cur} %")).width(64.0).show_ui(
                    ui,
                    |ui| {
                        for q in preview::QUALITIES {
                            if ui.selectable_label(cur == q, format!("{q} %")).clicked() && q != cur {
                                settings.preview_quality = q;
                                resp.settings_changed = true;
                            }
                        }
                    },
                );
                ui.add_space(6.0);
                // Source/Record: which transport Space/JKL/I/O drive (the accent outline says Source)
                let hint = if focused {
                    "Source is live: Space/JKL/I/O drive this player. Click to hand them back to the timeline."
                } else {
                    "Timeline is live. Click to make Space/JKL/I/O drive this player."
                };
                if b(ui, Glyph::SourceRecord, hint) {
                    resp.toggle_focus = true;
                }
                if markers_ui::x_button(ui).on_hover_text("Close the source").clicked() {
                    resp.close = true;
                }
                ui.weak(name);
            });
            // scrub bar: the one shared implementation (preview.rs), with the marks painted over it
            let width = ui.available_width();
            let scope = ui.scope(|ui| preview::scrub_bar(ui, duration, playhead, palette, width));
            if let Some(t) = scope.inner {
                resp.seek = Some(t);
            }
            paint_marks(ui.painter(), scope.response.rect, duration, st, palette);
        }

        // video, filling whatever is left
        let (rect, r) = ui.allocate_exact_size(ui.available_size_before_wrap(), egui::Sense::click());
        ui.painter().rect_filled(rect, 0.0, egui::Color32::BLACK);
        let lb = match frame {
            Some(f) => {
                let aspect = f.size[0].max(1) as f32 / f.size[1].max(1) as f32;
                let lb = preview::letterbox(rect, aspect, ui.pixels_per_point());
                ui.painter().image(f.tex, lb, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
                lb
            }
            // no frame yet: the pane's whole rect stands in for the letterbox
            None => rect,
        };
        // set the canvas even before the first frame arrives: the player starts at 0x0 and renders
        // nothing until it is told a size, so gating this on `frame` would deadlock at a black box
        let full = ((lb.width() * ui.pixels_per_point()) as u32, (lb.height() * ui.pixels_per_point()) as u32);
        let (cw, ch) = preview_canvas(full, settings.preview_quality);
        st.player.set_canvas(cw.max(16), ch.max(16), settings.preview_max_width);
        if resp.buffering {
            ui.put(egui::Rect::from_center_size(lb.center(), egui::vec2(32.0, 32.0)), egui::Spinner::new().size(32.0));
        }
        if !has_video {
            // cover art (an audio file's embedded picture) wins over the visualizer when there is one
            let cover = thumbs.and_then(|t| t.texture(ui.ctx(), &path, 0.0, (rect.height() * ui.pixels_per_point()) as u32));
            if let Some((tex, size)) = cover {
                let aspect = size[0].max(1) as f32 / size[1].max(1) as f32;
                let lb = preview::letterbox(rect, aspect, ui.pixels_per_point());
                ui.painter().image(tex, lb, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
            } else if settings.audio_visualizer {
                let amplitude = waveforms
                    .and_then(|w| w.get(&path, 0))
                    .map(|p| crate::ui::heartbeat::amplitude_at(&[p], playhead, 1.0 / 30.0))
                    .unwrap_or(0.0);
                st.heartbeat.update(amplitude, ui.input(|i| i.stable_dt));
                st.heartbeat.paint(ui.painter(), rect, palette);
                ui.ctx().request_repaint();
            }
            r.context_menu(|ui| {
                let mut on = settings.audio_visualizer;
                if ui.checkbox(&mut on, "Audio visualizer").changed() {
                    settings.audio_visualizer = on;
                    resp.settings_changed = true;
                }
            });
        }
        if focused {
            ui.painter().rect_stroke(pane_rect, 0.0, egui::Stroke::new(1.0, palette.accent), egui::StrokeKind::Inside);
        }
    });
    resp
}

/// Render size for a pane of `canvas` px at `quality` percent (25..100), aspect kept. Same rule as
/// the program monitor's (`ui::app::preview_canvas`, private to that module).
fn preview_canvas(canvas: (u32, u32), quality: u32) -> (u32, u32) {
    if canvas.0 == 0 || canvas.1 == 0 {
        return (0, 0);
    }
    let q = quality.clamp(25, 100) as f32 / 100.0;
    (((canvas.0 as f32 * q) as u32).max(16), ((canvas.1 as f32 * q) as u32).max(16))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::text::TextRasterizer;
    use crate::media::Backend;
    use std::sync::{Arc, Mutex};

    fn asset(dur: f64) -> Asset {
        Asset {
            id: 0,
            path: "C:/x.mp4".into(),
            kind: ClipKind::Video,
            duration: dur,
            width: 320,
            height: 240,
            fps: 25.0,
            audio_streams: Vec::new(),
            codec: String::new(),
            folder: String::new(),
            tags: Vec::new(),
            label: 0,
            description: String::new(),
            rel_path: None,
            parent: None,
            range: None,
            effects: Vec::new(),
        }
    }

    fn state(ctx: &egui::Context) -> SourceState {
        let player = Player::new(ctx.clone(), Backend::Auto, Arc::new(Mutex::new(TextRasterizer::new())));
        SourceState::new(player, &asset(4.0), PathBuf::from("C:/x.mp4"), Some(1))
    }

    #[test]
    fn source_seek_math() {
        assert_eq!(step_time(1.0, 25.0, true, 4.0), 1.04, "forward steps by 1/fps");
        assert_eq!(step_time(0.02, 25.0, false, 4.0), 0.0, "backward step clamps at 0");
        assert_eq!(step_time(3.99, 25.0, true, 4.0), 4.0, "forward step clamps at the file's duration");
        let bar = egui::Rect::from_min_size(egui::pos2(10.0, 0.0), egui::vec2(100.0, 10.0));
        assert_eq!(mark_x(bar, 4.0, 2.0), 60.0);
        assert_eq!(mark_x(bar, 4.0, 9.0), 110.0, "clamped to the bar");
    }

    /// `marks()` defaults an unset in/out to the clip's ends and refuses an inverted pair.
    #[test]
    fn marks_default_and_refuse_inverted() {
        let ctx = egui::Context::default();
        let mut st = state(&ctx);
        assert_eq!(st.marks(), None);
        st.src_in = Some(1.0);
        assert_eq!(st.marks(), Some((1.0, 4.0)));
        st.src_out = Some(0.5);
        assert_eq!(st.marks(), None, "out before in is no range");
        st.src_in = None;
        assert_eq!(st.marks(), Some((0.0, 0.5)));
        assert_eq!(st.three_point(&Project::new()), Some((1, Some((0.0, 0.5)))));
    }

    /// Pane::Source open, paused, 30 headless frames with no input request no repaint (the idle-CPU-0%
    /// gate every new pane test carries, named after the crate's `assert_no_idle_repaint_*` convention).
    #[test]
    fn assert_no_idle_repaint_source_pane() {
        let ctx = egui::Context::default();
        let mut st = state(&ctx);
        let mut settings = Settings::default();
        let palette = crate::theme::palette_with(&ctx, &settings.palette);
        let ctx2 = egui::Context::default(); // the pane's own ctx: the Player's ctx is a different one
        for focused in [false, true] {
            for _ in 0..30 {
                let _ = ctx2.run(egui::RawInput::default(), |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let c = SourceCtx {
                            palette: &palette,
                            settings: &mut settings,
                            thumbs: None,
                            waveforms: None,
                            frame: None,
                            focused,
                            smart: Some(0.25),
                        };
                        let r = show(ui, &mut st, c);
                        assert!(r.actions.is_empty() && !r.close && r.seek.is_none());
                    });
                });
            }
            assert!(!ctx2.has_requested_repaint(), "idle source pane (focused={focused}) requested a repaint");
        }
    }
}
