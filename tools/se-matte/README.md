# se-matte

Simple Editor's background-removal helper: MODNet (Fast) or BiRefNet lite (Best) through ONNX Runtime,
DirectML on the GPU with a CPU fallback. A separate program the app starts as a child process
(`src/engine/bgremove.rs`); it is never linked into the app. Part of the optional add-on described in
`library/README.md` (`bin/bgremove.json`) and `website/docs/background-removal.mdx`.

## Protocol

```
se-matte --model <file.onnx> --kind modnet|birefnet --width W --height H [--ep auto|dml|cpu] [--threads N]
se-matte --model <file.onnx> --kind modnet|birefnet --probe [--width W --height H] [--ep ...]
se-matte --version
```

- stdin: frames of exactly `W*H*3` bytes (RGB24, top-down), no headers. stdout: per frame exactly `W*H`
  bytes, an 8-bit matte (255 = subject), in order. EOF on stdin ends the run (exit 0).
- stderr: `ep dml` or `ep cpu` once the first frame (a warm-up on grey) ran; a failure is
  `error: ...` and exit 1. With `--ep auto` a DirectML failure on that first frame (too little GPU memory,
  a driver that cannot run the graph) reloads the model on the CPU.
- Pre-processing: MODNet gets the long side at 512 in multiples of 32, `x/127.5 - 1`; BiRefNet its fixed
  1024x1024 with ImageNet mean / std and a sigmoid on the output. The model output is resized back to
  `W`x`H` bilinearly. The app's temporal smoothing and the Edge control run in the app, not here.

## Build

```powershell
$env:CARGO_TARGET_DIR = "G:\se-target\addon-bg-removal\se-matte"; cargo build
```

The dev profile is optimised (`opt-level = 3`, no debug info): the per-pixel resizing is ours, inference
is `onnxruntime.dll`'s. `ort` is pinned to `=2.0.0-rc.10` (ONNX Runtime API 22, MSRV 1.81, builds on the
repo's 1.89) with `load-dynamic`: the DLL is loaded at run time from beside the exe (or `SE_ORT_DLL`), so
the build downloads no binaries. The add-on's `runtime` part is Microsoft's own 1.22.0 DirectML build.

To ship: copy `se-matte.exe` from the target dir, `gh release create bgremove-v0.1.0 se-matte.exe --repo
KashTheKing/simple-editor --title "Background removal 0.1.0"`, then set the `helper` part's `size` and
`sha256` in `library/bin/bgremove.json` to that file's.

## Licences

| Part | Licence |
|---|---|
| se-matte (this folder) | MIT |
| `ort` crate (pykeio) | MIT OR Apache-2.0 |
| ONNX Runtime 1.22.0 (Microsoft) | MIT; its `ThirdPartyNotices.txt` is installed beside it |
| DirectML | Windows' own `System32\DirectML.dll` (Windows 10 1903+); not downloaded |
| MODNet (ZHKKKe/MODNet, ONNX by Xenova/modnet) | Apache-2.0 |
| BiRefNet lite (ZhengPeng7/BiRefNet, ONNX by onnx-community/BiRefNet_lite-ONNX) | MIT |

RobustVideoMatting is deliberately not used: it is GPL-3.0.
