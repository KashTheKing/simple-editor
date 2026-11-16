# se-html: the fast HTML renderer helper

Simple Editor draws HTML + CSS layers with Microsoft Edge by default. This crate is the alternative: a small
program (about 11 MB) that renders a page with [Blitz](https://github.com/DioxusLabs/blitz) (Stylo for CSS,
Taffy for layout, Parley for text, vello_cpu for painting on the CPU). No browser, no JavaScript, no network;
about 25 MB of memory per page instead of ~700 MB for Edge, a frame in milliseconds, no start-up wait.

It is **not** part of `simple-editor.exe` and **not** a member of its Cargo workspace (own `Cargo.lock`, own
toolchain pin): linking it would add ~10 MB to the editor. The app starts it as a child process
(`src/engine/blitz.rs`) and talks to it over stdin / stdout (`src/proto.rs`, the same file is compiled into
the app).

## Build

```
scripts\build-helper.ps1            # release build, dist\se-html\bin\{se-html.exe,se-html.json}, size_log.csv row
cd tools\se-html; cargo build --ignore-rust-version      # a dev build (deps stay optimised)
```

* Toolchain: `rust-toolchain.toml` pins Rust 1.90. Blitz 0.3.0-beta.3/4 declare MSRV 1.91; they build and
  render identically on 1.89/1.90, hence `--ignore-rust-version` (cargo has no config key for it). Move the
  pin to 1.91+ whenever it is installed; the app's own pin (egui 0.33 / rustc 1.89) is unaffected.
* Features: `blitz-dom` `system-fonts` (DirectWrite), `floats` (without it a plain `float` hangs layout),
  `svg`, `woff`; images png / jpeg / gif / webp. Release profile: `opt-level = "s"`, fat LTO, `strip`.
* `panic = "unwind"` is required: a panic inside the renderer (vello_cpu has `unimplemented!` for colour
  filters) is caught per command and answered as an error.

## Ship (and update) the helper

The exe is NOT committed to git (11 MB per version would live in history forever). It is a **GitHub release
asset**; only the small manifest `library/bin/se-html.json` is in the repo. The app's **Install** button
(`engine::blitz::install`) reads that manifest from the library URL (default
`https://raw.githubusercontent.com/KashTheKing/simple-editor/main/library`), downloads the exe from the
manifest's `url`, checks size and SHA-256, runs it once to check the protocol, and moves it to
`%APPDATA%\SimpleEditor\bin\se-html.exe`. Nothing is fetched before that click. If `main` has no
`library/bin/se-html.json` the install fails with a 404 (that was the original bug).

To publish a new version (bump `version` in `Cargo.toml`; bump `PROTOCOL` in `src/proto.rs` too when a command
changes meaning - an app refuses a helper that speaks another protocol, and old apps keep the old release):

1. Build and write the manifest, pointing it at the new release tag:
   `scripts\build-helper.ps1 -Url https://github.com/KashTheKing/simple-editor/releases/download/se-html-vX.Y.Z/se-html.exe`
   (output: `dist\se-html\bin\se-html.exe` + `se-html.json`, and a size_log.csv row).
2. Upload the exe as a release asset:
   `gh release create se-html-vX.Y.Z dist\se-html\bin\se-html.exe --repo KashTheKing/simple-editor --title "se-html X.Y.Z" --notes "..."`
3. Check the upload: download it from that URL and compare its SHA-256 with the manifest
   (`certutil -hashfile <file> SHA256`).
4. Commit `dist\se-html\bin\se-html.json` as `library/bin/se-html.json` on `main` and push. The manifest is the
   switch: until it is on `main`, nobody sees the new version. If local `main` has unpushed work, commit the
   manifest in a throwaway worktree on top of `origin/main` (`git worktree add --detach <dir> origin/main`) and
   `git push origin HEAD:main`, so only that one commit goes out.
5. Existing installs keep the old exe; they get the new one by clicking Install again (it replaces the file).
   Test on a machine with `%APPDATA%\SimpleEditor\bin` emptied, or point Settings > Library folder at a local
   copy of `library/` to try the flow before publishing.

## Protocol (see `src/main.rs` for the exact fields)

`ping`, `load` (a page stays loaded), `frame` (one picture at time `t`, PNG or raw RGBA, with `vars`, `ptr`,
`scroll`), `frames` (a batch for export), `region` (a part of the page at any scale), `elements` (links, buttons,
fields, headings with boxes and text), `free`, `quit`. Pointer events give `:hover` and `:active` (and the
transitions they start) at exact times.

Test it by hand: `python probe.py path\to\se-html.exe`; render a file: `se-html --render in.html out.png 1920 1080 1 0.5,2`.

## What it cannot draw

See `src/engine/html_lint.rs` (the app's pre-flight, which sends such layers to Edge) and
`website/docs/html-layers.mdx`.
