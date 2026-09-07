//! ---- ws:forgiveness ----
//! First-frame startup hooks. Reuses the EXISTING `window_shown` first-frame gate in
//! `mod.rs::update` instead of adding a new booted-flag FRAME_HOOK — see the "Deliberate
//! simplifications" note in the PR body.

use super::*;

pub(crate) fn run(app: &mut App) {
    recovery::boot(app);
}
