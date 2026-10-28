# tileart.uop in UOSA.exe

Program: `UOSA.exe` (Enhanced Client), image base 0x400000,
SHA-256 `5a5e9dc536c836d1bf10b6f86bb88f61a6ae078f519d4060d3bc47aecbebad50`.

All names below were applied by `NameTileartCode.java --apply` (tag `UO_TILEART`, source ANALYSIS,
plate comment `NameTileartCode: UO_TILEART ...`). "(probable)" marks names whose evidence is indirect.
Record statistics come from the client's `tileart.uop` (37756 records), parsed with the layout below; every
record parses to its exact end.

## 1. Load path

```
UOAssets::Initialize                    0x004a9150  creates g_UOAssetManager (0x00e37220)
  UOAssets::MountPackages               0x00a6bb5a  "tileart.uop", "string_dictionary.uop", ... (about 33 packages)
    UOP::MountArchive                   0x0098a341  CreateFileA/GetFileSizeEx, reads the MYP table
  UOStringDictionary::Load              0x00a6d84a  "Build/StringDictionary/string_dictionary.bin"
    UOAssets::ReadPackagedFile          0x00a6d4b7  whole virtual file into a Blob
      UOP::OpenFilePtr (other agent)    0x00989f55  -> UOVirtualFileOpen 0x00989b72
    UOAssets::StreamRead                0x00a6fe55  ECX = stream, ESI = position, EDI = size; memcpy + bounds error
  (then AssetLoaderConfig.xml and preloading of asset types 1..26)

UOTileArt::GetObjectDefinition(tileId)  0x005bda40  41 callers; key type 0x06000000
  UOAssets::GetAsset                    0x00a6cfc8  (manager, out, id, 0x06000000, 0)
    UOAssets::ResolveAsset              0x00a6d346  error with type byte (key >> 24) on failure
      UOAssets::TryResolvers            0x00a749ed  resolver list, vtable +0
    UOAssets::LoadAssetBlob             0x00a6d38f  packaged (location type 1) or loose file
      UOAssets::ReadPackagedFile        0x00a6d4b7
      UOAssets::ReadFileFromPath        0x00a6d5e4  cwd checks "Runtime\client", "Runtime", "RuntimeLegacy\Unified"
    UOAssets::CreateAssetFromBlob       0x00a72861  factory vtable +4 (Create)
      UOObjectDefinitionBinaryBaseFactory::Create  0x0084466e  shared by both factory vtables
        UOObjectDefinitionBinaryFactory::NewInstance  0x004c9d30  alloc 0x100, base ctor, UOObjectDefinition vftable
          UOObjectDefinitionBase::UOObjectDefinitionBase  0x00842434  defaults
        UOTileArt::ParseObjectDefinition 0x0084478f  record parser (section 2)
```

UOVirtualFileOpen (0x00989b72) internals:

| Step | Function | Notes |
|---|---|---|
| Hash name | `UOP::HashFileName` 0x0098d487 | lowercases A-Z, then `UOP::HashString` 0x0098ed3d (strlen) |
| Hash | `UOP::HashLittle2` 0x0098ba02 | Jenkins lookup3 `hashlittle2`, seed 0xdeadbeef + length |
| Find entry | `UOP::FindEntryIndex` 0x0098d460 | per mounted archive (vector at 0x00e35238), -1 if missing |
| Lookup | `UOP::LookupHash` 0x0098dbe9 | open addressing, 12-byte slots {u32 lo, u32 hi, u32 index} |
| Entry | `UOP::MypArchive_GetEntry` 0x0098db06 (other agent) | 0x22-byte entry: i64 offset, u32 header len, u32 compressed, u32 decompressed, u64 hash, u32 adler32, u16 compression |
| Read | `UOP::MypArchive_ReadEntry` 0x0098d57c (other agent) | compression 1 -> `UOP::Decompress` 0x0098d6d3 (zlib `uncompress`, inflate 0x009a5d80); 0 -> memcpy |
| File object | `UOP::MythMypInputFile_ctor` 0x0098e5df, `UOP::MythMemoryInputFile_ctor` 0x0098e672 (other agent) | memory input file over the decompressed buffer |

The tile id is the asset id. The resolvers (configured from AssetLoaderConfig.xml) map it to the package
entry; no tile art entry-name format string exists in the executable, so the name is built by the
resolver configuration rather than by tile art code.

## 2. Record format (version 4)

Byte packed, little endian. The parser checks that the u16 version equals 4.

### 2.1 Fixed part (`/UOTileArt/TileArtRecordFixed`, 0x7d bytes)

