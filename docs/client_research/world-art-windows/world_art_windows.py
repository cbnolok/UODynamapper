"""Which tileart window holds the world-art (block 0) figure: the Enhanced Client's, Kingdom Reborn's or neither.

Usage: python3 -I -B world_art_windows.py <Enhanced Client dir> <Kingdom Reborn client dir> [--list]

Reads tileart.uop and string_dictionary.uop of both clients (EC record version 4, KR version 3; layout of
lib/uocf/src/enhanced/metadata/tileart.rs `TileArtEntry::parse_bytes`), the EC Texture.uop world-art images
(build/worldart/%08d.dds) and LegacyTexture.uop legacy images (build/tileartlegacy/%08d.dds), and prints the
counts of README.md. --list prints every record whose EC window differs from KR's. Needs Pillow (DDS DXT) and NumPy.
"""
import io, re, struct, sys, zlib
from collections import Counter
import numpy as np
from PIL import Image

UOP_HEADER_FMT, UOP_BLOCK_FMT, UOP_ENTRY_FMT = '<IIIQ', '<IQ', '<QIIIQIH'
UOP_ZLIB = 1
SDICT_HEADER_BYTES = 14
EC_VERSION, KR_VERSION = 4, 3
SHADER_BLOCKS, ENHANCED_BLOCK = 4, 2
SITTING_WORDS_EC, SITTING_WORDS_KR = 6, 4
APPEARANCE_BY_GENDER = 1
STATIC_NAMESPACE = 0x10000
USE_NEW_ART = 0x80000             # flags2 bit 19: UOTileArt::BuildSpriteDescriptor mode 1 (block 0, enhanced window)
ROTATED = 0x400000000             # flags1 bit 34: camera quad, the window is not a sprite window
CUT_SHARE = 0.02                  # alpha mass outside a window above which the window cuts the figure
COVER_SHARE = 0.0005              # at most this share outside: the window holds the whole figure
WORLDART_RE = re.compile(r'data\\worldart\\(\d+)', re.I)
MASK = 0xFFFFFFFF


def uop_payloads(path):
    d = open(path, 'rb').read()
    _, _, _, nxt = struct.unpack_from(UOP_HEADER_FMT, d, 0)
    while nxt:
        cnt, nxt2 = struct.unpack_from(UOP_BLOCK_FMT, d, nxt)
        p = nxt + struct.calcsize(UOP_BLOCK_FMT)
        for _ in range(cnt):
            off, hl, cl, dl, h, dh, comp = struct.unpack_from(UOP_ENTRY_FMT, d, p)
            p += struct.calcsize(UOP_ENTRY_FMT)
            if off:
                raw = d[off + hl: off + hl + cl]
                yield h, (zlib.decompress(raw) if comp == UOP_ZLIB else raw)
        nxt = nxt2


def rot(x, k):
    return ((x << k) | (x >> (32 - k))) & MASK


def hashlittle2(s):
    """UOP entry name hash (Jenkins lookup3 hashlittle2, seed 0xdeadbeef + length)."""
    length = len(s)
    a = b = c = (0xDEADBEEF + length) & MASK
    i = 0
    while length > 12:
        a = (a + struct.unpack_from('<I', s, i)[0]) & MASK
        b = (b + struct.unpack_from('<I', s, i + 4)[0]) & MASK
        c = (c + struct.unpack_from('<I', s, i + 8)[0]) & MASK
        a = (a - c) & MASK; a ^= rot(c, 4); c = (c + b) & MASK
        b = (b - a) & MASK; b ^= rot(a, 6); a = (a + c) & MASK
        c = (c - b) & MASK; c ^= rot(b, 8); b = (b + a) & MASK
        a = (a - c) & MASK; a ^= rot(c, 16); c = (c + b) & MASK
        b = (b - a) & MASK; b ^= rot(a, 19); a = (a + c) & MASK
        c = (c - b) & MASK; c ^= rot(b, 4); b = (b + a) & MASK
        i += 12; length -= 12
    if length == 0:
        return (b << 32) | c
    tail = s[i:] + b'\0' * (12 - length)
    a = (a + struct.unpack_from('<I', tail, 0)[0]) & MASK
    b = (b + struct.unpack_from('<I', tail, 4)[0]) & MASK
    c = (c + struct.unpack_from('<I', tail, 8)[0]) & MASK
    c ^= b; c = (c - rot(b, 14)) & MASK
    a ^= c; a = (a - rot(c, 11)) & MASK
    b ^= a; b = (b - rot(a, 25)) & MASK
    c ^= b; c = (c - rot(b, 16)) & MASK
    a ^= c; a = (a - rot(c, 4)) & MASK
    b ^= a; b = (b - rot(a, 14)) & MASK
    c ^= b; c = (c - rot(b, 24)) & MASK
    return (b << 32) | c


