#!/usr/bin/env python3
"""Replay the Enhanced client's gump paperdoll choice on a client's data and count what differs from the old rule.

Usage: python3 -I -B measure_ec_paperdoll_gump.py ENHANCED_CLIENT_DIR CLASSIC_CLIENT_DIR

Reads tileart.uop, string_dictionary.uop and Interface.uop of the Enhanced folder and tiledata.mul of the Classic
folder (read only; the Enhanced client ships no tiledata.mul, and tilemeta.uddp takes the tiledata anim from the
Classic one). For every item tiledata flags wearable it prints the gump the Enhanced client draws for a male and a
female wearer (results/uosa_paperdoll_gump.md section 2) and the gump of the old rule of dynamapper (tiledata anim
+ 50,000, a female wearer + 60,000 falling back to + 50,000; partial hue from the item's flags), and counts where
the number, the gump drawn, whether anything is drawn, and the partial hue differ. The tileart and UOP readers are
those of ../ghidra-cc-tileart/measure_cc_tileart.py.
"""

import collections
import importlib.util
import os
import struct
import sys
import zlib

HERE = os.path.dirname(os.path.abspath(__file__))
CC_TILEART = os.path.join(HERE, "..", "ghidra-cc-tileart", "measure_cc_tileart.py")

# UOPaperdoll::BuildImage 0x0050c790
KEY_APPEARANCE, KEY_PAPERDOLL = 8, 11
GUMP_MALE_BASE = 50000
GUMP_FEMALE_BASE = 60000
GUMP_ID_MASK = 0xFFFF  # RequestGumpArtTexture 0x0051af10 keys its cache by the low 16 bits
SKIPPED_GUMP_SIZE = (256, 256)  # DrawGumpLayer 0x00509b30 draws nothing from a 256x256 gump
FLAGS2_PARTIAL_HUE = 1 << 18  # HasFlags2(0x40000, 0) of the final definition
TILEDATA_PARTIAL_HUE = 1 << 18
DEFAULT_DEFINITION = 99999999
PLACEHOLDER_BASE = 10000000  # Appearance and Paperdoll values name the definition at this base + value
# SelectPaperdollBodyGumpForBody 0x0050d1c0 (living bodies; the ghosts take the same gumps)
BODY_GUMPS = {400: 12, 401: 13, 605: 14, 606: 15, 666: 666, 667: 665, 744: 12, 745: 13}
FEMALE_BODIES = {401, 606, 667, 745}

# Interface.uop gumps, as the converter names them (lib/udd-conv/src/gumps/ec_gumps.rs)
GUMP_NAMES = ("data/interface/default/textures/gumpart/%08d.tga", "data/interface/default/textures/gumpart/%08d.dds")
GUMP_ID_SPAN = 0x10000 * 2
TGA_SIZE_FORMAT = "<HH"
TGA_SIZE_OFFSET = 12
TGA_HEADER_SIZE = 18
DDS_MAGIC = b"DDS "
DDS_SIZE_FORMAT = "<II"  # height, width
DDS_SIZE_OFFSET = 12

MAX_LISTED = 16


def load_cc_tileart():
    spec = importlib.util.spec_from_file_location("measure_cc_tileart", CC_TILEART)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def read_tileart(m, folder):
    strings = m.read_string_dictionary(os.path.join(folder, "string_dictionary.uop"))
    ids = [*range(m.ENTRY_ID_SPAN), *range(m.PLACEHOLDER_BASE, m.PLACEHOLDER_BASE + m.ENTRY_ID_SPAN),
           DEFAULT_DEFINITION]
    names = {m.hashlittle2(m.ENTRY_NAME % i): i for i in ids}
    records = {}
    for name_hash, raw in m.uop_payloads(os.path.join(folder, "tileart.uop")):
        if name_hash in names:
            records[names[name_hash]] = m.parse_record(raw, strings)
    return records


def uop_entries(m, path):
    """(name hash, payload or None) of every entry of a UOP; None for a payload of another compression than none or
    zlib (EC fp Interface.uop packs its gumps with zlib + BWT, which this script does not undo)."""
    with open(path, "rb") as f:
        magic, _version, _misc, start, _block_size, _count = struct.unpack(
            m.UOP_HEADER_FORMAT, f.read(struct.calcsize(m.UOP_HEADER_FORMAT)))
        if magic != m.UOP_MAGIC:
            raise m.FormatError(f"{path}: not a UOP file")
        entry_size = struct.calcsize(m.UOP_ENTRY_FORMAT)
        block = start
        while block:
            f.seek(block)
            count, block_next = struct.unpack(m.UOP_BLOCK_HEADER_FORMAT,
                                              f.read(struct.calcsize(m.UOP_BLOCK_HEADER_FORMAT)))
            table = f.read(entry_size * count)
            for i in range(count):
                offset, header_len, compressed, _decompressed, name_hash, _adler, flag = struct.unpack_from(
                    m.UOP_ENTRY_FORMAT, table, i * entry_size)
                if offset == 0:
                    continue
                raw = None
                if flag in (m.UOP_COMPRESSION_NONE, m.UOP_COMPRESSION_ZLIB):
                    f.seek(offset + header_len)
                    raw = f.read(compressed)
                    if flag == m.UOP_COMPRESSION_ZLIB:
                        raw = zlib.decompress(raw)
                yield name_hash, raw
            block = block_next


