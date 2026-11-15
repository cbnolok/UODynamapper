#!/usr/bin/env python3
"""Locate PE strings/references and extract mapped bytes without executing the file."""

import argparse
import hashlib
import json
from pathlib import Path
import struct


class PeImage:
    def __init__(self, data):
        self.data = data
        if data[:2] != b"MZ":
            raise ValueError("missing DOS signature")
        header = struct.unpack_from("<I", data, 0x3c)[0]
        if data[header:header + 4] != b"PE\0\0":
            raise ValueError("missing PE signature")
        count, optional_size = struct.unpack_from("<H12xH", data, header + 6)
        optional = header + 24
        magic = struct.unpack_from("<H", data, optional)[0]
        if magic == 0x10b:
            self.pointer_size = 4
            self.base = struct.unpack_from("<I", data, optional + 28)[0]
        elif magic == 0x20b:
            self.pointer_size = 8
            self.base = struct.unpack_from("<Q", data, optional + 24)[0]
        else:
            raise ValueError("unsupported optional header")
        self.sections = []
        for index in range(count):
            offset = optional + optional_size + 40 * index
            name, virtual_size, rva, size, file_offset = struct.unpack_from("<8sIIII", data, offset)
            flags = struct.unpack_from("<I", data, offset + 36)[0]
            if file_offset + size > len(data):
                raise ValueError("section extends past file")
            self.sections.append(dict(name=name.rstrip(b"\0").decode("ascii"),
                va=self.base + rva, virtual_size=virtual_size, size=size,
                offset=file_offset, executable=bool(flags & 0x20000000)))

    def offset_to_va(self, offset):
        for section in self.sections:
            if section["offset"] <= offset < section["offset"] + section["size"]:
                return section["va"] + offset - section["offset"]
        raise ValueError(f"file offset {offset:#x} is outside mapped sections")

    def va_to_offset(self, va, size=1):
        for section in self.sections:
            delta = va - section["va"]
            if 0 <= delta < section["size"] and delta + size <= section["size"]:
                return section["offset"] + delta
        raise ValueError(f"VA {va:#x} has no contiguous file-backed range of {size} bytes")

    def occurrences(self, needle, executable_only=False):
        for section in self.sections:
            if executable_only and not section["executable"]:
                continue
            start, end = section["offset"], section["offset"] + section["size"]
            while (offset := self.data.find(needle, start, end)) >= 0:
                yield offset
                start = offset + 1

    def string_references(self, needle):
        for offset in self.occurrences(needle):
            va = self.offset_to_va(offset)
            pointer = va.to_bytes(self.pointer_size, "little")
            yield dict(offset=hex(offset), va=hex(va), candidate_code_references=[
                hex(self.offset_to_va(ref)) for ref in self.occurrences(pointer, True)])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("executable", type=Path)
    parser.add_argument("--string", action="append", default=[], help="ASCII bytes to locate (repeatable)")
    parser.add_argument("--extract-va", type=lambda value: int(value, 0))
    parser.add_argument("--size", type=lambda value: int(value, 0))
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    extract = [args.extract_va is not None, args.size is not None, args.output is not None]
    if any(extract) and not all(extract):
        parser.error("--extract-va, --size and --output must be supplied together")
    if args.size is not None and args.size <= 0:
        parser.error("--size must be positive")
    data = args.executable.read_bytes()
    pe = PeImage(data)
    report = dict(sha256=hashlib.sha256(data).hexdigest(), image_base=hex(pe.base),
        sections=pe.sections, strings={value: list(pe.string_references(value.encode("ascii")))
        for value in args.string})
    if all(extract):
        offset = pe.va_to_offset(args.extract_va, args.size)
        # Never overwrite an input executable or an existing research artifact.
        with args.output.open("xb") as output:
            output.write(data[offset:offset + args.size])
        report["extracted"] = dict(offset=hex(offset), size=args.size, output=str(args.output))
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
