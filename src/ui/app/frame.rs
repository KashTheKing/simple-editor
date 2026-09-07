//! ---- ws:layout-modes-onboarding ----
//! FRAME_HOOKS entry: once per frame, diff the selection (clips, transitions, subtitle cues and
//! snap-engine's `TimelineState.edit_point` — its real landed name) into a `SelectionKind`, react to a
//! CHANGE exactly once (auto-surface in Dynamic mode, tab glow in Granular / when pinned, the adaptive
//! tool strip's lead tool), and decay the tab glow list through `App::animate_until` — the one
//! sanctioned timed-repaint funnel, so a fully-decayed glow schedules nothing and the idle-CPU gate
//! holds.
//!
//! The `selection_changed` Luau hook is NOT fired here: command-palette's `palette_ctl::tick` already
//! fires it once per change of the widened selection signature — `SelSig`, covering clips, transitions,
//! subtitle cues AND the edit point, not just `App.selection` (see its `last_fired_selection`, typed
//! `SelSig` for exactly this reason) — and a second call site would double-fire every `@on
//! selection_changed` script. `selection_changed_hook_has_exactly_one_call_site` below pins that.

use super::*;
use crate::ui::layout::GLOW_SECS;
use crate::ui::timeline::EditPoint;

/// What the selection looked like when `tick` last reacted (a change in ANY part fires once).
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct SelSig {
    clips: Vec<Id>,
    transitions: Vec<Id>,
    cues: Vec<Id>,
    edit_point: Option<EditPoint>,
}

impl SelSig {
    fn matches(&self, app: &App) -> bool {
        self.clips == app.selection
            && self.transitions == app.sel_transitions
            && self.cues == app.timeline.sub_sel
            && self.edit_point == app.timeline.edit_point
    }
    /// `pub(super)`: `palette_ctl::tick` builds one of these each frame to diff against
    /// `App.last_fired_selection` so `selection_changed` fires on a transition/cue/edit-point-only
    /// change too, not just a change to `App.selection`.
    pub(super) fn of(app: &App) -> Self {
        Self {
            clips: app.selection.clone(),
            transitions: app.sel_transitions.clone(),
            cues: app.timeline.sub_sel.clone(),
            edit_point: app.timeline.edit_point,
        }
    }
}

/// `App::selection_kind` widened with the two selections wave-0b's version calls placeholders: a
/// subtitle-lane cue selection and snap-engine's edit point (both only when no clip/transition is
/// selected, which is how the timeline itself treats them).
pub(super) fn selection_kind_of(app: &App) -> SelectionKind {
    if app.selection.is_empty() && app.sel_transitions.is_empty() {
        if !app.timeline.sub_sel.is_empty() {
            return SelectionKind::Cue;
        }
        if app.timeline.edit_point.is_some() {
            return SelectionKind::EditPoint;
        }
    }
    app.selection_kind()
}

pub(super) fn tick(app: &mut App, ctx: &egui::Context) {
    if !app.sel_sig.matches(app) {
        app.sel_sig = SelSig::of(app);
        let kind = selection_kind_of(app);
        let dynamic = layout_ctl::is_dynamic(&app.settings);
        app.tools.lead = if dynamic { layout_ctl::lead_tool(kind) } else { None };
        layout_ctl::surface_for_kind(app, kind);
    }
    if let Some(at) = decay_glow(&mut app.layout.glow, Instant::now()) {
        app.animate_until(ctx, at);
    }
}

