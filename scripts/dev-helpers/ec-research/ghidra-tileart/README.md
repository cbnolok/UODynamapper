# ghidra-tileart

Names the code in `UOSA.exe` (Enhanced Client) that loads and decodes `tileart.uop`, and the code that uses
the decoded tile definitions (`UOObjectDefinition`).

Findings: [results/uosa_tileart.md](results/uosa_tileart.md) covers the load path, the record format with
every field, a comparison with UOETE and the repo's `tileart.rs`, usage sites, all applied names and the
proposed repo corrections.

## NameTileartCode.java

GhidraScript, category `ECResearch`.

```
NameTileartCode.java OUTPUT_FILE [--apply]
```

- `OUTPUT_FILE` is created with CREATE_NEW; an existing file stops the run before any change.
- Without `--apply` the script only reports. With `--apply` it:
  - renames DEFAULT (`FUN_...`) functions with source ANALYSIS;
  - adds the tag `UO_TILEART`;
  - sets a plate comment `NameTileartCode: UO_TILEART <name> [(probable)]: <evidence>` when the comment is empty;
  - creates or updates the `/UOTileArt` structure types;
  - types the three default shader globals;
  - labels five globals.
- Names with another source type are never changed; they are listed under "Kept names".
- A name this script applied earlier (source ANALYSIS, tag `UO_TILEART`, plate comment written by the script
  for that exact name) is replaced when the proposed name changes; its plate comment is refreshed only if it
  is still the script's single-line text. These renames are listed under "Own names corrected".
- A second `--apply` reports `Changes: none`.

Run it through ghidra-mcp:

```
run_ghidra_script(program="UOSA.exe", script_name="<abs path>/NameTileartCode.java",
                  args="<scratch>/tileart_report.md --apply", timeout_seconds=1800)
```

### How functions are found

No fixed addresses are used. Each function is found from evidence that doesn't depend on its address:

| Evidence | Finds |
|---|---|
| String refs: `string_dictionary.bin`, `tileart.uop`, `Runtime\client`, `%08d_TileArt`, `%08d_LegacyTileArt`, `TileArtLegacy\`, `incorrect header check` | `UOStringDictionary::Load`, `UOAssets::MountPackages`, `UOAssets::ReadFileFromPath`, UI texture requests, `SpriteTextureFileLocation` ctors, zlib chain |
| Named function `UOVirtualFileOpen` | `UOP::HashFileName`, `HashString`, `HashLittle2` (seed 0xdeadbeef), `FindEntryIndex`, `LookupHash` |
| Immediate 0x06000000 (tile art asset type), most callers | `UOTileArt::GetObjectDefinition`, then `UOAssets::GetAsset` and its callees |
| RTTI vtables (`.?AV<class>@@` -> type descriptor -> complete object locator -> vftable) | slot functions of `UOObjectDefinition`, `UOObjectDefinitionBase`, both binary factories, `UOShaderDefinition`, appearance filters; constructors as vftable writers |
| Call structure | parser = callee of factory Create that calls `StreamRead`; its 8 stream-reading callees in call order are the sub-parsers; inserters are their low-caller callees |
| Immediate 0x80000 (UseNewArt) | `UOTileArt::BuildSpriteDescriptor` and its cache |
| Virtual-call offsets (`CALL [reg+off]` or `MOV reg,[reg+off]; CALL reg`) | `GetGraphicForAmount` user (+0xc4), `TilesConnect` (+0x114/+0x11c/+0x12c) |
| RTTI `UOObject` vtable slots +0x44, +0x6c..+0x78, +0xb0 | `UOObject::SetDirection`, `Set/GetBodyType`, `Set/GetAlternateAnimation`, `GetFlags1` |
| RTTI component vtables, slot +4, checked for the definition slots they call | `UOObjectComponentSprite::Initialize` (+0x88), `UOObjectComponentLighting::Initialize` (+0x40/+0x48/+0x4c) |
| Common writer of the `UOObjectComponentLighting` and `UOObjectComponentEffects` vftables | `UOObject::CreateComponents` |
| GetObjectDefinition caller with immediate 99999999, then its caller using +0xb4 and +0xb8 | `UOObject::GetObjectDefinition`, `UOObject::InitFromDefinition` |
| Callee of `UOObject::SetDirection` whose callers all use +0xe8 | `UOObject::NearestAllowedDirection` |
| GetObjectDefinition caller using +0x6c and +0x70; caller using exactly +0x100/+0x104/+0x110 | `UOPaperdoll::BuildImage`, `UOHouseCustomization::IsPlacementAllowed` |

Vtable slot functions referenced from more tables than their class and base vtables (identical-code
folding) are skipped and reported.

The report also lists every caller of `UOTileArt::GetObjectDefinition`, with the virtual-call offsets it
uses (mapped to `UOObjectDefinition` slot names) and the strings it references. Offsets are reported per
function, so some belong to other objects.

## tileart_variables.spec

Applied by `../ApplyVariableSpecs.java` after `NameTileartCode.java`, and before the effects, animation and
housing specs (see [the shared description](../README.md#variable-names-prototypes-and-structures-applyvariablespecsjava)):

```
run_ghidra_script(program="UOSA.exe", script_name="<abs path>/ApplyVariableSpecs.java",
                  args="<abs path>/ghidra-tileart/tileart_variables.spec <scratch>/tileart_vars.md --apply")
```

- Creates the shared types `/UO/SharedPtr`, `AssetBlob`, `AssetBlobSharedPtr`, `AssetKey`,
  `ObjectDefinitionSharedPtr`, `UOObject`, `HouseCell` and `UOHouseCustomization`, and makes three namespaces
  classes whose `this` is a structure.
- Sets 184 prototypes (316 named parameters, 8 globals). Seven tagged functions are left out: their
  parameters are not established or they are shared library code (`UOObjectDefinition::AddEffect`,
  `UOObjectDefinitionBase::GetPrimaryArtScale`, `UOTileArt::RequestTileArtTexture`,
  `UOTileArt::RequestLegacyTileArtTexture`, `SpriteTextureFileLocation::CopyConstruct`,
  `UOAssets::LoadAssetBlob`, `UOP::Decompress`).
- `UOEffects::ParseEffectDefinition` and `UOAssets::StreamRead` (stream in ECX, position in ESI, size in
  EDI) are typed here because the other areas call them.

### Notes

- Another agent named part of the UOP archive layer (`UOP::MypArchive_*`, `UOP::Decompress`,
  `UOP::OpenFilePtr`, `UOP::*_ctor`). This script finds the same functions and keeps those names.
- `NameTileartCode.java` does not change calling conventions or `this` types; `tileart_variables.spec`
  does, for the functions whose parameters are established.
