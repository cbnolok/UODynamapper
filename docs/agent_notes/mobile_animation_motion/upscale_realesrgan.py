"""Enhanced walk frames upscaled 4x: nearest, bicubic, Real-ESRGAN realesr-animevideov3 and realesr-general-x4v3
(SRVGGNetCompact, BSD-3). Colour goes through the network premultiplied over 50% grey; alpha goes through it as a grey
image. Timing per frame on CPU, 8 threads."""
import sys, time
sys.path.insert(0, '/home/claudio/rust-target/uodynamapper-scratch/anim-interp/py')
import numpy as np, torch, torch.nn as nn, torch.nn.functional as F
from PIL import Image, ImageDraw
from clips import read_clips

class Compact(nn.Module):
    def __init__(self, num_conv, feat=64, up=4):
        super().__init__()
        self.up = up
        body = [nn.Conv2d(3, feat, 3, 1, 1), nn.PReLU(feat)]
        for _ in range(num_conv):
            body += [nn.Conv2d(feat, feat, 3, 1, 1), nn.PReLU(feat)]
        body += [nn.Conv2d(feat, 3 * up * up, 3, 1, 1)]
        self.body = nn.ModuleList(body)
        self.shuf = nn.PixelShuffle(up)
    def forward(self, x):
        y = x
        for m in self.body: y = m(y)
        return self.shuf(y) + F.interpolate(x, scale_factor=self.up, mode='nearest')

def load(path, n):
    m = Compact(n); sd = torch.load(path, map_location='cpu', weights_only=True)
    sd = sd.get('params', sd.get('params_ema', sd)); m.load_state_dict(sd); m.eval(); return m

torch.set_grad_enabled(False); torch.set_num_threads(8)
D = '/home/claudio/rust-target/uodynamapper-scratch/upscale/dl'
models = [('animevideov3', load(f'{D}/realesr-animevideov3.pth', 16)), ('general-x4v3', load(f'{D}/realesr-general-x4v3.pth', 32))]
print('params', [sum(p.numel() for p in m.parameters()) for _, m in models])
Z, BG = 4, (96, 110, 80)

def run(m, rgba):
    f = rgba.astype(np.float32) / 255; a = f[..., 3:]
    col = torch.from_numpy(np.ascontiguousarray(f[..., :3] * a + 0.5 * (1 - a))).permute(2, 0, 1)[None]
    al = torch.from_numpy(np.ascontiguousarray(np.repeat(a, 3, -1))).permute(2, 0, 1)[None]
    t = time.perf_counter(); c = m(col).clamp(0, 1); al4 = m(al).clamp(0, 1).mean(1, keepdim=True); dt = time.perf_counter() - t
    c = c[0].permute(1, 2, 0).numpy(); A = al4[0].permute(1, 2, 0).numpy()
    rgb = np.where(A > 1e-3, (c - 0.5 * (1 - A)) / np.maximum(A, 1e-3), 0)   # undo the grey matte
    return (np.clip(np.concatenate([rgb, A], -1), 0, 1) * 255).round().astype(np.uint8), dt

def comp(rgba):
    im = Image.new('RGBA', (rgba.shape[1], rgba.shape[0]), BG + (255,)); im.alpha_composite(Image.fromarray(rgba, 'RGBA')); return im.convert('RGB')

rows = []
for body, crop in [(200, None), (46, None), (217, None), (826, (0.25, 0.45, 0.75, 0.95))]:
    fr = [c for c in read_clips(f'/home/claudio/rust-target/uodynamapper-scratch/ingame-anim/ec/ec_{body:05}.bin.zst') if c['cls'] == 1][0]['frames'][2]
    ys, xs = np.nonzero(fr[..., 3]); fr = fr[ys.min():ys.max() + 1, xs.min():xs.max() + 1]
    if crop:
        h, w = fr.shape[:2]; fr = fr[int(crop[1] * h):int(crop[3] * h), int(crop[0] * w):int(crop[2] * w)]
    fr = np.ascontiguousarray(np.pad(fr, ((2, 2), (2, 2), (0, 0))))
    h, w = fr.shape[:2]
    pil = Image.fromarray(fr, 'RGBA')
    cells = [('nearest', comp(np.asarray(pil.resize((w * Z, h * Z), Image.NEAREST)))),
             ('bicubic', comp(np.asarray(pil.resize((w * Z, h * Z), Image.BICUBIC))))]
    for name, m in models:
        out, dt = run(m, fr); cells.append((f'{name} {dt * 1e3:.0f} ms', comp(out)))
    rows.append((f'ec {body} ({w}x{h})', cells))
LABEL, GAP = 16, 6
W = max(sum(c.width + GAP for _, c in cells) for _, cells in rows); H = sum(cells[0][1].height + LABEL + GAP for _, cells in rows)
sheet = Image.new('RGB', (W, H), (24, 24, 24)); d = ImageDraw.Draw(sheet); y = 0
for name, cells in rows:
    x = 0
    for t, c in cells:
        d.text((x + 2, y + 2), f'{name}: {t}', fill=(255, 255, 220)); sheet.paste(c, (x, y + LABEL)); x += c.width + GAP
    y += cells[0][1].height + LABEL + GAP
sheet.save('/home/claudio/rust-target/uodynamapper-scratch/upscale/out/sr_ec.png'); print(sheet.size)
