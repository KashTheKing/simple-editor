//! ---- ws:parity-audio ----
//! Shotcut-parity audio, App side: MCP tools over a clip's own filters (`Clip.audio_filters`) and a
//! track's auto-fade / seams.

use super::tools_args::Args;
use super::tools_helpers::req;
use super::*;
use crate::mcp::tools::{ToolDef, ToolKind, ToolOutcome};
use crate::model::{AudioFilter, FilterKind};

/// A filter as the tools report it: kind, enabled and every parameter by name (its value at clip time 0
/// and whether it is keyed).
fn filter_json(f: &AudioFilter) -> Value {
    let params: serde_json::Map<String, Value> = f
        .kind
        .params()
        .iter()
        .enumerate()
        .map(|(i, s)| (s.name.to_string(), json!(f.params.get(i).map(|a| a.value).unwrap_or(s.default))))
        .collect();
    let keyed: Vec<&str> = f
        .kind
        .params()
        .iter()
        .zip(&f.params)
        .filter(|(_, a)| a.is_animated())
        .map(|(s, _)| s.name)
        .collect();
    json!({"kind": format!("{:?}", f.kind), "name": f.kind.name(), "enabled": f.enabled, "params": params, "keyed": keyed})
}

/// Write `{name: value}` overrides onto an existing filter (constant values; a keyed parameter gets the
/// value at clip time `t`).
fn set_params(f: &mut AudioFilter, params: &serde_json::Map<String, Value>, t: f64) -> Result<(), String> {
    f.fill_params();
    for (name, v) in params {
        let i = f
            .kind
            .params()
            .iter()
            .position(|s| s.name.eq_ignore_ascii_case(name))
            .ok_or_else(|| format!("unknown param '{name}' for {}", f.kind.name()))?;
        let v = v.as_f64().ok_or("param values must be numbers")?;
        f.params[i].set_at(t, v);
    }
    Ok(())
}

