-- @name Marker at every cut
-- @desc Adds a marker where each clip on the timeline starts (one undo step).
local seen = {}
for _, c in ipairs(project.clips()) do
  local t = math.floor(c.start * 1000 + 0.5) / 1000
  if not seen[t] then
    seen[t] = true
    tool("markers.add", { t = t, name = "Cut" })
  end
end
print("Added " .. #query.keys(seen) .. " markers")
