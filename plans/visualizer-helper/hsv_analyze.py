# Hardstyle visualizer: analyse a song (+ its Demucs stems) into hand keys for the Olympus Storm shader.
#
#   python hsv_analyze.py SONG STEMS_DIR OUT_DIR [--drops 26.39,52.68] [--calm 78.1] [--ffmpeg ffmpeg]
#
# STEMS_DIR holds vocals.wav / other.wav / drums.wav (Simple Editor's audio.stems cache folder).
# Writes OUT_DIR/analysis.json (times, for reading) and OUT_DIR/keys.lua (a complete Luau script:
# paste it as scripts_eval `code`, after setting CLIP at its top, to key the visualizer clip).
#
# Keys written (param name -> what it carries):
#   kclock  one key per hit, value = hit number, linear between: fract() = phase since the hit, floor() = count
#   kpow    each hit's strength, Hold keys: 1.0 on the bar's downbeat, 0.55-0.8 other grid beats, <=0.5 off-grid stabs
#   sclock  same idea for the high screeches, ramping over 0.6 s then holding (phase = time since the screech)
#   screech the synth stem's 1.5-8 kHz loudness, 0..2.5 (electricity goes berserk above ~2)
#   vox     the vocal level: slow held envelope when singing, a small fast one when talking
#   vib     vibrato (-1..1), pitch wobble in the 3.5-8.5 Hz band, only on held sung notes
#   fx      0 before the first drop, 1 from it
#   kick    1.2 normally, 2.2 between the second drop and the calm part ("go harder" dial)
#   beam    pre-drop glow rising 0.3 -> 1, 3 (white flash) on each drop decaying to 1 in 0.4 s
# The app's own audio.visualizer bake supplies level / band1 / band4 / wave; this script never touches those.
import argparse, json, os, subprocess, sys
import numpy as np, scipy.signal as sg
from scipy.ndimage import uniform_filter1d, median_filter

ap = argparse.ArgumentParser()
ap.add_argument('song'); ap.add_argument('stems'); ap.add_argument('out')
ap.add_argument('--drops', default='', help='comma list of drop times (s); default: detected')
ap.add_argument('--calm', type=float, default=None, help='time the song calms down after the last drop')
ap.add_argument('--after', type=float, default=0.0, help='no drop before this time (from the lyrics: the line before a [DROP] header)')
ap.add_argument('--ffmpeg', default='ffmpeg')
ap.add_argument('--bpm', default='130,175', help='tempo search range')
A = ap.parse_args()
os.makedirs(A.out, exist_ok=True)
SR = 22050

def load(path, ch):
    raw = subprocess.run([A.ffmpeg, '-v', 'error', '-i', path, '-ac', str(ch), '-ar', str(SR), '-f', 'f32le', '-'],
                         stdout=subprocess.PIPE, check=True).stdout
    x = np.frombuffer(raw, dtype=np.float32)
    return x.reshape(-1, ch) if ch > 1 else x

song = load(A.song, 2).mean(1)
voc = load(os.path.join(A.stems, 'vocals.wav'), 1)
oth = load(os.path.join(A.stems, 'other.wav'), 1)
drm = load(os.path.join(A.stems, 'drums.wav'), 1)
DUR = len(song) / SR

# ---------------------------------------------------------------- hits (Ellis DP beat tracking + stabs)
hop = 256; fps = SR / hop
f, _, Z = sg.stft(song, SR, nperseg=1024, noverlap=1024 - hop)
P = np.abs(Z) ** 2 + 1e-10
def onset(lo, hi):
    L = np.log(P[(f >= lo) & (f < hi)])
    return np.concatenate([[0], np.maximum(L[:, 1:] - L[:, :-1], 0).mean(0)])
def norm(o):
    o = np.maximum(o - median_filter(o, int(fps)), 0)
    return o / (np.percentile(o[o > 0], 98) + 1e-9)
