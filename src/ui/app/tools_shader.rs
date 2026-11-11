//! ---- ws:shader-library ----
//! Shader effects and shader transitions, App side: the `clip.set_shader` / `shader.check` /
//! `transition.set_shader` MCP tools and the two "apply" entry points the library uses.
//! GL work (compiling a source to read its log) happens here, on the UI thread, where tools run.

use super::tools_helpers::*;
use super::*;
use crate::mcp::tools::{ToolDef, ToolKind, ToolOutcome};

#[derive(Default)]
pub struct ShaderLibState {
    /// Library-effect thumbnails handed to the Effects list (kept alive here).
    thumbs: Vec<egui::TextureHandle>,
}

pub(super) fn act(_app: &mut App, _a: Action) -> bool {
    false
}

/// While the Effects list is on screen: render the thumbnails of installed library effects that have
/// none yet, two per frame, through the same `effect_preview` path as the built-in cards. Each
/// source compiles once (the program cache keys it); one that fails gets the plain stock picture, so
/// nothing is retried.
pub(super) fn tick(app: &mut App, ctx: &egui::Context) {
    if app.gpu.is_none() || app.export.is_some() || !app.pane_drawn(Pane::Effects) {
        return;
    }
    let items = crate::ui::transitions_ui::library_items(crate::engine::library::Kind::Effect, ctx.input(|i| i.time));
    let todo: Vec<_> = items.iter().filter(|i| !effects_ui::has_library_thumbnail(&i.id)).take(2).collect();
    if todo.is_empty() {
        return;
    }
    let (w, h) = (super::thumbs::STOCK_W, super::thumbs::STOCK_H);
    let src = super::thumbs::effect_thumb_source(&app.settings.effect_thumb_image, w, h, app.backend());
    for item in todo {
        let effect = Effect::from_shader(&item.src);
        let mut out = Frame::default();
        let ok = match app.gpu.as_mut() {
            Some(gpu) => super::guarded(|| gpu.effect_preview(&src, &effect, 0.35, &mut out)).unwrap_or(false),
            None => false,
        };
        let f = if ok && out.rgba.len() == (out.width * out.height * 4) as usize { &out } else { &src };
        let img = egui::ColorImage::from_rgba_premultiplied([f.width as usize, f.height as usize], &f.rgba);
        let tex = ctx.load_texture(format!("libfx_{}", item.id), img, egui::TextureOptions::LINEAR);
        effects_ui::set_library_thumbnail(&item.id, tex.id());
        app.shader_lib.thumbs.push(tex);
    }
    ctx.request_repaint(); // ends once every installed effect has its thumbnail
}

pub(super) fn windows(_app: &mut App, _ctx: &egui::Context) {}

const NO_GPU: &str = "the GPU renderer is off - this shader cannot be compiled or previewed";

impl App {
    /// CONTRACT (used by the library's "apply"): add a shader effect (source in the
    /// `library/README.md` format) to `clip`, or to every selected clip when None. Returns how many
    /// clips got it. Does NOT push undo itself.
    pub(crate) fn apply_shader_effect(&mut self, src: &str, clip: Option<Id>) -> Result<usize, String> {
        let ids: Vec<Id> = match clip {
            Some(id) => vec![id],
            None => self.selection.clone(),
        };
        let mut n = 0;
        for id in ids {
            // a node-graph clip never reads its effect list (see effects_ui), an audio clip has no pixels
            if let Some(c) = self.project.clip_mut(id).filter(|c| c.is_visual() && !c.uses_graph()) {
                c.effects.push(Effect::from_shader(src));
                n += 1;
            }
        }
        if n == 0 {
            return Err(if clip.is_some() { "that clip cannot take a shader effect" } else { "select a clip first" }.into());
        }
        Ok(n)
    }

