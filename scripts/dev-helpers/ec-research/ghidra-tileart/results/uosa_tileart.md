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
| 0x14 | u32 | field_60 | +0x60 | `GetField60` +0x34 and `HasLegacyArtData` +0x38 read it; neither slot is called on a definition (section 8.10); 0 in all records | high (unused) |
| 0x18 | u32 | old_id | +0x64 | default 0xffffff; `GetOldId` +0x78; equals the tile id on 2236 records, another id on 141 (103->102, 129->128, 722->720); `GetOldId` is never called on a definition (section 8.10) | high (unused by the client; meaning from data only) |
| 0x1c | f32 | rotation | +0x68 | `GetRotation` +0x7c; 0/90/180/270; read by `BuildSpriteDescriptor` only when flags1 bit 34 is set | high |
| 0x20 | u32 | body_type | +0xe4 | default 400; `GetBodyType` +0xb4. 400 = human male, 666 = gargoyle male; creature placeholders (ids >= 10000000) hold the body id. `FUN_004387f0` refuses an equipment appearance whose body type differs from the wearer's body ("Not letting a %u(sq %u) wear a %u(sq %u)"); `UOObject::InitFromDefinition` copies it to the world object (+0x50) | high |
| 0x24 | u8 | alternate_animation (repo `field_e8`) | +0xe8 | `GetAlternateAnimation` +0xb8 -> `UOObject::SetAlternateAnimation` (+0x54). `FUN_004386a0` reads it from the item in the wearer's layer 2 and passes it to the animation action lookup `FUN_008361ed` (alternate entry +0x14 instead of +0x10) for stand (0) and walk/run (1/2). Set on 30 records: gargoyle two-handed weapons (body 666), halberds 5182/5183, creature placeholders | high (mechanism) |
| 0x25 | f32 | light_scale | +0x6c | default 1.0; `GetLightScale` +0x40; 2.0 on 866 records (windows, arrow loops). `UOObjectComponentLighting::Initialize` stores it as the light sprite scale | high |
| 0x29 | u32 | light_field_70 | +0x70 | `GetLightField70` +0x44; 0 in all records; no virtual call on a definition reads it | unknown (unused) |
| 0x2d | f32 | light_offset_x | +0x74 | `GetLightOffsetX` +0x48; non-zero only with block 3; e.g. -5, -1.35. `UOObjectComponentLighting::Initialize`: (0, y, x) times the view matrix, added to the light sprite position | high |
| 0x31 | f32 | light_offset_y | +0x78 | `GetLightOffsetY` +0x4c; e.g. -4, -3; second component of the same offset | high |
| 0x35 | u32 | house_join_bits | +0x7c | stored through vtable +0x14c; tested bit by bit (slots +0x100..+0x138); bits 6..13 are house-customization connectors (section 4.6), bits 1, 2, 3 placement rules, bit 5 rests on the floor, bit 14 supports the level above (section 8.4); bits 0, 4, 15 not read | high (1-3, 5, 6..13, 14), unused (0, 4, 15) |
| 0x39 | u64 | flags1 | +0x80 | `HasFlags1`/`GetFlags1`; classic TileData flags plus EC bits | high |
| 0x41 | u64 | flags2 | +0x88 | `HasFlags2`/`GetFlags2`; `UseNewArt` (0x80000) is tested here | high |
| 0x49 | u32 | facing | +0x90 | default 1; `GetFacing` +0x88 -> `UOObjectComponentSprite::Initialize` stores it at component +0x2c; no reader of that component field was found (section 8.10). 1 x34276, 3 x2915, 2 x411, 0 x150, 4 x4 | medium (probably unused) |
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
| field_04 | u8 | +0x04 | `GetField04`: the shader implementation index passed to `NiShaderFactory::RetrieveShader(name, field_04, 1)` (0x0045da8d-0x0045dace, `ghidra-texture-roles/results/uosa_texture_roles.md` section 1); 0 in all records |
| shader name | u32 string index | +0x08 std::string | `UOSpriteShader` (14608), `UOStaticTerrainShader` (341), `UOWaterShader` (322) in block 0 |
| item count | u8 | vector +0x24 | |
| item | u32 path index, u8 source_kind, f32 stretch, u32 sampler_index, u32 uv_set | `UOTileArtTextureItem` 0x2c | see below |
| technique | u32 n, n x i32 | int[4] +0x34 | n is 0 or 4; `SetTechniqueValue` (index < 4) |
| tweaks | u32 n, n x f32 | float[4] +0x44 | n is 0 or 4; `SetTweakValue` |

Texture item (`UOTileArtTextureItem`, ctor `UOTileArtTextureItem::Init` 0x004491a0):

| Field | Memory | Meaning | Conf. |
|---|---|---|---|
| path | +0x00 std::string | `Data\WorldArt\...`, `TileArtLegacy\...`, `TileArtEnhanced\...` | high |
| source_kind (u8) | +0x1c | material builder 0x0045cd80 (0x0045e23f): 0 loads the texture, 2 loads it the same way and forces wrap addressing (`OR [map+4],0x3000` at 0x0045e2f2), any other value loads nothing; 0 in all records | high |
| stretch (f32) | +0x20 | != 1.0 sets address mode wrap (bits 12-13 = 3) | high |
| sampler_index (u32) | +0x24 | not read: the material builder binds items by position (item i -> shader map i); usually equals the position (section 8.10). The `NiTexturingProperty` base map is the hue atlas, not an item (`ghidra-texture-roles/results/uosa_texture_roles.md`) | high (unused) |
| uv_set (u32) | +0x28 | texture coordinate set: low byte of the `NiTexturingProperty::Map` flags (0x0045de94, 0x0045e1ec); 0/1 | high |

Blocks: 0 = WorldArt (present on 15271 records; implies `has_world_art`), 1 = TileArtLegacy (always present),
2 = TileArtEnhanced (always present, 2 items), 3 = light texture (`*_light_window.tga`, 1042 records).
Defaults for blocks 1-3 come from `g_UOTileArtDefaultLegacyShader` 0x00e346c4,
`g_UOTileArtDefaultEnhancedShader` 0x00e346d4 and `g_UOTileArtDefaultLightShader` 0x00e346cc.

Sitting (`UOTileArtSitting`, 6 x i32, section 8.2): field 0 is the allowed-direction mask (1 north, 2 east,
4 south, 8 west); fields 1 and 2 are the seated mobile's sprite offset (the client subtracts them); field 3
(0, 1, -6, 6, 3, -2) shifts the chair-back overlay split when the direction is north or west
(`BuildSpriteDescriptor`, slot +0xf4); fields 4 and 5 are not read by the client.

