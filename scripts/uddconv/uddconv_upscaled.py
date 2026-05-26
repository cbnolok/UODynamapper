#!/usr/bin/env python
from __future__ import annotations

import argparse
import subprocess
import sys
from pathlib import Path


DEFAULT_ZSTD_VALUE = ""
DEFAULT_ZSTD_ARGS = ["--zstd"]
DEFAULT_TABLES_DIR = str(Path(__file__).resolve().parents[2] / "dynamapper/assets/cc_ec_convtables")
CC_ART_UPSCALE = ["kl-depixelize2x", "hq2x-true"]
CC_LAND_64_UPSCALE = ["xbr4x"]
CC_LAND_128_UPSCALE = ["xbr2x"]
EC_ART_UPSCALE = ["fsr-easu3x"]
EC_LAND_64_UPSCALE = ["hq2x-true", "fsr-easu-rcas2x"]
EC_LAND_128_UPSCALE = ["xbr2x", "fsr-easu-rcas2x"]
EC_LAND_256_UPSCALE = ["fsr-easu-rcas2x"]
EC_LAND_512_UPSCALE: list[str] = []
CC_ANIM_UPSCALE = ["hq3x-true"]
EC_ANIM_UPSCALE = ["fsr-easu-rcas2x"]
CC_GUMP_PAPERDOLL_UPSCALE = ["nedi2x", "xbr2x"]
CC_GUMP_SINGLE_UPSCALE = ["fsr-easu3x"]
EC_GUMP_PAPERDOLL_UPSCALE = ["lq3x"]
EC_GUMP_SINGLE_UPSCALE = ["fsr-easu2x"]


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
        help="Texture output format: raw, rgba8888, jxl, bc7, bc7-rdo, or bc7-rdo-lambda=VALUE.",
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


def add_upscale_arg(parser: argparse.ArgumentParser, *, prefix: str = "") -> None:
    parser.add_argument(
        f"--{prefix}upscale-pass",
        action="append",
        default=None,
        metavar="FILTER",
        help="Add an upscale pass. Repeat to chain filters; omit to use this helper's defaults.",
    )


def upscale_args_from_namespace(
    args: argparse.Namespace,
    *,
    prefix: str = "",
    cli_option: str = "upscale-pass",
    defaults: list[str] | None = None,
) -> list[str]:
    option_name = prefix.replace("-", "_") + "upscale_pass"
    passes = getattr(args, option_name, None)
    if passes is None:
        passes = defaults or []
    result: list[str] = []
    for upscale_filter in passes:
        result.extend([f"--{cli_option}", upscale_filter])
    return result


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
    if selected == "jxl":
        return [f"--{cli_prefix}jxl"]
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
        "(expected raw, rgba8888, jxl, bc7, bc7-rdo, or bc7-rdo-lambda=VALUE)",
        file=sys.stderr,
    )
    raise SystemExit(2)


def selected_format(args: argparse.Namespace, *, prefix: str = "", default: str | None = None) -> str | None:
    option_prefix = prefix.replace("-", "_") + "format"
    return getattr(args, option_prefix, None) or default


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
    default_format: str | None = None,
    default_zstd: bool = False,
    global_zstd: list[str] | None = None,
) -> list[str]:
    requested_format = selected_format(args, prefix=prefix, default=default_format)
    if requested_format is None:
        result = []
    else:
        local_args = argparse.Namespace(**vars(args))
        setattr(local_args, prefix.replace("-", "_") + "format", requested_format)
        result = format_args_from_namespace(local_args, prefix=prefix, cli_prefix=cli_prefix)
    prefixed_zstd = zstd_args_from_namespace(args, prefix=prefix, cli_prefix=cli_prefix)
    if prefixed_zstd:
        result.extend(prefixed_zstd)
    elif global_zstd:
        result.extend(global_zstd)
    elif default_zstd and requested_format != "jxl":
        result.extend([f"--{cli_prefix}zstd"])
    return result


def default_or_requested_zstd(args: argparse.Namespace) -> list[str]:
    return zstd_args_from_namespace(args) or DEFAULT_ZSTD_ARGS


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


