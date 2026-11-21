# Hardstyle visualizer: every analysis step in one go, then the generated scripts installed for scripts.run.
#
#   python hsv_build.py SONG STEMS_DIR WORDS.json OUT_DIR --vis VIS_CLIP --bg BG_CLIP [--track N] [--no-greek]
#
# OUT_DIR must contain lyrics.txt (the user's lyrics, exactly as given). Installs hsv-keys, hsv-lyrics, hsv-bg
# and hsv-shaders into the app's scripts folder: run them in that order with scripts.run.
import argparse, json, os, subprocess, sys
ap = argparse.ArgumentParser()
ap.add_argument('song'); ap.add_argument('stems'); ap.add_argument('words'); ap.add_argument('out')
ap.add_argument('--vis', type=int, required=True); ap.add_argument('--bg', type=int, default=0)
ap.add_argument('--track', type=int, default=3); ap.add_argument('--no-greek', action='store_true')
ap.add_argument('--drops', default=''); ap.add_argument('--calm', default='')
A = ap.parse_args()
H = os.path.dirname(os.path.abspath(__file__)); O = A.out
py = lambda *a: subprocess.run([sys.executable, os.path.join(H, a[0]), *a[1:]], check=True,
                               env=dict(os.environ, PYTHONIOENCODING='utf8'))
extra = (['--drops', A.drops] if A.drops else []) + (['--calm', A.calm] if A.calm else [])
lyr = ['hsv_lyrics.py', os.path.join(O, 'lyrics.txt'), A.words, os.path.join(A.stems, 'vocals.wav'), O,
       '--analysis', os.path.join(O, 'analysis.json'), '--track', str(A.track), '--vis', str(A.vis)] + ([] if A.no_greek else ['--greek'])
py('hsv_analyze.py', A.song, A.stems, O, *extra)
py(*lyr)
after = json.load(open(os.path.join(O, 'structure.json'))).get('drop_after')
if after and not A.drops:                       # the lyrics say where the drop is: re-analyse with that
    py('hsv_analyze.py', A.song, A.stems, O, '--after', str(after), *extra)
    py(*lyr)
py('hsv_shaders.py', os.path.join(O, 'analysis.json'), O)
order = []
def inst(src, name, *sets): py('hsv_install.py', os.path.join(O, src), name, *sets); order.append(name)
n = lambda pre: sorted((f for f in os.listdir(O) if f.startswith(pre + '.') and f.endswith('.lua')), key=lambda f: int(f.split('.')[1]))
for f in n('keys'): inst(f, 'hsv-keys-' + f.split('.')[1], 'CLIP=%d' % A.vis)
inst('lyrics.lua', 'hsv-lyrics')
if A.bg: inst('background.lua', 'hsv-bg', 'BG=%d' % A.bg)
for f in n('shaders'): inst(f, 'hsv-shaders-' + f.split('.')[1], 'VIS=%d' % A.vis)
print('RUN IN ORDER:', ' '.join(order))
print(json.dumps(json.load(open(os.path.join(O, 'analysis.json')))['info']))
