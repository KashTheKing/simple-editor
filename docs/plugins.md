# Plugins: format and runtime spec

A plugin is a folder (or a zip of one) with a `plugin.json` at its root. It can bundle Luau scripts, Luau
**panels** (dockable panes drawn with a small UI API), Assistant skills, and library content: shader effects
and transitions, LUTs, HTML templates, icons and OFL fonts. Users install plugins from a GitHub link, a zip or
a folder; nothing a plugin ships runs until the user opens one of its panels or runs one of its scripts.

The user guide is `website/docs/plugins.mdx`. A complete example is `examples/plugins/hello-panel/`. The code
is `src/engine/plugins/` (format, install, sandbox, panels) and `src/ui/app/plugins_ctl.rs` (the app side).

## Layout

```
my-plugin/
  plugin.json
  panels/main.luau
  scripts/tidy.luau
  effects/glow.glsl
  luts/teal.cube
  fonts/MyFont-Bold.ttf     (+ its OFL licence text, recommended)
```

Paths in `plugin.json` are relative to the folder, use `/`, and must stay inside it (no `..`, no drive or root).

## plugin.json

```json
{
  "id": "hello-panel",
  "name": "Hello Panel",
  "version": "1.0.0",
  "author": "Your Name",
  "description": "One or two sentences shown in the Plugins pane and the install confirm.",
  "min_app_version": "1.4.0",
  "homepage": "https://github.com/you/hello-panel",
  "icon": "bolt",
  "contents": {
    "panels":      [{ "id": "main", "title": "Hello", "script": "panels/hello.luau", "icon": "bolt" }],
    "scripts":     ["scripts/marker-at-every-cut.luau"],
    "skills":      ["skills/hello.md"],
    "effects":     ["effects/glow.glsl"],
    "transitions": ["transitions/swipe.glsl"],
    "luts":        ["luts/hello-teal.cube"],
    "templates":   ["templates/lower-third.html"],
    "icons":       ["icons/star.svg"],
    "fonts":       [{ "file": "fonts/MyFont-Bold.ttf", "license": "OFL-1.1" }]
  },
  "permissions": {
    "network": ["api.example.com"],
    "files": ["project", "pick"],
    "run_programs": false,
    "mcp_tools": ["export.video"]
  }
}
```

| Field | Rule |
|---|---|
| `id` | 2-64 characters: `a-z`, `0-9`, `-`; not starting or ending with `-`. The install folder's name. |
| `name`, `version`, `author` | Required. `version` starts with a number (`1.0.0`, `v2`). |
| `description`, `homepage`, `icon` | Optional. `icon` is one of the app's icon names (`bolt`, `wrench`, `terminal`, `star`, ...). |
| `min_app_version` | Optional (`minAppVersion` is read too). Older apps refuse to install the plugin. |
| `contents.*` | Every listed file must exist with the right extension: scripts and panels `.luau`/`.lua`, skills `.md`, effects (`shaders` is the same list) and transitions `.glsl`, LUTs `.cube`, templates `.html`, icons `.svg`, fonts `.ttf`/`.otf`. |
| `contents.panels[]` | `id` unique in the plugin (`a-z0-9-`), `title` shown on the tab, `script` the panel's Luau, `icon` optional. |
| `contents.fonts[]` | `license` must be `OFL-1.1` (or `OFL`): only SIL Open Font License fonts can be bundled. |
| `permissions.network` | Exact host names. No wildcards, schemes or paths. |
| `permissions.files` | `"project"` and/or `"pick"`. |
| `permissions.mcp_tools` | App tool names (`tools/list`). Unknown names install with a warning. |

## What installing does

