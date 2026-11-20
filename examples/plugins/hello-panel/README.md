# Hello Panel - an example Simple Editor plugin

A complete, small plugin: one panel, one script and one LUT. Read `docs/plugins.md` (or the
"Plugins" page of the guide) for the format.

- `plugin.json` - the manifest: id, name, version, author, what it contains, what it may do.
- `panels/hello.luau` - the **Hello** panel: counts clips, drops a named marker at the playhead,
  nudges the selected clips.
- `scripts/marker-at-every-cut.luau` - a script run from Window ▸ Plugins ▸ Hello Panel.
- `luts/hello-teal.cube` - installed into the library's LUTs while the plugin is enabled.

It asks for no permissions: it only reads and edits the open project.

Install it from a folder (Plugins pane ▸ Install from Folder…) while you work on it, or publish the
folder as a GitHub repository (or a release with a zip of it) and install it from the link.
