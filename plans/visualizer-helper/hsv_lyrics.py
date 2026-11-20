# Hardstyle visualizer: the exact lyrics -> timed karaoke text clips (Lyric Chrome shader), no hand timing.
#
#   python hsv_lyrics.py LYRICS.txt TRANSCRIPT.json VOCALS.wav OUT_DIR --analysis analysis.json
#        [--greek] [--track 3] [--vis CLIP] [--font "Palatino Linotype" --ttf C:/Windows/Fonts/pala.ttf --size 58 --y 330]
#
# LYRICS.txt  the user's words, one on-screen line per text line. [SECTION] headers, blank lines and quotes are
#             ignored; a line is shown exactly as written (upper-cased).
# TRANSCRIPT  the app's word transcript of the song clip (media.transcript JSON: {"words":[{text,start,end}]}).
# Writes OUT_DIR/lyrics.json (the plan, for reading) and OUT_DIR/lyrics.lua: a complete Luau script for
# scripts_eval that creates every line as a text clip with the Lyric Chrome shader and all its keys, and keys
# the visualizer's `lyric` param (flattens the waveform under a line) when --vis is given.
#
# Word times: each lyric word is matched in order to the transcript (fuzzy), unmatched words are spread by
# letter count between matched neighbours, then every start is snapped to the nearest syllable onset on the
# vocal stem; the last word of a line is held until the voice actually stops (so "NOWWWW" fills slowly).
# Line clips: on screen 0.35 s before the first word, off when the voice stops (+0.3 s), never overlapping.
import argparse, difflib, json, os, re, subprocess
import numpy as np, scipy.signal as sg
from PIL import ImageFont

ap = argparse.ArgumentParser()
ap.add_argument('lyrics'); ap.add_argument('transcript'); ap.add_argument('vocals'); ap.add_argument('out')
ap.add_argument('--analysis', help='analysis.json from hsv_analyze.py (drops -> gold lines, giant line)')
ap.add_argument('--greek', action='store_true', help='A->Λ E->Σ O->Θ Y->Ψ (Greek look)')
ap.add_argument('--track', type=int, default=3, help='video track index for the lyric clips (above the visualizer)')
ap.add_argument('--vis', type=int, default=0, help='visualizer clip id: key its `lyric` param')
ap.add_argument('--font', default='Palatino Linotype'); ap.add_argument('--ttf', default='C:/Windows/Fonts/pala.ttf')
ap.add_argument('--size', type=int, default=58); ap.add_argument('--y', type=float, default=330)
ap.add_argument('--shader', default=os.path.join(os.path.dirname(os.path.abspath(__file__)), 'lyric-chrome.glsl'))
ap.add_argument('--ffmpeg', default='ffmpeg')
A = ap.parse_args()
os.makedirs(A.out, exist_ok=True)

# ---------------------------------------------------------------- lyrics
lines = []
for raw in open(A.lyrics, encoding='utf8'):
    s = raw.strip().strip('"“”').strip()
    if not s or re.fullmatch(r'\[.*\]', s): continue
    lines.append(s.upper())
norm = lambda t: re.sub(r'[^A-Z0-9]', '', t.upper())
LW = [(li, w) for li, ln in enumerate(lines) for w in ln.split()]

# ---------------------------------------------------------------- transcript + fuzzy in-order match
TW = [w for w in json.load(open(A.transcript, encoding='utf8'))['words'] if norm(w['text'])]
a = [norm(w) for _, w in LW]; b = [norm(w['text']) for w in TW]
st = [None] * len(LW); en = [None] * len(LW)
sm = difflib.SequenceMatcher(None, a, b, autojunk=False)
for tag, i1, i2, j1, j2 in sm.get_opcodes():
    if tag == 'equal' or (tag == 'replace' and i2 - i1 == j2 - j1):
        for k in range(i2 - i1):
            if tag == 'equal' or difflib.SequenceMatcher(None, a[i1 + k], b[j1 + k]).ratio() >= 0.5:
                st[i1 + k] = TW[j1 + k]['start']; en[i1 + k] = TW[j1 + k]['end']
# fill gaps between matched words by letter count
k = 0
while k < len(LW):
    if st[k] is not None: k += 1; continue
    j = k
    while j < len(LW) and st[j] is None: j += 1
    t0 = en[k - 1] if k > 0 else (st[j] - 0.4 * (j - k) if j < len(LW) else 0.5)
    t1 = st[j] if j < len(LW) else t0 + 0.4 * (j - k)
    wts = [len(a[q]) + 1 for q in range(k, j)]; acc = t0
    for q, wv in zip(range(k, j), wts):
        d = (t1 - t0) * wv / sum(wts); st[q] = acc; en[q] = acc + d * 0.95; acc += d
    k = j

