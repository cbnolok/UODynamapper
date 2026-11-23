"""Sheets and flicker videos of `ec-frame-quantization-bench frames` output (docs/agent_notes/ec_frame_quantization.md).

    python3 -I -B sheets.py sheet <frames dir> <out dir> <name> <x>,<y>,<w>,<h> [variant,...]
    python3 -I -B sheets.py flicker <frames dir> <out dir> <name> <x>,<y>,<w>,<h> <variant>,<variant>
    python3 -I -B sheets.py motion <clip dir> <out dir> <name> <fps> <variant>,<variant>

`sheet`: one row per variant: the whole frame at 1x and 2x, the crop (in source texels) at 3x, and SPAN 2x of the crop
at 1.5x (3x of the source), all nearest, over grey. `flicker`: an mp4 cycling source, the first variant, source, the
second, 0.7 s each, with the frame at 1x and the crop at 3x. `motion`: the clip playing at `fps`, the variants side by
side at 3x nearest, then the SPAN 2x frames at 1.5x, then the RIFE x2 sequence at twice the fps; mp4.
Frames are composited premultiplied over BACKDROP. Needs PIL and ffmpeg.
"""
import glob
import os
import re
import subprocess
import sys

from PIL import Image, ImageDraw, ImageFont

BACKDROP = (90, 96, 84)
SHEET_BACKGROUND = (32, 32, 32)
LABEL = (235, 235, 235)
PAD = 8
LABEL_HEIGHT = 18
CROP_SCALE = 3
FLICKER_SECONDS = 0.7
FLICKER_CYCLES = 3
VIDEO_FPS = 30
CRF = 28
DEFAULT_VARIANTS = ['source', 'package', 'highlight', 'median', 'bilateral', 'guided', 'palette', 'nomos']
MOTION_LOOPS = 4


def font():
    try:
        return ImageFont.load_default(size=14)
    except TypeError:
        return ImageFont.load_default()


def load(path):
    w, h = map(int, re.search(r'_(\d+)x(\d+)\.rgba$', path).groups())
    img = Image.frombytes('RGBA', (w, h), open(path, 'rb').read())
    back = Image.new('RGBA', (w, h), BACKDROP + (255,))
    return Image.alpha_composite(back, img).convert('RGB')


def find(frames_dir, name, variant):
    hits = glob.glob(os.path.join(frames_dir, f'{name}_{variant}_*x*.rgba'))
    return load(hits[0]) if hits else None


def scaled(img, scale):
    return img.resize((round(img.width * scale), round(img.height * scale)), Image.NEAREST)


def crop_of(img, rect, factor=1):
    x, y, w, h = rect
    return img.crop((x * factor, y * factor, (x + w) * factor, (y + h) * factor))


def sheet(frames_dir, out_dir, name, rect, variants):
    f = font()
    rows = []
    for v in variants:
        one = find(frames_dir, name, v)
        if one is None:
            continue
        span = find(frames_dir, name, f'span-{v}')
        cells = [one, scaled(one, 2), scaled(crop_of(one, rect), CROP_SCALE)]
        if span is not None:
            # SPAN's canvas has a 2-texel border at 1x (4 at 2x) around the frame.
            border = (span.width - 2 * one.width) // 2
            x, y, w, h = rect
            cells.append(scaled(span.crop((2 * x + border, 2 * y + border, 2 * (x + w) + border,
                                           2 * (y + h) + border)), CROP_SCALE / 2))
        rows.append((v, cells))
    # Frames over the upscaler's side have no SPAN column.
    widths = [max((cells[i].width for _, cells in rows if i < len(cells)), default=0) for i in range(4)]
    height = sum(max(c.height for c in cells) + LABEL_HEIGHT + PAD for _, cells in rows) + PAD
    out = Image.new('RGB', (sum(widths) + PAD * 5, height), SHEET_BACKGROUND)
    draw = ImageDraw.Draw(out)
    y = PAD
    titles = ['1x', '2x', f'crop {CROP_SCALE}x', f'SPAN 2x crop ({CROP_SCALE}x)']
    for v, cells in rows:
        x = PAD
        for i, c in enumerate(cells):
            draw.text((x, y), f'{v} {titles[i]}', fill=LABEL, font=f)
            out.paste(c, (x, y + LABEL_HEIGHT))
            x += widths[i] + PAD
        y += max(c.height for c in cells) + LABEL_HEIGHT + PAD
    os.makedirs(out_dir, exist_ok=True)
    path = os.path.join(out_dir, f'{name}_sheet.png')
    out.save(path)
    print(path)