Effects: `ParseEffectDefinition` reads a u32 type key, asks each factory in `g_UOEffectDefinitionFactories`
(0x00e3737c) through vtable +0 whether it handles the key, then creates the effect through vtable +8;
an unknown key is an error. 313 records have 1 effect, 16 have 2. The effect body format belongs to the
effect classes and is decoded in `../ghidra-effects/results/uosa_effects.md`.

Property keys (props1, section 8.3): 0 = weight (tested > 90), 3 = height (copied to the world object),
6 = equipment layer (`FUN_005b6490` with Wearable), 8 = appearance (`GetAppearance` +0xbc), 9 = race
(`GetRace`, default 1), 10 = gender (`GetGender`, default 0), 11 = paperdoll/appearance override
(+10000000). props2 uses its own map and accessors (+0x6c/+0x70); only `UOPaperdoll::BuildImage` reads it
(keys 11 and 8).

### 2.3 Flags used by the code

| Flag | Bit | Where | Use |
|---|---|---|---|
| Foliage 0x20000 | flags1 17 | `BuildSpriteDescriptor`, `UOObject::CreateComponents` | alpha sprite (`|2`) |
| PartialHue 0x40000 | flags1 18, flags2 18 | `FUN_00595010` (flags1), `UOPaperdoll::BuildImage` (flags2) | hue shader data; partial hue of paperdoll gumps |
| UseNewArt 0x80000 | flags2 19 | `BuildSpriteDescriptor` (`HasFlags2(0x80000, 0)`), `UOObject::CreateComponents` | mode 1: WorldArt block 0; set only in flags2 (795 records, all with has_world_art) |
| Animation 0x1000000 | flags1 24, flags2 24 | `UOObject::CreateComponents` (both words) | animated component selection |
| ArtUsed 0x4000000 | flags2 26 | data only | set only in flags2 (886 records) |
| 0x200000000 | flags1 33 | `FUN_00466940`, `FUN_0044bb40`, `FUN_005b6490` (definition); `UOObject::CreateComponents`, `FUN_005c28b0` (object copy) | repo calls it NoDraw |
| 0x400000000 | flags1 34 | `BuildSpriteDescriptor`, `RequestTileArtTexture` | use `rotation`; full texture size in the UI; cleared in mode 0. Set on 1658 records (1589 with a zero EC window), never in flags2. Repo and UOETE call it Unused1 |
| AlphaBlend 0x800000000 | flags1 35 | `BuildSpriteDescriptor` | alpha sprite |

The complete list of flag tests and per-bit record counts is in section 8.6.

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
| +0xe8 | alternate_animation (`field_e8`) | |
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
  `GetEffectCount`/`GetEffect`, sprite descriptor), `FUN_0042ef10`.
- World object setup: `UOObject::GetObjectDefinition` 0x005bc130 (cached definition, fallback tile 99999999),
  `UOObject::InitFromDefinition` 0x005bdac0 (flags1 copy, body type, alternate animation, height),
  `UOObject::CreateComponents` 0x005befd0 (lighting component only when `GetLightShaderName` is set),
  `UOObjectComponentSprite::Initialize` 0x00444b10 (facing, sitting window, sprite descriptor),
  `UOObjectComponentLighting::Initialize` 0x00449740 (light scale and offsets).
- Hue shader data: `FUN_00595010` ("CustomShaderData", `Build\Hues\Hues.dds`; HasFlags1 bit 18, HasSitting).
- Direction: `UOObject::SetDirection` 0x005b3690 and `FUN_00437e60` snap a facing to the sitting direction
  mask through `UOObject::NearestAllowedDirection` 0x0043a1a0. Their +0x40/+0x4c calls are on the world
  object (direction get/set), not light getters as the first round assumed.

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
- `FUN_004394a0` (appearance), `FUN_00468810` (race/gender), `FUN_00436d40` (", mounted"; seated mobile
  offset from sitting fields 1 and 2), `FUN_005b6490` (Wearable, props1 key 6 layer, flags1 bit 33).
- `FUN_005b3ce0` is `UOObject` vtable +0xcc: true when props1 key 0 (weight) > 90.
- `FUN_004386a0`: alternate animation flag of the layer-2 item (section 8.1).
- `UOPaperdoll::BuildImage` 0x0050c790: the only props2 reader (section 8.3).

### 4.4 Radar
- `FUN_00535500` (radar color, Wall/Roof/Surface), `FUN_005110c0` (radar color). Round 2 found no flags1
  bit 33 test in `FUN_00535500` (its +0x60 calls are on the world object); see section 8.6.

### 4.5 Network object creation
- `FUN_005ea110` (radar color, flags, join bits, legacy window, body type).

### 4.6 House customization
- `UOHouseCustomization::TilesConnect` 0x006588f0 (probable), `(dx, dy)`:
  dx = -1: A.bit6 & B.bit8 or A.bit7 & B.bit9; dx = +1 reversed;
  dy = -1: A.bit10 & B.bit12 or A.bit11 & B.bit13; dy = +1 reversed.
  So bits 6/7 connect west, 8/9 east, 10/11 north, 12/13 south.
- `UOHouseCustomization::IsPlacementAllowed` 0x00654aa0 (probable) and the support solver
  `UOHouseCustomization::UpdateSupport` 0x00657d50 (`ComputeObjectSupport` 0x00657e20,
  `PropagateSupportFromBelow` 0x00658e30 -> `MarkSupportAbove` 0x00658fc0) read join bits 1, 2, 3, 5, 14
  (section 8.4); `UpdateSupport` runs after loading the house design (`FUN_00653ad0`) and after edits
  (`FUN_00655c80`).

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
| unkByte6 | unknown | field_e8 | alternate animation of the layer-2 item | new |
| unkDW7 | int | f32 light_scale | f32 light scale | agree |
| unkDW8 | int | u32 | light_field_70, 0, not read | agree (unused) |
| light1/light2 | float | float | light offset x/y | refine |
| unkDW9 | unknown | unknown | house join bits (6..13 connectors) | new |
| facing | int | facing | default 1 | agree |
| Image windows | xStart..yOff | same | x_end/y_end inclusive | agree |
| props1/props2 | key/value | PropertyKey 0..11 | separate maps; keys 6, 8, 9, 10 used | agree |
| Stack aliases | amount/id | same | map amount -> id; last alias with amount <= n | agree |
| Appearance subtype | 1 = gender, else list | 1 = gender, else list | 0 body list, 1 gender, 2 animation body list, other = error | correct |
| Sitting | 4 x u32 | 4 x u32 | 6 x u32 | correct |
| Radar color | rgba | rgba | rgba + present flag | agree |
| Texture blocks | 4 | 4 | WorldArt, Legacy, Enhanced, Light | refine |
| Block unk1 | byte | byte | field_04: shader implementation index, 0 | refine |
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

