//! ---- ws:animator ----
//! Whole-timeline animation tools for an agent that only has tools: `animate.audit` (one compact read of what
//! is keyed, what blocks own, and how the cuts are set up), `keyblocks.convert_all` (hand keys -> blocks, motion
//! kept) and `animate.spans_from_transitions` (per-cut transitions -> spans). Also the filters of
//! `timeline.list` (`list_json`). The model side is `model::ops::convert`.

use super::tools_helpers::*;
use super::*;
use crate::mcp::tools::{ToolDef, ToolKind, ToolOutcome};
use crate::model::ops::convert::{clip_props, SpanOpts};
use crate::model::{Ease, TransitionEdge, ABUT_EPS};

/// Round to 1/1000 s (keeps the JSON short).
fn r3(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

/// `track` argument: a track index or a track name ("V1", "A2"), case-insensitive.
fn track_is(p: &Project, ti: usize, want: &Value) -> bool {
    match want {
        Value::Number(n) => n.as_u64() == Some(ti as u64),
        Value::String(s) => p.tracks[ti].name.eq_ignore_ascii_case(s.trim()) || s.trim().parse::<usize>().ok() == Some(ti),
        _ => true,
    }
}

fn kind_name(c: &Clip) -> String {
    format!("{:?}", c.kind)
}

/// The clips a call is about: `clip_ids`, or every clip matching `track` / `from`..`to` (overlapping) / `kind`,
/// or `all: true`. Err when none of those is given and `need_scope`.
fn scope(p: &Project, args: &Value, need_scope: bool) -> Result<Vec<(usize, Id)>, String> {
    if let Some(ids) = arg_ids(args, "clip_ids") {
        return Ok(ids.into_iter().filter_map(|id| p.find(id).map(|(ti, _)| (ti, id))).collect());
    }
    let (track, from, to, kind) = (args.get("track"), arg_f64(args, "from"), arg_f64(args, "to"), arg_str(args, "kind"));
    let all = arg_bool(args, "all").unwrap_or(false);
    if need_scope && !all && track.is_none() && from.is_none() && to.is_none() && kind.is_none() {
        return Err("say which clips: clip_ids, track, from/to (timeline s) or all: true".into());
    }
    let mut out = Vec::new();
    for (ti, t) in p.tracks.iter().enumerate() {
        if track.is_some_and(|w| !track_is(p, ti, w)) {
            continue;
        }
        for c in &t.clips {
            let on = from.map_or(true, |f| c.end() > f) && to.map_or(true, |e| c.start < e);
            if on && kind.map_or(true, |k| kind_name(c).eq_ignore_ascii_case(k)) {
                out.push((ti, c.id));
            }
        }
    }
    Ok(out)
}

fn page(args: &Value, def: usize) -> (usize, usize) {
    (arg_u64(args, "offset").unwrap_or(0) as usize, (arg_u64(args, "limit").unwrap_or(def as u64) as usize).clamp(1, 500))
}

fn ease_name(e: Option<Ease>) -> &'static str {
    e.map_or("design", |e| e.name())
}

/// The fields `timeline.list` gives per clip when `fields` is not set.
const LIST_FIELDS: [&str; 14] =
    ["id", "kind", "name", "asset", "sequence", "start", "duration", "src_in", "speed", "reverse", "freeze", "enabled", "label", "link"];

