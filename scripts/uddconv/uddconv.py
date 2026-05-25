#!/usr/bin/env python
from __future__ import annotations

import argparse
import subprocess
import sys
from pathlib import Path


def run(command: list[str]) -> None:
    completed = subprocess.run(command)
    if completed.returncode != 0:
        raise SystemExit(completed.returncode)


def run_pack(*args: str) -> None:
    print()
    run(["cargo", "run", "-p", "udd-conv-cli", "--bin", "udd-pack", "--", *args])


def require_value(value: str, usage: str) -> str:
    if value:
        return value
    print(usage, file=sys.stderr)
    raise SystemExit(2)


def parse_map_ids(raw_maps: str) -> list[str]:
    map_ids = [item.strip() for item in raw_maps.split(",") if item.strip()]
    if not map_ids:
        print("usage: just uddconv-all <ccdir> [ecdir] [output_dir] [maps]", file=sys.stderr)
        raise SystemExit(2)
    return map_ids


def ensure_output_dir(path: str) -> None:
    Path(path).mkdir(parents=True, exist_ok=True)


def run_animations(ccdir: str, ecdir: str, output_dir: str) -> None:
    if not ccdir and not ecdir:
        print("usage: just uddconv-animations [ccdir] [ecdir] [output_dir]", file=sys.stderr)
        raise SystemExit(2)
    ensure_output_dir(output_dir)
    if ccdir:
        run_pack("pack-mobile-anims", "--ccdir", ccdir, "--output", f"{output_dir}/mobile_anim_cc.uddp")
    if ecdir:
        run_pack("pack-ec-mobile-anims", "--ecdir", ecdir, "--output", f"{output_dir}/mobile_anim_ec.uddp")


def run_gumps(ccdir: str, output_dir: str) -> None:
    require_value(ccdir, "usage: just uddconv-gumps <ccdir> [output_dir]")
    ensure_output_dir(output_dir)
    run_pack("pack-gumps", "--ccdir", ccdir, "--output", f"{output_dir}/gumps_cc.uddp")


def run_ec_gumps(ecdir: str, output_dir: str) -> None:
    require_value(ecdir, "usage: just uddconv-ec-gumps <ecdir> [output_dir]")
    ensure_output_dir(output_dir)
    run_pack("pack-ec-gumps", "--ecdir", ecdir, "--output", f"{output_dir}/gumps_ec.uddp")


def run_all(ccdir: str, ecdir: str, output_dir: str, maps: str) -> None:
    ccdir = require_value(ccdir, "usage: just uddconv-all <ccdir> [ecdir] [output_dir] [maps]")
    ensure_output_dir(output_dir)
    map_ids = parse_map_ids(maps)

    source_args = ["--ccdir", ccdir]
    if ecdir:
        source_args.extend(["--ecdir", ecdir])

    print("Converting common world packages...")
    run([
        "cargo",
        "run",
        "-p",
        "udd-conv-cli",
        "--bin",
        "udd-pack",
        "--",
        "pack-tilemeta",
        *source_args,
        "--output",
        f"{output_dir}/tilemeta.uddp",
    ])
    run_pack("pack-art", "--raw", "--ccdir", ccdir, "--output", f"{output_dir}/tex_art_cc.uddp")
    run_pack("pack-texmaps", "--raw", "--ccdir", ccdir, "--output", f"{output_dir}/tex_land_cc.uddp")
    if ecdir:
        run_pack(
            "pack-ec-textures",
            "--ecdir",
            ecdir,
            "--art-output",
            f"{output_dir}/tex_art_ec.uddp",
            "--land-output",
            f"{output_dir}/tex_land_ec.uddp",
        )
    run_pack("pack-lights", *source_args, "--output", f"{output_dir}/world_lights.uddp")
    run_pack("pack-hues", "--ccdir", ccdir, "--output", f"{output_dir}/hues.uddp")
    for map_id in map_ids:
        run_pack("pack-map", "--ccdir", ccdir, "--map-id", map_id, "--output", f"{output_dir}/map{map_id}.uddp")
        run_pack(
            "pack-statics",
            "--ccdir",
            ccdir,
            "--map-id",
            map_id,
            "--output",
            f"{output_dir}/statics{map_id}.uddp",
        )
    run_animations(ccdir, ecdir, output_dir)
    run_gumps(ccdir, output_dir)
    if ecdir:
        run_ec_gumps(ecdir, output_dir)


def main() -> int:
    parser = argparse.ArgumentParser()
    subparsers = parser.add_subparsers(dest="command", required=True)

    all_parser = subparsers.add_parser("all")
    all_parser.add_argument("--ccdir", default="")
    all_parser.add_argument("--ecdir", default="")
    all_parser.add_argument("--output-dir", default="target/uddp")
    all_parser.add_argument("--maps", default="0,1,2,3,4,5")

    animations_parser = subparsers.add_parser("animations")
    animations_parser.add_argument("--ccdir", default="")
    animations_parser.add_argument("--ecdir", default="")
    animations_parser.add_argument("--output-dir", default="target/uddp")

    gumps_parser = subparsers.add_parser("gumps")
    gumps_parser.add_argument("--ccdir", default="")
    gumps_parser.add_argument("--output-dir", default="target/uddp")

    ec_gumps_parser = subparsers.add_parser("ec-gumps")
    ec_gumps_parser.add_argument("--ecdir", default="")
    ec_gumps_parser.add_argument("--output-dir", default="target/uddp")

    args = parser.parse_args()
    if args.command == "all":
        run_all(args.ccdir, args.ecdir, args.output_dir, args.maps)
    elif args.command == "animations":
        run_animations(args.ccdir, args.ecdir, args.output_dir)
    elif args.command == "gumps":
        run_gumps(args.ccdir, args.output_dir)
    else:
        run_ec_gumps(args.ecdir, args.output_dir)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
