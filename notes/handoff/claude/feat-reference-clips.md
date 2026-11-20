# Hand-off: reference-clips (`claude/feat-reference-clips`)

New from `origin/develop` (a3a0538). Section "reference-clips" of `plans/cloud-handoff.md`.

## Done

- **Find a Reference pane** (`Pane::References`, title "Find a Reference", glyph Search). Window menu (MAIN_PANES),
  Library empty-area right-click ▸ Find a Reference. Tab-stacked hidden like the other late panes.
  - Query field; one button per site (Yarn, PlayPhrase, Clip.cafe, Tenor, GIPHY, YouTube), ordered best-first by
    `engine::references::suggest` (quote / reaction / video guess); opens the search in the browser
    (`engine::browsers::open_url`). Tooltips carry the "what it's for" text (labels are just the name).
  - GIPHY / Tenor in-app search (official APIs: `api.giphy.com/v1/gifs/search`, `tenor.googleapis.com/v2/search`)
    with the user's own key (Settings ▸ General ▸ GIPHY key / Tenor key, masked, "Get a Key" button). Worker thread +
    channel; stills fetched one at a time on one worker, decoded with `media::thumbs::wic_thumb`, cached under
    `cache_dir()/references`. "Powered by GIPHY / Tenor" under the results. Click a result: MP4 downloaded (curl,
    https only, `.part` + rename) to the downloads folder, imported, marked as a reference.
  - Import a Link + **Just a part** (range `1:23-1:45`, prefilled from `?t=`). The same "Just a part" row is on the
    Import URL window (`files.rs::url_window`). yt-dlp gets `--download-sections *a-b --force-keyframes-at-cuts` and
    a range in the output name. yt-dlp now also prints `meta:<title>\t<uploader>` (for credits).
  - "In This Project": the references as `ui::list_rows` rows (fixed order), Search by quote, a lit Library button
    (toggles the Library), Copy Credits.
  - One-time fair-use note before the first import (`Settings.reference_note_seen`).
- **Model**: `Project.references: Vec<RefSource>` (asset, site, url, title, credit, range, quote; skipped when empty,
  old files load). Ops in `model/ops/references.rs`: `set_reference` (tags `reference`, creates the
  "References" smart bin `q:reference` once), `set_reference_quote`, `find_references`, `reference_credits`. Both
  `&mut self` ops are mapped in `OP_TOOLS` to `references.set`, and the file is in `OP_FILES`.
- **Auto-transcription**: after the probe lands, one whisper run at a time (`transcribe::start`, the Subtitles pane's
  chosen model + language) fills the quote. Skipped silently when whisper / the model is missing or the asset has no
  audio (GIFs). Off switch: Settings ▸ General ▸ Transcribe references.
- **Library search** also matches a reference's quote (`library.rs::quote_matches`).
- **MCP** (`reference_ctl::TOOLS`): `references.suggest` (Read, no network), `references.search_links` (Ui; `open`),
  `references.import_range` (Job; in `leaves_project`, so the guard asks), `references.list`, `references.set`
  (Mutate), `references.credits`. Listed in `docs/customizing.md`.
- Docs: `website/docs/find-a-reference.mdx` (+ sidebar), CHANGELOG under `## Unreleased`.

## Verified here (Linux container)

- `cargo check --target x86_64-pc-windows-gnu --all-targets`: passes, no new warnings.
- The app does not build for Linux (winit), so `cargo test` can't run here. A scratch harness crate that `#[path]`
  includes the real `engine/references.rs`, `ui/reference_ui.rs` and `ui/list_rows.rs` ran **15/15 passing**:
  URL building for all six sites, percent-encoding / slugs, kind guessing, time / range parsing, `?t=` parsing,
  GIPHY and Tenor reply parsing (incl. error replies), request URLs (only query + key, no locale), credits lines,
  no network without a key, and `assert_no_idle_repaint_find_a_reference` / `..._note`.

## Not verified (needs Windows)

- The tests that need the whole crate: `model/ops/references.rs` (tagging, bin once, quote search, credits, serde
  round trip), `reference_ctl.rs` (range parsing, note gating, still names, tools registered once + guarded),
  `ytdlp.rs::meta_lines_parse`, and the registry tests (`every_edit_op_has_a_tool` with the two new OP_TOOLS rows).
  Type-checked only.
- **Nothing rendered**: no screenshot of the pane or the Settings rows was taken. Please look at it live.
- **Live network**: the container's proxy blocks all six sites, so no real request was made. The Clip.cafe search
  pattern (`https://clip.cafe/?s=…`) is the least certain; Yarn uses `getyarn.io/yarn-find?text=…`. Please click
  each button once. GIPHY / Tenor search, still decoding (WIC on GIF / PNG stills) and MP4 import need a real key.
- yt-dlp: `--download-sections` and the extra `--print after_move:meta:…` line were not run against a real yt-dlp.
  The path is still the last printed line; `meta:` lines are filtered out before it.

## Risks / notes for the merge

- Shared-file edits are small and marked `feat/reference-clips`: `Pane` enum/ALL/glyph/title (end of list),
  `MAIN_PANES` length 20 -> 21 (conflicts if another branch also adds a main pane: bump the number),
  `TOOL_TABLES` / `FRAME_HOOKS` / `PANE_DRAWERS` (end of each), `Settings` (4 fields at the end), `Project` (one
  field at the end + `Project::new`), `LibraryResponse.find_reference`, `leaves_project`, `OP_TOOLS` / `OP_FILES`.
- `App::start_download` now takes a range and returns the progress (only caller was the Import URL window).
- The `downloads` poll in `actions.rs` carries the progress + yt-dlp meta so references are recognised.
- Keys are stored in `settings.json` in plain text (they are free, per-user API keys; the Assistant's key is the
  only one in Credential Manager). Say if they should move there.
- Size: not measured (needs a Windows release build); no new dependencies.
