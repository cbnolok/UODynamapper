# Classic client facets: wrap, file ids, variants and map patches

Builds, hashes and how to reproduce: [../README.md](../README.md). Names in `UOClassicFacet::` come from
`NameClassicFacetCode.java`. The confidence labels mean:

- **traced**: read in the decompiled code or the assembly of that build;
- **bytes**: decoded from the file without analysis;
- **inferred**: not traced; deduced from structure.

## The facet record

Every build has a hard-coded `.rdata` table of 48-byte records. It has 5 records up to 6.0.14.3 and
6 records (Ter Mur added) from 7.0.20. Traced in all four imported builds; bytes in the other three.

| Offset | Field | Readers |
|---|---|---|
| +0x00 | `char *name` ("Felucca", ...) | `FacetIndexFromName` (strstr), `GetFacetName` |
| +0x04 | u32 width | `GetFacetWidth` → `SetFacet` |
| +0x08 | u32 height | `GetFacetHeight` → `SetFacet` |
| +0x0c | u32 wrap_width | `GetFacetWrapWidth` → `SetFacet` → `g_WrapWidth` |
| +0x10 | u32 wrap_height | `GetFacetWrapHeight` → `SetFacet` → `g_WrapHeight` |
| +0x14 | u32 statics file id | `GetFacetStaticsFileId` |
| +0x18 | u32 staidx file id | `GetFacetStaidxFileId` |
| +0x1c | u32 mapdif file id | Gold/ML/5.0.9.1 only (`GetFacetMapdifFileId`); **no reader from 6.0.14.3 on** |
| +0x20 | u32 stadif file id | same |
| +0x24 | u32 stadifi file id | same |
| +0x28 | u32 map file id | `GetFacetMapFileId` |
| +0x2c | u32 felucca_rules | `CurrentFacetHasFeluccaRules` |

### The six ids

The six ids are indexes into the client's file name table. Each entry is a fixed name plus open/cache
fields. The stride is 56 bytes up to 7.0.20 and 60 bytes from 7.0.50; in 7.0.103 the entry is
0x00710900 + id·0x3c and the name is at +4. The names are static strings. No build makes the `x` suffix
with sprintf: the `x` variants are separate entries of the name table (`map0xLegacyMUL.uop` id 50,
`staidx0x.mul` id 51, `statics0x.mul` id 52, ... in 7.0.103), and only the second facet table points at
them. Traced.

### felucca_rules (+0x2c)

The flag is 1 only for Felucca, in every build. `CurrentFacetHasFeluccaRules` returns it for the current
facet. It returns 1 on every facet when `g_SiegeRules` is set (0xA9 flags bit 2, 0x4).

The only consumer is the PvP confirmation. The check (7.0.103 0x0050aeb0) needs flag && the "query
criminal actions" option (0x00742ce9, from the config file or the options gump) && a target with
notoriety hue 0x59 (innocent). Then `ConfirmCriminalAction` 0x0050acf0 shows "This may flag you
criminal!". 7.0.103 has a raw-index accessor of +0x2c at 0x005a0540, but nothing calls it and Ghidra
made no function there. Traced.

## Q1: wrap_width / wrap_height

Values in every build: Felucca and Trammel have wrap 5120 × 4096. Every other facet has wrap equal to its
size. Felucca and Trammel are 6144 wide in Gold and 7168 in ML and later.

Meaning (traced in 7.0.103; the same globals are read the same way in Gold, `BuildLandTileWrapped`
0x004e9df0):

- **The wrap region.** [0, wrap_w) × [0, wrap_h) is a torus. The region x ≥ wrap_w is flat: for Felucca
  and Trammel that is x ≥ 5120, the dungeons and T2A strip.
- **Movement.** `MovePlayerWrapped` 0x00596d50: when the player x < wrap_w, the new x and y are taken
  modulo wrap_w and wrap_h. Every world object is then shifted by ±wrap (0x00596ba0), so the view stays
  continuous. Outside the wrap region, nothing wraps.
