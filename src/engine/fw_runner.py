# Simple Editor's faster-whisper runner (written into the add-on folder by the app; MIT, like the app).
#
#   python.exe se_fw.py <job.json>
#
# job.json: {wavs: [{wav, shift}], model_dir, language, prompt, align, align_dir, threads}
# stdout, one JSON object per line:
#   {"p": 0.42, "s": "status"}                    progress
#   {"seg": [start, end, text]}                  a transcribed segment (shown while it runs)
#   {"words": [[start, end, text], ...], "aligned": n, "language": "en", "device": "cpu"}   the result
#   {"error": "..."}                             fatal
#
# Word times come from faster-whisper (CTranslate2 cross-attention timestamps). With "align" they are then
# pinned to the audio the way WhisperX does it (BSD-2, github.com/m-bain/whisperX): a wav2vec2 CTC model
# (facebook/wav2vec2-base-960h, Apache-2.0, ONNX export) scores every 20 ms frame per character, and a
# Viterbi pass over the segment's characters finds where each one is spoken. Here the model runs on
# onnxruntime instead of PyTorch, so the add-on stays small.
import json
import os
import sys
import time
import wave


def emit(o):
    sys.stdout.write(json.dumps(o) + "\n")
    sys.stdout.flush()


def read_wav(path):
    import numpy as np

    with wave.open(path, "rb") as w:
        if w.getsampwidth() != 2 or w.getnchannels() != 1 or w.getframerate() != 16000:
            raise RuntimeError("expected 16 kHz mono 16-bit wav")
        raw = w.readframes(w.getnframes())
    return np.frombuffer(raw, dtype=np.int16).astype(np.float32) / 32768.0


def pick_device():
    # CUDA only when NVIDIA's cuBLAS 12 / cuDNN 9 libraries are already on this machine (they are not
    # part of the add-on: their licence does not allow bundling them here)
    # The libraries are looked for FIRST: asking CTranslate2 for CUDA devices starts the CUDA runtime, and
    # on a machine without them that stalled the CPU model load for minutes.
    try:
        import ctypes

        ctypes.WinDLL("cublas64_12.dll")
        ctypes.WinDLL("cudnn_ops64_9.dll")
        import ctranslate2

        if ctranslate2.get_cuda_device_count() > 0:
            return "cuda", "float16"
    except Exception:
        pass
    return "cpu", "int8"


# ------------------------------------------------------------------------------------------ alignment


def log_softmax(x):
    import numpy as np

    m = x.max(axis=-1, keepdims=True)
    y = x - m
    return y - np.log(np.exp(y).sum(axis=-1, keepdims=True))


def emissions(session, audio):
    import numpy as np

    x = audio.astype(np.float32)
    x = (x - x.mean()) / np.sqrt(x.var() + 1e-7)  # Wav2Vec2FeatureExtractor do_normalize
    name = session.get_inputs()[0].name
    logits = session.run(None, {name: x[None, :]})[0][0]
    return log_softmax(logits)


def force_align(em, tokens, blank=0):
    """CTC Viterbi: frame index each token starts and ends at (torchaudio's forced-alignment tutorial,
    the algorithm WhisperX uses). em: (T, C) log-probs; tokens: list of ids. Returns [(t0, t1)] per token
    or None when the audio is too short for the text."""
    import numpy as np

    T, J = em.shape[0], len(tokens)
    if J == 0 or T < J:
        return None
    tok = np.asarray(tokens)
    # trellis[t, j]: best score with j tokens emitted after frames 0..t-1. Staying on token j costs its
    # own score or a blank, whichever is likelier (CTC repeats a character over a held sound).
    trellis = np.full((T + 1, J + 1), -np.inf, dtype=np.float32)
    trellis[0, 0] = 0.0
    trellis[1:, 0] = np.cumsum(em[:, blank])
    for t in range(T):
        own = em[t, tok]
        stay = trellis[t, 1:] + np.maximum(em[t, blank], own)
        step = trellis[t, :-1] + own
        trellis[t + 1, 1:] = np.maximum(stay, step)
    if not np.isfinite(trellis[T, J]):
        return None
    # backtrack from the window's last frame, so every frame counts (stopping at the first frame where
    # all tokens fit ignores the frames after it and squeezes the last words together). A token's span
    # runs from the frame it is emitted to the last frame it is still heard; the blank frames after it
    # belong to nobody.
    spans = [[None, None] for _ in range(J)]
    j = J
    for f in range(T - 1, -1, -1):
        if j == 0:
            break
        tk = tok[j - 1]
        s = spans[j - 1]
        if trellis[f, j - 1] + em[f, tk] >= trellis[f, j] + max(em[f, blank], em[f, tk]):
            s[0] = f
            if s[1] is None:
                s[1] = f
            j -= 1
        elif s[1] is None and em[f, tk] > em[f, blank]:
            s[1] = f
    if j > 0:
        return None
    return [(a, b + 1) for a, b in spans]


