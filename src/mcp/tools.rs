//! The MCP tool catalogue — the single source of truth for tool names/arguments. `mcp/mod.rs` serves it
//! (`tools/list` with JSON schemas built from `ARGS`); `ui/app.rs` executes calls by `name` with exactly
//! these arguments (unknown tool → error "unknown tool"). Keep both in sync with this table.
//!
//! Conventions: ids are the model's u64 ids; times are seconds; "property" names are the inspector labels
//! ("Position X", "Position Y", "Scale", "Rotation", "Opacity", "Volume", "Pan") or "<Effect>: <Param>";
//! every mutating tool pushes one undo step and returns `{"ok":true, ...}`; read tools return JSON.

/// (name, description, argument docs "name:type:required:description" …)
pub const TOOLS: &[(&str, &str, &[&str])] = &[
    ("project.summary", "Project overview: format, duration, tracks, clips per track, assets, sequences, subtitles count, planner progress, notes — and the markdown style summary.", &[]),
    ("project.get", "Full project JSON (the .sedit document).", &[]),
    ("project.new", "New empty project (discards unsaved changes).", &["width:integer:false:default 1920", "height:integer:false:default 1080", "fps:number:false:default 30"]),
    ("project.open", "Open a .sedit project or a media file (creates a project around it).", &["path:string:true:absolute path"]),
    ("project.save", "Save the project (.sedit). Without a path: the current project file (error if none).", &["path:string:false:.sedit path"]),
    ("project.set", "Change project format/name.", &["name:string:false:", "width:integer:false:", "height:integer:false:", "fps:number:false:"]),
    ("media.import", "Import media files into the library; returns asset ids.", &["paths:array:true:absolute paths"]),
    ("media.list", "Library assets (id, path, kind, duration, size, tags, label, folder, description, used).", &[]),
    ("media.set", "Edit asset metadata.", &["id:integer:true:", "description:string:false:", "tags:array:false:strings", "label:integer:false:0..8", "folder:string:false:"]),
    ("media.convert", "Convert a file with ffmpeg (gif/mp4/mov/mkv/webm/mp3/wav…); returns the output path when done (blocks up to 10 min).", &["path:string:true:source", "ext:string:true:target extension", "width:integer:false:", "height:integer:false:", "scaler:string:false:neighbor|bilinear|bicubic|lanczos"]),
    ("timeline.list", "Tracks and clips of the timeline being edited (main or the open sequence): ids, kind, asset, start, duration, src_in, speed, effects, label.", &[]),
    ("timeline.add_clip", "Place an asset, a sequence, or a new text clip at a time.", &["asset_id:integer:false:", "sequence_id:integer:false:", "text:string:false:creates a text clip", "at:number:true:timeline seconds", "track:integer:false:video track index", "duration:number:false:text/image length"]),
    ("timeline.split", "Split clips at t (all clips crossing t when clip_ids is omitted).", &["t:number:true:", "clip_ids:array:false:"]),
    ("timeline.delete", "Delete clips (linked clips follow).", &["clip_ids:array:true:", "ripple:boolean:false:close the gap"]),
    ("timeline.move", "Move clips by dt seconds (and dtrack tracks within their kind).", &["clip_ids:array:true:", "dt:number:true:", "dtrack:integer:false:"]),
    ("timeline.trim", "Trim a clip's edges to new timeline times.", &["clip_id:integer:true:", "start:number:false:new start", "end:number:false:new end"]),
    ("timeline.add_transition", "Transition at the cut on the left of a clip.", &["right_clip_id:integer:true:", "kind:string:true:CrossFade|FadeToColor|Push|Wipe", "duration:number:false:default 1"]),
    ("timeline.auto_cut", "Silence-based auto-cut of audio clips (+ linked video).", &["clip_ids:array:true:", "threshold_db:number:false:default -35", "min_silence:number:false:", "min_speech:number:false:", "padding:number:false:", "keep_quiet:boolean:false:", "ripple:boolean:false:default true"]),
    ("timeline.nest", "Nest clips into a new sequence; returns the sequence id.", &["clip_ids:array:true:", "name:string:false:"]),
    ("sequence.list", "Sequences (id, name, format, duration).", &[]),
    ("sequence.open", "Edit a sequence (swap it into the timeline); omit id to go back to the main timeline.", &["id:integer:false:"]),
    ("clip.set", "Set clip fields: name, enabled, label, speed, reverse, freeze (source time or null), blend, fade_in, fade_out, and constant values of properties (x, y, scale, rotation, opacity, volume, pan); text clips: text style fields (text, font, size, color [r,g,b,a], outline_width, …).", &["clip_id:integer:true:", "fields:object:true:"]),
    ("clip.keyframe", "Set (or remove with remove=true) a keyframe of a property at clip-local time t.", &["clip_id:integer:true:", "property:string:true:", "t:number:true:", "value:number:false:", "ease:string:false:Linear|EaseIn|EaseOut|EaseInOut|Hold|cubic-bezier(x1,y1,x2,y2)", "remove:boolean:false:"]),
    ("clip.add_effect", "Append an effect with optional params {name: value}.", &["clip_id:integer:true:", "kind:string:true:", "params:object:false:"]),
    ("clip.remove_effect", "Remove effect at index.", &["clip_id:integer:true:", "index:integer:true:"]),
    ("clip.apply_motion", "Apply a motion preset (built-in or saved) to a clip.", &["clip_id:integer:true:", "name:string:true:", "scaled:boolean:false:default true"]),
    ("subtitles.get", "Subtitle cues.", &[]),
    ("subtitles.set", "Replace all cues: [{start,end,text}].", &["cues:array:true:"]),
    ("subtitles.import", "Import .srt/.vtt (replaces).", &["path:string:true:"]),
    ("plan.get", "Planner tree + notes.", &[]),
    ("plan.add", "Add a planner item (optionally under a parent).", &["title:string:true:", "parent:integer:false:", "notes:string:false:", "assets:array:false:asset ids for the moodboard"]),
    ("plan.set", "Update a planner item.", &["id:integer:true:", "title:string:false:", "done:boolean:false:", "notes:string:false:"]),
    ("plan.remove", "Remove a planner item.", &["id:integer:true:"]),
    ("notes.get", "Free-form project notes.", &[]),
    ("notes.set", "Replace the project notes (append=true to append a paragraph).", &["text:string:true:", "append:boolean:false:"]),
    ("render.frame", "Render the timeline at time t as a PNG (base64 data url), max `width` px wide (default 640).", &["t:number:true:", "width:integer:false:"]),
    ("playback.seek", "Move the playhead.", &["t:number:true:"]),
    ("playback.play", "Start playback.", &[]),
    ("playback.pause", "Pause playback.", &[]),
    ("export.video", "Export the timeline to a file (blocks until done, up to 30 min); encoder/crf default to settings.", &["path:string:true:", "encoder:string:false:", "crf:integer:false:", "width:integer:false:", "height:integer:false:", "scaler:string:false:"]),
    ("style.summary", "Markdown style summary of the project (how it was edited) — for writing style guides.", &[]),
    ("templates.list", "Saved clip templates and motion presets.", &[]),
    ("templates.apply", "Place a saved template at a time.", &["name:string:true:", "at:number:true:"]),
    // ---- round 3 ----
    ("clip.add_mask", "Add a mask to a clip (or to one of its effects with `effect`).", &["clip_id:integer:true:", "shape:string:false:Rect|Ellipse|Polygon|Path (default Ellipse)", "effect:integer:false:effect index; omit for the clip itself"]),
    ("clip.set_mask", "Edit a mask: fields {shape, cx, cy, rx, ry, rotation, feather, expand, opacity, invert, enabled, points:[[x,y],…]} in project pixels relative to the layer centre.", &["clip_id:integer:true:", "fields:object:true:", "effect:integer:false:"]),
    ("clip.add_node", "Add a node to the clip's node graph (created from its effect stack on first use).", &["clip_id:integer:true:", "kind:string:true:an effect name, or Blend|Combine|Merge|Matte|Mask|Color|Text|Input", "x:number:false:", "y:number:false:"]),
    ("clip.connect_nodes", "Wire one node's output into another node's input port (cycles are refused).", &["clip_id:integer:true:", "from:integer:true:", "to:integer:true:", "port:integer:false:default 0"]),
    ("markers.list", "Every marker in timeline time (project markers + clip markers).", &[]),
    ("markers.add", "Add a marker at a timeline time (on a clip with clip_id).", &["t:number:true:", "name:string:false:", "note:string:false:", "label:integer:false:", "duration:number:false:range markers", "clip_id:integer:false:"]),
    ("markers.remove", "Remove a marker by id.", &["id:integer:true:"]),
    ("audio.buses", "Mixer buses (id, name, gain, pan, mute/solo/mono, output, filters).", &[]),
    ("audio.add_bus", "Create a bus (it feeds Main until routed elsewhere).", &["name:string:false:"]),
    ("audio.add_filter", "Add a filter to a bus with optional params {name: value}.", &["bus:integer:true:", "kind:string:true:Eq|HighPass|LowPass|Reverb|Echo|Distortion|Compressor|NoiseGate|Noise|Gain", "params:object:false:"]),
    ("audio.route", "Send a clip, a track or a bus into a bus (bus 0 = Main / inherit).", &["bus:integer:true:destination bus", "clip_id:integer:false:", "track:integer:false:track index", "from_bus:integer:false:"]),
    ("shapes.add", "Add a vector shape clip.", &["kind:string:false:Rect|Ellipse|Triangle|Polygon|Star|Line|Arrow|Draw", "at:number:true:", "duration:number:false:default 5", "fill:array:false:[r,g,b,a]", "stroke:array:false:[r,g,b,a]", "stroke_width:number:false:", "sides:integer:false:", "width:number:false:project px", "height:number:false:project px"]),
    ("timeline.import", "Import a timeline from another editor (FCP7 XML, EDL, .prproj); returns the report and opens it in the app (replace=true swaps the project in).", &["path:string:true:", "replace:boolean:false:"]),
    ("frame.export", "Save the frame at time t as PNG/JPG/WebP (by the path's extension).", &["path:string:true:", "t:number:false:default playhead", "width:integer:false:", "height:integer:false:", "with_effects:boolean:false:default true", "quality:integer:false:1..100 for JPG/WebP", "resize:string:false:neighbor|bilinear|bicubic|lanczos"]),
    ("labels.list", "Colour labels of the project (index is 1-based; 0 = none).", &[]),
    ("labels.set", "Rename / recolour a label (index), add one (no index) or remove one (index + remove=true).", &["index:integer:false:1-based", "name:string:false:", "color:array:false:[r,g,b]", "remove:boolean:false:"]),
    ("container.add", "Add a container clip pair (video slot + audio slot) at a time.", &["at:number:true:timeline seconds", "duration:number:false:default 5", "label:string:false:slot label"]),
    ("container.replace", "Replace media in a container clip (effects, transforms, keyframes preserved).", &["clip_id:integer:true:", "asset_id:integer:true:", "pair:boolean:false:also replace linked audio container"]),
    ("container.make", "Convert clips to containers (slots).", &["clip_ids:array:true:"]),
    ("container.unmake", "Remove container flag from clips.", &["clip_ids:array:true:"]),
    ("container.list", "List all container clips on the timeline.", &[]),
];

