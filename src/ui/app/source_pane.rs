//! ---- ws:source-monitor ----
//! `Pane::Source`'s PANE_DRAWERS entry and the App-side half of the Source monitor: opening a file
//! into its own `Player` (a 1:1 port of the deleted `start_lib_preview`/`lib_preview_frame`), the
//! once-per-update texture upload (FRAME_HOOKS: `tick`), transport-focus bookkeeping, and applying
//! `source_ui::show`'s response. Every edit verb goes through `source_ctl` (Actions) instead.

use super::*;
use crate::ui::source_ui::{self, SourceCtx, SourceState, Tape};

/// An open request queued for `tick`: a new `Player` needs the egui `Context`, which `App` never
/// stores, so callers without one (library click, Match Frame, the `source.*` MCP tools) park the
/// request here and the next frame's hook fulfils it.
pub(super) enum Pending {
    /// `id` is the resolved library asset (when known) — carried through so `source_open_now` can
    /// look it up by id instead of by path alone, which always finds the FIRST asset row with that
    /// path (the parent, when the real target is a subclip sharing the parent's path).
    File {
        path: PathBuf,
        seek: Option<f64>,
        id: Option<Id>,
    },
    Tape(Vec<Id>),
}

impl App {
    fn queue_source_open(&mut self, path: PathBuf, seek: Option<f64>, id: Option<Id>) {
        self.source_pending = Some(Pending::File { path, seek, id });
        self.source_focus = true;
        self.surface_source();
    }

    /// Load `path` into the Source monitor next frame (and seek to `seek` seconds once open). Takes
    /// transport focus, so Space/JKL/I/O drive it — the same "you just picked a clip to look at" rule
    /// the old library preview had — and surfaces the pane unless that would hide the Library. No
    /// known asset id (e.g. drag-and-drop from outside the project) — `open_asset_in_source` carries
    /// one when the caller has it.
    pub(crate) fn open_in_source(&mut self, path: PathBuf, seek: Option<f64>) {
        self.queue_source_open(path, seek, None);
    }

    /// Same, by library asset id. False = no such asset.
    pub(crate) fn open_asset_in_source(&mut self, asset: Id, seek: Option<f64>) -> bool {
        match self.project.asset(asset) {
            Some(a) => {
                self.queue_source_open(PathBuf::from(&a.path), seek, Some(asset));
                true
            }
            None => false,
        }
    }

    /// Space/JKL/I/O route to the Source monitor: it holds transport focus and has something open.
    pub(crate) fn source_active(&self) -> bool {
        self.source_focus && self.source.is_some()
    }

    pub(crate) fn close_source(&mut self) {
        self.source = None;
        self.source_tex = None;
        self.source_live = None;
        self.source_focus = false;
    }

    /// Reveal `Pane::Source` — except when it is tab-stacked in the Library's own group (the Simple /
    /// Fast-cut presets), where surfacing it would hide the very list the user is clicking in.
    /// ponytail: a same-tab-group check, not a general pin/auto-surface policy — layout-modes-
    /// onboarding's `reveal_auto` (pin-aware) is the upgrade path once it lands.
    fn surface_source(&mut self) {
        let tiles = &self.layout.tree.tiles;
        let same_group = match (tiles.find_pane(&Pane::Source), tiles.find_pane(&Pane::Library)) {
            (Some(s), Some(l)) => tiles.parent_of(s).is_some() && tiles.parent_of(s) == tiles.parent_of(l),
            _ => false,
        };
        if !same_group {
            self.surface(Pane::Source);
        }
    }