| File off | Type | Name | Memory | Meaning and evidence | Conf. |
|---|---|---|---|---|---|
| 0x00 | u16 | version | - | must be 4, else error | high |
| 0x02 | u32 | name_index | +0x04 std::string | string dictionary index; `GetName` +0xc | high |
| 0x06 | u32 | tile_id | +0x20 | `GetTileId` +0x10 | high |
| 0x0a | u8 | has_world_art | +0x54 | `HasWorldArt` +0x1c; 1 exactly when shader block 0 (WorldArt) is present (15271 records); selects block 0 and +0x58 in `GetPrimaryTextureLocation`/`GetPrimaryArtScale` | high |
| 0x0b | u8 | uses_enhanced_tileart | +0x55 | `UsesEnhancedTileArt` +0x20; `BuildSpriteDescriptor` mode 2 (block 2, TileArtEnhanced, window x1.5); 0 in all records | high (code), unused in data |
| 0x0c | f32 | world_art_scale | +0x58 | default 1.0; `GetWorldArtScale` +0x2c; sprite scale in mode 1. Values 1.0, 1.4, 5, 4, 0.8 ... | high |
| 0x10 | f32 | legacy_art_scale | +0x5c | default 1.5; `GetLegacyArtScale` +0x30; sprite scale in modes 0/2. 1.5, or 1.0 on 1113 records | high |
| 0x14 | u32 | field_60 | +0x60 | `GetField60` +0x34; part of `HasLegacyArtData`; 0 in all records | unknown |
| 0x18 | u32 | old_id | +0x64 | default 0xffffff; `GetOldId` +0x78; equals the tile id on 2236 records, another id on 141 (103->102, 129->128, 722->720) | medium |
| 0x1c | f32 | rotation | +0x68 | `GetRotation` +0x7c; 0/90/180/270; read by `BuildSpriteDescriptor` only when flags1 bit 34 is set | high |
| 0x20 | u32 | body_type | +0xe4 | default 400; `GetBodyType` +0xb4. 400 = human male, 666 = gargoyle male; creature placeholders (ids >= 10000000) hold the body id | medium |
| 0x24 | u8 | field_e8 | +0xe8 | `GetFieldE8` +0xb8; set on 30 records (some weapons, e.g. 2301, and creatures); meaning unknown | unknown |
| 0x25 | f32 | light_scale | +0x6c | default 1.0; `GetLightScale` +0x40; 2.0 on 866 records (windows, arrow loops) | probable |
| 0x29 | u32 | light_field_70 | +0x70 | `GetLightField70` +0x44; 0 in all records | unknown |
| 0x2d | f32 | light_offset_x | +0x74 | `GetLightOffsetX` +0x48; non-zero only with block 3; e.g. -5, -1.35 | probable |
| 0x31 | f32 | light_offset_y | +0x78 | `GetLightOffsetY` +0x4c; e.g. -4, -3 | probable |
| 0x35 | u32 | house_join_bits | +0x7c | stored through vtable +0x14c; tested bit by bit (slots +0x100..+0x138); bits 6..13 are house-customization connectors (section 4.6) | high (6..13), low (others) |
| 0x39 | u64 | flags1 | +0x80 | `HasFlags1`/`GetFlags1`; classic TileData flags plus EC bits | high |
| 0x41 | u64 | flags2 | +0x88 | `HasFlags2`/`GetFlags2`; `UseNewArt` (0x80000) is tested here | high |
| 0x49 | u32 | facing | +0x90 | default 1; `GetFacing` +0x88. 1 x34276, 3 x2915, 2 x411, 0 x150, 4 x4 | medium |
| 0x4d | i32[6] | enhanced_window | +0x24 | `GetEnhancedImageWindow` +0x14: x_start, y_start, x_end, y_end, x_offset, y_offset; the sprite builder adds 1 to the end values (inclusive) | high |
| 0x65 | i32[6] | legacy_window | +0x3c | `GetLegacyImageWindow` +0x18; scaled x1.5 in mode 2 | high |

### 2.2 Variable part (in file order)

| Part | Format | Parser | Memory |
|---|---|---|---|
| props1 | u8 n, n x {u8 key, u32 value} | `UOTileArt::ParseProperties1` 0x00844e6b -> `SetProperty1` 0x0084299e | map +0xc8 |
| props2 | u8 n, n x {u8 key, u32 value} | `UOTileArt::ParseProperties2` 0x00844f05 -> `SetProperty2` 0x00842a38 | map +0xd4 (separate map) |
| stack aliases | u32 n, n x {u32 amount, u32 tile id} | `UOTileArt::ParseStackAliases` 0x00844dda -> `AddStackAlias` 0x00842ae3 | map* +0xe0, allocated on first use |
| appearance filters | u32 n, n x {u8 subtype, data} | `UOTileArt::ParseAppearanceFilters` 0x00845060 | vector* +0xbc |
| - subtype 0 | u32 m, m x {u32 body, u32 appearance} | `ParseAppearanceFilterByOwnerBodyType` 0x0084ac82 | `AppearanceFilterByOwnerBodyType` |
| - subtype 1 | u8 gender, u32 appearance | `ParseAppearanceFilterByOwnerGender` 0x0084ad2d | `AppearanceFilterByOwnerGender` |
| - subtype 2 | u32 m, m x {u32 body, u32 animation appearance} | `ParseAnimationAppearanceFilterByOwnerBodyType` 0x0084ada8 | `AnimationAppearanceFilterByOwnerBodyType` |
| - other | error | | |
| sitting | u8 flag (0/1, other = error); if 1: 6 x u32 | `UOTileArt::ParseSitting` 0x00845168 | shared_ptr +0xc0 via `SetSitting` +0x150 |
| radar color | u8 r, g, b, a | `UOTileArt::ParseRadarColor` 0x008452f4 | +0x94, flag +0x98 via `SetRadarColor` +0xe0 |
| shader block x4 | see below | `UOTileArt::ParseShaderBlock` 0x008456a3 | blocks at +0x9c/+0xa4/+0xac/+0xb4 via setters +0xa4..+0xb0 |
| effects | u8 n, n x effect definition | `UOTileArt::ParseEffects` 0x00844f9f -> `UOEffects::ParseEffectDefinition` 0x008415ae | vector +0xf0 via `AddEffect` +0x144 |

Shader block (`UOShaderDefinition`, 0x54 bytes in memory):

| Field | Format | Memory | Meaning |
|---|---|---|---|
| present | u8 | - | 0 = keep the default block |
| field_04 | u8 | +0x04 | `GetField04`; 0 in all records |
| shader name | u32 string index | +0x08 std::string | `UOSpriteShader` (14608), `UOStaticTerrainShader` (341), `UOWaterShader` (322) in block 0 |
| item count | u8 | vector +0x24 | |
| item | u32 path index, u8 source_kind, f32 stretch, u32 sampler_index, u32 uv_set | `UOTileArtTextureItem` 0x2c | see below |
| technique | u32 n, n x i32 | int[4] +0x34 | n is 0 or 4; `SetTechniqueValue` (index < 4) |
| tweaks | u32 n, n x f32 | float[4] +0x44 | n is 0 or 4; `SetTweakValue` |