def run_animations(
    ccdir: str,
    ecdir: str,
    output_dir: str,
    cc_format: list[str],
    ec_format: list[str],
    cc_upscale: list[str],
    ec_upscale: list[str],
    tables: str = DEFAULT_TABLES_DIR,
) -> None:
    if not ccdir and not ecdir:
        print("usage: just uddconv-animations [ccdir] [ecdir] [output_dir]", file=sys.stderr)
        raise SystemExit(2)
    ensure_output_dir(output_dir)
    if ccdir:
        run_pack("pack-mobile-anims", *cc_format, *cc_upscale, "--ccdir", ccdir, "--output", f"{output_dir}/mobile_anim_cc.uddp")
    if ecdir:
        run_pack("pack-ec-mobile-anims", *ec_format, *ec_upscale, "--tables", tables, "--ecdir", ecdir, "--output", f"{output_dir}/mobile_anim_ec.uddp")


def run_gumps(
    ccdir: str,
    output_dir: str,
    zstd_args: list[str] | None = None,
    paperdoll_upscale: list[str] | None = None,
    single_upscale: list[str] | None = None,
) -> None:
    require_value(ccdir, "usage: just uddconv-gumps <ccdir> [output_dir]")
    ensure_output_dir(output_dir)
    run_pack("pack-gumps", "--raw", *(zstd_args or DEFAULT_ZSTD_ARGS), *(paperdoll_upscale or []), *(single_upscale or []), "--ccdir", ccdir, "--output", f"{output_dir}/gumps_cc.uddp")


def run_ec_gumps(
    ecdir: str,
    output_dir: str,
    zstd_args: list[str] | None = None,
    paperdoll_upscale: list[str] | None = None,
    single_upscale: list[str] | None = None,
) -> None:
    require_value(ecdir, "usage: just uddconv-ec-gumps <ecdir> [output_dir]")
    ensure_output_dir(output_dir)
    run_pack("pack-ec-gumps", "--raw", *(zstd_args or DEFAULT_ZSTD_ARGS), *(paperdoll_upscale or []), *(single_upscale or []), "--ecdir", ecdir, "--output", f"{output_dir}/gumps_ec.uddp")


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
    non_texture_zstd: list[str],
    gump_zstd: list[str],
    tables: str,
    cc_art_upscale: list[str],
    cc_land_upscale: list[str],
    ec_art_upscale: list[str],
    ec_land_upscale: list[str],
    cc_anim_upscale: list[str],
    ec_anim_upscale: list[str],
    cc_gump_paperdoll_upscale: list[str],
    cc_gump_single_upscale: list[str],
    ec_gump_paperdoll_upscale: list[str],
    ec_gump_single_upscale: list[str],
) -> None:
    ccdir = require_value(ccdir, "usage: just uddconv-all <ccdir> [ecdir] [output_dir] [maps]")
    ensure_output_dir(output_dir)
    map_ids = parse_map_ids(maps)

    source_args = ["--ccdir", ccdir]
    if ecdir:
        source_args.extend(["--ecdir", ecdir])

    print("Converting common world packages...")
    run_pack("pack-art", *cc_art_format, *cc_art_upscale, "--ccdir", ccdir, "--output", f"{output_dir}/tex_art_cc.uddp")
    run_pack("pack-texmaps", *cc_land_format, *cc_land_upscale, "--ccdir", ccdir, "--output", f"{output_dir}/tex_land_cc.uddp")
    if ecdir:
        run_pack(
            "pack-ec-textures",
            *ec_art_format,
            *ec_land_format,
            *ec_art_upscale,
            *ec_land_upscale,
            *source_args,
            "--art-output",
            f"{output_dir}/tex_art_ec.uddp",
            "--land-output",
            f"{output_dir}/tex_land_ec.uddp",
            "--tilemeta-output",
            f"{output_dir}/tilemeta.uddp",
        )
    else:
        run_pack("pack-tilemeta", *non_texture_zstd, *source_args, "--output", f"{output_dir}/tilemeta.uddp")
    run_pack("pack-lights", *non_texture_zstd, *source_args, "--output", f"{output_dir}/world_lights.uddp")
    run_pack("pack-hues", *non_texture_zstd, "--ccdir", ccdir, "--output", f"{output_dir}/hues.uddp")
    for map_id in map_ids:
        run_pack(
            "pack-map-statics",
            *non_texture_zstd,
            "--ccdir",
            ccdir,
            "--map-id",
            map_id,
            "--map-output",
            f"{output_dir}/map{map_id}.uddp",
            "--statics-output",
            f"{output_dir}/statics{map_id}.uddp",
        )
    run_animations(ccdir, ecdir, output_dir, cc_anim_format, ec_anim_format, cc_anim_upscale, ec_anim_upscale, tables)
    run_gumps(ccdir, output_dir, gump_zstd, cc_gump_paperdoll_upscale, cc_gump_single_upscale)
    if ecdir:
        run_ec_gumps(ecdir, output_dir, gump_zstd, ec_gump_paperdoll_upscale, ec_gump_single_upscale)


