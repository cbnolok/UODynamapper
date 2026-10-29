#!/usr/bin/env python3
"""Decode build/multicollection/housing.bin and the multis of MultiCollection.uop the way UOSA.exe reads them.

Read-only verification of the layouts documented in results/uosa_housing.md. housing.bin and every multi
entry must be consumed to their exact last byte. Optional inputs resolve the references: tileart.uop (tile
ids), string_dictionary.uop (multi part strings) and a classic Cliloc.enu (tooltip and entitlement clilocs).

Usage:
    python3 -I verify_housing.py --multicollection MULTI_COLLECTION_UOP [--tileart TILEART_UOP]
                                 [--strings STRING_DICTIONARY_UOP] [--cliloc CLILOC_ENU] [--dump]
"""

import argparse
import collections
import struct
import sys
import zlib

# UOP container
UOP_MAGIC = b"MYP\x00"
UOP_HEADER_FORMAT = "<4sIIQII"
UOP_BLOCK_HEADER_FORMAT = "<IQ"
UOP_ENTRY_FORMAT = "<qIIIQIh"
UOP_COMPRESSION_NONE = 0
UOP_COMPRESSION_ZLIB = 1

# Jenkins lookup3 hashlittle2
HASH_SEED = 0xDEADBEEF
HASH_BLOCK = 12
WORD_MASK = 0xFFFFFFFF
WORD_BITS = 32

# MultiCollection.uop entry names
HOUSING_NAME = "build/multicollection/housing.bin"
MULTI_NAME = "build/multicollection/%06d.bin"
MAX_MULTI_ID = 0x10000

# housing.bin
U32 = "<I"
PIECE_HEADER_FORMAT = "<IIII"  # category, group, feature mask, tooltip cliloc
SLOT_ENTRY_FORMAT = "<II"  # slot, id
LIST_SOLID = "Solid"
LIST_MULTI = "Multi"
LIST_WINDOW = "Window"
LISTS_IN_FILE_ORDER = (LIST_SOLID, LIST_MULTI, LIST_WINDOW)
TILE_TYPE_NAMES = {1: "stairs", 2: "roof", 3: "door", 4: "floor", 5: "wall", 6: "teleporter", 7: "misc"}
FIRST_TILE_TYPE = 1  # BuildPieceTables visits tile types 1..7 (0 and 8 are skipped)
LAST_TILE_TYPE = 7
CLIENT_EXPANSION_FILTER = 0x6F3BFF  # filter +0xc in BuildPieceTables
NO_PURCHASED_EXPANSIONS = 0x1
# Client strings of HousingPieceFilter::Describe, and the 0xB9 flags that pick an entitlement cliloc.
EXPANSION_NAMES = {0x40: "AOW", 0x80: "EP1", 0x100: "8th", 0x200: "9th", 0x10000: "SA"}
ENTITLEMENT_CLILOCS = {0x40000: 1150774, 0x80000: 1150773, 0x400000: 1156114}
DEFAULT_ENTITLEMENT_CLILOC = 1062930
# Names of the theme-pack bits, taken from the texts of their entitlement clilocs (not client strings).
THEME_NAMES = {0x40000: "Gothic", 0x80000: "Rustic", 0x400000: "Time of Legends"}

# MultiCollection entry
MULTI_HEADER_FORMAT = "<II"  # multi id, part count
MULTI_PART_FORMAT = "<HhhhBBI"  # tile, x, y, z, not drawn, flag2, string count
APPEARANCE_PARTS = 10  # LoadMultiAppearance keeps the first 10 drawn parts

# tileart.uop record: u16 version, u32 name index, u32 tile id
TILEART_TILE_ID_OFFSET = 6

# string_dictionary.uop
STRING_DICTIONARY_HEADER_FORMAT = "<QIH"

# Classic Cliloc.enu: i32 + i16 header, then u32 id, u8 flag, u16 length, UTF-8 text
CLILOC_HEADER_SIZE = 6
CLILOC_ENTRY_FORMAT = "<IBH"

MAX_SHOWN_VALUES = 12


class FormatError(Exception):
    pass


