//! ---- ws:inspector-gallery ----
//! Effect-stack ops the UI (`effects_ui.rs`'s local drag-reorder) and MCP (`clip.reorder_effect`/
//! `clip.effects_bulk`) both call. Pure index operations on `Clip.effects` - neither adds a field to
//! `struct Effect` (not this workstream's to change; see `plans/ui-overhaul/issues/inspector-gallery.md`'s
//! risk table on why fold/drag identity uses a synthesized key instead of a real `Effect.id`).

use crate::model::*;

impl Project {
    /// Move `clip.effects[from]` to index `to` (a remove+insert, not a swap - every effect between the
    /// two indices shifts by one). `false` and no mutation when the clip is missing or either index is
    /// out of range.
    pub fn reorder_effect(&mut self, clip: Id, from: usize, to: usize) -> bool {
        let Some(c) = self.clip_mut(clip) else { return false };
        if from >= c.effects.len() || to >= c.effects.len() {
            return false;
        }
        if from != to {
            let e = c.effects.remove(from);
            c.effects.insert(to, e);
        }
        true
    }

    /// Set named params on every listed clip's effect at stack `index`, but only clips whose effect at
    /// that index shares the FIRST matching clip's `EffectKind` (a clip with a different kind there, or
    /// no effect at that index at all, is skipped rather than erroring the whole call). Returns how many
    /// clips were actually changed.
    pub fn bulk_set_effect_params(
        &mut self,
        clip_ids: &[Id],
        index: usize,
        params: &std::collections::HashMap<String, f64>,
    ) -> usize {
        let Some(kind) = clip_ids.iter().find_map(|&id| self.clip(id)?.effects.get(index).map(|e| e.kind)) else {
            return 0;
        };
        let mut n = 0;
        for &id in clip_ids {
            let Some(c) = self.clip_mut(id) else { continue };
            let Some(fx) = c.effects.get_mut(index) else { continue };
            if fx.kind != kind {
                continue;
            }
            let specs = fx.kind.params();
            for (pname, &v) in params {
                if let Some(i) = specs.iter().position(|s| s.name.eq_ignore_ascii_case(pname)) {
                    fx.params[i].value = v;
                }
            }
            n += 1;
        }
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn project_with_clip(fx: Vec<Effect>) -> (Project, Id) {
        let mut p = Project::new();
        let mut c = Clip::new(7, ClipKind::Video, "v", 0.0, 4.0);
        c.effects = fx;
        p.tracks[0].clips.push(c);
        (p, 7)
    }

    #[test]
    fn reorder_effect_moves_and_shifts_between() {
        let (mut p, id) = project_with_clip(vec![
            Effect::new(EffectKind::Blur),
            Effect::new(EffectKind::Vignette),
            Effect::new(EffectKind::Sharpen),
        ]);
        assert!(p.reorder_effect(id, 0, 2));
        let kinds: Vec<EffectKind> = p.clip(id).unwrap().effects.iter().map(|e| e.kind).collect();
        assert_eq!(kinds, [EffectKind::Vignette, EffectKind::Sharpen, EffectKind::Blur]);
    }

    #[test]
    fn reorder_effect_rejects_out_of_range() {
        let (mut p, id) = project_with_clip(vec![Effect::new(EffectKind::Blur)]);
        assert!(!p.reorder_effect(id, 0, 5));
        assert!(!p.reorder_effect(id, 5, 0));
        assert_eq!(p.clip(id).unwrap().effects.len(), 1);
        assert!(!p.reorder_effect(999, 0, 0), "no such clip");
    }

    #[test]
    fn bulk_set_effect_params_skips_kind_mismatch() {
        let (mut p, a) = project_with_clip(vec![Effect::new(EffectKind::Blur)]);
        let b = {
            let mut c = Clip::new(8, ClipKind::Video, "b", 4.0, 4.0);
            c.effects = vec![Effect::new(EffectKind::Blur)];
            let id = c.id;
            p.tracks[0].clips.push(c);
            id
        };
        let c_mismatch = {
            let mut c = Clip::new(9, ClipKind::Video, "c", 8.0, 4.0);
            c.effects = vec![Effect::new(EffectKind::Vignette)];
            let id = c.id;
            p.tracks[0].clips.push(c);
            id
        };
        let mut params = HashMap::new();
        params.insert("Radius".to_string(), 12.0);
        let n = p.bulk_set_effect_params(&[a, b, c_mismatch], 0, &params);
        assert_eq!(n, 2, "only the two Blur clips are touched");
        let radius_idx = EffectKind::Blur.params().iter().position(|s| s.name == "Radius").unwrap();
        assert_eq!(p.clip(a).unwrap().effects[0].params[radius_idx].value, 12.0);
        assert_eq!(p.clip(b).unwrap().effects[0].params[radius_idx].value, 12.0);
        // the mismatched clip's Vignette effect is untouched
        assert_eq!(p.clip(c_mismatch).unwrap().effects[0].kind, EffectKind::Vignette);
    }

    #[test]
    fn bulk_set_effect_params_no_effect_at_index_is_skipped() {
        let (mut p, a) = project_with_clip(vec![Effect::new(EffectKind::Blur)]);
        let short = {
            let c = Clip::new(8, ClipKind::Video, "b", 4.0, 4.0); // no effects at all
            let id = c.id;
            p.tracks[0].clips.push(c);
            id
        };
        let params = HashMap::new();
        let n = p.bulk_set_effect_params(&[a, short], 0, &params);
        assert_eq!(n, 1);
    }
}
