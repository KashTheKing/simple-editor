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

/// The full coalescing key `wants_new_request` compares: the bare `AltRequest` enum value alone isn't
/// enough — the same `AltRequest` can resolve to a different clip (selection/playhead moved off the old
/// target) or a different render time (playback, scrub) without the enum value itself changing, and both
/// must be treated as a fresh request rather than "already satisfied". `target_clip`'s resolved id plus a
/// frame-quantized playhead (`quantize_time`) captures that context.
type AltKey = (AltRequest, Option<Id>, i64);

/// Coalescing one-in-flight alt-render state behind `App.alt_render`.
pub(crate) struct AltRenderState {
    /// What should be showing right now (set by `preview.hover`, or a future UI hover). `None` = the
    /// live frame — `request(None)` also drops whatever is in flight or ready.
    request: Option<AltRequest>,
    /// The decode `tick` is waiting on: its `request_layers` id, the resolved key it was for, and the
    /// time it was decoded at (so the eventual GPU render uses the exact time its layers were decoded
    /// for).
    inflight: Option<(u64, AltKey, f64)>,
    /// The resolved key `ready`'s texture currently shows — lets an unchanged `request` skip a redundant
    /// re-decode every tick while a hover continues at the same target/time.
    shown: Option<AltKey>,
    /// The last frame successfully rendered, ready for `preview::show` to paint this frame.
    ready: Option<(egui::TextureId, [u32; 2])>,
    /// Own decoder, spun up lazily on first request — never the live `App.player`, so a hover preview
    /// can't steal frames from (or stall) playback.
    player: Option<Player>,
    /// Upload target, reused across requests (same sub-image-when-same-size convention as
    /// `PreviewState.texture`).
    texture: Option<egui::TextureHandle>,
    /// Set by `request(Some(...))`, consumed once per `tick` by `clear_if_stale` — see that fn's doc
    /// comment for why this exists (a hover-owning pane that stops being drawn otherwise leaves a
    /// `request` nothing ever clears).
    asserted: bool,
}

