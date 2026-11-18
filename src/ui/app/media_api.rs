//! ---- ws:assistant-luau ----
//! The library as data, for agents and scripts: `media.list` with filters / fields / paging,
//! `media.summary` (counts and the bin tree in a few KB), `media.bulk_set` (one call files, tags or labels
//! many assets) and `library.bins` (create / rename / move / delete bins). The pure halves take a
//! `&Project` / `&mut Project` so tests need no `App`; the `scripts.eval` Luau API (`script_ctl`) is
//! built on these same tools. Nothing here touches the disk except reading a file's size when the `size`
//! field is asked for, and listing a linked folder (`linked_list`, read only, inside a linked root).

use super::tools_helpers::*;
use super::*;
use crate::mcp::tools::{ToolDef, ToolKind, ToolOutcome};
use crate::model::{Asset, Track};
use std::collections::{BTreeMap, HashMap, HashSet};

/// `media.list` page size when none is given, and the most one call returns.
pub(super) const LIST_DEFAULT: usize = 100;
pub(super) const LIST_MAX: usize = 5000;
/// Fields `media.list` returns when `fields` is not given (paged mode).
const COMPACT: &[&str] = &["id", "name", "kind", "folder", "duration", "used", "tags", "label"];
/// Every field `media.list` knows (`fields: "all"`).
pub(super) const ALL_FIELDS: &[&str] = &[
    "id", "name", "path", "ext", "kind", "duration", "width", "height", "fps", "codec", "size", "audio_streams",
    "tags", "label", "folder", "description", "used", "uses", "parent", "range", "proxy",
];
/// Above this many folders, `media.summary` lists the biggest and says how many it left out.
const SUMMARY_FOLDERS: usize = 120;

/// "video" / "audio" / "image" / "html" (an HTML layer is an Image asset with a source).
pub(super) fn kind_name(a: &Asset) -> &'static str {
    if a.html.is_some() {
        return "html";
    }
    match a.kind {
        ClipKind::Video => "video",
        ClipKind::Audio => "audio",
        ClipKind::Image => "image",
        ClipKind::Text => "text",
        ClipKind::Sequence => "sequence",
        ClipKind::Shape => "shape",
        ClipKind::Adjustment => "adjustment",
        ClipKind::Browser => "browser",
    }
}

/// The library's display name: the user's rename, else the file name (an HTML layer's title).
pub(super) fn display_name(p: &Project, a: &Asset) -> String {
    p.asset_names.get(&a.id).cloned().unwrap_or_else(|| a.name())
}

fn ext_of(path: &str) -> String {
    std::path::Path::new(path).extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default()
}

/// `folder` is `bin` or nested under it ("" = the whole library when `nested`, else only the root).
fn in_bin(folder: &str, bin: &str, nested: bool) -> bool {
    let bin = bin.trim().trim_matches('/');
    if bin.is_empty() {
        return nested || folder.is_empty();
    }
    folder == bin || (nested && folder.len() > bin.len() && folder.starts_with(bin) && folder.as_bytes()[bin.len()] == b'/')
}

/// A parsed asset filter (the shape `media.list`, `media.bulk_set {filter}` and the Luau API share).
#[derive(Default, Debug)]
pub(super) struct Filter {
    ids: Option<HashSet<Id>>,
    kinds: Vec<String>,
    folder: Option<String>,
    nested: bool,
    used: Option<bool>,
    query: Vec<String>,
    tags: Vec<String>,
    label: Option<u8>,
    exts: Vec<String>,
    untagged: bool,
}

fn str_list(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::String(s)) => s.split(',').map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty()).collect(),
        Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).map(|s| s.trim().to_lowercase()).collect(),
        _ => Vec::new(),
    }
}

impl Filter {
    pub(super) fn parse(args: &Value) -> Filter {
        Filter {
            ids: arg_ids(args, "ids").map(|v| v.into_iter().collect()),
            kinds: str_list(args.get("kind")),
            folder: arg_str(args, "folder").map(str::to_string),
            nested: arg_bool(args, "nested").unwrap_or(true),
            used: arg_bool(args, "used"),
            query: arg_str(args, "query").unwrap_or("").split_whitespace().map(str::to_lowercase).collect(),
            tags: str_list(args.get("tags")),
            label: arg_u64(args, "label").map(|l| l.min(8) as u8),
            exts: str_list(args.get("ext")).into_iter().map(|e| e.trim_start_matches('.').to_string()).collect(),
            untagged: arg_bool(args, "untagged").unwrap_or(false),
        }
    }

