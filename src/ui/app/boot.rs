//! ---- ws:forgiveness ----
//! First-frame startup hooks. Reuses the EXISTING `window_shown` first-frame gate in
//! `mod.rs::update` instead of adding a new booted-flag FRAME_HOOK - see the "Deliberate
//! simplifications" note in the PR body.
//!
//! ---- ws:layout-modes-onboarding ----
//! Also the first-run decision: a fresh install (nothing onboarded, no file argument, not a
//! `--screenshot` run) arms the welcome wizard instead of the silent Explorer context-menu install
//! `App::new` used to do; an already-onboarded user who left the box ticked keeps the old re-point
//! behaviour under the exact same guard. Two dev-only env hooks, in the spirit of `SE_SCREENSHOT_DELAY`:
//! `SE_FIRST_RUN=1` forces the wizard (even with `--screenshot`, for a visual check) and
//! `SE_LAYOUT=<workspace>` applies a workspace at boot without persisting the name.

use super::*;
use crate::ui::onboarding::Onboarding;

pub(crate) fn run(app: &mut App) {
    recovery::boot(app);
    // ---- ws:layout-modes-onboarding ----
    if let Ok(name) = std::env::var("SE_LAYOUT") {
        let mut chars = name.trim().chars();
        let cap = chars.next().map(|c| c.to_uppercase().collect::<String>() + &chars.as_str().to_ascii_lowercase());
        if let Some(make) = cap.as_deref().and_then(crate::ui::layout::workspace_layout) {
            app.layout.switch_to(make());
            app.layout_dirty = true;
        }
    }
    first_run(app);
}

/// Arm the wizard, or apply the consented context-menu re-point - never both, never silently.
fn first_run(app: &mut App) {
    let opened = app.project_path.is_some() || !app.project.is_empty() || !app.project.assets.is_empty();
    let forced = std::env::var("SE_FIRST_RUN").is_ok_and(|v| v == "1");
    if should_onboard(app.settings.onboarded, opened, app.screenshot.is_some(), forced) {
        app.onboarding = Some(Onboarding::new(&app.settings));
        return;
    }
    // The guard App::new applied on every launch before this wave, verbatim (settings.context_menu,
    // no --screenshot, release build, not already pointing at this exe) - now additionally behind
    // settings.onboarded, i.e. the user has been through the wizard (or dismissed it) and the flag
    // records their answer, so no launch ever writes the registry without consent.
    if app.settings.onboarded
        && app.settings.context_menu
        && app.screenshot.is_none()
        && !cfg!(debug_assertions)
        && !crate::contextmenu::is_installed()
    {
        let _ = crate::contextmenu::install();
    }
}

/// Pure decision behind `first_run`: the wizard shows once - never when a file was passed on the
/// command line (an Explorer "Open with" launch is a player, not a first run), never for a
/// `--screenshot` run - unless `SE_FIRST_RUN=1` forces it for a visual check.
pub(super) fn should_onboard(onboarded: bool, opened: bool, screenshot: bool, forced: bool) -> bool {
    forced || (!onboarded && !opened && !screenshot)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `App::new` needs a real `eframe::CreationContext`, so this pins the pure decision the
    /// first-frame hook makes from `App::new`'s `open` / `screenshot` arguments.
    #[test]
    fn onboarding_skipped_for_screenshot_and_open() {
        assert!(should_onboard(false, false, false, false), "fresh install, plain launch: wizard");
        assert!(!should_onboard(false, true, false, false), "a file argument (open=Some): no wizard");
        assert!(!should_onboard(false, false, true, false), "--screenshot: no wizard");
        assert!(!should_onboard(false, true, true, false));
        assert!(!should_onboard(true, false, false, false), "already onboarded (or dismissed): never again");
        // the dev-only override for screenshot verification
        assert!(should_onboard(true, false, true, true));
    }
}
