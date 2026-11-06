#!/usr/bin/env python
from __future__ import annotations

import argparse
import subprocess
import sys
from pathlib import Path


DEFAULT_ZSTD_VALUE = ""
DEFAULT_ZSTD_ARGS = ["--zstd"]
# No format flag: the packer's default, lossless Zstd 19 atlas pages.
DEFAULT_GUMP_ARGS: list[str] = []
# The Classic gump package, which gumps_ec.uddp shares the Classic gumps with unless --no-gump-sharing.
GUMPS_CC_FILE_NAME = "gumps_cc.uddp"
NO_GUMP_SHARING_HELP = (
    "pack gumps_ec.uddp standalone, keeping the Classic gumps it repeats, instead of leaving them out for the "
    "gumps_cc.uddp in the output folder to draw"
)
# Mobile animation pages without a format flag: lossless RGBA8888 with Zstd 19, as the art, land and gump pages.
LOSSLESS_ANIM_FORMAT = "raw"
LOSSLESS_ZSTD_LEVEL = "19"
DEFAULT_TABLES_DIR = str(Path(__file__).resolve().parents[2] / "dynamapper/assets/cc_ec_convtables")


def run(command: list[str]) -> None:
    completed = subprocess.run(command)
    if completed.returncode != 0:
        raise SystemExit(completed.returncode)


def run_pack(*args: str) -> None:
    print()
    run(["cargo", "run", "-p", "udd-conv-cli", "--bin", "udd-cli", "--", "pack", *args])


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
    default_zstd_level: str | None = None,
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
    elif default_zstd_level is not None:
        result.append(f"--{cli_prefix}zstd={default_zstd_level}")
    return result


def default_or_requested_zstd(args: argparse.Namespace) -> list[str]:
    return zstd_args_from_namespace(args) or DEFAULT_ZSTD_ARGS


def default_or_requested_gump_args(args: argparse.Namespace) -> list[str]:
    return zstd_args_from_namespace(args) or DEFAULT_GUMP_ARGS


def require_value(value: str, usage: str) -> str:
    if value:
        return value
    print(usage, file=sys.stderr)
    raise SystemExit(2)


def parse_map_ids(raw_maps: str, usage: str = "usage: just uddconv-all <ccdir> [ecdir] [output_dir] [maps]") -> list[str]:
    map_ids = [item.strip() for item in raw_maps.split(",") if item.strip()]
    if not map_ids:
        print(usage, file=sys.stderr)
        raise SystemExit(2)
    return map_ids


def ensure_output_dir(path: str) -> None:
    Path(path).mkdir(parents=True, exist_ok=True)


def run_animations(ccdir: str, ecdir: str, output_dir: str, cc_format: list[str], ec_format: list[str], tables: str = DEFAULT_TABLES_DIR) -> None:
    if not ccdir and not ecdir:
        print("usage: just uddconv-animations [ccdir] [ecdir] [output_dir]", file=sys.stderr)
        raise SystemExit(2)
    ensure_output_dir(output_dir)
    if ccdir:
        run_pack(
            "pack-mobile-anims",
            *cc_format,
            "--ccdir",
            ccdir,
            "--output",
            f"{output_dir}/mobile_anim_cc.uddp",
        )
    if ecdir:
        run_pack(
            "pack-ec-mobile-anims",
            *ec_format,
            "--tables",
            tables,
            "--ecdir",
            ecdir,
            "--output",
            f"{output_dir}/mobile_anim_ec.uddp",
        )


def run_gumps(ccdir: str, output_dir: str, format_args: list[str] | None = None) -> None:
    require_value(ccdir, "usage: just uddconv-gumps <ccdir> [output_dir]")
    ensure_output_dir(output_dir)
    run_pack("pack-gumps", *(format_args or DEFAULT_GUMP_ARGS), "--ccdir", ccdir, "--output", f"{output_dir}/{GUMPS_CC_FILE_NAME}")