    pub(super) fn matches(&self, p: &Project, a: &Asset, used: &HashSet<Id>) -> bool {
        if self.ids.as_ref().is_some_and(|ids| !ids.contains(&a.id)) {
            return false;
        }
        if !self.kinds.is_empty() {
            let k = kind_name(a);
            // "image" also finds HTML layers (they are pictures in the library)
            if !self.kinds.iter().any(|w| w == k || (w == "image" && k == "html")) {
                return false;
            }
        }
        if self.folder.as_deref().is_some_and(|f| !in_bin(&a.folder, f, self.nested)) {
            return false;
        }
        if self.used.is_some_and(|u| u != used.contains(&a.id)) {
            return false;
        }
        if self.label.is_some_and(|l| l != a.label) {
            return false;
        }
        if self.untagged && !a.tags.is_empty() {
            return false;
        }
        if !self.tags.is_empty() && !self.tags.iter().all(|t| a.tags.iter().any(|x| x.eq_ignore_ascii_case(t))) {
            return false;
        }
        if !self.exts.is_empty() && !self.exts.contains(&ext_of(&a.path)) {
            return false;
        }
        if !self.query.is_empty() {
            let hay = format!("{} {} {} {}", display_name(p, a), a.path, a.tags.join(" "), a.description).to_lowercase();
            if !self.query.iter().all(|w| hay.contains(w.as_str())) {
                return false;
            }
        }
        true
    }
}

/// How many clips (main timeline, stash, sequences) use each asset.
fn use_counts(p: &Project) -> HashMap<Id, usize> {
    let mut n = HashMap::new();
    let mut add = |tracks: &[Track]| {
        for c in tracks.iter().flat_map(|t| t.clips.iter()) {
            if c.uses_asset() {
                *n.entry(c.asset).or_insert(0) += 1;
            }
        }
    };
    add(&p.tracks);
    if let Some(st) = &p.main_stash {
        add(&st.tracks);
    }
    for s in &p.sequences {
        add(&s.tracks);
    }
    n
}

/// One asset as JSON with just `fields`.
fn asset_json(p: &Project, a: &Asset, fields: &[&str], uses: &HashMap<Id, usize>) -> Value {
    let mut m = serde_json::Map::new();
    for f in fields {
        let v = match *f {
            "id" => json!(a.id),
            "name" => json!(display_name(p, a)),
            "path" => json!(a.path),
            "ext" => json!(ext_of(&a.path)),
            "kind" => json!(kind_name(a)),
            "duration" => json!((a.duration * 1000.0).round() / 1000.0),
            "width" => json!(a.width),
            "height" => json!(a.height),
            "fps" => json!(a.fps),
            "codec" => json!(a.codec),
            // the file's size on disk (a metadata read of the asset's own file - no content is read)
            "size" => json!(if a.html.is_some() { 0 } else { std::fs::metadata(&a.path).map(|m| m.len()).unwrap_or(0) }),
            "audio_streams" => json!(a.audio_streams.len()),
            "tags" => json!(a.tags),
            "label" => json!(a.label),
            "folder" => json!(a.folder),
            "description" => json!(a.description),
            "used" => json!(uses.contains_key(&a.id)),
            "uses" => json!(uses.get(&a.id).copied().unwrap_or(0)),
            "parent" => json!(a.parent),
            "range" => json!(a.range),
            "proxy" => json!(match a.proxy {
                None => "auto",
                Some(true) => "on",
                Some(false) => "off",
            }),
            _ => continue,
        };
        m.insert((*f).to_string(), v);
    }
    Value::Object(m)
}

