#!/usr/bin/env python3
"""Compare what the Classic client reads from tileart.uop with tiledata.mul.

Usage: python3 -I -B measure_cc_tileart.py CLASSIC_CLIENT_DIR

Reads tileart.uop, string_dictionary.uop and tiledata.mul of a Classic client folder (read only) and prints the
record counts and every comparison listed in results/cc_tileart.md section 5. With mobtypes.txt, Equipconv.def and
gumpartLegacyMUL.uop present it also replays the paperdoll gump choice (section 3.1) for each paperdoll body. The
record layout is the version 4 layout of ../ghidra-tileart/results/uosa_tileart.md, which the Classic parser
(ParseTileDefinition 0x00411c6e) reads in the same order. Effects end a record and are not decoded here: the
parse stops after the four shader blocks.
"""

import collections
import os
import struct
import sys
import zlib

# UOP container (as in ../ghidra-animation/verify_animation.py)
UOP_MAGIC = b"MYP\x00"
UOP_HEADER_FORMAT = "<4sIIQII"
UOP_BLOCK_HEADER_FORMAT = "<IQ"
UOP_ENTRY_FORMAT = "<qIIIQIh"
UOP_COMPRESSION_NONE = 0
UOP_COMPRESSION_ZLIB = 1

# Jenkins lookup3 hashlittle2 of the entry name (as in ../ghidra-animation/verify_animation.py)
HASH_SEED = 0xDEADBEEF
HASH_BLOCK = 12
WORD_MASK = 0xFFFFFFFF
WORD_BITS = 32

# tileart record, version 4. The client looks a definition up by its entry name, not by the stored tile id.
ENTRY_NAME = "build/tileart/%08d.bin"
ENTRY_ID_SPAN = 1 << 16
TILEART_VERSION = 4
FIXED_FORMAT = "<HIIBBffIIfIBfIffIQQi6i6i"
FIXED_SIZE = struct.calcsize(FIXED_FORMAT)
EXPECTED_FIXED_SIZE = 0x7D
SITTING_ABSENT, SITTING_PRESENT = 0, 1
SITTING_WORDS = 6
SHADER_BLOCKS = 4
SHADER_BLOCK_LEGACY = 1
FILTER_BY_BODY, FILTER_BY_GENDER, FILTER_ANIMATION_BY_BODY = 0, 1, 2
WATER_SHADER = "UOWaterShader"
WATER_PRIMARY_TEXTURE_POSITION = 1
PLACEHOLDER_BASE = 10000000
KEY_WEIGHT, KEY_HEIGHT, KEY_LAYER, KEY_APPEARANCE, KEY_GENDER = 0, 3, 6, 8, 10

# tiledata.mul (new format, 64-bit flags) as LoadTiledata 0x005b2ce0 reads it
LAND_COUNT = 0x4000
GROUP_SIZE = 32
GROUP_HEADER_SIZE = 4
LAND_FORMAT = "<QH20s"
STATIC_FORMAT = "<QBBIHHHB20s"
CLASSIC_STATIC_COUNT = 0x2FF940 // 0x30  # 65500: the client's loop bound
WEARABLE_FLAG = 0x400000

# artLegacyMUL.uop, the art.mul records in a UOP (static art id = item id + 0x4000)
ART_ENTRY_NAME = "build/artlegacymul/%08d.tga"
STATIC_ART_BASE = 0x4000
ART_ID_SPAN = 0x14000
ART_HEADER_FORMAT = "<IHH"
TILE_WIDTH = 44  # a land tile's width; most legacy offsets centre wider art on it

# Paperdoll gump choice (DrawPaperdollGumpLayers 0x0051a4a0, 0x004cde80, 0x005189e0)
PAPERDOLL_BODIES = (400, 401, 605, 606, 666, 667, 1253)  # what the body switch at 0x0051a6d4 can yield
FEMALE_PAPERDOLL_BODIES = (401, 606, 667)
MOBTYPES_UOP_FRAMES = 0x10000  # BodyUsesUopFrames 0x005978a0
GUMP_ITEM_BASE = 50000  # EquipConv 0x005624a0: no row, or a row with gump 0, gives anim + 50000
GUMP_FEMALE_OFFSET = 10000  # used when the wearer is female and that gump exists (0x00567d40)
EQUIPCONV_GUMP_FROM_CONVERTED = -1
GUMP_ENTRY_NAME = "build/gumpartlegacymul/%08d.tga"
GUMP_ID_SPAN = 0x10000 * 2