def strings(path):
    ((_, b),) = list(uop_payloads(path))
    p, out = SDICT_HEADER_BYTES, []
    while p < len(b):
        n = struct.unpack_from('<H', b, p)[0]
        out.append(b[p + 2: p + 2 + n].decode('latin-1'))
        p += 2 + n
    return out


class Reader:
    def __init__(self, b):
        self.b, self.p = b, 0

    def u(self, fmt):
        v = struct.unpack_from('<' + fmt, self.b, self.p)
        self.p += struct.calcsize('<' + fmt)
        return v if len(v) > 1 else v[0]


def parse(b, sd):
    """The record up to its shader blocks (the effects after them are not needed)."""
    r, rec = Reader(b), {}
    version = r.u('H')
    assert version in (EC_VERSION, KR_VERSION), version
    kr = version == KR_VERSION
    rec['name'] = sd[r.u('I')]
    rec['tile_id'] = r.u('I')
    rec['has_world_art'] = r.u('B')
    if not kr:
        r.u('B')                                            # uses_enhanced_tileart
    rec['world_art_scale'], rec['legacy_art_scale'] = round(r.u('f'), 4), round(r.u('f'), 4)
    r.u('IifI')                                             # field_60, old_id, rotation, body_type
    if not kr:
        r.u('B')                                            # alternate_animation
    r.u('fIffI')                                            # light scale, field_70, light offsets, house join bits
    rec['flags1'], rec['flags2'] = r.u('II') if kr else r.u('QQ')
    r.u('i')                                                # facing
    rec['enhanced_window'], rec['legacy_window'] = list(r.u('6i')), list(r.u('6i'))
    for _ in range(2):
        [r.u('BI') for _ in range(r.u('B'))]                # props1, props2
    [r.u('II') for _ in range(r.u('I'))]                    # stack aliases
    for _ in range(r.u('I')):                               # appearance filters
        if r.u('B') == APPEARANCE_BY_GENDER:
            r.u('BI')
        else:
            [r.u('II') for _ in range(r.u('I'))]
    if r.u('B'):
        r.u('%di' % (SITTING_WORDS_KR if kr else SITTING_WORDS_EC))
    if not kr:
        r.u('4B')                                           # radar colour
    blocks = []
    for i in range(SHADER_BLOCKS):
        if (kr and i == ENHANCED_BLOCK) or r.u('B') != 1:
            blocks.append(None)
            continue
        r.u('B'); shader = sd[r.u('I')]
        items = [sd[r.u('IBfII')[0]] for _ in range(r.u('B'))]
        [r.u('i') for _ in range(r.u('I'))]
        [r.u('f') for _ in range(r.u('I'))]
        blocks.append({'shader': shader, 'items': items})
    rec['world_art'] = blocks[0]['items'][0] if blocks[0] else None
    return rec


def records(client):
    """Static-namespace records by stored tile id (the 9 repeated and 22 misnamed ids of
    docs/agent_notes/ec_sprite_descriptor_geometry.md are not resolved by entry name here)."""
    sd = strings(f'{client}/string_dictionary.uop')
    recs = (parse(b, sd) for _, b in uop_payloads(f'{client}/tileart.uop'))
    return {r['tile_id']: r for r in recs if r['tile_id'] < STATIC_NAMESPACE}


def alpha(textures, path):
    raw = textures.get(hashlittle2(path))
    return None if raw is None else np.asarray(Image.open(io.BytesIO(raw)).convert('RGBA'))[..., 3].astype(np.float64)


def outside(a, win, inclusive):
    """Share of the image's alpha mass outside a window; mode 1 (enhanced) ends are inclusive, mode 0 exclusive."""
    if a is None or not a.sum():
        return None
    x0, y0, x1, y1 = (max(v, 0) for v in win[:4])
    return float(1 - a[y0:y1 + inclusive, x0:x1 + inclusive].sum() / a.sum())


