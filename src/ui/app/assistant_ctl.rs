//! ---- ws:assistant-pane ----
//! `Pane::Assistant`'s App side: sending a request (MCP server switched on, the attached context and
//! images, one undo group per request), the request queue (task scripts, "Handle my comments"),
//! draining the agent's events from a FRAME_HOOKS tick (wakes through `App::animate_until` only while
//! something is in flight), Stop, Undo, the conversation store (loaded per project, saved off the UI
//! thread on change), running the task scripts, "Add to Assistant", the `>` palette prefix, the guard
//! that holds an agent's MCP calls which leave the project until the user allows them, and the
//! `assistant.*` / `tasks.*` tools. The process side is `engine::agent`; the widget is
//! `ui::assistant_ui` (data: `ui::assistant_data`).

use super::tools_helpers::*;
use super::*;
use crate::engine::agent::{self, Event};
use crate::engine::skills::{self, Skill, Source};
use crate::mcp::tools::{ToolDef, ToolKind, ToolOutcome};
use crate::ui::assistant_ui::{self as aui, Attach, Line, Out, Pin, TaskStatus, View};
use std::collections::VecDeque;
use std::sync::mpsc::TryRecvError;

/// While a request is in flight the tick drains the agent's events at this cadence.
const POLL: Duration = Duration::from_millis(150);

/// What a task request asks of the agent, on top of the usual preamble: write a script, don't edit.
const TASK_BRIEF: &str = "Write a Luau script for the task below instead of editing the project yourself: do not call \
any tool that changes the project. Read what you need first (editor.context, project.summary, the list / get tools, \
transcript.get, render.frame), then hand the script over with tasks.add{title, code, note} - title: a few words; note: \
one line on what it does; code: the script. The user reviews it and runs it later, as one undo step.\n\
Script API (Luau, sandboxed - no io, os or require): editor.tool(name, args) calls one of this editor's tools - the \
same names and arguments as your MCP tools - and returns its JSON result as a table (a failing call raises an error \
and stops the script); editor.tools() lists {name, description}; editor.log(text) shows the user a message. A run has \
a 5 second budget. Tools that start background jobs (export.*, media.convert, assistant.wait) can't be called from a \
script. Times are seconds, ids are integers; several cuts or deletes go back to front. Take every tool name and \
argument from your tool list - never guess one.";

/// MCP tools an agent may only call after the user clicks Allow (they leave the project).
/// ws:skills: `skills.save` / `skills.delete` too - a saved skill is instructions a later request
/// follows. ponytail: they are not in `mcp::tools::leaves_project`, so the MCP client waits its
/// usual 60 s for them, not `HELD_WAIT`; `held_wait` matches that.
pub(super) fn leaves_project(tool: &str, args: &Value) -> bool {
    mcp::tools::leaves_project(tool, args) || matches!(tool, "skills.save" | "skills.delete")
}

/// How long the MCP client waits for a held call before it gives up.
fn held_wait(tool: &str, args: &Value) -> Duration {
    if mcp::tools::leaves_project(tool, args) {
        mcp::HELD_WAIT
    } else {
        Duration::from_secs(60)
    }
}

/// What "Always allow this session" remembers: the tool, or `ui.action:<id>` for one Action.
fn allow_key(tool: &str, args: &Value) -> String {
    match args.get("id").and_then(Value::as_str) {
        Some(id) if tool == "ui.action" => format!("ui.action:{id}"),
        _ => tool.to_string(),
    }
}

/// Whether `guard` would hold a call to `tool` (also what `edit.batch` refuses to run inside a batch).
pub(super) fn needs_approval(app: &App, tool: &str, args: &Value) -> bool {
    let rt = &app.assistant_rt;
    // ws:skills: the user ran /create-skill, so saving its result is what they asked for.
    // ponytail: the server doesn't know which client calls; another MCP client saving during that run
    // passes too
    let creating = tool == "skills.save" && rt.run.is_some() && rt.skill.as_deref() == Some(skills::CREATE);
    !rt.bypass
        && app.settings.assistant_guard
        && leaves_project(tool, args)
        && !creating
        && !rt.always.iter().any(|t| *t == allow_key(tool, args))
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    Chat,
    /// Writes the script for this task placeholder.
    Task(u64),
    /// Answers this comment marker (marked done when the request succeeds).
    Comment(Id),
}

/// One request to the agent.
struct Req {
    kind: Kind,
    /// The conversation it was asked in (its replies go there, its session is resumed).
    conv: u64,
    /// The undo label (and what the queue shows).
    label: String,
    /// The whole prompt the agent gets.
    full: String,
    images: Vec<aui::Attachment>,
    /// ws:skills: the skill it runs (`/name`), if any.
    skill: Option<String>,
}

#[derive(Default)]
pub(super) struct AssistantRt {
    run: Option<agent::Run>,
    /// A request waiting for the MCP server to come up.
    queued: Option<Req>,
    /// Requests waiting for the running one (tasks, comments), oldest first.
    backlog: VecDeque<Req>,
    /// The request in flight: its kind, undo label and the undo-stack length when it began.
    active: Option<(Kind, String, usize)>,
    /// The agent reported success (its `Done { ok }`).
    ok: bool,
    /// The request's group was closed under it (a hand edit, Undo) and re-opened: it is more than one
    /// undo step, so Undo only reaches its last part.
    split: bool,
    /// The current request's images (`assistant.attachments`).
    attachments: Vec<aui::Attachment>,
    /// MCP calls held until the user answers the approval window, with when each was held (the
    /// client gives up after `mcp::HELD_WAIT`; an Allow after that must not run it).
    held: Vec<(mcp::ToolCall, Instant)>,
    /// Tools the user allowed for the rest of the session.
    always: Vec<String>,
    /// Set while a released call runs, so the guard lets it through.
    bypass: bool,
    /// A task script is running (tasks.* refuse re-entry from inside it).
    running_task: bool,
    /// The project's chats are loaded into `App::assistant` (false after a project change, until idle).
    loaded: bool,
    /// The new project's chats, waiting for the running request (if any) to finish.
    pending: Vec<aui::Conversation>,
    /// "Handle my comments": markers up to this id are not new; the candidate settling.
    comment_since: Option<Id>,
    comment_slot: Option<(Id, String, Instant)>,
    /// Screenshots asked for the composer, oldest first (see `Shot`).
    shots: Vec<Shot>,
    next_shot: u64,
    /// Ctrl+V was down last frame (image paste: egui reports no key when the clipboard holds no text).
    paste_down: bool,
    /// The root context (from the tick), so a capture asked by a tool call can wake the next frame.
    ctx: Option<egui::Context>,
    // ---- ws:skills ----
    /// The skill the running request runs.
    skill: Option<String>,
    /// A library skill waiting for its first-run Run / Cancel, and whether it came from the Tasks box.
    read_first: Option<(Skill, bool)>,
}

/// A screenshot on its way into the composer: asked one frame after it is queued, so the menu or the
/// area overlay that asked for it is gone from the picture.
struct Shot {
    tag: u64,
    /// The crop, in points.
    rect: egui::Rect,
    name: String,
    /// Context that goes with it (the timeline range's clips and tracks).
    chip: Option<aui::Chip>,
    asked: bool,
}

/// Tags the screenshots the Assistant asks for (`ui.screenshot` and `--screenshot` take only theirs).
pub(super) struct AssistantShot(u64);

fn busy(app: &App) -> bool {
    app.assistant_rt.run.is_some() || app.assistant_rt.queued.is_some()
}

// ---------------------------------------------------------------------------------------------------
// context

/// One chip per piece of `editor.context` worth showing: (key, label). Keys: selection, range,
/// playhead, comment.
pub(super) fn chips_of(c: &Value) -> Vec<(&'static str, String)> {
    let n = |k: &str| c[k].as_array().map_or(0, Vec::len);
    let mut out = Vec::new();
    let words = c["selected_words"]["text"].as_str().filter(|t| !t.is_empty());
    let sel = match (words, n("selected_clips"), n("selected_cues")) {
        (Some(w), _, _) => Some(format!("Words: \u{201c}{}\u{201d}", aui::short(w, 30))),
        (_, 1, _) => Some(format!("Clip: {}", c["selected_clips"][0]["name"].as_str().unwrap_or("1 clip"))),
        (_, k, _) if k > 1 => Some(format!("{k} clips")),
        (_, _, k) if k > 0 => Some(format!("{k} subtitle{}", if k == 1 { "" } else { "s" })),
        _ => None,
    };
    if let Some(s) = sel {
        out.push(("selection", s));
    }
    if let (Some(a), Some(b)) = (c["in"].as_f64(), c["out"].as_f64()) {
        out.push(("range", format!("In/Out {a:.1}s–{b:.1}s")));
    }
    if let Some(t) = c["playhead"].as_f64() {
        out.push(("playhead", format!("Playhead {t:.1}s")));
    }
    let note = c["selected_markers"].as_array().into_iter().flatten().find_map(|m| m["note"].as_str().filter(|s| !s.trim().is_empty()));
    if let Some(note) = note {
        out.push(("comment", format!("Comment: \u{201c}{}\u{201d}", aui::short(note, 30))));
    }
    out
}

