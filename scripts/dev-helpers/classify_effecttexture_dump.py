#!/usr/bin/env python3
"""
Classify extracted Enhanced Client EffectTexture payload files by content.

The EffectTexture dump mixes several payload types that are often mislabeled by
extension. This script classifies each file from its magic/header bytes and
reports the canonical extension that should be used for that payload.

Known signatures handled here:
- `Gamebryo File Format, Version ...` -> `.nif`
- `particlesystem ...` text payloads     -> `.ems`
- DDS magic (`DDS `)                     -> `.dds`
- Photoshop magic (`8BPS`)              -> `.psd`
- TGA headers                            -> `.tga`

Unknown files are kept visible in the output instead of being guessed.
"""

from __future__ import annotations

import argparse
import json
from dataclasses import asdict, dataclass
from pathlib import Path
from typing import Iterable


NIF_MAGIC = b"Gamebryo File Format, Version "
EMS_MAGIC = b"particlesystem "
DDS_MAGIC = b"DDS "
PSD_MAGIC = b"8BPS"
TEXT_BYTES = set(range(32, 127)) | {9, 10, 13}


@dataclass(frozen=True)
class Classification:
    path: str
    size: int
    detected_format: str
    canonical_extension: str
    current_extension: str
    matches_extension: bool
    confidence: str
    note: str


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Classify EffectTexture dump payloads by file signature."
    )
    parser.add_argument("root", type=Path, help="File or directory to inspect.")
    parser.add_argument(
        "--json",
        action="store_true",
        help="Emit JSON instead of tab-separated text.",
    )
    parser.add_argument(
        "--only-mismatches",
        action="store_true",
        help="Only print files whose current extension does not match the detected format.",
    )
    return parser.parse_args()


def iter_files(root: Path) -> Iterable[Path]:
    if root.is_file():
        yield root
        return

    for path in sorted(root.rglob("*")):
        if path.is_file():
            yield path


def is_probably_text(blob: bytes) -> bool:
    if not blob:
        return True
    if b"\x00" in blob:
        return False
    non_text = sum(byte not in TEXT_BYTES for byte in blob)
    return non_text / len(blob) <= 0.05


def is_tga(blob: bytes) -> bool:
    if len(blob) < 18:
        return False

    color_map_type = blob[1]
    image_type = blob[2]
    width = int.from_bytes(blob[12:14], "little")
    height = int.from_bytes(blob[14:16], "little")
    pixel_depth = blob[16]

    if color_map_type not in (0, 1):
        return False
    if image_type not in (0, 1, 2, 3, 9, 10, 11):
        return False
    if width <= 0 or height <= 0:
        return False
    if pixel_depth not in (8, 15, 16, 24, 32):
        return False

    return True


def classify_file(path: Path) -> Classification:
    blob = path.read_bytes()
    head = blob[:256]
    current_extension = path.suffix.lower()

    if head.startswith(NIF_MAGIC):
        detected_format = "nif"
        canonical_extension = ".nif"
        confidence = "high"
        note = "Gamebryo binary scene/effect payload"
    elif head.startswith(EMS_MAGIC):
        detected_format = "ems"
        canonical_extension = ".ems"
        confidence = "high"
        note = "Gamebryo particle/effect script text"
    elif head.startswith(DDS_MAGIC):
        detected_format = "dds"
        canonical_extension = ".dds"
        confidence = "high"
        note = "DirectDraw Surface texture"
    elif head.startswith(PSD_MAGIC):
        detected_format = "psd"
        canonical_extension = ".psd"
        confidence = "high"
        note = "Adobe Photoshop document"
    elif is_tga(head):
        detected_format = "tga"
        canonical_extension = ".tga"
        confidence = "medium"
        note = "Targa texture header matched"
    elif is_probably_text(head):
        detected_format = "text"
        canonical_extension = ".txt"
        confidence = "low"
        note = "Plain text payload with no known EMS signature"
    else:
        detected_format = "unknown"
        canonical_extension = ""
        confidence = "low"
        note = "Unrecognized binary signature"

    return Classification(
        path=str(path),
        size=path.stat().st_size,
        detected_format=detected_format,
        canonical_extension=canonical_extension,
        current_extension=current_extension,
        matches_extension=(canonical_extension == current_extension),
        confidence=confidence,
        note=note,
    )


def emit_tsv(rows: list[Classification]) -> None:
    print(
        "\t".join(
            [
                "path",
                "size",
                "detected_format",
                "canonical_extension",
                "current_extension",
                "matches_extension",
                "confidence",
                "note",
            ]
        )
    )
    for row in rows:
        print(
            "\t".join(
                [
                    row.path,
                    str(row.size),
                    row.detected_format,
                    row.canonical_extension,
                    row.current_extension,
                    "yes" if row.matches_extension else "no",
                    row.confidence,
                    row.note,
                ]
            )
        )


def main() -> int:
    args = parse_args()
    rows = [classify_file(path) for path in iter_files(args.root)]

    if args.only_mismatches:
        rows = [row for row in rows if not row.matches_extension]

    if args.json:
        print(json.dumps([asdict(row) for row in rows], indent=2))
    else:
        emit_tsv(rows)

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
