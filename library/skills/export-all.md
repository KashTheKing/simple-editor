---
name: export-all
description: Export the video for one or more platforms using the preset list (asks first).
args: "[platforms, e.g. 'youtube, shorts', or a folder]"
---
Queue exports for the platforms the user wants. Exports write files and take time, so this skill always asks before it starts.

1. Call `editor.context`, `project.summary` (length, size, name) and `export.presets` (the list of preset names with extension, size and loudness settings). Check `export.status` for exports already running or queued.
2. Choose targets from $ARGS by matching preset names (a platform name like "youtube" or "shorts" picks the preset with that name). If $ARGS is empty or matches nothing, show the preset list and ask which ones. A vertical preset on a landscape project would crop badly: warn, and suggest `/shorts` first.
3. Range: use the whole timeline unless In/Out is set (`timeline.in_out`). If it is set, ask whether to export just that range (`range_in`, `range_out`).
4. Output folder: from $ARGS if it names one; otherwise propose the project's folder (from `project.summary`). File names: `<project name> - <preset>.<ext>`. If a file with that name may already exist, add a number; never overwrite.
5. Before starting, show the plan in a short list (preset, file path, estimated length) and ask "Export these N files?". Wait for a clear yes. With the safety setting on, the app shows its own Allow prompt as well; tell the user to expect it.
6. After the yes: for each target `export.queue {preset, path, range_in, range_out}`. Queued jobs run in order in the background; use `export.video {path, ...}` only for a single export the user wants to wait for. Do not use `export.quick` unasked (it silently reuses the last preset).
7. Report progress with `export.status`, and `jobs.list` if asked. Do not edit the project while an export runs. If an export fails, read the error from `jobs.list` and tell the user plainly.

Platforms requested: $ARGS

Report back: which files are queued (full paths), the preset for each, and that exports can be cancelled from the Jobs pane (`jobs.cancel`). Exporting does not change the project, so there is nothing to undo.
