//! ---- feat/plugins ----
//! Plugins: a folder (or a zip of one) with a `plugin.json` that bundles Luau scripts, Luau panels,
//! skills and library content (effects, transitions, LUTs, HTML templates, icons, OFL fonts), plus the
//! permissions it asks for. The format is specified in `docs/plugins.md`.
//!
//! On disk: `%APPDATA%\SimpleEditor\plugins\<id>\` holds each installed plugin as it was unpacked, and
//! `plugins\state.json` what the user decided about it (enabled, the permissions they granted, folders they
//! picked) and what it put into the library, so Disable and Uninstall take exactly that back out.
//!
//! Nothing here runs plugin code: installing only unpacks, checks and copies files. Code runs through
//! `runtime` (scripts and panels), sandboxed, with the permissions the user granted.

pub mod github;
pub mod panel;
pub mod runtime;
pub mod zip;

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::engine::library::Kind;

/// What a plugin folder must contain at its root.
pub const MANIFEST: &str = "plugin.json";

/// `plugin.json`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Manifest {
    /// Lower-case letters, digits and dashes: the folder name and the prefix of everything it registers.
    pub id: String,
    pub name: String,
    pub version: String,
    pub author: String,
    pub description: String,
    /// The oldest Simple Editor that can run it ("" = any).
    #[serde(alias = "minAppVersion")]
    pub min_app_version: String,
    /// Usually the GitHub repository: "View on GitHub" and Update use it.
    pub homepage: String,
    /// One of the app's icon names (`bolt`, `wrench`, ...), shown on its menu rows and panel tabs.
    pub icon: String,
    pub contents: Contents,
    pub permissions: Permissions,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Contents {
    /// `.luau` files run from Window ▸ Plugins ▸ <plugin>.
    pub scripts: Vec<String>,
    pub panels: Vec<PanelDecl>,
    /// Assistant skills (`.md`).
    pub skills: Vec<String>,
    /// Shader effects (`.glsl`). `shaders` is read as the same list.
    #[serde(alias = "shaders")]
    pub effects: Vec<String>,
    pub transitions: Vec<String>,
    /// `.cube` LUTs.
    pub luts: Vec<String>,
    /// HTML templates (`.html`).
    pub templates: Vec<String>,
    /// `.svg` icons.
    pub icons: Vec<String>,
    /// `.ttf` / `.otf` fonts under the SIL Open Font License only.
    pub fonts: Vec<FontDecl>,
}

/// A dockable panel drawn by a Luau script.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PanelDecl {
    /// Unique inside the plugin (`main`).
    pub id: String,
    /// The tab's name.
    pub title: String,
    /// The `.luau` file that draws it.
    pub script: String,
    /// One of the app's icon names; the plugin's icon when empty.
    pub icon: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FontDecl {
    pub file: String,
    /// Must be the SIL Open Font License (`OFL-1.1`).
    pub license: String,
}

/// What a plugin may do beyond reading and editing the open project. Declared in `plugin.json`, shown at
/// install, and granted (all or some) by the user; anything not granted is refused at run time.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Permissions {
    /// Hosts it may download from (`api.example.com`, exact names).
    pub network: Vec<String>,
    /// `"project"` = the open project's folder, `"pick"` = folders the user picks in the Plugins pane.
    pub files: Vec<String>,
    /// Start other programs. Off unless declared and granted.
    pub run_programs: bool,
    /// App tools beyond the project / library ones every plugin may call (`export.video`, ...).
    pub mcp_tools: Vec<String>,
}

impl Permissions {
    pub fn is_empty(&self) -> bool {
        self.network.is_empty() && self.files.is_empty() && !self.run_programs && self.mcp_tools.is_empty()
    }
    /// The granted part of `self`: only what was also declared (a state file can't widen a plugin).
    pub fn within(&self, declared: &Permissions) -> Permissions {
        Permissions {
            network: self.network.iter().filter(|h| declared.network.contains(h)).cloned().collect(),
            files: self.files.iter().filter(|f| declared.files.contains(f)).cloned().collect(),
            run_programs: self.run_programs && declared.run_programs,
            mcp_tools: self.mcp_tools.iter().filter(|t| declared.mcp_tools.contains(t)).cloned().collect(),
        }
    }
    /// One line per permission, for the install confirm and the manager.
    pub fn lines(&self) -> Vec<String> {
        let mut out = Vec::new();
        if !self.network.is_empty() {
            out.push(format!("Network: {}", self.network.join(", ")));
        }
        for f in &self.files {
            out.push(match f.as_str() {
                "project" => "Files: the open project's folder".to_string(),
                "pick" => "Files: folders you pick".to_string(),
                other => format!("Files: {other}"),
            });
        }
        if self.run_programs {
            out.push("Run programs on this PC".to_string());
        }
        if !self.mcp_tools.is_empty() {
            out.push(format!("App tools: {}", self.mcp_tools.join(", ")));
        }
        out
    }
}