def bbox(a):
    ys, xs = np.nonzero(a > 0)
    return [int(xs.min()), int(ys.min()), int(xs.max()), int(ys.max())] if len(xs) else None


def main(ec_dir, kr_dir, listing=False):
    ec, kr = records(ec_dir), records(kr_dir)
    world = dict(uop_payloads(f'{ec_dir}/Texture.uop'))
    legacy = dict(uop_payloads(f'{ec_dir}/LegacyTexture.uop'))
    print(f'EC records (static namespace, stored id): {len(ec)}; KR: {len(kr)}; EC with UseNewArt: '
          f'{sum(1 for r in ec.values() if r["flags2"] & USE_NEW_ART)}')

    # 1. Records with world art: does the EC window equal KR's?
    census, differing = Counter(), []
    for tid, e in sorted(ec.items()):
        if not e['has_world_art'] or not e['world_art']:
            continue
        k = kr.get(tid)
        if k is None:
            census['no KR record'] += 1
        elif not k['has_world_art'] or not k['world_art']:
            census['KR record without world art'] += 1
        elif k['enhanced_window'] == e['enhanced_window']:
            census['same enhanced window as KR'] += 1
        else:
            same = (k['world_art'].lower() == e['world_art'].lower() and k['world_art_scale'] == e['world_art_scale'])
            census['other window, ' + ('same texture and scale' if same else 'other texture or scale')] += 1
            differing.append((tid, e, k, same))
    print('\nEC records with world art:', sum(census.values()))
    for key, n in census.most_common():
        print(f'  {n:6} {key}')

    # 2. The differing ones: flags, window pattern, and what each window holds of the figure.
    table = Counter()
    for tid, e, k, same in differing:
        m = WORLDART_RE.search(e['world_art'])
        a = alpha(world, b'build/worldart/%08d.dds' % int(m.group(1))) if m else None
        out_ec, out_kr = outside(a, e['enhanced_window'], 1), outside(a, k['enhanced_window'], 1)
        la = alpha(legacy, b'build/tileartlegacy/%08d.dds' % tid)
        out_legacy = outside(la, e['legacy_window'], 0)
        b = bbox(a) if a is not None else None
        row = ('UseNewArt' if e['flags2'] & USE_NEW_ART else 'no UseNewArt',
               'EC window = EC legacy window' if e['enhanced_window'] == e['legacy_window'] else
               'EC window mixes legacy and KR values',
               'blank image' if out_ec is None else
               'KR window = alpha bounds' if b == k['enhanced_window'][:4] else
               'KR window holds the figure' if out_kr <= COVER_SHARE else 'KR window cuts a little',
               'EC window cuts' if out_ec is not None and out_ec > CUT_SHARE else 'EC window holds the figure',
               'legacy window holds the legacy image' if out_legacy is not None and out_legacy <= COVER_SHARE else
               'legacy image cut or missing')
        if same:
            table[row] += 1
        if listing:
            print(f'{tid:#06x} {e["name"]!r:28} scale {e["world_art_scale"]:<5} EC {e["enhanced_window"]} '
                  f'legacy {e["legacy_window"]} KR {k["enhanced_window"]} EC cuts '
                  f'{"-" if out_ec is None else f"{out_ec:.1%}"} same texture {same}')
    print('\nOther window, same texture and scale, by (flags2, EC window, KR window, EC window, legacy window):')
    for key, n in table.most_common():
        print(f'  {n:4} {" | ".join(key)}')

    # 3. EC-only records with world art whose enhanced window copies the legacy window.
    copied = [(tid, e) for tid, e in sorted(ec.items()) if tid not in kr and e['has_world_art'] and e['world_art']
              and e['enhanced_window'] == e['legacy_window'] and not e['flags1'] & ROTATED]
    print('\nEC-only records with world art and EC window = legacy window:', len(copied))
    for tid, e in copied:
        m = WORLDART_RE.search(e['world_art'])
        a = alpha(world, b'build/worldart/%08d.dds' % int(m.group(1))) if m else None
        if a is not None and a.sum():
            print(f'  {tid:#06x} {e["name"]!r} {e["world_art"]} EC window {e["enhanced_window"]} '
                  f'cuts {outside(a, e["enhanced_window"], 1):.1%}')


if __name__ == '__main__':
    args = [a for a in sys.argv[1:] if a != '--list']
    main(*args, listing='--list' in sys.argv)