/// Drop expired glow entries; while any remain, the next wake (a ~25 fps fade — a glow is a cue, not
/// an animation worth 60 Hz). `None` = nothing glowing, nothing scheduled.
pub(super) fn decay_glow(glow: &mut Vec<(Pane, Instant)>, now: Instant) -> Option<Instant> {
    glow.retain(|&(_, at)| now.duration_since(at).as_secs_f32() < GLOW_SECS);
    (!glow.is_empty()).then(|| now + Duration::from_millis(40))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::layout::Surfaced;

    fn in_front(l: &Layout, pane: Pane) -> bool {
        let id = l.tree.tiles.find_pane(&pane).unwrap();
        match l.tree.tiles.parent_of(id).and_then(|p| l.tree.tiles.get_container(p)) {
            Some(egui_tiles::Container::Tabs(tabs)) => tabs.active == Some(id),
            _ => true,
        }
    }

    /// The same selection change: Dynamic switches the tab; Granular pushes a glow entry and leaves
    /// the active tab alone; a pinned sibling makes Dynamic glow too.
    #[test]
    fn auto_surface_only_in_dynamic_mode() {
        let now = Instant::now();
        // Granular: Subtitles (behind Mixer in the default layout) glows, Mixer stays in front
        let mut l = Layout::default_layout();
        assert!(in_front(&l, Pane::Mixer));
        assert_eq!(layout_ctl::react(&mut l, false, Pane::Subtitles, now), Surfaced::Pinned);
        assert!(in_front(&l, Pane::Mixer), "Granular must never switch tabs");
        assert_eq!(l.glow.iter().map(|(p, _)| *p).collect::<Vec<_>>(), vec![Pane::Subtitles]);
        // Dynamic: the same change switches, no glow
        let mut l = Layout::default_layout();
        assert_eq!(layout_ctl::react(&mut l, true, Pane::Subtitles, now), Surfaced::Shown);
        assert!(in_front(&l, Pane::Subtitles));
        assert!(l.glow.is_empty());
        // Dynamic with the active sibling pinned: glow instead of switching
        let mut l = Layout::default_layout();
        l.set_pinned(Pane::Mixer, true);
        assert_eq!(layout_ctl::react(&mut l, true, Pane::Subtitles, now), Surfaced::Pinned);
        assert!(in_front(&l, Pane::Mixer));
        assert_eq!(l.glow.len(), 1);
        // a hidden / absent pane is reported and glows nothing in either mode
        let mut l = Layout::default_layout();
        l.toggle(Pane::Curves);
        assert_eq!(layout_ctl::react(&mut l, true, Pane::Curves, now), Surfaced::Hidden);
        assert_eq!(layout_ctl::react(&mut l, false, Pane::Curves, now), Surfaced::Hidden);
        assert!(l.glow.is_empty());
        // re-glowing a pane refreshes its entry rather than duplicating it
        let mut l = Layout::default_layout();
        layout_ctl::react(&mut l, false, Pane::Subtitles, now);
        layout_ctl::react(&mut l, false, Pane::Subtitles, now + Duration::from_millis(300));
        assert_eq!(l.glow.len(), 1);
        assert_eq!(l.glow[0].1, now + Duration::from_millis(300));
    }

    /// assert_no_idle_repaint harness: a live glow schedules a wake through the funnel; once every
    /// entry's age exceeds `GLOW_SECS` the list is empty, nothing is scheduled, and an idle frame
    /// requests no repaint.
    #[test]
    fn glow_decays_and_stops_repainting() {
        let now = Instant::now();
        let mut glow = vec![(Pane::Mixer, now - Duration::from_millis(200))];
        let at = decay_glow(&mut glow, now).expect("a live glow schedules a wake");
        assert!(at > now && at <= now + Duration::from_millis(40));
        assert_eq!(glow.len(), 1);
        // the same funnel App::animate_until uses: a wake in the future is request_repaint_after
        let ctx = egui::Context::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            if let Some(dt) = at.checked_duration_since(Instant::now()) {
                ctx.request_repaint_after(dt);
            }
        });
        assert!(ctx.has_requested_repaint(), "a live glow must keep the fade going");

        // expired: dropped, nothing scheduled, and 30 idle frames request nothing
        let mut glow =
            vec![(Pane::Mixer, now - Duration::from_secs(2)), (Pane::Inspector, now - Duration::from_secs(3))];
        let ctx = egui::Context::default();
        for _ in 0..30 {
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                if let Some(at) = decay_glow(&mut glow, Instant::now()) {
                    ctx.request_repaint_after(at.saturating_duration_since(Instant::now()));
                }
                egui::CentralPanel::default().show(ctx, |_| {});
            });
        }
        assert!(glow.is_empty());
        assert!(!ctx.has_requested_repaint(), "a decayed glow must not keep requesting repaints");
        assert!(decay_glow(&mut Vec::new(), now).is_none());
    }

    /// Deviation from the issue's `selection_changed_hook_fires_once_per_change`: the event is fired by
    /// command-palette's `palette_ctl::tick` (one call site, `last_fired_selection` diff), so this
    /// workstream must NOT add a second one — pinned by counting call sites in the two files.
    #[test]
    fn selection_changed_hook_has_exactly_one_call_site() {
        let here = include_str!("frame.rs");
        let palette = include_str!("palette_ctl.rs");
        let needle = "fire_hook(\"selection_changed\"";
        assert_eq!(palette.matches(needle).count(), 1, "palette_ctl::tick owns the selection_changed call site");
        assert_eq!(here.matches(needle).count(), 0, "frame.rs must not fire selection_changed a second time");
        assert!(palette.contains("last_fired_selection"), "…and it diffs, so the hook fires once per change");
    }

    #[test]
    fn selection_signature_changes_on_every_selection_part() {
        let a = SelSig { clips: vec![1], ..Default::default() };
        let b = SelSig { clips: vec![1], transitions: vec![2], ..Default::default() };
        let c = SelSig { clips: vec![1], cues: vec![3], ..Default::default() };
        let d = SelSig {
            clips: vec![1],
            edit_point: Some(EditPoint { track: 0, t: 1.0, side: crate::ui::timeline::Side::Both }),
            ..Default::default()
        };
        assert_ne!(a, b);
        assert_ne!(a, c);
        assert_ne!(a, d);
        assert_eq!(a, a.clone());
    }

    /// `palette_ctl::tick` fires `selection_changed` by comparing `SelSig::of(app)` against
    /// `App.last_fired_selection` each frame (no headless `App` to call `tick` itself — see this
    /// module's and `fire_hook`'s doc comments). This exercises that exact predicate for the three
    /// selection kinds the bug report named: selecting a transition, a subtitle cue, or moving the
    /// edit point, all with the clip selection held constant, must each look like a change (so the
    /// hook fires); settling on that same state afterward must look unchanged (so it fires exactly
    /// once, not every frame).
    #[test]
    fn selection_changed_predicate_fires_once_for_each_widened_kind() {
        let clip_only = SelSig { clips: vec![1], ..Default::default() };

        let transition = SelSig { clips: vec![1], transitions: vec![9], ..Default::default() };
        assert_ne!(transition, clip_only, "transition-only change (clip selection unchanged) must fire");
        assert_eq!(transition, transition.clone(), "the same state again must not re-fire");

        let cue = SelSig { clips: vec![1], cues: vec![3], ..Default::default() };
        assert_ne!(cue, clip_only, "subtitle-cue-only change (clip selection unchanged) must fire");
        assert_eq!(cue, cue.clone(), "the same state again must not re-fire");

        let edit_point = SelSig {
            clips: vec![1],
            edit_point: Some(EditPoint { track: 0, t: 2.0, side: crate::ui::timeline::Side::Both }),
            ..Default::default()
        };
        assert_ne!(edit_point, clip_only, "edit-point-only change (clip selection unchanged) must fire");
        assert_eq!(edit_point, edit_point.clone(), "the same state again must not re-fire");
    }
}
