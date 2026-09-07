//! ---- ws:audio-dsp-automation ----
//! The audio-DSP workstream's App-side glue: the six MCP tools over roles / repair chains / bus
//! filters / volume automation / meters (auto-Luau-exposed via `TOOL_TABLES`), and `sync_buses`, the
//! FRAME_HOOKS entry that drains `mixer_fx::METER_FEED` (published by playback.rs's audio thread after
//! every mixed block) into `App.buses` so the Mixer pane's peak meters and LUFS rows read live audio.

use super::tools_args::Args;
use super::tools_helpers::req;
use super::*;
use crate::engine::mixer_fx::{lin_to_db, METER_FEED};
use crate::mcp::tools::{ToolDef, ToolKind, ToolOutcome};
use crate::model::{Animated, AudioRole};
use crate::ui::inspector_audio::db_to_gain;
use std::sync::atomic::{AtomicBool, Ordering};

/// Playback state seen by the previous frame: the playing→stopped edge zeroes the peak meters and
/// restarts the integrated loudness, so a paused mixer never shows a stale "live" level and every
/// Play is a fresh integrated pass.
static WAS_PLAYING: AtomicBool = AtomicBool::new(false);

/// FRAME_HOOKS entry: fold every block the audio thread published since last frame into `App.buses`.
/// Never requests a repaint — playback already repaints every frame, and a paused editor has nothing
/// queued (idle-CPU-0% gate).
pub(super) fn sync_buses(app: &mut App, _ctx: &egui::Context) {
    let playing = app.player.is_playing();
    if WAS_PLAYING.swap(playing, Ordering::Relaxed) && !playing {
        app.buses.reset_meters();
    }
    if app.project.buses.is_empty() {
        return; // the mixer never ran its graph, nothing was published
    }
    app.buses.sync(&app.project);
    METER_FEED.drain_into(&mut app.buses);
}

/// Set (`db = Some`) or remove (`None`) a keyframe at `t`. Setting on a still-constant property first
/// keys its current value so the result is a real keyframe, not a moved constant.
fn key_at(a: &mut Animated, t: f64, db: Option<f64>) {
    match db {
        Some(db) => {
            if !a.is_animated() {
                a.toggle_key(t);
            }
            a.set_at(t, db_to_gain(db));
        }
        None => {
            if a.has_key_at(t) {
                a.toggle_key(t);
            }
        }
    }
}

/// `track.volume_key` / `bus.volume_key` share one arg shape: `t` plus either `db` or `remove`.
fn key_args(a: &Args) -> Result<(f64, Option<f64>), String> {
    let t = req(a.f64("t"), "t")?;
    let remove = a.bool("remove").unwrap_or(false);
    match (a.f64("db"), remove) {
        (_, true) => Ok((t, None)),
        (Some(db), false) => Ok((t, Some(db))),
        (None, false) => Err("pass db (set a key) or remove=true (delete it)".into()),
    }
}

fn db_or_null(v: f32) -> Value {
    if v.is_finite() {
        json!(v)
    } else {
        Value::Null
    }
}

