"""Remove the debug build artifacts that no current build uses.

The script rebuilds the debug configurations listed in BUILDS and records every artifact and build-script
output that cargo reports, fresh or rebuilt. It then deletes the files in deps/ and examples/ whose
`-<hash>` none of those builds named. Those are stale copies left by dependency or feature changes and by
other `-p` feature sets, and they are rebuilt on demand if something still needs them. The fingerprint and
build-script folders are left alone: they are small, and cargo rebuilds them when their artifacts are gone.

usage: python3 scripts/build/sweep_debug_target.py [--dry-run]
"""

import json
import os
import re
import subprocess
import sys

# The debug configurations kept: the workspace build, its test build, and dynamapper's own feature set.
BUILDS = (
    ("cargo", "build", "--workspace", "--all-targets"),
    ("cargo", "test", "--workspace", "--no-run"),
    ("cargo", "build", "-p", "dynamapper", "--bin", "dynamapper"),
    ("cargo", "test", "-p", "dynamapper", "--no-run"),
)
PROFILE_DIR_NAME = "debug"
SWEPT_SUBDIRS = ("deps", "examples")
# cargo's file name hash: `-` followed by 16 hex digits, before an extension or at the end of the name.
ARTIFACT_HASH_RE = re.compile(r"-([0-9a-f]{16})(?=[.]|$)")
BLOCK_SIZE = 512
GIB = 1 << 30


def artifact_hash(name):
    match = ARTIFACT_HASH_RE.search(name)
    return match.group(1) if match else None


def message_paths(message):
    reason = message.get("reason")
    if reason == "compiler-artifact":
        paths = list(message.get("filenames") or [])
        if message.get("executable"):
            paths.append(message["executable"])
        return paths
    if reason == "build-script-executed":
        return [message.get("out_dir") or ""]
    return []


def kept_hashes():
    keep = set()
    for build in BUILDS:
        print("==", " ".join(build), file=sys.stderr, flush=True)
        result = subprocess.run(
            [*build, "--message-format=json-render-diagnostics"], stdout=subprocess.PIPE, text=True
        )
        if result.returncode != 0:
            sys.exit(f"build failed: {' '.join(build)}; nothing was removed")
        for line in result.stdout.splitlines():
            try:
                message = json.loads(line)
            except ValueError:
                continue
            for path in message_paths(message):
                for part in path.split(os.sep):
                    found = artifact_hash(part)
                    if found:
                        keep.add(found)
    return keep


def target_directory():
    metadata = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--no-deps"], stdout=subprocess.PIPE, text=True, check=True
    )
    return json.loads(metadata.stdout)["target_directory"]


def main():
    dry_run = "--dry-run" in sys.argv[1:]
    keep = kept_hashes()
    profile_dir = os.path.join(target_directory(), PROFILE_DIR_NAME)
    removed = freed = 0
    for subdir in SWEPT_SUBDIRS:
        folder = os.path.join(profile_dir, subdir)
        if not os.path.isdir(folder):
            continue
        for name in os.listdir(folder):
            path = os.path.join(folder, name)
            found = artifact_hash(name)
            if found is None or found in keep or not os.path.isfile(path):
                continue
            freed += os.stat(path).st_blocks * BLOCK_SIZE
            removed += 1
            if not dry_run:
                os.remove(path)
    verb = "would remove" if dry_run else "removed"
    print(f"{len(keep)} artifact hashes in use; {verb} {removed} files ({freed / GIB:.2f} GiB)")


if __name__ == "__main__":
    main()