MAX_LISTED = 12


class FormatError(Exception):
    pass


def rotate(value, bits):
    return ((value << bits) | (value >> (WORD_BITS - bits))) & WORD_MASK


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


def uop_payloads(path):
    with open(path, "rb") as f:
        magic, _version, _misc, start, _block_size, _count = struct.unpack(
            UOP_HEADER_FORMAT, f.read(struct.calcsize(UOP_HEADER_FORMAT)))
        if magic != UOP_MAGIC:
            raise FormatError(f"{path}: not a UOP file")
        entry_size = struct.calcsize(UOP_ENTRY_FORMAT)
        block = start
        while block:
            f.seek(block)
            count, block_next = struct.unpack(UOP_BLOCK_HEADER_FORMAT,
                                              f.read(struct.calcsize(UOP_BLOCK_HEADER_FORMAT)))
            table = f.read(entry_size * count)
            for i in range(count):
                offset, header_len, compressed, decompressed, name_hash, _adler, flag = struct.unpack_from(
                    UOP_ENTRY_FORMAT, table, i * entry_size)
                if offset == 0:
                    continue
                f.seek(offset + header_len)
                raw = f.read(compressed)
                if flag == UOP_COMPRESSION_ZLIB:
                    raw = zlib.decompress(raw)
                elif flag != UOP_COMPRESSION_NONE:
                    raise FormatError(f"{path}: unsupported compression {flag}")
                if len(raw) != decompressed:
                    raise FormatError(f"{path}: entry size {len(raw)} != {decompressed}")
                yield name_hash, raw
            block = block_next


def read_string_dictionary(path):
    payloads = list(uop_payloads(path))
    if len(payloads) != 1:
        raise FormatError(f"{path}: expected one entry, found {len(payloads)}")
    data = payloads[0][1]
    _unk, _count, _max_length = struct.unpack_from("<QIH", data, 0)
    pos = struct.calcsize("<QIH")
    strings = []
    while pos + 2 <= len(data):
        (length,) = struct.unpack_from("<H", data, pos)
        pos += 2
        strings.append(data[pos:pos + length].decode("utf-8", "replace"))
        pos += length
    return strings


class Reader:
    def __init__(self, data):
        self.data = data
        self.pos = 0

    def take(self, fmt):
        values = struct.unpack_from(fmt, self.data, self.pos)
        self.pos += struct.calcsize(fmt)
        return values[0] if len(values) == 1 else values


def texture_id_from_path(path):
    """The id the converter derives (lib/uocf/src/utils/path.rs extract_texture_id_from_path)."""
    name = path.replace("/", "\\").rsplit("\\", 1)[-1]
    stem = name.split(".", 1)[0]
    for segment in (stem.split("_", 1)[0], stem):
        digits = ""
        for ch in segment:
            if ch.isdigit():
                digits += ch
            elif digits:
                break
        if digits:
            return int(digits)
    return None


