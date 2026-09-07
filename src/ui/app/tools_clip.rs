use super::tools_helpers::*;
use super::*;

pub(super) fn dispatch(app: &mut App, name: &str, args: &Value) -> Option<Result<Value, String>> {
    let prefix = name.split('.').next().unwrap_or("");
    if !matches!(prefix, "clip" | "audio" | "shapes" | "style" | "container") {
        return None;
    }
    pub(super) fn run(app: &mut App, name: &str, args: &Value) -> Result<Value, String> {
        match name {
            "clip.set" => {
                let id = req(arg_u64(args, "clip_id"), "clip_id")?;
                let fields = req(args.get("fields"), "fields")?.clone();
                // speed/reverse go through Project::set_speed so neighbour collisions are respected
                let speed = arg_f64(&fields, "speed");
                let reverse = arg_bool(&fields, "reverse");
                {
                    let c = app.project.clip_mut(id).ok_or("no such clip")?;
                    apply_clip_fields(c, &fields)?;
                }
                if speed.is_some() || reverse.is_some() {
                    let cur = app.project.clip(id).ok_or("no such clip")?;
                    let (s, r) = (speed.unwrap_or(cur.speed), reverse.unwrap_or(cur.reverse));
                    if !app.project.set_speed(&[id], s, r) {
                        return Err("speed change blocked by a neighbouring clip".into());
                    }
                }
                Ok(json!({"ok": true}))
            }
            "clip.keyframe" => {
                let id = req(arg_u64(args, "clip_id"), "clip_id")?;
                let prop = req(arg_str(args, "property"), "property")?.to_string();
                let t = req(arg_f64(args, "t"), "t")?;
                let remove = arg_bool(args, "remove").unwrap_or(false);
                let value = arg_f64(args, "value");
                let ease = arg_str(args, "ease").map(|s| parse_ease(s).ok_or(format!("bad ease '{s}'"))).transpose()?;
                let c = app.project.clip_mut(id).ok_or("no such clip")?;
                let a = anim_of(c, &prop).ok_or_else(|| format!("unknown property '{prop}'"))?;
                if remove {
                    if let Some(i) = a.key_index_at(t) {
                        a.keys.remove(i);
                    }
                } else {
                    if !a.is_animated() {
                        a.toggle_key(t); // first key: set_at alone would only change the constant value
                    }
                    a.set_at(t, value.unwrap_or_else(|| a.at(t)));
                    if let Some(e) = ease {
                        a.set_ease_at(t, e);
                    }
                }
                Ok(json!({"ok": true}))
            }
            "clip.add_effect" => {
                let id = req(arg_u64(args, "clip_id"), "clip_id")?;
                let kind_s = req(arg_str(args, "kind"), "kind")?;
                let kind = EffectKind::ALL
                    .into_iter()
                    .find(|k| k.name().eq_ignore_ascii_case(kind_s) || format!("{k:?}").eq_ignore_ascii_case(kind_s))
                    .ok_or_else(|| format!("unknown effect '{kind_s}'"))?;
                let mut effect = crate::model::Effect::new(kind);
                if let Some(params) = args.get("params").and_then(|v| v.as_object()) {
                    for (pname, pval) in params {
                        let i = kind
                            .params()
                            .iter()
                            .position(|s| s.name.eq_ignore_ascii_case(pname))
                            .ok_or_else(|| format!("unknown param '{pname}' for {}", kind.name()))?;
                        effect.params[i].value = pval.as_f64().ok_or("param values must be numbers")?;
                    }
                }
                let c = app.project.clip_mut(id).ok_or("no such clip")?;
                c.effects.push(effect);
                Ok(json!({"ok": true, "index": c.effects.len() - 1}))
            }
            "clip.remove_effect" => {
                let id = req(arg_u64(args, "clip_id"), "clip_id")?;
                let i = req(arg_u64(args, "index"), "index")? as usize;
                let c = app.project.clip_mut(id).ok_or("no such clip")?;
                if i >= c.effects.len() {
                    return Err("no effect at that index".into());
                }
                c.effects.remove(i);
                Ok(json!({"ok": true}))
            }
            "clip.apply_motion" => {
                let id = req(arg_u64(args, "clip_id"), "clip_id")?;
                let name = req(arg_str(args, "name"), "name")?;
                let scaled = arg_bool(args, "scaled").unwrap_or(true);
                let preset = app
                    .settings
                    .motion_presets
                    .iter()
                    .find(|p| p.name.eq_ignore_ascii_case(name))
                    .cloned()
                    .or_else(|| {
                        crate::engine::presets::builtin_motions()
                            .into_iter()
                            .find(|p| p.name.eq_ignore_ascii_case(name))
                    })
                    .ok_or_else(|| format!("no motion preset '{name}'"))?;
                let c = app.project.clip_mut(id).ok_or("no such clip")?;
                crate::engine::presets::apply_motion(&preset, c, scaled);
                Ok(json!({"ok": true}))
            }
            "style.summary" => Ok(json!({"markdown": crate::engine::style::style_summary(&app.export_project())})),
            "clip.add_mask" => {
                let id = req(arg_u64(args, "clip_id"), "clip_id")?;
                let shape = mask_shape(arg_str(args, "shape").unwrap_or("Ellipse"))?;
                let slot = mask_slot(&mut app.project, id, arg_u64(args, "effect").map(|i| i as usize))?;
                if slot.is_some() {
                    return Err("that clip / effect already has a mask".into());
                }
                *slot = Some(Mask::new(shape));
                Ok(json!({"ok": true}))
            }
            "clip.set_mask" => {
                let id = req(arg_u64(args, "clip_id"), "clip_id")?;
                let fields = req(args.get("fields"), "fields")?.clone();
                let slot = mask_slot(&mut app.project, id, arg_u64(args, "effect").map(|i| i as usize))?;
                let mask = slot.as_mut().ok_or("no mask on that clip / effect (call clip.add_mask first)")?;
                apply_mask_fields(mask, &fields)?;
                Ok(json!({"ok": true}))
            }
            "clip.add_node" => {
                let id = req(arg_u64(args, "clip_id"), "clip_id")?;
                let kind = node_kind(req(arg_str(args, "kind"), "kind")?)?;
                let (x, y) = (arg_f64(args, "x").unwrap_or(160.0) as f32, arg_f64(args, "y").unwrap_or(120.0) as f32);
                let node = app.project.add_node(id, kind, x, y).ok_or("no such clip")?;
                Ok(json!({"ok": true, "node_id": node}))
            }
            "clip.connect_nodes" => {
                let id = req(arg_u64(args, "clip_id"), "clip_id")?;
                let from = req(arg_u64(args, "from"), "from")?;
                let to = req(arg_u64(args, "to"), "to")?;
                let port = arg_u64(args, "port").unwrap_or(0) as usize;
                app.project.ensure_graph(id);
                let g = app.project.clip_mut(id).and_then(|c| c.graph.as_mut()).ok_or("no such clip")?;
                if !g.connect(from, to, port) {
                    return Err("connection refused (unknown node, bad port or a cycle)".into());
                }
                Ok(json!({"ok": true}))
            }
            "audio.buses" => {
                app.project.main_bus();
                let list: Vec<Value> = app
                    .project
                    .buses
                    .iter()
                    .map(|b| {
                        json!({"id": b.id, "name": b.name, "gain": b.gain.value, "pan": b.pan.value,
                               "muted": b.muted, "solo": b.solo, "mono": b.mono, "output": b.output,
                               "filters": b.filters.iter().map(|f| f.kind.name()).collect::<Vec<_>>()})
                    })
                    .collect();
                Ok(json!(list))
            }
            "audio.add_bus" => {
                let name = arg_str(args, "name").unwrap_or("Bus").to_string();
                let id = app.project.add_bus(name);
                Ok(json!({"ok": true, "bus_id": id}))
            }
            "audio.add_filter" => {
                let bus = req(arg_u64(args, "bus"), "bus")?;
                let kind_s = req(arg_str(args, "kind"), "kind")?;
                let kind = FilterKind::ALL
                    .into_iter()
                    .find(|k| k.name().eq_ignore_ascii_case(kind_s) || format!("{k:?}").eq_ignore_ascii_case(kind_s))
                    .ok_or_else(|| format!("unknown filter '{kind_s}'"))?;
                let mut f = crate::model::AudioFilter::new(kind);
                if let Some(params) = args.get("params").and_then(|v| v.as_object()) {
                    for (pname, pval) in params {
                        let i = kind
                            .params()
                            .iter()
                            .position(|s| s.name.eq_ignore_ascii_case(pname))
                            .ok_or_else(|| format!("unknown param '{pname}' for {}", kind.name()))?;
                        f.params[i].value = pval.as_f64().ok_or("param values must be numbers")?;
                    }
                }
                let b = app.project.bus_mut(bus).ok_or("no such bus")?;
                b.filters.push(f);
                Ok(json!({"ok": true, "index": b.filters.len() - 1}))
            }
            "audio.route" => {
                let bus = req(arg_u64(args, "bus"), "bus")?;
                if bus != 0 && app.project.bus(bus).is_none() {
                    return Err("no such bus".into());
                }
                if let Some(clip) = arg_u64(args, "clip_id") {
                    app.project.clip_mut(clip).ok_or("no such clip")?.bus = bus;
                } else if let Some(track) = arg_u64(args, "track") {
                    let t = app.project.tracks.get_mut(track as usize).ok_or("no such track")?;
                    t.bus = bus;
                } else if let Some(from) = arg_u64(args, "from_bus") {
                    let main = app.project.main_bus();
                    if from == main {
                        return Err("the Main bus has no output".into());
                    }
                    app.project.bus_mut(from).ok_or("no such bus")?.output = bus;
                } else {
                    return Err("pass clip_id, track or from_bus".into());
                }
                Ok(json!({"ok": true}))
            }
            "shapes.add" => {
                let kind_s = arg_str(args, "kind").unwrap_or("Rect");
                let kind = ShapeKind::ALL
                    .into_iter()
                    .find(|k| k.name().eq_ignore_ascii_case(kind_s) || format!("{k:?}").eq_ignore_ascii_case(kind_s))
                    .ok_or_else(|| format!("unknown shape '{kind_s}'"))?;
                let at = req(arg_f64(args, "at"), "at")?;
                let dur = arg_f64(args, "duration").unwrap_or(5.0).max(0.1);
                let id = app.project.add_shape_clip(kind, at, dur);
                if let Some(s) = app.project.clip_mut(id).and_then(|c| c.shape.as_mut()) {
                    if let Some(c) = args.get("fill").and_then(color_arg) {
                        s.fill = c;
                    }
                    if let Some(c) = args.get("stroke").and_then(color_arg) {
                        s.stroke = c;
                    }
                    if let Some(w) = arg_f64(args, "stroke_width") {
                        s.stroke_width = w as f32;
                    }
                    if let Some(n) = arg_u64(args, "sides") {
                        s.sides = n.clamp(3, 64) as u32;
                    }
                    if let Some(w) = arg_f64(args, "width") {
                        s.w.value = w;
                    }
                    if let Some(h) = arg_f64(args, "height") {
                        s.h.value = h;
                    }
                }
                Ok(json!({"ok": true, "clip_id": id}))
            }
            "container.add" => {
                let at = req(arg_f64(args, "at"), "at")?;
                let dur = arg_f64(args, "duration").unwrap_or(5.0).max(0.1);
                let (vid, aid) = app.project.add_container_clip(at, dur);
                if let Some(lbl) = arg_str(args, "label") {
                    if let Some(vc) = app.project.clip_mut(vid) {
                        vc.container_label = lbl.to_string();
                    }
                    if let Some(ac) = app.project.clip_mut(aid) {
                        ac.container_label = lbl.to_string();
                    }
                }
                Ok(json!({"ok": true, "video_clip_id": vid, "audio_clip_id": aid}))
            }
            "container.replace" => {
                let clip_id = req(arg_u64(args, "clip_id"), "clip_id")?;
                let asset_id = req(arg_u64(args, "asset_id"), "asset_id")?;
                let pair = arg_bool(args, "pair").unwrap_or(false);
                let ok = if pair {
                    app.project.replace_container_pair(clip_id, asset_id)
                } else {
                    app.project.replace_container_media(clip_id, asset_id)
                };
                if ok {
                    Ok(json!({"ok": true}))
                } else {
                    Err("failed to replace (clip not a container or asset not found)".into())
                }
            }
            "container.make" => {
                let ids = req(arg_ids(args, "clip_ids"), "clip_ids")?;
                app.project.make_container(&ids);
                Ok(json!({"ok": true}))
            }
            "container.unmake" => {
                let ids = req(arg_ids(args, "clip_ids"), "clip_ids")?;
                app.project.unmake_container(&ids);
                Ok(json!({"ok": true}))
            }
            "container.list" => {
                let containers: Vec<Value> = app
                    .project
                    .all_clips()
                    .filter(|(_, c)| c.container)
                    .map(|(ti, c)| {
                        json!({
                            "clip_id": c.id,
                            "track_index": ti,
                            "kind": format!("{:?}", c.kind),
                            "name": c.name,
                            "label": c.container_label,
                            "is_empty": c.is_empty_container(),
                            "asset_id": c.asset,
                            "start": c.start,
                            "duration": c.duration,
                            "link": c.link,
                        })
                    })
                    .collect();
                Ok(json!(containers))
            }
            _ => unreachable!(),
        }
    }
    Some(run(app, name, args))
}

