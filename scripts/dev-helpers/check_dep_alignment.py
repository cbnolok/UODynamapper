#!/usr/bin/env python3
"""
check_dep_alignment.py — Report workspace direct dependencies that resolve to a
different version than the one already pulled in by a reference package's
transitive dependency tree (default: bevy).

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
import subprocess
import sys
from collections import defaultdict, deque


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
             "(default: bevy). Can be specified multiple times.",
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
    reference_names = args.references if args.references else ["bevy"]

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
        if p["name"] in reference_names
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

    # --- Report ---
    ref_label = " + ".join(sorted(reference_names))

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
        return 0

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
