# ghidra-image-decoders

How `UOSA.exe` (Enhanced Client) picks the decoder for an image it loads from a UOP package
(`Texture.uop`, `LegacyTexture.uop`, `TerrainTexture.uop`, `Interface.uop`, `EffectTexture.uop`,
`SystemTextures.uop`, `Hues.uop`), and what each decoder checks in the header. Names the code of the texture
request chain, the asset loader configuration, the `assetmap.xml` parsers and the Gamebryo image readers that
were still unnamed. Also covers the Classic 7.0.103 client's `artLegacyMUL.uop` reads.

Findings: [results/uosa_image_decoders.md](results/uosa_image_decoders.md) has the per-package table
(decision point, decoder, header checks, failure behaviour, with addresses), the reader details, the asset
resolver path of tile textures and the consequences for a TGA/BMP writer.
[results/uosa_image_decoders_names.md](results/uosa_image_decoders_names.md) is the script report of a
second `--apply` (`changes: none`); the first `--apply` created 5 functions, renamed 35, tagged 35 and set
35 plate comments.

In short: the decoder is chosen by the extension of the path the client builds, and that path is the
UOP entry name. Nothing looks at the content first.

Earlier work used here: `ghidra-effects` (converter, `NiDX9SourceTextureData::Create`, effect textures and
the hue atlas), `ghidra-animation` (`UOHues::LoadHueBitmap`), `ghidra-tileart` (asset manager, texture
items, `SpriteTextureFileLocation::ConstructFromPath`), `ghidra-lua-names` (UI texture chain) and
`ghidra-source-names` (UOP path hashing).

## NameImageDecoderCode.java

GhidraScript, category `ECResearch`.

```
NameImageDecoderCode.java OUTPUT_FILE [--apply]
```

- Names fixed addresses, so it refuses any program whose SHA-256 is not the tested `UOSA.exe`
  (`5a5e9dc5...ebad50`).
- Before naming, each address is checked: a function exists there, and its stated evidence holds (vtable
  slot, callers, callees, constants, referenced strings). A failed check is reported as a mismatch and
  nothing is named.
- The Gamebryo reader vftables at 0x00ccc5ec (BMP), 0x00ccc5fc (DDS), 0x00ccc634 (SGI) and 0x00ccc654 (TGA)
  have no labels, so their slots are checked with `vtslot:ADDR:N`. When no function exists at an address
  that such a slot holds, `--apply` disassembles it and creates the function, then runs the other checks.
  This applies to `NiBMPReader::CanReadFile`, `NiBMPReader::ReadFile`, `NiDDSReader::CanReadFile`,
  `NiDDSReader::ReadHeader` and `NiSGIReader::ReadFile`.
- `OUTPUT_FILE` is created with CREATE_NEW, so an existing file stops the run before any change. Pass
  spaces in the path as `%20`.
- Without `--apply` the script only reports. With `--apply` it:
  - renames DEFAULT (`FUN_...`) functions with source ANALYSIS, creating namespaces as needed;
  - adds the tag `UO_IMAGE`;
  - sets the plate comment `NameImageDecoderCode: UO_IMAGE: <evidence>` when the comment is empty, and
    refreshes its own comment from an earlier run when the evidence has changed.
- Names with another source type are kept and reported. A name already used in the namespace is reported as
  a conflict. A second `--apply` reports `changes: none`.

Run it through ghidra-mcp, then save the program:

```
run_ghidra_script(program="UOSA.exe", script_name="<abs path>/NameImageDecoderCode.java",
                  args="<scratch>/image_decoder_names.md --apply")
save_program(program="UOSA.exe")
```