def read_gump_sizes(m, folder):
    """Gump id -> (width, height) of every gump Interface.uop holds; (0, 0) for one without a TGA or DDS header,
    None for one whose payload this script cannot decompress."""
    names = {}
    for i in range(GUMP_ID_SPAN):
        for name in GUMP_NAMES:
            names[m.hashlittle2(name % i)] = i
    sizes = {}
    for name_hash, raw in uop_entries(m, os.path.join(folder, "Interface.uop")):
        if name_hash not in names:
            continue
        if raw is None:
            size = None
        elif raw[:4] == DDS_MAGIC:
            height, width = struct.unpack_from(DDS_SIZE_FORMAT, raw, DDS_SIZE_OFFSET)
            size = (width, height)
        elif len(raw) >= TGA_HEADER_SIZE:
            size = struct.unpack_from(TGA_SIZE_FORMAT, raw, TGA_SIZE_OFFSET)
        else:
            size = (0, 0)
        sizes.setdefault(names[name_hash], size)
    return sizes


def definition_number(records, tile_id):
    """The number and final definition BuildImage reaches for an item (equipconv.csv ships nowhere, so its map is
    empty): props2 Paperdoll, else props2 Appearance, each naming the definition at 10,000,000 + value, else the
    item's own number and definition. Returns (number, definition or None)."""
    definition = records.get(tile_id)
    if definition is None:
        return tile_id, None
    for key in (KEY_PAPERDOLL, KEY_APPEARANCE):
        value = definition["props2"].get(key)
        if value is not None:
            number = value - PLACEHOLDER_BASE if value > PLACEHOLDER_BASE else value
            return number, records.get(value + PLACEHOLDER_BASE)
    return tile_id, definition


def drawable(sizes, gump, mask=GUMP_ID_MASK):
    """Whether DrawGumpLayer draws gump `gump & mask`: it exists and is not 256x256."""
    gump &= mask
    return gump in sizes and sizes[gump] != SKIPPED_GUMP_SIZE


def enhanced_gump(sizes, number, female):
    """The gump DrawGumpLayer draws, or None: female + 60,000 first, then + 50,000."""
    if female and drawable(sizes, number + GUMP_FEMALE_BASE):
        return (number + GUMP_FEMALE_BASE) & GUMP_ID_MASK
    gump = number + GUMP_MALE_BASE
    return gump & GUMP_ID_MASK if drawable(sizes, gump) else None


def old_gump(sizes, anim, female):
    """dynamapper's old Enhanced rule: the profile offsets on the tiledata anim, the female gump falling back to the
    male one; the 256x256 skip as compose_paperdoll applies it. Its gump ids are not masked."""
    if anim == 0:
        return None
    if female and drawable(sizes, anim + GUMP_FEMALE_BASE, mask=-1):
        return anim + GUMP_FEMALE_BASE
    gump = anim + GUMP_MALE_BASE
    return gump if drawable(sizes, gump, mask=-1) else None


def listed(values):
    values = sorted(values)
    shown = ", ".join(str(v) for v in values[:MAX_LISTED])
    return shown + (f", ... ({len(values)})" if len(values) > MAX_LISTED else "")


