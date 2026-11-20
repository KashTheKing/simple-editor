# Stem Separation helper (`se-demucs-win64.zip`)

The program half of the Stem Separation add-on (`library/bin/demucs.json` ▸ `program`;
`src/engine/demucs.rs` installs and runs it). It is [demucs.cpp](https://github.com/sevagh/demucs.cpp)
(MIT), a Python-free C++ port of Demucs v4, built for Windows x64. The weights (`model` in the same
manifest) are the htdemucs 4-source f16 file from the `Retrobear/demucs.cpp` Hugging Face dataset and are
downloaded as they are.

## Build

On Linux or WSL (Ubuntu 24.04: `apt install mingw-w64 zip git`):

```bash
tools/demucs-helper/build.sh
```

It clones demucs.cpp at the pinned commit (with the Eigen and libnyquist submodules), applies two small
patches, cross-compiles two static exes and writes `target/demucs-helper/se-demucs-win64.zip`, then prints
the `program` block (URL, SHA-256, size) for the manifest. About a minute on 4 cores.

| In the zip | What |
|---|---|
| `demucs_mt.cpp.main.exe` | `-march=x86-64-v3` (AVX2 + FMA): what nearly every PC since 2013 runs |
| `demucs_mt.cpp.main.v2.exe` | `-march=x86-64-v2` (SSE4.2): `engine::demucs::exe()` picks it on a CPU without AVX2 |
| `README.txt`, `licenses/` | the commit, flags, patches; MIT (demucs.cpp, Demucs), MPL-2.0 (Eigen headers), BSD-2 (libnyquist), the mingw-w64 runtime note |

- No OpenMP and no BLAS: `demucs_mt` splits the song over its own threads (the app asks for half the
  cores), so there is no `vcomp140.dll` and no DLL at all (static libstdc++ / winpthreads).
- libnyquist is compiled with the WAV reader and writer only: the app hands the separator a 44.1 kHz WAV
  and reads WAVs back, so the other codecs (and their licences) stay out.
- An embedded manifest sets the process code page to UTF-8, so the model path opens under a non-ASCII
  Windows user name; the app also passes the WAVs by name inside the work folder.
- GCC's runtime is GPLv3 with the Runtime Library Exception, which puts no GPL terms on the exes (the
  same as every MinGW-built program); nothing GPL is linked in otherwise.
- Same compiler, same bytes: the PE timestamp is off, the zip's dates and order are fixed. Built on
  Ubuntu 24.04 with `x86_64-w64-mingw32-g++-posix` 13 it gives the SHA-256 in `library/bin/demucs.json`.
  Another compiler version gives other bytes: put the new SHA-256 and size in the manifest.

The CLI the app relies on (checked by `engine::demucs` tests and its install check):
`demucs_mt.cpp.main.exe <model file> <wav file> <out dir> <num threads>`; no arguments prints that usage
and exits 1; progress lines are `[THREAD i] ( 41.250%) …` on stdout; the output is
`target_0_drums.wav`, `target_1_bass.wav`, `target_2_other.wav`, `target_3_vocals.wav` (32-bit float).

## Publish

1. Build as above (or take the zip from the build that made the manifest's hash).
2. `gh release create demucs-v1 target/demucs-helper/se-demucs-win64.zip --repo KashTheKing/simple-editor --title "Stem Separation add-on"`
   (a later rebuild: `demucs-v2`, and update `url` and `version`).
3. Check the uploaded file's size and SHA-256 match `library/bin/demucs.json` ▸ `program`.
4. Try it: Settings ▸ General ▸ Stem Separation ▸ Install (with Settings ▸ Library folder pointed at a
   local `library/` copy to test the manifest before it reaches `main`), then
   `cargo test demucs_real_run -- --ignored --nocapture`.
5. Commit the manifest on `main`: the manifest on `main` is the switch for every installed app.
