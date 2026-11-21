-- Visualizer Helper: the five steps of building a music visualizer, top to bottom, each one button.
-- The heavy work (stem separation, analysis, keys, the background loop) runs in the app, in the background;
-- this panel only calls its tools with tool(). Every click is one undo step.
--
--   1. Analyse Song          audio.analyze_song        hits, drops, the calm part, the voice (stored on the project)
--   2. Visualizer Templates  visualizer.template  the spiky chrome ball / waveform / lightning, keyed from it
--   3. Lyrics Karaoke        lyrics.karaoke       the exact lyrics, each word filling on its sung syllable
--   4. Drop FX               visualizer.drop_fx   flash + invert at the drops, the shatter into the biggest one
--   5. Background Loop       media.loop           a low-fps GIF or still as a seamless 60 fps loop

-- step 1
local tempo_min, tempo_max, gate = 130, 175, 18
local drop_text = ""
local after = nil -- from the lyrics: a [DROP] header means no drop before that line has ended
-- step 2
local template, palette = "Olympus Storm", "Silver to Gold"
local spikes, sensitivity, smoothing, harder = 4, 1, 0, 2.2
-- step 3
local lyrics_text, font, size, look, giant = "", "Palatino Linotype", 58, "Normal", true
-- step 4
local flash, invert, shatter, grade, build = true, true, true, true, 12.7
-- step 5
local seconds = 8

local said = {} -- the last thing each step said (its result or why it failed)

local function mmss(t)
  if type(t) ~= "number" then return "?" end
  return string.format("%d:%04.1f", math.floor(t / 60), t % 60)
end

-- run a step's tool; what it says (or the error, without the Luau position) becomes the step's status
local function step(n, name, args, ok)
  local good, r = pcall(tool, name, args)
  if good then
    said[n] = ok(r)
  else
    said[n] = string.gsub(tostring(r), "^.-:%d+: ", "")
  end
  return good and r or nil
end

local function drops()
  local out = {}
  for v in string.gmatch(drop_text, "[%d%.:]+") do
    local m, s = string.match(v, "^(%d+):([%d%.]+)$")
    local t = m and (tonumber(m) * 60 + tonumber(s)) or tonumber(v)
    if t then table.insert(out, t) end
  end
  return out
end