class Reader:
    def __init__(self, data, label):
        self.data = data
        self.pos = 0
        self.label = label

    def take(self, fmt):
        size = struct.calcsize(fmt)
        if self.pos + size > len(self.data):
            raise FormatError(f"{self.label}: read of {size} bytes at {self.pos} past end {len(self.data)}")
        values = struct.unpack_from(fmt, self.data, self.pos)
        self.pos += size
        return values if len(values) > 1 else values[0]

    def skip(self, size):
        if self.pos + size > len(self.data):
            raise FormatError(f"{self.label}: skip of {size} bytes at {self.pos} past end {len(self.data)}")
        self.pos += size

    def remaining(self):
        return len(self.data) - self.pos


def rotate(x, k):
    return ((x << k) | (x >> (WORD_BITS - k))) & WORD_MASK


def hashlittle2(text):
    data = text.encode("ascii")
    length = len(data)
    a = b = c = (HASH_SEED + length) & WORD_MASK
    k = 0
    while length > HASH_BLOCK:
        x, y, z = struct.unpack_from("<III", data, k)
        a = (a + x) & WORD_MASK
        b = (b + y) & WORD_MASK
        c = (c + z) & WORD_MASK
        a = (a - c) & WORD_MASK; a ^= rotate(c, 4); c = (c + b) & WORD_MASK
        b = (b - a) & WORD_MASK; b ^= rotate(a, 6); a = (a + c) & WORD_MASK
        c = (c - b) & WORD_MASK; c ^= rotate(b, 8); b = (b + a) & WORD_MASK
        a = (a - c) & WORD_MASK; a ^= rotate(c, 16); c = (c + b) & WORD_MASK
        b = (b - a) & WORD_MASK; b ^= rotate(a, 19); a = (a + c) & WORD_MASK
        c = (c - b) & WORD_MASK; c ^= rotate(b, 4); b = (b + a) & WORD_MASK
        length -= HASH_BLOCK
        k += HASH_BLOCK
    if length == 0:
        return (b << WORD_BITS) | c
    tail = data[k:] + b"\x00" * HASH_BLOCK
    x, y, z = struct.unpack_from("<III", tail, 0)
    a = (a + x) & WORD_MASK
    if length > 4:
        b = (b + y) & WORD_MASK
    if length > 8:
        c = (c + z) & WORD_MASK
    c ^= b; c = (c - rotate(b, 14)) & WORD_MASK
    a ^= c; a = (a - rotate(c, 11)) & WORD_MASK
    b ^= a; b = (b - rotate(a, 25)) & WORD_MASK
    c ^= b; c = (c - rotate(b, 16)) & WORD_MASK
    a ^= c; a = (a - rotate(c, 4)) & WORD_MASK
    b ^= a; b = (b - rotate(a, 14)) & WORD_MASK
    c ^= b; c = (c - rotate(b, 24)) & WORD_MASK
    return (b << WORD_BITS) | c


def uop_entries(path):
    """Return {hash: payload} for every non-empty entry."""
    with open(path, "rb") as f:
        blob = f.read()
    magic, _version, _misc, start, _block_size, _count = struct.unpack_from(UOP_HEADER_FORMAT, blob, 0)
    if magic != UOP_MAGIC:
        raise FormatError(f"{path}: not a UOP file")
    entries = {}
    entry_size = struct.calcsize(UOP_ENTRY_FORMAT)
    block = start
    while block:
        count, block_next = struct.unpack_from(UOP_BLOCK_HEADER_FORMAT, blob, block)
        pos = block + struct.calcsize(UOP_BLOCK_HEADER_FORMAT)
        for _ in range(count):
            offset, header_len, compressed, decompressed, name_hash, _adler, flag = struct.unpack_from(
                UOP_ENTRY_FORMAT, blob, pos)
            pos += entry_size
            if offset == 0:
                continue
            raw = blob[offset + header_len: offset + header_len + compressed]
            if flag == UOP_COMPRESSION_ZLIB:
                raw = zlib.decompress(raw)
            elif flag != UOP_COMPRESSION_NONE:
                raise FormatError(f"{path}: unsupported compression {flag}")
            if len(raw) != decompressed:
                raise FormatError(f"{path}: entry {name_hash:#x} size {len(raw)} != {decompressed}")
            entries[name_hash] = raw
        block = block_next
    return entries


