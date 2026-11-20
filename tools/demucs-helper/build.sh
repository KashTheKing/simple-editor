#!/usr/bin/env bash
# Builds se-demucs-win64.zip, the Stem Separation add-on's program (library/bin/demucs.json ▸ program).
#
# demucs.cpp (MIT, github.com/sevagh/demucs.cpp) cross-compiled for Windows x64 with mingw-w64 on Linux
# (or WSL). Pure Eigen, no BLAS, no OpenMP: demucs_mt.cpp.main splits the song across its own std::threads,
# so nothing beyond the exe is needed (no vcomp140.dll, no DLLs at all: static libstdc++/winpthreads).
#
# Two exes, same program:
#   demucs_mt.cpp.main.exe     x86-64-v3 (AVX2 + FMA; Haswell 2013 / Zen 1 and newer) - the fast one
#   demucs_mt.cpp.main.v2.exe  x86-64-v2 (SSE4.2), picked by engine::demucs::exe() on CPUs without AVX2
#
# Only libnyquist's WAV reader and writer are compiled (the app hands the separator a WAV and reads WAVs
# back), so its other codecs and their licences stay out.
#
# Needs: git, x86_64-w64-mingw32-g++-posix (Debian/Ubuntu: apt install mingw-w64), zip, sha256sum.
# Usage:  tools/demucs-helper/build.sh [out-dir]       (default out-dir: target/demucs-helper)
# Prints the "program" block for library/bin/demucs.json at the end.
set -euo pipefail

COMMIT=4edae497a114de1ccb17ac9b4a2b8fc0144714d7   # sevagh/demucs.cpp main, 2024 (the version in demucs.json)
HERE="$(cd "$(dirname "$0")" && pwd)"
OUT="$(mkdir -p "${1:-target/demucs-helper}" && cd "${1:-target/demucs-helper}" && pwd)"
CXX="${CXX:-x86_64-w64-mingw32-g++-posix}"
WINDRES="${WINDRES:-x86_64-w64-mingw32-windres}"
SRC="$OUT/demucs.cpp"

if [ ! -d "$SRC/.git" ]; then
  git clone -q https://github.com/sevagh/demucs.cpp "$SRC"
fi
git -C "$SRC" fetch -q origin "$COMMIT" 2>/dev/null || true
git -C "$SRC" checkout -q --force "$COMMIT"
git -C "$SRC" submodule update -q --init --depth 1 vendor/eigen vendor/libnyquist vendor/demucs

# --- patches (kept tiny and printed into the zip's README) -----------------------------------------
# 1. Windows: std::filesystem::path holds wchar_t there, so it does not convert to std::string implicitly.
sed -i 's/write_audio_file(target_waveform, p_target);/write_audio_file(target_waveform, p_target.string());/' \
  "$SRC/cli-apps/demucs_mt.cpp"
grep -q 'p_target.string()' "$SRC/cli-apps/demucs_mt.cpp"
# 2. libnyquist: register the WAV decoder only, and drop the Opus encoder (it pulls in libopus/libogg).
NQ="$SRC/vendor/libnyquist"
sed -i '/AddDecoderToTable(std::make_shared<\(WavPack\|Flac\|Vorbis\|Opus\|Musepack\|Mp3\)Decoder>());/d' "$NQ/src/Common.cpp"
grep -q 'make_shared<WavDecoder>' "$NQ/src/Common.cpp"
! grep -q 'make_shared<FlacDecoder>' "$NQ/src/Common.cpp"
if grep -q 'Opus File Encoding' "$NQ/src/Encoders.cpp"; then
  # everything from the Opus banner (two lines above the title) to the end of the file
  line=$(grep -n 'Opus File Encoding' "$NQ/src/Encoders.cpp" | cut -d: -f1)
  head -n $((line - 2)) "$NQ/src/Encoders.cpp" > "$NQ/src/Encoders.cpp.new"
  mv "$NQ/src/Encoders.cpp.new" "$NQ/src/Encoders.cpp"
fi

# --- build ----------------------------------------------------------------------------------------
# The UTF-8 manifest makes the narrow (ANSI) file APIs take UTF-8, so a model path under a non-ASCII
# user name opens (Windows 10 1903+); the app also passes the WAV paths relative to the working folder.
"$WINDRES" -I "$HERE" "$HERE/utf8.rc" -O coff -o "$OUT/utf8.res"