    /// CONTRACT (used by the library's "apply"): put a shader transition (source in the
    /// `library/README.md` format) on the cut at the start of `clip` - or, when None, on the selected
    /// transition, else on the selected clip's cut, else on the cut nearest the playhead - replacing
    /// what is there. Returns the transition's id. Does NOT push undo itself.
    pub(crate) fn apply_transition_shader(&mut self, src: &str, clip: Option<Id>) -> Result<Id, String> {
        let dur = self.transitions_ui.duration;
        let selected = self.sel_transitions.iter().copied().find(|&t| self.project.transition_mut(t).is_some());
        let tid = match (clip, selected) {
            (None, Some(t)) => t,
            _ => {
                let right = match clip.or_else(|| self.selection.iter().copied().find(|&c| self.project.clip(c).is_some())) {
                    Some(c) => c,
                    None => self.cut_near_playhead().ok_or("no cut to put a transition on: select a clip")?,
                };
                let kind = TransitionKind::CrossFade;
                self.project
                    .add_transition(right, kind, dur)
                    .or_else(|| self.project.add_edge_transition(right, kind, dur, false))
                    .ok_or("no such clip")?
            }
        };
        let tr = self.project.transition_mut(tid).ok_or("no such transition")?;
        tr.set_shader(src);
        let dur = tr.duration;
        // Ctrl+T repeats it, like every other way a transition is applied
        self.transitions_ui.pick_shader("", src);
        self.transitions_ui.remember(TransitionKind::Shader, dur);
        Ok(tid)
    }

    /// The right-hand clip of the cut nearest the playhead, on any video track.
    fn cut_near_playhead(&self) -> Option<Id> {
        let t = self.playhead;
        let mut best: Option<(f64, Id)> = None;
        for track in self.project.tracks.iter().filter(|tr| tr.kind == TrackKind::Video) {
            for c in track.clips.iter().filter(|c| track.left_of(c).is_some()) {
                let d = (c.start - t).abs();
                if best.is_none_or(|(b, _)| d < b) {
                    best = Some((d, c.id));
                }
            }
        }
        best.map(|(_, id)| id)
    }
}

/// Compile `src` as an effect (`transition` false) or a transition on the GL context: "" = it links.
fn compile_log(app: &mut App, src: &str, transition: bool) -> String {
    match app.gpu.as_mut() {
        Some(g) if transition => g.check_transition(src).err().unwrap_or_default(),
        Some(g) => g.check_shader(src).err().unwrap_or_default(),
        None => NO_GPU.into(),
    }
}

/// `[{name, value, min, max}]` for a shader's named controls.
fn params_json(specs: &[crate::model::ParamSpec], value: impl Fn(usize) -> f64) -> Value {
    specs
        .iter()
        .enumerate()
        .map(|(i, s)| json!({"name": s.name, "value": value(i), "min": s.min, "max": s.max}))
        .collect()
}

fn set_shader(app: &mut App, args: &Value) -> Result<Value, String> {
    let id = req(arg_u64(args, "clip"), "clip")?;
    let src = req(arg_str(args, "src"), "src")?.to_string();
    let c = app.project.clip_mut(id).ok_or("no such clip")?;
    if !c.is_visual() {
        return Err("a shader effect needs a visual clip".into());
    }
    let index = match arg_u64(args, "index") {
        Some(i) => {
            let i = i as usize;
            match c.effects.get(i) {
                Some(e) if e.kind == EffectKind::Shader => i,
                Some(_) => return Err(format!("effect {i} is not a shader effect")),
                None => return Err(format!("the clip has no effect {i}")),
            }
        }
        None => match c.effects.iter().position(|e| e.kind == EffectKind::Shader) {
            Some(i) => i,
            None => {
                c.effects.push(Effect::new(EffectKind::Shader));
                c.effects.len() - 1
            }
        },
    };
    let fx = &mut c.effects[index];
    fx.set_shader(&src);
    let lt = 0.0;
    let params = params_json(&fx.specs(), |i| fx.at(i, lt));
    let name = fx.name();
    let log = compile_log(app, &src, false);
    Ok(json!({"ok": log.is_empty(), "log": log, "index": index, "name": name, "params": params}))
}

