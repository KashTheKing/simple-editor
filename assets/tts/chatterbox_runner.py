# Simple Editor's Chatterbox runner (MIT, like the app). Runs Resemble AI's Chatterbox through the
# onnx-community ONNX export (MIT) on onnxruntime (DirectML when there is a GPU, else the CPU).
# Ported from the export's model card; no torch, librosa or transformers.
#
# Request: one JSON object on stdin:
#   {"models": dir, "ref": 24 kHz mono 16-bit WAV, "lines": [str], "exaggeration": 0.5,
#    "out": WAV path, "provider": "auto" | "dml" | "cpu", "lm": "language_model_q4"}
# Progress on stdout: "provider <name>", then "progress <0..1>" per line; errors on stderr, exit 1.
import json
import os
import sys
import time
import wave

import numpy as np
import onnxruntime as ort
from tokenizers import Tokenizer

SR = 24000
START, STOP = 6561, 6562
LAYERS, KV_HEADS, HEAD_DIM = 30, 16, 64
PENALTY = 1.2
GAP = np.zeros(int(SR * 0.18), dtype=np.float32)


def say(line):
    sys.stdout.write(line + "\n")
    sys.stdout.flush()


def session(models, name, gpu):
    opts = ort.SessionOptions()
    opts.log_severity_level = 3
    path = os.path.join(models, "onnx", name + ".onnx")
    if gpu:
        opts.enable_mem_pattern = False
        opts.execution_mode = ort.ExecutionMode.ORT_SEQUENTIAL
        return ort.InferenceSession(path, opts, providers=["DmlExecutionProvider", "CPUExecutionProvider"])
    return ort.InferenceSession(path, opts, providers=["CPUExecutionProvider"])


def sessions(models, lm, provider):
    # The language model stays on the CPU: DirectML rejects its GroupQueryAttention node, and the q4
    # MatMulNBits kernels are fast on the CPU anyway. The encoder and the decoder (the heavy convolutions)
    # go to the GPU through DirectML when there is one.
    gpu = provider != "cpu" and "DmlExecutionProvider" in ort.get_available_providers()
    if gpu:
        try:
            return [session(models, n, True) for n in ("speech_encoder", "embed_tokens")] + [
                session(models, lm, False),
                session(models, "conditional_decoder", True),
            ], "directml"
        except Exception:  # a GPU without DirectML 12 support: the CPU
            pass
    return [session(models, n, False) for n in ("speech_encoder", "embed_tokens", lm, "conditional_decoder")], "cpu"


def read_wav(path):
    with wave.open(path, "rb") as w:
        if w.getframerate() != SR or w.getnchannels() != 1 or w.getsampwidth() != 2:
            raise ValueError("reference must be 24 kHz mono 16-bit")
        pcm = np.frombuffer(w.readframes(w.getnframes()), dtype=np.int16)
    return (pcm.astype(np.float32) / 32768.0)[np.newaxis, :]


def speak(s, tok, text, cond, exaggeration, max_tokens):
    enc, emb, lm, _ = s
    ids = np.array([tok.encode(text).ids], dtype=np.int64)
    pos = np.where(ids >= START, 0, np.arange(ids.shape[1])[np.newaxis, :] - 1)
    feed = {"input_ids": ids, "position_ids": pos, "exaggeration": np.array([exaggeration], dtype=np.float32)}
    generated = np.array([[START]], dtype=np.int64)
    past = None
    for i in range(max_tokens):
        x = emb.run(None, feed)[0]
        if i == 0:
            x = np.concatenate((cond[0], x), axis=1)
            past = {
                f"past_key_values.{l}.{kv}": np.zeros([1, KV_HEADS, 0, HEAD_DIM], dtype=np.float32)
                for l in range(LAYERS)
                for kv in ("key", "value")
            }
            mask = np.ones((1, x.shape[1]), dtype=np.int64)
        logits, *present = lm.run(None, dict(inputs_embeds=x, attention_mask=mask, **past))
        logits = logits[:, -1, :].astype(np.float32)
        seen = np.take_along_axis(logits, generated, axis=1)
        np.put_along_axis(logits, generated, np.where(seen < 0, seen * PENALTY, seen / PENALTY), axis=1)
        nxt = np.argmax(logits, axis=-1, keepdims=True).astype(np.int64)
        generated = np.concatenate((generated, nxt), axis=-1)
        if int(nxt[0, 0]) == STOP:
            break
        feed["input_ids"] = nxt
        feed["position_ids"] = np.full((1, 1), i + 1, dtype=np.int64)
        mask = np.concatenate([mask, np.ones((1, 1), dtype=np.int64)], axis=1)
        for j, k in enumerate(past):
            past[k] = present[j]
    tokens = generated[:, 1:]
    if tokens.shape[1] and int(tokens[0, -1]) == STOP:
        tokens = tokens[:, :-1]
    return tokens


def main():
    req = json.loads(sys.stdin.read())
    t0 = time.time()
    s, used = sessions(req["models"], req.get("lm", "language_model_q4"), req.get("provider", "auto"))
    say("provider " + used)
    tok = Tokenizer.from_file(os.path.join(req["models"], "tokenizer.json"))
    cond_emb, prompt_token, x_vector, prompt_feat = s[0].run(None, {"audio_values": read_wav(req["ref"])})
    say(f"timing load {time.time() - t0:.2f}")
    t0 = time.time()
    lines = [l.strip() for l in req["lines"] if l.strip()]
    out = []
    for n, line in enumerate(lines):
        # ~25 speech tokens a second; a line rarely runs past 6 characters a token-second
        budget = min(1500, 60 + len(line) * 6)
        tokens = speak(s, tok, line, (cond_emb,), float(req.get("exaggeration", 0.5)), budget)
        tokens = np.concatenate([prompt_token, tokens], axis=1)
        wav = s[3].run(None, {"speech_tokens": tokens, "speaker_embeddings": x_vector, "speaker_features": prompt_feat})[0]
        out.append(np.squeeze(wav, axis=0).astype(np.float32))
        out.append(GAP)
        say(f"progress {(n + 1) / len(lines):.3f}")
    say(f"timing speak {time.time() - t0:.2f}")
    audio = np.concatenate(out[:-1]) if out else GAP
    peak = float(np.max(np.abs(audio))) if audio.size else 0.0
    if peak > 0.89:  # keep 1 dB of headroom
        audio = audio * (0.89 / peak)
    pcm = (np.clip(audio, -1.0, 1.0) * 32767.0).astype("<i2")
    with wave.open(req["out"], "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(SR)
        w.writeframes(pcm.tobytes())


if __name__ == "__main__":
    try:
        main()
    except Exception as e:
        sys.stderr.write(f"{type(e).__name__}: {e}\n")
        sys.exit(1)
