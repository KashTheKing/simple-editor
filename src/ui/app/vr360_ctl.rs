//! ---- ws:parity-360 ----
//! App side of 360 Stabilize (`engine::vr360`): the job (frames decoded and matched on a worker), the
//! keys it writes onto the clip's 360 Transform effect as one undo step, the effect row's menu and the
//! `clip.stabilize_360` tool. The other 360° filters are library shaders (`library/effects/360-*.glsl`,
//! `fisheye-to-360`, `flat-to-360`) and need nothing here.

use super::feedback::{Toast, ToastKind};
use super::tools_args::Args;
use super::*;
use crate::engine::vr360::{self, StabilizeOptions, V3};
use crate::mcp::tools::{ToolDef, ToolKind, ToolOutcome};
use crate::model::{Clip, Effect};

/// One 360 Stabilize in flight.
struct Run {
    clip: Id,
    fps: f64,
    opts: StabilizeOptions,
    inner: Arc<Progress>,
    deltas: Arc<Mutex<Vec<Option<V3>>>>,
    outer: Arc<Progress>,
}

#[derive(Default)]
pub(super) struct State {
    runs: Vec<Run>,
}

/// Start stabilizing `clip`: its frames, one per project frame, are analysed on a job thread.
pub(super) fn start(app: &mut App, clip: Id, opts: StabilizeOptions) -> Result<Arc<Progress>, String> {
    if app.vr360.runs.iter().any(|r| r.clip == clip) {
        return Err("that clip is already being stabilized".into());
    }
    let c = app.project.clip(clip).ok_or("pick a 360° clip to stabilize")?;
    if !c.is_visual() {
        return Err("360 Stabilize needs a video clip".into());
    }
    let path = app.project.asset(c.asset).map(|a| a.path.clone()).ok_or("that clip has no footage to stabilize")?;
    let fps = app.project.fps.max(1.0);
    let n = ((c.duration * fps).round() as usize).clamp(2, 200_000);
    // clip-local frame i -> source time, up front, so the worker never touches the Project
    let times: Vec<f64> = (0..n).map(|i| c.src_time(c.start + i as f64 / fps)).collect();
    let backend = app.backend();
    let deltas: Arc<Mutex<Vec<Option<V3>>>> = Arc::default();
    let sink = deltas.clone();
    let inner = export::spawn_job("stabilize-360", move |prog| {
        let d = vr360::analyse(&path, &times, backend, prog)?;
        if d.is_empty() {
            return Err("the clip is too short to stabilize".into());
        }
        *sink.lock().unwrap_or_else(|e| e.into_inner()) = d;
        prog.set(1.0, "Done");
        Ok(())
    });
    let outer = Progress::new();
    app.vr360.runs.push(Run { clip, fps, opts, inner, deltas, outer: outer.clone() });
    Ok(outer)
}

/// Index of the clip's 360 Transform effect, adding one at the front of the stack (stabilize before
/// any reprojection such as 360 to Flat) when it has none.
pub(crate) fn transform_effect(c: &mut Clip) -> usize {
    let is_it = |e: &Effect| {
        e.kind == EffectKind::Shader && crate::engine::shaderlib::parse_meta(&e.shader).name == vr360::TRANSFORM_NAME
    };
    if let Some(i) = c.effects.iter().position(is_it) {
        return i;
    }
    let mut e = Effect::new(EffectKind::Shader);
    e.set_shader(vr360::TRANSFORM_SHADER);
    // the file's defaults show the effect off in the Gallery; a stabilize starts from no turn at all
    e.params.iter_mut().for_each(|p| p.value = 0.0);
    c.effects.insert(0, e);
    0
}

