//! Style summary: a deterministic Markdown description of how a project was edited — statistics an AI (or a
//! human) can turn into a reusable style guide: format, duration, tracks, cut cadence (count, mean/median
//! clip length, shortest/longest), transitions used (kinds, durations), effects (kinds + typical params),
//! text styles (fonts, sizes, colours, outlines/shadows), colour labels, retimes, masks and node graphs,
//! audio (levels, fades, music/SFX split by length), subtitles usage, the planner (done/todo), and the
//! free-form notes verbatim.
//! Also served by the MCP tool `style.summary` and exported via File ▸ Export Style Summary (.md).

use crate::model::{
    ClipKind, EffectKind, NodeKind, PlanItem, Project, TextStyle, Track, TrackKind, TransitionKind, LABEL_COLORS,
};
use std::fmt::Write;

/// Threshold between "short SFX" and "music/long" audio clips (matches the library filter).
const SFX_MAX: f64 = 10.0;

fn median(mut v: Vec<f64>) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(f64::total_cmp);
    let m = v.len() / 2;
    if v.len() % 2 == 0 {
        (v[m - 1] + v[m]) / 2.0
    } else {
        v[m]
    }
}

/// "8", "0.5", "1.25" — up to two decimals, trailing zeros trimmed.
fn num(v: f64) -> String {
    let s = format!("{v:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

fn hex(c: [u8; 4]) -> String {
    if c[3] == 255 {
        format!("#{:02X}{:02X}{:02X}", c[0], c[1], c[2])
    } else {
        format!("#{:02X}{:02X}{:02X}{:02X}", c[0], c[1], c[2], c[3])
    }
}

/// One-line description of a text style ("\"Segoe UI\" 72 px #FFFFFF, bold, outline 3 px #000000, shadow").
fn text_desc(ts: &TextStyle) -> String {
    let mut d = format!("\"{}\" {} px {}", ts.font, num(ts.size as f64), hex(ts.color));
    if ts.bold {
        d.push_str(", bold");
    }
    if ts.italic {
        d.push_str(", italic");
    }
    if ts.outline_width > 0.0 {
        let _ = write!(d, ", outline {} px {}", num(ts.outline_width as f64), hex(ts.outline_color));
    }
    if ts.shadow {
        d.push_str(", shadow");
    }
    if ts.box_color[3] > 0 {
        let _ = write!(d, ", box {}", hex(ts.box_color));
    }
    d
}

fn plan_walk(items: &[PlanItem], indent: usize, out: &mut String, done: &mut usize, total: &mut usize) {
    for it in items {
        *total += 1;
        if it.done {
            *done += 1;
        }
        let _ = writeln!(out, "{}- [{}] {}", "  ".repeat(indent), if it.done { 'x' } else { ' ' }, it.title);
        plan_walk(&it.children, indent + 1, out, done, total);
    }
}

/// Sorted `(item, count)` pairs from an iterator of strings (deterministic listing of duplicates).
fn counted(items: impl Iterator<Item = String>) -> Vec<(String, usize)> {
    let mut v: Vec<String> = items.collect();
    v.sort();
    let mut out: Vec<(String, usize)> = Vec::new();
    for s in v {
        match out.last_mut() {
            Some((p, n)) if *p == s => *n += 1,
            _ => out.push((s, 1)),
        }
    }
    out
}

pub fn style_summary(project: &Project) -> String {
    let p = project;
    let mut s = String::with_capacity(2048);
    // The main timeline (the stash while a sequence is open for editing).
    let main: &[Track] = match (&p.main_stash, p.editing) {
        (Some(st), Some(_)) => &st.tracks,
        _ => &p.tracks,
    };
    // Every track exactly once: an open sequence's stored tracks are empty (they live in p.tracks).
    let mut all: Vec<&Track> = p.tracks.iter().collect();
    if let Some(st) = &p.main_stash {
        all.extend(st.tracks.iter());
    }
    for sq in &p.sequences {
        all.extend(sq.tracks.iter());
    }
    let all_clips = || all.iter().flat_map(|t| t.clips.iter());

    let _ = writeln!(s, "# Style Summary — {}\n", p.name);

    // ---- Format ----
    let (w, h, fps) = match (&p.main_stash, p.editing) {
        (Some(st), Some(_)) => (st.width, st.height, st.fps),
        _ => (p.width, p.height, p.fps),
    };
    let duration = main.iter().map(|t| t.end()).fold(0.0, f64::max);
    let nv = main.iter().filter(|t| t.kind == TrackKind::Video).count();
    let na = main.len() - nv;
    let _ = writeln!(s, "## Format");
    let _ = writeln!(s, "- {}×{} @ {} fps", w, h, num(fps));
    let _ = writeln!(s, "- Duration: {} s", num(duration));
    let _ = writeln!(s, "- Tracks: {nv} video, {na} audio");
    let _ = writeln!(s, "- Scaler: {}\n", p.scaler.name());

    // ---- Timeline stats ----
    let _ = writeln!(s, "## Timeline");
    let mut cadence_cuts = 0usize;
    for kind in [TrackKind::Video, TrackKind::Audio] {
        let lens: Vec<f64> =
            main.iter().filter(|t| t.kind == kind).flat_map(|t| t.clips.iter().map(|c| c.duration)).collect();
        let cuts: usize = main.iter().filter(|t| t.kind == kind).map(|t| t.clips.len().saturating_sub(1)).sum();
        if kind == TrackKind::Video || (cadence_cuts == 0 && !lens.is_empty()) {
            cadence_cuts = cadence_cuts.max(cuts);
        }
        let label = if kind == TrackKind::Video { "Video" } else { "Audio" };
        if lens.is_empty() {
            let _ = writeln!(s, "- {label}: no clips");
            continue;
        }
        let mean = lens.iter().sum::<f64>() / lens.len() as f64;
        let (min, max) = lens.iter().fold((f64::MAX, f64::MIN), |(a, b), &v| (a.min(v), b.max(v)));
        let _ = writeln!(
            s,
            "- {label}: {} clips, {} cuts — length mean {} s, median {} s, min {} s, max {} s",
            lens.len(),
            cuts,
            num(mean),
            num(median(lens.clone())),
            num(min),
            num(max)
        );
    }
    if duration > 0.0 && cadence_cuts > 0 {
        let _ = writeln!(s, "- Cut cadence: {} cuts/min", num(cadence_cuts as f64 / (duration / 60.0)));
    }
    let _ = writeln!(s);

    // ---- Transitions ----
    let _ = writeln!(s, "## Transitions");
    let mut any = false;
    for kind in TransitionKind::ALL {
        let durs: Vec<f64> =
            all.iter().flat_map(|t| t.transitions.iter()).filter(|x| x.kind == kind).map(|x| x.duration).collect();
        if !durs.is_empty() {
            any = true;
            let _ = writeln!(s, "- {} ×{} — median {} s", kind.name(), durs.len(), num(median(durs)));
        }
    }
    if !any {
        let _ = writeln!(s, "(none)");
    }
    let _ = writeln!(s);

    // ---- Effects ----
    // Every effect on a clip's linear stack *and* every Effect node in a clip's graph, grouped by the
    // catalogue category so 24 kinds still read as a handful of lines.
    let all_effects = || {
        all_clips().flat_map(|c| {
            let stack = c.effects.iter();
            let nodes = c.graph.iter().flat_map(|g| {
                g.nodes.iter().filter_map(|n| match &n.kind {
                    NodeKind::Effect(e) => Some(e),
                    _ => None,
                })
            });
            stack.chain(nodes)
        })
    };
    let _ = writeln!(s, "## Effects");
    let mut any = false;
    let mut category = "";
    for kind in EffectKind::ALL {
        let instances: Vec<_> = all_effects().filter(|e| e.kind == kind).collect();
        if instances.is_empty() {
            continue;
        }
        any = true;
        if kind.category() != category {
            category = kind.category();
            let _ = writeln!(s, "### {category}");
        }
        let params: Vec<String> = kind
            .params()
            .iter()
            .enumerate()
            .map(|(i, spec)| {
                let m = median(instances.iter().map(|e| e.at(i, 0.0)).collect());
                if kind.is_bool_param(i) {
                    format!("{} {}", spec.name, if m >= 0.5 { "on" } else { "off" })
                } else {
                    format!("{} {}", spec.name, num(m))
                }
            })
            .collect();
        let mut line = format!("- {} ×{} — median {}", kind.name(), instances.len(), params.join(", "));
        let off = instances.iter().filter(|e| !e.enabled).count();
        if off > 0 {
            let _ = write!(line, " ({off} disabled)");
        }
        let masked = instances.iter().filter(|e| e.mask.is_some()).count();
        if masked > 0 {
            let _ = write!(line, " ({masked} masked)");
        }
        let _ = writeln!(s, "{line}");
    }
    if !any {
        let _ = writeln!(s, "(none)");
    }
    let _ = writeln!(s);

    // ---- Masks & node graphs ----
    let _ = writeln!(s, "## Masks & node graphs");
    let clip_masks = counted(all_clips().filter_map(|c| c.mask.as_ref()).map(|m| m.shape.name().to_string()));
    let effect_masks = counted(all_effects().filter_map(|e| e.mask.as_ref()).map(|m| m.shape.name().to_string()));
    let graphs: Vec<_> = all_clips().filter_map(|c| c.graph.as_ref()).collect();
    if clip_masks.is_empty() && effect_masks.is_empty() && graphs.is_empty() {
        let _ = writeln!(s, "(none)");
    }
    for (label, list) in [("Clip masks", &clip_masks), ("Effect masks", &effect_masks)] {
        if list.is_empty() {
            continue;
        }
        let total: usize = list.iter().map(|(_, n)| n).sum();
        let by: Vec<String> = list.iter().map(|(name, n)| format!("{n} {name}")).collect();
        let _ = writeln!(s, "- {label}: {total} ({})", by.join(", "));
    }
    if !graphs.is_empty() {
        let nodes: Vec<String> = graphs.iter().flat_map(|g| g.nodes.iter()).map(|n| n.kind.title()).collect();
        let total = nodes.len();
        let by: Vec<String> = counted(nodes.into_iter()).iter().map(|(name, n)| format!("{n} {name}")).collect();
        let _ = writeln!(s, "- Node graphs: {} clips, {total} nodes ({})", graphs.len(), by.join(", "));
    }
    let _ = writeln!(s);

    // ---- Text styles ----
    let _ = writeln!(s, "## Text styles");
    let styles = counted(all_clips().filter_map(|c| c.text.as_ref()).map(text_desc));
    if styles.is_empty() {
        let _ = writeln!(s, "(none)");
    }
    for (desc, n) in styles {
        let _ = writeln!(s, "- {desc} ×{n}");
    }
    let _ = writeln!(s);

    // ---- Labels ----
    let _ = writeln!(s, "## Labels");
    let mut counts = [0usize; 8];
    for c in all_clips() {
        let l = p.clip_label(c) as usize;
        if (1..=8).contains(&l) {
            counts[l - 1] += 1;
        }
    }
    if counts.iter().all(|&n| n == 0) {
        let _ = writeln!(s, "(none)");
    }
    for (i, &n) in counts.iter().enumerate() {
        if n > 0 {
            let _ = writeln!(s, "- {} ×{}", LABEL_COLORS[i].0, n);
        }
    }
    let _ = writeln!(s);

    // ---- Retimes ----
    let _ = writeln!(s, "## Retimes");
    let speeds = counted(
        all_clips()
            .filter(|c| (c.speed - 1.0).abs() > 1e-9 && c.freeze.is_none())
            .map(|c| format!("{}×", num(c.speed))),
    );
    let reversed = all_clips().filter(|c| c.reverse).count();
    let frozen = all_clips().filter(|c| c.freeze.is_some()).count();
    if speeds.is_empty() && reversed == 0 && frozen == 0 {
        let _ = writeln!(s, "(none)");
    }
    for (sp, n) in speeds {
        let _ = writeln!(s, "- Speed {sp} ×{n}");
    }
    if reversed > 0 {
        let _ = writeln!(s, "- Reversed ×{reversed}");
    }
    if frozen > 0 {
        let _ = writeln!(s, "- Freeze frames ×{frozen}");
    }
    let _ = writeln!(s);

    // ---- Audio ----
    let _ = writeln!(s, "## Audio");
    let audio: Vec<_> = all_clips().filter(|c| c.kind == ClipKind::Audio).collect();
    if audio.is_empty() {
        let _ = writeln!(s, "(none)");
    } else {
        let sfx = audio.iter().filter(|c| c.duration <= SFX_MAX).count();
        let _ =
            writeln!(s, "- {} audio clips: {} short SFX (≤10 s), {} music/long", audio.len(), sfx, audio.len() - sfx);
        let vol = audio.iter().filter(|c| !c.volume.is_default(1.0)).count();
        let pan = audio.iter().filter(|c| !c.pan.is_default(0.0)).count();
        let fades: Vec<f64> = audio.iter().flat_map(|c| [c.fade_in, c.fade_out]).filter(|&f| f > 0.0).collect();
        let faded = audio.iter().filter(|c| c.fade_in > 0.0 || c.fade_out > 0.0).count();
        let mut line = format!("- Volume adjusted on {vol}, pan on {pan}");
        if faded > 0 {
            let _ = write!(line, ", fades on {} (median {} s)", faded, num(median(fades)));
        }
        let _ = writeln!(s, "{line}");
    }
    let _ = writeln!(s);

    // ---- Subtitles ----
    let _ = writeln!(s, "## Subtitles");
    if p.subtitles.is_empty() {
        let _ = writeln!(s, "(none)");
    } else {
        let covered: f64 = p.subtitles.iter().map(|c| c.end - c.start).sum();
        let shown = if p.show_subtitles { "shown" } else { "hidden" };
        let _ = writeln!(s, "- {} cues, {} s covered, {shown}", p.subtitles.len(), num(covered));
        let _ = writeln!(s, "- Style: {}", text_desc(&p.subtitle_style));
    }
    let _ = writeln!(s);

    // ---- Sequences ----
    let _ = writeln!(s, "## Sequences");
    if p.sequences.is_empty() {
        let _ = writeln!(s, "(none)");
    }
    for sq in &p.sequences {
        let ntracks = p.sequence_tracks(sq.id).map(|t| t.len()).unwrap_or(0);
        let _ = writeln!(s, "- {} — {} s, {} tracks", sq.name, num(p.sequence_duration(sq.id)), ntracks);
    }
    let _ = writeln!(s);

    // ---- Planner ----
    let _ = writeln!(s, "## Planner");
    if p.plan.is_empty() {
        let _ = writeln!(s, "(none)");
    } else {
        let mut list = String::new();
        let (mut done, mut total) = (0, 0);
        plan_walk(&p.plan, 0, &mut list, &mut done, &mut total);
        let _ = writeln!(s, "{done}/{total} done");
        s.push_str(&list);
    }
    let _ = writeln!(s);

    // ---- Notes ----
    let _ = writeln!(s, "## Notes");
    if p.notes.is_empty() {
        let _ = writeln!(s, "(none)");
    } else {
        let _ = writeln!(s, "{}", p.notes);
    }

    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Asset, AudioStreamInfo, Effect, Id, Mask, MaskShape};

    fn asset(id: Id, dur: f64, streams: usize) -> Asset {
        Asset {
            id,
            path: format!("C:/style-test-{id}.mp4"),
            kind: ClipKind::Video,
            duration: dur,
            width: 1280,
            height: 720,
            fps: 30.0,
            audio_streams: (0..streams)
                .map(|i| AudioStreamInfo { index: i, channels: 2, sample_rate: 48000, ..Default::default() })
                .collect(),
            codec: "h264".into(),
            folder: String::new(),
            tags: Vec::new(),
            label: 0,
            description: String::new(),
        }
    }

    #[test]
    fn summary_of_hand_built_project() {
        let mut p = Project::from_media(asset(0, 10.0, 1));
        let right = p.split_at(4.0, None)[0]; // V: [0,4)[4,10)  A: same
        p.add_transition(right, TransitionKind::CrossFade, 1.0);
        let v = p.tracks[0].clips[0].id;
        p.clip_mut(v).unwrap().effects.push(Effect::new(EffectKind::Blur));
        // a round-3 effect with a bool param and a mask, so both render in the summary
        let mut vhs = Effect::new(EffectKind::Vhs);
        vhs.mask = Some(Mask::new(MaskShape::Ellipse));
        p.clip_mut(v).unwrap().effects.push(vhs);
        p.clip_mut(v).unwrap().effects.push(Effect::new(EffectKind::Curves));
        p.clip_mut(v).unwrap().mask = Some(Mask::new(MaskShape::Rect));
        p.clip_mut(v).unwrap().label = 3; // Yellow
                                          // a node graph on the other half: Input -> Threshold -> Output, counted with the stack effects
        p.add_node(right, NodeKind::Effect(Effect::new(EffectKind::Threshold)), 160.0, 0.0);
        let txt = p.add_text_clip(1.0, 2.0);
        p.clip_mut(txt).unwrap().text.as_mut().unwrap().outline_width = 3.0;
        let a = p.tracks[2].clips[0].id; // audio left half (text clip pushed V onto a new track? no: audio tracks after video)
        let a = if p.clip(a).map(|c| c.kind) == Some(ClipKind::Audio) { a } else { p.tracks[1].clips[0].id };
        p.clip_mut(a).unwrap().fade_in = 0.5;
        p.clip_mut(a).unwrap().volume.value = 0.8;
        p.set_speed(&[right], 2.0, false);
        p.add_cue(0.0, 2.5, "hello");
        let t = p.plan_add(None, "Intro");
        p.plan_item_mut(t).unwrap().done = true;
        p.plan_add(Some(t), "Hook");
        p.notes = "fast cuts, punchy".into();
        let md = style_summary(&p);
        for h in [
            "# Style Summary — style-test-0",
            "## Format",
            "- 1280×720 @ 30 fps",
            "## Timeline",
            "## Transitions",
            "- Cross Fade ×2 — median 1 s", // mirrored onto the linked audio cut
            "## Effects",
            "### Stylize",
            "- Blur ×1 — median Radius 8",
            // round-3 kinds read as well as the old ones: bool params as on/off, masks called out
            "- VHS ×1 — median Noise 0.3, Chroma bleed 0.5, Scanlines 0.4, Tracking jitter 0.2, \
             Head switching 0.3, Sharpen ringing 0.4, Colour bleed only off, Tape wear 0.2 (1 masked)",
            "### Adjustments",
            "- Threshold ×1 — median Level 0.5, Softness 0.05, Per channel off",
            "- Color Curves ×1 — median Master 1/4 0.25",
            "## Masks & node graphs",
            "- Clip masks: 1 (1 Rectangle)",
            "- Effect masks: 1 (1 Ellipse)",
            "- Node graphs: 1 clips, 3 nodes (1 Input, 1 Output, 1 Threshold)",
            "## Text styles",
            "outline 3 px #000000 ×1",
            "## Labels",
            "- Yellow ×1",
            "## Retimes",
            "- Speed 2× ×2", // linked audio follows
            "## Audio",
            "- Volume adjusted on 1, pan on 0, fades on 1 (median 0.5 s)",
            "## Subtitles",
            "- 1 cues, 2.5 s covered, shown",
            "## Sequences",
            "## Planner",
            "1/2 done",
            "- [x] Intro",
            "  - [ ] Hook",
            "## Notes",
            "fast cuts, punchy",
        ] {
            assert!(md.contains(h), "missing {h:?} in:\n{md}");
        }
        // timeline numbers: 3 video clips (2 halves + text), 2 audio clips
        assert!(md.contains("- Audio: 2 clips, 1 cuts"), "{md}");
    }

    #[test]
    fn summary_of_empty_project() {
        let md = style_summary(&Project::new());
        for h in ["## Format", "- Video: no clips", "## Effects", "(none)", "## Notes"] {
            assert!(md.contains(h), "missing {h:?} in:\n{md}");
        }
    }
}
