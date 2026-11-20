//! ---- ws:timeline-tabs ----
//! Several timelines open at once, as tabs in the Timeline's tab (`layout::Chrome::timelines`): the main
//! timeline plus any sequences opened. Only one is ever swapped into `Project.tracks` (`open_sequence`),
//! so a switch is the same cheap swap entering a nest always was; each tab's playhead, zoom, scroll,
//! selection and in / out wait in `TabsState::views` while it is in the background. The tab list is
//! `Project.timeline_tabs` (saved with the project, kept across undo / redo); closing a tab only hides it.

use super::tools_helpers::*;
use super::*;
use crate::mcp::tools::{ToolDef, ToolKind, ToolOutcome};
use crate::ui::layout::TabCmd;
use std::collections::HashMap;

/// What one background tab remembers.
#[derive(Clone, Default)]
pub(crate) struct TabView {
    playhead: f64,
    zoom: f32,
    scroll_x: f64,
    scroll_y: f32,
    selection: Vec<Id>,
    in_out: (Option<f64>, Option<f64>),
}

#[derive(Default)]
pub(crate) struct TabsState {
    /// Keyed by the tab (None = Main). A tab never seen yet opens at 0 and zoomed to fit.
    views: HashMap<Option<Id>, TabView>,
    /// Rename Sequence window: the sequence and the name field.
    rename: Option<(Id, Option<String>)>,
}

/// The tab's label: "Main", or the sequence's name.
pub(crate) fn tab_name(p: &Project, tab: Option<Id>) -> String {
    tab.and_then(|id| p.sequence(id)).map_or_else(|| "Main".to_string(), |s| s.name.clone())
}

fn capture(app: &App) -> TabView {
    TabView {
        playhead: app.playhead,
        zoom: app.timeline.zoom,
        scroll_x: app.timeline.scroll_x,
        scroll_y: app.timeline.scroll_y,
        selection: app.selection.clone(),
        in_out: (app.project.in_point, app.project.out_point),
    }
}

/// Bring back the active tab's view after a swap. `in_out` = also restore the remembered in / out of a
/// sequence (Main keeps its own in the stash; after an undo the snapshot's are the truth).
fn restore(app: &mut App, in_out: bool) {
    let tab = app.project.editing;
    let view = app.tabs.views.remove(&tab);
    if in_out && tab.is_some() {
        if let Some(v) = &view {
            (app.project.in_point, app.project.out_point) = v.in_out;
        }
    }
    app.player.set_project(&app.project);
    match view {
        Some(v) => {
            let p = &app.project;
            app.selection = v.selection.into_iter().filter(|&id| p.clip(id).is_some()).collect();
            app.seek(v.playhead);
            (app.timeline.zoom, app.timeline.scroll_x, app.timeline.scroll_y) = (v.zoom, v.scroll_x, v.scroll_y);
        }
        None => {
            app.selection.clear();
            app.seek(0.0);
            app.timeline.scroll_y = 0.0;
            app.timeline.zoom_to_fit(app.project.view_len(), app.timeline.lanes_rect.width().max(800.0));
        }
    }
}

/// Make `to` the active tab (opening a tab for it if there is none). A view change, not an edit: no undo
/// step, no dirty flag. False = no such sequence.
pub(crate) fn switch(app: &mut App, to: Option<Id>) -> bool {
    if to.is_some_and(|id| app.project.sequence(id).is_none()) {
        return false;
    }
    let mut tabs = app.project.timeline_tabs();
    if !tabs.contains(&to) {
        tabs.push(to);
    }
    if app.project.editing != to {
        app.player.pause();
        let from = app.project.editing;
        let view = capture(app);
        app.tabs.views.insert(from, view);
        match to {
            None => app.project.close_sequence(),
            Some(id) => drop(app.project.open_sequence(id)),
        }
        restore(app, true);
    }
    app.project.timeline_tabs = tabs;
    true
}

