# Hardstyle visualizer: a low-frame-rate GIF / clip / still -> a smooth, seamless 1920x1080 60 fps ambient loop.
#
#   python hsv_background.py INPUT OUT.mp4 [--seconds 8] [--ffmpeg ffmpeg]
#
# Motion-interpolates to 60 fps (minterpolate, mci), cover-crops to 1920x1080, slows to --seconds, then plays
# it forward and back (ping-pong) so the loop point never jumps. Import the result and loop it under the song
# (or set the clip's loop / duplicate it), then put the LUT and grade on it in the app.
# A still image: it gets a slow 6% push-in instead (zoompan), which loops by ping-pong the same way.
import argparse, os, subprocess, tempfile
ap = argparse.ArgumentParser()
ap.add_argument('input'); ap.add_argument('out')
ap.add_argument('--seconds', type=float, default=8.0, help='length of the one-way pass (loop = 2x)')
ap.add_argument('--ffmpeg', default='ffmpeg')
A = ap.parse_args()
ff = lambda *a: subprocess.run([A.ffmpeg, '-v', 'error', '-y', *a], check=True)
probe = subprocess.run([A.ffmpeg.replace('ffmpeg', 'ffprobe'), '-v', 'error', '-select_streams', 'v:0', '-count_frames',
                        '-show_entries', 'stream=nb_read_frames,r_frame_rate', '-of', 'csv=p=0', A.input],
                       stdout=subprocess.PIPE, text=True).stdout.strip().split(',')
frames = int(probe[-1]) if probe and probe[-1].isdigit() else 1
cover = 'scale=1920:1080:force_original_aspect_ratio=increase,crop=1920:1080'
tmp = tempfile.mkdtemp()
one = os.path.join(tmp, 'one.mp4')
if frames <= 1:
    n = int(A.seconds * 60)
    ff('-loop', '1', '-i', A.input, '-vf', f"scale=3840:-2,zoompan=z='1+0.06*on/{n}':x='iw/2-iw/zoom/2':y='ih/2-ih/zoom/2':d={n}:s=1920x1080:fps=60",
       '-frames:v', str(n), '-pix_fmt', 'yuv420p', '-c:v', 'libx264', '-crf', '18', one)
else:
    # stretch the source to --seconds, then invent the in-between frames
    ff('-i', A.input, '-vf', f"{cover},setpts=PTS*{A.seconds}/({frames}/FRAME_RATE),minterpolate=fps=60:mi_mode=mci:mc_mode=aobmc:vsbmc=1,format=yuv420p",
       '-t', str(A.seconds), '-an', '-c:v', 'libx264', '-crf', '18', one)
ff('-i', one, '-filter_complex', '[0:v]split[a][b];[b]reverse[r];[a][r]concat=n=2:v=1[v]', '-map', '[v]',
   '-c:v', 'libx264', '-crf', '21', '-pix_fmt', 'yuv420p', '-movflags', '+faststart', A.out)
print(A.out)