/// `editor.context` minus what the user clicked off.
pub(super) fn filter_context(mut c: Value, off: &[&str]) -> Value {
    if let Some(o) = c.as_object_mut() {
        for key in off {
            let fields: &[&str] = match *key {
                "selection" => &["selected_clips", "selected_transitions", "selected_cues", "selected_words", "edit_point"],
                "range" => &["in", "out"],
                "playhead" => &["playhead"],
                "comment" => &["selected_markers"],
                _ => &[],
            };
            for f in fields {
                o.remove(*f);
            }
        }
    }
    c
}

/// What a request carries: the live context (only while "Assistant follows the selection" is on, minus
/// the chips removed) plus the pinned chips' snapshots under `pinned`. `{}` = nothing attached.
pub(super) fn compose_context(live: Option<Value>, off: &[&str], pinned: &[aui::Chip]) -> Value {
    let mut c = live.map_or_else(|| json!({}), |c| filter_context(c, off));
    if !pinned.is_empty() {
        c["pinned"] = Value::Array(pinned.iter().map(|p| p.data.clone()).collect());
    }
    c
}

/// The preamble every request starts with.
pub(super) fn preamble(ctx: &Value, atts: &[aui::Attachment]) -> String {
    let mut s = String::from(
        "You are editing the video project open in Simple Editor, live, through its MCP tools. This whole request \
         is already one undo step: do not call edit.begin or edit.end.",
    );
    // ws:assistant-luau
    s.push_str(concat!(
        " For the library or any bulk work, read media.summary first and media.list with filters and fields",
        " (never the whole list), then edit with media.bulk_set / library.bins, or run a Luau script with",
        " scripts.eval{code, purpose} - it reaches the project and library only, needs no Allow, and is one",
        " undo step. You have no shell or file access, and need none.",
    ));
    if ctx.as_object().is_some_and(|o| !o.is_empty()) {
        s.push_str(&format!(
            " The context the user attached (editor.context JSON; `pinned` = what they added on purpose):\n{ctx}"
        ));
    } else {
        s.push_str(" No context is attached - call editor.context when you need it.");
    }
    if !atts.is_empty() {
        let list: Vec<String> = atts
            .iter()
            .enumerate()
            .map(|(i, a)| match &a.file {
                Some(f) if !a.is_image() => format!("{i}: file {}", f.display()),
                _ => format!("{i}: image \"{}\" {}x{}", a.name, a.width, a.height),
            })
            .collect();
        s.push_str(&format!(
            "\nThe user attached {} item{}: assistant.attachment{{index}} returns each one (an image, or a text \
             file's text):\n{}",
            atts.len(),
            if atts.len() == 1 { "" } else { "s" },
            list.join("\n")
        ));
    }
    s
}

fn live_context(app: &App) -> Option<Value> {
    app.settings.assistant_follow.then(|| tools_context::context_json(app))
}

// ---------------------------------------------------------------------------------------------------
// sending

/// **The one entry point for other parts of the app** (the Ctrl+P palette's `/ask` and `/<skill>`,
/// `assistant.send`): send `text` to the Assistant as if the user typed it in the composer and pressed
/// Enter. `text` is a request ("tighten the intro") or a skill run ("/captions bold": the skill's name
/// after `/`, then its arguments); the composer's attachments and pinned context go with it. When the
/// pane isn't on screen it opens on a new chat. Err = nothing was sent (empty, or a request is still
/// running). Keep this signature stable: the palette calls it.
pub(crate) fn ask(app: &mut App, text: &str) -> Result<(), String> {
    if text.trim().is_empty() {
        return Err("empty request".into());
    }
    if busy(app) {
        return Err("the assistant is still working on the last request".into());
    }
    if !app.layout.rects.iter().any(|(p, _)| *p == Pane::Assistant) {
        // opened for this request: it goes into a fresh chat, which the pane then keeps on screen
        app.assistant.new_chat();
        app.assistant.keep_on_open = true;
    }
    app.assistant.prompt = text.to_string();
    app.assistant.tab = aui::Tab::Chat;
    app.surface(Pane::Assistant);
    send(app);
    Ok(())
}

/// Send the chat composer: its prompt, chips and images.
fn send(app: &mut App) {
    if busy(app) {
        app.toast("The assistant is still working on the last request");
        return;
    }
    let prompt = std::mem::take(&mut app.assistant.prompt).trim().to_string();
    if prompt.is_empty() {
        return;
    }
    let Some(skill) = resolve_skill(app, &prompt, false) else {
        app.assistant.prompt = prompt; // waits for the library skill's Run / Cancel
        return;
    };
    let off = std::mem::take(&mut app.assistant.chips_off);
    let pinned = std::mem::take(&mut app.assistant.pinned);
    let images = std::mem::take(&mut app.assistant.images);
    let live = live_context(app);
    let mut labels: Vec<String> = pinned.iter().map(|c| c.label.clone()).collect();
    if let Some(c) = &live {
        // (a live chip that only repeats a pinned one is not shown in the composer either)
        let live_labels: Vec<String> =
            chips_of(c).into_iter().filter(|(k, l)| !off.contains(k) && !labels.contains(l)).map(|(_, l)| l).collect();
        labels.extend(live_labels);
    }
    // the chat shows what the user typed, plus the skill's name; the agent gets the skill's text
    let request = match &skill {
        Some((s, args)) => {
            labels.insert(0, format!("Skill: /{}", s.name));
            format!("(the user ran the skill /{} - follow its instructions)\n{}", s.name, skills::expand(s, args))
        }
        None => prompt.clone(),
    };
    let ctx = compose_context(live, &off, &pinned);
    let st = &mut app.assistant;
    if st.conv_mut().title.is_empty() {
        st.conv_mut().title = aui::short(&crate::ui::markdown::plain(&prompt), 60);
    }
    st.push(Line::User(prompt.clone()));
    if !labels.is_empty() || !images.is_empty() {
        st.push(Line::Attached { chips: labels, images: images.clone() });
    }
    st.dirty = true;
    if app.settings.assistant_backend == "external" {
        // assistant.wait attaches the live context itself; pinned chips ride along in the text
        let text = match ctx.get("pinned") {
            Some(p) => format!("{request}\n\nPinned context: {p}"),
            None => request,
        };
        app.assistant.pending.push_back(text);
        app.assistant_rt.attachments = images;
        app.assistant.push(Line::Info("Waiting for a connected agent (assistant.wait)…".into()));
        switch_mcp_on(app);
        return;
    }
    let full = format!("{}\n\nRequest: {request}", preamble(&ctx, &images));
    let skill = skill.map(|(s, _)| s.name);
    let conv = app.assistant.conv_mut().id;
    enqueue(app, Req { kind: Kind::Chat, conv, label: prompt, full, images, skill });
}

/// ws:skills: `/name args` -> Some(Some((skill, args))); plain text or an unknown `/name` ->
/// Some(None); None = a library skill's first run, waiting for its read-first window (`task`: it came
/// from the Tasks box). A run is remembered for the chip row.
fn resolve_skill(app: &mut App, text: &str, task: bool) -> Option<Option<(Skill, String)>> {
    let Some((s, args)) = skills::parse_invocation(text).and_then(|(n, a)| Some((skills::get(n)?, a.to_string()))) else {
        return Some(None);
    };
    if s.source == Source::Library && app.settings.skills_accepted.get(&s.name) != Some(&skills::content_hash(&s)) {
        app.assistant_rt.read_first = Some((s, task));
        return None;
    }
    let recent = &mut app.settings.skills_recent;
    if recent.first() != Some(&s.name) {
        recent.retain(|n| *n != s.name);
        recent.insert(0, s.name.clone());
        recent.truncate(8);
        app.settings.save();
    }
    Some(Some((s, args)))
}

/// The Tasks tab's Add: a "Writing…" card, and a request asking the agent for its script.
fn add_task(app: &mut App) {
    let desc = std::mem::take(&mut app.assistant.task_prompt).trim().to_string();
    if desc.is_empty() {
        return;
    }
    let Some(skill) = resolve_skill(app, &desc, true) else {
        app.assistant.task_prompt = desc;
        return;
    };
    let what = skill.as_ref().map_or_else(|| desc.clone(), |(s, args)| skills::expand(s, args));
    let skill = skill.map(|(s, _)| s.name);
    let st = &mut app.assistant;
    let id = aui::task_writing(st.tasks_mut(), &desc);
    if st.conv_mut().title.is_empty() {
        st.conv_mut().title = aui::short(&desc, 60);
    }
    st.push(Line::User(format!("Task: {desc}")));
    st.dirty = true;
    let ctx = compose_context(live_context(app), &[], &[]);
    let full = format!("{}\n\n{TASK_BRIEF}\n\nTask: {what}", preamble(&ctx, &[]));
    let conv = app.assistant.conv_mut().id;
    enqueue(app, Req { kind: Kind::Task(id), conv, label: format!("Task: {desc}"), full, images: Vec::new(), skill });
}

