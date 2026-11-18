//! ---- ws:mcp-permissions ----
//! Which MCP tools run without asking, which wait for the user, which never run. Every tool has a
//! default from its kind (`default_perm`); Settings ▸ Permissions stores the user's overrides
//! (`Settings.mcp_perms`, tool name -> state; `ui.action:<id>` for one Action). A chat's permission
//! mode (`Mode`) or the full-access flag can lift Ask, and only the flag lifts Block. Nothing an agent
//! calls writes the overrides: `permissions.request` only shows the question, the user's click answers it
//! (`ui::app::assistant_ctl`).

use super::tools::ToolKind;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Perm {
    /// Runs without asking.
    #[default]
    Allow,
    /// Waits for the user's Allow.
    Ask,
    /// Never runs (only full access gets past it).
    Block,
}

impl Perm {
    pub const ALL: [Perm; 3] = [Perm::Allow, Perm::Ask, Perm::Block];
    pub fn label(self) -> &'static str {
        match self {
            Perm::Allow => "Allow",
            Perm::Ask => "Ask",
            Perm::Block => "Block",
        }
    }
}

/// A chat's permission mode (and the one for MCP clients outside the Assistant).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// Every Ask waits for the user.
    Ask,
    /// Edits to the project run; what leaves it still asks. What the app did before permissions existed.
    #[default]
    AcceptEdits,
    /// Ask runs too; Block still never does.
    Bypass,
}

impl Mode {
    pub const ALL: [Mode; 3] = [Mode::Ask, Mode::AcceptEdits, Mode::Bypass];
    pub fn label(self) -> &'static str {
        match self {
            Mode::Ask => "Ask permissions",
            Mode::AcceptEdits => "Accept edits",
            Mode::Bypass => "Bypass permissions",
        }
    }
    pub fn hint(self) -> &'static str {
        match self {
            Mode::Ask => "Every tool set to Ask waits for your answer",
            Mode::AcceptEdits => "Edits to the project run; exports, opening files and settings still ask",
            Mode::Bypass => "Tools set to Ask run without asking. Blocked tools still never run.",
        }
    }
}

/// Tools that must answer at once whatever the settings say: the assistant's own permission tools and
/// the live-mode loop.
const ALWAYS: &[&str] = &["permissions.list", "permissions.request", "assistant.wait", "editor.context"];

/// A tool's state before the user changes it: reads Allow; edits, jobs and anything that leaves the
/// project Ask.
pub fn default_perm(tool: &str, kind: ToolKind, leaves: bool) -> Perm {
    if ALWAYS.contains(&tool) {
        return Perm::Allow;
    }
    match kind {
        _ if leaves => Perm::Ask,
        ToolKind::Read | ToolKind::Ui => Perm::Allow,
        ToolKind::Mutate | ToolKind::Job => Perm::Ask,
    }
}

/// `default_perm` for a tool by name, with no arguments (Settings ▸ Permissions, `permissions.list`).
pub fn default_for(tool: &str) -> Perm {
    let kind = super::tools::find(tool).map_or(ToolKind::Mutate, |t| t.kind);
    default_perm(tool, kind, super::tools::leaves_project(tool, &serde_json::Value::Null))
}

/// What one call gets. `key` is `ui.action:<id>` or the tool name (an override on the key wins over one on
/// the tool); `edit` = it changes the project but doesn't leave it (Accept edits lets it through).
pub fn resolve(overrides: &BTreeMap<String, Perm>, key: &str, tool: &str, default: Perm, mode: Mode, edit: bool, full: bool) -> Perm {
    if full {
        return Perm::Allow;
    }
    if ALWAYS.contains(&tool) {
        return Perm::Allow;
    }
    let p = overrides.get(key).or_else(|| overrides.get(tool)).copied().unwrap_or(default);
    match (p, mode) {
        (Perm::Ask, Mode::Bypass) => Perm::Allow,
        (Perm::Ask, Mode::AcceptEdits) if edit => Perm::Allow,
        _ => p,
    }
}

/// The area a tool is listed under: its name up to the first dot.
pub fn group(tool: &str) -> &str {
    tool.split('.').next().unwrap_or(tool)
}

