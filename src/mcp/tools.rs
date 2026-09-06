//! The MCP tool catalogue — the single source of truth for tool names/arguments. `mcp/mod.rs` serves it
//! (`tools/list` with JSON schemas built from `ARGS`); `ui::app::App` executes calls by `name` with exactly
//! these arguments (unknown tool → error "unknown tool"). Keep both in sync with this table.
//!
//! Conventions: ids are the model's u64 ids; times are seconds; "property" names are the inspector labels
//! ("Position X", "Position Y", "Scale", "Rotation", "Opacity", "Volume", "Pan") or "<Effect>: <Param>";
//! every mutating tool pushes one undo step and returns `{"ok":true, ...}`; read tools return JSON.
//!
//! ---- ws:registries-schema-hooks ----
//! Wave-0b: the old 64-row `(name, desc, args)` tuple + the hand-kept `run_tool` match + the old
//! hand-kept mutating-tool name list are replaced by one `ToolDef` per tool (each still living next to its handler in a `ui::app::tools_*`
//! module) flattened here through `crate::ui::app::TOOL_TABLES` — see the registry protocol in
//! `plans/ui-overhaul/README.md`. `all()`/`find()`/`list_json()` below are now generic over that
//! registry instead of one local array, so `tools/list` and `editor.tools()` need no change when a
//! later workstream adds its own `tools_<ws>.rs` file and registers it in `TOOL_TABLES`.

use serde_json::{json, Value};

/// What a mutating call does to the undo stack — replaces the old hand-kept mutating-tool name list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolKind {
    /// Reads state; never pushes undo.
    Read,
    /// Edits the project; the caller snapshots before and pushes undo iff the JSON actually changed.
    Mutate,
    /// Starts a background job (export/convert) and replies when it finishes; never pushes undo here
    /// (the job's own edits, if any, are none — these tools write a file, not the project).
    Job,
    /// UI-only state (selection, dispatching an Action); never touches the project, never undoes.
    Ui,
}

/// What a tool's `run` fn returns: either the JSON reply directly, or a background job whose reply is
/// sent later (the caller polls `Progress` and replies on completion — see `App::handle_tool`).
pub enum ToolOutcome {
    Done(Value),
    Job(std::sync::Arc<crate::engine::export::Progress>, std::path::PathBuf),
}

/// One MCP tool: name/description/argument docs (unchanged shape from the old tuple) plus the kind
/// (drives undo policy) and the handler itself. Rows live beside their free-fn bodies in
/// `ui::app::tools_*.rs`, grouped exactly as the old `run_tool` dispatch chain grouped them.
pub struct ToolDef {
    pub name: &'static str,
    pub desc: &'static str,
    /// "name:type:required:description" per argument (same mini-format the old tuple used).
    pub args: &'static [&'static str],
    pub kind: ToolKind,
    pub run: fn(&mut crate::ui::app::App, &Value) -> Result<ToolOutcome, String>,
}

/// Every tool, in registry (wave-then-name) order.
pub fn all() -> impl Iterator<Item = &'static ToolDef> {
    crate::ui::app::TOOL_TABLES.iter().flat_map(|t| t.iter())
}

pub fn find(name: &str) -> Option<&'static ToolDef> {
    all().find(|t| t.name == name)
}

/// JSON schema for one tool's arguments ("name:type:required:description" docs).
pub fn input_schema(args: &[&str]) -> Value {
    let mut props = serde_json::Map::new();
    let mut required = Vec::new();
    for a in args {
        let mut it = a.splitn(4, ':');
        let name = it.next().unwrap_or("");
        let ty = it.next().unwrap_or("string");
        let req = it.next().unwrap_or("false");
        let desc = it.next().unwrap_or("");
        let mut p = serde_json::Map::new();
        p.insert("type".into(), Value::String(ty.into()));
        if !desc.is_empty() {
            p.insert("description".into(), Value::String(desc.into()));
        }
        props.insert(name.into(), Value::Object(p));
        if req == "true" {
            required.push(Value::String(name.into()));
        }
    }
    json!({"type": "object", "properties": props, "required": required})
}

/// Every effect kind the `clip.add_effect` handler accepts — listed from the model, because a
/// hand-written list in the catalogue goes stale the moment an effect is added.
fn effect_kinds() -> String {
    crate::model::EffectKind::ALL.iter().map(|k| k.name()).collect::<Vec<_>>().join(", ")
}

/// The `tools/list` payload: [{name, description, inputSchema}].
pub fn list_json() -> Value {
    Value::Array(
        all()
            .map(|t| {
                let desc = match t.name {
                    "clip.add_effect" => format!("{} Kinds: {}.", t.desc, effect_kinds()),
                    _ => t.desc.to_string(),
                };
                json!({"name": t.name, "description": desc, "inputSchema": input_schema(t.args)})
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_builder() {
        let s = input_schema(&["id:integer:true:the id", "path:string:false:"]);
        assert_eq!(s["type"], "object");
        assert_eq!(s["properties"]["id"]["type"], "integer");
        assert_eq!(s["properties"]["id"]["description"], "the id");
        assert_eq!(s["required"], json!(["id"]));
        assert!(s["properties"]["path"].get("description").is_none()); // empty description omitted
        assert_eq!(s["properties"]["path"]["type"], "string");
    }

    /// Every catalogue entry builds a valid object schema with known types (was: every row of the old
    /// TOOLS tuple; now every row of `all()`).
    #[test]
    fn every_arg_spec_parses() {
        for t in all() {
            let v = input_schema(t.args);
            assert_eq!(v["type"], "object", "{}", t.name);
            for (_, p) in v["properties"].as_object().unwrap() {
                let ty = p["type"].as_str().unwrap();
                assert!(
                    matches!(ty, "string" | "number" | "integer" | "boolean" | "array" | "object"),
                    "{}: bad type {ty}",
                    t.name
                );
            }
        }
        assert_eq!(list_json().as_array().unwrap().len(), all().count());
    }

    /// Tool names are unique and every one is namespaced ("prefix.verb") — a later workstream adding a
    /// duplicate/unnamespaced row is a build failure, not a runtime surprise.
    #[test]
    fn tool_names_unique_and_namespaced() {
        let names: Vec<&str> = all().map(|t| t.name).collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), names.len(), "duplicate tool name in TOOL_TABLES");
        for n in &names {
            assert!(n.contains('.'), "tool '{n}' is not namespaced as prefix.verb");
        }
    }

    /// Every effect the handler accepts is discoverable from the tool description.
    #[test]
    fn add_effect_lists_every_kind() {
        let list = list_json();
        let d = list.as_array().unwrap().iter().find(|t| t["name"] == "clip.add_effect").unwrap()["description"]
            .as_str()
            .unwrap()
            .to_string();
        for k in crate::model::EffectKind::ALL {
            assert!(d.contains(k.name()), "{} missing from the description", k.name());
        }
    }
}
