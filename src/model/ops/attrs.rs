use crate::model::*;

impl Project {
    // ---------- copy / paste attributes ----------
    /// Snapshot of a clip to paste attributes from (Ctrl+Alt+C).
    pub fn copy_attributes(&self, clip: Id) -> Option<Clip> {
        self.clip(clip).cloned()
    }
    /// Apply the selected attributes of `src` onto `ids` (timing and media are never touched).
    /// Returns how many clips changed.
    pub fn paste_attributes(&mut self, src: &Clip, ids: &[Id], set: AttrSet) -> usize {
        let mut n = 0;
        for &id in ids {
            let Some(c) = self.clip_mut(id) else { continue };
            if c.id == src.id {
                continue;
            }
            if set.transform {
                c.x = src.x.clone();
                c.y = src.y.clone();
                c.scale = src.scale.clone();
                c.scale_x = src.scale_x.clone();
                c.scale_y = src.scale_y.clone();
                c.rotation = src.rotation.clone();
            }
            if set.opacity {
                c.opacity = src.opacity.clone();
            }
            if set.blend {
                c.blend = src.blend;
            }
            if set.effects {
                c.effects = src.effects.clone();
            }
            if set.graph {
                c.graph = src.graph.clone();
            }
            if set.mask {
                c.mask = src.mask.clone();
            }
            if set.speed {
                c.set_speed(src.speed);
                c.reverse = src.reverse;
                c.freeze = src.freeze;
            }
            if set.audio {
                c.volume = src.volume.clone();
                c.pan = src.pan.clone();
                c.fade_in = src.fade_in;
                c.fade_out = src.fade_out;
                c.bus = src.bus;
            }
            if (set.text_content || set.text_style) && src.text.is_some() {
                let s = src.text.as_ref().unwrap();
                if set.text_content && set.text_style {
                    c.text = Some(s.clone());
                } else if set.text_style {
                    // style only: keep the destination's own wording
                    let content = c.text.as_ref().map(|t| t.text.clone());
                    let dst = c.text.get_or_insert_with(Default::default);
                    *dst = s.clone();
                    if let Some(content) = content {
                        dst.text = content;
                    }
                    // the copied spans index the SOURCE's wording — clamp them to the destination's
                    // so no dangling range is saved (it could resurrect on a later text edit)
                    dst.clamp_spans();
                } else {
                    // content only: keep the destination's own style
                    c.text.get_or_insert_with(Default::default).text = s.text.clone();
                }
            }
            if set.shape {
                if let Some(sh) = &src.shape {
                    c.shape = Some(sh.clone());
                }
            }
            if set.label {
                c.label = src.label;
            }
            if set.markers {
                c.markers = src.markers.clone();
            }
            n += 1;
        }
        if n > 0 {
            self.tidy();
        }
        n
    }
}
