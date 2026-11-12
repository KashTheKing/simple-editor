//! ---- ws:mcp-context ----
//! What an agent co-editing over MCP needs beyond the edit tools:
//! - `editor.context`: one cheap snapshot of what the user is looking at (selection, playhead, in/out,
//!   selected transcript words with their text, page, sequence);
//! - undo groups: `App::begin_group` / `end_group` (tools `edit.begin` / `edit.end` / `edit.batch`) turn
//!   one request's many Mutate calls into ONE undo entry labelled "<author>: <label>", glow the changed
//!   clips and toast an Undo button. An open group closes itself after 30 s without a tool call, when
//!   the user edits by hand (the undo stack grows under it) or presses Undo / Redo;
//! - images: `render.frame` / `render.contact_sheet` return `ToolOutcome::Image` (MCP image content);
//! - `assistant.wait`: a long poll that returns the user's next Assistant-pane request or new comment;
//! - comments: the Add Comment action / `markers.comment` (a marker with a note over In/Out, else the
//!   selected clips, else the playhead).
//!
//! Everything here that can run without a live `App` (none is buildable in tests) is a plain function
//! with a test at the bottom.

use super::thumbs::base64;
use super::tools_helpers::*;
use super::*;
use crate::mcp::tools::{ToolDef, ToolKind, ToolOutcome};

/// An open group closes itself after this long without a tool call.
const GROUP_IDLE: Duration = Duration::from_secs(30);
/// How long the changed clips stay outlined after a group ends.
const FLASH_SECS: f32 = 1.5;
/// A new comment goes to `assistant.wait` once its note has stopped changing for this long (so a
/// comment being typed is not sent after its first letter).
pub(super) const COMMENT_SETTLE: Duration = Duration::from_millis(1500);
/// `render.contact_sheet` stops rendering tiles once this much time is used (UI thread).
const SHEET_BUDGET: Duration = Duration::from_secs(5);

/// An open undo group (`App::begin_group`).
pub(super) struct UndoGroup {
    /// "<author>: <label>" - the undo entry's History label and the toast.
    pub(super) label: String,
    /// The project when the group opened: the one entry `end_group` pushes.
    pub(super) before: String,
    /// `undo.len()` when it opened. Mutate tools push nothing while a group is open, so a longer stack
    /// means the user edited by hand (see `tick`).
    pub(super) undo_len: usize,
    /// The top undo entry's timestamp when it opened: a hand edit changes it even when the 200-entry
    /// cap keeps the length unchanged (see `hand_edited`).
    pub(super) top_at: Option<f64>,
    pub(super) last_call: Instant,
}

impl UndoGroup {
    /// The user pushed an undo entry by hand since the group opened.
    pub(super) fn hand_edited(&self, undo: &[UndoEntry]) -> bool {
        undo.len() > self.undo_len || undo.last().map(|e| e.at) != self.top_at
    }
}

/// A parked `assistant.wait` call.
pub(super) struct Wait {
    reply: Sender<Result<mcp::ToolReply, String>>,
    deadline: Instant,
    /// Markers with a higher id are new since the call.
    since: Id,
}

#[derive(Default)]
pub(super) struct ContextState {
    pub(super) group: Option<UndoGroup>,
    waits: Vec<Wait>,
    /// The comment candidate `assistant.wait` is watching settle: (marker id, note, first seen).
    comment: Option<(Id, String, Instant)>,
    /// When the changed-clip outline started (`TimelineState.flash`).
    flash_at: Option<Instant>,
    /// The configured port the running MCP server was started for (`App::sync_mcp`).
    pub(super) port_wanted: u16,
    /// Markers up to this id are not (or no longer) new comments for `assistant.wait`: the highest id
    /// when the first wait was parked, then the last comment delivered. Session-wide, so a comment
    /// left while the agent was busy between two waits is still delivered by the next one.
    pub(super) seen: Option<Id>,
}

impl App {
    /// Open an undo group: until `end_group`, Mutate tool calls push no undo entries of their own (each
    /// still rolls itself back when it fails). `author` "" = "Assistant"; an open group is closed first.
    pub(crate) fn begin_group(&mut self, label: &str, author: &str) {
        self.end_group();
        let author = if author.trim().is_empty() { "Assistant" } else { author.trim() };
        let label = if label.trim().is_empty() { "Edit" } else { label.trim() };
        self.mcp_ctx.group = Some(UndoGroup {
            label: format!("{author}: {label}"),
            before: self.project.to_json(),
            undo_len: self.undo.len(),
            top_at: self.undo.last().map(|e| e.at),
            last_call: Instant::now(),
        });
    }

