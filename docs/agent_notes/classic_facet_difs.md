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