/// "Handle my comments": a settled new comment becomes a background request.
fn comment_req(comment: Value, id: Id, conv: u64) -> Req {
    let note = comment["note"].as_str().unwrap_or("").to_string();
    let ctx = json!({"comment": comment});
    let full = format!(
        "{}\n\nThe user left this comment on the timeline (marker {id}). Do what it asks; the editor marks the \
         comment done when you finish, so don't mark it yourself.\nComment: {note}",
        preamble(&ctx, &[])
    );
    Req { kind: Kind::Comment(id), conv, label: format!("Comment: {}", aui::short(&note, 50)), full, images: Vec::new(), skill: None }
}

fn switch_mcp_on(app: &mut App) {
    if !app.settings.mcp_enabled {
        app.settings.mcp_enabled = true; // sync_mcp starts it this frame
        app.settings.save();
    }
}

/// Run `req` now, or after the running request.
fn enqueue(app: &mut App, req: Req) {
    switch_mcp_on(app);
    if busy(app) {
        app.assistant_rt.backlog.push_back(req);
        return;
    }
    app.assistant_rt.queued = Some(req);
    start_queued(app);
}

/// A request that never reached the agent: log it, give back its task placeholder.
fn fail_req(app: &mut App, req: Req, why: &str) {
    if let Kind::Task(id) = req.kind {
        mark_unwritten(app, id, why);
    }
    app.assistant.push(Line::Error(why.to_string()));
    app.assistant.push(Line::Done { undo: None });
    app.assistant.dirty = true;
}

fn mark_unwritten(app: &mut App, id: u64, why: &str) {
    if let Some(t) = app.assistant.target_mut().tasks.iter_mut().find(|t| t.id == id && t.status == TaskStatus::Writing) {
        (t.status, t.error) = (TaskStatus::Failed, why.to_string());
    }
}

fn start_queued(app: &mut App) {
    let Some(url) = app.mcp.as_ref().map(|(s, _)| s.url()) else {
        if !app.settings.mcp_enabled {
            // sync_mcp tried every port and switched it back off
            if let Some(req) = app.assistant_rt.queued.take() {
                fail_req(app, req, "The MCP server couldn't start (see Settings ▸ General ▸ MCP).");
            }
        }
        return;
    };
    let Some(req) = app.assistant_rt.queued.take() else { return };
    let s = &app.settings;
    let (backend, key_env) = match s.assistant_backend.as_str() {
        "codex" => (agent::Backend::Codex, "OPENAI_API_KEY".to_string()),
        "custom" => (agent::Backend::Custom, s.assistant_key_env.clone()),
        _ => (agent::Backend::Claude, "ANTHROPIC_API_KEY".to_string()),
    };
    app.assistant.run_conv = Some(req.conv);
    let resume = if backend == agent::Backend::Claude { app.assistant.target_mut().session.clone() } else { None };
    let spec = agent::Spec {
        backend,
        prompt: req.full,
        mcp_url: url,
        model: s.assistant_model.clone(),
        effort: s.assistant_effort.clone(),
        resume,
        command: s.assistant_command.clone(),
        key: agent::key_load().map(|k| (key_env, k)),
    };
    let label: String = req.label.replace(['\n', '\r'], " ").chars().take(60).collect();
    app.begin_group(&label, "Assistant");
    let group_label = app.mcp_ctx.group.as_ref().map(|g| g.label.clone()).unwrap_or_default();
    let rt = &mut app.assistant_rt;
    rt.active = Some((req.kind, group_label, app.undo.len()));
    (rt.split, rt.ok) = (false, false);
    rt.attachments = req.images;
    rt.skill = req.skill;
    rt.run = Some(agent::start(spec));
}

/// The request ended (finished, failed or stopped): tick off its comment, close its undo group, log
/// "Done" with its Undo when its entry is on the stack, then the next queued request may start.
fn finish(app: &mut App) {
    app.assistant_rt.run = None; // drops the Run: kills the child if it's still alive
    app.assistant_rt.skill = None;
    let active = app.assistant_rt.active.take();
    let ok = app.assistant_rt.ok;
    match active.as_ref().map(|a| a.0) {
        // inside the group, so undoing the request reopens the comment too
        Some(Kind::Comment(id)) if ok => {
            if let Some(m) = app.project.marker_mut(id) {
                m.done = true;
                app.after_edit();
            }
        }
        Some(Kind::Task(id)) => mark_unwritten(app, id, "The assistant finished without handing over a script"),
        _ => {}
    }
    app.end_group();
    let undo = active.and_then(|(_, label, start)| {
        let top = app.undo.last().is_some_and(|e| e.label == label);
        (top && app.undo.len() > start).then(|| (app.undo.len(), label))
    });
    if std::mem::take(&mut app.assistant_rt.split) && undo.is_some() {
        app.assistant.push(Line::Info(
            "You edited while this ran, so it is split around your edit: Undo undoes only its last part (the \
             earlier part is further down in History)."
                .into(),
        ));
    }
    app.assistant.push(Line::Done { undo });
    app.assistant.run_conv = None;
    app.assistant.dirty = true;
}

fn stop(app: &mut App) {
    if let Some(run) = &app.assistant_rt.run {
        run.stop();
    }
    if let Some(req) = app.assistant_rt.queued.take() {
        fail_req(app, req, "Stopped.");
    }
    if app.assistant_rt.run.is_some() {
        // the stopped agent's held calls must not run on a later Allow
        deny_held(app, "the user stopped the request");
        app.assistant.push(Line::Info("Stopped.".into()));
        finish(app);
    }
}

// ---------------------------------------------------------------------------------------------------
// tasks, attachments, pins

/// Run one task (`only`) or every Ready one in order (a failure stops the queue); each run is one undo
/// step labelled "Assistant task: <title>".
fn run_tasks(app: &mut App, only: Option<u64>) -> Result<usize, String> {
    if app.assistant_rt.running_task {
        return Err("a task is already running".into());
    }
    app.close_group_for_user_edit();
    app.assistant_rt.running_task = true;
    let mut tasks = std::mem::take(app.assistant.tasks_mut());
    let (ran, err) = aui::run_queue(&mut tasks, only, &mut |t| {
        app.run_script_src(&t.code, &t.title, Some(&format!("Assistant task: {}", t.title)))
    });
    *app.assistant.tasks_mut() = tasks;
    app.assistant_rt.running_task = false;
    app.assistant.dirty = true;
    match err {
        Some(e) if only.is_none() => {
            app.toast("A task failed - the queue stopped there");
            Err(e)
        }
        Some(e) => Err(e),
        None if ran == 0 => Err("nothing to run".into()),
        None => Ok(ran),
    }
}

fn save_script(app: &mut App, id: u64) {
    let Some(t) = app.assistant.conv().and_then(|c| c.tasks.iter().find(|t| t.id == id)).cloned() else { return };
    let dir = crate::scripting::scripts_dir();
    let _ = crate::scripting::list(); // creates the folder the first time
    let path = aui::script_path(&dir, &t.title);
    match std::fs::write(&path, aui::task_script(&t)) {
        Ok(()) => app.toast(format!("Saved to Scripts: {}", path.file_name().unwrap_or_default().to_string_lossy())),
        Err(e) => app.toast(format!("Couldn't save the script: {e}")),
    }
}

/// The store folder this project's attachments go in.
fn attach_dir(app: &App) -> PathBuf {
    aui::store_dir().join(aui::bucket(app.project_path.as_deref()))
}

fn attach(app: &mut App, what: Attach) {
    match what {
        Attach::Pin(pin) => pin_add(app, pin),
        Attach::Frame => {
            let t = app.playhead;
            let made = app.render_frame_now(t, aui::ATTACH_MAX).ok_or_else(|| "Couldn't render the frame".to_string()).and_then(|f| {
                let f = tools_context::tile(&[f], 1); // RGBA, opaque
                aui::write_attachment(&attach_dir(app), &format!("Frame at {t:.2}s"), &f.rgba, f.width, f.height)
            });
            match made {
                Ok(a) => {
                    app.assistant.images.push(a);
                    show_composer(app);
                }
                Err(e) => app.toast(e),
            }
        }
        Attach::File => {
            if let Some(paths) = rfd::FileDialog::new().set_title("Attach files").pick_files() {
                attach_paths(app, &paths);
            }
        }
        Attach::Panel(pane) => capture_pane(app, pane),
        Attach::Timeline => capture_timeline(app),
    }
}

/// The chat composer comes to the front with the cursor in it.
fn show_composer(app: &mut App) {
    app.assistant.tab = aui::Tab::Chat;
    app.assistant.focus_prompt = true;
    app.surface(Pane::Assistant);
}