# ---- housing.bin (UOCustomHousingPieces::Parse) ----

def read_housing(payload):
    r = Reader(payload, HOUSING_NAME)
    types = []
    for _ in range(r.take(U32)):
        tile_type = r.take(U32)
        pieces = []
        for _ in range(r.take(U32)):
            category, group, mask, tooltip = r.take(PIECE_HEADER_FORMAT)
            lists = {}
            for name in LISTS_IN_FILE_ORDER:
                lists[name] = [r.take(SLOT_ENTRY_FORMAT) for _ in range(r.take(U32))]
            pieces.append({"category": category, "group": group, "mask": mask, "tooltip": tooltip, **lists})
        types.append((tile_type, pieces))
    if r.remaining():
        raise FormatError(f"{HOUSING_NAME}: {r.remaining()} bytes left after {len(types)} tile types")
    return types


# ---- multi entries (UOMultiCollection::Parse) ----

def read_multi(payload, label):
    r = Reader(payload, label)
    multi_id, count = r.take(MULTI_HEADER_FORMAT)
    parts = []
    for _ in range(count):
        tile, x, y, z, not_drawn, flag2, n = r.take(MULTI_PART_FORMAT)
        strings = [r.take(U32) for _ in range(n)]
        parts.append((tile, x, y, z, not_drawn, flag2, strings))
    if r.remaining():
        raise FormatError(f"{label}: {r.remaining()} bytes left after {count} parts")
    return multi_id, parts


# ---- optional references ----

def read_tile_ids(path):
    return {struct.unpack_from(U32, p, TILEART_TILE_ID_OFFSET)[0] for p in uop_entries(path).values()}


def read_string_dictionary(path):
    entries = list(uop_entries(path).values())
    if len(entries) != 1:
        raise FormatError(f"{path}: expected one payload, found {len(entries)}")
    r = Reader(entries[0], path)
    _stamp, count, _max_len = r.take(STRING_DICTIONARY_HEADER_FORMAT)
    strings = []
    for _ in range(count):
        length = r.take("<H")
        strings.append(r.data[r.pos:r.pos + length].decode("utf-8", "replace"))
        r.skip(length)
    return strings


def read_cliloc(path):
    with open(path, "rb") as f:
        r = Reader(f.read(), path)
    r.skip(CLILOC_HEADER_SIZE)
    table = {}
    while r.remaining():
        number, _flag, length = r.take(CLILOC_ENTRY_FORMAT)
        table[number] = r.data[r.pos:r.pos + length].decode("utf-8", "replace")
        r.skip(length)
    return table


# ---- report ----

def histogram(counter):
    items = sorted(counter.items())
    shown = ", ".join(f"{k}: {v}" for k, v in items[:MAX_SHOWN_VALUES])
    return shown + (f", ... ({len(items)} values)" if len(items) > MAX_SHOWN_VALUES else "")


def mask_text(mask):
    if mask == NO_PURCHASED_EXPANSIONS:
        return "1 (no purchased expansions)"
    names = [name for bit, name in {**EXPANSION_NAMES, **THEME_NAMES}.items() if mask & bit]
    return f"{mask:#x}" + (f" ({', '.join(names)})" if names else "")


def entitlement_cliloc(mask):
    return ENTITLEMENT_CLILOCS.get(mask, DEFAULT_ENTITLEMENT_CLILOC)


