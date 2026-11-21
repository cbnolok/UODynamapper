"""Exports 4xmssim_span_pretrain for the wgpu port: each Conv3XC folded into its single inference 3x3 conv
(`eval_conv`), the other convs as they are, fp16 safetensors; and reference tensors (input = frame premultiplied over
50% grey, CHW f32; output = the 4x RGB, CHW f32) for an Enhanced dog and horse frame, computed with the fp16-rounded
weights."""
import json, struct, sys
sys.path.insert(0, '/home/claudio/rust-target/uodynamapper-scratch/anim-interp/py')
sys.path.insert(0, '/home/claudio/rust-target/uodynamapper-scratch/upscale/arch')
import numpy as np, torch
from safetensors.torch import load_file
from clips import read_clips
from span import SPAN

OUT = '/home/claudio/rust-target/uodynamapper-scratch/upscale/export'
torch.set_grad_enabled(False)
sd = load_file('/home/claudio/rust-target/uodynamapper-scratch/upscale/dl/4xmssim_span_pretrain.safetensors')
m = SPAN(num_in_ch=3, num_out_ch=3, feature_channels=48, upscale=4, norm='no_norm' not in sd)
m.load_state_dict(sd, strict=False); m.eval()
m(torch.zeros(1, 3, 8, 8))                          # folds every Conv3XC into eval_conv
keep = {k: v for k, v in m.state_dict().items()
        if ('eval_conv' in k or not any(s in k for s in ('.sk.', '.conv.0.', '.conv.1.', '.conv.2.'))) and k != 'no_norm'}
print('norm', m.is_norm, 'tensors', len(keep), 'params', sum(v.numel() for v in keep.values()))
header, blobs, off = {}, [], 0
for k in sorted(keep):
    a = keep[k].to(torch.float16).contiguous().numpy().tobytes()
    header[k] = {'dtype': 'F16', 'shape': list(keep[k].shape), 'data_offsets': [off, off + len(a)]}; blobs.append(a); off += len(a)
h = json.dumps(header, separators=(',', ':')).encode(); h += b' ' * ((8 - len(h) % 8) % 8)
with open(f'{OUT}/span_4x_mssim_f16.safetensors', 'wb') as f:
    f.write(struct.pack('<Q', len(h))); f.write(h); [f.write(b) for b in blobs]
print('bytes', 8 + len(h) + off)
# reference with fp16-rounded folded weights
cur = m.state_dict()
for k, v in keep.items(): cur[k] = v.to(torch.float16).to(torch.float32)
m.load_state_dict(cur)
def frozen_forward(mod, x):  # eval forward uses eval_conv weights already set; prevent re-folding from fp32 branches
    return mod(x)
for mod in m.modules():
    if hasattr(mod, 'update_params'): mod.update_params = lambda: None
index = {}
for name, body in (('dog', 217), ('horse', 200)):
    fr = [c for c in read_clips(f'/home/claudio/rust-target/uodynamapper-scratch/ingame-anim/ec/ec_{body:05}.bin.zst') if c['cls'] == 1][0]['frames'][2]
    ys, xs = np.nonzero(fr[..., 3]); fr = fr[ys.min():ys.max() + 1, xs.min():xs.max() + 1].astype(np.float32) / 255
    a = fr[..., 3:]; x = torch.from_numpy(np.ascontiguousarray(fr[..., :3] * a + 0.5 * (1 - a))).permute(2, 0, 1)[None]
    y = m(x)
    x[0].numpy().astype('<f4').tofile(f'{OUT}/{name}_input.f32'); y[0].numpy().astype('<f4').tofile(f'{OUT}/{name}_output.f32')
    index[name] = {'width': x.shape[3], 'height': x.shape[2], 'scale': 4,
                   'layout': 'CHW f32 LE; input = RGB premultiplied over 50% grey in 0..1; output = 4x RGB, unclamped'}
json.dump(index, open(f'{OUT}/reference.json', 'w'), indent=1); print(index)
