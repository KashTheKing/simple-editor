# Hand-off: gallery-stock (`claude/feat-gallery-stock-b07f2c`)

Cloud session, 2026-11-20. Section "gallery-stock" of `plans/cloud-handoff.md`.

## Done

The paused WIP already covered the whole scope. This session merged `origin/develop` (clean, no conflicts),
made it compile and fixed what the tests and a review turned up.

- **Stock tab in the Gallery** (`src/ui/stock_ui.rs`, `src/ui/app/stock_ctl.rs`, wired in `gallery.rs` /
  `gallery_ctl.rs`): search, kind / shape / size / licence filters, cards with lazy thumbnails, hover previews
  (video frames or a quiet sound snippet made by ffmpeg on a worker thread), details, Add / Add at playhead /
  drag onto the timeline, More results.
- **Keyless sources** (`src/engine/stock/sources.rs`): Wikimedia Commons, Internet Archive, NASA, Openverse.
- **Keyed sources**: Pexels, Pixabay, Freesound, with the user's own key in Settings ▸ Stock
  (`src/ui/settings_ui/stock.rs`), stored in Windows Credential Manager (`engine::stock::keys`), never in settings.json.
- **Library import with licence and attribution**: downloads run as Jobs (curl), land in `Stock Footage` beside
  the project, write `<file>.stock.json`, and stamp the asset's description and `stock` tag.
- **Credits export**: ⋯ ▸ Export credits… and `stock.credits`.
- **MCP**: `stock.search`, `stock.add`, `stock.credits` (`stock_ctl::TOOLS`, in `TOOL_TABLES`); listed in `docs/customizing.md`.
- Docs: `website/docs/stock.mdx`. CHANGELOG line under `## Unreleased`.

### Changes this session

1. `fix(stock)`: a local `key` shadowed the `key()` fn in `start_search` (the only compile error).
2. `fix(stock)`: a Commons **video or picture** search returned audio files (`"AUDIO" => kind` took on any kind).
   Caught by the existing `stock_commons_parses_video_picture_and_audio` test.
3. `perf(stock)`: the Stock tab stat'ed a file per result **on every frame** for the "downloaded" tick. Now cached
   in `Ctl.have`, rechecked only when results, the folder or a download change.
4. House rules: "Export credits…" and "Stock settings" are now buttons where the text only named them; with no
   source on, the status line says so with a Stock settings button (new test
   `stock_with_no_source_on_offers_a_settings_button`).

## Verified

- `cargo check --target x86_64-pc-windows-gnu --all-targets`: passes (mingw-w64; only pre-existing warnings plus
  two dead-code warnings, `Source::from_id` / `LicenceLevel::id`).
- `engine::stock` unit tests (14 of 14 non-network) **pass on Linux**: the module compiled in a scratch crate with
  stubs for `ffpipe::find_exe/command`, `export::Progress/stderr_tail/CANCELLED` and `library::slug`. Covers the
  parsers for all 7 sources against the recorded fixtures, licence filtering, attribution, credits text, file names.

## Not verified here (please run on Windows)

- The crate's own test binary can't run on Linux (winit refuses the platform) and Wine crashes on any exe in this
  container, so these are **compiled only**: `stock_ctl` tests (planned paths, sources on by default, search
  cache, MCP args, credit stamp) and `stock_ui` tests (cards + click, filter re-search, offline, the new
  settings button, `assert_no_idle_repaint_gallery_stock_tab`). Run: `cargo test stock`.
- Network tests are `#[ignore]`d and the sandbox proxy refuses commons.wikimedia.org, archive.org,
  images-api.nasa.gov and api.openverse.org. Run: `cargo test stock -- --ignored`.
- Live app: search each source, hover preview (sound volume), drag a card onto the timeline while it is still
  downloading (the placeholder-then-probe path in `stock_ctl::defer`), Add at playhead, Export credits, a key
  saved / removed in Settings ▸ Stock, the Jobs pane rows. Take a screenshot of the tab.
- `--selftest`, the perf gate and the size gate weren't run.

## Risks

- Needs `curl.exe` (in Windows 10 1803+). Without it each search shows "curl.exe not found".
- Thumbnails and previews use up to 3 worker threads; searches spawn one thread each. Nothing runs on the playback thread.
- Size: about 2,800 lines of Rust (3,500 with tests) and no new crates. The JSON fixtures are test-only (`include_str!` in `#[cfg(test)]`).
- `stock.add` only accepts ids from a search made in this app session (by design: no blind downloads).
