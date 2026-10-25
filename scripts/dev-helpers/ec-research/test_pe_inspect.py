"""Exercise section-aware address mapping with deliberately unequal file/RVA offsets."""

import struct
import unittest

from pe_inspect import PeImage


def fixture(pe64=False):
    data = bytearray(0x700)
    data[:2] = b"MZ"
    struct.pack_into("<I", data, 0x3c, 0x80)
    data[0x80:0x84] = b"PE\0\0"
    struct.pack_into("<H", data, 0x86, 2)
    struct.pack_into("<H", data, 0x94, 0xf0)
    struct.pack_into("<H", data, 0x98, 0x20b if pe64 else 0x10b)
    base = 0x140000000 if pe64 else 0x400000
    struct.pack_into("<Q" if pe64 else "<I", data, 0xb0 if pe64 else 0xb4, base)
    for index, (name, rva, offset, flags) in enumerate([
            (b".text", 0x1000, 0x300, 0x20000000), (b".rdata", 0x3000, 0x500, 0)]):
        section = 0x188 + 40 * index
        struct.pack_into("<8sIIII", data, section, name, 0x200, rva, 0x100, offset)
        struct.pack_into("<I", data, section + 36, flags)
    data[0x520:0x52d] = b"UOWaterShader"
    struct.pack_into("<Q" if pe64 else "<I", data, 0x310, base + 0x3020)
    return PeImage(bytes(data)), base


class PeTests(unittest.TestCase):
    def test_section_mapping_and_pointer_width(self):
        for pe64 in [False, True]:
            pe, base = fixture(pe64)
            self.assertEqual(pe.offset_to_va(0x520), base + 0x3020)
            self.assertEqual(pe.va_to_offset(base + 0x3020, 13), 0x520)
            self.assertEqual(list(pe.string_references(b"UOWaterShader")), [dict(
                offset="0x520", va=hex(base + 0x3020),
                candidate_code_references=[hex(base + 0x1010)])])

    def test_unbacked_and_cross_section_ranges_are_rejected(self):
        pe, base = fixture()
        for va, size in [(base + 0x3100, 1), (base + 0x30f0, 32)]:
            with self.assertRaises(ValueError):
                pe.va_to_offset(va, size)
        with self.assertRaises(ValueError):
            pe.offset_to_va(0x450)

    def test_rejects_invalid_files(self):
        with self.assertRaises(ValueError):
            PeImage(b"not a PE")


if __name__ == "__main__":
    unittest.main()
