# Checks the faster-whisper runner's aligner (src/engine/fw_runner.py) on synthetic data: the CTC
# Viterbi, the edge snap and the per-segment re-timing. Needs only Python 3 and numpy (no models):
#
#   python -I tools/fw-runner-check/check.py src/engine/fw_runner.py
import sys, importlib.util, numpy as np
spec = importlib.util.spec_from_file_location("r", sys.argv[1]); r = importlib.util.module_from_spec(spec); spec.loader.exec_module(r)
# synthetic emissions: vocab blank=0, A=1, B=2, |=3 ; 50 frames; A at 10-12, | at 20, B at 30-33
T, C = 50, 4
em = np.full((T, C), -10.0, np.float32); em[:, 0] = -0.01
for f in range(10, 13): em[f] = [-10, -0.01, -10, -10]
em[20] = [-10, -10, -10, -0.01]
for f in range(30, 34): em[f] = [-10, -10, -0.01, -10]
sp = r.force_align(em, [1, 3, 2])
print("spans", sp)
assert sp == [(10, 13), (20, 21), (30, 34)], sp
assert r.force_align(em[:2], [1, 2, 3]) is None
# snap_edges: tone from 1.0 to 1.5 s, word guessed 1.06..1.4 with pauses around
sr = 16000; a = np.zeros(sr * 3, np.float32); a[sr:int(1.5 * sr)] = 0.5 * np.sin(np.arange(sr // 2) * 0.3)
w = [[1.06, 1.40, "x"]]; r.snap_edges(a, w); print("snapped", w)
assert abs(w[0][0] - 1.0) <= 0.006 and abs(w[0][1] - 1.5) <= 0.006, w
# align end to end with a fake session that returns emissions over the window (20 ms frames)
class S:
    class I: name = "x"
    def get_inputs(self): return [self.I()]
    def run(self, _, feed):
        n = feed["x"].shape[1] // 320; lg = np.full((n, 4), -10.0, np.float32); lg[:, 0] = 0
        lo = 0.7  # window starts at s0-0.3 = 0.7
        def put(t0, t1, c):
            for f in range(int((t0 - lo) / 0.02), int((t1 - lo) / 0.02)): lg[f] = -10; lg[f, c] = 0
        put(1.0, 1.1, 1); put(1.2, 1.24, 3); put(1.3, 1.5, 2)
        return [lg[None]]
vocab = {"A": 1, "B": 2, "|": 3}
out, k = r.align(a, [[0.9, 1.2, "a"], [1.25, 1.6, "b"], [1.6, 1.7, "42"]], [(1.0, 1.6)], S(), vocab)
print("aligned", out, k); assert k == 2 and abs(out[0][0] - 1.0) < 0.03 and abs(out[1][0] - 1.3) < 0.03
assert out[2][0] >= out[1][1]
print("OK")
