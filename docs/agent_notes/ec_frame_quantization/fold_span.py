"""Folds a SPAN 48nf 4x checkpoint (safetensors) into the layout uot-gpu-compute's SPAN reads (the 44 tensors of
models/span_4x_mssim_f16.safetensors), with numpy only, for the ec_frame_quantization note's option (c):

    python3 -I -B fold_span.py <checkpoint.safetensors> <out.safetensors>

- Each Conv3XC (1x1 -> 3x3 -> 1x1, plus a 1x1 skip) becomes its single 3x3 `eval_conv`, composed here from the trained
  branches (a checkpoint's own `eval_conv` may be stale: SPAN refreshes it only in an eval forward). The fold is exact
  except at the frame border, as in SPAN's own fold (its bias of the first 1x1 reaches the zero padding).
- A checkpoint without a `no_norm` key normalizes its input, x' = (x - mean) * img_range, and does not undo it at the
  output; that is folded into conv_1 (exact except at the border, where the padding stands for x = mean).
"""
import json
import struct
import sys

import numpy as np

HEADER_LENGTH_BYTES = 8
# SPAN's default normalization (DIV2K mean, img_range 255).
SPAN_MEAN = np.array([0.4488, 0.4371, 0.4040], dtype=np.float64)
SPAN_IMG_RANGE = 255.0
DTYPES = {'F16': np.float16, 'F32': np.float32, 'BF16': None}


def read(path):
    data = open(path, 'rb').read()
    length = struct.unpack('<Q', data[:HEADER_LENGTH_BYTES])[0]
    header = json.loads(data[HEADER_LENGTH_BYTES:HEADER_LENGTH_BYTES + length])
    start = HEADER_LENGTH_BYTES + length
    tensors = {}
    for name, info in header.items():
        if name == '__metadata__':
            continue
        a, b = info['data_offsets']
        raw = data[start + a:start + b]
        if info['dtype'] == 'BF16':
            words = np.frombuffer(raw, dtype=np.uint16).astype(np.uint32) << 16
            array = words.view(np.float32)
        else:
            array = np.frombuffer(raw, dtype=DTYPES[info['dtype']])
        tensors[name] = array.astype(np.float64).reshape(info['shape'])
    return tensors


def fold_conv3xc(t, prefix):
    w1, b1 = t[f'{prefix}.conv.0.weight'][:, :, 0, 0], t[f'{prefix}.conv.0.bias']
    w2, b2 = t[f'{prefix}.conv.1.weight'], t[f'{prefix}.conv.1.bias']
    w3, b3 = t[f'{prefix}.conv.2.weight'][:, :, 0, 0], t[f'{prefix}.conv.2.bias']
    weight = np.einsum('on,nmyx,mi->oiyx', w3, w2, w1)
    bias = w3 @ (np.einsum('nmyx,m->n', w2, b1) + b2) + b3
    weight[:, :, 1, 1] += t[f'{prefix}.sk.weight'][:, :, 0, 0]
    return weight, bias + t[f'{prefix}.sk.bias']


def main():
    t = read(sys.argv[1])
    out = {}
    for name in sorted({k.rsplit('.conv.', 1)[0] for k in t if '.conv.0.' in k}):
        weight, bias = fold_conv3xc(t, name)
        if f'{name}.eval_conv.weight' in t:
            stale = np.abs(t[f'{name}.eval_conv.weight'] - weight).max()
            print(f'{name}: largest difference to the checkpoint eval_conv {stale:.4g}')
        out[f'{name}.eval_conv.weight'], out[f'{name}.eval_conv.bias'] = weight, bias
    for name in ('conv_cat', 'upsampler.0'):
        out[f'{name}.weight'], out[f'{name}.bias'] = t[f'{name}.weight'], t[f'{name}.bias']
    if 'no_norm' not in t:
        w, b = out['conv_1.eval_conv.weight'], out['conv_1.eval_conv.bias']
        out['conv_1.eval_conv.bias'] = b - SPAN_IMG_RANGE * np.einsum('oiyx,i->o', w, SPAN_MEAN)
        out['conv_1.eval_conv.weight'] = w * SPAN_IMG_RANGE
        print('normalization folded into conv_1')
    header, blobs, offset = {}, [], 0
    for name in sorted(out):
        blob = out[name].astype(np.float16).tobytes()
        header[name] = {'dtype': 'F16', 'shape': list(out[name].shape), 'data_offsets': [offset, offset + len(blob)]}
        blobs.append(blob)
        offset += len(blob)
    text = json.dumps(header, separators=(',', ':')).encode()
    text += b' ' * ((8 - len(text) % 8) % 8)
    with open(sys.argv[2], 'wb') as f:
        f.write(struct.pack('<Q', len(text)))
        f.write(text)
        for blob in blobs:
            f.write(blob)
    print(len(out), 'tensors written to', sys.argv[2])


if __name__ == '__main__':
    main()