impl Default for AltRenderState {
    fn default() -> Self {
        Self { request: None, inflight: None, shown: None, ready: None, player: None, texture: None, asserted: false }
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
        if req.is_some() {
            self.asserted = true;
        }
        self.request = req;
    }
    /// Drop `request` if nothing re-asserted it (via `request(Some(...))`) since the last call to this.
    /// `tick` calls this once, at the top of every frame — BEFORE panes are drawn (see FRAME_HOOKS'
    /// ordering in `app/mod.rs`) — so the assertion it's checking for is the one made during the
    /// PREVIOUS frame's pane-draw phase, and it resets the flag so THIS frame's pane-draw phase can
    /// assert fresh for the next tick to check.
    ///
    /// A hover-owning pane's own match arm (`Pane::Transitions` in panes.rs, the Gallery pane's drawer in
    /// gallery_ctl.rs) is the only thing that calls `request(Some(...))`, and only while it is drawn —
    /// while the user keeps hovering the same card in a pane that's still on screen, that arm re-asserts
    /// every single frame, so this never fires. But if the user switches away from that pane entirely
    /// (its match arm no longer runs at all), nothing calls `request` any more — without this, the last
    /// request it made would otherwise go on being trusted, and painted over the live preview, forever.
    pub(crate) fn clear_if_stale(&mut self) {
        if self.request.is_some() && !std::mem::take(&mut self.asserted) {
            self.request = None;
        }
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
    app.alt_render.clear_if_stale();
    let Some(want) = app.alt_render.request.clone() else {
        app.alt_render.inflight = None;
        app.alt_render.shown = None;
        app.alt_render.ready = None;
        return;
    };
    let want_key = alt_key(&app.project, &app.selection, app.playhead, &want);
    if wants_new_request(&Some(want_key.clone()), &app.alt_render.inflight, &app.alt_render.shown) {
        start_request(app, ctx, want, want_key);
    }
    let reply = app.alt_render.player.as_ref().and_then(|p| p.take_layers_reply());
    let Some((rid, layers)) = reply else { return };
    let Some((key, t)) = accept_reply(&app.alt_render.inflight, rid) else { return };
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
    app.alt_render.shown = Some(key);
}

/// Whether a new decode should be started this tick: `want` is Some and differs from both what's
/// already in flight and what's already shown — the coalescing rule ("newest wins, no queue") in a form
/// that needs no live `App` to test. Compared as the full `AltKey` (request + resolved target clip +
/// quantized playhead), not the bare `AltRequest`, so a `shown` request whose target/time context has
/// moved on is correctly treated as stale rather than "already satisfied".
fn wants_new_request(want: &Option<AltKey>, inflight: &Option<(u64, AltKey, f64)>, shown: &Option<AltKey>) -> bool {
    let Some(w) = want else { return false };
    inflight.as_ref().map(|(_, k, _)| k) != Some(w) && shown.as_ref() != Some(w)
}

/// `take_layers_reply`'s id against what's in flight: `None` for a reply superseded by (or older than)
/// the current request — exactly `request_layers`' own "newest wins" contract, checked on this side too.
fn accept_reply(inflight: &Option<(u64, AltKey, f64)>, reply_id: u64) -> Option<(AltKey, f64)> {
    inflight.as_ref().filter(|(id, _, _)| *id == reply_id).map(|(_, k, t)| (k.clone(), *t))
}

/// `req`'s full coalescing key: the request itself, the clip it resolves against right now, and a
/// frame-quantized playhead. See `AltKey`'s doc comment for why the bare `AltRequest` isn't enough.
fn alt_key(project: &Project, selection: &[Id], playhead: f64, req: &AltRequest) -> AltKey {
    (req.clone(), target_clip(project, selection, playhead), quantize_time(playhead))
}

/// Round the playhead to whole milliseconds: coarse enough that re-reading the same paused playhead
/// twice still compares equal (no float-noise re-decodes), fine enough that real playback/scrub motion
/// always lands on a different key.
fn quantize_time(t: f64) -> i64 {
    (t * 1000.0).round() as i64
}

/// Resolves `req` via `alt_project`. On failure (nothing renderable — target clip gone, deselected,
/// playhead moved off it, or the `Gallery` stub) clears `state.inflight`, `state.shown`, AND
/// `state.ready`: clearing only `inflight` (the pre-fix behavior) leaves a stale texture from an earlier
/// successful decode of this same `AltRequest` painting over the live frame indefinitely. Takes no
/// `App`/`egui::Context` so the exact clearing behavior `start_request` relies on is unit-testable.
fn resolve_or_clear(
    project: &Project,
    selection: &[Id],
    playhead: f64,
    req: &AltRequest,
    state: &mut AltRenderState,
) -> Option<(Project, f64)> {
    let resolved = alt_project(project, selection, playhead, req);
    if resolved.is_none() {
        state.inflight = None;
        state.shown = None;
        state.ready = None;
    }
    resolved
}

fn start_request(app: &mut App, ctx: &egui::Context, req: AltRequest, key: AltKey) {
    let Some((clone, t)) = resolve_or_clear(&app.project, &app.selection, app.playhead, &req, &mut app.alt_render)
    else {
        return;
    };
    let max_w = app.canvas.0.max(16);
    let (backend, text) = (app.backend(), app.text.clone());
    let player = app.alt_render.player.get_or_insert_with(|| Player::new(ctx.clone(), backend, text));
    player.set_project(&clone);
    let id = player.request_layers(t, max_w);
    app.alt_render.inflight = Some((id, key, t));
}

/// The selected visual clip a hover request previews against (today's rule — a future consumer with no
/// selection to lean on can widen this).
fn target_clip(project: &Project, selection: &[Id], playhead: f64) -> Option<Id> {
    selection
        .iter()
        .copied()
        .find(|&id| project.clip(id).is_some_and(|cl| cl.is_visual() && cl.enabled && cl.contains(playhead)))
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
pub(crate) fn reframe(
    project: &mut Project,
    tracking: &TrackState,
    selection: &[Id],
    want: Option<Id>,
) -> Result<(), &'static str> {
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
        assert!(
            (c.start..c.end()).contains(&t),
            "render time {t} falls inside the clip's own span {:?}",
            c.start..c.end()
        );
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
        let key_a: AltKey = (a.clone(), Some(1u64), 0i64);
        let key_b: AltKey = (b.clone(), Some(1u64), 0i64);
        assert!(!wants_new_request(&None, &None, &None), "no request, nothing to start");
        assert!(
            wants_new_request(&Some(key_a.clone()), &None, &None),
            "a fresh request with nothing in flight or shown"
        );
        assert!(!wants_new_request(&Some(key_a.clone()), &Some((1, key_a.clone(), 0.0)), &None), "already in flight");
        assert!(
            !wants_new_request(&Some(key_a.clone()), &None, &Some(key_a.clone())),
            "already shown — no redundant re-decode"
        );
        assert!(
            wants_new_request(&Some(key_b.clone()), &Some((1, key_a.clone(), 0.0)), &None),
            "a different request supersedes it"
        );
        let key_a_other_clip: AltKey = (a.clone(), Some(2u64), 0i64);
        assert!(
            wants_new_request(&Some(key_a.clone()), &None, &Some(key_a_other_clip)),
            "same request, different resolved target clip is stale, not already-satisfied"
        );
        let key_a_later: AltKey = (a.clone(), Some(1u64), 500i64);
        assert!(
            wants_new_request(&Some(key_a_later), &None, &Some(key_a)),
            "same request/target, different quantized playhead is stale, not already-satisfied"
        );
    }