    /// The user is about to edit by hand (a palette-run tool): close the agent's group first so the
    /// user's edit is its own undo step (inside a group it would push nothing of its own). A running
    /// in-app request continues in a fresh group (`assistant_ctl::tick`).
    pub(crate) fn close_group_for_user_edit(&mut self) {
        self.end_group();
    }

    /// Close the open group: push ONE labelled undo entry iff the project changed, outline the clips it
    /// changed and toast "<author>: <label>" with Undo. Returns whether an entry was pushed (false when
    /// no group was open or nothing changed).
    pub(crate) fn end_group(&mut self) -> bool {
        let Some(g) = self.mcp_ctx.group.take() else { return false };
        let now = self.project.to_json();
        if !push_group_entry(&mut self.undo, &mut self.redo, &g, &now) {
            return false;
        }
        if let Ok(old) = Project::from_json(&g.before) {
            self.timeline.flash = changed_clips(&old, &self.project);
            self.mcp_ctx.flash_at = Some(Instant::now());
        }
        self.toast_undo(g.label, Action::Undo);
        true
    }

    /// After a Mutate tool succeeded (`before` = its snapshot): one undo entry for it, or none while a
    /// group is open (the group pushes one when it ends).
    pub(super) fn commit_tool_edit(&mut self, before: String) {
        let now = self.project.to_json();
        record_edit(&mut self.undo, &mut self.redo, self.mcp_ctx.group.is_some(), before, &now);
        self.after_edit();
    }

    /// Park an `assistant.wait` reply; `tick` answers it.
    pub(super) fn park_wait(&mut self, reply: Sender<Result<mcp::ToolReply, String>>, deadline: Instant) {
        let max = max_marker_id(&self.project);
        let since = *self.mcp_ctx.seen.get_or_insert(max);
        self.mcp_ctx.waits.push(Wait { reply, deadline, since });
    }
}

/// Push `before` iff it differs from `now`, unless a group is open (`commit_tool_edit`'s pure half).
pub(super) fn record_edit(
    undo: &mut Vec<UndoEntry>,
    redo: &mut Vec<UndoEntry>,
    group_open: bool,
    before: String,
    now: &str,
) {
    if !group_open && before != now {
        push_undo_json(undo, redo, before);
    }
}

/// `end_group`'s undo half: insert the group's entry where the stack stood when it opened (beneath any
/// entry the user pushed by hand since - that entry's snapshot is the state right after the agent's
/// edits), labelled. False = nothing changed, nothing pushed.
/// ponytail: if the 200-entry cap dropped entries meanwhile the slot is off by that many; harmless.
pub(super) fn push_group_entry(undo: &mut Vec<UndoEntry>, redo: &mut Vec<UndoEntry>, g: &UndoGroup, now: &str) -> bool {
    let at = g.undo_len.min(undo.len());
    let after = undo.get(at).map_or(now, |e| e.json.as_str());
    if g.before == after {
        return false;
    }
    let entry =
        UndoEntry { json: g.before.clone(), label: g.label.clone(), at: now_secs(), category: HistoryCategory::Editing };
    undo.insert(at, entry);
    if undo.len() > 200 {
        undo.remove(0);
    }
    redo.clear();
    true
}

/// Clips that are new or differ (any field) in `new` compared to `old`.
/// ponytail: O(clips²) id lookups once per group end; index by id if 10k-clip timelines show up.
pub(super) fn changed_clips(old: &Project, new: &Project) -> Vec<Id> {
    new.all_clips()
        .filter(|(_, c)| old.clip(c.id).is_none_or(|o| serde_json::to_value(o).ok() != serde_json::to_value(*c).ok()))
        .map(|(_, c)| c.id)
        .collect()
}

pub(super) fn max_marker_id(p: &Project) -> Id {
    p.markers
        .iter()
        .map(|m| m.id)
        .chain(p.all_clips().flat_map(|(_, c)| c.markers.iter().map(|m| m.id)))
        .max()
        .unwrap_or(0)
}

/// A marker by id with the timeline offset of its clip (0 for timeline markers).
fn find_marker(p: &Project, id: Id) -> Option<(&crate::model::Marker, f64)> {
    if let Some(m) = p.markers.iter().find(|m| m.id == id) {
        return Some((m, 0.0));
    }
    p.all_clips().find_map(|(_, c)| c.markers.iter().find(|m| m.id == id).map(|m| (m, c.start)))
}

// ---------------------------------------------------------------------------------------------------
// editor.context