def main():
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    ec_folder, cc_folder = sys.argv[1:]
    m = load_cc_tileart()
    records = read_tileart(m, ec_folder)
    sizes = read_gump_sizes(m, ec_folder)
    statics, _ = m.read_tiledata(os.path.join(cc_folder, "tiledata.mul"))
    limit = min(len(statics), m.ENTRY_ID_SPAN)
    wearables = [i for i in range(limit) if statics[i]["flags"] & m.WEARABLE_FLAG]
    print(f"tileart records {len(records)}; Interface.uop gumps {len(sizes)}; wearable tiledata items "
          f"{len(wearables)}")

    no_record = [i for i in wearables if i not in records]
    by_key = collections.Counter()
    for i in wearables:
        rec = records.get(i)
        if rec is None:
            continue
        key = next((k for k in (KEY_PAPERDOLL, KEY_APPEARANCE) if k in rec["props2"]), None)
        by_key[key] += 1
    print(f"no item record (nothing drawn): {len(no_record)} [{listed(no_record)}]")
    print(f"number from props2 Paperdoll {by_key[KEY_PAPERDOLL]}, props2 Appearance {by_key[KEY_APPEARANCE]}, "
          f"own tile id {by_key[None]}")
    props1_only = [i for i in wearables if i in records and KEY_APPEARANCE not in records[i]["props2"]
                   and KEY_APPEARANCE in records[i]["props1"]]
    print(f"props1 Appearance without props2 Appearance (props1 not read): {len(props1_only)} "
          f"[{listed(props1_only)}]")
    own_number = [i for i in wearables if i in records and
                  not any(k in records[i]["props2"] for k in (KEY_PAPERDOLL, KEY_APPEARANCE))]
    print(f"  own tile id items: [{listed(own_number)}]")

    results = {}
    for i in wearables:
        number, definition = definition_number(records, i)
        results[i] = (number, definition)
    no_final = [i for i in wearables if i in records and results[i][1] is None]
    print(f"props2 value whose 10,000,000 + value record is missing (nothing drawn): {len(no_final)} "
          f"[{listed(no_final)}]")
    number_differs = [i for i in wearables if results[i][0] != statics[i]["anim"]]
    print(f"number != tiledata anim: {len(number_differs)} [{listed(number_differs)}]")
    print(f"  pairs (item: tiledata anim -> number): "
          f"{', '.join(f'{i}: {statics[i]['anim']}->{results[i][0]}' for i in sorted(number_differs)[:MAX_LISTED])}")

    partial_differs = []
    for i in wearables:
        number, definition = results[i]
        if definition is None:
            continue
        ec_partial = bool(definition["flags2"] & FLAGS2_PARTIAL_HUE)
        old_partial = bool(statics[i]["flags"] & TILEDATA_PARTIAL_HUE)
        if ec_partial != old_partial:
            partial_differs.append(i)
    print(f"partial hue (final definition flags2) != tiledata partial hue, items drawn: {len(partial_differs)} "
          f"[{listed(partial_differs)}]")
    item_partial_differs = [i for i in wearables if results[i][1] is not None and i in records
                            and bool(results[i][1]["flags2"] & FLAGS2_PARTIAL_HUE)
                            != bool(records[i]["flags2"] & FLAGS2_PARTIAL_HUE)]
    print(f"  final definition flags2 != the item record's own flags2: {len(item_partial_differs)} "
          f"[{listed(item_partial_differs)}]")

    skipped = sorted(g for g, s in sizes.items() if s == SKIPPED_GUMP_SIZE)
    print(f"256x256 gumps: {len(skipped)} [{listed(skipped)}]")
    for female, label in ((False, "male"), (True, "female")):
        ec = {}
        old = {}
        for i in wearables:
            number, definition = results[i]
            ec[i] = enhanced_gump(sizes, number, female) if definition is not None else None
            old[i] = old_gump(sizes, statics[i]["anim"], female)
        gump_differs = [i for i in wearables if ec[i] != old[i]]
        ec_only = [i for i in gump_differs if old[i] is None]
        old_only = [i for i in gump_differs if ec[i] is None]
        both = [i for i in gump_differs if ec[i] is not None and old[i] is not None]
        wraps = [i for i in wearables if results[i][1] is not None and female
                 and results[i][0] + GUMP_FEMALE_BASE > GUMP_ID_MASK]
        drawn = sum(1 for i in wearables if ec[i] is not None)
        print(f"{label}: drawn by the client {drawn}, by the old rule {sum(1 for i in wearables if old[i])}; "
              f"gump differs {len(gump_differs)}: client only {len(ec_only)} [{listed(ec_only)}], old rule only "
              f"{len(old_only)} [{listed(old_only)}], other gump {len(both)} [{listed(both)}]")
        print(f"  other gump pairs (item: old -> client): "
              f"{', '.join(f'{i}: {old[i]}->{ec[i]}' for i in sorted(both)[:MAX_LISTED])}")
        if female:
            print(f"  female number + 60,000 past 65,535 (wraps to the low 16 bits): {len(wraps)} "
                  f"[{listed(wraps)}]; of them drawn from the wrapped id: "
                  f"{sum(1 for i in wraps if drawable(sizes, results[i][0] + GUMP_FEMALE_BASE))}")

    for body, gump in sorted(BODY_GUMPS.items()):
        size = sizes.get(gump)
        print(f"body {body} ({'female' if body in FEMALE_BODIES else 'male'}): body gump {gump} size {size}")
    body_sizes = {sizes.get(g) for g in BODY_GUMPS.values()}
    item_sizes = collections.Counter(sizes.get(g) for i in wearables
                                     for g in [enhanced_gump(sizes, results[i][0], False)]
                                     if g is not None and results[i][1] is not None)
    print(f"item gump sizes (male): {item_sizes.most_common(8)}; body gump sizes {body_sizes}")


if __name__ == "__main__":
    main()