    /// Open `path` now (needs `ctx` for the Player). Re-opening the current file only seeks — unless
    /// `id` names a different asset than what's already open (a subclip shares its parent's `path`,
    /// so a path-only match would wrongly treat opening one after the other as "already open").
    /// Pauses the timeline: previewing a source and the program monitor should not both be making
    /// sound.
    fn source_open_now(&mut self, ctx: &egui::Context, path: PathBuf, seek: Option<f64>, id: Option<Id>) {
        self.player.pause();
        let same = self
            .source
            .as_ref()
            .is_some_and(|s| s.path == path && s.tape.is_none() && id.map_or(true, |i| s.asset == Some(i)));
        if same {
            if let (Some(t), Some(s)) = (seek, self.source.as_mut()) {
                s.player.seek(t.clamp(0.0, s.duration));
            }
            return;
        }
        // an asset the project already knows carries its probed duration; anything else (a Global/
        // Recent file never imported) is probed on the spot — one ffprobe call for metadata only.
        // `id` (when known) wins over the path match, which always finds the FIRST asset row with
        // that path — the parent, when the real target is a subclip (see `asset_for_source`).
        let path_s = path.to_string_lossy().into_owned();
        let (asset, id) = match self.project.asset_for_source(id, &path_s) {
            Some(a) => (a.clone(), Some(a.id)),
            None => match crate::media::probe(&path_s, self.backend()) {
                Ok(a) => (a, None),
                Err(_) => {
                    self.close_source();
                    return;
                }
            },
        };
        let mut player = Player::new(ctx.clone(), self.backend(), self.text.clone());
        player.set_project(&Project::from_media(asset.clone()));
        match seek {
            Some(t) => player.seek(t.clamp(0.0, asset.duration.max(0.0))),
            None => player.play(),
        }
        self.source = Some(SourceState::new(player, &asset, path, id));
        self.source_tex = None;
        self.source_live = None;
    }

    /// Build (or rebuild) the Source Tape from `ids` (empty = the library's current selection, else
    /// every media asset) and play it through the source player.
    fn source_tape_now(&mut self, ctx: &egui::Context, ids: Vec<Id>) {
        let ids = if ids.is_empty() { self.tape_default_ids() } else { ids };
        let Some(&first) = ids.first() else {
            self.toast("Source Tape: the library is empty");
            return;
        };
        if self.source.is_none() {
            let Some(path) = self.project.asset(first).map(|a| PathBuf::from(&a.path)) else { return };
            self.source_open_now(ctx, path, Some(0.0), Some(first));
        }
        let (project, offsets) = source_ctl::source_tape(&ids, &self.project);
        let own = self.source_own_project();
        if let Some(s) = self.source.as_mut() {
            s.set_tape(Some(Tape { project, offsets, assets: ids }), &own);
            s.player.play();
        }
    }

    /// Tape order when none is given: the library's multi-selection, else every media asset.
    /// ponytail: the library's search/kind filter isn't readable from here (its filter helpers are
    /// private to library.rs, media-library's file this wave) — the same-day library.rs follow-up
    /// exposes the filtered order; until then "selection, else everything" is the bin.
    fn tape_default_ids(&self) -> Vec<Id> {
        let sel: Vec<Id> =
            self.library.sel_ids.iter().copied().filter(|&id| self.project.asset(id).is_some()).collect();
        if !sel.is_empty() {
            return sel;
        }
        self.project.assets.iter().filter(|a| a.parent.is_none()).map(|a| a.id).collect()
    }

    /// The single-clip project the source player shows off tape.
    pub(super) fn source_own_project(&self) -> Project {
        let Some(s) = &self.source else { return Project::new() };
        let path = s.path.to_string_lossy().into_owned();
        match self.project.assets.iter().find(|a| a.path == path) {
            Some(a) => Project::from_media(a.clone()),
            None => crate::media::probe(&path, self.backend()).map(Project::from_media).unwrap_or_default(),
        }
    }

    /// The source's current frame, uploaded for the pane to paint. Called exactly once per update
    /// (`Player::take_frame` consumes the buffered frame, so a second call would come back empty).
    fn source_upload_frame(&mut self, ctx: &egui::Context) -> Option<library::PreviewFrame> {
        let s = self.source.as_mut()?;
        let playing = s.player.is_playing();
        if let Some(f) = s.player.take_frame() {
            let (w, h) = (f.width as usize, f.height as usize);
            if w > 0 && h > 0 && f.rgba.len() == w * h * 4 {
                let img = egui::ColorImage::from_rgba_premultiplied([w, h], &f.rgba);
                match self.source_tex.as_mut() {
                    Some(t) if t.size() == [w, h] => t.set_partial([0, 0], img, egui::TextureOptions::LINEAR),
                    Some(t) => t.set(img, egui::TextureOptions::LINEAR),
                    None => {
                        self.source_tex = Some(ctx.load_texture("source_monitor", img, egui::TextureOptions::LINEAR))
                    }
                }
            }
        }
        if playing {
            ctx.request_repaint();
        }
        let t = self.source_tex.as_ref()?;
        Some(library::PreviewFrame { tex: t.id(), size: [t.size()[0] as u32, t.size()[1] as u32], playing })
    }
}

