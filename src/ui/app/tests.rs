use super::lib_preview::*;
use super::thumbs::*;
use super::tools_helpers::*;
use super::*;

use crate::model::{Asset, Ease};

/// `App::new` needs a real `eframe::CreationContext` (a GL context), so there is no headless
/// App harness to build one against here — this tests the same `Toast` construction that
/// `toast`/`toast_with_folder` do (both are one-line wrappers around it).
#[test]
fn toast_with_folder_sets_open_path_plain_toast_does_not() {
    let plain = Toast::new("saved");
    assert!(plain.open_path.is_none());
    let with_folder = Toast::with_folder("saved", PathBuf::from("C:/out.mp4"));
    assert_eq!(with_folder.open_path, Some(PathBuf::from("C:/out.mp4")));
}

// ---- ws:audio-analysis ----

/// `mark_instead_fires_marker_added_once_per_marker` (see the audio-analysis PR's tests) against the
/// real `App` shape: `fire_markers_added`/`fire_hook` need a live `App` (see the App-construction
/// deviation note above), so this exercises the plain half they're built from — mirrors
/// `mcp_exec.rs`'s `snapshot_if_mutate`/`rollback_project` split.
#[test]
fn fire_marker_added_for_each_fires_once_per_id_in_order() {
    let ids = [1u64, 2, 3];
    let mut calls: Vec<(String, Value)> = Vec::new();
    fire_marker_added_for_each(&ids, &mut |event, payload| calls.push((event.to_string(), payload)));
    assert_eq!(calls.len(), 3, "exactly once per marker");
    for (i, (event, payload)) in calls.iter().enumerate() {
        assert_eq!(event, "marker_added");
        assert_eq!(payload["marker_id"], json!(ids[i]));
    }
    // zero ids -> zero calls (not called "at least once")
    let mut none: Vec<(String, Value)> = Vec::new();
    fire_marker_added_for_each(&[], &mut |event, payload| none.push((event.to_string(), payload)));
    assert!(none.is_empty());
}

#[test]
fn box_blur_spreads_and_preserves_flat_areas() {
    // a flat image stays exactly flat
    let (w, h) = (8, 8);
    let mut flat = vec![100u8; w * h * 4];
    box_blur(&mut flat, w, h, 2);
    assert!(flat.iter().all(|&v| v == 100));
    // a single bright pixel spreads to its neighbours and dims in place
    let mut img = vec![0u8; w * h * 4];
    let centre = ((4 * w + 4) * 4) as usize;
    img[centre] = 255;
    box_blur(&mut img, w, h, 1);
    assert!(img[centre] < 255, "the spike must dim");
    let neighbour = ((4 * w + 5) * 4) as usize;
    assert!(img[neighbour] > 0, "and bleed into the pixel beside it");
    // radius 0 is a no-op and empty buffers don't panic
    let mut same = vec![7u8; 16];
    box_blur(&mut same, 2, 2, 0);
    assert_eq!(same, vec![7u8; 16]);
    box_blur(&mut [], 0, 0, 3);
}

fn asset(path: &str) -> Asset {
    Asset {
        id: Id::default(),
        path: path.into(),
        kind: ClipKind::Video,
        duration: 1.0,
        width: 0,
        height: 0,
        fps: 0.0,
        audio_streams: Vec::new(),
        codec: String::new(),
        folder: String::new(),
        tags: Vec::new(),
        label: 0,
        description: String::new(),
        rel_path: None,
        parent: None,
        range: None,
        effects: Vec::new(),
    }
}