/// Picture files: attached as images the agent can see. Anything else is attached by its path.
fn is_picture(p: &Path) -> bool {
    p.extension().is_some_and(|e| ["png", "jpg", "jpeg", "bmp", "webp", "gif", "tif", "tiff"].iter().any(|x| e.eq_ignore_ascii_case(x)))
}

/// Files into the composer (the paperclip, a drop on the pane, a paste of files copied in Explorer).
pub(super) fn attach_paths(app: &mut App, paths: &[PathBuf]) {
    let dir = attach_dir(app);
    for p in paths {
        let made = if is_picture(p) {
            let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            load_image(p).and_then(|(rgba, w, h)| aui::write_attachment(&dir, &name, &rgba, w, h)).map(|mut a| {
                a.file = Some(p.clone());
                a
            })
        } else if p.is_file() {
            Ok(aui::Attachment::file(p))
        } else {
            Err(format!("Not a file: {}", p.display()))
        };
        match made {
            Ok(a) => app.assistant.images.push(a),
            Err(e) => app.toast(e),
        }
    }
    show_composer(app);
}

/// "Screenshot a panel" / a pane tab's "Attach to Assistant".
pub(super) fn capture_pane(app: &mut App, pane: Pane) {
    match app.layout.rects.iter().find(|(p, _)| *p == pane) {
        Some(&(_, r)) => queue_shot(app, r, format!("{} panel", pane.title()), None),
        None => app.toast(format!("The {} panel isn't on screen", pane.title())),
    }
}

fn queue_shot(app: &mut App, rect: egui::Rect, name: String, chip: Option<aui::Chip>) {
    let rt = &mut app.assistant_rt;
    rt.next_shot += 1;
    rt.shots.push(Shot { tag: rt.next_shot, rect, name, chip, asked: false });
    if let Some(ctx) = &rt.ctx {
        ctx.request_repaint();
    }
}

/// "Timeline range": In/Out (else the selected clips' span) as a picture of the Timeline cut to the range,
/// plus a chip with the clips and tracks in it, so the agent knows exactly what is meant.
fn capture_timeline(app: &mut App) {
    let span = || {
        let clips: Vec<_> = app.selection.iter().filter_map(|&id| app.project.clip(id)).collect();
        let a = clips.iter().map(|c| c.start).fold(f64::INFINITY, f64::min);
        let b = clips.iter().map(|c| c.end()).fold(f64::NEG_INFINITY, f64::max);
        (b > a).then_some((a, b))
    };
    let range = match (app.project.in_point, app.project.out_point) {
        (Some(a), Some(b)) if b > a => Some((a, b)),
        _ => span(),
    };
    let Some((a, b)) = range else {
        app.toast("Set In and Out (or select clips) first");
        return;
    };
    let chip = timeline_chip(&app.project, a, b);
    let Some(&(_, pane)) = app.layout.rects.iter().find(|(p, _)| *p == Pane::Timeline) else {
        aui::pin_add(&mut app.assistant.pinned, chip);
        app.toast("The Timeline isn't on screen: attached its clips without a picture");
        show_composer(app);
        return;
    };
    let tl = &app.timeline;
    let (x0, x1) = (tl.x_at(a).max(tl.lanes_rect.left()), tl.x_at(b).min(tl.lanes_rect.right()));
    // the range's columns of the pane (ruler and every track), or the whole pane when it is scrolled away
    let rect = if x1 - x0 >= 8.0 { egui::Rect::from_x_y_ranges(x0..=x1, pane.y_range()) } else { pane };
    queue_shot(app, rect, format!("Timeline {a:.1}s-{b:.1}s"), Some(chip));
}

/// The timeline range's context: every clip that overlaps `a..b` and the tracks they are on.
pub(super) fn timeline_chip(p: &crate::model::Project, a: f64, b: f64) -> aui::Chip {
    let clips: Vec<Value> = p
        .all_clips()
        .filter(|(_, c)| c.start < b && c.end() > a)
        .map(|(t, c)| json!({"id": c.id, "name": c.name, "kind": format!("{:?}", c.kind), "track": t, "start": c.start, "end": c.end()}))
        .collect();
    let tracks: Vec<Value> = p
        .tracks
        .iter()
        .enumerate()
        .filter(|(i, _)| clips.iter().any(|c| c["track"] == *i))
        .map(|(i, t)| json!({"index": i, "name": t.name, "kind": format!("{:?}", t.kind)}))
        .collect();
    let n = clips.len();
    aui::Chip {
        label: format!("Timeline {a:.1}s–{b:.1}s · {n} clip{}", if n == 1 { "" } else { "s" }),
        data: json!({"kind": "timeline_range", "in": a, "out": b, "clips": clips, "tracks": tracks}),
    }
}

/// A screenshot cropped to `rect` (points) as an attachment.
pub(super) fn shot_attachment(img: &egui::ColorImage, rect: egui::Rect, ppp: f32, dir: &Path, name: &str) -> Result<aui::Attachment, String> {
    let f = super::tools_uikit::crop_frame(img, Some(rect * ppp));
    aui::write_attachment_max(dir, name, &f.rgba, f.width, f.height, aui::CAPTURE_MAX)
}

/// Ask for the queued screenshots and turn the ones that came back into attachments.
fn shots_tick(app: &mut App, ctx: &egui::Context) {
    for p in aui::take_captures() {
        capture_pane(app, p);
    }
    if app.assistant_rt.shots.is_empty() {
        return;
    }
    for s in app.assistant_rt.shots.iter_mut().filter(|s| !s.asked) {
        ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Screenshot(egui::UserData::new(AssistantShot(s.tag))));
        s.asked = true;
    }
    let images: Vec<(u64, std::sync::Arc<egui::ColorImage>)> = ctx.input(|i| {
        i.events
            .iter()
            .filter_map(|e| match e {
                egui::Event::Screenshot { user_data, image, .. } => {
                    user_data.data.as_ref()?.downcast_ref::<AssistantShot>().map(|t| (t.0, image.clone()))
                }
                _ => None,
            })
            .collect()
    });
    let ppp = ctx.pixels_per_point();
    for (tag, img) in images {
        let Some(k) = app.assistant_rt.shots.iter().position(|s| s.tag == tag) else { continue };
        let s = app.assistant_rt.shots.remove(k);
        match shot_attachment(&img, s.rect, ppp, &attach_dir(app), &s.name) {
            Ok(a) => {
                app.assistant.images.push(a);
                if let Some(c) = s.chip {
                    aui::pin_add(&mut app.assistant.pinned, c);
                }
                show_composer(app);
            }
            Err(e) => app.toast(format!("Couldn't capture: {e}")),
        }
    }
    ctx.request_repaint(); // the picture comes back on a later frame
}

/// Ctrl+V in the composer with a picture (or files copied in Explorer) on the clipboard: egui-winit only
/// reports a paste when the clipboard holds text, so the key is read from Windows directly.
/// ponytail: polled while the composer has focus; a paste shortcut bound elsewhere would need the same.
fn paste_tick(app: &mut App, ctx: &egui::Context) {
    let focused = ctx.memory(|m| m.has_focus(aui::prompt_id()));
    // SAFETY: GetAsyncKeyState has no preconditions; the high bit is "down now".
    let v = focused
        && ctx.input(|i| i.modifiers.command)
        && unsafe { windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState(i32::from(b'V')) } < 0;
    let pressed = v && !app.assistant_rt.paste_down;
    app.assistant_rt.paste_down = v;
    if !pressed || ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Paste(_)))) {
        return;
    }
    let Ok(mut cb) = arboard::Clipboard::new() else { return };
    if let Ok(files) = cb.get().file_list() {
        if !files.is_empty() {
            attach_paths(app, &files);
            return;
        }
    }
    if let Ok(img) = cb.get_image() {
        match aui::write_attachment_max(&attach_dir(app), "Pasted image", &img.bytes, img.width as u32, img.height as u32, aui::CAPTURE_MAX) {
            Ok(a) => {
                app.assistant.images.push(a);
                show_composer(app);
            }
            Err(e) => app.toast(format!("Couldn't paste the picture: {e}")),
        }
    }
}

/// Retry: send the user message at line `i` of the shown chat again (its text and attachments); the
/// composer's draft is kept.
fn retry(app: &mut App, i: usize) {
    let lines = app.assistant.log().to_vec();
    let Some(Line::User(text)) = lines.get(i) else { return };
    let images = aui::messages(&lines).into_iter().find(|m| m.lines.start == i).map(|m| aui::msg_attachments(&lines, &m)).unwrap_or_default();
    let draft = (std::mem::replace(&mut app.assistant.prompt, text.clone()), std::mem::replace(&mut app.assistant.images, images));
    send(app);
    (app.assistant.prompt, app.assistant.images) = draft;
}