Totals: 179 functions proposed (51 probable), 171 renamed, 8 kept (names by another agent, listed below),
179 tagged, 171 plate comments, 5 labels, 12 types plus 3 typed globals, 0 anomalies. A second `--apply`
reports "Changes: none". Round 2 added the 14 world-object functions marked "renamed (round 2)" and
updated the `UOTileArtSitting`, `UOObjectDefinition` and `TileArtRecordFixed` field names and comments.

Own names corrected: a name this script applied earlier (source ANALYSIS, tag `UO_TILEART`, plate comment
written by the script for that exact name) is replaced when the proposal changes; the plate comment is
refreshed only while it is still the script's single-line text. Names from other sources, tags or comments
are never touched. Applied (plate comments refreshed):

| Address | Old name | New name |
|---|---|---|
| 0x004c9740 | `UOObjectDefinitionBase::GetFieldE8` | `UOObjectDefinitionBase::GetAlternateAnimation` |
| 0x004c9880 | `UOObjectDefinitionBase::GetSittingField04` | `UOObjectDefinitionBase::GetSeatOffsetX` |
| 0x004c98a0 | `UOObjectDefinitionBase::GetSittingField08` | `UOObjectDefinitionBase::GetSeatOffsetY` |

Other plate comments keep the evidence text they were written with (the script only fills empty comments,
so e.g. `UOObject::InitFromDefinition` still says `GetFieldE8`); the slot evidence in the script and in this
document is the current one.

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
| 0x0043a1a0 | `UOObject::NearestAllowedDirection` (probable) | renamed (round 2) |
| 0x00444b10 | `UOObjectComponentSprite::Initialize` | renamed (round 2) |
| 0x004491a0 | `UOTileArtTextureItem::Init` (probable) | renamed |
| 0x00449740 | `UOObjectComponentLighting::Initialize` | renamed (round 2) |
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
| 0x004c94e0 | `UOObjectDefinitionBase::GetLightScale` | renamed |
| 0x004c94f0 | `UOObjectDefinitionBase::GetLightField70` (probable) | renamed |
| 0x004c9500 | `UOObjectDefinitionBase::GetLightOffsetX` | renamed |
| 0x004c9510 | `UOObjectDefinitionBase::GetLightOffsetY` | renamed |
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
| 0x004c9730 | `UOObjectDefinitionBase::GetBodyType` | renamed |
| 0x004c9740 | `UOObjectDefinitionBase::GetAlternateAnimation` | renamed from `GetFieldE8` |
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
| 0x004c9870 | `UOObjectDefinitionBase::GetSittingDirections` | renamed |
| 0x004c9880 | `UOObjectDefinitionBase::GetSeatOffsetX` | renamed from `GetSittingField04` |
| 0x004c98a0 | `UOObjectDefinitionBase::GetSeatOffsetY` | renamed from `GetSittingField08` |
| 0x004c98c0 | `UOObjectDefinitionBase::GetSittingField10` | renamed |
| 0x004c98e0 | `UOObjectDefinitionBase::GetSittingField14` | renamed |
| 0x004c9900 | `UOObjectDefinitionBase::GetSittingWindowShift` | renamed |
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
| 0x0050c790 | `UOPaperdoll::BuildImage` (probable) | renamed (round 2) |
| 0x00519c50 | `UOTileArt::RequestTileArtTexture` (probable) | renamed |
| 0x0051a330 | `UOTileArt::RequestLegacyTileArtTexture` (probable) | renamed |
| 0x0058d130 | `SpriteTextureFileLocation::ConstructFromPath` (probable) | renamed |
| 0x0058d1f0 | `SpriteTextureFileLocation::CopyConstruct` (probable) | renamed |
| 0x005b3070 | `UOObject::SetBodyType` | renamed (round 2) |
| 0x005b3080 | `UOObject::GetBodyType` | renamed (round 2) |
| 0x005b3090 | `UOObject::SetAlternateAnimation` | renamed (round 2) |
| 0x005b30a0 | `UOObject::GetAlternateAnimation` | renamed (round 2) |
| 0x005b3690 | `UOObject::SetDirection` (probable) | renamed (round 2) |
| 0x005b3910 | `UOObject::GetFlags1` (probable) | renamed (round 2) |
| 0x005bc130 | `UOObject::GetObjectDefinition` (probable) | renamed (round 2) |
| 0x005bda40 | `UOTileArt::GetObjectDefinition` | renamed |
| 0x005bdac0 | `UOObject::InitFromDefinition` (probable) | renamed (round 2) |
| 0x005befd0 | `UOObject::CreateComponents` (probable) | renamed (round 2) |
| 0x005bfe70 | `UOTileArt::GetGraphicForAmount` (probable) | renamed |
| 0x00654aa0 | `UOHouseCustomization::IsPlacementAllowed` (probable) | renamed (round 2) |
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


## 8. Unknown-field investigation (round 2)

Method: every reader of each definition slot was looked for as a virtual call `[vtable + slot]` in the 41
callers of `UOTileArt::GetObjectDefinition`, then in the functions that receive a definition as an argument
(component `Initialize` slots). Each site was checked in the assembly, because the decompiler mixes up the
definition and the world object: `UOObject` (vftable 0x00cb501c) has its own slots at the same offsets
(+0x40/+0x44 direction, +0x4c set direction, +0x60 tile id, +0x6c..+0x78 body type and alternate animation,
+0xb0 flags1 copy). The first round's "lighting" readers (`FUN_00437e60`, `FUN_005b3690`) and the +0x48/+0x4c
calls in `RequestTileArtTexture` (texture width/height on another object) were such mix-ups.

Data counts are over the 37756 records of the client's `tileart.uop`.

### 8.1 `field_e8` (record byte 0x24, +0xe8): alternate animation

- `UOObject::InitFromDefinition` 0x005bdac0 copies `GetAlternateAnimation` (+0xb8) into the world object with
  `UOObject::SetAlternateAnimation` (vtable +0x74, byte +0x54).
- `FUN_004386a0` (EBX = mobile): takes mobile component 6, the layer-2 item serial (+0x18/+0x1c), finds the
  object (`FUN_005bd9d0`) and returns its `UOObject::GetAlternateAnimation` (+0x78).
- `FUN_00437e60` (action 0, stand) and `FUN_00438010` (actions 1/2, walk/run) pass that value to
  `FUN_008361ed`, the animation action map lookup, which returns entry +0x14 when it is set and +0x10
  otherwise.
- Data: 30 records. Gargoyle two-handed weapons (body type 666), halberds 5182/5183, creature placeholders
  (10000xxx).