low = norm(onset(35, 160)); full = norm(onset(35, 8000))
env = 0.7 * low + 0.3 * full
n = len(env)
ac = np.correlate(env - env.mean(), env - env.mean(), 'full')[n - 1:]
lags = np.arange(len(ac)); bpm = 60 * fps / np.maximum(lags, 1)
b0, b1 = (float(v) for v in A.bpm.split(','))
m = (bpm > b0) & (bpm < b1)
lag = lags[m][np.argmax(ac[m])]
y0, y1, y2 = ac[lag - 1], ac[lag], ac[lag + 1]
period = lag + 0.5 * (y0 - y2) / (y0 - 2 * y1 + y2)
score = env.copy(); back = -np.ones(n, int)
for i in range(n):
    lo_, hi_ = max(int(i - 2 * period), 0), int(i - period / 2)
    if hi_ <= 0: continue
    prev = np.arange(lo_, hi_)
    c = score[prev] - 200.0 * np.log((i - prev) / period) ** 2
    j = np.argmax(c); score[i] = env[i] + c[j]; back[i] = prev[j]
i = n - 1 - np.argmax(score[::-1][:int(period)])
beats = []
while i >= 0: beats.append(i); i = back[i]
beats = np.array(beats[::-1])
near = lambda o, i, w=3: o[max(i - w, 0):i + w + 1].max()
st = np.array([max(near(low, i), 0.5 * near(full, i)) for i in beats])
keep = st > 0.06
bi, st = beats[keep], st[keep]; bt = bi / fps
hits = [(float(a), float(min(b, 1.0))) for a, b in zip(bt, st)]
for a, b in zip(bi[:-1], bi[1:]):                       # off-beat stabs half way between beats
    if b - a > 1.6 * period: continue
    c = (a + b) // 2; s = max(near(low, c, 4), 0.6 * near(full, c, 4))
    if s > 0.45: hits.append((c / fps, min(0.75 * s, 1.0)))
pk, _ = sg.find_peaks(env, height=0.28, distance=int(0.085 * fps))   # rolls the grid missed
hits += [(p / fps, min(0.8 * env[p], 1.0)) for p in pk]
hits.sort(); mh = []
for h in hits:                                          # merge hits closer than 85 ms
    if mh and h[0] - mh[-1][0] < 0.085: mh[-1] = (mh[-1][0], max(mh[-1][1], h[1]))
    else: mh.append(h)
best = max(range(4), key=lambda ph: st[ph::4].sum())    # downbeat phase = most low end
down = set(np.round(bt[best::4], 3))
K = []
for a, w in mh:
    w = 0.25 + 0.75 * w ** 0.8
    g = bt[np.argmin(np.abs(bt - a))]
    if abs(g - a) < 0.04: w = 1.0 if round(g, 3) in down else min(max(w, 0.55), 0.8)
    else: w = min(w, 0.5)
    K.append((round(a, 3), round(w, 2)))
# kick gate: hits with no real low-end thump (just hats/claps) drop to 0 strength so nothing pops on them
lo_b, lo_a = sg.butter(4, [40, 150], btype='band', fs=SR); lowsig = sg.filtfilt(lo_b, lo_a, song)
def db(i, j): seg = lowsig[max(i, 0):max(j, 1)]; return 20 * np.log10(np.sqrt((seg ** 2).mean()) + 1e-9)
after = np.array([db(int(t * SR), int((t + 0.06) * SR)) for t, _ in K])
lvl = np.clip((after - np.percentile(after, 95) + 18) / 6, 0, 1)
ons = np.array([low[max(int(t * fps) - 2, 0):int(t * fps) + 4].max() for t, _ in K])
fo = np.array([full[max(int(t * fps) - 2, 0):int(t * fps) + 4].max() for t, _ in K])
gate = lvl * np.clip((ons - 0.2) / 0.15, 0, 1)

# ---------------------------------------------------------------- drops (drums stem: quiet -> loud and staying)
# A drop = the bass coming back after >= 1 s of silence. The first one is the first drop; a later one only
# counts when the 4 s after it are louder (bass + drums) than any section before it (a bigger drop).
bas = load(os.path.join(A.stems, 'bass.wav'), 1)
hopd = int(SR * 0.05)
def db50(x):
    k = len(x) // hopd; return 20 * np.log10(np.sqrt((x[:k * hopd].reshape(k, hopd) ** 2).mean(1)) + 1e-7)
bdb, ddb = db50(bas), db50(drm)
m = min(len(bdb), len(ddb)); bdb, ddb = bdb[:m], ddb[:m]
td = np.arange(m) * 0.05
pk_b = np.percentile(bdb, 99)
cs = np.concatenate([[0], np.cumsum(bdb < pk_b - 35)])
quiet = np.array([k >= 30 and cs[k - 10] - cs[k - 30] >= 18 for k in range(m)])  # silent for 1 s ending 0.5 s before k
energy = 10 * np.log10((10 ** (bdb / 10) + 10 ** (ddb / 10)))
if A.drops:
    drops = [float(v) for v in A.drops.split(',')]