/// The `fields` argument: a list, a comma string, "all", or nothing (`default`).
fn parse_fields(args: &Value, default: &[&'static str]) -> Result<Vec<&'static str>, String> {
    let asked = match args.get("fields") {
        Some(Value::String(s)) if s.trim() == "all" => return Ok(ALL_FIELDS.to_vec()),
        v @ Some(_) => str_list(v),
        None => return Ok(default.to_vec()),
    };
    if asked.iter().any(|f| f == "all") {
        return Ok(ALL_FIELDS.to_vec());
    }
    let mut out = Vec::new();
    for f in asked {
        let known = ALL_FIELDS.iter().find(|k| **k == f).ok_or_else(|| format!("unknown field '{f}' (fields: {})", ALL_FIELDS.join(", ")))?;
        out.push(*known);
    }
    if !out.contains(&"id") {
        out.insert(0, "id");
    }
    Ok(out)
}

/// `sort`: a field name, "-name" for descending. Default: the library's order.
fn sort_assets(p: &Project, list: &mut [&Asset], key: &str, uses: &HashMap<Id, usize>) -> Result<(), String> {
    let (key, desc) = match key.strip_prefix('-') {
        Some(k) => (k, true),
        None => (key, false),
    };
    use std::cmp::Ordering;
    type Cmp<'a> = Box<dyn Fn(&&Asset, &&Asset) -> Ordering + 'a>;
    let cmp: Cmp = match key {
        "" | "id" => Box::new(|a: &&Asset, b: &&Asset| a.id.cmp(&b.id)),
        "name" => Box::new(|a: &&Asset, b: &&Asset| {
            crate::engine::folder_import::natural_cmp(&display_name(p, a).to_lowercase(), &display_name(p, b).to_lowercase())
        }),
        "duration" => Box::new(|a: &&Asset, b: &&Asset| a.duration.total_cmp(&b.duration)),
        "kind" => Box::new(|a: &&Asset, b: &&Asset| kind_name(a).cmp(kind_name(b))),
        "folder" => Box::new(|a: &&Asset, b: &&Asset| a.folder.cmp(&b.folder)),
        "ext" => Box::new(|a: &&Asset, b: &&Asset| ext_of(&a.path).cmp(&ext_of(&b.path))),
        "uses" => Box::new(|a: &&Asset, b: &&Asset| uses.get(&a.id).cmp(&uses.get(&b.id))),
        "label" => Box::new(|a: &&Asset, b: &&Asset| a.label.cmp(&b.label)),
        "size" => Box::new(|a: &&Asset, b: &&Asset| {
            let s = |x: &Asset| std::fs::metadata(&x.path).map(|m| m.len()).unwrap_or(0);
            s(a).cmp(&s(b))
        }),
        k => return Err(format!("sort: unknown field '{k}' (id, name, duration, kind, folder, ext, uses, label, size)")),
    };
    list.sort_by(|a, b| if desc { cmp(b, a) } else { cmp(a, b) });
    Ok(())
}

/// `media.list`. With no arguments on a library of at most `LIST_DEFAULT` assets: the plain array every
/// field-ful row it always returned (scripts and skills read it that way). Otherwise a page:
/// `{total, offset, count, next_offset, assets, note?}` - `next_offset` is null on the last page.
pub(super) fn list(p: &Project, args: &Value) -> Result<Value, String> {
    let legacy = args.as_object().is_none_or(|o| o.is_empty()) && p.assets.len() <= LIST_DEFAULT;
    let used_set: HashSet<Id> = p.used_assets();
    let uses = use_counts(p);
    let filter = Filter::parse(args);
    let mut hits: Vec<&Asset> = p.assets.iter().filter(|a| filter.matches(p, a, &used_set)).collect();
    if let Some(key) = arg_str(args, "sort") {
        sort_assets(p, &mut hits, key, &uses)?;
    }
    if legacy {
        let fields = ["id", "name", "path", "kind", "duration", "width", "height", "tags", "label", "folder", "description", "used"];
        return Ok(Value::Array(hits.iter().map(|a| asset_json(p, a, &fields, &uses)).collect()));
    }
    let fields = parse_fields(args, COMPACT)?;
    let total = hits.len();
    let offset = (arg_u64(args, "offset").unwrap_or(0) as usize).min(total);
    let limit = (arg_u64(args, "limit").map(|l| l as usize).unwrap_or(LIST_DEFAULT)).clamp(1, LIST_MAX);
    let page: Vec<Value> = hits[offset..].iter().take(limit).map(|a| asset_json(p, a, &fields, &uses)).collect();
    let end = offset + page.len();
    let mut out = json!({
        "total": total, "offset": offset, "count": page.len(),
        "next_offset": if end < total { json!(end) } else { Value::Null },
        "assets": page,
    });
    if end < total && args.get("limit").is_none() {
        out["note"] = json!(format!(
            "{total} assets match; this is {}-{}. Narrow with kind / folder / used / query / tags, page with offset, \
             or use media.summary for counts.",
            offset + 1,
            end
        ));
    }
    Ok(out)
}

/// A bin in the tree: its full path, asset count directly in it, and with every sub-bin.
#[derive(Default)]
struct Node {
    direct: usize,
    total: usize,
    kids: BTreeMap<String, Node>,
}

fn bin_tree(p: &Project) -> Node {
    let mut root = Node::default();
    let mut add = |path: &str, count: bool| {
        let mut n = &mut root;
        if count {
            n.total += 1;
        }
        if path.is_empty() {
            if count {
                n.direct += 1;
            }
            return;
        }
        let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        for (i, part) in parts.iter().enumerate() {
            n = n.kids.entry((*part).to_string()).or_default();
            if count {
                n.total += 1;
                if i + 1 == parts.len() {
                    n.direct += 1;
                }
            }
        }
    };
    for f in &p.folders {
        add(f, false);
    }
    for a in &p.assets {
        add(&a.folder, true);
    }
    root
}

/// Flat rows `{bin, assets, total}` of the tree, depth first (`bin` is the full "A/B" path).
fn tree_rows(n: &Node, prefix: &str, out: &mut Vec<Value>) {
    for (name, kid) in &n.kids {
        let path = if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
        out.push(json!({"bin": path, "assets": kid.direct, "total": kid.total}));
        tree_rows(kid, &path, out);
    }
}

pub(super) fn bins_json(p: &Project) -> Vec<Value> {
    let mut rows = Vec::new();
    tree_rows(&bin_tree(p), "", &mut rows);
    rows
}

/// `media.summary`: counts by kind, extension, bin and used / unused, the bin tree, linked folders, smart
/// bins and sequences - bounded by the number of bins, not assets (a few KB for a library of thousands).
pub(super) fn summary(p: &Project) -> Value {
    let used = p.used_assets();
    let mut by_kind: BTreeMap<&str, usize> = BTreeMap::new();
    let mut by_ext: BTreeMap<String, usize> = BTreeMap::new();
    let (mut untagged, mut unlabelled, mut root) = (0, 0, 0);
    for a in &p.assets {
        *by_kind.entry(kind_name(a)).or_default() += 1;
        if a.html.is_none() {
            *by_ext.entry(ext_of(&a.path)).or_default() += 1;
        }
        untagged += a.tags.is_empty() as usize;
        unlabelled += (a.label == 0) as usize;
        root += a.folder.is_empty() as usize;
    }
    let mut bins = bins_json(p);
    let n_bins = bins.len();
    let mut note = Value::Null;
    if n_bins > SUMMARY_FOLDERS {
        bins.sort_by_key(|b| std::cmp::Reverse(b["total"].as_u64().unwrap_or(0)));
        bins.truncate(SUMMARY_FOLDERS);
        note = json!(format!("{n_bins} bins; the {SUMMARY_FOLDERS} biggest are listed - library.bins lists all"));
    }
    let mut tags: BTreeMap<String, usize> = BTreeMap::new();
    for t in p.assets.iter().flat_map(|a| a.tags.iter()) {
        *tags.entry(t.to_lowercase()).or_default() += 1;
    }
    let mut top_tags: Vec<(String, usize)> = tags.into_iter().collect();
    top_tags.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    top_tags.truncate(30);
    json!({
        "assets": p.assets.len(),
        "used": used.len().min(p.assets.len()),
        "unused": p.assets.iter().filter(|a| !used.contains(&a.id)).count(),
        "by_kind": by_kind,
        "by_ext": by_ext,
        "in_root": root,
        "untagged": untagged,
        "unlabelled": unlabelled,
        "tags": top_tags.into_iter().map(|(t, n)| json!([t, n])).collect::<Vec<_>>(),
        "bins": bins,
        "bins_total": n_bins,
        "bins_note": note,
        "linked": linked_roots(p).into_iter().map(|(bin, path)| json!({"bin": bin, "path": path})).collect::<Vec<_>>(),
        "smart_bins": p.smart_bins.iter().map(|b| b.name.clone()).collect::<Vec<_>>(),
        "sequences": p.sequences.iter().map(|s| json!({"id": s.id, "name": s.name})).collect::<Vec<_>>(),
    })
}

fn clean_bin(s: &str) -> String {
    s.split('/').map(str::trim).filter(|s| !s.is_empty()).collect::<Vec<_>>().join("/")
}

/// `media.bulk_set`: the assets `ids` names, or that `filter` matches, get `folder` (a bin, created when
/// missing; "" = the root), `tags` (replaces), `add_tags` / `remove_tags`, `label` and / or `description`.
/// Returns how many changed. Validates everything before touching the project.
pub(super) fn bulk_set(p: &mut Project, args: &Value) -> Result<Value, String> {
    let filter = match (args.get("ids"), args.get("filter")) {
        (Some(_), Some(_)) => return Err("give ids or filter, not both".into()),
        (Some(_), None) => {
            let ids = arg_ids(args, "ids").ok_or("ids: an array of asset ids")?;
            Filter { ids: Some(ids.into_iter().collect()), ..Default::default() }
        }
        (None, Some(f)) if f.is_object() => Filter::parse(f),
        (None, Some(_)) => return Err("filter: an object like {kind: \"audio\", folder: \"\", query: \"whoosh\"}".into()),
        (None, None) => return Err("ids or filter is required (filter: {} = every asset)".into()),
    };
    let folder = arg_str(args, "folder").map(clean_bin);
    let tags = args.get("tags").map(|_| str_list(args.get("tags")));
    let add: Vec<String> = str_list(args.get("add_tags"));
    let remove: Vec<String> = str_list(args.get("remove_tags"));
    let label = match args.get("label") {
        None => None,
        Some(v) => Some(v.as_u64().filter(|l| *l <= 8).ok_or("label: 0..8 (0 = none)")? as u8),
    };
    let description = arg_str(args, "description");
    if folder.is_none() && tags.is_none() && add.is_empty() && remove.is_empty() && label.is_none() && description.is_none() {
        return Err("nothing to set: folder, tags, add_tags, remove_tags, label or description".into());
    }
    if let Some(Some(ids)) = args.get("ids").map(|_| arg_ids(args, "ids")) {
        if let Some(bad) = ids.iter().find(|id| p.asset(**id).is_none()) {
            return Err(format!("no asset {bad}"));
        }
    }
    let used = p.used_assets();
    let hits: Vec<Id> = p.assets.iter().filter(|a| filter.matches(p, a, &used)).map(|a| a.id).collect();
    if let Some(f) = folder.as_deref().filter(|f| !f.is_empty()) {
        p.add_folder(f);
    }
    let mut changed = Vec::new();
    for id in &hits {
        let a = p.asset_mut(*id).expect("matched above");
        let before = (a.folder.clone(), a.tags.clone(), a.label, a.description.clone());
        if let Some(f) = &folder {
            a.folder = f.clone();
        }
        if let Some(t) = &tags {
            a.tags = t.clone();
        }
        for t in &add {
            if !a.tags.iter().any(|x| x.eq_ignore_ascii_case(t)) {
                a.tags.push(t.clone());
            }
        }
        a.tags.retain(|x| !remove.iter().any(|r| x.eq_ignore_ascii_case(r)));
        if let Some(l) = label {
            a.label = l;
        }
        if let Some(d) = description {
            a.description = d.to_string();
        }
        if (a.folder.clone(), a.tags.clone(), a.label, a.description.clone()) != before {
            changed.push(*id);
        }
    }
    let shown: Vec<Id> = changed.iter().copied().take(50).collect();
    Ok(json!({"ok": true, "matched": hits.len(), "changed": changed.len(), "ids": shown, "ids_truncated": changed.len() > 50}))
}

/// Rename / move a bin: every bin and asset under `old` follows to `new`. Linked bins whose bin is
/// renamed keep pointing at their folder (see `linked_roots`).
fn move_bin(p: &mut Project, old: &str, new: &str) {
    let swap = |s: &mut String| {
        if s == old {
            *s = new.to_string();
        } else if let Some(rest) = s.strip_prefix(&format!("{old}/")) {
            *s = format!("{new}/{rest}");
        }
    };
    for f in &mut p.folders {
        swap(f);
    }
    for a in &mut p.assets {
        swap(&mut a.folder);
    }
    p.folders.sort();
    p.folders.dedup();
}

/// `library.bins`: list (the tree as `{bin, assets, total}` rows), create, rename (a new last name, same
/// parent), move (under `to`, "" = the top level) or delete (its assets go to the root - never deleted).
pub(super) fn bins(p: &mut Project, args: &Value) -> Result<Value, String> {
    let action = arg_str(args, "action").unwrap_or("list");
    if action == "list" {
        return Ok(json!({"bins": bins_json(p)}));
    }
    let bin = clean_bin(req(arg_str(args, "bin"), "bin")?);
    if bin.is_empty() {
        return Err("bin: a bin path like \"Footage/Day 1\"".into());
    }
    let exists = p.folder_names().iter().any(|f| *f == bin || f.starts_with(&format!("{bin}/")));
    match action {
        "create" => {
            p.add_folder(&bin);
            Ok(json!({"ok": true, "bin": bin}))
        }
        "rename" | "move" => {
            if !exists {
                return Err(format!("no bin '{bin}'"));
            }
            let to = arg_str(args, "to").ok_or(if action == "rename" { "to: the new name" } else { "to: the new parent bin (\"\" = top level)" })?;
            let leaf = bin.rsplit('/').next().unwrap_or(&bin).to_string();
            let parent = bin.rsplit_once('/').map(|(p, _)| p.to_string()).unwrap_or_default();
            let new = if action == "rename" {
                let name = to.trim();
                if name.is_empty() || name.contains('/') {
                    return Err("to: a name without '/' (use action=move to change the parent)".into());
                }
                if parent.is_empty() { name.to_string() } else { format!("{parent}/{name}") }
            } else {
                let to = clean_bin(to);
                if to == bin || to.starts_with(&format!("{bin}/")) {
                    return Err(format!("can't move '{bin}' into itself"));
                }
                if to.is_empty() { leaf } else { format!("{to}/{leaf}") }
            };
            if new != bin && p.folder_names().iter().any(|f| *f == new) {
                // merging into an existing bin is what a user means by "move these together"
                move_bin(p, &bin, &new);
                return Ok(json!({"ok": true, "bin": new, "merged": true}));
            }
            move_bin(p, &bin, &new);
            Ok(json!({"ok": true, "bin": new}))
        }
        "delete" => {
            if !exists {
                return Err(format!("no bin '{bin}'"));
            }
            let moved = p.assets.iter().filter(|a| in_bin(&a.folder, &bin, true)).count();
            p.remove_folder(&bin);
            Ok(json!({"ok": true, "moved_to_root": moved}))
        }
        a => Err(format!("action: list | create | rename | move | delete (not '{a}')")),
    }
}

// ---------------------------------------------------------------------------------------------------
// Linked bins - the one place the library API reads the disk, read only.
//
// Interface for feat/linked-bins: today a linked folder is `Project::linked_folders` (a path, shown as a
// bin named after the folder). When linked bins (`Project::linked_bins: Vec<LinkedBin {bin, path}>`) land,
// `linked_roots` returns those pairs instead and `linked_list` can use `engine::linked_bins::list_dir`;
// nothing else here (or in the Luau API) changes.

/// Every linked folder as (bin name, folder path).
pub(super) fn linked_roots(p: &Project) -> Vec<(String, String)> {
    p.linked_folders
        .iter()
        .map(|path| {
            let t = path.trim_end_matches(['\\', '/']);
            let name = t.rsplit(['\\', '/']).next().filter(|s| !s.is_empty()).unwrap_or(t).to_string();
            (name, path.clone())
        })
        .collect()
}

/// One level of a linked folder: `sub` is a relative path below it ("SFX/Impacts"; "" = the folder
/// itself). Only media files are listed; nothing outside the linked folder can be named (no "..", no
/// absolute paths, no drive letters). Files that are already assets carry their id.
pub(super) fn linked_list(p: &Project, bin: &str, sub: &str) -> Result<Value, String> {
    let roots = linked_roots(p);
    let (name, root) = roots
        .iter()
        .find(|(b, path)| b.eq_ignore_ascii_case(bin) || path.eq_ignore_ascii_case(bin))
        .ok_or_else(|| format!("no linked bin '{bin}' (linked: {})", roots.iter().map(|r| r.0.as_str()).collect::<Vec<_>>().join(", ")))?;
    let parts: Vec<&str> = sub.split(['/', '\\']).filter(|s| !s.is_empty() && *s != ".").collect();
    if parts.iter().any(|s| *s == ".." || s.contains(':')) {
        return Err("sub: a folder below the linked bin (no '..' or drive letters)".into());
    }
    let mut dir = std::path::PathBuf::from(root);
    dir.extend(&parts);
    let rd = std::fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut dirs = Vec::new();
    let mut files = Vec::new();
    for e in rd.flatten() {
        let fname = e.file_name().to_string_lossy().into_owned();
        if fname.starts_with('.') {
            continue;
        }
        let Ok(t) = e.file_type() else { continue };
        if t.is_dir() {
            dirs.push(fname);
        } else if t.is_file() && MEDIA_EXTS.contains(&ext_of(&fname).as_str()) {
            let path = e.path().to_string_lossy().into_owned();
            let id = p.asset_by_path(&path).map(|a| a.id);
            files.push(json!({"name": fname, "path": path, "asset": id}));
        }
    }
    let nat = crate::engine::folder_import::natural_cmp;
    dirs.sort_by(|a, b| nat(a, b));
    files.sort_by(|a, b| nat(a["name"].as_str().unwrap_or(""), b["name"].as_str().unwrap_or("")));
    Ok(json!({"bin": name, "sub": parts.join("/"), "dirs": dirs, "files": files}))
}

pub const TOOLS: &[ToolDef] = &[
    ToolDef {
        name: "media.summary",
        desc: "The library at a glance, in a few KB whatever its size: asset counts by kind, extension, bin and \
               used / unused, untagged / unlabelled counts, the top tags, the bin tree ({bin, assets, total} \
               rows), linked folders, smart bins and sequences. Start here before media.list.",
        args: &[],
        kind: ToolKind::Read,
        run: |app, _| Ok(ToolOutcome::Done(summary(&app.project))),
    },
    ToolDef {
        name: "media.bulk_set",
        desc: "File, tag or label many library assets in one call (one undo step): the assets in `ids`, or every \
               asset `filter` matches (same keys as media.list: kind, folder, nested, used, query, tags, label, \
               ext, untagged; {} = all). Sets folder (a bin path like \"Audio/SFX\", created when missing; \"\" = \
               the root), tags (replaces), add_tags, remove_tags, label (0-8) and / or description. Never \
               deletes anything. Returns matched / changed counts.",
        args: &[
            "ids:array:false:asset ids",
            "filter:object:false:{kind, folder, nested, used, query, tags, label, ext, untagged}",
            "folder:string:false:target bin",
            "tags:array:false:replace the tags",
            "add_tags:array:false:",
            "remove_tags:array:false:",
            "label:integer:false:0..8",
            "description:string:false:",
        ],
        kind: ToolKind::Mutate,
        run: |app, args| Ok(ToolOutcome::Done(bulk_set(&mut app.project, args)?)),
    },
    ToolDef {
        name: "library.bins",
        desc: "Library bins (folders; nested by '/'). action=list (default): every bin as {bin, assets, total}. \
               create {bin} (parents too). rename {bin, to: new name}. move {bin, to: parent bin, \"\" = top \
               level}; onto an existing bin it merges. delete {bin}: its assets go to the root, nothing is \
               deleted. Assets follow their bin. One undo step per change.",
        args: &[
            "action:string:false:list | create | rename | move | delete",
            "bin:string:false:bin path, e.g. \"Footage/Day 1\"",
            "to:string:false:rename: the new name; move: the new parent",
        ],
        kind: ToolKind::Mutate,
        run: |app, args| Ok(ToolOutcome::Done(bins(&mut app.project, args)?)),
    },
    ToolDef {
        name: "media.linked",
        desc: "Linked folders (bins that show a folder on disk). No bin: the list {bin, path}. With bin: one level \
               of that folder (sub: a folder below it, e.g. \"SFX/Impacts\") - its sub-folders and media files, \
               each file with its asset id if it is already in the library. Read only; nothing outside a linked \
               folder can be listed.",
        args: &["bin:string:false:a linked bin's name", "sub:string:false:a folder below it"],
        kind: ToolKind::Read,
        run: |app, args| {
            let v = match arg_str(args, "bin") {
                None => json!({"linked": linked_roots(&app.project).into_iter().map(|(b, p)| json!({"bin": b, "path": p})).collect::<Vec<_>>()}),
                Some(bin) => linked_list(&app.project, bin, arg_str(args, "sub").unwrap_or(""))?,
            };
            Ok(ToolOutcome::Done(v))
        },
    },
];

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    pub(crate) fn asset(id: Id, path: &str, kind: ClipKind, folder: &str) -> Asset {
        Asset {
            id,
            path: path.into(),
            kind,
            duration: 2.0,
            width: 0,
            height: 0,
            fps: 0.0,
            audio_streams: Vec::new(),
            codec: String::new(),
            folder: folder.into(),
            tags: Vec::new(),
            label: 0,
            description: String::new(),
            rel_path: None,
            parent: None,
            range: None,
            effects: Vec::new(),
            html: None,
            proxy: None,
        }
    }

    pub(crate) fn sample() -> Project {
        let mut p = Project::new();
        p.assets = vec![
            asset(1, r"C:\v\intro.mp4", ClipKind::Video, ""),
            asset(2, r"C:\v\whoosh 1.wav", ClipKind::Audio, ""),
            asset(3, r"C:\v\song.mp3", ClipKind::Audio, "Old"),
            asset(4, r"C:\v\logo.png", ClipKind::Image, "Old/Nested"),
            asset(5, r"C:\v\b-roll 2.mov", ClipKind::Video, ""),
        ];
        p.assets[1].tags = vec!["SFX".into()];
        p
    }

    #[test]
    fn media_list_filters_fields_and_pages() {
        let p = sample();
        // no args, small library: the plain array, as before
        let all = list(&p, &json!({})).unwrap();
        assert_eq!(all.as_array().unwrap().len(), 5);
        assert!(all[0].get("path").is_some());
        let r = list(&p, &json!({"kind": "audio", "fields": ["name", "ext"]})).unwrap();
        assert_eq!(r["total"], 2);
        assert_eq!(r["assets"][0], json!({"id": 2, "name": "whoosh 1.wav", "ext": "wav"}));
        // folder includes nested bins unless nested=false
        assert_eq!(list(&p, &json!({"folder": "Old"})).unwrap()["total"], 2);
        assert_eq!(list(&p, &json!({"folder": "Old", "nested": false})).unwrap()["total"], 1);
        assert_eq!(list(&p, &json!({"folder": ""})).unwrap()["total"], 5);
        assert_eq!(list(&p, &json!({"folder": "", "nested": false})).unwrap()["total"], 3);
        assert_eq!(list(&p, &json!({"tags": ["sfx"]})).unwrap()["assets"][0]["id"], 2);
        assert_eq!(list(&p, &json!({"query": "ROLL"})).unwrap()["assets"][0]["id"], 5);
        assert_eq!(list(&p, &json!({"used": false})).unwrap()["total"], 5);
        assert_eq!(list(&p, &json!({"ext": ".png,mp3"})).unwrap()["total"], 2);
        // paging
        let pg = list(&p, &json!({"limit": 2, "offset": 2, "sort": "-id"})).unwrap();
        assert_eq!(pg["count"], 2);
        assert_eq!(pg["assets"][0]["id"], 3);
        assert_eq!(pg["next_offset"], 4);
        assert!(list(&p, &json!({"fields": ["nope"]})).unwrap_err().contains("unknown field"));
        // a big library never answers with everything by default
        let mut big = Project::new();
        big.assets = (0..300).map(|i| asset(i, &format!(r"C:\x\f{i}.mp4"), ClipKind::Video, "")).collect();
        let r = list(&big, &json!({})).unwrap();
        assert_eq!((r["total"].as_u64(), r["count"].as_u64()), (Some(300), Some(LIST_DEFAULT as u64)));
        assert!(r["note"].as_str().unwrap().contains("media.summary"));
    }

    #[test]
    fn media_summary_stays_small_for_1000_assets() {
        let mut p = Project::new();
        let kinds = [ClipKind::Video, ClipKind::Audio, ClipKind::Image];
        p.assets = (0..1000)
            .map(|i| {
                let mut a = asset(i, &format!(r"C:\Users\someone\Videos\Some Long Project Folder\clip number {i}.mp4"), kinds[i as usize % 3], "");
                a.folder = format!("Bin {}/Sub {}", i % 20, i % 3);
                a.tags = vec![format!("tag{}", i % 7)];
                a.description = "a long description that must not end up in the summary at all".into();
                a
            })
            .collect();
        let s = summary(&p).to_string();
        assert!(s.len() < 8_000, "summary is {} bytes", s.len());
        let v: Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["assets"], 1000);
        assert_eq!(v["unused"], 1000);
        assert_eq!(v["by_kind"]["video"], 334);
        assert_eq!(v["bins_total"], 80);
        assert!(v["bins"].as_array().unwrap().iter().any(|b| b["bin"] == "Bin 3" && b["total"] == 50));
    }

    #[test]
    fn bulk_set_files_and_tags_by_filter_and_ids() {
        let mut p = sample();
        let r = bulk_set(&mut p, &json!({"filter": {"kind": "audio", "query": "whoosh"}, "folder": "Audio/SFX", "add_tags": ["sfx", "whoosh"]})).unwrap();
        assert_eq!((r["matched"].as_u64(), r["changed"].as_u64()), (Some(1), Some(1)));
        let a = p.asset(2).unwrap();
        assert_eq!(a.folder, "Audio/SFX");
        assert_eq!(a.tags, vec!["SFX".to_string(), "whoosh".to_string()]); // "sfx" was already there
        assert!(p.folders.contains(&"Audio/SFX".to_string()));
        bulk_set(&mut p, &json!({"ids": [1, 5], "label": 3, "folder": " Footage / "})).unwrap();
        assert_eq!((p.asset(1).unwrap().label, p.asset(5).unwrap().folder.as_str()), (3, "Footage"));
        // validation happens before anything changes
        let snap = p.to_json();
        assert!(bulk_set(&mut p, &json!({"ids": [1, 99], "folder": "X"})).is_err());
        assert!(bulk_set(&mut p, &json!({"ids": [1], "label": 12})).is_err());
        assert!(bulk_set(&mut p, &json!({"folder": "X"})).is_err());
        assert!(bulk_set(&mut p, &json!({"ids": [1]})).is_err());
        assert_eq!(p.to_json(), snap);
        // nothing is ever deleted
        bulk_set(&mut p, &json!({"filter": {}, "remove_tags": ["whoosh"]})).unwrap();
        assert_eq!(p.assets.len(), 5);
    }

    #[test]
    fn library_bins_create_rename_move_delete() {
        let mut p = sample();
        bins(&mut p, &json!({"action": "create", "bin": "Footage/Day 1"})).unwrap();
        assert!(p.folder_names().contains(&"Footage/Day 1".to_string()));
        bins(&mut p, &json!({"action": "rename", "bin": "Old", "to": "Archive"})).unwrap();
        assert_eq!(p.asset(4).unwrap().folder, "Archive/Nested");
        bins(&mut p, &json!({"action": "move", "bin": "Archive/Nested", "to": "Footage"})).unwrap();
        assert_eq!(p.asset(4).unwrap().folder, "Footage/Nested");
        assert!(bins(&mut p, &json!({"action": "move", "bin": "Footage", "to": "Footage/Nested"})).is_err());
        let r = bins(&mut p, &json!({"action": "delete", "bin": "Footage"})).unwrap();
        assert_eq!(r["moved_to_root"], 1);
        assert_eq!((p.assets.len(), p.asset(4).unwrap().folder.as_str()), (5, ""));
        let l = bins(&mut p, &json!({})).unwrap();
        assert_eq!(l["bins"], json!([{"bin": "Archive", "assets": 1, "total": 1}]));
    }

    #[test]
    fn linked_listing_stays_inside_the_linked_folder() {
        let root = std::env::temp_dir().join(format!("se-linked-{}", std::process::id()));
        std::fs::create_dir_all(root.join("SFX")).unwrap();
        std::fs::write(root.join("SFX").join("hit.wav"), b"x").unwrap();
        std::fs::write(root.join("notes.txt"), b"x").unwrap();
        let mut p = sample();
        p.linked_folders.push(root.to_string_lossy().into_owned());
        let name = root.file_name().unwrap().to_string_lossy().into_owned();
        let top = linked_list(&p, &name, "").unwrap();
        assert_eq!(top["dirs"], json!(["SFX"]));
        assert_eq!(top["files"], json!([])); // .txt is not media
        let sfx = linked_list(&p, &name, "SFX").unwrap();
        assert_eq!(sfx["files"][0]["name"], "hit.wav");
        assert!(linked_list(&p, &name, "..").is_err());
        assert!(linked_list(&p, &name, r"C:\Windows").is_err());
        assert!(linked_list(&p, "not-linked", "").is_err());
        let _ = std::fs::remove_dir_all(&root);
    }
}
