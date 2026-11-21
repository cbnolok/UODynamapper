"""RIFE in-betweens of small sprites with the final warp sampled bilinear (as now), nearest, and nearest with each
pixel taken whole from one frame (mask > 0.5): the frames around them for reference, 4x nearest enlarged."""
import sys
sys.path.insert(0, '/home/claudio/rust-target/uodynamapper-scratch/rife')
sys.path.insert(0, '/home/claudio/rust-target/uodynamapper-scratch/anim-interp/py')
import numpy as np
import torch
import torch.nn.functional as F
from PIL import Image, ImageDraw
from clips import read_clips
from model.ifnet_v425 import IFNet

PAD, ZOOM, BG = 64, 4, (96, 110, 80)
D = '/home/claudio/rust-target/uodynamapper-scratch/ingame-anim'
net = IFNet()
st = torch.load('/home/claudio/rust-target/uodynamapper-scratch/rife/models/v4.25/train_log/flownet.pkl', map_location='cpu', weights_only=True)
net.load_state_dict({k.replace('module.', ''): v for k, v in st.items()}, False)
net.eval(); torch.set_grad_enabled(False)


def warp(img, flow, mode):
    _, _, h, w = flow.shape
    hor = torch.linspace(-1, 1, w).view(1, 1, 1, w).expand(1, -1, h, -1)
    ver = torch.linspace(-1, 1, h).view(1, 1, h, 1).expand(1, -1, -1, w)
    g = torch.cat([hor + flow[:, 0:1] / ((w - 1) / 2), ver + flow[:, 1:2] / ((h - 1) / 2)], 1).permute(0, 2, 3, 1)
    return F.grid_sample(img, g, mode=mode, padding_mode='border', align_corners=True)


def tensor(f, ph, pw):
    f = f.astype(np.float32) / 255; a = f[..., 3:]
    x = np.concatenate([f[..., :3] * a + 0.5 * (1 - a), f[..., :3] * a, a], -1)
    return F.pad(torch.from_numpy(np.ascontiguousarray(x)).permute(2, 0, 1)[None], (0, pw - f.shape[1], 0, ph - f.shape[0]))


def straight(p, h, w):
    p = p[0, 3:, :h, :w].permute(1, 2, 0).numpy()
    a = np.clip(p[..., 3:], 0, 1)
    rgb = np.where(a > 1e-3, p[..., :3] / np.maximum(a, 1e-3), 0)
    return (np.clip(np.concatenate([rgb, a], -1), 0, 1) * 255).round().astype(np.uint8)


def on_bg(rgba):
    im = Image.new('RGBA', (rgba.shape[1], rgba.shape[0]), BG + (255,))
    im.alpha_composite(Image.fromarray(rgba, 'RGBA'))
    return im.convert('RGB').resize((rgba.shape[1] * ZOOM, rgba.shape[0] * ZOOM), Image.NEAREST)


rows = []
for src, body, pair in [('cc', 217, 1), ('ec', 217, 2), ('ec', 238, 1), ('cc', 200, 2)]:
    fr = [c for c in read_clips(f'{D}/{src}/{src}_{body:05}.bin.zst') if c['cls'] == 1][0]['frames']
    fr = np.ascontiguousarray(fr[:, :, ::-1])
    n, h, w, _ = fr.shape
    a_, b_ = fr[pair], fr[(pair + 1) % n]
    ys, xs = np.nonzero(np.maximum(a_[..., 3], b_[..., 3]))
    y0, y1, x0, x1 = max(ys.min() - 2, 0), ys.max() + 3, max(xs.min() - 2, 0), xs.max() + 3
    ph, pw = (h + PAD - 1) // PAD * PAD, (w + PAD - 1) // PAD * PAD
    i0, i1 = tensor(a_, ph, pw), tensor(b_, ph, pw)
    flows, mask_list, merged = net(torch.cat([i0, i1], 1), 0.5, [16, 8, 4, 2, 1])
    flow, m = flows[-1], torch.sigmoid(mask_list[-1])
    cells = [('frame A', a_)]
    for title, mode, hard in [('bilinear (now)', 'bilinear', False), ('nearest', 'nearest', False),
                              ('nearest, whole pixels', 'nearest', True)]:
        w0, w1 = warp(i0, flow[:, :2], mode), warp(i1, flow[:, 2:4], mode)
        mm = (m > 0.5).float() if hard else m
        cells.append((title, straight(w0 * mm + w1 * (1 - mm), h, w)))
    cells.append(('frame B', b_))
    rows.append((f'{src} {body}', [(t, on_bg(np.ascontiguousarray(c[y0:y1, x0:x1]))) for t, c in cells]))
LABEL, GAP = 16, 6
W = max(sum(im.width for _, im in cells) + GAP * len(cells) for _, cells in rows)
H = sum(cells[0][1].height + LABEL for _, cells in rows) + GAP * len(rows)
sheet = Image.new('RGB', (W, H), (24, 24, 24)); d = ImageDraw.Draw(sheet)
y = 0
for name, cells in rows:
    x = 0
    for title, im in cells:
        d.text((x + 2, y + 2), f'{name}: {title}', fill=(255, 255, 220)); sheet.paste(im, (x, y + LABEL)); x += im.width + GAP
    y += cells[0][1].height + LABEL + GAP
sheet.save('/home/claudio/rust-target/uodynamapper-scratch/stride/crisp.png'); print(sheet.size)
