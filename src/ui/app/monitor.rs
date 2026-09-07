//! ---- ws:canvas-handles-monitor ----
//! The monitor's async alt-render pipeline: `AltRenderState` (behind `App.alt_render`, retyped from
//! wave-0b's `AltRenderKind` placeholder — see the plan's risk table) coalesces a hover request
//! (`preview.hover`, or a future UI hover — none exists yet, see `AltRequest::Gallery`'s doc comment)
//! into at most one in-flight decode: `tick` (FRAME_HOOK) clones the project, temporarily applies the
//! requested effect/transition, decodes it on its OWN `Player` (never the live one — same isolation
//! `LibPreview` uses for the library pane) via `Player::request_layers`/`take_layers_reply`
//! (player-rate-loop's async API), GPU-renders the reply and uploads it as a normal egui texture. The
//! clone is never written back: a hover preview can never mutate `App.project` or push undo. Alt renders
//! never start while `App.export` is running (export owns the GPU/decode capacity).
//!
//! Also `act` (ACT_HANDLERS): AutoReframe, ToggleProxies, ViewerFit.
//!
//! deviation (see PR body): the plan describes `gpu.render_preview_texture`'s zero-copy native GL
//! texture for the alt-render output, registered via `eframe::Frame::register_native_glow_texture`.
//! FRAME_HOOKS' signature is `fn(&mut App, &egui::Context)` — no `&mut eframe::Frame` — and widening it
//! would touch every other workstream's already-merged `tick` fn, outside this workstream's owned
//! files. `tick` instead reads back the GPU-rendered frame to CPU (`App::gpu_frame` — already the
//! established GPU-accurate fallback path, not the forbidden blocking `Player::render_once`) and
//! uploads it the same way `PreviewState`'s own CPU-frame path does (`ctx.load_texture`). One extra
//! copy per hover-preview frame; the live preview's own zero-copy path is untouched.
//!
//! deviation (see PR body): `App::enabled`/`enabled_for`/`enabled_for2` (app/mod.rs) are pinned to a
//! 3-/4-bool pure-function shape another workstream's test already depends on, and this file's
//! app/mod.rs edits are restricted to the registry lines only — so AutoReframe's "no tracked box" guard
//! lives entirely in `act`/`reframe` below (a toast, not an `App::enabled` Err) rather than the central
//! `enabled()` dispatch the plan's test row names. `ui.action`'s generic disabled-toast still doesn't
//! apply to it, but the hotkey is unbound (menu/palette/MCP only) and every one of those paths goes
//! through `act`, so the toast still always fires.
//!
//! deviation (see PR body): every test below that the plan's table describes against a live `&mut App`
//! (`viewer_fit_resets_view`, `toggle_proxies_flips_setting`, `auto_reframe_with_tracked_box_writes_
//! apply_path`, the alt-render coalescing/pause/no-mutate tests) is rewritten against the pure logic
//! `act`/`tick` actually dispatch to — this crate has no headless `App`-construction path anywhere
//! (`eframe::CreationContext` has no public constructor outside `eframe::run_native`; see
//! tools_registry_tests.rs's own "deviation" comment and drops.rs's `drop_target` doc comment for the
//! same, pre-existing constraint every other workstream's ACT_HANDLERS tests already work around).

use super::*;
use crate::playback::Player;
use crate::ui::tracking_ui::TrackState;

/// What the monitor should render instead of the live frame.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum AltRequest {
    Effect(EffectKind),
    Transition(TransitionKind),
    /// Reserved for inspector-gallery's wave-2 `gallery.hover` tool (a catalogue tab + item name): that
    /// workstream should target this same `App.alt_render` pipeline instead of inventing a second hover
    /// mechanism. No catalogue data model exists yet this wave, so `tick` treats it as a no-op stub.
    Gallery(String, String),
}

/// Coalescing one-in-flight alt-render state behind `App.alt_render`.
pub(crate) struct AltRenderState {
    /// What should be showing right now (set by `preview.hover`, or a future UI hover). `None` = the
    /// live frame — `request(None)` also drops whatever is in flight or ready.
    request: Option<AltRequest>,
    /// The decode `tick` is waiting on: its `request_layers` id, the request it was for, and the time it
    /// was decoded at (so the eventual GPU render uses the exact time its layers were decoded for).
    inflight: Option<(u64, AltRequest, f64)>,
    /// The request `ready`'s texture currently shows — lets an unchanged `request` skip a redundant
    /// re-decode every tick while a hover continues.
    shown: Option<AltRequest>,
    /// The last frame successfully rendered, ready for `preview::show` to paint this frame.
    ready: Option<(egui::TextureId, [u32; 2])>,
    /// Own decoder, spun up lazily on first request — never the live `App.player`, so a hover preview
    /// can't steal frames from (or stall) playback.
    player: Option<Player>,
    /// Upload target, reused across requests (same sub-image-when-same-size convention as
    /// `PreviewState.texture`).
    texture: Option<egui::TextureHandle>,
}