- **Distance.** `WrappedTileDistance` 0x00593ba0 and `WrappedTileDistanceXY` 0x00593c20 return 9999 when
  the two points are on opposite sides of x = wrap_w. Otherwise they return the Chebyshev distance; in the
  wrap region each axis uses min(d, wrap − d). Callers: 0x0044aec0, 0x005fabe0, 0x005a7e70 (range and
  visibility tests).
- **Tile build / rendering.** `RebuildTilesAroundPlayer` 0x00596840 fills every missing cell around the
  player:
  - player x < wrap_w: `BuildTileWrapped` 0x00596710, cell coordinates taken modulo the wrap rectangle;
  - otherwise: `BuildTileUnwrapped` 0x005965f0. A cell with x < wrap_w or y ≥ wrap_h becomes land 0x244
    (the black void tile). So from the dungeon strip you never see the wrap region.
- **Block cache.** The block cache window, 0x0044fb60, also recentres modulo the wrap.
- **Radar.** `Radar_RebuildAroundPlayer` 0x0052b180 is called from the tile rebuild when the radar is
  open.
  - In the wrap region it wraps coordinates the same way.
  - In the flat region, cells at x < wrap_w, x ≥ width, y < 0 or y > height take colour index 0x8001
    (void).
- **Other readers.**
  - 0x0059c5b0 wraps the probe coordinates of a walk/z test.
  - 0x005bca70 wraps a pathfinding target near the seam ("Can't get there (follow)").
  - 0x004c1e70 clamps a rectangle to the wrap (vtable method, no direct caller).
  - Land altitude 0x00595f40 wraps by the full width and height, not the wrap.
- **Pathfinding.** 0x005bca70 shifts the target by ±wrap_w/±wrap_h when the player and target are near
  opposite edges, so a path crosses the seam. Traced for the shift, inferred for the full pathfinder.

Server interaction: none. The wrap is client-side only and is chosen by the 0xBF/8 facet index
(`SetFacet`).

## Q2: the two table copies (7.0.x)

- `g_UseFacetVariants` (7.0.103 0x00a58d7e, 7.0.20 0x00a1bcde) is written in one place,
  `ApplyCharListFlags` (7.0.103 0x005640b0, 7.0.20 0x00559c20). That function stores each bit of the 0xA9
  u32 flags in its own byte (0x00a58d70.. in 7.0.103). Bit 15 (0x8000) goes to `g_UseFacetVariants` and
  bit 2 (0x4) to `g_SiegeRules`. Its only caller is `Handle_0xA9_CharList` (0x0047f460 / 0x0047dfd0).
  That handler reads the characters, then the cities (index byte, 32-byte name, 32-byte area, 6 × u32),
  then the u32 flags. Traced. Gold's `ApplyCharListFlags` 0x004ca750 keeps bits 0..7 only: **Gold has no
  bit 15 and no second copy**. ServUO calls bit 15 `NewFeluccaAreas` (naming not verified in a client).
- **Copy selection.** Every table accessor tests `g_UseFacetVariants` and reads `g_FacetTableVariants`
  instead of `g_FacetTable` (7.0.103 0x006b75c8 / 0x006b7478; 7.0.20 0x006851d8 / 0x00685088). Traced.
  **This is the only mechanism that picks map0 vs map0x.** No file-existence test or fallback chooses a
  variant. A missing variant file then fails like a missing base file. Inferred from the open path; the
  open itself was not traced to the error.