/// The view state `editor.context` reports beside the project (plain values, so a test can build one).
pub(super) struct View<'a> {
    pub(super) playhead: f64,
    pub(super) clips: &'a [Id],
    pub(super) transitions: &'a [Id],
    pub(super) cues: &'a [Id],
    pub(super) edit_point: Option<crate::ui::timeline::EditPoint>,
    /// The marker clicked on the timeline plus the Markers pane's selection.
    pub(super) markers: Vec<Id>,
    /// Transcript shown in the Subtitles pane and its selected word range (inclusive indices).
    pub(super) words: Option<(Id, (usize, usize))>,
    pub(super) page: &'a str,
    pub(super) mcp_port: u16,
}

fn view_of(app: &App) -> View<'_> {
    let mut markers: Vec<Id> = app.timeline.selected_marker.into_iter().collect();
    markers.extend(app.markers.selected.iter().copied().filter(|id| Some(*id) != app.timeline.selected_marker));
    let tr = &app.subtitles_ui.transcript;
    View {
        playhead: app.playhead,
        clips: &app.selection,
        transitions: &app.sel_transitions,
        cues: &app.timeline.sub_sel,
        edit_point: app.timeline.edit_point,
        markers,
        words: tr.clip.zip(tr.sel),
        page: &app.settings.page,
        mcp_port: app.mcp_port_running,
    }
}

pub(super) fn context_json(app: &App) -> Value {
    context_of(&app.project, &view_of(app))
}

/// `editor.context`: counts and lookups only (no project serialisation) - it runs at the start of
/// every agent request.
pub(super) fn context_of(p: &Project, v: &View) -> Value {
    let clips: Vec<Value> = v
        .clips
        .iter()
        .filter_map(|&id| p.clip(id))
        .map(|c| {
            json!({"id": c.id, "name": c.name, "kind": format!("{:?}", c.kind), "track": p.track_of(c.id),
                   "start": c.start, "end": c.end()})
        })
        .collect();
    let cues: Vec<Value> = p
        .subtitles
        .iter()
        .filter(|c| v.cues.contains(&c.id))
        .map(|c| json!({"id": c.id, "start": c.start, "end": c.end, "text": c.text}))
        .collect();
    let markers: Vec<Value> = v
        .markers
        .iter()
        .filter_map(|&id| find_marker(p, id))
        .map(|(m, off)| {
            json!({"id": m.id, "t": off + m.t, "duration": m.duration, "name": m.name, "note": m.note,
                   "done": m.done, "author": m.author})
        })
        .collect();
    let words = v.words.and_then(|(clip, (a, b))| {
        let w = p.transcript(clip)?.words.get(a.min(b)..=b.max(a))?;
        Some(json!({
            "clip_id": clip,
            "text": w.iter().map(|w| w.2.as_str()).collect::<Vec<_>>().join(" "),
            "start": w.first()?.0, "end": w.last()?.1, "first": a.min(b), "last": b.max(a),
        }))
    });
    let sequence = p.editing.and_then(|id| p.sequences.iter().find(|s| s.id == id)).map(|s| json!({"id": s.id, "name": s.name}));
    json!({
        "playhead": v.playhead,
        "in": p.in_point,
        "out": p.out_point,
        "duration": p.duration(),
        "fps": p.fps,
        "page": v.page,
        "sequence": sequence, // null = the main timeline
        "selected_clips": clips,
        "selected_transitions": v.transitions,
        "selected_cues": cues,
        "edit_point": v.edit_point.map(|e| json!({"track": e.track, "t": e.t, "side": format!("{:?}", e.side)})),
        "selected_markers": markers,
        "selected_words": words,
        "mcp_port": v.mcp_port,
    })
}

// ---------------------------------------------------------------------------------------------------
// images

/// `render.frame` (registered in tools_playback.rs): the frame at `t` as an image result.
pub(super) fn render_frame_image(app: &mut App, args: &Value) -> Result<ToolOutcome, String> {
    let t = req(arg_f64(args, "t"), "t")?;
    let w = arg_u64(args, "width").map(|w| w as u32).unwrap_or(640).clamp(16, 3840);
    let frame = app.render_frame_now(t, w).ok_or("render timed out")?;
    Ok(png_outcome(&frame, json!({"t": t, "width": frame.width, "height": frame.height})))
}

fn png_outcome(f: &Frame, meta: Value) -> ToolOutcome {
    ToolOutcome::Image { png_b64: base64(&mcp::png_encode(f)), meta }
}