function draw()
  local good, st = pcall(tool, "audio.analyze_song", {})
  ui.heading("Visualizer Helper")
  if not good then
    ui.label(string.gsub(tostring(st), "^.-:%d+: ", ""))
    return
  end
  ui.label("Song: " .. st.name)
  ui.tip("The song the steps work on: the selected song clip, else the one already analysed, else the longest audio clip")
  local a = st.analysis
  local made = st.made or {}

  -- 1 ---------------------------------------------------------------------------------------------------
  ui.separator()
  ui.label("1. Analyse Song")
  tempo_min = ui.slider("Tempo from", tempo_min, 60, 200)
  ui.tip("Slowest tempo to look for (BPM). Hardstyle sits around 150")
  tempo_max = ui.slider("Tempo to", tempo_max, 80, 240)
  ui.tip("Fastest tempo to look for (BPM)")
  gate = ui.slider("Kick gate", gate, 6, 36)
  ui.tip("How quiet a kick may be and still pop the visualizer (dB under the loudest kicks). Lower it if hats and claps pop it")
  drop_text = ui.text("Drop times", drop_text)
  ui.tip("Leave empty to find the drops. To correct one, type the times, e.g. 0:26.4, 0:52.7")
  if ui.button("Analyse Song") then
    local args = { action = "start", tempo_min = tempo_min, tempo_max = tempo_max, gate = gate, after = after }
    local d = drops()
    if #d > 0 then args.drops = d end
    step(1, "audio.analyze_song", args, function() return "Analysing in the background (Jobs shows the progress)" end)
  end
  ui.tip("Separates the stems (when Stem Separation is installed), then finds the hits, drops, the calm part and the voice. Runs in the background; the song stays playable")
  if st.state == "running" then
    ui.label(string.format("Analysing: %s %d%%", st.step or "", math.floor((st.progress or 0) * 100)))
  elseif st.state == "failed" then
    ui.label("Failed: " .. (st.error or "?"))
  elseif a then
    local dl = {}
    for _, d in ipairs(a.drops) do table.insert(dl, mmss(d)) end
    ui.label(string.format("%d BPM · %d hits · drops %s", math.floor(a.bpm + 0.5), a.hits, #dl > 0 and table.concat(dl, ", ") or "none"))
    ui.tip(string.format("First vocal at %s, calm part from %s. %s", mmss(a.first_vocal), mmss(a.calm), a.from_stems and "From the stems." or "From the mix: install Stem Separation for exact vocals and kicks."))
  else
    ui.label(said[1] or "Not analysed")
  end
  if not st.stems.installed then
    if ui.button("Get Stem Separation") then tool("audio.analyze_song", { action = "stems" }) end
    ui.tip("Opens Settings ▸ Add-ons. With the stems, the voice and the kicks are found far more exactly")
  end

  -- 2 ---------------------------------------------------------------------------------------------------
  ui.separator()
  ui.label("2. Visualizer Templates")
  template = ui.dropdown("Template", template, st.templates)
  ui.tip("Olympus Storm: the spiky chrome ball, the waveform and lightning. Chrome Waveform: only the waveform. Storm Lightning: lightning and beams")
  palette = ui.dropdown("Colours", palette, st.palettes)
  ui.tip("The metal before and after the first drop, and the electricity")
  spikes = ui.slider("Spikes", spikes, 1, 6)
  ui.tip("Spines popped per hit")
  sensitivity = ui.slider("Sensitivity", sensitivity, 0.25, 2)
  ui.tip("How readily a hit pops: below 1 only the big hits, above 1 the small ones too")
  smoothing = ui.slider("Smoothing", smoothing, 0, 1)
  ui.tip("Smooths the voice and synth movement")
  harder = ui.slider("Go harder", harder, 1.2, 3)
  ui.tip("How hard the kicks hit from the second drop to the calm part (1.2 = no change)")
  if ui.button("Apply Template") then
    step(2, "visualizer.template", {
      template = template, palette = palette, spikes = spikes, sensitivity = sensitivity,
      smoothing = smoothing, harder = harder,
    }, function(r) return r.template .. " on the visualizer" end)
  end
  ui.tip("Makes the song's visualizer if it has none and keys it from the analysis. The baked audio keys stay")
  ui.label(said[2] or (made.visualizer and ((made.template ~= "" and made.template or "A template") .. " on the visualizer")) or (a and "No template yet" or "Analyse the song first"))

  -- 3 ---------------------------------------------------------------------------------------------------
  ui.separator()
  ui.label("3. Lyrics Karaoke")
  lyrics_text = ui.text("Lyrics", lyrics_text, true)
  ui.tip("Paste the lyrics exactly as sung, one on-screen line per line. [Verse] headers and blank lines are skipped; a [Drop] header tells it where the drop is")
  font = ui.text("Font", font)
  ui.tip("The font family")
  size = ui.slider("Size", size, 16, 160)
  ui.tip("The font size. The text is drawn at this size, never scaled up, so it stays sharp")
  look = ui.dropdown("Letters", look, { "Normal", "Greek" })
  ui.tip("Greek: A, E, O and Y drawn as Λ, Σ, Θ and Ψ")
  giant = ui.checkbox("Giant line", giant)
  ui.tip("The line that ends into the biggest drop wraps round a cylinder")
  if ui.button("Make Lyrics") then
    local r = step(3, "lyrics.karaoke", { lyrics = lyrics_text, font = font, size = size, look = string.lower(look), giant = giant },
      function(r) return #r.lines .. " lines" .. (r.transcript and ", anchored on the transcript" or "") end)
    if r and r.drop_after then
      after = r.drop_after
      said[3] = said[3] .. ". The lyrics put the drop after " .. mmss(after) .. ": Analyse Song again to use it"
    end
  end
  ui.tip("Times every word on the vocal stem's syllables and makes one text clip per line with the Lyric Chrome shader")
  ui.label(said[3] or ((made.lyrics or 0) > 0 and (made.lyrics .. " lines") or "No lyrics yet"))

  -- 4 ---------------------------------------------------------------------------------------------------
  ui.separator()
  ui.label("4. Drop FX")
  flash = ui.checkbox("Flash", flash)
  ui.tip("A bright flash on each drop")
  invert = ui.checkbox("Invert", invert)
  ui.tip("The colours inverted for one beat on each drop (the Invert effect)")
  shatter = ui.checkbox("Shatter", shatter)
  ui.tip("Before the biggest drop the light is sucked into the ball and glass shards invert on every hit (needs the Olympus Storm template)")
  build = ui.slider("Shatter build", build, 2, 30)
  ui.tip("Seconds the shatter builds before the biggest drop")
  grade = ui.checkbox("Grade background", grade)
  ui.tip("The Background Loop gets a golden-hour look: dark until the first drop, a gold duotone through the biggest drop")
  if ui.button("Add Drop FX") then
    step(4, "visualizer.drop_fx", { flash = flash, invert = invert, shatter = shatter, build = build, grade = grade },
      function(r) return #r.drops .. " drops" .. (r.shatter and ", shatter into the biggest" or "") .. (r.note and (" (" .. r.note .. ")") or "") end)
  end
  ui.tip("Puts the effects on a Drop FX track on top. Running it again replaces them")
  ui.label(said[4] or ((made.fx or 0) > 0 and (made.fx .. " drop effects" .. ((made.shatter or 0) > 0 and ", shatter on" or "")) or "No drop effects yet"))

  -- 5 ---------------------------------------------------------------------------------------------------
  ui.separator()
  ui.label("5. Background Loop")
  seconds = ui.slider("Loop length", seconds, 2, 20)
  ui.tip("Seconds one way; the loop plays forward then back, so it is twice this")
  if ui.button("Make Loop") then
    step(5, "media.loop", { action = "start", seconds = seconds }, function() return "Making the loop in the background" end)
  end
  ui.tip("Select a GIF, picture or clip first. It is smoothed to 60 fps and ping-ponged so the loop point never jumps, then tiled under the song")
  local loops = st.loops or {}
  if #loops > 0 then
    ui.label(string.format("Looping: %s %d%%", loops[1].step or "", math.floor((loops[1].progress or 0) * 100)))
  else
    ui.label(said[5] or ((made.background or 0) > 0 and (made.background .. " loop clips under the song") or "Select a GIF or picture"))
  end
end
