# Classic facet resolution and dif files

- Every reader of a Classic facet (udd-conv `resolve_classic_map_source`, `uocf-cli facet from-classic`, the
  real-file tests) goes through `uocf::classic::facet_sources::ClassicFacetSources::resolve`. Do not look up
  `mapN.mul` / `staidxN.mul` / dif files per call site.
- UO Gold and Mondain's Legacy ship no `map1.mul`/`staidx1.mul`/`statics1.mul`: Trammel is `map0` + `staidx0` +
  `statics0` with `mapdif1`/`mapdifl1` and `stadif1`/`stadifl1`/`stadifi1` applied. The resolver does this
  automatically (warns), whatever the dif options. `MapPlane::for_facet` keeps the file's own map index for UOP
  entry names (`build/map0legacymul/...`).
- Dif lookup files list block ids with duplicates (Gold `mapdifl1`: 14359 entries, 10135 distinct); the last entry
  wins, as in the clients.
- A `stadifiN` entry with an invalid lookup (0xFFFFFFFF) empties its block; it does not fall back to the base
  statics. Gold `stadif1` has 4792 such entries and empties 180 Felucca blocks that held statics.
- Gold Trammel: 6144x4096, 4875 land blocks and 6381 static blocks differ from Felucca, 3,160,101 statics
  (Felucca 3,138,822). `lib/uocf/tests/real_files/classic_facet_difs.rs` checks this (`UO_PRE7_CLIENT_DIR`).
- `verdata.mul` file ids 0..2 are `map0`/`staidx0`/`statics0` only; the resolver applies verdata to facet 0 files
  only (also to a Trammel built from them).
- 7.0+ Classic clients ("_Ultima Online Classic") still ship `stadif0..2` but no `mapdif`; applying them is opt-in.
- Map editions: `ClassicFacetSources::resolve` takes a `uocf::map_edition::MapEdition`. `Variant` reads the facet's
  `x` files (`mapNxLegacyMUL.uop` with the standard `build/mapNlegacymul/` entry names, or `mapNx.mul`, plus
  `staidxNx.mul`/`staticsNx.mul`), the alternate edition the server selects with the 0xA9 flag 0x8000. It is not a
  patch layer and never falls back: a facet of `CLASSIC_VARIANT_FACETS` (0, 1, 2, 5) without its `x` map is an
  error naming the files (7.0.20 ships only 0x/1x as MUL; 7.0.50+ ships 0x/1x/2x/5x); facets 3 and 4 have no
  variant and read their standard files, as the client does. Difs are applied to whichever edition is selected,
  with the same rules; verdata patches the standard map0/staidx0/statics0 only. `map_file_names` lists standard
  names only (it used to include the `x` names as a UOP fallback). `from_files` infers the edition from the map's
  file name. The options: `--map-edition standard|variant` (udd-cli map/statics/radar pack commands,
  `uocf-cli facet from-classic`), the udd-conv-gui world tab selector, `ClassicPatchOptions::map_edition`.
