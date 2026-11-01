#!/usr/bin/env python3
"""
check_dep_alignment.py — Report workspace dependencies that resolve to a
different version than the one already pulled in by a reference package's
transitive dependency tree (default: bevy and bevy_egui).

Two checks run: the workspace's direct dependencies, and every crate reached
through them (e.g. egui pulled by eframe while bevy_egui pins another egui).
Cargo can't express "the version bevy_egui uses", so this script is how the
workspace keeps such crates on one version: when bevy or bevy_egui moves,
it fails until the workspace declarations follow.

A mismatch means both versions are compiled separately, inflating build times
and binary size. Updating the workspace declaration to accept the reference
version eliminates the duplicate.

Usage:
    # Run from workspace root (calls cargo metadata automatically):
    python3 scripts/dev-helpers/check_dep_alignment.py

    # Or pipe metadata explicitly:
    cargo metadata --format-version 1 | python3 scripts/dev-helpers/check_dep_alignment.py

    # Use a different reference root (e.g. eframe):
    python3 scripts/dev-helpers/check_dep_alignment.py --reference eframe

    # Only show normal (non-dev, non-build) deps:
    python3 scripts/dev-helpers/check_dep_alignment.py --no-dev --no-build
"""

import argparse
import json
import os
import re
import subprocess
import sys
from collections import defaultdict, deque

DEFAULT_REFERENCES = ("bevy", "bevy_egui")
# The platforms we build for are the ones the workspace-hack is generated for.
HAKARI_CONFIG = os.path.join(".config", "hakari.toml")
# The generated feature-unification crate mirrors the other members' resolution; a mismatch only it pulls means
# it is stale (`cargo hakari generate`), not that a declaration needs changing.
WORKSPACE_HACK = "workspace-hack"
# Direct dependencies whose own requirements cause duplicates we can't remove by changing our declarations:
# crate -> why. Mismatches reached only through these are reported, never a failure.
KNOWN_SOURCES = {
    "dds": "dds 0.2, its latest release, requires glam 0.25",
    "full_moon": "full_moon 3.0, its latest release, requires derive_more 1, smol_str 0.3 and (its derive macros) syn 1 and indexmap 1",
    "knuffel": "knuffel 3.2, its latest release (2023), requires syn 1, chumsky 0.9 and miette 5",
    "miette": "miette 5 is the version knuffel 3.2 requires",
}


# ---------------------------------------------------------------------------
# Metadata loading
# ---------------------------------------------------------------------------

def load_metadata() -> dict:
    if not sys.stdin.isatty():
        return json.load(sys.stdin)
    result = subprocess.run(
        ["cargo", "metadata", "--format-version", "1"],
        capture_output=True, text=True, check=True,
    )
    return json.loads(result.stdout)


# ---------------------------------------------------------------------------
# Graph helpers
# ---------------------------------------------------------------------------

def bfs_subtree(root_ids: set, node_map: dict) -> set:
    """Return all package IDs reachable (transitively) from root_ids."""
    visited = set()
    queue = deque(root_ids)
    while queue:
        pkg_id = queue.popleft()
        if pkg_id in visited:
            continue
        visited.add(pkg_id)
        node = node_map.get(pkg_id)
        if node:
            for dep in node.get("deps", []):
                queue.append(dep["pkg"])
    return visited


def built_packages() -> set:
    """(name, version) of every package some workspace build compiles on a platform we build for.

    cargo metadata's resolve graph also lists packages that the feature resolver never activates (it reported
    rand 0.8 through phf_macros, which nothing builds) and the ones only other platforms (iOS) build, so the
    transitive check keeps only what cargo tree shows for our platforms.
    """
    with open(HAKARI_CONFIG, encoding="utf-8") as config:
        text = config.read()
    try:
        import tomllib  # Python 3.11+; the CI runner's Python is older.
        platforms = tomllib.loads(text)["platforms"]
    except ImportError:
        platforms = re.findall(r'"([^"]+)"', re.search(r"^platforms\s*=\s*\[(.*?)\]", text, re.S | re.M).group(1))
    targets = [arg for platform in platforms for arg in ("--target", platform)]
    result = subprocess.run(
        ["cargo", "tree", "--workspace", *targets, "-e", "normal,build", "--prefix", "none", "--format", "{p}"],
        capture_output=True, text=True, check=True,
    )
    built = set()
    for line in result.stdout.splitlines():
        parts = line.split()
        if len(parts) >= 2 and parts[1].startswith("v"):
            built.add((parts[0], parts[1][1:]))
    return built