impl Default for AltRenderState {
    fn default() -> Self {
        Self { request: None, inflight: None, shown: None, ready: None, player: None, texture: None }
    }
}

impl AltRenderState {
    /// The texture `preview::show` should paint instead of the live frame this frame, if any.
    pub(crate) fn texture(&self) -> Option<(egui::TextureId, [u32; 2])> {
        self.ready
    }
    /// Ask the monitor to substitute `req`'s render for the live frame (`None` = back to the live
    /// frame). Newest wins: a second call before the first resolves simply retargets the same slot —
    /// `tick` never queues more than one decode.
    pub(crate) fn request(&mut self, req: Option<AltRequest>) {
        self.request = req;
    }
}

/// `(1.0, ZERO)` — the fitted letterbox, no zoom or pan. Shared by `ViewerFit` and the `preview.view`
/// tool's `fit: true` arg so both reset the exact same way.
pub(crate) fn fit_view() -> (f32, egui::Vec2) {
    (1.0, egui::Vec2::ZERO)
}

/// FRAME_HOOK: services `App.alt_render`. See this file's doc comment for the full pipeline and the
/// zero-copy-texture deviation.
pub(crate) fn tick(app: &mut App, ctx: &egui::Context) {
    if app.export.is_some() {
        return; // never render while exporting — export owns the GPU/decode capacity
    }
    let want = app.alt_render.request.clone();
    if want.is_none() {
        app.alt_render.inflight = None;
        app.alt_render.shown = None;
        app.alt_render.ready = None;
        return;
    }
    if wants_new_request(&want, &app.alt_render.inflight, &app.alt_render.shown) {
        start_request(app, ctx, want.expect("checked Some above"));
    }
    let reply = app.alt_render.player.as_ref().and_then(|p| p.take_layers_reply());
    let Some((rid, layers)) = reply else { return };
    let Some((req, t)) = accept_reply(&app.alt_render.inflight, rid) else { return };
    app.alt_render.inflight = None; // consumed either way — a failed render below just leaves `ready` be
    let (w, h) = app.canvas;
    let Some(frame) = app.gpu_frame(&layers, t, w, h) else { return };
    let (fw, fh) = (frame.width as usize, frame.height as usize);
    if fw == 0 || fh == 0 || frame.rgba.len() != fw * fh * 4 {
        return;
    }
    let img = egui::ColorImage::from_rgba_premultiplied([fw, fh], &frame.rgba);
    let tex = match app.alt_render.texture.take() {
        Some(mut t) if t.size() == [fw, fh] => {
            t.set_partial([0, 0], img, egui::TextureOptions::LINEAR);
            t
        }
        _ => ctx.load_texture("alt_render", img, egui::TextureOptions::LINEAR),
    };
    app.alt_render.ready = Some((tex.id(), [frame.width, frame.height]));
    app.alt_render.texture = Some(tex);
    app.alt_render.shown = Some(req);
}

/// Whether a new decode should be started this tick: `want` is Some and differs from both what's
/// already in flight and what's already shown — the coalescing rule ("newest wins, no queue") in a form
/// that needs no live `App` to test.
fn wants_new_request(
    want: &Option<AltRequest>,
    inflight: &Option<(u64, AltRequest, f64)>,
    shown: &Option<AltRequest>,
) -> bool {
    let Some(w) = want else { return false };
    inflight.as_ref().map(|(_, r, _)| r) != Some(w) && shown.as_ref() != Some(w)
}

/// `take_layers_reply`'s id against what's in flight: `None` for a reply superseded by (or older than)
/// the current request — exactly `request_layers`' own "newest wins" contract, checked on this side too.
fn accept_reply(inflight: &Option<(u64, AltRequest, f64)>, reply_id: u64) -> Option<(AltRequest, f64)> {
    inflight.as_ref().filter(|(id, _, _)| *id == reply_id).map(|(_, r, t)| (r.clone(), *t))
}

fn start_request(app: &mut App, ctx: &egui::Context, req: AltRequest) {
    let Some((clone, t)) = alt_project(&app.project, &app.selection, app.playhead, &req) else {
        app.alt_render.inflight = None;
        return;
    };
    let max_w = app.canvas.0.max(16);
    let (backend, text) = (app.backend(), app.text.clone());
    let player = app.alt_render.player.get_or_insert_with(|| Player::new(ctx.clone(), backend, text));
    player.set_project(&clone);
    let id = player.request_layers(t, max_w);
    app.alt_render.inflight = Some((id, req, t));
}

