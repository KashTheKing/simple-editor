# Notes — Simple Editor

Freeform scratchpad: decisions, gotchas, things worth remembering that don't belong in
[agents.md](agents.md) (workflow) or [goals.md](goals.md) (priorities/roadmap). Append via
`/se-notes`, by editing this file directly, or by telling an agent to jot something down.
Newest at the top. No required format — a bullet or a short paragraph is fine.

---

- **Playback-cache work (PRs #12+#14), gotchas worth keeping:** `Cache::avg_entry_bytes` must
  refuse empty/zero-byte caches — an empty timeline's GPU LayerSets insert at 0 bytes and the
  horizon math would divide by zero. `DecoderPool::set_proxies` keys decoders by RESOLVED path, so
  a remap must drop the source, its OLD proxy and its NEW proxy keys. The proxy "building" badge
  is set/cleared entirely inside the build job via a drop guard — setting it after `spawn_job`
  returns would race a fast-failing job and stick forever. The source-frame cache keys by (source
  path, exact f64 µs, w, h): replays hit because callers re-derive bit-identical times from the
  fps grid; animated-scale clips request a new size every frame and never hit (commented ceiling).
  History labels are derived from an entry's NEXT neighbour — any delete must clear the label
  cache (Sonnet review caught this; test pins it).