/// Every content list with the library kind it installs as (`None` = stays in the plugin folder).
fn content_lists(c: &Contents) -> Vec<(&'static str, Option<Kind>, Vec<&str>)> {
    vec![
        ("scripts", None, c.scripts.iter().map(String::as_str).collect()),
        ("panels", None, c.panels.iter().map(|p| p.script.as_str()).collect()),
        ("skills", Some(Kind::Skill), c.skills.iter().map(String::as_str).collect()),
        ("effects", Some(Kind::Effect), c.effects.iter().map(String::as_str).collect()),
        ("transitions", Some(Kind::Transition), c.transitions.iter().map(String::as_str).collect()),
        ("luts", Some(Kind::Lut), c.luts.iter().map(String::as_str).collect()),
        ("templates", Some(Kind::Html), c.templates.iter().map(String::as_str).collect()),
        ("icons", Some(Kind::Icon), c.icons.iter().map(String::as_str).collect()),
        ("fonts", None, c.fonts.iter().map(|f| f.file.as_str()).collect()),
    ]
}

fn exts(list: &str) -> &'static [&'static str] {
    match list {
        "scripts" | "panels" => &["luau", "lua"],
        "skills" => &["md"],
        "effects" | "transitions" => &["glsl"],
        "luts" => &["cube"],
        "templates" => &["html", "htm"],
        "icons" => &["svg"],
        "fonts" => &["ttf", "otf"],
        _ => &[],
    }
}

fn is_slug(s: &str) -> bool {
    (2..=64).contains(&s.len())
        && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !s.starts_with('-')
        && !s.ends_with('-')
}