- Conclusion: when the item in layer 2 (two-handed) has it, the wearer's stand/walk/run actions use the
  alternate entry of the animation action map. Confidence: high for the mechanism; the alternate
  animations themselves were not looked up.

### 8.2 Sitting words

| Word | Repo | Reader | Meaning | Conf. |
|---|---|---|---|---|
| 0 | `direction_mask` | `UOObject::SetDirection` 0x005b3690, `FUN_00437e60` -> `UOObject::NearestAllowedDirection` 0x0043a1a0; table 0x00dbaf2c maps direction 0 -> 1, 2 -> 2, 4 -> 4, 6 -> 8 | allowed seated facings: 1 north, 2 east, 4 south, 8 west; the requested facing snaps to the nearest allowed one | high |
| 1 | `field_04` | `FUN_00436d40` 0x4375c2..0x437604 (`GetSeatOffsetX` +0xec) | seated mobile sprite x offset; the client draws at `-value` instead of the animation's default | high |
| 2 | `field_08` | same site (`GetSeatOffsetY` +0xf0) | seated mobile sprite y offset, also negated | high |
| 3 | `window_shift` | `BuildSpriteDescriptor` 0x4595b9..0x459684 (+0xf4) | for mask 1 (north) or 8 (west) the chair sprite is split into the chair and a chair-back overlay drawn over the mobile; this value moves the split column | high |
| 4 | `field_10` | none on a definition (`GetSittingField10` +0xf8 has no call site) | not read by the client | medium |
| 5 | `field_14` | none (`GetSittingField14` +0xfc) | not read by the client | medium |

Distributions (170 records with sitting data): word 0 {2: 39, 4: 37, 5: 28, 10: 25, 8: 16, 1: 16, 15: 8,
6: 1}; word 1 {0: 136, 10: 9, 5: 6, 3: 6, -3: 5, -5: 4, ...}; word 2 {16: 101, 20: 27, 18: 13, 0: 10, 22: 8,
13: 5}; word 3 {0: 140, 1: 14, -6: 6, 6: 6, 3: 3, -2: 1}; word 4 {0: 67, 4: 24, 3: 24, 2: 15, 6: 14, 8: 7,
-4: 6, 10: 6, -45: 5, -8: 2}; word 5 {0: 40, 10: 33, 4: 25, 8: 24, 6: 17, 2: 10, -4: 10, -45: 4}. Words 1-5
are signed.

### 8.3 Property keys

props1 and props2 hold the same key/value pairs in 31522 of the 32654 records that have both; 226 records
have only props1, 36 only props2. Where they differ, props2 key 1 carries props1 key 1 (1802) or key 6
(1120), and props2 key 2 carries props1 key 2 (926) or key 5 (869).

Readers with a constant key:

| Key | Repo `PropertyKey` | Reader | Use |
|---|---|---|---|
| 0 | `Weight` | `FUN_005b3ce0` (`UOObject` vtable +0xcc), `FUN_005f3b20` | test `weight > 90`; the consumer was not traced |
| 3 | `Height` | `UOObject::InitFromDefinition` | copied to the world object (+0x40) |
| 6 | `Slot` | `FUN_005b6490` (after HasFlags1 Wearable), `FUN_004d2170`, `UOLua::UserActionEquipItemsAddItem` (`layer - 1 <= 0x18`) | equipment layer |
| 8 | `Appearance` | `GetAppearance` +0xbc, appearance fallback after key 11 | appearance tile id |
| 9 | `Race` | `GetRace` (default 1) | only on 10000xxx bodies |
| 10 | `Gender` | `GetGender` (default 0) | only on 10000xxx bodies |
| 11 | `Paperdoll` | `FUN_0050aef0`, `FUN_0050ba30`, `FUN_0050c550`, `UOPaperdoll::BuildImage` | appearance override: definition of `value + 10000000`, falling back to key 8 |

Keys 1, 2, 4, 5 and 7 are not read with a constant key. They can still be read through an effect
`PropertyFlag` definition (`UOEffectPropertyFlagDefinition::Instantiate` 0x0040d490, key from the effect
data), and `FUN_00437e60`/`FUN_005ea110` read keys from variables. Key 7 (`OffC8`) is set on beds, curtains,
banners, cloth, pillows and towels (values 2..95); no client meaning was found, so the repo name
`OffC8` has no support.

props2 (`HasProperty2`/`GetProperty2` +0x6c/+0x70) is read only by `UOPaperdoll::BuildImage` 0x0050c790:
for each equipped layer the appearance tile is props2 key 11, else props2 key 8, else props1 key 11, else
props1 key 8 (each + 10000000); it then draws gump `id + 60000` for a female wearer (falling back to
`id + 50000`) or `id + 50000`, with HasFlags2 bit 18 (PartialHue) selecting a partial hue.

### 8.4 House join bits 1-5, 14, 15

Readers on a definition: `UOHouseCustomization::IsPlacementAllowed` 0x00654aa0,
`UOHouseCustomization::ComputeObjectSupport` 0x00657e20 (result in cell +0xc0) and
`UOHouseCustomization::PropagateSupportFromBelow` 0x00658e30 -> `MarkSupportAbove` 0x00658fc0 (round 3 names).
Coordinates are relative to the house origin (+0x87710/+0x87714); x grows east, y grows south.

| Bit | Slot | Rule | Data | Conf. |
|---|---|---|---|---|
| 0 | none | not read | 1888 records | unused |
| 1 | +0x100 | cell (0, 0), the north-west corner, needs it | 165 (posts, corners) | high |
| 2 | +0x104 | cells with x = 0 (west edge) need it; in mode 1 also the row y = height (outside the front) at x = 0 | 734 (walls running north-south) | high |
| 3 | +0x108 | cells with y = 0 (north edge) need it | 729 (walls running east-west) | high |
| 4 | +0x10c | not read | 1335 (walls) | unused |
| 5 | +0x110 | the piece rests on the floor (section 8.4.1): placement refuses it on a stairs or teleporter cell or above stairs (`IsClearOfStairs`); the support solver supports (+0xc0) a bit-5 piece directly from its own cell (piece below or floor) or a neighbouring floor, while a piece without bit 5 is supported only through a connected supported neighbour; unsupported pieces are drawn with hue 43 | 1797 (walls, columns) | high |
| 14 | +0x134 | a supported piece with it supports the cell above (section 8.4.1): `PropagateSupportFromBelow` passes it to `MarkSupportAbove`, which sets +0xbc on the cell one level up (plus floor/slot support around it) | 961 (walls, windows, fences) | high |
| 15 | +0x138 | not read on a definition (the +0x138 calls in `FUN_00466940`/`FUN_0044bb40` are on other objects) | 10 records, all roofs (join bits 0x8001) | unused |

