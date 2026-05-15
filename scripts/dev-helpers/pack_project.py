#!/usr/bin/env python3
"""
pack_project.py — Concatenate a project into a single file with clear file boundaries.

Highlights:
- DEFAULT_EXCLUDE_EXTS for common binary/media/archive/cached file types.
- --ignore-exts / --exclude-exts default to DEFAULT_EXCLUDE_EXTS; set to "" to disable.
- Shell-style wildcards for include/ignore globs (*, **, ?).
- Clear START/END headers and safe code fences per file.
- Skips likely-binary files via heuristic.
- Deterministic ordering; configurable size limit and filters.

Usage:
  python3 pack_project.py -r /path/to/project -o project_bundle.txt
"""

import argparse
import fnmatch
import os
import re
import sys
from pathlib import Path
from typing import Iterable, List, Set


DEFAULT_EXCLUDE_DIRS = {
    "archive", 
    ".git", "target", ".idea", ".vscode", "node_modules", ".venv", ".tox",
    "dist", "build", ".mypy_cache", ".pytest_cache", "__pycache__", ".cargo",
}

# Default include extensions: text/source/config/docs
DEFAULT_EXTS = {
    # Shaders
    "wgsl",
    # Rust core
    "rs", "toml", #"lock",
    # docs/config
    "json", "yml", "yaml", "cfg", "ini", "env",
}

# Default exclude extensions: binaries, media, archives, office, caches
DEFAULT_EXCLUDE_EXTS = {
    # Rust
    "lock",
    # Git
    "gitignore",
    # scripts and CI
    "sh", "bash", "zsh", "ps1", "bat", "cmd", "makefile", "mk", "dockerfile",
    # web-ish bits (if present)
    "html", "css", "scss", "js", "jsx", "ts", "tsx",
    # misc common text
    "sql", "proto", "patch", "diff", "csv", "tsv", "xml", "svg", "gradle",
    # Python helpers sometimes present in repos
    "py",

    # Compiled/binary
    "exe", "dll", "so", "dylib", "a", "lib", "o", "obj", "pdb", "wasm",
    # Archives/compressed
    "zip", "tar", "gz", "tgz", "bz2", "tbz2", "xz", "7z", "rar",
    # Images
    "png", "jpg", "jpeg", "gif", "webp", "bmp", "tiff", "tif", "ico", "icns", "psd",
    # Fonts
    "ttf", "otf", "woff", "woff2",
    # Audio/video
    "mp3", "mp4", "m4a", "wav", "flac", "mov", "avi", "mkv", "webm", "ogg",
    # Databases
    "sqlite", "db", "rdb", "mdb", "accdb", "realm",
    # Coverage/profiling
    "gcda", "gcno", "profraw", "profdata",
    # Documents/office/PDFs
    "pdf", "doc", "docx", "ppt", "pptx", "xls", "xlsx",
    # Misc noisy
    "ico", "icns", "map",
}

# Best-effort mapping for nicer code block language hints
LANG_MAP = {
    "rs": "rust",
    "toml": "toml",
    "md": "markdown",
    "markdown": "markdown",
    "txt": "text",
    "json": "json",
    "yml": "yaml",
    "yaml": "yaml",
    "sh": "bash",
    "bash": "bash",
    "zsh": "bash",
    "ps1": "powershell",
    "bat": "batch",
    "cmd": "batch",
    "mk": "makefile",
    "makefile": "makefile",
    "dockerfile": "dockerfile",
    "html": "html",
    "css": "css",
    "scss": "scss",
    "js": "javascript",
    "jsx": "jsx",
    "ts": "typescript",
    "tsx": "tsx",
    "sql": "sql",
    "proto": "protobuf",
    "csv": "csv",
    "tsv": "tsv",
    "xml": "xml",
    "svg": "xml",
    "py": "python",
    "cfg": "ini",
    "ini": "ini",
    "env": "dotenv",
    "lock": "text",
    "gradle": "groovy",
}

# Files with no extension that should be included and their language hints
NAME_LANG_OVERRIDES = {
    "dockerfile": "dockerfile",
    "makefile": "makefile",
    "license": "text",
    "cargo.toml": "toml",
    "cargo.lock": "text",  # large; still text
    "build.rs": "rust",
    "rust-toolchain": "text",
    "rustfmt.toml": "toml",
    ".gitignore": "gitignore",
    ".gitattributes": "text",
    ".editorconfig": "ini",
    "readme": "markdown",
    "readme.md": "markdown",
}


