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
    ("clip.add_effect", "Append an effect (Blur, Pixelate, Tint, Color, Vignette, Sharpen, Invert, Grayscale, Flip, Crop, Wobble) with optional params {name: value}.", &["clip_id:integer:true:", "kind:string:true:", "params:object:false:"]),
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

/// The `tools/list` payload: [{name, description, inputSchema}].
pub fn list_json() -> Value {
    Value::Array(
        TOOLS
            .iter()
            .map(|(name, desc, args)| json!({"name": name, "description": desc, "inputSchema": input_schema(args)}))
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
}
