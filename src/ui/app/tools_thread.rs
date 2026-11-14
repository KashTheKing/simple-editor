//! ---- ws:thread-viewers ----
//! The `thread.*` MCP tools and the Add ▸ Chat Thread / Forum Thread actions. A thread layer is an HTML
//! layer whose page carries its data (`engine::thread`); every call here reads that data back out of the
//! clip's source, changes it and writes the regenerated page (one undo step, then the usual re-render).

use super::tools_helpers::*;
use super::*;
use crate::engine::thread::{self, Item, Kind, Member, Thread};
use crate::mcp::tools::{ToolDef, ToolKind, ToolOutcome};

fn clip_arg(args: &Value) -> Result<Id, String> {
    req(arg_u64(args, "clip"), "clip")
}

/// (asset, its HTML, its thread) of clip `clip`.
fn load(app: &App, clip: Id) -> Result<(Id, crate::model::HtmlSource, Thread), String> {
    let (aid, h) = html_ctl::html_of(app, clip).ok_or_else(|| format!("clip {clip} is not an HTML layer"))?;
    let t = thread::from_src(&h.src).ok_or_else(|| format!("clip {clip} is not a thread layer (make one with thread.create)"))?;
    Ok((aid, h, t))
}

/// Write `t` into the clip's asset and say what happened.
fn save(app: &mut App, clip: Id, aid: Id, mut h: crate::model::HtmlSource, t: &Thread, extra: Value) -> ToolOutcome {
    thread::store(&mut h, t);
    let lint = crate::engine::html::lint(&h.src);
    let dur = h.duration;
    if let Some(a) = app.project.asset_mut(aid) {
        a.html = Some(h);
    }
    let mut out = json!({"ok": true, "clip": clip, "items": t.items.len(), "duration": dur, "lint": lint, "rendering": true});
    if let (Value::Object(o), Value::Object(e)) = (&mut out, extra) {
        o.extend(e);
    }
    ToolOutcome::Done(out)
}

fn edit(app: &mut App, args: &Value, f: impl FnOnce(&mut Thread) -> Result<Value, String>) -> Result<ToolOutcome, String> {
    let clip = clip_arg(args)?;
    let (aid, h, mut t) = load(app, clip)?;
    let extra = f(&mut t)?;
    Ok(save(app, clip, aid, h, &t, extra))
}

/// The starter named by `kind` (chat, chat_typing, forum, comments, comments_typing).
fn recipe_arg(args: &Value) -> Result<Thread, String> {
    let kind = arg_str(args, "kind").unwrap_or("chat");
    let mode = arg_str(args, "mode").unwrap_or("");
    Thread::recipe(kind, mode, arg_str(args, "style").unwrap_or("")).ok_or_else(|| format!("kind '{kind}': chat, chat_typing, forum, comments or comments_typing"))
}

/// The item fields an agent may give flat, beside `item`.
const ITEM_KEYS: &[&str] = &[
    "kind", "who", "text", "date", "time", "age", "reply", "images", "embed", "reactions", "edited", "pinned", "score", "awards", "op",
    "depth", "collapsed", "more", "delay", "typing", "pulse",
];

fn item_args(args: &Value) -> Value {
    let mut v = args.get("item").cloned().filter(Value::is_object).unwrap_or_else(|| json!({}));
    for k in ITEM_KEYS {
        if let Some(x) = args.get(*k) {
            v[*k] = x.clone();
        }
    }
    v
}

fn id_arg(args: &Value) -> Result<u32, String> {
    req(arg_u64(args, "id"), "id").map(|v| v as u32)
}

fn view(t: &Thread) -> Value {
    let slots = t.schedule();
    let times: Vec<Value> = t.items.iter().zip(&slots).map(|(i, s)| json!({"id": i.id, "appear": s.appear, "typing": s.typing, "shown": s.shown})).collect();
    json!({"thread": t, "times": times, "duration": t.duration(), "script": t.to_script(), "presets": thread::PRESETS.iter().filter(|p| p.1 == t.kind).map(|p| p.0).collect::<Vec<_>>()})
}

fn get(app: &mut App, args: &Value) -> Result<ToolOutcome, String> {
    let clip = clip_arg(args)?;
    let (_, h, t) = load(app, clip)?;
    let mut v = view(&t);
    v["clip"] = clip.into();
    v["lint"] = json!(crate::engine::html::lint(&h.src));
    v["targets"] = match t.comment_button() {
        Some((r, press)) => json!({"comment_button": {"x": r[0], "y": r[1], "w": r[2], "h": r[3], "pressed_at": press}}),
        None => json!({}),
    };
    Ok(ToolOutcome::Done(v))
}

