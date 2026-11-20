//! ---- ws:time-remap ----
//! MCP surface of time remapping (`model::timemap`, `Project::time_remap_*`): one tool, `clip.time_remap`,
//! that reads the keys or edits them (one undo step per call).

use super::tools_helpers::*;
use super::*;
use crate::mcp::tools::{ToolDef, ToolKind, ToolOutcome};
use crate::model::TimeKey;

/// The map as the tool reports it: keys (clip-local t, source s, smooth), duration and the speed of each
/// segment (signed; 0 = hold).
pub(super) fn describe(c: &crate::model::Clip) -> Value {
    let Some(m) = &c.time_map else {
        return json!({"enabled": false, "speed": c.speed, "reverse": c.reverse, "freeze": c.freeze});
    };
    let keys: Vec<Value> = m.keys.iter().map(|k| json!({"t": k.t, "s": k.s, "smooth": k.smooth})).collect();
    let segs: Vec<Value> = m
        .keys
        .windows(2)
        .map(|w| {
            let v = (w[1].s - w[0].s) / (w[1].t - w[0].t).max(1e-12);
            json!({"from": w[0].t, "to": w[1].t, "speed": v, "kind": if v.abs() < crate::model::STILL { "hold" } else if v < 0.0 { "reverse" } else { "forward" }})
        })
        .collect();
    json!({"enabled": true, "start": c.start, "duration": c.duration, "keys": keys, "segments": segs})
}

fn parse_keys(v: &Value) -> Result<Vec<TimeKey>, String> {
    let arr = v.as_array().ok_or("keys must be an array of {t, s, smooth?}")?;
    arr.iter()
        .map(|k| {
            let t = k.get("t").and_then(Value::as_f64).ok_or("each key needs a number t")?;
            let s = k.get("s").and_then(Value::as_f64).ok_or("each key needs a number s")?;
            let smooth = k.get("smooth").and_then(Value::as_bool).unwrap_or(true);
            Ok(TimeKey { smooth, ..TimeKey::new(t, s) })
        })
        .collect()
}

fn run(app: &mut App, args: &Value) -> Result<ToolOutcome, String> {
    let id = req(arg_u64(args, "clip_id"), "clip_id")?;
    let action = arg_str(args, "action").unwrap_or("get");
    let playhead = app.playhead;
    let p = &mut app.project;
    let at = |_: &Project| -> Result<f64, String> { Ok(arg_f64(args, "t").unwrap_or(playhead)) };
    let index = || arg_u64(args, "index").map(|i| i as usize).ok_or("index (segment or key) is required");
    let delta = match action {
        "get" => 0.0,
        "enable" => {
            if !p.time_remap_enable(id) && p.clip(id).is_some_and(|c| c.time_map.is_none()) {
                return Err("this clip cannot be remapped (video, audio or sequence clips only, unlocked track)".into());
            }
            0.0
        }
        "disable" => {
            p.time_remap_disable(id);
            0.0
        }
        "set" => p.time_remap_set_keys(id, parse_keys(req(args.get("keys"), "keys")?)?)?,
        "hold" => {
            let t = at(p)?;
            p.time_remap_hold(id, t, arg_f64(args, "len").unwrap_or(1.0))?
        }
        "reverse" => {
            let t = at(p)?;
            p.time_remap_reverse(id, t)?
        }
        "add_key" => {
            let t = at(p)?;
            p.time_remap_add_key(id, t)?;
            0.0
        }
        "remove_key" => {
            let i = index()?;
            p.time_remap_edit(id, |m| {
                m.remove_key(i);
            })?
        }
        "speed" => {
            let (i, v) = (index()?, req(arg_f64(args, "speed"), "speed")?);
            p.time_remap_edit(id, |m| {
                m.set_segment_speed(i, v);
            })?
        }
        "length" => {
            let (i, v) = (index()?, req(arg_f64(args, "len"), "len")?);
            p.time_remap_edit(id, |m| {
                m.set_segment_len(i, v);
            })?
        }
        "smooth" => {
            let (i, on) = (index()?, arg_bool(args, "smooth").unwrap_or(true));
            p.time_remap_edit(id, |m| m.set_smooth(i, on))?
        }
        other => return Err(format!("unknown action '{other}'")),
    };
    let c = app.project.clip(id).ok_or("no such clip")?;
    let mut v = describe(c);
    v["delta"] = json!(delta);
    Ok(ToolOutcome::Done(v))
}

pub const TOOLS: &[ToolDef] = &[ToolDef {
    name: "clip.time_remap",
    desc: "Time remapping: a clip's source time as a function of its own time. action: get (default) | enable (keys that play what it plays now) | disable (back to one constant speed) | set (keys=[{t clip-local s, s source s, smooth}], the last key is the clip's length) | hold (freeze at timeline t for len s) | reverse (the stretch around timeline t plays back, then forward again) | add_key (at timeline t, changes nothing) | remove_key (index) | speed (segment index, speed: 0.5 = half) | length (segment index, len s) | smooth (segment index, smooth=true/false). Length changes ripple the clip's track; a linked picture/sound pair is edited together. Replies with keys, segments (speed, hold/reverse/forward) and delta (length change).",
    args: &[
        "clip_id:integer:true:the clip",
        "action:string:false:get|enable|disable|set|hold|reverse|add_key|remove_key|speed|length|smooth",
        "t:number:false:timeline seconds (hold / reverse / add_key; default the playhead)",
        "len:number:false:seconds (hold: freeze length, default 1; length: segment length)",
        "index:integer:false:segment (from key index to the next) or key index",
        "speed:number:false:segment speed (1 = normal)",
        "smooth:boolean:false:segment eases (true) or keeps one speed (false)",
        "keys:array:false:set: [{t, s, smooth}]",
    ],
    kind: ToolKind::Mutate,
    run,
}];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_remap_tool_is_registered_as_a_mutation() {
        let def = crate::mcp::tools::find("clip.time_remap").expect("registered");
        assert_eq!(def.kind, ToolKind::Mutate);
        let keys = parse_keys(&json!([{"t": 0.0, "s": 1.0}, {"t": 2.0, "s": 1.0, "smooth": false}])).unwrap();
        assert!(keys[0].smooth && !keys[1].smooth);
        assert!(parse_keys(&json!([{"t": 0.0}])).is_err());
    }
}
