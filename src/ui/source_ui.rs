//! ---- ws:source-monitor ----
//! The Source monitor (`Pane::Source`): the old library preview (`App.lib_preview`, deleted) promoted
//! to a dockable two-up with its own `Player`, in/out marks on the shared bar's seek line, the
//! program monitor's bar (`media_bar`) plus Insert / Overwrite, and - on the right-click - the marks, the
//! smart edits, Source Tape and Subclip. Pure UI - `App`-side glue (opening a file,
//! the per-frame texture upload, focus routing, the edits themselves) lives in `ui::app::source_pane`
//! and `ui::app::source_ctl`; button clicks bubble up through `SourceResponse`, never mutate `App`.

use crate::hotkeys::Action;
use crate::media::thumbs::ThumbCache;
use crate::media::waveform::WaveformCache;
use crate::model::{Asset, ClipKind, Id, Project, MIN_CLIP};
use crate::playback::Player;
use crate::settings::Settings;
use crate::theme::Palette;
use crate::ui::library::PreviewFrame;
use crate::ui::spiky_ball::Orb;
use crate::ui::tools::{self, Glyph};
use crate::ui::{media_bar, menu, preview};
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
/// needs (the asset's own probed values - this player's project is a synthetic single-clip one), the
/// in/out marks and, while on, the Source Tape. Replaces `LibPreview` 1:1 (same fields) plus marks/tape.
pub struct SourceState {
    pub player: Player,
    /// The library asset `path` belongs to; None for a Global/Recent file never imported.
    pub asset: Option<Id>,
    pub path: PathBuf,
    pub duration: f64,
    pub fps: f64,
    pub has_video: bool,
    /// A still image: no transport, no timecode, no scrub bar - just the picture.
    pub is_image: bool,
    pub src_in: Option<f64>,
    pub src_out: Option<f64>,
    pub tape: Option<Tape>,
    ball: Orb,
    /// (style name, shader source) the visualizer draws: re-read only when the Settings pick changes.
    viz: Option<(String, String)>,
    /// The timecode's text while it is being typed (`preview::timecode_label`).
    tc_edit: Option<String>,
    /// Viewer zoom (1 = fit) and pan, like `PreviewState.view`; `fit_px` = the fitted picture's width.
    pub(crate) view: (f32, egui::Vec2),
    fit_px: f32,
    /// The media's own width in px, for the Zoom ▾ percentages.
    width: u32,
    // ---- ws:play-source-monitor ----
    /// What `player` is showing (the file's own single-clip project, or the tape's): the GPU path
    /// renders the player's layers against it on the UI thread.
    pub project: Project,
    /// The canvas last handed to `player.set_canvas` (before its `max_width` clamp) - the size the
    /// GPU path renders the player's layers at.
    pub(crate) canvas: (u32, u32),
    // ---- ws:preview-audio-only ----
    /// Right-click ▸ Audio Only: the file's sound with the visualizer, no picture decoded.
    pub audio_only: bool,
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
            ball: Orb::default(),
            viz: None,
            tc_edit: None,
            view: (1.0, egui::Vec2::ZERO),
            fit_px: 0.0,
            width: asset.width,
            project: Project::from_media(asset.clone()),
            canvas: (0, 0),
            audio_only: false,
        }
    }

    /// Audio Only on / off: the player swaps to the same project without (or with) its picture.
    pub fn set_audio_only(&mut self, on: bool) {
        self.audio_only = on;
        self.project.audio_only_preview = on;
        self.player.set_project(&self.project);
    }

    /// The marked source range, if any mark is set (an unset in = 0, an unset out = the end). None
    /// when nothing is marked - the same as "the whole clip" to every ranged op.
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
                self.project = t.project.clone();
                self.project.audio_only_preview = self.audio_only;
                self.player.set_project(&self.project);
                self.duration = t.project.duration().max(MIN_CLIP);
                self.has_video = t.project.assets.iter().any(|a| a.has_video());
                self.is_image = false;
            }
            None => {
                self.project = own.clone();
                self.project.audio_only_preview = self.audio_only;
                self.player.set_project(&self.project);
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
    /// Space/JKL/I/O currently route here (last-clicked transport) - painted as an accent outline.
    pub focused: bool,
    /// Signed seconds from the record playhead to the nearest timeline cut, within the UI threshold
    /// (`source_ctl::smart_indicator`); None = no cut nearby.
    pub smart: Option<f64>,
}