def encode(frames, out_path, fps):
    """Writes PIL frames (one size) to an mp4: libx264, crf CRF, faststart, yuv420p."""
    w, h = frames[0].size
    w2, h2 = w + w % 2, h + h % 2
    cmd = ['ffmpeg', '-v', 'error', '-y', '-f', 'rawvideo', '-pix_fmt', 'rgb24', '-s', f'{w2}x{h2}', '-r', str(fps),
           '-i', '-', '-c:v', 'libx264', '-crf', str(CRF), '-preset', 'slow', '-pix_fmt', 'yuv420p',
           '-movflags', '+faststart', out_path]
    proc = subprocess.Popen(cmd, stdin=subprocess.PIPE)
    for fr in frames:
        if fr.size != (w2, h2):
            padded = Image.new('RGB', (w2, h2), SHEET_BACKGROUND)
            padded.paste(fr, (0, 0))
            fr = padded
        proc.stdin.write(fr.tobytes())
    proc.stdin.close()
    proc.wait()
    print(out_path, os.path.getsize(out_path) // 1024, 'KiB')


def flicker(frames_dir, out_dir, name, rect, variants):
    f = font()
    states = ['source', variants[0], 'source', variants[1]]
    images = []
    for v in states:
        one = find(frames_dir, name, v)
        crop = scaled(crop_of(one, rect), CROP_SCALE)
        w = one.width + crop.width + 3 * PAD
        h = max(one.height, crop.height) + LABEL_HEIGHT + 2 * PAD
        img = Image.new('RGB', (w, h), SHEET_BACKGROUND)
        d = ImageDraw.Draw(img)
        d.text((PAD, PAD), f'{name}: {v} (1x | crop {CROP_SCALE}x)', fill=LABEL, font=f)
        img.paste(one, (PAD, PAD + LABEL_HEIGHT))
        img.paste(crop, (2 * PAD + one.width, PAD + LABEL_HEIGHT))
        images.append(img)
    hold = round(FLICKER_SECONDS * VIDEO_FPS)
    frames = [img for _ in range(FLICKER_CYCLES) for img in images for _ in range(hold)]
    os.makedirs(out_dir, exist_ok=True)
    encode(frames, os.path.join(out_dir, f'{name}_flicker.mp4'), VIDEO_FPS)


def read_clip(clip_dir, name):
    rows = {}
    for line in open(os.path.join(clip_dir, 'clip.tsv')).read().splitlines()[1:]:
        n, variant, index, w, h, ax, ay, file = line.split('\t')
        if n != name:
            continue
        rows.setdefault(variant, []).append((int(index), int(ax), int(ay), os.path.join(clip_dir, file)))
    return {v: sorted(r) for v, r in rows.items()}


def card(entries, scale):
    """The clip's frames on one card, each placed by its anchor, scaled nearest."""
    loaded = [(load(p), ax, ay) for _, ax, ay, p in entries]
    left = max(ax for _, ax, _ in loaded)
    top = max(ay for _, _, ay in loaded)
    right = max(img.width - ax for img, ax, _ in loaded)
    bottom = max(img.height - ay for img, _, ay in loaded)
    out = []
    for img, ax, ay in loaded:
        c = Image.new('RGB', (left + right, top + bottom), BACKDROP)
        c.paste(img, (left - ax, top - ay))
        out.append(scaled(c, scale))
    return out


def motion(clip_dir, out_dir, name, fps, variants):
    f = font()
    clip = read_clip(clip_dir, name)
    stages = [('', CROP_SCALE, fps, '1x frames, 3x nearest'), ('span-', CROP_SCALE / 2, fps, 'SPAN 2x, 1.5x nearest'),
              ('rife-', CROP_SCALE, 2 * fps, 'RIFE x2, 3x nearest')]
    video = []
    for prefix, scale, stage_fps, title in stages:
        if not all(prefix + v in clip for v in variants):
            continue
        cards = [card(clip[prefix + v], scale) for v in variants]
        count = len(cards[0])
        repeat = round(VIDEO_FPS / stage_fps)
        cw = sum(c[0].width for c in cards) + PAD * (len(cards) + 1)
        ch = max(c[0].height for c in cards) + LABEL_HEIGHT + 2 * PAD
        for _ in range(MOTION_LOOPS):
            for i in range(count):
                img = Image.new('RGB', (cw, ch), SHEET_BACKGROUND)
                d = ImageDraw.Draw(img)
                x = PAD
                for v, c in zip(variants, cards):
                    d.text((x, PAD), f'{v}: {title}', fill=LABEL, font=f)
                    img.paste(c[i], (x, PAD + LABEL_HEIGHT))
                    x += c[0].width + PAD
                video.extend([img] * repeat)
    width = max(v.width for v in video)
    height = max(v.height for v in video)
    framed = []
    for v in video:
        if v.size != (width, height):
            p = Image.new('RGB', (width, height), SHEET_BACKGROUND)
            p.paste(v, (0, 0))
            v = p
        framed.append(v)
    os.makedirs(out_dir, exist_ok=True)
    encode(framed, os.path.join(out_dir, f'{name}_motion.mp4'), VIDEO_FPS)


def main():
    cmd = sys.argv[1]
    if cmd == 'sheet':
        rect = tuple(map(int, sys.argv[5].split(',')))
        variants = sys.argv[6].split(',') if len(sys.argv) > 6 else DEFAULT_VARIANTS
        sheet(sys.argv[2], sys.argv[3], sys.argv[4], rect, variants)
    elif cmd == 'flicker':
        flicker(sys.argv[2], sys.argv[3], sys.argv[4], tuple(map(int, sys.argv[5].split(','))), sys.argv[6].split(','))
    elif cmd == 'motion':
        motion(sys.argv[2], sys.argv[3], sys.argv[4], float(sys.argv[5]), sys.argv[6].split(','))
    else:
        print(__doc__)


if __name__ == '__main__':
    main()