/// Full access: every MCP call runs, nothing asks, nothing is blocked. `--mcp-full-access` (main sets
/// `SE_MCP_FULL_ACCESS`) or `SE_MCP_FULL_ACCESS=1`; on by default for debug builds and builds run from a
/// development worktree, off for the installed release exe. `SE_MCP_FULL_ACCESS=0` turns it off.
pub fn full_access() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        let dev = cfg!(debug_assertions) || std::env::current_exe().is_ok_and(|p| crate::winpos::is_dev_build(&p));
        full_access_from(std::env::var_os("SE_MCP_FULL_ACCESS").as_deref(), dev)
    })
}

pub fn full_access_from(env: Option<&std::ffi::OsStr>, dev: bool) -> bool {
    match env {
        Some(v) => v != "0",
        None => dev,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;

    #[test]
    fn the_flag_and_the_dev_build_default() {
        // a debug / worktree build is on unless told otherwise; the installed release exe is off unless asked
        assert!(full_access_from(None, true));
        assert!(!full_access_from(None, false), "the release exe defaults off");
        assert!(full_access_from(Some(OsStr::new("1")), false), "--mcp-full-access on the release exe");
        assert!(!full_access_from(Some(OsStr::new("0")), true), "SE_MCP_FULL_ACCESS=0 on a dev build");
        // the installed exe is not a worktree build
        assert!(!crate::winpos::is_dev_build(std::path::Path::new("D:/Projects/simple-editor/target/release/simple-editor.exe")));
    }

    #[test]
    fn defaults_follow_the_tool_kind() {
        assert_eq!(default_perm("timeline.list", ToolKind::Read, false), Perm::Allow);
        assert_eq!(default_perm("selection.set", ToolKind::Ui, false), Perm::Allow);
        assert_eq!(default_perm("timeline.split", ToolKind::Mutate, false), Perm::Ask);
        assert_eq!(default_perm("export.video", ToolKind::Job, true), Perm::Ask);
        assert_eq!(default_perm("settings.set", ToolKind::Ui, true), Perm::Ask);
        assert_eq!(default_perm("assistant.wait", ToolKind::Job, false), Perm::Allow);
    }

    #[test]
    fn allow_ask_block_resolution() {
        let mut o = BTreeMap::new();
        let r = |o: &BTreeMap<String, Perm>, key: &str, tool: &str, d, m, edit| resolve(o, key, tool, d, m, edit, false);
        // no override: the default
        assert_eq!(r(&o, "media.import", "media.import", Perm::Ask, Mode::Ask, true), Perm::Ask);
        // Accept edits lets an edit through, not something that leaves the project
        assert_eq!(r(&o, "media.import", "media.import", Perm::Ask, Mode::AcceptEdits, true), Perm::Allow);
        assert_eq!(r(&o, "export.video", "export.video", Perm::Ask, Mode::AcceptEdits, false), Perm::Ask);
        // overrides
        o.insert("media.import".into(), Perm::Allow);
        o.insert("timeline.delete".into(), Perm::Block);
        assert_eq!(r(&o, "media.import", "media.import", Perm::Ask, Mode::Ask, true), Perm::Allow);
        assert_eq!(r(&o, "timeline.delete", "timeline.delete", Perm::Ask, Mode::AcceptEdits, true), Perm::Block);
        // a key override beats the tool's
        o.insert("ui.action".into(), Perm::Allow);
        o.insert("ui.action:export".into(), Perm::Block);
        assert_eq!(r(&o, "ui.action:export", "ui.action", Perm::Ask, Mode::Ask, false), Perm::Block);
        assert_eq!(r(&o, "ui.action:undo", "ui.action", Perm::Allow, Mode::Ask, false), Perm::Allow);
        // the assistant's own tools always answer
        o.insert("permissions.request".into(), Perm::Block);
        assert_eq!(r(&o, "permissions.request", "permissions.request", Perm::Allow, Mode::Ask, false), Perm::Allow);
    }

    #[test]
    fn bypass_skips_ask_but_not_block() {
        let mut o = BTreeMap::new();
        o.insert("export.video".into(), Perm::Block);
        assert_eq!(resolve(&o, "project.open", "project.open", Perm::Ask, Mode::Bypass, false, false), Perm::Allow);
        assert_eq!(resolve(&o, "export.video", "export.video", Perm::Ask, Mode::Bypass, false, false), Perm::Block);
        // only full access gets past Block
        assert_eq!(resolve(&o, "export.video", "export.video", Perm::Ask, Mode::Ask, false, true), Perm::Allow);
    }
}