Outside the house rectangle, or above the top level, placement is refused before any bit is read.

#### 8.4.1 Support solver (round 3)

House grid: one 0x90-byte cell per (level, x, y), index `(level + (x * 31 + y) * 4) * 0x90` from the house
object; level count +0x876f4, width +0x87728, height +0x8772c. Cell fields: +0x54 floor tile type (1 stairs,
6 teleporter, 8 none; the Lua `CustomizeHouseWindow` uses the same numbers, `TILE_TYPE_STAIRS = 1`,
`TILE_TYPE_TELEPORTERS = 6`), +0x64 floor piece, +0x88 second piece slot, +0xac object piece; support bytes
+0xbc (object supported from below), +0xbd (floor support), +0xbe (slot support), +0xbf (floor supported),
+0xc0 (object supported), +0xc1 (slot supported).

- `IsPlacementAllowed` 0x00654aa0 (ECX = x, EDX = level from +0x18, pushed y): after the edge bits,
  `0x00654bf1` calls `HasHouseJoinBit5` (+0x110); if set, `IsClearOfStairs` 0x00659950 must return true:
  cell +0x54 is neither 6 nor 1 (0x0065996e, 0x0065997a) and the cell one level below is not 1
  (`CMP [EDI-0x3c],1` at 0x006599a6, i.e. +0x54 of the cell 0x90 bytes lower).
- `UpdateSupport` 0x00657d50, per level: `ResetLevelSupport` 0x00658dc0 (+0xbc..+0xbe and +0xc2 = level == 0,
  +0xbf..+0xc1 = 0), `PropagateSupportFromBelow` 0x00658e30, `FUN_006592e0`, `FUN_00658c10` (+0xbf),
  `FUN_00658b10` (+0xc1), `ComputeObjectSupport` 0x00657e20.
- `ComputeObjectSupport` clears +0xc0 on the level (0x00657e80), then for each object (+0xac) that passes the
  edge bits 1-3 (pieces failing them stay unsupported). First loop: only pieces with bit 5 can be supported
  directly (`0x00658182` +0x110, `JZ 0x0065828e` skips the rest); after `IsClearOfStairs`, the piece is
  supported when its own cell has +0xbc or +0xbf, or bit 3 and the +y
  cell has +0xbf (`[EBX+0x2ff]`), or bit 2 and the +x cell (`+0x467f`), or bit 1 and the +x+y cell (`+0x48bf`)
  -> `0x00658287 MOV byte [EBX+0xc0],1`. Second loop (0x006585e2, no bit 5 test): any still unsupported piece not on a
  stairs/teleporter cell (+0x54 6/1 at 0x006585ee/0x006585f3) nor above stairs (0x0065862b) becomes supported
  (0x0065864d) when `IsConnectedToSupportedNeighbour` 0x006586f0 finds a supported piece at x-1, y-1, y+1 or
  x+1 that `TilesConnect` joins to it.
- `PropagateSupportFromBelow` (level > 0): for each cell whose level-below object is supported (+0xc0 at
  0x00658e92) and has bit 14 (`+0x134` at 0x00658ede), calls `MarkSupportAbove` 0x00658fc0, which sets +0xbc on
  the cell above (0x00658fec); with bit 2 the +x cell, else with bit 3 the +y cell, gets +0xbd/+0xbe
  (0x00659060, 0x00659095); a floor on the cell sets +0xbd on its 4 neighbours, a slot piece +0xbe in the 3 x 3
  window.
- Display: `ShowSupport` 0x00657ce0 -> `UpdateCellSupportHues` 0x006505c0 -> `SetPieceSupportHue` 0x0064fc80:
  hue 0x2b (43) on unsupported pieces, 0 otherwise.
- Commit: `UOLua::SendHousingSystemMsg` 0x00656bb0 -> `FUN_00656f80`, message 4 (commit) calls
  `HasUnsupportedPieces` 0x00657c50 (object without +0xc0, slot piece without +0xc1, floor with type != 8
  without +0xbf) and only logs the result (`FUN_0040c6c0`); the commit proceeds.

So bit 5 = "rests on the floor": the piece can stand directly on a floor or on a load-bearing piece below,
and is therefore kept off stairs and teleporters (walls, columns); bit 14 = "supports the level above"
(walls, windows, fences). Other pieces must hang on a connected supported piece. The client only shows and
logs unsupported pieces; it does not block the commit. The earlier names "needs support" and "marks
neighbours" were close but incomplete.

### 8.5 Other confidence upgrades

| Field | Before | Now | Evidence |
|---|---|---|---|
| `body_type` | medium | high | `FUN_004387f0` refuses an appearance whose body type differs from the wearer's body; copied by `UOObject::InitFromDefinition` |
| `light_scale` | probable | high | `UOObjectComponentLighting::Initialize` 0x00449740 (vtable 0x00c9fdfc +4; definition = third stack argument, ESI at 0x449907..0x449988): `GetLightScale` -> light sprite descriptor +0x44 |
| `light_offset_x/y` | probable | high | same function: (0, `GetLightOffsetY`, `GetLightOffsetX`) times the matrix at settings 0x00e37224 +0x7c (+0x2c..+0x4c), added to descriptor +0x68/+0x6c/+0x70. The component exists only when `GetLightShaderName` (+0x3c, block 3) is set (`UOObject::CreateComponents`) |
| `light_field_70` | unknown | unused | no call to +0x44 on a definition; 0 in all records |
| `field_60` | unknown | unused (high, round 3) | read only inside `HasLegacyArtData`; neither slot is called on a definition (section 8.10); 0 in all records |
| `old_id` | medium | unused by the client (high, round 3) | no call to +0x78 on a definition (section 8.10) |
| `facing` | medium | medium (probably unused, round 3) | `UOObjectComponentSprite::Initialize` 0x00444b10 (vtable 0x00c9fc6c +4) stores it at component +0x2c; no reader of +0x2c found (section 8.10) |
| `sampler_index`, `uv_set`, `source_kind` | low, medium, medium | unused / high / high (round 3) | section 8.10 |

### 8.6 Flags

Records with each bit set (flags1 / flags2 / both). Bits 8, 32, 38 and 41-63 are never set. Every bit that
is set has a `TaeFlag` name, so the data has no undefined bits.

