#!/usr/bin/env python3
"""Checks the data block headers of UOP packages against their entries' data_block_hash.

For every entry the header is the `data_block_length` bytes at `data_block_address`. Each header is parsed
into records (u16 type, u16 length, data; little-endian) and its hash is computed by its first record:
Adler-32 for a timestamp record (type 3), lookup3 hashlittle with initial value 0 for a signature record
(type 4). Prints one line per package and the totals; exits 1 on any mismatch.

Usage (stdlib only; run isolated):
    python3 -I verify_uop_headers.py PACKAGE_OR_DIR [...]
A directory stands for every *.uop file in it.
"""

import collections
import os
import struct
import sys
import zlib

# Package header: magic, version, misc, first block address, block size, file count (little-endian).
PACKAGE_HEADER = struct.Struct("<IIIQII")
# Block header: entry count, next block address.
BLOCK_HEADER = struct.Struct("<IQ")
# Entry: offset, header length, compressed size, decompressed size, name hash, data_block_hash, compression.
ENTRY = struct.Struct("<QIIIQIH")
# Data block header record prefix: type, length.
RECORD_PREFIX = struct.Struct("<HH")
# Signature record data prefix: key id, signature length (big-endian).
SIGNATURE_PREFIX = struct.Struct(">HH")
RECORD_TYPE_TIMESTAMP = 3
RECORD_TYPE_SIGNATURE = 4
# lookup3 constants.
MASK32 = 0xFFFFFFFF
LOOKUP3_INIT = 0xDEADBEEF
LOOKUP3_BLOCK = 12
PACKAGE_EXTENSION = ".uop"


def rot(value, count):
    return ((value << count) | (value >> (32 - count))) & MASK32


def hashlittle(data, initval=0):
    """Bob Jenkins' lookup3 hashlittle (the `c` output of hashlittle2 with pc = initval, pb = 0)."""
    length = len(data)
    a = b = c = (LOOKUP3_INIT + length + initval) & MASK32
    i = 0
    while length > LOOKUP3_BLOCK:
        a = (a + int.from_bytes(data[i:i + 4], "little")) & MASK32
        b = (b + int.from_bytes(data[i + 4:i + 8], "little")) & MASK32
        c = (c + int.from_bytes(data[i + 8:i + 12], "little")) & MASK32
        a = (a - c) & MASK32; a ^= rot(c, 4); c = (c + b) & MASK32
        b = (b - a) & MASK32; b ^= rot(a, 6); a = (a + c) & MASK32
        c = (c - b) & MASK32; c ^= rot(b, 8); b = (b + a) & MASK32
        a = (a - c) & MASK32; a ^= rot(c, 16); c = (c + b) & MASK32
        b = (b - a) & MASK32; b ^= rot(a, 19); a = (a + c) & MASK32
        c = (c - b) & MASK32; c ^= rot(b, 4); b = (b + a) & MASK32
        length -= LOOKUP3_BLOCK
        i += LOOKUP3_BLOCK
    if length == 0:
        return c
    tail = data[i:] + bytes(LOOKUP3_BLOCK - length)
    a = (a + int.from_bytes(tail[0:4], "little")) & MASK32
    b = (b + int.from_bytes(tail[4:8], "little")) & MASK32
    c = (c + int.from_bytes(tail[8:12], "little")) & MASK32
    c ^= b; c = (c - rot(b, 14)) & MASK32
    a ^= c; a = (a - rot(c, 11)) & MASK32
    b ^= a; b = (b - rot(a, 25)) & MASK32
    c ^= b; c = (c - rot(b, 16)) & MASK32
    a ^= c; a = (a - rot(c, 4)) & MASK32
    b ^= a; b = (b - rot(a, 14)) & MASK32
    c ^= b; c = (c - rot(b, 24)) & MASK32
    return c


def entries(data):
    """Yields (offset, header length, data_block_hash) of every entry with a nonzero offset."""
    _magic, version, _misc, block, _size, _count = PACKAGE_HEADER.unpack_from(data, 0)
    found = []
    while block:
        count, next_block = BLOCK_HEADER.unpack_from(data, block)
        position = block + BLOCK_HEADER.size
        for _ in range(count):
            offset, header_length, _cs, _ds, _name, stored_hash, _flag = ENTRY.unpack_from(data, position)
            position += ENTRY.size
            if offset:
                found.append((offset, header_length, stored_hash))
        block = next_block
    return version, found


def records(header):
    """Header records as (type, data); raises ValueError on a record running past the end."""
    out, position = [], 0
    while position < len(header):
        if position + RECORD_PREFIX.size > len(header):
            raise ValueError("truncated record prefix")
        record_type, length = RECORD_PREFIX.unpack_from(header, position)
        start = position + RECORD_PREFIX.size
        if start + length > len(header):
            raise ValueError("truncated record")
        out.append((record_type, header[start:start + length]))
        position = start + length
    return out


def expected_hash(header):
    parsed = records(header)
    if not parsed:
        return None
    if parsed[0][0] == RECORD_TYPE_TIMESTAMP:
        return zlib.adler32(header) & MASK32
    if parsed[0][0] == RECORD_TYPE_SIGNATURE:
        return hashlittle(header)
    return None


def describe(header):
    """Short header shape: record type, plus key id for a signature record."""
    parsed = records(header)
    if not parsed:
        return "none"
    record_type, data = parsed[0]
    if record_type == RECORD_TYPE_SIGNATURE and len(data) >= SIGNATURE_PREFIX.size:
        key_id, length = SIGNATURE_PREFIX.unpack_from(data, 0)
        if length == len(data) - SIGNATURE_PREFIX.size:
            return f"signature key {key_id}"
    return f"type {record_type}"


def package_paths(arguments):
    for argument in arguments:
        if os.path.isdir(argument):
            for name in sorted(os.listdir(argument)):
                if name.lower().endswith(PACKAGE_EXTENSION):
                    yield os.path.join(argument, name)
        else:
            yield argument


def main(arguments):
    if not arguments:
        print(__doc__)
        return 2
    total_matched = total_entries = 0
    for path in package_paths(arguments):
        with open(path, "rb") as package:
            data = package.read()
        version, found = entries(data)
        matched, shapes, lengths = 0, collections.Counter(), collections.Counter()
        for offset, header_length, stored_hash in found:
            header = data[offset:offset + header_length]
            shapes[describe(header)] += 1
            lengths[header_length] += 1
            matched += expected_hash(header) == stored_hash
        total_matched += matched
        total_entries += len(found)
        shape_text = ", ".join(f"{shape}: {count}" for shape, count in shapes.most_common())
        length_text = ", ".join(f"{length}: {count}" for length, count in sorted(lengths.items()))
        print(f"{os.path.basename(path)}: version {version}, {matched} of {len(found)} match; "
              f"headers {shape_text}; lengths {length_text}")
    print(f"total: {total_matched} of {total_entries} match")
    return 0 if total_matched == total_entries else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
