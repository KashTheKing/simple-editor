use crate::model::*;

impl Project {
    // ---------- planner ----------
    fn plan_find_in(items: &mut [PlanItem], id: Id) -> Option<&mut PlanItem> {
        for it in items {
            if it.id == id {
                return Some(it);
            }
            if let Some(f) = Self::plan_find_in(&mut it.children, id) {
                return Some(f);
            }
        }
        None
    }
    pub fn plan_item_mut(&mut self, id: Id) -> Option<&mut PlanItem> {
        Self::plan_find_in(&mut self.plan, id)
    }
    /// Add an item (under `parent` or at the top level); returns its id.
    pub fn plan_add(&mut self, parent: Option<Id>, title: impl Into<String>) -> Id {
        let id = self.new_id();
        let item = PlanItem { id, title: title.into(), ..Default::default() };
        match parent.and_then(|p| self.plan_item_mut(p)) {
            Some(p) => p.children.push(item),
            None => self.plan.push(item),
        }
        id
    }
    pub fn plan_remove(&mut self, id: Id) {
        fn rm(items: &mut Vec<PlanItem>, id: Id) {
            items.retain(|i| i.id != id);
            for i in items {
                rm(&mut i.children, id);
            }
        }
        rm(&mut self.plan, id);
    }
    /// All asset ids referenced by any moodboard - the per-task planner moodboards and the standalone
    /// Moodboard pane alike - never counted as "unused".
    pub fn plan_assets(&self) -> std::collections::HashSet<Id> {
        fn walk(items: &[PlanItem], out: &mut std::collections::HashSet<Id>) {
            for i in items {
                out.extend(i.assets.iter().copied());
                walk(&i.children, out);
            }
        }
        let mut s = std::collections::HashSet::new();
        walk(&self.plan, &mut s);
        s.extend(self.moodboard.iter().map(|m| m.asset));
        s
    }

    // ---------- notes ----------
    pub fn add_note(&mut self, title: impl Into<String>) -> Id {
        let id = self.new_id();
        self.notes.push(Note { id, title: title.into(), ..Default::default() });
        id
    }
    pub fn note_mut(&mut self, id: Id) -> Option<&mut Note> {
        self.notes.iter_mut().find(|n| n.id == id)
    }
    pub fn remove_note(&mut self, id: Id) {
        self.notes.retain(|n| n.id != id);
    }
}