/// Ctrl+V was dead because egui-winit only emits `Event::Paste` when the SYSTEM clipboard holds
/// text, and an internal clip copy never wrote to it — so the chord produced no event at all and no
/// binding could see it. Copying clips must therefore also queue text for the OS clipboard.
#[test]
fn copying_clips_also_writes_the_os_clipboard() {
    let mut p = Project::from_media(long_asset("C:/x.mp4"));
    let id = p.tracks[0].clips[0].id;
    let t = crate::engine::presets::capture_template("clipboard", &p, &[id]);
    assert!(!t.json.is_empty(), "a captured clip serialises to something");
    // what act(CopyClips) stores: the same JSON goes to both clipboards
    let os = t.json.clone();
    assert!(
        crate::engine::presets::decode_template(&t).is_some(),
        "the internal clipboard still decodes back into clips"
    );
    assert!(os.contains("clips") || os.contains("start"), "the OS text is the template JSON: {os:.80}");
    // and the ripple that Paste Insert performs opens exactly the span it is given
    let before = p.tracks[0].clips[0].start;
    p.ripple_open(before, 2.0);
    assert!(
        (p.tracks[0].clips[0].start - (before + 2.0)).abs() < 1e-6,
        "ripple_open slides the clip right by the span: {} -> {}",
        before,
        p.tracks[0].clips[0].start
    );
}

/// A 10 s video asset, long enough to split a few times.
fn long_asset(path: &str) -> Asset {
    Asset { duration: 10.0, width: 320, height: 240, fps: 30.0, ..asset(path) }
}