pub const TOOLS: &[ToolDef] = &[
    ToolDef {
        name: "thread.create",
        desc: "Add a fake chat, forum or video-comment thread as an HTML layer, made from DATA (not hand-written HTML): it has \
               starter content so it looks right at once, then change it with thread.add / thread.set / \
               thread.from_script / thread.cast. kind: chat (messages only: just the people messaging; switch the \
               server rail, channel list, member list, header, top bar, message box, typing line, date dividers, \
               NEW line, scrollbar on with thread.set), chat_typing (a DM, group DM or server channel where `me`'s \
               messages are typed into the message box and sent, the others show 'is typing...'), forum, comments \
               (a video comment section), comments_typing (a comment typed into the box, the Comment button \
               pressed, the comment posted). Items arrive one after another, all CSS, frame-exact. Looks (style): \
               Dark chat, Light chat, Compact chat, Classic forum, Card forum, Dark forum, Light comments, Dark \
               comments, Compact comments - generic UI, no real logos. Returns {clip, duration, items}.",
        args: &[
            "kind:string:false:chat | chat_typing | forum | comments | comments_typing (default chat)",
            "mode:string:false:chat layout: server | dm | group (default server; dm for chat_typing is common)",
            "style:string:false:a look: Dark chat | Light chat | Compact chat | Classic forum | Card forum | Dark forum | Light comments | Dark comments | Compact comments",
            "script:string:false:lines that become the messages / comments instead of the starter content (see thread.from_script)",
            "patch:object:false:thread fields to set (see thread.get): title, topic, accent, anim, post ...",
            "at:number:false:timeline second (default: the playhead)",
            "duration:number:false:seconds (default: the thread's own animation length)",
            "name:string:false:clip name",
        ],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let mut t = recipe_arg(args)?;
            let kind = t.kind;
            if arg_str(args, "script").is_some() {
                t.items.clear();
            }
            if let Some(p) = args.get("patch").filter(|p| p.is_object()) {
                t.patch(p)?;
            }
            if let Some(s) = arg_str(args, "script") {
                t.from_script(s, true);
            }
            let dur = arg_f64(args, "duration").unwrap_or_else(|| t.duration().max(3.0));
            let id = app.add_html_clip(&t.html(), arg_str(args, "name"), arg_f64(args, "at"), Some(dur), &[])?;
            Ok(ToolOutcome::Done(json!({"ok": true, "clip": id, "kind": kind.label(), "duration": dur, "items": t.items.iter().map(|i| i.id).collect::<Vec<_>>(), "rendering": true})))
        },
    },
    ToolDef {
        name: "thread.get",
        desc: "A thread layer's data: the whole thread (look, cast, items with ids, post, animation), when every item \
               appears (times), its length, and the thread as a plain-lines script. Kinds: chat or forum.",
        args: &["clip:integer:true:a thread layer clip id"],
        kind: ToolKind::Read,
        run: get,
    },
    ToolDef {
        name: "thread.set",
        desc: "Change a thread layer (chat toggles by name: rail, channels, sidebar, topbar, header, composer, indicator, dividers, unread_line, scrollbar, transparent, mode server|dm|group, me; typing: anim.compose, anim.cps, anim.typos, anim.pause, items typed/cps/pause/there). With id: patch that item ({who, text, date, time, age, reply, images, embed:{title,desc,color,thumb}, reactions:[{emoji,count,me}], edited, pinned, \
               score, awards, op, depth, collapsed, more, delay, typing, pulse, kind: msg|system|divider|unread}; null clears a field). Without id: patch the thread ({style, theme, layout, accent, font, radius, \
               density: compact|normal|roomy, size, width, height, title, topic, server, icon, time_format: today|time|24h|full|relative, now, group, rail, sidebar, header, composer, post:{title,who,flair,flair_color,score,comments,awards,age,body,image,link,thumb,sort}, \
               anim:{on,start,gap,typing,effect:pop|slide|fade|cut,autoscroll,tick,shown,hold}}). Or give thread: a whole thread (as thread.get returns it) to replace everything. Text uses light markup: **bold** *italic* `code` ```block``` > quote @mention #channel :emoji: ||spoiler||.",
        args: &[
            "clip:integer:true:a thread layer clip id",
            "id:integer:false:an item id (thread.get lists them): patch that item instead of the thread",
            "patch:object:false:the fields to change",
            "thread:object:false:a whole thread to replace this one (an export)",
            "mouse:boolean:false:video comments: true = aim the fake mouse at the Comment button (it glides there and clicks when the typing ends), false = remove the mouse",
        ],
        kind: ToolKind::Mutate,
        run: |app, args| {
            if let Some(m) = arg_bool(args, "mouse") {
                // the fake mouse is the layer's, not the thread's: set it with the rest of this call
                let clip = clip_arg(args)?;
                let (aid, h, t) = load(app, clip)?;
                let mouse = if m { Some(t.mouse().ok_or("no typed comment with a Comment button in this thread")?) } else { None };
                if let Some(a) = app.project.asset_mut(aid) {
                    a.html = Some(crate::model::HtmlSource { mouse, ..h });
                }
                if args.get("patch").is_none() && args.get("thread").is_none() {
                    return Ok(ToolOutcome::Done(json!({"ok": true, "clip": clip, "mouse": m})));
                }
            }
            edit(app, args, |t| {
                if let Some(whole) = args.get("thread").filter(|v| v.is_object()) {
                    let mut n: Thread = serde_json::from_value(whole.clone()).map_err(|e| format!("thread: {e}"))?;
                    n.normalize();
                    *t = n;
                    return Ok(json!({}));
                }
                let patch = req(args.get("patch").filter(|v| v.is_object()), "patch")?;
                match arg_u64(args, "id") {
                    Some(id) => t.patch_item(id as u32, patch)?,
                    None => t.patch(patch)?,
                }
                Ok(json!({}))
            })
        },
    },
    ToolDef {
        name: "thread.add",
        desc: "Add a message (chat) or comment (forum) to a thread layer: who, text and any item field (see thread.set), \
               flat or inside item. A new speaker joins the cast; a chat message without a time follows the one before \
               it by a minute. Forum: depth 0 is a top-level comment, 1 a reply to the comment above, and so on. \
               at = index to insert at, or after = an item id (default: the end); duplicate = an item id to copy. \
               Returns {id}.",
        args: &[
            "clip:integer:true:a thread layer clip id",
            "who:string:false:the speaker (a cast name)",
            "text:string:false:the message (light markup)",
            "item:object:false:more fields: {time, date, age, score, depth, reply, images, embed, reactions, ...}",
            "at:integer:false:index to insert at",
            "after:integer:false:insert after this item id",
            "duplicate:integer:false:copy this item id instead",
        ],
        kind: ToolKind::Mutate,
        run: |app, args| {
            edit(app, args, |t| {
                let v = item_args(args);
                let at = match (arg_u64(args, "at"), arg_u64(args, "after")) {
                    (Some(i), _) => Some(i as usize),
                    (_, Some(id)) => Some(t.index_of(id as u32).ok_or_else(|| format!("no item {id}"))? + 1),
                    _ => None,
                };
                let id = match arg_u64(args, "duplicate") {
                    Some(d) => {
                        let id = t.duplicate(d as u32).ok_or_else(|| format!("no item {d}"))?;
                        if v.as_object().is_some_and(|o| !o.is_empty()) {
                            t.patch_item(id, &v)?;
                        }
                        id
                    }
                    None => {
                        let it: Item = serde_json::from_value(v).map_err(|e| format!("item: {e}"))?;
                        t.add(it, at)
                    }
                };
                Ok(json!({"id": id}))
            })
        },
    },
    ToolDef {
        name: "thread.remove",
        desc: "Remove an item from a thread layer (replies to it lose their quote).",
        args: &["clip:integer:true:a thread layer clip id", "id:integer:true:the item id"],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let id = id_arg(args)?;
            edit(app, args, |t| if t.remove(id) { Ok(json!({})) } else { Err(format!("no item {id}")) })
        },
    },
    ToolDef {
        name: "thread.move",
        desc: "Move an item of a thread layer: to = the index it ends at, or by = steps up (negative) / down. Forum: \
               depth = its nesting (0 top level), or indent = +1 / -1.",
        args: &[
            "clip:integer:true:a thread layer clip id",
            "id:integer:true:the item id",
            "to:integer:false:the index to move it to",
            "by:integer:false:steps to move (negative = up)",
            "depth:integer:false:forum: set the nesting level",
            "indent:integer:false:forum: nest one level deeper (+1) or shallower (-1)",
        ],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let id = id_arg(args)?;
            edit(app, args, |t| {
                let ok = if let Some(to) = arg_u64(args, "to") {
                    t.move_to(id, to as usize)
                } else if let Some(by) = args.get("by").and_then(Value::as_i64) {
                    t.move_by(id, by as i32)
                } else if let Some(d) = arg_u64(args, "depth") {
                    t.patch_item(id, &json!({"depth": d})).is_ok()
                } else if let Some(i) = args.get("indent").and_then(Value::as_i64) {
                    t.indent(id, i as i32)
                } else {
                    return Err("give to, by, depth or indent".into());
                };
                if ok { Ok(json!({"order": t.items.iter().map(|i| i.id).collect::<Vec<_>>()})) } else { Err(format!("no item {id}")) }
            })
        },
    },
    ToolDef {
        name: "thread.from_script",
        desc: "Turn plain lines into items of a thread layer. Chat: `name: message`, `name (4:32 PM): message`, \
               `name (11/13/2026 4:32 PM): message`, `--- label` (a divider), `* dave joined the server` (a system \
               line), `!unread`. Forum: `# post title`, `## post body`, `name (3h, 4.2k): comment` (age, score), `>` or \
               indentation for replies (`> bob: reply`, `>> cy: deeper`). A line that is not `name: text` continues \
               the message before it; `\\n` is a line break. New speakers join the cast. mode append (default) or replace.",
        args: &[
            "clip:integer:true:a thread layer clip id",
            "script:string:true:the lines",
            "mode:string:false:append (default) or replace",
        ],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let script = req(arg_str(args, "script"), "script")?;
            let replace = match arg_str(args, "mode").unwrap_or("append") {
                "append" => false,
                "replace" => true,
                m => return Err(format!("mode '{m}': append or replace")),
            };
            edit(app, args, |t| Ok(json!({"added": t.from_script(script, replace)})))
        },
    },
    ToolDef {
        name: "thread.cast",
        desc: "The cast of a thread layer: the people messages are from, each with an avatar (a project image's file \
               name, or a path), a name colour and a badge (BOT, verified, any short text). Change a person here and \
               every one of their messages follows. action: list (default), set (add or update by name), remove, \
               rename (name -> to; every message follows).",
        args: &[
            "clip:integer:true:a thread layer clip id",
            "action:string:false:list | set | remove | rename",
            "name:string:false:the person",
            "avatar:string:false:a project image's file name, or a path ('' = a coloured circle with the initial)",
            "color:string:false:#rrggbb: the name's colour",
            "badge:string:false:BOT, verified or any short text ('' = none)",
            "to:string:false:rename: the new name",
        ],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let action = arg_str(args, "action").unwrap_or("list");
            let name = arg_str(args, "name").unwrap_or("").trim().to_string();
            if action == "list" {
                let (_, _, t) = load(app, clip_arg(args)?)?;
                return Ok(ToolOutcome::Done(json!({"cast": t.cast})));
            }
            if name.is_empty() {
                return Err("name is required".into());
            }
            edit(app, args, |t| {
                match action {
                    "set" => {
                        let mut m = t.member(&name).cloned().unwrap_or(Member { name: name.clone(), ..Default::default() });
                        for (k, slot) in [("avatar", &mut m.avatar), ("color", &mut m.color), ("badge", &mut m.badge)] {
                            if let Some(v) = arg_str(args, k) {
                                *slot = v.trim().to_string();
                            }
                        }
                        t.upsert(m);
                    }
                    "remove" => {
                        if !t.remove_member(&name) {
                            return Err(format!("no cast member '{name}'"));
                        }
                    }
                    "rename" => {
                        let to = req(arg_str(args, "to"), "to")?;
                        if !t.rename_member(&name, to) {
                            return Err(format!("cannot rename '{name}' to '{to}' (unknown, or the name is taken)"));
                        }
                    }
                    a => return Err(format!("action '{a}': list, set, remove or rename")),
                }
                Ok(json!({"cast": t.cast}))
            })
        },
    },
];