/// Write the per-frame correction (`vr360::correction`, degrees) as yaw / pitch / roll keys of the
/// clip's 360 Transform effect, replacing the keys it had. A turn the user set by hand (a constant,
/// not keys: "this way is forward") is kept, applied after the correction. Returns the keys written.
pub(crate) fn apply_correction(c: &mut Clip, corr: &[V3], fps: f64) -> usize {
    let i = transform_effect(c);
    let fx = &mut c.effects[i];
    if fx.params.len() < 3 {
        fx.set_shader(vr360::TRANSFORM_SHADER); // a hand-edited copy lost its controls: start over
    }
    let base: Vec<f64> = fx.params.iter().take(3).map(|p| if p.keys.is_empty() { p.value } else { 0.0 }).collect();
    let corr: Vec<V3> = if base.iter().all(|v| v.abs() < 1e-9) {
        corr.to_vec()
    } else {
        // the source is read at R_corr R_user d: steady (R_corr) and then turned the user's way
        let user = vr360::Quat::from_ypr([base[0], base[1], base[2]].map(f64::to_radians));
        corr.iter()
            .map(|v| vr360::Quat::from_ypr(v.map(f64::to_radians)).mul(user).ypr().map(f64::to_degrees))
            .collect()
    };
    let mut written = 0;
    for (axis, p) in fx.params.iter_mut().take(3).enumerate() {
        let keys: Vec<(f64, f64)> = corr.iter().enumerate().map(|(f, v)| (f as f64 / fps, v[axis])).collect();
        let keys = vr360::thin_keys(&keys, 0.02);
        p.clear_keys(0.0);
        p.value = 0.0;
        if keys.iter().all(|k| k.1.abs() < 1e-6) {
            continue; // an axis left alone (or never moved) stays a plain 0
        }
        for (t, v) in keys {
            p.key_at(t, v);
            written += 1;
        }
    }
    written
}

fn finish(app: &mut App, run: Run) {
    if let Some(e) = run.inner.error() {
        app.push_toast(Toast::new(format!("360 Stabilize failed: {e}")).kind(ToastKind::Error));
        run.outer.finish(Some(e));
        return;
    }
    let deltas = std::mem::take(&mut *run.deltas.lock().unwrap_or_else(|e| e.into_inner()));
    let missed = deltas.iter().filter(|d| d.is_none()).count();
    let corr = vr360::correction(&deltas, run.fps, run.opts);
    let before = app.project.to_json();
    let Some(c) = app.project.clip_mut(run.clip) else {
        let e = "the clip was deleted while it was being stabilized".to_string();
        run.outer.finish(Some(e));
        return;
    };
    apply_correction(c, &corr, run.fps);
    app.push_undo_labeled(before, "360 Stabilize");
    app.after_edit();
    let mut msg = format!("Stabilized {} frames: keys on the clip's 360 Transform effect", corr.len());
    if missed * 5 > deltas.len() {
        msg += &format!(" ({missed} frames had too little detail to measure)");
    }
    app.push_toast(Toast::new(msg).kind(ToastKind::Success));
    run.outer.finish(None);
}

pub(super) fn tick(app: &mut App, ctx: &egui::Context) {
    if let Some((clip, idx, opts)) = crate::ui::inspector::take_stabilize_360() {
        // the menu sits on the clip's own 360 Transform row: keep the user's other effects where they are
        let ok = app.project.clip(clip).and_then(|c| c.effects.get(idx)).is_some();
        match ok.then(|| start(app, clip, opts)).unwrap_or(Err("no such effect".into())) {
            Ok(p) => {
                let mut t = Toast::new("360 Stabilize…");
                t.progress = Some(p);
                app.push_toast(t);
            }
            Err(e) => app.toast(e),
        }
    }
    if app.vr360.runs.is_empty() {
        return;
    }
    let mut i = 0;
    while i < app.vr360.runs.len() {
        if app.vr360.runs[i].inner.is_done() {
            let run = app.vr360.runs.remove(i);
            finish(app, run);
        } else {
            let r = &app.vr360.runs[i];
            r.outer.set(r.inner.fraction(), "Analysing 360° motion…");
            i += 1;
        }
    }
    ctx.request_repaint_after(std::time::Duration::from_millis(150));
}

