#!/usr/bin/env python3
"""Decode Enhanced Client AnimationSequence.uop and AnimationDefinition.uop the way UOSA.exe reads them.

Read-only verification of the layouts documented in results/uosa_animation.md. Every entry must be consumed
to its exact last byte. With --frames, the AnimationFrame*.uop headers are cross-checked against the
sequences (frame counts, action ids and 1-based frame numbers).

Usage:
    python3 -I verify_animation.py --sequence ANIMATION_SEQUENCE_UOP [--definition ANIMATION_DEFINITION_UOP]
                                   [--frames ENHANCED_CLIENT_DIR] [--dump BODY]
"""

import argparse
import collections
import glob
import os
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

# Entry names (asset ids). Definitions of mobiles use 10000000 + body.
SEQUENCE_NAME = "build/animationsequence/%08d.bin"
DEFINITION_NAME = "build/animationdefinition/%08d.bin"
FRAME_NAME = "build/animationframe/%06d/%02d.bin"
MAX_SEQUENCE_ID = 100000
MAX_ITEM_ID = 0x10000
MOBILE_DEFINITION_BASE = 10000000
MAX_BODY = 5000
MAX_ACTION = 100
FRAME_FILE_GLOB = "AnimationFrame*.uop"

# Jenkins lookup3 hashlittle2
HASH_SEED = 0xDEADBEEF
HASH_BLOCK = 12
WORD_MASK = 0xFFFFFFFF
WORD_BITS = 32

# AnimationSequence
DIRECTIONS = 8
STORED_DIRECTIONS = 5  # directions 3..7; 0..2 are mirrors of 6..4
FIRST_STORED_DIRECTION = 3
MIRROR_SUM = 6
FIRST_BASE_FRAME = 1
NO_ALIAS = -1
NO_PARENT = -1
UNSET_BYTE_OFFSET = -128
UNSET_SHORT_OFFSET = -0x8000
ANY_SUB_TYPE = 0xFF
MISSING_ACTION_FPS = 16.0

# AnimationFrame header and frame entry
FRAME_SIGNATURE = b"AMOU"
FRAME_HEADER_FORMAT = "<4sIIIhhhhIIII"
FRAME_ENTRY_FORMAT = "<HHhhhhI"
FRAME_ENTRY_SIZE = struct.calcsize(FRAME_ENTRY_FORMAT)

# AnimationDefinition
NO_NEXT_SET = -1

# Effect definitions (see ../ghidra-effects/results/uosa_effects.md)
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
    EFFECT_PARTICLE: "Particle", EFFECT_SOUND: "Sound", EFFECT_ROUND_ROBIN: "RoundRobin",
    EFFECT_POSITION_OFFSET: "PositionOffset", EFFECT_WAIT: "Wait", EFFECT_LOD: "LOD",
    EFFECT_PROPERTY_FLAG: "PropertyFlag", EFFECT_EARTHQUAKE: "Earthquake", EFFECT_RANDOM: "Random",
    EFFECT_USER_PREFERENCE: "UserPreference",
}
PARTICLE_FORMAT = "<I4fBfBBB"
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


def uop_entries(path, read_payload=True, payload_prefix=None):
    """Yield (hash, payload) for every non-empty entry; payload_prefix limits the bytes decompressed."""
    with open(path, "rb") as f:
        magic, _version, _misc, start, _block_size, _count = struct.unpack(
            UOP_HEADER_FORMAT, f.read(struct.calcsize(UOP_HEADER_FORMAT)))
        if magic != UOP_MAGIC:
            raise FormatError(f"{path}: not a UOP file")
        entry_size = struct.calcsize(UOP_ENTRY_FORMAT)
        block = start
        while block:
            f.seek(block)
            count, block_next = struct.unpack(UOP_BLOCK_HEADER_FORMAT, f.read(struct.calcsize(UOP_BLOCK_HEADER_FORMAT)))
            table = f.read(entry_size * count)
            for i in range(count):
                offset, header_len, compressed, decompressed, name_hash, _adler, flag = struct.unpack_from(
                    UOP_ENTRY_FORMAT, table, i * entry_size)
                if offset == 0:
                    continue
                if not read_payload:
                    yield name_hash, None
                    continue
                f.seek(offset + header_len)
                raw = f.read(compressed)
                if flag == UOP_COMPRESSION_ZLIB:
                    raw = zlib.decompress(raw)
                elif flag != UOP_COMPRESSION_NONE:
                    raise FormatError(f"{path}: unsupported compression {flag}")
                if len(raw) != decompressed:
                    raise FormatError(f"{path}: entry {name_hash:#x} size {len(raw)} != {decompressed}")
                yield name_hash, raw
            block = block_next