/// Hide a tab (the sequence stays). The only tab left can't be closed. Closing the active one moves to
/// its neighbour.
pub(crate) fn close(app: &mut App, tab: Option<Id>) -> bool {
    let mut tabs = app.project.timeline_tabs();
    let Some(i) = tabs.iter().position(|t| *t == tab).filter(|_| tabs.len() > 1) else { return false };
    tabs.remove(i);
    if app.project.editing == tab {
        switch(app, tabs[i.min(tabs.len() - 1)]);
    }
    app.project.timeline_tabs = tabs;
    app.tabs.views.remove(&tab);
    true
}

/// Keep only `tab`.
pub(crate) fn close_others(app: &mut App, tab: Option<Id>) -> bool {
    if !switch(app, tab) {
        return false;
    }
    app.project.timeline_tabs = vec![tab];
    app.tabs.views.clear();
    true
}

/// Drag-reorder: the tab at `from` moves to index `to` (of the list without it).
pub(crate) fn move_tab(app: &mut App, from: usize, to: usize) {
    let mut tabs = app.project.timeline_tabs();
    if from < tabs.len() {
        let t = tabs.remove(from);
        tabs.insert(to.min(tabs.len()), t);
        app.project.timeline_tabs = tabs;
    }
}

/// Ctrl+Tab / Ctrl+Shift+Tab.
pub(crate) fn cycle(app: &mut App, step: isize) {
    let tabs = app.project.timeline_tabs();
    let i = tabs.iter().position(|t| *t == app.project.editing).unwrap_or(0) as isize;
    let n = tabs.len() as isize;
    switch(app, tabs[(i + step).rem_euclid(n) as usize]);
}

/// The "+": a new empty sequence in the main timeline's format, opened in a tab of its own (one undo step).
pub(crate) fn new_tab(app: &mut App) -> Id {
    app.push_undo();
    let p = &app.project;
    let (w, h, fps) = p.main_stash.as_ref().map_or((p.width, p.height, p.fps), |s| (s.width, s.height, s.fps));
    let mut n = p.sequences.len() + 1;
    while p.sequences.iter().any(|s| s.name == format!("Sequence {n}")) {
        n += 1;
    }
    let id = app.project.new_sequence(format!("Sequence {n}"), w, h, fps);
    app.after_edit();
    switch(app, Some(id));
    id
}

fn rename(app: &mut App, id: Id, name: &str) -> bool {
    let name = name.trim();
    if name.is_empty() || app.project.sequence(id).is_none_or(|s| s.name == name) {
        return false;
    }
    app.push_undo();
    app.project.sequence_mut(id).unwrap().name = name.to_string();
    app.after_edit();
    true
}

/// What the tab strip asked for this frame (`layout::Shown::tabs`); indexes are into `timeline_tabs()`.
pub(crate) fn apply(app: &mut App, cmds: Vec<TabCmd>) {
    for cmd in cmds {
        let tabs = app.project.timeline_tabs();
        let at = |i: usize| tabs.get(i).copied();
        match cmd {
            TabCmd::Switch(i) => drop(at(i).map(|t| switch(app, t))),
            TabCmd::Close(i) => drop(at(i).map(|t| close(app, t))),
            TabCmd::CloseOthers(i) => drop(at(i).map(|t| close_others(app, t))),
            TabCmd::Move(from, to) => move_tab(app, from, to),
            TabCmd::New => drop(new_tab(app)),
            TabCmd::Rename(i) => {
                if let Some(Some(id)) = at(i) {
                    app.tabs.rename = Some((id, Some(tab_name(&app.project, Some(id)))));
                }
            }
        }
    }
}

/// Undo / redo restore a whole project: keep the tab list as it is now (tabs are a view), and when the
/// snapshot was taken on another timeline, go there so the change is in sight. Call before `self.project = p`.
pub(crate) fn before_restore(app: &App, p: &mut Project) -> (Option<Id>, TabView) {
    p.timeline_tabs = app.project.timeline_tabs();
    if !p.timeline_tabs.contains(&p.editing) {
        p.timeline_tabs.push(p.editing);
    }
    (app.project.editing, capture(app))
}

