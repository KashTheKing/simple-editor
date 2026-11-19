//! ---- ws:inspector-gallery ----
//! 7 MCP tools for capabilities this workstream actually adds: gallery.list/apply/hover (the Gallery
//! pane's tool surface - the canonical resolution for Looks, via `find_look`/`apply_look_by_name` below;
//! color-engine's `looks.list`/`looks.apply` cover the same ground for a bare clip_id/name call, and are
//! now thin wrappers around these same two fns - see tools_color.rs - so a Look name resolves/applies
//! identically no matter which tool name is called, rather than this file re-registering those names),
//! clip.reorder_effect/clip.effects_bulk (Project::reorder_effect/bulk_set_effect_params),
//! subtitles.style_preset, inspector.folds. Does NOT register clip.add_lut/color.auto/color.match
//! (color-engine's) - `tool_names_are_sole_registration` (tools_registry_tests.rs, color-engine's own
//! structural test) already pins those three to exactly one registration crate-wide.

use super::tools_args::Args;
use super::tools_helpers::*;
use super::*;
use crate::mcp::tools::{ToolDef, ToolKind, ToolOutcome};
use crate::ui::gallery::GalleryTab;

fn done(v: Value) -> Result<ToolOutcome, String> {
    Ok(ToolOutcome::Done(v))
}

/// Resolve a Look by name across BOTH `builtin_looks()` and non-graph `Settings.effect_presets` - the
/// single source of truth `gallery.apply(tab=Looks)` and color-engine's `looks.apply` (tools_color.rs, a
/// thin wrapper around this + `apply_look_by_name` below) both go through, so the same Look name
/// succeeds or fails identically no matter which tool name is called. Takes `&Settings` rather than
/// `&App` so it (and `apply_look_by_name`) stay unit-testable without a live App (see this file's own
/// "deviation" doc comment).
pub(super) fn find_look(settings: &Settings, name: &str) -> Option<crate::settings::EffectPreset> {
    crate::engine::presets::builtin_looks()
        .into_iter()
        .chain(settings.effect_presets.iter().filter(|p| !p.is_graph()).cloned())
        .find(|p| p.name.eq_ignore_ascii_case(name))
}

/// Apply a Look (resolved via `find_look`) to every clip in `clip_ids` at `intensity` - the pure logic
/// shared by `gallery.apply(tab=Looks)` and color-engine's `looks.apply`.
pub(super) fn apply_look_by_name(
    project: &mut Project,
    settings: &Settings,
    name: &str,
    clip_ids: &[Id],
    intensity: f32,
) -> Result<Value, String> {
    let preset = find_look(settings, name).ok_or_else(|| format!("no such Look '{name}'"))?;
    let mut n = 0;
    for &id in clip_ids {
        if crate::engine::presets::apply_look(&preset, project, id, intensity) {
            n += 1;
        }
    }
    Ok(json!({"ok": true, "count": n}))
}

/// Which clips can take `effect`: a pixel effect goes on any non-audio clip that renders from its
/// effect list (not a node graph), an audio effect on audio clips - the rule the old Effects pane's
/// click and drag always used.
fn takes_effect(c: &Clip, effect: &Effect) -> bool {
    (c.kind == ClipKind::Audio) == effect.kind.applies_to_audio() && !c.uses_graph()
}

/// Gallery > Effects: push `effect` onto every eligible clip of `ids` - or, with `whole_track`, of every
/// clip on the track the first existing one of `ids` sits on. Returns how many clips got it.
pub(crate) fn add_effect_card(project: &mut Project, effect: &Effect, ids: &[Id], whole_track: bool) -> usize {
    let targets: Vec<Id> = if whole_track {
        ids.iter()
            .find_map(|&id| project.track_of(id))
            .map(|ti| project.tracks[ti].clips.iter().map(|c| c.id).collect())
            .unwrap_or_default()
    } else {
        ids.to_vec()
    };
    let mut n = 0;
    for id in targets {
        if let Some(c) = project.clip_mut(id).filter(|c| takes_effect(c, effect)) {
            c.effects.push(effect.clone());
            n += 1;
        }
    }
    n
}

/// A Gallery > Effects card by name: a built-in effect's display name, or a library id
/// (`effects/glow`, read from the installed file).
fn effect_card(name: &str) -> Result<Vec<Effect>, String> {
    use crate::engine::gallery::{self as store, Kind as GKind};
    let mine = |n: &str| -> Option<Vec<Effect>> {
        let it = store::find(&store::root(), GKind::Effect, n)?;
        serde_json::from_value(it.data.get("effects")?.clone()).ok().filter(|v: &Vec<Effect>| !v.is_empty())
    };
    if let Some(n) = name.strip_prefix("mine:") {
        return mine(n).ok_or_else(|| format!("no saved effect '{n}' in your Gallery"));
    }
    if let Some((crate::engine::library::Kind::Effect, short)) = crate::engine::library::split_id(name) {
        let src = crate::engine::library::read(crate::engine::library::Kind::Effect, &short)
            .map_err(|e| format!("{name} is not installed: {e}"))?;
        return Ok(vec![Effect::from_shader(&src)]);
    }
    if let Some(k) = EffectKind::ALL.into_iter().find(|k| k.name().eq_ignore_ascii_case(name)) {
        return Ok(vec![Effect::new(k)]);
    }
    mine(name).ok_or_else(|| format!("unknown effect '{name}'"))
}

/// A Gallery > Transitions card by name: a built-in kind's display name, or a library id
/// (`transitions/paper-tear`) - the latter also picks the shader in the panel state.
fn transition_card(app: &mut App, name: &str) -> Result<crate::model::TransitionKind, String> {
    use crate::engine::library::{self, Kind};
    if let Some((Kind::Transition, short)) = library::split_id(name) {
        let src = library::read(Kind::Transition, &short).map_err(|e| format!("{name} is not installed: {e}"))?;
        app.transitions_ui.pick_shader(&short, &src);
        return Ok(crate::model::TransitionKind::Shader);
    }
    let mine = |app: &mut App, n: &str| -> Option<crate::model::TransitionKind> {
        use crate::engine::gallery::{self as store, Kind as GKind};
        let d = store::find(&store::root(), GKind::Transition, n)?.data;
        let kind = d.get("kind").and_then(Value::as_str).and_then(crate::model::TransitionKind::from_name);
        let st = &mut app.transitions_ui;
        if let Some(x) = d.get("duration").and_then(Value::as_f64) {
            st.duration = x.clamp(0.1, 5.0);
        }
        if let Some(c) = d.get("color").and_then(|c| serde_json::from_value::<[u8; 4]>(c.clone()).ok()) {
            st.color = c;
        }
        st.direction = d.get("direction").and_then(Value::as_u64).unwrap_or(0) as u8;
        let shader = d.get("shader").and_then(Value::as_str).unwrap_or("");
        if !shader.is_empty() {
            st.pick_shader("", shader);
            if let Some(p) = d.get("params").and_then(|p| serde_json::from_value::<Vec<f64>>(p.clone()).ok()) {
                if !p.is_empty() {
                    st.params = p;
                }
            }
            return Some(crate::model::TransitionKind::Shader);
        }
        kind.or(Some(crate::model::TransitionKind::CrossFade))
    };
    if let Some(n) = name.strip_prefix("mine:") {
        return mine(app, n).ok_or_else(|| format!("no saved transition '{n}' in your Gallery"));
    }
    crate::model::TransitionKind::from_name(name)
        .or_else(|| mine(app, name))
        .ok_or_else(|| format!("unknown transition '{name}'"))
}

