//! ---- ws:transcript-captions ----
//! MCP tools over transcripts (auto-Luau-exposed via `TOOL_TABLES`): transcript.get/set/cut_words/
//! remove_fillers/search/export, transcribe.run/install, tracking.run, subtitles.animation, tts.speak,
//! plus the agent-facing media.transcribe / media.transcript pair. Every cut goes through
//! `Project::cut_word_ranges`; every whisper run through `App::transcribe_clip` (the same job the
//! clip menu and the Subtitles pane use). Job-kind rows return the OUTER progress that completes only
//! once the result is in the project (see transcript_ctl.rs).

use super::tools_args::Args;
use super::tools_helpers::*;
use super::*;
use crate::engine::transcribe;
use crate::mcp::tools::{ToolDef, ToolKind, ToolOutcome};
use crate::model::ops::subtitles::transcript_hits;
use crate::model::SubtitleAnim;
use crate::ui::transcript_ui;

fn words_json(words: &[(f64, f64, String)]) -> Vec<Value> {
    words.iter().map(|w| json!({"start": w.0, "end": w.1, "text": w.2})).collect()
}

/// `[{start,end,text}]` or `[[start,end,text]]` → word triples.
fn parse_words(v: Option<&Value>) -> Result<Vec<(f64, f64, String)>, String> {
    let arr = v.and_then(|v| v.as_array()).ok_or("words must be an array")?;
    let mut out = Vec::with_capacity(arr.len());
    for w in arr {
        let (s, e, t) = if let Some(a) = w.as_array() {
            (a.first().and_then(Value::as_f64), a.get(1).and_then(Value::as_f64), a.get(2).and_then(Value::as_str))
        } else {
            (arg_f64(w, "start"), arg_f64(w, "end"), arg_str(w, "text"))
        };
        let (s, e, t) = (req(s, "words[].start")?, req(e, "words[].end")?, req(t, "words[].text")?);
        out.push((s, e.max(s), t.to_string()));
    }
    Ok(out)
}

/// `{clip_id, words:[{start,end,text}], text}`; a clip with no transcript yields empty words plus a
/// hint, not an error.
fn transcript_value(app: &App, clip: Id) -> Value {
    let words = app.project.transcript(clip).map(|t| t.words.as_slice()).unwrap_or(&[]);
    let mut v = json!({
        "clip_id": clip,
        "words": words_json(words),
        "text": words.iter().map(|w| w.2.as_str()).collect::<Vec<_>>().join(" "),
    });
    if words.is_empty() {
        v["hint"] = json!("no transcript for this clip yet — run media.transcribe / transcribe.run first");
    }
    v
}

/// `clip_id`, or `asset_id` → the first timeline clip (video or audio) of that asset.
fn clip_from(app: &App, a: &Args) -> Result<Id, String> {
    if let Some(c) = a.id("clip_id") {
        return app.project.clip(c).map(|c| c.id).ok_or_else(|| "no such clip".into());
    }
    let aid = req(a.id("asset_id"), "clip_id or asset_id")?;
    app.project.asset(aid).ok_or("no such asset")?;
    app.project
        .all_clips()
        .find(|(_, c)| c.uses_asset() && c.asset == aid && matches!(c.kind, ClipKind::Video | ClipKind::Audio))
        .map(|(_, c)| c.id)
        .ok_or_else(|| "that asset is not on the timeline — place it first (timeline.add_clip)".into())
}

fn anim_from(kind: &str, color: Option<[u8; 4]>) -> Result<SubtitleAnim, String> {
    Ok(match kind.trim().to_ascii_lowercase().as_str() {
        "none" | "off" => SubtitleAnim::None,
        "highlight" => SubtitleAnim::Highlight(color.unwrap_or([255, 220, 0, 255])),
        "pop" | "popword" | "pop_word" => SubtitleAnim::PopWord,
        "typewriter" => SubtitleAnim::Typewriter,
        other => return Err(format!("unknown animation '{other}' (none|highlight|pop|typewriter)")),
    })
}