Texture item (`UOTileArtTextureItem`, ctor `UOTileArtTextureItem::Init` 0x004491a0):

| Field | Memory | Meaning | Conf. |
|---|---|---|---|
| path | +0x00 std::string | `Data\WorldArt\...`, `TileArtLegacy\...`, `TileArtEnhanced\...` | high |
| source_kind (u8) | +0x1c | material builder 0x0045cd80 branches on 0 vs 2 to pick the load path; 0 in all records | medium |
| stretch (f32) | +0x20 | != 1.0 sets address mode wrap (bits 12-13 = 3) | high |
| sampler_index (u32) | +0x24 | 0..3 per block; no code use found | low |
| uv_set (u32) | +0x28 | ORed into the low bits of the sampler flag word; 0/1 | medium |

Blocks: 0 = WorldArt (present on 15271 records; implies `has_world_art`), 1 = TileArtLegacy (always present),
2 = TileArtEnhanced (always present, 2 items), 3 = light texture (`*_light_window.tga`, 1042 records).
Defaults for blocks 1-3 come from `g_UOTileArtDefaultLegacyShader` 0x00e346c4,
`g_UOTileArtDefaultEnhancedShader` 0x00e346d4 and `g_UOTileArtDefaultLightShader` 0x00e346cc.

Sitting (`UOTileArtSitting`, 6 x u32): field 0 is a probable direction bitmask (values 1, 2, 4, 5, 6, 8, 10,
15); field 2 is 16/20/22; field 3 (0, 1, -6, 6, 3, -2) shifts the sprite window when field 0 is 1 or 8
(`BuildSpriteDescriptor`, slot +0xf4); fields 4/5 look like y offsets.

Effects: `ParseEffectDefinition` reads a u32 type key, asks each factory in `g_UOEffectDefinitionFactories`
(0x00e3737c) through vtable +0 whether it handles the key, then creates the effect through vtable +8;
an unknown key is an error. 313 records have 1 effect, 16 have 2. The effect body format belongs to the
effect classes (probably the same records as EffectDefinitionCollection.uop) and is not decoded here.

Property keys (props1): 6 = equipment layer (`FUN_005b3ce0` with Wearable), 8 = appearance
(`GetAppearance` +0xbc), 9 = race (`GetRace`, default 1), 10 = gender (`GetGender`, default 0). props2
uses its own map and accessors (+0x6c/+0x70); its keys are not interpreted here.

### 2.3 Flags used by the code

| Flag | Bit | Where | Use |
|---|---|---|---|
| Foliage 0x20000 | flags1 17 | `BuildSpriteDescriptor` | alpha sprite (`|2`) |
| UseNewArt 0x80000 | flags2 19 | `BuildSpriteDescriptor` (`HasFlags2(0x80000, 0)`) | mode 1: WorldArt block 0; set only in flags2 (795 records, all with has_world_art) |
| ArtUsed 0x4000000 | flags2 26 | data only | set only in flags2 (886 records) |
| 0x200000000 | flags1 33 | `FUN_00535500`, `FUN_005b3ce0` | radar/equipment code; repo calls it NoDraw |
| 0x400000000 | flags1 34 | `BuildSpriteDescriptor`, `RequestTileArtTexture` | use `rotation`; full texture size in the UI; cleared in mode 0. Set on 1658 records (1589 with a zero EC window), never in flags2. Repo and UOETE call it Unused1 |
| AlphaBlend 0x800000000 | flags1 35 | `BuildSpriteDescriptor` | alpha sprite |

## 3. In-memory object

`/UOTileArt/UOObjectDefinition` (0x100) = `UOObjectDefinitionBase` (0xf0) + effect vector at +0xf0.
Vtables: `UOObjectDefinition` 0x00ca5b4c (85 slots), `UOObjectDefinitionBase` 0x00cdf224,
`UOObjectDefinitionBinaryFactory` 0x00ca5ca4, `UOObjectDefinitionBinaryBaseFactory` 0x00cdf40c
(its NewInstance 0x00844569 allocates 0xf0 bytes, base object without effects), `UOShaderDefinition` 0x00ca5cbc.
Slots shared with the base vtable are named in the `UOObjectDefinitionBase` namespace.

| Offset | Field | Default |
|---|---|---|
| +0x04 | name (std::string) | |
| +0x20 | tile_id | |
| +0x24 / +0x3c | enhanced / legacy window | |
| +0x54 / +0x55 | has_world_art / uses_enhanced_tileart | |
| +0x58 / +0x5c | world / legacy art scale | 1.0 / 1.5 |
| +0x60 | field_60 | DAT_00e3216c |
| +0x64 | old_id | 0xffffff |
| +0x68 | rotation | 0 |
| +0x6c..+0x78 | light scale, field_70, offset x, offset y | 1.0, globals 0x00e32170..0x00e32178 |
| +0x7c | house_join_bits | 0 |
| +0x80 / +0x88 | flags1 / flags2 | |
| +0x90 | facing | 1 |
| +0x94 / +0x98 | radar rgba / has radar | |
| +0x9c..+0xbb | shader blocks 0..3 (shared_ptr) | 1..3 from the default globals |
| +0xbc | appearance filter vector* | null |
| +0xc0 | sitting shared_ptr | null |
| +0xc8 / +0xd4 | props1 / props2 maps | |
| +0xe0 | stack alias map* | null |
| +0xe4 | body_type | 400 |
| +0xe8 | field_e8 | |
| +0xf0 | effects vector (derived class only) | |

