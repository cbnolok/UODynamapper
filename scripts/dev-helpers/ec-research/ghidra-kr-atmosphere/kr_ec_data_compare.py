"""Compare Kingdom Reborn and Enhanced Client data for the atmosphere packages (third pass). Read only.

Inputs are scratch extractions, never committed:
- `uoppackage-cli extract --dictionary <client>/string_dictionary.uop <client>/<Package>.uop <dir>`: named entries
  land under their path, the rest at the folder root as `0x<hash>.<ext>` (see uop_hash.py);
- `ec_research --client <client> items <ids...>` dumps (`items` below) and `materials` dumps.

Subcommands (run with `python3 -I -B kr_ec_data_compare.py <subcommand> ...`; `-I` ignores PYTHONDONTWRITEBYTECODE,
so `-B` keeps the import of uop_hash from writing a `__pycache__` here):

  terraintexture KR_DIR EC_DIR
      TerrainTexture.uop entries 0..MAX_TERRAINTEXTURE_ID: size, mips, format, SHA-256, identical or not.
  light-colours DIR ID [ID ...]
      Per TerrainTexture light decal: size, alpha-weighted mean RGB, alpha mean and maximum, coverage.
  terrain-textures KR_MATERIALS EC_MATERIALS KR_TEXTURE_DIR EC_TEXTURE_DIR
      Every Texture.uop id the two TerrainDefinition dumps reference: identical, same art (luma correlation at
      least SAME_ART_CORRELATION) or other art, then where EC keeps a byte-identical copy of a differing KR one.
  grunge KR_ITEMS EC_ITEMS
      Per tileart block, the items whose shader map 1 is the grunge noise in each client.
  light-fields KR_TILEART_DIR EC_TILEART_DIR KR_ITEMS EC_ITEMS MAX_ID
      Per item: the light scale and offsets (raw tileart header) and the light texture (block 3, map 0).
  shaders KR_DIR EC_DIR
      Shaders.uop entries: identical, differing or present in one client only (named by data/shaders/<file>).
"""
import collections
import hashlib
import re
import struct
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from uop_hash import uop_hash  # noqa: E402

SHA_PREFIX = 12
# DDS header fields (byte offsets).
DDS_HEIGHT, DDS_WIDTH, DDS_MIPS, DDS_FOURCC = 12, 16, 28, 84
U32_BYTES = 4
MAX_TERRAINTEXTURE_ID = 100
TERRAINTEXTURE_PATH = "build/terraintexture/%08d.dds"
WORLDART_PATH = "build/worldart/%08d.dds"
TILEART_PATH = "build/tileart/%08d.bin"
SHADER_PATH = "data/shaders/%s"
SHADER_FILES = ("uosprite.psh", "uosprite.vsh", "uospriteui.psh", "uospriteui.vsh", "postprocess.psh",
                "postprocess.vsh", "uoeffects.psh", "uoeffects.vsh", "uodeath.psh", "uodeath.vsh")
SAME_ART_CORRELATION = 0.9
# EC keeps some replaced KR terrain textures at id + this offset (2000040 -> 3000040).
LEGACY_TERRAIN_ID_OFFSET = 1000000
ALPHA_CHANNEL = 3
GRUNGE_ID = 1000045
GRUNGE_SAMPLER = 1
TILEART_BLOCKS = (0, 1, 2, 3)
LIGHT_BLOCK = 3
# Tileart record header: version 3 is Kingdom Reborn, 4 the Enhanced Client. The light fields follow body_type;
# EC records carry two more bytes before them (uses_enhanced_tileart, alternate_animation).
KR_RECORD_VERSION, EC_RECORD_VERSION = 3, 4
LIGHT_FIELDS_OFFSET = {KR_RECORD_VERSION: 0x23, EC_RECORD_VERSION: 0x25}
LIGHT_FIELDS_FORMAT = "<fIff"
FLOAT_DIGITS = 4
MAX_EXAMPLES = 12

ITEM_RE = re.compile(r"^item=(\d+) ")
SOURCE_RE = re.compile(r"^  block=(\d+) source=(\d+) sampler=(\d+) id=(\d+) path=Some\(\"([^\"]*)\"\)")
MATERIAL_SOURCE_RE = re.compile(r"id=Some\((\d+)\) path=Some\(\"([^\"]*)\"\)")


def short_sha(data):
    return hashlib.sha256(data).hexdigest()[:SHA_PREFIX]


def hash_stem(path):
    return "0x%016x" % uop_hash(path)


_HASHED_INDEX = {}


def hashed(root, path):
    """The entry for `path` in an extraction: under its name if the dictionary had it, else under its hash.

    uoppackage-cli picks a hashed entry's extension from its content (.dds, else .dat), not from the path."""
    named = Path(root) / path
    if named.exists():
        return named
    root = Path(root)
    if root not in _HASHED_INDEX:
        _HASHED_INDEX[root] = {entry.stem: entry for entry in root.iterdir() if entry.is_file()}
    stem = hash_stem(path)
    return _HASHED_INDEX[root].get(stem, root / (stem + Path(path).suffix))


