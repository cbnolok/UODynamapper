"""Enhanced walk frames at 4x: nearest, bicubic, and three SPAN models (~2.2 M parameters): 4xpix_span_pretrain (L1 pixel
loss, faithful), 4xmssim_span_pretrain (MS-SSIM loss) and 4xNomosUni_span_multijpg (perceptual/GAN, general).
Colour premultiplied over 50% grey through the network, the grey matte removed with alpha upscaled bicubic."""
import sys, time
sys.path.insert(0, '/home/claudio/rust-target/uodynamapper-scratch/anim-interp/py')
sys.path.insert(0, '/home/claudio/rust-target/uodynamapper-scratch/upscale/arch')
import numpy as np, torch
from PIL import Image, ImageDraw
from safetensors.torch import load_file
from clips import read_clips
from span import SPAN

torch.set_grad_enabled(False); torch.set_num_threads(8)
D = '/home/claudio/rust-target/uodynamapper-scratch/upscale/dl'
models = []
for name in ('4xpix_span_pretrain', '4xmssim_span_pretrain', '4xNomosUni_span_multijpg'):
    sd = load_file(f'{D}/{name}.safetensors')
    m = SPAN(num_in_ch=3, num_out_ch=3, feature_channels=48, upscale=4, norm='no_norm' not in sd)
    missing, unexpected = m.load_state_dict(sd, strict=False)
    assert not [k for k in missing if 'eval_conv' not in k] and not unexpected, (missing, unexpected)
    m.eval(); models.append((name.replace('_pretrain', '').replace('4x', ''), m))
Z, BG = 4, (96, 110, 80)


def run(m, rgba):
    f = rgba.astype(np.float32) / 255; a = f[..., 3:]
    col = torch.from_numpy(np.ascontiguousarray(f[..., :3] * a + 0.5 * (1 - a))).permute(2, 0, 1)[None]
    t = time.perf_counter(); c = m(col).clamp(0, 1)[0].permute(1, 2, 0).numpy(); dt = time.perf_counter() - t
    h, w = rgba.shape[:2]
    A = np.asarray(Image.fromarray(rgba[..., 3]).resize((w * Z, h * Z), Image.BICUBIC)).astype(np.float32)[..., None] / 255
    rgb = np.where(A > 1e-3, (c - 0.5 * (1 - A)) / np.maximum(A, 1e-3), 0)
    return (np.clip(np.concatenate([rgb, A], -1), 0, 1) * 255).round().astype(np.uint8), dt


def comp(rgba):
    im = Image.new('RGBA', (rgba.shape[1], rgba.shape[0]), BG + (255,)); im.alpha_composite(Image.fromarray(rgba, 'RGBA')); return im.convert('RGB')


rows = []
for body, crop in [(200, None), (46, (0.0, 0.3, 0.6, 1.0)), (217, None), (826, (0.25, 0.45, 0.75, 0.95))]:
    fr = [c for c in read_clips(f'/home/claudio/rust-target/uodynamapper-scratch/ingame-anim/ec/ec_{body:05}.bin.zst') if c['cls'] == 1][0]['frames'][2]
    ys, xs = np.nonzero(fr[..., 3]); fr = fr[ys.min():ys.max() + 1, xs.min():xs.max() + 1]
    if crop:
        h, w = fr.shape[:2]; fr = fr[int(crop[1] * h):int(crop[3] * h), int(crop[0] * w):int(crop[2] * w)]
    fr = np.ascontiguousarray(np.pad(fr, ((2, 2), (2, 2), (0, 0)))); h, w = fr.shape[:2]
    pil = Image.fromarray(fr, 'RGBA')
    cells = [('nearest', comp(np.asarray(pil.resize((w * Z, h * Z), Image.NEAREST)))),
             ('bicubic', comp(np.asarray(pil.resize((w * Z, h * Z), Image.BICUBIC))))]
    for name, m in models:
        out, dt = run(m, fr); cells.append((f'{name} {dt * 1e3:.0f} ms', comp(out)))
    rows.append((f'ec {body}', cells))
LABEL, GAP = 16, 6
W = max(sum(c.width + GAP for _, c in cells) for _, cells in rows); H = sum(cells[0][1].height + LABEL + GAP for _, cells in rows)
sheet = Image.new('RGB', (W, H), (24, 24, 24)); d = ImageDraw.Draw(sheet); y = 0
for name, cells in rows:
    x = 0
    for t, c in cells:
        d.text((x + 2, y + 2), f'{name}: {t}', fill=(255, 255, 220)); sheet.paste(c, (x, y + LABEL)); x += c.width + GAP
    y += cells[0][1].height + LABEL + GAP
sheet.save('/home/claudio/rust-target/uodynamapper-scratch/upscale/out/sr_ec_span.png'); print(sheet.size)
