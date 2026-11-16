"""Draw tools/visualizer-sample/dance-loop_4x2.png: a placeholder dance loop as a 4x2 sprite sheet (8 poses,
256x256 cells, transparent background) of a blocky avatar, with ffmpeg's drawbox (no image libraries).
Swap in your own loop (a recorded dance as a video, an image sequence or a sprite sheet) with
right-click an audio clip > Audio > Add Beat-Synced Clip... or `audio.beat_sprite`.

    python tools/visualizer-sample/make-dance-loop.py
"""
import os
import subprocess

CELL = 256
COLS, ROWS = 4, 2
SKIN, SHIRT, PANTS = "0xF2C14E", "0x2E7DF6", "0x2FA35A"


def box(x, y, w, h, c):
    return f"drawbox=x={x}:y={y}:w={w}:h={h}:color={c}@1:t=fill"


def avatar(ox, oy, arm_l, arm_r, legs, lift):
    """Boxes for one pose. arm_*: 'down' | 'out' | 'up'; legs: 'together' | 'apart'; lift = jump px."""
    cx, top = ox + CELL // 2, oy + 28 - lift
    out = [box(cx - 28, top, 56, 56, SKIN)]  # head
    ty = top + 60
    out.append(box(cx - 32, ty, 64, 76, SHIRT))  # torso
    for side, pose in ((-1, arm_l), (1, arm_r)):
        sx = cx + side * 32 - (24 if side < 0 else 0)
        if pose == "down":
            out.append(box(sx, ty, 24, 70, SKIN))
        elif pose == "out":
            out.append(box(sx + (-46 if side < 0 else 0), ty, 70, 22, SKIN))
        else:  # up
            out.append(box(sx, ty - 62, 24, 70, SKIN))
    ly = ty + 76
    spread = 14 if legs == "apart" else 0
    out.append(box(cx - 30 - spread, ly, 28, 72, PANTS))
    out.append(box(cx + 2 + spread, ly, 28, 72, PANTS))
    return out


POSES = [
    ("down", "down", "together", 0),
    ("up", "down", "apart", 0),
    ("out", "out", "apart", 10),
    ("down", "up", "apart", 0),
    ("up", "up", "together", 22),
    ("out", "down", "apart", 0),
    ("down", "out", "apart", 0),
    ("up", "out", "together", 8),
]

boxes = []
for i, (al, ar, lg, lift) in enumerate(POSES):
    boxes += avatar((i % COLS) * CELL, (i // COLS) * CELL, al, ar, lg, lift)

here = os.path.dirname(os.path.abspath(__file__))
out = os.path.join(here, "dance-loop_4x2.png")
# drawbox leaves alpha alone: draw on a key colour, key it out, and zero the colour under the
# transparency (premultiply) so nothing magenta leaks when a tool drops the alpha
graph = f"color=c=0xFF00FF:s={COLS * CELL}x{ROWS * CELL},format=rgb24," + ",".join(boxes) + ",colorkey=0xFF00FF:0.01:0,format=rgba,premultiply=inplace=1"
subprocess.run(["ffmpeg", "-v", "error", "-y", "-f", "lavfi", "-i", graph, "-frames:v", "1", out], check=True)
print(out)