else:
    drops = []; best_e = -1e9; k = 40          # ignore the first 2 s
    while k < m:
        if quiet[k] and bdb[k] > pk_b - 15:
            e4 = float(energy[k:k + 80].mean())
            # a real drop has the kick in it too (bass under an intro vocal is not a drop)
            kicks_in = sum(1 for (t, _), g in zip(K, gate) if td[k] - 0.3 <= t <= td[k] + 4 and g > 0.3) >= 6
            if kicks_in and (not drops or e4 > best_e + 2.0):
                # where the bass started rising out of the silence, snapped to the nearest hit
                k0 = k
                while k0 > 0 and bdb[k0 - 1] > pk_b - 55: k0 -= 1
                t0 = float(td[k0]) - 0.05
                cand = [t for t, _ in K if abs(t - t0) <= 0.15]
                drops.append(min(cand, key=lambda t: abs(t - t0)) if cand else t0)
            best_e = max(best_e, e4); k += 80
        k += 1
drops = [d for d in drops if d > max(2.0, A.after - 1.0)]
calm = A.calm
if calm is None and drops:
    ce_ = np.concatenate([[0], np.cumsum(energy)])
    e2 = np.array([(ce_[min(k + 160, m)] - ce_[k]) / max(min(k + 160, m) - k, 1) for k in range(m)])   # mean of the next 8 s
    ref = float(energy[int(drops[-1] / 0.05):int(drops[-1] / 0.05) + 160].mean())
    # the calm outro: from the last moment the next 8 s are still near the drop's energy, to the end
    loud = np.where((td > drops[-1]) & (e2 >= ref - 4))[0]
    calm = float(td[loud[-1]] + 8.0) if len(loud) else DUR
    if calm > DUR - 4: calm = DUR
fx0 = drops[0] if drops else 0.0
# after the first drop, strong percussive onsets that the kick gate killed come back as medium hits
KW = []
for (t, w), g, o in zip(K, gate, fo):
    w2 = round(float(w * g), 2)
    if t >= fx0 and w2 < 0.3 and o > 0.35: w2 = max(w2, round(float(min(0.75, 0.3 + 0.5 * (o - 0.35) / 0.65)), 2))
    if calm is not None and t >= calm: w2 = round(w2 * 0.35, 2)      # the calm outro: barely any pops
    KW.append(w2)
KT = [t for t, _ in K]

# ---------------------------------------------------------------- screeches (brightness jumps in the mix)
Hb = P[(f >= 3000) & (f < 9000)].sum(0); Mb = P[(f >= 200) & (f < 3000)].sum(0)
br = uniform_filter1d(np.log(Hb) - np.log(Mb), 3)
jump = uniform_filter1d(br, int(0.12 * fps), origin=-int(0.06 * fps)) - uniform_filter1d(br, int(0.25 * fps), origin=int(0.125 * fps))
loud = uniform_filter1d(np.log(Hb), int(0.1 * fps), origin=-int(0.05 * fps))
pk, _ = sg.find_peaks(jump, height=0.5, distance=int(0.15 * fps))
ST = [round(p / fps, 3) for p in pk if loud[p] > np.percentile(loud, 40) and p / fps >= fx0 - 0.05]

# ---------------------------------------------------------------- synth stem 1.5-8 kHz envelope -> screech 0..2.5
sb, sa = sg.butter(4, [1500, 8000], btype='band', fs=SR); hs = sg.filtfilt(sb, sa, oth)
h10 = int(SR * 0.01); nn = len(hs) // h10
se = 20 * np.log10(np.sqrt((hs[:nn * h10].reshape(nn, h10) ** 2).mean(1)) + 1e-7)
lo_, hi_ = np.percentile(se, 50), np.percentile(se, 99.5)
scr = np.clip((se - lo_) / (hi_ - lo_), 0, 1) * 2.5
ys = np.zeros_like(scr); at, rl = 1 - np.exp(-0.01 / 0.03), 1 - np.exp(-0.01 / 0.25)
for k in range(1, nn): ys[k] = ys[k - 1] + (scr[k] - ys[k - 1]) * (at if scr[k] > ys[k - 1] else rl)
ts10 = np.arange(nn) * 0.01
ys[ts10 < fx0] = 0.0