fn check(app: &mut App, args: &Value) -> Result<Value, String> {
    let src = req(arg_str(args, "src"), "src")?;
    let transition = match arg_str(args, "kind").unwrap_or("effect") {
        "effect" => false,
        "transition" => true,
        k => return Err(format!("kind must be effect or transition, not '{k}'")),
    };
    let meta = crate::engine::shaderlib::parse_meta(src);
    let specs = crate::model::shader_specs(src);
    let log = compile_log(app, src, transition);
    Ok(json!({
        "ok": log.is_empty(),
        "log": log,
        "name": meta.name,
        "params": params_json(&specs, |i| specs[i].default),
        "needs_previous": meta.needs_previous,
        // what keeps it out of the shared library (header, forbidden lines), not a compile error
        "library_lint": crate::engine::shaderlib::lint(src, transition),
    }))
}

fn transition_set_shader(app: &mut App, args: &Value) -> Result<Value, String> {
    let src = req(arg_str(args, "src"), "src")?.to_string();
    let tid = match arg_u64(args, "id") {
        Some(t) => {
            app.project.transition_mut(t).ok_or("no such transition")?.set_shader(&src);
            t
        }
        None => app.apply_transition_shader(&src, arg_u64(args, "clip"))?,
    };
    let tr = app.project.transition_mut(tid).ok_or("no such transition")?;
    if let Some(vals) = args.get("params").and_then(|v| v.as_array()) {
        for (i, v) in vals.iter().enumerate().take(tr.params.len()) {
            tr.params[i] = v.as_f64().ok_or("params must be numbers")?;
        }
    }
    let specs = crate::model::shader_specs(&src);
    let vals = tr.params.clone();
    let name = tr.name();
    let log = compile_log(app, &src, true);
    Ok(json!({
        "ok": log.is_empty(),
        // a failing source stays on the transition and renders as a cross fade until it is fixed
        "log": log,
        "transition_id": tid,
        "name": name,
        "params": params_json(&specs, |i| vals.get(i).copied().unwrap_or(specs[i].default)),
    }))
}

pub const TOOLS: &[ToolDef] = &[
    ToolDef {
        name: "clip.set_shader",
        desc: "Set the GLSL source of a clip's shader effect (library/README.md format: a `vec4 effect(vec4 src, vec2 uv)` body with `// @name` / `// @param id default min max` / `// @needs previous` header lines, or a shadertoy mainImage). Targets the shader effect at `index`, else the clip's first one, else adds one. Compiles it and returns {ok, log, params}: a compile error is returned in `log` (not as a tool failure) - read it, fix the source and call again.",
        args: &[
            "clip:integer:true:clip id",
            "index:integer:false:index in the clip's effect stack (default: its first shader effect)",
            "src:string:true:GLSL source",
        ],
        kind: ToolKind::Mutate,
        run: |a, v| set_shader(a, v).map(ToolOutcome::Done),
    },
    ToolDef {
        name: "shader.check",
        desc: "Compile a shader source on the GPU without changing anything: {ok, log, name, params, needs_previous, library_lint}. kind = effect (default) or transition (gl-transitions format: `vec4 transition(vec2 uv)` with getFromColor / getToColor / progress).",
        args: &["src:string:true:GLSL source", "kind:string:false:effect | transition"],
        kind: ToolKind::Read,
        run: |a, v| check(a, v).map(ToolOutcome::Done),
    },
    ToolDef {
        name: "transition.set_shader",
        desc: "Make a transition a shader transition (gl-transitions format, library/README.md): the transition `id`, else a new one on the cut at the start of `clip`, else on the selected transition / selected clip / cut nearest the playhead. Optional `params` sets its `@param` values in header order. Returns {ok, log, transition_id, params}; a source that does not compile renders as a cross fade and its log is returned (not a tool failure).",
        args: &[
            "id:integer:false:transition id",
            "clip:integer:false:clip on the right of the cut",
            "src:string:true:GLSL source",
            "params:array:false:control values in @param order",
        ],
        kind: ToolKind::Mutate,
        run: |a, v| transition_set_shader(a, v).map(ToolOutcome::Done),
    },
];