# ---------------------------------------------------------------- vocal stem: onsets and voiced
SR = 22050
x = np.frombuffer(subprocess.run([A.ffmpeg, '-v', 'error', '-i', A.vocals, '-ac', '1', '-ar', str(SR), '-f', 'f32le', '-'],
                                 stdout=subprocess.PIPE, check=True).stdout, dtype=np.float32)
h = SR // 100; n = len(x) // h
rms = np.sqrt((x[:n * h].reshape(n, h).astype(np.float64) ** 2).mean(1)); ld = 20 * np.log10(rms + 1e-7)
lo, hi = np.percentile(ld[ld > -80], 55), np.percentile(ld, 99.5)
lv = np.clip((ld - lo) / (hi - lo), 0, 1) ** 1.3
yv = np.zeros(n); at, rl = 1 - np.exp(-0.01 / 0.08), 1 - np.exp(-0.01 / 0.4)
for i in range(1, n): yv[i] = yv[i - 1] + (lv[i] - yv[i - 1]) * (at if lv[i] > yv[i - 1] else rl)
d2 = np.concatenate([[0, 0], np.maximum(ld[2:] - ld[:-2], 0)])
ons = sg.find_peaks(d2, height=4.0, distance=6)[0] * 0.01
voiced = yv > 0.1
def voiced_end(t0, lim):
    i, gap = int(t0 * 100), 0
    while i < n and i * 0.01 < lim:
        gap = 0 if voiced[i] else gap + 1
        if gap > 15: return (i - 15) * 0.01
        i += 1
    return lim

# ---------------------------------------------------------------- per line: snapped words, clip span
info = json.load(open(A.analysis)) ['info'] if A.analysis else {'drops': []}
drops = info.get('drops', [])
plan = []
idx = 0
for li, ln in enumerate(lines):
    ws = ln.split(); m = len(ws)
    s_ = st[idx:idx + m]; e_ = en[idx:idx + m]; idx += m
    snapped = []; prev = -1.0
    for s in s_:
        c = [o for o in ons if s - 0.18 <= o <= s + 0.3 and o > prev + 0.08]
        s2 = min(c, key=lambda o: abs(o - s)) if c else max(s, prev + 0.1)
        snapped.append(float(s2)); prev = s2
    plan.append({'text': ln, 'words': ws, 'starts': snapped, 'ends': [float(v) for v in e_]})
for k, p in enumerate(plan):
    nxt = plan[k + 1]['starts'][0] - 0.35 if k + 1 < len(plan) else 1e9
    last = p['starts'][-1]
    held = voiced_end(last + 0.08, min(nxt, last + 12.0))
    p['ends'][-1] = max(p['ends'][-1], held, last + 0.25)
    p['clip_start'] = round(max(p['starts'][0] - 0.35, plan[k - 1]['clip_end'] if k else 0.0), 3)
    p['clip_end'] = round(min(p['ends'][-1] + 0.3, nxt), 3)
    p['gold'] = 1 if drops and p['clip_start'] >= drops[0] - 0.5 else 0
# the line that ends right into the last drop becomes the giant line wrapped round a cylinder
giant = None
if len(drops) >= 2:
    for k, p in enumerate(plan):
        if drops[-1] - 3.5 <= p['clip_end'] <= drops[-1] + 0.6: giant = k
for k, p in enumerate(plan): p['giant'] = k == giant

# ---------------------------------------------------------------- fill keys from the font's real glyph widths
font = ImageFont.truetype(A.ttf, A.size)
GR = str.maketrans('AEOY', 'ΛΣΘΨ')
PAD = 6
for p in plan:
    shown = p['text'].translate(GR) if A.greek else p['text']
    p['shown'] = shown
    words = shown.split(); full = font.getlength(shown) + 2 * PAD
    keys = [(0.0, PAD / full)]; acc = ''
    cs = p['clip_start']
    for w, s, e, nx in zip(words, p['starts'], p['ends'], p['starts'][1:] + [None]):
        l = PAD + font.getlength(acc); r = l + font.getlength(w)
        e2 = min(e, nx) if nx else e
        keys.append((round(s - cs, 3), round(l / full, 4)))
        keys.append((round(max(e2 - cs - 0.01, s - cs + 0.005), 3), round(r / full, 4)))
        acc += w + ' '
    p['fill'] = keys

json.dump(plan, open(os.path.join(A.out, 'lyrics.json'), 'w', encoding='utf8'), ensure_ascii=False, indent=1)
for p in plan:
    print(f"{p['clip_start']:7.2f}-{p['clip_end']:7.2f} {'G' if p['gold'] else ' '}{'*' if p['giant'] else ' '} "
          + ' '.join(f"{w}@{s:.2f}" for w, s in zip(p['words'], p['starts'])))

