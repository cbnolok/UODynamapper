# KR land mappings

`mappings.bin` contains 1,556 canonical Classic -> KR land routes derived from
all 69,746,240 on-map squares in the original KR facets 0-4. Sector padding and
the 1,133 unverified supplemental routes are excluded.
`KrLandMap::shipped_verified()` reads this binary directly; the former duplicate
1,556-row Rust array has been removed. The original facet data is the source of
truth, with the binary as its generated conversion artifact.

Regenerate from the repository root:

```sh
cargo run -p uocf-devtools-cli --features dev-tool-tests -- kr-land-map \
  --client-dir '/path/to/UO - Old KR' \
  --output-dir lib/uocf/assets/kr --verify-shipped
```

Every facet 0-4 is required; missing or invalid source data fails generation.
`--verify-shipped` also requires the routes to match the bundled binary.
Omit that check only when intentionally examining another
source bundle. Output is deterministic for the same facet contents.

`sources.csv` records each source's dimensions, sectors, bytes and on-map square
count. `sources.sha256` identifies the original packages used for this artifact;
refresh it when intentionally changing the source bundle. `observations.csv`
retains every observed `(classic_land, material, subtype)` pair and its square
count, including alternatives. Material and subtype are selected independently
by highest count for each Classic id; ties choose the smaller value. This is a
canonical route, not a lossless reconstruction of every original KR square.

The binary is embedded into the library, so installed tools do not need an
external mapping file. `KrLandMap::from_binary` also supports explicitly supplied
KRLM data and rejects invalid headers, length mismatches, duplicate ids and
unsorted records. `to_binary` is shared by the generation command.

`mappings_unverified.bin` holds the 1,133 supplemental routes in a separate
5,677-byte KRLM file. These are an exact transfer of the former
`BUILDER_KR_LAND_MAP_SUPPLEMENT` Rust array in commit `66467be9`
(`lib/uocf/src/kr/facet/kr_land_map.rs`), originally taken from Manawydan's
`get_tile_map` table. They have no confirming source evidence in the shipped
KR facets. Moving them into a binary does not verify them. Its SHA-256 is
`20fe78dc1491fc7b2ae9f5db886078306b4aaff8f3ac1bd891c00e51cc02e11f`.

`KrLandMap::shipped_verified()` loads only trusted `mappings.bin`.
`KrLandMap::shipped()` loads both binaries, preserving conversion coverage and
giving verified routes precedence. The two files currently have no Classic ids
in common. Ids absent from both sources still fail unless the caller chooses
an explicit override or fallback. Both former Rust arrays have been removed.

The facet generation command writes only the trusted binary and its observation
and source CSV files. It never merges or rewrites `mappings_unverified.bin`.

## Binary format (KRLM version 1)

All integers are unsigned and little endian. The header is 12 bytes:

| Offset | Type | Value |
| --- | --- | --- |
| 0 | 4 bytes | ASCII `KRLM` |
| 4 | u16 | Version: `1` |
| 6 | u16 | Record size: `5` |
| 8 | u32 | Record count: `1556` (trusted) or `1133` (unverified) |

The header is followed by that many packed records: `u16 classic_land`,
`u16 material`, `u8 subtype`. Records are strictly ascending by Classic id,
with no duplicates, padding or trailing bytes. Total size is `12 + count * 5`
bytes: 7,792 trusted or 5,677 unverified. Both files use the same format; their
separate filenames and documented provenance distinguish their trust levels.
This is unrelated to the removed .NET BinaryFormatter dictionaries.

For source research and conversion limits, see
[KR facet mappings](../../src/kr/facet/KR_FACET_MAPPINGS.md).
