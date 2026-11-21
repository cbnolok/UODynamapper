"""Walk pacing candidates side by side, RIFE x4 + 50% fade, camera following a walk east at 0.4 s a step: each column
plays the same clip over a different cycle length.
usage: pace.py <src> <file body> <rife npz> <ground png> <out mp4> <title=cycle_secs> ..."""
import subprocess, sys
sys.path.insert(0, '/home/claudio/rust-target/uodynamapper-scratch/anim-interp/py')
import numpy as np
from PIL import Image, ImageDraw
from clips import read_clips

STEP_SECS, STEP_PX, FPS, SPAN = 0.4, 22, 50, 0.5
MARGIN, LABEL_H, SMALL_PX, MIN_SECS = 24, 18, 700, 3.2
src, body, npz, ground_path, out = sys.argv[1:6]
cols = [(a.split('=')[0], float(a.split('=')[1])) for a in sys.argv[6:]]
x4 = np.load(npz)['frames']
alpha = x4[..., 3].max(0)
ys, xs = np.nonzero(alpha)
x0, y0, x1, y1 = xs.min(), ys.min(), xs.max() + 1, ys.max() + 1
pw, ph = (x1 - x0) + 2 * MARGIN, (y1 - y0) + 2 * MARGIN


def weight(t):
    x = min(max((t - (1 - SPAN) / 2) / SPAN, 0.0), 1.0)
    return x * x * (3 - 2 * x)


def premul(f):
    f = f.astype(np.float32)
    return np.concatenate([f[..., :3] * f[..., 3:] / 255, f[..., 3:]], -1)


def fade(a, b, t):
    p = (1 - t) * premul(a) + t * premul(b)
    al = p[..., 3:]
    rgb = np.where(al > 0, p[..., :3] * 255 / np.maximum(al, 1e-6), 0)
    return np.clip(np.rint(np.concatenate([rgb, al], -1)), 0, 255).astype(np.uint8)


ground = Image.open(ground_path).convert('RGB')
secs = max(2 * max(c for _, c in cols), MIN_SECS)
travel = int(secs / STEP_SECS * STEP_PX) + 2
gx0 = min(ground.width - pw - 4, 900 - pw // 2 + travel)
gy0 = min(ground.height - ph - 4, 560 - ph // 2 + travel)
m = len(x4)
ff = None
for k in range(int(secs * FPS)):
    now = k / FPS
    shift = now / STEP_SECS * STEP_PX
    left, top = int(round(gx0 - travel + shift)), int(round(gy0 - travel + shift))
    bg = ground.crop((left, top, left + pw, top + ph))
    canvas = Image.new('RGB', (len(cols) * (pw + 4), ph + LABEL_H), (20, 20, 20))
    d = ImageDraw.Draw(canvas)
    for j, (title, cycle) in enumerate(cols):
        pos = now / (cycle / m)
        i, t = int(pos) % m, pos - int(pos)
        wt = weight(t)
        spr = x4[i] if wt == 0 else fade(x4[i], x4[(i + 1) % m], wt)
        panel = bg.convert('RGBA')
        panel.alpha_composite(Image.fromarray(np.ascontiguousarray(spr[y0:y1, x0:x1]), 'RGBA'), (MARGIN, MARGIN))
        canvas.paste(panel.convert('RGB'), (j * (pw + 4), LABEL_H))
        d.text((j * (pw + 4) + 4, 3), f'{title} {cycle:.2f}s', fill=(255, 255, 230))
    if canvas.width < SMALL_PX:
        canvas = canvas.resize((canvas.width * 2, canvas.height * 2), Image.NEAREST)
    if ff is None:
        w, h = canvas.size
        ff = subprocess.Popen(['ffmpeg', '-loglevel', 'error', '-y', '-f', 'rawvideo', '-pix_fmt', 'rgb24', '-s', f'{w}x{h}',
                               '-r', str(FPS), '-i', '-', '-vf', 'pad=ceil(iw/2)*2:ceil(ih/2)*2', '-c:v', 'libx264',
                               '-crf', '14', '-preset', 'slow', '-pix_fmt', 'yuv420p', '-movflags', '+faststart', out],
                              stdin=subprocess.PIPE)
    ff.stdin.write(canvas.tobytes())
ff.stdin.close(); ff.wait(); print(out)
