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
    /// Whose conversations are loaded (`App::project_path`) and their store bucket.
    loaded: Option<(Option<PathBuf>, String)>,
    /// "Handle my comments": markers up to this id are not new; the candidate settling.
    comment_since: Option<Id>,
    comment_slot: Option<(Id, String, Instant)>,
    // ---- ws:skills ----
    /// The skill the running request runs.
    skill: Option<String>,
    /// A library skill waiting for its first-run Run / Cancel, and whether it came from the Tasks box.
    read_first: Option<(Skill, bool)>,
}

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
pub(super) fn preamble(ctx: &Value, images: usize) -> String {
    let mut s = String::from(
        "You are editing the video project open in Simple Editor, live, through its MCP tools. This whole request \
         is already one undo step: do not call edit.begin or edit.end.",
    );
    if ctx.as_object().is_some_and(|o| !o.is_empty()) {
        s.push_str(&format!(
            " The context the user attached (editor.context JSON; `pinned` = what they added on purpose):\n{ctx}"
        ));
    } else {
        s.push_str(" No context is attached - call editor.context when you need it.");
    }
    if images > 0 {
        let what = if images == 1 { "1 image".to_string() } else { format!("{images} images") };
        s.push_str(&format!(
            "\nThe user attached {what}: assistant.attachment{{index}} (0 to {}) shows each one.",
            images - 1
        ));
    }
    s
}

fn live_context(app: &App) -> Option<Value> {
    app.settings.assistant_follow.then(|| tools_context::context_json(app))
}

// ---------------------------------------------------------------------------------------------------
// sending