    #[test]
    fn accept_reply_discards_a_mismatched_id() {
        let key: AltKey = (AltRequest::Effect(EffectKind::Blur), Some(1u64), 0i64);
        let inflight = Some((2u64, key.clone(), 1.5));
        assert_eq!(accept_reply(&inflight, 2), Some((key, 1.5)));
        assert_eq!(accept_reply(&inflight, 1), None, "a stale (superseded) request's reply is discarded");
        assert_eq!(accept_reply(&None, 2), None);
    }

    #[test]
    fn resolve_or_clear_wipes_shown_and_ready_when_the_target_becomes_unrenderable() {
        let (p, id) = project_with_clip();
        let req = AltRequest::Effect(EffectKind::Blur);
        let mut state = AltRenderState::default();
        // Simulate an earlier successful decode: `shown`/`ready` populated, nothing in flight.
        state.shown = Some(alt_key(&p, &[id], 1.0, &req));
        state.ready = Some((egui::TextureId::Managed(1), [4, 4]));
        // The playhead moves off the clip: `alt_project` can no longer resolve a target for the SAME
        // `AltRequest` — this is the bug scenario (selection/playhead/deletion made a shown request
        // unrenderable).
        assert!(resolve_or_clear(&p, &[id], 10.0, &req, &mut state).is_none());
        assert!(state.shown.is_none(), "a stale shown request must not keep being treated as satisfied");
        assert!(state.ready.is_none(), "a stale texture must not keep painting over the live frame");
        assert!(state.inflight.is_none());
    }

    #[test]
    fn a_shown_requests_key_goes_stale_when_the_playhead_advances_under_it() {
        let (p, id) = project_with_clip();
        let req = AltRequest::Effect(EffectKind::Blur);
        // Shown at t=1.0 (already decoded and painting).
        let shown = Some(alt_key(&p, &[id], 1.0, &req));
        // Same `AltRequest`, but the playhead has since moved to t=2.0 while the hover stays active.
        let want = Some(alt_key(&p, &[id], 2.0, &req));
        assert!(
            wants_new_request(&want, &None, &shown),
            "the widened key must catch a playhead change under an unchanged AltRequest instead of \
             treating the hover as already-satisfied (frozen preview during playback)"
        );
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

    /// A request that nothing re-asserts across a frame boundary must go stale and clear — the scenario
    /// a hover-owning pane produces when the user switches away from it while a hover was still active:
    /// its match arm in panes.rs/gallery_ctl.rs simply doesn't run any more, so nothing ever calls
    /// `request(Some(...))` again. Distinct from `wants_new_request_only_starts_once_per_distinct_request`
    /// above, which only covers one request being superseded by a DIFFERENT one — never "nobody called
    /// request at all this frame".
    #[test]
    fn clear_if_stale_drops_a_request_nothing_reasserted() {
        let mut state = AltRenderState::default();
        // Frame N: a pane hovers a transition and asserts the request (mirrors panes.rs's
        // `Pane::Transitions` arm calling `app.alt_render.request(Some(...))` during its pane-draw phase).
        state.request(Some(AltRequest::Transition(TransitionKind::CrossFade)));
        // Frame N+1's tick runs BEFORE panes are drawn — it sees frame N's assertion, so the request
        // survives this check.
        state.clear_if_stale();
        assert!(state.request.is_some(), "an assertion from the previous frame must survive the next tick");
        // The user switches away from the pane: frame N+1's pane-draw phase never calls `request(...)`
        // for it at all (its match arm doesn't run any more) — nothing reasserts.
        // Frame N+2's tick must now treat the stale leftover request exactly like `request(None)`.
        state.clear_if_stale();
        assert!(state.request.is_none(), "a request nobody reasserted since the last tick must be cleared");
    }

    /// The normal case — the pane stays on screen and the mouse stays on the same card, so its match arm
    /// calls `request(Some(...))` again every single frame — must never go stale.
    #[test]
    fn clear_if_stale_keeps_a_request_thats_reasserted_every_frame() {
        let mut state = AltRenderState::default();
        for _ in 0..5 {
            state.request(Some(AltRequest::Transition(TransitionKind::CrossFade)));
            state.clear_if_stale();
            assert!(state.request.is_some(), "reasserted every frame — must never go stale");
        }
    }
}