1. **Download** (Install from URL): the link is read as a repository (`github.com/owner/repo`), a release
   (`.../releases/tag/v1.2`), the latest release (`.../releases/latest`), or a folder on a branch
   (`.../tree/main/plugins/hello`). For a repository or release, the GitHub API's release is asked for its
   first `.zip` asset; without one, the release tag's source archive is used; with no release at all (or a
   `tree/` link), the branch archive from `codeload.github.com`. Downloads use `curl.exe` with a generic user
   agent, at most 256 MB.
2. **Unpack and check**, into `plugins\.staging\`: the zip reader refuses entries that point outside the
   folder, encrypted or zip64 archives and more than 256 MB unpacked. `plugin.json` is found at the top, in the
   single wrapper folder GitHub adds (`repo-main/`), or in the link's folder; it is parsed and validated.
3. **Confirm**: the user sees the name, author, version (and the one it replaces), the source link, the
   contents and each permission as a checkbox (`run_programs` unticked, with a warning when ticked), plus
   "Only install plugins from people you trust". Nothing is installed without **Install**, including from
   the `plugins.install` tool.
4. **Install**: the folder moves to `%APPDATA%\SimpleEditor\plugins\<id>\` (replacing an older copy), the
   granted permissions go into `plugins\state.json`, and library content is copied into the library
   (`library\effects`, `luts`, ...). An item whose name the user already has gets the plugin id as a prefix
   (`hello-panel-teal.cube`); the plugin never overwrites the user's own files. Fonts load from the plugin
   folder while it is enabled.

**Disable** removes the library copies (panels and scripts stop); **Enable** copies them back. **Uninstall**
removes them, the plugin folder and its state entry; projects are never touched. **Update** asks the GitHub
API for the latest release and, when its tag is newer than the installed one, installs it through the same
confirm (picked folders are kept).

`plugins\state.json` holds, per plugin: `enabled`, `source` (the link), `tag`, `granted`, `folders` (picked)
and `registered` (library ids it added); and `panel_slots`, the panel each `Pane::Plugin(n)` shows, so stored
layouts reopen plugin panels after a restart.

## Running code

Plugin scripts and panels run in their own Luau VM with the same sandbox as the Assistant's scripts:

- no `io`, `require`, `package`, `load`/`loadstring`, `dofile`, `debug`, `getfenv`/`setfenv`,
  `collectgarbage`; `os` has `clock`, `time`, `date` and `difftime` only;
- 64 MB of memory, and a wall-clock deadline enforced by Luau's interrupt (see Limits).

They get the project API the user's scripts have:

| API | |
|---|---|
| `editor.tool(name, args)` | Call an app tool. Any project / library read or edit is allowed (the same list `scripts.eval` uses); other tools only if named in `mcp_tools` and granted. |
| `editor.tools()` | The tools this plugin may call, with descriptions. |
| `editor.log(...)` / `print(...)` | A line for the user: toasts for scripts, under the panel for panels. |
| `editor.json(v)` | A value as JSON text. |
| `library.*`, `project.*`, `query.*` | The helpers from `scripting::PRELUDE` (`project.clips()`, `library.assets(filter)`, `query.filter`, ...). |
| `plugin.id`, `plugin.name`, `plugin.version` | Who is running. |

And, with their permission:

| API | Needs | |
|---|---|---|
| `net.get(url)` | `network` with the URL's host granted | Non-blocking: `nil` while the download runs, then `{ok=true, status, body}` or `{ok=false, error}`. A panel redraws when it lands. Each URL is fetched once per panel load (Reload fetches again); bodies up to 8 MB. |
| `fs.read(path)`, `fs.write(path, text)`, `fs.list(dir)`, `fs.exists(path)` | `files` | Relative paths are inside the open project's folder. `project` = that folder (the project must be saved); `pick` = folders the user added in the Plugins pane. `..` is refused; resolved paths must stay inside. Reads up to 16 MB. |
| `programs.run(exe, {args})` | `run_programs` | Starts a program; returns its process id. |

A call without its permission fails with a message naming the plugin and the missing permission (whether it
wasn't declared in `plugin.json` or the user didn't grant it). Tool calls that edit the project are grouped:
one script run, or one panel draw, is **one undo step** labelled with the plugin and the panel or script; a run
that fails is rolled back.

### Scripts

A script runs top to bottom when the user picks it in **Window ▸ Plugins ▸ <plugin>**, presses its button in
the Plugins pane, or presses **Run** on it in the Script Editor. The Script Editor's Run on any file inside an
installed plugin goes through the plugin's sandbox, never the full-access path of the user's own scripts.
Plugin scripts don't take `-- @on` hooks or hotkeys in this version.

### Panels

The panel's file runs once when the panel opens (its top level sets up state), and must define a global
`draw()`. `draw()` records widgets; the app draws them natively, forms in two aligned columns:

| Call | Returns |
|---|---|
| `ui.heading(text)`, `ui.label(text)` | |
| `ui.button(label)` | `true` on the draw after it was clicked |
| `ui.slider(label, value, min, max)` | the value (the user's new one after a drag) |
| `ui.text(label, value, multiline?)` | the text |
| `ui.checkbox(label, value)` | the state |
| `ui.dropdown(label, value, {options})` | the chosen option |
| `ui.row(fn)`, `ui.column(fn)` | lay out the widgets `fn` makes side by side / stacked |
| `ui.separator()`, `ui.space(px?)` | |
| `ui.image(file, w?, h?)` | a `.png` inside the plugin (decoded once) |

Widgets are identified by their label; repeats get `#2`, `#3`, and `"Label##key"` shows `Label` with the id
`##key` (as in Dear ImGui). Keep values in Luau locals and pass them back in, immediate-mode style:

