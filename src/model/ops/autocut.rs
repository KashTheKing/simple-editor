use crate::model::*;

impl Project {
    // ---------- auto-cut ----------
    /// Split the clips (+ linked) at every time in `cuts`, then delete the pieces lying inside any of the
    /// `remove` ranges (timeline times, half-open), optionally closing the gaps (ripple). Returns the number
    /// of pieces removed.
    pub fn auto_cut(&mut self, ids: &[Id], cuts: &[f64], remove: &[(f64, f64)], ripple: bool) -> usize {
        let ids = self.expand_links(ids);
        let mut group = ids.clone();
        for &t in cuts {
            let new = self.split_at(t, Some(&group));
            group.extend(new);
        }
        let victims: Vec<Id> = group
            .iter()
            .filter_map(|&id| self.clip(id))
            .filter(|c| {
                let mid = c.start + c.duration / 2.0;
                remove.iter().any(|&(a, b)| mid >= a && mid < b)
            })
            .map(|c| c.id)
            .collect();
        let n = victims.len();
        if n > 0 {
            self.delete_clips(&victims, ripple);
        }
        n
    }
}