pub(crate) fn after_restore(app: &mut App, (was, view): (Option<Id>, TabView)) {
    if app.project.editing != was {
        app.tabs.views.insert(was, view);
        restore(app, false);
    }
}

/// A different project: its tabs start fresh.
pub(crate) fn project_changed(app: &mut App) {
    app.tabs = TabsState::default();
}

pub(super) fn act(app: &mut App, a: Action) -> bool {
    match a {
        Action::NextTimeline => cycle(app, 1),
        Action::PrevTimeline => cycle(app, -1),
        Action::NewTimeline => drop(new_tab(app)),
        Action::CloseTimeline => {
            if !close(app, app.project.editing) {
                app.toast("The only open timeline stays open");
            }
        }
        _ => return false,
    }
    true
}

pub(super) fn windows(app: &mut App, ctx: &egui::Context) {
    let Some((id, mut field)) = app.tabs.rename.take() else { return };
    if let Some(name) = App::name_window(ctx, "Rename Sequence", &mut field) {
        rename(app, id, &name);
    }
    if field.is_some() {
        app.tabs.rename = Some((id, field));
    }
}

fn list_json(app: &App) -> Value {
    let p = &app.project;
    let tabs: Vec<Value> = p
        .timeline_tabs()
        .into_iter()
        .map(|t| json!({"id": t, "name": tab_name(p, t), "active": t == p.editing}))
        .collect();
    json!({"tabs": tabs, "active": p.editing})
}