#[test]
fn relocate_assets_falls_back_to_project_dir() {
    let dir = std::env::temp_dir().join(format!("se-relocate-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.mp4"), b"x").unwrap();
    let mut p = Project::new();
    p.assets.push(asset("Z:\\gone\\a.mp4"));
    p.assets.push(asset("Z:\\gone\\b.mp4"));
    let missing = relocate_assets(&mut p, Some(&dir));
    assert_eq!(p.assets[0].path, dir.join("a.mp4").to_string_lossy());
    assert_eq!(missing, vec!["Z:\\gone\\b.mp4".to_string()]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn converted_path_never_hits_the_source_or_an_existing_file() {
    let dir = std::env::temp_dir().join(format!("se-conv-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("clip.mp4");
    std::fs::write(&src, b"x").unwrap();
    // converting to the same container must not write over the source
    let out = converted_path(&src, "mp4");
    assert_ne!(out, src);
    assert_eq!(out.file_name().unwrap(), "clip_converted.mp4");
    // nor over a file that is already there (the timeline may be using it)
    std::fs::write(&out, b"y").unwrap();
    assert_eq!(converted_path(&src, "mp4").file_name().unwrap(), "clip_converted_2.mp4");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_empty_open_sequence_is_not_an_empty_timeline() {
    let mut p = Project::from_media(asset("a.mp4"));
    assert!(!timeline_is_empty(&p));
    let ids: Vec<Id> = p.all_clips().map(|(_, c)| c.id).collect();
    let seq = p.nest_selection(&ids, "S").unwrap();
    assert!(p.open_sequence(seq));
    let inner: Vec<Id> = p.all_clips().map(|(_, c)| c.id).collect();
    p.delete_clips(&inner, false);
    assert!(p.is_empty()); // this sequence is empty…
    assert!(!timeline_is_empty(&p)); // …but the main timeline still holds the Sequence clip
    p.close_sequence();
    let all: Vec<Id> = p.all_clips().map(|(_, c)| c.id).collect();
    p.delete_clips(&all, false);
    assert!(timeline_is_empty(&p));
}

#[test]
fn undo_snapshot_is_capped_and_clears_redo() {
    let mut undo: Vec<UndoEntry> = Vec::new();
    let mut redo: Vec<UndoEntry> =
        vec![UndoEntry { json: "r".into(), label: "r".into(), category: HistoryCategory::Editing, at: 0.0 }];
    for i in 0..205 {
        push_undo_json(&mut undo, &mut redo, i.to_string());
    }
    assert_eq!(undo.len(), 200);
    assert_eq!(undo[0].json, "5");
    assert!(redo.is_empty());
}

#[test]
fn base64_rfc4648_vectors() {
    assert_eq!(base64(b""), "");
    assert_eq!(base64(b"f"), "Zg==");
    assert_eq!(base64(b"fo"), "Zm8=");
    assert_eq!(base64(b"foo"), "Zm9v");
    assert_eq!(base64(b"foob"), "Zm9vYg==");
    assert_eq!(base64(b"fooba"), "Zm9vYmE=");
    assert_eq!(base64(b"foobar"), "Zm9vYmFy");
}

#[test]
fn parse_ease_names_and_bezier() {
    assert_eq!(parse_ease("Linear"), Some(Ease::Linear));
    assert_eq!(parse_ease("Hold"), Some(Ease::Hold));
    assert_eq!(
        parse_ease("cubic-bezier(0.42, 0, 0.58, 1)"),
        Some(Ease::Bezier { x1: 0.42, y1: 0.0, x2: 0.58, y2: 1.0 })
    );
    assert_eq!(parse_ease("cubic-bezier(1,2,3)"), None);
    assert_eq!(parse_ease("bogus"), None);
}

#[test]
fn clip_fields_apply_and_reject() {
    let mut c = Clip::new(1, ClipKind::Video, "c", 0.0, 4.0);
    apply_clip_fields(
        &mut c,
        &json!({"name": "renamed", "enabled": false, "label": 3, "blend": "Screen",
                     "fade_in": 0.5, "opacity": 0.25, "freeze": 1.5}),
    )
    .unwrap();
    assert_eq!(c.name, "renamed");
    assert!(!c.enabled);
    assert_eq!(c.label, 3);
    assert_eq!(c.blend, BlendMode::Screen);
    assert_eq!(c.fade_in, 0.5);
    assert_eq!(c.opacity.value, 0.25);
    assert_eq!(c.freeze, Some(1.5));
    apply_clip_fields(&mut c, &json!({"freeze": null})).unwrap();
    assert_eq!(c.freeze, None);
    // setting a constant clears animation
    c.opacity.toggle_key(1.0);
    assert!(c.opacity.is_animated());
    apply_clip_fields(&mut c, &json!({"opacity": 1.0})).unwrap();
    assert!(!c.opacity.is_animated());
    // unknown fields / text fields on a non-text clip are errors
    assert!(apply_clip_fields(&mut c, &json!({"nope": 1})).is_err());
    assert!(apply_clip_fields(&mut c, &json!({"text": "hi"})).is_err());
    // text clip accepts text style fields
    let mut t = Clip::new(2, ClipKind::Text, "t", 0.0, 4.0);
    apply_clip_fields(&mut t, &json!({"text": "hello", "size": 90, "color": [10, 20, 30, 255], "align": 0})).unwrap();
    let ts = t.text.as_ref().unwrap();
    assert_eq!(ts.text, "hello");
    assert_eq!(ts.size, 90.0);
    assert_eq!(ts.color, [10, 20, 30, 255]);
    assert_eq!(ts.align, 0);
}

/// Ctrl+T repeats the last transition: same kind and duration, on the selected clip's left cut.
#[test]
fn last_transition_is_remembered_and_reapplied() {
    let mut p = Project::from_media(long_asset("a.mp4"));
    let ids: Vec<Id> = p.split_at(1.0, None);
    let (first, second) = (p.all_clips().next().unwrap().1.id, ids[0]);
    // the "Add Transition" action's default: cross fade, 1 s — remembered in the panel state
    let mut st = transitions_ui::TransitionsState::default();
    let add = transitions_ui::add_transitions;
    assert_eq!(add(&mut p, &[second], &mut st, TransitionKind::CrossFade, 1.0, false), 1);
    let tr = p.tracks[0].transitions[0].clone();
    assert_eq!((tr.kind, tr.duration), (TransitionKind::CrossFade, 1.0));
    // a different choice replaces the memory and Ctrl+T applies exactly that at the next cut
    let ids2 = p.split_at(2.0, None);
    assert_eq!(add(&mut p, &ids2, &mut st, TransitionKind::Wipe, 0.4, false), 1);
    assert_eq!(st.kind(), TransitionKind::Wipe, "Ctrl+T follows whatever went through the funnel");
    let tr = p.tracks[0].transitions.iter().find(|t| t.right == ids2[0]).expect("second transition");
    assert_eq!((tr.kind, tr.duration), (TransitionKind::Wipe, 0.4));
    // nothing abuts the very first clip's left edge → it blends in from nothing instead
    assert_eq!(add(&mut p, &[first], &mut st, TransitionKind::Wipe, 0.4, false), 1);
    let tr = p.tracks[0].transitions.iter().find(|t| t.right == first).expect("edge transition");
    assert_eq!(tr.edge, crate::model::TransitionEdge::In);
}

#[test]
fn masks_land_on_the_last_effect_then_the_clip() {
    let mut p = Project::from_media(long_asset("a.mp4"));
    let id = p.all_clips().next().unwrap().1.id;
    // no effects: the clip itself gets the mask
    assert!(add_mask(&mut p, id, MaskShape::Ellipse));
    assert_eq!(p.clip(id).unwrap().mask.as_ref().map(|m| m.shape), Some(MaskShape::Ellipse));
    assert!(!add_mask(&mut p, id, MaskShape::Rect), "a second mask on the same clip is refused");
    // with an effect, the mask goes on the effect (that is what a mask usually means)
    p.clip_mut(id).unwrap().effects.push(Effect::new(EffectKind::Blur));
    assert!(add_mask(&mut p, id, MaskShape::Polygon));
    assert_eq!(p.clip(id).unwrap().effects[0].mask.as_ref().map(|m| m.shape), Some(MaskShape::Polygon));
    assert!(!add_mask(&mut p, 999, MaskShape::Rect), "unknown clip");
    // a mask shapes pixels: audio takes none, through either route (Ctrl+Shift+M or MCP)
    let a = p.new_id();
    p.tracks[1].clips.push(Clip::new(a, ClipKind::Audio, "a", 0.0, 1.0));
    assert!(!add_mask(&mut p, a, MaskShape::Rect), "audio clips take no mask");
    assert!(mask_slot(&mut p, a, None).is_err(), "and clip.add_mask / clip.set_mask refuse them");
    assert!(p.clip(a).unwrap().mask.is_none());
}

/// Paste Attributes only touches the boxes that were ticked (and never timing or media).
#[test]
fn paste_attributes_applies_only_the_chosen_fields() {
    let mut p = Project::from_media(long_asset("a.mp4"));
    let ids = p.split_at(2.0, None);
    let src_id = p.all_clips().next().unwrap().1.id;
    {
        let c = p.clip_mut(src_id).unwrap();
        c.opacity.value = 0.25;
        c.blend = BlendMode::Screen;
        c.effects.push(Effect::new(EffectKind::Blur));
        c.label = 3;
    }
    let src = p.copy_attributes(src_id).unwrap();
    let target = ids[0];
    let (start, dur) = (p.clip(target).unwrap().start, p.clip(target).unwrap().duration);
    let set = crate::model::AttrSet { opacity: true, ..crate::model::AttrSet::NONE };
    assert_eq!(p.paste_attributes(&src, &[target], set), 1);
    let c = p.clip(target).unwrap();
    assert_eq!(c.opacity.value, 0.25);
    assert_eq!(c.blend, BlendMode::Normal, "blend was not ticked");
    assert!(c.effects.is_empty(), "effects were not ticked");
    assert_eq!(c.label, 0, "label was not ticked");
    assert_eq!((c.start, c.duration), (start, dur), "timing is never pasted");
    // ticking effects + label copies those too
    let set = crate::model::AttrSet { effects: true, label: true, ..crate::model::AttrSet::NONE };
    p.paste_attributes(&src, &[target], set);
    let c = p.clip(target).unwrap();
    assert_eq!(c.effects.len(), 1);
    assert_eq!(c.label, 3);
}

#[test]
fn frame_export_and_preview_sizes() {
    // downscale: render straight at the target
    assert_eq!(frame_render_size((1920, 1080), (960, 540)), (960, 540));
    assert_eq!(frame_render_size((1920, 1080), (1920, 1080)), (1920, 1080));
    // upscale (2x / 4x buttons): render at project size, ffmpeg enlarges with the chosen flag
    assert_eq!(frame_render_size((1920, 1080), (3840, 2160)), (1920, 1080));
    // mixed (wider but shorter) counts as an upscale, and degenerate sizes are clamped
    assert_eq!(frame_render_size((1920, 1080), (4000, 100)), (1920, 1080));
    assert_eq!(frame_render_size((0, 0), (0, 0)), (16, 16));
    // preview quality scales the canvas, keeps zero at zero and never goes below 16 px
    assert_eq!(preview_canvas((800, 600), 100), (800, 600));
    assert_eq!(preview_canvas((800, 600), 50), (400, 300));
    assert_eq!(preview_canvas((800, 600), 1), (200, 150)); // clamped to 25 %
    assert_eq!(preview_canvas((0, 600), 50), (0, 0));
    assert_eq!(preview_canvas((10, 10), 25), (16, 16));
}

/// The GPU renders at `self.canvas`, the player decodes at its own clamp — they must agree, or the
/// preview comes out squashed whenever the pane is wider than preview_max_width.
#[test]
fn canvas_clamp_keeps_the_aspect_ratio() {
    assert_eq!(clamp_canvas(800, 450, 1280), (800, 450), "under the limit: untouched");
    assert_eq!(clamp_canvas(800, 450, 320), (320, 180), "height scales with the width");
    assert_eq!(clamp_canvas(1920, 1080, 0), (1920, 1080), "0 = no limit (same as Player::set_canvas)");
    assert_eq!(clamp_canvas(1000, 3, 100), (100, 1), "never collapses to zero");
    // the same numbers the player would land on
    let (w, h) = (1600u32, 900u32);
    let max = 640u32;
    assert_eq!(clamp_canvas(w, h, max), (max, ((h as u64 * max as u64) / w as u64) as u32));
}

#[test]
fn effect_thumbnail_cache_keys() {
    let a = effect_thumb_key(EffectKind::Blur, (96, 54), "");
    assert_eq!(a, effect_thumb_key(EffectKind::Blur, (96, 54), ""), "stable for the same inputs");
    assert_ne!(a, effect_thumb_key(EffectKind::Vhs, (96, 54), ""), "kind matters");
    assert_ne!(a, effect_thumb_key(EffectKind::Blur, (192, 108), ""), "size matters");
    assert_ne!(a, effect_thumb_key(EffectKind::Blur, (96, 54), "C:/pic.png"), "stock image matters");
    // every kind gets its own key at one size
    let mut keys: Vec<u64> = EffectKind::ALL.iter().map(|&k| effect_thumb_key(k, (96, 54), "x")).collect();
    keys.sort_unstable();
    keys.dedup();
    assert_eq!(keys.len(), EffectKind::ALL.len());
    // no stock image set => the embedded default, unscaled at card size and resampled elsewhere
    assert_eq!(STOCK.len(), (STOCK_W * STOCK_H * 4) as usize, "embedded RGBA is not W*H*4");
    let card = effect_thumb_source("", STOCK_W, STOCK_H, Backend::Ffmpeg);
    assert_eq!(card.rgba, STOCK, "at card size the embedded image is copied through untouched");
    let f = effect_thumb_source("", 32, 24, Backend::Ffmpeg);
    assert_eq!((f.width, f.height), (32, 24));
    assert_eq!(f.rgba.len(), 32 * 24 * 4);
    assert!(f.rgba.chunks_exact(4).all(|p| p[3] == 255), "the stock image must be opaque");
    assert!(f.rgba.chunks_exact(4).any(|p| p[0] != p[1] || p[1] != p[2]), "the stock image has colour in it");
}

/// The frame writer really produces an image of the requested size (needs ffmpeg; skipped without).
#[test]
fn write_image_scales_and_writes() {
    if media::ffpipe::ffmpeg_exe().is_none() {
        println!("write_image test: no ffmpeg, skipped");
        return;
    }
    let dir = std::env::temp_dir().join(format!("se-frame-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let frame = effect_thumb_source("", 64, 48, Backend::Ffmpeg);
    for (name, quality) in [("shot.png", 100), ("shot.jpg", 80), ("shot.webp", 80)] {
        let out = dir.join(name);
        let opts = frame_ui::FrameExport {
            out: out.clone(),
            size: (128, 96), // upscaled by ffmpeg, like a 2x frame export
            scaler: Scaler::Bilinear,
            resize: "lanczos".into(),
            with_effects: true,
            quality,
        };
        write_image(&frame, &opts).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(out.is_file(), "{name} not written");
        let probe = media::probe(&out.to_string_lossy(), Backend::Ffmpeg).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!((probe.width, probe.height), (128, 96), "{name} size");
    }
    // a path ffmpeg cannot write is an error, not a panic
    let bad = frame_ui::FrameExport {
        out: PathBuf::from("Z:/nope/shot.png"),
        size: (64, 48),
        scaler: Scaler::Bilinear,
        resize: String::new(),
        with_effects: true,
        quality: 90,
    };
    assert!(write_image(&frame, &bad).is_err());
    assert!(write_image(&Frame::default(), &bad).is_err(), "an empty frame is refused");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Library-preview frame-step math: clamps at both ends of the file. The scrub-bar fraction-to-time
/// math (`scrub_time`) moved to `ui/preview.rs` along with the scrub bar itself — see
/// `preview::tests::scrub_time_clamps_to_bar`.
#[test]
fn lib_preview_seek_math() {
    assert_eq!(step_time(1.0, 25.0, true, 4.0), 1.04, "forward steps by 1/fps");
    assert_eq!(step_time(0.02, 25.0, false, 4.0), 0.0, "backward step clamps at 0");
    assert_eq!(step_time(3.99, 25.0, true, 4.0), 4.0, "forward step clamps at the file's duration");
}

/// Every action is dispatched (`act` matches exhaustively) and every pane can be toggled from
/// the View menu; here we only check the round-3 actions still carry their advertised bindings.
#[test]
fn round3_actions_are_bound() {
    use crate::hotkeys::Hotkeys;
    let h = Hotkeys::defaults();
    for (a, text) in [
        (Action::AddLastTransition, "Ctrl+T"),
        (Action::CopyAttributes, "Ctrl+Alt+C"),
        (Action::PasteAttributes, "Ctrl+Alt+V"),
        // bare M adds a marker (the user's explicit ask); the Marker tool sits on Shift+M
        (Action::AddMarker, "M"),
        (Action::AddMask, "Ctrl+Shift+M"),
        (Action::ExportFrame, "Ctrl+Shift+F"),
    ] {
        assert_eq!(h.text(a), text, "{a:?}");
    }
}

#[test]
fn anim_of_props_and_effect_params() {
    let mut c = Clip::new(1, ClipKind::Video, "c", 0.0, 4.0);
    assert!(anim_of(&mut c, "Position X").is_some());
    assert!(anim_of(&mut c, "Volume").is_some());
    assert!(anim_of(&mut c, "Nope").is_none());
    c.effects.push(crate::model::Effect::new(EffectKind::Blur));
    let pname = EffectKind::Blur.params()[0].name;
    assert!(anim_of(&mut c, &format!("Blur: {pname}")).is_some());
    assert!(anim_of(&mut c, "Blur: Nope").is_none());
}

// deviation (see PR body): the plan's `presets_pane_applies_reuse_rows` and
// `help_changelog_and_templates_save_round_trip` tests both need a live `&mut App` (to draw
// `Pane::Presets` / to call a `ToolDef::run`), and — as `tools_registry_tests.rs` already documents —
// there is no headless App-construction path anywhere in this crate (`eframe::CreationContext` has no
// public constructor). `Pane::Presets`'s new body is `library::reuse_ui` + the same 4 response-field
// handlers `library_pane.rs` already has for `Pane::Library`, copied verbatim — already covered by
// `library.rs`'s own `reuse_sections`/`reuse_pick`/`LibraryResponse` tests, which this PR does not touch.
// `help.changelog`/`templates.save`'s registration (name/kind/args, without invoking `run`) is checked
// below instead, the same non-App-dependent technique `run_tool_undoable_snapshots_only_mutate` above uses.

/// `help.changelog` and `templates.save` are registered in `TOOL_TABLES` (via `whatsnew::TOOLS`) with
/// the kind/args the plan specifies — `mcp::tools::find` reads the static registry and needs no `&mut App`.
#[test]
fn help_changelog_and_templates_save_are_registered() {
    use crate::mcp::tools::ToolKind;
    let help = crate::mcp::tools::find("help.changelog").expect("help.changelog must be registered");
    assert_eq!(help.kind, ToolKind::Read);
    assert!(help.args.is_empty());
    let save = crate::mcp::tools::find("templates.save").expect("templates.save must be registered");
    assert_eq!(save.kind, ToolKind::Ui);
    assert!(save.args.iter().any(|a| a.starts_with("name:string:true")), "{:?}", save.args);
    assert!(save.args.iter().any(|a| a.starts_with("clip_ids:")), "{:?}", save.args);
}

/// Tripwire: this PR does not migrate the 17 pre-existing raw `ctx.request_repaint_after(...)` call
/// sites named in CHANGELOG.md/goals.md (most live in files another workstream owns exclusively in a
/// later wave) — only the NEW code it adds (winpos.rs, whatsnew.rs) routes through `App::animate_until`.
/// Counts real call lines across the files that had them before this PR (skipping doc-comment text and
/// `animate_until`'s own internal `ctx.request_repaint_after(dt)` funnel call), so a future edit that
/// silently adds, removes or migrates one of the 17 is caught here instead of going unnoticed.
#[test]
fn pre_existing_repaint_sites_unchanged_and_named() {
    let files = [
        include_str!("mod.rs"),
        include_str!("mcp_exec.rs"),
        include_str!("lib_preview.rs"),
        include_str!("jobs.rs"),
        include_str!("../planner.rs"),
        include_str!("../preview.rs"),
        include_str!("../subtitles_ui.rs"),
    ];
    let count = files
        .iter()
        .flat_map(|f| f.lines())
        .filter(|l| !l.trim_start().starts_with("//"))
        .filter(|l| l.contains("request_repaint_after(") && !l.contains("request_repaint_after(dt)"))
        .count();
    assert_eq!(
        count, 17,
        "the count of pre-existing raw request_repaint_after sites moved — if that was intentional, \
         update this count AND the tracked-gap note in CHANGELOG.md/goals.md"
    );
}

// ---- ws:command-palette ----
/// `App::enabled`'s wave-0b `enabled_for` half already has its own pinned test
/// (`tools_registry_tests::action_enabled_toasts_reason`) against its own 3-arg shape; this exercises
/// the second guard match this workstream added (`enabled_for2`) the same way — via the pure fn
/// directly, since `enabled` itself needs a live `App` (no headless harness — see `App::new`'s doc
/// comment / the App-construction note in `tools_registry_tests.rs`).
#[test]
fn enabled_for2_reports_reason_for_known_disabled_actions() {
    assert_eq!(App::enabled_for2(Action::Undo, true, false, false, false), Err("Nothing to undo"));
    assert_eq!(App::enabled_for2(Action::Undo, false, false, false, false), Ok(()));
    assert_eq!(App::enabled_for2(Action::Redo, false, true, false, false), Err("Nothing to redo"));
    assert_eq!(
        App::enabled_for2(Action::Split, false, false, true, false),
        Err("Nothing to split — the timeline is empty")
    );
    for a in [Action::Delete, Action::RippleDelete] {
        assert_eq!(App::enabled_for2(a, false, false, false, true), Err("Select something to delete first"));
        assert_eq!(App::enabled_for2(a, false, false, false, false), Ok(()));
    }
    // an action this guard doesn't know about is always Ok
    assert_eq!(App::enabled_for2(Action::CommandPalette, true, true, true, true), Ok(()));
}
