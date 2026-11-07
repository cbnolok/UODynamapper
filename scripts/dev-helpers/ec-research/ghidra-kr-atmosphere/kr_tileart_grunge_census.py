"""Count which KR tileart sprites carry the grunge map (sampler 1 of block 0) in the ec_research `items` dump.

Read only; prints a summary. Usage: python3 -I kr_tileart_grunge_census.py ITEMS_DUMP.txt
"""
import collections
import re
import sys

ITEM_RE = re.compile(r"^item=(\d+) type=(\S+) flags=TaeFlag\(([^)]*)\)")
BLOCK_RE = re.compile(r"^  block=(\d+) shader=Some\(\"([^\"]+)\"\)")
GRUNGE_SAMPLER = 1
SPRITE_BLOCK = 0
SPRITE_SHADER = "UOSpriteShader"
TOP_REFS = 40
TEXTURE_NAME_TAIL = 24
TOP_FLAGS = 12
MAX_RANGES = 60
TOP_BASE_NAMES = 40
REF_RE = re.compile(r"^  block=(\d+) source=(\d+) sampler=(\d+) id=(\d+) path=Some\(\"([^\"]*)\"\) repetition=(\S+) source_kind=(\d+) uv_set=(\d+)")


def main(path):
    items = {}
    current = None
    with open(path, encoding="utf-8") as handle:
        for line in handle:
            match = ITEM_RE.match(line)
            if match:
                current = {"id": int(match[1]), "type": match[2], "flags": match[3], "blocks": {}, "refs": []}
                items[current["id"]] = current
                continue
            match = BLOCK_RE.match(line)
            if match and current is not None:
                current["blocks"][int(match[1])] = match[2]
                continue
            match = REF_RE.match(line)
            if match and current is not None:
                current["refs"].append({
                    "block": int(match[1]), "sampler": int(match[3]), "id": int(match[4]),
                    "path": match[5], "rep": match[6], "kind": int(match[7]), "uv": int(match[8]),
                })
    sampler_textures = collections.Counter()
    no_grunge = []
    for item in items.values():
        for ref in item["refs"]:
            if ref["sampler"] >= GRUNGE_SAMPLER:
                key = (ref["block"], item["blocks"].get(ref["block"]), ref["sampler"],
                       ref["path"].split("\\")[-1] if ref["sampler"] == GRUNGE_SAMPLER
                       else ref["path"].split("_", 1)[-1][-TEXTURE_NAME_TAIL:],
                       ref["rep"], ref["uv"])
                sampler_textures[key] += 1
        if item["blocks"].get(SPRITE_BLOCK) == SPRITE_SHADER:
            if not any(r["block"] == SPRITE_BLOCK and r["sampler"] == GRUNGE_SAMPLER for r in item["refs"]):
                no_grunge.append(item)
    print("items", len(items))
    print("sampler>=1 refs (block, shader, sampler, texture, repetition, uv_set): count")
    for key, count in sampler_textures.most_common(TOP_REFS):
        print(" ", key, count)
    print("block-0 sprite items without a grunge ref:", len(no_grunge))
    flag_counts = collections.Counter()
    for item in no_grunge:
        for flag in item["flags"].split(" | "):
            flag_counts[flag] += 1
    print("  flags:", flag_counts.most_common(TOP_FLAGS))
    ranges = []
    for item in sorted(no_grunge, key=lambda entry: entry["id"]):
        if ranges and item["id"] == ranges[-1][1] + 1:
            ranges[-1][1] = item["id"]
        else:
            ranges.append([item["id"], item["id"]])
    print("  id ranges (first %d):" % MAX_RANGES,
          ["%d-%d" % (a, b) if a != b else str(a) for a, b in ranges[:MAX_RANGES]])
    paths = collections.Counter()
    for item in no_grunge:
        for ref in item["refs"]:
            if ref["block"] == SPRITE_BLOCK and ref["sampler"] == 0:
                name = ref["path"].split("\\")[-1]
                paths[re.sub(r"^\d+_", "", name)] += 1
    print("  base texture names:", paths.most_common(TOP_BASE_NAMES))


if __name__ == "__main__":
    main(sys.argv[1])
