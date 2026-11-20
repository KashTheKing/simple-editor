//! ---- feat/styles-context ----
//! Project Context edits (`model::context` holds the text work). Each is one undo step for its caller.

use crate::model::{context, Project};

impl Project {
    /// Replace the Project Context document. Returns whether it changed.
    pub fn set_context(&mut self, text: &str) -> bool {
        let text = text.replace("\r\n", "\n");
        if self.context == text {
            return false;
        }
        self.context = text;
        true
    }

    /// Replace one section's body ("About", "Rules", "Style guide", "Links"; added when missing).
    pub fn set_context_section(&mut self, section: &str, body: &str) -> bool {
        let base = if self.context.trim().is_empty() { context::template() } else { self.context.clone() };
        let next = context::set_section(&base, section, &body.replace("\r\n", "\n"));
        self.set_context(&next)
    }

    /// Write an editing style into the document: `guide` at the top of Style guide, `rules` at the top of
    /// Rules, replacing any style applied before. Returns whether it changed.
    pub fn apply_context_style(&mut self, name: &str, guide: &str, rules: &str) -> bool {
        let next = context::apply_style(&self.context, name, guide, rules);
        self.set_context(&next)
    }

    /// Take the applied style back out (what the user wrote stays). Returns whether there was one.
    pub fn remove_context_style(&mut self) -> bool {
        let next = context::remove_style(&self.context);
        self.set_context(&next)
    }
}
