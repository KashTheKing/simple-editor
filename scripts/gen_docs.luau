-- @name Tool Catalogue Spot-check
-- @desc Dumps editor.tools()'s count + first few names as a toast, to eyeball against a live
--       `curl` of tools/list (docs/customizing.md's transcription source). NOT a way to regenerate
--       that doc: editor.log() is a 5-10s auto-expiring toast with no copy button, and
--       editor.tools() only returns {name, description} — no kind/args — so it can't produce the
--       full table either way. Copy this file into %APPDATA%\SimpleEditor\scripts and run it from
--       the Scripts menu (or `scripts.run` over MCP) after a `curl` against tools/list, then check:
--       does the count match, and do the first few names look right?
--
-- ponytail: a full markdown table here would either overflow the toast or get truncated before
-- anyone could read it (236 tools at wave-3-complete) — a count + sample is the useful signal a
-- human can actually verify in 5-10 seconds. Full transcription happens by hand from curl's JSON
-- (see docs/customizing.md); this script is a smoke test for that transcription, not a source for it.

local tools = editor.tools()

local sample = {}
for i = 1, math.min(8, #tools) do
	table.insert(sample, tools[i].name)
end

editor.log(#tools .. " tools via editor.tools(); first " .. #sample .. ": " .. table.concat(sample, ", "))