/// `timeline.list`: tracks and clips, filtered (`track`, `from`/`to`, `kind`, `has_keyframes`, `has_blocks`,
/// `clip_ids`), with only `fields`, `limit` clips from `offset`. No argument at all: every clip, as before.
pub(super) fn list_json(p: &Project, args: &Value) -> Value {
    let filtered = args.as_object().is_some_and(|o| !o.is_empty());
    let mut ids = scope(p, args, false).unwrap_or_default();
    let keyed = |c: &Clip| crate::model::BLOCK_PROPS.iter().any(|q| crate::model::prop_ref(c, q).is_some_and(|a| !a.keys.is_empty()));
    if let Some(want) = arg_bool(args, "has_keyframes") {
        ids.retain(|(_, id)| p.clip(*id).is_some_and(|c| keyed(c) == want));
    }
    if let Some(want) = arg_bool(args, "has_blocks") {
        ids.retain(|(_, id)| p.clip(*id).is_some_and(|c| !c.blocks.is_empty() == want));
    }
    let fields: Vec<String> = match args.get("fields").and_then(|v| v.as_array()) {
        Some(a) => a.iter().filter_map(|v| v.as_str()).map(String::from).collect(),
        None => LIST_FIELDS.iter().map(|s| s.to_string()).chain(["effects".to_string()]).collect(),
    };
    let total = ids.len();
    let (off, lim) = if filtered { page(args, 200) } else { (0, usize::MAX) };
    let shown: Vec<(usize, Id)> = ids.into_iter().skip(off).take(lim).collect();
    let row = |c: &Clip, ti: usize| -> Value {
        let mut o = serde_json::Map::new();
        for f in &fields {
            let v = match f.as_str() {
                "id" => json!(c.id),
                "kind" => json!(kind_name(c)),
                "name" => json!(c.name),
                "asset" => json!(c.asset),
                "sequence" => json!(c.sequence),
                "start" => json!(c.start),
                "duration" => json!(c.duration),
                "end" => json!(c.end()),
                "src_in" => json!(c.src_in),
                "speed" => json!(c.speed),
                "reverse" => json!(c.reverse),
                "keep_pitch" => json!(c.keep_pitch),
                "freeze" => json!(c.freeze),
                "enabled" => json!(c.enabled),
                "label" => json!(p.clip_label(c)),
                "link" => json!(c.link),
                "track" => json!(ti),
                "effects" => json!(c.effects.iter().map(|e| e.name()).collect::<Vec<_>>()),
                // property -> number of keys (and how many no block owns)
                "keys" => Value::Object(
                    c.key_audit().iter().map(|a| (a.prop.to_string(), json!({"n": a.keys, "hand": a.hand}))).collect(),
                ),
                "blocks" => json!(c.blocks.iter().map(|b| json!([b.name, r3(b.t), r3(b.dur)])).collect::<Vec<_>>()),
                _ => continue,
            };
            o.insert(f.clone(), v);
        }
        Value::Object(o)
    };
    let tracks: Vec<Value> = p
        .tracks
        .iter()
        .enumerate()
        .filter_map(|(ti, t)| {
            let clips: Vec<Value> =
                shown.iter().filter(|(i, _)| *i == ti).filter_map(|(_, id)| p.clip(*id)).map(|c| row(c, ti)).collect();
            (!filtered || !clips.is_empty())
                .then(|| json!({"index": ti, "id": t.id, "name": t.name, "kind": format!("{:?}", t.kind), "clips": clips}))
        })
        .collect();
    let mut out = json!({"editing_sequence": p.editing, "duration": p.duration(), "tracks": tracks});
    if filtered {
        out["total"] = json!(total);
        out["next_offset"] = if off + shown.len() < total { json!(off + shown.len()) } else { Value::Null };
    }
    out
}

fn span_plan_json(s: &crate::model::ops::convert::SpanPlan) -> Value {
    let mut v = json!({
        "t": r3(s.t), "dur": r3(s.dur), "kind": s.name, "cut_dur": s.cut_dur, "cuts": s.cuts.len(),
        "durations_before": [r3(s.before.0), r3(s.before.1)],
    });
    if s.hard > 0 {
        v["hard_cuts_taken"] = json!(s.hard);
    }
    if s.span_id != 0 {
        v["span_id"] = json!(s.span_id);
    }
    v
}

