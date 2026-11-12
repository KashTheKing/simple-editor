//! ---- ws:html-layer ----
//! The `html.*` MCP tools (create / get / set / render / check).
//!
//! `html.check` replies with an image, so it renders on the spot: Edge is asked for one frame on a
//! worker thread and this call waits for it (a warm Edge: page load + one frame, ~0.1-0.3 s; a cold
//! one ~1.3 s more) - the same shape as `render.frame`. A `ToolKind::Job` reply carries JSON only, so
//! it cannot return the picture.

use super::tools_helpers::*;
use super::*;
use crate::engine::html;
use crate::mcp::tools::{ToolDef, ToolKind, ToolOutcome};

/// `fields` as (name, value) pairs; a non-string value is its JSON text.
fn fields_arg(args: &Value) -> Result<Vec<(String, String)>, String> {
    match args.get("fields") {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Object(m)) => Ok(m
            .iter()
            .map(|(k, v)| {
                (
                    k.clone(),
                    v.as_str().map(str::to_string).unwrap_or_else(|| {
                        if v.is_null() {
                            String::new()
                        } else {
                            v.to_string()
                        }
                    }),
                )
            })
            .collect()),
        Some(_) => Err("fields: an object {name: value}".into()),
    }
}

/// An ease by name: linear, ease_in, ease_out, ease_in_out, hold, or a named curve (smooth, snap,
/// slow_start, slow_end, overshoot, anticipate).
fn ease_named(s: &str) -> Result<crate::model::Ease, String> {
    use crate::model::Ease;
    let k = s.trim().to_ascii_lowercase().replace([' ', '-'], "_");
    Ok(match k.as_str() {
        "" | "linear" => Ease::Linear,
        "ease_in" => Ease::EaseIn,
        "ease_out" => Ease::EaseOut,
        "ease_in_out" => Ease::EaseInOut,
        "hold" | "step" => Ease::Hold,
        other => Ease::PRESETS
            .iter()
            .find(|(n, _)| n.to_ascii_lowercase().replace(' ', "_") == other)
            .map(|(_, e)| *e)
            .ok_or_else(|| format!("unknown ease '{s}' (linear, ease_in, ease_out, ease_in_out, hold, smooth, snap, overshoot, ...)"))?,
    })
}

fn ease_text(e: crate::model::Ease) -> String {
    use crate::model::Ease;
    match e {
        Ease::Linear => "linear".into(),
        Ease::EaseIn => "ease_in".into(),
        Ease::EaseOut => "ease_out".into(),
        Ease::EaseInOut => "ease_in_out".into(),
        Ease::Hold => "hold".into(),
        Ease::Bezier { .. } => Ease::PRESETS
            .iter()
            .find(|(_, p)| *p == e)
            .map_or("bezier".to_string(), |(n, _)| n.to_ascii_lowercase().replace(' ', "_")),
    }
}