/// A picture file decoded (through ffmpeg, like an image clip) to RGBA at most `ATTACH_MAX` on its long
/// edge - every attachment reaches the agent as a PNG, whatever it was on disk.
fn load_image(path: &std::path::Path) -> Result<(Vec<u8>, u32, u32), String> {
    let p = path.to_string_lossy();
    let a = crate::media::ffpipe::probe(&p)?;
    let (w, h) = (a.width.max(1), a.height.max(1));
    let s = (aui::ATTACH_MAX as f32 / w.max(h) as f32).min(1.0);
    let (cw, ch) = (((w as f32 * s) as u32).max(1), ((h as f32 * s) as u32).max(1));
    let mut src = crate::media::ffpipe::open_still(&p, (w, h), (cw, ch))?;
    let f = src.still(cw, ch).ok_or("couldn't decode the image")?;
    Ok((f.rgba.clone(), f.width, f.height))
}

/// "Add to Assistant": pin a snapshot of `pin` to the chat composer and show it.
fn pin_add(app: &mut App, pin: Pin) {
    match aui::pin_chip(&tools_context::context_json(app), pin) {
        Some(chip) => {
            aui::pin_add(&mut app.assistant.pinned, chip);
            app.assistant.tab = aui::Tab::Chat;
            app.assistant.focus_prompt = true;
            app.surface(Pane::Assistant);
        }
        None => app.toast(match pin {
            Pin::Range => "Set In and Out first",
            Pin::Marker => "Select a marker first",
            Pin::Words => "Select transcript words first",
            _ => "Select something to add first",
        }),
    }
}

// ---------------------------------------------------------------------------------------------------
// store

/// The chats as the JSON array a project file stores them in (None until they are loaded).
pub(super) fn chats_json(app: &App) -> Option<String> {
    app.assistant_rt.loaded.then(|| aui::snapshot(&app.assistant.convs).0)
}

/// A different project is now open (`App::set_project`): its chats come from its file - or, for a
/// project saved before chats lived in it, from the old per-user store - and a new one starts empty.
pub(super) fn project_changed(app: &mut App, path: Option<&Path>) {
    app.assistant_rt.pending = path
        .map(|p| aui::chats_in_file(p).unwrap_or_else(|| aui::load(&aui::store_dir(), &aui::bucket(Some(p)))))
        .unwrap_or_default();
    app.assistant_rt.loaded = false;
}

/// Swap the chats in once nothing is running, and count a chat change as an unsaved project change.
fn sync_store(app: &mut App) {
    if !app.assistant_rt.loaded && !busy(app) {
        let convs = std::mem::take(&mut app.assistant_rt.pending);
        app.assistant.load(convs);
        app.assistant.dirty = false;
        app.assistant_rt.loaded = true;
        return;
    }
    if std::mem::take(&mut app.assistant.dirty) {
        app.dirty = true;
    }
}

// ---------------------------------------------------------------------------------------------------
// MCP guard

/// `handle_tool`'s first step: None = the call is held for the user's approval.
pub(super) fn guard(app: &mut App, call: mcp::ToolCall) -> Option<mcp::ToolCall> {
    if !needs_approval(app, &call.name, &call.args) {
        return Some(call);
    }
    app.assistant_rt.held.push((call, Instant::now()));
    None
}

/// Answer every held call with an error (the user stopped the request, the app closed).
pub(super) fn deny_held(app: &mut App, why: &str) {
    for (call, _) in app.assistant_rt.held.drain(..) {
        let _ = call.reply.send(Err(format!("'{}': {why}", call.name)));
    }
}

/// WINDOW_DRAWERS entry: one row per held call - Allow / Always allow this session / Deny. Non-modal;
/// the agent's call simply waits (or times out on its side).
pub(super) fn approvals(app: &mut App, ctx: &egui::Context) {
    if app.assistant_rt.held.is_empty() {
        return;
    }
    let mut pick: Option<(usize, u8)> = None;
    // ponytail: the MCP server doesn't track which client sent a call; name it by what is running
    let who = if app.assistant_rt.run.is_some() {
        "The Assistant (or another connected MCP client) asks"
    } else {
        "A connected MCP client asks"
    };
    egui::Window::new("An agent wants to leave the project")
        .id(egui::Id::new("assistant_approvals"))
        .collapsible(false)
        .resizable(false)
        .order(egui::Order::Foreground)
        .anchor(egui::Align2::RIGHT_TOP, [-12.0, 60.0])
        .show(ctx, |ui| {
            ui.label(who);
            for (i, (c, _)) in app.assistant_rt.held.iter().enumerate() {
                let what = match c.args.get("id").and_then(Value::as_str).and_then(Action::from_id) {
                    Some(a) if c.name == "ui.action" => format!("ui.action: {}", a.label()),
                    _ => c.name.clone(),
                };
                ui.label(egui::RichText::new(what).strong());
                ui.weak(aui::short(&c.args.to_string(), 120));
                ui.horizontal(|ui| {
                    if ui.button("Allow").clicked() {
                        pick = Some((i, 0));
                    }
                    if ui.button("Always allow this session").clicked() {
                        pick = Some((i, 1));
                    }
                    if ui.button("Deny").clicked() {
                        pick = Some((i, 2));
                    }
                });
                ui.separator();
            }
            ui.weak("Turn this check off in Settings ▸ Assistant.");
        });
    let Some((i, choice)) = pick else { return };
    let (call, held_at) = app.assistant_rt.held.remove(i);
    if choice == 2 {
        let _ = call.reply.send(Err(format!("the user denied '{}'", call.name)));
        return;
    }
    if held_at.elapsed() >= held_wait(&call.name, &call.args) {
        // the agent was already told it timed out: running it now would surprise everyone
        app.toast(format!("'{}' waited too long - the agent gave up on it; nothing was run", call.name));
        return;
    }
    if choice == 1 {
        app.assistant_rt.always.push(allow_key(&call.name, &call.args));
    }
    app.assistant_rt.bypass = true;
    app.handle_tool(call);
    app.assistant_rt.bypass = false;
    ctx.request_repaint();
}

// ---------------------------------------------------------------------------------------------------
// registries

fn queued_comments(app: &App) -> usize {
    app.assistant_rt.backlog.iter().filter(|r| matches!(r.kind, Kind::Comment(_))).count()
}

/// PANE_DRAWERS entry.
pub(super) fn draw(app: &mut App, ui: &mut egui::Ui, pane: Pane) -> bool {
    if pane != Pane::Assistant {
        return false;
    }
    let chips = live_context(app).map(|c| chips_of(&c)).unwrap_or_default();
    let running = busy(app);
    let queued = queued_comments(app);
    let all = skills::all();
    let panes: Vec<Pane> = app.layout.rects.iter().map(|(p, _)| *p).filter(|p| *p != Pane::Assistant).collect();
    let out = {
        let s = &app.settings;
        let v = View {
            running,
            chips: &chips,
            skills: &all,
            recent: &s.skills_recent,
            undo_top: (app.undo.len(), app.undo.last().map(|e| e.label.as_str())),
            backend: &s.assistant_backend,
            model: &s.assistant_model,
            effort: &s.assistant_effort,
            queued,
            panes: &panes,
        };
        aui::draw(ui, &mut app.assistant, &v)
    };
    match out {
        Out::Send => send(app),
        Out::Stop => stop(app),
        Out::Undo => app.pending_actions.push(Action::Undo),
        Out::Retry(i) => retry(app, i),
        Out::Backend(b) => {
            app.settings.assistant_backend = b.to_string();
            app.settings.assistant_model.clear(); // each agent has its own models
            app.assistant.model_other = false;
            app.settings.save();
        }
        Out::Model(m) => {
            app.settings.assistant_model = m;
            app.settings.save();
        }
        Out::Effort(e) => {
            app.settings.assistant_effort = e;
            app.settings.save();
        }
        Out::AddTask => add_task(app),
        Out::Run(only) => {
            if let Err(e) = run_tasks(app, only) {
                if only.is_some() {
                    app.toast(format!("Task failed: {e}"));
                }
            }
        }
        Out::SaveScript(id) => save_script(app, id),
        Out::Attach(what) => attach(app, what),
        Out::Comments(on) => {
            let rt = &mut app.assistant_rt;
            rt.comment_slot = None;
            if on {
                rt.comment_since = Some(tools_context::max_marker_id(&app.project));
            } else {
                rt.comment_since = None;
                rt.backlog.retain(|r| !matches!(r.kind, Kind::Comment(_)));
            }
        }
        Out::Skill(o) => skill_out(app, o),
        Out::None => {}
    }
    if app.assistant_rt.shots.iter().any(|s| !s.asked) {
        ui.ctx().request_repaint(); // asked for on the next frame
    }
    true
}

// ---------------------------------------------------------------------------------------------------
// skills (ws:skills)