def report_housing(types, dump):
    print(f"housing.bin: {len(types)} tile types, consumed exactly")
    keys = collections.Counter()
    lookup = collections.defaultdict(list)
    for tile_type, pieces in types:
        name = TILE_TYPE_NAMES.get(tile_type, "?")
        categories = sorted({p["category"] for p in pieces})
        groups = collections.Counter(p["group"] for p in pieces)
        masks = collections.Counter(p["mask"] for p in pieces)
        print(f"== tile type {tile_type} ({name}): {len(pieces)} pieces, categories {categories[0]}..{categories[-1]}"
              f" ({len(categories)} distinct)")
        missing = sorted(set(range(categories[0], categories[-1] + 1)) - set(categories))
        if missing:
            print(f"   categories without pieces (entries still created): {missing}")
        print(f"   groups: {histogram(groups)}")
        print(f"   feature masks: " + ", ".join(f"{mask_text(m)}: {n}" for m, n in sorted(masks.items())))
        for list_name in LISTS_IN_FILE_ORDER:
            slots = collections.Counter(s for p in pieces for s, _ in p[list_name])
            lengths = collections.Counter(len(p[list_name]) for p in pieces)
            if slots:
                print(f"   {list_name}: entries per piece {histogram(lengths)}; slots {histogram(slots)}")
        for p in pieces:
            keys[(tile_type, p["category"], p["group"])] += 1
            for list_name in (LIST_SOLID, LIST_WINDOW):
                for slot, tile in p[list_name]:
                    lookup[tile].append((tile_type, p["category"], p["group"], list_name, slot))
            if dump:
                print(f"   {p}")
        if not FIRST_TILE_TYPE <= tile_type <= LAST_TILE_TYPE:
            print(f"   tile type outside {FIRST_TILE_TYPE}..{LAST_TILE_TYPE}: ignored by BuildPieceTables")
    duplicates = [k for k, n in keys.items() if n > 1]
    print(f"duplicate (tile type, category, group) keys: {len(duplicates)} {duplicates[:MAX_SHOWN_VALUES]}")
    unfiltered = [(t, p["category"], p["group"]) for t, ps in types for p in ps if not p["mask"] & CLIENT_EXPANSION_FILTER]
    print(f"pieces dropped by the client expansion filter {CLIENT_EXPANSION_FILTER:#x}: {len(unfiltered)}")
    shared = {tile: refs for tile, refs in lookup.items() if len(refs) > 1}
    print(f"tile ids in more than one Solid/Window slot (the tile lookup keeps the last): {len(shared)}")
    for tile, refs in sorted(shared.items())[:MAX_SHOWN_VALUES]:
        print(f"   {tile}: {refs}")
    report_categories(types)


def report_categories(types):
    mixed_masks = []
    tooltip_counts = collections.Counter()
    several = []
    entitlement = collections.Counter()
    for tile_type, pieces in types:
        by_category = collections.defaultdict(list)
        for p in pieces:
            by_category[p["category"]].append(p)
        for category, members in sorted(by_category.items()):
            masks = [p["mask"] for p in members]
            if len(set(masks)) > 1:
                mixed_masks.append((tile_type, category, masks))
            # GetFeatureMask returns the mask of the last piece of the category.
            entitlement[(members[-1]["mask"], entitlement_cliloc(members[-1]["mask"]))] += 1
            tooltips = [p["tooltip"] for p in members if p["tooltip"]]
            tooltip_counts[len(tooltips)] += 1
            if len(tooltips) > 1:
                several.append((tile_type, category, tooltips))
    print(f"categories per number of nonzero tooltips: {histogram(tooltip_counts)}")
    print(f"categories with several tooltips (GetTooltip fails, keeps the first): {several}")
    print(f"categories whose groups differ in feature mask (the last piece decides): {mixed_masks}")
    print("category entitlement (last mask, entitlementTID): "
          + ", ".join(f"{mask_text(m)} -> {c}: {n}" for (m, c), n in sorted(entitlement.items())))