def parse_record(data, strings):
    if FIXED_SIZE != EXPECTED_FIXED_SIZE:
        raise FormatError("fixed part layout")
    fixed = struct.unpack_from(FIXED_FORMAT, data, 0)
    (version, _name, tile_id, _world, _enhanced, _wscale, _lscale, _f60, _old_id, _rot, body_type, alt_anim,
     _lsc, _l70, _lx, _ly, _join, flags1, flags2, _facing) = fixed[:20]
    legacy_window = fixed[26:32]
    if version != TILEART_VERSION:
        raise FormatError(f"record version {version}")
    r = Reader(data)
    r.pos = FIXED_SIZE
    props1 = dict(r.take("<BI") for _ in range(r.take("<B")))
    props2 = dict(r.take("<BI") for _ in range(r.take("<B")))
    stack_aliases = [r.take("<II") for _ in range(r.take("<I"))]
    filters = []
    for _ in range(r.take("<I")):
        kind = r.take("<B")
        if kind in (FILTER_BY_BODY, FILTER_ANIMATION_BY_BODY):
            filters.append((kind, [r.take("<II") for _ in range(r.take("<I"))]))
        elif kind == FILTER_BY_GENDER:
            filters.append((kind, r.take("<BI")))
        else:
            raise FormatError(f"tile {tile_id}: filter kind {kind}")
    sitting_flag = r.take("<B")
    if sitting_flag not in (SITTING_ABSENT, SITTING_PRESENT):
        raise FormatError(f"tile {tile_id}: sitting flag {sitting_flag}")
    sitting = r.take("<" + "i" * SITTING_WORDS) if sitting_flag else None
    _radar = r.take("<4B")
    legacy_texture_id = None
    for index in range(SHADER_BLOCKS):
        if r.take("<B") == 0:
            continue
        _field04, shader = r.take("<BI")
        items = [r.take("<IBfII") for _ in range(r.take("<B"))]
        r.take("<" + "i" * r.take("<I"))
        r.take("<" + "f" * r.take("<I"))
        if index == SHADER_BLOCK_LEGACY:
            position = WATER_PRIMARY_TEXTURE_POSITION if strings[shader] == WATER_SHADER else 0
            if position < len(items):
                legacy_texture_id = texture_id_from_path(strings[items[position][0]])
    return dict(tile_id=tile_id, body_type=body_type, alt_anim=alt_anim, flags1=flags1, flags2=flags2,
                legacy_window=legacy_window, props1=props1, props2=props2, stack_aliases=stack_aliases,
                filters=filters, sitting=sitting, legacy_texture_id=legacy_texture_id)


def read_tiledata(path):
    data = open(path, "rb").read()
    land_size = struct.calcsize(LAND_FORMAT)
    static_size = struct.calcsize(STATIC_FORMAT)
    pos = 0
    for index in range(LAND_COUNT):
        if index % GROUP_SIZE == 0:
            pos += GROUP_HEADER_SIZE
        pos += land_size
    statics = []
    index = 0
    while pos + (GROUP_HEADER_SIZE if index % GROUP_SIZE == 0 else 0) + static_size <= len(data):
        if index % GROUP_SIZE == 0:
            pos += GROUP_HEADER_SIZE
        flags, weight, layer, _misc, anim, _hue, _light, height, name = struct.unpack_from(STATIC_FORMAT, data, pos)
        statics.append(dict(flags=flags, weight=weight, layer=layer, anim=anim, height=height,
                            name=name.split(b"\0", 1)[0].decode("latin-1")))
        pos += static_size
        index += 1
    return statics, len(data) - pos


def read_static_art_sizes(path):
    names = {hashlittle2(ART_ENTRY_NAME % i): i for i in range(STATIC_ART_BASE, ART_ID_SPAN)}
    sizes = {}
    for name_hash, raw in uop_payloads(path):
        if name_hash in names:
            _flag, width, height = struct.unpack_from(ART_HEADER_FORMAT, raw, 0)
            sizes[names[name_hash] - STATIC_ART_BASE] = (width, height)
    return sizes


