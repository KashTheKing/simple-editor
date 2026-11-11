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
    Ok(ToolOutcome::Done(json!({
        "clip": clip, "asset": aid, "name": h.name, "html": h.src, "fields": declared,
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
    let (src, fields, scale) = match (arg_u64(args, "clip"), arg_str(args, "html")) {
        (Some(clip), None) => {
            let s = html_ctl::spec_of_clip(app, clip).ok_or_else(|| format!("clip {clip} is not an HTML layer"))?;
            let mut fields = s.fields;
            fields.extend(fields_arg(args)?);
            (s.html, fields, s.scale)
        }
        (None, Some(src)) => (src.to_string(), fields_arg(args)?, 1.0),
        _ => return Err("give clip (an HTML layer) or html (source to try), not both".into()),
    };
    let tpl = html::parse(&src);
    // the picture an agent looks at: the project's own scale, at most 960 px on the longer side
    let scale = scale.min(960.0 / tpl.size.0.max(tpl.size.1) as f64);
    let t = arg_f64(args, "t").unwrap_or_else(|| tpl.thumb_at()).max(0.0);
    let (tx, rx) = std::sync::mpsc::channel();
    let (s2, f2) = (src.clone(), fields.clone());
    std::thread::spawn(move || {
        let _ = tx.send(html::render_png(&s2, &f2, scale, t));
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
            "name:string:false:new display name",
        ],
        kind: ToolKind::Mutate,
        run: set,
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
               \"name | text|color|number|image | default [| min | max]\" used as {{name}} in markup or CSS. No \
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

    #[test]
    fn fields_are_an_object() {
        assert_eq!(
            fields_arg(&json!({"fields": {"a": "x", "n": 3}})).unwrap(),
            vec![("a".into(), "x".into()), ("n".into(), "3".into())]
        );
        assert!(fields_arg(&json!({"fields": [1]})).is_err());
    }
}
