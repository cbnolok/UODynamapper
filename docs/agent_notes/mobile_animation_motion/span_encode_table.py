"""The error table of `block_encode::tests::source_encodes::evaluate_source_frame_encodes`: for each encode of a frame,
how far its SPAN 2x frame, and that frame after the renderer's UltraFast encode, lie from the SPAN 2x frame of the
lossless frame, and how much its block edges stand out.

  python3 -I -B span_encode_table.py FOLDER REFERENCE NAME...

REFERENCE is the lossless frame's name (`{REFERENCE}_2x.png`), each NAME an encode's (`{NAME}_2x.png` and
`{NAME}_UltraFast.png`), e.g. `daemon_stand daemon_stand_package daemon_standCpu daemon_standUltraFast`. Columns:

- patches: the RMS of the colour error low-passed over LOWPASS x LOWPASS texels, over the texels opaque in both
  frames: the washed areas a viewer sees as patches, without the texel noise;
- texels: the plain RMS of the same error;
- block edges: how much larger the colour step is across the 2x lines where the 1x BC7 blocks meet than across the
  other lines between 1x texels, in percent (opaque pairs, both axes). The 1x frame starts on a block, as frames do in
  the packages' pages, and the upscale's border puts its 1x line 4k at 2x line BLOCK_LINE_STRIDE k + BLOCK_LINE_OFFSET.
  The lossless frame's own figure is the art's; an encode's excess over it is the straight seams.
"""
import sys
from pathlib import Path

import numpy as np
from PIL import Image

LOWPASS = 9
OPAQUE = 255
# 1x BC7 blocks of 4 texels, upscaled 2x past a 2-texel border.
BLOCK_LINE_STRIDE = 8
BLOCK_LINE_OFFSET = 4
# The 2x lines between 1x texels.
TEXEL_LINE_STRIDE = 2


def load(path):
    return np.asarray(Image.open(path).convert('RGBA')).astype(float)


def box_blur(image, side):
    kernel = np.ones(side) / side
    pad = side // 2
    out = np.pad(image, ((pad, pad), (pad, pad), (0, 0)), mode='edge')
    out = np.apply_along_axis(lambda line: np.convolve(line, kernel, 'valid'), 0, out)
    return np.apply_along_axis(lambda line: np.convolve(line, kernel, 'valid'), 1, out)


def errors(image, reference):
    solid = (image[..., 3] == OPAQUE) & (reference[..., 3] == OPAQUE)
    error = (image[..., :3] - reference[..., :3]) * solid[..., None]
    count = box_blur(solid[..., None].astype(float), LOWPASS)
    low = box_blur(error, LOWPASS) / np.maximum(count, 1e-6)
    patches = np.sqrt((low[solid & (count[..., 0] > 0.5)] ** 2).mean())
    return patches, np.sqrt((error[solid] ** 2).mean())


def block_edges(image):
    alpha = image[..., 3]
    color = image[..., :3]
    excess = []
    for axis in (0, 1):
        step = np.abs(np.diff(color, axis=axis)).sum(-1)
        lines = alpha.shape[axis] - 1
        both = np.minimum(np.take(alpha, range(lines), axis=axis),
                          np.take(alpha, range(1, lines + 1), axis=axis)) == OPAQUE
        line = np.arange(lines) + 1
        shape = [1, 1]
        shape[axis] = -1
        block = (line % BLOCK_LINE_STRIDE == BLOCK_LINE_OFFSET).reshape(shape)
        texel = (line % TEXEL_LINE_STRIDE == 0).reshape(shape) & ~block
        excess.append(step[both & block].mean() / step[both & texel].mean() - 1)
    return 100 * np.mean(excess)


def main():
    folder, reference, names = Path(sys.argv[1]), sys.argv[2], sys.argv[3:]
    target = load(folder / f'{reference}_2x.png')
    print('| encode | SPAN patches | SPAN texels | + UltraFast patches | + UltraFast texels | block edges % |')
    print('|---|---:|---:|---:|---:|---:|')
    for name in [reference] + names:
        upscaled = load(folder / f'{name}_2x.png')
        encoded = load(folder / f'{name}_UltraFast.png')
        span, encode = errors(upscaled, target), errors(encoded, target)
        print(f'| {name} | {span[0]:.2f} | {span[1]:.2f} | {encode[0]:.2f} | {encode[1]:.2f} | '
              f'{block_edges(encoded):+.1f} |')


if __name__ == '__main__':
    main()
