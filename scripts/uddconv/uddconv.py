#!/usr/bin/env python
from __future__ import annotations

import argparse
import subprocess
import sys
from pathlib import Path


DEFAULT_ZSTD_VALUE = ""


def run(command: list[str]) -> None:
    completed = subprocess.run(command)
    if completed.returncode != 0:
        raise SystemExit(completed.returncode)


def run_pack(*args: str) -> None:
    print()
    run(["cargo", "run", "-p", "udd-conv-cli", "--bin", "udd-pack", "--", *args])


def add_format_args(parser: argparse.ArgumentParser, *, prefix: str = "") -> None:
    parser.add_argument(
        f"--{prefix}format",
        default=None,
        metavar="FORMAT",
        help="Texture output format: raw, rgba8888, bc7, bc7-rdo, or bc7-rdo-lambda=VALUE.",
    )


def add_zstd_arg(parser: argparse.ArgumentParser, *, prefix: str = "") -> None:
    option_name = f"--{prefix}zstd"
    parser.add_argument(
        option_name,
        nargs="?",
        const=DEFAULT_ZSTD_VALUE,
        default=None,
        metavar="LEVEL",
        help=f"Apply a file-level Zstd pass. Optionally pass {option_name}=LEVEL.",
    )


def format_args_from_namespace(
    args: argparse.Namespace,
    *,
    prefix: str = "",
    cli_prefix: str = "",
) -> list[str]:
    option_prefix = prefix.replace("-", "_") + "format"
    selected = getattr(args, option_prefix, None)
    if selected is None:
        return []
    if selected in ("raw", "rgba8888"):
        return [f"--{cli_prefix}raw"]
    if selected == "bc7":
        return [f"--{cli_prefix}bc7"]
    if selected == "bc7-rdo":
        return [f"--{cli_prefix}bc7-rdo"]
    if selected.startswith("bc7-rdo-lambda="):
        value = selected.split("=", 1)[1]
        if not value:
            print(f"--{prefix}format=bc7-rdo-lambda=VALUE requires a value", file=sys.stderr)
            raise SystemExit(2)
        return [f"--{cli_prefix}bc7-rdo", "--bc7-rdo-lambda", value]
    print(
        f"invalid --{prefix}format: {selected} "
        "(expected raw, rgba8888, bc7, bc7-rdo, or bc7-rdo-lambda=VALUE)",
        file=sys.stderr,
    )
    raise SystemExit(2)


def zstd_args_from_namespace(
    args: argparse.Namespace,
    *,
    prefix: str = "",
    cli_prefix: str = "",
) -> list[str]:
    option_prefix = prefix.replace("-", "_") + "zstd"
    selected = getattr(args, option_prefix, None)
    if selected is None:
        return []
    if selected == DEFAULT_ZSTD_VALUE:
        return [f"--{cli_prefix}zstd"]
    return [f"--{cli_prefix}zstd={selected}"]


def format_and_zstd_args(
    args: argparse.Namespace,
    *,
    prefix: str = "",
    cli_prefix: str = "",
    global_zstd: list[str] | None = None,
) -> list[str]:
    result = format_args_from_namespace(args, prefix=prefix, cli_prefix=cli_prefix)
    prefixed_zstd = zstd_args_from_namespace(args, prefix=prefix, cli_prefix=cli_prefix)
    result.extend(prefixed_zstd or global_zstd or [])
    return result


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


def run_animations(ccdir: str, ecdir: str, output_dir: str, cc_format: list[str], ec_format: list[str]) -> None:
    if not ccdir and not ecdir:
        print("usage: just uddconv-animations [ccdir] [ecdir] [output_dir]", file=sys.stderr)
        raise SystemExit(2)
    ensure_output_dir(output_dir)
    if ccdir:
        run_pack("pack-mobile-anims", *cc_format, "--ccdir", ccdir, "--output", f"{output_dir}/mobile_anim_cc.uddp")
    if ecdir:
        run_pack("pack-ec-mobile-anims", *ec_format, "--ecdir", ecdir, "--output", f"{output_dir}/mobile_anim_ec.uddp")


def run_gumps(ccdir: str, output_dir: str) -> None:
    require_value(ccdir, "usage: just uddconv-gumps <ccdir> [output_dir]")
    ensure_output_dir(output_dir)
    run_pack("pack-gumps", "--ccdir", ccdir, "--output", f"{output_dir}/gumps_cc.uddp")