/// `render.contact_sheet`: `count` frames evenly spread over [from, to] (both ends included), tiled
/// left-to-right, top-to-bottom, through the same render path as `render.frame`.
fn contact_sheet(app: &mut App, args: &Value) -> Result<ToolOutcome, String> {
    let from = req(arg_f64(args, "from"), "from")?;
    let to = req(arg_f64(args, "to"), "to")?;
    if to < from {
        return Err("'to' must not be before 'from'".into());
    }
    // runs on the UI thread: few tiles and a total budget, so the app never freezes for long
    let count = arg_u64(args, "count").unwrap_or(9).clamp(1, 16) as u32;
    let cols = ((count as f64).sqrt().ceil() as u32).max(1);
    let w = arg_u64(args, "width").map(|w| w as u32).unwrap_or(320).clamp(16, 3840 / cols);
    let mut times: Vec<f64> = (0..count)
        .map(|i| if count == 1 { from } else { from + (to - from) * i as f64 / (count - 1) as f64 })
        .collect();
    let started = Instant::now();
    let mut frames = Vec::with_capacity(times.len());
    for &t in &times {
        if !frames.is_empty() && started.elapsed() >= SHEET_BUDGET {
            break;
        }
        frames.push(app.render_frame_now(t, w).ok_or("render timed out")?);
    }
    let truncated = frames.len() < times.len();
    times.truncate(frames.len());
    let sheet = tile(&frames, cols);
    let meta = json!({"times": times, "cols": cols, "rows": (frames.len() as u32).div_ceil(cols),
                      "width": sheet.width, "height": sheet.height, "truncated": truncated});
    Ok(png_outcome(&sheet, meta))
}

/// Frames on one opaque black sheet, `cols` per row, each cell the size of the first frame.
pub(super) fn tile(frames: &[Arc<Frame>], cols: u32) -> Frame {
    let cols = cols.max(1);
    let (tw, th) = frames.first().map_or((1, 1), |f| (f.width.max(1), f.height.max(1)));
    let rows = (frames.len() as u32).div_ceil(cols).max(1);
    let mut sheet = Frame::new(tw * cols, th * rows);
    for px in sheet.rgba.chunks_exact_mut(4) {
        px[3] = 255;
    }
    for (i, f) in frames.iter().enumerate() {
        let (ox, oy) = ((i as u32 % cols) * tw, (i as u32 / cols) * th);
        let w = f.width.min(tw) as usize;
        for y in 0..f.height.min(th) {
            let src = &f.rgba[(y * f.width) as usize * 4..][..w * 4];
            let at = ((oy + y) * sheet.width + ox) as usize * 4;
            let dst = &mut sheet.rgba[at..at + w * 4];
            dst.copy_from_slice(src);
            if f.bgrx {
                for px in dst.chunks_exact_mut(4) {
                    px.swap(0, 2);
                    px[3] = 255;
                }
            }
        }
    }
    sheet
}

// ---------------------------------------------------------------------------------------------------
// edit.batch

/// Run `calls` ([{name, args}]) in order through `run`; the first failure stops it and reports
/// (index, name, error). `edit.*` calls can't be nested in a batch.
pub(super) fn run_calls(
    calls: &[Value],
    run: &mut dyn FnMut(&str, &Value) -> Result<Value, String>,
) -> Result<Vec<Value>, (usize, String, String)> {
    let empty = json!({});
    let mut out = Vec::with_capacity(calls.len());
    for (i, c) in calls.iter().enumerate() {
        let name = c.get("name").and_then(Value::as_str).ok_or((i, String::new(), "missing 'name'".to_string()))?;
        if name.starts_with("edit.") {
            return Err((i, name.to_string(), "edit.* can't be nested in a batch".into()));
        }
        let args = c.get("args").unwrap_or(&empty);
        out.push(run(name, args).map_err(|e| (i, name.to_string(), e))?);
    }
    Ok(out)
}

fn batch(app: &mut App, args: &Value) -> Result<ToolOutcome, String> {
    let calls = req(args.get("calls").and_then(Value::as_array), "calls")?.clone();
    // joins a group opened by edit.begin; otherwise it is its own group
    let own = app.mcp_ctx.group.is_none();
    if own {
        app.begin_group(arg_str(args, "label").unwrap_or("Batch edit"), arg_str(args, "author").unwrap_or(""));
    }
    let snap = app.project.to_json();
    let r = run_calls(&calls, &mut |name, a| {
        // a batch must not get around the Assistant guard's Allow window
        if super::assistant_ctl::needs_approval(app, name, a) {
            return Err("needs the user's approval - call it on its own, not inside edit.batch".into());
        }
        app.run_tool_undoable(name, a)
    });
    match r {
        Ok(results) => {
            if own {
                app.end_group();
            }
            Ok(ToolOutcome::Done(json!({"ok": true, "results": results})))
        }
        Err((i, name, e)) => {
            app.run_rollback(snap);
            app.after_edit();
            if own {
                app.mcp_ctx.group = None; // back where it started: nothing to undo
            }
            Err(format!("call {i} ('{name}') failed: {e} - the whole batch was rolled back"))
        }
    }
}