def report_multis(multis, strings):
    flags = collections.Counter()
    z_values = []
    string_counts = collections.Counter()
    names = collections.Counter()
    for _multi_id, parts in multis.values():
        for tile, _x, _y, z, not_drawn, flag2, refs in parts:
            flags[(not_drawn, flag2)] += 1
            z_values.append(z)
            string_counts[len(refs)] += 1
            for index in refs:
                if strings is not None:
                    if index >= len(strings):
                        raise FormatError(f"string dictionary index {index} out of range {len(strings)}")
                    names[strings[index]] += 1
    mismatched = [name_id for name_id, (multi_id, _) in multis.items() if name_id != multi_id]
    print(f"multis: {len(multis)} entries, all consumed exactly; header id != name id: {len(mismatched)}")
    print(f"part (not drawn, flag2): {histogram(flags)}")
    print(f"part z: {min(z_values)}..{max(z_values)}; outside i8: {sum(not -128 <= z <= 127 for z in z_values)}")
    print(f"strings per part: {histogram(string_counts)}")
    if strings is not None:
        print("part strings: " + ", ".join(f"{s!r}: {n}" for s, n in names.most_common(MAX_SHOWN_VALUES)))


def report_cross(types, multis, tiles, cliloc):
    multi_refs = [(t, p["category"], p["group"], slot, mid) for t, ps in types for p in ps
                  for slot, mid in p[LIST_MULTI]]
    missing = [r for r in multi_refs if r[-1] not in multis]
    print(f"housing Multi references: {len(multi_refs)}, missing in MultiCollection: {missing}")
    truncated = 0
    for ref in multi_refs:
        if ref[-1] in multis:
            drawn = [part for part in multis[ref[-1]][1] if not part[4]]
            truncated += len(drawn) > APPEARANCE_PARTS
    print(f"referenced multis with more than {APPEARANCE_PARTS} drawn parts (appearance truncated): {truncated}")
    if tiles is not None:
        tile_refs = [(t, p["category"], p["group"], name, slot, tile) for t, ps in types for p in ps
                     for name in (LIST_SOLID, LIST_WINDOW) for slot, tile in p[name]]
        absent = [r for r in tile_refs if r[-1] not in tiles]
        print(f"housing tile references: {len(tile_refs)}, missing in tileart: {absent[:MAX_SHOWN_VALUES]}")
        part_tiles = {part[0] for _, parts in multis.values() for part in parts}
        print(f"multi part tiles: {len(part_tiles)} distinct, missing in tileart: "
              f"{sorted(part_tiles - tiles)[:MAX_SHOWN_VALUES]}")
    if cliloc is not None:
        tooltips = sorted({p["tooltip"] for _, ps in types for p in ps if p["tooltip"]})
        unresolved = [c for c in tooltips if c not in cliloc]
        print(f"tooltip clilocs: {len(tooltips)} distinct, unresolved: {unresolved}")
        for number in sorted(set(ENTITLEMENT_CLILOCS.values()) | {DEFAULT_ENTITLEMENT_CLILOC}):
            print(f"   entitlement {number}: {cliloc.get(number)!r}")


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--multicollection", required=True, help="path to MultiCollection.uop")
    parser.add_argument("--tileart", help="path to tileart.uop (checks tile ids)")
    parser.add_argument("--strings", help="path to string_dictionary.uop (resolves multi part strings)")
    parser.add_argument("--cliloc", help="path to a classic Cliloc.enu (resolves tooltip clilocs)")
    parser.add_argument("--dump", action="store_true", help="print every housing piece")
    args = parser.parse_args()
    try:
        entries = uop_entries(args.multicollection)
        housing_hash = hashlittle2(HOUSING_NAME)
        if housing_hash not in entries:
            raise FormatError(f"{args.multicollection}: no {HOUSING_NAME}")
        types = read_housing(entries.pop(housing_hash))
        names = {hashlittle2(MULTI_NAME % i): i for i in range(MAX_MULTI_ID)}
        unknown = [h for h in entries if h not in names]
        if unknown:
            raise FormatError(f"{len(unknown)} entries match neither {HOUSING_NAME} nor {MULTI_NAME}")
        multis = {names[h]: read_multi(p, MULTI_NAME % names[h]) for h, p in entries.items()}
        strings = read_string_dictionary(args.strings) if args.strings else None
        tiles = read_tile_ids(args.tileart) if args.tileart else None
        cliloc = read_cliloc(args.cliloc) if args.cliloc else None
        report_housing(types, args.dump)
        report_multis(multis, strings)
        report_cross(types, multis, tiles, cliloc)
    except FormatError as error:
        print(f"FORMAT ERROR: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