/// `gallery.apply`'s per-tab body: Looks/Luts/Captions/SpeedRamps target `clip_ids` (Captions is
/// project-wide, `clip_ids` ignored), Effects pushes an effect (`place` = "track": onto the whole track),
/// Transitions adds at the edges `place` names (`transitions_ui::Where`; none = the old behaviour: the cut
/// at the start of each clip, or a fade-in). Templates are NOT handled here - `gallery.rs`'s own
/// `GalleryResponse.place` routes those through the existing `App::place_template`/`templates.apply`,
/// since a template places POSITIONALLY, not per-clip.
fn apply_card(
    app: &mut App,
    tab: GalleryTab,
    name: &str,
    clip_ids: &[Id],
    intensity: f32,
    place: Option<&str>,
    skip_existing: Option<bool>,
) -> Result<Value, String> {
    match tab {
        GalleryTab::Looks => apply_look_by_name(&mut app.project, &app.settings, name, clip_ids, intensity),
        GalleryTab::Luts => {
            crate::engine::lut::load(name).map_err(|e| format!("bad LUT: {e}"))?;
            let mut n = 0;
            for &id in clip_ids {
                if let Some(c) = app.project.clip_mut(id) {
                    let mut e = Effect::new(EffectKind::Lut);
                    e.lut = name.to_string();
                    e.params[0].value = intensity as f64;
                    c.effects.push(e);
                    n += 1;
                }
            }
            Ok(json!({"ok": true, "count": n}))
        }
        GalleryTab::Captions => {
            let style = crate::engine::presets::builtin_caption_styles()
                .into_iter()
                .chain(app.settings.caption_presets.iter().cloned())
                .find(|s| s.text.eq_ignore_ascii_case(name))
                .ok_or_else(|| format!("no such caption style '{name}'"))?;
            app.project.subtitle_style = style;
            Ok(json!({"ok": true}))
        }
        GalleryTab::SpeedRamps => {
            let want = name.strip_prefix("mine:").unwrap_or(name);
            let builtin = crate::engine::presets::builtin_speed_ramps()
                .into_iter()
                .find(|r| r.name.eq_ignore_ascii_case(want) && !name.starts_with("mine:"));
            let mine = || {
                use crate::engine::gallery::{self as store, Kind as GKind};
                let d = store::find(&store::root(), GKind::Speed, want)?.data;
                Some(crate::settings::CurvePreset {
                    name: want.to_string(),
                    keys: serde_json::from_value(d.get("keys")?.clone()).ok()?,
                    absolute: d.get("absolute").and_then(Value::as_bool).unwrap_or(true),
                })
            };
            let preset = builtin.or_else(mine).ok_or_else(|| format!("no such speed ramp '{name}'"))?;
            let mut n = 0;
            for &id in clip_ids {
                if let Some(c) = app.project.clip_mut(id) {
                    crate::engine::presets::apply_speed_ramp(&preset, c); // ws:animator: as a block
                    n += 1;
                }
            }
            Ok(json!({"ok": true, "count": n}))
        }
        GalleryTab::Effects => {
            let effects = effect_card(name)?;
            let mut n = 0;
            for e in &effects {
                n = n.max(add_effect_card(&mut app.project, e, clip_ids, place == Some("track")));
            }
            if n == 0 {
                return Err(if clip_ids.is_empty() {
                    "Select a clip first"
                } else {
                    "no selected clip can take that effect"
                }
                .into());
            }
            Ok(json!({"ok": true, "count": n}))
        }
        GalleryTab::Transitions => {
            let kind = transition_card(app, name)?;
            let dur = app.transitions_ui.duration;
            let n = match place {
                None => crate::ui::transitions_ui::add_transitions(
                    &mut app.project,
                    clip_ids,
                    &mut app.transitions_ui,
                    kind,
                    dur,
                    false,
                ),
                Some(w) => {
                    let w = crate::ui::transitions_ui::Where::from_name(w)
                        .ok_or_else(|| format!("where must be start|end|connection|all, not '{w}'"))?;
                    let skip = skip_existing.unwrap_or(app.settings.transition_skip_existing);
                    let n = crate::ui::transitions_ui::apply_where(
                        &mut app.project,
                        &mut app.transitions_ui,
                        clip_ids,
                        kind,
                        dur,
                        w,
                        skip,
                    );
                    if n == 0 {
                        return Err("no transition could go there (nothing selected, no clip edge of that kind, \
                                    or the clip is too short)"
                            .into());
                    }
                    n
                }
            };
            Ok(json!({"ok": true, "count": n}))
        }
        GalleryTab::Sounds => Err("Sounds are placed by gallery.apply with `at` (handled before this)".into()),
        GalleryTab::AudioEffects => apply_audio_fx(&mut app.project, name, clip_ids),
        GalleryTab::Visualizers => {
            Err("Visualizers are placed by gallery.apply with `at` (handled before this)".into())
        }
        GalleryTab::AudioTransitions => {
            let skip = skip_existing.unwrap_or(app.settings.transition_skip_existing);
            apply_audio_transition(&mut app.project, &mut app.transitions_ui, skip, name, clip_ids, place)
        }
        GalleryTab::Templates => Err("gallery.apply doesn't place Templates - use templates.apply".into()),
        // ws:text-titles: Titles places positionally too (and needs the exposed-field zip) - use
        // titles.place, not gallery.apply.
        GalleryTab::Titles => Err("gallery.apply doesn't place Titles - use titles.place".into()),
        // ws:keyframe-blocks: at the playhead, like keyblocks.apply
        GalleryTab::Animate | GalleryTab::AudioAnimate => {
            let n = super::tools_keyblocks::apply_block(app, name, clip_ids, Some(app.playhead), None)?;
            Ok(json!({"ok": true, "count": n}))
        }
    }
}

// ---- ws:gallery-audio ----
/// The audio clips `ids` mean: an audio clip itself, a video clip's linked sound. In order, no repeats.
pub(super) fn audio_targets(project: &Project, ids: &[Id]) -> Vec<Id> {
    let mut out: Vec<Id> = Vec::new();
    for &id in ids {
        let Some(c) = project.clip(id) else { continue };
        let ids = if c.kind == ClipKind::Audio { vec![id] } else { project.linked(id) };
        for a in ids {
            if project.clip(a).is_some_and(|c| c.kind == ClipKind::Audio) && !out.contains(&a) {
                out.push(a);
            }
        }
    }
    out
}