// ---------------------------------------------------------------------------------------------------
// comments

/// A marker over In/Out, else over the selected clips, else a point at the playhead.
fn add_comment(app: &mut App, note: &str, author: &str) -> Id {
    let p = &app.project;
    let sel = app.selection.iter().filter_map(|&id| p.clip(id)).fold(None, |acc: Option<(f64, f64)>, c| {
        Some(acc.map_or((c.start, c.end()), |(a, b)| (a.min(c.start), b.max(c.end()))))
    });
    let (t, dur) = match (p.in_point, p.out_point) {
        (Some(a), Some(b)) if b > a => (a, b - a),
        _ => sel.map_or((app.playhead, 0.0), |(a, b)| (a, b - a)),
    };
    let id = app.project.add_marker(t, "Comment");
    if let Some(m) = app.project.marker_mut(id) {
        m.duration = dur;
        m.note = note.to_string();
        m.author = author.to_string();
    }
    id
}

/// The oldest marker the user (no author) added after `since` with an open, non-empty note that has
/// stopped changing for `COMMENT_SETTLE`: `({kind: "comment", ...}, id)`. `slot` is the candidate being
/// watched settle - one per consumer (`assistant.wait`, the Assistant's "Handle my comments").
pub(super) fn settled_comment(
    p: &Project,
    since: Id,
    now: Instant,
    slot: &mut Option<(Id, String, Instant)>,
) -> Option<(Value, Id)> {
    let (m, off) = p
        .markers
        .iter()
        .map(|m| (m, 0.0))
        .chain(p.all_clips().flat_map(|(_, c)| c.markers.iter().map(move |m| (m, c.start))))
        .filter(|(m, _)| m.id > since && m.author.is_empty() && !m.done && !m.note.trim().is_empty())
        .min_by_key(|(m, _)| m.id)?;
    let (id, note) = (m.id, m.note.clone());
    let value = json!({"kind": "comment", "marker_id": m.id, "t": off + m.t, "duration": m.duration,
                       "name": m.name, "note": m.note});
    match slot {
        Some((cid, cnote, at)) if *cid == id && *cnote == note => {
            if now.duration_since(*at) < COMMENT_SETTLE {
                return None;
            }
            *slot = None;
            Some((value, id))
        }
        _ => {
            *slot = Some((id, note, now));
            None
        }
    }
}

/// Answer every parked `assistant.wait` with an error (the app is closing).
pub(super) fn end_waits(app: &mut App, why: &str) {
    for w in app.mcp_ctx.waits.drain(..) {
        let _ = w.reply.send(Err(format!("assistant.wait: {why}")));
    }
}

/// Answer due `assistant.wait` calls: timeouts, then the oldest waiter gets the next Assistant-pane
/// request or a settled new comment.
fn poll_waits(app: &mut App, now: Instant) {
    let (due, waiting): (Vec<Wait>, Vec<Wait>) = std::mem::take(&mut app.mcp_ctx.waits).into_iter().partition(|w| now >= w.deadline);
    app.mcp_ctx.waits = waiting;
    for w in due {
        let _ = w.reply.send(Ok(json!({"timeout": true}).into()));
    }
    let Some(since) = app.mcp_ctx.waits.first().map(|w| w.since) else { return };
    let (request, text, comment) = if let Some(text) = app.assistant.pending.pop_front() {
        (json!({"kind": "request", "text": text.clone()}), Some(text), None)
    } else {
        let Some((v, id)) = settled_comment(&app.project, since, now, &mut app.mcp_ctx.comment) else { return };
        (v, None, Some(id))
    };
    let w = app.mcp_ctx.waits.remove(0);
    let reply = json!({"request": request, "context": context_json(app)});
    if w.reply.send(Ok(reply.into())).is_err() {
        // the client went away: keep the user's request (or comment) for the next wait
        if let Some(t) = text {
            app.assistant.pending.push_front(t);
        }
    } else if let Some(id) = comment {
        // delivered once, not to every waiter
        app.mcp_ctx.seen = Some(id);
        for w in &mut app.mcp_ctx.waits {
            w.since = w.since.max(id);
        }
    }
}

// ---------------------------------------------------------------------------------------------------
// registries