pub const TOOLS: &[ToolDef] = &[ToolDef {
    name: "timeline.tabs",
    desc: "The timelines open as tabs above the timeline (id null = the main timeline). op: list (default), \
           open (adds a tab for sequence `id` and shows it, or shows its tab), switch (same as open), close \
           (hides tab `id`; the sequence stays; the only tab can't close), new (a new empty sequence in a tab \
           of its own). Tabs are a view: no undo step, except new. Returns the tabs and the active one.",
    args: &[
        "op:string:false:list | open | switch | close | new",
        "id:integer:false:sequence id; omit for the main timeline",
    ],
    kind: ToolKind::Ui,
    run: |app, args| {
        let id = arg_u64(args, "id");
        match arg_str(args, "op").unwrap_or("list") {
            "list" => {}
            "open" | "switch" => {
                if !switch(app, id) {
                    return Err(format!("no sequence {}", id.unwrap_or_default()));
                }
            }
            "close" => {
                if !close(app, id) {
                    return Err("not an open tab, or the only one".into());
                }
            }
            "new" => drop(new_tab(app)),
            op => return Err(format!("unknown op '{op}'")),
        }
        Ok(ToolOutcome::Done(list_json(app)))
    },
}];

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> App {
        let scratch = std::env::temp_dir().join(format!("se-tabs-test-{}", std::process::id()));
        *crate::settings::TEST_DIR.lock().unwrap_or_else(|e| e.into_inner()) = Some(scratch);
        let app = App::build(&egui::Context::default(), None, None);
        *crate::settings::TEST_DIR.lock().unwrap_or_else(|e| e.into_inner()) = None;
        app
    }

    /// A project with one 10 s text clip on main and two sequences (A, B) with a clip each.
    fn app_with_seqs() -> (App, Id, Id) {
        let mut app = app();
        app.project.add_text_clip(0.0, 10.0);
        let a = app.project.new_sequence("A", 1920, 1080, 30.0);
        let b = app.project.new_sequence("B", 1920, 1080, 30.0);
        for id in [a, b] {
            app.project.open_sequence(id);
            app.project.add_text_clip(0.0, 20.0);
            app.project.close_sequence();
        }
        app.player.set_project(&app.project);
        (app, a, b)
    }

    #[test]
    fn opening_a_sequence_adds_a_tab_and_focuses_an_open_one() {
        let (mut app, a, b) = app_with_seqs();
        assert_eq!(app.project.timeline_tabs(), vec![None], "an old / new project: just Main");
        app.enter_sequence(a);
        assert_eq!(app.project.timeline_tabs(), vec![None, Some(a)]);
        app.enter_sequence(b);
        assert_eq!(app.project.timeline_tabs(), vec![None, Some(a), Some(b)]);
        app.enter_sequence(a);
        assert_eq!(
            app.project.timeline_tabs(),
            vec![None, Some(a), Some(b)],
            "an open tab is focused, not added again"
        );
        assert_eq!(app.project.editing, Some(a));
        // MCP: sequence.open / timeline.tabs go the same way
        let out = (TOOLS[0].run)(&mut app, &json!({"op": "switch"})).ok().unwrap();
        let ToolOutcome::Done(v) = out else { panic!() };
        assert_eq!(v["active"], Value::Null);
        assert_eq!(v["tabs"].as_array().unwrap().len(), 3);
        assert!(super::super::tools_context::context_json(&app)["timeline_tabs"]
            .as_array()
            .is_some_and(|t| t.len() == 3));
    }

    #[test]
    fn each_tab_keeps_its_own_view() {
        let (mut app, a, _) = app_with_seqs();
        let main_clip = app.project.tracks.iter().flat_map(|t| &t.clips).next().unwrap().id;
        app.seek(4.0);
        app.selection = vec![main_clip];
        app.project.in_point = Some(1.0);
        app.timeline.zoom = 33.0;
        assert!(switch(&mut app, Some(a)));
        assert_eq!((app.playhead, app.selection.len(), app.project.in_point), (0.0, 0, None), "a new tab starts fresh");
        let seq_clip = app.project.tracks.iter().flat_map(|t| &t.clips).next().unwrap().id;
        app.seek(12.0);
        app.selection = vec![seq_clip];
        app.project.out_point = Some(15.0);
        app.timeline.zoom = 7.0;
        switch(&mut app, None);
        assert_eq!((app.playhead, app.selection.clone(), app.project.in_point), (4.0, vec![main_clip], Some(1.0)));
        assert_eq!(app.timeline.zoom, 33.0);
        cycle(&mut app, 1); // Ctrl+Tab
        assert_eq!(app.project.editing, Some(a));
        assert_eq!((app.playhead, app.selection.clone(), app.project.out_point), (12.0, vec![seq_clip], Some(15.0)));
        assert_eq!(app.timeline.zoom, 7.0);
        cycle(&mut app, -1);
        assert_eq!(app.project.editing, None);
        cycle(&mut app, -1); // wraps
        assert_eq!(app.project.editing, Some(a));
    }

    #[test]
    fn close_hides_the_tab_never_the_sequence() {
        let (mut app, a, b) = app_with_seqs();
        app.enter_sequence(a);
        app.enter_sequence(b);
        switch(&mut app, Some(a));
        assert!(close(&mut app, Some(a)));
        assert_eq!(app.project.timeline_tabs(), vec![None, Some(b)]);
        assert_eq!(app.project.editing, Some(b), "the neighbour takes over");
        assert!(app.project.sequence(a).is_some_and(|s| s.duration() == 20.0), "the sequence and its clip stay");
        assert!(close(&mut app, None), "Main can close while another tab is open");
        assert_eq!(app.project.timeline_tabs(), vec![Some(b)]);
        assert!(!close(&mut app, Some(b)), "the only tab stays");
        // Back to Parent Timeline brings Main back
        app.act(Action::OpenParentSequence);
        assert_eq!((app.project.editing, app.project.timeline_tabs()), (None, vec![Some(b), None]));
        move_tab(&mut app, 1, 0);
        assert_eq!(app.project.timeline_tabs(), vec![None, Some(b)]);
        app.enter_sequence(a);
        assert!(close_others(&mut app, Some(b)));
        assert_eq!((app.project.editing, app.project.timeline_tabs()), (Some(b), vec![Some(b)]));
    }

    /// User report: switching to another timeline showed the start-up cards ("Simple Editor": Open, Import,
    /// formats). They asked "is the project empty?" of the timeline in front - an empty sequence tab. The same
    /// question gated "drop one file = open it as the project" and autosave. They ask the whole project now.
    #[test]
    fn an_empty_tab_is_not_an_empty_project() {
        let mut app = app();
        assert!(app.project.is_blank() && app.project.no_clips_anywhere(), "a new project: the home cards");
        app.project.add_text_clip(0.0, 10.0);
        new_tab(&mut app); // "+": an empty sequence in front, Main full behind it
        assert!(app.project.is_empty(), "the tab in front has no clips");
        assert!(!app.project.is_blank(), "but the project is not new: no home cards, no replacing it");
        assert!(!app.project.no_clips_anywhere(), "and autosave keeps running");
        // a project whose only timeline content is an empty sequence is still not a new project
        let mut app = super::tests::app();
        new_tab(&mut app);
        switch(&mut app, None);
        assert!(app.project.no_clips_anywhere() && !app.project.is_blank());
    }

    #[test]
    fn tabs_round_trip_through_save_and_load() {
        let (mut app, a, b) = app_with_seqs();
        app.enter_sequence(b);
        app.enter_sequence(a);
        move_tab(&mut app, 2, 1); // Main, B, A -> Main, A, B
        let json = app.project.to_json();
        let p = Project::from_json(&json).unwrap();
        assert_eq!(p.timeline_tabs(), vec![None, Some(a), Some(b)]);
        assert_eq!(p.editing, Some(a), "the active tab");
        // an old file has no list: just Main (plus the sequence it was saved in)
        let mut old: Value = serde_json::from_str(&json).unwrap();
        old.as_object_mut().unwrap().remove("timeline_tabs");
        let p = Project::from_json(&old.to_string()).unwrap();
        assert_eq!(p.timeline_tabs(), vec![None, Some(a)]);
        // nothing open but Main: nothing extra is written
        assert!(!Project::new().to_json().contains("timeline_tabs"));
        // a deleted sequence's tab goes
        let mut p = Project::from_json(&json).unwrap();
        p.close_sequence();
        p.sequences.retain(|s| s.id != b);
        assert_eq!(p.timeline_tabs(), vec![None, Some(a)]);
    }

    #[test]
    fn undo_across_tabs_undoes_where_the_edit_was() {
        let (mut app, a, _) = app_with_seqs();
        app.enter_sequence(a);
        app.push_undo();
        app.project.add_text_clip(30.0, 5.0); // an edit inside A
        app.after_edit();
        switch(&mut app, None);
        app.seek(3.0);
        let clips = |p: &Project, id: Id| p.sequence_tracks(id).unwrap().iter().map(|t| t.clips.len()).sum::<usize>();
        assert_eq!(clips(&app.project, a), 2);
        app.act(Action::Undo);
        assert_eq!(clips(&app.project, a), 1, "the clip added in A is gone");
        assert_eq!(app.project.editing, Some(a), "undo shows the timeline it changed");
        assert_eq!(app.project.timeline_tabs(), vec![None, Some(a)], "tabs are a view: undo keeps them");
        app.act(Action::Redo);
        assert_eq!(clips(&app.project, a), 2);
        switch(&mut app, None);
        assert_eq!(app.playhead, 3.0, "Main's view survived the trip");
        // "+" is one undo step; undoing it drops the sequence and its tab
        let n = app.project.sequences.len();
        let id = new_tab(&mut app);
        assert_eq!((app.project.editing, app.project.sequences.len()), (Some(id), n + 1));
        assert_eq!(
            app.project.sequence(id).map(|s| (s.width, s.height)),
            Some((app.project.main_stash.as_ref().unwrap().width, app.project.main_stash.as_ref().unwrap().height))
        );
        app.act(Action::Undo);
        assert_eq!(app.project.sequences.len(), n);
        assert!(!app.project.timeline_tabs().contains(&Some(id)));
        assert_eq!(app.project.editing, None);
    }
}
