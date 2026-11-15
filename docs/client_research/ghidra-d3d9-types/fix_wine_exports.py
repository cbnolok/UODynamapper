#!/usr/bin/env python3
"""Rename entries of a Ghidra .exports file to the DLL's real export names.

Ghidra builds .exports files from the symbol at each export address. For Wine DLLs that
symbol comes from DWARF and is decorated (``_WSAStartup@8``) or a Wine stub name
(``___wine_stub_WSCUpdateProvider``), which breaks signature matching for imports by
ordinal. The export table itself has the undecorated names.
"""
import argparse
import re
import shutil
import subprocess
from pathlib import Path

EXPORT_LINE = re.compile(r"^\s+\[\s*\d+\]\s+\+base\[\s*(\d+)\]\s+[0-9a-f]+\s+(\S+)$")
EXPORTS_ENTRY = re.compile(r'(ORDINAL="(\d+)" NAME=")([^"]*)(")')
BACKUP_SUFFIX = ".orig"


def export_names(dll: Path) -> dict[int, str]:
    dump = subprocess.run(["objdump", "-p", str(dll)], check=True, capture_output=True,
                          text=True).stdout
    names = {}
    for line in dump.splitlines():
        match = EXPORT_LINE.match(line)
        if match:
            names[int(match.group(1))] = match.group(2)
    return names


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("dll", type=Path)
    parser.add_argument("exports", type=Path)
    args = parser.parse_args()

    names = export_names(args.dll)
    if not names:
        raise SystemExit(f"No named exports found in {args.dll}")
    renamed = []

    def rename(match: re.Match) -> str:
        real = names.get(int(match.group(2)))
        if real is None or real == match.group(3):
            return match.group(0)
        renamed.append(f"{match.group(2)}: {match.group(3)} -> {real}")
        return match.group(1) + real + match.group(4)

    text = EXPORTS_ENTRY.sub(rename, args.exports.read_text())
    if renamed:
        backup = args.exports.with_name(args.exports.name + BACKUP_SUFFIX)
        if backup.exists():
            raise SystemExit(f"Backup already exists: {backup}")
        shutil.copy2(args.exports, backup)
        args.exports.write_text(text)
    print("\n".join(renamed))
    print(f"Renamed {len(renamed)} entries")


if __name__ == "__main__":
    main()