/// Add ▸ Chat Thread / Forum Thread.
pub(super) fn act(app: &mut App, a: Action) -> bool {
    let kind = match a {
        Action::AddChatThread => Kind::Chat,
        Action::AddForumThread => Kind::Forum,
        Action::AddVideoComments => Kind::Comments,
        _ => return false,
    };
    let t = Thread::starter(kind, "");
    let before = app.project.to_json();
    match app.add_html_clip(&t.html(), None, None, Some(t.duration().max(3.0)), &[]) {
        Ok(id) => {
            app.push_undo_labeled(before, kind.label());
            app.after_edit();
            app.selection = vec![id];
        }
        Err(e) => app.toast(e),
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn item_fields_come_flat_or_inside_item() {
        let v = item_args(&json!({"clip": 1, "who": "A", "text": "hi", "item": {"time": "4:32 PM", "who": "B"}, "extra": 1}));
        assert_eq!((v["who"].as_str(), v["text"].as_str(), v["time"].as_str()), (Some("A"), Some("hi"), Some("4:32 PM")), "flat wins");
        assert!(v.get("extra").is_none() && v.get("clip").is_none());
        assert!(recipe_arg(&json!({"kind": "wiki"})).is_err() && recipe_arg(&json!({})).unwrap().kind == Kind::Chat);
        assert!(recipe_arg(&json!({"kind": "comments_typing"})).unwrap().anim.compose);
        assert_eq!(recipe_arg(&json!({"kind": "chat_typing", "mode": "dm"})).unwrap().mode, "dm");
    }
}