fn is_host(h: &str) -> bool {
    !h.is_empty()
        && h.len() <= 253
        && h.contains('.')
        && h.split('.').all(|l| !l.is_empty() && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'))
}

/// The running app's version.
pub fn app_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

impl Manifest {
    pub fn parse(text: &str) -> Result<Manifest, String> {
        serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(|e| format!("plugin.json: {e}"))
    }

    /// Check the manifest against the folder it came from: required fields, a usable id, files that exist
    /// inside the folder with the right extensions, OFL fonts, well-formed permissions, and an app new
    /// enough. Err = the first problem, in words a plugin author can act on; Ok = warnings (not fatal).
    pub fn validate(&self, dir: &Path) -> Result<Vec<String>, String> {
        let mut warn = Vec::new();
        if !is_slug(&self.id) {
            return Err(format!(
                "id '{}' must be 2-64 lower-case letters, digits and dashes (e.g. \"hello-panel\")",
                self.id
            ));
        }
        for (field, v) in [("name", &self.name), ("version", &self.version), ("author", &self.author)] {
            if v.trim().is_empty() {
                return Err(format!("plugin.json needs a \"{field}\""));
            }
        }
        if !self.version.trim_start_matches(['v', 'V']).starts_with(|c: char| c.is_ascii_digit()) {
            return Err(format!("version '{}' must start with a number (e.g. \"1.0.0\")", self.version));
        }
        if !self.min_app_version.is_empty() && github::newer(&self.min_app_version, app_version()) {
            return Err(format!("{} needs Simple Editor {} or newer (this is {})", self.name, self.min_app_version, app_version()));
        }
        for (list, _, files) in content_lists(&self.contents) {
            for f in files {
                let Some(rel) = zip::safe_name(f) else {
                    return Err(format!("{list}: '{f}' must be a path inside the plugin folder"));
                };
                let ext = rel.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
                if !exts(list).contains(&ext.as_str()) {
                    return Err(format!("{list}: '{f}' must be a {} file", exts(list).join(" / ")));
                }
                if !dir.join(&rel).is_file() {
                    return Err(format!("{list}: '{f}' is listed but not in the plugin"));
                }
            }
        }
        let mut ids = std::collections::HashSet::new();
        for p in &self.contents.panels {
            if !is_slug(&p.id) && !(p.id.len() == 1 && p.id.bytes().all(|b| b.is_ascii_lowercase())) {
                return Err(format!("panel id '{}' must be lower-case letters, digits and dashes", p.id));
            }
            if !ids.insert(p.id.as_str()) {
                return Err(format!("two panels are called '{}'", p.id));
            }
            if p.title.trim().is_empty() {
                return Err(format!("panel '{}' needs a \"title\"", p.id));
            }
        }
        for f in &self.contents.fonts {
            let l = f.license.trim().to_ascii_uppercase();
            if !(l == "OFL" || l == "OFL-1.1" || l == "OFL 1.1") {
                return Err(format!("font '{}': only fonts under the SIL Open Font License (\"license\": \"OFL-1.1\") can be bundled", f.file));
            }
        }
        for h in &self.permissions.network {
            if !is_host(h) {
                return Err(format!("network: '{h}' must be a host name like api.example.com (no wildcards, no paths)"));
            }
        }
        for f in &self.permissions.files {
            if f != "project" && f != "pick" {
                return Err(format!("files: '{f}' must be \"project\" or \"pick\""));
            }
        }
        for t in &self.permissions.mcp_tools {
            if crate::mcp::tools::find(t).is_none() {
                warn.push(format!("app tool '{t}' does not exist in this version"));
            }
        }
        if self.contents.scripts.is_empty() && self.contents.panels.is_empty() && content_lists(&self.contents).iter().all(|l| l.2.is_empty()) {
            warn.push("the plugin has no contents".into());
        }
        Ok(warn)
    }

    /// The contents, one line per non-empty list ("2 scripts", "1 panel: Hello").
    pub fn content_lines(&self) -> Vec<String> {
        let c = &self.contents;
        let mut out = Vec::new();
        let mut n = |count: usize, one: &str, many: &str, names: String| {
            if count > 0 {
                let what = if count == 1 { one } else { many };
                out.push(if names.is_empty() { format!("{count} {what}") } else { format!("{count} {what}: {names}") });
            }
        };
        n(c.panels.len(), "panel", "panels", c.panels.iter().map(|p| p.title.as_str()).collect::<Vec<_>>().join(", "));
        n(c.scripts.len(), "script", "scripts", c.scripts.iter().map(|s| stem(s)).collect::<Vec<_>>().join(", "));
        n(c.skills.len(), "skill", "skills", String::new());
        n(c.effects.len(), "effect", "effects", String::new());
        n(c.transitions.len(), "transition", "transitions", String::new());
        n(c.luts.len(), "LUT", "LUTs", String::new());
        n(c.templates.len(), "template", "templates", String::new());
        n(c.icons.len(), "icon", "icons", String::new());
        n(c.fonts.len(), "font", "fonts", String::new());
        out
    }
}

/// "scripts/tidy-bins.luau" -> "tidy-bins".
pub fn stem(rel: &str) -> String {
    Path::new(rel).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

// ------------------------------------------------------------------ state

/// What the user decided about one installed plugin.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Installed {
    pub enabled: bool,
    /// The link it was installed from ("" = a local folder or zip).
    pub source: String,
    /// The release tag it came from ("" = a branch archive or a local install).
    pub tag: String,
    /// Granted at install (a subset of what plugin.json declares).
    pub granted: Permissions,
    /// Folders the user picked for a `files: pick` plugin.
    pub folders: Vec<String>,
    /// Library items it installed: "kind/id" (`library::full_id`), taken back out on Disable / Uninstall.
    pub registered: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    pub plugins: BTreeMap<String, Installed>,
    /// `Pane::Plugin(n)` shows `panel_slots[n]` ("plugin-id/panel-id"). Slots are never reused for another
    /// panel, so a layout saved with a plugin's panel finds it again (or a closed slot) after a restart.
    pub panel_slots: Vec<String>,
}

/// Where plugins live. Tests point every root at a scratch folder.
#[derive(Clone, Debug)]
pub struct Roots {
    pub plugins: PathBuf,
    pub library: PathBuf,
}

impl Roots {
    pub fn app() -> Roots {
        Roots { plugins: crate::settings::Settings::dir().join("plugins"), library: crate::engine::library::root() }
    }
    pub fn dir_of(&self, id: &str) -> PathBuf {
        self.plugins.join(id)
    }
    fn state_path(&self) -> PathBuf {
        self.plugins.join("state.json")
    }
    pub fn load_state(&self) -> State {
        std::fs::read_to_string(self.state_path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
    }
    pub fn save_state(&self, s: &State) -> Result<(), String> {
        std::fs::create_dir_all(&self.plugins).map_err(|e| format!("{}: {e}", self.plugins.display()))?;
        let p = self.state_path();
        let tmp = p.with_extension("json.part");
        let text = serde_json::to_string_pretty(s).map_err(|e| e.to_string())?;
        std::fs::write(&tmp, text).map_err(|e| format!("{}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, &p).map_err(|e| format!("{}: {e}", p.display()))
    }
}

/// One installed plugin as the app sees it.
#[derive(Clone, Debug, PartialEq)]
pub struct Plugin {
    pub manifest: Manifest,
    pub dir: PathBuf,
    pub state: Installed,
    /// A plugin.json that no longer reads or validates (edited by hand): listed, never run.
    pub error: Option<String>,
}

impl Plugin {
    /// What it may do now: granted, and still declared by its plugin.json.
    pub fn granted(&self) -> Permissions {
        self.state.granted.within(&self.manifest.permissions)
    }
    pub fn runnable(&self) -> bool {
        self.state.enabled && self.error.is_none()
    }
}

/// Every installed plugin (state entries whose folder exists), by id.
pub fn list(roots: &Roots) -> Vec<Plugin> {
    let state = roots.load_state();
    let mut out = Vec::new();
    for (id, st) in &state.plugins {
        let dir = roots.dir_of(id);
        let (manifest, error) = match std::fs::read_to_string(dir.join(MANIFEST)).map_err(|e| e.to_string()).and_then(|t| Manifest::parse(&t)) {
            Ok(m) => {
                let err = match m.validate(&dir) {
                    Ok(_) if m.id != *id => Some(format!("plugin.json says its id is '{}'", m.id)),
                    Ok(_) => None,
                    Err(e) => Some(e),
                };
                (m, err)
            }
            Err(e) => (Manifest { id: id.clone(), name: id.clone(), ..Default::default() }, Some(e)),
        };
        if dir.is_dir() {
            out.push(Plugin { manifest, dir, state: st.clone(), error });
        }
    }
    out
}

// ------------------------------------------------------------------ install

/// A plugin unpacked into a staging folder, checked, waiting for the user's confirm.
#[derive(Clone, Debug)]
pub struct Staged {
    pub manifest: Manifest,
    /// The folder holding plugin.json (inside `scratch`).
    pub dir: PathBuf,
    /// The staging folder to delete afterwards.
    pub scratch: PathBuf,
    pub source: String,
    pub tag: String,
    pub warnings: Vec<String>,
    /// The version already installed, when this would replace it.
    pub replaces: Option<String>,
}

impl Staged {
    pub fn discard(&self) {
        let _ = std::fs::remove_dir_all(&self.scratch);
    }
}

/// The folder under `root` that holds plugin.json: `root` itself, its single top folder (GitHub's
/// `repo-main/` wrapper), or `subpath` inside that.
pub fn find_root(root: &Path, subpath: &str) -> Result<PathBuf, String> {
    let mut base = root.to_path_buf();
    if !base.join(MANIFEST).is_file() && subpath.is_empty() || !subpath.is_empty() && !base.join(subpath).is_dir() {
        let dirs: Vec<PathBuf> = std::fs::read_dir(root).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
        if dirs.len() == 1 {
            base = dirs[0].clone();
        }
    }
    if !subpath.is_empty() {
        let rel = zip::safe_name(subpath).ok_or("the link's folder is not a plain path")?;
        base = base.join(rel);
    }
    if base.join(MANIFEST).is_file() {
        Ok(base)
    } else {
        Err(format!("no {MANIFEST} found{}", if subpath.is_empty() { " at the top of the download".to_string() } else { format!(" in '{subpath}'") }))
    }
}

fn scratch_dir(roots: &Roots) -> PathBuf {
    let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    roots.plugins.join(".staging").join(format!("{n:x}"))
}

/// Read and check a staged folder.
fn stage_dir(roots: &Roots, scratch: PathBuf, dir: PathBuf, source: &str, tag: &str) -> Result<Staged, String> {
    let fail = |e: String| {
        let _ = std::fs::remove_dir_all(&scratch);
        e
    };
    let text = std::fs::read_to_string(dir.join(MANIFEST)).map_err(|e| fail(format!("{MANIFEST}: {e}")))?;
    let manifest = Manifest::parse(&text).map_err(fail)?;
    let warnings = manifest.validate(&dir).map_err(fail)?;
    let replaces = list(roots).into_iter().find(|p| p.manifest.id == manifest.id).map(|p| p.manifest.version);
    Ok(Staged { manifest, dir, scratch, source: source.into(), tag: tag.into(), warnings, replaces })
}

/// Unpack a downloaded (or local) zip and check it. Nothing is installed yet.
pub fn stage_zip(roots: &Roots, bytes: &[u8], subpath: &str, source: &str, tag: &str) -> Result<Staged, String> {
    let scratch = scratch_dir(roots);
    if let Err(e) = zip::extract(bytes, &scratch) {
        let _ = std::fs::remove_dir_all(&scratch);
        return Err(e);
    }
    let dir = match find_root(&scratch, subpath) {
        Ok(d) => d,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&scratch);
            return Err(e);
        }
    };
    stage_dir(roots, scratch, dir, source, tag)
}

/// Stage a plugin folder on disk (an author's working copy): it is copied, never moved.
pub fn stage_folder(roots: &Roots, folder: &Path) -> Result<Staged, String> {
    let scratch = scratch_dir(roots);
    copy_tree(folder, &scratch).inspect_err(|_| {
        let _ = std::fs::remove_dir_all(&scratch);
    })?;
    let dir = find_root(&scratch, "")?;
    stage_dir(roots, scratch, dir, "", "")
}

fn copy_tree(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::create_dir_all(to).map_err(|e| format!("{}: {e}", to.display()))?;
    for e in std::fs::read_dir(from).map_err(|e| format!("{}: {e}", from.display()))?.flatten() {
        let (src, dst) = (e.path(), to.join(e.file_name()));
        let ft = e.file_type().map_err(|e| e.to_string())?;
        if ft.is_dir() {
            if e.file_name() != ".git" {
                copy_tree(&src, &dst)?;
            }
        } else if ft.is_file() {
            std::fs::copy(&src, &dst).map_err(|e| format!("{}: {e}", src.display()))?;
        }
    }
    Ok(())
}

/// Install a confirmed `staged` plugin with the permissions the user `granted`: its folder replaces any
/// older copy, its library content is registered (if `enable`), and nothing of it runs. An update keeps
/// the user's picked folders.
pub fn commit(roots: &Roots, staged: &Staged, granted: &Permissions, enable: bool) -> Result<Plugin, String> {
    let id = staged.manifest.id.clone();
    let dest = roots.dir_of(&id);
    let mut state = roots.load_state();
    if let Some(old) = state.plugins.get(&id).cloned() {
        unregister(roots, &old.registered);
    }
    let _ = std::fs::remove_dir_all(&dest);
    std::fs::create_dir_all(&roots.plugins).map_err(|e| format!("{}: {e}", roots.plugins.display()))?;
    if std::fs::rename(&staged.dir, &dest).is_err() {
        copy_tree(&staged.dir, &dest)?;
    }
    staged.discard();
    let prev = state.plugins.remove(&id).unwrap_or_default();
    let mut inst = Installed {
        enabled: enable,
        source: if staged.source.is_empty() { prev.source } else { staged.source.clone() },
        tag: staged.tag.clone(),
        granted: granted.within(&staged.manifest.permissions),
        folders: prev.folders,
        registered: Vec::new(),
    };
    if enable {
        inst.registered = register(roots, &staged.manifest, &dest)?;
    }
    state.plugins.insert(id.clone(), inst);
    roots.save_state(&state)?;
    list(roots).into_iter().find(|p| p.manifest.id == id).ok_or_else(|| "the plugin vanished after installing".into())
}

/// Copy the plugin's library content into the library. An item whose name is already taken by something
/// that isn't this plugin's gets the plugin's id as a prefix, so a plugin never overwrites the user's own.
fn register(roots: &Roots, m: &Manifest, dir: &Path) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for (_, kind, files) in content_lists(&m.contents) {
        let Some(kind) = kind else { continue };
        for f in files {
            let src = dir.join(zip::safe_name(f).ok_or("unsafe path")?);
            let base = crate::engine::library::slug(&stem(f));
            let path_for = |id: &str| roots.library.join(kind.dir()).join(format!("{id}.{}", kind.ext()));
            let id = if path_for(&base).exists() { format!("{}-{base}", m.id) } else { base };
            let dest = path_for(&id);
            if let Some(d) = dest.parent() {
                std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
            }
            std::fs::copy(&src, &dest).map_err(|e| format!("{}: {e}", src.display()))?;
            out.push(crate::engine::library::full_id(kind, &id));
        }
    }
    Ok(out)
}

fn unregister(roots: &Roots, items: &[String]) {
    for full in items {
        if let Some((kind, id)) = crate::engine::library::split_id(full) {
            let _ = std::fs::remove_file(roots.library.join(kind.dir()).join(format!("{id}.{}", kind.ext())));
        }
    }
}

/// Turn a plugin on (registers its library content) or off (takes it back out). Its panels and scripts
/// follow `Installed::enabled`.
pub fn set_enabled(roots: &Roots, id: &str, on: bool) -> Result<(), String> {
    let mut state = roots.load_state();
    let plugin = list(roots).into_iter().find(|p| p.manifest.id == id).ok_or_else(|| format!("no plugin '{id}' is installed"))?;
    let inst = state.plugins.get_mut(id).ok_or_else(|| format!("no plugin '{id}' is installed"))?;
    if inst.enabled == on {
        return Ok(());
    }
    if on {
        if let Some(e) = &plugin.error {
            return Err(format!("{} can't be enabled: {e}", plugin.manifest.name));
        }
        inst.registered = register(roots, &plugin.manifest, &plugin.dir)?;
    } else {
        unregister(roots, &inst.registered);
        inst.registered.clear();
    }
    inst.enabled = on;
    roots.save_state(&state)
}

/// Remove a plugin: what it registered, its folder and its state entry. Projects are never touched.
pub fn uninstall(roots: &Roots, id: &str) -> Result<(), String> {
    let mut state = roots.load_state();
    let inst = state.plugins.remove(id).ok_or_else(|| format!("no plugin '{id}' is installed"))?;
    unregister(roots, &inst.registered);
    let dir = roots.dir_of(id);
    if dir.exists() {
        std::fs::remove_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    roots.save_state(&state)
}

/// The pane slot of panel `key` ("plugin-id/panel-id"), assigned (and saved) the first time it is asked for.
pub fn slot_for(roots: &Roots, key: &str) -> Result<u16, String> {
    let mut state = roots.load_state();
    if let Some(i) = state.panel_slots.iter().position(|k| k == key) {
        return Ok(i as u16);
    }
    if state.panel_slots.len() >= u16::MAX as usize {
        return Err("too many plugin panels".into());
    }
    state.panel_slots.push(key.to_string());
    roots.save_state(&state)?;
    Ok((state.panel_slots.len() - 1) as u16)
}

/// "plugin-id/panel-id".
pub fn panel_key(plugin: &str, panel: &str) -> String {
    format!("{plugin}/{panel}")
}

/// Keep the user's folder choice for a `files: pick` plugin.
pub fn add_folder(roots: &Roots, id: &str, folder: &Path) -> Result<(), String> {
    let mut state = roots.load_state();
    let inst = state.plugins.get_mut(id).ok_or_else(|| format!("no plugin '{id}' is installed"))?;
    let f = folder.to_string_lossy().into_owned();
    if !inst.folders.contains(&f) {
        inst.folders.push(f);
    }
    roots.save_state(&state)
}

/// The GitHub link a plugin can be updated from: its install link, else its homepage when that is GitHub.
pub fn update_source(p: &Plugin) -> Option<github::Source> {
    [p.state.source.as_str(), p.manifest.homepage.as_str()].into_iter().find_map(|u| github::parse(u).ok().filter(|_| !u.is_empty()))
}

/// Download and stage `url` (blocking: a background thread). The staged copy waits for the confirm.
pub fn download(roots: &Roots, url: &str) -> Result<Staged, String> {
    let src = github::parse(url)?;
    let (bytes, tag) = github::fetch(&src)?;
    stage_zip(roots, &bytes, &src.path, url.trim(), &tag)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn scratch(name: &str) -> Roots {
        let base = std::env::temp_dir().join(format!("se-plugins-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        Roots { plugins: base.join("plugins"), library: base.join("library") }
    }

    /// The example plugin shipped in `examples/plugins/hello-panel`, as zip entries under a GitHub-style
    /// `hello-panel-main/` wrapper.
    pub(crate) fn example_zip(version: &str) -> Vec<u8> {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/plugins/hello-panel");
        let mut files: Vec<(String, Vec<u8>)> = Vec::new();
        fn walk(base: &Path, d: &Path, out: &mut Vec<(String, Vec<u8>)>) {
            for e in std::fs::read_dir(d).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    walk(base, &p, out);
                } else {
                    let rel = p.strip_prefix(base).unwrap().to_string_lossy().replace('\\', "/");
                    out.push((format!("hello-panel-main/{rel}"), std::fs::read(&p).unwrap()));
                }
            }
        }
        walk(&dir, &dir, &mut files);
        for (name, data) in &mut files {
            if name.ends_with(MANIFEST) {
                let mut m: Value = serde_json::from_slice(data).unwrap();
                m["version"] = Value::String(version.into());
                *data = serde_json::to_vec(&m).unwrap();
            }
        }
        let refs: Vec<(&str, &[u8])> = files.iter().map(|(n, d)| (n.as_str(), d.as_slice())).collect();
        zip::write(&refs)
    }
    use serde_json::Value;

    fn manifest(json: &str) -> Manifest {
        Manifest::parse(json).unwrap()
    }

    #[test]
    fn manifest_parses_and_validates() {
        let roots = scratch("validate");
        let dir = roots.plugins.join("src");
        std::fs::create_dir_all(dir.join("panels")).unwrap();
        std::fs::write(dir.join("panels/main.luau"), "function draw() end").unwrap();
        std::fs::write(dir.join("look.cube"), "LUT_3D_SIZE 2").unwrap();
        let ok = r#"{"id":"my-tool","name":"My Tool","version":"1.2.0","author":"Ann","minAppVersion":"0.1",
            "contents":{"panels":[{"id":"main","title":"My Tool","script":"panels/main.luau"}],"luts":["look.cube"]},
            "permissions":{"network":["api.example.com"],"files":["project"]}}"#;
        let m = manifest(ok);
        assert_eq!(m.min_app_version, "0.1");
        assert!(m.validate(&dir).unwrap().is_empty());
        assert_eq!(m.content_lines(), vec!["1 panel: My Tool".to_string(), "1 LUT".to_string()]);
        let bad = |patch: &str, why: &str| {
            let mut v: Value = serde_json::from_str(ok).unwrap();
            let p: Value = serde_json::from_str(patch).unwrap();
            for (k, val) in p.as_object().unwrap() {
                v[k] = val.clone();
            }
            let e = manifest(&v.to_string()).validate(&dir).unwrap_err();
            assert!(e.contains(why), "{patch}: {e}");
        };
        bad(r#"{"id":"My Tool"}"#, "lower-case");
        bad(r#"{"author":""}"#, "author");
        bad(r#"{"version":"beta"}"#, "number");
        bad(r#"{"minAppVersion":"99.0"}"#, "or newer");
        bad(r#"{"contents":{"scripts":["../outside.luau"]}}"#, "inside the plugin");
        bad(r#"{"contents":{"scripts":["missing.luau"]}}"#, "not in the plugin");
        bad(r#"{"contents":{"luts":["panels/main.luau"]}}"#, "cube");
        bad(r#"{"contents":{"fonts":[{"file":"look.cube","license":"OFL"}]}}"#, "ttf");
        std::fs::write(dir.join("f.ttf"), "x").unwrap();
        bad(r#"{"contents":{"fonts":[{"file":"f.ttf","license":"Proprietary"}]}}"#, "Open Font License");
        bad(r#"{"permissions":{"network":["*.example.com"]}}"#, "host name");
        bad(r#"{"permissions":{"files":["C:/"]}}"#, "project");
        bad(r#"{"contents":{"panels":[{"id":"a","title":"A","script":"panels/main.luau"},{"id":"a","title":"B","script":"panels/main.luau"}]}}"#, "two panels");
        assert!(Manifest::parse("{not json").is_err());
    }

    #[test]
    fn granted_permissions_never_exceed_the_declared_ones() {
        let declared = Permissions { network: vec!["a.com".into()], files: vec!["project".into()], run_programs: false, mcp_tools: vec![] };
        let asked = Permissions { network: vec!["a.com".into(), "evil.com".into()], files: vec!["pick".into()], run_programs: true, mcp_tools: vec!["export.video".into()] };
        let g = asked.within(&declared);
        assert_eq!(g.network, vec!["a.com".to_string()]);
        assert!(g.files.is_empty() && !g.run_programs && g.mcp_tools.is_empty());
    }

    #[test]
    fn install_update_disable_and_uninstall_from_a_zip() {
        let roots = scratch("lifecycle");
        let staged = stage_zip(&roots, &example_zip("1.0.0"), "", "https://github.com/someone/hello-panel", "v1.0.0").unwrap();
        assert_eq!(staged.manifest.id, "hello-panel");
        assert!(staged.replaces.is_none());
        assert!(list(&roots).is_empty(), "staging installs nothing");
        let none = Permissions::default();
        let p = commit(&roots, &staged, &none, true).unwrap();
        assert!(p.runnable());
        assert!(!staged.scratch.exists(), "the staging folder is cleaned up");
        assert!(p.granted().is_empty(), "nothing granted = nothing allowed, whatever plugin.json asks");
        // its LUT went into the library, under its own name
        let lut = roots.library.join("luts/hello-teal.cube");
        assert!(lut.is_file());
        assert_eq!(p.state.registered, vec!["lut/hello-teal".to_string()]);
        // disable takes the library content back out; enable puts it back
        set_enabled(&roots, "hello-panel", false).unwrap();
        assert!(!lut.exists());
        assert!(!list(&roots)[0].runnable());
        set_enabled(&roots, "hello-panel", true).unwrap();
        assert!(lut.is_file());
        // an update replaces the folder, keeps picked folders and says what it replaces
        add_folder(&roots, "hello-panel", Path::new("D:/Footage")).unwrap();
        let up = stage_zip(&roots, &example_zip("1.1.0"), "", "", "v1.1.0").unwrap();
        assert_eq!(up.replaces.as_deref(), Some("1.0.0"));
        let p = commit(&roots, &up, &none, true).unwrap();
        assert_eq!(p.manifest.version, "1.1.0");
        assert_eq!(p.state.folders, vec!["D:/Footage".to_string()]);
        assert_eq!(p.state.source, "https://github.com/someone/hello-panel", "an update keeps the link it came from");
        assert!(update_source(&p).is_some());
        // uninstall removes the folder, the registered items and the state entry
        uninstall(&roots, "hello-panel").unwrap();
        assert!(!lut.exists() && !roots.dir_of("hello-panel").exists() && list(&roots).is_empty());
        assert!(uninstall(&roots, "hello-panel").is_err());
    }

    #[test]
    fn a_plugin_never_overwrites_the_users_own_library_items() {
        let roots = scratch("collide");
        let mine = roots.library.join("luts/hello-teal.cube");
        std::fs::create_dir_all(mine.parent().unwrap()).unwrap();
        std::fs::write(&mine, "mine").unwrap();
        let staged = stage_zip(&roots, &example_zip("1.0.0"), "", "", "").unwrap();
        let p = commit(&roots, &staged, &Permissions::default(), true).unwrap();
        assert_eq!(p.state.registered, vec!["lut/hello-panel-hello-teal".to_string()]);
        uninstall(&roots, "hello-panel").unwrap();
        assert_eq!(std::fs::read_to_string(&mine).unwrap(), "mine", "the user's item is untouched");
    }

    #[test]
    fn a_zip_without_plugin_json_or_with_a_bad_one_installs_nothing() {
        let roots = scratch("badzip");
        let z = zip::write(&[("repo-main/readme.md", b"hi")]);
        assert!(stage_zip(&roots, &z, "", "", "").unwrap_err().contains("plugin.json"));
        let z = zip::write(&[("plugin.json", br#"{"id":"x y"}"#)]);
        assert!(stage_zip(&roots, &z, "", "", "").is_err());
        assert!(list(&roots).is_empty());
        let staging = roots.plugins.join(".staging");
        assert!(std::fs::read_dir(&staging).map(|d| d.count()).unwrap_or(0) == 0, "failed stages are cleaned up");
    }

    #[test]
    fn a_subfolder_link_finds_its_plugin() {
        let roots = scratch("subfolder");
        let z = zip::write(&[
            ("mono-main/readme.md", b"x"),
            ("mono-main/plugins/tiny/plugin.json", br#"{"id":"tiny","name":"Tiny","version":"1","author":"A","contents":{"scripts":["s.luau"]}}"#),
            ("mono-main/plugins/tiny/s.luau", b"print(1)"),
        ]);
        let s = stage_zip(&roots, &z, "plugins/tiny", "", "").unwrap();
        assert_eq!(s.manifest.id, "tiny");
        s.discard();
    }
}
