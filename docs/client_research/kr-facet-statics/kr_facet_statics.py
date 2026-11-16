#!/usr/bin/env python3
"""Compares the statics and land of Kingdom Reborn's facetN.uop sectors with the Enhanced Client's facetN.uop and
the Classic client's statics/staidx/map MUL files. Read only; run with `python3 -I -B`.

  kr_facet_statics.py area FACET X0 Y0 X1 Y1   the cave entrance art (0x3B60-0x3B83) in a rectangle, per client,
                                                and the squares whose KR statics or land differ from Classic
  kr_facet_statics.py census FACET              the whole facet: squares whose KR statics or land differ from
                                                Classic, the graphics only one side has, the cave entrance art

The folders default to the ones the study used; pass --kr, --ec and --classic to point elsewhere. The sector
format is the one UOKR.exe 2.58.0.6 and UOSA.exe read (../ghidra-facets/results/uokr_facets.md).
"""
import argparse
import collections
import os
import struct
import sys
import zlib

CLIENTS = '/mnt/dati/_proj_local/_uo_clients'
DEFAULT_KR = f'{CLIENTS}/UO - Old KR'
DEFAULT_EC = f'{CLIENTS}/_Ultima Online Enhanced'
DEFAULT_CLASSIC = f'{CLIENTS}/Ultima Online Classic_7_0_20_0'

# Facet sizes in squares (width, height) of the KR and Classic facets 0-4.
FACET_SIZE = {0: (7168, 4096), 1: (7168, 4096), 2: (2304, 1600), 3: (2560, 2048), 4: (1448, 1448)}
SECTOR_SIDE = 64
BLOCK_SIDE = 8
# The KR "cave entrance" art, the pieces of the rock arch over a cave's mouth.
CAVE_ENTRANCE_ART = range(0x3B60, 0x3B84)
DELIMITER_DIRECTIONS = 8
TOP_GRAPHICS = 25


def hashlittle2(text):
    """The UOP entry hash (Bob Jenkins' lookup3 hashlittle2), high word b, low word c."""
    data = text.encode('ascii')
    mask = 0xFFFFFFFF
    a = b = c = (0xDEADBEEF + len(data)) & mask

    def rot(x, k):
        return ((x << k) | (x >> (32 - k))) & mask
    i = 0
    while len(data) - i > 12:
        a = (a + int.from_bytes(data[i:i + 4], 'little')) & mask
        b = (b + int.from_bytes(data[i + 4:i + 8], 'little')) & mask
        c = (c + int.from_bytes(data[i + 8:i + 12], 'little')) & mask
        a = (a - c) & mask; a ^= rot(c, 4); c = (c + b) & mask
        b = (b - a) & mask; b ^= rot(a, 6); a = (a + c) & mask
        c = (c - b) & mask; c ^= rot(b, 8); b = (b + a) & mask
        a = (a - c) & mask; a ^= rot(c, 16); c = (c + b) & mask
        b = (b - a) & mask; b ^= rot(a, 19); a = (a + c) & mask
        c = (c - b) & mask; c ^= rot(b, 4); b = (b + a) & mask
        i += 12
    rest = len(data) - i
    if rest == 0:
        return (b << 32) | c
    tail = data[i:] + b'\0' * (12 - rest)
    a = (a + int.from_bytes(tail[0:4], 'little')) & mask
    b = (b + int.from_bytes(tail[4:8], 'little')) & mask
    c = (c + int.from_bytes(tail[8:12], 'little')) & mask
    c ^= b; c = (c - rot(b, 14)) & mask
    a ^= c; a = (a - rot(c, 11)) & mask
    b ^= a; b = (b - rot(a, 25)) & mask
    c ^= b; c = (c - rot(b, 16)) & mask
    a ^= c; a = (a - rot(c, 4)) & mask
    b ^= a; b = (b - rot(a, 14)) & mask
    c ^= b; c = (c - rot(b, 24)) & mask
    return (b << 32) | c


