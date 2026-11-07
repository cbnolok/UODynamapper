# Generated KR land mapping

`mappings.bin` contains 1,556 canonical Classic -> KR land routes derived from
all 69,746,240 on-map squares in the original KR facets 0-4. Sector padding and
the 1,133 unverified supplemental routes are excluded. The generated routes
match `KrLandMap::shipped_verified()` exactly.

Regenerate from the repository root:

```sh
cargo run -p uocf-devtools-cli --features dev-tool-tests -- kr-land-map \
  --client-dir '/path/to/UO - Old KR' \
  --output-dir lib/uocf/assets/kr --verify-shipped
```

Every facet 0-4 is required; missing or invalid source data fails generation.
`--verify-shipped` also requires the routes to match the existing verified Rust
reference table. Omit that check only when intentionally examining another
source bundle. Output is deterministic for the same facet contents.

`sources.csv` records each source's dimensions, sectors, bytes and on-map square
count. `sources.sha256` identifies the original packages used for this artifact;
refresh it when intentionally changing the source bundle. `observations.csv`
retains every observed `(classic_land, material, subtype)` pair and its square
count, including alternatives. Material and subtype are selected independently
by highest count for each Classic id; ties choose the smaller value. This is a
canonical route, not a lossless reconstruction of every original KR square.

The converter currently uses its compiled tables. This artifact does not yet
change conversion defaults or add a binary loader. Ids absent from the binary
need a separately reviewed override or explicit fallback.

## Binary format (KRLM version 1)

All integers are unsigned and little endian. The header is 12 bytes:

| Offset | Type | Value |
| --- | --- | --- |
| 0 | 4 bytes | ASCII `KRLM` |
| 4 | u16 | Version: `1` |
| 6 | u16 | Record size: `5` |
| 8 | u32 | Record count: `1556` |

The header is followed by that many packed records: `u16 classic_land`,
`u16 material`, `u8 subtype`. Records are strictly ascending by Classic id,
with no duplicates, padding or trailing bytes. Total size: 7,792 bytes.
This is unrelated to the removed .NET BinaryFormatter dictionaries.

For source research and conversion limits, see
[KR facet mappings](../../src/kr/facet/KR_FACET_MAPPINGS.md).