```lua
local size = 0.5
function draw()
  size = ui.slider("Size", size, 0, 1)
  if ui.button("Apply") then editor.tool("clip.set", { clip_id = 1, fields = { scale = size } }) end
end
```

`draw()` does **not** run every frame. It runs when the panel opens, when the user interacts with it (and once
more right after, so widgets above a clicked button show the new state), when the project is edited or the
selection changes, when a `net.get` lands, and when the script file changes on disk (the panel reloads). An
idle panel costs nothing, and playback never makes it run.

### Limits

| | |
|---|---|
| A panel's `draw()` with nothing to answer | 4 ms |
| A panel's `draw()` answering a click or edit (it may run tools) | 250 ms |
| Loading a panel's file | 250 ms |
| A script run | 5 s |
| Widgets per draw | 2000 |
| Memory per VM | 64 MB |

Over a limit, the panel stops and shows the error with **Retry**, **Edit Script** and **Open Folder**; a
script's error is a toast. Runs happen on the UI thread, which is why the budgets are strict.

## Security model

Plugins are community code, so the design assumes a plugin may be hostile:

- installing never runs code; the confirm shows everything the plugin asks for, and each permission can be
  refused; `run_programs` is off unless the user ticks it under a warning;
- permissions are enforced at run time against what was **granted and is still declared** (an updated
  `plugin.json` can't widen access without a new confirm);
- the Luau sandbox has no file, process or network access except through the API above;
- zips are unpacked with path checks, size caps and CRC checks; library content never overwrites the user's
  files; uninstall removes only what the plugin registered;
- downloads send no personal data (a generic user agent, no tokens).

What a granted permission allows is real: a plugin with `files: project` can rewrite files in the project
folder, and one with `run_programs` can do anything the user can. The confirm says so.

## MCP

| Tool | |
|---|---|
| `plugins.list` | Installed plugins with contents, granted permissions, panels (open or not) and scripts. |
| `plugins.install {url}` | Downloads, then shows the user the install confirm. Never installs silently. |
| `plugins.uninstall {id}` | Asks the user (the same confirm as the Uninstall button). |
| `plugins.enable {id, on}` | Turns a plugin on or off. |

None of them is callable from a plugin or from `scripts.eval`.
