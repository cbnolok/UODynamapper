"""Walk/run stride of a clip from RIFE v4.25 flow: per frame pair, the motion of the silhouette's lowest band along
the facing's screen direction at t = 0.5 (flow to frame 1 minus flow to frame 0); planted feet move backward at the
body's speed, so a low percentile of that motion is the body's travel. Summed over the cycle and divided by the
screen pixels of one tile in that facing, it is the stride in tiles per cycle; the five stored facings should agree.
usage: measure.py <src> <class 1 walk | 2 run> <body,...>"""
import sys
sys.path.insert(0, '/home/claudio/rust-target/uodynamapper-scratch/rife')
sys.path.insert(0, '/home/claudio/rust-target/uodynamapper-scratch/anim-interp/py')
import numpy as np
import torch
import torch.nn.functional as F
from clips import read_clips
from model.ifnet_v425 import IFNet

S = '/home/claudio/rust-target/uodynamapper-scratch/stride'
PAD, BAND, PCT, ALPHA = 64, 0.2, 10, 0.5
R2 = np.sqrt(0.5)
# Server direction -> (unit screen direction of travel, screen px per tile); 3..7 are the stored, unmirrored ones.
FACINGS = {3: ((0.0, 1.0), 44.0), 4: ((-R2, R2), 44 * R2), 5: ((-1.0, 0.0), 44.0), 6: ((-R2, -R2), 44 * R2),
           7: ((0.0, -1.0), 44.0)}
net = IFNet()
st = torch.load('/home/claudio/rust-target/uodynamapper-scratch/rife/models/v4.25/train_log/flownet.pkl', map_location='cpu', weights_only=True)
net.load_state_dict({k.replace('module.', ''): v for k, v in st.items()}, False)
net.eval(); torch.set_grad_enabled(False); torch.set_num_threads(8)


def tensor(f, ph, pw):
    f = f.astype(np.float32) / 255; a = f[..., 3:]
    x = np.concatenate([f[..., :3] * a + 0.5 * (1 - a), f[..., :3] * a, a], -1)
    t = torch.from_numpy(np.ascontiguousarray(x)).permute(2, 0, 1)[None]
    return F.pad(t, (0, pw - f.shape[1], 0, ph - f.shape[0]))


src, cls, bodies = sys.argv[1], int(sys.argv[2]), [int(b) for b in sys.argv[3].split(',')]
for body in bodies:
    row = []
    for d, (u, px_per_tile) in FACINGS.items():
        try:
            clip = [c for c in read_clips(f'{S}/{src}/d{d}/{src}_{body:05}.bin.zst') if c['cls'] == cls][0]
        except (IndexError, FileNotFoundError):
            row.append(None); continue
        fr = clip['frames']; n, h, w, _ = fr.shape
        ph, pw = (h + PAD - 1) // PAD * PAD, (w + PAD - 1) // PAD * PAD
        travel = 0.0
        for i in range(n):
            x = torch.cat([tensor(fr[i], ph, pw), tensor(fr[(i + 1) % n], ph, pw)], 1)
            flows, _, merged = net(x, 0.5, [16, 8, 4, 2, 1])
            fl = flows[-1][0].numpy()
            disp = fl[2:4] - fl[0:2]                       # frame 0 -> frame 1 motion of each mid-frame pixel
            alpha = merged[-1][0, 6].numpy()
            ys, xs = np.nonzero(alpha > ALPHA)
            top, bottom = ys.min(), ys.max()
            low = ys >= bottom - BAND * (bottom - top)
            along = disp[0, ys[low], xs[low]] * u[0] + disp[1, ys[low], xs[low]] * u[1]
            travel -= np.percentile(along, PCT)
        row.append((travel / px_per_tile, n, clip['fps']))
    ok = [r for r in row if r]
    strides = np.array([r[0] for r in ok])
    n, fps = ok[0][1], ok[0][2]
    print(f'{src} {"walk" if cls == 1 else "run "} {body:4d} {n:2d} fr @{fps:5.2f}: tiles/cycle per facing '
          + ' '.join('  -  ' if r is None else f'{r[0]:5.2f}' for r in row)
          + f' | median {np.median(strides):.2f}, spread {(strides.max() - strides.min()) / np.median(strides):.0%}',
          flush=True)
