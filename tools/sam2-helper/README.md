# se-sam2: the object-mask helper

Simple Editor's SAM 2 runner. A separate program, never linked into `simple-editor.exe`: the editor starts it
as a child process (`se-sam2 serve`) and talks JSON lines over stdin / stdout (the protocol is at the top of
`src/main.rs`; the editor's side is `src/engine/sam2.rs`).

It runs SAM 2.1 hiera-tiny as five ONNX graphs (vision encoder, mask decoder, memory encoder, memory
attention, object-pointer positions: the
[square-zero-labs export](https://huggingface.co/square-zero-labs/sam2.1-tiny-video-onnx), numerically checked
against the PyTorch video predictor by its authors) through ONNX Runtime, on the GPU with DirectML or on the
CPU. Frames come from `ffmpeg.exe` (rgb24 at 1024×1024), masks go out as 8-bit greyscale PNGs. The video
loop is SAM 2's: the prompted frame's memory plus the six most recent frames' memories (padded to seven
blocks) and up to sixteen object pointers condition every new frame; tracking runs forwards from the
prompted frame, then backwards in chunks of 24 frames.

## Build

`scripts/build-sam2.ps1` (plain `cargo build`; the dev profile is optimised) writes `dist/sam2/se-sam2.exe`
and updates its entry in `library/bin/sam2.json`. The add-on's other parts (ONNX Runtime and DirectML from
NuGet, the models from Hugging Face, each pinned by SHA-256) are fetched by the editor's installer.

Layout of an installed add-on (`%APPDATA%\SimpleEditor\bin\sam2`):

```
se-sam2.exe  onnxruntime.dll  onnxruntime_providers_shared.dll  DirectML.dll  sam2.json
models\vision_encoder.onnx  mask_decoder.onnx  memory_encoder.onnx  memory_attention.onnx  pointer_tpos.onnx  constants.json
licenses\...
```

## Knobs

- `--cpu`: never try DirectML.
- `SE_SAM2_CPU_GRAPHS=enc,dec,menc,mattn`: graphs kept on the CPU with DirectML on (default `dec`: on
  DirectML the decoder recompiles for every new number of click points, 48 s for the first click on an
  RTX 2060 Super; 1.4 s on the CPU).
- `SE_SAM2_OPT=0|1|2`: ONNX Runtime graph optimisation level (default all).

## Licences

| Part | Licence |
|---|---|
| this helper | MIT |
| SAM 2.1 weights (Meta, facebookresearch/sam2) and the ONNX export | Apache-2.0 |
| ONNX Runtime (`onnxruntime.dll`) | MIT |
| DirectML (`DirectML.dll`, NuGet Microsoft.AI.DirectML) | Microsoft DirectML redistributable terms (redistribution with applications allowed; not open source) |
| `ort` crate (Rust bindings) | MIT OR Apache-2.0 |