pub const TOOLS: &[ToolDef] = &[
    ToolDef {
        name: "clip.audio_filter_add",
        desc: "Add an audio filter to each clip's own chain (runs before the clip's volume, pan and fades, \
               then the clip goes to its bus). Kinds: every Mixer filter - Eq, Eq15, HighPass, LowPass, \
               BandPass, Notch, Compressor, Expander, NoiseGate, Limiter, DeEsser, DeHum, Declick, MidSide, \
               StereoWidth, Channels (Mode 0 copy L>R, 1 copy R>L, 2 swap, 3 invert L, 4 invert R, 5 invert \
               both), Downmix, Normalize (two-pass; see audio.normalize), Reverb, Echo, Pitch, Gain, ...",
        args: &[
            "clip_ids:array:true:clip ids",
            "kind:string:true:filter kind (name or variant, any case)",
            "params:object:false:{param name: value} overrides",
        ],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let a = Args(args);
            let ids = req(a.ids("clip_ids"), "clip_ids")?;
            let kind_s = req(a.str("kind"), "kind")?;
            let kind = FilterKind::parse(kind_s).ok_or_else(|| format!("unknown filter '{kind_s}'"))?;
            let f = AudioFilter::with_params(kind, args.get("params").and_then(|v| v.as_object()))?;
            let mut out = Vec::new();
            for id in ids {
                let c = app.project.clip_mut(id).ok_or_else(|| format!("no clip {id}"))?;
                c.audio_filters.push(f.clone());
                out.push(json!({"clip_id": id, "index": c.audio_filters.len() - 1}));
            }
            Ok(ToolOutcome::Done(json!({"ok": true, "added": out})))
        },
    },
    ToolDef {
        name: "clip.audio_filter_set",
        desc: "Change one of a clip's own audio filters: parameter values by name (a keyed parameter is \
               set at clip time t), and/or enabled.",
        args: &[
            "clip_id:integer:true:",
            "index:integer:true:position in the clip's chain (clip.audio_filters lists them)",
            "params:object:false:{param name: value}",
            "enabled:boolean:false:",
            "t:number:false:clip-local seconds for keyed parameters (default 0)",
        ],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let a = Args(args);
            let id = req(a.id("clip_id"), "clip_id")?;
            let i = req(a.id("index"), "index")? as usize;
            let t = a.f64("t").unwrap_or(0.0);
            let c = app.project.clip_mut(id).ok_or("no such clip")?;
            let f = c.audio_filters.get_mut(i).ok_or("no filter at that index")?;
            if let Some(p) = args.get("params").and_then(|v| v.as_object()) {
                set_params(f, p, t)?;
            }
            if let Some(e) = a.bool("enabled") {
                f.enabled = e;
            }
            Ok(ToolOutcome::Done(filter_json(f)))
        },
    },
    ToolDef {
        name: "clip.audio_filter_remove",
        desc: "Remove one of a clip's own audio filters (by its index), or all of them with index omitted.",
        args: &["clip_id:integer:true:", "index:integer:false:omit to remove every filter"],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let a = Args(args);
            let id = req(a.id("clip_id"), "clip_id")?;
            let c = app.project.clip_mut(id).ok_or("no such clip")?;
            match a.id("index") {
                Some(i) if (i as usize) < c.audio_filters.len() => {
                    c.audio_filters.remove(i as usize);
                }
                Some(_) => return Err("no filter at that index".into()),
                None => c.audio_filters.clear(),
            }
            Ok(ToolOutcome::Done(json!({"ok": true, "remaining": c.audio_filters.len()})))
        },
    },
    ToolDef {
        name: "clip.audio_filters",
        desc: "A clip's own audio filter chain, in order: kind, enabled, parameters by name, which are keyed.",
        args: &["clip_id:integer:true:"],
        kind: ToolKind::Read,
        run: |app, args| {
            let id = req(Args(args).id("clip_id"), "clip_id")?;
            let c = app.project.clip(id).ok_or("no such clip")?;
            Ok(ToolOutcome::Done(json!(c.audio_filters.iter().map(filter_json).collect::<Vec<_>>())))
        },
    },
    ToolDef {
        name: "track.audio_edges",
        desc: "An audio track's automatic fades at clip edges (fade_ms, 0 = off; Shotcut's track auto-fade) \
               and/or smooth seams (clips that touch cross-fade over 10 ms). Omitted values are kept.",
        args: &["track_index:integer:true:", "fade_ms:number:false:0..1000, 0 = off", "seam:boolean:false:"],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let a = Args(args);
            let ti = req(a.id("track_index"), "track_index")? as usize;
            let t = app.project.tracks.get(ti).ok_or("no such track")?;
            if t.kind != crate::model::TrackKind::Audio {
                return Err("auto-fade and seams are for audio tracks".into());
            }
            app.project.set_track_audio_edges(ti, a.f64("fade_ms").map(|f| f as f32), a.bool("seam"));
            let t = &app.project.tracks[ti];
            Ok(ToolOutcome::Done(json!({"ok": true, "fade_ms": t.auto_fade_ms, "seam": t.seam})))
        },
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tools_audio_parity_rows_resolve() {
        for t in TOOLS {
            assert_eq!(crate::mcp::tools::input_schema(t.args)["type"], "object", "{}", t.name);
            assert!(crate::mcp::tools::find(t.name).is_some(), "{} missing from mcp::tools::all()", t.name);
        }
        assert_eq!(crate::mcp::tools::find("clip.audio_filters").unwrap().kind, ToolKind::Read);
    }

    #[test]
    fn filter_json_and_set_params_round_trip() {
        let mut f = AudioFilter::new(FilterKind::Notch);
        set_params(&mut f, json!({"frequency": 60.0, "q": 10.0}).as_object().unwrap(), 0.0).unwrap();
        let j = filter_json(&f);
        assert_eq!(j["kind"], "Notch");
        assert_eq!(j["params"]["Frequency"], 60.0);
        assert_eq!(j["params"]["Q"], 10.0);
        assert!(set_params(&mut f, json!({"Gain": 1.0}).as_object().unwrap(), 0.0).is_err());
        // an old, shorter filter is topped up before a later parameter is written
        let mut short = AudioFilter::new(FilterKind::Expander);
        short.params.truncate(2);
        set_params(&mut short, json!({"Range dB": 20.0}).as_object().unwrap(), 0.0).unwrap();
        assert_eq!(short.params.len(), 5);
    }

    #[test]
    fn set_track_audio_edges_only_on_audio_tracks() {
        let mut p = Project::new();
        let ai = p.add_track(crate::model::TrackKind::Audio);
        let vi = p.add_track(crate::model::TrackKind::Video);
        assert!(p.set_track_audio_edges(ai, Some(10.0), None));
        assert!(!p.set_track_audio_edges(ai, Some(10.0), None), "no change");
        assert!(p.set_track_audio_edges(ai, None, Some(true)));
        assert_eq!((p.tracks[ai].auto_fade_ms, p.tracks[ai].seam), (10.0, true));
        assert!(p.set_track_audio_edges(ai, Some(5000.0), None));
        assert_eq!(p.tracks[ai].auto_fade_ms, 1000.0, "clamped");
        assert!(!p.set_track_audio_edges(vi, Some(10.0), Some(true)), "video tracks have no audio edges");
        // a track saved without the fields loads as off, and one with them round-trips
        let back: crate::model::Track = serde_json::from_value(serde_json::to_value(&p.tracks[ai]).unwrap()).unwrap();
        assert_eq!((back.auto_fade_ms, back.seam), (1000.0, true));
        let mut plain = serde_json::to_value(&p.tracks[vi]).unwrap();
        assert!(plain.get("seam").is_none() && plain.get("auto_fade_ms").is_none(), "off = not written");
        plain["kind"] = json!("Audio");
        let t: crate::model::Track = serde_json::from_value(plain).unwrap();
        assert_eq!((t.auto_fade_ms, t.seam), (0.0, false));
    }
}
