"""Exports RIFE v4.25 for the wgpu port: the weights as fp16 safetensors (BatchNorm-free model; names as in
IFNet_HDv3.py), and reference tensors of one 64x64 and one 256x256 pair (Classic dog, Enhanced wyrm) at t = 0.25,
0.5, 0.75: the 14-channel input, each block's flow and mask, and the merged 7-channel output, all f32 little-endian,
with a JSON index."""
import json, struct, sys
sys.path.insert(0, '/home/claudio/rust-target/uodynamapper-scratch/rife')
sys.path.insert(0, '/home/claudio/rust-target/uodynamapper-scratch/anim-interp/py')
import numpy as np
import torch
import torch.nn.functional as F
from clips import read_clips
from model.ifnet_v425 import IFNet

OUT = '/home/claudio/rust-target/uodynamapper-scratch/rife/export'
state = torch.load('/home/claudio/rust-target/uodynamapper-scratch/rife/models/v4.25/train_log/flownet.pkl', map_location='cpu', weights_only=True)
state = {k.replace('module.', ''): v for k, v in state.items()}
net = IFNet()
missing, unexpected = net.load_state_dict(state, False)
used = {k: v for k, v in state.items() if k in net.state_dict()}
print('tensors', len(used), 'unused in checkpoint', [k for k in state if k not in used][:8], 'missing', missing)
# safetensors, fp16
header, blobs, off = {}, [], 0
for k in sorted(used):
    a = used[k].to(torch.float16).contiguous().numpy().tobytes()
    header[k] = {'dtype': 'F16', 'shape': list(used[k].shape), 'data_offsets': [off, off + len(a)]}
    blobs.append(a); off += len(a)
h = json.dumps(header, separators=(',', ':')).encode()
h += b' ' * ((8 - len(h) % 8) % 8)
with open(f'{OUT}/rife_v4_25_f16.safetensors', 'wb') as f:
    f.write(struct.pack('<Q', len(h))); f.write(h); [f.write(b) for b in blobs]
print('weights bytes', 8 + len(h) + off)

# fp16-rounded weights for the reference, so the port is compared with what it will load
net.load_state_dict({k: v.to(torch.float16).to(torch.float32) for k, v in used.items()}, False)
net.eval(); torch.set_grad_enabled(False)
D = '/home/claudio/rust-target/uodynamapper-scratch/ingame-anim'
index = {}
for name, src, body, pair in [('dog', 'cc', 217, 1), ('wyrm', 'ec', 46, 2)]:
    clip = [c for c in read_clips(f'{D}/{src}/{src}_{body:05}.bin.zst') if c['cls'] == 1][0]
    fr = np.ascontiguousarray(clip['frames'][:, :, ::-1])
    h_, w_ = fr.shape[1:3]
    ph, pw = (h_ + 63) // 64 * 64, (w_ + 63) // 64 * 64
    def tensor(f):
        f = f.astype(np.float32) / 255; a = f[..., 3:]
        x = np.concatenate([f[..., :3] * a + 0.5 * (1 - a), f[..., :3] * a, a], -1)
        return F.pad(torch.from_numpy(np.ascontiguousarray(x)).permute(2, 0, 1)[None], (0, pw - w_, 0, ph - h_))
    x = torch.cat([tensor(fr[pair]), tensor(fr[pair + 1])], 1)
    files = {'input': f'{name}_input.f32'}
    x[0].numpy().astype('<f4').tofile(f'{OUT}/{name}_input.f32')
    for t in (0.25, 0.5, 0.75):
        flows, mask, merged = net(x, t, [16, 8, 4, 2, 1])
        tag = f'{name}_t{int(t * 100):03d}'
        flows[-1][0].numpy().astype('<f4').tofile(f'{OUT}/{tag}_flow.f32')
        mask[0].numpy().astype('<f4').tofile(f'{OUT}/{tag}_mask_logit.f32')
        merged[-1][0].numpy().astype('<f4').tofile(f'{OUT}/{tag}_merged.f32')
        for i, fl in enumerate(flows):
            fl[0].numpy().astype('<f4').tofile(f'{OUT}/{tag}_flow_block{i}.f32')
        files[str(t)] = tag
    index[name] = {'width': pw, 'height': ph, 'sprite': [w_, h_], 'channels_per_image': 7, 'files': files,
                   'layout': 'CHW f32 LE; input = image0 (7) then image1 (7); flow = 4 channels (t->0 xy, t->1 xy); '
                             'mask_logit before sigmoid; merged = 7 channels (grey RGB, premultiplied RGB, alpha)'}
json.dump(index, open(f'{OUT}/reference.json', 'w'), indent=1)
print('ok')