/// `animate.audit`: bounded whatever the project's size - totals, `limit` clip rows from `offset` (only clips
/// with keys or blocks), the cuts summary and up to 30 suggested spans.
pub(super) fn audit_json(p: &Project, args: &Value) -> Value {
    let ids = scope(p, args, false).unwrap_or_default();
    let (off, lim) = page(args, 40);
    let mut rows = Vec::new();
    let (mut keyed, mut hand_clips, mut block_clips, mut runs_total, mut blocks_total, mut listed) = (0, 0, 0, 0, 0, 0);
    let mut props: std::collections::BTreeMap<&str, usize> = Default::default();
    let mut named: std::collections::BTreeMap<String, usize> = Default::default();
    for (ti, id) in &ids {
        let Some(c) = p.clip(*id) else { continue };
        let ka = c.key_audit();
        if ka.is_empty() && c.blocks.is_empty() {
            continue;
        }
        let runs = c.hand_runs();
        keyed += (!ka.is_empty()) as usize;
        hand_clips += (!runs.is_empty()) as usize;
        block_clips += (!c.blocks.is_empty()) as usize;
        runs_total += runs.len();
        blocks_total += c.blocks.len();
        for a in ka.iter().filter(|a| a.hand > 0) {
            *props.entry(a.prop).or_default() += 1;
        }
        for b in &c.blocks {
            *named.entry(b.name.clone()).or_default() += 1;
        }
        listed += 1;
        if listed <= off || rows.len() >= lim {
            continue;
        }
        // property -> [keys, hand keys, first key t, last key t] (+ "linked")
        let keys: serde_json::Map<String, Value> = ka
            .iter()
            .map(|a| {
                let mut v = json!([a.keys, a.hand, r3(a.t0), r3(a.t1)]);
                if a.linked {
                    v.as_array_mut().unwrap().push(json!("linked"));
                }
                (a.prop.to_string(), v)
            })
            .collect();
        let mut row = json!({"id": c.id, "track": p.tracks[*ti].name, "kind": kind_name(c), "start": r3(c.start), "dur": r3(c.duration), "keys": keys});
        if !runs.is_empty() {
            row["hand_runs"] = json!(runs.iter().map(|r| json!([r3(r.t0), r3(r.t1), r.props])).collect::<Vec<_>>());
        }
        if !c.blocks.is_empty() {
            row["blocks"] = json!(c
                .blocks
                .iter()
                .map(|b| json!([b.name, r3(b.t), r3(b.dur), b.props, if b.ctl.as_ref().is_some_and(|c| !c.edited) { "live" } else { "plain" }]))
                .collect::<Vec<_>>());
        }
        rows.push(row);
    }
    // cuts
    let cuts = p.video_cuts();
    let mut kinds: Vec<(String, f64, usize)> = Vec::new();
    for x in cuts.iter().filter_map(|c| c.tr.as_ref()) {
        let (n, d) = (x.name(), r3(x.duration));
        match kinds.iter_mut().find(|k| k.0 == n && (k.1 - d).abs() < 1e-9) {
            Some(k) => k.2 += 1,
            None => kinds.push((n, d, 1)),
        }
    }
    kinds.sort_by(|a, b| b.2.cmp(&a.2));
    let edge = p.video_tracks().iter().map(|&ti| p.tracks[ti].transitions.iter().filter(|x| x.edge != TransitionEdge::Cut).count()).sum::<usize>();
    let gaps: usize = p
        .video_tracks()
        .iter()
        .map(|&ti| {
            let mut c: Vec<&Clip> = p.tracks[ti].clips.iter().collect();
            c.sort_by(|a, b| a.start.total_cmp(&b.start));
            c.windows(2).filter(|w| w[1].start - w[0].end() > ABUT_EPS).count()
        })
        .sum();
    let suggested = p.clone().spans_from_transitions(&SpanOpts::default(), true);
    let faded = cuts.iter().filter(|c| c.tr.is_some()).count();
    json!({
        "clips_total": ids.len(),
        "clips_with_keys": keyed,
        "clips_with_hand_keys": hand_clips,
        "clips_with_blocks": block_clips,
        "hand_runs": runs_total,
        "blocks": blocks_total,
        "blocks_by_name": named,
        "hand_keyed_props": props,
        "clips": rows,
        "clip_rows": "keys: prop -> [keys, hand keys (no block owns them), first t, last t]; hand_runs: [from, to, props] clip-local s; blocks: [name, t, dur, props, live|plain]",
        "next_offset": if off + rows.len() < listed { json!(off + rows.len()) } else { Value::Null },
        "transitions": {
            "cuts": cuts.len(),
            "with_transition": faded,
            "hard_cuts": cuts.len() - faded,
            "in_spans": cuts.iter().filter(|c| c.span.is_some()).count(),
            "spans": p.spans.len(),
            "edge_fades": edge,
            "gaps": gaps,
            "kinds": kinds.iter().take(12).map(|k| json!({"kind": k.0, "dur": k.1, "count": k.2})).collect::<Vec<_>>(),
            "suggested_spans": suggested.spans.iter().take(30).map(span_plan_json).collect::<Vec<_>>(),
            "suggested_spans_total": suggested.spans.len(),
        },
        "next": "keyblocks.convert_all {all: true} turns every hand run into blocks (motion kept); animate.spans_from_transitions {} groups the faded runs into spans, hard cuts stay hard. Both take dry_run: true to preview.",
    })
}

