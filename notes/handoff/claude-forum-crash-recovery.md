# Hand-off: forum-crash (`claude/forum-crash-recovery`)

Session C of `plans/shotcut-forum-insights.md`: crash resilience and recovery. Branched from
`origin/develop` at `a3a0538`. Item 7 (signing) left out as asked.

## What's done

| Item | Where | What it does |
|---|---|---|
| 1. Crash fingerprint | `src/crashguard.rs` (new) | Risky work registers an in-flight entry in memory (`begin` guard, `linger` for N s, `linger_while` a check). A writer thread mirrors it to `%APPDATA%\SimpleEditor\inflight-<pid>.json` 150 ms later (short entries never reach the disk; nothing is written on the caller's thread). The panic hook (UI thread only) and a `SetUnhandledExceptionFilter` handler write it synchronously and freeze it. The file also carries the open project's path. On start, `crashguard::init` reads the files of dead pids (`OpenProcess` liveness check, so a second running instance isn't a crash) and deletes them. A clean close removes this run's file. |
| 1. Culprit offline | `src/media/offline.rs` (new), `media/mod.rs` | `media::probe`, `open_video`, `open_known` and `open_audio` refuse an offline path (a read lock + hash), and each one is fingerprinted (`Probe` / `Decode`). The file a run died on goes in `offline.json` until **Bring the File Back**. |
| What is fingerprinted | | probe and decoder opens, project open (`files.rs::open_project`), OS file drops (6 s, `drops.rs`), a clip that gained an effect (8 s, `recovery::note_new_effects` from `after_edit`), File / Overwrite exports until the job is done (`files.rs::note_export`), graphics start until the first revealed frame, each hardware-encoder test. |
| 2. Safe start | `recovery.rs::boot`, `RecoverPlan`, `confirm::ask_many` (new N-button prompt), `model/repair.rs` (`disable_all_effects`, `media_paths`; `repair` is now `pub mod`) | The prompt says "closed unexpectedly while playing clip.mp4" (or adding / applying an effect to / exporting / opening), says the file is offline, and offers **Recover**, **Recover Without Media** (every asset offline for this run only), **Recover Without Effects** (one undo step), **Bring the File Back**, Cancel. Without a newer autosave it reopens the crashed project file itself. Plain newer-autosave case keeps the old "Recover unsaved project?" title, now with the safe-start buttons too. |
| 3. Guarded probe | `engine/import.rs::guarded_probe` | Each file's probe runs on its own thread: a panic is caught, and after 30 s the batch moves on ("took too long to read; skipped") with the stuck thread left behind. A native crash still kills the process, but item 1 then takes that file offline. |
| 4. Encoder test | `engine/encoders.rs` | It was already a child ffmpeg per encoder. Now: the cache key adds every hardware adapter's `vendor:device:UMD driver version` from DXGI (a driver update re-tests); progress (`tested`, `testing`) is written before each test, so an encoder that took the app or PC down is counted broken and never re-tested for that driver. Old cache files (no `done` field) still read as finished. |
| 5. Compatibility graphics | `src/graphics.rs` (new), `main.rs`, `settings.rs` (`compat_graphics`), Settings ▸ Performance row, `ui/app/gpu.rs` | `run_native` failing shows a Win32 MessageBox "Start in compatibility mode?" → Yes ticks the setting and relaunches with `--compat-graphics`. A run that died before its first frame asks the same at the next start (before the window). Compat = CPU compositor preview, vsync off, no MSAA/depth/stencil, and `SetDllDirectoryW` to a `compat-gl` folder (next to the exe or `%LOCALAPPDATA%\SimpleEditor\compat-gl`) when it holds Mesa's `opengl32.dll`. Nothing is bundled. `SE_FORCE_GL_FAIL=1` fakes the failure (outside compat mode) to test the dialog path. |
| 6. Backups | `autosave.rs::to_delete`, restore window | Keeps the newest 20 plus the newest of each clock hour for 48 h. Restore Autosave now uses `ui::list_rows` (project name, age column, ↺ button, double-click, full path on hover). Tests: 3 h of 30 s saves → 23 kept, one per hour; 250 markers survive save/load. |
| Docs | `CHANGELOG.md` (Unreleased ▸ Crash recovery), `website/docs/getting-started.mdx`, `website/docs/report-a-bug.mdx` (window doesn't open / white) | |

## Verified here (Linux container)

- `cargo check --target x86_64-pc-windows-gnu --all-targets`: clean, no new warnings from these files.
- `cargo test --target x86_64-pc-windows-gnu --no-run`: the Windows test exe links.
- **Could not run the Windows test exe**: Wine 9 in this container dies on any program, even a hello world
  (`free(): invalid pointer`). The crate doesn't build for Linux (Windows-only deps).
- Instead, ran on Linux:
  - `src/crashguard.rs` included as-is in a scratch crate: its 3 unit tests pass, plus an end-to-end writer
    test (a short guard never hits the disk; a longer one is written with the project path; the file goes
    when the guard ends; `flush_for_crash` writes and freezes; leftovers are read once). This caught and
    fixed a bug: file names in messages are now split on both `\` and `/`.
  - Verbatim extracts of `autosave::to_delete`, `import::guarded`, `encoders::parse_cache`,
    `graphics::decide` / `relaunch_args`, `recovery::ago` / `backup_name` with their tests: 8/8 pass.

## Not verified (coordinator, on Windows)

1. `cargo test crashguard graphics offline recovery autosave repair encoders a_panicking_or_hung_probe`, then the full suite.
2. Live: start with `SE_FORCE_GL_FAIL=1` → dialog → Yes → the app relaunches in compat mode (preview on CPU,
   Settings ▸ Performance ▸ Compatibility graphics ticked). Untick it after.
3. Live crash: open a project, start playback, kill the exe from Task Manager while a clip is opening or an
   export runs (or add a temporary `panic!` in a decoder open). Next start should name the file/action and
   offer the safe starts; the file should show as missing until Bring the File Back.
4. `--selftest` and the perf gate (decoder opens now take a mutex and notify a condvar, and every edit walks
   the open timeline's clips once in `note_new_effects`; both should be noise).
5. The SEH filter: confirm a real access violation (not a panic) leaves `inflight-<pid>.json`.

## Risks / notes

- `Kind::Decode` covers in-process opens only; ffmpeg child processes can't take the app down anyway. The
  standalone media player (`player/engine.rs` calls `mf::open_video_gpu` directly) isn't fingerprinted.
- A crash within 8 s of adding an effect blames that effect's clip file only when no probe/decode was in
  flight (`culprit_files` prefers the narrower entry). Killing the app from Task Manager during an export
  is reported as a crash during export (the prompt is still useful).
- Bring the File Back re-sends the project to the player; if the decoder pool remembers the failed open,
  the file may only come back after a reopen. Check live.
- Compatibility mode without a Mesa DLL still uses the hardware OpenGL driver: it helps when the GPU
  preview's shaders are the problem, not when no GL 2+ context can be made at all. Bundling or
  downloading Mesa (MIT) as an add-on would close that gap; not done.
- Shared files touched lightly (one or two lines each): `ui/app/mod.rs` (`set_project`, `after_edit`,
  first frame, close), `ui/app/files.rs`, `ui/app/drops.rs`, `ui/app/gpu.rs`, `ui/confirm.rs`
  (`Kind::Many`), `engine/import.rs` (probe loop; forum-preview's import probe may touch the same
  function), `settings.rs`, `ui/settings_ui/performance.rs`, `Cargo.toml` (3 `windows` features:
  `Win32_System_Diagnostics_Debug`, `Win32_System_Kernel`, `Win32_System_LibraryLoader`).
- Size: three small modules, no new crates. Expect well under +64 KB.