def describe_dds(data):
    if data is None:
        return "absent"

    def u32(offset):
        return int.from_bytes(data[offset:offset + U32_BYTES], "little")

    fourcc = data[DDS_FOURCC:DDS_FOURCC + U32_BYTES].decode("ascii", "replace")
    return f"{u32(DDS_WIDTH)}x{u32(DDS_HEIGHT)} mips {u32(DDS_MIPS)} {fourcc} sha {short_sha(data)}"


def read_or_none(path):
    return path.read_bytes() if path.exists() else None


def cmd_terraintexture(kr_dir, ec_dir):
    for tid in range(MAX_TERRAINTEXTURE_ID + 1):
        k = read_or_none(hashed(kr_dir, TERRAINTEXTURE_PATH % tid))
        e = read_or_none(hashed(ec_dir, TERRAINTEXTURE_PATH % tid))
        if k is None and e is None:
            continue
        verdict = "only ec" if k is None else "only kr" if e is None else "identical" if k == e else "differs"
        print(tid, verdict, "| kr", describe_dds(k), "| ec", describe_dds(e))


def load_rgba(path):
    import numpy as np
    from PIL import Image
    return np.asarray(Image.open(path).convert("RGBA"), dtype=np.float64)


def cmd_light_colours(directory, *ids):
    for tid in map(int, ids):
        px = load_rgba(hashed(directory, TERRAINTEXTURE_PATH % tid))
        alpha = px[..., ALPHA_CHANNEL]
        weight = alpha.sum()
        rgb = px[..., :ALPHA_CHANNEL]
        mean = (rgb * alpha[..., None]).reshape(-1, ALPHA_CHANNEL).sum(axis=0) / weight if weight else \
            rgb.reshape(-1, ALPHA_CHANNEL).mean(axis=0)
        print(tid, f"{px.shape[1]}x{px.shape[0]}", "rgb (alpha-weighted)", mean.round(1),
              f"alpha mean {alpha.mean():.1f} max {alpha.max():.0f} covered {100.0 * (alpha > 0).mean():.0f}%")


def material_refs(path):
    return {int(m[1]): m[2] for m in MATERIAL_SOURCE_RE.finditer(Path(path).read_text(encoding="utf-8"))}


def luma(path):
    return load_rgba(path)[..., :ALPHA_CHANNEL].mean(axis=2)


def cmd_terrain_textures(kr_materials, ec_materials, kr_dir, ec_dir):
    import numpy as np
    kr_refs, ec_refs = material_refs(kr_materials), material_refs(ec_materials)
    counts = collections.Counter()
    # Several KR ids can share one image (the cliff faces), so a hash maps to every id that carries it.
    differing = collections.defaultdict(list)
    for tid in sorted(set(kr_refs) | set(ec_refs)):
        kp, ep = hashed(kr_dir, WORLDART_PATH % tid), hashed(ec_dir, WORLDART_PATH % tid)
        k, e = read_or_none(kp), read_or_none(ep)
        detail = ""
        if k is None or e is None:
            verdict = "missing"
            detail = f"kr {k is not None} ec {e is not None}"
        elif k == e:
            verdict = "identical"
        else:
            a, b = luma(kp), luma(ep)
            if a.shape != b.shape:
                verdict, detail = "other-art", f"sizes {a.shape} {b.shape}"
            else:
                corr = float(np.corrcoef(a.ravel(), b.ravel())[0, 1])
                verdict = "same-art" if corr >= SAME_ART_CORRELATION else "other-art"
                detail = f"corr {corr:.3f} luma kr {a.mean():.1f} ec {b.mean():.1f}"
            if tid in kr_refs:
                differing[hashlib.sha256(k).hexdigest()].append(tid)
        counts[verdict] += 1
        print(tid, verdict, "kr", kr_refs.get(tid), "ec", ec_refs.get(tid), detail)
    print("counts", dict(counts))
    copies = collections.defaultdict(list)
    for path in sorted(Path(ec_dir).rglob("*.dds")):
        sha = hashlib.sha256(path.read_bytes()).hexdigest()
        for tid in differing.get(sha, ()):
            copies[tid].append(path)
    for tid, paths in sorted(copies.items()):
        where = []
        for path in paths:
            ids = [i for i in (tid + LEGACY_TERRAIN_ID_OFFSET,) if hashed(ec_dir, WORLDART_PATH % i) == path]
            where.append(f"EC id {ids[0]}" if ids else f"EC entry {path.name}")
        print("KR texture", tid, "is byte-identical to", ", ".join(where))
    missing = sorted({tid for tids in differing.values() for tid in tids} - set(copies))
    print("differing KR textures with no byte-identical copy in EC:", missing)


