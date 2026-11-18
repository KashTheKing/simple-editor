---
name: organize-library
description: Sort the media library into bins - Footage, SFX, Music, Voiceover, Images, Graphics, Unused - and tag every asset. Nothing is deleted; one Undo puts it back.
args: "[what to change, e.g. 'keep my bins' or 'music over 30 s']"
---
Organise the user's media library into clear bins with one Luau script. Never delete or remove an asset, never touch the timeline, and never ask for shell or file access: the library tools and `scripts.eval` reach everything needed.

1. Call `media.summary`. It gives the counts by kind, extension and bin, used / unused, and the bin tree. Do not call `media.list` without filters on a big library. If the library is empty, say so and stop.
2. Adapt the script below to $ARGS if it asks for something (other bin names, other keywords, a different music length). Keep its rules otherwise:
   - Video: Footage (Graphics when its name or bin says overlay, title, logo, graphics...). Audio: Voiceover (voice words), Music (music words, or 45 s and longer), else SFX. Images: Graphics (logos, icons, overlays, titles, GIF / SVG, HTML layers), else Images.
   - An asset that no timeline uses goes under `Unused/<bin>` - except sounds: SFX and Music are a library to draw from, so they stay in their bin, tagged `unused`.
   - The user's own bins are kept inside the new ones (`Day 1` becomes `Footage/Day 1`); a bin whose name already says the category is folded into it (`7 Sound effects/Hits` becomes `SFX/Hits`). Assets already under one of the new bins stay where they are. Bins left empty afterwards are removed (bins only - their assets have all moved).
   - Every asset gets its bin's tag (`footage`, `sfx`, `music`, `voiceover`, `image`, `graphic`), plus `unused` when unused.
3. Run it once with `scripts.eval {purpose: "Organize library", dry_run: true, code}` and read the plan it prints. If it looks wrong (say, most audio landed in Music), adjust the keywords and dry-run again.
4. Run it for real: the same call without `dry_run`. It is one undo step, "Script: Organize library".
5. Call `media.summary` again to confirm the bins.

```lua
local CATS = {Footage = "footage", SFX = "sfx", Music = "music", Voiceover = "voiceover", Images = "image", Graphics = "graphic", Unused = "unused"}
local WORDS = {
  sfx = {"sfx", "fx", "whoosh", "swoosh", "swish", "hit", "impact", "click", "pop", "boom", "riser", "ding", "glitch", "punch", "bell", "notification", "sound effect", "transition", "meme"},
  music = {"music", "song", "bgm", "beat", "instrumental", "soundtrack", "ost", "theme", "remix", "lofi", "phonk", "loop"},
  voice = {"voiceover", "voice over", "vo", "narration", "dialogue", "voice", "vocal", "recording"},
  graphic = {"logo", "icon", "overlay", "title", "lower third", "lowerthird", "graphic", "graphics", "badge", "button", "banner", "sticker", "emoji", "thumbnail", "mockup", "card", "cards", "template", "watermark"},
}
local MUSIC_SECONDS = 45
-- an old top bin whose name says the category is folded into it ("7 Sound effects" -> "SFX")
local ALIASES = {
  Footage = {"footage", "clips", "recordings", "videos"}, SFX = {"sfx", "sound effects", "sound effect", "sounds"},
  Music = {"music", "songs"}, Voiceover = {"voiceover", "voice over", "vo", "narration"},
  Images = {"images", "pictures", "photos", "screenshots"}, Graphics = {"graphics", "overlays", "titles"}, Unused = {"unused"},
}

-- whole words only: "hit" must not match "white"
local function words(s) return " " .. (string.gsub(string.lower(s or ""), "[^%w]+", " ")) .. " " end
local function any(text, list)
  for _, k in ipairs(list) do
    if string.find(text, " " .. k .. " ", 1, true) or string.find(text, " " .. k .. "s ", 1, true) then return true end
  end
  return false
end

local function category(a)
  local t = words(a.name .. " " .. a.folder .. " " .. table.concat(a.tags, " ") .. " " .. a.description)
  if a.kind == "audio" then
    if any(t, WORDS.voice) then return "Voiceover" end
    if any(t, WORDS.music) or a.duration >= MUSIC_SECONDS then return "Music" end
    return "SFX"
  elseif a.kind == "video" then
    if a.ext == "gif" or any(t, WORDS.graphic) then return "Graphics" end
    return "Footage"
  elseif a.kind == "html" then
    return "Graphics"
  end
  if a.ext == "svg" or a.ext == "gif" or any(t, WORDS.graphic) then return "Graphics" end
  return "Images"
end

local plan, moves, counts, old_tops = {}, {}, {}, {}
for _, a in ipairs(library.assets()) do
  local cat = category(a)
  local unused = not a.used
  local top = string.match(a.folder, "^[^/]+") or ""
  local trimmed = string.match(top, "^%s*(.-)%s*$")
  local target = a.folder
  if CATS[trimmed] == nil then
    local base = cat
    if unused and cat ~= "SFX" and cat ~= "Music" then base = "Unused/" .. cat end
    local keep = a.folder
    -- fold an old top bin that already names this category (or "unused") into the new bin
    if any(words(top), ALIASES[cat]) or any(words(top), ALIASES.Unused) then
      keep = string.match(a.folder, "^[^/]+/(.*)$") or ""
    end
    target = (keep ~= "" and (base .. "/" .. keep)) or base
    if top ~= "" then old_tops[top] = true end
  end
  local tags = {CATS[cat]}
  if unused then table.insert(tags, "unused") end
  local key = target .. "|" .. table.concat(tags, ",")
  plan[key] = plan[key] or {folder = target, tags = tags, ids = {}}
  table.insert(plan[key].ids, a.id)
  counts[target] = (counts[target] or 0) + 1
  if target ~= a.folder then table.insert(moves, a.name .. ": " .. (a.folder == "" and "(root)" or a.folder) .. " -> " .. target) end
end

for _, p in pairs(plan) do library.set(p.ids, {folder = p.folder, add_tags = p.tags}) end

-- the user's old top-level bins that are now empty (their assets all moved): bins only, never assets
local removed = {}
for _, b in ipairs(library.bins()) do
  if (old_tops[b.bin] or old_tops[b.bin .. " "]) and b.total == 0 then
    library.delete_bin(b.bin)
    table.insert(removed, b.bin)
  end
end

print("Moved " .. #moves .. " assets.")
for _, bin in ipairs(query.keys(counts)) do print(string.format("%-40s %d", bin, counts[bin])) end
for i = 1, math.min(#moves, 25) do print("  " .. moves[i]) end
if #moves > 25 then print("  ... and " .. (#moves - 25) .. " more") end
if #removed > 0 then print("Removed empty bins: " .. table.concat(removed, ", ")) end
return {moved = #moves, bins = counts, removed_bins = removed}
```

Request: $ARGS

Report back: how many assets moved and into which bins (the counts per bin), which empty bins were removed, that sequences stay in the library's Sequences list (they have no bin), that nothing was deleted, and that one Undo ("Script: Organize library") puts everything back.
