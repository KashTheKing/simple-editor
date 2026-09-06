use super::tools_helpers::*;
use super::*;

pub(super) fn dispatch(app: &mut App, name: &str, args: &Value) -> Option<Result<Value, String>> {
    let prefix = name.split('.').next().unwrap_or("");
    if !matches!(prefix, "subtitles" | "labels" | "markers" | "notes" | "plan") {
        return None;
    }
    pub(super) fn run(app: &mut App, name: &str, args: &Value) -> Result<Value, String> {
        match name {
            "subtitles.get" => {
                let cues: Vec<Value> = app
                    .project
                    .subtitles
                    .iter()
                    .map(|c| json!({"id": c.id, "start": c.start, "end": c.end, "text": c.text}))
                    .collect();
                Ok(json!(cues))
            }
            "subtitles.set" => {
                let cues = req(args.get("cues").and_then(|v| v.as_array()), "cues")?.clone();
                app.project.subtitles.clear();
                for c in cues {
                    let start = req(arg_f64(&c, "start"), "cues[].start")?;
                    let end = req(arg_f64(&c, "end"), "cues[].end")?;
                    let text = req(arg_str(&c, "text"), "cues[].text")?;
                    app.project.add_cue(start, end, text);
                }
                app.project.sort_cues();
                Ok(json!({"ok": true, "count": app.project.subtitles.len()}))
            }
            "subtitles.import" => {
                let path = req(arg_str(args, "path"), "path")?;
                let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
                let cues = crate::engine::subtitles::parse(&text);
                if cues.is_empty() {
                    return Err("no cues found".into());
                }
                app.project.subtitles.clear();
                for (start, end, text) in cues {
                    app.project.add_cue(start, end, text);
                }
                app.project.sort_cues();
                app.project.show_subtitles = true;
                Ok(json!({"ok": true, "count": app.project.subtitles.len()}))
            }
            "plan.get" => Ok(json!({
                "plan": serde_json::to_value(&app.project.plan).map_err(|e| e.to_string())?,
                "notes": app.project.notes,
            })),
            "plan.add" => {
                let title = req(arg_str(args, "title"), "title")?.to_string();
                let parent = arg_u64(args, "parent");
                let id = app.project.plan_add(parent, title);
                if parent.is_some() && app.project.plan_item_mut(id).is_none() {
                    return Err("no such parent".into());
                }
                let assets = arg_ids(args, "assets").unwrap_or_default();
                if let Some(item) = app.project.plan_item_mut(id) {
                    if let Some(n) = arg_str(args, "notes") {
                        item.notes = n.to_string();
                    }
                    for a in assets {
                        item.assets.push(a);
                        item.asset_notes.push(String::new());
                    }
                }
                Ok(json!({"ok": true, "id": id}))
            }
            "plan.set" => {
                let id = req(arg_u64(args, "id"), "id")?;
                let item = app.project.plan_item_mut(id).ok_or("no such planner item")?;
                if let Some(t) = arg_str(args, "title") {
                    item.title = t.to_string();
                }
                if let Some(d) = arg_bool(args, "done") {
                    item.done = d;
                }
                if let Some(n) = arg_str(args, "notes") {
                    item.notes = n.to_string();
                }
                Ok(json!({"ok": true}))
            }
            "plan.remove" => {
                let id = req(arg_u64(args, "id"), "id")?;
                app.project.plan_remove(id);
                Ok(json!({"ok": true}))
            }
            "notes.get" => Ok(json!({"notes": app.project.notes})),
            // `notes` is now a titled list (see the planner's Notes tab); this tool predates that and
            // keeps working against the first note (creating an untitled one if there isn't one yet).
            "notes.set" => {
                let text = req(arg_str(args, "text"), "text")?;
                if app.project.notes.is_empty() {
                    app.project.add_note("");
                }
                let n = &mut app.project.notes[0];
                if arg_bool(args, "append").unwrap_or(false) {
                    if !n.body.is_empty() {
                        n.body.push_str("\n\n");
                    }
                    n.body.push_str(text);
                } else {
                    n.body = text.to_string();
                }
                Ok(json!({"ok": true}))
            }
            "markers.list" => {
                let list: Vec<Value> = app
                    .project
                    .markers_in_timeline()
                    .into_iter()
                    .map(|(id, t, dur, name, label)| json!({"id": id, "t": t, "duration": dur, "name": name,
                                                            "label": label, "label_name": app.project.label_name(label)}))
                    .collect();
                Ok(json!(list))
            }
            "markers.add" => {
                let t = req(arg_f64(args, "t"), "t")?;
                let name = arg_str(args, "name").unwrap_or("Marker").to_string();
                let id = match arg_u64(args, "clip_id") {
                    Some(clip) => {
                        let start = app.project.clip(clip).ok_or("no such clip")?.start;
                        app.project.add_clip_marker(clip, t - start, name).ok_or("no such clip")?
                    }
                    None => app.project.add_marker(t, name),
                };
                if let Some(m) = app.project.marker_mut(id) {
                    if let Some(n) = arg_str(args, "note") {
                        m.note = n.to_string();
                    }
                    if let Some(l) = arg_u64(args, "label") {
                        m.label = l.min(255) as u8;
                    }
                    if let Some(d) = arg_f64(args, "duration") {
                        m.duration = d.max(0.0);
                    }
                }
                Ok(json!({"ok": true, "id": id}))
            }
            "markers.remove" => {
                let id = req(arg_u64(args, "id"), "id")?;
                app.project.remove_marker(id);
                Ok(json!({"ok": true}))
            }
            "labels.list" => {
                let list: Vec<Value> = app
                    .project
                    .labels
                    .iter()
                    .enumerate()
                    .map(|(i, l)| json!({"index": i + 1, "name": l.name, "color": l.color}))
                    .collect();
                Ok(json!(list))
            }
            "labels.set" => {
                let color = args.get("color").and_then(color_arg).map(|c| [c[0], c[1], c[2]]);
                match arg_u64(args, "index") {
                    Some(i) if arg_bool(args, "remove").unwrap_or(false) => {
                        app.project.remove_label(i as u8);
                        Ok(json!({"ok": true, "labels": app.project.labels.len()}))
                    }
                    Some(i) => {
                        let l = app
                            .project
                            .labels
                            .get_mut((i as usize).checked_sub(1).ok_or("index is 1-based")?)
                            .ok_or("no such label")?;
                        if let Some(n) = arg_str(args, "name") {
                            l.name = n.to_string();
                        }
                        if let Some(c) = color {
                            l.color = c;
                        }
                        Ok(json!({"ok": true, "index": i}))
                    }
                    None => {
                        let name = req(arg_str(args, "name"), "name")?.to_string();
                        let idx = app.project.add_label(name, color.unwrap_or([128, 128, 128]));
                        Ok(json!({"ok": true, "index": idx}))
                    }
                }
            }
            _ => unreachable!(),
        }
    }
    Some(run(app, name, args))
}