/// A transcribe job for `clip` (see `App::transcribe_clip`) as a Job outcome — the reply's `path`
/// is empty on purpose: the result is in the project, read it with media.transcript.
fn transcribe_job(app: &mut App, clip: Id, a: &Args, gen_cues: bool) -> Result<ToolOutcome, String> {
    let prog = app.transcribe_clip(clip, a.str("model"), a.str("language"), gen_cues)?;
    Ok(ToolOutcome::Job(prog, PathBuf::new()))
}

pub const TOOLS: &[ToolDef] = &[
    ToolDef {
        name: "transcript.get",
        desc: "Word timings (timeline seconds) for one transcribed clip, or every transcript when clip_id is omitted.",
        args: &["clip_id:integer:false:omit for all clips"],
        kind: ToolKind::Read,
        run: |app, args| {
            let a = Args(args);
            Ok(ToolOutcome::Done(match a.id("clip_id") {
                Some(c) => transcript_value(app, c),
                None => Value::Array(app.project.transcripts.iter().map(|t| transcript_value(app, t.clip)).collect()),
            }))
        },
    },
    ToolDef {
        name: "transcript.set",
        desc: "Write (replace) a clip's transcript: words as [{start,end,text}] or [[start,end,text]] in timeline seconds; an empty list removes it.",
        args: &["clip_id:integer:true:", "words:array:true:"],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let a = Args(args);
            let clip = req(a.id("clip_id"), "clip_id")?;
            app.project.clip(clip).ok_or("no such clip")?;
            let words = parse_words(args.get("words"))?;
            app.project.set_transcript(clip, words);
            let n = app.project.transcript(clip).map_or(0, |t| t.words.len());
            Ok(ToolOutcome::Done(json!({"ok": true, "words": n})))
        },
    },
    ToolDef {
        name: "transcript.cut_words",
        desc: "Ripple-cut timeline ranges (or the given word indices) out of a transcribed clip; cues, markers and words shift along (Project::cut_word_ranges).",
        args: &[
            "clip_id:integer:true:",
            "ranges:array:false:[[start,end],...] timeline seconds",
            "word_indices:array:false:alternative to ranges: indices into transcript.get's words",
        ],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let a = Args(args);
            let clip = req(a.id("clip_id"), "clip_id")?;
            let mut ranges: Vec<(f64, f64)> = Vec::new();
            if let Some(list) = args.get("ranges").and_then(|v| v.as_array()) {
                for r in list {
                    let pair = r.as_array().ok_or("ranges[] must be [start,end]")?;
                    let s = req(pair.first().and_then(Value::as_f64), "ranges[][0]")?;
                    let e = req(pair.get(1).and_then(Value::as_f64), "ranges[][1]")?;
                    ranges.push((s, e));
                }
            }
            if let Some(idx) = args.get("word_indices").and_then(|v| v.as_array()) {
                let words = app.project.transcript(clip).map(|t| t.words.clone()).unwrap_or_default();
                for i in idx {
                    let i = req(i.as_u64(), "word_indices[]")? as usize;
                    let w = words.get(i).ok_or_else(|| format!("word index {i} is out of range"))?;
                    ranges.push((w.0, w.1));
                }
            }
            if ranges.is_empty() {
                return Err("give ranges or word_indices".into());
            }
            let n = app.project.cut_word_ranges(clip, &ranges);
            if n == 0 {
                return Err("nothing cut (no such clip, a locked track, a speed-ramped clip, or ranges outside it)".into());
            }
            Ok(ToolOutcome::Done(json!({"ok": true, "pieces_removed": n, "ranges": ranges.len()})))
        },
    },
    ToolDef {
        name: "transcript.remove_fillers",
        desc: "Find filler words in a transcribed clip (Settings.filler_words unless `words` overrides) and ripple-cut them. dry_run returns the ranges only; as_markers drops a range marker per hit instead (Mark instead).",
        args: &[
            "clip_id:integer:true:",
            "words:array:false:filler words/phrases, overrides Settings.filler_words",
            "pad_ms:integer:false:default Settings.filler_pad_ms",
            "dry_run:boolean:false:default false",
            "as_markers:boolean:false:default false, Mark instead",
        ],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let a = Args(args);
            let clip = req(a.id("clip_id"), "clip_id")?;
            let words = app.project.transcript(clip).map(|t| t.words.clone()).unwrap_or_default();
            if words.is_empty() {
                return Err("no transcript for that clip yet".into());
            }
            let custom: Option<Vec<String>> = args
                .get("words")
                .and_then(|v| v.as_array())
                .map(|l| l.iter().filter_map(|v| v.as_str().map(String::from)).collect());
            let list: Vec<&str> = match &custom {
                Some(c) => c.iter().map(String::as_str).collect(),
                None => app.settings.filler_words.iter().map(String::as_str).collect(),
            };
            let pad = a.id("pad_ms").map(|p| p as u32).unwrap_or(app.settings.filler_pad_ms);
            let ranges = transcribe::filler_ranges(&words, &list, pad);
            let ranges_json: Vec<Value> = ranges.iter().map(|r| json!([r.0, r.1])).collect();
            if a.bool("dry_run").unwrap_or(false) {
                return Ok(ToolOutcome::Done(json!({"ok": true, "dry_run": true, "ranges": ranges_json})));
            }
            if a.bool("as_markers").unwrap_or(false) {
                let ids = transcript_ui::mark_ranges(&mut app.project, &words, &ranges);
                app.fire_markers_added(&ids);
                return Ok(ToolOutcome::Done(json!({"ok": true, "marker_ids": ids, "ranges": ranges_json})));
            }
            if ranges.is_empty() {
                return Ok(ToolOutcome::Done(json!({"ok": true, "removed": 0, "ranges": []})));
            }
            let n = app.project.cut_word_ranges(clip, &ranges);
            if n == 0 {
                return Err("nothing cut (a locked track or a speed-ramped clip)".into());
            }
            Ok(ToolOutcome::Done(json!({"ok": true, "removed": ranges.len(), "pieces_removed": n, "ranges": ranges_json})))
        },
    },
    ToolDef {
        name: "transcript.search",
        desc: "Word hits across every transcribed clip: [{clip_id, index, t}] (case/punctuation-insensitive; a phrase matches consecutive words).",
        args: &["query:string:true:"],
        kind: ToolKind::Read,
        run: |app, args| {
            let q = req(arg_str(args, "query"), "query")?;
            let hits: Vec<Value> = transcript_hits(&app.project.transcripts, q)
                .into_iter()
                .map(|(clip, i, t)| json!({"clip_id": clip, "index": i, "t": t}))
                .collect();
            Ok(ToolOutcome::Done(json!(hits)))
        },
    },
    ToolDef {
        name: "transcript.export",
        desc: "Write a clip's transcript to a file as txt (plain), srt (sentences → cues) or json ({clip_id, words}); format defaults to the path's extension. Mutates nothing.",
        args: &["clip_id:integer:true:", "path:string:true:", "format:string:false:txt|srt|json"],
        kind: ToolKind::Read,
        run: |app, args| {
            let a = Args(args);
            let clip = req(a.id("clip_id"), "clip_id")?;
            let path = PathBuf::from(req(a.str("path"), "path")?);
            let words = app.project.transcript(clip).map(|t| t.words.clone()).ok_or("no transcript for that clip yet")?;
            let fmt = super::transcript_ctl::write_transcript(clip, &words, &path, a.str("format"))?;
            Ok(ToolOutcome::Done(json!({"ok": true, "path": path.to_string_lossy(), "format": fmt, "words": words.len()})))
        },
    },
    ToolDef {
        name: "transcribe.run",
        desc: "Transcribe a clip's audio with whisper (background job; word timings always on). On completion the words are in Project.transcripts (media.transcript) and, with cues=true (default), captions are generated.",
        args: &[
            "clip_id:integer:true:",
            "model:string:false:tiny.en|base.en|small.en|base (default: the Subtitles pane's pick)",
            "language:string:false:auto, en, de, …",
            "cues:boolean:false:default true — also generate subtitle cues",
        ],
        kind: ToolKind::Job,
        run: |app, args| {
            let a = Args(args);
            let clip = req(a.id("clip_id"), "clip_id")?;
            let cues = a.bool("cues").unwrap_or(true);
            transcribe_job(app, clip, &a, cues)
        },
    },
    ToolDef {
        name: "transcribe.install",
        desc: "Download a whisper model into the cache (job; 75–466 MB, see the model names). Already downloaded = done at once.",
        args: &["model:string:false:tiny.en|base.en|small.en|base, default Settings.transcribe_model"],
        kind: ToolKind::Job,
        run: |app, args| {
            let a = Args(args);
            let (_, file, _) = match a.str("model").map(str::trim).filter(|m| !m.is_empty()) {
                Some(m) => *transcribe::MODELS
                    .iter()
                    .find(|(n, f, _)| *f == m || n.starts_with(m))
                    .ok_or_else(|| format!("unknown model '{m}'"))?,
                None => app.subtitles_ui.transcribe.model(),
            };
            Ok(ToolOutcome::Job(transcribe::download_model(file), transcribe::model_path(file)))
        },
    },
    ToolDef {
        name: "tracking.run",
        desc: "NCC point-track a region of a clip (job): box centre (cx,cy) and half-size (hw,hh) in canvas px relative to the centre. With apply (default true) the path is written as the clip's X/Y keyframes when done (read them with clip.get).",
        args: &[
            "clip_id:integer:true:",
            "cx:number:true:",
            "cy:number:true:",
            "hw:number:true:",
            "hh:number:true:",
            "search:number:false:search radius px, default 24",
            "backward:boolean:false:",
            "apply:boolean:false:default true, writes X/Y keyframes",
        ],
        kind: ToolKind::Job,
        run: |app, args| {
            let a = Args(args);
            let clip = req(a.id("clip_id"), "clip_id")?;
            let rect = (
                req(a.f64("cx"), "cx")? as f32,
                req(a.f64("cy"), "cy")? as f32,
                req(a.f64("hw"), "hw")? as f32,
                req(a.f64("hh"), "hh")? as f32,
            );
            let search = a.f64("search").unwrap_or(24.0) as f32;
            let prog = app.start_tracking(clip, rect, search, a.bool("backward").unwrap_or(false), a.bool("apply").unwrap_or(true))?;
            Ok(ToolOutcome::Job(prog, PathBuf::new()))
        },
    },
    ToolDef {
        name: "subtitles.animation",
        desc: "Set how generated captions animate against the transcript: none | highlight (color [r,g,b,a]) | pop | typewriter.",
        args: &["kind:string:true:none|highlight|pop|typewriter", "color:array:false:[r,g,b,a] for highlight"],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let kind = req(arg_str(args, "kind"), "kind")?;
            let color = args.get("color").and_then(color_arg);
            app.project.subtitle_anim = anim_from(kind, color)?;
            Ok(ToolOutcome::Done(json!({"ok": true, "animation": format!("{:?}", app.project.subtitle_anim)})))
        },
    },
    ToolDef {
        name: "tts.speak",
        desc: "Windows SAPI text-to-speech (job): writes a WAV and imports it at `at` (default: the playhead), linked to the text clip `clip_id` when given. OS voices only — see the Speech panel for names.",
        args: &[
            "text:string:true:",
            "voice:string:false:installed SAPI voice name",
            "clip_id:integer:false:text clip to link the audio to",
            "at:number:false:timeline seconds, default playhead",
        ],
        kind: ToolKind::Job,
        run: |app, args| {
            let a = Args(args);
            let text = req(a.str("text"), "text")?.to_string();
            let link_to = a.id("clip_id");
            if let Some(c) = link_to {
                app.project.clip(c).ok_or("no such clip")?;
            }
            let at = a.t_or_playhead("at", app);
            let prog = app.speak(&text, a.str("voice"), link_to, at)?;
            Ok(ToolOutcome::Job(prog, PathBuf::new()))
        },
    },
    ToolDef {
        name: "media.transcribe",
        desc: "Transcribe a clip (or the first timeline clip of an asset) with whisper — same job as transcribe.run without generating cues; fills Project.transcripts when done (read with media.transcript).",
        args: &["clip_id:integer:false:one of clip_id / asset_id", "asset_id:integer:false:", "model:string:false:tiny.en|base.en|small.en|base"],
        kind: ToolKind::Job,
        run: |app, args| {
            let a = Args(args);
            let clip = clip_from(app, &a)?;
            transcribe_job(app, clip, &a, false)
        },
    },
    ToolDef {
        name: "media.transcript",
        desc: "{clip_id, words:[{start,end,text}], text} for one clip (clip_id, or asset_id's first timeline clip), or every transcript when both are omitted. A clip without one gives empty words plus a hint.",
        args: &["clip_id:integer:false:", "asset_id:integer:false:"],
        kind: ToolKind::Read,
        run: |app, args| {
            let a = Args(args);
            if a.id("clip_id").is_none() && a.id("asset_id").is_none() {
                let all: Vec<Value> = app.project.transcripts.iter().map(|t| transcript_value(app, t.clip)).collect();
                return Ok(ToolOutcome::Done(Value::Array(all)));
            }
            let clip = clip_from(app, &a)?;
            Ok(ToolOutcome::Done(transcript_value(app, clip)))
        },
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_words_accepts_objects_and_triples() {
        let v = json!([{"start": 0.0, "end": 0.4, "text": "a"}, [0.5, 0.9, "b"]]);
        let w = parse_words(Some(&v)).unwrap();
        assert_eq!(w, vec![(0.0, 0.4, "a".to_string()), (0.5, 0.9, "b".to_string())]);
        assert!(parse_words(Some(&json!([{"start": 1.0}]))).is_err());
        assert!(parse_words(None).is_err());
        // end never precedes start
        assert_eq!(parse_words(Some(&json!([[2.0, 1.0, "x"]]))).unwrap()[0].1, 2.0);
    }

    #[test]
    fn animation_names_resolve() {
        assert_eq!(anim_from("none", None).unwrap(), SubtitleAnim::None);
        assert_eq!(anim_from("Highlight", Some([1, 2, 3, 4])).unwrap(), SubtitleAnim::Highlight([1, 2, 3, 4]));
        assert!(matches!(anim_from("highlight", None).unwrap(), SubtitleAnim::Highlight(_)));
        assert_eq!(anim_from("pop", None).unwrap(), SubtitleAnim::PopWord);
        assert_eq!(anim_from("typewriter", None).unwrap(), SubtitleAnim::Typewriter);
        assert!(anim_from("wobble", None).is_err());
    }

    /// The extension's three agent-facing tools plus the plan's rows are all registered, uniquely.
    #[test]
    fn transcript_tools_are_registered() {
        for name in [
            "transcript.get",
            "transcript.set",
            "transcript.cut_words",
            "transcript.remove_fillers",
            "transcript.search",
            "transcript.export",
            "transcribe.run",
            "transcribe.install",
            "tracking.run",
            "subtitles.animation",
            "tts.speak",
            "media.transcribe",
            "media.transcript",
        ] {
            assert_eq!(mcp::tools::all().filter(|t| t.name == name).count(), 1, "{name}");
        }
        for t in TOOLS {
            let job = matches!(t.name, "transcribe.run" | "transcribe.install" | "tracking.run" | "tts.speak" | "media.transcribe");
            assert_eq!(t.kind == ToolKind::Job, job, "{} has the wrong kind", t.name);
        }
        let export = mcp::tools::find("transcript.export").unwrap();
        assert_eq!(export.kind, ToolKind::Read, "writes a file, mutates no project state");
    }
}