def run_ec_gumps(ecdir: str, output_dir: str, format_args: list[str] | None = None, share: bool = True) -> None:
    """Packs gumps_ec.uddp. With `share`, the default, it leaves out the Classic gumps the gumps_cc.uddp in the output
    folder draws the same (pack-ec-gumps --share-with), so it needs that package; without one there it packs standalone.
    """
    require_value(ecdir, "usage: just uddconv-ec-gumps <ecdir> [output_dir]")
    ensure_output_dir(output_dir)
    share_args: list[str] = []
    classic_gumps = Path(output_dir) / GUMPS_CC_FILE_NAME
    if share and classic_gumps.is_file():
        share_args = ["--share-with", str(classic_gumps)]
    elif share:
        print(f"No {classic_gumps} to share the Classic gumps with: packing gumps_ec.uddp standalone.")
    run_pack(
        "pack-ec-gumps",
        *(format_args or DEFAULT_GUMP_ARGS),
        *share_args,
        "--ecdir",
        ecdir,
        "--output",
        f"{output_dir}/gumps_ec.uddp",
    )


def run_maps_statics(ccdir: str, output_dir: str, maps: str, non_texture_zstd: list[str]) -> None:
    ccdir = require_value(ccdir, "usage: uddconv.py maps-statics --ccdir CCDIR [--output-dir DIR] [--maps MAPS]")
    ensure_output_dir(output_dir)
    for map_id in parse_map_ids(maps, "usage: uddconv.py maps-statics --ccdir CCDIR [--output-dir DIR] [--maps MAPS]"):
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
    tables: str,
    non_texture_zstd: list[str],
    gump_args: list[str],
    include_animations: bool = True,
    share_gumps: bool = True,
    paperdoll_zstd: list[str] | None = None,
) -> None:
    ccdir = require_value(ccdir, "usage: just uddconv-all <ccdir> [ecdir] [output_dir] [maps]")
    ensure_output_dir(output_dir)
    map_ids = parse_map_ids(maps)

    source_args = ["--ccdir", ccdir]
    if ecdir:
        source_args.extend(["--ecdir", ecdir])

    print("Converting common world packages...")
    run_pack("pack-art", *cc_art_format, *source_args, "--output", f"{output_dir}/tex_art_cc.uddp")
    run_pack("pack-texmaps", *cc_land_format, "--ccdir", ccdir, "--output", f"{output_dir}/tex_land_cc.uddp")
    if ecdir:
        run_pack(
            "pack-ec-textures",
            *ec_art_format,
            *ec_land_format,
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
    if include_animations:
        run_animations(ccdir, ecdir, output_dir, cc_anim_format, ec_anim_format, tables)
    # gumps_cc.uddp first: gumps_ec.uddp shares the Classic gumps with it.
    run_gumps(ccdir, output_dir, gump_args)
    if ecdir:
        run_ec_gumps(ecdir, output_dir, gump_args, share=share_gumps)
        run_pack(
            "pack-ec-tex-aux",
            *non_texture_zstd,
            *source_args,
            "--tilemeta",
            f"{output_dir}/tilemeta.uddp",
            "--output",
            f"{output_dir}/tex_aux_ec.uddp",
        )
        run_pack("pack-ec-effects", *non_texture_zstd, *source_args, "--output", f"{output_dir}/effects_ec.uddp")
        # Without a requested level, the packer's lossless Zstd 19 frames.
        run_pack(
            "pack-ec-paperdoll",
            *(paperdoll_zstd or []),
            "--ecdir",
            ecdir,
            "--output",
            f"{output_dir}/paperdoll_ec.uddp",
        )


def main() -> int:
    parser = argparse.ArgumentParser()
    subparsers = parser.add_subparsers(dest="command", required=True)

    all_parser = subparsers.add_parser("all")
    all_parser.add_argument("--ccdir", default="")
    all_parser.add_argument("--ecdir", default="")
    all_parser.add_argument("--output-dir", default="target/uddp")
    all_parser.add_argument("--maps", default="0,1,2,3,4,5")
    all_parser.add_argument("--tables", default=DEFAULT_TABLES_DIR)
    all_parser.add_argument("--no-animations", action="store_true", help="leave out the mobile animation packages")
    all_parser.add_argument("--no-gump-sharing", action="store_true", help=NO_GUMP_SHARING_HELP)
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
    animations_parser.add_argument("--tables", default=DEFAULT_TABLES_DIR)
    add_zstd_arg(animations_parser)
    add_format_args(animations_parser, prefix="cc-")
    add_zstd_arg(animations_parser, prefix="cc-")
    add_format_args(animations_parser, prefix="ec-")
    add_zstd_arg(animations_parser, prefix="ec-")

    gumps_parser = subparsers.add_parser("gumps")
    gumps_parser.add_argument("--ccdir", default="")
    gumps_parser.add_argument("--output-dir", default="target/uddp")
    add_zstd_arg(gumps_parser)

    ec_gumps_parser = subparsers.add_parser("ec-gumps")
    ec_gumps_parser.add_argument("--ecdir", default="")
    ec_gumps_parser.add_argument("--output-dir", default="target/uddp")
    ec_gumps_parser.add_argument("--no-gump-sharing", action="store_true", help=NO_GUMP_SHARING_HELP)
    add_zstd_arg(ec_gumps_parser)

    maps_statics_parser = subparsers.add_parser("maps-statics")
    maps_statics_parser.add_argument("--ccdir", default="")
    maps_statics_parser.add_argument("--output-dir", default="target/uddp")
    maps_statics_parser.add_argument("--maps", default="0,1,2,3,4,5")
    add_zstd_arg(maps_statics_parser)

    args = parser.parse_args()
    if args.command == "all":
        global_zstd = zstd_args_from_namespace(args)
        non_texture_zstd = default_or_requested_zstd(args)
        gump_args = default_or_requested_gump_args(args)
        run_all(
            args.ccdir,
            args.ecdir,
            args.output_dir,
            args.maps,
            # No format flag: the packers' default, lossless Zstd 19 art pages.
            format_and_zstd_args(args, prefix="cc-art-", global_zstd=global_zstd),
            format_and_zstd_args(args, prefix="cc-land-", global_zstd=global_zstd),
            format_and_zstd_args(
                args,
                prefix="cc-anim-",
                default_format=LOSSLESS_ANIM_FORMAT,
                default_zstd_level=LOSSLESS_ZSTD_LEVEL,
                global_zstd=global_zstd,
            ),
            format_and_zstd_args(
                args,
                prefix="ec-anim-",
                default_format=LOSSLESS_ANIM_FORMAT,
                default_zstd_level=LOSSLESS_ZSTD_LEVEL,
                global_zstd=global_zstd,
            ),
            format_and_zstd_args(args, prefix="art-", cli_prefix="art-", global_zstd=global_zstd),
            format_and_zstd_args(args, prefix="land-", cli_prefix="land-", global_zstd=global_zstd),
            args.tables,
            non_texture_zstd,
            gump_args,
            include_animations=not args.no_animations,
            share_gumps=not args.no_gump_sharing,
            paperdoll_zstd=global_zstd,
        )
    elif args.command == "animations":
        global_zstd = zstd_args_from_namespace(args)
        run_animations(
            args.ccdir,
            args.ecdir,
            args.output_dir,
            format_and_zstd_args(
                args,
                prefix="cc-",
                default_format=LOSSLESS_ANIM_FORMAT,
                default_zstd_level=LOSSLESS_ZSTD_LEVEL,
                global_zstd=global_zstd,
            ),
            format_and_zstd_args(
                args,
                prefix="ec-",
                default_format=LOSSLESS_ANIM_FORMAT,
                default_zstd_level=LOSSLESS_ZSTD_LEVEL,
                global_zstd=global_zstd,
            ),
            args.tables,
        )
    elif args.command == "gumps":
        run_gumps(args.ccdir, args.output_dir, default_or_requested_gump_args(args))
    elif args.command == "ec-gumps":
        run_ec_gumps(
            args.ecdir,
            args.output_dir,
            default_or_requested_gump_args(args),
            share=not args.no_gump_sharing,
        )
    else:
        run_maps_statics(args.ccdir, args.output_dir, args.maps, default_or_requested_zstd(args))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