/// `params` of html.set: {name: number | [{t, v, ease?}, ...] | null}. A number is a constant (any keys go),
/// a list is keyframes (t = seconds of the layer's own time), null = back to the default.
fn apply_params(h: &mut crate::model::HtmlSource, tpl: &html::Template, args: &Value) -> Result<(), String> {
    use crate::model::Animated;
    let Some(m) = args.get("params").filter(|v| !v.is_null()) else { return Ok(()) };
    let Value::Object(m) = m else { return Err("params: an object {name: number | [{t, v, ease?}]}".into()) };
    for (name, v) in m {
        let decl = tpl.params.iter().find(|p| p.name == *name).ok_or_else(|| {
            let names: Vec<&str> = tpl.params.iter().map(|p| p.name.as_str()).collect();
            format!("unknown param '{name}' (this page declares: {})", if names.is_empty() { "none - add <meta name=\"se:param\" content=\"name | number | default | min | max\">".into() } else { names.join(", ") })
        })?;
        let clamp = |x: f64| match (decl.min, decl.max) {
            (Some(a), Some(b)) if a <= b => x.clamp(a, b),
            _ => x,
        };
        match v {
            Value::Null => {
                h.params.remove(name);
            }
            Value::Number(n) => {
                h.params.insert(name.clone(), Animated::new(clamp(n.as_f64().unwrap_or(0.0))));
            }
            Value::Array(keys) => {
                let mut ks = Vec::new();
                for k in keys {
                    let t = k["t"].as_f64().ok_or_else(|| format!("param '{name}': every key needs t (seconds)"))?;
                    let val = k["v"].as_f64().ok_or_else(|| format!("param '{name}': every key needs v (a number)"))?;
                    let ease = match k["ease"].as_str() {
                        Some(e) => ease_named(e)?,
                        None => crate::model::Ease::Linear,
                    };
                    ks.push(crate::model::Keyframe { t: t.max(0.0), v: clamp(val), ease });
                }
                if ks.is_empty() {
                    return Err(format!("param '{name}': an empty list of keys (use a number for a constant)"));
                }
                ks.sort_by(|a, b| a.t.total_cmp(&b.t));
                let mut a = Animated::new(ks[0].v);
                a.keys = ks;
                h.params.insert(name.clone(), a);
            }
            _ => return Err(format!("param '{name}': a number or a list of {{t, v, ease?}} keys")),
        }
    }
    Ok(())
}

fn clip_arg(args: &Value) -> Result<Id, String> {
    req(arg_u64(args, "clip"), "clip")
}

fn get(app: &mut App, args: &Value) -> Result<ToolOutcome, String> {
    let clip = clip_arg(args)?;
    let (aid, h) = html_ctl::html_of(app, clip).ok_or_else(|| format!("clip {clip} is not an HTML layer"))?;
    let tpl = html::parse(&h.src);
    let declared: Vec<Value> = tpl
        .fields
        .iter()
        .map(|f| {
            json!({"name": f.name, "type": format!("{:?}", f.ty).to_lowercase(), "default": f.default,
                   "min": f.min, "max": f.max, "value": h.fields.get(&f.name).unwrap_or(&f.default)})
        })
        .collect();
    let params: Vec<Value> = tpl
        .params
        .iter()
        .map(|p| {
            let track = h.params.get(&p.name);
            let default = p.default.trim().parse::<f64>().unwrap_or(0.0);
            json!({"name": p.name, "default": default, "min": p.min, "max": p.max,
                   "value": track.map_or(default, |a| a.at(0.0)),
                   "keys": track.map(|a| a.keys.iter().map(|k| json!({"t": k.t, "v": k.v, "ease": ease_text(k.ease)})).collect::<Vec<_>>()).unwrap_or_default()})
        })
        .collect();
    Ok(ToolOutcome::Done(json!({
        "clip": clip, "asset": aid, "name": h.name, "html": h.src, "fields": declared, "file": h.file, "params": params, "width": h.width,
        "duration": h.duration, "size": [tpl.size.0, tpl.size.1], "scene": tpl.scene,
        "rendered": html_ctl::rendered(app, clip), "lint": html::lint(&h.src),
    })))
}

fn set(app: &mut App, args: &Value) -> Result<ToolOutcome, String> {
    let clip = clip_arg(args)?;
    let (aid, mut h) = html_ctl::html_of(app, clip).ok_or_else(|| format!("clip {clip} is not an HTML layer"))?;
    if let Some(src) = arg_str(args, "html") {
        if src.trim().is_empty() {
            return Err("html: empty".into());
        }
        h.src = src.to_string();
    }
    let tpl = html::parse(&h.src);
    let given = fields_arg(args)?;
    html_ctl::known_fields(&tpl, &given)?; // an unknown name is an error
    for (k, v) in given {
        if args["fields"][k.as_str()].is_null() {
            h.fields.remove(&k); // null = back to the default
        } else {
            h.fields.insert(k, v);
        }
    }
    // values for fields the (new) source no longer declares are dropped
    h.fields.retain(|k, _| tpl.fields.iter().any(|f| f.name == *k));
    apply_params(&mut h, &tpl, args)?;
    if let Some(w) = arg_u64(args, "width") {
        h.width = if w == 0 { 0 } else { w.clamp(320, 3840) as u32 };
    }
    h.params.retain(|k, _| tpl.params.iter().any(|p| p.name == *k));
    if let Some(n) = arg_str(args, "name").map(str::trim).filter(|n| !n.is_empty()) {
        h.name = n.to_string();
    }
    let lint = html::lint(&h.src);
    if let Some(a) = app.project.asset_mut(aid) {
        a.html = Some(h);
    }
    Ok(ToolOutcome::Done(json!({"ok": true, "clip": clip, "lint": lint, "rendering": true})))
}