def load_items(path):
    items, current = {}, None
    with open(path, encoding="utf-8") as handle:
        for line in handle:
            m = ITEM_RE.match(line)
            if m:
                current = []
                items[int(m[1])] = current
                continue
            m = SOURCE_RE.match(line)
            if m and current is not None:
                current.append((int(m[1]), int(m[3]), int(m[4])))
    return items


def has_grunge(refs, block):
    return any(b == block and sampler == GRUNGE_SAMPLER and tid == GRUNGE_ID for b, sampler, tid in refs)


def cmd_grunge(kr_items, ec_items):
    kr, ec = load_items(kr_items), load_items(ec_items)
    common = sorted(set(kr) & set(ec))
    print("items kr", len(kr), "ec", len(ec), "common", len(common))
    for block in TILEART_BLOCKS:
        k = {i for i in common if has_grunge(kr[i], block)}
        e = {i for i in common if has_grunge(ec[i], block)}
        same_refs = sum(1 for i in common if [r for r in kr[i] if r[0] == block] == [r for r in ec[i] if r[0] == block])
        print(f"block {block}: grunge kr {len(k)}, ec on common ids {len(e)}, both {len(k & e)}, "
              f"ec only {len(e - k)}, kr only {len(k - e)}, ec all items {sum(has_grunge(ec[i], block) for i in ec)}; "
              f"items whose refs in this block are identical {same_refs} of {len(common)}")
        print("   ec only (first):", sorted(e - k)[:MAX_EXAMPLES], "kr only (first):", sorted(k - e)[:MAX_EXAMPLES])


def light_fields(directory, tid):
    path = hashed(directory, TILEART_PATH % tid)
    if not path.exists():
        return None
    data = path.read_bytes()
    version = struct.unpack_from("<H", data)[0]
    values = struct.unpack_from(LIGHT_FIELDS_FORMAT, data, LIGHT_FIELDS_OFFSET[version])
    return tuple(round(v, FLOAT_DIGITS) if isinstance(v, float) else v for v in values)


def light_textures(items):
    return {i: tid for i, refs in items.items() for block, sampler, tid in refs if block == LIGHT_BLOCK and sampler == 0}


def cmd_light_fields(kr_dir, ec_dir, kr_items, ec_items, max_id):
    kr_light, ec_light = light_textures(load_items(kr_items)), light_textures(load_items(ec_items))
    stats = collections.Counter()
    scales = {"kr": collections.Counter(), "ec": collections.Counter()}
    examples = []
    for tid in range(int(max_id) + 1):
        k, e = light_fields(kr_dir, tid), light_fields(ec_dir, tid)
        if k is None or e is None:
            continue
        stats["common items"] += 1
        if tid in kr_light:
            scales["kr"][k[0]] += 1
        if tid in ec_light:
            scales["ec"][e[0]] += 1
        if tid not in kr_light and tid not in ec_light:
            continue
        stats["light items in either"] += 1
        if k == e and kr_light.get(tid) == ec_light.get(tid):
            stats["light items identical (fields and texture)"] += 1
        elif len(examples) < MAX_EXAMPLES:
            examples.append((tid, kr_light.get(tid), ec_light.get(tid), k, e))
    print(dict(stats))
    print("KR light items", len(kr_light), "EC light items", len(ec_light))
    for client, counter in scales.items():
        print(f"light scale on {client} light items", dict(sorted(counter.items())))
    for row in examples:
        print("differs (id, kr texture, ec texture, kr scale/field/x/y, ec ...)", row)


def cmd_shaders(kr_dir, ec_dir):
    named = set()
    for name in SHADER_FILES:
        named.add(hash_stem(SHADER_PATH % name))
        k = read_or_none(hashed(kr_dir, SHADER_PATH % name))
        e = read_or_none(hashed(ec_dir, SHADER_PATH % name))
        verdict = "absent" if k is None and e is None else "only ec" if k is None else "only kr" if e is None \
            else "identical" if k == e else "differs"
        print(name, verdict, "kr", None if k is None else len(k), "ec", None if e is None else len(e))
    for client, directory in (("kr", kr_dir), ("ec", ec_dir)):
        for path in sorted(Path(directory).iterdir()):
            if path.stem not in named:
                print("unnamed", client, path.name, path.stat().st_size)


COMMANDS = {
    "terraintexture": cmd_terraintexture,
    "light-colours": cmd_light_colours,
    "terrain-textures": cmd_terrain_textures,
    "grunge": cmd_grunge,
    "light-fields": cmd_light_fields,
    "shaders": cmd_shaders,
}

if __name__ == "__main__":
    if len(sys.argv) < 2 or sys.argv[1] not in COMMANDS:
        sys.exit(__doc__)
    COMMANDS[sys.argv[1]](*sys.argv[2:])