fn convert_json(r: &crate::model::ops::convert::ConvertReport, dry: bool) -> Value {
    let mut by: std::collections::BTreeMap<&str, usize> = Default::default();
    for m in &r.made {
        *by.entry(m.name.as_str()).or_default() += 1;
    }
    let mut clips: Vec<(Id, Vec<Value>)> = Vec::new();
    for m in &r.made {
        let mut b = json!([m.name, r3(m.t), r3(m.dur), m.props]);
        if m.named && ((m.strength - 1.0).abs() > 1e-6 || m.ease.is_some()) {
            b.as_array_mut().unwrap().push(json!({"strength": (m.strength * 1000.0).round() / 1000.0, "ease": ease_name(m.ease)}));
        }
        match clips.iter_mut().find(|c| c.0 == m.clip) {
            Some(c) => c.1.push(b),
            None => clips.push((m.clip, vec![b])),
        }
    }
    json!({
        "dry_run": dry,
        "clips_scanned": r.scanned,
        "clips_converted": r.clips,
        "blocks_made": r.made.len(),
        "named": r.made.iter().filter(|m| m.named).count(),
        "custom": r.made.iter().filter(|m| !m.named).count(),
        "by_name": by,
        "clips": clips.iter().take(40).map(|(id, b)| json!({"id": id, "blocks": b})).collect::<Vec<_>>(),
        "clips_more": clips.len().saturating_sub(40),
        "skipped": r.skipped.iter().take(20).map(|s| json!({"clip": s.clip, "t": r3(s.t), "why": s.why})).collect::<Vec<_>>(),
        "skipped_total": r.skipped.len(),
        "motion": "every property of every converted clip was sampled before and after: unchanged",
    })
}

fn span_opts(args: &Value) -> SpanOpts {
    SpanOpts {
        range: match (arg_f64(args, "from"), arg_f64(args, "to")) {
            (None, None) => None,
            (a, b) => Some((a.unwrap_or(0.0), b.unwrap_or(f64::INFINITY))),
        },
        merge_gap: arg_f64(args, "merge_gap"),
        include_hard_cuts: arg_bool(args, "include_hard_cuts").unwrap_or(false),
        tolerance: arg_f64(args, "tolerance").unwrap_or(0.3).clamp(0.0, 10.0),
    }
}

pub(super) fn spans_json(r: &crate::model::ops::convert::SpansReport, dry: bool) -> Value {
    json!({
        "dry_run": dry,
        "spans": r.spans.iter().map(span_plan_json).collect::<Vec<_>>(),
        "cuts_grouped": r.spans.iter().map(|s| s.cuts.len()).sum::<usize>(),
        "hard_cuts_left_hard": r.hard_cuts,
        "skipped": r.skipped.iter().take(20).map(|(t, why)| json!({"t": r3(*t), "why": why})).collect::<Vec<_>>(),
    })
}