#[derive(Default)]
pub struct SourceResponse {
    /// Edit verbs the buttons / right-click dispatch (Splice/Overwrite/Append/Ripple Overwrite/Close
    /// Up/Place on Top/Source Tape) - the same `Action`s the hotkeys and palette fire: one path each.
    pub actions: Vec<Action>,
    pub settings_changed: bool,
    pub close: bool,
    pub seek: Option<f64>,
    pub toggle_play: bool,
    pub stop: bool,
    /// Right-click Mark In / Out / Clear - applied by the caller regardless of which transport has focus.
    pub mark_in: bool,
    pub mark_out: bool,
    pub clear_marks: bool,
    /// Right-click "New Subclip from Marks": `Project::subclip_from_marks` on the current marks.
    pub subclip: bool,
    /// A press (any button) landed inside the pane this frame - the caller moves transport focus here.
    pub clicked: bool,
    /// The player is refilling its read-ahead: the caller schedules a poll repaint (`animate_until`).
    pub buffering: bool,
    /// The bar's speed menu (unsigned), Reverse and Loop.
    pub rate: Option<f64>,
    pub reverse: bool,
    pub toggle_loop: bool,
}

/// The Source Tape's previous / next cut from `t` (its clip starts, and the end).
fn tape_cut(st: &SourceState, t: f64, next: bool) -> f64 {
    let mut cuts: Vec<f64> = st.tape.as_ref().map(|tp| tp.offsets.clone()).unwrap_or_default();
    cuts.push(st.duration);
    if next {
        cuts.into_iter().find(|&c| c > t + 1e-6).unwrap_or(st.duration)
    } else {
        cuts.into_iter().rev().find(|&c| c < t - 1e-6).unwrap_or(0.0)
    }
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

/// Draw the Source monitor: the video (its file name in the corner) over the media player's bar
/// (`media_bar`, the program monitor's too) with the marks on its seek bar and Insert, Overwrite and
/// Zoom ▾ on the right. Marks, the smart edits, Source Tape, Subclip, playback resolution and Close are on the video's
/// right-click. Ports `draw_lib_preview`'s transport/scrub/video body.
pub fn show(ui: &mut egui::Ui, st: &mut SourceState, c: SourceCtx<'_>) -> SourceResponse {
    let mut resp = SourceResponse::default();
    let pane_rect = ui.max_rect();
    // any press, a right-click too: the monitor clicked last drives Space/JKL/I/O
    resp.clicked = ui.input(|i| i.pointer.any_pressed()) && ui.rect_contains_pointer(pane_rect);
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
    let (fit_px, width) = (st.fit_px, st.width);
    let SourceCtx { settings, thumbs, waveforms, frame, focused, smart, .. } = c;

    let mut thumbs = thumbs;
    ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
        if !is_image {
            // the media player's bar (`media_bar`): the plain player controls for the file; the Source
            // Tape (a run of clips) also gets the editor's frame steps, cuts and marks
            let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), media_bar::BAR_H), egui::Sense::hover());
            let edit = st.tape.is_some();
            let cuts = st.tape.as_ref().map(|t| t.offsets.iter().skip(1).copied().collect()).unwrap_or_default();
            let ctx = ui.ctx().clone();
            let (project, rate, looping) = (&st.project, st.player.rate(), st.player.loop_range().is_some());
            let mut thumb = |t: f64| {
                let (path, src) = media_bar::thumb_source(project, t)?;
                thumbs.as_deref_mut()?.texture(&ctx, &path, src, 90)
            };
            let mut edit_btn = None;
            let (out, _) = media_bar::show(
                ui,
                palette,
                rect,
                media_bar::Bar {
                    t: playhead,
                    dur: duration,
                    fps,
                    playing,
                    rate,
                    looping,
                    edit,
                    marks: (st.src_in, st.src_out),
                    markers: Vec::new(),
                    cuts,
                    fullscreen: false,
                    fullscreen_button: false,
                    tc_edit: Some(&mut st.tc_edit),
                    thumb: &mut thumb,
                },
                |_| {},
                |ui| {
                    for (a, g) in [
                        (Action::SpliceInsert, Glyph::Indent(true)),
                        (Action::OverwriteAtPlayhead, Glyph::Indent(false)),
                    ] {
                        if tools::glyph_text_button(ui, g, "").on_hover_text(preview::tip(a)).clicked() {
                            edit_btn = Some(a);
                        }
                    }
                    if has_video {
                        preview::zoom_button(ui, &mut st.view, fit_px, width);
                    }
                },
            );
            resp.actions.extend(edit_btn);
            for a in out.actions {
                match a {
                    Action::GoStart => resp.seek = Some(0.0),
                    Action::GoEnd => resp.seek = Some(duration),
                    Action::StepBack => resp.seek = Some(step_time(playhead, fps, false, duration)),
                    Action::StepForward => resp.seek = Some(step_time(playhead, fps, true, duration)),
                    Action::PrevCut | Action::NextCut => resp.seek = Some(tape_cut(st, playhead, a == Action::NextCut)),
                    Action::PlayPause => resp.toggle_play = true,
                    Action::MarkIn => resp.mark_in = true,
                    Action::MarkOut => resp.mark_out = true,
                    _ => {}
                }
            }
            if let Some(s) = out.skip {
                resp.seek = Some((playhead + s).clamp(0.0, duration));
            }
            resp.seek = out.seek.or(resp.seek);
            resp.rate = out.rate;
            resp.reverse = out.reverse;
            resp.toggle_loop = out.toggle_loop;
        }

        // video, filling whatever is left
        let (rect, r) = ui.allocate_exact_size(ui.available_size_before_wrap(), egui::Sense::click_and_drag());
        ui.painter().rect_filled(rect, 0.0, egui::Color32::BLACK);
        preview::zoom_pan(ui, &r, &mut st.view);
        let lb = match frame {
            Some(f) => {
                let aspect = f.size[0].max(1) as f32 / f.size[1].max(1) as f32;
                let lb = preview::letterbox(rect, aspect, ui.pixels_per_point());
                st.fit_px = lb.width() * ui.pixels_per_point();
                // zoomed, the same texture is magnified (the render size stays the fitted one)
                ui.painter_at(rect).image(
                    f.tex,
                    preview::apply_view(lb, st.view),
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
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
        st.canvas = (cw.max(16), ch.max(16));
        if resp.buffering {
            ui.put(egui::Rect::from_center_size(lb.center(), egui::vec2(32.0, 32.0)), egui::Spinner::new().size(32.0));
        }
        if !has_video || st.audio_only {
            // cover art (an audio file's embedded picture) wins over the visualizer when there is one
            // (not in Audio Only: a video's "cover" would be its own first frame)
            let cover = thumbs
                .filter(|_| !st.audio_only)
                .and_then(|t| t.texture(ui.ctx(), &path, 0.0, (rect.height() * ui.pixels_per_point()) as u32));
            let cover =
                if st.audio_only { None } else { cover.or_else(|| crate::ui::audio_card::cover(ui.ctx(), &path)) };
            if cover.is_some() || !settings.audio_visualizer {
                // ws:audio-visualizer: the "now playing" card - cover (or the music-note icon) + title
                let title = crate::ui::audio_card::title(ui.ctx(), &path);
                crate::ui::audio_card::paint(ui, rect, cover, &title);
            } else if settings.audio_visualizer {
                // the song's own analysis at the playhead's source time (the same one a visualizer clip's keys
                // are baked from), built once in the background; repaint only while playing / until it lands
                let backend = crate::media::Backend::parse(&settings.decoder);
                let ctx = ui.ctx().clone();
                let song = crate::engine::visualizer::analysis_async(&path, 0, backend, move || ctx.request_repaint());
                st.ball.show(song.as_deref(), playhead);
                if playing {
                    ui.ctx().request_repaint();
                }
                let want = &settings.audio_visualizer_style;
                if st.viz.as_ref().is_none_or(|(n, _)| n != want) {
                    let name = (!want.is_empty()).then_some(want.as_str());
                    let src = crate::engine::visualizer::find(name).or_else(|_| crate::engine::visualizer::find(None));
                    st.viz = Some((want.clone(), src.map(|s| s.src).unwrap_or_default()));
                }
                if let Some((_, src)) = &st.viz {
                    st.ball.paint(ui.painter(), rect, src);
                }
            }
        }
        // what is open: the file name in the corner, over a dim plate
        let font = egui::TextStyle::Small.resolve(ui.style());
        let galley = ui.painter().layout_no_wrap(name, font, palette.text);
        let plate = egui::Rect::from_min_size(rect.min + egui::vec2(6.0, 6.0), galley.size() + egui::vec2(12.0, 6.0));
        ui.painter().rect_filled(plate, 3.0, palette.panel.gamma_multiply(0.8));
        ui.painter().galley(plate.min + egui::vec2(6.0, 3.0), galley, palette.text);
        menu::context(&r, |ui| {
            if !is_image {
                for (a, hit) in [
                    (Action::MarkIn, &mut resp.mark_in),
                    (Action::MarkOut, &mut resp.mark_out),
                    (Action::ClearInOut, &mut resp.clear_marks),
                ] {
                    *hit |= menu::item(ui, a, true);
                }
                ui.separator();
                // the smart edits place the marked range at the RECORD playhead: say where that is
                let near = match smart {
                    Some(d) => format!("Nearest timeline cut: {d:+.2} s"),
                    None => "No timeline cut near the playhead".into(),
                };
                ui.add_enabled_ui(false, |ui| menu::row(ui, None, &near, ""));
                for a in [Action::AppendAtEnd, Action::RippleOverwrite, Action::CloseUp, Action::PlaceOnTop] {
                    if menu::item(ui, a, true) {
                        resp.actions.push(a);
                    }
                }
                ui.separator();
                let tape_on = st.tape.is_some();
                let tape = menu::check(ui, tape_on, "Source Tape", &menu::shortcut(Action::SourceTape));
                if tape.on_hover_text("Play the bin end to end").clicked() {
                    resp.actions.push(Action::SourceTape);
                }
                resp.subclip |= menu::item(ui, Action::NewSubclip, !tape_on && st.asset.is_some());
                menu::sub(ui, None, "Playback Resolution", |ui| {
                    for q in preview::QUALITIES {
                        if menu::check(ui, settings.preview_quality == q, &format!("{q} %"), "").clicked() {
                            settings.preview_quality = q;
                            resp.settings_changed = true;
                        }
                    }
                });
            }
            // ws:preview-audio-only: a video's sound with the visualizer, no picture
            if has_video && !is_image {
                if menu::check(ui, st.audio_only, "Audio Only", "")
                    .on_hover_text("Play the sound with a visualizer, no picture")
                    .clicked()
                {
                    let on = !st.audio_only;
                    st.set_audio_only(on);
                }
            }
            if !has_video || st.audio_only {
                // ws:audio-visualizer: Off, or one of the styles (built-ins + custom library shaders)
                menu::sub(ui, None, "Audio Visualizer", |ui| {
                    if menu::check(ui, !settings.audio_visualizer, "Off", "").clicked() {
                        settings.audio_visualizer = false;
                        resp.settings_changed = true;
                    }
                    let current = st.viz.as_ref().map(|(n, _)| n.clone()).unwrap_or_default();
                    for (i, s) in crate::engine::visualizer::styles().into_iter().enumerate() {
                        let on = settings.audio_visualizer && (s.name == current || (current.is_empty() && i == 0));
                        if menu::check(ui, on, &s.name, "").clicked() {
                            settings.audio_visualizer = true;
                            settings.audio_visualizer_style = if i == 0 { String::new() } else { s.name };
                            resp.settings_changed = true;
                        }
                    }
                });
            }
            if has_video {
                menu::sub(ui, Some(Glyph::Zoom), "Zoom", |ui| preview::zoom_rows(ui, &mut st.view, fit_px, width));
            }
            ui.separator();
            resp.close |= menu::row(ui, Some(Glyph::Cross), "Close Source", "").clicked();
        });
        if focused {
            ui.painter().rect_stroke(pane_rect, 0.0, egui::Stroke::new(1.0, palette.accent), egui::StrokeKind::Inside);
        }
    });
    resp
}

