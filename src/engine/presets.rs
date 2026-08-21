//! Keyframe presets, motion presets and clip templates — capture from clips and apply back.
//! Curve presets store keys normalised to 0..1 of the clip length (or absolute seconds); applying can
//! stretch them to the target clip's duration ("scaled") or keep the saved timing ("exact").

use crate::model::{Animated, Asset, Clip, Keyframe, Project};
use crate::settings::{CurvePreset, MotionPreset, Template};

/// Snapshot a property's keys as a preset (normalised unless `absolute`). None if the property has no keys.
pub fn capture_curve(_name: &str, _anim: &Animated, _clip_duration: f64, _absolute: bool) -> Option<CurvePreset> {
    todo!("engine::presets::capture_curve")
}

/// Apply a curve preset to a property. `scaled` = stretch normalised times to `clip_duration`
/// (absolute presets are placed from 0 unchanged). Replaces existing keys.
pub fn apply_curve(_preset: &CurvePreset, _anim: &mut Animated, _clip_duration: f64, _scaled: bool) {
    todo!("engine::presets::apply_curve")
}

/// Built-in motion presets (slide in/out from each side, zoom in/out "Ken Burns", pop, fade in/out, spin).
pub fn builtin_motions() -> Vec<MotionPreset> {
    todo!("engine::presets::builtin_motions")
}

/// Capture every animated property of `clip` (incl. effect params as "<Effect>: <Param>").
pub fn capture_motion(_name: &str, _clip: &Clip) -> MotionPreset {
    todo!("engine::presets::capture_motion")
}

/// Apply a motion preset to a clip (properties it doesn't have are skipped; effect params create the
/// effect when missing). `scaled` as in `apply_curve`.
pub fn apply_motion(_preset: &MotionPreset, _clip: &mut Clip, _scaled: bool) {
    todo!("engine::presets::apply_motion")
}

/// Serialise a group of clips (+ the assets they use) into a Template (times relative to the earliest start).
pub fn capture_template(_name: &str, _project: &Project, _clip_ids: &[crate::model::Id]) -> Template {
    todo!("engine::presets::capture_template")
}

/// Decode a template into (clips, assets) ready for `Project::place_clips`. None on malformed JSON.
pub fn decode_template(_t: &Template) -> Option<(Vec<Clip>, Vec<Asset>)> {
    todo!("engine::presets::decode_template")
}

/// Convenience: keyframes of a preset scaled to `duration` (used by the curve editor preview).
pub fn scaled_keys(_keys: &[Keyframe], _absolute: bool, _duration: f64) -> Vec<Keyframe> {
    todo!("engine::presets::scaled_keys")
}