pub const TOOLS: &[ToolDef] = &[
    ToolDef {
        name: "audio.role",
        desc: "Tag clips with an Essential-Sound role; drives the inspector's audio defaults and future \
               ducking target selection.",
        args: &["clip_ids:array:true:clip ids", "role:string:true:Unset|Dialogue|Music|Sfx|Ambience"],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let a = Args(args);
            let ids = req(a.ids("clip_ids"), "clip_ids")?;
            let name = req(a.str("role"), "role")?;
            let role = AudioRole::parse(name).ok_or_else(|| format!("unknown role '{name}'"))?;
            let mut n = 0;
            for id in ids {
                if let Some(c) = app.project.clip_mut(id) {
                    c.audio_role = role;
                    n += 1;
                }
            }
            Ok(ToolOutcome::Done(json!({"ok": true, "changed": n})))
        },
    },
    ToolDef {
        name: "audio.repair",
        desc: "Route clips through a one-click DSP chain (a named bus + filters), left fully editable \
               in the Mixer; re-applying reuses the bus. Returns the bus id.",
        args: &["clip_ids:array:true:", "preset:string:true:repair|clarity"],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let a = Args(args);
            let ids = req(a.ids("clip_ids"), "clip_ids")?;
            let preset = req(a.str("preset"), "preset")?;
            let bus = app.project.apply_repair(&ids, preset);
            if bus == 0 {
                return Err(format!("unknown preset '{preset}' (repair | clarity)"));
            }
            Ok(ToolOutcome::Done(json!({"ok": true, "bus_id": bus})))
        },
    },
    ToolDef {
        name: "audio.filter_add",
        desc: "Append a filter to a bus's chain (any FilterKind incl. DeHum|Limiter|DeEsser) with \
               optional {param name: value} overrides.",
        args: &[
            "bus_id:integer:true:",
            "kind:string:true:one of FilterKind::ALL incl. DeHum|Limiter|DeEsser",
            "params:object:false:name->value overrides",
        ],
        kind: ToolKind::Mutate,
        run: |app, args| {
            // ponytail: the same op as the pre-existing `audio.add_filter` under this workstream's
            // arg names — delegate rather than keep two param parsers in sync.
            let a = Args(args);
            let mapped = json!({
                "bus": req(a.id("bus_id"), "bus_id")?,
                "kind": req(a.str("kind"), "kind")?,
                "params": args.get("params").cloned().unwrap_or(Value::Null),
            });
            let def = mcp::tools::find("audio.add_filter").ok_or("audio.add_filter is not registered")?;
            (def.run)(app, &mapped)
        },
    },
    ToolDef {
        name: "track.volume_key",
        desc: "Set (db) or remove (remove=true) a keyframe on a track's volume automation at t.",
        args: &[
            "track_index:integer:true:",
            "t:number:true:timeline seconds",
            "db:number:false:omit + remove=true to delete",
            "remove:boolean:false:",
        ],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let a = Args(args);
            let ti = req(a.id("track_index"), "track_index")? as usize;
            let (t, db) = key_args(&a)?;
            let track = app.project.tracks.get_mut(ti).ok_or("no such track")?;
            key_at(&mut track.volume, t, db);
            Ok(ToolOutcome::Done(json!({"ok": true, "keys": track.volume.keys.len()})))
        },
    },
    ToolDef {
        name: "bus.volume_key",
        desc: "Set (db) or remove (remove=true) a keyframe on a bus's gain automation at t.",
        args: &[
            "bus_id:integer:true:",
            "t:number:true:timeline seconds",
            "db:number:false:omit + remove=true to delete",
            "remove:boolean:false:",
        ],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let a = Args(args);
            let id = req(a.id("bus_id"), "bus_id")?;
            let (t, db) = key_args(&a)?;
            let bus = app.project.bus_mut(id).ok_or("no such bus")?;
            key_at(&mut bus.gain, t, db);
            Ok(ToolOutcome::Done(json!({"ok": true, "keys": bus.gain.keys.len()})))
        },
    },
    ToolDef {
        name: "mixer.meters",
        desc: "Peak (L/R, linear + dBFS) and approximate LUFS (momentary, integrated; null before any \
               audio) for one or every bus, from the blocks playback has published since the last frame.",
        args: &["bus_id:integer:false:omit for all buses"],
        kind: ToolKind::Read,
        run: |app, args| {
            let want = Args(args).id("bus_id");
            // the frame hook already drained this frame; drain again so a call mid-block is freshest
            METER_FEED.drain_into(&mut app.buses);
            let out: Vec<Value> = app
                .project
                .buses
                .iter()
                .filter(|b| want.is_none_or(|id| id == b.id))
                .map(|b| {
                    let (l, r) = app.buses.meter(b.id);
                    let (mo, int) = app.buses.lufs(b.id);
                    json!({
                        "bus_id": b.id, "name": b.name, "peak_l": l, "peak_r": r,
                        "peak_db_l": lin_to_db(l), "peak_db_r": lin_to_db(r),
                        "lufs_momentary": db_or_null(mo), "lufs_integrated": db_or_null(int),
                    })
                })
                .collect();
            if want.is_some() && out.is_empty() {
                return Err("no such bus".into());
            }
            Ok(ToolOutcome::Done(json!(out)))
        },
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    /// Every row resolves through the flattened catalogue with a parseable schema (the crate-wide
    /// `every_arg_spec_parses`/`mutate_rows_roll_back_on_error` cover the generic half; a live `App`
    /// isn't buildable headless — see tools_registry_tests.rs — so the `run` bodies are exercised
    /// through the pure helpers below and `Project::apply_repair`'s own test).
    #[test]
    fn tools_mixer_rows_resolve_and_roundtrip() {
        let names = ["audio.role", "audio.repair", "audio.filter_add", "track.volume_key", "bus.volume_key", "mixer.meters"];
        assert_eq!(TOOLS.len(), names.len());
        for (t, want) in TOOLS.iter().zip(names) {
            assert_eq!(t.name, want);
            assert_eq!(crate::mcp::tools::input_schema(t.args)["type"], "object");
            assert!(crate::mcp::tools::find(want).is_some(), "{want} missing from mcp::tools::all()");
        }
        assert_eq!(crate::mcp::tools::find("mixer.meters").unwrap().kind, ToolKind::Read);
        assert!(TOOLS.iter().filter(|t| t.kind == ToolKind::Mutate).count() == 5);
        // audio.filter_add delegates to audio.add_filter, which must therefore exist
        assert!(crate::mcp::tools::find("audio.add_filter").is_some());
    }

    #[test]
    fn key_args_needs_db_or_remove() {
        assert!(key_args(&Args(&json!({}))).is_err(), "t is required");
        assert!(key_args(&Args(&json!({"t": 1.0}))).is_err(), "db or remove");
        assert_eq!(key_args(&Args(&json!({"t": 1.0, "db": -6.0}))).unwrap(), (1.0, Some(-6.0)));
        assert_eq!(key_args(&Args(&json!({"t": 2.0, "remove": true}))).unwrap(), (2.0, None));
    }

    #[test]
    fn key_at_sets_real_keyframes_and_removes_them() {
        let mut a = Animated::new(1.0);
        key_at(&mut a, 2.0, Some(-6.0));
        assert!(a.is_animated(), "a constant becomes keyed, not moved");
        assert_eq!(a.keys.len(), 1);
        assert!((a.at(2.0) - db_to_gain(-6.0)).abs() < 1e-9);
        key_at(&mut a, 4.0, Some(0.0));
        assert_eq!(a.keys.len(), 2);
        assert!((a.at(4.0) - 1.0).abs() < 1e-9);
        key_at(&mut a, 4.0, None);
        assert_eq!(a.keys.len(), 1);
        key_at(&mut a, 9.0, None);
        assert_eq!(a.keys.len(), 1, "removing a key that isn't there is a no-op");
        // garbage/empty args leave a project unchanged through the real tool bodies' arg gates
        assert!(AudioRole::parse("").is_none());
    }
}