| Bit | `TaeFlag` | flags1 | flags2 | both |
|---|---|---:|---:|---:|
| 0 | Background | 7841 | 7838 | 7837 |
| 1 | Weapon | 1032 | 1035 | 1032 |
| 2 | Transparent | 0 | 275 | 0 |
| 3 | Translucent | 1 | 272 | 1 |
| 4 | Wall | 3272 | 3253 | 3253 |
| 5 | Damaging | 0 | 553 | 0 |
| 6 | Impassable | 13083 | 13066 | 13062 |
| 7 | Wet | 405 | 461 | 396 |
| 9 | Surface | 8221 | 8221 | 8221 |
| 10 | Bridge | 577 | 577 | 577 |
| 11 | Generic | 885 | 885 | 885 |
| 12 | Window | 305 | 305 | 305 |
| 13 | NoShoot | 4725 | 4704 | 4704 |
| 14 | ArticleA | 152 | 8361 | 152 |
| 15 | ArticleAn | 3 | 522 | 0 |
| 16 | Mongen | 72 | 72 | 72 |
| 17 | Foliage | 2680 | 2680 | 2680 |
| 18 | PartialHue | 12848 | 12822 | 12820 |
| 19 | UseNewArt | 0 | 795 | 0 |
| 20 | Map | 2 | 2 | 2 |
| 21 | Container | 1662 | 523 | 523 |
| 22 | Wearable | 1126 | 1126 | 1126 |
| 23 | LightSource | 1111 | 1128 | 1109 |
| 24 | Animation | 3665 | 3048 | 2509 |
| 25 | HoverOver | 3 | 3 | 3 |
| 26 | ArtUsed | 0 | 886 | 0 |
| 27 | Armor | 25 | 25 | 25 |
| 28 | Roof | 1040 | 1040 | 1040 |
| 29 | Door | 558 | 558 | 558 |
| 30 | StairBack | 101 | 101 | 101 |
| 31 | StairRight | 4539 | 4539 | 4539 |
| 33 | NoDraw | 102 | 3 | 3 |
| 34 | Rotated (Unused1) | 1658 | 0 | 0 |
| 35 | AlphaBlend | 501 | 58 | 58 |
| 36 | NoShadow | 26 | 26 | 26 |
| 37 | PixelBleed | 11482 | 11482 | 11482 |
| 39 | PlayAnimOnce | 15 | 15 | 15 |
| 40 | MultiMovable | 4 | 4 | 4 |

flags2 is not a copy of flags1: bits 2, 3, 5, 14, 15, 19, 21, 24, 26, 33, 34 and 35 differ widely. The
client tests flags2 only for bits 18 (`UOPaperdoll::BuildImage`), 19 (`BuildSpriteDescriptor`,
`UOObject::CreateComponents`) and 24 (`UOObject::CreateComponents`).

Client tests found. Definition tests are `HasFlags1`/`HasFlags2` with constant masks; object tests are
`UOObject::GetFlags1` (+0xb0, the flags1 copy made by `InitFromDefinition`) followed by `AND`/`TEST`. The
object sites outside `UOObject::CreateComponents`, `FUN_005c28b0` and `FUN_005ea110` were not checked one by
one for the object type.

| Bit | Definition (`HasFlags`) | Object (`GetFlags1` copy) |
|---|---|---|
| 2 | | `FUN_0044f090` |
| 6 | | `FUN_00447360`, `FUN_004d1940`, `FUN_005debb0` |
| 9 | | `FUN_0044e970`, `FUN_0044f090`, `FUN_004d1940`, `FUN_005ccf10`, `FUN_005e3790` |
| 10 | | `FUN_005e3790` |
| 11 | | `FUN_004d0300`, `FUN_004d31d0` |
| 17 | `BuildSpriteDescriptor` | `FUN_0044f090`, `UOObject::CreateComponents` |
| 18 | flags1: `FUN_00595010`; flags2: `UOPaperdoll::BuildImage` | |
| 19 | flags2: `BuildSpriteDescriptor`, `UOObject::CreateComponents` | |
| 21 | | `FUN_004c5a70`, `FUN_004d2330`, `FUN_0056fff0`, `FUN_00570270` |
| 22 | `FUN_005b6490` | |
| 23 | | `UOObject::CreateComponents`, `FUN_005c28b0` |
| 24 | flags2: `UOObject::CreateComponents` | `UOObject::CreateComponents`, `FUN_005ea110` |
| 25 | | `FUN_0044f090`, `UOObject::CreateComponents` |
| 28 | | `FUN_0044e970`, `FUN_0044eab0`, `FUN_0044ee40` |
| 33 | `FUN_00466940`, `FUN_0044bb40`, `FUN_005b6490` | `UOObject::CreateComponents`, `FUN_005c28b0` |
| 34 | `RequestTileArtTexture`, `RequestLegacyTileArtTexture`, `BuildSpriteDescriptor` | |
| 35 | `BuildSpriteDescriptor` | |
| 39 | | `FUN_0042c710` |

No test was found for bits 0, 1, 3, 4, 5, 7, 12-16, 20, 26, 27, 29-31, 36, 37, 38 and 40 (they may be
tested through code these patterns miss, e.g. shifts or the server side). Bit 38 (`Unused2`) is never set
and never tested: unused, high confidence.

### 8.7 Effect definition fields

- Particle `field_30` (u8), round 3: attach mode, see `../ghidra-effects/results/uosa_effects.md`. The only
  reader is `UOObject::CreateComponents` 0x005befd0: for each tileart effect it gets the definition
  (`GetEffect` 0x005bf66e), checks `GetType` == 0 (particle, 0x005bf68c) and then
  `0x005bf69d CMP dword [EAX+0x30],2; JNZ`: value 2 skips attaching the effect to the world object (jump to
  0x005bf7e3). The moving-effect paths (`FUN_0044bb40` at 0x0044c5e2/0x0044c606, `FUN_00466940` at
  0x00466d0a) instantiate the same effects without testing it. 0 and 1 are not told apart anywhere. Data:
  2 on thrown models (boomerang, glaives, `rock02` boulders, `eaglestrike.ems`: tiles 2303, 2305, 2314,
  4962-4973, 16487, 16491, 16492, 16506), 0 on arrow projectiles (3906, 7166, 20346), 1 on the other 550.
  High: value 2 = "projectile only" (shown only while the item flies).
- Sound `value_08` (f32), round 3: not read. `UOSoundEffect::Init` 0x004b88d0 copies it to the instance
  (`FSTP [ECX+0x14]` 0x004b88e8; constructor default 1.0) and no `UOSoundEffect` method (vftable 0x00ca5594)
  reads +0x14; `Start` 0x004b8900 -> `FUN_004b5770` computes its own distance volume. 1.0 on the 2 collection
  sounds. High (unused); probably an intended gain.