// ---- ws:preview-audio-only ----
/// Gallery ▸ Audio ▸ Visualizers: a visualizer layer of type `name` at `at` on video track `track` (None = the
/// first free one) that shows the timeline mix, as long as the sound from `at` on (10 s on a silent stretch).
/// The type becomes the last-used one. Returns the new clip.
pub(super) fn place_visualizer(app: &mut App, name: &str, at: f64, track: Option<usize>) -> Result<Id, String> {
    let style = crate::engine::visualizer::find(Some(name))?;
    let end = app
        .project
        .tracks
        .iter()
        .filter(|t| t.kind == TrackKind::Audio)
        .flat_map(|t| &t.clips)
        .map(|c| c.start + c.duration)
        .fold(0.0, f64::max);
    let dur = if end > at + 1.0 { end - at } else { 10.0 };
    let id = crate::engine::visualizer::add_mix(&mut app.project, at, dur, track, &style)?;
    if app.settings.visualizer_style != style.name {
        app.settings.visualizer_style = style.name.clone();
        app.settings.save();
    }
    app.selection = vec![id];
    Ok(id)
}

/// Gallery ▸ Audio ▸ Effects: route the audio clips through the preset's bus (`Project::apply_repair`).
fn apply_audio_fx(project: &mut Project, name: &str, clip_ids: &[Id]) -> Result<Value, String> {
    let ids = audio_targets(project, clip_ids);
    if ids.is_empty() {
        return Err("Select an audio clip first".into());
    }
    let id = crate::engine::mixer_fx::fx_id(name).ok_or_else(|| format!("unknown audio effect '{name}'"))?;
    let bus = project.apply_repair(&ids, id);
    if bus == 0 {
        return Err(format!("couldn't apply '{name}'"));
    }
    Ok(json!({"ok": true, "count": ids.len(), "bus_id": bus}))
}

/// Gallery ▸ Audio ▸ Transitions: a crossfade with a curve or a dip at the spots `place` names (like the
/// Transitions tab) on the AUDIO clips only - a linked picture is left exactly as it was - or a J / L-cut:
/// the sound's cut rolled earlier / later than the picture's by the Transitions tab's duration.
fn apply_audio_transition(
    project: &mut Project,
    st: &mut crate::ui::transitions_ui::TransitionsState,
    skip_existing: bool,
    name: &str,
    clip_ids: &[Id],
    place: Option<&str>,
) -> Result<Value, String> {
    use crate::model::{TrackKind, TransitionKind, XfadeCurve};
    let ids = audio_targets(&project, clip_ids);
    if ids.is_empty() {
        return Err("Select an audio clip first".into());
    }
    let dur = st.duration;
    let card = super::super::gallery::AUDIO_TRANSITIONS
        .iter()
        .find(|c| c.eq_ignore_ascii_case(name))
        .ok_or_else(|| format!("unknown audio transition '{name}'"))?;
    if let Some(j) = match *card {
        "J-Cut" => Some(true),
        "L-Cut" => Some(false),
        _ => None,
    } {
        let mut n = 0;
        for id in ids {
            let Some(start) = project.clip(id).map(|c| c.start) else { continue };
            n += project.roll_edit(id, if j { start - dur } else { start + dur }) as usize;
        }
        if n == 0 {
            return Err(format!("{card} needs an audio clip with another one right before it, long enough to roll"));
        }
        return Ok(json!({"ok": true, "count": n}));
    }
    let (kind, curve) = match *card {
        "Dip" => (TransitionKind::FadeToColor, XfadeCurve::Linear),
        "Equal Power" => (TransitionKind::CrossFade, XfadeCurve::EqualPower),
        "Exponential" => (TransitionKind::CrossFade, XfadeCurve::Exponential),
        _ => (TransitionKind::CrossFade, XfadeCurve::Linear),
    };
    // the picture keeps its own transitions: `add_transition` mirrors onto linked clips
    let before: Vec<(usize, Vec<crate::model::Transition>)> = project
        .tracks
        .iter()
        .enumerate()
        .filter(|(_, t)| t.kind != TrackKind::Audio)
        .map(|(i, t)| (i, t.transitions.clone()))
        .collect();
    let old: std::collections::HashSet<Id> =
        project.tracks.iter().flat_map(|t| t.transitions.iter().map(|x| x.id)).collect();
    let n = match place {
        None => crate::ui::transitions_ui::add_transitions(project, &ids, st, kind, dur, false),
        Some(w) => {
            let w = crate::ui::transitions_ui::Where::from_name(w)
                .ok_or_else(|| format!("where must be start|end|connection|all, not '{w}'"))?;
            crate::ui::transitions_ui::apply_where(project, st, &ids, kind, dur, w, skip_existing)
        }
    };
    for (i, trs) in before {
        project.tracks[i].transitions = trs;
    }
    for t in project.tracks.iter_mut().flat_map(|t| t.transitions.iter_mut()) {
        if !old.contains(&t.id) {
            t.curve = curve;
        }
    }
    if n == 0 {
        return Err("no transition could go there (the clip is too short)".into());
    }
    Ok(json!({"ok": true, "count": n}))
}

/// Show the Gallery pane on `tab` (also what the Effects / Transitions shortcuts do now).
pub(super) fn open_tab(app: &mut App, tab: GalleryTab) {
    app.gallery.tab = tab;
    app.gallery.lib = None;
    if tab.is_audio() {
        app.settings.gallery_audio_tab = tab.name().to_string();
    } else {
        app.settings.gallery_tab = tab.name().to_string();
    }
    app.surface(Pane::Presets);
}