/// Send `text` as a chat request (the palette's `>` prefix, `assistant.send`).
pub(super) fn ask(app: &mut App, text: &str) -> Result<(), String> {
    if text.trim().is_empty() {
        return Err("empty request".into());
    }
    if busy(app) {
        return Err("the assistant is still working on the last request".into());
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
        st.conv_mut().title = aui::short(&prompt, 60);
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
    let full = format!("{}\n\nRequest: {request}", preamble(&ctx, images.len()));
    let skill = skill.map(|(s, _)| s.name);
    enqueue(app, Req { kind: Kind::Chat, label: prompt, full, images, skill });
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
    let full = format!("{}\n\n{TASK_BRIEF}\n\nTask: {what}", preamble(&ctx, 0));
    enqueue(app, Req { kind: Kind::Task(id), label: format!("Task: {desc}"), full, images: Vec::new(), skill });
}

/// "Handle my comments": a settled new comment becomes a background request.
fn comment_req(comment: Value, id: Id) -> Req {
    let note = comment["note"].as_str().unwrap_or("").to_string();
    let ctx = json!({"comment": comment});
    let full = format!(
        "{}\n\nThe user left this comment on the timeline (marker {id}). Do what it asks; the editor marks the \
         comment done when you finish, so don't mark it yourself.\nComment: {note}",
        preamble(&ctx, 0)
    );
    Req { kind: Kind::Comment(id), label: format!("Comment: {}", aui::short(&note, 50)), full, images: Vec::new(), skill: None }
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
    if let Some(t) = app.assistant.tasks_mut().iter_mut().find(|t| t.id == id && t.status == TaskStatus::Writing) {
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
    let resume = if backend == agent::Backend::Claude { app.assistant.conv().and_then(|c| c.session.clone()) } else { None };
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
    let made = match what {
        Attach::Pin(pin) => return pin_add(app, pin),
        Attach::Frame => {
            let t = app.playhead;
            app.render_frame_now(t, aui::ATTACH_MAX).ok_or_else(|| "Couldn't render the frame".to_string()).and_then(|f| {
                let f = tools_context::tile(&[f], 1); // RGBA, opaque
                aui::write_attachment(&attach_dir(app), &format!("Frame at {t:.2}s"), &f.rgba, f.width, f.height)
            })
        }
        Attach::File => {
            let Some(path) =
                rfd::FileDialog::new().add_filter("Images", &["png", "jpg", "jpeg", "bmp", "webp"]).pick_file()
            else {
                return;
            };
            load_image(&path).and_then(|(rgba, w, h)| {
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                aui::write_attachment(&attach_dir(app), &name, &rgba, w, h)
            })
        }
    };
    match made {
        Ok(a) => {
            app.assistant.images.push(a);
            app.assistant.tab = aui::Tab::Chat;
            app.assistant.focus_prompt = true;
        }
        Err(e) => app.toast(e),
    }
}

/// A PNG / JPG / … decoded (through ffmpeg, like an image clip) to RGBA at most `ATTACH_MAX` on its long
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

fn snapshot(app: &App) -> Option<(String, String, Vec<PathBuf>)> {
    let (_, bucket) = app.assistant_rt.loaded.as_ref()?;
    let (json, remove) = aui::snapshot(&app.assistant.convs);
    Some((bucket.clone(), json, remove))
}

/// Save the conversations now, on this thread (the app is closing).
pub(super) fn save_now(app: &mut App) {
    if let Some((bucket, json, _)) = snapshot(app) {
        let _ = aui::save(&aui::store_dir(), &bucket, &json);
    }
}

/// Load the project's conversations when the project changed (saving the old ones), and save them off
/// the UI thread when something changed.
fn sync_store(app: &mut App) {
    let path = app.project_path.clone();
    if app.assistant_rt.loaded.as_ref().is_none_or(|(p, _)| *p != path) && !busy(app) {
        if app.assistant.dirty {
            if let Some((bucket, json, remove)) = snapshot(app) {
                aui::save_async(aui::store_dir(), bucket, json, remove);
            }
        }
        let bucket = aui::bucket(path.as_deref());
        app.assistant.load(aui::load(&aui::store_dir(), &bucket));
        app.assistant.dirty = false;
        app.assistant_rt.loaded = Some((path, bucket));
        return;
    }
    if std::mem::take(&mut app.assistant.dirty) {
        if let Some((bucket, json, remove)) = snapshot(app) {
            aui::save_async(aui::store_dir(), bucket, json, remove);
        }
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
        };
        aui::draw(ui, &mut app.assistant, &v)
    };
    match out {
        Out::Send => send(app),
        Out::Stop => stop(app),
        Out::Undo => app.pending_actions.push(Action::Undo),
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
    migrate_quick_prompts(app); // ws:skills: once
    sync_store(app);
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
            enqueue(app, comment_req(v, id));
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
            Event::Text(t) | Event::Line(t) => app.assistant.push(Line::Text(t)),
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
    let c = app.assistant.conv_mut();
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
    }
}

fn task_json(t: &aui::Task) -> Value {
    json!({"id": t.id, "title": t.title, "note": t.note, "status": t.status.label(), "error": t.error, "code": t.code})
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
        name: "assistant.attachments",
        desc: "The images the user attached to the current Assistant request: [{index, name, width, height}]. \
               assistant.attachment{index} returns one.",
        args: &[],
        kind: ToolKind::Read,
        run: |app, _| {
            let list: Vec<Value> = app
                .assistant_rt
                .attachments
                .iter()
                .enumerate()
                .map(|(i, a)| json!({"index": i, "name": a.name, "width": a.width, "height": a.height}))
                .collect();
            Ok(ToolOutcome::Done(json!(list)))
        },
    },
    ToolDef {
        name: "assistant.attachment",
        desc: "One image the user attached to the current Assistant request, as an image (PNG).",
        args: &["index:integer:true:0-based, see assistant.attachments"],
        kind: ToolKind::Read,
        run: |app, args| {
            let i = req(arg_u64(args, "index"), "index")? as usize;
            let a = app.assistant_rt.attachments.get(i).ok_or(format!("no attachment {i}"))?;
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
            let id = aui::task_hand_over(app.assistant.tasks_mut(), slot, title, code, arg_str(args, "note").unwrap_or(""));
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
        assert!(preamble(&off, 0).contains("No context is attached"));
        let c = compose_context(None, &[], std::slice::from_ref(&pin));
        assert_eq!(c, json!({"pinned": [pin.data.clone()]}));
        let c = compose_context(Some(live), &["selection"], &[pin]);
        assert!(c.get("selected_clips").is_none() && c["playhead"] == 2.0 && c["pinned"].is_array());
        let p = preamble(&c, 2);
        assert!(p.contains("\"pinned\"") && p.contains("2 images") && p.contains("assistant.attachment{index} (0 to 1)"), "{p}");
        assert!(!preamble(&c, 0).contains("assistant.attachment"));
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
        let r = comment_req(json!({"note": "trim this", "marker_id": 4}), 4);
        assert!(r.full.contains("Comment: trim this") && r.kind == Kind::Comment(4) && r.label == "Comment: trim this");
    }
}
