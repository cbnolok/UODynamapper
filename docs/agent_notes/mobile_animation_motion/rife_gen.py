"""RIFE v4.25 in-betweens of the comparison clips at t = 1/4, 1/2, 3/4, saved per clip as the frames x4 (straight
RGBA). The network sees the frames over grey (for its flow) and warps, with the same flow, the premultiplied colour
and the alpha, so the in-betweens keep a clean alpha. CPU only; weights loaded with weights_only=True.
usage: gen.py <src cc|ec> <file body> <out npz>"""
import sys
import time

sys.path.insert(0, '/home/claudio/rust-target/uodynamapper-scratch/rife')
sys.path.insert(0, '/home/claudio/rust-target/uodynamapper-scratch/anim-interp/py')
import numpy as np
import torch
import torch.nn.functional as F
from clips import read_clips
from model.ifnet_v425 import IFNet

PAD, GREY, TS = 64, 0.5, (0.25, 0.5, 0.75)
WEIGHTS = '/home/claudio/rust-target/uodynamapper-scratch/rife/models/v4.25/train_log/flownet.pkl'
D = '/home/claudio/rust-target/uodynamapper-scratch/ingame-anim'
src, body, out = sys.argv[1], int(sys.argv[2]), sys.argv[3]
clip = [c for c in read_clips(f'{D}/{src}/{src}_{body:05}.bin.zst') if c['cls'] == 1][0]
frames = np.ascontiguousarray(clip['frames'][:, :, ::-1])   # facing east is the mirrored stored direction
n, h, w, _ = frames.shape
net = IFNet()
state = torch.load(WEIGHTS, map_location='cpu', weights_only=True)
net.load_state_dict({k.replace('module.', ''): v for k, v in state.items()}, False)
net.eval()
torch.set_grad_enabled(False)


def tensor(f):
    f = f.astype(np.float32) / 255
    a = f[..., 3:]
    grey = f[..., :3] * a + GREY * (1 - a)
    x = np.concatenate([grey, f[..., :3] * a, a], -1)       # 7 channels: flow input, premultiplied colour, alpha
    t = torch.from_numpy(np.ascontiguousarray(x)).permute(2, 0, 1)[None]
    ph, pw = (h + PAD - 1) // PAD * PAD, (w + PAD - 1) // PAD * PAD
    return F.pad(t, (0, pw - w, 0, ph - h))


seq = []
start = time.time()
for i in range(n):
    a, b = tensor(frames[i]), tensor(frames[(i + 1) % n])
    seq.append(frames[i])
    for t in TS:
        _, _, merged = net(torch.cat([a, b], 1), t, [16, 8, 4, 2, 1])
        m = merged[-1][0, :, :h, :w].permute(1, 2, 0).numpy()
        alpha = np.clip(m[..., 6:7], 0, 1)
        col = np.where(alpha > 1e-3, m[..., 3:6] / np.maximum(alpha, 1e-3), 0)
        seq.append((np.clip(np.concatenate([col, alpha], -1), 0, 1) * 255).round().astype(np.uint8))
np.savez_compressed(out, frames=np.stack(seq), fps=clip['fps'])
print(src, body, n, 'pairs', f'{(time.time() - start) / n / len(TS) * 1e3:.0f} ms per in-between', (w, h))