fn check(app: &mut App, args: &Value) -> Result<ToolOutcome, String> {
    let (src, fields, scale, base, params, width, mouse) = match (arg_u64(args, "clip"), arg_str(args, "html")) {
        (Some(clip), None) => {
            let s = html_ctl::spec_of_clip(app, clip).ok_or_else(|| format!("clip {clip} is not an HTML layer"))?;
            let mut fields = s.fields;
            fields.extend(fields_arg(args)?);
            (s.html, fields, s.scale, s.base, s.params, s.width, s.mouse)
        }
        (None, Some(src)) => (src.to_string(), fields_arg(args)?, 1.0, String::new(), Vec::new(), 0, None),
        _ => return Err("give clip (an HTML layer) or html (source to try), not both".into()),
    };
    let tpl = html::parse(&src);
    // the picture an agent looks at: the project's own scale, at most 960 px on the longer side
    let scale = scale.min(960.0 / tpl.size.0.max(tpl.size.1) as f64);
    let t = arg_f64(args, "t").unwrap_or_else(|| tpl.thumb_at()).max(0.0);
    let (tx, rx) = std::sync::mpsc::channel();
    let (s2, f2) = (src.clone(), fields.clone());
    std::thread::spawn(move || {
        let _ = tx.send(html::render_png_in(&s2, &f2, scale, t, &html::View { base: &base, params: &params, width, mouse: mouse.as_ref() }));
    });
    let png =
        rx.recv_timeout(Duration::from_secs(30)).map_err(|_| "html.check: the render timed out".to_string())??;
    let (w, h) = html::png_size(&png).unwrap_or((0, 0));
    let meta = json!({"t": t, "width": w, "height": h, "duration": tpl.duration, "lint": html::lint(&src),
                      "note": "transparent where the layer is transparent"});
    Ok(ToolOutcome::Image { png_b64: thumbs::base64(&png), meta })
}

