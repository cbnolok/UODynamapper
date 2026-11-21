"""Classic horse (body 200, walk, east) pair 0 -> 1 in-betweens at t = 1/4, 1/2, 3/4, front legs 6x: RIFE as now;
with the last block at 2x resolution (scale list halved); on 2x (nearest) upscaled frames then downsampled; and with
each pixel taken whole from one frame (mask > 0.5) plus nearest warp."""
import sys
sys.path.insert(0, '/home/claudio/rust-target/uodynamapper-scratch/rife')
sys.path.insert(0, '/home/claudio/rust-target/uodynamapper-scratch/anim-interp/py')
import numpy as np, torch, torch.nn.functional as F
from PIL import Image, ImageDraw
from clips import read_clips
from model.ifnet_v425 import IFNet

net = IFNet()
st = torch.load('/home/claudio/rust-target/uodynamapper-scratch/rife/models/v4.25/train_log/flownet.pkl', map_location='cpu', weights_only=True)
net.load_state_dict({k.replace('module.', ''): v for k, v in st.items()}, False); net.eval(); torch.set_grad_enabled(False)
fr = [c for c in read_clips('/home/claudio/rust-target/uodynamapper-scratch/ingame-anim/cc/cc_00200.bin.zst') if c['cls'] == 1][0]['frames']
fr = np.ascontiguousarray(fr[:, :, ::-1]); n, h, w, _ = fr.shape


def tensor(f, up=1):
    f = f.astype(np.float32) / 255; a = f[..., 3:]
    x = torch.from_numpy(np.ascontiguousarray(np.concatenate([f[..., :3] * a + 0.5 * (1 - a), f[..., :3] * a, a], -1))).permute(2, 0, 1)[None]
    if up > 1: x = F.interpolate(x, scale_factor=up, mode='nearest')
    H, W = x.shape[2:]; ph, pw = (H + 63) // 64 * 64, (W + 63) // 64 * 64
    return F.pad(x, (0, pw - W, 0, ph - H))


def warp(img, flow, mode):
    _, _, hh, ww = flow.shape
    g = torch.cat([torch.linspace(-1, 1, ww).view(1, 1, 1, ww).expand(1, -1, hh, -1) + flow[:, 0:1] / ((ww - 1) / 2),
                   torch.linspace(-1, 1, hh).view(1, 1, hh, 1).expand(1, -1, -1, ww) + flow[:, 1:2] / ((hh - 1) / 2)], 1).permute(0, 2, 3, 1)
    return F.grid_sample(img, g, mode=mode, padding_mode='border', align_corners=True)


def run(a, b, t, scales, up=1, hard=False):
    i0, i1 = tensor(a, up), tensor(b, up)
    flows, masks, merged = net(torch.cat([i0, i1], 1), t, scales)
    if hard:
        fl, m = flows[-1], torch.sigmoid(masks[-1])
        mm = (m > 0.5).float()
        out = warp(i0, fl[:, :2], 'nearest') * mm + warp(i1, fl[:, 2:4], 'nearest') * (1 - mm)
    else:
        out = merged[-1]
    if up > 1: out = F.avg_pool2d(out, up)
    p = out[0, 3:, :h, :w].permute(1, 2, 0).numpy(); al = np.clip(p[..., 3:], 0, 1)
    rgb = np.where(al > 1e-3, p[..., :3] / np.maximum(al, 1e-3), 0)
    return (np.clip(np.concatenate([rgb, al], -1), 0, 1) * 255).round().astype(np.uint8)


S1 = [16, 8, 4, 2, 1]; S2 = [8, 4, 2, 1, 0.5]
variants = [('now', dict(scales=S1)), ('finer scales', dict(scales=S2)), ('2x input', dict(scales=S1, up=2)),
            ('2x input, finer', dict(scales=S2, up=2)), ('whole px + nearest', dict(scales=S1, hard=True))]
a_all = fr[..., 3].max(0); ys, xs = np.nonzero(a_all)
y0, y1, x0, x1 = ys.min() + (ys.max() - ys.min()) // 2, ys.max() + 2, xs.min() + (xs.max() - xs.min()) * 2 // 5, xs.max() + 2
Z = 5
def cell(img, label):
    c = img[y0:y1, x0:x1]; im = Image.new('RGBA', (c.shape[1], c.shape[0]), (96, 110, 80, 255))
    im.alpha_composite(Image.fromarray(np.ascontiguousarray(c), 'RGBA'))
    im = im.convert('RGB').resize((c.shape[1] * Z, c.shape[0] * Z), Image.NEAREST); ImageDraw.Draw(im).text((3, 3), label, fill=(255, 255, 0)); return im
rows = []
for pair in (0, 4):
    a, b = fr[pair], fr[(pair + 1) % n]
    for name, kw in variants:
        rows.append([cell(a, f'{name}: A')] + [cell(run(a, b, t, **kw), f't={t}') for t in (0.25, 0.5, 0.75)] + [cell(b, 'B')])
cw, ch = rows[0][0].size
sheet = Image.new('RGB', (5 * (cw + 4), len(rows) * (ch + 4)), (20, 20, 20))
for r, row in enumerate(rows):
    for c, im in enumerate(row): sheet.paste(im, (c * (cw + 4), r * (ch + 4)))
sheet.save('/home/claudio/rust-target/uodynamapper-scratch/stride/out/legfix.png'); print(sheet.size)