# ---- effects (subset of verify_effects.py: decoded, values not collected) ----

def read_effect_list(r, counts):
    return [read_effect(r, counts) for _ in range(r.take("<I"))]


def read_effect_map(r, counts):
    return [(r.take("<I"), read_effect_list(r, counts)) for _ in range(r.take("<I"))]


def read_effect(r, counts):
    key = r.take("<I")
    if key not in EFFECT_NAMES:
        raise FormatError(f"{r.label}: unknown effect type key {key}")
    counts[EFFECT_NAMES[key]] += 1
    if key == EFFECT_PARTICLE:
        return ("Particle", r.take(PARTICLE_FORMAT)[0])
    if key == EFFECT_SOUND:
        return ("Sound",) + r.take("<If")
    if key == EFFECT_ROUND_ROBIN:
        return ("RoundRobin", read_effect_list(r, counts))
    if key == EFFECT_POSITION_OFFSET:
        return ("PositionOffset", [r.take("<bb") for _ in range(DIRECTIONS)])
    if key == EFFECT_WAIT:
        effects = read_effect_list(r, counts)
        return ("Wait", effects) + r.take("<IIB")
    if key == EFFECT_LOD:
        return ("LOD", read_effect_map(r, counts))
    if key == EFFECT_PROPERTY_FLAG:
        mode, mask, prop = r.take("<BQI")
        return ("PropertyFlag", mode, mask, prop, read_effect_list(r, counts), read_effect_map(r, counts))
    if key == EFFECT_EARTHQUAKE:
        return ("Earthquake",) + r.take("<3d")
    if key == EFFECT_RANDOM:
        return ("Random", [(r.take("<f"), read_effect_list(r, counts)) for _ in range(r.take("<I"))])
    mask = r.take("<I")
    return ("UserPreference", mask, read_effect_list(r, counts))


# ---- AnimationSequence ----

def read_direction_offsets(r):
    byte_pairs = [r.take("<bb") for _ in range(DIRECTIONS)]
    short_pairs = [r.take("<hh") for _ in range(DIRECTIONS)]
    return byte_pairs, short_pairs


def parse_sequence(data, label, effect_counts):
    r = Reader(data, label)
    seq = {"id": r.take("<I")}
    seq["offsets"], seq["extra"] = read_direction_offsets(r)
    actions = []
    for _ in range(r.take("<I")):
        action, frame_count, alias, fps = r.take("<IIif")
        offsets, extra = read_direction_offsets(r)
        frame_effects = [(r.take("<I"), read_effect_list(r, effect_counts)) for _ in range(r.take("<I"))]
        skipped = [r.take("<I") for _ in range(r.take("<I"))]
        actions.append(dict(action=action, frame_count=frame_count, alias=alias, fps=fps, offsets=offsets,
                            extra=extra, frame_effects=frame_effects, skipped=skipped))
    groups = []
    for _ in range(r.take("<I")):
        key, parent, default = r.take("<bbI")
        transitions = [r.take("<bI") for _ in range(r.take("<I"))]
        moves = [r.take("<BII") for _ in range(r.take("<I"))]
        typed = []
        for _ in range(r.take("<I")):
            anim_type, sub_type, count = r.take("<bBI")
            typed.append((anim_type, sub_type, [r.take("<I") for _ in range(count)]))
        groups.append(dict(key=key, parent=parent, default=default, transitions=transitions, moves=moves,
                           typed=typed))
    if r.remaining():
        raise FormatError(f"{label}: {r.remaining()} trailing bytes")
    seq["actions"] = actions
    seq["groups"] = groups
    # Base frames as UOAnimationSequence::NextBaseFrame assigns them: running 1-based index in file order.
    next_base = FIRST_BASE_FRAME
    for a in actions:
        if a["alias"] == NO_ALIAS:
            a["base"] = next_base
            next_base += STORED_DIRECTIONS * a["frame_count"]
    return seq


def direction_frames(action, direction):
    """(first global frame, count, mirrored) as UOAnimationSequence::GetDirectionFrames computes them."""
    mirrored = direction < FIRST_STORED_DIRECTION
    if mirrored:
        direction = MIRROR_SUM - direction
    return action["base"] + (direction - FIRST_STORED_DIRECTION) * action["frame_count"], action["frame_count"], mirrored