// ---- ws:registries-schema-hooks ----
use crate::mcp::tools::{ToolDef, ToolKind, ToolOutcome};

macro_rules! row {
    ($name:literal, $kind:expr, $desc:literal, $args:expr) => {
        ToolDef {
            name: $name,
            desc: $desc,
            args: $args,
            kind: $kind,
            run: |a, v| dispatch(a, $name, v).unwrap().map(ToolOutcome::Done),
        }
    };
}

pub const TOOLS: &[ToolDef] = &[
    row!("clip.set", ToolKind::Mutate, "Set clip fields: name, enabled, label, speed, reverse, freeze (source time or null), blend, fade_in, fade_out, and constant values of properties (x, y, scale, rotation, opacity, volume, pan); text clips: text style fields (text, font, size, color [r,g,b,a], outline_width, …).", &["clip_id:integer:true:", "fields:object:true:"]),
    row!("clip.keyframe", ToolKind::Mutate, "Set (or remove with remove=true) a keyframe of a property at clip-local time t.", &["clip_id:integer:true:", "property:string:true:", "t:number:true:", "value:number:false:", "ease:string:false:Linear|EaseIn|EaseOut|EaseInOut|Hold|cubic-bezier(x1,y1,x2,y2)", "remove:boolean:false:"]),
    row!("clip.add_effect", ToolKind::Mutate, "Append an effect with optional params {name: value}.", &["clip_id:integer:true:", "kind:string:true:", "params:object:false:"]),
    row!("clip.remove_effect", ToolKind::Mutate, "Remove effect at index.", &["clip_id:integer:true:", "index:integer:true:"]),
    row!("clip.apply_motion", ToolKind::Mutate, "Apply a motion preset (built-in or saved) to a clip.", &["clip_id:integer:true:", "name:string:true:", "scaled:boolean:false:default true"]),
    row!("style.summary", ToolKind::Read, "Markdown style summary of the project (how it was edited) - for writing style guides.", &[]),
    row!("clip.add_mask", ToolKind::Mutate, "Add a mask to a clip (or to one of its effects with `effect`).", &["clip_id:integer:true:", "shape:string:false:Rect|Ellipse|Polygon|Path (default Ellipse)", "effect:integer:false:effect index; omit for the clip itself"]),
    row!("clip.set_mask", ToolKind::Mutate, "Edit a mask: fields {shape, cx, cy, rx, ry, rotation, feather, expand, opacity, invert, enabled, points:[[x,y],…]} in project pixels relative to the layer centre.", &["clip_id:integer:true:", "fields:object:true:", "effect:integer:false:"]),
    row!("clip.add_node", ToolKind::Mutate, "Add a node to the clip's node graph (created from its effect stack on first use).", &["clip_id:integer:true:", "kind:string:true:an effect name, or Blend|Combine|Merge|Matte|Mask|Color|Text|Input", "x:number:false:", "y:number:false:"]),
    row!("clip.connect_nodes", ToolKind::Mutate, "Wire one node's output into another node's input port (cycles are refused).", &["clip_id:integer:true:", "from:integer:true:", "to:integer:true:", "port:integer:false:default 0"]),
    row!("audio.buses", ToolKind::Read, "Mixer buses (id, name, gain, pan, mute/solo/mono, output, filters).", &[]),
    row!("audio.add_bus", ToolKind::Mutate, "Create a bus (it feeds Main until routed elsewhere).", &["name:string:false:"]),
    row!("audio.add_filter", ToolKind::Mutate, "Add a filter to a bus with optional params {name: value}.", &["bus:integer:true:", "kind:string:true:Eq|HighPass|LowPass|Reverb|Echo|Distortion|Compressor|NoiseGate|Noise|Gain", "params:object:false:"]),
    row!("audio.route", ToolKind::Mutate, "Send a clip, a track or a bus into a bus (bus 0 = Main / inherit).", &["bus:integer:true:destination bus", "clip_id:integer:false:", "track:integer:false:track index", "from_bus:integer:false:"]),
    row!("shapes.add", ToolKind::Mutate, "Add a vector shape clip.", &["kind:string:false:Rect|Ellipse|Triangle|Polygon|Star|Line|Arrow|Draw", "at:number:true:", "duration:number:false:default 5", "fill:array:false:[r,g,b,a]", "stroke:array:false:[r,g,b,a]", "stroke_width:number:false:", "sides:integer:false:", "width:number:false:project px", "height:number:false:project px"]),
    row!("container.add", ToolKind::Mutate, "Add a container clip pair (video slot + audio slot) at a time.", &["at:number:true:timeline seconds", "duration:number:false:default 5", "label:string:false:slot label"]),
    row!("container.replace", ToolKind::Mutate, "Replace media in a container clip (effects, transforms, keyframes preserved).", &["clip_id:integer:true:", "asset_id:integer:true:", "pair:boolean:false:also replace linked audio container"]),
    row!("container.make", ToolKind::Mutate, "Convert clips to containers (slots).", &["clip_ids:array:true:"]),
    row!("container.unmake", ToolKind::Mutate, "Remove container flag from clips.", &["clip_ids:array:true:"]),
    row!("container.list", ToolKind::Read, "List all container clips on the timeline.", &[]),
];