def snap_edges(audio, words, sr=16000):
    """CTC fires a character a few frames after it starts sounding (~60 ms on the test speech), so a
    word edge next to a pause is moved to where the sound itself begins / dies away: 5 ms RMS frames,
    the edge walks to the first / last frame above a tenth of the word's loudest. Edges between two
    words that run together are left where the aligner put them."""
    import numpy as np

    hop = 80  # 5 ms
    n = len(audio) // hop
    if n < 4:
        return
    rms = np.sqrt(np.mean(audio[: n * hop].reshape(n, hop) ** 2, axis=1))
    for i, w in enumerate(words):
        a, b = int(w[0] * sr / hop), int(w[1] * sr / hop)
        if b <= a or b > n:
            continue
        peak = float(rms[a:b].max())
        if peak <= 1e-4:
            continue
        thr = 0.1 * peak
        prev_end = words[i - 1][1] if i > 0 else 0.0
        nxt = words[i + 1][0] if i + 1 < len(words) else len(audio) / sr
        if w[0] - prev_end >= 0.08:
            lo = max(int(prev_end * sr / hop), a - 40)  # look back at most 200 ms, never into the last word
            on = next((k for k in range(lo, b) if rms[k] > thr), a)
            w[0] = round(min(on * hop / sr, w[1]), 3)
        if nxt - w[1] >= 0.08:
            hi = min(int(nxt * sr / hop), b + 40, n)
            k = hi - 1
            while k > int(w[0] * sr / hop) and rms[k] <= thr:
                k -= 1
            w[1] = round(max((k + 1) * hop / sr, w[0]), 3)


def align(audio, words, segs, session, vocab):
    """Re-time `words` ([start, end, text], seconds) segment by segment. Words with nothing the model can
    spell (digits, symbols) keep faster-whisper's times, clamped between their aligned neighbours."""
    import numpy as np

    sr = 16000
    out = [list(w) for w in words]
    aligned = 0
    wi = 0
    for si, (s0, s1) in enumerate(segs):
        idx = []
        while wi < len(words) and words[wi][0] < s1 + 1e-3:
            idx.append(wi)
            wi += 1
        if not idx:
            continue
        # the window: the segment plus a little room, never into the neighbouring segments
        lo = max(0.0, s0 - 0.3, segs[si - 1][1] if si > 0 else 0.0)
        hi = min(len(audio) / sr, s1 + 0.3, segs[si + 1][0] if si + 1 < len(segs) else 1e9)
        a, b = int(lo * sr), int(hi * sr)
        if b - a < 400:
            continue
        em = emissions(session, audio[a:b])
        ratio = (b - a) / sr / em.shape[0]
        tokens, owner = [], []
        for k, i in enumerate(idx):
            chars = [c for c in words[i][2].upper() if c in vocab]
            if k > 0 and chars and tokens:
                tokens.append(vocab["|"])
                owner.append(-1)
            for c in chars:
                tokens.append(vocab[c])
                owner.append(i)
        spans = force_align(em, tokens) if tokens else None
        if spans is None:
            continue
        got = {}
        for (f0, f1), i in zip(spans, owner):
            if i < 0:
                continue
            g = got.setdefault(i, [f0, f1])
            g[0], g[1] = min(g[0], f0), max(g[1], f1)
        for i, (f0, f1) in got.items():
            out[i][0] = round(lo + f0 * ratio, 3)
            out[i][1] = round(lo + f1 * ratio, 3)
            aligned += 1
    snap_edges(audio, out)
    # unaligned words: inside the gap their neighbours leave
    for i, w in enumerate(out):
        prev_end = out[i - 1][1] if i > 0 else 0.0
        nxt = out[i + 1][0] if i + 1 < len(out) else 1e9
        w[0] = min(max(w[0], prev_end), nxt)
        w[1] = max(min(w[1], nxt), w[0])
    return out, aligned