def verify_sequences(path, dump):
    names = {hashlittle2(SEQUENCE_NAME % i): i for i in range(MAX_SEQUENCE_ID)}
    effect_counts = collections.Counter()
    stats = collections.defaultdict(collections.Counter)
    notes = []
    sequences = {}
    for name_hash, data in uop_entries(path):
        name = names.get(name_hash)
        if name is None:
            raise FormatError(f"{path}: entry {name_hash:#x} has no {SEQUENCE_NAME} name below {MAX_SEQUENCE_ID}")
        seq = parse_sequence(data, f"sequence {name}", effect_counts)
        sequences[name] = seq
        if seq["id"] != name:
            notes.append(f"sequence {name} stores id {seq['id']} (frames are keyed by the stored id)")
        stats["header offsets set"][any(p != (0, 0) for p in seq["offsets"] + seq["extra"])] += 1
        stats["action count"][len(seq["actions"])] += 1
        by_id = {}
        for a in seq["actions"]:
            if a["action"] in by_id:
                stats["duplicate action ids"][name] += 1
            by_id[a["action"]] = a
        for a in seq["actions"]:
            stats["alias"][a["alias"]] += 1
            if a["alias"] == NO_ALIAS:
                stats["frame count"][a["frame_count"]] += 1
                stats["fps"][a["fps"]] += 1
            else:
                stats["alias record frame count/fps"][(a["frame_count"], a["fps"])] += 1
                target = by_id.get(a["alias"])
                stats["alias target"]["missing" if target is None else
                                      "alias of alias (client logs, falls back to lowest plain action)" if target["alias"] != NO_ALIAS
                                      else "plain"] += 1
            stats["byte offsets set"][any(p != (UNSET_BYTE_OFFSET, UNSET_BYTE_OFFSET) for p in a["offsets"])] += 1
            stats["short offsets"][tuple(sorted(set(a["extra"])))] += 1
            stats["frame effect count"][len(a["frame_effects"])] += 1
            for frame, _effects in a["frame_effects"]:
                stats["frame effect key < frame count"][frame < a["frame_count"]] += 1
            stats["skipped frames"][tuple(a["skipped"])] += 1
        keys = {g["key"] for g in seq["groups"]}
        stats["group count"][len(seq["groups"])] += 1
        for g in seq["groups"]:
            stats["group key"][g["key"]] += 1
            stats["group parent"]["none" if g["parent"] == NO_PARENT else
                                  "present" if g["parent"] in keys else "missing"] += 1
            for key, action in g["transitions"]:
                stats["transition (group, key, action)"][(g["key"], key, action)] += 1
            for move, action, alternate in g["moves"]:
                stats["move key"][move] += 1
                stats["move alternate == action"][alternate == action] += 1
            for anim_type, sub_type, choices in g["typed"]:
                stats["typed (type, sub)"][(anim_type, "any" if sub_type == ANY_SUB_TYPE else sub_type)] += 1
                stats["typed choices"][len(choices)] += 1
                for choice in choices:
                    stats["typed action exists"][choice in by_id] += 1
    print(f"AnimationSequence: {len(sequences)} entries, all consumed")
    print_stats(stats)
    print("  effects:", dict(effect_counts))
    for note in notes:
        print("  note:", note)
    if dump is not None and dump in sequences:
        dump_sequence(dump, sequences[dump])
    return sequences


def dump_sequence(name, seq):
    print(f"sequence {name} (stored id {seq['id']})")
    for a in seq["actions"]:
        if a["alias"] != NO_ALIAS:
            print(f"  action {a['action']:3}: alias of {a['alias']}")
            continue
        dirs = [direction_frames(a, d) for d in range(DIRECTIONS)]
        print(f"  action {a['action']:3}: {a['frame_count']} frames @ {a['fps']} fps, base {a['base']},"
              f" dir 0..7 first frames {[d[0] for d in dirs]}, effects at {[k for k, _ in a['frame_effects']]},"
              f" skipped {a['skipped']}")
    for g in seq["groups"]:
        print(f"  group {g['key']} parent {g['parent']} default {g['default']}: transitions {g['transitions']},"
              f" moves {g['moves']}, typed {g['typed']}")


