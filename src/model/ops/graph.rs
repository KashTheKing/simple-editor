use crate::model::*;

impl Project {
    // ---------- node graphs ----------
    /// Give the clip a node graph built from its current effect stack (idempotent).
    pub fn ensure_graph(&mut self, clip: Id) -> bool {
        let Some(c) = self.clip(clip) else { return false };
        if c.graph.is_some() {
            return true;
        }
        let effects = c.effects.clone();
        let mut next = || {
            self.next_id += 1;
            self.next_id
        };
        let g = NodeGraph::from_effects(&effects, &mut next);
        if let Some(c) = self.clip_mut(clip) {
            c.graph = Some(g);
        }
        true
    }
    /// Drop a clip's node graph back onto its linear effect stack; returns how many effects landed.
    /// Err when the graph is more than a chain (the caller toasts it) — nothing is touched then.
    /// Drop a clip's node graph entirely, keeping whatever of it can be expressed as an effect chain.
    /// A graph that will not linearise (a branch, a cycle) still goes: "unlink" means the graph is gone
    /// and the clip is back on its effect list, so leaving the nodes in place would be the one outcome
    /// the user did not ask for. Err only when there was no graph to begin with.
    pub fn unlink_graph(&mut self, clip: Id) -> Result<usize, String> {
        let g = self.clip(clip).and_then(|c| c.graph.as_ref()).ok_or("that clip has no node graph")?;
        let converted = g.to_effects().ok();
        let c = self.clip_mut(clip).ok_or("that clip is gone")?;
        let n = match converted {
            Some(effects) => {
                let n = effects.len();
                c.effects = effects;
                n
            }
            // nothing salvageable: the clip keeps the effects it already had
            None => 0,
        };
        c.graph = None;
        Ok(n)
    }
    /// Add a node to a clip's graph at editor position (x, y); returns its id.
    pub fn add_node(&mut self, clip: Id, kind: NodeKind, x: f32, y: f32) -> Option<Id> {
        self.ensure_graph(clip);
        let id = self.new_id();
        let c = self.clip_mut(clip)?;
        let g = c.graph.as_mut()?;
        g.nodes.push(Node { id, kind, x, y, enabled: true });
        Some(id)
    }
}