fn skill_out(app: &mut App, o: aui::SkillOut) {
    match o {
        aui::SkillOut::Delete(name) => crate::ui::confirm::ask_app(
            "Delete skill?",
            format!("Delete /{name}? Its file is removed from your skills folder."),
            move |app| match skills::delete(&name) {
                Ok(()) => app.toast(format!("Deleted /{name}")),
                Err(e) => app.toast(e),
            },
        ),
        aui::SkillOut::Draft => {
            let Some(d) = app.assistant.skill_draft.take() else { return };
            let mut args = d.what.trim().to_string();
            if !d.name.is_empty() {
                args += &format!("\nName it `{}`.", d.name);
            }
            app.assistant.prompt = format!("/{} {args}", skills::CREATE);
            app.assistant.tab = aui::Tab::Chat;
            send(app);
        }
        aui::SkillOut::Save => {
            let Some(d) = app.assistant.skill_draft.as_mut() else { return };
            let taken = d.editing.as_deref() != Some(d.name.as_str())
                && skills::all().iter().any(|s| s.name == d.name && s.source == Source::User);
            if taken {
                d.error = format!("You already have a skill named {}", d.name);
                return;
            }
            match skills::save(&d.name, &d.description, &d.args, &d.body) {
                Ok(_) => {
                    if let Some(old) = d.editing.as_deref().filter(|old| *old != d.name) {
                        let _ = skills::delete(old);
                    }
                    let name = d.name.clone();
                    app.assistant.skill_draft = None;
                    app.toast(format!("Saved /{name}"));
                }
                Err(e) => d.error = e,
            }
        }
    }
}

/// WINDOW_DRAWERS entry: the New / Edit skill window and a library skill's read-first window.
pub(super) fn skill_windows(app: &mut App, ctx: &egui::Context) {
    aui::preview_window(ctx, &mut app.assistant);
    if let Some(rect) = aui::area_overlay(ctx, &mut app.assistant) {
        queue_shot(app, rect, "Area".into(), None);
        ctx.request_repaint();
    }
    if app.assistant.skill_draft.is_some() {
        let running = busy(app);
        if let Out::Skill(o) = aui::skill_window(ctx, &mut app.assistant, running) {
            skill_out(app, o);
        }
    }
    let Some((s, _)) = &app.assistant_rt.read_first else { return };
    let Some(run) = aui::read_first_window(ctx, s) else { return };
    let Some((s, task)) = app.assistant_rt.read_first.take() else { return };
    if run {
        app.settings.skills_accepted.insert(s.name.clone(), skills::content_hash(&s));
        app.settings.save();
        if task {
            add_task(app);
        } else {
            send(app);
        }
    }
    ctx.request_repaint();
}

/// Quick prompts become user skills, once: off the UI thread, then the listing is re-read.
fn migrate_quick_prompts(app: &mut App) {
    if app.settings.skills_migrated {
        return;
    }
    app.settings.skills_migrated = true;
    app.settings.save();
    // only prompts the user wrote or changed: the stock five are covered by the built-in skills
    let stock = crate::settings::default_assistant_quick();
    let quick: Vec<(String, String)> =
        app.settings.assistant_quick.iter().filter(|q| !stock.contains(q)).cloned().collect();
    if quick.is_empty() {
        return;
    }
    let _ = std::thread::Builder::new().name("skills-migrate".into()).spawn(move || {
        let taken: Vec<String> = skills::all().iter().filter(|s| s.source != Source::User).map(|s| s.name.clone()).collect();
        if skills::migrate_in(&skills::user_dir(), &quick, &taken) > 0 {
            skills::invalidate();
        }
    });
}

/// FRAME_HOOKS entry: the conversation store, "Add to Assistant" clicks, a dictated / quick prompt
/// waiting to go, new comments to handle, the queue, the server coming up for a queued request, and
/// the running agent's events. Schedules nothing when idle.
pub(super) fn tick(app: &mut App, ctx: &egui::Context) {
    app.assistant_rt.ctx.get_or_insert_with(|| ctx.clone());
    migrate_quick_prompts(app); // ws:skills: once
    sync_store(app);
    shots_tick(app, ctx);
    paste_tick(app, ctx);
    for pin in aui::take_pins() {
        pin_add(app, pin);
    }
    // dictation (Push to Talk, voice.start) fills the chat prompt: show it
    if app.assistant.mic_wanted {
        app.assistant.tab = aui::Tab::Chat;
    }
    if std::mem::take(&mut app.assistant.send_now) {
        app.assistant.tab = aui::Tab::Chat;
        send(app);
    }
    if app.assistant.comments && app.settings.assistant_backend != "external" {
        let rt = &mut app.assistant_rt;
        let since = *rt.comment_since.get_or_insert_with(|| tools_context::max_marker_id(&app.project));
        if let Some((v, id)) = tools_context::settled_comment(&app.project, since, Instant::now(), &mut rt.comment_slot) {
            rt.comment_since = Some(id);
            let conv = app.assistant.conv_mut().id;
            enqueue(app, comment_req(v, id, conv));
        }
        if let Some((_, _, at)) = app.assistant_rt.comment_slot {
            app.animate_until(ctx, at + tools_context::COMMENT_SETTLE);
        }
    }
    if !busy(app) {
        if let Some(next) = app.assistant_rt.backlog.pop_front() {
            app.assistant_rt.queued = Some(next);
        }
    }
    if app.assistant_rt.queued.is_some() {
        start_queued(app);
        if app.assistant_rt.queued.is_some() {
            app.animate_until(ctx, Instant::now() + POLL);
        }
    }
    if app.assistant_rt.run.is_none() {
        return;
    }
    // the agent may think for longer than the group's 30 s idle timeout between tool calls: while it
    // runs, its request stays one undo group
    if let Some(g) = app.mcp_ctx.group.as_mut() {
        g.last_call = Instant::now();
    } else if let Some((_, label, _)) = &app.assistant_rt.active {
        // a hand edit / Undo closed it (tools_context::tick, close_group_for_user_edit): the rest of
        // the request is a fresh group under the same label, above the user's own entry
        app.mcp_ctx.group = Some(super::tools_context::UndoGroup {
            label: label.clone(),
            before: app.project.to_json(),
            undo_len: app.undo.len(),
            top_at: app.undo.last().map(|e| e.at),
            last_call: Instant::now(),
            idle: super::tools_context::GROUP_IDLE,
        });
        app.assistant_rt.split = true;
    }
    let Some(run) = app.assistant_rt.run.as_ref() else { return };
    let mut events = Vec::new();
    let mut ended = false;
    loop {
        match run.rx.try_recv() {
            Ok(e) => events.push(e),
            Err(TryRecvError::Empty) => break,
            Err(TryRecvError::Disconnected) => {
                ended = true;
                break;
            }
        }
    }
    let had = !events.is_empty();
    for e in events {
        match e {
            Event::Session(s) => remember_session(app, s),
            Event::Text(t) => app.assistant.push(Line::Text(t)),
            // stdout lines (codex, custom) arrive one by one: they are one Markdown text, so a fenced
            // block or a list stays whole
            Event::Line(t) => match app.assistant.target_mut().lines.last_mut() {
                Some(Line::Text(prev)) => {
                    prev.push('\n');
                    prev.push_str(&t);
                }
                _ => app.assistant.push(Line::Text(t)),
            },
            Event::Tool(t) => app.assistant.push(Line::Tool(t)),
            Event::Info(t) => app.assistant.push(Line::Info(t)),
            Event::Error(t) => app.assistant.push(Line::Error(t)),
            Event::Done { ok, session } => {
                if let Some(s) = session {
                    remember_session(app, s);
                }
                app.assistant_rt.ok = ok;
                ended = true;
            }
        }
    }
    if ended {
        finish(app);
        ctx.request_repaint();
    } else {
        if had {
            ctx.request_repaint();
        }
        app.animate_until(ctx, Instant::now() + POLL);
    }
}

/// The agent's session id belongs to the conversation the request runs in (switching is disabled
/// while one runs), so its next message resumes it.
fn remember_session(app: &mut App, s: String) {
    let c = app.assistant.target_mut();
    if c.session.as_deref() != Some(s.as_str()) {
        c.session = Some(s);
        app.assistant.dirty = true;
    }
}

/// ACT_HANDLERS entry.
pub(super) fn act(app: &mut App, a: Action) -> bool {
    match a {
        Action::ToggleAssistant => {
            app.toggle_pane(Pane::Assistant);
            true
        }
        Action::AssistantAddSelection => {
            pin_add(app, Pin::Smart);
            true
        }
        Action::ToggleAssistantFollow => {
            app.settings.assistant_follow = !app.settings.assistant_follow;
            app.settings.save();
            true
        }
        _ => false,
    }
}

/// The palette's `>` row: "Ask the assistant: …".
pub(super) fn palette_row(text: &str) -> crate::ui::palette::Row {
    let text = text.trim();
    crate::ui::palette::Row {
        cmd: crate::ui::palette::Command::Action(Action::ToggleAssistant),
        label: if text.is_empty() { "Ask the assistant: type a request".into() } else { format!("Ask the assistant: {text}") },
        shortcut: String::new(),
        glyph: Some(crate::ui::tools::Glyph::Assistant),
        enabled: !text.is_empty(),
        reason: None,
        ..Default::default()
    }
}

