#!/usr/bin/env python3
"""Decode the effect definitions of Enhanced Client tileart.uop and EffectDefinitionCollection.uop.

Read-only verification of the effect definition format documented in results/uosa_effects.md.
Every tileart record and every collection entry must be consumed to its exact last byte.

Usage:
    python3 -I verify_effects.py --tileart TILEART_UOP [--collection EFFECT_COLLECTION_UOP]
                                 [--strings STRING_DICTIONARY_UOP] [--dump]
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

# String dictionary: u64 stamp, u32 count, u16 max length; then count x {u16 length, bytes}
STRING_DICTIONARY_HEADER_FORMAT = "<QIH"

# tileart.uop record (version 4)
TILEART_VERSION = 4
TILEART_VERSION_SIZE = 2
TILEART_FIXED_SIZE = 0x7D
TILEART_TILE_ID_OFFSET = 6  # after u16 version and u32 name index
TILEART_PROPERTY_SIZE = 5  # u8 key + u32 value
TILEART_STACK_ALIAS_SIZE = 8  # u32 amount + u32 tile id
APPEARANCE_BODY_LIST = 0
APPEARANCE_GENDER = 1
APPEARANCE_ANIMATION_BODY_LIST = 2
APPEARANCE_PAIR_SIZE = 8
APPEARANCE_GENDER_SIZE = 5  # u8 gender + u32 appearance
SITTING_FIELD_COUNT = 6
SITTING_FIELD_SIZE = 4
RADAR_COLOR_SIZE = 4
SHADER_BLOCK_COUNT = 4
TEXTURE_ITEM_SIZE = 17  # u32 path, u8 source kind, f32 stretch, u32 sampler index, u32 uv set
SHADER_HEADER_SIZE = 1 + 4  # u8 field_04, u32 shader name index
SHADER_VALUE_SIZE = 4  # i32 technique / f32 tweak values

# Effect type keys (CanHandle of each factory registered in g_UOEffectDefinitionFactories)
EFFECT_PARTICLE = 0
EFFECT_SOUND = 1
EFFECT_ROUND_ROBIN = 2
EFFECT_POSITION_OFFSET = 7
EFFECT_WAIT = 10
EFFECT_LOD = 11
EFFECT_PROPERTY_FLAG = 12
EFFECT_EARTHQUAKE = 15
EFFECT_RANDOM = 16
EFFECT_USER_PREFERENCE = 17
EFFECT_NAMES = {
    EFFECT_PARTICLE: "Particle",
    EFFECT_SOUND: "Sound",
    EFFECT_ROUND_ROBIN: "RoundRobin",
    EFFECT_POSITION_OFFSET: "PositionOffset",
    EFFECT_WAIT: "Wait",
    EFFECT_LOD: "LOD",
    EFFECT_PROPERTY_FLAG: "PropertyFlag",
    EFFECT_EARTHQUAKE: "Earthquake",
    EFFECT_RANDOM: "Random",
    EFFECT_USER_PREFERENCE: "UserPreference",
}
EFFECT_TYPE_KEY_FORMAT = "<I"
POSITION_OFFSET_DIRECTIONS = 8
# Particle body: u32 name index, f32 x4, u8, f32, u8, u8, u8 (28 bytes). The client stores the bytes
# without checking them; loop and hued are tested as booleans.
PARTICLE_FORMAT = "<I4fBfBBB"
PARTICLE_FIELDS = ("offset_x_20", "offset_y_24", "offset_z_28", "scale_2c", "u8_30", "duration_34",
                   "loop_38", "u8_3c", "hued_40")
EARTHQUAKE_FIELDS = ("amplitude", "duration", "frequency")
BOOL_VALUES = (0, 1)
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


def read_uop(path):
    """Return a list of (hash, payload) for every non-empty entry."""
    with open(path, "rb") as f:
        blob = f.read()
    magic, _version, _misc, start, _block_size, _count = struct.unpack_from(UOP_HEADER_FORMAT, blob, 0)
    if magic != UOP_MAGIC:
        raise FormatError(f"{path}: not a UOP file")
    entries = []
    block = start
    entry_size = struct.calcsize(UOP_ENTRY_FORMAT)
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
            entries.append((name_hash, raw))
        block = block_next
    return entries


def read_string_dictionary(path):
    entries = read_uop(path)
    if len(entries) != 1:
        raise FormatError(f"{path}: expected one payload, found {len(entries)}")
    r = Reader(entries[0][1], path)
    _stamp, count, _max_len = r.take(STRING_DICTIONARY_HEADER_FORMAT)
    strings = []
    for _ in range(count):
        length = r.take("<H")
        strings.append(r.data[r.pos:r.pos + length].decode("utf-8", "replace"))
        r.skip(length)
    return strings


class Stats:
    def __init__(self, strings):
        self.strings = strings
        self.type_counts = collections.Counter()
        self.top_level = collections.Counter()
        self.values = collections.defaultdict(collections.Counter)
        self.max_depth = 0
        self.particle_names = set()

    def note(self, type_key, field, value):
        self.values[(EFFECT_NAMES[type_key], field)][value] += 1

    def name(self, index):
        if self.strings is None:
            return index
        return self.strings[index] if index < len(self.strings) else f"<bad index {index}>"


def read_bool(r, stats, type_key, field):
    value = r.take("<B")
    if value not in BOOL_VALUES:
        raise FormatError(f"{r.label}: {EFFECT_NAMES[type_key]}.{field} = {value}, client rejects values other than 0/1")
    stats.note(type_key, field, value)
    return value


def read_effect_list(r, stats, depth):
    count = r.take("<I")
    return [read_effect(r, stats, depth) for _ in range(count)]


def read_effect_map(r, stats, depth, type_key):
    count = r.take("<I")
    result = []
    for _ in range(count):
        key = r.take("<I")
        stats.note(type_key, "map_key", key)
        result.append((key, read_effect_list(r, stats, depth)))
    return result


def read_effect(r, stats, depth=0):
    stats.max_depth = max(stats.max_depth, depth)
    start = r.pos
    type_key = r.take(EFFECT_TYPE_KEY_FORMAT)
    if type_key not in EFFECT_NAMES:
        raise FormatError(f"{r.label}: unknown effect type key {type_key} at {start}")
    stats.type_counts[type_key] += 1
    child = depth + 1
    if type_key == EFFECT_PARTICLE:
        name, f0, f1, f2, f3, b0, f4, b1, b2, b3 = r.take(PARTICLE_FORMAT)
        stats.note(type_key, "name", stats.name(name))
        stats.particle_names.add(stats.name(name))
        for field, value in zip(PARTICLE_FIELDS, (f0, f1, f2, f3, b0, f4, b1, b2, b3)):
            stats.note(type_key, field, value)
        return (type_key, stats.name(name), f0, f1, f2, f3, b0, f4, b1, b2, b3)
    if type_key == EFFECT_SOUND:
        sound, value = r.take("<If")
        stats.note(type_key, "sound_id", sound)
        stats.note(type_key, "value_08", value)
        return (type_key, sound, value)
    if type_key == EFFECT_ROUND_ROBIN:
        return (type_key, read_effect_list(r, stats, child))
    if type_key == EFFECT_POSITION_OFFSET:
        offsets = [r.take("<bb") for _ in range(POSITION_OFFSET_DIRECTIONS)]
        stats.note(type_key, "offsets", tuple(offsets))
        return (type_key, offsets)
    if type_key == EFFECT_WAIT:
        effects = read_effect_list(r, stats, child)
        min_ms, max_ms = r.take("<II")
        aligned = read_bool(r, stats, type_key, "aligned")
        stats.note(type_key, "min_max_ms", (min_ms, max_ms))
        return (type_key, effects, min_ms, max_ms, aligned)
    if type_key == EFFECT_LOD:
        return (type_key, read_effect_map(r, stats, child, type_key))
    if type_key == EFFECT_PROPERTY_FLAG:
        mode = read_bool(r, stats, type_key, "mode")
        mask, prop = r.take("<QI")
        stats.note(type_key, "mask", hex(mask))
        stats.note(type_key, "property", prop)
        default = read_effect_list(r, stats, child)
        cases = read_effect_map(r, stats, child, type_key)
        return (type_key, mode, mask, prop, default, cases)
    if type_key == EFFECT_EARTHQUAKE:
        values = r.take("<3d")
        for field, value in zip(EARTHQUAKE_FIELDS, values):
            stats.note(type_key, field, value)
        return (type_key, values)
    if type_key == EFFECT_RANDOM:
        count = r.take("<I")
        choices = []
        for _ in range(count):
            weight = r.take("<f")
            stats.note(type_key, "weight", weight)
            choices.append((weight, read_effect_list(r, stats, child)))
        return (type_key, choices)
    if type_key == EFFECT_USER_PREFERENCE:
        mask = r.take("<I")
        stats.note(type_key, "mask", mask)
        return (type_key, mask, read_effect_list(r, stats, child))
    raise AssertionError(type_key)


def skip_shader_block(r):
    present = r.take("<B")
    if not present:
        return
    r.skip(SHADER_HEADER_SIZE)
    items = r.take("<B")
    r.skip(items * TEXTURE_ITEM_SIZE)
    r.skip(r.take("<I") * SHADER_VALUE_SIZE)  # technique values
    r.skip(r.take("<I") * SHADER_VALUE_SIZE)  # tweak values


def skip_tileart_known_part(r):
    version = r.take("<H")
    if version != TILEART_VERSION:
        raise FormatError(f"{r.label}: tileart version {version}")
    tile_id = struct.unpack_from("<I", r.data, TILEART_TILE_ID_OFFSET)[0]
    r.skip(TILEART_FIXED_SIZE - TILEART_VERSION_SIZE)
    r.skip(r.take("<B") * TILEART_PROPERTY_SIZE)
    r.skip(r.take("<B") * TILEART_PROPERTY_SIZE)
    r.skip(r.take("<I") * TILEART_STACK_ALIAS_SIZE)
    for _ in range(r.take("<I")):
        subtype = r.take("<B")
        if subtype in (APPEARANCE_BODY_LIST, APPEARANCE_ANIMATION_BODY_LIST):
            r.skip(r.take("<I") * APPEARANCE_PAIR_SIZE)
        elif subtype == APPEARANCE_GENDER:
            r.skip(APPEARANCE_GENDER_SIZE)
        else:
            raise FormatError(f"{r.label}: appearance subtype {subtype}")
    sitting = r.take("<B")
    if sitting not in BOOL_VALUES:
        raise FormatError(f"{r.label}: sitting flag {sitting}")
    if sitting:
        r.skip(SITTING_FIELD_COUNT * SITTING_FIELD_SIZE)
    r.skip(RADAR_COLOR_SIZE)
    for _ in range(SHADER_BLOCK_COUNT):
        skip_shader_block(r)
    return tile_id


def print_stats(title, stats):
    print(f"== {title}")
    print("effect definitions per type key (all nesting levels):")
    for key in sorted(stats.type_counts):
        print(f"  {key:3d} {EFFECT_NAMES[key]:15s} {stats.type_counts[key]}")
    print("top-level definitions per type key:")
    for key in sorted(stats.top_level):
        print(f"  {key:3d} {EFFECT_NAMES[key]:15s} {stats.top_level[key]}")
    print(f"maximum nesting depth: {stats.max_depth}")
    print("value distributions (value x count, most common first):")
    for (type_name, field), counter in sorted(stats.values.items()):
        shown = ", ".join(f"{v!r} x{c}" for v, c in counter.most_common(MAX_SHOWN_VALUES))
        more = "" if len(counter) <= MAX_SHOWN_VALUES else f", ... ({len(counter)} distinct)"
        print(f"  {type_name}.{field}: {shown}{more}")


def verify_tileart(path, strings, dump):
    stats = Stats(strings)
    entries = read_uop(path)
    per_record = collections.Counter()
    for name_hash, payload in entries:
        r = Reader(payload, f"tileart {name_hash:#018x}")
        tile_id = skip_tileart_known_part(r)
        count = r.take("<B")
        per_record[count] += 1
        for _ in range(count):
            effect = read_effect(r, stats)
            stats.top_level[effect[0]] += 1
            if dump:
                print(f"tile {tile_id}: {effect}")
        if r.remaining():
            raise FormatError(f"{r.label}: {r.remaining()} bytes left after the effects")
    print(f"tileart: {len(entries)} records, all consumed exactly")
    print("records per effect count: " + ", ".join(f"{k}: {v}" for k, v in sorted(per_record.items())))
    print_stats("tileart.uop effects", stats)
    return stats


def verify_collection(path, strings, dump):
    stats = Stats(strings)
    entries = read_uop(path)
    total = 0
    for name_hash, payload in entries:
        r = Reader(payload, f"collection {name_hash:#018x}")
        count = r.take("<I")
        for _ in range(count):
            effect = read_effect(r, stats)
            stats.top_level[effect[0]] += 1
            total += 1
            if dump:
                print(f"collection {name_hash:#018x}: {effect}")
        if r.remaining():
            raise FormatError(f"{r.label}: {r.remaining()} bytes left after {count} definitions")
    print(f"collection: {len(entries)} entries, {total} top-level definitions, all consumed exactly")
    print_stats("EffectDefinitionCollection.uop", stats)
    return stats


def print_name_overlap(tileart, collection):
    shared = tileart.particle_names & collection.particle_names
    print("== particle names")
    print(f"tileart: {len(tileart.particle_names)} distinct, collection: {len(collection.particle_names)} distinct,"
          f" in both: {len(shared)}")
    for name in sorted(shared, key=str):
        print(f"  {name}")


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--tileart", required=True, help="path to tileart.uop")
    parser.add_argument("--collection", help="path to EffectDefinitionCollection.uop")
    parser.add_argument("--strings", help="path to string_dictionary.uop (resolves particle names)")
    parser.add_argument("--dump", action="store_true", help="print every decoded top-level definition")
    args = parser.parse_args()
    strings = read_string_dictionary(args.strings) if args.strings else None
    try:
        tileart = verify_tileart(args.tileart, strings, args.dump)
        if args.collection:
            collection = verify_collection(args.collection, strings, args.dump)
            print_name_overlap(tileart, collection)
    except FormatError as error:
        print(f"FORMAT ERROR: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