- UserPreference mask: the global 0x00e33584 is the `filter` attribute (hex) of the user setting element
  `Effects` (loaded in `FUN_0048c9f0`, saved in `FUN_0048cdf0`). Bit 0 is "footsteps disabled":
  `FUN_0048d330` ORs 1 when `Footsteps.enabled` is false or "0", `FUN_00499860` toggles bit 0 for
  "%s.footsteps.enabled", and 0x00496bf2/0x0048d724 write `Footsteps enabled` back from bit 0. High for
  bit 0; the other bits come only from the settings file. No tileart or collection effect is a
  UserPreference effect.

### 8.8 Other findings

- `UOObject::GetObjectDefinition` 0x005bc130 caches the definition at object +0x44 and, when the tile id
  has no definition, replaces the id with 99999999 and uses that definition.
- `UOObject::CreateComponents` 0x005befd0 picks the sprite component type from HasFlags2 bits 19/24 and the
  object flags (bits 17, 23, 24, 25, 33) and adds a `UOObjectComponentLighting` only when the definition
  has a light shader name.
- Ghidra types: `UOTileArtSitting` fields are now `directions`, `seat_offset_x`, `seat_offset_y`,
  `window_shift`, `field_10`, `field_14`; `UOObjectDefinition`/`TileArtRecordFixed` +0xe8 is
  `alternate_animation` (bool).

### 8.9 Proposed tileart.rs changes

Applied in `lib/uocf` (`tileart.rs`, `effect_definition.rs`) by another agent, committed together with
this round.

1. `TileArtEntry::field_e8: u8` -> `alternate_animation: bool`, doc: "When this item is equipped in layer 2
   (two-handed), the wearer's stand, walk and run actions use the alternate entry of the animation action
   map. Set on gargoyle two-handed weapons and some halberds."
2. `TaeSittingAnimation`:
   - `direction_mask: u32`, doc: "Allowed seated facings: 1 north, 2 east, 4 south, 8 west; the client snaps
     the mobile's facing to the nearest allowed one."
   - `field_04: i32` -> `seat_offset_x: i32`, doc: "Seated mobile sprite x offset; the client draws the
     mobile at `-seat_offset_x`."
   - `field_08: u32` -> `seat_offset_y: i32`, doc: "Seated mobile sprite y offset; the client draws the
     mobile at `-seat_offset_y`." (signed, like the other words)
   - `window_shift: i32`, doc: "For north (1) and west (8) facings the chair sprite is split into the chair
     and a chair-back overlay drawn over the mobile; this moves the split column." (drop "probable")
   - `field_10`, `field_14`: keep the names, doc: "Not read by the client." (drop "looks like a y offset")
3. `body_type`: doc confidence high: "An equipment appearance is only worn by a mobile with this body type
   (400 human male, 666 gargoyle male; creature placeholders hold their body id)."
4. `light_scale`, `light_offset_x`, `light_offset_y`: drop "probable"; doc: "Scale of the light sprite"
   and "Light sprite offset (rotated by the view, added to the sprite position); only used with a light
   shader block." `light_field_70`: doc "0 in all records; not read by the client."
5. `field_60`: doc "0 in all records; only part of the client's legacy-art-data test."
6. `facing`: doc "Copied to the sprite component; use not traced (medium)."
7. `house_join_bits`: document the bits and add named constants, e.g. `HOUSE_JOIN_CORNER = 1 << 1`
   (allowed on the north-west corner cell), `HOUSE_JOIN_WEST_EDGE = 1 << 2` (x = 0), `HOUSE_JOIN_NORTH_EDGE
   = 1 << 3` (y = 0), `HOUSE_JOIN_NEEDS_SUPPORT = 1 << 5` (probable), `HOUSE_JOIN_MARKS_NEIGHBOURS = 1 << 14`
   (probable), connectors 6..13 as already documented; bits 0, 4, 15 "not read by the client".
8. `TaeFlag::Unused2` (bit 38): confirmed unused (never set, never tested). Doc on the flags2 field:
   "Not a copy of flags1; the client tests only bits 18 (paperdoll partial hue), 19 (UseNewArt) and 24
   (Animation) in it."
9. `PropertyKey`: `Slot` -> `Layer` (equipment layer); `Paperdoll` doc "appearance override: the
   definition of `value + 10000000`, used before `Appearance`"; `Weight` doc "the client tests
   `weight > 90`"; `Height` doc "copied to the world object"; `OffC8` -> `Key7` (no client meaning; set
   on beds, curtains, banners, cloth). props2 doc: "Mostly mirrors props1; the client reads it only for
   the paperdoll gump (keys 11 and 8, before props1)."
10. `effect_definition.rs` (not tileart.rs): add `USER_PREFERENCE_FOOTSTEPS: u32 = 0x1` for the
    UserPreference mask; keep particle `field_30` and sound `value_08` unknown, with the data notes above.

### 8.10 Round 3: fields confirmed unread, texture items, facing

Method as in section 8; probe scripts `~/ghidra_scripts/ProbeUnknownFields01.java` (slot callers, register
chains, displacement scans), `ProbeHouseSupport02.java` (call graph of the house support code) and
`ProbeComments03.java` (stale plate comments). The 52 definition users are the callers of
`UOTileArt::GetObjectDefinition` plus the component `Initialize` slots.

- Sitting words 4 and 5 (`field_10`/`field_14`): the only virtual calls to +0xf8/+0xfc are on objects that are
  not definitions. A scan of all 319 loads of `[X+0xc0]` (the sitting shared_ptr) found no `[R+0x10]` or
  `[R+0x14]` read of the sitting record; `UOTileArt::ParseSitting` 0x00845168 only passes it to `SetSitting`.
  Not exposed to Lua (the only tileart Lua bindings are the texture requests). High: not read.
- `field_60`: slot +0x34 (`GetField60`) is called in `UOObject::InitFromDefinition` 0x005bdb6c/0x005be2c0,
  `FUN_005bbe30` 0x005bbf85, `FUN_005db6f0` 0x005db93d and `FUN_005ea110` 0x005ea531/0x005eaac6, all on world
  objects (e.g. at 0x005bdb6c ECX is the new 0x98-byte `UOObject` from 0x005b32d0, and the call passes an
  argument, a setter). Slot +0x38 (`HasLegacyArtData`, the only other reader of +0x60) has 188 callers and
  none among the definition users. High: unused.
- `old_id`: none of the 32 callers of slot +0x78 is a definition user. High: unused by the client; the value
  only documents the classic id the art came from.
- `facing`: `UOObjectComponentSprite::Initialize` 0x00444b10 stores it (`0x00444c53 CALL [vt+0x88]`,
  `0x00444c5b MOV [EBX+0x2c],EAX`). None of the 29 `UOObjectComponentSprite` vtable functions (0x00c9fc6c)
  nor the overrides of the foliage subclass (0x00c9fce4) reads component +0x2c; there is no
  `dynamic_cast` to the sprite type (TD 0x00de41ec); the graphics base constructor 0x00433620 only zeroes it
  (0x00433679). The animation component uses its own +0x2c as a shared_ptr. Medium: probably unused (a reader
  through a non-virtual path cannot be fully excluded).