/// Nothing open: the Preview's empty canvas - black around a project-aspect letterbox filled with the
/// project's background (`preview_bg`; the checkerboard in the renderer's 16 px tiles and greys) - with
/// a hint over it, so the monitor never reads as an empty panel.
pub fn blank(ui: &mut egui::Ui, aspect: f32, bg: crate::model::BackgroundMode, palette: &Palette) {
    use crate::model::BackgroundMode as B;
    let (rect, _) = ui.allocate_exact_size(ui.available_size_before_wrap(), egui::Sense::hover());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 0.0, egui::Color32::BLACK);
    let lb = preview::letterbox(rect, aspect, ui.pixels_per_point());
    let grey = |v: u8| egui::Color32::from_gray(v);
    p.rect_filled(
        lb,
        0.0,
        match bg {
            B::Checkerboard => grey(153),
            B::Black => egui::Color32::BLACK,
            B::White => egui::Color32::WHITE,
            B::Custom([r, g, b, _]) => egui::Color32::from_rgb(r, g, b),
        },
    );
    if bg == B::Checkerboard {
        let tile = 16.0 / ui.pixels_per_point();
        let p = p.with_clip_rect(lb);
        let (cols, rows) = ((lb.width() / tile).ceil() as i32, (lb.height() / tile).ceil() as i32);
        for (x, y) in (0..rows).flat_map(|y| (0..cols).map(move |x| (x, y))).filter(|(x, y)| (x + y) % 2 == 1) {
            let min = lb.min + egui::vec2(x as f32 * tile, y as f32 * tile);
            p.rect_filled(egui::Rect::from_min_size(min, egui::vec2(tile, tile)), 0.0, grey(204));
        }
    }
    let hint = "Click a clip in the Library to open it here - or press F on a timeline clip (Match Frame).";
    let galley = p.layout(hint.into(), egui::TextStyle::Body.resolve(ui.style()), palette.text, lb.width() - 24.0);
    let plate = egui::Rect::from_center_size(lb.center(), galley.size() + egui::vec2(16.0, 10.0));
    p.rect_filled(plate, 4.0, palette.panel.gamma_multiply(0.85));
    p.galley(plate.min + egui::vec2(8.0, 5.0), galley, palette.text);
}