def run_ec_gumps(ecdir: str, output_dir: str) -> None:
    require_value(ecdir, "usage: just uddconv-ec-gumps <ecdir> [output_dir]")
    ensure_output_dir(output_dir)
    run_pack("pack-ec-gumps", "--ecdir", ecdir, "--output", f"{output_dir}/gumps_ec.uddp")


def run_all(
    ccdir: str,
    ecdir: str,
    output_dir: str,
    maps: str,
    cc_art_format: list[str],
    cc_land_format: list[str],
    cc_anim_format: list[str],
    ec_anim_format: list[str],
    ec_art_format: list[str],
    ec_land_format: list[str],
) -> None:
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
    run_pack("pack-art", *cc_art_format, "--ccdir", ccdir, "--output", f"{output_dir}/tex_art_cc.uddp")
    run_pack("pack-texmaps", *cc_land_format, "--ccdir", ccdir, "--output", f"{output_dir}/tex_land_cc.uddp")
    if ecdir:
        run_pack(
            "pack-ec-textures",
            *ec_art_format,
            *ec_land_format,
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
    run_animations(ccdir, ecdir, output_dir, cc_anim_format, ec_anim_format)
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
    add_zstd_arg(all_parser)
    add_format_args(all_parser, prefix="cc-art-")
    add_zstd_arg(all_parser, prefix="cc-art-")
    add_format_args(all_parser, prefix="cc-land-")
    add_zstd_arg(all_parser, prefix="cc-land-")
    add_format_args(all_parser, prefix="cc-anim-")
    add_zstd_arg(all_parser, prefix="cc-anim-")
    add_format_args(all_parser, prefix="ec-anim-")
    add_zstd_arg(all_parser, prefix="ec-anim-")
    add_format_args(all_parser, prefix="art-")
    add_zstd_arg(all_parser, prefix="art-")
    add_format_args(all_parser, prefix="land-")
    add_zstd_arg(all_parser, prefix="land-")

    animations_parser = subparsers.add_parser("animations")
    animations_parser.add_argument("--ccdir", default="")
    animations_parser.add_argument("--ecdir", default="")
    animations_parser.add_argument("--output-dir", default="target/uddp")
    add_zstd_arg(animations_parser)
    add_format_args(animations_parser, prefix="cc-")
    add_zstd_arg(animations_parser, prefix="cc-")
    add_format_args(animations_parser, prefix="ec-")
    add_zstd_arg(animations_parser, prefix="ec-")

    gumps_parser = subparsers.add_parser("gumps")
    gumps_parser.add_argument("--ccdir", default="")
    gumps_parser.add_argument("--output-dir", default="target/uddp")

    ec_gumps_parser = subparsers.add_parser("ec-gumps")
    ec_gumps_parser.add_argument("--ecdir", default="")
    ec_gumps_parser.add_argument("--output-dir", default="target/uddp")

    args = parser.parse_args()
    if args.command == "all":
        global_zstd = zstd_args_from_namespace(args)
        run_all(
            args.ccdir,
            args.ecdir,
            args.output_dir,
            args.maps,
            format_and_zstd_args(args, prefix="cc-art-", global_zstd=global_zstd),
            format_and_zstd_args(args, prefix="cc-land-", global_zstd=global_zstd),
            format_and_zstd_args(args, prefix="cc-anim-", global_zstd=global_zstd),
            format_and_zstd_args(args, prefix="ec-anim-", global_zstd=global_zstd),
            [
                *format_args_from_namespace(args, prefix="art-", cli_prefix="art-"),
                *global_zstd,
                *zstd_args_from_namespace(args, prefix="art-", cli_prefix="art-"),
            ],
            [
                *format_args_from_namespace(args, prefix="land-", cli_prefix="land-"),
                *zstd_args_from_namespace(args, prefix="land-", cli_prefix="land-"),
            ],
        )
    elif args.command == "animations":
        global_zstd = zstd_args_from_namespace(args)
        run_animations(
            args.ccdir,
            args.ecdir,
            args.output_dir,
            format_and_zstd_args(args, prefix="cc-", global_zstd=global_zstd),
            format_and_zstd_args(args, prefix="ec-", global_zstd=global_zstd),
        )
    elif args.command == "gumps":
        run_gumps(args.ccdir, args.output_dir)
    else:
        run_ec_gumps(args.ecdir, args.output_dir)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