/// FRAME_HOOKS entry: close an idle / overtaken group, fade the changed-clip outline, answer
/// `assistant.wait`. Every wake goes through `App::animate_until`; with nothing open it schedules none.
pub(super) fn tick(app: &mut App, ctx: &egui::Context) {
    let now = Instant::now();
    let group = app.mcp_ctx.group.as_ref().map(|g| (g.hand_edited(&app.undo), g.last_call + GROUP_IDLE));
    if let Some((manual_edit, idle_at)) = group {
        if manual_edit || now >= idle_at {
            app.end_group();
        } else {
            app.animate_until(ctx, idle_at);
        }
    }
    if let Some(at) = app.mcp_ctx.flash_at {
        let a = 1.0 - now.duration_since(at).as_secs_f32() / FLASH_SECS;
        if a > 0.0 {
            app.timeline.flash_alpha = a;
            app.animate_until(ctx, now + Duration::from_millis(40));
        } else {
            app.mcp_ctx.flash_at = None;
            app.timeline.flash.clear();
            app.timeline.flash_alpha = 0.0;
        }
    }
    if !app.mcp_ctx.waits.is_empty() {
        poll_waits(app, now);
        // wake only when something can change without a frame of its own: the next deadline, and a
        // watched comment's note settling (a new request / marker arrives on a frame that runs anyway)
        let deadline = app.mcp_ctx.waits.iter().map(|w| w.deadline).min();
        let settle = app.mcp_ctx.comment.as_ref().map(|(_, _, at)| *at + COMMENT_SETTLE);
        if let Some(at) = deadline.into_iter().chain(settle).min() {
            app.animate_until(ctx, at);
        }
    }
}

/// ACT_HANDLERS entry.
pub(super) fn act(app: &mut App, a: Action) -> bool {
    match a {
        // the user takes over: close the agent's group first, so this steps over it as one entry
        Action::Undo | Action::Redo => {
            app.end_group();
            false
        }
        Action::AddComment => {
            app.push_undo();
            let id = add_comment(app, "", "");
            app.after_edit();
            app.fire_markers_added(&[id]);
            app.markers.selected = vec![id];
            app.markers.focus_note = Some(id);
            app.timeline.selected_marker = Some(id);
            app.surface(Pane::Markers);
            true
        }
        _ => false,
    }
}

pub const TOOLS: &[ToolDef] = &[
    ToolDef {
        name: "editor.context",
        desc: "What the user is looking at: playhead, in/out, selected clips (name, kind, track, times), \
               transitions, subtitle cues, markers (with notes), edit point, selected transcript words (text, \
               start, end), sequence and page. Call it first in every request.",
        args: &[],
        kind: ToolKind::Read,
        run: |app, _| Ok(ToolOutcome::Done(context_json(app))),
    },
    ToolDef {
        name: "edit.begin",
        desc: "Start one undo step for a request: until edit.end every edit joins it, and the user can undo it \
               all at once ('<author>: <label>' in History). Closes by itself after 30 s without calls.",
        args: &["label:string:true:what the request does, e.g. 'Tighten the intro'", "author:string:false:default Assistant"],
        kind: ToolKind::Ui,
        run: |app, args| {
            app.begin_group(req(arg_str(args, "label"), "label")?, arg_str(args, "author").unwrap_or(""));
            Ok(ToolOutcome::Done(json!({"ok": true})))
        },
    },
    ToolDef {
        name: "edit.end",
        desc: "End the undo step edit.begin started: one History entry if anything changed, the changed clips \
               glow, the user gets an Undo toast.",
        args: &[],
        kind: ToolKind::Ui,
        run: |app, _| {
            let open = app.mcp_ctx.group.is_some();
            let changed = app.end_group();
            Ok(ToolOutcome::Done(json!({"ok": open, "changed": changed})))
        },
    },
    ToolDef {
        name: "edit.batch",
        desc: "Run several tool calls in order, in one frame, as one undo step; returns each result. A failing \
               call stops the batch and rolls the whole batch back (the error names the call).",
        args: &[
            "calls:array:true:[{name, args}] in order",
            "label:string:false:undo label (default 'Batch edit')",
            "author:string:false:default Assistant",
        ],
        kind: ToolKind::Ui,
        run: batch,
    },
    ToolDef {
        name: "render.contact_sheet",
        desc: "Tile `count` frames evenly spread over [from, to] into one PNG image (left to right, top to \
               bottom; `times` lists each tile's time).",
        args: &[
            "from:number:true:",
            "to:number:true:",
            "count:integer:false:1..16, default 9 (stops early after ~5 s: `truncated`)",
            "width:integer:false:px per tile, default 320",
        ],
        kind: ToolKind::Read,
        run: contact_sheet,
    },
    ToolDef {
        name: "assistant.wait",
        desc: "Block until the user sends a request from the Assistant pane or adds a comment (a marker with a \
               note); returns {request, context}, or {timeout: true}. Loop on it for live mode.",
        args: &["timeout_s:number:false:default 300, max 1500"],
        kind: ToolKind::Job,
        run: |_, args| {
            let s = arg_f64(args, "timeout_s").unwrap_or(300.0).clamp(1.0, 1500.0);
            Ok(ToolOutcome::Wait(Instant::now() + Duration::from_secs_f64(s)))
        },
    },
    ToolDef {
        name: "markers.comment",
        desc: "Leave a comment (a marker with a note, author = you) over In/Out, else the selected clips, else \
               the playhead - or at t / duration when given. Same as the Add Comment action.",
        args: &[
            "note:string:true:",
            "author:string:false:default Assistant",
            "t:number:false:timeline seconds",
            "duration:number:false:with t",
        ],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let note = req(arg_str(args, "note"), "note")?.to_string();
            let id = add_comment(app, &note, arg_str(args, "author").unwrap_or("Assistant"));
            if let Some(t) = arg_f64(args, "t") {
                if let Some(m) = app.project.markers.iter_mut().find(|m| m.id == id) {
                    m.t = t.max(0.0);
                    m.duration = arg_f64(args, "duration").unwrap_or(0.0).max(0.0);
                }
                app.project.sort_markers();
            }
            app.fire_markers_added(&[id]);
            Ok(ToolOutcome::Done(json!({"ok": true, "id": id})))
        },
    },
];