# ------------------------------------------------------------------------------------------ main


def main():
    job = json.load(open(sys.argv[1], encoding="utf-8"))
    t0 = time.time()
    emit({"p": 0.01, "s": "Loading the model"})
    from faster_whisper import WhisperModel

    # every span is its own wav (speech islands), all through one model load; "shift" moves its times
    spans = job.get("wavs") or [{"wav": job["wav"], "shift": 0.0}]
    audios = [read_wav(s["wav"]) for s in spans]
    total = max(sum(len(a) for a in audios) / 16000.0, 0.1)
    device, compute = pick_device()
    threads = int(job.get("threads") or 0)
    try:
        model = WhisperModel(job["model_dir"], device=device, compute_type=compute, cpu_threads=threads)
    except Exception:
        if device == "cpu":
            raise
        device, compute = "cpu", "int8"
        model = WhisperModel(job["model_dir"], device=device, compute_type=compute, cpu_threads=threads)
    lang = (job.get("language") or "").strip().lower()
    want_align = bool(job.get("align"))
    share = 0.8 if want_align else 0.97
    session = vocab = None
    words_all, aligned, language, done = [], 0, lang, 0.0
    for n, (span, audio) in enumerate(zip(spans, audios)):
        dur = len(audio) / 16000.0
        segments, info = model.transcribe(
            audio,
            language=None if lang in ("", "auto") else lang,
            initial_prompt=(job.get("prompt") or "").strip() or None,
            word_timestamps=True,
            vad_filter=True,
            condition_on_previous_text=False,  # WhisperX's choice: no repetition loops after a misheard line
            beam_size=5,
        )
        language = info.language
        words, segs = [], []
        for s in segments:
            segs.append((s.start, s.end))
            p = 0.02 + share * (done + min(s.end, dur)) / total
            emit({"seg": [round(s.start + span["shift"], 3), round(s.end + span["shift"], 3), s.text.strip()], "p": round(p, 4)})
            for w in s.words or []:
                t = w.word.strip()
                if t:
                    words.append([w.start, max(w.end, w.start), t])
        if want_align and words and info.language == "en":
            emit({"p": round(0.02 + share * (done + dur) / total, 4), "s": "Pinning words to the audio"})
            if session is None:
                import onnxruntime as ort

                d = job["align_dir"]
                vocab = json.load(open(os.path.join(d, "vocab.json"), encoding="utf-8"))
                opts = ort.SessionOptions()
                if threads:
                    opts.intra_op_num_threads = threads
                session = ort.InferenceSession(os.path.join(d, "model.onnx"), opts, providers=["CPUExecutionProvider"])
            words, k = align(audio, words, segs, session, vocab)
            aligned += k
        for w in words:
            words_all.append([round(w[0] + span["shift"], 3), round(w[1] + span["shift"], 3), w[2]])
        done += dur
    if want_align and words_all and language != "en":
        emit({"p": 0.97, "s": "Precise word timing is English only: kept the model's word times"})
    emit({"words": words_all, "aligned": aligned, "language": language, "device": device, "secs": round(time.time() - t0, 2)})


if __name__ == "__main__":
    try:
        main()
    except Exception as e:  # noqa: BLE001 - the app shows the message
        emit({"error": f"{type(e).__name__}: {e}"})
        sys.exit(1)