/// The selected visual clip a hover request previews against (today's rule — a future consumer with no
/// selection to lean on can widen this).
fn target_clip(project: &Project, selection: &[Id], playhead: f64) -> Option<Id> {
    selection.iter().copied().find(|&id| project.clip(id).is_some_and(|cl| cl.is_visual() && cl.enabled && cl.contains(playhead)))
}

/// The hypothetical project + render time a hover request previews: a clone with the requested
/// effect/transition applied to the target clip, never written back to `project` — proving this takes
/// `&Project` (not `&mut`), a hover request structurally cannot mutate the live project or push undo.
/// `None` when there is nothing to preview against (no selected visual clip, or the `Gallery` stub).
fn alt_project(project: &Project, selection: &[Id], playhead: f64, req: &AltRequest) -> Option<(Project, f64)> {
    let id = target_clip(project, selection, playhead)?;
    let mut p = project.clone();
    match req {
        AltRequest::Effect(kind) => {
            p.clip_mut(id)?.effects.push(Effect::new(*kind));
            Some((p, playhead))
        }
        AltRequest::Transition(kind) => {
            let dur = 1.0;
            p.add_edge_transition(id, *kind, dur, true)?;
            let (start, end) = {
                let c = p.clip(id)?;
                (c.start, c.end())
            };
            Some((p, (end - dur / 2.0).max(start)))
        }
        // ws:canvas-handles-monitor: reserved for inspector-gallery's gallery.hover — no catalogue data
        // model exists yet this wave, so there is nothing renderable to build here.
        AltRequest::Gallery(_, _) => None,
    }
}

/// Auto Reframe moves the CLIP opposite the tracked point/box so the point stays put in frame — the
/// inverse of the Tracking pane's own "Apply to clip", which makes the clip follow the point.
pub(crate) fn invert_points(points: &[(f32, f32, f32)]) -> Vec<(f32, f32, f32)> {
    points.iter().map(|&(x, y, t)| (-x, -y, t)).collect()
}

/// Auto-reframe `want` (or, when `None`, whichever clip the Tracking pane targets — see
/// `TrackState::tracked`) using its EXISTING tracked box only: no motion/saliency fallback exists
/// in-tree, so absence is an error, never a silently invented heuristic. Pure mutation only — no undo
/// push, no toast: `act` (unbound hotkey/menu/palette) and `timeline.reframe` (MCP, `ToolKind::Mutate`,
/// auto-wrapped by the generic snapshot-before/push-undo-iff-changed handler) each wrap this their own
/// way, so it isn't done twice.
pub(crate) fn reframe(project: &mut Project, tracking: &TrackState, selection: &[Id], want: Option<Id>) -> Result<(), &'static str> {
    let (id, points) = tracking
        .tracked(project, selection)
        .ok_or("Auto Reframe needs a tracked point or box first — track one in the Tracking pane")?;
    if want.is_some_and(|w| w != id) {
        return Err("the tracked box belongs to a different clip");
    }
    let inverted = invert_points(points);
    if project.apply_path(id, &inverted) {
        Ok(())
    } else {
        Err("the track is too short to reframe from")
    }
}