class Uop:
    def __init__(self, path):
        self.file = open(path, 'rb')
        magic, _version, _signature, next_block = struct.unpack('<4sIIQ', self.file.read(20))
        if magic != b'MYP\0':
            raise ValueError(f'{path}: not a UOP package')
        self.entries = {}
        while next_block:
            self.file.seek(next_block)
            count, next_block = struct.unpack('<IQ', self.file.read(12))
            for _ in range(count):
                offset, header, packed, _size, name_hash, _adler, compression = struct.unpack(
                    '<QIIIQIH', self.file.read(34))
                if offset:
                    self.entries[name_hash] = (offset + header, packed, compression)

    def read(self, name):
        offset, packed, compression = self.entries[hashlittle2(name)]
        self.file.seek(offset)
        data = self.file.read(packed)
        return zlib.decompress(data) if compression == 1 else data


class FacetSectors:
    """facetN.uop sectors; `kr` reads KR's square (z, material, subtype, legacy land) instead of EC's (z, land)."""

    def __init__(self, folder, facet, kr):
        self.uop = Uop(os.path.join(folder, f'facet{facet}.uop'))
        self.facet = facet
        self.kr = kr
        self.rows = -(-FACET_SIZE[facet][1] // SECTOR_SIDE)
        self.cached = (None, None)

    def sector(self, sector_id):
        if self.cached[0] == sector_id:
            return self.cached[1]
        data = self.uop.read(f'build/sectors/facet_{self.facet:02}/{sector_id:08}.bin')
        offset = 3
        squares = []
        for _ in range(SECTOR_SIDE * SECTOR_SIDE):
            if self.kr:
                z, _material, _subtype, land = struct.unpack_from('<bHBH', data, offset)
                offset += 6
            else:
                z, land = struct.unpack_from('<bH', data, offset)
                offset += 3
            delimiters = data[offset]
            offset += 1
            for _ in range(delimiters):
                offset += 1 + (5 if data[offset] < DELIMITER_DIRECTIONS else 0)
            count = data[offset]
            offset += 1
            statics = [struct.unpack_from('<IbI', data, offset + 9 * k) for k in range(count)]
            offset += 9 * count
            squares.append((land, z, statics))
        if offset != len(data):
            raise ValueError(f'sector {sector_id}: {len(data) - offset} bytes left')
        self.cached = (sector_id, squares)
        return squares

    def square(self, x, y):
        squares = self.sector(self.rows * (x // SECTOR_SIDE) + y // SECTOR_SIDE)
        return squares[(x % SECTOR_SIDE) * SECTOR_SIDE + y % SECTOR_SIDE]


class ClassicMul:
    def __init__(self, folder, facet):
        self.rows = FACET_SIZE[facet][1] // BLOCK_SIDE
        self.index = open(os.path.join(folder, f'staidx{facet}.mul'), 'rb').read()
        self.statics_data = open(os.path.join(folder, f'statics{facet}.mul'), 'rb').read()
        self.map = open(os.path.join(folder, f'map{facet}.mul'), 'rb').read()

    def block_statics(self, bx, by):
        offset, length, _ = struct.unpack_from('<iiI', self.index, (bx * self.rows + by) * 12)
        squares = collections.defaultdict(list)
        if offset >= 0 and length > 0:
            for k in range(length // 7):
                graphic, x, y, z, hue = struct.unpack_from('<HBBbH', self.statics_data, offset + 7 * k)
                squares[(x, y)].append((graphic, z, hue))
        return squares

    def land(self, x, y):
        block = (x // BLOCK_SIDE) * self.rows + y // BLOCK_SIDE
        cell = (y % BLOCK_SIDE) * BLOCK_SIDE + x % BLOCK_SIDE
        return struct.unpack_from('<Hb', self.map, block * 196 + 4 + cell * 3)


def static_set(statics):
    return collections.Counter((graphic, z) for graphic, z, _hue in statics)


def area(args):
    kr = FacetSectors(args.kr, args.facet, True)
    ec = FacetSectors(args.ec, args.facet, False)
    classic = ClassicMul(args.classic, args.facet)
    sources = {
        'KR facet': lambda x, y: kr.square(x, y)[2],
        'EC facet': lambda x, y: ec.square(x, y)[2],
        'Classic MUL': lambda x, y: classic.block_statics(x // BLOCK_SIDE, y // BLOCK_SIDE)[
            (x % BLOCK_SIDE, y % BLOCK_SIDE)],
    }
    squares = [(x, y) for y in range(args.y0, args.y1 + 1) for x in range(args.x0, args.x1 + 1)]
    for name, statics in sources.items():
        art = [f'{x},{y} {g:#06x} z{z}' for x, y in squares for g, z, _ in statics(x, y) if g in CAVE_ENTRANCE_ART]
        print(f'{name}: {len(art)} cave entrance pieces')
        for line in art:
            print('   ', line)
    print('Squares whose KR statics or land differ from Classic (graphic z):')
    for x, y in squares:
        kr_land, kr_z, kr_statics = kr.square(x, y)
        classic_land, classic_z = classic.land(x, y)
        classic_statics = sources['Classic MUL'](x, y)
        if static_set(kr_statics) == static_set(classic_statics) and (kr_land, kr_z) == (classic_land, classic_z):
            continue
        print(f'  {x},{y} land KR {kr_land:#06x} z{kr_z} Classic {classic_land:#06x} z{classic_z}'
              f' | KR {sorted((hex(g), z) for g, z, _ in kr_statics)}'
              f' | Classic {sorted((hex(g), z) for g, z, _ in classic_statics)}')


def census(args):
    width, height = FACET_SIZE[args.facet]
    kr = FacetSectors(args.kr, args.facet, True)
    classic = ClassicMul(args.classic, args.facet)
    only_kr, only_classic = collections.Counter(), collections.Counter()
    counts = collections.Counter()
    art = {'KR': collections.Counter(), 'Classic': collections.Counter()}
    for bx in range(width // BLOCK_SIDE):
        for by in range(height // BLOCK_SIDE):
            block = classic.block_statics(bx, by)
            for sx in range(BLOCK_SIDE):
                for sy in range(BLOCK_SIDE):
                    x, y = bx * BLOCK_SIDE + sx, by * BLOCK_SIDE + sy
                    kr_land, kr_z, kr_statics = kr.square(x, y)
                    classic_land, classic_z = classic.land(x, y)
                    counts['land z differs'] += kr_z != classic_z
                    counts['land id differs'] += kr_land != classic_land
                    counts['KR graphic or hue above u16'] += any(g > 0xFFFF or h > 0xFFFF for g, _, h in kr_statics)
                    for side, statics in (('KR', kr_statics), ('Classic', block[(sx, sy)])):
                        art[side].update(g for g, _, _ in statics if g in CAVE_ENTRANCE_ART)
                    kr_set, classic_set = static_set(kr_statics), static_set(block[(sx, sy)])
                    if kr_set == classic_set:
                        continue
                    if set(kr_set) == set(classic_set):
                        counts['statics differ only by repeats'] += 1
                        continue
                    counts['statics differ'] += 1
                    only_kr.update((kr_set - classic_set).elements())
                    only_classic.update((classic_set - kr_set).elements())
    print(f'Facet {args.facet}, {width}x{height} squares, KR {args.kr} vs Classic {args.classic}')
    for name, value in counts.items():
        print(f'  squares whose {name}: {value}')
    for name, counter in (('only in KR', only_kr), ('only in Classic', only_classic)):
        by_graphic = collections.Counter()
        for (graphic, _z), n in counter.items():
            by_graphic[graphic] += n
        print(f'  statics {name}: {sum(by_graphic.values())}; top graphics',
              ', '.join(f'{g:#06x}:{n}' for g, n in by_graphic.most_common(TOP_GRAPHICS)))
    for side, counter in art.items():
        print(f'  cave entrance pieces in {side}: {sum(counter.values())}')


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--kr', default=DEFAULT_KR)
    parser.add_argument('--ec', default=DEFAULT_EC)
    parser.add_argument('--classic', default=DEFAULT_CLASSIC)
    commands = parser.add_subparsers(dest='command', required=True)
    area_parser = commands.add_parser('area')
    census_parser = commands.add_parser('census')
    for sub in (area_parser, census_parser):
        sub.add_argument('facet', type=int, choices=sorted(FACET_SIZE))
    for name in ('x0', 'y0', 'x1', 'y1'):
        area_parser.add_argument(name, type=int)
    args = parser.parse_args()
    {'area': area, 'census': census}[args.command](args)


if __name__ == '__main__':
    sys.exit(main())