def is_probably_binary(path: Path, sniff_bytes: int = 4096) -> bool:
    try:
        with path.open("rb") as f:
            chunk = f.read(sniff_bytes)
    except Exception:
        return True  # if unreadable, treat as binary to be safe
    if not chunk:
        return False
    if b"\x00" in chunk:
        return True
    text_chars = bytearray({7, 8, 9, 10, 12, 13, 27} | set(range(0x20, 0x7F)))
    nontext = sum(b not in text_chars for b in chunk)
    return (nontext / max(1, len(chunk))) > 0.30


def choose_code_fence(content: str) -> str:
    runs = re.findall(r"`+", content)
    longest = max((len(r) for r in runs), default=0)
    return "`" * (longest + 2 if longest >= 3 else 3)


def detect_lang_for(path: Path) -> str:
    name = path.name.lower()
    if name in NAME_LANG_OVERRIDES:
        return NAME_LANG_OVERRIDES[name]
    stem = path.suffix.lower().lstrip(".")
    if stem in LANG_MAP:
        return LANG_MAP[stem]
    return "text"


def should_include_file(path: Path, exts: Set[str], include_names: Set[str]) -> bool:
    name_lower = path.name.lower()
    if name_lower in include_names:
        return True
    ext = path.suffix.lower().lstrip(".")
    if ext in exts:
        return True
    if path.name.lower() in NAME_LANG_OVERRIDES:
        return True
    return False


def _read_ignore_file(ignore_file: Path) -> List[str]:
    patterns: List[str] = []
    try:
        for line in ignore_file.read_text(encoding="utf-8", errors="replace").splitlines():
            s = line.strip()
            if not s or s.startswith("#"):
                continue
            patterns.append(s)
    except Exception:
        pass
    return patterns


def _match_any_glob(rel_posix_lower: str, patterns: List[str]) -> bool:
    for pat in patterns:
        p = pat.replace("\\", "/").lower()
        if p.startswith("/"):
            p = p.lstrip("/")
        if fnmatch.fnmatch(rel_posix_lower, p):
            return True
    return False


def iter_files(
    root: Path,
    exclude_dirs: Set[str],
    include_globs: List[str],
    ignore_globs: List[str],
    ignore_exts: Set[str],
    exts: Set[str],
    prefer_include: bool,
    follow_symlinks: bool,
) -> Iterable[Path]:
    exclude_dirs_norm = {d.lower() for d in exclude_dirs}
    include_names = {n.lower() for n in NAME_LANG_OVERRIDES.keys()}
    inc_globs = [g.strip() for g in include_globs if g.strip()]
    ign_globs = [g.strip() for g in ignore_globs if g.strip()]

    for dirpath, dirnames, filenames in os.walk(root, followlinks=follow_symlinks):
        # Prune excluded directories
        for d in list(dirnames):
            if d.lower() in exclude_dirs_norm:
                dirnames.remove(d)

        for fname in filenames:
            p = Path(dirpath) / fname
            rel = p.relative_to(root)
            rel_posix = str(rel).replace("\\", "/")
            rel_lower = rel_posix.lower()

            # Prefer include globs override ignores when requested
            if prefer_include and inc_globs and _match_any_glob(rel_lower, inc_globs):
                yield p
                continue

            # Ignore by glob/extension (default extensions included)
            ext = p.suffix.lower().lstrip(".")
            if (ign_globs and _match_any_glob(rel_lower, ign_globs)) or (ext and ext in ignore_exts):
                continue

            # Non-preferred include globs (force-include things that otherwise wouldn't pass)
            if not prefer_include and inc_globs and _match_any_glob(rel_lower, inc_globs):
                yield p
                continue

            # Base include rule (extensions + name overrides)
            if should_include_file(p, exts, include_names):
                yield p