#[cfg(test)]
mod tests {
    use super::super::mcp_exec::rollback_project;
    use super::*;

    fn group(label: &str, p: &Project, undo_len: usize) -> UndoGroup {
        UndoGroup { label: label.into(), before: p.to_json(), undo_len, top_at: None, last_call: Instant::now() }
    }

    /// N mutating calls inside a group push nothing of their own; closing it pushes ONE entry, labelled,
    /// holding the pre-group state. A group that changed nothing pushes nothing.
    #[test]
    fn group_makes_one_labelled_undo_entry() {
        let (mut undo, mut redo) = (Vec::new(), Vec::new());
        let mut p = Project::new();
        let g = group("Claude: Add beats", &p, 0);
        for i in 0..4 {
            let before = p.to_json();
            p.add_marker(i as f64, "beat");
            record_edit(&mut undo, &mut redo, true, before, &p.to_json());
        }
        assert!(undo.is_empty(), "calls inside a group push nothing");
        assert!(push_group_entry(&mut undo, &mut redo, &g, &p.to_json()));
        assert_eq!(undo.len(), 1);
        assert_eq!(undo[0].label, "Claude: Add beats");
        assert_eq!(undo[0].json, g.before, "undoing it restores the pre-group project");
        // outside a group every call is its own entry
        let before = p.to_json();
        p.add_marker(9.0, "x");
        record_edit(&mut undo, &mut redo, false, before, &p.to_json());
        assert_eq!(undo.len(), 2);
        // an empty group pushes nothing
        let idle = group("Claude: nothing", &p, undo.len());
        assert!(!push_group_entry(&mut undo, &mut redo, &idle, &p.to_json()));
        assert_eq!(undo.len(), 2);
    }

    /// A hand edit is seen even when the 200-entry cap keeps the stack length unchanged.
    #[test]
    fn hand_edit_seen_at_the_undo_cap() {
        let (mut undo, mut redo) = (Vec::new(), Vec::new());
        for i in 0..200 {
            push_undo_json(&mut undo, &mut redo, format!("{i}"));
        }
        let p = Project::new();
        let mut g = group("Claude: x", &p, undo.len());
        g.top_at = undo.last().map(|e| e.at);
        assert!(!g.hand_edited(&undo));
        undo.last_mut().unwrap().at += 1.0; // a new push at the cap: same length, new top entry
        assert_eq!(undo.len(), 200);
        assert!(g.hand_edited(&undo));
    }

    /// A manual edit while a group is open: the group's entry goes beneath the user's, so Ctrl+Z undoes
    /// the user's edit first, then the agent's whole group.
    #[test]
    fn group_entry_goes_beneath_a_manual_edit() {
        let (mut undo, mut redo) = (Vec::new(), Vec::new());
        let mut p = Project::new();
        let g = group("Claude: x", &p, 0);
        p.add_marker(1.0, "agent");
        let after_agent = p.to_json();
        push_undo_json(&mut undo, &mut redo, after_agent.clone()); // the user's own edit's snapshot
        p.add_marker(2.0, "user");
        assert!(push_group_entry(&mut undo, &mut redo, &g, &p.to_json()));
        assert_eq!(undo.iter().map(|e| e.json.as_str()).collect::<Vec<_>>(), vec![g.before.as_str(), after_agent.as_str()]);
    }

