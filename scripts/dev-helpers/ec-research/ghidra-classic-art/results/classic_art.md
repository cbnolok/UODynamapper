# Classic art records

Program `classic_7.0.103.0_client.exe` (7.0.103.0), names applied by `NameClassicArtCode.java`.

## The land / static split

**Confirmed.** `UOClassicArt::GetArtRecord` 0x005d7c00 takes an art id: below 0x4000 it loads the record through
`UOClassicArt::LoadLandRecord` 0x00543320, otherwise through `UOClassicArt::LoadStaticRecord` 0x005433b0. Both read
MUL file id 6 (`art`) with `UOClassicFiles::GetMulRecord` 0x0056e000, which serves the MUL pair or the
`artLegacyMUL.uop` entry (`build/artlegacymul/%08i.tga`, see `ghidra-image-decoders`). Nothing in the record says
which kind it is: the id does. Item graphics reach it as graphic + 0x4000 (`UOClassicArt::GetItemArtRecord`
0x0044b340, **probable**: + 0x4001 / + 0x4002 for the stack amount variants, graphics above 0xffdc refused).

## The static header

**Confirmed** for the fields read: `UOClassicArt::SetStaticScreenBox` 0x0044c400 (**probable** name) reads the u16
width at +4 and the u16 height at +6 of the record of item id + 0x4000. The u32 at +0 is not read there; in every
shipped static it is the byte length of the rows after the lookup table (measured, uocf
`classic_art_encoding` real-file test).

| Address | Name | Evidence |
|---|---|---|
| `0x005d7c00` | `UOClassicArt::GetArtRecord` | `id < 0x4000` picks the land loader, else the static loader |
| `0x00543320` | `UOClassicArt::LoadLandRecord` | `GetMulRecord(6, id, &size)` in its own static slot |
| `0x005433b0` | `UOClassicArt::LoadStaticRecord` | `GetMulRecord(6, id, &size)` in its own static slot |
| `0x0056e000` | `UOClassicFiles::GetMulRecord` (probable) | file id, entry index, out size; MUL pair or LegacyMUL UOP |
| `0x0044c400` | `UOClassicArt::SetStaticScreenBox` (probable) | width +4, height +6 of `GetArtRecord(item + 0x4000)` |
| `0x0044b340` | `UOClassicArt::GetItemArtRecord` (probable) | `GetArtRecord(graphic + 0x4000 / 0x4001 / 0x4002)` |

## Shipped encoding (measured)

On every record of the Classic, Classic fp, 7.0.20, Mondain's Legacy and UO Gold clients: land records are 2,048
bytes (1,012 diamond colors, then 24 bytes, zero or old memory); static rows are `(x offset, run)` pairs and colors
ended by `(0, 0)`, each row padded to 4 bytes with one `u16` of zero or old memory, a row equal to an earlier one
sharing its offset. Details: `docs/agent_notes/cc_file_coverage.md`, "Classic art encoding".