fn task_json(t: &aui::Task) -> Value {
    json!({"id": t.id, "title": t.title, "note": t.note, "status": t.status.label(), "error": t.error, "code": t.code})
}

/// ws:assistant-luau: a task script is running (`scripts.eval` refuses to start inside one).
pub(super) fn running_task(app: &App) -> bool {
    app.assistant_rt.running_task
}

fn no_reentry(app: &App) -> Result<(), String> {
    if app.assistant_rt.running_task {
        return Err("not from inside a running task".into());
    }
    Ok(())
}

pub const TOOLS: &[ToolDef] = &[
    ToolDef {
        name: "assistant.send",
        desc: "Send a request to the Assistant pane's chat, as if the user typed it (one request at a time).",
        args: &["prompt:string:true:the request"],
        kind: ToolKind::Ui,
        run: |app, args| {
            ask(app, req(arg_str(args, "prompt"), "prompt")?)?;
            Ok(ToolOutcome::Done(json!({"ok": true, "backend": app.settings.assistant_backend})))
        },
    },
    ToolDef {
        name: "assistant.status",
        desc: "The Assistant pane: whether a request is running, the backend, requests waiting (for assistant.wait, \
               and the queue behind the running one), and the last 20 lines of the shown conversation.",
        args: &[],
        kind: ToolKind::Read,
        run: |app, _| {
            let log: Vec<String> = app.assistant.log().iter().rev().take(20).rev().map(|l| format!("{l:?}")).collect();
            Ok(ToolOutcome::Done(json!({
                "running": busy(app),
                "backend": app.settings.assistant_backend,
                "model": app.settings.assistant_model,
                "pending": app.assistant.pending.len(),
                "queued": app.assistant_rt.backlog.len(),
                "held_calls": app.assistant_rt.held.iter().map(|(c, _)| c.name.clone()).collect::<Vec<_>>(),
                "log": log,
            })))
        },
    },
    ToolDef {
        name: "assistant.stop",
        desc: "Stop the Assistant pane's running request (its edits so far stay, as one undo step).",
        args: &[],
        kind: ToolKind::Ui,
        run: |app, _| {
            let was = busy(app);
            stop(app);
            Ok(ToolOutcome::Done(json!({"ok": true, "stopped": was})))
        },
    },
    ToolDef {
        name: "assistant.add_selection",
        desc: "Pin a snapshot of the user's selection to the Assistant's next chat message and show the pane \
               (same as right-click ▸ Add to Assistant). what: selection (clips, else transcript words, a marker, \
               else In/Out), clips, words, marker or range.",
        args: &["what:string:false:default selection"],
        kind: ToolKind::Ui,
        run: |app, args| {
            let pin = Pin::parse(arg_str(args, "what").unwrap_or("")).ok_or("what: selection, clips, words, marker or range")?;
            let chip = aui::pin_chip(&tools_context::context_json(app), pin).ok_or("nothing of that kind is selected")?;
            pin_add(app, pin);
            Ok(ToolOutcome::Done(json!({"ok": true, "label": chip.label, "pinned": app.assistant.pinned.len()})))
        },
    },
    ToolDef {
        name: "assistant.capture",
        desc: "Attach something to the Assistant's next chat message (it shows in the composer): what = frame (the                frame at the playhead), panel (a screenshot of `pane`, a pane title that is on screen), timeline (the                In/Out range - else the selected clips - as a picture of the Timeline plus its clips and tracks) or                files (`paths`). Screenshots land a frame or two later.",
        args: &["what:string:true:frame, panel, timeline or files", "pane:string:false:for panel", "paths:array:false:for files"],
        kind: ToolKind::Ui,
        run: |app, args| {
            let what = match req(arg_str(args, "what"), "what")? {
                "frame" => Attach::Frame,
                "timeline" => Attach::Timeline,
                "panel" => {
                    let name = req(arg_str(args, "pane"), "pane")?;
                    let pane = Pane::by_title(name).ok_or_else(|| format!("unknown pane '{name}'"))?;
                    if !app.layout.rects.iter().any(|(p, _)| *p == pane) {
                        return Err(format!("pane '{name}' is not on screen"));
                    }
                    Attach::Panel(pane)
                }
                "files" => {
                    let paths: Vec<PathBuf> = args
                        .get("paths")
                        .and_then(Value::as_array)
                        .ok_or("paths: an array of file paths")?
                        .iter()
                        .filter_map(|p| p.as_str().map(PathBuf::from))
                        .collect();
                    attach_paths(app, &paths);
                    return Ok(ToolOutcome::Done(json!({"ok": true, "attached": app.assistant.images.len()})));
                }
                w => return Err(format!("what: frame, panel, timeline or files (not '{w}')")),
            };
            attach(app, what);
            Ok(ToolOutcome::Done(json!({"ok": true, "attached": app.assistant.images.len(), "capturing": app.assistant_rt.shots.len()})))
        },
    },
    ToolDef {
        name: "assistant.attachments",
        desc: "What the user attached to the current Assistant request (pictures, screenshots, files): \
               [{index, name, kind: image|file, width, height, path}]. assistant.attachment{index} returns one.",
        args: &[],
        kind: ToolKind::Read,
        run: |app, _| {
            let list: Vec<Value> = app
                .assistant_rt
                .attachments
                .iter()
                .enumerate()
                .map(|(i, a)| {
                    json!({"index": i, "name": a.name, "kind": if a.is_image() { "image" } else { "file" },
                           "width": a.width, "height": a.height, "path": a.path()})
                })
                .collect();
            Ok(ToolOutcome::Done(json!(list)))
        },
    },
    ToolDef {
        name: "assistant.attachment",
        desc: "One thing the user attached to the current Assistant request: a picture as an image (PNG); a text \
               file as its text (up to 200 KB); any other file as its path and size.",
        args: &["index:integer:true:0-based, see assistant.attachments"],
        kind: ToolKind::Read,
        run: |app, args| {
            let i = req(arg_u64(args, "index"), "index")? as usize;
            let a = app.assistant_rt.attachments.get(i).ok_or(format!("no attachment {i}"))?;
            if !a.is_image() {
                let path = a.path();
                let size = std::fs::metadata(path).map(|m| m.len()).map_err(|e| format!("attachment {i}: {e}"))?;
                let text = (size <= 200_000).then(|| std::fs::read_to_string(path).ok()).flatten();
                return Ok(ToolOutcome::Done(json!({"index": i, "name": a.name, "path": path, "size": size, "text": text})));
            }
            let png = std::fs::read(&a.png).map_err(|e| format!("attachment {i}: {e}"))?;
            Ok(ToolOutcome::Image {
                png_b64: super::thumbs::base64(&png),
                meta: json!({"index": i, "name": a.name, "width": a.width, "height": a.height}),
            })
        },
    },
    ToolDef {
        name: "tasks.add",
        desc: "Hand a Luau script over to the Assistant's Tasks queue for the user to review and run (it is not \
               run now). title: a few words; note: one line on what it does.",
        args: &["title:string:true:", "code:string:true:the Luau script (editor.tool(name, args) calls tools)", "note:string:false:"],
        kind: ToolKind::Ui,
        run: |app, args| {
            no_reentry(app)?;
            let (title, code) = (req(arg_str(args, "title"), "title")?, req(arg_str(args, "code"), "code")?);
            let slot = match app.assistant_rt.active.as_ref().map(|a| a.0) {
                Some(Kind::Task(id)) => Some(id),
                _ => None,
            };
            let id = aui::task_hand_over(&mut app.assistant.target_mut().tasks, slot, title, code, arg_str(args, "note").unwrap_or(""));
            app.assistant.dirty = true;
            Ok(ToolOutcome::Done(json!({"ok": true, "id": id})))
        },
    },
    ToolDef {
        name: "tasks.list",
        desc: "The Assistant's Tasks queue in order: [{id, title, note, status, error, code}].",
        args: &[],
        kind: ToolKind::Read,
        run: |app, _| {
            let list: Vec<Value> = app.assistant.conv().map(|c| c.tasks.iter().map(task_json).collect()).unwrap_or_default();
            Ok(ToolOutcome::Done(json!(list)))
        },
    },
    ToolDef {
        name: "tasks.run",
        desc: "Run one task, or (no id) every Ready task in queue order; each run is one undo step and a failing \
               task stops the queue.",
        args: &["id:integer:false:"],
        kind: ToolKind::Ui,
        run: |app, args| {
            no_reentry(app)?;
            let ran = run_tasks(app, arg_u64(args, "id"))?;
            Ok(ToolOutcome::Done(json!({"ok": true, "ran": ran})))
        },
    },
    ToolDef {
        name: "tasks.remove",
        desc: "Remove a task from the Assistant's queue.",
        args: &["id:integer:true:"],
        kind: ToolKind::Ui,
        run: |app, args| {
            no_reentry(app)?;
            let id = req(arg_u64(args, "id"), "id")?;
            if !aui::task_remove(app.assistant.tasks_mut(), id) {
                return Err(format!("no task {id}"));
            }
            app.assistant.dirty = true;
            Ok(ToolOutcome::Done(json!({"ok": true})))
        },
    },
    ToolDef {
        name: "tasks.set",
        desc: "Change a task's title and/or code (new code makes it Ready again).",
        args: &["id:integer:true:", "code:string:false:", "title:string:false:"],
        kind: ToolKind::Ui,
        run: |app, args| {
            no_reentry(app)?;
            let id = req(arg_u64(args, "id"), "id")?;
            aui::task_set(app.assistant.tasks_mut(), id, arg_str(args, "title"), arg_str(args, "code"))?;
            app.assistant.dirty = true;
            Ok(ToolOutcome::Done(json!({"ok": true})))
        },
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chips_and_filtering() {
        let c = json!({
            "playhead": 12.0, "in": 1.0, "out": 4.0,
            "selected_clips": [{"id": 3, "name": "intro.mp4"}],
            "selected_transitions": [], "selected_cues": [], "edit_point": null, "selected_words": null,
            "selected_markers": [{"id": 9, "note": "too loud here"}],
        });
        let chips = chips_of(&c);
        let keys: Vec<&str> = chips.iter().map(|(k, _)| *k).collect();
        assert_eq!(keys, ["selection", "range", "playhead", "comment"]);
        assert_eq!(chips[0].1, "Clip: intro.mp4");
        let f = filter_context(c.clone(), &["selection", "comment"]);
        assert!(f.get("selected_clips").is_none() && f.get("selected_markers").is_none());
        assert_eq!(f["in"], 1.0, "chips left on stay");
        assert_eq!(filter_context(c.clone(), &[]), c);
        // nothing selected, no range, no comment: just the playhead
        let bare = json!({"playhead": 0.0, "in": null, "out": null, "selected_clips": [], "selected_markers": []});
        assert_eq!(chips_of(&bare).len(), 1);
    }

    /// What a request carries: follow off = only the pinned snapshots; the preamble says how to see images.
    #[test]
    fn request_context_and_preamble() {
        let live = json!({"playhead": 2.0, "in": 1.0, "out": 3.0, "selected_clips": [{"id": 1}]});
        let pin = aui::Chip { label: "In/Out".into(), data: json!({"kind": "range", "in": 1.0, "out": 3.0}) };
        let off = compose_context(None, &[], &[]);
        assert_eq!(off, json!({}), "follow off, nothing pinned: nothing attached");
        assert!(preamble(&off, &[]).contains("No context is attached"));
        let c = compose_context(None, &[], std::slice::from_ref(&pin));
        assert_eq!(c, json!({"pinned": [pin.data.clone()]}));
        let c = compose_context(Some(live), &["selection"], &[pin]);
        assert!(c.get("selected_clips").is_none() && c["playhead"] == 2.0 && c["pinned"].is_array());
        let img = aui::Attachment { name: "Frame".into(), png: "a.png".into(), width: 4, height: 2, ..Default::default() };
        let file = aui::Attachment::file(Path::new("C:\\brief.txt"));
        let p = preamble(&c, &[img, file]);
        assert!(p.contains("\"pinned\"") && p.contains("2 items") && p.contains("assistant.attachment{index}"), "{p}");
        assert!(p.contains("0: image \"Frame\" 4x2") && p.contains("1: file C:\\brief.txt"), "{p}");
        assert!(!preamble(&c, &[]).contains("assistant.attachment"));
    }

    #[test]
    fn guarded_tools() {
        let none = json!({});
        for t in [
            "export.video",
            "export.edl",
            "export.xml",
            "project.open",
            "project.new",
            "settings.set",
            "scripts.run",
            "tasks.run",
            "media.convert",
            "skills.save",
            "skills.delete",
        ] {
            assert!(leaves_project(t, &none), "{t}");
        }
        assert_eq!(held_wait("skills.save", &none), mcp::HELD_WAIT);
        assert_eq!(held_wait("timeline.cut", &none), Duration::from_secs(60));
        assert_eq!(held_wait("export.video", &none), mcp::HELD_WAIT);
        for t in ["timeline.cut", "editor.context", "markers.add", "project.summary", "ui.action", "tasks.add", "tasks.list", "skills.list", "skills.get"] {
            assert!(!leaves_project(t, &none), "{t}");
        }
        // ui.action: only the Actions that leave the project, and every listed id is a real Action
        for id in ["new_project", "open_project", "export", "quick_export", "export_xml", "export_edl", "save", "settings"] {
            assert!(Action::from_id(id).is_some(), "{id}");
            assert!(leaves_project("ui.action", &json!({"id": id})), "{id}");
        }
        assert!(!leaves_project("ui.action", &json!({"id": "undo"})));
        assert_eq!(allow_key("ui.action", &json!({"id": "export"})), "ui.action:export");
        assert_eq!(allow_key("export.xml", &json!({"id": "x"})), "export.xml");
    }

    /// A panel screenshot is the pane's rectangle in pixels (points x scale); an area is the dragged
    /// rectangle; a huge capture is capped at `CAPTURE_MAX` on its long edge.
    #[test]
    fn panel_and_area_captures_have_the_right_size() {
        let dir = std::env::temp_dir().join(format!("se-assistant-shot-{}", std::process::id()));
        let img = egui::ColorImage::new([800, 600], vec![egui::Color32::from_rgb(10, 20, 30); 800 * 600]);
        let pane = egui::Rect::from_min_size(egui::pos2(100.0, 50.0), egui::vec2(200.0, 120.0));
        let a = shot_attachment(&img, pane, 2.0, &dir, "Timeline panel").unwrap();
        assert_eq!((a.width, a.height), (400, 240), "200 x 120 points at 2 px/pt");
        let area = egui::Rect::from_min_max(egui::pos2(10.0, 10.0), egui::pos2(47.0, 31.0));
        let a = shot_attachment(&img, area, 1.0, &dir, "Area").unwrap();
        assert_eq!((a.width, a.height), (37, 21));
        let png = std::fs::read(&a.png).unwrap();
        assert_eq!(&png[1..4], b"PNG");
        // off the edge: clamped to the window
        let a = shot_attachment(&img, egui::Rect::from_min_size(egui::pos2(700.0, 500.0), egui::vec2(300.0, 300.0)), 1.0, &dir, "x").unwrap();
        assert_eq!((a.width, a.height), (100, 100));
        let big = egui::ColorImage::new([4096, 1024], vec![egui::Color32::WHITE; 4096 * 1024]);
        let a = shot_attachment(&big, egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(4096.0, 1024.0)), 1.0, &dir, "big").unwrap();
        assert_eq!((a.width, a.height), (aui::CAPTURE_MAX, 512));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The timeline range's chip lists exactly the clips overlapping it and their tracks.
    #[test]
    fn timeline_range_chip_has_its_clips_and_tracks() {
        let mut p = crate::model::Project::new();
        let t = p.add_track(crate::model::TrackKind::Video);
        for (i, (start, name)) in [(0.0, "a"), (5.0, "b"), (12.0, "c")].into_iter().enumerate() {
            p.tracks[t].clips.push(crate::model::Clip::new(70 + i as Id, crate::model::ClipKind::Video, name, start, 4.0));
        }
        let chip = timeline_chip(&p, 3.0, 10.0);
        let names: Vec<&str> = chip.data["clips"].as_array().unwrap().iter().map(|c| c["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["a", "b"], "c starts after the range");
        assert_eq!(chip.data["kind"], "timeline_range");
        assert_eq!(chip.data["tracks"][0]["index"], t);
        assert_eq!(chip.label, "Timeline 3.0s–10.0s · 2 clips");
    }

    #[test]
    fn palette_row_needs_text() {
        assert!(!palette_row("  ").enabled);
        let r = palette_row(" cut the ums ");
        assert!(r.enabled && r.label == "Ask the assistant: cut the ums");
    }

    /// Every new tool is registered, with the undo kind its handler assumes.
    #[test]
    fn assistant_and_task_tools_are_registered() {
        for (name, kind) in [
            ("assistant.add_selection", ToolKind::Ui),
            ("assistant.capture", ToolKind::Ui),
            ("assistant.attachments", ToolKind::Read),
            ("assistant.attachment", ToolKind::Read),
            ("tasks.add", ToolKind::Ui),
            ("tasks.list", ToolKind::Read),
            ("tasks.run", ToolKind::Ui),
            ("tasks.remove", ToolKind::Ui),
            ("tasks.set", ToolKind::Ui),
        ] {
            assert_eq!(mcp::tools::find(name).map(|d| d.kind), Some(kind), "{name}");
        }
        assert!(TASK_BRIEF.contains("tasks.add{title, code, note}") && TASK_BRIEF.contains("editor.tool(name, args)"));
        let r = comment_req(json!({"note": "trim this", "marker_id": 4}), 4, 7);
        assert!(r.full.contains("Comment: trim this") && r.kind == Kind::Comment(4) && r.label == "Comment: trim this");
    }
}