pub const TOOLS: &[ToolDef] = &[ToolDef {
    name: "clip.stabilize_360",
    desc: "360 Stabilize (job): measure how a 360° equirectangular clip's camera turns frame to frame and write yaw/pitch/roll keys on its 360 Transform effect (added first in the stack if missing) that hold the view steady. smooth = seconds of motion to keep (default 1; 0 locks the view to the first frame). yaw/pitch/roll false leave that axis alone (pitch+roll only = level the horizon, keep the pans). One undo step.",
    args: &[
        "clip_id:integer:false:default: the first selected clip",
        "smooth:number:false:seconds, default 1; 0 = lock to the first frame",
        "yaw:boolean:false:default true",
        "pitch:boolean:false:default true",
        "roll:boolean:false:default true",
    ],
    kind: ToolKind::Job,
    run: |app, args| {
        let a = Args(args);
        let clip = a.id("clip_id").or_else(|| app.selection.first().copied()).ok_or("give clip_id or select a clip")?;
        let opts = StabilizeOptions {
            smooth: a.f64("smooth").unwrap_or(1.0).clamp(0.0, 30.0),
            axes: [a.bool("yaw").unwrap_or(true), a.bool("pitch").unwrap_or(true), a.bool("roll").unwrap_or(true)],
        };
        if opts.axes == [false; 3] {
            return Err("nothing to stabilize: yaw, pitch and roll are all off".into());
        }
        Ok(ToolOutcome::Job(start(app, clip, opts)?, PathBuf::new()))
    },
}];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ClipKind;

    #[test]
    fn stabilize_keys_land_on_one_360_transform_effect_at_the_front() {
        let mut c = Clip::new(1, ClipKind::Video, "360", 0.0, 1.0);
        c.effects.push(Effect::new(EffectKind::Blur));
        let corr: Vec<V3> = (0..30).map(|f| [0.0, if f % 2 == 0 { 1.0 } else { -1.0 }, f as f64 * 0.1]).collect();
        let n = apply_correction(&mut c, &corr, 30.0);
        assert_eq!(c.effects.len(), 2);
        assert_eq!(transform_effect(&mut c), 0, "found again, not added twice");
        let fx = &c.effects[0];
        assert_eq!(fx.name(), vr360::TRANSFORM_NAME);
        assert!(fx.params[0].keys.is_empty() && fx.params[0].value == 0.0, "yaw never moved: no keys");
        assert_eq!(fx.params[1].keys.len(), 30, "a shake needs a key every frame");
        assert_eq!(fx.params[2].keys.len(), 2, "a straight roll ramp needs two");
        assert_eq!(n, 32);
        assert!((fx.at(1, 1.0 / 30.0) + 1.0).abs() < 1e-9);
        // running it again replaces the keys rather than adding to them
        apply_correction(&mut c, &corr[..10], 30.0);
        assert_eq!(c.effects[0].params[1].keys.len(), 10);
    }

    #[test]
    fn stabilize_keeps_a_hand_set_forward_direction() {
        let mut c = Clip::new(1, ClipKind::Video, "360", 0.0, 1.0);
        let i = transform_effect(&mut c);
        assert!(c.effects[i].params.iter().all(|p| p.value == 0.0), "not the file's show-off default");
        c.effects[i].params[0].value = 30.0; // the user turned forward 30° right
        let corr: Vec<V3> = vec![[0.0, 2.0, 0.0], [0.0, -2.0, 0.0]];
        apply_correction(&mut c, &corr, 30.0);
        let fx = &c.effects[i];
        for (f, v) in corr.iter().enumerate() {
            let t = f as f64 / 30.0;
            let got = vr360::Quat::from_ypr([fx.at(0, t), fx.at(1, t), fx.at(2, t)].map(f64::to_radians));
            let want = vr360::Quat::from_ypr(v.map(f64::to_radians)).mul(vr360::Quat::from_ypr([
                30f64.to_radians(),
                0.0,
                0.0,
            ]));
            let d = want.conj().mul(got).normalized().rotvec();
            assert!(d.iter().all(|x| x.abs() < 1e-6), "frame {f}: {d:?}");
        }
    }

    #[test]
    fn stabilize_360_tool_is_registered_once_as_a_job() {
        let t: Vec<_> = crate::mcp::tools::all().filter(|t| t.name == "clip.stabilize_360").collect();
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].kind, ToolKind::Job);
    }
}