def main() -> int:
    parser = argparse.ArgumentParser()
    subparsers = parser.add_subparsers(dest="command", required=True)

    all_parser = subparsers.add_parser("all")
    all_parser.add_argument("--ccdir", default="")
    all_parser.add_argument("--ecdir", default="")
    all_parser.add_argument("--output-dir", default="target/uddp")
    all_parser.add_argument("--maps", default="0,1,2,3,4,5")
    all_parser.add_argument("--tables", default=DEFAULT_TABLES_DIR)
    add_zstd_arg(all_parser)
    add_format_args(all_parser, prefix="cc-art-")
    add_zstd_arg(all_parser, prefix="cc-art-")
    add_upscale_arg(all_parser, prefix="cc-art-")
    add_format_args(all_parser, prefix="cc-land-")
    add_zstd_arg(all_parser, prefix="cc-land-")
    add_upscale_arg(all_parser, prefix="cc-land-64-")
    add_upscale_arg(all_parser, prefix="cc-land-128-")
    add_format_args(all_parser, prefix="cc-anim-")
    add_zstd_arg(all_parser, prefix="cc-anim-")
    add_upscale_arg(all_parser, prefix="cc-anim-")
    add_format_args(all_parser, prefix="ec-anim-")
    add_zstd_arg(all_parser, prefix="ec-anim-")
    add_upscale_arg(all_parser, prefix="ec-anim-")
    add_format_args(all_parser, prefix="art-")
    add_zstd_arg(all_parser, prefix="art-")
    add_upscale_arg(all_parser, prefix="art-")
    add_format_args(all_parser, prefix="land-")
    add_zstd_arg(all_parser, prefix="land-")
    add_upscale_arg(all_parser, prefix="land-64-")
    add_upscale_arg(all_parser, prefix="land-128-")
    add_upscale_arg(all_parser, prefix="land-256-")
    add_upscale_arg(all_parser, prefix="land-512-")
    add_upscale_arg(all_parser, prefix="cc-gump-paperdoll-")
    add_upscale_arg(all_parser, prefix="cc-gump-single-")
    add_upscale_arg(all_parser, prefix="ec-gump-paperdoll-")
    add_upscale_arg(all_parser, prefix="ec-gump-single-")

    animations_parser = subparsers.add_parser("animations")
    animations_parser.add_argument("--ccdir", default="")
    animations_parser.add_argument("--ecdir", default="")
    animations_parser.add_argument("--output-dir", default="target/uddp")
    animations_parser.add_argument("--tables", default=DEFAULT_TABLES_DIR)
    add_zstd_arg(animations_parser)
    add_format_args(animations_parser, prefix="cc-")
    add_zstd_arg(animations_parser, prefix="cc-")
    add_upscale_arg(animations_parser, prefix="cc-")
    add_format_args(animations_parser, prefix="ec-")
    add_zstd_arg(animations_parser, prefix="ec-")
    add_upscale_arg(animations_parser, prefix="ec-")

    gumps_parser = subparsers.add_parser("gumps")
    gumps_parser.add_argument("--ccdir", default="")
    gumps_parser.add_argument("--output-dir", default="target/uddp")
    add_zstd_arg(gumps_parser)
    add_upscale_arg(gumps_parser, prefix="paperdoll-")
    add_upscale_arg(gumps_parser, prefix="single-")

    ec_gumps_parser = subparsers.add_parser("ec-gumps")
    ec_gumps_parser.add_argument("--ecdir", default="")
    ec_gumps_parser.add_argument("--output-dir", default="target/uddp")
    add_zstd_arg(ec_gumps_parser)
    add_upscale_arg(ec_gumps_parser, prefix="paperdoll-")
    add_upscale_arg(ec_gumps_parser, prefix="single-")

    args = parser.parse_args()
    if args.command == "all":
        global_zstd = zstd_args_from_namespace(args)
        non_texture_zstd = default_or_requested_zstd(args)
        gump_zstd = non_texture_zstd
        run_all(
            args.ccdir,
            args.ecdir,
            args.output_dir,
            args.maps,
            format_and_zstd_args(args, prefix="cc-art-", default_format="bc7", default_zstd=True, global_zstd=global_zstd),
            format_and_zstd_args(args, prefix="cc-land-", default_format="bc7", default_zstd=True, global_zstd=global_zstd),
            format_and_zstd_args(args, prefix="cc-anim-", default_format="bc7", default_zstd=True, global_zstd=global_zstd),
            format_and_zstd_args(args, prefix="ec-anim-", default_format="bc7", default_zstd=True, global_zstd=global_zstd),
            format_and_zstd_args(args, prefix="art-", cli_prefix="art-", default_format="bc7", default_zstd=True, global_zstd=global_zstd),
            format_and_zstd_args(args, prefix="land-", cli_prefix="land-", default_format="bc7", default_zstd=True, global_zstd=global_zstd),
            non_texture_zstd,
            gump_zstd,
            args.tables,
            upscale_args_from_namespace(args, prefix="cc-art-", defaults=CC_ART_UPSCALE),
            [
                *upscale_args_from_namespace(args, prefix="cc-land-64-", cli_option="upscale-64-pass", defaults=CC_LAND_64_UPSCALE),
                *upscale_args_from_namespace(args, prefix="cc-land-128-", cli_option="upscale-128-pass", defaults=CC_LAND_128_UPSCALE),
            ],
            upscale_args_from_namespace(args, prefix="art-", cli_option="art-upscale-pass", defaults=EC_ART_UPSCALE),
            [
                *upscale_args_from_namespace(args, prefix="land-64-", cli_option="upscale-64-pass", defaults=EC_LAND_64_UPSCALE),
                *upscale_args_from_namespace(args, prefix="land-128-", cli_option="upscale-128-pass", defaults=EC_LAND_128_UPSCALE),
                *upscale_args_from_namespace(args, prefix="land-256-", cli_option="upscale-256-pass", defaults=EC_LAND_256_UPSCALE),
                *upscale_args_from_namespace(args, prefix="land-512-", cli_option="upscale-512-pass", defaults=EC_LAND_512_UPSCALE),
            ],
            upscale_args_from_namespace(args, prefix="cc-anim-", defaults=CC_ANIM_UPSCALE),
            upscale_args_from_namespace(args, prefix="ec-anim-", defaults=EC_ANIM_UPSCALE),
            upscale_args_from_namespace(args, prefix="cc-gump-paperdoll-", cli_option="paperdoll-upscale-pass", defaults=CC_GUMP_PAPERDOLL_UPSCALE),
            upscale_args_from_namespace(args, prefix="cc-gump-single-", cli_option="single-upscale-pass", defaults=CC_GUMP_SINGLE_UPSCALE),
            upscale_args_from_namespace(args, prefix="ec-gump-paperdoll-", cli_option="paperdoll-upscale-pass", defaults=EC_GUMP_PAPERDOLL_UPSCALE),
            upscale_args_from_namespace(args, prefix="ec-gump-single-", cli_option="single-upscale-pass", defaults=EC_GUMP_SINGLE_UPSCALE),
        )
    elif args.command == "animations":
        global_zstd = zstd_args_from_namespace(args)
        run_animations(
            args.ccdir,
            args.ecdir,
            args.output_dir,
            format_and_zstd_args(args, prefix="cc-", default_format="bc7", default_zstd=True, global_zstd=global_zstd),
            format_and_zstd_args(args, prefix="ec-", default_format="bc7", default_zstd=True, global_zstd=global_zstd),
            upscale_args_from_namespace(args, prefix="cc-", defaults=CC_ANIM_UPSCALE),
            upscale_args_from_namespace(args, prefix="ec-", defaults=EC_ANIM_UPSCALE),
            args.tables,
        )
    elif args.command == "gumps":
        run_gumps(
            args.ccdir,
            args.output_dir,
            default_or_requested_zstd(args),
            upscale_args_from_namespace(args, prefix="paperdoll-", cli_option="paperdoll-upscale-pass", defaults=CC_GUMP_PAPERDOLL_UPSCALE),
            upscale_args_from_namespace(args, prefix="single-", cli_option="single-upscale-pass", defaults=CC_GUMP_SINGLE_UPSCALE),
        )
    else:
        run_ec_gumps(
            args.ecdir,
            args.output_dir,
            default_or_requested_zstd(args),
            upscale_args_from_namespace(args, prefix="paperdoll-", cli_option="paperdoll-upscale-pass", defaults=EC_GUMP_PAPERDOLL_UPSCALE),
            upscale_args_from_namespace(args, prefix="single-", cli_option="single-upscale-pass", defaults=EC_GUMP_SINGLE_UPSCALE),
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