pub const TOOLS: &[ToolDef] = &[
    ToolDef {
        name: "animate.audit",
        desc: "START HERE to convert keyframes to animation blocks or per-cut transitions to transition spans - one small read for the whole timeline (paged, never the full timeline.list). Per clip that has keys or blocks: keys {prop: [keys, hand keys no block owns, first t, last t]}, hand_runs [[from, to, props]] (what keyblocks.convert_all would turn into blocks), blocks [[name, t, dur, props, live|plain]]. Totals: clips with hand keys, blocks by name. transitions: cuts with / without a transition, kinds and lengths, edge fades, gaps, existing spans, and suggested_spans (what animate.spans_from_transitions would make). Filters: clip_ids, track (index or name), from/to (timeline s), kind; limit (default 40) / offset for more clip rows. Typical job in 3 calls: animate.audit -> keyblocks.convert_all {all: true} -> animate.spans_from_transitions {}.",
        args: &[
            "clip_ids:array:false:",
            "track:string:false:track index or name (V1, A2)",
            "from:number:false:timeline s",
            "to:number:false:timeline s",
            "kind:string:false:Video | Audio | Image | Text | Shape | ...",
            "limit:integer:false:clip rows (default 40)",
            "offset:integer:false:",
        ],
        kind: ToolKind::Read,
        run: |app, args| Ok(ToolOutcome::Done(audit_json(&app.project, args))),
    },
    ToolDef {
        name: "keyblocks.convert_all",
        desc: "Turn hand-made keyframes into animation blocks (the Animator panel's bars with controls) across many clips in ONE call and one undo step, the motion exactly unchanged. Each run of hand keys becomes a named block when one reproduces it exactly with its controls set (Fade In / Fade Out, Slide In/Out, Pop, Zoom Punch, Pulse, Shake, Ken Burns, Drift, Sound Swell / Fade, Duck, ...: strength, ease, origin / target), else a 'Custom' block that keeps the exact curve. Skips keys a block already owns, linked properties and keys past a clip's ends; every property of every converted clip is sampled before and after, and a clip whose motion would change is left as it was (listed in skipped). Scope (one is required): clip_ids, track, from/to (timeline s), kind, or all: true. dry_run: true reports what it would make without changing anything. Safe default for 'convert my keyframes to blocks': {all: true}.",
        args: &[
            "all:boolean:false:every clip on the timeline",
            "clip_ids:array:false:",
            "track:string:false:track index or name",
            "from:number:false:timeline s",
            "to:number:false:timeline s",
            "kind:string:false:clip kind",
            "dry_run:boolean:false:report only",
        ],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let ids: Vec<Id> = scope(&app.project, args, true)?.into_iter().map(|x| x.1).collect();
            let dry = arg_bool(args, "dry_run").unwrap_or(false);
            let r = app.project.convert_keys_to_blocks(&ids, dry);
            Ok(ToolOutcome::Done(convert_json(&r, dry)))
        },
    },
    ToolDef {
        name: "animate.spans_from_transitions",
        desc: "Simplify the transitions in ONE call and one undo step: every run of consecutive cuts (over all video tracks, in time order) that already carry the same transition (kind, ease, direction, shader) of a similar length becomes one transition span (animate.span_add) managing them, at one length (the run's median: tidies 0.33 / 0.35 / 0.4 into 0.35). Hard cuts end a run and stay hard (include_hard_cuts: true gives the hard cuts between faded ones the run's transition too); gaps and cuts already in a span end it too. Every span is tried first: one that would move a clip or touch another cut is split until it doesn't. Linked sound crossfades keep their shape. Options: from/to (timeline s), merge_gap (s: split a run where two cuts are further apart), tolerance (length difference that still joins, default 0.3 = 30%), dry_run. Safe default: {} (only the existing faded runs).",
        args: &[
            "from:number:false:timeline s",
            "to:number:false:timeline s",
            "merge_gap:number:false:seconds",
            "include_hard_cuts:boolean:false:default false",
            "tolerance:number:false:default 0.3",
            "dry_run:boolean:false:report only",
        ],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let dry = arg_bool(args, "dry_run").unwrap_or(false);
            if app.project.editing.is_some() {
                return Err("spans live on the main timeline: close the open sequence first (sequence.open)".into());
            }
            let r = app.project.spans_from_transitions(&span_opts(args), dry);
            Ok(ToolOutcome::Done(spans_json(&r, dry)))
        },
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    /// 200 clips on V1, each with a hand fade in and a fade out, every other one a cross fade before it.
    fn big() -> Project {
        let mut p = Project::new();
        let v1 = p.video_tracks()[0];
        for i in 0..200u64 {
            let mut c = Clip::new(1000 + i, ClipKind::Video, "a clip with a long-ish name", i as f64 * 3.0, 3.0);
            for (t, v) in [(0.0, 0.0), (0.3, 1.0), (2.7, 1.0), (3.0, 0.0)] {
                c.opacity.keys.push(crate::model::Keyframe { t, v, ease: Ease::Linear });
            }
            p.tracks[v1].clips.push(c);
        }
        for i in (1..200u64).step_by(2) {
            p.add_transition(1000 + i, TransitionKind::CrossFade, 0.35);
        }
        p
    }

    #[test]
    fn the_audit_stays_small_on_200_clips() {
        let p = big();
        let v = audit_json(&p, &json!({}));
        let n = v.to_string().len();
        assert!(n < 16_000, "audit is {n} bytes");
        assert_eq!(v["clips_with_hand_keys"], 200);
        assert_eq!(v["clips"].as_array().unwrap().len(), 40);
        assert_eq!(v["next_offset"], 40);
        assert_eq!(v["transitions"]["with_transition"], 100);
        // the next page
        let v2 = audit_json(&p, &json!({"offset": 40}));
        assert_eq!(v2["clips"][0]["id"], 1040);
        // the old full list of the same project is far bigger; the filtered one is paged
        let full = list_json(&p, &json!({})).to_string().len();
        let paged = list_json(&p, &json!({"has_keyframes": true, "fields": ["id", "start"], "limit": 20}));
        assert!(paged.to_string().len() < full / 10);
        assert_eq!(paged["total"], 200);
        assert_eq!(paged["next_offset"], 20);
    }

    #[test]
    fn timeline_list_filters() {
        let mut p = big();
        let a1 = p.add_track(TrackKind::Audio);
        p.tracks[a1].clips.push(Clip::new(5000, ClipKind::Audio, "music", 0.0, 30.0));
        let v = list_json(&p, &json!({"kind": "Audio", "fields": ["id"]}));
        assert_eq!(v["total"], 1);
        assert_eq!(v["tracks"][0]["clips"][0]["id"], 5000);
        let v = list_json(&p, &json!({"track": "V1", "from": 10.0, "to": 16.0, "fields": ["id", "keys"]}));
        assert_eq!(v["total"], 3); // 9..12, 12..15, 15..18
        assert_eq!(v["tracks"][0]["clips"][0]["keys"]["opacity"]["hand"], 4);
        let v = list_json(&p, &json!({"has_keyframes": false}));
        assert_eq!(v["total"], 1);
        // no arguments: as before, every clip and every track
        let v = list_json(&p, &json!({}));
        assert!(v.get("total").is_none());
        assert_eq!(v["tracks"].as_array().unwrap().len(), p.tracks.len());
    }

    #[test]
    fn convert_all_and_spans_on_the_big_project() {
        let mut p = big();
        let before = p.clone();
        let ids: Vec<Id> = scope(&p, &json!({"all": true}), true).unwrap().into_iter().map(|x| x.1).collect();
        assert!(scope(&p, &json!({}), true).is_err(), "a scope is required");
        let r = p.convert_keys_to_blocks(&ids, false);
        assert_eq!(r.clips, 200);
        let j = convert_json(&r, false);
        assert_eq!(j["by_name"]["Fade In"], 200);
        assert_eq!(j["by_name"]["Fade Out"], 200);
        assert!(j.to_string().len() < 12_000);
        for (a, b) in before.tracks[0].clips.iter().zip(&p.tracks[0].clips) {
            assert!(crate::model::ops::convert::same_motion(a, b, clip_props(a)));
        }
        // transitions on every other cut: no two faded cuts are consecutive, so each is its own span; the hard
        // cuts between them stay hard
        let s = p.spans_from_transitions(&SpanOpts::default(), false);
        assert_eq!(s.spans.len(), 100);
        assert_eq!(s.hard_cuts, 99);
        assert!(spans_json(&s, false).to_string().len() < 20_000);
    }
}
