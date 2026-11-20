# Handoff: installer (`claude/feat-installer`)

Branch from `origin/develop`. Section "installer" of `plans/cloud-handoff.md`.

## What's done

- **`tools/se-setup/`**: a separate crate (its own `[workspace]`, like `tools/se-html`) that builds ONE exe,
  `simple-editor-setup.exe`, with three roles picked by file name or flag. The installer copies itself into
  the install folder as `update.exe` and `uninstall.exe`, so every exe is the same file and under 1 MB.
  Deps: `windows-sys` 0.60 (raw Win32, WinHTTP) and `miniz_oxide` (inflate for zips; already in the app's
  own tree). Release profile: `opt-level="z"`, LTO, `panic="abort"`, strip, 1 codegen unit.
  The manifest (`setup.manifest`) says asInvoker, Common Controls v6, PerMonitorV2 DPI.
- **Install** (`setup::install`): per-user into `%LOCALAPPDATA%\Programs\Simple Editor`, no admin.
  - The app comes from a `simple-editor.exe` beside the setup exe, else the latest GitHub release, verified
    against the asset's API `digest` (SHA-256). An asset without a digest is refused.
  - FFmpeg missing: the page says so, and **Install FFmpeg** is ticked by default. It's pinned in the new
    `library/bin/ffmpeg.json` (gyan.dev 7.1.1 essentials, size + SHA-256 I computed from the real download).
    Only `ffmpeg.exe` and `ffprobe.exe` are unpacked, into `<install>\ffmpeg`, which the app already searches.
  - Add-on checklist with one-line uses: yt-dlp, whisper.cpp, Fast HTML renderer, AI enhancer. Ones already
    present (detected with the app's own lookup order: settings folder, beside the exe, `ffmpeg\`, PATH,
    `%APPDATA%\SimpleEditor\bin`, the cache's `whisper\`) show "(Installed)", ticked and greyed out. Each
    goes where the app looks for it. A failed add-on doesn't fail the app install; the last page lists it.
  - Start menu shortcut (always) and Desktop (optional), both through IShellLinkW with the app's
    AppUserModelID `SimpleEditor`; Apps & Features entry; App Paths; the `.sedit` association
    (`SimpleEditor.Project`); the right-click menu with exactly the app's keys.
  - **`src/shell_keys.rs`** (new) holds every HKCU key, shared by the app (`contextmenu.rs` now includes it,
    no behaviour change) and the installer by `#[path]`, so they can't drift apart.
  - `install.json` in the install folder records version, FFmpeg/add-ons we installed, the options.
- **Updater** (`setup::check_update` / `apply_update`): GitHub latest release vs the installed version,
  download with digest check, file-version check of the download, wait for the app to close (process
  match by full image path), swap: old exe renamed aside, new renamed in, old restored on failure. Settings
  are never touched. Uninstall entry's DisplayVersion follows.
- **Uninstaller** (`setup::uninstall`): Keep / Remove preferences (radio buttons). Removes the shortcuts,
  every key in `shell_keys.rs` (legacy `SystemFileAssociations\video` key and `Applications\simple-editor.exe`
  too; a `.sedit` taken over by another app is left alone), `%APPDATA%\SimpleEditor\bin` (add-ons), the
  cache's `whisper`/`models`, the default cache, the install folder (FFmpeg and yt-dlp with it). Remove
  preferences also deletes `%APPDATA%\SimpleEditor`, the rest of `%LOCALAPPDATA%\SimpleEditor` and the
  Assistant's Credential Manager key. **Never deletes a `.sedit` file or the `autosave` folder.** It re-runs
  itself from `%TEMP%` (so it can delete its own folder) and that copy deletes itself after exit.
- `--silent` for all three roles (QuietUninstallString uses it).
- **`scripts/release.ps1`** builds the app and the setup exe into `dist\`, prints sizes + SHA-256, fails if
  the setup exe reaches 1 MB. `agents.md` "Releasing" step 5 now attaches both. CHANGELOG (Unreleased ▸
  Installer), README requirements and `website/docs/getting-started.mdx` describe it.

## Verified here (Linux container)

- `cargo check --target x86_64-pc-windows-gnu --all-targets`: the app (after the `shell_keys` refactor) and
  `tools/se-setup`, both clean (the app's 326 warnings are pre-existing; none from these files).
- `cargo test` in `tools/se-setup`: **28 pass** on Linux, against a scratch folder and an in-memory registry:
  SHA-256 vectors, JSON, zip (stored/deflate, CRC, zip-slip), add-on detection and digest checks, install
  (local and downloaded), FFmpeg skipped when present / attempted when missing, update swap + settings kept,
  rollback (bad version, tampered download, gave up waiting), uninstall keep/remove, projects and autosave
  untouched, foreign `.sedit` kept, reinstall without the menu.
- The real pinned FFmpeg zip (92,234,348 B) unpacks with CRC checks: `SE_FFMPEG_ZIP=<zip> cargo test real_ffmpeg`.
- Release build with mingw (`--target x86_64-pc-windows-gnu`), icon + manifest + version info embedded via
  windres: **535,040 B**. Imports are system DLLs only.

## Not verified (needs Windows)

- Nothing was run on Windows: wine crashes on start in this container. The window layout, DPI scaling,
  tooltips, WinHTTP downloads, shortcuts (COM vtables written by hand), process wait, Credential Manager,
  the `%TEMP%` self-delete and `real_registry_round_trip_under_a_test_root` (`src/win/reg.rs`, uses the real
  HKCU under `Software\SimpleEditorSetupTest\<pid>`) are compiled but not run.
- Live checks for the coordinator: `cd tools/se-setup && cargo test` (adds the real-registry test), then
  `scripts/release.ps1 -SetupOnly`, run `dist\simple-editor-setup.exe` next to a `simple-editor.exe`, look at
  every page at 100% and 150% scaling, install, open a `.sedit`, right-click an `.mp4`, run `update.exe`,
  uninstall from Settings ▸ Apps both ways.

## Expected exe sizes

- `simple-editor-setup.exe` (= `update.exe` = `uninstall.exe`): **~0.5 MB**. Measured 535,040 B with mingw;
  MSVC (`scripts/release.ps1`) is usually the same or smaller. The script fails the build at 1 MB.
- `simple-editor.exe`: unchanged (the `shell_keys.rs` move is the same constants; about +0 KB).

## Risks and follow-ups

- **The app's GitHub release assets need digests.** GitHub adds `digest` to assets uploaded since mid-2025;
  install-from-download and the updater refuse an asset without one. Upload `simple-editor.exe` as a fresh asset.
- **Unsigned exe**: SmartScreen will warn on first run of the setup exe (signing is out of scope here).
- The updater swaps only `simple-editor.exe`; `update.exe`/`uninstall.exe` refresh when the installer runs
  again. Nothing in the app opens `update.exe` yet (a Help ▸ Check for Updates item would be a small follow-up).
- whisper.cpp / yt-dlp come from their latest GitHub releases; if a release renames `whisper-bin-x64.zip`
  or ships without a digest, that add-on fails with a clear message (the app install still succeeds).
- A cache folder the user moved elsewhere (`cache_folder`) keeps its folder on uninstall; only `whisper` and
  `models` inside it are removed (it could be a folder with other things in it).
- The checksum-pinned FFmpeg is 7.1.1; bump `library/bin/ffmpeg.json` (url, size, sha256) to move on.
