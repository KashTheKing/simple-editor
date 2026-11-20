-- Hello Panel: the example plugin panel (see docs/plugins.md).
-- The file runs once when the panel opens; draw() runs whenever something changed:
-- a click, an edit to the project, a download landing. Keep draw() quick (a few ms).

local note = "Plugin marker"
local step = 0.5
local count_markers = true

function draw()
  local clips = project.clips()
  local ctx = editor.tool("editor.context", {})
  local selected = ctx.selected_clips or {}

  ui.heading("Hello from a plugin")
  ui.label(#clips .. " clips on the timeline, " .. #selected .. " selected")
  if count_markers then
    ui.label(#project.markers() .. " markers")
  end
  ui.separator()

  note = ui.text("Marker name", note)
  step = ui.slider("Nudge (s)", step, 0.1, 2)
  count_markers = ui.checkbox("Count markers", count_markers)

  ui.row(function()
    if ui.button("Marker at playhead") then
      editor.tool("markers.add", { t = ctx.playhead or 0, name = note })
    end
    if ui.button("Nudge selection") and #selected > 0 then
      -- every edit made in one click is one undo step
      editor.tool("timeline.move", { clip_ids = query.ids(selected), dt = step })
    end
  end)
end