pub const TOOLS: &[ToolDef] = &[
    ToolDef {
        name: "html.create",
        desc:
            "Add an HTML + CSS layer or scene clip (titles, lower thirds, buttons, cards, charts, UI mock-ups) on top \
               of the timeline at its natural size, centred, and render it in the background (Jobs pane). Returns \
               {clip, lint}. Look at it with html.check. Format: see html.check.",
        args: &[
            "html:string:true:the whole HTML file (format in html.check's description)",
            "at:number:false:timeline second (default: the playhead)",
            "duration:number:false:seconds (default: the template's se:duration)",
            "name:string:false:clip name (default: the <title>)",
            "fields:object:false:{name: value} overriding the se:field defaults",
        ],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let src = req(arg_str(args, "html"), "html")?;
            let fields = fields_arg(args)?;
            let id =
                app.add_html_clip(src, arg_str(args, "name"), arg_f64(args, "at"), arg_f64(args, "duration"), &fields)?;
            Ok(ToolOutcome::Done(json!({"ok": true, "clip": id, "lint": html::lint(src), "rendering": true})))
        },
    },
    ToolDef {
        name: "html.get",
        desc: "An HTML layer clip's source, its declared fields with their current values, duration, size, whether \
               its render is done, and what breaks the library rules (lint).",
        args: &["clip:integer:true:an HTML layer clip id"],
        kind: ToolKind::Read,
        run: get,
    },
    ToolDef {
        name: "html.set",
        desc: "Change an HTML layer clip: replace its source (html) and/or set field values (fields: {name: value}, \
               null = back to the default). The clip keeps its position, effects and keyframes; it re-renders in \
               the background and the old picture stays until the new one is ready.",
        args: &[
            "clip:integer:true:an HTML layer clip id",
            "html:string:false:the whole new HTML file",
            "fields:object:false:{name: value}",
            "params:object:false:{name: number | [{t, v, ease?}] | null} - values of the page's se:param (CSS variable --name): a number is constant, a list is keyframes (t in seconds of the layer's own time, ease linear|ease_in|ease_out|ease_in_out|hold|smooth|snap|overshoot), null = default",
            "width:integer:false:a web page (no se:size): the width in CSS px it is laid out at (320-3840; 0 = default)",
            "name:string:false:new display name",
        ],
        kind: ToolKind::Mutate,
        run: set,
    },
    ToolDef {
        name: "html.import",
        desc: "Add a .html file from disk as an HTML layer clip. By default the clip stays LINKED to the file: edit \
               the file (you can write it directly) and the clip re-renders; edit the clip and the file follows. \
               Format and rules: see html.check.",
        args: &[
            "path:string:true:a .html file",
            "at:number:false:timeline second (default: the playhead)",
            "link:boolean:false:keep the clip and the file in sync (default true)",
        ],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let path = req(arg_str(args, "path"), "path")?;
            let id = html_ctl::import_file(app, std::path::Path::new(path), arg_f64(args, "at"), arg_bool(args, "link").unwrap_or(true))?;
            Ok(ToolOutcome::Done(json!({"ok": true, "clip": id, "rendering": true})))
        },
    },
    ToolDef {
        name: "html.snapshot",
        desc: "Snapshot a real web page by address (a private Edge opens it as a signed-out visitor sees it, keeps the                HTML and CSS, drops scripts) and add it as a linked HTML layer clip when done (a few seconds). The                page can then be edited, scrolled (html.set), animated with params and the fake mouse. To snapshot                the user's own browser (signed in), they use Add > Web Page Snapshot > From my browser.",
        args: &[
            "url:string:true:the page address (https:// optional)",
            "width:integer:false:layout width in CSS px (default: the frame's width)",
        ],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let url = req(arg_str(args, "url"), "url")?.trim().to_string();
            if url.is_empty() {
                return Err("url is empty".into());
            }
            let width = arg_f64(args, "width").map_or(app.project.width, |w| w as u32).clamp(320, 3840);
            if !html_ctl::start_snapshot(app, url, width) {
                return Err("a snapshot is already being taken".into());
            }
            Ok(ToolOutcome::Done(json!({"ok": true, "started": true, "note": "the clip appears on the timeline when the page is saved"})))
        },
    },
    ToolDef {
        name: "html.link",
        desc: "Link an HTML layer clip to a .html file so they stay in sync (the file is read every second; a change \
               in the clip is written to the file). write=false (default): the file's text becomes the clip's \
               source. write=true: the clip's source is written to path (new file or overwrite). path empty: \
               unlink.",
        args: &[
            "clip:integer:true:an HTML layer clip id",
            "path:string:true:the .html file (empty string = unlink)",
            "write:boolean:false:write the clip's source to the file instead of reading it (default false)",
        ],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let clip = clip_arg(args)?;
            let path = arg_str(args, "path").unwrap_or("");
            let p = (!path.trim().is_empty()).then(|| std::path::Path::new(path));
            html_ctl::link_file(app, clip, p, arg_bool(args, "write").unwrap_or(false))?;
            Ok(ToolOutcome::Done(json!({"ok": true, "clip": clip, "file": path})))
        },
    },
    ToolDef {
        name: "html.render",
        desc: "Render an HTML layer clip again if its current source has no render yet (also retries after a failed \
               or cancelled render). Returns {rendered}: true = already up to date.",
        args: &["clip:integer:true:an HTML layer clip id"],
        kind: ToolKind::Read,
        run: |app, args| {
            let clip = clip_arg(args)?;
            html_ctl::rerender(app, clip)?;
            Ok(ToolOutcome::Done(json!({"ok": true, "rendered": html_ctl::rendered(app, clip)})))
        },
    },
    ToolDef {
        name: "html.check",
        desc: "Render one frame of an HTML layer and return it as an image (transparent areas stay transparent) plus \
               lint. clip = an HTML layer clip (its source and field values); or html = any source, to look before \
               creating. t = second (default: se:thumb or 60 % of the duration); fields = values to try. FORMAT: \
               One HTML file, CSS in <style>, NO JavaScript; motion = CSS @keyframes / animation (time 0 = the \
               clip's first frame; use animation-fill-mode: both). Head: <title>, <meta name=\"se:kind\" \
               content=\"layer|scene\"> (layer = transparent over video, scene = full frame), se:desc, se:tags, \
               se:size \"WxH\" in CSS px against a 1080-high frame (1920x1080 when the position in the frame \
               matters, a tight size for free-floating buttons and badges), se:duration \"4s\", se:field \
               \"name | text|color|number|image | default [| min | max]\" used as {{name}} in markup or CSS; \
               se:param \"name | number | default | min | max\" = an ANIMATABLE number, the CSS variable var(--name) \
               (keyframe it with html.set params or in the Inspector; use it in calc(), transform, width, \
               opacity...). No \
               network URLs, web fonts or @import: installed fonts with fallbacks (\"Segoe UI\", Arial, \
               sans-serif), inline <svg> or data: URIs.",
        args: &[
            "clip:integer:false:an HTML layer clip id",
            "html:string:false:HTML source to render instead of a clip",
            "t:number:false:second to show",
            "fields:object:false:{name: value} to try",
        ],
        kind: ToolKind::Read,
        run: check,
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = r#"<html><head><title>t</title><meta name="se:param" content="amount | number | 5 | 0 | 10"></head><body style="opacity:var(--amount)"></body></html>"#;

    #[test]
    fn html_set_params_take_numbers_and_keys_and_refuse_what_the_page_does_not_declare() {
        let mut h = crate::model::HtmlSource { src: PAGE.into(), ..Default::default() };
        let tpl = html::parse(PAGE);
        apply_params(&mut h, &tpl, &json!({"params": {"amount": 99}})).unwrap();
        assert_eq!(h.params["amount"].value, 10.0, "clamped to the declared range");
        assert!(!h.params["amount"].is_animated());
        let keys = json!({"params": {"amount": [{"t": 2, "v": 8, "ease": "ease_out"}, {"t": 0, "v": 1}]}});
        apply_params(&mut h, &tpl, &keys).unwrap();
        let a = &h.params["amount"];
        assert_eq!(a.keys.len(), 2);
        assert_eq!((a.keys[0].t, a.keys[1].t), (0.0, 2.0), "sorted by time");
        assert_eq!(a.keys[1].ease, crate::model::Ease::EaseOut);
        assert!((a.at(1.0) - 4.5).abs() < 2.0 && a.at(0.0) == 1.0);
        apply_params(&mut h, &tpl, &json!({"params": {"amount": null}})).unwrap();
        assert!(h.params.is_empty(), "null = back to the default");
        let e = apply_params(&mut h, &tpl, &json!({"params": {"nope": 1}})).unwrap_err();
        assert!(e.contains("nope") && e.contains("amount"), "{e}");
        assert!(apply_params(&mut h, &tpl, &json!({"params": {"amount": []}})).is_err());
        assert!(apply_params(&mut h, &tpl, &json!({"params": {"amount": [{"t": 1}]}})).is_err());
        assert!(ease_named("sparkle").is_err() && ease_named("Overshoot").is_ok());
    }

    #[test]
    fn fields_are_an_object() {
        assert_eq!(
            fields_arg(&json!({"fields": {"a": "x", "n": 3}})).unwrap(),
            vec![("a".into(), "x".into()), ("n".into(), "3".into())]
        );
        assert!(fields_arg(&json!({"fields": [1]})).is_err());
    }
}