/// A primary press landed inside this pane's rect this frame — the transport-focus rule's trigger
/// (last-clicked transport wins; a press on the Preview or Timeline pane hands focus back there).
pub(super) fn pressed_in(ui: &egui::Ui) -> bool {
    ui.input(|i| i.pointer.primary_pressed()) && ui.rect_contains_pointer(ui.max_rect())
}

/// FRAME_HOOKS entry: fulfil a queued open, upload this update's frame once, and keep the two players
/// from talking over each other (the timeline starting pauses the source).
pub(super) fn tick(app: &mut App, ctx: &egui::Context) {
    // `--screenshot` + SE_SCREENSHOT_SOURCE=1: open the project's first asset here with marks set, so
    // the pane can be eyeballed the way SE_SCREENSHOT_DELAY lets thumbnails warm up (verification only).
    if app.screenshot.is_some() && app.source.is_none() && std::env::var_os("SE_SCREENSHOT_SOURCE").is_some() {
        if let Some((path, id)) = app.project.assets.first().map(|a| (PathBuf::from(&a.path), a.id)) {
            app.source_open_now(ctx, path, Some(1.0), Some(id));
            if let Some(s) = app.source.as_mut() {
                (s.src_in, s.src_out) = (Some(0.5), Some(s.duration * 0.6));
            }
            app.source_focus = true;
            app.surface(Pane::Source);
        }
    }
    match app.source_pending.take() {
        Some(Pending::File { path, seek, id }) => app.source_open_now(ctx, path, seek, id),
        Some(Pending::Tape(ids)) => app.source_tape_now(ctx, ids),
        None => {}
    }
    if app.player.is_playing() {
        if let Some(s) = app.source.as_mut().filter(|s| s.player.is_playing()) {
            s.player.pause();
        }
    }
    app.source_live = app.source_upload_frame(ctx);
}

/// PANE_DRAWERS entry for `Pane::Source`.
pub(super) fn draw(app: &mut App, ui: &mut egui::Ui, pane: Pane) -> bool {
    if pane != Pane::Source {
        return false;
    }
    if app.source.is_none() {
        ui.weak("Click a clip in the Library to open it here — or press F on a timeline clip (Match Frame).");
        return true;
    }
    let smart = source_ctl::smart_indicator(
        &app.project,
        app.playhead,
        source_ctl::SMART_PX / app.timeline.zoom.max(1.0) as f64,
    );
    let focused = app.source_active();
    let resp = {
        let live = app.source_live;
        let App { source, settings, thumbs, waveforms, palette, .. } = app;
        let Some(st) = source.as_mut() else { return true };
        source_ui::show(
            ui,
            st,
            SourceCtx {
                palette,
                settings,
                thumbs: Some(thumbs),
                waveforms: Some(waveforms),
                frame: live,
                focused,
                smart,
            },
        )
    };
    if resp.toggle_focus {
        app.source_focus = !focused; // the button's own press also counts as `clicked`: the flip wins
    } else if resp.clicked {
        app.source_focus = true;
    }
    if resp.settings_changed {
        app.settings.save();
    }
    if resp.buffering {
        app.animate_until(ui.ctx(), Instant::now() + Duration::from_millis(50));
    }
    if let Some(st) = app.source.as_mut() {
        let duration = st.duration;
        if resp.toggle_play {
            app.player.pause();
            st.player.toggle();
        }
        if resp.stop {
            st.player.pause();
        }
        if let Some(t) = resp.seek {
            st.player.seek(t.clamp(0.0, duration));
        }
        let t = st.player.time();
        if resp.mark_in {
            source_ctl::mark_in(st, t);
        }
        if resp.mark_out {
            source_ctl::mark_out(st, t);
        }
        if resp.clear_marks {
            st.src_in = None;
            st.src_out = None;
        }
    }
    if resp.subclip {
        app.source_subclip(None);
    }
    app.pending_actions.extend(resp.actions);
    if resp.close {
        app.close_source();
    }
    true
}
