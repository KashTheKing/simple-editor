use crate::model::*;
use serde::{Deserialize, Serialize};

// ---------- live links (paths / expressions) ----------

/// Samples per second when baking an expression into keyframes.
const EXPR_RATE: f64 = 30.0;

/// Evaluate a Luau expression over the clip's duration into linear samples. `t` (clip-local seconds)
/// and `value` (the property's own keyframed/constant value at that `t`) are in scope; the script must
/// end with `return <number>`, same as a Luau function body — no implicit-expression wrapping.
pub(crate) fn bake_expr(src: &str, base: &Animated, dur: f64) -> Result<Vec<Keyframe>, String> {
    let lua = mlua::Lua::new();
    lua.sandbox(true).map_err(|e| e.to_string())?;
    let func = lua
        .load(format!("local t, value = ...\n{src}"))
        .set_name("expression")
        .into_function()
        .map_err(|e| e.to_string())?;
    // ponytail: fixed 30 Hz sampling capped at 4096 keys; adaptive sampling if someone links a 10-min clip
    let n = ((dur * EXPR_RATE).ceil() as usize).clamp(1, 4096) + 1;
    let mut keys = Vec::with_capacity(n);
    for i in 0..n {
        let t = dur * i as f64 / (n - 1) as f64;
        let v: f64 = func.call((t, base.base_at(t))).map_err(|e| e.to_string())?;
        if !v.is_finite() {
            return Err("expression returned a non-finite number".into());
        }
        keys.push(Keyframe { t, v, ease: Ease::Linear });
    }
    Ok(keys)
}

/// Change stamp for a property's live link: hash of the link and everything its bake reads.
pub(crate) fn link_rev(a: &Animated, dur: f64, paths: &[PathAsset]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    dur.to_bits().hash(&mut h);
    match &a.link {
        AnimLink::None => {}
        AnimLink::PathX(id) | AnimLink::PathY(id) => {
            matches!(a.link, AnimLink::PathX(_)).hash(&mut h);
            id.hash(&mut h);
            if let Some(p) = paths.iter().find(|p| p.id == *id) {
                for (x, y, t) in &p.points {
                    x.to_bits().hash(&mut h);
                    y.to_bits().hash(&mut h);
                    t.to_bits().hash(&mut h);
                }
            }
        }
        AnimLink::Expr(s) => {
            s.hash(&mut h);
            a.value.to_bits().hash(&mut h);
            for k in &a.keys {
                k.t.to_bits().hash(&mut h);
                k.v.to_bits().hash(&mut h);
                std::mem::discriminant(&k.ease).hash(&mut h);
                if let Ease::Bezier { x1, y1, x2, y2 } = k.ease {
                    for f in [x1, y1, x2, y2] {
                        f.to_bits().hash(&mut h);
                    }
                }
            }
        }
    }
    h.finish().max(1) // 0 is reserved for "never baked"
}