/// ACT_HANDLERS entry: AutoReframe, ToggleProxies, ViewerFit. Every other `Action` falls through
/// (`false`) to the next handler / the big match in `actions.rs`.
pub(crate) fn act(app: &mut App, a: Action) -> bool {
    match a {
        Action::AutoReframe => {
            let snap = app.project.to_json();
            match reframe(&mut app.project, &app.tracking, &app.selection, None) {
                Ok(()) => {
                    push_undo_json(&mut app.undo, &mut app.redo, snap);
                    app.after_edit();
                }
                Err(reason) => app.toast(reason),
            }
            true
        }
        Action::ToggleProxies => {
            app.settings.use_proxies = !app.settings.use_proxies;
            app.settings.save();
            true
        }
        Action::ViewerFit => {
            app.preview.view = fit_view();
            true
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Asset, Clip, ClipKind};

    fn project_with_clip() -> (Project, Id) {
        let mut p = Project::new();
        let a = p.add_asset(Asset {
            id: 0,
            path: "C:/x.mp4".into(),
            kind: ClipKind::Video,
            duration: 4.0,
            width: 1280,
            height: 720,
            fps: 30.0,
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
        });
        let mut c = Clip::new(7, ClipKind::Video, "v", 0.0, 4.0);
        c.asset = a;
        p.tracks[0].clips.push(c);
        (p, 7)
    }

    #[test]
    fn target_clip_prefers_the_selected_visual_clip_at_the_playhead() {
        let (p, id) = project_with_clip();
        assert_eq!(target_clip(&p, &[id], 1.0), Some(id));
        assert_eq!(target_clip(&p, &[], 1.0), None, "no selection, nothing to target");
        assert_eq!(target_clip(&p, &[id], 10.0), None, "playhead outside the clip");
        assert_eq!(target_clip(&p, &[999], 1.0), None, "selected id doesn't exist");
    }

    #[test]
    fn alt_project_effect_clones_without_touching_the_original() {
        let (p, id) = project_with_clip();
        let (clone, t) = alt_project(&p, &[id], 1.0, &AltRequest::Effect(EffectKind::Blur)).expect("a target exists");
        assert_eq!(t, 1.0, "effect preview renders at the current playhead");
        assert_eq!(clone.clip(id).unwrap().effects.len(), 1, "the clone gained the hovered effect");
        assert_eq!(clone.clip(id).unwrap().effects[0].kind, EffectKind::Blur);
        assert!(p.clip(id).unwrap().effects.is_empty(), "the original project is never mutated");
    }

    #[test]
    fn alt_project_transition_lands_inside_the_clips_own_span() {
        let (p, id) = project_with_clip();
        let (clone, t) =
            alt_project(&p, &[id], 1.0, &AltRequest::Transition(TransitionKind::CrossFade)).expect("a target exists");
        let c = clone.clip(id).unwrap();
        assert!((c.start..c.end()).contains(&t), "render time {t} falls inside the clip's own span {:?}", c.start..c.end());
        assert_eq!(p.tracks[0].transitions.len(), 0, "the original project gained no transition");
    }

    #[test]
    fn alt_project_gallery_is_a_reserved_stub() {
        let (p, id) = project_with_clip();
        assert!(alt_project(&p, &[id], 1.0, &AltRequest::Gallery("effects".into(), "Blur".into())).is_none());
    }

    #[test]
    fn wants_new_request_only_starts_once_per_distinct_request() {
        let a = AltRequest::Effect(EffectKind::Blur);
        let b = AltRequest::Effect(EffectKind::Pixelate);
        assert!(!wants_new_request(&None, &None, &None), "no request, nothing to start");
        assert!(wants_new_request(&Some(a.clone()), &None, &None), "a fresh request with nothing in flight or shown");
        assert!(!wants_new_request(&Some(a.clone()), &Some((1, a.clone(), 0.0)), &None), "already in flight");
        assert!(!wants_new_request(&Some(a.clone()), &None, &Some(a.clone())), "already shown — no redundant re-decode");
        assert!(wants_new_request(&Some(b.clone()), &Some((1, a.clone(), 0.0)), &None), "a different request supersedes it");
    }

    #[test]
    fn accept_reply_discards_a_mismatched_id() {
        let req = AltRequest::Effect(EffectKind::Blur);
        let inflight = Some((2u64, req.clone(), 1.5));
        assert_eq!(accept_reply(&inflight, 2), Some((req, 1.5)));
        assert_eq!(accept_reply(&inflight, 1), None, "a stale (superseded) request's reply is discarded");
        assert_eq!(accept_reply(&None, 2), None);
    }

    #[test]
    fn invert_points_negates_xy_keeps_time() {
        let out = invert_points(&[(3.0, -4.0, 0.0), (-1.5, 2.0, 1.0)]);
        assert_eq!(out, vec![(-3.0, 4.0, 0.0), (1.5, -2.0, 1.0)]);
    }

    #[test]
    fn reframe_without_a_tracked_box_is_an_error() {
        let (mut p, id) = project_with_clip();
        let tracking = TrackState::default();
        assert!(reframe(&mut p, &tracking, &[id], None).is_err());
        assert!(p.clip(id).unwrap().x.keys.is_empty(), "no keyframes written on the error path");
    }

    #[test]
    fn reframe_with_tracked_box_writes_an_inverted_apply_path() {
        let (mut p, id) = project_with_clip();
        let tracking = TrackState::with_points(vec![(10.0, 20.0, 0.0), (30.0, 0.0, 2.0)]);
        assert!(reframe(&mut p, &tracking, &[id], None).is_ok());
        let c = p.clip(id).unwrap();
        assert!(c.x.keys.len() >= 2, "apply_path wrote keyframes");
        assert!(c.x.at(0.0) < 0.0, "the clip moves opposite the tracked point (inverted sign): {}", c.x.at(0.0));
    }

    #[test]
    fn reframe_rejects_a_tracked_box_that_belongs_to_a_different_clip() {
        let (mut p, id) = project_with_clip();
        let tracking = TrackState::with_points(vec![(0.0, 0.0, 0.0), (1.0, 1.0, 1.0)]);
        assert!(reframe(&mut p, &tracking, &[id], Some(id + 1)).is_err());
    }

    #[test]
    fn fit_view_is_no_zoom_no_pan() {
        assert_eq!(fit_view(), (1.0, egui::Vec2::ZERO));
    }
}