# ---------------------------------------------------------------- the Luau script
src = open(A.shader, encoding='utf8').read()
def lua_str(s): return '"' + s.replace('\\', '\\\\').replace('"', '\\"') + '"'
L = ['-- generated by hsv_lyrics.py: lyric clips with the Lyric Chrome shader (one undo step)',
     'local TRACK = %d' % A.track, 'local VIS = %d -- visualizer clip id (0 = skip its lyric keys)' % A.vis,
     'local FONT, SIZE, Y = %s, %d, %g' % (lua_str(A.font), A.size, A.y),
     'local SRC = [==[' + src + ']==]', 'local LINES = {']
for p in plan:
    L.append('  {t=%s, s=%g, e=%g, gold=%d, giant=%s, fill={%s}},' % (
        lua_str(p['shown']), p['clip_start'], p['clip_end'], p['gold'], 'true' if p['giant'] else 'false',
        ','.join('{%g,%g}' % kv for kv in p['fill'])))
L.append('}')
L.append(r'''
local function key(id, prop, t, v, ease)
  editor.tool("clip.keyframe", {clip_id = id, property = prop, t = t, value = v, ease = ease or "Linear"})
end
local made = {}
for _, ln in ipairs(LINES) do
  local d = ln.e - ln.s
  local r = editor.tool("timeline.add_clip", {at = ln.s, duration = d, text = ln.t, track = TRACK})
  local id = r.clip_id or r.id or (r.clip_ids and r.clip_ids[1])
  assert(id, "no clip id for " .. ln.t)
  local giant = ln.giant
  editor.tool("clip.set", {clip_id = id, fields = {font = FONT, size = giant and math.floor(SIZE * 1.8) or SIZE,
    color = {255, 255, 255, 255}, outline_width = 0}})
  editor.tool("clip.set", {clip_id = id, fields = {y = giant and 0 or Y}})
  local rs = editor.tool("clip.set_shader", {clip = id, src = SRC})
  assert(rs.ok, rs.log)
  local P = "Lyric Chrome: "
  for _, k in ipairs(ln.fill) do key(id, P .. "fill", k[1], k[2]) end
  key(id, P .. "gold", 0, ln.gold, "Hold")
  local io = math.min(0.4, d * 0.22)
  -- the box has a margin inside (curves and the shadow never clip): scale up to match
  key(id, "height", 0, giant and 2.3 * 1.25 or 1.25)
  if giant then
    key(id, "width", 0, 1.08)
    key(id, P .. "curve", 0, 0.4)
    key(id, P .. "roll", 0, 1.0, "EaseOut"); key(id, P .. "roll", io, 0.025)
    key(id, P .. "roll", d - io, -0.025, "EaseIn"); key(id, P .. "roll", d, -1.0)
  else
    local g = math.min(70, 40 * d)
    -- pulled on from the right, slows through the middle, drifts off left; turns in 3D to face the centre
    key(id, "x", 0, 1500, "EaseOut"); key(id, "x", io, g); key(id, "x", d - io, -g, "EaseIn"); key(id, "x", d, -450)
    key(id, P .. "yaw", 0, -1, "EaseOut"); key(id, P .. "yaw", io, -g / 1500)
    key(id, P .. "yaw", d - io, g / 1500, "EaseIn"); key(id, P .. "yaw", d, 0.6)
    key(id, "width", 0, 1.5 * 1.08, "EaseOut"); key(id, "width", io, 1.08)
    key(id, "width", d - io, 1.08, "EaseIn"); key(id, "width", d, 1.5 * 1.08)
    key(id, P .. "smear", 0, 1, "EaseOut"); key(id, P .. "smear", io, 0)
    key(id, P .. "smear", d - io, 0, "EaseIn"); key(id, P .. "smear", d, 0.45)
  end
  key(id, "opacity", 0, 0, "EaseOut"); key(id, "opacity", io * 0.4, 1)
  key(id, "opacity", math.max(d - 0.3, d - io), 1, "EaseIn"); key(id, "opacity", d, 0)
  if VIS ~= 0 then
    key(VIS, "Olympus Storm: lyric", math.max(ln.s - 0.2, 0), 0); key(VIS, "Olympus Storm: lyric", ln.s + 0.2, 1)
    key(VIS, "Olympus Storm: lyric", ln.e - 0.2, 1); key(VIS, "Olympus Storm: lyric", ln.e + 0.2, 0)
  end
  made[#made + 1] = id
end
return made''')
open(os.path.join(A.out, 'lyrics.lua'), 'w', encoding='utf8').write('\n'.join(L) + '\n')