- Texture items, material builder 0x0045cd80: item 0 becomes shader map 0 (`SetShaderMap(0, map)`, 0x0045e0a5)
  and items 1..n shader maps 1..n (loop from index 1 at 0x0045e145, slot = loop index), so the slot is the item
  position. Correction from `ghidra-texture-roles/results/uosa_texture_roles.md`: an earlier version of this note said item 0 becomes the base map. The
  base map is `Build\Hues\Hues.dds`, set by `NiTexturingProperty::SetBaseTexture` (call 0x0045e666).
  - `uv_set`: `MOV CX,word [item+0x28]; AND DX,0xff00; OR CX,DX` into the map flags (0x0045de94, 0x0045e1ec),
    the Gamebryo texture coordinate set index. High.
  - `stretch`: compared with 1.0 (0x0045e200) -> clamp bits 12-13 = 3 (wrap) when different; UV scale
    1/stretch (0x0045e076, 0x0045e57c). High.
  - `source_kind` (0x0045e23f): 0 -> load through the settings texture loader (+0xa8 vtable +4, mode 4;
    0x0045e301); 2 -> the same load, then `OR word [map+4],0x3000` (wrap, 0x0045e2f2); other values skip the
    load (0x0045e3d1). High.
  - `sampler_index`: no read of item +0x24 in the builder (`[ESI+0x24]` at 0x0045d02a is the sprite descriptor,
    0x0045dae2/0x0045db1e are vtable slots), in `GetPrimaryTextureLocation`, `BuildSpriteDescriptor` or
    `GetCachedSpriteDescriptor`. High: unused.
  - Data (repo parser, scratch tool): `source_kind` 0 on all 198182 items. `sampler_index` equals the item
    position except in block 1: position 1 holds 2 on 249 records and 0 on 1; position 2 holds 0 on 917
    and 1 on 1; position 3 holds 2 on 1; block 0 position 0 holds 1 on 1 record. Repo code that matches
    items by `sampler_index` (`udd-assets/src/tex_land_ec.rs`, water lookup on block 0) works on current
    data, but the client order is the item position.

Ghidra names added this round (`NameTileartCode.java`, tag `UO_TILEART`): `UOHouseCustomization::`
`IsClearOfStairs` 0x00659950, `IsNotAboveStairs` 0x006599c0 (probable), `UpdateSupport` 0x00657d50
(probable), `ResetLevelSupport` 0x00658dc0, `PropagateSupportFromBelow` 0x00658e30, `MarkSupportAbove`
0x00658fc0, `ComputeObjectSupport` 0x00657e20, `IsConnectedToSupportedNeighbour` 0x006586f0,
`ShowSupport` 0x00657ce0 (probable), `UpdateCellSupportHues` 0x006505c0 (probable), `SetPieceSupportHue`
0x0064fc80 (probable), `HasUnsupportedPieces` 0x00657c50. The script now also refreshes its own single-line
plate comments when the evidence text changed (26 refreshed, e.g. the old `GetFieldE8` text in
`UOObject::InitFromDefinition`); comments edited by hand are kept.

### 8.11 Proposed Rust changes (round 3)

Not applied; for `lib/uocf/src/enhanced/tileart.rs` and `effect_definition.rs`.

1. House join constants:
   - `HOUSE_JOIN_NEEDS_SUPPORT` -> `HOUSE_JOIN_RESTS_ON_FLOOR: u32 = 1 << 5`, doc: "House placement: the
     piece can stand directly on a floor or on a load-bearing piece below (`HOUSE_JOIN_SUPPORTS_ABOVE`), so
     the client refuses it on stairs and teleporter cells and above stairs. Pieces without it are supported
     only by a connected supported neighbour; the client draws unsupported pieces with hue 43."
   - `HOUSE_JOIN_MARKS_NEIGHBOURS` -> `HOUSE_JOIN_SUPPORTS_ABOVE: u32 = 1 << 14`, doc: "House placement: a
     supported piece with this bit supports the cell above it, and with the west/north edge bits also the
     floor of the next cell above."
   - Optional: `HOUSE_UNSUPPORTED_HUE: u16 = 43` where the repo shows support.
   - `house_join_bits` doc: replace "placement rules (bits 1, 2, 3, 5, 14)" with "placement rules (bits 1,
     2, 3), floor support (bit 5), support of the level above (bit 14)".
2. `TileArtEntry::field_60` doc: "0 in all client records; not read by the client." `old_id` doc:
   "Classic tile id the art replaces (default 0xffffff); not read by the client." `facing` doc: "Copied to
   the sprite component (client default 1); no client reader found (probably unused)."
3. `TaeTextureImage` (and `tileart.rs` 202-207 equivalents):
   - `source_kind` doc: "0 loads the texture; 2 loads it and forces wrap addressing; any other value makes
     the client skip the texture. 0 in all client records." Constants
     `TEXTURE_SOURCE_NORMAL: u8 = 0`, `TEXTURE_SOURCE_WRAP: u8 = 2`.
   - `sampler_index` doc: "Not read by the client, which binds item i to shader map i; usually equal to
     the item position."
   - `uv_set` doc: "Texture coordinate set index (low byte of the Gamebryo texture map flags); 0 or 1."
   - Repo users matching on `sampler_index` (`udd-assets/src/tex_land_ec.rs`) could use the item position
     instead; same result on current data.
4. `TaeSittingAnimation::field_10`/`field_14`: keep "Not read by the client." (now high).
5. `effect_definition.rs`: see `../ghidra-effects/results/uosa_effects.md` section "Proposed Rust changes":
   particle `field_30` -> `attach_mode` with `PARTICLE_ATTACH_PROJECTILE_ONLY: u8 = 2`; `field_3c` and sound
   `value_08` docs.

## 9. Open questions

- Alternate animation entries: which actions the alternate map entries hold (animation data, not tileart).
- `facing`: stored at sprite component +0x2c; no reader found (a non-virtual reader is not excluded).
- Weight > 90 test (`UOObject` vtable +0xcc): what the caller does with it.
- House support: `FUN_006592e0`, `FUN_00658c10` (floor support +0xbf) and `FUN_00658b10` (slot support +0xc1)
  were not analysed in detail; they do not read join bits.
- props1 keys 1, 2, 4, 5, 7: only reachable through effect or variable keys; no meaning found.
- Settled as not read by the client: sitting words 4 and 5, `old_id`, `light_field_70`, `field_60`, texture
  item `sampler_index`, sound `value_08` (section 8.10, effects document).
