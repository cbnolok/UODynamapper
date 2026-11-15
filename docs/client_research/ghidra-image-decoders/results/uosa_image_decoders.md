# Image decoders in UOSA.exe

Program: `UOSA.exe`, SHA-256 `5a5e9dc536c836d1bf10b6f86bb88f61a6ae078f519d4060d3bc47aecbebad50`.
Sources compared: Gamebryo 2.6 `CoreLibs/NiMain` (`NiDevImageConverter.cpp`, `NiTGAReader.cpp`,
`NiBMPReader.cpp`, `NiDDSReader.cpp`, `NiTexture.cpp`, `NiPixelData.cpp`) and `CoreLibs/NiDX9Renderer`
(`NiDX9SourceTextureData.cpp`). Data: the `_Ultima Online Enhanced` install, the `ec_kr_extract`
copies of `MainMisc.uop` (`data/assetmap.xml` of EC, KR and Classic) and `lib/uocf/assets/Dictionary.dic`.

Confidence: **confirmed** (read in code and checked on data), **high** (read in code, no counter-evidence),
**probable** (partial reading), **unknown**.

## Summary

- **The decoder is chosen by the file extension, and the extension is part of the UOP entry name.**
  Every image goes from a path through `NiSourceTexture::Create` to `NiDevImageConverter::ReadImageFile`,
  or through UO's own hue bitmap parser. `ReadImageFile` takes the path's extension (`_splitpath`) and asks
  each reader's `CanReadFile(extension)`, a plain `stricmp`. It opens the file only with a reader that
  accepts the extension. The UOP lookup hashes the full normalised path (lowercase, `\` to `/`) with the
  extension unchanged, so the extension the client asks for is the one in the entry name. **Confirmed.**
- No content sniffing is reachable. The D3DX path that would sniff (`D3DXGetImageInfoFromFileInMemory`)
  needs the load-direct hint, and UO's hint `g_UOTextureLoadDirectHint` 0x00e31e9b is never written (false).
  The other D3DX loader, `LoadScratchTextureD3DX` 0x00aecc3d, has no reference. **Confirmed.**
- The call site decides which extension is asked for. Tile and terrain textures take it from
  `data/assetmap.xml`, so the `.tga` in the tileart record path is dropped and `.dds` is used. Gumps use
  the `%08d.tga` format. UI XML, ManagedTextures, effect definitions and NIFs use the name as written.
  **Confirmed.**
- Content that does not match the extension's reader fails in `ReadHeader`. This happens for a DDS under a
  `.tga` name (bad image type) or a non-BMP under `.bmp` (no `BM`). `ReadImageFile` returns null and
  `NiDX9SourceTextureData::Create` substitutes Gamebryo's replacement image: magenta, failure code
  FILENOTFOUND. `NiD3DRenderer::Error` is an empty function in this build, so nothing is logged.
  **Confirmed** (code), **high** (the colour comes from the default callback, and UO installs no other).
- The anomalies found by the census have these effects in the client:
  - **TGA, trailing bytes:** harmless. The TGA reader reads the header, the palette and exactly `height`
    rows, then stops, so footers and the EC fp 32,784 zero bytes are never read.
  - **TGA, 0x0 header:** loads as a 1x1 texture whose texel is never written. It does not fail.
  - **Hue BMP file size field:** harmless. Hue BMPs are read by `UOHues::LoadHueBitmap`, which checks only
    the bit count, width and height. `NiBMPReader` ignores `bfSize` too.
  - **Classic `artLegacyMUL.uop` `.tga` names:** the `.tga` is only part of the hashed entry name. The
    Classic client reads these entries as MUL art.

## Per package

| Package (entry names) | Decision | Decoder | Header checks | On failure |
|---|---|---|---|---|
| `Texture.uop` (`build/worldart/%08d.dds`, WorldArt) | asset map: resource type 1 is packed. `Data\WorldArt\00000002_ankh.tga` gives id 2 (`atol` of the file name), and `Build/WorldArt/` + `%08d` + `.dds` (`assetmap.xml`) gives `.dds` | `NiDDSReader` (`.dds`) | `DDS ` magic (or byte-swapped), header size 0x7C | magenta replacement |
| `LegacyTexture.uop` (`build/tileartlegacy/%08d.dds`) | asset map: type 0x16 packed. `Data\TileArtLegacy\3.tga` gives `Build/TileArtLegacy/00000003.dds` | `NiDDSReader` | same | magenta |
| `TerrainTexture.uop` (`build/terraintexture/%08d.dds`) | asset map: type 8 (`Textures\` paths) packed, `Build/TerrainTexture/` + `.dds` | `NiDDSReader` | same | magenta |
| `Interface.uop` gumps (`data/interface/default/textures/gumpart/%08d.tga`) | call site: `MakeGumpTextureLocation` 0x00519700 formats `%08d.tga` | `NiTGAReader` (`.tga`, `.targa`) | no magic. Image type 1/2/3/9/10/11, pixel size 4 (palette only), 8, 15/16, 24 or 32. Bytes after the pixels are not read | magenta. A 0x0 header gives a 1x1 texture with an undefined texel |
| `Interface.uop` other textures (`data/interface/...`, `.dds` and `.tga`) | name as written in the UI XML (`<Texture file>`, `UITexture_CreateSourceTexture` 0x00911470) or in ManagedTextures.xml (`MakeManagedTextureLocation` 0x00519570) | the reader of that extension | per reader | magenta |
| `EffectTexture.uop` (`data/effects/*.tga`, `*.nif`) | name as written in the `.ems` script or in the NIF's `NiSourceTexture` (`ParticleSystemManager::LoadTexture` 0x008a8630, NIF streaming) | `NiTGAReader`. `.nif` goes to `NiNIFImageReader`, which accepts a NIF holding exactly one `NiPixelData` | per reader | magenta |
| `SystemTextures.uop` (`data/systemtextures/*.tga`) | `SimpleTextureFileLocation` with the literal `Data\SystemTextures\missing.tga` and `toolarge.tga` (static initialisers at 0x00c0d4e0, globals 0x00e33e94 and the next one). Slot 2 is always "direct", so the path is used as written | `NiTGAReader` | as above | magenta |
| `Hues.uop` atlas (`build/hues/hues.dds`) | literal `Build\Hues\Hues.dds` through `SpriteTextureFileLocation`. Type 1, but the path holds no WorldArt, Textures, TileArtLegacy or TileArtEnhanced, so it loads direct | `NiDDSReader` | DDS magic and size | magenta |
| `Hues.uop` bitmaps (`data/definitions/hues/hue%04d.bmp`) | fixed loader: `UOHues::LoadHueTables` 0x004274a0 calls `UOHues::LoadHueBitmap` 0x004271f0 through `UOAssets::ReadFileFromPath`. No image converter is involved | UO's own parser | only `biBitCount == 32`, `biWidth == 256`, `biHeight == 2`. Pixels at `bfOffBits`. `bfType`, `bfSize`, `biSize`, planes and compression are not read. The blob length is not checked (2,048 bytes are read from `bfOffBits`) | header mismatch: logged, the hue gets no table |
| KR `TileArtLegacy.uop` (UOKR.exe) | not traced. KR's `assetmap.xml` has the same type 22 line (`Build/TileArtLegacy/`, `.dds`), and UOKR.exe contains the Gamebryo reader extension strings (`.targa` at 0x00aa3d58), so the same rule is **probable** | - | - | - |
| Classic `artLegacyMUL.uop` (`build/artlegacymul/%08i.tga`) | 7.0.103 client: `FUN_0056b4b0` formats `build/` + `art` + `legacymul/` + `%08i.tga` for file ids 6 (art) and 7 (gumpart), `.dat` for the others. The generic MUL index lookup `FUN_0056e000` / `FUN_0056b5d0` turns the name into {entry, size}, and the bytes go to the same decoders as `art.mul` records | raw MUL art | none: the `.tga` is only hashed | as for a missing MUL record |

## The texture request chain

```
location (SpriteTextureFileLocation / WorldArt.. / Gump.. / Managed.. / SimpleTextureFileLocation)
  TextureManager::RequestTexture          0x0058d8e0  vftable 0x00cb39b4 slot 1; location slot 2 picks the path
    TextureManager::LoadTextureDirect     0x0058d600  key FNV-1a(lowercase path) / 0x11000000; UOP::FileExists(path)
      UOTextureData::UOTextureData        0x004ccc80  NiSourceTexture::Create(path, g_UOTextureLoadDirectHint = 0)
    TextureManager::RequestTextureAsset   0x0058d430  UOResourceRequest {slot 3 id, slot 4 type << 24}
      g_UOAssetManager resolvers          (UOAssetMap::ParsePatternMap 0x00a74200 mappings from Data/assetmap.xml)
      UOTextureFactory::Create            0x004cce20  (out, id, type, resolved path) -> UOTextureData(path)
NiSourceTexture::Create                   0x0068a8b0
  NiDX9SourceTextureData::Create          0x00770b90  hint false -> converter, never LoadTextureFile 0x00771af0
    NiDevImageConverter::ReadImageFile    0x006942d0  readers in order NIF, TGA, SGI, DDS, BMP; CanReadFile(ext)
      NiTGAReader / NiDDSReader / NiBMPReader / NiSGIReader / NiNIFImageReader ReadFile
    null -> NiTexture::RendererData::DefaultTextureFailureCallback 0x0068da60 (0x80000005), NiD3DRenderer::Error (empty)
```

- **Location slot 2, `SpriteTextureFileLocation::IsDirectLoad` 0x0058d270** (also WorldArt). It returns
  true (direct) when the location's type is not packed (`g_UOAssetManager + 0x160 + type * 4 != 1`).
  For a packed type it returns true only when the path contains `WorldArt\ref`, or contains none of
  `WorldArt`, `Textures\`, `TileArtLegacy` and `TileArtEnhanced`. Other locations return 1 (direct).
  `SpriteTextureFileLocation::ConstructFromPath` 0x0058d130 sets the type: 8 for `Textures\`, 0x16 for
  `TileArtLegacy\`, 0x1a for `TileArtEnhanced\`, else 1. **Confirmed.**
- **Location slot 3, `SpriteTextureFileLocation::GetAssetId` 0x0058d310.** `atol` of the file name part.
  **Confirmed.**
- **Packed or raw per type.** `UOAssets::Initialize` 0x004a9150 first marks 0x10, 0x12 and 0x19 packed.
  `UOAssets::SetDefaultPackedTypes` 0x00a6db3b then marks 1, 3, 6, 8, 9, 0xb, 0xc, 0xd, 0x13..0x17 and
  0x1a. `UOAssets::LoadAssetLoaderConfig` 0x00a6db93 would override this from `AssetLoaderConfig.xml`
  (`<loader resourcetype format="raw|packed">`, `SetTypeFormat` 0x00a6dadb). No install ships that file:
  its name hash is in no EC package, and it is not loose in either EC folder. So the WorldArt, Textures,
  TileArtLegacy and TileArtEnhanced types are packed. **Confirmed.**
- **The packed name.** `assetmap.xml` (`MainMisc.uop`, read by `UOAssetMap::Load` 0x00a74868) has
  `patternmap` lines with a `packed` file format. In EC, KR and Classic they are identical for these
  types:

  | Type | Directory | Padding | Suffix |
  |---|---|---|---|
  | 1 | `Build/WorldArt/` | 8 | `.dds` |
  | 8 | `Build/TerrainTexture/` | 8 | `.dds` |
  | 22 | `Build/TileArtLegacy/` | 8 | `.dds` |
  | 26 (not in KR) | `Build/TileArtEnhanced/` | 8 | `.dds` |

  The name is directory + id with `padding` digits + suffix. Example: `build/tileartlegacy/00000003.dds`
  is in `LegacyTexture.uop`, and `data/tileartlegacy/3.tga` is in no package. **Confirmed.**
- The `raw` lines (`Data/WorldArt/`, suffix `*`, `patternMatch="1"`) serve the unused raw mode.

## Readers (Gamebryo 2.6, binary matches the source)

The reader vftables are unlabelled. Each holds four slots:

| Reader | vftable | CanReadFile | ReadFile | ReadHeader |
|---|---|---|---|---|
| BMP | 0x00ccc5ec | 0x006d3140 `.bmp` | 0x006d3160 | 0x006d2de0 |
| DDS | 0x00ccc5fc | 0x006d3d20 `.dds` | 0x006d48f0 | 0x006d3810 |
| SGI | 0x00ccc634 | 0x006d5290 (`.sgi .rgb .rgba .int .inta`) | 0x006d5300 | 0x006d5040 |
| TGA | 0x00ccc654 | 0x006d64e0 `.tga .targa` | 0x006d6520 | 0x006d60d0 |
| NIF | 0x00ccc1d8 | 0x006ccde0 `.nif` | 0x006cce00 | 0x006cceb0 |

- **`ReadImageFile` 0x006942d0.** The `NiDevImageConverter` constructor 0x00693cf0 adds BMP, DDS, SGI, TGA
  and NIF with AddHead, so they are tried as NIF, TGA, SGI, DDS, BMP. For each reader whose `CanReadFile`
  accepts the extension, `ReadImageFile` opens the file through `NiFile` (UO's `UONdlFile`) and calls
  `ReadFile`. A failure moves on to the next reader that accepts the extension; for these readers there
  is none. An unknown extension (`.png`, for example) gives null. **Confirmed.**
- **TGA (`NiTGAReader`).**
  - There is no magic.
  - Image type: 1, 2, 3, 9, 10 or 11, else it fails.
  - Pixel size: 4 (colour-mapped only), 8, 15/16, 24 or 32. Another value leaves the previous file's
    bytes-per-pixel in the shared reader, so the result is undefined.
  - It skips the ID field, honours descriptor bit 5 (top-to-bottom) and ignores the other descriptor
    bits and the origin fields.
  - Alpha only at 32 bits.
  - It reads exactly `height` rows (RLE packets through `LoadRLERow`) and never looks at the rest of
    the file: no footer, no extension area, no length check.
  - A 0x0 header gives `NiPixelData(0, 0)`, which `NiPixelData::NiPixelData` 0x0068cad0 clamps to
    1x1, and no row is read.
  - **Confirmed** (binary ReadHeader against the source), **high** (no trailing read).
- **DDS (`NiDDSReader`).** `ReadHeader` 0x006d3810 requires the `DDS ` magic, also byte-swapped, and a
  header size of 0x7C. **Confirmed.**
- **BMP (`NiBMPReader`).** `ReadHeader` 0x006d2de0:
  - requires `bfType == 0x4D42`;
  - reads `bfSize`, the reserved fields and `biSizeImage`, and ignores them;
  - rejects `biSize == 4`;
  - accepts bit counts 4, 8, 24 and 32;
  - rejects compression 1 and 2 (RLE);
  - treats a negative height as top-down.

  `ReadFile` 0x006d3160 skips `bfOffBits - 54 - palette` bytes before the rows. **Confirmed.** No shipped
  `.bmp` goes through it: every `.bmp` entry is a hue bitmap (EC 293 + 72 + 2,635 = 3,000).
- **Hue bitmaps (`UOHues::LoadHueBitmap` 0x004271f0).**
  - The table is the second stored row, at `bfOffBits + 0x400`. Entry 256 is the first texel of the
    first stored row.
  - The checks are listed in the table above. A height of -2 (top-down) is rejected.
  - **Confirmed.**
- **Failure.** `NiDX9SourceTextureData::Create` 0x00770b90 in this build does not re-open the file to
  tell why it failed. It always asks for `FILENOTFOUND` (0x80000005, at 0x00770c8b) and calls
  `NiD3DRenderer::Error` 0x0067b1d0, which is a single `RET`.
  `NiTexture::RendererData::DefaultTextureFailureCallback` 0x0068da60 then makes a magenta image. UO
  never sets another callback: the only writes to the pointer 0x00da874c are Gamebryo's "if null, set
  the default". **Confirmed.**
- **UO's own fallback.** The tile material builder `FUN_0045cd80` loads
  `Data\SystemTextures\missing.tga` (0x0045ddf8, 0x0045e42e) when a request returns no texture data,
  for example when the direct path finds no file. That differs from a decode failure, which still
  returns texture data (magenta). **Probable.**

## Consequences for a TGA or BMP writer

1. **Write the format the name's extension names.** The extension picks the reader and the content is
   never sniffed. A DDS stored under a `.tga` gump name, or a TGA under a `build/worldart/*.dds` name,
   shows as magenta. The tile texture names (`build/{worldart,tileartlegacy,terraintexture}/%08d.dds`)
   must hold DDS, whatever extension the tileart record's source path has (`Data\TileArtLegacy\3.tga`).
2. **TGA.**
   - Allowed: image type 2 or 10 (24/32-bit), type 3 (8-bit grey) or a colour-mapped type.
   - Descriptor bit 5 sets the row order.
   - Alpha only at 32 bits.
   - Footer, extension area and padding are optional and ignored. The writer may keep the original
     ending, but the client does not need it.
   - A 0x0 image is accepted but yields an undefined 1x1 texel. To blank a gump, write a real 1x1
     image with alpha 0. Keep the 0x0 header only to reproduce the shipped bytes.
3. **Hue BMP.** Write 32 bpp, width 256, height +2 (bottom-up), the pixels at `bfOffBits`, and at least
   `bfOffBits + 2048` bytes. Put the table in the second stored row. The client does not check `bfType`,
   `bfSize`, `biSize` or the compression, so the shipped 525 is harmless. A writer should still store
   the true size (2,102 for the 54-byte header) for other tools.
4. **Classic `artLegacyMUL.uop`.** Write MUL art records under the `.tga` names. The name is a hash
   key, not a format.
5. **DDS.** Keep the magic and the 124-byte header (`dds_writer` already does).

Repo impact: `image_census.rs` and `dds_writer.rs` need no rule change. The census classifies by
content, which is the right input for a writer. Section 12 of `lib/uocf/src/enhanced/ENHANCED_CLIENT_FILES.md`
now says which mismatches matter to the client. One possible extra, not done: `replace_dds_entry` could
warn when the entry's known name has another extension than `.dds`. No shipped entry has a DDS under
another extension.

## Names added (`NameImageDecoderCode.java`, tag `UO_IMAGE`)

- Functions created at vftable slots: `NiBMPReader::CanReadFile` 0x006d3140, `NiBMPReader::ReadFile`
  0x006d3160, `NiDDSReader::CanReadFile` 0x006d3d20, `NiDDSReader::ReadHeader` 0x006d3810,
  `NiSGIReader::ReadFile` 0x006d5300.
- Gamebryo readers and failure path:
  - `NiBMPReader::ReadHeader` 0x006d2de0;
  - `NiDDSReader::NiDDSReader` 0x006d3760, `NiDDSReader::ReadFile` 0x006d48f0;
  - `NiSGIReader::NiSGIReader` 0x006d4e80, `NiSGIReader::ReadHeader` 0x006d5040;
  - `NiTGAReader::CanReadFile` 0x006d64e0, `NiTGAReader::ReadFile` 0x006d6520;
  - `NiTexture::RendererData::DefaultTextureFailureCallback` 0x0068da60, `NiD3DRenderer::Error`
    0x0067b1d0;
  - `LoadScratchTextureD3DX` 0x00aecc3d (unreferenced).
- UO:
  - texture requests: `TextureManager::RequestTexture` 0x0058d8e0, `LoadTextureDirect` 0x0058d600,
    `RequestTextureAsset` 0x0058d430;
  - locations: `SpriteTextureFileLocation::IsDirectLoad` 0x0058d270, `GetAssetId` 0x0058d310;
  - texture data: `UOTextureData::UOTextureData` 0x004ccc80, `UOTextureFactory::Create` 0x004cce20;
  - asset types: `UOAssets::SetDefaultPackedTypes` 0x00a6db3b, `LoadAssetLoaderConfig` 0x00a6db93,
    `SetTypeFormat` 0x00a6dadb, `SetAllTypesFormat` 0x00a6db20, `HasRawMapping` 0x00a6da73,
    `HasPackedMapping` 0x00a6da8a;
  - asset map: `UOAssetMap::Load` 0x00a74868, `ParsePatternMap` 0x00a74200, `ParseBitmaskMap`
    0x00a74549, `ParseSimpleMap` 0x00a73f5e;
  - UI textures: `MakeGumpTextureLocation` 0x00519700, `MakeManagedTextureLocation` 0x00519570,
    `UITexture_CreateSourceTexture` 0x00911470.

The Classic 7.0.103 functions above (`FUN_0056b4b0`, `FUN_0056b5d0`, `FUN_0056e000`) were only read, not
renamed.