def centring_offset(width):
    """The x offset that centres art wider than a land tile on it."""
    return -((width - TILE_WIDTH) // 2) if width > TILE_WIDTH else 0


def read_mobtype_flags(path):
    """Body -> flags column of mobtypes.txt (hex), as MobTypes_EnsureTable loads it."""
    flags = {}
    for line in open(path, encoding="latin-1"):
        fields = line.split("#", 1)[0].split()
        if len(fields) >= 3 and fields[0].isdigit():
            flags[int(fields[0])] = int(fields[2], 16)
    return flags


def read_equipconv(path):
    """(body, equipment id) -> (converted id, gump id) of Equipconv.def."""
    rows = {}
    for line in open(path, encoding="latin-1"):
        fields = line.split("#", 1)[0].split()
        if len(fields) >= 4:
            body, equip, converted, gump = (int(v) for v in fields[:4])
            rows[(body, equip)] = (converted, gump)
    return rows


def equipconv_gump(rows, body, anim):
    """EquipConv 0x005624a0."""
    row = rows.get((body, anim))
    if row is None or row[1] == 0:
        return anim + GUMP_ITEM_BASE
    converted, gump = row
    return converted + GUMP_ITEM_BASE if gump == EQUIPCONV_GUMP_FROM_CONVERTED else gump


def filter_key(placeholder):
    """What a body filter compares (filter Apply 0x0041717f / 0x00417287): property 8, else the stored id."""
    return placeholder["props1"].get(KEY_APPEARANCE, placeholder["tile_id"])


def get_appearance(tile, placeholder):
    """TileDefinition_GetAppearance +0xbc (0x00405bf1): property 8 or the stored id, then the first matching
    body (0) or gender (1) filter."""
    key = filter_key(placeholder)
    gender = placeholder["props1"].get(KEY_GENDER, 0)
    for kind, data in tile["filters"]:
        if kind == FILTER_BY_BODY:
            for k, value in data:
                if k == key:
                    return value
        elif kind == FILTER_BY_GENDER and data[0] == gender:
            return data[1]
    return tile["props1"].get(KEY_APPEARANCE, tile["tile_id"])


def get_animation_appearance(tile, placeholder):
    """TileDefinition_GetAnimationAppearance +0xc0 (0x00405ce4): the first matching animation filter (2), else 0."""
    key = filter_key(placeholder)
    for kind, data in tile["filters"]:
        if kind == FILTER_ANIMATION_BY_BODY:
            for k, value in data:
                if k == key:
                    return value
    return 0


def paperdoll(folder, items, placeholders, statics):
    """Replays the paperdoll gump choice per paperdoll body and counts where tileart changes the outcome."""
    paths = [os.path.join(folder, name) for name in ("mobtypes.txt", "Equipconv.def", "gumpartLegacyMUL.uop")]
    if not all(os.path.exists(path) for path in paths):
        return
    flags = read_mobtype_flags(paths[0])
    rows = read_equipconv(paths[1])
    gump_names = {hashlittle2(GUMP_ENTRY_NAME % i): i for i in range(GUMP_ID_SPAN)}
    gumps = {gump_names[h] for h, _ in uop_payloads(paths[2]) if h in gump_names}
    limit = min(len(statics), CLASSIC_STATIC_COUNT)
    wearables = [i for i in range(limit) if statics[i]["flags"] & WEARABLE_FLAG]
    anim_filtered = sorted(i for i, rec in items.items()
                           if any(k == FILTER_ANIMATION_BY_BODY for k, _ in rec["filters"]))
    print(f"paperdoll: {len(wearables)} wearable tiledata items (< {limit}); gumps present {len(gumps)}")
    for i in anim_filtered:
        keys = [pair for k, data in items[i]["filters"] if k == FILTER_ANIMATION_BY_BODY for pair in data]
        print(f"  animation filter on {i} (tiledata anim {statics[i]['anim']}, wearable "
              f"{bool(statics[i]['flags'] & WEARABLE_FLAG)}): {keys}")
    for body in PAPERDOLL_BODIES:
        placeholder = placeholders.get(PLACEHOLDER_BASE + body)
        uop = bool(flags.get(body, 0) & MOBTYPES_UOP_FRAMES)
        female = body in FEMALE_PAPERDOLL_BODIES
        switched, differ, gump_differ, visible, missing_def = [], [], [], 0, []
        for i in wearables:
            tile = items.get(i)
            tiledata_anim = statics[i]["anim"]
            if tile is None or placeholder is None:
                anim = 0 if uop else tiledata_anim
                missing_def.append(i)
            elif uop:
                anim = get_appearance(tile, placeholder)
            elif get_animation_appearance(tile, placeholder):
                anim = get_appearance(tile, placeholder)
                switched.append(i)
            else:
                anim = tiledata_anim
            gump = equipconv_gump(rows, body, anim)
            if female and gump + GUMP_FEMALE_OFFSET in gumps:
                gump += GUMP_FEMALE_OFFSET
            plain = equipconv_gump(rows, body, tiledata_anim)
            if female and plain + GUMP_FEMALE_OFFSET in gumps:
                plain += GUMP_FEMALE_OFFSET
            if anim != tiledata_anim:
                differ.append(i)
            if gump != plain:
                gump_differ.append(i)
                visible += gump in gumps or plain in gumps
        key = filter_key(placeholder) if placeholder else None
        print(f"body {body}: mobtypes 0x{flags.get(body, 0):x} {'UOP' if uop else 'MUL'} frames, placeholder key "
              f"{key}, gender {placeholder['props1'].get(KEY_GENDER) if placeholder else None}; "
              f"animation filter switches {len(switched)} [{listed(switched)}]; anim != tiledata anim "
              f"{len(differ)} [{listed(differ)}]; gump differs {len(gump_differ)} (either gump present: {visible}) "
              f"[{listed(gump_differ)}]; no definition {len(missing_def)}")


def listed(values):
    values = sorted(values)
    shown = ", ".join(str(v) for v in values[:MAX_LISTED])
    return shown + (f", ... ({len(values)})" if len(values) > MAX_LISTED else "")


def main():
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    folder = sys.argv[1]
    strings = read_string_dictionary(os.path.join(folder, "string_dictionary.uop"))
    names = {hashlittle2(ENTRY_NAME % i): i
             for i in [*range(ENTRY_ID_SPAN), *range(PLACEHOLDER_BASE, PLACEHOLDER_BASE + ENTRY_ID_SPAN)]}
    records = []
    unnamed = 0
    for name_hash, raw in uop_payloads(os.path.join(folder, "tileart.uop")):
        if name_hash not in names:
            unnamed += 1
            continue
        rec = parse_record(raw, strings)
        rec["entry_id"] = names[name_hash]
        records.append(rec)
    statics, trailing = read_tiledata(os.path.join(folder, "tiledata.mul"))
    by_id = {rec["entry_id"]: rec for rec in records}
    renamed = sorted((rec["entry_id"], rec["tile_id"]) for rec in records if rec["entry_id"] != rec["tile_id"])
    print(f"records named outside the looked-up ids: {unnamed}; storing another tile id than their name's: "
          f"{len(renamed)} {renamed}")
    items = {i: rec for i, rec in by_id.items() if i < PLACEHOLDER_BASE}
    placeholders = {i: rec for i, rec in by_id.items() if i >= PLACEHOLDER_BASE}

    def present(i):
        s = statics[i]
        return s["flags"] or s["name"] or s["height"] or s["anim"] or s["weight"]

    print(f"tileart records: {len(records)} (items {len(items)}, placeholders 10000000+ {len(placeholders)})")
    print(f"tiledata statics in file: {len(statics)} (trailing bytes {trailing}); read by the client: "
          f"{CLASSIC_STATIC_COUNT}")
    print(f"item ids: {min(items)}..{max(items)}")
    beyond_file = [i for i in items if i >= len(statics)]
    beyond_client = [i for i in items if i >= CLASSIC_STATIC_COUNT]
    empty = [i for i in items if i < len(statics) and not present(i)]
    missing_in_tileart = [i for i in range(min(len(statics), CLASSIC_STATIC_COUNT)) if present(i) and i not in items]
    print(f"tileart items past tiledata's end: {len(beyond_file)} [{listed(beyond_file)}]")
    print(f"tileart items at or past the client's 65500 bound: {len(beyond_client)} [{listed(beyond_client)}]")
    print(f"tileart items whose tiledata entry is empty: {len(empty)} [{listed(empty)}]")
    print(f"non-empty tiledata entries (< 65500) with no tileart record: {len(missing_in_tileart)} "
          f"[{listed(missing_in_tileart)}]")

    common = [i for i in items if i < len(statics)]
    flag_diff = [i for i in common if items[i]["flags1"] != statics[i]["flags"]]
    bit_counter = collections.Counter()
    for i in flag_diff:
        diff = items[i]["flags1"] ^ statics[i]["flags"]
        for bit in range(64):
            if diff >> bit & 1:
                side = "tileart" if items[i]["flags1"] >> bit & 1 else "tiledata"
                bit_counter[(bit, side)] += 1
    print(f"flags1 != tiledata flags: {len(flag_diff)} of {len(common)}")
    for (bit, side), n in sorted(bit_counter.items()):
        print(f"  bit {bit} (0x{1 << bit:x}) set only in {side}: {n}")

    for key, field, label in ((KEY_HEIGHT, "height", "height (props1 key 3)"),
                              (KEY_WEIGHT, "weight", "weight (props1 key 0)"),
                              (KEY_LAYER, "layer", "layer (props1 key 6)")):
        absent = [i for i in common if key not in items[i]["props1"] and statics[i][field]]
        differ = [i for i in common if key in items[i]["props1"] and items[i]["props1"][key] != statics[i][field]]
        print(f"{label}: differs {len(differ)} [{listed(differ)}]; key absent but tiledata non-zero {len(absent)}")

    appearance = [i for i in common if KEY_APPEARANCE in items[i]["props1"]]
    app_vs_anim = [i for i in appearance if items[i]["props1"][KEY_APPEARANCE] != statics[i]["anim"]]
    print(f"appearance (props1 key 8) present: {len(appearance)}; != tiledata anim: {len(app_vs_anim)} "
          f"[{listed(app_vs_anim)}]")
    filters = collections.Counter(kind for rec in items.values() for kind, _ in rec["filters"])
    anim_filters = [i for i, rec in items.items() if any(k == FILTER_ANIMATION_BY_BODY for k, _ in rec["filters"])]
    print(f"appearance filters on items: {dict(filters)}; items with an animation-appearance filter: "
          f"{len(anim_filters)} [{listed(anim_filters)}]")
    wear_anim = [i for i in common if statics[i]["flags"] & WEARABLE_FLAG and statics[i]["anim"]]
    print(f"wearable tiledata items with an anim id: {len(wear_anim)}")

    sitting = {i: rec["sitting"] for i, rec in items.items() if rec["sitting"]}
    print(f"items with sitting data: {len(sitting)}")
    for word in range(SITTING_WORDS):
        counter = collections.Counter(s[word] for s in sitting.values())
        print(f"  word {word}: {dict(counter.most_common())}")
    print(f"  sitting ids: {listed(sitting)}")

    alt = [i for i, rec in by_id.items() if rec["alt_anim"]]
    print(f"alternate animation set: {len(alt)} [{listed(alt)}]")
    body = collections.Counter(rec["body_type"] for rec in items.values())
    print(f"item body types: {dict(body.most_common(6))}")

    stack = [i for i, rec in items.items() if rec["stack_aliases"]]
    print(f"items with stack aliases: {len(stack)}")
    no_texture = [i for i in items if items[i]["legacy_texture_id"] is None]
    tex_diff = [i for i in items if items[i]["legacy_texture_id"] is not None and items[i]["legacy_texture_id"] != i]
    print(f"legacy (block 1) texture id missing: {len(no_texture)}; != tile id: {len(tex_diff)} [{listed(tex_diff)}]")
    if tex_diff:
        pairs = [f"{i}->{items[i]['legacy_texture_id']}" for i in sorted(tex_diff)[:MAX_LISTED]]
        print(f"  first pairs: {', '.join(pairs)}")
    offsets = [i for i in items if items[i]["legacy_window"][4] or items[i]["legacy_window"][5]]
    off_counter = collections.Counter((items[i]["legacy_window"][4], items[i]["legacy_window"][5]) for i in offsets)
    print(f"legacy window x/y offset non-zero: {len(offsets)}; most common {off_counter.most_common(8)}")

    paperdoll(folder, items, placeholders, statics)

    art_path = os.path.join(folder, "artLegacyMUL.uop")
    if not os.path.exists(art_path):
        return
    sizes = read_static_art_sizes(art_path)
    with_art = [i for i in items if i in sizes]
    print(f"static art records in artLegacyMUL.uop: {len(sizes)}; tileart items without art: "
          f"{len(items) - len(with_art)}")
    other_size = [i for i in with_art if items[i]["legacy_window"][:4] != (0, 0) + sizes[i]]
    same_size = [i for i in with_art if i not in other_size]
    centred = [i for i in same_size if items[i]["legacy_window"][4] == centring_offset(sizes[i][0])]
    y_offset = [i for i in with_art if items[i]["legacy_window"][5]]
    print(f"legacy window not 0,0,art width,art height: {len(other_size)} [{listed(other_size)}]")
    print(f"legacy x offset == -((art width - 44) // 2) for art wider than a tile, else 0: {len(centred)} of "
          f"{len(same_size)} with an art-sized window")
    print(f"legacy y offset non-zero: {len(y_offset)}; values "
          f"{collections.Counter(items[i]['legacy_window'][5] for i in y_offset).most_common()}")
    examples = [f"{i} art {sizes[i][0]}x{sizes[i][1]} window {items[i]['legacy_window']}" for i in sorted(other_size)[:4]]
    print(f"  e.g. {'; '.join(examples)}")
    own_art = [i for i in tex_diff if i in sizes]
    print(f"items whose legacy texture id differs but which have their own art record: {len(own_art)} "
          f"[{listed(own_art)}]")


if __name__ == "__main__":
    main()