pub const TOOLS: &[ToolDef] = &[
    ToolDef {
        name: "gallery.list",
        desc: "Card names in one Gallery tab (Video side: Looks|Luts|Captions|SpeedRamps|Effects|Transitions|Templates|Titles|Animate; Audio side: AudioAnimate|AudioTransitions|AudioEffects|Visualizers|Sounds), or every tab when omitted. Looks excludes saved node-graph presets and reuses color-engine's builtin_looks() - the sole tool surface for Looks. With `source`, `query`, `category` or `sort` (the Gallery header's chips, search box, category menu and sort order) the list is filtered the same way and also holds the library's items for Effects / Transitions / Luts (installed ones always; online ones once the online library is connected: see gallery.connect). `detail` returns {name, source (Built-in|Mine|Installed|Online), id, categories, uses} per card.",
        args: &[
            "tab:string:false:omit=all tabs",
            "mine:boolean:false:only the items you saved to the Gallery (= source mine)",
            "source:string:false:all|built-in|mine|installed|online",
            "query:string:false:words matched in name / tags / category / description",
            "category:string:false:an effect category, transition kind (Fades|Slides & wipes|Shader) or tag",
            "sort:string:false:default|name|recent|used (most used)",
            "detail:boolean:false:objects instead of names",
        ],
        kind: ToolKind::Read,
        run: |app, args| {
            use crate::ui::gallery_filter::{Filters, Sort, Src};
            let mut f = Filters { q: arg_str(args, "query").unwrap_or("").to_string(), ..Default::default() };
            f.cat = arg_str(args, "category").unwrap_or("").to_string();
            if arg_bool(args, "mine").unwrap_or(false) {
                f.src = Src::Mine;
            }
            if let Some(s) = arg_str(args, "source") {
                f.src = Src::from_name(s).ok_or_else(|| format!("unknown source '{s}' (all, built-in, mine, installed, online)"))?;
            }
            if let Some(s) = arg_str(args, "sort") {
                f.sort = Sort::from_name(s).ok_or_else(|| format!("unknown sort '{s}' (default, name, recent, used)"))?;
            }
            let detail = arg_bool(args, "detail").unwrap_or(false);
            let plain = !detail && !f.active() && f.sort == Sort::Catalogue;
            if !plain {
                library_ctl::ensure(app, false); // not connected: the installed items only, no network
            }
            let names = |t: GalleryTab| -> Vec<Value> {
                if plain {
                    return crate::ui::gallery::card_names(t, &app.settings).into_iter().map(|n| json!(n)).collect();
                }
                let rows = crate::ui::gallery::lib_kind_of(t).map(|k| app.online.rows(Some(k), "")).unwrap_or_default();
                crate::ui::gallery::card_infos(t, &app.settings, &rows, &f, 0.0)
                    .into_iter()
                    .map(|c| {
                        if detail {
                            json!({"name": c.name, "source": c.src.label(), "id": c.id, "categories": c.cats, "uses": c.uses})
                        } else {
                            json!(c.name)
                        }
                    })
                    .collect()
            };
            match Args(args).str("tab") {
                Some(s) => {
                    let tab = GalleryTab::from_name(s).ok_or_else(|| format!("unknown tab '{s}'"))?;
                    done(json!({tab.name(): names(tab)}))
                }
                None => {
                    let mut out = serde_json::Map::new();
                    for t in GalleryTab::ALL {
                        out.insert(t.name().to_string(), json!(names(t)));
                    }
                    done(Value::Object(out))
                }
            }
        },
    },
    ToolDef {
        name: "gallery.connect",
        desc: "Switch the online library on or off for the Gallery (the \"Browse online library\" button / the tab menu's \"Turn the online library off\"). Off by default: until it is on nothing is fetched from the network, and the Gallery lists only built-in, your own and already-installed items. On: the small catalogue index is fetched (nothing else; an item downloads when it is used). Remembered between sessions. UI-only, no undo.",
        args: &["on:boolean:false:default true"],
        kind: ToolKind::Ui,
        run: |app, args| {
            let on = arg_bool(args, "on").unwrap_or(true);
            library_ctl::set_connected(app, on);
            done(json!({"ok": true, "connected": library_ctl::connected(&app.settings)}))
        },
    },
    ToolDef {
        name: "gallery.apply",
        desc: "Apply a Gallery card (Look/LUT/Caption/SpeedRamp/Effect/Transition) to clip_ids (default selection); Captions is project-wide. Effects: name = an effect's display name or a library id (effects/glow), where=track adds it to every clip on the selection's track. Transitions: name = Cross Fade|Fade to Color|Push|Wipe or a library id (transitions/x); where = start|end (an edge transition on that side of each clip), connection (the cut at its start/end where a clip abuts), connection_start|connection_end, all (the cut at each clip's start, else a fade-in) - omit for the old default (= all, unclamped). Duration is the Transitions tab's, clamped to half the shorter clip. Audio side (audio clips, or a video clip's linked sound): AudioAnimate = a volume/pan block at the playhead; AudioEffects = route through the preset's Mixer bus (Telephone, Radio, Hall, Pitch Up, Low-pass Sweep…); AudioTransitions = Equal Power|Linear|Exponential crossfade or Dip (with where, as Transitions; the picture is left alone) or J-Cut|L-Cut (roll the sound's cut earlier / later by the transition duration); Visualizers = a visualizer layer of that type (Neon Orb, Waveform, Spectrum Bars, Voxel Blocks or a custom one) at `at` (default the playhead) on video `track`, showing the timeline's mixed sound. Templates place at the playhead instead - use templates.apply.",
        args: &[
            "tab:string:true:",
            "name:string:true:",
            "clip_ids:array:false:defaults to selection",
            "intensity:number:false:Looks/Luts only, 0..1 default 1.0",
            "where:string:false:Effects: track | Transitions: start|end|connection|connection_start|connection_end|all",
            "skip_existing:boolean:false:Transitions with where: leave a spot that already has a transition alone (default: the setting; off = replace)",
            "at:number:false:Sounds / Visualizers: timeline seconds (default the playhead)",
            "track:integer:false:Sounds: audio track index, -1 = a new track | Visualizers: video track index (default: the first free one)",
        ],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let tab_s = req(arg_str(args, "tab"), "tab")?;
            let tab = GalleryTab::from_name(tab_s).ok_or_else(|| format!("unknown tab '{tab_s}'"))?;
            let name = req(arg_str(args, "name"), "name")?.to_string();
            let ids = Args(args).ids_or_selection("clip_ids", app);
            let intensity = arg_f64(args, "intensity").unwrap_or(1.0).clamp(0.0, 1.0) as f32;
            if tab == GalleryTab::Visualizers {
                let at = arg_f64(args, "at").unwrap_or(app.playhead);
                let id = place_visualizer(app, &name, at, arg_u64(args, "track").map(|t| t as usize))?;
                return Ok(ToolOutcome::Done(json!({"ok": true, "clip_id": id})));
            }
            if tab == GalleryTab::Sounds {
                let at = arg_f64(args, "at").unwrap_or(app.playhead);
                let track = arg_u64(args, "track").map(|t| t as usize);
                let ids = gallery_mine::place_sound(app, &name, at, track, args.get("track").and_then(Value::as_i64) == Some(-1))?;
                return Ok(ToolOutcome::Done(json!({"ok": true, "clip_ids": ids})));
            }
            let place = arg_str(args, "where");
            apply_card(app, tab, &name, &ids, intensity, place, arg_bool(args, "skip_existing")).map(ToolOutcome::Done)
        },
    },
    ToolDef {
        name: "gallery.save",
        desc: "Save something of yours to your own Gallery (files in %APPDATA%\\SimpleEditor\\gallery\\<kind>\\, shown in the matching tab marked Mine). kind: look (a clip's effect stack) | effect (index = one effect, else the whole stack, added on use) | transition (transition_id, default the selected one: length, colour, direction, shader controls) | caption (the project's style) | speed (a clip's speed curve) | animate (a clip's keyframes) | template (the selected clips; titles too) | sound (an audio clip: its whole file, or the trimmed range) | shader_effect (index) | shader_transition | html (an HTML layer) | lut (index) - the last four go to the local library. A taken name is numbered unless replace=true. No undo (it is a file).",
        args: &[
            "kind:string:true:look|effect|transition|caption|speed|animate|template|sound|shader_effect|shader_transition|html|lut",
            "name:string:true:",
            "tags:array:false:strings",
            "replace:boolean:false:overwrite the item of that name (default: number the new one)",
            "clip_id:integer:false:default the first selected clip",
            "index:integer:false:effect index for effect / shader_effect / lut",
            "transition_id:integer:false:transition / shader_transition (default the selected one)",
        ],
        kind: ToolKind::Ui,
        run: |app, args| {
            let kind = req(arg_str(args, "kind"), "kind")?;
            let name = req(arg_str(args, "name"), "name")?.to_string();
            let tags: Vec<String> = args
                .get("tags")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_str).map(String::from).collect())
                .unwrap_or_default();
            let index = arg_u64(args, "index").map(|i| i as usize);
            let tid = arg_u64(args, "transition_id").or_else(|| app.sel_transitions.first().copied());
            let need_tr = || tid.ok_or_else(|| "select a transition (or pass transition_id)".to_string());
            use gallery_mine::Save;
            let what = match kind {
                "look" => Save::Look,
                "effect" => Save::Effect(index),
                "transition" => Save::Transition(need_tr()?),
                "caption" => Save::Caption,
                "speed" => Save::Speed,
                "animate" => Save::Animate,
                "template" | "title" => Save::Template,
                "sound" => Save::Sound,
                "shader_effect" => Save::ShaderEffect(index.ok_or("index: the effect's position in the stack")?),
                "shader_transition" => Save::ShaderTransition(need_tr()?),
                "html" => Save::Html,
                "lut" => Save::Lut(index.ok_or("index: the LUT effect's position in the stack")?),
                other => return Err(format!("unknown kind '{other}'")),
            };
            // (the capture reads the selection: point it at clip_id for the call)
            let keep = app.selection.clone();
            if let Some(c) = arg_u64(args, "clip_id") {
                app.selection = vec![c];
            }
            let r = gallery_mine::save(app, &what, &name, &crate::engine::gallery::parse_tags(&tags.join(",")), arg_bool(args, "replace").unwrap_or(false));
            app.selection = keep;
            let saved = r?;
            done(json!({"ok": true, "name": saved, "tab": what.tab()}))
        },
    },
    ToolDef {
        name: "gallery.import",
        desc: "Import to Gallery: Gallery item files, audio files (into Sounds), folders (searched through) and .zip files, into your gallery folder. A name that is taken is numbered, nothing is overwritten. Returns what was added and what was skipped.",
        args: &["paths:array:true:absolute paths of files, folders or zips"],
        kind: ToolKind::Ui,
        run: |app, args| {
            let paths: Vec<std::path::PathBuf> = args
                .get("paths")
                .and_then(Value::as_array)
                .ok_or("paths: array required")?
                .iter()
                .filter_map(Value::as_str)
                .map(std::path::PathBuf::from)
                .collect();
            done(gallery_mine::import(app, &paths))
        },
    },
    ToolDef {
        name: "gallery.manage",
        desc: "Change one of your saved Gallery items: action = rename (new_name) | tags (tags: replaces them) | duplicate | replace (with the current selection's version) | delete (moved to the gallery folder's .trash). kind as gallery.save's: look|effect|transition|caption|speed|animate|template|sound.",
        args: &[
            "action:string:true:rename|tags|duplicate|replace|delete",
            "kind:string:true:look|effect|transition|caption|speed|animate|template|sound",
            "name:string:true:the item's name",
            "new_name:string:false:rename",
            "tags:array:false:tags",
        ],
        kind: ToolKind::Ui,
        run: |app, args| {
            use gallery_mine::{MineAct, MineRef};
            let k = req(arg_str(args, "kind"), "kind")?;
            let kind = crate::engine::gallery::Kind::from_name(if k == "title" { "template" } else { k })
                .ok_or_else(|| format!("unknown kind '{k}'"))?;
            let r = MineRef { kind, name: req(arg_str(args, "name"), "name")?.to_string() };
            let tags = |a: &Value| -> Vec<String> {
                a.get("tags")
                    .and_then(Value::as_array)
                    .map(|v| v.iter().filter_map(Value::as_str).map(String::from).collect())
                    .unwrap_or_default()
            };
            let msg = match req(arg_str(args, "action"), "action")? {
                "rename" => {
                    let it = crate::engine::gallery::find(&crate::engine::gallery::root(), kind, &r.name).ok_or("no such item")?;
                    gallery_mine::edit_saved(app, &r, req(arg_str(args, "new_name"), "new_name")?, &it.tags, false)?
                }
                "tags" => gallery_mine::edit_saved(app, &r, &r.name.clone(), &crate::engine::gallery::parse_tags(&tags(args).join(",")), true)?,
                "duplicate" => gallery_mine::do_act(app, MineAct::Duplicate, r)?,
                "replace" => gallery_mine::do_act(app, MineAct::ReplaceCurrent, r)?,
                "delete" => gallery_mine::do_act(app, MineAct::Delete, r)?,
                other => return Err(format!("unknown action '{other}'")),
            };
            done(json!({"ok": true, "message": msg}))
        },
    },
    ToolDef {
        name: "gallery.reveal",
        desc: "Show a saved Gallery item's file in Explorer (kind + name), or - with only kind (or nothing) - open that gallery folder. The files are plain JSON (sounds: the audio file) you can zip and share.",
        args: &["kind:string:false:look|effect|transition|caption|speed|animate|template|sound", "name:string:false:an item"],
        kind: ToolKind::Ui,
        run: |_app, args| {
            let kind = arg_str(args, "kind").and_then(crate::engine::gallery::Kind::from_name);
            match (kind, arg_str(args, "name")) {
                (Some(k), Some(n)) => {
                    let it = crate::engine::gallery::find(&crate::engine::gallery::root(), k, n).ok_or("no such item")?;
                    gallery_mine::reveal(&it);
                    done(json!({"ok": true, "file": it.file.as_ref().unwrap_or(&it.path)}))
                }
                _ => {
                    gallery_mine::open_folder(kind);
                    done(json!({"ok": true, "folder": kind.map_or_else(crate::engine::gallery::root, |k| crate::engine::gallery::root().join(k.dir()))}))
                }
            }
        },
    },
    ToolDef {
        name: "gallery.tab",
        desc: "Open the Gallery pane on a tab (Video side: Looks|Luts|Captions|SpeedRamps|Effects|Transitions|Templates|Titles|Animate, or the library tabs HTML|Icons|Skills; Audio side: AudioAnimate|AudioTransitions|AudioEffects|Visualizers|Sounds), the same as clicking it; the pane is shown if it was hidden. What Ctrl+3 (Effects) and Ctrl+4 (Transitions) do. UI-only, no undo.",
        args: &["tab:string:true:a tab name from gallery.list"],
        kind: ToolKind::Ui,
        run: |app, args| {
            let tab_s = req(arg_str(args, "tab"), "tab")?;
            // the library's own tabs (HTML / Icons / Skills) are not `GalleryTab`s
            if let Some(&(kind, label)) = crate::ui::gallery::LIB_TABS.iter().find(|(_, l)| l.eq_ignore_ascii_case(tab_s)) {
                app.gallery.lib = Some(kind);
                app.surface(Pane::Presets);
                return done(json!({"ok": true, "tab": label}));
            }
            let tab = GalleryTab::from_name(tab_s).ok_or_else(|| format!("unknown tab '{tab_s}'"))?;
            open_tab(app, tab);
            done(json!({"ok": true, "tab": tab.name()}))
        },
    },
    ToolDef {
        name: "gallery.hover",
        desc: "Set/clear the monitor's alt-render preview to a Gallery card (name:null clears). UI-only, no mutation, no undo.",
        args: &["tab:string:true:", "name:string:false:null clears"],
        kind: ToolKind::Ui,
        run: |app, args| {
            let tab_s = req(arg_str(args, "tab"), "tab")?;
            let want = match arg_str(args, "name") {
                Some(n) => Some(monitor::AltRequest::Gallery(tab_s.to_string(), n.to_string())),
                None => None,
            };
            app.alt_render.request(want);
            done(json!({"ok": true}))
        },
    },
    ToolDef {
        name: "clip.reorder_effect",
        desc: "Move an effect to a new stack index (Project::reorder_effect).",
        args: &["clip_id:integer:true:", "from:integer:true:", "to:integer:true:"],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let id = req(arg_u64(args, "clip_id"), "clip_id")?;
            let from = req(arg_u64(args, "from"), "from")? as usize;
            let to = req(arg_u64(args, "to"), "to")? as usize;
            if !app.project.reorder_effect(id, from, to) {
                return Err("no such clip, or from/to out of range".into());
            }
            done(json!({"ok": true}))
        },
    },
    ToolDef {
        name: "clip.effects_bulk",
        desc: "Set one stack-index effect's params on every listed clip whose effect at that index shares its kind (Project::bulk_set_effect_params).",
        args: &["clip_ids:array:true:", "index:integer:true:", "params:object:true:name->value"],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let ids = req(arg_ids(args, "clip_ids"), "clip_ids")?;
            let index = req(arg_u64(args, "index"), "index")? as usize;
            let obj = req(args.get("params"), "params")?.as_object().ok_or("params: object")?;
            let mut params = std::collections::HashMap::new();
            for (k, v) in obj {
                params.insert(k.clone(), v.as_f64().ok_or_else(|| format!("params.{k}: number"))?);
            }
            let n = app.project.bulk_set_effect_params(&ids, index, &params);
            done(json!({"ok": true, "count": n}))
        },
    },
    ToolDef {
        name: "subtitles.style_preset",
        desc: "Set project.subtitle_style from a named caption style (builtin or Settings.caption_presets).",
        args: &["name:string:true:builtin or Settings.caption_presets"],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let name = req(arg_str(args, "name"), "name")?;
            let style = crate::engine::presets::builtin_caption_styles()
                .into_iter()
                .chain(app.settings.caption_presets.iter().cloned())
                .find(|s| s.text.eq_ignore_ascii_case(name))
                .ok_or_else(|| format!("no such caption style '{name}'"))?;
            app.project.subtitle_style = style;
            done(json!({"ok": true}))
        },
    },
    ToolDef {
        name: "inspector.folds",
        desc: "Set one inspector section's remembered open/closed state (Settings.inspector_folds). No undo - Settings-level, like other UI prefs.",
        args: &["section:string:true:", "open:boolean:true:"],
        kind: ToolKind::Ui,
        run: |app, args| {
            let section = req(arg_str(args, "section"), "section")?.to_string();
            let open = req(arg_bool(args, "open"), "open")?;
            app.settings.inspector_folds.insert(section, open);
            app.settings.save();
            done(json!({"ok": true}))
        },
    },
];