def verify_frames(directory, sequences):
    names = {}
    for body in range(MAX_BODY):
        for action in range(MAX_ACTION):
            names[hashlittle2(FRAME_NAME % (body, action))] = (body, action)
    frames = {}
    unnamed = 0
    for path in sorted(glob.glob(os.path.join(directory, FRAME_FILE_GLOB))):
        for name_hash, data in uop_entries(path):
            key = names.get(name_hash)
            if key is None:
                unnamed += 1
                continue
            signature, _version, _size, animation_id, *_bounds, _colors, _colors_offset, count, offset = \
                struct.unpack_from(FRAME_HEADER_FORMAT, data, 0)
            if signature != FRAME_SIGNATURE:
                raise FormatError(f"{path}: {key} signature {signature!r}")
            entries = [struct.unpack_from(FRAME_ENTRY_FORMAT, data, offset + i * FRAME_ENTRY_SIZE)[:2]
                       for i in range(count)]
            frames[key] = (animation_id, count, entries)
    stats = collections.defaultdict(collections.Counter)
    for name, seq in sequences.items():
        for a in seq["actions"]:
            if a["alias"] != NO_ALIAS:
                continue
            entry = frames.get((seq["id"], a["action"]))
            if entry is None:
                stats["frame file"]["missing"] += 1
                continue
            animation_id, count, entries = entry
            stats["frame file"]["present"] += 1
            stats["header animation id == stored sequence id"][animation_id == seq["id"]] += 1
            stats["frame count == 5 * sequence frame count"][count == STORED_DIRECTIONS * a["frame_count"]] += 1
            stats["entry action == action"][all(e[0] == a["action"] for e in entries)] += 1
            stats["entry frame numbers == 1..count"][[e[1] for e in entries] == list(range(1, count + 1))] += 1
    bodies = {k[0] for k in frames}
    stored = {s["id"] for s in sequences.values()}
    print(f"AnimationFrame: {len(frames)} named entries, {unnamed} unnamed, {len(bodies)} bodies,"
          f" {len(bodies - stored)} bodies without a sequence")
    print_stats(stats)


# ---- AnimationDefinition ----

def verify_definitions(path):
    ids = list(range(MAX_ITEM_ID)) + [MOBILE_DEFINITION_BASE + b for b in range(MAX_ITEM_ID)]
    names = {hashlittle2(DEFINITION_NAME % i): i for i in ids}
    effect_counts = collections.Counter()
    stats = collections.defaultdict(collections.Counter)
    notes = []
    count = 0
    for name_hash, data in uop_entries(path):
        name = names.get(name_hash)
        if name is None:
            raise FormatError(f"{path}: entry {name_hash:#x} has no {DEFINITION_NAME} name")
        r = Reader(data, f"definition {name}")
        stored, start, second, value = r.take("<IIIf")
        sets = {}
        for _ in range(r.take("<I")):
            key, next_key, interval, frame_count = r.take("<IifI")
            frames = []
            for _ in range(frame_count):
                tile = r.take("<I")
                effects = [read_effect(r, effect_counts) for _ in range(r.take("<B"))]
                frames.append((tile, effects))
            if key in sets:
                stats["duplicate set keys"][name] += 1
            sets[key] = (next_key, interval, frames)
        if r.remaining():
            raise FormatError(f"definition {name}: {r.remaining()} trailing bytes")
        count += 1
        if stored != name:
            notes.append(f"definition {name} stores id {stored}")
        stats["start/second keys exist (client requires)"][start in sets and second in sets] += 1
        stats["value >= 0 (client requires)"][value >= 0] += 1
        stats["(start, second, value)"][(start, second, value)] += 1
        stats["set count"][len(sets)] += 1
        for next_key, interval, frames in sets.values():
            stats["next"]["none" if next_key == NO_NEXT_SET else "present" if next_key in sets else "missing"] += 1
            stats["frame interval"][interval] += 1
            stats["frame count"][len(frames)] += 1
            stats["frames with effects"][sum(1 for _, e in frames if e)] += 1
            stats["kind"]["mobile" if name >= MOBILE_DEFINITION_BASE else "item"] += 1
    print(f"AnimationDefinition: {count} entries, all consumed")
    print_stats(stats)
    print("  effects:", dict(effect_counts))
    for note in notes:
        print("  note:", note)


def print_stats(stats):
    for field, counter in stats.items():
        shown = counter.most_common(MAX_SHOWN_VALUES)
        more = f" (+{len(counter) - len(shown)} more)" if len(counter) > len(shown) else ""
        print(f"  {field}: {shown}{more}")


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--sequence", required=True, help="path to AnimationSequence.uop")
    parser.add_argument("--definition", help="path to AnimationDefinition.uop")
    parser.add_argument("--frames", help="Enhanced Client folder holding AnimationFrame*.uop")
    parser.add_argument("--dump", type=int, help="print the decoded sequence with this entry id")
    args = parser.parse_args()
    try:
        sequences = verify_sequences(args.sequence, args.dump)
        if args.frames:
            verify_frames(args.frames, sequences)
        if args.definition:
            verify_definitions(args.definition)
    except FormatError as error:
        print(f"FORMAT ERROR: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