// Render size for a pane of `canvas` px at `quality` percent (25..100), aspect kept. Same rule as
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

    // ---- ws:preview-audio-only ----
    /// Right-click ▸ Audio Only: the player's project loses its picture for playback (the render thread's
    /// view), keeps the file's sound, and the choice survives switching the tape on and off.
    #[test]
    fn audio_only_plays_the_sound_without_the_picture() {
        let path = crate::media::ffpipe::tests::test_mp4();
        let asset = crate::media::probe(&path, Backend::Auto).unwrap();
        let player = Player::new(egui::Context::default(), Backend::Auto, Arc::new(Mutex::new(TextRasterizer::new())));
        let mut st = SourceState::new(player, &asset, PathBuf::from(&path), None);
        st.set_audio_only(true);
        let view = crate::playback::played_view(&st.player.project());
        let n = |k: crate::model::TrackKind| {
            view.tracks.iter().filter(|t| t.kind == k).map(|t| t.clips.len()).sum::<usize>()
        };
        assert_eq!(n(crate::model::TrackKind::Video), 0, "no picture decoded");
        assert!(n(crate::model::TrackKind::Audio) > 0, "the sound plays");
        st.set_tape(None, &Project::from_media(asset.clone()));
        assert!(st.audio_only && st.player.project().audio_only_preview, "kept across a tape switch");
        st.set_audio_only(false);
        assert!(!st.player.project().audio_only_preview);
    }

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
            html: None,
            proxy: None,
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
        ctx.set_fonts(crate::theme::test_fonts());
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
        ctx.set_fonts(crate::theme::test_fonts());
        let mut audio = asset(4.0);
        audio.kind = ClipKind::Audio;
        audio.width = 0;
        audio.height = 0;
        let audio_st = SourceState::new(ctx_player(&ctx), &audio, PathBuf::from("C:/x.wav"), Some(2));
        assert!(!audio_st.has_video, "the audio state draws the orb");
        for mut st in [state(&ctx), audio_st] {
            idle_frames(&mut st);
        }
    }

    /// The Source monitor zooms like the Preview (the shared `preview::zoom_pan`): Ctrl+wheel / pinch
    /// over the picture changes `view`, and its transport carries the same Zoom ▾ caption.
    #[test]
    fn source_zooms_like_the_preview() {
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::theme::test_fonts());
        let mut st = state(&ctx);
        let mut settings = Settings::default();
        let palette = crate::theme::palette_with(&egui::Context::default(), &settings.palette);
        let ctx2 = egui::Context::default();
        ctx2.set_fonts(crate::theme::test_fonts());
        let mut labels = String::new();
        for events in [vec![egui::Event::PointerMoved(egui::pos2(300.0, 200.0))], vec![egui::Event::Zoom(1.5)]] {
            let out = ctx2.run(egui::RawInput { events, ..Default::default() }, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let c = SourceCtx {
                        palette: &palette,
                        settings: &mut settings,
                        thumbs: None,
                        waveforms: None,
                        frame: None,
                        focused: false,
                        smart: None,
                    };
                    show(ui, &mut st, c);
                });
            });
            labels = format!("{:?}", out.shapes);
        }
        assert!((st.view.0 - 1.5).abs() < 1e-4, "Ctrl+wheel zooms the Source: {:?}", st.view);
        assert!(labels.contains("▾"), "the transport has the Zoom ▾ button");
    }

    fn ctx_player(ctx: &egui::Context) -> Player {
        Player::new(ctx.clone(), Backend::Auto, Arc::new(Mutex::new(TextRasterizer::new())))
    }

    fn idle_frames(st: &mut SourceState) {
        let mut settings = Settings::default();
        let palette = crate::theme::palette_with(&egui::Context::default(), &settings.palette);
        let ctx2 = egui::Context::default();
        ctx2.set_fonts(crate::theme::test_fonts()); // the pane's own ctx: the Player's ctx is a different one
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
                        let r = show(ui, st, c);
                        assert!(r.actions.is_empty() && !r.close && r.seek.is_none());
                    });
                });
            }
            assert!(!ctx2.has_requested_repaint(), "idle source pane (focused={focused}) requested a repaint");
        }
    }
}