// deviation (see PR body): no unit tests for the `run` closures / `apply_card` itself - every one needs
// `&mut App`, and (per tools_registry_tests.rs's/monitor.rs's own "deviation" doc comments) this crate
// has no headless `App`-construction path anywhere. Coverage comes from the crate-wide structural tests
// (tool_names_unique_and_namespaced, every_arg_spec_parses, tool_names_are_sole_registration,
// mutate_rows_roll_back_on_error, server_end_to_end) plus `Project::reorder_effect`/
// `bulk_set_effect_params`'s own unit tests in `src/model/ops/effects.rs`, which is the pure logic these
// two tools call directly. `find_look`/`apply_look_by_name` are the one exception - deliberately typed
// over `&Settings`/`&mut Project` instead of `&mut App` so the Looks resolution both `gallery.apply` and
// color-engine's `looks.apply` share (see tools_color.rs) gets real unit tests below.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Clip, ClipKind};

    /// A `Vec<Effect>` Look preset, same shape `engine::presets::capture_template` writes for a
    /// user-saved (non-graph) Look - `EffectPreset::is_graph()` is false for a JSON array.
    fn user_look(name: &str) -> crate::settings::EffectPreset {
        let fx = vec![Effect::new(EffectKind::Blur)];
        crate::settings::EffectPreset { name: name.to_string(), json: serde_json::to_string(&fx).unwrap() }
    }

    /// The bug this fixes: `looks.apply` used to only search `builtin_looks()`, so a Look saved from the
    /// Gallery pane (`Settings.effect_presets`) would apply via `gallery.apply(tab=Looks)` but fail via
    /// `looks.apply` for the exact same name. `find_look` is what both now resolve through.
    #[test]
    fn find_look_resolves_both_builtins_and_user_saved_presets() {
        let mut settings = Settings::default();
        let builtin_name = crate::engine::presets::builtin_looks()[0].name.clone();
        assert!(find_look(&settings, &builtin_name).is_some(), "a builtin Look must resolve");
        assert!(find_look(&settings, "My Custom Look").is_none(), "not saved yet");
        settings.effect_presets.push(user_look("My Custom Look"));
        assert!(
            find_look(&settings, "My Custom Look").is_some(),
            "a user-saved (non-graph) preset must resolve too - this is exactly what looks.apply used to miss"
        );
    }

    /// `apply_look_by_name` is the one function both `gallery.apply(tab=Looks)` (via `apply_card`) and
    /// color-engine's `looks.apply` call - so a builtin Look name and a user-saved preset name must both
    /// apply successfully through it, producing identical results regardless of which tool name reaches it.
    #[test]
    fn apply_look_by_name_succeeds_for_both_builtin_and_user_saved_names() {
        let mut p = Project::new();
        let c = Clip::new(500, ClipKind::Video, "v", 0.0, 3.0);
        let id = c.id;
        p.tracks[0].clips.push(c);
        let mut settings = Settings::default();
        let builtin_name = crate::engine::presets::builtin_looks()[0].name.clone();
        settings.effect_presets.push(user_look("My Custom Look"));

        let r1 = apply_look_by_name(&mut p, &settings, &builtin_name, &[id], 1.0).unwrap();
        assert_eq!(r1, json!({"ok": true, "count": 1}), "a builtin Look applies");
        let r2 = apply_look_by_name(&mut p, &settings, "My Custom Look", &[id], 1.0).unwrap();
        assert_eq!(r2, json!({"ok": true, "count": 1}), "a user-saved Look now applies too, not just builtins");

        assert!(apply_look_by_name(&mut p, &settings, "no such look", &[id], 1.0).is_err());
    }

    // ---- Effects / Transitions cards ----
    fn project_with(clips: &[(usize, ClipKind, Id)]) -> Project {
        let mut p = Project::new();
        for &(ti, kind, id) in clips {
            p.tracks[ti].clips.push(Clip::new(id, kind, "c", id as f64, 3.0));
        }
        p
    }

    /// A card click / drop / `gallery.apply` all land through `add_effect_card`: every eligible clip of the
    /// selection gets it (a pixel effect not on audio), or - `whole_track` - every clip of the first one's track.
    #[test]
    fn add_effect_card_targets_eligible_clips_only() {
        let mut p = project_with(&[(0, ClipKind::Video, 10), (0, ClipKind::Video, 20), (1, ClipKind::Audio, 30)]);
        let blur = Effect::new(EffectKind::Blur);
        assert_eq!(add_effect_card(&mut p, &blur, &[10, 30], false), 1, "the audio clip is skipped");
        assert_eq!(p.clip(10).unwrap().effects.len(), 1);
        assert_eq!(p.clip(30).unwrap().effects.len(), 0);
        assert_eq!(add_effect_card(&mut p, &blur, &[10], true), 2, "the whole track: both video clips");
        assert_eq!(p.clip(20).unwrap().effects.len(), 1);
        assert_eq!(p.clip(10).unwrap().effects.len(), 2, "stacked after the first");
        assert_eq!(add_effect_card(&mut p, &blur, &[], false), 0, "no selection");
        assert_eq!(add_effect_card(&mut p, &blur, &[999], true), 0, "a clip that is gone");
        let shader = Effect::from_shader("// @name Glow\nvec4 effect(vec4 src, vec2 uv) { return src; }\n");
        assert_eq!(add_effect_card(&mut p, &shader, &[20], false), 1, "a library shader effect takes the same path");
    }

    /// gpu::run_chain evaluates a node graph and never reads clip.effects - an add would be invisible, so
    /// a clip that renders from one takes no effect card (click, drop or tool).
    #[test]
    fn a_clip_rendering_from_a_node_graph_takes_no_effect_card() {
        let mut p = project_with(&[(0, ClipKind::Video, 7)]);
        p.ensure_graph(7);
        let _ = p.add_node(7, crate::model::NodeKind::Effect(Effect::new(EffectKind::Invert)), 0.0, 0.0);
        assert_eq!(add_effect_card(&mut p, &Effect::new(EffectKind::Blur), &[7], false), 0);
        assert!(p.clip(7).unwrap().effects.is_empty());
    }

    #[test]
    fn effect_and_transition_cards_resolve_by_name() {
        assert_eq!(effect_card("blur").unwrap()[0].kind, EffectKind::Blur, "case does not matter");
        assert!(effect_card("No Such Effect").is_err());
        assert!(effect_card("effects/not-installed-anywhere-xyz").is_err(), "a library id that is not on disk");
        assert_eq!(
            crate::model::TransitionKind::from_name("cross fade"),
            Some(crate::model::TransitionKind::CrossFade)
        );
        assert_eq!(crate::model::TransitionKind::from_name("CrossFade"), Some(crate::model::TransitionKind::CrossFade));
        assert_eq!(crate::model::TransitionKind::from_name("Nope"), None);
    }

    /// The tool surface: `gallery.apply` is one undo step (a `Mutate` tool), `gallery.tab` is UI-only, and
    /// the old call shape (tab / name / clip_ids) still parses next to the new `where`.
    #[test]
    fn the_gallery_tools_are_registered() {
        let apply = crate::mcp::tools::find("gallery.apply").expect("gallery.apply");
        assert!(matches!(apply.kind, ToolKind::Mutate), "one undo step per apply");
        for arg in ["tab:", "name:", "clip_ids:", "where:", "skip_existing:"] {
            assert!(apply.args.iter().any(|a| a.starts_with(arg)), "{arg}");
        }
        assert!(matches!(crate::mcp::tools::find("gallery.tab").expect("gallery.tab").kind, ToolKind::Ui));
        // your own Gallery: save / import / manage / reveal
        for name in ["gallery.save", "gallery.import", "gallery.manage", "gallery.reveal"] {
            let t = crate::mcp::tools::find(name).unwrap_or_else(|| panic!("{name} is not registered"));
            assert!(matches!(t.kind, ToolKind::Ui), "{name}: a file write, no project undo");
        }
        let save = crate::mcp::tools::find("gallery.save").unwrap();
        for arg in ["kind:", "name:", "tags:", "replace:", "clip_id:", "index:", "transition_id:"] {
            assert!(save.args.iter().any(|a| a.starts_with(arg)), "{arg}");
        }
        assert!(crate::mcp::tools::find("gallery.list").unwrap().args.iter().any(|a| a.starts_with("mine:")));
        assert!(GalleryTab::from_name("Sounds").is_some());
        assert!(GalleryTab::from_name("effects").is_some() && GalleryTab::from_name("Transitions").is_some());
    }

    // ---- ws:gallery-audio ----
    /// Video 101 [0,4) + 102 [4,8) linked to audio 201 [0,4) + 202 [4,8) (sound with room to roll).
    fn av() -> (Project, usize) {
        let mut p = Project::new();
        let ai = p.audio_tracks()[0];
        for (v, a, start, link) in [(101, 201, 0.0, 900), (102, 202, 4.0, 901)] {
            let mut vc = Clip::new(v, ClipKind::Video, "v", start, 4.0);
            let mut ac = Clip::new(a, ClipKind::Audio, "a", start, 4.0);
            vc.link = link;
            ac.link = link;
            ac.src_in = 2.0;
            p.tracks[0].clips.push(vc);
            p.tracks[ai].clips.push(ac);
        }
        (p, ai)
    }

    /// Every Audio card is ONE `gallery.apply` call, and that tool snapshots once for its undo.
    #[test]
    fn audio_cards_apply_through_one_undoable_tool() {
        let def = crate::mcp::tools::find("gallery.apply").unwrap();
        assert_eq!(def.kind, ToolKind::Mutate, "gallery.apply = one undo step per card");
        assert_eq!(crate::mcp::tools::find("audio.repair").unwrap().kind, ToolKind::Mutate);
        for t in ["AudioAnimate", "AudioTransitions", "AudioEffects", "Sounds"] {
            assert!(GalleryTab::from_name(t).is_some_and(|t| t.is_audio()), "{t}");
        }
    }

    #[test]
    fn every_audio_effect_routes_the_clip_through_its_chain() {
        use crate::engine::mixer_fx as fx;
        for (id, label) in fx::audio_fx_cards() {
            let (mut p, _) = av();
            // the linked video selected: its sound gets the effect, the picture nothing
            let r = apply_audio_fx(&mut p, label, &[101]).unwrap_or_else(|e| panic!("{label}: {e}"));
            assert_eq!(r["count"], 1, "{label}");
            let bus = r["bus_id"].as_u64().unwrap();
            assert_eq!(p.clip(201).unwrap().bus, bus, "{label}: the sound is routed");
            assert_eq!(p.clip(101).unwrap().bus, 0, "{label}: the picture is not");
            let b = p.bus(bus).unwrap();
            assert!(b.name.starts_with(label), "{label}: bus '{}'", b.name);
            let (_, want) = fx::fx_chain(id, (0.0, 4.0)).unwrap();
            let kinds = |v: &[crate::model::AudioFilter]| v.iter().map(|f| f.kind).collect::<Vec<_>>();
            assert_eq!(kinds(&b.filters), kinds(&want), "{label}");
            for (got, want) in b.filters.iter().zip(&want) {
                for (i, w) in want.params.iter().enumerate() {
                    assert!((got.params[i].value - w.value).abs() < 1e-9, "{label}: param {i}");
                }
            }
        }
        // the parameters a card promises
        let (mut p, _) = av();
        let bus = apply_audio_fx(&mut p, "Telephone", &[201]).unwrap()["bus_id"].as_u64().unwrap();
        let f = &p.bus(bus).unwrap().filters;
        assert_eq!((f[0].kind, f[0].params[0].value), (crate::model::FilterKind::HighPass, 400.0));
        assert_eq!((f[1].kind, f[1].params[0].value), (crate::model::FilterKind::LowPass, 3400.0));
        let bus = apply_audio_fx(&mut p, "pitch_up", &[201]).unwrap()["bus_id"].as_u64().unwrap();
        assert_eq!(p.bus(bus).unwrap().filters[0].params[0].value, 5.0, "an id works as well as a name");
        // a sweep is keyed across the clip, and each use gets its own bus
        let s1 = apply_audio_fx(&mut p, "Low-pass Sweep", &[201]).unwrap()["bus_id"].as_u64().unwrap();
        let s2 = apply_audio_fx(&mut p, "Low-pass Sweep", &[202]).unwrap()["bus_id"].as_u64().unwrap();
        assert_ne!(s1, s2);
        let a = &p.bus(s2).unwrap().filters[0].params[0];
        assert_eq!(a.keys.len(), 3);
        assert_eq!((a.keys[0].t, a.keys[2].t), (4.0, 8.0));
        assert!(a.at(4.0) > 10_000.0 && a.at(8.0) < 400.0);
        // nothing audio to take it
        let mut q = Project::new();
        q.tracks[0].clips.push(Clip::new(5, ClipKind::Video, "v", 0.0, 2.0));
        assert!(apply_audio_fx(&mut q, "Hall", &[5]).is_err());
        assert!(apply_audio_fx(&mut p, "No Such Effect", &[201]).is_err());
    }

    #[test]
    fn audio_crossfades_set_their_curve_and_leave_the_picture_alone() {
        use crate::model::{TransitionKind as K, XfadeCurve as C};
        for (card, kind, curve) in [
            ("Equal Power", K::CrossFade, C::EqualPower),
            ("Linear", K::CrossFade, C::Linear),
            ("Exponential", K::CrossFade, C::Exponential),
            ("Dip", K::FadeToColor, C::Linear),
        ] {
            let (mut p, ai) = av();
            let mut st = crate::ui::transitions_ui::TransitionsState::default();
            let r = apply_audio_transition(&mut p, &mut st, false, card, &[202], None).unwrap();
            assert_eq!(r["count"], 1, "{card}");
            let trs = &p.tracks[ai].transitions;
            assert_eq!(trs.len(), 1, "{card}");
            assert_eq!((trs[0].right, trs[0].kind, trs[0].curve), (202, kind, curve), "{card}");
            assert!(p.tracks[0].transitions.is_empty(), "{card}: the linked picture got no transition");
        }
        // the gains the mixer uses
        let (o, i) = crate::model::audio_curve_gains(false, C::EqualPower, 0.5);
        assert!((o - 0.7071).abs() < 1e-3 && (i - 0.7071).abs() < 1e-3);
        assert_eq!(crate::model::audio_curve_gains(true, C::Linear, 0.5), (0.0, 0.0), "a dip is silent mid-way");
        assert_eq!(crate::model::audio_curve_gains(false, C::Exponential, 0.5), (0.25, 0.25));
        // an existing picture transition at that cut survives an audio card
        let (mut p, _) = av();
        let vt = p.add_transition(102, K::Wipe, 1.0).unwrap();
        let mut st = crate::ui::transitions_ui::TransitionsState::default();
        apply_audio_transition(&mut p, &mut st, false, "Dip", &[202], None).unwrap();
        assert_eq!(p.tracks[0].transitions.iter().map(|t| (t.id, t.kind)).collect::<Vec<_>>(), [(vt, K::Wipe)]);
    }

    #[test]
    fn j_and_l_cuts_roll_only_the_sound() {
        for (card, cut) in [("J-Cut", 3.0), ("L-Cut", 5.0)] {
            let (mut p, _) = av();
            let mut st = crate::ui::transitions_ui::TransitionsState { duration: 1.0, ..Default::default() };
            apply_audio_transition(&mut p, &mut st, false, card, &[102], None).unwrap_or_else(|e| panic!("{card}: {e}"));
            assert_eq!(p.clip(201).unwrap().end(), cut, "{card}");
            assert_eq!(p.clip(202).unwrap().start, cut, "{card}");
            assert_eq!((p.clip(101).unwrap().end(), p.clip(102).unwrap().start), (4.0, 4.0), "{card}: picture kept");
        }
        // the first clip has nothing before it to roll into
        let (mut p, _) = av();
        let mut st = crate::ui::transitions_ui::TransitionsState::default();
        assert!(apply_audio_transition(&mut p, &mut st, false, "J-Cut", &[201], None).is_err());
    }

    #[test]
    fn audio_animate_cards_key_volume_and_pan() {
        let s = Settings::default();
        let audio = crate::ui::gallery::card_names(GalleryTab::AudioAnimate, &s);
        let video = crate::ui::gallery::card_names(GalleryTab::Animate, &s);
        for n in ["Sound Swell", "Sound Fade", "Duck", "Tremolo", "Pan Sweep", "Pan Swing"] {
            assert!(audio.iter().any(|a| a == n) && !video.iter().any(|v| v == n), "{n} is on the Audio side only");
        }
        assert!(video.iter().any(|v| v == "Pop In") && !audio.iter().any(|a| a == "Pop In"));
        for name in &audio {
            let (mut p, _) = av();
            let b = crate::model::find_block(&[], name).unwrap();
            assert!(p.apply_key_block(201, &b, Some(0.0), None).is_some(), "{name}");
            let c = p.clip(201).unwrap();
            assert!(c.volume.keys.len() + c.pan.keys.len() >= 2, "{name} keyed nothing");
            assert!(p.clip(101).unwrap().opacity.keys.is_empty(), "{name}");
        }
        let (mut p, _) = av();
        p.apply_key_block(201, &crate::model::find_block(&[], "Duck").unwrap(), Some(0.0), None).unwrap();
        assert!((p.clip(201).unwrap().volume.at(1.0) - 0.3).abs() < 1e-6, "ducked to 30 % mid-block");
        let (mut p, _) = av();
        p.apply_key_block(201, &crate::model::find_block(&[], "Pan Sweep").unwrap(), Some(0.0), None).unwrap();
        let pan = &p.clip(201).unwrap().pan;
        assert_eq!((pan.at(0.0), pan.at(2.0)), (-1.0, 1.0), "left to right");
    }
}
