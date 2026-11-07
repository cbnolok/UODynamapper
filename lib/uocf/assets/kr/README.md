# Generated KR land mapping

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

`KrLandMap::shipped()` adds the existing, explicitly unverified supplemental
Rust table to the verified binary routes, preserving conversion coverage.
The facets provide no stronger evidence for those 1,133 ids; they are not part
of this artifact. Ids absent from both sources still fail unless the caller
chooses an explicit override or fallback.

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
