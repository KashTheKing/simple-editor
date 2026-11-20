# se-setup: installer, updater, uninstaller

One small Win32 program (windows-sys + miniz_oxide, no GUI toolkit) that plays three roles, picked by its
file name or a flag:

| File | Role | Flags |
|---|---|---|
| `simple-editor-setup.exe` (the release download) | install | `--silent`, `--no-ffmpeg`, `--desktop`, `--no-context-menu`, `--addons=yt-dlp,whisper,se-html,enhancer` |
| `update.exe` (copied into the install folder) | update | `--silent` |
| `uninstall.exe` (copied into the install folder; Settings ▸ Apps runs it) | uninstall | `--silent`, `--remove-preferences` |

- **Install**: per-user into `%LOCALAPPDATA%\Programs\Simple Editor`, no admin (the manifest says
  `asInvoker`). The app comes from a `simple-editor.exe` beside the setup exe (offline), else the latest
  GitHub release, verified against the asset's SHA-256 digest. FFmpeg (pinned in `library/bin/ffmpeg.json`
  by size and SHA-256) goes into `<install>\ffmpeg`, the folder the app already searches. Add-ons go where
  the app looks for them (`src/media/ffpipe.rs`, `src/engine/transcribe.rs`, `src/engine/blitz.rs`,
  `src/engine/enhance.rs`). Registry keys come from `src/shell_keys.rs`, shared with the app's
  `contextmenu.rs`. `install.json` in the install folder records what was installed.
- **Update**: latest release newer than the installed version, download (digest-checked, file version
  checked), wait for the app to close, swap the exe (old one renamed aside and restored on failure).
- **Uninstall**: Keep or Remove preferences; removes the app, add-ons, FFmpeg, shortcuts and every key in
  `shell_keys.rs` (legacy ones too), never a `.sedit` file or the autosave folder; re-runs itself from
  `%TEMP%` so it can delete its own folder, then deletes that copy.

Build: `scripts/release.ps1` (or `cargo build --release` here). Tests: `cargo test` runs the logic on any OS
against a scratch folder and an in-memory registry; on Windows one more test uses the real registry under
`HKCU\Software\SimpleEditorSetupTest\<pid>`. `SE_FFMPEG_ZIP=<zip>` adds a test that unpacks the real
FFmpeg build.