    /// A failing call stops the batch where it is (later calls never run), names itself, and the
    /// batch's rollback restores the pre-batch project exactly.
    #[test]
    fn batch_stops_and_rolls_back() {
        let mut p = Project::new();
        let snap = p.to_json();
        let calls = vec![
            json!({"name": "markers.add", "args": {"t": 1.0}}),
            json!({"name": "markers.add", "args": {"t": 2.0}}),
            json!({"name": "bad.call"}),
            json!({"name": "markers.add", "args": {"t": 3.0}}),
        ];
        let mut ran = 0;
        let r = run_calls(&calls, &mut |name, a| {
            ran += 1;
            if name == "bad.call" {
                return Err("boom".into());
            }
            p.add_marker(a["t"].as_f64().unwrap(), "m");
            Ok(json!({"ok": true}))
        });
        assert_eq!(r.unwrap_err(), (2, "bad.call".to_string(), "boom".to_string()));
        assert_eq!(ran, 3, "nothing after the failing call runs");
        assert_ne!(p.to_json(), snap);
        p = rollback_project(&snap).unwrap();
        assert_eq!(p.to_json(), snap);
        // nesting edit.* is refused before anything runs
        let r = run_calls(&[json!({"name": "edit.begin"})], &mut |_, _| Ok(json!(null)));
        assert_eq!(r.unwrap_err().0, 0);
        // success returns every result in order
        let r = run_calls(&calls[..2], &mut |name, _| Ok(json!(name))).unwrap();
        assert_eq!(r, vec![json!("markers.add"), json!("markers.add")]);
    }

    /// `markers.set` changes only the fields it is given.
    #[test]
    fn markers_set_patches_only_given_fields() {
        let mut p = Project::new();
        let id = p.add_marker(2.0, "Intro");
        let m = p.marker_mut(id).unwrap();
        super::super::tools_subtitles::apply_marker_patch(m, &json!({"note": "too long", "done": true}));
        let m = p.marker_mut(id).unwrap();
        assert_eq!((m.name.as_str(), m.note.as_str(), m.done, m.t), ("Intro", "too long", true, 2.0));
        super::super::tools_subtitles::apply_marker_patch(m, &json!({"author": "Claude", "duration": 1.5}));
        assert_eq!((m.author.as_str(), m.duration, m.done), ("Claude", 1.5, true));
        assert!(crate::mcp::tools::find("markers.set").is_some_and(|d| d.kind == ToolKind::Mutate));
    }

    /// `editor.context`'s shape: every key present, selections resolved against the project (unknown ids
    /// dropped), marker notes included.
    #[test]
    fn editor_context_shape() {
        let mut p = Project::new();
        p.in_point = Some(1.0);
        p.out_point = Some(4.0);
        let mid = p.add_marker(3.0, "Fix");
        p.marker_mut(mid).unwrap().note = "too loud".into();
        let v = View {
            playhead: 2.5,
            clips: &[999],
            transitions: &[],
            cues: &[],
            edit_point: None,
            markers: vec![mid],
            words: None,
            page: "Edit",
            mcp_port: 7337,
        };
        let c = context_of(&p, &v);
        for k in [
            "playhead", "in", "out", "duration", "fps", "page", "sequence", "selected_clips", "selected_transitions",
            "selected_cues", "edit_point", "selected_markers", "selected_words",
        ] {
            assert!(c.get(k).is_some(), "editor.context lacks '{k}'");
        }
        assert_eq!(c["playhead"], 2.5);
        assert_eq!((c["in"].as_f64(), c["out"].as_f64()), (Some(1.0), Some(4.0)));
        assert_eq!(c["selected_clips"], json!([]), "an unknown clip id is dropped");
        assert_eq!(c["selected_markers"][0]["note"], "too loud");
        assert!(c["sequence"].is_null() && c["selected_words"].is_null());
    }

    #[test]
    fn contact_sheet_tiles_in_reading_order() {
        let px = |r: u8, bgrx: bool| {
            let mut f = Frame::new(2, 1);
            f.rgba = vec![r, 0, 9, 255, r, 0, 9, 255];
            f.bgrx = bgrx;
            Arc::new(f)
        };
        let s = tile(&[px(1, false), px(2, false), px(3, true)], 2);
        assert_eq!((s.width, s.height), (4, 2));
        assert_eq!(s.rgba[0], 1);
        assert_eq!(s.rgba[8], 2, "second tile to the right");
        let row2 = 4 * 4;
        assert_eq!((s.rgba[row2], s.rgba[row2 + 2]), (9, 3), "third tile below, BGRX swizzled to RGBA");
        assert_eq!(s.rgba[row2 + 8 + 3], 255, "the empty cell is opaque black");
    }
}