def dep_kind_set(dep_edge: dict) -> set:
    """Return the set of dep_kinds strings for a resolve graph dep edge."""
    return {dk.get("kind") for dk in dep_edge.get("dep_kinds", [{"kind": None}])}


# ---------------------------------------------------------------------------
# Main logic
# ---------------------------------------------------------------------------

def main() -> int:
    parser = argparse.ArgumentParser(
        description="Check workspace dep versions against a reference package's resolved tree."
    )
    parser.add_argument(
        "--reference", default=None,
        help="Name of the reference package whose transitive dep versions are authoritative "
             f"(default: {', '.join(DEFAULT_REFERENCES)}). Can be specified multiple times.",
        action="append", dest="references",
    )
    parser.add_argument(
        "--no-dev", action="store_true",
        help="Exclude dev dependencies from the workspace dep scan.",
    )
    parser.add_argument(
        "--no-build", action="store_true",
        help="Exclude build dependencies from the workspace dep scan.",
    )
    parser.add_argument(
        "--show-all", action="store_true",
        help="Also show deps that already match the reference version (informational).",
    )
    args = parser.parse_args()

    # The default argparse `append` adds to the default, so handle that:
    reference_names = args.references if args.references else list(DEFAULT_REFERENCES)

    try:
        meta = load_metadata()
    except subprocess.CalledProcessError as e:
        print(f"error: cargo metadata failed:\n{e.stderr}", file=sys.stderr)
        return 2

    pkg_map: dict = {p["id"]: p for p in meta["packages"]}
    node_map: dict = {n["id"]: n for n in meta["resolve"]["nodes"]}
    workspace_ids: set = set(meta.get("workspace_members", []))

    # --- Find reference package roots ---
    ref_root_ids = {
        p["id"]
        for p in meta["packages"]
        if p["name"] in reference_names and p["id"] in node_map
    }
    if not ref_root_ids:
        names_in_graph = sorted({p["name"] for p in meta["packages"]})
        print(
            f"error: reference package(s) {reference_names!r} not found in dependency graph.\n"
            f"Available package names (first 20): {names_in_graph[:20]}",
            file=sys.stderr,
        )
        return 2

    # --- Build reference version map: name -> set of resolved versions ---
    ref_subtree_ids = bfs_subtree(ref_root_ids, node_map)
    ref_versions: dict[str, set] = defaultdict(set)
    for pkg_id in ref_subtree_ids:
        pkg = pkg_map.get(pkg_id)
        if pkg:
            ref_versions[pkg["name"]].add(pkg["version"])

    # --- Scan workspace member direct deps ---
    # Structure: dep_name -> list of {ws_pkg, our_version, ref_versions, dep_kinds}
    mismatches: dict[str, list] = defaultdict(list)
    matches: dict[str, list] = defaultdict(list)

    for ws_id in sorted(workspace_ids, key=lambda i: pkg_map[i]["name"]):
        ws_pkg = pkg_map[ws_id]
        ws_name = ws_pkg["name"]
        node = node_map.get(ws_id)
        if not node:
            continue

        for dep_edge in node.get("deps", []):
            kinds = dep_kind_set(dep_edge)

            # Filter by dep kind
            if args.no_dev and "dev" in kinds:
                continue
            if args.no_build and "build" in kinds:
                continue

            dep_id = dep_edge["pkg"]
            dep_pkg = pkg_map.get(dep_id)
            if not dep_pkg:
                continue

            dep_name = dep_pkg["name"]
            dep_version = dep_pkg["version"]

            # Skip other workspace members (paths)
            if dep_id in workspace_ids:
                continue

            # Skip the reference package(s) themselves
            if dep_id in ref_root_ids:
                continue

            if dep_name not in ref_versions:
                # Not in the reference tree at all — no opinion
                continue

            ref_vers_for_dep = ref_versions[dep_name]

            entry = {
                "ws_pkg": ws_name,
                "our_version": dep_version,
                "ref_versions": sorted(ref_vers_for_dep),
                "dep_kinds": sorted(k or "normal" for k in kinds),
            }

            if dep_version in ref_vers_for_dep:
                matches[dep_name].append(entry)
            else:
                mismatches[dep_name].append(entry)

    # --- Transitive check: versions reached through a workspace direct dep that the reference tree resolves
    # differently. Attributed to the direct dep (and the members declaring it) that leads there.
    transitive: dict[str, dict] = defaultdict(lambda: defaultdict(set))
    built = built_packages()
    for ws_id in sorted(workspace_ids):
        ws_name = pkg_map[ws_id]["name"]
        for dep_edge in node_map.get(ws_id, {}).get("deps", []):
            direct_id = dep_edge["pkg"]
            if direct_id in workspace_ids or direct_id in ref_root_ids:
                continue
            direct = pkg_map[direct_id]
            for pkg_id in bfs_subtree({direct_id}, node_map) - ref_subtree_ids:
                pkg = pkg_map[pkg_id]
                if (pkg["name"] in ref_versions and pkg["version"] not in ref_versions[pkg["name"]]
                        and (pkg["name"], pkg["version"]) in built):
                    transitive[(pkg["name"], pkg["version"])][(direct["name"], direct["version"])].add(ws_name)

    # --- Report ---
    ref_label = " + ".join(sorted(reference_names))
    failed = False
    if transitive:
        print(f"=== TRANSITIVE VERSION MISMATCHES vs {ref_label} ===\n")
        for (name, version) in sorted(transitive):
            vias = transitive[(name, version)]
            # The hack declares everything the members do, so it only decides when it is the sole declarer.
            real = {via: members - {WORKSPACE_HACK} for via, members in vias.items() if members - {WORKSPACE_HACK}}
            if not real:
                tag = "stale workspace-hack: run `cargo hakari generate`"
            elif all(via_name in KNOWN_SOURCES for via_name, _ in real):
                tag = "known: " + "; ".join(sorted({KNOWN_SOURCES[via_name] for via_name, _ in real}))
            else:
                tag = "MISMATCH"
            failed |= tag == "MISMATCH"
            print(f"  {name} {version} (reference resolves {', '.join(sorted(ref_versions[name]))}) [{tag}]")
            for (via_name, via_version) in sorted(real or vias):
                members = (real or vias)[(via_name, via_version)]
                print(f"    via {via_name} {via_version}, declared by: {', '.join(sorted(members))}")
        print()

    if args.show_all and matches:
        print(f"=== ALIGNED with {ref_label} ({len(matches)} crates) ===\n")
        for dep_name in sorted(matches):
            entries = matches[dep_name]
            ref_v = entries[0]["ref_versions"]
            ws_pkgs = sorted({e["ws_pkg"] for e in entries})
            print(f"  {dep_name} {ref_v[0] if len(ref_v) == 1 else ref_v}")
            for ws in ws_pkgs:
                print(f"    used by: {ws}")
        print()

    if not mismatches:
        print(f"✓ All workspace direct dependencies already align with {ref_label}'s resolved versions.")
        return 1 if failed else 0

    print(f"=== VERSION MISMATCHES vs {ref_label} ({len(mismatches)} crates) ===\n")
    print(
        "  These workspace packages declare a dep version that differs from what\n"
        f"  {ref_label} resolves, causing both versions to be compiled separately.\n"
        "  Updating the version constraint in the workspace Cargo.toml to accept\n"
        f"  the {ref_label} version will unify them.\n"
    )

    for dep_name in sorted(mismatches):
        entries = mismatches[dep_name]
        ref_v = entries[0]["ref_versions"]
        print(f"  {dep_name}")
        print(f"    {ref_label} resolves: {', '.join(ref_v)}")
        for e in sorted(entries, key=lambda x: x["ws_pkg"]):
            kinds_str = f"  [{', '.join(e['dep_kinds'])}]" if e["dep_kinds"] != ["normal"] else ""
            print(f"    {e['ws_pkg']} uses: {e['our_version']}{kinds_str}")
        print()

    print(f"Total: {len(mismatches)} crate(s) with version mismatches.")
    return 1


if __name__ == "__main__":
    sys.exit(main())
