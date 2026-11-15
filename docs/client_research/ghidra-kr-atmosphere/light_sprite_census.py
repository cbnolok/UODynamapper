"""Census of the tileart light-sprite inputs in Kingdom Reborn and the Enhanced Client (light sprite proposal). Read only.

Usage: python3 -I -B light_sprite_census.py KR_TILEART_DIR EC_TILEART_DIR KR_ITEMS EC_ITEMS MAX_ID

Inputs are the scratch extractions described in kr_ec_data_compare.py: each client's tileart.uop extracted with
uoppackage-cli, and each client's full `ec_research items` dump. Both clients create the lighting component only
when shader block 3 names a texture (the definition's light shader name is not null: UOKR 0x0056a09c in
0x00569cb0, UOSA `CreateComponents` 0x005bf3f6), so an item is a "light item" here when its block 3 has a
sampler-0 texture, whatever its LightSource flag.

Prints, per client: light items, their light texture ids, scales, offset ranges, the LightSource flag overlap and
the unused field_70; then the KR / EC agreement on items both clients define.
"""
import collections
import re
import struct
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from kr_ec_data_compare import LIGHT_FIELDS_FORMAT, LIGHT_FIELDS_OFFSET, TILEART_PATH, hashed  # noqa: E402

ITEM_RE = re.compile(r"^item=(\d+) type=\S+ flags=TaeFlag\(([^)]*)\)")
LIGHT_REF_RE = re.compile(r"^  block=3 source=\d+ sampler=0 id=(\d+) ")
LIGHT_SOURCE_FLAG = "LightSource"
ROUND_DIGITS = 4
TOP = 16
MAX_EXAMPLES = 20
# Items whose light inputs are printed in full: 2594 is the lantern of docs/agent_notes/static_light_visibility.md,
# 2576 one of the items whose offsets the Enhanced Client changed, the rest common combinations.
SAMPLE_ITEMS = (2594, 2564, 2575, 2576, 3633, 2842, 2843)


def load(path):
    items = {}
    current = None
    with open(path, encoding="utf-8") as handle:
        for line in handle:
            match = ITEM_RE.match(line)
            if match:
                current = int(match[1])
                items[current] = {"flags": match[2], "light": None}
                continue
            match = LIGHT_REF_RE.match(line)
            if match and current is not None and items[current]["light"] is None:
                items[current]["light"] = int(match[1])
    return items


def fields(directory, tid):
    path = hashed(directory, TILEART_PATH % tid)
    if not path.exists():
        return None
    data = path.read_bytes()
    version = struct.unpack_from("<H", data)[0]
    scale, field_70, offset_x, offset_y = struct.unpack_from(LIGHT_FIELDS_FORMAT, data, LIGHT_FIELDS_OFFSET[version])
    return round(scale, ROUND_DIGITS), field_70, round(offset_x, ROUND_DIGITS), round(offset_y, ROUND_DIGITS)


def report(name, directory, items, max_id):
    lights = {tid: item for tid, item in items.items() if item["light"] is not None and tid <= max_id}
    data = {tid: fields(directory, tid) for tid in lights}
    flagged = {tid for tid, item in items.items() if LIGHT_SOURCE_FLAG in item["flags"]}
    print(f"== {name}: items {len(items)}, light items {len(lights)}, LightSource flag {len(flagged)}, "
          f"both {len(flagged & set(lights))}, light without flag {len(set(lights) - flagged)}, "
          f"flag without light {len(flagged - set(lights))}")
    print("  textures", dict(sorted(collections.Counter(item["light"] for item in lights.values()).items())))
    print("  scales", dict(sorted(collections.Counter(d[0] for d in data.values()).items())))
    print("  field_70", dict(collections.Counter(d[1] for d in data.values()).most_common(TOP)))
    for index, label in ((2, "offset_x"), (3, "offset_y")):
        values = [d[index] for d in data.values()]
        print(f"  {label} min {min(values)} max {max(values)} top",
              collections.Counter(values).most_common(TOP))
    per_texture = collections.defaultdict(collections.Counter)
    for tid, item in lights.items():
        per_texture[item["light"]][data[tid][0], data[tid][2], data[tid][3]] += 1
    for texture, counter in sorted(per_texture.items()):
        print(f"  texture {texture}: (scale, offset_x, offset_y)", counter.most_common(TOP // 2))
    print("  without light, flag only (first):", sorted(flagged - set(lights))[:MAX_EXAMPLES])
    return lights, data


def main(kr_dir, ec_dir, kr_items, ec_items, max_id):
    max_id = int(max_id)
    kr, ec = load(kr_items), load(ec_items)
    kr_lights, kr_data = report("KR", kr_dir, kr, max_id)
    ec_lights, ec_data = report("EC", ec_dir, ec, max_id)
    common = sorted(set(kr_lights) & set(ec_lights))
    same = [tid for tid in common if kr_lights[tid]["light"] == ec_lights[tid]["light"] and kr_data[tid] == ec_data[tid]]
    print(f"== agreement: light items in both {len(common)}, identical texture and fields {len(same)}, "
          f"KR only {len(set(kr_lights) - set(ec_lights))}, EC only {len(set(ec_lights) - set(kr_lights))}")
    print("  EC-only light items defined by KR (first):",
          sorted(t for t in set(ec_lights) - set(kr_lights) if t in kr)[:MAX_EXAMPLES])
    for tid in [t for t in common if t not in same][:MAX_EXAMPLES]:
        print("  differs", tid, "kr", kr_lights[tid]["light"], kr_data[tid], "ec", ec_lights[tid]["light"], ec_data[tid])
    for tid in SAMPLE_ITEMS:
        print("  sample", tid, "kr", kr_lights.get(tid, {}).get("light"), kr_data.get(tid),
              "ec", ec_lights.get(tid, {}).get("light"), ec_data.get(tid))


if __name__ == "__main__":
    if len(sys.argv) != 6:
        sys.exit(__doc__)
    main(*sys.argv[1:])