- **How the copies differ** (bytes): only the file ids differ. Sizes, wrap, dif ids and felucca_rules are
  equal.

  | Facet | 7.0.20 copy B | 7.0.50 and 7.0.103 copy B |
  |---|---|---|
  | Felucca | map0x / staidx0x / statics0x .mul | map0xLegacyMUL.uop / staidx0x / statics0x .mul |
  | Trammel | map1x / staidx1x / statics1x .mul | map1xLegacyMUL.uop / staidx1x / statics1x |
  | Ilshenar | base | map2xLegacyMUL.uop / staidx2x / statics2x |
  | Malas, Tokuno | base | base |
  | Ter Mur | base | map5xLegacyMUL.uop / staidx5x / statics5x |

  This matches EC's swap set {0, 1, 2, 5} from 7.0.50 on.
- **Ter Mur's dif ids.** In every 7.x table, both copies, Ter Mur's mapdif/stadif/stadifi ids name
  `mapdif4`/`stadif4`/`stadifi4` (Tokuno's). This is harmless because nothing reads them.
- **Map presence probe.** `ProbeLegacyMapUops` 0x0059ffb0 (7.0.103) builds a bit mask of the existing
  `map%uLegacyMUL.uop`, u = 0..15. It does not probe map1: bit 1 copies bit 0. 0x005a1c50 sends bytes 0
  and 1 of the mask to the server when entering the world. It never probes the `x` files. Traced for the
  probe; the packet's purpose is inferred.
- **Mapdif/stadif in 7.0.x.**
  - The patch counts still arrive with 0xBF/0x18, and `ReloadDifLists` still loads the lists.
  - But `StadifBlockLookup` 0x0056cf50 and `MapdifBlockLookup` 0x0056cfe0 (7.0.103) have **no caller**.
    7.0.20 has the same dead pair: `StadifBlockLookup` 0x00561850 and `MapdifBlockLookup` 0x005618e0. No
    code reads record +0x1c..+0x24. (An earlier pass called 0x00561850 the mapdif lookup; it reads the
    stadif maps 0x00a2e2c8. Ghidra had not made a function at 0x005618e0; it does now.)
  - So the difs are never applied from 6.0.14.3 on, and the variant flag does not interact with them.
  - Traced: the imm32 scans of the record fields find no other reader, and the xrefs of the lookups are
    empty.

## Q3: UO Gold, Mondain's Legacy, 5.0.9.1: Trammel from map0 + difs

### Layout

The layout is the same 48-byte record, 5 records, no second copy (bytes, all three; traced in Gold).

- **Gold.** Felucca and Trammel are 6144 × 4096, wrap 5120 × 4096, and **both use map0 / staidx0 /
  statics0**. Felucca's dif ids are mapdif0/stadif0/stadifi0 and Trammel's are mapdif1/stadif1/stadifi1.
- **ML folder (2005) and 5.0.9.1.** Same, with width 7168.
- **6.0.14.3 and later.** Trammel uses map1 / staidx1 / statics1 (bytes and traced).

So up to 5.0.x, Trammel = map0 + the mapdif1/stadif1 patches, and Felucca = map0 + the mapdif0/stadif0
patches.

### Code (Gold, traced)

- **The 0xBF handler.** `Handle_0xBF_GeneralInfo` 0x0041ff90:
  - sub 8: `SetFacet`;
  - sub 0x11 (legacy): 4 × u32 (statics map0, statics map1, land map0, land map1);
  - sub 0x18: u32 map count, then per map {u32 statics patch count, u32 land patch count}. Gold keeps
    up to 5 maps and 7.0.103 up to 6 (`if (i < 6)`).

  Both subcommands then call `LoadDifLists` 0x004d0cb0 (7.0.103: `ReloadDifLists` 0x0056e8a0 →
  `LoadStadifLists` 0x0056e5c0 + `LoadMapdifLists` 0x0056e700).
- **The counts.** They are BSS, so 0 until the server sends 0xBF/0x18 or 0x11. **Without that packet no
  patch is applied.**
- **The loader.** For each map it opens `stadifl%d.mul` then `mapdifl%d.mul` and reads u32 block numbers.
  Entry i maps block → dif record i in a std::map (`g_StadifBlockMaps` 0x00768eb8, `g_MapdifBlockMaps`
  0x00768e60). It assigns through operator[], so **a later duplicate block overwrites an earlier one**.
  It stops at **min(file entries, server count)**.
- **Land.** `MapdifBlockLookup` 0x004d1230 returns the dif record or −1. Every land reader uses it:
  - `LoadLandBlock` 0x004eaa70;
  - `GetLandGraphicAt` 0x004ea9d0;
  - `GetLandZAt` 0x004ea700;
  - `BuildLandTileWrapped` 0x004e9df0;
  - the cell builder 0x004e9ac0.

  When the result is ≥ 0, the reader reads the **mapdif** file (+0x1c id) at that record index; else the
  **map** file (+0x28) at the block.
- **Statics.** `StadifBlockLookup` 0x004d1140 is used by `LoadStaticsBlock` 0x004eaac0 and 0x004e9720.
  When the result is ≥ 0, the index entry comes from **stadifi** (+0x24) at the record and the data from
  **stadif** (+0x20). Else from staidx/statics.
- **The current facet.** `DAT_007e1c08` (Gold) selects the facet's row. So Felucca uses mapdifl0 and
  Trammel mapdifl1.
- **ML folder and 5.0.9.1** (bytes): their mapdif/stadif/stadifi accessors (5.0.9.1 0x0050b990 /
  0x0050b9c0 / 0x0050b9f0; ML 0x005009e0 / 0x00500a10 / 0x00500a40) have 7, 2 and 2 direct callers. The
  map accessor also has 7. That is the Gold pattern, so the difs are applied there too. Inferred from the
  call counts, not decompiled.
- **6.0.14.3** (traced): `LoadStadifLists` 0x004e0eb0 and `LoadMapdifLists` 0x004e0ff0 (5 maps) still
  run from `ReloadDifLists` 0x004e1160 ← `Handle_0xBF_GeneralInfo` 0x00430ae0. But the only other
  references to their maps (0x007aee70 / 0x007aeeac) are the static constructors at 0x00589360/0x005893a0.
  No lookup exists and no record +0x1c..+0x24 reader exists. The difs are dead from 6.0.14.3 on.

## Q4: EC and KR

See [../../ghidra-facets/results/uosa_facets.md](../../ghidra-facets/results/uosa_facets.md#facetnx-variants-and-the-classic-x-maps)
and [uokr_facets.md](../../ghidra-facets/results/uokr_facets.md#no-facetnx-variants-in-kr).

- EC uses the same server flag (0xA9 bit 15), but swaps packages (`facetN.uop` ↔ `facetNx.uop`, N = 0, 1,
  2, 5). It does not switch tables.
- EC has no wrap.
- KR has no variants.

## Address index

| Item | Gold | 6.0.14.3 | 7.0.20 | 7.0.103 |
|---|---|---|---|---|
| g_FacetTable | 0x0056f918 | 0x005a6db0 | 0x00685088 | 0x006b7478 |
| g_FacetTableVariants | none | none | 0x006851d8 | 0x006b75c8 |
| g_UseFacetVariants | none | none | 0x00a1bcde | 0x00a58d7e |
| g_SiegeRules | 0x00757285 | | 0x00a1bcd1 | 0x00a58d71 |
| ApplyCharListFlags | 0x004ca750 | | 0x00559c20 | 0x005640b0 |
| Handle_0xA9_CharList | 0x00427d20 | | 0x0047dfd0 | 0x0047f460 |
| Handle_0xBF_GeneralInfo | 0x0041ff90 | 0x00430ae0 | 0x00479440 | 0x00477f80 |
| SetFacet | 0x004eab40 | 0x004face0 | 0x00585de0 | 0x00593eb0 |
| g_WrapWidth / g_WrapHeight | 0x005cd66c / 0x005cd670 | | | 0x0074229c / 0x007422a0 |
| dif list loader | 0x004d0cb0 | 0x004e0eb0, 0x004e0ff0 | 0x00562300, 0x00562440 | 0x0056e5c0, 0x0056e700 |
| MapdifBlockLookup | 0x004d1230 (used) | none | 0x005618e0 (dead) | 0x0056cfe0 (dead) |
| StadifBlockLookup | 0x004d1140 (used) | none | 0x00561850 (dead) | 0x0056cf50 (dead) |
| file name table (names) | 0x0059bd1c, stride 56 | 0x005e1324, 56 | 0x006d77dc, 56 | 0x00710904, 60 |

Bytes only: ML table 0x0057d260 (file names 0x005aa4d4), 5.0.9.1 table 0x00590610 (0x005bfbc4), 7.0.50
tables 0x006a2338 / 0x006a2488 (0x006f820c, stride 60).

## Recommendations for the Classic map resolver

These are not implemented; `lib/**` is owned by another agent.

Implemented since: the facet size patcher (`lib/uocf/src/client_exe/classic_facets.rs`, `uocf-cli exe
show|patch|check-data`) locates the table copies, the file name table, the startup copy of facet 0 (the
`.data` `{wrap_width, wrap_height, width, height}` block `SetFacet` fills, initial values = Felucca) and the
two x = 5120 wrap seam compares by structure, and patches sizes and wraps in both copies. See
`lib/uocf/src/enhanced/ENHANCED_CLIENT_FILES.md`, "Facet sizes in the executable".

1. **Pre-6.0 clients.** Detect them by the tiledata/era, or by the absence of `map1.mul`:
   - Trammel (map 1) is derived as map0/staidx0/statics0 with the mapdif1/mapdifl1 land patches and the
     stadif1/stadifl1/stadifi1 statics patches;
   - Felucca (map 0) is map0 plus the mapdif0/stadif0 patches, when they exist.

   Apply the patches like the client:
   - patch i of `mapdiflN` replaces block `mapdiflN[i]` with record i of `mapdifN`;
   - a later duplicate wins;
   - the patch count is min(file length / 4, record count).

   With no server the count is the file length (the real client needs 0xBF/0x18).
2. **6.0.14.3 and later.** Map 1 is `map1`/`staidx1`/`statics1`, and the client ignores every dif file.
   Applying difs to these clients' data would diverge from what the client shows.
3. **`x` variants (7.0.x).**
   - 7.0.50 and later: `map0x/1x/2x/5x` (LegacyMUL.uop) with `staidx`/`statics` `0x/1x/2x/5x`;
   - 7.0.20: only `0x`/`1x` (`.mul`).

   The server selects them with 0xA9 flag 0x8000. They are an alternate facet source, never a patch
   layer, and they never fall back to the base. Expose them as an explicit option, not an automatic
   preference.
4. **Map size and wrap.** Use the client's record:
   - Gold Felucca/Trammel width 6144;
   - ML and later 7168;
   - wrap 5120 × 4096 for both.

   x ≥ 5120 does not wrap, and the client renders cells at x < 5120 as void when viewed from there.
5. **Ter Mur's dif ids.** Ter Mur's dif ids point to `mapdif4`/`stadif4`. Do not treat them as Ter Mur
   patches.

## Dif usage checker

`lib/uocf/src/client_exe/classic_difs.rs` (`inspect_classic_difs`, part of `ClientExeInspection::difs`,
printed by `uocf-cli exe show` and the patcher GUI) answers, per kind, whether a Classic `client.exe`
applies the mapdif / stadif patches. It reads the bytes only; it never runs the client.

- **Loader.** The `PUSH imm32` of the `mapdifl%d.mul` / `stadifl%d.mul` string, with a `MOV r32,imm32`
  (not ESP) of a writable VA in the 0x40 bytes before it and an `ADD r32,imm8` of that register in the
  0x200 bytes after it: the immediate is the map array, the ADD its map size (12 in every build). The
  static constructor `PUSH count; PUSH 12; PUSH array` (MSVC 6 guarded registrations put an `OR CL,AL`
  after the count) gives the map count: 5 up to 6.0.14.3, 6 in 7.0.
- **Lookups.** Every other u32 in `.text` inside the array is classified. A no-base SIB operand
  `[index*4 + disp32]` is a read of a facet's map (`MOV ECX,[ESI*4+maps]` up to 5.0.9.1, two per lookup;
  `LEA ESI,[EAX*4+maps]` in 7.0). Its function starts at the nearest 16-aligned address after a `RET`,
  `RET imm16` or `JMP` and its padding (NOP in MSVC 6, INT3 in MSVC 8).
- **Callers.** `CALL rel32` and `JMP rel32` to the start, the callers of a thunk (a `JMP rel32` that is
  itself a function start), and any stored u32 equal to the start in any section.
- **Status.** `Applied` when a lookup has a caller (the patches then apply once the server sends
  0xBF/0x18); `LoadedNotApplied` when the lists load but no lookup exists or none has a caller and every
  reference is explained; `Absent` with no format string; `Unknown` when the string is never pushed, the
  loader has another shape, or a reference is not explained (e.g. a direct `MOV EAX,[maps+12]`).

Every reference to the arrays in every build below is explained (loader, constructor / destructor,
indexed read). The ignored test `lib/uocf/tests/real_files/classic_dif_usage.rs` checks the table.

| Build | Land / statics | Loader | Maps (land / statics) | Lookups (land / statics) | Callers |
|---|---|---|---|---|---|
| UO Gold | applied / applied | 0x004d0cb0 | 0x00768e60 / 0x00768eb8, 5 | 0x004d1230 / 0x004d1140 | 7 / 2 CALL |
| Mondain's Legacy folder | applied / applied | 0x004dbf00 | 0x00777da8 / 0x00777e00, 5 | 0x004dc480 / 0x004dc390 | 7 / 2 CALL |
| 5.0.9.1 | applied / applied | 0x004e5240 | 0x00793a80 / 0x00793ad8, 5 | 0x004e57c0 / 0x004e56d0 | 7 / 2 CALL |
| 6.0.14.3 | loaded, never applied | 0x004e0ff0 / 0x004e0eb0 | 0x007aeeac / 0x007aee70, 5 | none | |
| 7.0.20.0 | loaded, never applied | 0x00562440 / 0x00562300 | 0x00a2e310 / 0x00a2e2c8, 6 | 0x005618e0 / 0x00561850 | 0 |
| 7.0.29.97 (`client_tc.exe`) | loaded, never applied | 0x005650b0 / 0x00564f70 | 0x00a319e8 / 0x00a319a0, 6 | 0x00563990 / 0x00563900 | 0 |
| 7.0.50.0 | loaded, never applied | 0x005643f0 / 0x005642b0 | 0x00a51778 / 0x00a51730, 6 | 0x00562cd0 / 0x00562c40 | 0 |
| 7.0.103.0 | loaded, never applied | 0x0056e700 / 0x0056e5c0 | 0x00a6b720 / 0x00a6b6d8, 6 | 0x0056cfe0 / 0x0056cf50 | 0 |
| 7.0.115.0 (Classic fp folder) | loaded, never applied | 0x00577fa0 / 0x00577e60 | 0x009b9100 / 0x009b90b8, 6 | 0x00575fe0 / 0x00575f50 | 0 |

The Gold, 6.0.14.3, 7.0.20 and 7.0.103 rows agree with Ghidra (decompiled lookups and their xrefs). The
ML and 5.0.9.1 rows confirm the call-count inference of Q3 from the lookups themselves.

A converter should apply the dif files only to a kind the client applies; for 6.0.14.3 and later it should
leave them out (recommendation 2).