use serde_json::{json, Value};

/// JSON schema for one tool's arguments ("name:type:required:description" docs).
pub fn input_schema(args: &[&str]) -> Value {
    let mut props = serde_json::Map::new();
    let mut required = Vec::new();
    for a in args {
        let mut it = a.splitn(4, ':');
        let name = it.next().unwrap_or("");
        let ty = it.next().unwrap_or("string");
        let req = it.next().unwrap_or("false");
        let desc = it.next().unwrap_or("");
        let mut p = serde_json::Map::new();
        p.insert("type".into(), Value::String(ty.into()));
        if !desc.is_empty() {
            p.insert("description".into(), Value::String(desc.into()));
        }
        props.insert(name.into(), Value::Object(p));
        if req == "true" {
            required.push(Value::String(name.into()));
        }
    }
    json!({"type": "object", "properties": props, "required": required})
}

/// Every effect kind the `clip.add_effect` handler accepts — listed from the model, because a
/// hand-written list in the catalogue goes stale the moment an effect is added.
fn effect_kinds() -> String {
    crate::model::EffectKind::ALL.iter().map(|k| k.name()).collect::<Vec<_>>().join(", ")
}

/// The `tools/list` payload: [{name, description, inputSchema}].
pub fn list_json() -> Value {
    Value::Array(
        TOOLS
            .iter()
            .map(|(name, desc, args)| {
                let desc = match *name {
                    "clip.add_effect" => format!("{desc} Kinds: {}.", effect_kinds()),
                    _ => (*desc).to_string(),
                };
                json!({"name": name, "description": desc, "inputSchema": input_schema(args)})
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_builder() {
        let s = input_schema(&["id:integer:true:the id", "path:string:false:"]);
        assert_eq!(s["type"], "object");
        assert_eq!(s["properties"]["id"]["type"], "integer");
        assert_eq!(s["properties"]["id"]["description"], "the id");
        assert_eq!(s["required"], json!(["id"]));
        assert!(s["properties"]["path"].get("description").is_none()); // empty description omitted
        assert_eq!(s["properties"]["path"]["type"], "string");
        // every catalogue entry builds a valid object schema with known types
        for (name, _, args) in TOOLS {
            let v = input_schema(args);
            assert_eq!(v["type"], "object", "{name}");
            for (_, p) in v["properties"].as_object().unwrap() {
                let ty = p["type"].as_str().unwrap();
                assert!(
                    matches!(ty, "string" | "number" | "integer" | "boolean" | "array" | "object"),
                    "{name}: bad type {ty}"
                );
            }
        }
        assert_eq!(list_json().as_array().unwrap().len(), TOOLS.len());
    }

    /// Every effect the handler accepts is discoverable from the tool description.
    #[test]
    fn add_effect_lists_every_kind() {
        let list = list_json();
        let d = list.as_array().unwrap().iter().find(|t| t["name"] == "clip.add_effect").unwrap()["description"]
            .as_str()
            .unwrap()
            .to_string();
        for k in crate::model::EffectKind::ALL {
            assert!(d.contains(k.name()), "{} missing from the description", k.name());
        }
    }
}