SOURCES=("$SRC"/src/*.cpp "$SRC/cli-apps/demucs_mt.cpp" "$NQ/src/Common.cpp" "$NQ/src/WavDecoder.cpp" "$NQ/src/Encoders.cpp")
# upstream's release flags, minus -march=native
FLAGS=(-std=c++17 -Ofast -fno-unsafe-math-optimizations -freciprocal-math -fno-signed-zeros -DNDEBUG
  -I"$SRC/src" -I"$SRC/cli-apps" -I"$SRC/vendor/eigen" -I"$NQ/include" -I"$NQ/include/libnyquist" -I"$NQ/src"
  -w -ffile-prefix-map="$OUT"=. -static -s -Wl,--no-insert-timestamp)

STAGE="$OUT/stage"
rm -rf "$STAGE" && mkdir -p "$STAGE/licenses"
build() { # $1 = -march level, $2 = exe name
  echo "building $2 ($1)…" >&2
  "$CXX" "${FLAGS[@]}" -march="$1" -mtune=generic "${SOURCES[@]}" "$OUT/utf8.res" -o "$STAGE/$2"
}
build x86-64-v3 demucs_mt.cpp.main.exe & p3=$!
build x86-64-v2 demucs_mt.cpp.main.v2.exe & p2=$!
wait $p3 && wait $p2

# --- licences and README --------------------------------------------------------------------------
cp "$SRC/LICENSE" "$STAGE/licenses/demucs.cpp-MIT.txt"
cp "$SRC/vendor/demucs/LICENSE" "$STAGE/licenses/demucs-MIT.txt"
cp "$SRC/vendor/eigen/COPYING.MPL2" "$STAGE/licenses/Eigen-MPL-2.0.txt"
cp "$NQ/LICENSE" "$STAGE/licenses/libnyquist-BSD-2-Clause.txt"
cat > "$STAGE/licenses/mingw-w64-runtime.txt" <<'EOF'
The exes are statically linked with the mingw-w64 runtime (mingw-w64 CRT: ZPL / public domain;
winpthreads: MIT) and GCC's libstdc++ / libgcc, which are GPLv3 with the GCC Runtime Library Exception:
the exception lets programs compiled with GCC be distributed under any licence, so no GPL terms apply
to these exes. Sources: https://www.mingw-w64.org/ and https://gcc.gnu.org/onlinedocs/libstdc++/manual/license.html
EOF
GCCV="$("$CXX" -dumpfullversion 2>/dev/null || "$CXX" -dumpversion)"
cat > "$STAGE/README.txt" <<EOF
Simple Editor - Stem Separation add-on

demucs.cpp (https://github.com/sevagh/demucs.cpp, MIT) at commit $COMMIT,
the Python-free C++ port of Demucs v4 (https://github.com/facebookresearch/demucs, MIT).
The htdemucs weights are a separate download (library/bin/demucs.json ▸ model).

Built by tools/demucs-helper/build.sh in the Simple Editor repository with $CXX $GCCV:
  demucs_mt.cpp.main.exe     -march=x86-64-v3 (AVX2 + FMA)
  demucs_mt.cpp.main.v2.exe  -march=x86-64-v2 (SSE4.2), for processors without AVX2
  flags: ${FLAGS[*]:0:6}, static, no OpenMP / BLAS; an embedded manifest sets the UTF-8 code page.
Patches:
  cli-apps/demucs_mt.cpp: write_audio_file(.., p_target.string()) (Windows paths are wide)
  libnyquist: WAV decoder and WAV encoder only (no other codecs compiled)

Usage: demucs_mt.cpp.main.exe <model file> <wav file> <out dir> <num threads>
Writes target_0_drums.wav, target_1_bass.wav, target_2_other.wav, target_3_vocals.wav (32-bit float).
Licences: licenses/
EOF

# --- zip (fixed times and order, so the same compiler gives the same bytes) ------------------------
find "$STAGE" -exec touch -h -d '2024-01-01 00:00:00 UTC' {} +
ZIP="$OUT/se-demucs-win64.zip"
rm -f "$ZIP"
(cd "$STAGE" && find . -type f | LC_ALL=C sort | sed 's|^\./||' | TZ=UTC zip -X -q -9 "$ZIP" -@)

SHA=$(sha256sum "$ZIP" | cut -d' ' -f1)
SIZE=$(stat -c %s "$ZIP")
echo "$ZIP"
cat <<EOF
  "program": {
    "url": "https://github.com/KashTheKing/simple-editor/releases/download/demucs-v1/se-demucs-win64.zip",
    "sha256": "$SHA",
    "size": $SIZE
  },
EOF
