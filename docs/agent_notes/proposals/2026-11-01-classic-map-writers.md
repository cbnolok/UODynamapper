# Classic map and statics writers: proposal (2026-11-01)

Option B is implemented (see the Decision at the end). It covers gap 1 of [cc_file_coverage.md](../cc_file_coverage.md):
writers for `mapN.mul`, `mapNLegacyMUL.uop`, `staidxN.mul` + `staticsN.mul`, their `x` editions, the dif files,
and how they fit the block cache and the converters.

All measurements come from a scratch real-file probe (not committed). It read every map, statics and dif file of
the five local clients under `/mnt/dati/_proj_local/_uo_clients/`:

- Classic (2024; map UOPs);
- Classic fp (map UOPs);
- 7.0.20 (map MULs);
- Mondain's Legacy;
- UO Gold.

None of the five has a `verdata.mul`, so nothing was measured for verdata.

## Summary

- **The modern files (7.0.20 to 2024) have one layout.** A writer that rewrites the whole file in that layout
  gives every shipped map and statics file back byte for byte. Each rule below held on every block and index
  entry of the three modern clients.
- **The old files (Mondain's Legacy, UO Gold) do not.** Their block headers and index `extra` fields hold
  arbitrary values. Mondain's Legacy `statics*.mul` has blocks out of order and dead bytes. A byte-for-byte
  rewrite therefore needs a "keep the layout" mode that stores those values and offsets.
- **Difs.** The client has not read the dif files since 6.0.14.3 (`ghidra-classic-facets`). The canonical
  client still ships `stadif*` files, but no `mapdif*`.
- **Uses.** There is no consumer today:
  - dynamapper renders `.uddp` packages and has no edit path;
  - udd-conv and `facet from-classic` convert one way only, from Classic.

  The writers would serve a future editor, a `.uddp` to Classic export, server map files, and the proof that the
  Classic to `.uddp` conversion is lossless.
- **Recommendation:** option B. Typed writers for maps and statics: the modern layout exact, the old layout kept
  as read, the difs baked into the base map rather than written. Effort is medium. The editor work (option C)
  waits until an edit path exists.

## Measured layout

### Map blocks (`mapN.mul`, `mapNLegacyMUL.uop`)

Each block is 196 bytes: a `u32` header, then 64 cells of `u16` tile id and `i8` z. The blocks are stored column
by column, as uocf reads them.

| | Classic, Classic fp, 7.0.20 | Mondain's Legacy, UO Gold |
|---|---|---|
| Block header | the block index, in every block | arbitrary: 54,415 distinct values in Gold map0's 393,216 blocks, 119,600 in Mondain's Legacy map0's 458,752; 1 equals the index |
| `mapN.mul` length | exactly blocks × 196 | same |
| Facet sizes (blocks) | map0/1 458,752 (896 columns of 512 blocks); map2 57,600; map3 81,920; map4 32,761; map5 81,920 | Gold map0 393,216 (6144 wide) |

`mapNLegacyMUL.uop` holds the same blocks, uncompressed, in entries `build/mapNlegacymul/%08d.dat` of 4,096
blocks, plus **one trailing block** (`UOP_MAP_TRAILING_BLOCKS`):

- When the facet fills whole entries (map0, map1, map3, map5), the trailing block is a 196-byte entry of its own,
  and it is a **copy of the last block**.
- Otherwise (map2: 257 blocks in the last entry; map4: 4,090), the trailing block ends the last entry, and it is
  **all zeros**.
- The entries are not stored in index order in the file: the data offsets do not ascend, except in Classic fp
  `map0x`. An unchanged package already saves back byte for byte (`SaveLayout::Preserve`,
  `uop_package_unchanged_save`). A writer that replaces the changed entries' payloads keeps everything else.

So in the modern clients a map block is fully determined by its cells and its index, and both containers can be
written exactly. In the old clients the header must be kept from the read block. uocf's `MapBlock` drops the
header today (`//header: u32, // unused`).

### Statics (`staidxN.mul` + `staticsN.mul`)

Each index entry is 12 bytes, one per map block in map order: `u32` lookup, `u32` length, `u32` extra. Each
record is 7 bytes: `u16` graphic, `u8` x, `u8` y, `i8` z, `u16` hue.

| | Classic, Classic fp, 7.0.20 (every file, `x` editions included) | Mondain's Legacy | UO Gold |
|---|---|---|---|
| Empty block | lookup, length and extra all `0xFFFFFFFF` | lookup and length `0xFFFFFFFF`; extra varies | same as Mondain's Legacy |
| `extra` of a non-empty block | always `0xAAAAAAAA` | arbitrary (119,145 distinct values in staidx0) | arbitrary (115,966 distinct) |
| Data order | in index order, back to back, from offset 0 | statics0: 2,151 in order, 114,519 backwards, 10 gaps; 325,675 dead bytes | in index order, back to back |
| Dead or shared bytes | none, nothing after the last block | dead bytes in statics0, 2, 3 and 4 | none |
| Records in a block | sorted by (y, x) in every multi-record block; within a cell the order is not by z (only about 9% of blocks are (x, y, z) sorted), so it must be kept | not sorted | not sorted |

So a modern statics pair is fully determined by each block's record list, in its order. A writer can produce it
exactly: an empty block becomes `0xFFFFFFFF` × 3; a non-empty block takes the running offset, the length and
`0xAAAAAAAA`; the records keep their order. The old files need their offsets and extras kept, or they are
rewritten in the modern layout, which is valid but not byte for byte.

### The `x` editions

The client picks `map0x`/`staidx0x`/`statics0x` (and the 1, 2 and 5 editions) only through the server flag
(`g_UseFacetVariants`, `ghidra-classic-facets`). The editions are separate files of the same layout:

| Client | `map0x` vs `map0` | `map1x` vs `map1` | `map2x`, `map5x` |
|---|---|---|---|
| Classic (2024) | identical | identical | identical |
| Classic fp | 13,325 blocks differ | identical | identical |
| 7.0.20 | 5,080 blocks differ | 89 blocks differ | absent |

The `staidx0x`/`staidx1x` editions differ from the base files in every client that has them (their data
lengths differ). `staidx2x`/`staidx5x` of the Classic clients have the base files' sizes and counts; their bytes
were not compared.
A writer treats an edition as one more file. Whether an edit goes to both editions is the caller's choice
(question 3).

### Difs and verdata

- **Dif files.** `mapdiflN` holds `u32` block ids and `mapdifN` the 196-byte blocks. `stadiflN` holds the ids,
  `stadifiN` 12-byte index entries, and `stadifN` the records, back to back in index order in every file.
- **Duplicate ids.** Ids repeat: Gold `mapdifl1` has 14,359 ids, of which 10,135 are distinct; the canonical
  `stadifl1` has 10,931 ids, of which 7,910 are distinct.
- **How the client applies them.** It maps each id to its record with `operator[]`, so a later duplicate wins.
  It applies only as many ids as the server's 0xBF/0x18 count. It has applied none since 6.0.14.3
  (`ghidra-classic-facets/results/classic_facets.md`, Q3).
- **Verdata.** It can also patch map and statics files (uocf's `MapPlane::with_verdata`,
  `StaticsReader::with_verdata`). There is no sample here to measure.

## What would use it

- **dynamapper.** It renders `.uddp` packages through its own read-only `MapPlane` (`dynamapper/src/core/maps.rs`),
  and nothing edits the world. A terrain or statics editor would need to save as Classic files: Classic clients
  and servers read `mapN`/`staidxN`/`staticsN`, or the map UOPs. The writers are its last step, so it would use
  them through an export from the edited `.uddp`, or by editing Classic files directly (question 6).
- **udd-conv.**
  - A Classic → `.uddp` → Classic round trip would prove that `cc_map`/`cc_statics` lose nothing. Today only the
    forward direction is tested.
  - Old data (Gold or Mondain's Legacy map + difs) could be baked into a modern map, which 7.x clients and
    servers read.
  - A future `facet to-classic` (EC `facetN.uop` → Classic) would end in these writers.
- **uocf's `MapPlane`.** It already has `block_as_mut` on its block cache, but nothing writes a block back, and
  an evicted block loses its edit. An editor on Classic files would need dirty tracking: dirty blocks are never
  evicted and are flushed through the writer.

## Options

| Option | Content | Effort | Byte for byte |
|---|---|---|---|
| A. Block patcher | Replace given map blocks in place (MUL offset or UOP entry), and rewrite a statics pair in the modern layout from an edited block list. No typed model kept; old headers and extras are lost on the blocks it rewrites. | small (2 to 3 days) | modern: yes; old: only untouched map blocks |
| B. Typed writers | `MapBlock` keeps its header. `ClassicMap::to_mul`/`to_uop` follow the trailing-block rule and keep the read entry layout. `ClassicStatics`: per-block record lists, written in the modern layout, or in the read layout (offsets and extras kept; a changed block is rewritten in place if it fits, else appended). Difs baked into a base map (later id wins, optional server count). Real-file round trips of every map, statics and dif file on the five clients. CLI: `uocf-cli map bake-difs` / `map write` and a KDL or CSV static patch. | medium (1 to 2 weeks) | modern: yes; old: yes in read-layout mode |
| C. Editing | B, plus dirty-block tracking in uocf's `MapPlane`, a `.uddp` → Classic export in udd-conv, and an edit layer in dynamapper. | large; depends on the editor design | as B |

Writing dif files is left out of every option. The client has ignored them since 6.0.14.3, and baking covers the
old data. Verdata writing is left out too: no sample, and it is pre-7.0.

## Recommendation

Do B:

1. Keep the map block header in `MapBlock`.
2. Add map writers (MUL, and UOP through the entry editor with `SaveLayout::Preserve`), with a real-file round trip
   of every map on the five clients.
3. Add the statics writer in the modern layout, with a round trip on the three modern clients. Add the read-layout
   mode, with a round trip on Mondain's Legacy and Gold.
4. Add dif baking, checked against uocf's own dif-applying readers on Mondain's Legacy and Gold.
5. Add the CLI.

Start C only when the user decides how dynamapper edits the world.

## Questions for the user

1. **Old clients.** Should the writers reproduce Mondain's Legacy and UO Gold files byte for byte (the read-layout
   mode, about a third of B's effort), or is a valid modern-layout rewrite of old files enough?
2. **Difs.** Bake them into the base map (recommended), also write dif files, or leave them out? Baking needs a
   choice of how many ids to apply, since the client applies only the server's count: all of them, or a given
   count?
3. **`x` editions.** Should an edit go to one file only (recommended: the caller names the edition), or to both
   editions of a facet at once?
4. **Statics order.** When a static is added, where does it go: after the other statics of its cell, keeping the
   (y, x) order of the shipped files (recommended), or wherever the caller puts it?
5. **Containers.** For a modern client, should the writer emit the client's own container only (UOP for 7.0.50
   and later, MUL before), or always both? Servers such as ServUO read either.
6. **Editor path.** Will dynamapper edit the `.uddp` packages and export to Classic, or edit Classic files
   through uocf's `MapPlane`? This decides whether C goes in udd-conv or in uocf.
7. **Priority.** Is there a consumer now (a server map, an editor milestone), or should B wait behind the other
   gaps?

## Decision (2026-11-01)

The user first deferred option B, then asked for it the same day, for a tool that converts the CC UOP and the EC
facets to MUL and back, and between clients with smaller maps. Option B is implemented with these answers:

1. **Old clients:** a valid modern-layout rewrite. Gold and ML maps come back with the block index as header and
   the statics in the modern layout; cells and statics are the same.
2. **Difs:** baked into the base map, every id applied, a later one winning (the client's rule). No dif writer.
3. **`x` editions:** an edit goes to one named edition (`--map-edition`, `--target-edition`).
4. **Statics order:** an added static goes after its cell's statics, keeping the (y, x) order.
5. **Containers:** a client gets its own container by default (`--container auto` from `--target-client`); the
   tool can write MUL, UOP or both.
6. **Editor path:** still open (option C).

What was built:

- uocf: `classic/map_write.rs` (`ClassicMapBlocks`: MUL and UOP, the trailing block rule, a template package for
  the shipped layout; `bake_map_difs`), `classic/statics_write.rs` (`ClassicStaticsBlocks`, `bake_static_difs`),
  `classic/world.rs` (`ClassicWorld`: load with difs, resize with fill or crop, ids beyond a tiledata),
  `classic/target_client.rs` (size table, Trammel-reads-map0, dif usage, container and tiledata of a client folder),
  `enhanced/facet_to_classic.rs` (EC or KR facet to Classic, with the losses), and the EC and KR facet builders
  taking a `ClassicWorld`.
- uocf-cli: `facet convert` (Classic MUL / Classic UOP / EC facet / KR facet, any direction; `--target-client` or
  `--size`; `--crop`; `--fill-land`/`--fill-z`; `--allow-lossy`), `facet patch-classic`, and `mul-convert` maps
  through the typed writer (`pack --uop-template-dir`).

Measured (`lib/uocf/tests/real_files/classic_map_writers.rs`, all `#[ignore]`):

- **MUL and UOP write-back.** 7.0.20: every map and statics file byte for byte. 2024: every map UOP (through MUL,
  with the shipped package as template) and statics byte for byte. fp: statics byte for byte; map0, map1 and map1x
  hold 1,076,787 / 960,540 / 960,540 stale bytes no entry or table uses, which any uocf UOP save zeroes (an
  unchanged save too), so they match an unchanged save, not the original. Gold, ML: the block headers differ only.
- **Difs.** Baked Gold and ML facets 0 to 2 equal the dif-applying readers (0 cells, 0 static blocks differ).
- **EC facet → Classic → EC.** Lossless but for delimiters that are not the ones the rule derives from the
  neighbours (stale copies; the builder rewrites them by the rule): facet0 13,884 squares, 0x 13,995, 1 22,848, 1x
  23,260, 2 and 2x 1,138, 3 198, 4 195, 5 and 5x 105,977. The Classic facets from the EC base editions equal the
  2024 client's files cell for cell and static for static (with the 2024 packages as template, map0, map4 and map5
  come out byte for byte); the 0x and 1x editions differ in content (38,821 and 35,970 cells).
- **KR facet → Classic → KR.** Squares whose material or subtype is not the shipped land map's: facet0 9,608,
  facet1 10,013, facet2 7, facet3 2,665, facet4 1,065; stale delimiters 23, 14, 0, 4, 0.
- **Classic → EC → Classic and Classic → KR → Classic.** Lossless on every facet of the 2024 and 7.0.20 clients.
- **Sizes.** UO Gold's Felucca and Trammel are 6144x4096 (Trammel is map0 with the facet-1 difs); ML's 7168x4096.
  The added strip of ML is land 0x244 at z 0 where it is empty, the fill a grow writes. Gold → 7168 → Gold is
  equal; ML → 6144 is refused (259,645 land cells and 21,204 statics outside) and drops them with `--crop`.

Open decisions: whether the stale delimiters should only warn (they are rebuilt by the rule, so the client sees
the same neighbours), and how to keep the KR materials (a sidecar, or the opt-in only).
