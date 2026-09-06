# Player Rate Loop

**Workstream:** `player-rate-loop` · **Issue:** [#25](https://github.com/KashTheKing/simple-editor/issues/25) · **Wave:** 1 · **Branch/worktree:** `feat/player-rate-loop` → `../simple-editor-wt/player-rate-loop` · **Depends on:** size-diet, split-god-files · **~560 new lines · Δ exe ≈ +48 KB**

Read [agents.md](../../../agents.md), [goals.md](../../../goals.md), [notes.md](../../../notes.md) and the master plan [README.md](../README.md) (registry protocol, keymap, modifier table) before starting.

## Summary

Parameterize Player's forward-1x assumption into Clock.rate/loop_range with direction- and stride-aware render-thread prefetch, rate-aware audio resampling, a non-blocking one-shot layer-request channel other waves will consume, and a track-id reorder guard in video_dirty_spans; play() forces rate=1.0 so Stop/Play always resumes forward. On top: JKL shuttle ladder, Loop In->Out, Play In->Out/Around/To-Out, Fast Review, Step±10, and audio-scrub-on-paused-playhead-change (one BLOCK, ~21ms), each wired through one new App fn, one Action+chord, and one MCP ToolDef — 9 new actions, 2 settings fields, ~11 MCP tools, zero new Panes/Glyphs/model fields. tools_playback.rs is modified (not created) here — split-god-files wave 0a owns its creation.

## Motivation

Closes gap-matrix rows 53-56 (JKL shuttle, loop in/out, audio scrub, fast review) plus critique C14 (play in/out/around/to-out) and C19 (dropped-frame counter), and lands architecture hook 6/10 (the async one-shot render channel every hover/trim/scope/wipe consumer in later waves needs) and missing-hook 13 (track-reorder safety in the playback cache). All three source designs (infra/pro/custom) converge on this scope; reconciled here to the skeleton's exact settings/hotkey/file decisions.

## In scope

- Clock.rate + Clock.loop_range + wrap/stop semantics in both directions
- Cmd::{Rate,Loop,Scrub,LayersAsync} through Player and both worker threads, including the required no-op match arms on the thread that doesn't act on a given variant
- Player::{set_rate,rate,set_loop,loop_range,step,scrub,dropped_frames,request_layers,take_layers_reply}, plus play() forcing rate=1.0
- Render-thread direction/stride-aware protect window, prefetch order, and direction-normalized dropped-frame accounting
- Audio-thread rate-aware mixed_until advance (resample <=2x forward, mute otherwise) and loop-wrap ring flush
- video_dirty_spans track-id reorder safety fix + regression test
- playback_ctl.rs: JKL shuttle ladder, Loop In->Out toggle, Play In->Out/Around/To-Out, Fast Review, Step±10, audio-scrub-on-paused-playhead-change
- 9 new Actions + default chords, 2 new Settings fields, MCP tools for every one of the above

## Out of scope

- Painting the rate/dropped-frame badge or any preview UI (canvas-handles-monitor, wave 2)
- Consuming request_layers for hover/trim-view/scopes/wipe (their respective wave-2/3 workstreams)
- Pausing alt-renders during export (each wave-2/3 consumer's own responsibility)
- Real audio pitch-shifting/resampling beyond the fixed-ratio block resize; reverse audio playback
- Per-cut pause-and-resume Fast Review; risk_spans()/prerender realtime-safety bands (export-deliver)
- Any Track.locked/ripple/magnetic field definitions (wave-0b/trim-model) — this workstream only guarantees video_dirty_spans ignores them
- Remembering a prior shuttle rate across Stop/Play (play() always resets to 1.0; see ponytail_notes)

## Files

| Op | Path | What |
|---|---|---|
| modify | src/playback.rs | Clock rate/loop_range/wraps + now() rewrite; Cmd::{Rate,Loop,Scrub,LayersAsync}; Shared req/reply/dropped/next_req; Player new API surface incl. play() forcing rate=1.0; render_thread rate-aware protect/prefetch + LayersAsync + direction-normalized dropped accounting + Cmd::Scrub no-op arm; audio_thread rate-aware mixed_until + Cmd::Scrub + wrap-flush + Cmd::{Rate,Loop,LayersAsync} no-op arm; video_dirty_spans track-id guard; ~15 new tests appended to the existing tests mod (1331+). |
| create | src/ui/app/playback_ctl.rs | pub fn act(app: &mut App, a: Action) -> bool (shuttle ladder, loop toggle, play-range starts, fast review), registered in ACT_HANDLERS; pub fn tick(app: &mut App, ctx: &egui::Context) filling the pre-placed playback_tick FRAME_HOOK (play-range auto-stop via app.play_stop_at; audio-scrub-on-paused-playhead-change via app.scrub_last_t). Pure fn shuttle_rate(rate,back)->f64, unit-tested standalone. |
| modify | src/ui/app/tools_playback.rs | Add pub const TOOLS entries for playback.rate/step/loop/play_range/scrub/status + render.layers_async/render.poll_layers into the file wave-0a already created (op:create there) with the relocated playback.seek/play/pause arms; append rather than recreate, and add one TOOL_TABLES line if not already wired. |
| modify | src/hotkeys.rs | Add ws:player-rate-loop actions! rows (see actions_and_hotkeys) plus FastReview unbound. |
| modify | src/settings.rs | Add ws:player-rate-loop fields: audio_scrub: bool (default true), preroll_secs: f32 (default 2.0), each with a #[serde(default=...)] fn and a round-trip test entry. |
| modify | src/ui/app/mod.rs | ws:player-rate-loop lines: `mod playback_ctl;`; App struct fields play_stop_at: Option<f64>, scrub_last_t: f64 (+ Default/new literals None, 0.0); one ACT_HANDLERS line, one FRAME_HOOKS line (fills the pre-placed playback_tick hook), one TOOL_TABLES line. |

## Model changes

- None. Track already has a stable `id: Id` field (src/model.rs:2767) — the fix is purely in playback.rs's comparison logic. Project.in_point/out_point already exist (src/model.rs:2871-2872) for Play In->Out / Loop In->Out / Play to Out to read. No new serde fields on Project/Clip/Track (Track.locked/ripple/magnetic belong to wave-0b/trim-model, not here).

## Engine changes

- src/playback.rs Clock (struct at 178-185, impl at 187-200): add fields rate: f64 (default 1.0), loop_range: Option<(f64,f64)>, wraps: u64 (default 0). Rewrite now(&mut self)->f64 to: return base_t when !playing; else raw = base_t + elapsed*rate; if loop_range=Some((a,b)): wrap forward when rate>=0 && raw>=b (rebase base_t=a+((raw-b) % (b-a).max(1e-6)), base_at=Instant::now(), wraps+=1) or backward when rate<0 && raw<=a (mirror), else clamp raw to [a,b]; else (no loop): rate>=0 stops at duration (existing behaviour), rate<0 stops at 0.0 (mirror, sets playing=false, base_t=0.0).
- src/playback.rs Cmd enum (220-241): add Rate(f64), Loop(Option<(f64,f64)>), Scrub(f64), LayersAsync. Cmd derives Clone already (line 220).
- src/playback.rs Shared (202-213): add req: Mutex<Option<(u64,f64,u32)>> (latest async layer request), reply: Mutex<Option<(u64,Arc<LayerSet>)>>, dropped: AtomicU64, next_req: AtomicU64.
- src/playback.rs impl Player (249-404): add set_rate/rate/set_loop/loop_range/step/scrub/dropped_frames/request_layers/take_layers_reply. scrub sends Cmd::Scrub to self.audio only, not self.both. Also edit play() (311-319): after the existing base_t/base_at/playing block, force c.rate = 1.0 unconditionally so Space/Play always resumes forward at 1x regardless of the last shuttle rate or a prior Stop; pause() (320-326) left as-is since now() ignores rate whenever playing is false.
- src/playback.rs render_thread Cmd match (498-583, exhaustive per-variant, no wildcard arm): add Cmd::Rate(_) => dirty=true; Cmd::Loop(_) => {} (loop_range read fresh from Clock each pass); Cmd::LayersAsync => read shared.req, decode via decode_layers (reuses existing helper at 985), write shared.reply — mirrors LayersOnce arm (563-573) but non-blocking; Cmd::Scrub(_) => {} // audio-thread only, grouped in the same style as the existing 'render-thread only' no-op line in audio_thread's match.
- src/playback.rs render_thread outer-loop tuple (585-589): extend to also read c.rate.
- src/playback.rs new pure fns (near 791): protect_bounds(idx,trail,read_ahead,rate)->(i64,i64); read_ahead_window(idx,read_ahead,last_idx,rate)->(i64,i64); prefetch_order(lo,hi,rate)->Vec<i64> (stride = rate.abs().round().max(1.0) as i64, reversed when rate<0); dropped_delta(prev_idx,idx,rate)->u64: direction-normalize first — jump = (idx - prev_idx) as f64 * rate.signum() (rate.signum()==0.0 treated as 1.0), stride = rate.abs().round().max(1.0); return (jump.round() as i64 - stride as i64).max(0) as u64.
- src/playback.rs render_thread: replace the 3 forward-only call sites (set_protect at 617-618; stall free-run find at 641-642 + hysteresis goal at 682-683; playing pacing loop's missing-frame lookup at 741-742) with the new helpers; pacing 'remain' math at 736 divides by rate.abs().max(1e-6) and picks idx+1 vs idx-1 by rate sign.
- src/playback.rs render_thread publish block (691-727): before 'last_pub=idx;' add shared.dropped.fetch_add(dropped_delta(last_pub, idx, rate), Ordering::Relaxed) when playing && last_pub>=0.
- src/playback.rs video_dirty_spans (791-855) track loop (820): add `if ot.id != nt.id { return None; }` as the first check inside `for (ot, nt) in old.tracks.iter().zip(&new.tracks)` — reorder now forces a full clear; per-track flag fields stay excluded from the comparison by construction (pinned by a test, not new code).
- src/playback.rs audio_thread (1203-1298, exhaustive per-variant with existing grouped no-op precedent 'Cmd::Gpu(_) \| Cmd::Proxies(_) \| Cmd::CacheBudget(_) => {} // render-thread only'): add Cmd::Rate(_) \| Cmd::Loop(_) \| Cmd::LayersAsync => {} // render-thread only, same grouping style. Cmd::Scrub(t) arm mixes exactly one BLOCK (1024 frames @ 48kHz = ~21ms) at t directly into the ring (reusing existing open/play-stream logic at 1258-1266, generalized so a scrub can start the device even while !playing) without touching mixed_until; steady-state mixing branch (1272-1288) reads c.rate each iteration — for 0<rate<=2 resizes the scratch mix window to round(BLOCK*rate) frames, mixes at mixed_until over that many frames, then decimates/duplicates to exactly BLOCK output frames before extending the ring, advancing mixed_until by (BLOCK*rate)/SAMPLE_RATE; for rate<=0 or rate>2, block.fill(0.0), mixed_until=lock(&shared.clock).now() (no drift tracking needed while muted), still extend the ring so pacing/underrun logic is untouched. Compare-and-flush lock(&shared.clock).wraps each iteration: on change, clear the ring and set filling=true (same as a fresh Play/Seek) before mixing the next block.
- src/engine/mixer.rs: NOT modified (owned by audio-dsp-automation) — rate handling lives entirely in playback.rs's own resize/decimate step around the unmodified Mixer::mix call.

## UI changes

- None visible in this workstream — all new capability is exposed via Actions (bindable, palette-visible once command-palette lands) and MCP tools. No new pane content, no new painted UI element; the rate/dropped-frame badge is explicitly out of scope (wave 2).

## New types and functions

- `fn now(&mut self) -> f64` — src/playback.rs (impl Clock, replaces 187-200): Rate- and loop-aware wall-clock read: wraps inside loop_range, stops at duration (rate>=0) or 0 (rate<0).
- `pub fn set_rate(&mut self, rate: f64)` — src/playback.rs (impl Player): Rebase the clock at the current time with a new rate; rejects/no-ops exactly 0.0 (use pause).
- `pub fn rate(&self) -> f64` — src/playback.rs (impl Player): Current shuttle rate for the (wave-2) rate badge.
- `pub fn set_loop(&mut self, range: Option<(f64,f64)>)` — src/playback.rs (impl Player): Enable/disable Loop In->Out.
- `pub fn loop_range(&self) -> Option<(f64,f64)>` — src/playback.rs (impl Player): Read back the current loop range.
- `pub fn step(&mut self, frames: i64, fps: f64)` — src/playback.rs (impl Player): Frame-accurate relative seek (StepBack10/StepFwd10, MCP playback.step).
- `pub fn scrub(&mut self, t: f64)` — src/playback.rs (impl Player): Audio-only one BLOCK (~21ms @ 48kHz/1024) at t, no clock change.
- `pub fn dropped_frames(&self) -> u64` — src/playback.rs (impl Player): Cumulative genuinely-dropped frame count for the (wave-2) badge.
- `pub fn request_layers(&self, t: f64, max_w: u32) -> u64` — src/playback.rs (impl Player): Non-blocking one-shot GPU-path layer decode; newest request wins.
- `pub fn take_layers_reply(&self) -> Option<(u64, Arc<LayerSet>)>` — src/playback.rs (impl Player): Poll the async layer-decode result.
- `fn protect_bounds(idx: i64, trail: i64, read_ahead: i64, rate: f64) -> (i64, i64)` — src/playback.rs: Cache eviction-protect window, direction-aware.
- `fn read_ahead_window(idx: i64, read_ahead: i64, last_idx: i64, rate: f64) -> (i64, i64)` — src/playback.rs: Prefetch bounds, direction-aware.
- `fn prefetch_order(lo: i64, hi: i64, rate: f64) -> Vec<i64>` — src/playback.rs: Nearest-first, stride-sampled frame indices to fill next.
- `fn dropped_delta(prev_idx: i64, idx: i64, rate: f64) -> u64` — src/playback.rs: Direction-normalized: jump = (idx-prev_idx) * rate.signum(), stride = rate.abs().round().max(1.0); returns (jump-stride).max(0) so a reverse-shuttle stall is counted, not silently zeroed by a raw negative jump.
- `pub fn act(app: &mut App, a: Action) -> bool` — src/ui/app/playback_ctl.rs: ACT_HANDLERS entry for the 9 new actions.
- `pub fn tick(app: &mut App, ctx: &egui::Context)` — src/ui/app/playback_ctl.rs: FRAME_HOOKS entry: play-range auto-stop + audio-scrub-on-paused-playhead-change.
- `fn shuttle_rate(rate: f64, back: bool) -> f64` — src/ui/app/playback_ctl.rs: Pure JKL ladder step: 0/opposite-sign -> unit rate; same-sign -> double magnitude, capped at 8.

## Actions and hotkeys

| Action id | Label | Chord | Note |
|---|---|---|---|
| shuttle_back | Shuttle Reverse | J | free (verified); repeated presses step the ladder -1,-2,-4,-8 |
| shuttle_fwd | Shuttle Forward | L | free (verified); existing Stop=K (hotkeys.rs:66) calls player.pause() unchanged; rate resets to 1.0 only on the next play(), not on Stop itself |
| loop_in_out | Loop In→Out | Ctrl+Shift+L | free (verified; distinct exact-match from Ctrl+L LinkToggle at hotkeys.rs:87 and Ctrl+Alt+L AddAdjustment at 124) |
| play_in_out | Play In→Out | Ctrl+Shift+Space | free (verified; PlayPause is bare Space, hotkeys.rs:65) |
| play_around | Play Around Playhead | / | free (verified, Key::Slash unused); pre/post roll from Settings.preroll_secs |
| play_to_out | Play to Out | Ctrl+Space | free (verified) |
| step_back_10 | Step Back 10 Frames | Shift+ArrowLeft | free (verified; StepBack is bare ArrowLeft, hotkeys.rs:67) |
| step_fwd_10 | Step Forward 10 Frames | Shift+ArrowRight | free (verified; StepForward is bare ArrowRight, hotkeys.rs:68) |
| fast_review | Fast Review |  | unbound per skeleton; declared as `None` like AddVideoTrack (hotkeys.rs:92) — transport menu / palette only |

## Persisted fields

**Settings:**

- audio_scrub: bool (default true) — gates the paused-playhead-change scrub in playback_tick
- preroll_secs: f32 (default 2.0) — symmetric pre/post roll for Play Around Playhead

**Project (.sedit):**

- (none)

## MCP tools (required — every capability must be scriptable)

| Tool | Kind | Args | Description | Maps to |
|---|---|---|---|---|
| playback.rate | ui | rate:number:true:playback speed, -8..8, 0 rejected (use playback.pause) | Set shuttle/playback rate. | Player::set_rate |
| playback.step | ui | frames:integer:true:signed frame count | Step the playhead by N frames (negative = back), pauses first. | Player::step + App::seek |
| playback.loop | ui | on:boolean:true:, in:number:false:defaults to Project.in_point, out:number:false:defaults to Project.out_point | Enable/disable Loop In->Out playback. | Player::set_loop |
| playback.play_range | ui | mode:string:true:'in_out'\|'around'\|'to_out' | Play In->Out, Play Around Playhead, or Play to Out, auto-stopping at the target. | playback_ctl::act (App.play_stop_at) |
| playback.scrub | ui | t:number:true:timeline seconds | Emit one BLOCK (~21ms) of audio at t without moving the clock (paused only). | Player::scrub |
| playback.status | read |  | Current rate, dropped-frame count, buffering flag, loop range. | Player::{rate,dropped_frames,is_buffering,loop_range} |
| playback.seek | ui | t:number:true: | Move the playhead (pre-existing tool, relocated into tools_playback.rs). | App::seek |
| playback.play | ui |  | Start playback, forcing rate to 1.0 (pre-existing tool, relocated; rate reset added by this workstream). | Player::play |
| playback.pause | ui |  | Pause (pre-existing tool, relocated). | Player::pause |
| render.layers_async | ui | t:number:true:, max_w:integer:false:default 640 | Queue a non-blocking one-shot layer decode; returns a request id. | Player::request_layers |
| render.poll_layers | read | id:integer:true:id returned by render.layers_async | Poll for the async layer-decode reply (null until ready or superseded by a newer request). | Player::take_layers_reply |

**Luau:** "editor.tool(\"playback.rate\", {rate=...}) etc. work automatically once tools_playback.rs registers in TOOL_TABLES (mcp::tools::all() feeds editor.tools()); no Luau-specific code needed. @on event hooks (fire_hook, owned by command-palette) are out of scope: playback state changes fire no hook."

## Tests

| Test | File | Asserts |
|---|---|---|
| clock_rate_and_loop_wrap | src/playback.rs (tests mod) | Clock.now() with rate=2.0 advances at 2x wall-clock; with loop_range=Some((1.0,3.0)) and rate=1.0, now() wraps back to ~1.0 (not past 3.0) and increments wraps; with rate=-1.0 and loop_range set, wraps at the low end symmetrically. |
| reverse_stops_at_zero | src/playback.rs | No loop_range, rate=-2.0: now() clamps to 0.0 and playing flips false once elapsed time would put raw<=0, mirroring the existing duration-stop test's shape. |
| play_forces_rate_1 | src/playback.rs (integration, real Player) | set_rate(-4.0), then pause(), then play(): rate() returns 1.0 immediately after play() returns, before any frame is decoded. |
| protect_and_prefetch_flip_for_reverse | src/playback.rs | protect_bounds/read_ahead_window swap which side of idx is 'ahead' when rate<0 vs rate>=0 (table-driven over a few (idx,rate) pairs). |
| prefetch_order_strides_and_orders_nearest_first | src/playback.rs | prefetch_order(2,10,4.0)==[2,6,10]; the reverse-bounds case returns hi,hi-1,...,lo (nearest-to-idx first, no stride skip at \|rate\|<=1). |
| dropped_delta_ignores_intentional_stride | src/playback.rs | dropped_delta(10,11,1.0)==0; dropped_delta(10,15,4.0)==0 (exact stride at rate=4); dropped_delta(10,20,1.0)>0 (real stall at rate=1); dropped_delta(-1,5,1.0)==0 (no prior publish); dropped_delta(10,9,-1.0)==0 (exact reverse stride); dropped_delta(10,3,-1.0)>0 (real reverse stall) — the direction-normalized fix for the finding that the un-normalized formula returned 0 for every reverse stall. |
| track_id_reorder_full_clears | src/playback.rs | Two same-length track lists differing only by which Track.id sits at index 0 vs 1 make video_dirty_spans return None. |
| flags_do_not_dirty_video | src/playback.rs | Toggling Track.name/height/bus (and, once present, locked/ripple/magnetic) alone — same id, index, clips/transitions/muted/solo — makes video_dirty_spans return no spans, pinning that flag fields never force a repaint. |
| scrub_mixes_without_moving_clock | src/playback.rs (integration, real Player) | While paused, Player::scrub(t) produces audible ring output (queued bytes > 0 shortly after, approx one BLOCK's worth) but Player::time() is unchanged before and after. |
| request_layers_newest_wins | src/playback.rs (integration) | Two request_layers calls in quick succession before either resolves: take_layers_reply eventually returns only the id of the second call, never the first's stale id. |
| shuttle_rate_ladder | src/ui/app/playback_ctl.rs | shuttle_rate(0.0,true)==-1.0; -1.0->-2.0->-4.0->-8.0, clamps at -8.0; shuttle_rate(2.0,true)==-1.0 (direction switch resets to unit); mirrored for back=false. |
| play_range_auto_stops | src/ui/app/playback_ctl.rs (integration via App test harness) | PlayInOut/PlayAround/PlayToOut each end with player paused at the expected target time (±1 frame) and app.play_stop_at cleared. |
| audio_scrub_toggle_respected | src/ui/app/playback_ctl.rs | With Settings.audio_scrub=false, tick() never calls player.scrub even when app.playhead changes between ticks; with it true and paused, it does exactly once per change. |
| tool_registry_parity_unaffected | src/ui/app/tools_registry_tests.rs | Existing every_edit_op_has_a_tool / tool_names_unique_and_namespaced / server_end_to_end structural tests still pass with the new playback.*/render.* tools added — no new test file, this workstream must not break them. |
| idle_no_repaint_while_paused_with_pending_scrub_state | src/playback.rs or ui test harness | assert_no_idle_repaint-style check: after a scrub settles and no request_layers is outstanding, 30 headless frames paused produce no further repaint requests (selftest idle-step compatibility). |

## Verification checklist

- [ ] cargo test (whole crate) green, including the ~15 new tests listed above
- [ ] cargo test --release bench_4k_preview -- --ignored: no regression at rate=1.0 (default path must be untouched in cost)
- [ ] cargo run -- --selftest: idle step green after loading a clip, playing at 2x then -1x for 2s, pausing (audio flushed, no leftover dropped-frame growth once stable)
- [ ] manual: J J J -> -4x reverse smooth with proxies on; K stops; L -> 1x; Space after Stop resumes forward at 1x even after a prior shuttle; Loop In->Out wraps audio with no audible click; Ctrl+Shift+Space (Play In->Out) and Ctrl+Space (Play to Out) stop exactly at Out; Slash (Play Around) pre/post-rolls the current preroll_secs
- [ ] scripts/size.ps1 -Note player-rate-loop: delta within the +48 KB estimate or PR body carries size: +N KB — reason
- [ ] grep confirms no edits landed outside owns_files/touches_shared marker sections (src/playback.rs, src/ui/app/playback_ctl.rs, src/ui/app/tools_playback.rs, plus only the ws:player-rate-loop lines in hotkeys.rs/settings.rs/src/ui/app/mod.rs)

## Acceptance criteria

- [ ] cargo test passes incl. new playback.rs/tools_playback.rs tests; no existing test regresses
- [ ] JKL: J from stop/forward sets rate=-1; repeated J doubles magnitude to -8 cap; L symmetric forward; K (existing Stop) calls player.pause() (playing=false); the next bare-Space Play always resumes forward at rate=1.0 regardless of the last shuttle rate, because play() now forces rate=1.0
- [ ] Loop In->Out wraps audio+video with no audible click/gap (manual + wraps-counter test)
- [ ] Play In->Out / Play Around / Play to Out start at the right time and auto-pause exactly at the target (±1 frame)
- [ ] Audio scrub emits one BLOCK (~21ms at 48kHz/1024) of audio on every paused playhead change when Settings.audio_scrub is true, silent otherwise, and never runs while playing
- [ ] dropped_frames() increases only when decode genuinely falls behind the current rate's stride (direction-normalized), not from intentional rate-driven index skipping in either playback direction
- [ ] reversing a track's order in Project.tracks forces video_dirty_spans to return None (full clear); reordering flags (locked/ripple/magnetic) alone never dirties video
- [ ] request_layers/take_layers_reply deliver the newest requested time only, never block the caller, and idle (no outstanding request, paused) costs 0 extra CPU
- [ ] every new Action has a ToolDef row in tools_playback.rs; tools/list exposes all playback.* + render.* tools with correct kind
- [ ] --selftest idle step stays green with a clip loaded, paused, no pending scrub/request_layers
- [ ] scripts/size.ps1 delta for this PR is within +48 KB of the wave-0 baseline (matches the skeleton's 0.08 KB/line density at ~560 new lines) or the PR body states size: +N KB — reason

## Risks

| Risk | Mitigation |
|---|---|
| Five load-bearing forward-1x assumptions (Clock::now, protect window, prefetch scan/order, hysteresis resume check, audio mixed_until) all change in one PR — a mistake in any one silently reintroduces stutter only at rate!=1, easy to miss in review. | Land as the ordered small commits in implementation_order (steps 1-5), each with its own pure-fn test before wiring into the threads; keep bench_4k_preview and the existing player_seek_play_pause_release/replay_serves_cached_frames tests green after every commit (rate=1.0 path must stay byte-identical to today). |
| Reverse or >4x shuttle on un-proxied long-GOP 4K forces an MF seek near-per-frame; the render thread can appear to hang instead of the UI staying responsive. | prefetch_order's stride sampling bounds decodes/second at high \|rate\|; is_buffering()/dropped_frames() surface the condition for the wave-2 badge to suggest proxies — no fix attempted here, just parity with today's forward-only stall path. |
| Cmd::Scrub racing a real Play/Seek could leave the audio stream started-but-orphaned, or a scrub after Pause leaves the device running with nothing queued. | Reuse the existing pause condition verbatim (`!playing && running && ring.is_empty() => stream.pause()`, ~1267-1271) — it already tears the stream down once the ring drains regardless of why it started, so Scrub needs only a special start path (guarded by the same stream.is_none()\|\|dead check used for Play), no special teardown. |
| Clock.wraps flush-on-change in the audio thread could false-trigger on the first block after a real Play/Seek (which already resets mixed_until via its own arm), double-flushing the ring. | The wraps check runs only inside the steady-state mixing branch after `filling` clears, never inside the Cmd::Play/Seek handler — a fresh Play/Seek already flushes via its own arm, so wraps only fires for an in-progress loop wrap. |
| prefetch_order allocates a Vec every call; at rate=8 with a large read_ahead this is a few hundred i64s per outer-loop pass. | read_ahead is already clamped to budget/2/per_idx (existing line 613), bounding the vec to the cache's own size limit; render_thread already allocates per-frame Arc<Frame>/LayerSet, so this is not a new order of magnitude. Revisit only if profiling shows it. |
| play() unconditionally forcing rate=1.0 could surprise a future caller that wants to resume a paused scrub session at its prior shuttle rate. | No such caller exists in this workstream's scope (shuttle only calls set_rate directly, never play()); documented as a ponytail note with the upgrade path (last_play_rate field) if a real need shows up. |

## Suggested implementation order

1. 1. playback.rs: Clock fields + now() rewrite + Cmd variants + Shared fields (compiles standalone, no callers yet)
2. 2. playback.rs: Player API surface (set_rate/rate/set_loop/loop_range/step/scrub/dropped_frames/request_layers/take_layers_reply) + edit play() to force rate=1.0
3. 3. playback.rs: render_thread Cmd handling (Rate/Loop/LayersAsync/Scrub-noop) + pure helpers (protect_bounds/read_ahead_window/prefetch_order/direction-normalized dropped_delta) + wire into the 3 call sites + dropped-frame accounting at publish
4. 4. playback.rs: video_dirty_spans track-id guard (one-line, its own commit)
5. 5. playback.rs: audio_thread Cmd::Scrub handling + Rate/Loop/LayersAsync no-op arm + rate-aware mixed_until (resample block for 0<rate<=2, mute + clock-locked mixed_until otherwise) + wraps-triggered ring flush
6. 6. playback.rs tests: the ~15 tests listed in `tests` (incl. the reverse-rate dropped_delta case)
7. 7. settings.rs: audio_scrub + preroll_secs fields + round-trip test
8. 8. hotkeys.rs: 9 new actions in ws:player-rate-loop section
9. 9. src/ui/app/playback_ctl.rs: shuttle_rate pure fn + unit tests, then act()/tick() against the new Player API and app.play_stop_at/scrub_last_t
10. 10. src/ui/app/mod.rs: register mod + the 3 registry lines + App fields
11. 11. src/ui/app/tools_playback.rs: append ToolDef rows into the wave-0a-created file (fold in relocated legacy playback.seek/play/pause), register in TOOL_TABLES if not already; run every_edit_op_has_a_tool / ui_action_covers_every_action / tool_names_unique_and_namespaced
12. 12. cargo test full suite, cargo run -- --selftest (idle step), scripts/size.ps1 -Note player-rate-loop, manual JKL/loop/play-range smoke pass, PR

## Deliberate simplifications (`// ponytail:`)

- Fast Review = flat 4x forward shuttle from the playhead to the end, no per-cut pause-and-resume. Upgrade path: reuse Project::cut_points() (already used by PrevCut/NextCut) to schedule holds in playback_tick.
- Reverse and >2x audio is muted outright, not resampled/pitch-shifted. 0<rate<=2 uses a cheap fixed-ratio block resize (decimate/duplicate), not a real resampler. Upgrade path: a proper resampler in the audio thread if quality complaints come in.
- dropped_frames() is a heuristic (direction-normalized idx jump minus the rate's own stride), not a frame-accurate decoder-level counter. Good enough for a UI badge.
- Play Around Playhead uses one Settings.preroll_secs for both pre- and post-roll (the skeleton's single-field decision), not independent pre/post. Add postroll_secs later if asked.
- Loop wrap-detection (Clock.wraps, polled by the audio thread) is a coarse discontinuity flag; the audio thread reacts like a fresh Seek, which can drop at most one ~21ms block of tail audio at the loop point.
- request_layers/take_layers_reply is unthrottled at the Player layer — pausing it during export (self.export.is_some()) is each wave-2 consumer's own job, called out here so it isn't silently skipped.
- play() forcing rate=1.0 is a deliberate simplification over a 'last non-shuttle rate' memory: nobody asked for resuming at a remembered shuttle speed, and it makes Stop->Play always predictable. Upgrade path: store last_play_rate on Player if a future workflow wants it.

## Review trail

- Finding 1 (rate never reset): confirmed via source (play():311-319, pause():320-326 — neither touches a rate field, both verified pre-patch). Added an engine_changes bullet: play() now force-sets Clock.rate=1.0 every call, so any bare-Space/toolbar Play after a JKL shuttle or Stop always resumes forward at 1x. pause() left untouched — rate is irrelevant while !playing since now() only applies rate when playing. Reworded the K/Stop acceptance criterion and hotkey note to stop claiming 'Stop sets rate=0' (Stop just calls pause(); the reset happens on the next play()).
- Finding 2 (dropped_delta blind to reverse): confirmed no test in the plan covers rate<0. Changed dropped_delta's contract to direction-normalize: jump = (idx - prev_idx) as f64 * rate.signum(), then (jump.round() as i64 - stride).max(0) as u64, so reverse stalls are counted instead of silently clamped to 0. Added a reverse-rate case to dropped_delta_ignores_intentional_stride's asserts.
- Finding 3 (scrub duration 40ms vs 1 BLOCK/21ms): confirmed BLOCK=1024, SAMPLE_RATE=48000 (playback.rs:44, media/mod.rs:17) => 21.3ms, not 40ms. Kept the cheaper 'one BLOCK' engine implementation and corrected acceptance_criteria/new_types_and_fns wording from '~40ms' to 'one BLOCK (~21ms)' so both halves of the plan agree.
- Finding 4 (missing exhaustive-match no-op arms): confirmed both render_thread's (playback.rs:498) and audio_thread's (playback.rs:1236) Cmd matches are explicit per-variant with no wildcard. Added the two missing bullets: render_thread gets 'Cmd::Scrub(_) => {} // audio-thread only'; audio_thread gets 'Cmd::Rate(_) \| Cmd::Loop(_) \| Cmd::LayersAsync => {} // render-thread only', in the same style — otherwise the plan as written does not compile.
- Finding 5 (size estimate inconsistent with skeleton's own 0.08 KB/line density): 560 lines * 0.08 = ~45 KB, not 72 KB. Lowered size_delta_kb from 72 to 48 rather than inflating est_new_lines, consistent with the wave-1 methodology (712 KB / 8900 lines = 0.08 KB/line).
- reconcile: confirmed src/ui/app/tools_playback.rs does not exist in the current tree (no src/ui/app dir at all yet), so it is genuinely created once, by split-god-files wave 0a, and this workstream's own text already said it 'relocates whatever wave-0a parked' there — same file-ownership-map double-create bug already fixed for media-library/tools_media.rs. Changed this plan's files[] entry for tools_playback.rs from op:'create' to op:'modify' and reworded its 'what' to describe appending TOOLS entries into the wave-0a file rather than creating it. Also added split-god-files to depends_on (wave 0a must land first so the file exists before this workstream modifies it).