## 4. Usage sites

The script's report lists all 41 callers of `UOTileArt::GetObjectDefinition` with the virtual-call offsets
they use. Grouped by subsystem:

### 4.1 Rendering and sprites
- `UOObjectDefinition::GetSpriteDescriptor` 0x004c9240 (slot +0x13c) -> `UOTileArt::RequestObjectDefinition`
  0x00842f87 and `UOTileArt::GetCachedSpriteDescriptor` 0x00457a70 (0x120-byte descriptors per tile id,
  critical section 0x00e330a8, map 0x00e330e8; legacy switch from settings 0x00e37224 +0x54).
- `UOTileArt::BuildSpriteDescriptor` 0x004592e0: mode 1 (UseNewArt in flags2, not legacy): block 0,
  world_art_scale, EC window (+1; 44x44 if no world art); mode 2 (`uses_enhanced_tileart`): block 2,
  legacy scale x DAT_00d05174, legacy window x1.5; mode 0: block 1, legacy scale, legacy window, bit 34
  cleared. Foliage or bit 35 -> alpha; bit 34 -> rotation; sitting window shift.
- `UOObjectDefinition::GetPrimaryTextureLocation` 0x004c9050 (slot +0x80): block 0 or 1, special case for
  `UOWaterShader`, builds a `SpriteTextureFileLocation` (`ConstructFromPath` 0x0058d130 /
  `CopyConstruct` 0x0058d1f0 classify "Textures\" 8, "TileArtLegacy\" 0x16, "TileArtEnhanced\" 0x1a).
- Material builder 0x0045cd80: texture item uv_set, stretch -> wrap, source_kind.
- World objects and effects: `FUN_00466940`, `FUN_0044bb40` (world art scale, flags, `AddEffect`,
  `GetEffectCount`/`GetEffect`, sprite descriptor), `FUN_0042ef10`, `FUN_00444b10`.
- Hue shader data: `FUN_00595010` ("CustomShaderData", `Build\Hues\Hues.dds`; HasFlags1, HasSitting).
- Lighting (probable): `FUN_00437e60` (light scale, light offsets), `FUN_005b3690` (`HasLightData`, light offset).

### 4.2 UI
- `UOTileArt::RequestTileArtTexture` 0x00519c50 ("%08d_TileArt") and
  `UOTileArt::RequestLegacyTileArtTexture` 0x0051a330 ("%08d_LegacyTileArt"), used by
  `UOLua::RequestTileArt` 0x0051b650 / `UOLua::RequestLegacyTileArt` 0x0051bed0.
  Flags1 bit 34 -> full texture size.
- `UOTileArt::GetGraphicForAmount` 0x005bfe70 (slot +0xc4), called from `FUN_0057d550`.
- `UOLua::UserActionEquipItemsAddItem` 0x00541920 (world art scale, props1).
- `FUN_005798a0` (object properties next to CurrentHealth/MaxHealth/CurrentMana/MaxMana).
- `FUN_0050a650`..`FUN_0050c790`, `FUN_005099f0`, `FUN_004d7230`, `FUN_004d2170`: property, flag and window
  queries for UI windows (not named).

### 4.3 Paperdoll, equipment and mobiles
- `FUN_004387f0` ("Not letting a %u(sq %u) wear a %u(sq %u) because bad appearance tags"):
  `GetAppearance`/`GetAnimationAppearance`, body type, props.
- `FUN_004394a0` (appearance), `FUN_00468810` (race/gender), `FUN_00436d40` (", mounted"),
  `FUN_005b3ce0` (Wearable, props1 key 6 layer, flags1 bit 33).

### 4.4 Radar
- `FUN_00535500` (radar color, flags 0x200000000, Wall/Roof/Surface), `FUN_005110c0` (radar color).

### 4.5 Network object creation
- `FUN_005ea110` (radar color, flags, join bits, legacy window, body type).

### 4.6 House customization
- `UOHouseCustomization::TilesConnect` 0x006588f0 (probable), `(dx, dy)`:
  dx = -1: A.bit6 & B.bit8 or A.bit7 & B.bit9; dx = +1 reversed;
  dy = -1: A.bit10 & B.bit12 or A.bit11 & B.bit13; dy = +1 reversed.
  So bits 6/7 connect west, 8/9 east, 10/11 north, 12/13 south.
- `FUN_00657e20`, `FUN_00658e30`, `FUN_00654aa0` (join bits 1, 2, 3, 5, 14), in the LoadHouseFromMultiData chain.

## 5. Comparison with UOETE and repo `tileart.rs`

| Item | UOETE | tileart.rs | Client code | Verdict |
|---|---|---|---|---|
| Fixed field order and sizes | as table 2.1 | same | same | agree |
| unkBool1 / unk_bool1 | unknown | unknown | has_world_art (block 0 present) | new |
| unkBool2 / unk_bool2 | unknown | unknown | uses_enhanced_tileart (mode 2) | new |
| unkFloat3/4 | unknown | unknown | world / legacy art scale | new |
| fixedZero | 0 | 0 | field_60, only in `HasLegacyArtData` | agree (unused) |
| oldID | old id | old_id | old_id, default 0xffffff | agree |
| unkFloat5 | unknown | unknown | rotation in degrees, used with flags1 bit 34 | new |
| type / type_val | "type" | Static/Solid/Liquid | body type (default 400, 666 gargoyle) | correct |
| unkByte6 | unknown | unknown | field_e8, unknown | agree (unknown) |
| unkDW7 | int | i32 | f32 light scale (probable) | correct (type) |
| unkDW8 | int | u32 | light_field_70, 0 | agree (unused) |
| light1/light2 | float | float | light offset x/y (probable) | refine |
| unkDW9 | unknown | unknown | house join bits (6..13 connectors) | new |
| facing | int | facing | default 1 | agree |
| Image windows | xStart..yOff | same | x_end/y_end inclusive | agree |
| props1/props2 | key/value | PropertyKey 0..11 | separate maps; keys 6, 8, 9, 10 used | agree |
| Stack aliases | amount/id | same | map amount -> id; last alias with amount <= n | agree |
| Appearance subtype | 1 = gender, else list | 1 = gender, else list | 0 body list, 1 gender, 2 animation body list, other = error | correct |
| Sitting | 4 x u32 | 4 x u32 | 6 x u32 | correct |
| Radar color | rgba | rgba | rgba + present flag | agree |
| Texture blocks | 4 | 4 | WorldArt, Legacy, Enhanced, Light | refine |
| Block unk1 | byte | byte | field_04, 0 | agree (unused) |
| Item unk4 | byte | byte | source_kind (0/2 load path) | new |
| Item unk5 | float | float | stretch (wrap if != 1) | new |
| Item unk6 | u32 | u32 | sampler index (probable) | new (low) |
| Item unk7 | u32 | u32 | uv set | new |
| unk8/unk9 vectors | counts + values | counts + values | Technique int[4], Tweaks float[4] | new |
| Tail after blocks | copied raw | not parsed | u8 effect count + effect definitions | new |
| Flag bit 34 | Unused1 | Unused1 | rotated / full texture | correct |
| UseNewArt | flags | flags | tested in flags2 only | refine |
| String dictionary | u64, u32 count, u32 unk (16 bytes), index - 1 | 14-byte header, index - 1 | u64 + u32 count + u16 max length (14 bytes); string 0 is empty; index used directly | correct (same strings, see 6.3) |

## 6. Proposed corrections for the repo

Not applied; files are read-only for this task.

1. `TaeSittingAnimation`: 6 x u32, not 4.
2. Parse the tail: u8 effect count, then effect definitions (u32 type key + effect body). Records with
   effects currently leave bytes unparsed.
3. String dictionary: header is u64 stamp, u32 count, u16 max length (14 bytes); string 0 is empty and the
   client indexes directly. Skipping string 0 and subtracting 1 from every index (UOETE's 16-byte header
   swallows the empty string's length) gives the same strings, but indexing directly with all `count`
   strings matches the client and also handles index 0 (empty name) without `checked_sub(1)`.
4. Appearance: handle subtype 0 and 2 as body lists and treat any other subtype as an error.
5. Rename `type_val` to `body_type` and drop the Static/Solid/Liquid mapping.
6. `unk_bool1` -> `has_world_art`, `unk_bool2` -> `uses_enhanced_tileart`.
7. `unk_float3`/`unk_float4` -> `world_art_scale`/`legacy_art_scale`; `unk_float5` -> `rotation_degrees`.
8. `unk_dw7` is f32 (`light_scale`, probable); `light1`/`light2` -> `light_offset_x/y` (probable).
9. `unk_dw9` -> `house_join_bits`.
10. `TaeFlag` bit 34 (`Unused1`) -> rotated tile art (uses rotation, full texture size in the UI).
11. Test `UseNewArt` (and `ArtUsed`) in flags2.
12. Texture item: `unk4` -> `source_kind`, `unk5` -> `stretch` (wrap when != 1), `unk6` -> `sampler_index`
    (probable), `unk7` -> `uv_set`.
13. Block vectors: Technique (i32) and Tweaks (f32), fixed capacity 4.
14. Block 3 is the light texture (`*_light_window.tga`).

## 7. Names applied

Totals: 165 functions proposed, 157 renamed, 8 kept (names by another agent, listed below), 165 tagged,
157 plate comments, 5 labels, 12 types plus 3 typed globals, 0 anomalies. A second `--apply` reports
"Changes: none".

Kept names (another source type, not renamed; the proposed name is given for reference only):

| Address | Existing name | Proposed |
|---|---|---|
| 0x00989f55 | `UOP::OpenFilePtr` | `UOP::OpenVirtualFileOwned` |
| 0x0098d503 | `UOP::MypArchive_GetEntryDecompressedSize` | `UOP::GetEntryDecompressedSize` |
| 0x0098d520 | `UOP::MypArchive_GetEntryCompressedSize` | `UOP::GetEntryCompressedSize` |
| 0x0098d57c | `UOP::MypArchive_ReadEntry` | `UOP::ReadEntryData` |
| 0x0098d6d3 | `UOP::Decompress` | `UOP::ZlibUncompress` |
| 0x0098db06 | `UOP::MypArchive_GetEntry` | `UOP::GetEntryInfo` |
| 0x0098e5df | `UOP::MythMypInputFile_ctor` | `MythMypInputFile::MythMypInputFile` |
| 0x0098e672 | `UOP::MythMemoryInputFile_ctor` | `MythMemoryInputFile::MythMemoryInputFile` |

These agree in meaning; no disagreement.

Global labels: `g_UOAssetManager` 0x00e37220, `g_UOTileArtDefaultLegacyShader` 0x00e346c4,
`g_UOTileArtDefaultLightShader` 0x00e346cc, `g_UOTileArtDefaultEnhancedShader` 0x00e346d4 (the three typed
`UOShaderDefinitionSharedPtr`), `g_UOEffectDefinitionFactories` 0x00e3737c.

Types in `/UOTileArt`: `MsvcString`, `MsvcVector`, `MsvcMap`, `UOTileArtImageWindow`, `UOTileArtSitting`,
`UOTileArtTextureItem`, `UOShaderDefinition`, `UOShaderDefinitionSharedPtr`, `UOTileArtSittingSharedPtr`,
`UOObjectDefinition`, `TileArtRecordFixed`, `StringDictionaryHeader`.

Functions:

| Address | Name | Status |
|---|---|---|
| 0x004491a0 | `UOTileArtTextureItem::Init` (probable) | renamed |
| 0x00457a70 | `UOTileArt::GetCachedSpriteDescriptor` (probable) | renamed |
| 0x004592e0 | `UOTileArt::BuildSpriteDescriptor` (probable) | renamed |
| 0x004a9150 | `UOAssets::Initialize` (probable) | renamed |
| 0x004c9050 | `UOObjectDefinition::GetPrimaryTextureLocation` | renamed |
| 0x004c9240 | `UOObjectDefinition::GetSpriteDescriptor` | renamed |
| 0x004c9360 | `UOObjectDefinition::GetMemorySize` | renamed |
| 0x004c93d0 | `UOObjectDefinitionBase::GetName` | renamed |
| 0x004c93e0 | `UOObjectDefinitionBase::GetTileId` | renamed |
| 0x004c93f0 | `UOObjectDefinitionBase::GetEnhancedImageWindow` | renamed |
| 0x004c9420 | `UOObjectDefinitionBase::GetLegacyImageWindow` | renamed |
| 0x004c9450 | `UOObjectDefinitionBase::HasWorldArt` | renamed |
| 0x004c9460 | `UOObjectDefinitionBase::UsesEnhancedTileArt` | renamed |
| 0x004c9470 | `UOObjectDefinitionBase::GetWorldArtScale` | renamed |
| 0x004c9480 | `UOObjectDefinitionBase::GetLegacyArtScale` | renamed |
| 0x004c9490 | `UOObjectDefinitionBase::GetField60` | renamed |
| 0x004c94a0 | `UOObjectDefinitionBase::HasLegacyArtData` (probable) | renamed |
| 0x004c94e0 | `UOObjectDefinitionBase::GetLightScale` (probable) | renamed |
| 0x004c94f0 | `UOObjectDefinitionBase::GetLightField70` (probable) | renamed |
| 0x004c9500 | `UOObjectDefinitionBase::GetLightOffsetX` (probable) | renamed |
| 0x004c9510 | `UOObjectDefinitionBase::GetLightOffsetY` (probable) | renamed |
| 0x004c9520 | `UOObjectDefinitionBase::HasLightData` (probable) | renamed |
| 0x004c9580 | `UOObjectDefinitionBase::HasFlags1` | renamed |
| 0x004c95b0 | `UOObjectDefinitionBase::GetFlags1` | renamed |
| 0x004c95c0 | `UOObjectDefinitionBase::HasFlags2` | renamed |
| 0x004c95f0 | `UOObjectDefinitionBase::GetFlags2` | renamed |
| 0x004c9600 | `UOObjectDefinitionBase::HasFlags1OrProperties1` | renamed |
| 0x004c9620 | `UOObjectDefinitionBase::HasFlags2OrProperties2` | renamed |
| 0x004c9640 | `UOObjectDefinitionBase::GetOldId` | renamed |
| 0x004c9650 | `UOObjectDefinitionBase::GetRotation` (probable) | renamed |
| 0x004c9660 | `UOObjectDefinitionBase::GetFacing` | renamed |
| 0x004c9670 | `UOObjectDefinitionBase::GetWorldArtShader` | renamed |
| 0x004c96a0 | `UOObjectDefinitionBase::GetLegacyShader` | renamed |
| 0x004c96d0 | `UOObjectDefinitionBase::GetEnhancedShader` | renamed |
| 0x004c9700 | `UOObjectDefinitionBase::GetLightShader` | renamed |
| 0x004c9730 | `UOObjectDefinitionBase::GetBodyType` (probable) | renamed |
| 0x004c9740 | `UOObjectDefinitionBase::GetFieldE8` | renamed |
| 0x004c9750 | `UOObjectDefinitionBase::HasHouseJoinBit1` | renamed |
| 0x004c9760 | `UOObjectDefinitionBase::HasHouseJoinBit2` | renamed |
| 0x004c9770 | `UOObjectDefinitionBase::HasHouseJoinBit3` | renamed |
| 0x004c9780 | `UOObjectDefinitionBase::HasHouseJoinBit4` | renamed |
| 0x004c9790 | `UOObjectDefinitionBase::HasHouseJoinBit5` | renamed |
| 0x004c97a0 | `UOObjectDefinitionBase::ConnectsWestA` (probable) | renamed |
| 0x004c97b0 | `UOObjectDefinitionBase::ConnectsWestB` (probable) | renamed |
| 0x004c97c0 | `UOObjectDefinitionBase::ConnectsEastA` (probable) | renamed |
| 0x004c97d0 | `UOObjectDefinitionBase::ConnectsEastB` (probable) | renamed |
| 0x004c97e0 | `UOObjectDefinitionBase::ConnectsSouthA` (probable) | renamed |
| 0x004c97f0 | `UOObjectDefinitionBase::ConnectsSouthB` (probable) | renamed |
| 0x004c9800 | `UOObjectDefinitionBase::ConnectsNorthA` (probable) | renamed |
| 0x004c9810 | `UOObjectDefinitionBase::ConnectsNorthB` (probable) | renamed |
| 0x004c9820 | `UOObjectDefinitionBase::HasHouseJoinBit14` | renamed |
| 0x004c9830 | `UOObjectDefinitionBase::HasHouseJoinBit15` | renamed |
| 0x004c9840 | `UOObjectDefinitionBase::GetHouseJoinBits` | renamed |
| 0x004c9850 | `UOObjectDefinitionBase::SetHouseJoinBits` | renamed |
| 0x004c9860 | `UOObjectDefinitionBase::HasSitting` | renamed |
| 0x004c9870 | `UOObjectDefinitionBase::GetSittingDirections` (probable) | renamed |
| 0x004c9880 | `UOObjectDefinitionBase::GetSittingField04` | renamed |
| 0x004c98a0 | `UOObjectDefinitionBase::GetSittingField08` | renamed |
| 0x004c98c0 | `UOObjectDefinitionBase::GetSittingField10` | renamed |
| 0x004c98e0 | `UOObjectDefinitionBase::GetSittingField14` | renamed |
| 0x004c9900 | `UOObjectDefinitionBase::GetSittingWindowShift` (probable) | renamed |
| 0x004c9920 | `UOObjectDefinitionBase::SetSitting` | renamed |
| 0x004c9a70 | `UOObjectDefinition::AddEffect` | renamed |
| 0x004c9b10 | `UOObjectDefinition::GetEffectCount` | renamed |
| 0x004c9b30 | `UOObjectDefinition::GetEffect` | renamed |
| 0x004c9b60 | `UOObjectDefinition::SetWorldArtShader` | renamed |
| 0x004c9bc0 | `UOObjectDefinition::SetLegacyShader` | renamed |
| 0x004c9c20 | `UOObjectDefinition::SetEnhancedShader` | renamed |
| 0x004c9c80 | `UOObjectDefinition::SetLightShader` | renamed |
| 0x004c9ce0 | `UOObjectDefinition::ScalarDeletingDestructor` | renamed |
| 0x004c9d30 | `UOObjectDefinitionBinaryFactory::NewInstance` | renamed |
| 0x004c9e00 | `UOShaderDefinition::GetTextureCount` | renamed |
| 0x004c9e20 | `UOShaderDefinition::GetTexture` | renamed |
| 0x004c9e80 | `UOShaderDefinition::GetField04` | renamed |
| 0x004c9e90 | `UOShaderDefinition::GetShaderName` | renamed |
| 0x004c9ec0 | `UOShaderDefinition::AddTexture` | renamed |
| 0x004c9f10 | `UOShaderDefinition::SetField04` | renamed |
| 0x004c9f20 | `UOShaderDefinition::SetShaderName` | renamed |
| 0x004c9f50 | `UOShaderDefinition::GetTechniqueCount` | renamed |
| 0x004c9f60 | `UOShaderDefinition::GetTweakCount` | renamed |
| 0x004c9f70 | `UOShaderDefinition::GetTechniqueValues` | renamed |
| 0x004c9f80 | `UOShaderDefinition::GetTweakValues` | renamed |
| 0x004c9f90 | `UOShaderDefinition::SetTechniqueValue` | renamed |
| 0x004c9fa0 | `UOShaderDefinition::SetTweakValue` | renamed |
| 0x004c9fc0 | `UOShaderDefinition::ScalarDeletingDestructor` | renamed |
| 0x00519c50 | `UOTileArt::RequestTileArtTexture` (probable) | renamed |
| 0x0051a330 | `UOTileArt::RequestLegacyTileArtTexture` (probable) | renamed |
| 0x0058d130 | `SpriteTextureFileLocation::ConstructFromPath` (probable) | renamed |
| 0x0058d1f0 | `SpriteTextureFileLocation::CopyConstruct` (probable) | renamed |
| 0x005bda40 | `UOTileArt::GetObjectDefinition` | renamed |
| 0x005bfe70 | `UOTileArt::GetGraphicForAmount` (probable) | renamed |
| 0x006588f0 | `UOHouseCustomization::TilesConnect` (probable) | renamed |
| 0x008415ae | `UOEffects::ParseEffectDefinition` (probable) | renamed |
| 0x00842434 | `UOObjectDefinitionBase::UOObjectDefinitionBase` | renamed |
| 0x00842884 | `UOObjectDefinitionBase::GetWorldArtShaderName` | renamed |
| 0x008428e2 | `UOObjectDefinitionBase::GetLegacyShaderName` | renamed |
| 0x00842940 | `UOObjectDefinitionBase::GetLightShaderName` | renamed |
| 0x0084299e | `UOObjectDefinitionBase::SetProperty1` | renamed |
| 0x008429bc | `UOObjectDefinitionBase::HasProperty1` | renamed |
| 0x008429ed | `UOObjectDefinitionBase::GetProperty1` | renamed |
| 0x00842a38 | `UOObjectDefinitionBase::SetProperty2` | renamed |
| 0x00842a56 | `UOObjectDefinitionBase::HasProperty2` | renamed |
| 0x00842a87 | `UOObjectDefinitionBase::GetProperty2` | renamed |
| 0x00842ad5 | `UOObjectDefinitionBase::GetPrimaryArtScale` | renamed |
| 0x00842ae3 | `UOObjectDefinitionBase::AddStackAlias` | renamed |
| 0x00842b55 | `UOObjectDefinitionBase::AddAppearanceFilter` | renamed |
| 0x00842bb2 | `UOObjectDefinitionBase::GetAppearance` | renamed |
| 0x00842c6b | `UOObjectDefinitionBase::GetAnimationAppearance` | renamed |
| 0x00842d90 | `UOObjectDefinitionBase::GetGraphicForAmount` | renamed |
| 0x00842dbd | `UOObjectDefinitionBase::HasRace` | renamed |
| 0x00842df1 | `UOObjectDefinitionBase::GetRace` | renamed |
| 0x00842e34 | `UOObjectDefinitionBase::HasGender` | renamed |
| 0x00842e68 | `UOObjectDefinitionBase::GetGender` | renamed |
| 0x00842ea8 | `UOObjectDefinitionBase::GetRadarColor` | renamed |
| 0x00842eb7 | `UOObjectDefinitionBase::SetRadarColor` | renamed |
| 0x00842ecb | `UOObjectDefinitionBase::HasRadarColor` | renamed |
| 0x00842f87 | `UOTileArt::RequestObjectDefinition` (probable) | renamed |
| 0x00844569 | `UOObjectDefinitionBinaryBaseFactory::NewInstance` | renamed |
| 0x0084466e | `UOObjectDefinitionBinaryBaseFactory::Create` | renamed |
| 0x0084478f | `UOTileArt::ParseObjectDefinition` | renamed |
| 0x00844dda | `UOTileArt::ParseStackAliases` | renamed |
| 0x00844e6b | `UOTileArt::ParseProperties1` | renamed |
| 0x00844f05 | `UOTileArt::ParseProperties2` | renamed |
| 0x00844f9f | `UOTileArt::ParseEffects` | renamed |
| 0x00845060 | `UOTileArt::ParseAppearanceFilters` | renamed |
| 0x00845168 | `UOTileArt::ParseSitting` | renamed |
| 0x008452f4 | `UOTileArt::ParseRadarColor` | renamed |
| 0x008456a3 | `UOTileArt::ParseShaderBlock` | renamed |
| 0x0084aa9f | `AppearanceFilterByOwnerBodyType::AppearanceFilterByOwnerBodyType` | renamed |
| 0x0084aad8 | `AppearanceFilterByOwnerBodyType::AddBodyMapping` (probable) | renamed |
| 0x0084aaf9 | `AppearanceFilterByOwnerBodyType::GetFilterType` | renamed |
| 0x0084ab55 | `AnimationAppearanceFilterByOwnerBodyType::AnimationAppearanceFilterByOwnerBodyType` | renamed |
| 0x0084ab88 | `AnimationAppearanceFilterByOwnerBodyType::GetFilterType` | renamed |
| 0x0084abcd | `AppearanceFilterByOwnerGender::AppearanceFilterByOwnerGender` | renamed |
| 0x0084ac3c | `AppearanceFilterByOwnerGender::GetFilterType` | renamed |
| 0x0084ac82 | `UOTileArt::ParseAppearanceFilterByOwnerBodyType` | renamed |
| 0x0084ad2d | `UOTileArt::ParseAppearanceFilterByOwnerGender` | renamed |
| 0x0084ada8 | `UOTileArt::ParseAnimationAppearanceFilterByOwnerBodyType` | renamed |
| 0x0084b3c4 | `AppearanceFilterByOwnerBodyType::Apply` (probable) | renamed |
| 0x0084b485 | `AppearanceFilterByOwnerGender::Apply` (probable) | renamed |
| 0x0084b4d0 | `AnimationAppearanceFilterByOwnerBodyType::Apply` (probable) | renamed |
| 0x00989f55 | `UOP::OpenVirtualFileOwned` (probable) | kept UOP::OpenFilePtr (ANALYSIS) |
| 0x0098a341 | `UOP::MountArchive` (probable) | renamed |
| 0x0098ba02 | `UOP::HashLittle2` | renamed |
| 0x0098d460 | `UOP::FindEntryIndex` | renamed |
| 0x0098d487 | `UOP::HashFileName` | renamed |
| 0x0098d503 | `UOP::GetEntryDecompressedSize` (probable) | kept UOP::MypArchive_GetEntryDecompressedSize (ANALYSIS) |
| 0x0098d520 | `UOP::GetEntryCompressedSize` (probable) | kept UOP::MypArchive_GetEntryCompressedSize (ANALYSIS) |
| 0x0098d57c | `UOP::ReadEntryData` (probable) | kept UOP::MypArchive_ReadEntry (ANALYSIS) |
| 0x0098d6d3 | `UOP::ZlibUncompress` | kept UOP::Decompress (ANALYSIS) |
| 0x0098db06 | `UOP::GetEntryInfo` (probable) | kept UOP::MypArchive_GetEntry (ANALYSIS) |
| 0x0098dbe9 | `UOP::LookupHash` | renamed |
| 0x0098e5df | `MythMypInputFile::MythMypInputFile` (probable) | kept UOP::MythMypInputFile_ctor (ANALYSIS) |
| 0x0098e672 | `MythMemoryInputFile::MythMemoryInputFile` (probable) | kept UOP::MythMemoryInputFile_ctor (ANALYSIS) |
| 0x0098ed3d | `UOP::HashString` | renamed |
| 0x00a6bb5a | `UOAssets::MountPackages` (probable) | renamed |
| 0x00a6cfc8 | `UOAssets::GetAsset` | renamed |
| 0x00a6d346 | `UOAssets::ResolveAsset` (probable) | renamed |
| 0x00a6d38f | `UOAssets::LoadAssetBlob` (probable) | renamed |
| 0x00a6d4b7 | `UOAssets::ReadPackagedFile` (probable) | renamed |
| 0x00a6d5e4 | `UOAssets::ReadFileFromPath` (probable) | renamed |
| 0x00a6d84a | `UOStringDictionary::Load` | renamed |
| 0x00a6fe55 | `UOAssets::StreamRead` | renamed |
| 0x00a72861 | `UOAssets::CreateAssetFromBlob` (probable) | renamed |
| 0x00a749ed | `UOAssets::TryResolvers` (probable) | renamed |

## 8. Open questions

- `field_e8` (record byte 0x24), `field_60`, `light_field_70`: no meaning found; the last two are 0 everywhere.
- Texture item `sampler_index`: no code reads it.
- props2 keys: stored and queried, but the queries were not traced to meanings.
- Join bits 1-5, 14, 15: used by house customization code (`FUN_00657e20`, `FUN_00654aa0`, `FUN_00658e30`),
  meaning not established.
- Sitting fields 1, 2, 4, 5.
- Effect body format.
