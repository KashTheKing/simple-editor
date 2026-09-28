//! ---- ws:size-diet ----
//! Version-gate + the winpos window-rect debounce (FRAME_HOOK: `tick` - the sole call site that turns
//! `winpos::tick`'s returned wake `Instant` into `app.animate_until(..)`, so every NEW timed-repaint
//! request this PR adds stays behind that one sanctioned funnel), the non-blocking "What's New" window
//! (WINDOW_DRAWER: `window`), its `Action::WhatsNew` (ACT_HANDLER: `act`), and two MCP tools:
//! `help.changelog` (read-only) and `templates.save` (closes the capability gap left by deleting
//! `presets_ui.rs`'s "Save from selection" button).

use super::tools_helpers::*;
use super::*;
use crate::mcp::tools::{ToolDef, ToolKind, ToolOutcome};
use crate::ui::markdown;

/// Version-gate (opens the What's New window once per version bump) + the winpos window-rect debounce.
pub(super) fn tick(app: &mut App, ctx: &egui::Context) {
    let current = env!("CARGO_PKG_VERSION");
    if app.settings.last_seen_version != current {
        app.settings.last_seen_version = current.to_string();
        app.settings.save();
        app.whatsnew_open = true;
    }
    if let Some(at) = crate::winpos::tick(ctx, &mut app.settings, &mut app.winpos_pending) {
        app.animate_until(ctx, at);
    }
}

/// Non-blocking "What's New" window: CHANGELOG.md rendered through the new in-house markdown painter
/// (replaces `egui_commonmark`). Closable; opened by a version bump (`tick`, above) or `Action::WhatsNew`.
pub(super) fn window(app: &mut App, ctx: &egui::Context) {
    if !app.whatsnew_open {
        return;
    }
    let mut open = true;
    egui::Window::new("What's New").open(&mut open).default_width(480.0).default_height(420.0).show(ctx, |ui| {
        egui::ScrollArea::vertical().show(ui, |ui| {
            markdown::show(ui, include_str!("../../../CHANGELOG.md"), &app.palette);
        });
    });
    app.whatsnew_open = open;
}

pub(super) fn act(app: &mut App, a: Action) -> bool {
    match a {
        Action::WhatsNew => {
            app.whatsnew_open = true;
            true
        }
        _ => false,
    }
}

pub const TOOLS: &[ToolDef] = &[
    ToolDef {
        name: "help.changelog",
        desc: "Current app version and the full CHANGELOG.md text (what the What's New window shows).",
        args: &[],
        kind: ToolKind::Read,
        run: |_app, _args| {
            Ok(ToolOutcome::Done(json!({
                "version": env!("CARGO_PKG_VERSION"),
                "changelog": include_str!("../../../CHANGELOG.md"),
            })))
        },
    },
    ToolDef {
        name: "templates.save",
        desc: "Save the given (or currently selected) clips as a reusable effect chain / node graph / clip \
               template in settings.json (mirrors the deleted presets_ui.rs \"Save from selection\" button).",
        args: &["name:string:true:preset/template name", "clip_ids:array:false:defaults to current selection"],
        kind: ToolKind::Ui,
        run: |app, args| {
            let name = req(arg_str(args, "name"), "name")?;
            let ids = arg_ids(args, "clip_ids").unwrap_or_else(|| app.selection.clone());
            app.save_preset(name, &ids);
            Ok(ToolOutcome::Done(json!({"ok": true})))
        },
    },
];
