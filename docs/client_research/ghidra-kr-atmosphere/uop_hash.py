"""UOP entry hash: Bob Jenkins' lookup3 hashlittle2, as Mythic's UOP packages name their entries.

`uop_hash(name)` returns the 64-bit value (b << 32 | c) that names an entry. uoppackage-cli extracts entries the
string dictionary does not name as `0x<hash>.<ext>`; this finds them from their path, for example
`uop_hash("build/terraintexture/00000021.dds")`. From the shell, `python3 -I uop_hash.py PATH ...` prints the
hash of each path.
"""

MASK32 = 0xFFFFFFFF
HASH_SEED = 0xDEADBEEF
BLOCK_BYTES = 12
WORD_BYTES = 4
HIGH_SHIFT = 32
# lookup3 mix() and final() rotation amounts, in order.
MIX_ROTATIONS = (4, 6, 8, 16, 19, 4)
FINAL_ROTATIONS = (14, 11, 25, 16, 4, 14, 24)


def _rot(value, count):
    return ((value << count) | (value >> (HIGH_SHIFT - count))) & MASK32


def _word(data, index):
    return int.from_bytes(data[index * WORD_BYTES:(index + 1) * WORD_BYTES], "little")


def _mix(a, b, c):
    r = MIX_ROTATIONS
    a = (a - c) & MASK32; a ^= _rot(c, r[0]); c = (c + b) & MASK32
    b = (b - a) & MASK32; b ^= _rot(a, r[1]); a = (a + c) & MASK32
    c = (c - b) & MASK32; c ^= _rot(b, r[2]); b = (b + a) & MASK32
    a = (a - c) & MASK32; a ^= _rot(c, r[3]); c = (c + b) & MASK32
    b = (b - a) & MASK32; b ^= _rot(a, r[4]); a = (a + c) & MASK32
    c = (c - b) & MASK32; c ^= _rot(b, r[5]); b = (b + a) & MASK32
    return a, b, c


def _final(a, b, c):
    r = FINAL_ROTATIONS
    c ^= b; c = (c - _rot(b, r[0])) & MASK32
    a ^= c; a = (a - _rot(c, r[1])) & MASK32
    b ^= a; b = (b - _rot(a, r[2])) & MASK32
    c ^= b; c = (c - _rot(b, r[3])) & MASK32
    a ^= c; a = (a - _rot(c, r[4])) & MASK32
    b ^= a; b = (b - _rot(a, r[5])) & MASK32
    c ^= b; c = (c - _rot(b, r[6])) & MASK32
    return a, b, c


def uop_hash(name):
    data = name.encode("ascii")
    length = len(data)
    a = b = c = (HASH_SEED + length) & MASK32
    offset = 0
    while length - offset > BLOCK_BYTES:
        block = data[offset:offset + BLOCK_BYTES]
        a = (a + _word(block, 0)) & MASK32
        b = (b + _word(block, 1)) & MASK32
        c = (c + _word(block, 2)) & MASK32
        a, b, c = _mix(a, b, c)
        offset += BLOCK_BYTES
    if length == offset:
        return (b << HIGH_SHIFT) | c
    tail = data[offset:] + b"\0" * BLOCK_BYTES
    a = (a + _word(tail, 0)) & MASK32
    b = (b + _word(tail, 1)) & MASK32
    c = (c + _word(tail, 2)) & MASK32
    a, b, c = _final(a, b, c)
    return (b << HIGH_SHIFT) | c


if __name__ == "__main__":
    import sys
    for path in sys.argv[1:]:
        print(f"0x{uop_hash(path):016x} {path}")