# ---------------------------------------------------------------- vocals: level, talk vs sing, vibrato
n = len(voc) // h10
rms = np.sqrt((voc[:n * h10].reshape(n, h10) ** 2).mean(1)); ldb = 20 * np.log10(rms + 1e-7)
lo_, hi_ = np.percentile(ldb[ldb > -80], 55), np.percentile(ldb, 99.5)
lv = np.clip((ldb - lo_) / (hi_ - lo_), 0, 1) ** 1.3
def follow(x, att, rel):
    y = np.zeros_like(x); a, r = 1 - np.exp(-0.01 / att), 1 - np.exp(-0.01 / rel)
    for k in range(1, len(x)): y[k] = y[k - 1] + (x[k] - y[k - 1]) * (a if x[k] > y[k - 1] else r)
    return y
yv = follow(lv, 0.08, 0.4)                               # held, smooth: 80 ms attack, 400 ms release
ft = follow(lv, 0.03, 0.12)                              # fast syllables for talking
vb, va = sg.butter(4, [80, 1200], btype='band', fs=SR); c = sg.filtfilt(vb, va, voc)
win = int(SR * 0.04); lagmin, lagmax = int(SR / 1000), int(SR / 80)
cents = np.zeros(n); ok = np.zeros(n, bool)
for k in range(min(n, (len(c) - win) // h10)):
    s = c[k * h10:k * h10 + win]; s = s - s.mean()
    if (s * s).sum() < 1e-5: continue
    a2 = np.correlate(s, s, 'full')[win - 1:]; a2 = a2 / a2[0]; seg = a2[lagmin:lagmax]; j = np.argmax(seg)
    if seg[j] < 0.8: continue
    L = j + lagmin
    if 1 <= j < len(seg) - 1:
        q0, q1, q2 = seg[j - 1], seg[j], seg[j + 1]; L = L + 0.5 * (q0 - q2) / (q0 - 2 * q1 + q2 + 1e-9)
    cents[k] = 1200 * np.log2(SR / L / 100.0); ok[k] = True
tot = np.zeros(n); k = n - 1                              # length of each voiced run
while k >= 0:
    if ok[k]:
        j = k
        while j >= 0 and ok[j]: j -= 1
        tot[j + 1:k + 1] = k - j; k = j
    else: k -= 1
W = 50
vf = uniform_filter1d(ok.astype(float), W)
held = uniform_filter1d(np.clip(tot / 25.0, 0, 1) * ok, W)
ph = np.array([np.median(cents[max(0, k - W // 2):k + W // 2][ok[max(0, k - W // 2):k + W // 2]])
               if ok[max(0, k - W // 2):k + W // 2].any() else 0 for k in range(n)])
ph_n = np.clip((ph - np.percentile(ph[ph > 0], 20)) / 600.0, 0, 1) if (ph > 0).any() else ph * 0
sing = np.clip(1.6 * held * (0.4 + 0.6 * vf), 0, 1)
sing = uniform_filter1d(np.clip(0.55 * sing + 0.25 * ph_n + 0.3 * uniform_filter1d(lv, W), 0, 1) * (yv > 0.05), 30)
s12 = np.clip(sing, 0, 1) ** 1.2
vox = (1 - s12) * ft * 0.4 + s12 * yv
note = median_filter(cents, 31)
dv = np.where(ok, np.clip(cents - note, -80, 80), 0)
bb, ba = sg.butter(2, [3.5, 8.5], btype='band', fs=100.0)
vib = np.clip(sg.filtfilt(bb, ba, dv) / 55.0, -1, 1) * np.clip(yv * 1.5, 0, 1) * s12
tv = np.arange(n) * 0.01

# ---------------------------------------------------------------- simplify curves (Ramer-Douglas-Peucker)
sys.setrecursionlimit(20000)
def simp(ts, ys, tol):
    keep = {0, len(ts) - 1}
    def rdp(i, j):
        if j <= i + 1: return
        seg = ys[i] + (ys[j] - ys[i]) * (ts[i + 1:j] - ts[i]) / (ts[j] - ts[i]); e = np.abs(ys[i + 1:j] - seg); k = int(np.argmax(e))
        if e[k] > tol: m = i + 1 + k; keep.add(m); rdp(i, m); rdp(m, j)
    rdp(0, len(ts) - 1); return sorted(keep)
def curve(t, y, step, tol):
    ts = np.arange(0, DUR, step); vs = np.interp(ts, t, y)
    return [[round(float(ts[i]), 3), round(float(vs[i]), 3), 'Linear'] for i in simp(ts, vs, tol)]

keys = {}
keys['kclock'] = [[0.0, 0.0, 'Linear']] + [[t, float(i + 1), 'Linear'] for i, t in enumerate(KT)]
keys['kpow'] = [[0.0, 0.0, 'Hold']] + [[t, w, 'Hold'] for t, w in zip(KT, KW)]
sc = [[0.0, 0.0, 'Hold']]
for i, t in enumerate(ST):
    nxt = ST[i + 1] if i + 1 < len(ST) else t + 1.0
    sc.append([t, float(i + 1), 'Linear']); sc.append([round(min(t + 0.6, nxt - 0.01), 3), float(i + 1) + 0.99, 'Hold'])
keys['sclock'] = sc
keys['screech'] = curve(ts10, ys, 0.02, 0.08)
keys['vox'] = curve(tv, vox, 0.03, 0.035)
keys['vib'] = curve(tv, vib, 1 / 30, 0.09)
keys['fx'] = [[0.0, 0.0, 'Hold'], [round(fx0, 3), 1.0, 'Hold']] if drops else [[0.0, 1.0, 'Hold']]
kk = [[0.0, 1.2, 'Hold']]
if len(drops) >= 2: kk.append([round(drops[1], 3), 2.2, 'Hold']); kk.append([round(calm or DUR, 3), 1.2, 'Hold'])
keys['kick'] = kk
bm = [[0.0, 0.3, 'Linear']]
for d in drops:
    bm += [[round(max(d - 6.0, bm[-1][0] + 0.01), 3), 0.45, 'EaseIn'], [round(d - 0.02, 3), 1.0, 'Linear'],
           [round(d, 3), 3.0, 'EaseOut'], [round(d + 0.4, 3), 1.0, 'Linear']]
keys['beam'] = bm

info = {'duration': DUR, 'bpm': round(60 * fps / period, 2), 'hits': len(KT), 'screeches': len(ST),
        'drops': [round(d, 3) for d in drops], 'calm': calm,
        'hits_per_8s': [sum(1 for t, w in zip(KT, KW) if s <= t < s + 8 and w > 0.05) for s in range(0, int(DUR) + 1, 8)],
        'key_counts': {k: len(v) for k, v in keys.items()}}
json.dump({'info': info, 'hits': list(zip(KT, KW)), 'screeches': ST, 'keys': keys}, open(os.path.join(A.out, 'analysis.json'), 'w'))
# keys.N.lua: complete Luau scripts of <= 900 keys each (a plain scripts.run is cut off after 5 s)
APPLY = ['assert(CLIP ~= 0, "set CLIP")', 'local n = 0', 'for name, ks in pairs(KEYS) do', '  for _, k in ipairs(ks) do',
         '    editor.tool("clip.keyframe", {clip_id = CLIP, property = PREFIX .. name, t = k[1], value = k[2], ease = k[3]})',
         '    n = n + 1', '  end', 'end', 'return n']
chunks, cur, cnt = [], {}, 0
for name, ks in keys.items():
    for i in range(0, len(ks), 900):
        part = ks[i:i + 900]
        if cnt + len(part) > 900 and cur: chunks.append(cur); cur, cnt = {}, 0
        cur.setdefault(name, []).extend(part); cnt += len(part)
if cur: chunks.append(cur)
for f in os.listdir(A.out):
    if f.startswith('keys.') and f.endswith('.lua'): os.remove(os.path.join(A.out, f))
for ci, ch in enumerate(chunks, 1):
    L = ['-- generated by hsv_analyze.py: visualizer keys, part %d of %d' % (ci, len(chunks)),
         'local CLIP = 0 -- set to the visualizer clip id', 'local PREFIX = "Olympus Storm: "', 'local KEYS = {']
    for name, ks in ch.items():
        L.append('  %s = {%s},' % (name, ','.join('{%g,%g,"%s"}' % (t, v, e) for t, v, e in ks)))
    L += ['}'] + APPLY
    open(os.path.join(A.out, 'keys.%d.lua' % ci), 'w', encoding='utf8').write('\n'.join(L) + '\n')
info['key_scripts'] = len(chunks)
print(json.dumps(info, indent=1))