def main():
    ap = argparse.ArgumentParser(description="Pack a project into a single text file with file headers.")
    ap.add_argument("-r", "--root", type=Path, required=True, help="Project root directory.")
    ap.add_argument("-o", "--output", type=Path, required=True, help="Output file path.")
    ap.add_argument("--max-bytes", type=int, default=2_000_000,
                    help="Skip files larger than this many bytes (0 = no limit). Default: 2,000,000.")
    ap.add_argument("--no-code-fences", action="store_true",
                    help="Do not wrap each file content in a code fence.")
    ap.add_argument("--exclude-dirs", type=str, default=",".join(sorted(DEFAULT_EXCLUDE_DIRS)),
                    help="Comma-separated list of directory names to exclude (exact names).")
    ap.add_argument("--exts", type=str, default=",".join(sorted(DEFAULT_EXTS)),
                    help="Comma-separated list of file extensions to include (without dots).")
    ap.add_argument("--include-globs", type=str, default="",
                    help="Comma-separated glob patterns (relative to root) to force-include. Supports *, **, ?.")
    ap.add_argument("--ignore-globs", "--unwanted-globs", dest="ignore_globs", type=str, default="",
                    help="Comma-separated glob patterns to skip (relative to root). Supports *, **, ?.")
    ap.add_argument("--ignore-exts", "--exclude-exts", dest="ignore_exts", type=str,
                    default=",".join(sorted(DEFAULT_EXCLUDE_EXTS)),
                    help="Comma-separated file extensions to skip (without dots). "
                         "Default includes common binary/media/archive types.")
    ap.add_argument("--ignore-file", type=Path, default=None,
                    help="Path to a file with one ignore pattern per line (shell-style globs).")
    ap.add_argument("--prefer-include", action="store_true",
                    help="If set, include-globs override ignore globs/exts.")
    ap.add_argument("--follow-symlinks", action="store_true", help="Follow symlinks.")
    args = ap.parse_args()

    root: Path = args.root.resolve()
    if not root.exists() or not root.is_dir():
        print(f"Error: root directory not found or not a directory: {root}", file=sys.stderr)
        sys.exit(1)

    exclude_dirs = set(filter(None, (s.strip() for s in args.exclude_dirs.split(","))))
    exts = set(filter(None, (s.strip().lower() for s in args.exts.split(","))))
    include_globs = [g.strip() for g in args.include_globs.split(",") if g.strip()]

    ignore_globs_cli = [g.strip() for g in args.ignore_globs.split(",") if g.strip()]
    ignore_globs_file = _read_ignore_file(args.ignore_file) if args.ignore_file else []
    ignore_globs = ignore_globs_cli + ignore_globs_file

    ignore_exts = set(filter(None, (s.strip().lower() for s in args.ignore_exts.split(","))))

    files = list(iter_files(
        root=root,
        exclude_dirs=exclude_dirs,
        include_globs=include_globs,
        ignore_globs=ignore_globs,
        ignore_exts=ignore_exts,
        exts=exts,
        prefer_include=args.prefer_include,
        follow_symlinks=args.follow_symlinks,
    ))
    files.sort(key=lambda p: str(p.relative_to(root)).replace("\\", "/").lower())

    written = 0
    skipped_binary = 0
    skipped_too_large = 0
    skipped_errors = 0

    try:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        with args.output.open("w", encoding="utf-8", newline="\n") as out:
            out.write(f"PROJECT_ROOT: {root}\n")
            out.write(f"FILE_COUNT: {len(files)}\n")
            out.write("=" * 80 + "\n\n")

            for fp in files:
                rel = fp.relative_to(root)
                rel_str = str(rel).replace("\\", "/")
                try:
                    size = fp.stat().st_size
                    if args.max_bytes > 0 and size > args.max_bytes:
                        skipped_too_large += 1
                        continue
                    if is_probably_binary(fp):
                        skipped_binary += 1
                        continue

                    content = fp.read_text(encoding="utf-8", errors="replace")
                    content = content.replace("\r\n", "\n").replace("\r", "\n")
                    lang = detect_lang_for(fp)

                    out.write(f"===== START FILE: {rel_str} =====\n")
                    if args.no_code_fences:
                        out.write(content)
                        if not content.endswith("\n"):
                            out.write("\n")
                    else:
                        fence = choose_code_fence(content)
                        out.write(f"{fence}{lang}\n")
                        out.write(content)
                        if not content.endswith("\n"):
                            out.write("\n")
                        out.write(f"{fence}\n")
                    out.write(f"===== END FILE: {rel_str} =====\n\n")
                    written += 1

                except Exception as e:
                    skipped_errors += 1
                    sys.stderr.write(f"[warn] Skipping {rel_str}: {e}\n")

            out.write("=" * 80 + "\n")
            out.write(f"SUMMARY: written={written}, skipped_binary={skipped_binary}, "
                      f"skipped_too_large={skipped_too_large}, skipped_errors={skipped_errors}\n")

    except Exception as e:
        print(f"Failed to write output: {e}", file=sys.stderr)
        sys.exit(2)

    print(f"Done. Wrote {written} files to {args.output}. "
          f"(binary skipped: {skipped_binary}, too large: {skipped_too_large}, errors: {skipped_errors})")
    sys.exit(0)


if __name__ == "__main__":
    main()
