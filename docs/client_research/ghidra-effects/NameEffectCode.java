// Name the effect definition code of UOSA.exe: the factory list that parses the effect definitions at the end
// of tileart.uop records and in EffectDefinitionCollection.uop, each factory's CanHandle/Create/NewInstance/Init,
// each definition class's Instantiate/GetType, the shared list/map readers, the collection asset parser and the
// runtime effect slots whose behaviour explains the definition fields. With --apply, rename DEFAULT functions,
// add the UO_EFFECTS tag, plate comments and /UOEffects types.
// @category ECResearch
import ghidra.app.script.GhidraScript;
import ghidra.program.model.address.Address;
import ghidra.program.model.data.ArrayDataType;
import ghidra.program.model.data.BooleanDataType;
import ghidra.program.model.data.ByteDataType;
import ghidra.program.model.data.CategoryPath;
import ghidra.program.model.data.CharDataType;
import ghidra.program.model.data.DataType;
import ghidra.program.model.data.DataTypeConflictHandler;
import ghidra.program.model.data.DataTypeManager;
import ghidra.program.model.data.DoubleDataType;
import ghidra.program.model.data.FloatDataType;
import ghidra.program.model.data.IntegerDataType;
import ghidra.program.model.data.PointerDataType;
import ghidra.program.model.data.QWordDataType;
import ghidra.program.model.data.SignedByteDataType;
import ghidra.program.model.data.StructureDataType;
import ghidra.program.model.data.UnsignedIntegerDataType;
import ghidra.program.model.data.UnsignedShortDataType;
import ghidra.program.model.data.VoidDataType;
import ghidra.program.model.lang.Register;
import ghidra.program.model.listing.Function;
import ghidra.program.model.listing.FunctionManager;
import ghidra.program.model.listing.FunctionTag;
import ghidra.program.model.listing.Instruction;
import ghidra.program.model.listing.Listing;
import ghidra.program.model.mem.Memory;
import ghidra.program.model.mem.MemoryBlock;
import ghidra.program.model.scalar.Scalar;
import ghidra.program.model.symbol.Namespace;
import ghidra.program.model.symbol.RefType;
import ghidra.program.model.symbol.Reference;
import ghidra.program.model.symbol.SourceType;
import ghidra.program.model.symbol.Symbol;
import ghidra.program.model.symbol.SymbolTable;
import ghidra.util.exception.DuplicateNameException;
import java.io.PrintWriter;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardOpenOption;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.LinkedHashMap;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.TreeMap;

public class NameEffectCode extends GhidraScript {
    private static final String APPLY_FLAG = "--apply";
    private static final String TAG = "UO_EFFECTS";
    private static final String TAG_COMMENT = "effect definition factories, readers, definition classes and runtime effects";
    private static final String COMMENT_PREFIX = "NameEffectCode: ";
    private static final String PROBABLE = " (probable)";
    private static final String SCOPE = "::";
    private static final String NAMESPACE = "UOEffects";
    private static final CategoryPath CATEGORY = new CategoryPath("/UOEffects");
    private static final String FACTORY_LIST_LABEL = "g_UOEffectDefinitionFactories";
    private static final String MEMCPY = "_memcpy";

    // MSVC RTTI: TypeDescriptor name at +8; CompleteObjectLocator {signature, offset, cdOffset, pTypeDescriptor}.
    private static final int TYPE_DESCRIPTOR_NAME_OFFSET = 8;
    private static final int COL_TYPE_DESCRIPTOR_OFFSET = 12;
    private static final int COL_OFFSET_FIELD = 4;
    private static final String RTTI_PREFIX = ".?AV";
    private static final String RTTI_SUFFIX = "@@";

    private static final int POINTER_SIZE = 4;
    private static final long DWORD_MASK = 0xffffffffL;
    private static final int BYTE_MASK = 0xff;
    private static final int BITS_PER_BYTE = 8;
    private static final int HEX_RADIX = 16;
    // A thin wrapper: functions at most this large.
    private static final int SMALL_FUNCTION = 0x40;
    private static final int UNKNOWN_KEY = -1;

    // Effect factory vtable (IUOEffectDefinitionFactory): +0 CanHandle(key), +8 Create(out, stream, ctrl, pos, ctx),
    // +0x10 scalar deleting destructor, +0x14 NewInstance(out), +0x18 Init(sp, fields...).
    private static final int FACTORY_CAN_HANDLE = 0x0;
    private static final int FACTORY_CREATE = 0x8;
    private static final int FACTORY_DESTRUCTOR = 0x10;
    private static final int FACTORY_NEW_INSTANCE = 0x14;
    private static final int FACTORY_INIT = 0x18;
    // Effect definition vtable (IUOEffectDefinition): +0 destructor, +8 Instantiate(out ok, out effect, object),
    // +0xc GetType.
    private static final int DEFINITION_DESTRUCTOR = 0x0;
    private static final int DEFINITION_INSTANTIATE = 0x8;
    private static final int DEFINITION_GET_TYPE = 0xc;
    // UOEffectDefinitionCollectionBinaryFactory (asset factory): +4 Create.
    private static final int COLLECTION_FACTORY_CREATE = 0x4;

    private static final String COLLECTION_FACTORY = "UOEffectDefinitionCollectionBinaryFactory";
    private static final String COLLECTION = "BinaryUOEffectDefinitionCollection";
    private static final String CONTAINER = "UOEffectContainer";
    private static final String COMPONENT = "UOObjectComponentEffects";
    private static final String PROJECTILE_EFFECT = "UOProjectileEffect";
    private static final String PROJECTILE_DEFINITION = "UOProjectileEffectDefinition";
    private static final String TILE_DEFINITION = "UOTileEffectDefinition";

    // Runtime use of the definitions (InstantiateEffectById and the graphical effect packets).
    private static final String MISSING_EFFECT_STRING = "Missing Effect Definition %d\n";
    private static final int EFFECT_INSTANTIATE = 0x8;
    private static final int EFFECT_UPDATE = 0x14;
    private static final int EFFECT_SET_LIFETIME = 0x30;
    private static final long TARGET_INDICATOR_EFFECT_ID = 10001;
    private static final int PACKET_COPIER_COUNT = 3;
    // Packet ids of the copiers ordered by the largest packet field offset they read (0x1e, 0x24, 0x36).
    private static final String[][] EFFECT_PACKETS = {
        {"0x70", "GraphicalEffect", "type, serials, item id, positions, speed, duration, fixed direction, explode"},
        {"0xC0", "HuedEffect", "0x70 fields plus hue (+0x20) and render mode (+0x24)"},
        {"0xC7", "ParticleEffect", "0xC0 fields plus effect (+0x28), explode effect (+0x2a), explode sound (+0x2c),"
            + " serial (+0x30), layer (+0x34) and two bytes"},
    };
    // Hued Nif effects, the camera every effect node faces and effects played on objects (results/uosa_effects.md
    // "Hued NIF effects", "Effect camera and lighting", "Frame effects on objects").
    private static final String PARTICLE_DEFINITION = "UOParticleEffectDefinition";
    private static final String HUED_NIF = "UOHuedParticleEffectNif";
    private static final String TILE_EFFECT = "UOTileEffect";
    private static final String DEFINITION_INTERFACE = "IUOEffectDefinition";
    private static final String WORLD_OBJECT = "UOObject";
    private static final String WORLD_VIEWER = "UOWorldViewer";
    private static final String EFFECTS_SHADER_HLSL = "UOEffectsShaderHLSL";
    private static final String EFFECTS_SHADER_FFP = "UOEffectsShaderFFP";
    private static final String HUE_ATLAS_STRING = "Build\\Hues\\Hues.dds";
    private static final String EFFECTS_SHADER_STRING = "UOEffectsShader";
    private static final String WORLD_VIEWER_INIT_STRING = "UOWorldViewer::Init()\n";
    private static final String DRAW_VISIBLE_ARRAY = "NiDrawVisibleArray";
    private static final String SET_VIEW_FRUSTUM = "SetViewFrustum";
    private static final String WORLD_VIEWER_LABEL = "g_UOWorldViewer";
    // IUOEffectDefinition +4 PlayOnObject(out ok, object); IUOEffect +8 SetVisible, +0x38 Init(out ok, definition).
    private static final int DEFINITION_PLAY_ON_OBJECT = 0x4;
    private static final int EFFECT_SET_VISIBLE = 0x8;
    private static final int EFFECT_INIT = 0x38;
    // NiD3DShader +0x1c UpdatePipeline.
    private static final int SHADER_UPDATE_PIPELINE = 0x1c;
    private static final String ACTIVE_EFFECTS_LABEL = "g_UOActiveEffects";
    private static final String LOD_LABEL = "g_UOParticleLODLevel";
    private static final String FILTER_LABEL = "g_UOParticleFilter";
    private static final String USER_FILTER_LABEL = "g_UOUserEffectFilter";

    // Effect rendering: render groups, renderer background colour, texture loading (results/uosa_effects.md
    // "Effect camera and lighting", "Effect textures and mipmaps").
    private static final String NIF_EFFECT = "UOParticleEffectNif";
    // IUOEffect +0x18 Release.
    private static final int EFFECT_RELEASE = 0x18;
    private static final String RENDER_GROUPS_LABEL = "g_UOEffectRenderGroups";
    // The render group effect NIFs attach to.
    private static final int EFFECT_RENDER_GROUP = 7;
    private static final String DX9_RENDERER = "NiDX9Renderer";
    private static final String DX9_GET_RTTI = DX9_RENDERER + "::GetRTTI";
    // NiDX9Renderer vtable: +8 GetRTTI, +0x58 SetBackgroundColor(NiColorA), +0x5c SetBackgroundColor(NiColor),
    // +0x60 GetBackgroundColor(NiColorA), +0x98 UseLegacyPipelineAsDefaultMaterial.
    private static final int RENDERER_GET_RTTI = 0x8;
    private static final int RENDERER_SET_BACKGROUND_RGBA = 0x58;
    private static final int RENDERER_SET_BACKGROUND_RGB = 0x5c;
    private static final int RENDERER_GET_BACKGROUND = 0x60;
    private static final int RENDERER_USE_LEGACY_PIPELINE = 0x98;
    private static final String RENDER_TO_TEXTURE = HUED_NIF + "::RenderToTexture";
    private static final String SOURCE_TEXTURE_DATA = "NiDX9SourceTextureData";
    private static final String CREATE_SURF_FROM_RENDERER_DATA = SOURCE_TEXTURE_DATA + "::CreateSurfFromRendererData";
    private static final String MIPMAP_BY_DEFAULT_LABEL = "NiTexture_ms_bMipmapByDefault";
    private static final String DEV_IMAGE_CONVERTER = "NiDevImageConverter";
    private static final String GENERATE_MIPMAP_LEVELS = DEV_IMAGE_CONVERTER + "::GenerateMipmapLevels";
    private static final String DEV_IMAGE_CONVERTER_DTOR = DEV_IMAGE_CONVERTER + "::~" + DEV_IMAGE_CONVERTER;
    // NiImageConverter vtable: +0xc ReadImageFile, +0x14 ConvertPixelData.
    private static final int CONVERTER_READ_IMAGE_FILE = 0xc;
    private static final int CONVERTER_CONVERT_PIXEL_DATA = 0x14;
    // Distance from the vtable start to the GenerateMipmapLevels slot is at most this many slots.
    private static final int CONVERTER_MAX_SLOTS = 32;
    private static final String NIF_IMAGE_READER = "NiNIFImageReader";
    private static final String NIF_EXTENSION = ".nif";
    // NiImageReader vtable: +0 ScalarDeletingDestructor, +4 CanReadFile, +8 ReadFile, +0xc ReadHeader.
    private static final int READER_DESTRUCTOR = 0x0;
    private static final int READER_CAN_READ_FILE = 0x4;
    private static final int READER_READ_FILE = 0x8;
    private static final int READER_READ_HEADER = 0xc;

    // Draw order, NiStandardMaterial texture handlers, render targets and Chaos resources (drawOrderAndMaterials).
    // Gamebryo classes carry no MSVC RTTI: their vtables come from the NiRTTI object (name pointer at +0) that
    // GetRTTI returns, GetRTTI sitting at vtable +8.
    private static final int NI_GET_RTTI_SLOT = 0x8;
    // Byte size above which a function cannot be a GetRTTI (MOV EAX, imm32; RET).
    private static final int GET_RTTI_MAX_SIZE = 8;
    // Render group drawn after the effects group (7) in the world pass.
    private static final int GROUP_AFTER_EFFECTS = 9;
    private static final String SORT_ADJUST_NODE = "NiSortAdjustNode";
    private static final int NODE_ON_VISIBLE = 0x80;
    private static final String BOUND_UPDATE_MODIFIER = "NiPSysBoundUpdateModifier";
    private static final int MODIFIER_UPDATE = 0x48;
    private static final String ALPHA_ACCUMULATOR = "NiAlphaAccumulator";
    private static final String SHADER_OVERRIDE_SORTER = "UOShaderOverrideSorter";
    private static final int SORTER_REGISTER_OBJECT_ARRAY = 0x50;

    // Texture filtering (textureFiltering; results "Texture filtering and anisotropy").
    private static final String DEVICE_CAPS_ERROR = "Creation failed: Device does not support single-pass multi-texture";
    // Gamebryo 2.3 FilterMode count (NEAREST .. BILERP_MIPNEAREST; no ANISOTROPIC) and ClampMode count.
    private static final int FILTER_MODES = 6;
    private static final int CLAMP_MODES = 4;
    // Words per NiDX9Renderer filter mapping entry (mag, min, mip) and address mapping entry (u, v).
    private static final int FILTER_MAPPING_WORDS = 3;
    private static final int ADDRESS_MAPPING_WORDS = 2;
    private static final String FILTER_MAPPING_LABEL = "NiDX9Renderer_ms_akD3DFilterMapping";
    private static final String ADDRESS_MAPPING_LABEL = "NiDX9Renderer_ms_akD3DAddressMapping";
    private static final String INITIAL_SAMPLER_STATES_LABEL = "NiDX9RenderState_ms_akInitialSamplerStateSettings";
    // NiDX9RenderState::ms_akInitialSamplerStateSettings: {D3DSAMPLERSTATETYPE, value} pairs, ADDRESSU .. DMAPOFFSET
    // (value 10 = D3DSAMP_MAXANISOTROPY, initial 1), then the {-1, -1} terminator.
    private static final long[] INITIAL_SAMPLER_STATES = {
        1, 1, 2, 1, 3, 1, 4, 0, 5, 1, 6, 1, 7, 0, 8, 0, 9, 0, 10, 1, 11, 0, 12, 0, 13, 256, DWORD_MASK, DWORD_MASK};
    private static final String STANDARD_MATERIAL = "NiStandardMaterial";
    private static final String RENDERED_TEXTURE = "NiRenderedTexture";
    private static final String BUILD_HUE_QUAD = HUED_NIF + "::BuildHueQuad";
    private static final String NEAREST_FRAME_BUFFER_FORMAT = "NiDX9DeviceDesc::GetNearestFrameBufferFormat";
    private static final String MULTISAMPLE_TYPE = DX9_RENDERER + "::GetMultiSampleType";
    private static final String SOURCE_TEXTURE = "NiSourceTexture";
    private static final String CREATE_RENDERER_DATA = SOURCE_TEXTURE + "::CreateRendererData";
    private static final String DEFAULT_PREFS_LABEL = "NiSourceTexture_ms_kDefaultPrefs";
    private static final String LOAD_DIRECT_HINT_LABEL = "g_UOTextureLoadDirectHint";
    // NiTexture::FormatPrefs {PixelLayout, AlphaFormat, MipFlag} defaults: PIX_DEFAULT, ALPHA_DEFAULT, MIP_DEFAULT.
    private static final long[] DEFAULT_FORMAT_PREFS = {6, 3, 2};
    // NiAVObject vtable +0x64: Update(time, update controllers).
    private static final int AV_OBJECT_UPDATE = 0x64;
    private static final String EFFECTS_DIRECTORY = "Data/Effects";
    // NiStandardMaterial texture handlers (vtable offset, name, evidence); the order matches the Gamebryo 2.6 header
    // (HandleParallaxMap .. HandleProjShadowMap), and the decompiled InsertTexture arguments confirm the map slots.
    private static final String[][] STANDARD_MATERIAL_HANDLERS = {
        {"0xd0", "HandleParallaxMap", "parallax map (map 0): offsets the UVs; not used by effect NIFs"},
        {"0xd4", "HandleBaseMap", "base map (map 1): InsertTexture MULTIPLY into the texture accumulator, RGBA (alpha"
            + " only when the opacity is wanted), then opacity *= base alpha"},
        {"0xd8", "HandleNormalMap", "normal map (map 2): only with lights and tangents"},
        {"0xdc", "HandleDarkMap", "dark map (map 3): InsertTexture ADD into the RGB accumulator; first stage, so it"
            + " acts as a multiplier of the later stages"},
        {"0xe0", "HandleDetailMap", "detail map (map 4): accumulator.rgb *= detail.rgb * 2.0; alpha untouched"},
        {"0xe4", "HandleGlossMap", "gloss map (map 6): scales the specular/env term; RGB"},
        {"0xe8", "HandleBumpMap", "bump map (map 5): perturbs the env map lookup; RGB"},
        {"0xec", "HandleGlowMap", "glow map (map 7): InsertTexture ADD of glow.rgb after lighting (same pass);"
            + " alpha untouched"},
        {"0xf0", "HandleCustomMaps", "returns true: no custom maps"},
        {"0xf4", "HandleEnvMap", "environment map from an NiTextureEffect (sphere, specular cube, diffuse cube,"
            + " world parallel/perspective) in the effect state; added to the specular term"},
        {"0xf8", "HandleDecalMap", "decal map n (map 0xd + n): InsertTexture LERP rgb by decal alpha into the texture"
            + " accumulator (before the material colour multiply); alpha untouched"},
        {"0xfc", "HandleProjLightMap", "projected light map from an NiTextureEffect"},
        {"0x100", "HandleProjShadowMap", "projected shadow map from an NiTextureEffect"},
    };

    // Effect type keys (the u32 before each definition body).
    private static final int KEY_PARTICLE = 0;
    // Particle definition +0x30 value that UOObject::CreateComponents does not attach to the object.
    private static final int PARTICLE_ATTACH_PROJECTILE_ONLY = 2;
    private static final int KEY_SOUND = 1;
    private static final int KEY_ROUND_ROBIN = 2;
    private static final int KEY_POSITION_OFFSET = 7;
    private static final int KEY_WAIT = 10;
    private static final int KEY_LOD = 11;
    private static final int KEY_PROPERTY_FLAG = 12;
    private static final int KEY_EARTHQUAKE = 15;
    private static final int KEY_RANDOM = 16;
    private static final int KEY_USER_PREFERENCE = 17;
    // Create functions reading an effect list through the shared reader.
    private static final int[] LIST_READER_KEYS = {KEY_WAIT, KEY_PROPERTY_FLAG, KEY_RANDOM, KEY_USER_PREFERENCE};

    private record EffectType(int key, String factory, String baseFactory, String definition, String create,
            String init, String instantiate, boolean instantiateProbable) {
    }

    // Registered factories (StartUp state), key from CanHandle and the definition's GetType.
    private static final EffectType[] EFFECT_TYPES = {
        new EffectType(KEY_PARTICLE, "UOParticleEffectBinaryFactory", "UOParticleEffectBinaryBaseFactory", "UOParticleEffectDefinition",
            "u32 name string index (.nif or .ems), f32 offset_x, f32 offset_y, f32 camera_offset, f32 scale, u8 attach_mode,"
                + " f32 duration, u8 loop (!= 0), u8 field_3c, u8 hued; NewInstance(name), Init(fields)",
            "stores the particle fields at +0x20..+0x40",
            "'.ems' in the name -> UOParticleEffectChaos; object hue (vtable +0x58) != 0 and hued (+0x40) ->"
                + " UOHuedParticleEffectNif; else UOParticleEffectNif; then the effect Init from the definition", false),
        new EffectType(KEY_SOUND, "UOSoundEffectBinaryFactory", "UOSoundEffectBinaryBaseFactory", "UOSoundEffectDefinition",
            "u32 sound id, f32 value_08 (1.0 in the data)", "stores sound id at +4 and value at +8",
            "creates a UOSoundEffect (0x1c bytes) and copies sound id and value_08 (to +0x14, never read)", false),
        new EffectType(KEY_ROUND_ROBIN, "UORoundRobinEffectBinaryFactory", "UORoundRobinEffectBinaryBaseFactory",
            "UORoundRobinEffectCollection",
            "u32 count, count x ParseEffectDefinition, each appended through Init (vtable +0x18)",
            "appends one child definition to the vector at +4",
            "instantiates the child at the index +0x14 directly (no container) and advances the index modulo the"
                + " child count; the index lives in the definition, so it is shared by every use of the definition",
            false),
        new EffectType(KEY_POSITION_OFFSET, "UOPositionOffsetBinaryFactory", "UOPositionOffsetBinaryBaseFactory",
            "UOPositionOffsetDefinition", "8 x {i8 x, i8 y} (sign-extended), Init(direction, x, y) per entry",
            "stores x[direction] at +4 + 4*direction and y[direction] at +0x24 + 4*direction",
            "creates a UOPositionOffset (0x50 bytes) holding the 8 x/y offsets", false),
        new EffectType(KEY_WAIT, "UOWaitEffectBinaryFactory", "UOWaitEffectBinaryBaseFactory", "UOWaitCollectionDefinition",
            "effect list (u32 count + definitions), u32 min_delay_ms, u32 max_delay_ms, u8 aligned (stored as == 1;"
                + " values other than 0/1 mark the parse context failed)",
            "stores children, min (+0x14, default 2000), max (+0x18, default 4000), aligned (+0x1c)",
            "QueryPerformanceCounter ms; fires when now >= next (+0x20): next = now + min (aligned: next multiple of"
                + " min) + MT19937 uniform [0, max - min]; instantiates all children", false),
        new EffectType(KEY_LOD, "UOLODEffectBinaryFactory", "UOLODEffectBinaryBaseFactory", "UOEffectLODDefinition",
            "effect map: u32 count, count x {u32 LOD level, effect list}", "inserts one level -> children entry",
            "instantiates the children of the entry for the particle LOD level (g_UOParticleLODLevel), falling back"
                + " to lower keys down to 0", false),
        new EffectType(KEY_PROPERTY_FLAG, "UOPropertyFlagEffectBinaryFactory", "UOPropertyFlagEffectBinaryBaseFactory",
            "UOEffectPropertyFlagDefinition",
            "u8 mode (0/1; other values mark the parse context failed), u64 mask, u32 property, default effect list,"
                + " effect map {u32 value -> list}",
            "stores mode (+4), mask (+8), property (+0x10), default children (+0x14) and the value map (+0x24)",
            "mode 0: key = (object flags (vtable +0xb0) & mask) != 0; mode 1: key = object value for property 2"
                + " (+0x80 & 0xffff), 3 (+0xb4), 7 (hue, +0x58), 8 (+0x68), else the tile definition GetProperty1(property);"
                + " children of the matching map entry, mode 1 falls back to the default children", false),
        new EffectType(KEY_EARTHQUAKE, "UOEarthquakeEffectBinaryFactory", "UOEarthquakeEffectBinaryBaseFactory",
            "UOEarthquakeEffectDefinition", "f64 amplitude, f64 duration (s), f64 frequency (Hz)",
            "stores the doubles at +8, +0x10, +0x18", "creates a UOEarthquakeEffect (0x28 bytes) with the three doubles", false),
        new EffectType(KEY_RANDOM, "UORandomEffectCollectionBinaryFactory", "UORandomEffectCollectionBinaryBaseFactory",
            "UORandomEffectCollectionDefinition", "u32 count, count x {f32 weight, effect list}",
            "appends one {weight, children} entry (0x14 bytes)",
            "r = MT19937 u32 * 2^-32 * 100 (a percentage); walks the entries subtracting weights; instantiates the"
                + " first entry with r < weight; nothing when the weights sum to r or less", false),
        new EffectType(KEY_USER_PREFERENCE, "UOUserPreferenceEffectFilterBinaryFactory", "UOUserPreferenceEffectFilterBinaryBaseFactory",
            "UOUserPrefEffectFilterDefinition", "u32 mask, effect list", "stores mask (+4) and children (+8)",
            "skips the children when (mask & user effect setting) == mask, or mask == 1 and object vtable +0xa0 < 0;"
                + " otherwise instantiates them", true),
    };

    private record Slot(String cls, int offset, String name, String evidence, boolean probable) {
    }

    // Runtime effects (IUOEffect): +0 Start, +4 Stop, +0x10 IsFinished, +0x14 Update(dt), +0x1c/+0x20 position;
    // +0x34 destructor and +0x38 Init(definition) except UOEarthquakeEffect (Init at +0x34, no destructor slot).
    private static final Slot[] RUNTIME_SLOTS = {
        new Slot("UOParticleEffectNif", 0x0, "Start", "elapsed (+0x14) = 0, active (+0x24) and started (+0x2c) set", false),
        new Slot("UOParticleEffectNif", 0x4, "Stop", "clears active (+0x24)", false),
        new Slot("UOParticleEffectNif", 0x10, "IsFinished", "false before Start; true when the countdown (+0x40)"
            + " is 0 or active (+0x24) is cleared", true),
        new Slot("UOParticleEffectNif", 0x14, "Update", "(out, dt): elapsed += dt; not looping (+0x25) and duration"
            + " (+0x28) <= elapsed -> Stop; looping with duration != 0 -> elapsed = fmod(elapsed, duration)", false),
        new Slot("UOParticleEffectNif", 0x18, "Release", "removes the scene node (+0x10) from its render group and"
            + " releases it", true),
        new Slot("UOParticleEffectNif", 0x1c, "GetPosition", "node translation (+0x50..+0x58) minus the offset (+0x18)", false),
        new Slot("UOParticleEffectNif", 0x20, "SetPosition", "node translation = position + offset (+0x18)", false),
        new Slot("UOParticleEffectNif", 0x34, "ScalarDeletingDestructor", "restores the vftables, releases the node", false),
        new Slot("UOParticleEffectNif", 0x38, "Init", "(out, definition): fails when definition +0x3c == 1; loads the"
            + " .nif, scale = definition +0x2c, loop/duration from +0x38/+0x34, offset from +0x20/+0x24/+0x28 and the"
            + " camera vectors", false),
        new Slot("UOParticleEffectNif", 0x30, "SetLifetimeTicks", "(n): stores the update countdown +0x40 (Init sets"
            + " -1, unlimited); Update decrements it while active and IsFinished is true at 0; set from the packet"
            + " duration by UOEffects::PlayEffectAtTile", false),
        new Slot("UOParticleEffectChaos", 0x0, "Start", "fade (+0x2c) and elapsed (+0x20) = 0; starts the Chaos"
            + " system with (duration +0x28, !loop +0x26); active (+0x24) set, stopping (+0x25) cleared", false),
        new Slot("UOParticleEffectChaos", 0x4, "Stop", "clears active (+0x24), sets stopping (+0x25), stops the"
            + " system's emission", false),
        new Slot("UOParticleEffectChaos", 0x10, "IsFinished", "true without a node, when the countdown (+0x3c) is 0"
            + " or the fade timer (+0x2c) reached 5.0 s; false while stopping; while active, true once no particle"
            + " system of the node is alive", true),
        new Slot("UOParticleEffectChaos", 0x14, "Update", "(out, dt): elapsed (+0x20) += dt; countdown (+0x3c)"
            + " decremented when > 0; after Stop (+0x25) the fade timer +0x2c advances; updates the node while the"
            + " fade timer < 5.0", false),
        new Slot("UOParticleEffectChaos", 0x18, "Release", "Stop, then releases the node (+0x1c)", true),
        new Slot("UOParticleEffectChaos", 0x30, "SetLifetimeTicks", "(n): stores the update countdown +0x3c (Init"
            + " sets -1, unlimited); IsFinished is true at 0", false),
        new Slot("UOParticleEffectChaos", 0x1c, "GetPosition", "node translation minus the offset (+0x10)", false),
        new Slot("UOParticleEffectChaos", 0x20, "SetPosition", "node translation = position + offset (+0x10)", false),
        new Slot("UOParticleEffectChaos", 0x34, "ScalarDeletingDestructor", "restores the vftables, releases the node", false),
        new Slot("UOParticleEffectChaos", 0x38, "Init", "(out, definition): requires definition +0x3c == 0; loads the"
            + " .ems, scale, loop/duration and offset as for the Nif effect", false),
        new Slot("UOHuedParticleEffectNif", 0x34, "ScalarDeletingDestructor", "destroys the hued effect", false),
        new Slot("UOHuedParticleEffectNif", 0x38, "Init", "(out, definition): loads the .nif into a private scene"
            + " (+0x44) with the camera-facing node rotation, scale, loop and duration of the Nif effect; creates the"
            + " offscreen orthographic camera (+0x74) and the hue quad drawn in the world", false),
        new Slot("UOHuedParticleEffectNif", 0x14, "Update", "(out, dt): elapsed (+0x48) += dt * 0.5 (hued effects"
            + " run at half speed); countdown, loop and duration as the Nif Update; then updates the private scene and"
            + " renders it into the 512x512 texture of the quad", false),
        new Slot("UOSoundEffect", 0x0, "Start", "plays sound id (+4) at the position (+8..+0x10) when sound is"
            + " enabled; stores the handle at +0x18", false),
        new Slot("UOSoundEffect", 0x4, "Stop", "stops and clears the sound handle (+0x18)", false),
        new Slot("UOSoundEffect", 0x10, "IsFinished", "no handle, or the handle reports not playing", false),
        new Slot("UOSoundEffect", 0x1c, "GetPosition", "copies +8..+0x10", false),
        new Slot("UOSoundEffect", 0x20, "SetPosition", "stores +8..+0x10 and moves a playing sound", false),
        new Slot("UOSoundEffect", 0x34, "ScalarDeletingDestructor", "restores the vftable", false),
        new Slot("UOSoundEffect", 0x38, "Init", "(out, definition): sound id = definition +4, +0x14 = definition +8", false),
        new Slot("UOPositionOffset", 0x1c, "GetPosition", "copies +0x44..+0x4c", false),
        new Slot("UOPositionOffset", 0x20, "SetPosition", "stores +0x44..+0x4c", false),
        new Slot("UOPositionOffset", 0x34, "ScalarDeletingDestructor", "restores the vftable", false),
        new Slot("UOPositionOffset", 0x38, "Init", "(out, definition): copies the 8 x and 8 y offsets", false),
        new Slot("UOEarthquakeEffect", 0x0, "Start", "elapsed (+8) = 0, active (+4) set", false),
        new Slot("UOEarthquakeEffect", 0x4, "Stop", "clears active (+4)", false),
        new Slot("UOEarthquakeEffect", 0x10, "IsFinished", "inactive or elapsed (+8) > duration (+0x18)", false),
        new Slot("UOEarthquakeEffect", 0x14, "Update", "(out, dt): elapsed += dt; camera target x += amplitude"
            + " (+0x10) * sin(elapsed * frequency (+0x20) * 2pi)", false),
        new Slot("UOEarthquakeEffect", 0x34, "Init", "(out, definition): copies amplitude, duration, frequency"
            + " (definition +8/+0x10/+0x18) to +0x10/+0x18/+0x20, elapsed = 0, active", false),
        new Slot(CONTAINER, 0x0, "Start", "starts every child", false),
        new Slot(CONTAINER, 0x4, "Stop", "stops every child", false),
        new Slot(CONTAINER, 0x8, "SetVisible", "stores the flag at +0x1c and forwards it to every child", true),
        new Slot(CONTAINER, 0x10, "IsFinished", "child count (+0x18) == 0", false),
        new Slot(CONTAINER, 0x14, "Update", "(out, dt): updates every child, then releases and erases the finished"
            + " ones", false),
        new Slot(CONTAINER, 0x18, "Release", "releases every child", false),
        new Slot(CONTAINER, 0x20, "SetPosition", "forwards the position to every child", false),
        new Slot(CONTAINER, 0x2c, "GetKind", "returns 6", true),
        new Slot(CONTAINER, 0x34, "ScalarDeletingDestructor", "destroys the child list", true),
        new Slot(CONTAINER, 0x38, "Init", "creates the child list", true),
        new Slot("UOEffectContainerDefinition", 0x8, "Instantiate", "(out ok, out effect, object): creates a"
            + " UOEffectContainer (0x20 bytes) and initialises it; UOEffects::InstantiateEffectById then adds one"
            + " child per collection definition", false),
        new Slot("UOEffectCollectionDefinition", 0x8, "Instantiate", "(out ok, out effect, object): registers a"
            + " container and instantiates every definition of the vector into it", false),
        new Slot("UOProjectileEffect", 0x14, "Update", "(out, dt): moves the projectile; at the target spawns the"
            + " explosion (UOProjectileEffect::SpawnExplosion) and, with the return flag (+0x2d), a projectile back", true),
        new Slot("UOProjectileEffect", 0x38, "Init", "(out, definition): creates the moving item object and"
            + " instantiates the tileart effects of its graphic without the attach_mode test", true),
        new Slot("UOTileEffect", 0x10, "IsFinished", "true when the countdown (+0xc) is <= 0 and no attached effect"
            + " remains", true),
        new Slot("UOTileEffect", 0x14, "Update", "(out, dt): countdown (+0xc) -= 1 per update (packet duration in"
            + " ticks)", false),
        new Slot("UOTileEffect", 0x38, "Init", "(out, definition): creates a world object of the item id; when its"
            + " tileart has effects, instantiates and registers each one (no attach_mode test), attaches and starts"
            + " it; otherwise shows the animated art", false),
        new Slot(COMPONENT, 0x14, "HandleNotify", "(message): 1 = moved: drops finished effects, moves the others to"
            + " the object position; 0: forwards the object vector (vtable +0x28) to effect +0x24", true),
        new Slot(COLLECTION, 0x0, "GetMemorySize", "0x14 + 8 per definition", false),
        new Slot(COLLECTION, 0x4, "ScalarDeletingDestructor", "destroys the definition vector, IAsset vftable", false),
        new Slot(COLLECTION, 0xc, "GetCount", "size of the shared_ptr vector at +8", false),
        new Slot(COLLECTION, 0x10, "GetDefinition", "shared_ptr copy of definition i", false),
    };

    // UOObject accessors the effect code calls through the vtable.
    private static final Slot[] OBJECT_SLOTS = {
        new Slot(WORLD_OBJECT, 0x54, "SetHue", "stores +0x44; when it changed, notifies the components (vtable +0x1c,"
            + " message 3)", false),
        new Slot(WORLD_OBJECT, 0x58, "GetHue", "returns +0x44; the hue UOParticleEffectDefinition::Instantiate tests"
            + " (non-zero and hued -> UOHuedParticleEffectNif)", false),
        new Slot(WORLD_OBJECT, 0x138, "SetRenderMode", "stores +0x8c (graphical effect render mode)", false),
        new Slot(WORLD_OBJECT, 0x13c, "GetRenderMode", "returns +0x8c", false),
        new Slot(WORLD_OBJECT, 0x140, "SetSuppressTileartEffects", "stores byte +0x90; the projectile and tile effect"
            + " Init set it from the effect-9502 flag of their definition", false),
        new Slot(WORLD_OBJECT, 0x144, "GetSuppressTileartEffects", "returns byte +0x90; when set, the projectile and"
            + " tile effect Init skip the tileart effects of the item", false),
    };

    // ---- Chaos particle engine (.ems scripts; results/uosa_chaos.md) ----
    private static final String CHAOS_EMITTER = "Emitter";
    private static final String CHAOS_SEQUENCE = "ParticleEventSequence";
    private static final String CHAOS_EMITTER_EFFECT = "EmitterEffect";
    private static final String CHAOS_PARTICLE_SET = "ParticleSet";
    private static final String CHAOS_TRISHAPE = "ParticleTriShape";
    private static final String CHAOS_BILLBOARD = "ParticleBillboard";
    private static final String CHAOS_MESH_PARTICLE = "MeshParticle";
    private static final String CHAOS_EMITTER_PARTICLE = "EmitterParticle";
    private static final String CHAOS_MANAGER = "ParticleSystemManager";
    private static final String CHAOS_SCRIPT = "ChaosScript";
    private static final String MERSENNE_TWISTER = "MersenneTwister";
    private static final String CHAOS_EFFECT = "UOParticleEffectChaos";
    // Emitter vtable: +0xc Stop(pool, killStayAtEmitter, killAll), +0x14 Update(time, dt).
    private static final int EMITTER_STOP = 0xc;
    private static final int EMITTER_UPDATE = 0x14;
    private static final int EMITTER_IS_ACTIVE = 0x1c;
    private static final int EMITTER_COMPILE_SCRIPT = 0x20;
    // ParticleEvent vtable: +4 Apply(particle), +0xc Parse(tokens).
    private static final int EVENT_APPLY = 0x4;
    private static final int EVENT_PARSE = 0xc;
    private static final int TRISHAPE_BUILD_VERTICES = 0xa8;
    private static final int CHAOS_EFFECT_INIT = 0x38;
    // MT19937 seeding constants (init_genrand multiplier, init_by_array second multiplier); the CelSetup float
    // to integer conversion sets the x87 rounding control to truncation.
    private static final long MT_INIT_MULTIPLIER = 0x6c078965L;
    private static final long MT_ARRAY_MULTIPLIER = 0x5d588b65L;
    private static final long X87_TRUNCATE_CONTROL = 0xc00L;
    private static final String PARSE_FLOAT = "_atof";
    private static final String PARSE_INT = "_atol";
    private static final String IS_SPACE = "_isspace";
    private static final String TIME64 = "__time64";

    // {event class, keyword, what Apply writes in the particle}
    private static final String[][] CHAOS_EVENTS = {
        {"ParticleEventScale", "SCALE", "size +0x8 / step +0xc"},
        {"ParticleEventColor", "COLOR", "RGBA +0x28..+0x34 / steps +0x38..+0x44"},
        {"ParticleEventRedColor", "REDCOLOR", "red +0x28 / step +0x38"},
        {"ParticleEventGreenColor", "GREENCOLOR", "green +0x2c / step +0x3c"},
        {"ParticleEventBlueColor", "BLUECOLOR", "blue +0x30 / step +0x40"},
        {"ParticleEventAlpha", "ALPHA", "alpha +0x34 / step +0x44"},
        {"ParticleEventVelocity", "VELOCITY", "velocity +0x48..+0x50 / steps +0x54..+0x5c"},
        {"ParticleEventVelocityX", "VELOCITYX", "velocity x +0x48 / step +0x54"},
        {"ParticleEventVelocityY", "VELOCITYY", "velocity y +0x4c / step +0x58"},
        {"ParticleEventVelocityZ", "VELOCITYZ", "velocity z +0x50 / step +0x5c"},
        {"ParticleEventThrust", "THRUST", "thrust +0x60 / step +0x64 (step reset without a later fade)"},
        {"ParticleEventEventTimer", "EVENTTIMER", "event timer +0x18 (no fade)"},
        {"ParticleEventRotation", "ROTATION", "rotation +0x80..+0x88 radians / steps +0x8c..+0x94 (step reset"
            + " without a later fade)"},
        {"ParticleEventLength", "LENGTH", "length +0x98 / step +0x9c (step reset without a later fade)"},
    };

    private static final String FADE_RULE = "non-fade: value = sample(range); a later fade event of the same type:"
        + " step = (sample(fade range) - value) / (fade time - this time, 0 -> 1)";

    private static final Slot[] CHAOS_SLOTS = {
        new Slot(CHAOS_EMITTER, 0x0, "ScalarDeletingDestructor", "frees the sequences and deques", true),
        new Slot(CHAOS_EMITTER, 0x4, "Start", "(startTime, duration, notLooping): MinEmitTime = sample(+0x6c/+0x70);"
            + " notLooping and MinEmitTime > 0 -> continuous for MinEmitTime, else one-shot = notLooping; remaining"
            + " (+0x64) = duration, infinite (+0x68) = duration <= 0, first burst (+0x69), skip-first-update (+0x75)",
            false),
        new Slot(CHAOS_EMITTER, 0x8, "ClearStarted", "started (+0x77) = 0, owner (+8) = null", false),
        new Slot(CHAOS_EMITTER, EMITTER_STOP, "Stop", "(pool, killStayAtEmitter, killAll): ClearStarted; releases all"
            + " particles, or only the StayAtEmitter sequences' particles", false),
        new Slot(CHAOS_EMITTER, 0x10, "Clone", "copies the compiled sequences into a new emitter with its own"
            + " renderers", true),
        new Slot(CHAOS_EMITTER, EMITTER_UPDATE, "Update", "(time, dt): owner world position/velocity/rotation and"
            + " local scale; first burst, continuous emission at 1/(emitrate max + boost) (>= 1/60 s) with 1/30 s"
            + " sub-steps, Stop when the emission time runs out; then sequence updates and render buffer copies",
            false),
        new Slot(CHAOS_EMITTER, 0x18, "Reset", "resets every sequence", true),
        new Slot(CHAOS_EMITTER, EMITTER_IS_ACTIVE, "IsActive", "returns started (+0x77)", false),
        new Slot(CHAOS_EMITTER, EMITTER_COMPILE_SCRIPT, "CompileScript", "tokenizes the .ems text, then"
            + " 'ParticleSystem name version {' and the system block; status '0 errors, started emitter.'", false),
        new Slot(CHAOS_EMITTER, 0x24, "CopyTo", "copies the emitter settings", true),
        new Slot(CHAOS_EMITTER_EFFECT, 0xa8, "Start", "(duration, notLooping): finds or clones an inactive emitter"
            + " of the group, Emitter::Start(0, duration, notLooping), registers it with the manager", false),
        new Slot(CHAOS_EMITTER_EFFECT, 0xac, "Stop", "Emitter::Stop(pool, 1, 0), emitter index (+0xe0) = -1", false),
        new Slot(CHAOS_EMITTER_EFFECT, 0xb4, "GetMinEmitTime", "first emitter's MinEmitTime max, else 1.0", true),
        new Slot(CHAOS_EMITTER_EFFECT, 0xb8, "GetVelocity", "(dt): (world translate - previous) / dt when dt > 0",
            false),
        new Slot(CHAOS_EMITTER_EFFECT, 0xbc, "GetEmitterGroup", "returns the group (+0xf0) with a reference", true),
        new Slot(CHAOS_PARTICLE_SET, 0x0, "Update", "(time): updates the per-sequence renderers; bounds every"
            + " 0.033 s", true),
        new Slot(CHAOS_PARTICLE_SET, 0x4, "SetCount", "(sequence, count): live particle count of a renderer", true),
        new Slot(CHAOS_PARTICLE_SET, 0x8, "AddSequence", "adds a per-sequence render buffer (stride 0x74)", true),
        new Slot(CHAOS_TRISHAPE, 0x88, "Render", "draws the active quads", true),
        new Slot(CHAOS_TRISHAPE, 0xa0, "InitBuffers", "(numparticles): 4 vertices per particle, template corners"
            + " (-0.5,0,-0.5)..(0.5,0,-0.5), UVs (0,1),(0,0),(1,0),(1,1), triangles (n,n+2,n+1),(n,n+2,n+3)", false),
        new Slot(CHAOS_TRISHAPE, 0xa4, "SetActiveCount", "number of quads drawn", true),
        new Slot(CHAOS_TRISHAPE, TRISHAPE_BUILD_VERTICES, "BuildVertices", "camera-facing quads: AXIAL from the"
            + " particle along its velocity by length, POINT camera right/up of size/2, rotated about the view"
            + " axis by -rotation.y when ROTATION is used; colours clamped to [0,1]; CelSetup UVs", false),
        new Slot(CHAOS_BILLBOARD, 0x10, "Init", "creates the node and its ParticleTriShape", true),
        new Slot(CHAOS_BILLBOARD, 0x14, "SetBound", "(cx, cy, cz, radius): model bound of the shape and of its"
            + " geometry data; ParticleSet::Update sets it every 0.033 s to the AABB centre of the particle positions"
            + " and the largest distance + 0.01, so each sequence sorts as one object by that centre", false),
        new Slot(CHAOS_MESH_PARTICLE, 0x4, "SetActiveCount", "(n): clears APP_CULLED (NiAVObject +0xac bit 0) on the"
            + " first n clones and sets it on the rest", false),
    };

    private static final class Block {
        long start;
        byte[] bytes;
        boolean exec;
    }

    private static final class Proposal {
        long address;
        String path;
        String evidence;
        boolean probable;
        String status = "proposed";
    }

    private Memory memory;
    private Listing listing;
    private FunctionManager functions;
    private SymbolTable symbols;
    private DataTypeManager dtm;
    private final List<Block> blocks = new ArrayList<>();
    // Aligned pointers into executable code found in non-executable blocks (vtables, tables).
    private final Map<Long, Integer> codePointerCounts = new HashMap<>();
    private final Map<String, Long> vtables = new LinkedHashMap<>();
    // g_UOEffectRenderGroups, found by effectRendering (-1 when not found).
    private long renderGroups = -1;
    private final Map<Long, Proposal> proposals = new TreeMap<>();
    private final List<String> anomalies = new ArrayList<>();
    private final List<String> kept = new ArrayList<>();
    private final List<String> shared = new ArrayList<>();
    private final List<String> labels = new ArrayList<>();
    private final List<String> typeLines = new ArrayList<>();
    private final List<String> anchors = new ArrayList<>();
    private final List<String> factoryRows = new ArrayList<>();
    private final List<String> otherUsers = new ArrayList<>();
    private final Map<String, Integer> changes = new LinkedHashMap<>();

    public void run() throws Exception {
        String[] args = getScriptArgs();
        if (args.length < 1) {
            throw new IllegalArgumentException("Expected OUTPUT_FILE [" + APPLY_FLAG + "]");
        }
        boolean apply = false;
        for (int i = 1; i < args.length; i++) {
            if (args[i].equals(APPLY_FLAG)) {
                apply = true;
            } else {
                throw new IllegalArgumentException("Unknown argument " + args[i]);
            }
        }
        // Created before any change, so an existing output file stops the run unchanged.
        try (PrintWriter output = new PrintWriter(Files.newBufferedWriter(Path.of(args[0]),
                StandardOpenOption.CREATE_NEW))) {
            analyze(output, apply);
        }
        println("Wrote " + args[0] + (apply ? " (applied: " + changes + ")" : ""));
    }

    private void analyze(PrintWriter out, boolean apply) throws Exception {
        memory = currentProgram.getMemory();
        listing = currentProgram.getListing();
        functions = currentProgram.getFunctionManager();
        symbols = currentProgram.getSymbolTable();
        dtm = currentProgram.getDataTypeManager();
        loadBlocks();

        Set<String> classes = new LinkedHashSet<>();
        for (EffectType t : EFFECT_TYPES) {
            classes.add(t.factory());
            classes.add(t.baseFactory());
            classes.add(t.definition());
        }
        for (Slot s : RUNTIME_SLOTS) {
            classes.add(s.cls());
        }
        classes.add(COLLECTION_FACTORY);
        for (String cls : classes) {
            long vt = rttiVtable(cls);
            vtables.put(cls, vt);
            anchors.add("RTTI `" + cls + "` vftable " + hexOrNone(vt));
        }

        // 1. Factories and definition classes, keyed by the constant compared in CanHandle.
        Map<Integer, Function> creates = new HashMap<>();
        for (EffectType t : EFFECT_TYPES) {
            long vt = vtables.get(t.factory());
            long defVt = vtables.get(t.definition());
            Function canHandle = vt < 0 ? null : slotFunction(vt, FACTORY_CAN_HANDLE);
            Function getType = defVt < 0 ? null : slotFunction(defVt, DEFINITION_GET_TYPE);
            long handled = canHandle == null ? UNKNOWN_KEY : comparedKey(canHandle);
            long typed = getType == null ? UNKNOWN_KEY : returnedConstant(getType);
            if (handled != t.key() || typed != t.key()) {
                anomalies.add(t.factory() + ": CanHandle key " + handled + ", " + t.definition() + " GetType " + typed
                    + ", expected " + t.key());
            }
            Function create = vt < 0 ? null : slotFunction(vt, FACTORY_CREATE);
            if (create != null) {
                creates.put(t.key(), create);
            }
            factoryRows.add("| " + t.key() + " | `" + t.factory() + "` | " + hexOrNone(vt) + " | " + entry(canHandle)
                + " | " + entry(create) + " | `" + t.definition() + "` | " + hexOrNone(defVt) + " | " + handled + "/"
                + typed + " |");
            List<Slot> factorySlots = List.of(
                new Slot(t.factory(), FACTORY_CAN_HANDLE, "CanHandle", "returns key == " + t.key(), false),
                new Slot(t.factory(), FACTORY_CREATE, "Create", "reads " + t.create(), false),
                new Slot(t.factory(), FACTORY_DESTRUCTOR, "ScalarDeletingDestructor", "restores the vftable, frees"
                    + " when flag&1", true),
                new Slot(t.factory(), FACTORY_NEW_INSTANCE, "NewInstance", "allocates a " + t.definition()
                    + " into a shared_ptr", false),
                new Slot(t.factory(), FACTORY_INIT, "Init", t.init(), false));
            Map<Integer, Function> named = nameSlots(t.factory(), t.baseFactory(), factorySlots);
            nameSlots(t.definition(), null, List.of(
                new Slot(t.definition(), DEFINITION_DESTRUCTOR, "ScalarDeletingDestructor", "frees the children and"
                    + " fields, frees when flag&1", true),
                new Slot(t.definition(), DEFINITION_INSTANTIATE, "Instantiate", "(out ok, out effect, object): "
                    + t.instantiate(), t.instantiateProbable()),
                new Slot(t.definition(), DEFINITION_GET_TYPE, "GetType", "returns " + t.key(), false)));
            Function newInstance = named.get(FACTORY_NEW_INSTANCE);
            List<Function> writers = vtableWriters(t.definition());
            writers.remove(newInstance);
            if (writers.size() == 1 && newInstance != null && distinct(calls(newInstance)).contains(writers.get(0))) {
                propose(writers.get(0), t.definition() + SCOPE + t.definition(), "writes the " + t.definition()
                    + " vftable; called by NewInstance", false);
            }
        }

        // 2. StreamRead (named by NameTileartCode) and ParseEffectDefinition, called right after pushing the list.
        Function streamRead = null;
        Function particleCreate = creates.get(KEY_PARTICLE);
        if (particleCreate != null) {
            for (Function callee : distinct(calls(particleCreate))) {
                if (bodySize(callee) <= SMALL_FUNCTION && callsNamed(callee, MEMCPY)) {
                    streamRead = callee;
                }
            }
        }
        anchors.add("stream read helper " + entry(streamRead));
        Function roundRobinCreate = creates.get(KEY_ROUND_ROBIN);
        long factoryList = -1;
        Function parseDefinition = null;
        if (roundRobinCreate != null) {
            long[] pushed = globalPushedBeforeCall(roundRobinCreate);
            if (pushed != null) {
                factoryList = pushed[0];
                parseDefinition = functions.getFunctionAt(toAddr(pushed[1]));
            }
        }
        if (parseDefinition != null) {
            Set<Long> offsets = virtualCallOffsets(parseDefinition);
            if (!offsets.contains((long) FACTORY_CAN_HANDLE) || !offsets.contains((long) FACTORY_CREATE)) {
                anomalies.add("ParseEffectDefinition candidate " + entry(parseDefinition) + " virtual calls " + offsets);
            }
        }
        propose(parseDefinition, NAMESPACE + SCOPE + "ParseEffectDefinition", "(factories, out sp, stream, ctx):"
            + " u32 type key; first factory whose CanHandle (+0) accepts it runs Create (+8); an unknown key marks the"
            + " parse context failed and returns a null definition without consuming a body",
            false);
        label(factoryList, FACTORY_LIST_LABEL, "std::vector of effect factory pointers (+4 first, +8 last), pushed"
            + " before every UOEffects::ParseEffectDefinition call", apply);

        // 3. Shared readers: effect list (Wait, PropertyFlag, Random, UserPreference) and effect map (LOD, PropertyFlag).
        Function parseList = null;
        if (parseDefinition != null) {
            Set<Function> candidates = null;
            for (int key : LIST_READER_KEYS) {
                Set<Function> callingParse = new LinkedHashSet<>();
                if (creates.containsKey(key)) {
                    for (Function callee : distinct(calls(creates.get(key)))) {
                        if (distinct(calls(callee)).contains(parseDefinition)) {
                            callingParse.add(callee);
                        }
                    }
                }
                if (candidates == null) {
                    candidates = callingParse;
                } else {
                    candidates.retainAll(callingParse);
                }
            }
            parseList = single(new ArrayList<>(candidates), "effect list reader");
        }
        propose(parseList, NAMESPACE + SCOPE + "ParseEffectList", "u32 count, count x ParseEffectDefinition appended to"
            + " a shared_ptr vector; common callee of the Wait, PropertyFlag, Random and UserPreference Create", false);
        Function parseMap = null;
        Function lodCreate = creates.get(KEY_LOD);
        if (lodCreate != null && parseList != null) {
            List<Function> found = new ArrayList<>();
            for (Function callee : distinct(calls(lodCreate))) {
                if (distinct(calls(callee)).contains(parseList)) {
                    found.add(callee);
                }
            }
            parseMap = single(found, "effect map reader");
        }
        propose(parseMap, NAMESPACE + SCOPE + "ParseEffectMap", "u32 count, count x {u32 key, ParseEffectList} inserted"
            + " into a std::map<unsigned, vector>; callee of the LOD Create", false);

        // 4. EffectDefinitionCollection.uop asset: factory Create -> collection parser -> per definition Add.
        long collectionFactoryVt = vtables.get(COLLECTION_FACTORY);
        Function collectionCreate = collectionFactoryVt < 0 ? null
            : slotFunction(collectionFactoryVt, COLLECTION_FACTORY_CREATE);
        propose(collectionCreate, COLLECTION_FACTORY + SCOPE + "Create", "asset factory vtable +4: parses the"
            + " EffectDefinitionCollection payload, tries the asset resolvers on failure", false);
        Function parseCollection = null;
        if (collectionCreate != null && parseDefinition != null) {
            List<Function> found = new ArrayList<>();
            for (Function callee : distinct(calls(collectionCreate))) {
                if (distinct(calls(callee)).contains(parseDefinition)) {
                    found.add(callee);
                }
            }
            parseCollection = single(found, "collection parser");
        }
        propose(parseCollection, NAMESPACE + SCOPE + "ParseEffectDefinitionCollection", "u32 count, count x"
            + " ParseEffectDefinition into a new BinaryUOEffectDefinitionCollection (0x14 bytes)", false);
        if (parseCollection != null) {
            Function add = singleCallerCallAfter(parseCollection, parseDefinition);
            propose(add, COLLECTION + SCOPE + "AddDefinition", "first single-caller callee after"
                + " ParseEffectDefinition in the collection parser; push_back into the vector at +4", true);
            for (Function caller : callers(parseCollection)) {
                if (!caller.equals(collectionCreate)) {
                    otherUsers.add(entry(caller) + " also calls UOEffects::ParseEffectDefinitionCollection");
                }
            }
        }
        if (parseDefinition != null) {
            Set<Function> known = new LinkedHashSet<>();
            known.add(parseList);
            known.add(parseCollection);
            known.add(roundRobinCreate);
            for (Function caller : callers(parseDefinition)) {
                if (!known.contains(caller)) {
                    otherUsers.add(entry(caller) + " calls UOEffects::ParseEffectDefinition");
                }
            }
        }

        // 5. Registration: the one function writing every derived factory vftable (StartUp state; not renamed).
        Set<Function> registrars = null;
        for (EffectType t : EFFECT_TYPES) {
            Set<Function> writers = new LinkedHashSet<>(vtableWriters(t.factory()));
            if (registrars == null) {
                registrars = writers;
            } else {
                registrars.retainAll(writers);
            }
        }
        Function registrar = single(new ArrayList<>(registrars), "factory registration");
        if (registrar != null) {
            List<String> order = new ArrayList<>();
            for (Instruction i : listing.getInstructions(registrar.getBody(), true)) {
                for (EffectType t : EFFECT_TYPES) {
                    long vt = vtables.get(t.factory());
                    if (referencesAddress(i, vt) && !order.contains(t.factory())) {
                        order.add(t.factory());
                    }
                }
            }
            anchors.add("factory registration in " + entry(registrar) + " (not renamed), order: "
                + String.join(", ", order));
        }

        // 6. Runtime effects and the collection class.
        for (Slot slot : RUNTIME_SLOTS) {
            nameSlots(slot.cls(), null, List.of(slot));
        }

        // 7. Runtime use: effect id lookup, the active effect list and the graphical effect packets.
        runtimeUse(apply);
        huedEffects(apply);
        effectRendering(apply);

        // 8. Chaos particle engine.
        chaosEngine();
        drawOrderAndMaterials(apply);
        textureFiltering(apply);

        defineTypes(apply);

        // 9. Apply names.
        if (apply) {
            ensureTag();
            for (Proposal proposal : proposals.values()) {
                applyName(proposal);
            }
        } else {
            for (Proposal proposal : proposals.values()) {
                Function f = functions.getFunctionAt(toAddr(proposal.address));
                proposal.status = f.getName(true).equals(proposal.path)
                    ? (hasStaleOwnComment(f, proposal) ? "already named, plate comment would be refreshed" : "already named")
                    : f.getSymbol().getSource() == SourceType.DEFAULT ? "would rename"
                    : "kept " + f.getName(true) + " (" + f.getSymbol().getSource() + ")";
                if (proposal.status.startsWith("kept")) {
                    kept.add(hex(proposal.address) + " `" + f.getName(true) + "` (" + f.getSymbol().getSource()
                        + "), proposed `" + proposal.path + "`");
                }
            }
        }
        report(out, apply);
    }

    private void runtimeUse(boolean apply) throws Exception {
        for (String cls : List.of(PROJECTILE_DEFINITION, TILE_DEFINITION)) {
            vtables.put(cls, rttiVtable(cls));
            anchors.add("RTTI `" + cls + "` vftable " + hexOrNone(vtables.get(cls)));
        }
        Function byId = single(stringUsers(MISSING_EFFECT_STRING), "effect id lookup (user of the missing-id string)");
        propose(byId, NAMESPACE + SCOPE + "InstantiateEffectById", "(out ok, u32 id, out effect, object), cdecl:"
            + " requests asset {id, 0x0d000000} (EffectDefinitionCollection.uop build/effectdefinitioncollection/%08d.bin);"
            + " missing -> logs 'Missing Effect Definition %d' and retries id 0, then fails; one definition is"
            + " instantiated directly, several go into one UOEffectContainer (they all play at once); the result is"
            + " registered in g_UOActiveEffects", false);
        if (byId == null) {
            return;
        }

        // Registration: the callee of the lookup whose only callee makes the Instantiate virtual call.
        List<Function> wrappers = new ArrayList<>();
        for (Function callee : distinct(calls(byId))) {
            List<Function> inner = distinct(calls(callee));
            if (inner.size() == 1 && virtualCallOffsets(inner.get(0)).contains((long) EFFECT_INSTANTIATE)) {
                wrappers.add(callee);
            }
        }
        Function wrapper = single(wrappers, "instantiate-and-register wrapper");
        Function register = wrapper == null ? null : distinct(calls(wrapper)).get(0);
        propose(wrapper, NAMESPACE + SCOPE + "InstantiateAndRegisterByValue", "(EAX out, EDX object; ok, definition"
            + " shared_ptr by value): UOEffects::InstantiateAndRegister, then releases the definition reference", false);
        propose(register, NAMESPACE + SCOPE + "InstantiateAndRegister", "(ECX definition, ESI ok, EDI out, EDX"
            + " object): definition Instantiate (vtable +8); on success pushes the effect into g_UOActiveEffects", false);
        if (register != null) {
            for (long list : addressTakenGlobals(register)) {
                List<Function> users = referencingFunctions(list);
                users.remove(register);
                Function update = null;
                for (Function user : users) {
                    if (virtualCallOffsets(user).contains((long) EFFECT_UPDATE)) {
                        update = user;
                    }
                }
                if (users.size() != 2 || update == null) {
                    continue;
                }
                users.remove(update);
                label(list, ACTIVE_EFFECTS_LABEL, "std::list of every running IUOEffect shared_ptr; filled by"
                    + " UOEffects::InstantiateAndRegister, updated each frame by UOEffects::UpdateActiveEffects", apply);
                propose(update, NAMESPACE + SCOPE + "UpdateActiveEffects", "(float dt): Update (+0x14) of every active"
                    + " effect; IsFinished (+0x10) -> Release (+0x18) and erase; then updates the weather subsystem with"
                    + " dt * 1000", false);
                propose(users.get(0), NAMESPACE + SCOPE + "ReleaseActiveEffects", "releases every active effect and"
                    + " clears g_UOActiveEffects", false);
            }
        }

        // Callers of the lookup, told apart by what they do.
        long projectileVt = vtables.getOrDefault(PROJECTILE_EFFECT, -1L);
        Function projectileUpdate = projectileVt < 0 ? null : slotFunction(projectileVt, EFFECT_UPDATE);
        Function playAtTile = null;
        List<Function> others = new ArrayList<>();
        for (Function caller : callers(byId)) {
            if (usesScalar(caller, TARGET_INDICATOR_EFFECT_ID)) {
                propose(caller, NAMESPACE + SCOPE + "ShowTargetIndicator", "(serial): instantiates collection effect"
                    + " 10001 into a global, attaches it to the new target mobile and stops the previous one; called"
                    + " when the current target changes to target type 2 while an option byte is 0", false);
            } else if (projectileUpdate != null && distinct(calls(projectileUpdate)).contains(caller)) {
                propose(caller, PROJECTILE_EFFECT + SCOPE + "SpawnExplosion", "at the target: explode effect (+0x2e) =="
                    + " 0 -> classic tile effect 0x36CB for 10 ticks, else InstantiateEffectById(explode effect - 1)"
                    + " attached to the target; plays the explode sound (+0x30)", false);
            } else if (virtualCallOffsets(caller).contains((long) EFFECT_SET_LIFETIME)) {
                playAtTile = caller;
            } else {
                others.add(caller);
            }
        }
        if (others.size() == 1) {
            propose(others.get(0), "UOWaypoints" + SCOPE + "SetWaypointEffect", "(show): removes the waypoint effect,"
                + " or instantiates the effect id the waypoint type (+0x12) selects and attaches it to the waypoint"
                + " object or a placeholder object at its location", true);
        } else {
            anomalies.add("other InstantiateEffectById callers: " + names(others));
        }
        propose(playAtTile, NAMESPACE + SCOPE + "PlayEffectAtTile", "(EAX id, EDI tile x/y/z float3, ESI out; ticks,"
            + " ok): InstantiateEffectById(id, object 0), SetPosition (+0x20) at the tile, ticks != 0 ->"
            + " SetLifetimeTicks (+0x30), Start (+0)", false);
        if (playAtTile == null) {
            return;
        }

        // The graphical effect dispatcher and the per-packet field copiers, handlers and readers.
        Function dispatcher = single(callers(playAtTile), "graphical effect dispatcher");
        propose(dispatcher, NAMESPACE + SCOPE + "PlayGraphicalEffect", "(out ok, 0x38-byte effect record): type 0"
            + " moving -> UOEffects::MakeProjectileEffect; 1 lightning -> PlayEffectAtTile(666); 2 fixed location /"
            + " 3 fixed on mobile -> particle filter, then effect 9501 = nothing, 0 or 9502 = classic tile effect"
            + " of the item id, else PlayEffectAtTile(effect - 1) (type 3 attaches it to the mobile); 4 -> screen"
            + " fade", false);
        if (dispatcher == null) {
            return;
        }
        Set<Long> dispatcherReads = readGlobals(dispatcher);
        for (Function callee : distinct(calls(dispatcher))) {
            Instruction first = listing.getInstructionAt(callee.getEntryPoint());
            Set<Long> firstReads = first == null ? Set.of() : readTargets(first);
            if (firstReads.size() == 1 && dispatcherReads.containsAll(firstReads)) {
                propose(callee, NAMESPACE + SCOPE + "IsParticleFiltered", "(serial, serial high, location effect):"
                    + " particle filter 0 None -> filtered; 1 Player/Target -> shown for the player, the current"
                    + " target or location effects; 2 Party/Target adds party members; 3 All -> shown; 4 Hide AoE ->"
                    + " filtered for location effects", false);
                label(firstReads.iterator().next(), FILTER_LABEL, "particle filter setting (settings file"
                    + " Particles.filter, Lua SystemData.Settings.Resolution.particleFilter): 0 None, 1 Player/Target,"
                    + " 2 Party/Target, 3 All (default), 4 Hide AoE", apply);
            }
        }
        propose(writerAmong(dispatcher, PROJECTILE_DEFINITION), NAMESPACE + SCOPE + "MakeProjectileEffect",
            "(record): UOProjectileEffectDefinition from the moving effect fields (item id, hue + 1, speed, serials,"
            + " explode flags, explode effect and sound, render mode; effect 9502 flag); particle filter; then"
            + " InstantiateAndRegister", false);
        propose(writerAmong(dispatcher, TILE_DEFINITION), NAMESPACE + SCOPE + "MakeTileEffect", "(record):"
            + " UOTileEffectDefinition (item id, hue + 1, position, duration ticks, on-mobile and effect-9502 flags);"
            + " then InstantiateAndRegister", false);
        List<Function> copiers = callers(dispatcher);
        if (copiers.size() != PACKET_COPIER_COUNT) {
            anomalies.add("graphical effect copiers: " + names(copiers));
            return;
        }
        copiers.sort((a, b) -> Long.compare(maxFieldOffset(a), maxFieldOffset(b)));
        for (int i = 0; i < PACKET_COPIER_COUNT; i++) {
            Function copier = copiers.get(i);
            String id = EFFECT_PACKETS[i][0];
            String name = EFFECT_PACKETS[i][1];
            propose(copier, NAMESPACE + SCOPE + "PlayGraphicalEffect_" + id, "(ESI parsed packet, EDI out): copies the "
                + id + " " + EFFECT_PACKETS[i][2] + " into the 0x38-byte record (largest field +"
                + hexShort(maxFieldOffset(copier)) + ") and calls UOEffects::PlayGraphicalEffect", false);
            Function handler = single(callers(copier), id + " handler");
            propose(handler, "UOObjectManager" + SCOPE + "Receive_" + id + "_" + name, "UOObjectManager packet callback"
                + " for " + id + ": parses the packet, then UOEffects::PlayGraphicalEffect_" + id, false);
            if (handler != null) {
                propose(callBefore(handler, copier), "UOPacket" + SCOPE + "Read_" + id + "_" + name, "parses " + id
                    + " (the call before the copier in its handler)", false);
            }
        }

        // Settings read by the composite definitions.
        labelSingleRead(definitionInstantiate("UOEffectLODDefinition"), LOD_LABEL, "particle LOD level (settings file"
            + " Particles.lod, 0..3, default 3; Lua SystemData.Settings.Resolution.particleLOD 1 Low .. 3 High); the"
            + " key UOEffectLODDefinition::Instantiate looks up", apply);
        labelSingleRead(definitionInstantiate("UOUserPrefEffectFilterDefinition"), USER_FILTER_LABEL, "user effect"
            + " filter bits (settings file Effects.filter, hex); bit 0 is set when footsteps are disabled; read by"
            + " UOUserPrefEffectFilterDefinition::Instantiate", apply);
    }

    // Chaos particle engine: RTTI vtables, the unique parser strings and the call graph below Emitter::Update.
    private void chaosEngine() {
        List<String> classes = new ArrayList<>(List.of(CHAOS_EMITTER, CHAOS_SEQUENCE, CHAOS_EMITTER_EFFECT,
            CHAOS_PARTICLE_SET, CHAOS_TRISHAPE, CHAOS_BILLBOARD, CHAOS_MESH_PARTICLE, CHAOS_EMITTER_PARTICLE,
            CHAOS_MANAGER));
        for (String[] event : CHAOS_EVENTS) {
            classes.add(event[0]);
        }
        for (String cls : classes) {
            if (!vtables.containsKey(cls)) {
                long vt = rttiVtable(cls);
                vtables.put(cls, vt);
                anchors.add("RTTI `" + cls + "` vftable " + hexOrNone(vt));
            }
        }
        for (Slot slot : CHAOS_SLOTS) {
            nameSlots(slot.cls(), null, List.of(slot));
        }
        for (String[] event : CHAOS_EVENTS) {
            boolean fades = !event[1].equals("EVENTTIMER");
            nameSlots(event[0], null, List.of(
                new Slot(event[0], EVENT_APPLY, "Apply", "(particle): " + event[2] + (fades ? "; " + FADE_RULE : ""),
                    false),
                new Slot(event[0], EVENT_PARSE, "Parse", "parses '" + event[1] + " = value' (number, Random(min,"
                    + " max), XYZ or RGBA)", false)));
        }

        long emitterVt = vtables.get(CHAOS_EMITTER);
        Function update = emitterVt < 0 ? null : slotFunction(emitterVt, EMITTER_UPDATE);
        Function stop = emitterVt < 0 ? null : slotFunction(emitterVt, EMITTER_STOP);
        Function compile = emitterVt < 0 ? null : slotFunction(emitterVt, EMITTER_COMPILE_SCRIPT);
        if (update == null || stop == null || compile == null) {
            anomalies.add("Chaos: Emitter Update/Stop/CompileScript slots not found");
            return;
        }

        // Runtime: the sequence update is the callee of Emitter::Update that releases particles like Stop does.
        List<Function> updateCalls = distinct(calls(update));
        Set<Function> stopCalls = new LinkedHashSet<>(calls(stop));
        List<Function> sequenceUpdates = new ArrayList<>();
        for (Function c : updateCalls) {
            for (Function inner : distinct(calls(c))) {
                if (stopCalls.contains(inner) && !sequenceUpdates.contains(c)) {
                    sequenceUpdates.add(c);
                }
            }
        }
        Function sequenceUpdate = single(sequenceUpdates, "Chaos sequence update");
        if (sequenceUpdate == null) {
            return;
        }
        List<Function> sequenceCalls = distinct(calls(sequenceUpdate));
        List<Function> releases = new ArrayList<>(sequenceCalls);
        releases.retainAll(stopCalls);
        Function release = single(releases, "Chaos particle release (callee of Stop and the sequence update)");
        propose(sequenceUpdate, CHAOS_SEQUENCE + SCOPE + "Update", "(sequence, pool, particles, node position,"
            + " time): gravity = sample(+0x88..+0x9c) * (0,0,1) per call; each particle: Particle update, dead ->"
            + " release and erase, alive -> events due by the event timer (COLLISIONCHECK loops forever)", false);
        propose(release, CHAOS_MANAGER + SCOPE + "ReleaseParticle", "(particle, template): copies the pool template"
            + " over the particle (all fields back to their defaults)", true);

        List<Function> creates = new ArrayList<>();
        for (Function c : updateCalls) {
            if (!c.equals(sequenceUpdate) && !java.util.Collections.disjoint(calls(c), sequenceCalls)) {
                creates.add(c);
            }
        }
        Function create = single(creates, "Chaos particle creation (callee of Emitter::Update sharing a callee"
            + " with the sequence update)");
        if (create == null) {
            return;
        }
        propose(create, CHAOS_SEQUENCE + SCOPE + "CreateParticle", "(sequence, particles, node position, node"
            + " velocity, node rotation, time, node scale; pool in EDX): fails at numparticles (+0xe8); lifetime ="
            + " sample(+0x78/+0x7c); position = R * (sample(chaoslocation) * scale) (+ node position unless Local);"
            + " events at time 0; velocity = R * (v + sample(chaosvelocity)) + node velocity * sample(inherited);"
            + " size, velocity and their steps * scale", false);
        List<Function> samplers = new ArrayList<>(distinct(calls(create)));
        samplers.retainAll(sequenceCalls);
        Function sampleVector = single(samplers, "Chaos vector range sampler");
        propose(sampleVector, CHAOS_SCRIPT + SCOPE + "SampleVectorRange", "(min xyz, max xyz) by value: per"
            + " component sample(min, max) with one MT19937 draw each", false);
        Function genRand = sampleVector == null ? null : single(distinct(calls(sampleVector)), "MT19937 output");
        propose(genRand, MERSENNE_TWISTER + SCOPE + "NextU32", "global MT19937: next state word with the standard"
            + " tempering; twists when the 624-word index runs out", false);

        List<Function> intSamplers = new ArrayList<>();
        List<Function> floatSamplers = new ArrayList<>();
        for (Function c : updateCalls) {
            if (genRand == null || c.equals(create) || !distinct(calls(c)).contains(genRand)) {
                continue;
            }
            (usesScalar(c, X87_TRUNCATE_CONTROL) ? intSamplers : floatSamplers).add(c);
        }
        propose(single(intSamplers, "Chaos integer range sampler"), CHAOS_SCRIPT + SCOPE + "SampleIntRange",
            "(min EAX, max ECX): min == max -> min; else r = u32 * 2^-32; min < max -> trunc(min + (max - min) * r),"
            + " min > max -> trunc(r) = 0", false);
        propose(single(floatSamplers, "Chaos float range sampler"), CHAOS_SCRIPT + SCOPE + "SampleFloatRange",
            "(range* EAX): min == max -> min; min < max -> min + (max - min) * r; min > max -> r", false);

        List<Function> particleUpdates = new ArrayList<>();
        List<Function> eventRunners = new ArrayList<>();
        for (Function c : sequenceCalls) {
            if (c.equals(release) || c.equals(sampleVector) || !callers(c).equals(List.of(sequenceUpdate))) {
                continue;
            }
            (calls(c).isEmpty() ? eventRunners : particleUpdates).add(c);
        }
        propose(single(particleUpdates, "Chaos particle update"), "Particle" + SCOPE + "Update", "(time, gravity,"
            + " sequence flags; particle in EDI): age >= lifetime -> dead; event timer += dt; StayAtEmitter ->"
            + " position = emitter; else thrust along the velocity, v += (gravity + velocity step + thrust) * dt,"
            + " p += v * dt; rotation, colour, size and length += step * dt", false);
        propose(single(eventRunners, "Chaos event runner"), CHAOS_SEQUENCE + SCOPE + "RunEvents", "(sequence;"
            + " particle in EDI): applies the events whose time <= the event timer; an EVENTTIMER change restarts at"
            + " the last event before the new timer", false);

        Function twist = genRand == null ? null : single(distinct(calls(genRand)), "MT19937 twist");
        propose(twist, MERSENNE_TWISTER + SCOPE + "Twist", "regenerates the 624 state words (0x9908b0df)", false);
        if (twist != null) {
            for (Function c : callers(twist)) {
                if (callsNamed(c, TIME64)) {
                    propose(c, MERSENNE_TWISTER + SCOPE + "SeedFromClock", "CRT initializer: seeds from a hash of"
                        + " _time64() and clock(), then twists", false);
                    for (Function inner : distinct(calls(c))) {
                        if (usesScalar(inner, MT_INIT_MULTIPLIER)) {
                            propose(inner, MERSENNE_TWISTER + SCOPE + "Seed", "init_genrand (1812433253)", false);
                        }
                    }
                } else if (usesScalar(c, MT_ARRAY_MULTIPLIER)) {
                    propose(c, MERSENNE_TWISTER + SCOPE + "SeedByArray", "init_by_array (1664525, 1566083941)", false);
                }
            }
        }

        // Script compiler.
        List<Function> tokenizers = new ArrayList<>();
        for (Function c : distinct(calls(compile))) {
            if (callsNamed(c, IS_SPACE)) {
                tokenizers.add(c);
            }
        }
        Function tokenize = single(tokenizers, "Chaos tokenizer");
        propose(tokenize, CHAOS_SCRIPT + SCOPE + "Tokenize", "(text, ...): whitespace, words, quoted strings, '//'"
            + " comments; delimiters ( ) , \" { }; '=' is not a delimiter", false);
        Function classify = single(stringUsers("EMITRADIUS"), "Chaos token classifier");
        propose(classify, CHAOS_SCRIPT + SCOPE + "ClassifyToken", "(token in ESI): uppercases in place, token type"
            + " (+0x1c) by ordered substring checks", false);
        if (tokenize != null && classify != null) {
            List<Function> adders = new ArrayList<>(distinct(calls(tokenize)));
            adders.retainAll(callers(classify));
            propose(single(adders, "Chaos token adder"), CHAOS_SCRIPT + SCOPE + "AddToken", "classifies and"
                + " appends a non-empty token", false);
        }
        String[][] parserStrings = {
            {"Unexpected \"%s\" in Particle System Block!", CHAOS_EMITTER + SCOPE + "CompileSystemBlock",
                "MINEMITTIME (+0x6c/+0x70), LOCATIONONLY (+0x76), EVENTSEQUENCE blocks"},
            {"Must name particle sequence block!", CHAOS_SEQUENCE + SCOPE + "CompileBlock", "properties before"
                + " events; event times sampled once, events sorted by time, fades linked"},
            {"Cannot have velocityX event when StayAtEmitter is true!", CHAOS_SEQUENCE + SCOPE + "CreateEvent",
                "event factory by keyword"},
            {"Expecting CELSETUP(...)!", CHAOS_SCRIPT + SCOPE + "ParseCelSetup", "CelSetup(frames, u step, v step,"
                + " columns, rows, fps, loop > 0)"},
            {"Expecting ( after COLLISIONCHECK!", CHAOS_SCRIPT + SCOPE + "ParseCollision", "CollisionCheck(...)"},
            {"Expecting XYZ(...)!", CHAOS_SCRIPT + SCOPE + "ParseVector", "XYZ(x, y, z) of number | Random"},
            {"Expecting RGBA(...)!", CHAOS_SCRIPT + SCOPE + "ParseColor", "RGBA(r, g, b, a) of number | Random"},
            {"Expecting value boolean (true/false/yes/no/0/1 after boolean property!", CHAOS_SCRIPT + SCOPE
                + "ParseBool", "true/false/yes/no/0/1"},
            {"Expecting particle type (point, mesh, axial, emitter, etc..).", CHAOS_SCRIPT + SCOPE
                + "ParseParticleType", "POINT 1, MESH 2, BILLBOARD 3, EMITTER 4, AXIAL 5, LIGHT 6"},
            {"Expecting alpha blend mode (ALPHA_ZERO, ALPHA_ONE, etc.).", CHAOS_SCRIPT + SCOPE + "ParseBlendMode",
                "ALPHA_* name table"},
            {"Expected time specifier: \"[fade so] at\", \"initial\", or \"[fade so] final\"", CHAOS_SCRIPT + SCOPE
                + "ParseTime", "INITIAL = 0, FINAL = lifetime max, AT number | Random, FADE SO prefix"},
        };
        Map<String, Function> parsers = new HashMap<>();
        for (String[] p : parserStrings) {
            Function f = single(stringUsers(p[0]), "user of '" + p[0] + "'");
            parsers.put(p[1], f);
            propose(f, p[1], p[2], false);
        }
        Function celSetup = parsers.get(CHAOS_SCRIPT + SCOPE + "ParseCelSetup");
        Function sequenceBlock = parsers.get(CHAOS_SEQUENCE + SCOPE + "CompileBlock");
        if (celSetup != null) {
            List<Function> ranges = new ArrayList<>();
            for (Function c : distinct(calls(celSetup))) {
                if (callsNamed(c, PARSE_FLOAT)) {
                    ranges.add(c);
                }
            }
            propose(single(ranges, "Chaos float range parser"), CHAOS_SCRIPT + SCOPE + "ParseRange", "number |"
                + " Random(min, max) -> {min, max} (atof)", false);
        }
        if (sequenceBlock != null) {
            List<Function> intRanges = new ArrayList<>();
            List<Function> resolvers = new ArrayList<>();
            List<Function> sorters = new ArrayList<>();
            for (Function c : distinct(calls(sequenceBlock))) {
                List<Function> inner = distinct(calls(c));
                if (callsNamed(c, PARSE_INT)) {
                    intRanges.add(c);
                }
                if (twist != null && inner.contains(twist)) {
                    resolvers.add(c);
                }
                if (inner.contains(c)) {
                    sorters.add(c);
                }
            }
            propose(single(intRanges, "Chaos integer range parser"), CHAOS_SCRIPT + SCOPE + "ParseIntRange", "number"
                + " | Random(min, max) -> {min, max} (atol & 0xffff)", false);
            propose(single(resolvers, "Chaos event time resolver"), CHAOS_SEQUENCE + SCOPE + "ResolveEventTimes",
                "event time (+0x10) = sample(time range) once per compiled sequence (inlined MT19937)", false);
            propose(single(sorters, "Chaos event sort"), CHAOS_SEQUENCE + SCOPE + "SortEvents", "introsort by event"
                + " time", true);
        }
        List<Function> sequenceCtors = new ArrayList<>();
        for (Function w : vtableWriters(CHAOS_SEQUENCE)) {
            if (callsNamed(w, "InterlockedIncrement")) {
                sequenceCtors.add(w);
            }
        }
        propose(single(sequenceCtors, "ParticleEventSequence constructor"), CHAOS_SEQUENCE + SCOPE + CHAOS_SEQUENCE,
            "(sequence on the stack): defaults numparticles 100, loops -1, multipliers 1.0, burst 1..1, cel 1 frame",
            false);

        // Loading: .ems -> EmitterGroup -> Emitter; the effect node; the per-sequence renderers.
        Function loadEmitter = single(stringUsers("Can't compile emitter script for "), "Chaos emitter loader");
        propose(loadEmitter, "EmitterGroup" + SCOPE + "LoadEmitter", "opens the .ems through UOP, CompileScript,"
            + " builds one renderer per sequence", true);
        Function createEmitter = loadEmitter == null ? null : single(callers(loadEmitter), "Chaos emitter creation");
        propose(createEmitter, "EmitterGroup" + SCOPE + "CreateEmitter", "allocates an Emitter (0x7c) and loads it",
            true);
        Function loadGroup = createEmitter == null ? null : single(callers(createEmitter), "Chaos group loader");
        propose(loadGroup, CHAOS_MANAGER + SCOPE + "LoadEmitterGroup", "(path): uppercased, cached; only paths"
            + " containing EMS create an EmitterGroup", false);
        long chaosVt = vtables.getOrDefault(CHAOS_EFFECT, -1L);
        Function chaosInit = chaosVt < 0 ? null : slotFunction(chaosVt, CHAOS_EFFECT_INIT);
        if (loadGroup != null && chaosInit != null) {
            List<Function> effectLoaders = new ArrayList<>();
            for (Function c : distinct(calls(chaosInit))) {
                if (distinct(calls(c)).contains(loadGroup)) {
                    effectLoaders.add(c);
                }
            }
            Function effectLoader = single(effectLoaders, "Chaos effect loader");
            propose(effectLoader, NAMESPACE + SCOPE + "LoadChaosEmitterEffect", "(path): LoadEmitterGroup, then an"
                + " EmitterEffect node named after the file", true);
            List<Function> groupCallers = callers(loadGroup);
            if (effectLoader != null) {
                List<Function> effectFactories = new ArrayList<>();
                for (Function c : distinct(calls(effectLoader))) {
                    if (!c.equals(loadGroup) && callers(c).equals(groupCallers)) {
                        effectFactories.add(c);
                    }
                }
                propose(single(effectFactories, "Chaos EmitterEffect factory"), CHAOS_MANAGER + SCOPE
                    + "CreateEmitterEffect", "(group name): new EmitterEffect node over the cached group", false);
                List<Function> rendererFactories = new ArrayList<>(groupCallers);
                rendererFactories.remove(effectLoader);
                Function rendererFactory = single(rendererFactories, "Chaos renderer factory");
                propose(rendererFactory, CHAOS_PARTICLE_SET + SCOPE + "CreateSequenceRenderer", "POINT/AXIAL ->"
                    + " ParticleBillboard (texture, alpha blend src/dst, z test without write, AXIAL draws both"
                    + " sides), MESH -> MeshParticle (numparticles .nif clones), EMITTER -> EmitterParticle", true);
                if (rendererFactory != null) {
                    for (String cls : List.of(CHAOS_BILLBOARD, CHAOS_MESH_PARTICLE, CHAOS_EMITTER_PARTICLE)) {
                        propose(writerAmong(rendererFactory, cls), cls + SCOPE + cls, "writes the " + cls
                            + " vftable; called by the renderer factory", false);
                    }
                }
            }
        }

        // Per-frame driver: the callee of UOEffects::UpdateActiveEffects calling Emitter Update and IsActive.
        Function activeUpdate = null;
        for (Proposal p : proposals.values()) {
            if (p.path.equals(NAMESPACE + SCOPE + "UpdateActiveEffects")) {
                activeUpdate = functions.getFunctionAt(toAddr(p.address));
            }
        }
        if (activeUpdate != null) {
            List<Function> drivers = new ArrayList<>();
            for (Function c : distinct(calls(activeUpdate))) {
                Set<Long> offsets = virtualCallOffsets(c);
                if (offsets.contains((long) EMITTER_UPDATE) && offsets.contains((long) EMITTER_IS_ACTIVE)) {
                    drivers.add(c);
                }
            }
            Function driver = single(drivers, "Chaos manager update");
            propose(driver, CHAOS_MANAGER + SCOPE + "Update", "(manager, dt ms): time (+0xc8) += ms * 0.001; every"
                + " active emitter: Update(time, dt); inactive with no particles -> removed", false);
            if (driver != null) {
                List<Function> counters = new ArrayList<>();
                for (Function c : distinct(calls(driver))) {
                    if (!c.isExternal() && calls(c).isEmpty() && callers(c).contains(driver)
                            && bodySize(c) < SMALL_FUNCTION * 2) {
                        counters.add(c);
                    }
                }
                propose(single(counters, "Chaos particle counter"), CHAOS_EMITTER + SCOPE + "CountParticles",
                    "(emitter in EDI): sum of the per-sequence deque sizes", false);
            }
        } else {
            anomalies.add("Chaos: UOEffects::UpdateActiveEffects not proposed");
        }

        long triVt = vtables.get(CHAOS_TRISHAPE);
        Function build = triVt < 0 ? null : slotFunction(triVt, TRISHAPE_BUILD_VERTICES);
        if (build != null) {
            List<Function> cels = new ArrayList<>();
            for (Function c : distinct(calls(build))) {
                if (callers(c).equals(List.of(build))) {
                    cels.add(c);
                }
            }
            propose(single(cels, "Chaos cel UV writer"), CHAOS_TRISHAPE + SCOPE + "SetCelTexCoords", "(shape, index;"
                + " frame in CX, cel setup in EDI): column = frame % columns, row = frame / columns, u0 = column * u"
                + " step, v0 = row * v step; corners (u0,v0+v),(u0,v0),(u0+u,v0),(u0+u,v0+v)", false);
        }
    }

    // Hued Nif effects (offscreen render + hue quad), the world camera effect nodes face, the effects shader and the
    // shared definition slot that plays an effect on an object (animation frame effects).
    private void huedEffects(boolean apply) throws Exception {
        for (String cls : List.of(WORLD_OBJECT, DEFINITION_INTERFACE, EFFECTS_SHADER_HLSL, EFFECTS_SHADER_FFP)) {
            vtables.put(cls, rttiVtable(cls));
            anchors.add("RTTI `" + cls + "` vftable " + hexOrNone(vtables.get(cls)));
        }
        nameSlots(WORLD_OBJECT, null, List.of(OBJECT_SLOTS));

        // Every definition class points its +4 at one function (the interface slot itself is pure), so nameSlots
        // would skip it as folded code.
        long particleVt = vtables.getOrDefault(PARTICLE_DEFINITION, -1L);
        Function playOnObject = particleVt < 0 ? null : slotFunction(particleVt, DEFINITION_PLAY_ON_OBJECT);
        for (EffectType t : EFFECT_TYPES) {
            long vt = vtables.getOrDefault(t.definition(), -1L);
            if (playOnObject != null && vt >= 0 && !playOnObject.equals(slotFunction(vt, DEFINITION_PLAY_ON_OBJECT))) {
                anomalies.add(t.definition() + " +4 differs from " + PARTICLE_DEFINITION + " +4");
                playOnObject = null;
            }
        }
        propose(playOnObject, DEFINITION_INTERFACE + SCOPE + "PlayOnObject", "vtable +4 (out ok, object), shared by"
            + " every definition class: requires object component 4 (effects); InstantiateAndRegister with the object"
            + " (so a hued particle takes the object hue), UOObjectComponentEffects::AddEffect, SetPosition (+0x20) at"
            + " the object tile position (vtable +0x28, ints to float), Start; called for the frame effects of"
            + " mobile and item animations", false);
        long tileVt = vtables.getOrDefault(TILE_EFFECT, -1L);
        Function tileInit = tileVt < 0 ? null : slotFunction(tileVt, EFFECT_INIT);
        if (playOnObject != null && tileInit != null) {
            List<Function> adders = new ArrayList<>();
            List<Function> tileCalls = distinct(calls(tileInit));
            for (Function callee : distinct(calls(playOnObject))) {
                if (tileCalls.contains(callee) && virtualCallOffsets(callee).contains((long) EFFECT_SET_VISIBLE)) {
                    adders.add(callee);
                }
            }
            propose(single(adders, "AddEffect (called by PlayOnObject and UOTileEffect::Init, SetVisible call)"),
                COMPONENT + SCOPE + "AddEffect", "(component, effect shared_ptr): pushes the effect into the list"
                + " (+0xc), SetVisible (+8) from the component flag (+0x18), notifies the owner", true);
        }

        // UOHuedParticleEffectNif: constructor (from UOParticleEffectDefinition::Instantiate), Init callees, Update.
        Function instantiate = definitionInstantiate(PARTICLE_DEFINITION);
        Function ctor = instantiate == null ? null : writerAmong(instantiate, HUED_NIF);
        propose(ctor, HUED_NIF + SCOPE + HUED_NIF, "(object): built by UOParticleEffectDefinition::Instantiate when"
            + " the object hue (vtable +0x58) != 0 and the definition is hued; +0x14 = the object hue as stored,"
            + " +0x18 = object position (vtable +0x28), +0x30 = camera direction * depth bias from the object", false);
        long huedVt = vtables.getOrDefault(HUED_NIF, -1L);
        Function init = huedVt < 0 ? null : slotFunction(huedVt, EFFECT_INIT);
        Function update = huedVt < 0 ? null : slotFunction(huedVt, EFFECT_UPDATE);
        List<Function> quads = new ArrayList<>();
        for (Function user : stringUsers(HUE_ATLAS_STRING)) {
            if (init != null && distinct(calls(init)).contains(user)) {
                quads.add(user);
            }
        }
        Function quad = single(quads, "Init callee using '" + HUE_ATLAS_STRING + "'");
        propose(quad, HUED_NIF + SCOPE + "BuildHueQuad", "camera-facing quad of half size 4.854 (camera right/up),"
            + " vertex colour white, UV2 = hue atlas origin (H / 1024 * 255 / 1024, (H % 1024) / 1024; 0 = unhued);"
            + " material UOEffectsShader with the 512x512 render target as shader map 0 and Build\\Hues\\Hues.dds as"
            + " base map (map flags 0x3100: bilinear, wrap); alpha blend SRCALPHA/INVSRCALPHA, alpha test > 0", false);
        List<Function> cameras = new ArrayList<>();
        if (init != null) {
            for (Function callee : distinct(calls(init))) {
                if (callsNamed(callee, SET_VIEW_FRUSTUM)) {
                    cameras.add(callee);
                }
            }
        }
        propose(single(cameras, "offscreen camera builder (Init callee calling SetViewFrustum)"), HUED_NIF + SCOPE
            + "CreateOffscreenCamera", "NiCamera with an orthographic frustum: left/right -/+4 * width/height,"
            + " top/bottom +/-4, near 8, far 12", false);
        propose(quad == null ? null : callBefore(init, quad), HUED_NIF + SCOPE + "PlaceOffscreenCamera", "copies the"
            + " world camera rotation and puts the camera at the world camera offset (UOWorldViewer +0x18) from the"
            + " effect position", true);
        List<Function> renders = new ArrayList<>();
        if (update != null) {
            for (Function callee : distinct(calls(update))) {
                if (callsNamed(callee, DRAW_VISIBLE_ARRAY)) {
                    renders.add(callee);
                }
            }
        }
        propose(single(renders, "hued render-to-texture (Update callee calling " + DRAW_VISIBLE_ARRAY + ")"), HUED_NIF
            + SCOPE + "RenderToTexture", "begins the render target group (+0x40) with a full clear, sets the offscreen"
            + " camera (+0x74), culls and draws the private scene (+0x44); no lights are set", false);

        // UOWorldViewer: the global both the hued constructor and Init read; Init builds the world camera.
        Set<Long> viewerGlobals = new LinkedHashSet<>();
        if (ctor != null && init != null) {
            viewerGlobals.addAll(readGlobals(ctor));
            viewerGlobals.retainAll(readGlobals(init));
        }
        if (viewerGlobals.size() == 1) {
            label(viewerGlobals.iterator().next(), WORLD_VIEWER_LABEL, "UOWorldViewer singleton (0xb4 bytes): +0xc"
                + " camera direction, +0x18 camera offset (-10 * direction), +0x24 -camera up, +0x30 effect node"
                + " rotation (camera rotation * permutation: node x -> right, y -> view direction, z -> up), +0x7c"
                + " world NiCamera, +0x80 ortho half-height 16", apply);
        } else {
            anomalies.add("UOWorldViewer global: globals read by both the hued constructor and Init: "
                + viewerGlobals);
        }
        Function viewerInit = single(stringUsers(WORLD_VIEWER_INIT_STRING), "user of 'UOWorldViewer::Init()'");
        propose(viewerInit, WORLD_VIEWER + SCOPE + "Init", "creates the world camera, the texture manager and the"
            + " sprite shaders; stores the camera vectors and the effect node rotation (+0x30)", false);
        if (viewerInit != null) {
            List<Function> creators = new ArrayList<>();
            Function frustum = null;
            for (Function callee : distinct(calls(viewerInit))) {
                for (Function inner : distinct(calls(callee))) {
                    if (callsNamed(inner, SET_VIEW_FRUSTUM) && !creators.contains(callee)) {
                        creators.add(callee);
                        frustum = inner;
                    }
                }
            }
            Function creator = single(creators, "world camera builder");
            propose(creator, WORLD_VIEWER + SCOPE + "CreateCamera", "world NiCamera: rotation about z by -pi/2, tilt"
                + " pi/4 - 0.001 and -0.05, then SetCameraFrustum", true);
            propose(creator == null ? null : frustum, WORLD_VIEWER + SCOPE + "SetCameraFrustum", "orthographic"
                + " frustum: top/bottom +/-f, left/right -/+f / 0.75 (aspect), near 8, far 12; f = +0x80 (16) * the"
                + " viewport height fraction", true);
        }

        // UOEffectsShader (the hued quad material): constructors and the stage binding slot.
        List<Function> named = stringUsers(EFFECTS_SHADER_STRING);
        for (String cls : List.of(EFFECTS_SHADER_HLSL, EFFECTS_SHADER_FFP)) {
            List<Function> ctors = new ArrayList<>(vtableWriters(cls));
            ctors.retainAll(named);
            propose(single(ctors, cls + " vftable writer using '" + EFFECTS_SHADER_STRING + "'"), cls + SCOPE + cls,
                "writes the " + cls + " vftable and names the shader '" + EFFECTS_SHADER_STRING + "'", false);
        }
        nameSlots(EFFECTS_SHADER_HLSL, null, List.of(new Slot(EFFECTS_SHADER_HLSL, SHADER_UPDATE_PIPELINE,
            "UpdatePipeline", "binds texturing shader map 0 (the effect render target) to sampler +0x120 and the base"
            + " map (Hues.dds) to sampler +0x140 (flag +0x160); sampler clamp = map flags >> 12 & 3, filter ="
            + " flags >> 8 & 0xf; clears SRGBWRITEENABLE",
            true)));
    }

    // Round 6: where effect NIFs are drawn, the renderer background colour of the hued pass, and how a texture
    // named *.nif reaches the renderer (NiNIFImageReader) with its mipmap settings.
    private void effectRendering(boolean apply) throws Exception {
        // Render groups: the global array UOParticleEffectNif::Release indexes by group, whose entry 7 the hued
        // Init reads (the plain Init's indexed read sits in a chunk Ghidra split off).
        long nifVt = vtables.getOrDefault(NIF_EFFECT, -1L);
        long huedVt = vtables.getOrDefault(HUED_NIF, -1L);
        Function nifRelease = nifVt < 0 ? null : slotFunction(nifVt, EFFECT_RELEASE);
        Function huedInit = huedVt < 0 ? null : slotFunction(huedVt, EFFECT_INIT);
        Set<Long> groups = nifRelease == null ? new LinkedHashSet<>() : indexedTables(nifRelease);
        Set<Long> huedReads = huedInit == null ? Set.of() : readGlobals(huedInit);
        groups.removeIf(base -> !huedReads.contains(base + (long) EFFECT_RENDER_GROUP * POINTER_SIZE));
        if (groups.size() == 1) {
            renderGroups = groups.iterator().next();
            label(renderGroups, RENDER_GROUPS_LABEL, "render group scene nodes (NiNode*[]), indexed by"
                + " group; UOParticleEffectNif::Init attaches effect scenes to group 7, a parentless node with no"
                + " dynamic effects (no lights)", apply);
        } else {
            anomalies.add("render groups: tables indexed by " + entry(nifRelease) + " with entry 7 read by "
                + entry(huedInit) + ": " + groups);
        }

        // NiDX9Renderer slots, from the vtable holding GetRTTI.
        Function getRtti = functionNamed(DX9_GET_RTTI);
        List<Long> rttiSlots = getRtti == null ? List.of() : findDwords(getRtti.getEntryPoint().getOffset());
        if (rttiSlots.size() == 1) {
            long vt = rttiSlots.get(0) - RENDERER_GET_RTTI;
            Function render = functionNamed(RENDER_TO_TEXTURE);
            Set<Long> used = render == null ? Set.of() : virtualCallOffsets(render);
            if (!used.contains((long) RENDERER_SET_BACKGROUND_RGBA) || !used.contains((long) RENDERER_GET_BACKGROUND)) {
                anomalies.add(RENDER_TO_TEXTURE + " virtual calls " + used + " lack the background colour slots");
            }
            propose(slotFunctionCreating(vt, RENDERER_SET_BACKGROUND_RGBA, apply), DX9_RENDERER + SCOPE + "SetBackgroundColor",
                "(NiColorA): packs r, g, b and a into the D3DCOLOR clear colour (+0x3fc); the hued pass sets"
                + " (0, 0, 0, 0)", false);
            propose(slotFunctionCreating(vt, RENDERER_SET_BACKGROUND_RGB, apply), DX9_RENDERER + SCOPE + "SetBackgroundColor",
                "(NiColor): packs r, g, b into the D3DCOLOR clear colour (+0x3fc), alpha opaque", false);
            propose(slotFunctionCreating(vt, RENDERER_GET_BACKGROUND, apply), DX9_RENDERER + SCOPE + "GetBackgroundColor",
                "(NiColorA&): unpacks the D3DCOLOR clear colour (+0x3fc)", false);
            propose(slotFunctionCreating(vt, RENDERER_USE_LEGACY_PIPELINE, apply), DX9_RENDERER + SCOPE
                + "UseLegacyPipelineAsDefaultMaterial", "swaps the default material (+0x8a4) for the legacy one;"
                + " no caller found, so effects keep NiStandardMaterial on SM2+ hardware", true);
        } else {
            anomalies.add(DX9_GET_RTTI + " vtable slots: " + rttiSlots.size());
        }

        // NiDX9SourceTextureData::Create and NiTexture::ms_bMipmapByDefault (the byte its MIP_DEFAULT test reads).
        Function createSurf = functionNamed(CREATE_SURF_FROM_RENDERER_DATA);
        Function create = createSurf == null ? null : single(distinct(callers(createSurf)),
            "caller of " + CREATE_SURF_FROM_RENDERER_DATA);
        propose(create, SOURCE_TEXTURE_DATA + SCOPE + "Create", "renderer data of an NiSourceTexture: .bmp/.tga/.dds"
            + " with the load-direct hint through D3DX, otherwise NiImageConverter::ReadImageFile; mipmapped when"
            + " the prefs say YES, or MIP_DEFAULT and ms_bMipmapByDefault", false);
        Set<Long> flags = new LinkedHashSet<>();
        if (create != null) {
            for (Instruction i : listing.getInstructions(create.getBody(), true)) {
                Long target = i.getMnemonicString().equals("CMP") && i.getDefaultOperandRepresentation(0)
                    .startsWith("byte ptr") ? absoluteMemoryOperand(i, 0) : null;
                if (target != null) {
                    flags.add(target);
                }
            }
        }
        if (flags.size() == 1) {
            label(flags.iterator().next(), MIPMAP_BY_DEFAULT_LABEL, "NiTexture::ms_bMipmapByDefault (bool): in .bss"
                + " (0, false) and never written", apply);
        } else {
            anomalies.add("ms_bMipmapByDefault: byte globals compared in Create: " + flags);
        }

        // NiDevImageConverter: the vtable written by its destructor that holds GenerateMipmapLevels.
        Function dtor = functionNamed(DEV_IMAGE_CONVERTER_DTOR);
        Function mipmaps = functionNamed(GENERATE_MIPMAP_LEVELS);
        long converterVt = -1;
        if (dtor != null && mipmaps != null) {
            for (long slot : findDwords(mipmaps.getEntryPoint().getOffset())) {
                for (long vt : addressTakenGlobals(dtor)) {
                    if (vt <= slot && slot - vt < (long) CONVERTER_MAX_SLOTS * POINTER_SIZE) {
                        converterVt = vt;
                    }
                }
            }
        }
        if (converterVt < 0) {
            anomalies.add(DEV_IMAGE_CONVERTER + " vtable not found");
            return;
        }
        List<Function> converterCtors = new ArrayList<>(referencingFunctions(converterVt));
        converterCtors.remove(dtor);
        Function converterCtor = single(converterCtors, DEV_IMAGE_CONVERTER + " vtable writer other than the destructor");
        propose(converterCtor, DEV_IMAGE_CONVERTER + SCOPE + DEV_IMAGE_CONVERTER, "registers the image readers"
            + " BMP, DDS, SGI, TGA and NIF (NiNIFImageReader, first in the list)", false);
        propose(slotFunctionCreating(converterVt, CONVERTER_READ_IMAGE_FILE, apply), DEV_IMAGE_CONVERTER + SCOPE + "ReadImageFile",
            "(file name, optional destination): opens the file through NiFile and asks the first reader whose"
            + " CanReadFile accepts the extension", false);
        // Ghidra splits the tail of ConvertPixelData (the part calling GenerateMipmapLevels) into a function that
        // ConvertPixelData jumps to.
        Function convert = slotFunction(converterVt, CONVERTER_CONVERT_PIXEL_DATA);
        boolean reachesMipmaps = false;
        for (Function caller : distinct(callers(mipmaps))) {
            reachesMipmaps |= caller.equals(convert) || referencingFunctions(caller.getEntryPoint().getOffset())
                .contains(convert);
        }
        if (convert != null && !reachesMipmaps) {
            anomalies.add(DEV_IMAGE_CONVERTER + " +0x14 does not reach GenerateMipmapLevels");
        }
        propose(convert, DEV_IMAGE_CONVERTER + SCOPE + "ConvertPixelData", "(source, format, optional destination,"
            + " mipmap): generates levels only when mipmap is set, the source has one level and power-of-two sides,"
            + " and the format is RGB, RGBA or a bump format; stored levels are converted and kept", false);

        // NiNIFImageReader: the reader vtable the converter constructor writes whose CanReadFile compares ".nif".
        List<Long> readers = new ArrayList<>();
        if (converterCtor != null) {
            for (long vt : addressTakenGlobals(converterCtor)) {
                long canRead = dwordAt(vt + READER_CAN_READ_FILE);
                if (canRead >= 0 && isExec(canRead) && pushesString(canRead, NIF_EXTENSION)) {
                    readers.add(vt);
                }
            }
        }
        if (readers.size() != 1) {
            anomalies.add(NIF_IMAGE_READER + " vtables: " + readers);
            return;
        }
        long readerVt = readers.get(0);
        propose(slotFunctionCreating(readerVt, READER_DESTRUCTOR, apply), NIF_IMAGE_READER + SCOPE + "ScalarDeletingDestructor",
            "destroys the stream and the critical section", true);
        propose(slotFunctionCreating(readerVt, READER_CAN_READ_FILE, apply), NIF_IMAGE_READER + SCOPE + "CanReadFile",
            "(extension): stricmp with \".nif\"", false);
        propose(slotFunctionCreating(readerVt, READER_READ_FILE, apply), NIF_IMAGE_READER + SCOPE + "ReadFile", "(NiFile&, optional"
            + " destination): loads the file as an NiStream; succeeds only when it holds exactly one object and that"
            + " object is an NiPixelData, returned as stored (all its levels)", false);
        propose(slotFunctionCreating(readerVt, READER_READ_HEADER, apply), NIF_IMAGE_READER + SCOPE + "ReadHeader", "reads the whole"
            + " file (ReadFile) and reports width, height, format, mipmapped (levels > 1) and faces", false);
    }

    private void drawOrderAndMaterials(boolean apply) throws Exception {
        // World pass: the caller of NiDrawVisibleArray that reads the effects group and the group drawn after it.
        Function draw = functionNamed(DRAW_VISIBLE_ARRAY);
        Function world = null;
        if (draw != null && renderGroups >= 0) {
            List<Function> drawers = new ArrayList<>();
            for (Function caller : distinct(callers(draw))) {
                Set<Long> reads = readGlobals(caller);
                if (reads.contains(renderGroups + (long) EFFECT_RENDER_GROUP * POINTER_SIZE)
                    && reads.contains(renderGroups + (long) GROUP_AFTER_EFFECTS * POINTER_SIZE)) {
                    drawers.add(caller);
                }
            }
            world = single(drawers, "caller of " + DRAW_VISIBLE_ARRAY + " reading render groups 7 and 9");
        }
        propose(world, "DrawWorldRenderGroups", "draws the render groups 0, 1, 2, 3, 11, 6, then 7 (effects and"
            + " ParticleSystemRoot, skipped while the viewer +0x54 is set) and 9: each with its own culling and"
            + " NiDrawVisibleArray through the renderer's NiAlphaAccumulator, so each group is sorted on its own and"
            + " only the depth buffer is shared", true);

        // ParticleSystemRoot set-up: the user of "Data/Effects" that reads the effects group slot.
        List<Function> roots = new ArrayList<>();
        for (Function f : stringUsers(EFFECTS_DIRECTORY)) {
            if (renderGroups >= 0 && readGlobals(f).contains(renderGroups + (long) EFFECT_RENDER_GROUP * POINTER_SIZE)) {
                roots.add(f);
            }
        }
        propose(single(roots, "user of \"" + EFFECTS_DIRECTORY + "\" reading render group 7"),
            "UOEffects::InitParticleSystemRoot", "sets the Chaos gravity axis and resource directories, then attaches"
            + " ParticleSystemRoot to render group 7, so Chaos sequences sort together with the NIF effects", true);

        // Sorting: NiAlphaAccumulator constructor and its UO creator, NiSortAdjustNode, the bound update modifier and
        // the UO sorter used for the optional first pass.
        long accumulatorVt = niRttiVtable(ALPHA_ACCUMULATOR);
        Function createObject = functionNamed(ALPHA_ACCUMULATOR + SCOPE + "CreateObject");
        Function accumulatorCtor = null;
        if (accumulatorVt >= 0 && createObject != null) {
            List<Function> ctors = new ArrayList<>(referencingFunctions(accumulatorVt));
            ctors.retainAll(distinct(calls(createObject)));
            accumulatorCtor = single(ctors, ALPHA_ACCUMULATOR + " vtable writer called by CreateObject");
        }
        propose(accumulatorCtor, ALPHA_ACCUMULATOR + SCOPE + ALPHA_ACCUMULATOR, "ObserveNoSortHint = 1,"
            + " SortByClosestPoint = 0: blended objects without the no-sort bit are sorted far to near by world bound"
            + " centre along the camera direction; the others are drawn at once", false);
        if (accumulatorCtor != null) {
            List<Function> creators = new ArrayList<>();
            for (Function caller : distinct(callers(accumulatorCtor))) {
                if (!caller.getName(true).startsWith(ALPHA_ACCUMULATOR + SCOPE)) {
                    creators.add(caller);
                }
            }
            propose(single(creators, "UO caller of the " + ALPHA_ACCUMULATOR + " constructor"),
                "UOGraphics::CreateRendererAndSorter", "creates the DX9 renderer, then a new NiAlphaAccumulator that"
                + " it sets as the renderer's sorter", true);
        }
        long sortAdjustVt = niRttiVtable(SORT_ADJUST_NODE);
        propose(sortAdjustVt < 0 ? null : slotFunctionCreating(sortAdjustVt, NODE_ON_VISIBLE, apply),
            SORT_ADJUST_NODE + SCOPE + "OnVisible", "mode (+0xe0) SORTING_OFF: sets the no-sort bit (0x40 at"
            + " NiAVObject +0xac) so the accumulator draws the subtree unsorted in traversal order", false);
        long boundVt = niRttiVtable(BOUND_UPDATE_MODIFIER);
        propose(boundVt < 0 ? null : slotFunctionCreating(boundVt, MODIFIER_UPDATE, apply),
            BOUND_UPDATE_MODIFIER + SCOPE + "Update", "AABB of the particle positions +- radius * size, sampled every"
            + " skip particles (auto: count / 50 + 1); centre = AABB centre, radius = half diagonal: the sort key of a"
            + " particle system", false);
        long sorterVt = rttiVtable(SHADER_OVERRIDE_SORTER);
        propose(sorterVt < 0 ? null : slotFunctionCreating(sorterVt, SORTER_REGISTER_OBJECT_ARRAY, apply),
            SHADER_OVERRIDE_SORTER + SCOPE + "RegisterObjectArray", "draws every object at once in array order"
            + " (no sort), each wrapped in a shader override; used only by the optional first pass of groups 1 and"
            + " 10", false);

        // NiStandardMaterial texture handlers.
        long materialVt = niRttiVtable(STANDARD_MATERIAL);
        for (String[] handler : STANDARD_MATERIAL_HANDLERS) {
            int offset = Integer.decode(handler[0]);
            propose(materialVt < 0 ? null : slotFunctionCreating(materialVt, offset, apply),
                STANDARD_MATERIAL + SCOPE + handler[1], handler[2], false);
        }

        // Render targets: NiRenderedTexture::Create (called by BuildHueQuad, writes the vtable) and the present
        // parameters (the caller of both GetNearestFrameBufferFormat and GetMultiSampleType).
        long renderedVt = niRttiVtable(RENDERED_TEXTURE);
        Function hueQuad = functionNamed(BUILD_HUE_QUAD);
        if (renderedVt >= 0 && hueQuad != null) {
            List<Function> creates = new ArrayList<>(referencingFunctions(renderedVt));
            creates.retainAll(distinct(calls(hueQuad)));
            propose(single(creates, RENDERED_TEXTURE + " vtable writer called by " + BUILD_HUE_QUAD),
                RENDERED_TEXTURE + SCOPE + "Create", "(width, height, renderer, prefs): the hue pass asks 512 x 512,"
                + " TRUE_COLOR_32 / SMOOTH / NO mip, i.e. A8R8G8B8 with destination alpha", false);
        }
        Function nearest = functionNamed(NEAREST_FRAME_BUFFER_FORMAT);
        Function multisample = functionNamed(MULTISAMPLE_TYPE);
        if (nearest != null && multisample != null) {
            List<Function> present = distinct(callers(nearest));
            present.retainAll(callers(multisample));
            propose(single(present, "caller of " + NEAREST_FRAME_BUFFER_FORMAT + " and " + MULTISAMPLE_TYPE),
                DX9_RENDERER + SCOPE + "InitializePresentParams", "back buffer format: FBFMT_UNKNOWN from the UO"
                + " renderer set-up becomes the nearest display mode format (X8R8G8B8 on a 32-bit desktop), so the"
                + " world pass has no destination alpha (DestAlpha reads 1)", false);
        }

        // Chaos resources: NiSourceTexture::Create (vtable writer calling CreateRendererData), the Chaos texture
        // cache calling it from CreateSequenceRenderer, its prefs and load-direct globals, and the mesh cache.
        // NiSourceTexture creators: vtable writers calling CreateRendererData (the file name and pixel data overloads).
        long sourceVt = niRttiVtable(SOURCE_TEXTURE);
        Function createRendererData = functionNamed(CREATE_RENDERER_DATA);
        List<Function> sourceCreates = new ArrayList<>();
        if (sourceVt >= 0 && createRendererData != null) {
            sourceCreates.addAll(referencingFunctions(sourceVt));
            sourceCreates.retainAll(distinct(callers(createRendererData)));
        }
        Function sequenceRenderer = functionNamed(CHAOS_PARTICLE_SET + SCOPE + "CreateSequenceRenderer");
        if (sequenceRenderer == null) {
            return;
        }
        List<Function> rendererCallees = distinct(calls(sequenceRenderer));
        List<Function> textureLoaders = new ArrayList<>();
        List<Function> meshLoaders = new ArrayList<>();
        List<Function> unsharers = new ArrayList<>();
        for (Function callee : rendererCallees) {
            List<Function> calleeCalls = distinct(calls(callee));
            List<Function> creates = new ArrayList<>(sourceCreates);
            creates.retainAll(calleeCalls);
            if (!creates.isEmpty()) {
                textureLoaders.add(callee);
            }
            if (calleeCalls.contains(callee)) {
                unsharers.add(callee);
            }
            if (callers(callee).equals(List.of(sequenceRenderer))
                && virtualCallOffsets(callee).contains((long) AV_OBJECT_UPDATE) && !calleeCalls.contains(callee)) {
                meshLoaders.add(callee);
            }
        }
        Function textureLoader = single(textureLoaders, "callee of CreateSequenceRenderer calling NiSourceTexture::Create");
        Function sourceCreate = null;
        if (textureLoader != null) {
            List<Function> creates = new ArrayList<>(sourceCreates);
            creates.retainAll(distinct(calls(textureLoader)));
            sourceCreate = single(creates, SOURCE_TEXTURE + " creator called by the Chaos texture loader");
        }
        propose(sourceCreate, SOURCE_TEXTURE + SCOPE + "Create", "(name, load-direct hint; prefs in EAX): copies the"
            + " prefs, sets the hint, standardises the path and preloads (CreateRendererData) when ms_bPreload", false);
        propose(textureLoader, CHAOS_MANAGER + SCOPE + "LoadTexture", "(name): cached by name; otherwise"
            + " NiSourceTexture::Create with ms_kDefaultPrefs (PIX_DEFAULT, ALPHA_DEFAULT, MIP_DEFAULT) and the"
            + " load-direct hint byte (0): image converter path, mipmapped only when ms_bMipmapByDefault (false), so"
            + " Chaos textures get no generated levels", false);
        propose(single(meshLoaders, "callee of CreateSequenceRenderer updating a loaded scene"), CHAOS_MANAGER + SCOPE
            + "LoadMesh", "(name): loads the .nif once through NiStream (cached by name), Update(0, 1),"
            + " UpdateProperties, UpdateEffects; returns a clone updated the same way", false);
        propose(single(unsharers, "recursive callee of CreateSequenceRenderer"), CHAOS_PARTICLE_SET + SCOPE
            + "UnshareAnimatedGeometry", "walks the clone: an NiGeometry whose properties have controllers gets a deep"
            + " copy of its geometry data (NiStream copy), so each MESH particle animates on its own", false);
        if (textureLoader == null) {
            return;
        }
        List<Long> prefs = new ArrayList<>();
        for (long global : addressTakenGlobals(textureLoader)) {
            boolean match = true;
            for (int i = 0; i < DEFAULT_FORMAT_PREFS.length; i++) {
                match &= dwordAt(global + (long) i * POINTER_SIZE) == DEFAULT_FORMAT_PREFS[i];
            }
            if (match) {
                prefs.add(global);
            }
        }
        if (prefs.size() == 1) {
            label(prefs.get(0), DEFAULT_PREFS_LABEL, "NiSourceTexture::ms_kDefaultPrefs (NiTexture::FormatPrefs):"
                + " PIX_DEFAULT, ALPHA_DEFAULT, MIP_DEFAULT; passed by the Chaos texture loader", apply);
        } else {
            anomalies.add(DEFAULT_PREFS_LABEL + ": candidates " + prefs);
        }
        Set<Long> hints = new LinkedHashSet<>();
        for (Instruction i : listing.getInstructions(textureLoader.getBody(), true)) {
            if (i.getMnemonicString().equals("MOVZX")) {
                hints.addAll(readTargets(i));
            }
        }
        if (hints.size() == 1) {
            label(hints.iterator().next(), LOAD_DIRECT_HINT_LABEL, "bool, .bss (0) and never written: passed as"
                + " bLoadDirectToRendererHint by the Chaos texture loader and 6 other UO loaders, and read by NiSourceTexture"
                + " LoadBinary / CreateObject; 0 keeps the NiImageConverter path", apply);
        } else {
            anomalies.add(LOAD_DIRECT_HINT_LABEL + ": byte reads " + hints);
        }
    }

    // Round 8: the renderer's filter and clamp tables, the stage setters that read them and the initial sampler states
    // (no FILTER_ANISOTROPIC entry; D3DSAMP_MAXANISOTROPY stays 1).
    private void textureFiltering(boolean apply) throws Exception {
        Function caps = single(stringUsers(DEVICE_CAPS_ERROR), "user of \"" + DEVICE_CAPS_ERROR + "\"");
        propose(caps, DX9_RENDERER + SCOPE + "InitializeDeviceCaps", "GetDeviceCaps into the renderer (+0xa8); needs"
            + " two or more blend stages and simultaneous textures; fills the filter mapping (six FilterModes: no"
            + " FILTER_ANISOTROPIC entry) and the address mapping; reads TextureCaps but neither the anisotropic filter"
            + " caps nor MaxAnisotropy", false);
        if (caps == null) {
            return;
        }
        // The two tables are one run of written dwords: six {mag, min, mip} entries, then four {u, v} entries.
        TreeMap<Long, Boolean> written = new TreeMap<>();
        for (Instruction i : listing.getInstructions(caps.getBody(), true)) {
            Long target = i.getMnemonicString().equals("MOV") ? absoluteMemoryOperand(i, 0) : null;
            if (target != null) {
                written.put(target, true);
            }
        }
        int runWords = FILTER_MODES * FILTER_MAPPING_WORDS + CLAMP_MODES * ADDRESS_MAPPING_WORDS;
        List<Long> starts = new ArrayList<>();
        for (long start : written.keySet()) {
            if (written.containsKey(start - POINTER_SIZE)) {
                continue;
            }
            int length = 0;
            while (written.containsKey(start + (long) length * POINTER_SIZE)) {
                length++;
            }
            if (length == runWords) {
                starts.add(start);
            }
        }
        if (starts.size() != 1) {
            anomalies.add("filter/address mapping: runs of " + runWords + " written dwords in " + entry(caps) + ": " + starts);
            return;
        }
        long filterMap = starts.get(0);
        long addressMap = filterMap + (long) FILTER_MODES * FILTER_MAPPING_WORDS * POINTER_SIZE;
        label(filterMap, FILTER_MAPPING_LABEL, "NiDX9Renderer::ms_akD3DFilterMapping, {mag, min, mip} per FilterMode:"
            + " NEAREST (POINT, POINT, NONE), BILERP (LINEAR, LINEAR, NONE), TRILERP (LINEAR x3), NEAREST_MIPNEAREST"
            + " (POINT x3), NEAREST_MIPLERP (POINT, POINT, LINEAR), BILERP_MIPNEAREST (LINEAR, LINEAR, POINT); six"
            + " entries, no FILTER_ANISOTROPIC", apply);
        label(addressMap, ADDRESS_MAPPING_LABEL, "NiDX9Renderer::ms_akD3DAddressMapping, {u, v} per ClampMode: CLAMP/CLAMP,"
            + " CLAMP/WRAP, WRAP/CLAMP, WRAP/WRAP", apply);
        List<Function> filterUsers = referencingFunctions(filterMap);
        filterUsers.retainAll(referencingFunctions(filterMap + (long) (FILTER_MAPPING_WORDS - 1) * POINTER_SIZE));
        // The caps set-up writes the tables; the setters only read them.
        filterUsers.remove(caps);
        propose(single(filterUsers, "function reading the filter mapping's mag and mip words"), "NiD3DTextureStage"
            + SCOPE + "SetFilterMode", "sets MAGFILTER, MINFILTER and MIPFILTER of the stage from"
            + " ms_akD3DFilterMapping[mode]; no MAXANISOTROPY", false);
        List<Function> clampUsers = referencingFunctions(addressMap);
        clampUsers.retainAll(referencingFunctions(addressMap + POINTER_SIZE));
        clampUsers.remove(caps);
        propose(single(clampUsers, "function reading the address mapping's u and v words"), "NiD3DTextureStage"
            + SCOPE + "SetClampMode", "sets ADDRESSU and ADDRESSV of the stage from ms_akD3DAddressMapping[mode]", false);

        byte[] pattern = new byte[INITIAL_SAMPLER_STATES.length * POINTER_SIZE];
        for (int i = 0; i < INITIAL_SAMPLER_STATES.length; i++) {
            for (int b = 0; b < POINTER_SIZE; b++) {
                pattern[i * POINTER_SIZE + b] = (byte) (INITIAL_SAMPLER_STATES[i] >> (b * BITS_PER_BYTE) & BYTE_MASK);
            }
        }
        List<Long> initial = findBytes(pattern, true);
        if (initial.size() == 1) {
            label(initial.get(0), INITIAL_SAMPLER_STATES_LABEL, "NiDX9RenderState::ms_akInitialSamplerStateSettings,"
                + " {state, value} pairs applied to every stage at start-up; D3DSAMP_MAXANISOTROPY (10) = 1, and no code"
                + " sets it later", apply);
        } else {
            anomalies.add(INITIAL_SAMPLER_STATES_LABEL + ": candidates " + initial);
        }
    }

    // The vtable of a Gamebryo class from its NiRTTI: the NUL-terminated name, the static NiRTTI holding a pointer
    // to it, the small GetRTTI returning that NiRTTI, and the vtable slot +8 holding GetRTTI.
    private long niRttiVtable(String cls) throws Exception {
        Set<Long> found = new LinkedHashSet<>();
        for (long name : findBytes((cls + "\0").getBytes(StandardCharsets.US_ASCII), true)) {
            if (memory.getByte(toAddr(name - 1)) != 0) {
                continue;
            }
            for (long rtti : findDwords(name)) {
                for (Function getRtti : referencingFunctions(rtti)) {
                    if (bodySize(getRtti) > GET_RTTI_MAX_SIZE) {
                        continue;
                    }
                    for (long slot : findDwords(getRtti.getEntryPoint().getOffset())) {
                        found.add(slot - NI_GET_RTTI_SLOT);
                    }
                }
            }
        }
        if (found.size() != 1) {
            anomalies.add(cls + " NiRTTI vtables: " + found);
            return -1;
        }
        return found.iterator().next();
    }

    // Tables indexed as [reg * 4 + table] in f.
    private Set<Long> indexedTables(Function f) {
        Set<Long> out = new LinkedHashSet<>();
        for (Instruction i : listing.getInstructions(f.getBody(), true)) {
            for (int op = 0; op < i.getNumOperands(); op++) {
                if (!i.getDefaultOperandRepresentation(op).contains("*0x4 + 0x")) {
                    continue;
                }
                for (Object o : i.getOpObjects(op)) {
                    long value = o instanceof Scalar sc ? sc.getUnsignedValue() & DWORD_MASK
                        : o instanceof Address a ? a.getOffset() : -1;
                    if (value >= 0 && memory.contains(toAddr(value)) && !isExec(value)) {
                        out.add(value);
                    }
                }
            }
        }
        return out;
    }

    // Whether the code at start pushes the address of a NUL-terminated copy of text before its first return
    // (a small leaf such as CanReadFile, which may have no function yet).
    private boolean pushesString(long start, String text) throws Exception {
        byte[] expected = (text + "\0").getBytes(StandardCharsets.US_ASCII);
        for (Instruction i = listing.getInstructionAt(toAddr(start)); i != null; i = i.getNext()) {
            if (i.getMnemonicString().equals("PUSH") && i.getNumOperands() == 1) {
                for (Object o : i.getOpObjects(0)) {
                    long value = o instanceof Scalar sc ? sc.getUnsignedValue() & DWORD_MASK
                        : o instanceof Address a ? a.getOffset() : -1;
                    byte[] actual = new byte[expected.length];
                    if (value >= 0 && memory.contains(toAddr(value))
                        && memory.getBytes(toAddr(value), actual) == expected.length
                        && java.util.Arrays.equals(actual, expected)) {
                        return true;
                    }
                }
            }
            if (i.getFlowType().isTerminal()) {
                return false;
            }
        }
        return false;
    }

    // The function at a vtable slot. With apply, creates it when the slot points at code with no function yet.
    private Function slotFunctionCreating(long vt, int offset, boolean apply) throws Exception {
        Function f = slotFunction(vt, offset);
        long target = dwordAt(vt + offset);
        if (f != null || target < 0 || !isExec(target)) {
            return f;
        }
        if (!apply) {
            anomalies.add("no function at " + hex(target) + " (vtable " + hex(vt) + " +" + hexShort(offset)
                + "); --apply creates it");
            return null;
        }
        f = createFunction(toAddr(target), null);
        if (f != null) {
            count("functions created");
        }
        return f;
    }

    private Function functionNamed(String path) {
        for (Function f : functions.getFunctions(true)) {
            if (f.getName(true).equals(path)) {
                return f;
            }
        }
        anomalies.add("function `" + path + "` not found");
        return null;
    }

    private Function definitionInstantiate(String cls) {
        long vt = vtables.getOrDefault(cls, -1L);
        return vt < 0 ? null : slotFunction(vt, DEFINITION_INSTANTIATE);
    }

    private void labelSingleRead(Function f, String name, String evidence, boolean apply) throws Exception {
        Set<Long> reads = f == null ? Set.of() : readGlobals(f);
        if (reads.size() != 1) {
            anomalies.add("`" + name + "`: reads " + reads + " in " + entry(f));
            return;
        }
        label(reads.iterator().next(), name, evidence, apply);
    }

    // The callee of f that writes the vftable of cls, directly or through one of its own callees.
    private Function writerAmong(Function f, String cls) {
        List<Function> writers = vtableWriters(cls);
        List<Function> found = new ArrayList<>();
        for (Function callee : distinct(calls(f))) {
            if (writers.contains(callee)) {
                found.add(callee);
            }
        }
        if (found.isEmpty()) {
            for (Function callee : distinct(calls(f))) {
                for (Function inner : distinct(calls(callee))) {
                    if (writers.contains(inner) && !found.contains(callee)) {
                        found.add(callee);
                    }
                }
            }
        }
        return single(found, cls + " builder");
    }

    // The function called right before the first call to target in f.
    private Function callBefore(Function f, Function target) {
        Function previous = null;
        for (Instruction i : listing.getInstructions(f.getBody(), true)) {
            if (!i.getFlowType().isCall() || i.getFlows().length == 0) {
                continue;
            }
            Function callee = functions.getFunctionAt(i.getFlows()[0]);
            if (callee != null && callee.equals(target)) {
                return previous;
            }
            previous = callee;
        }
        return null;
    }

    // Largest displacement of a [reg + disp] operand whose base is not the stack (packet record field offsets).
    private long maxFieldOffset(Function f) {
        long max = 0;
        for (Instruction i : listing.getInstructions(f.getBody(), true)) {
            for (int op = 0; op < i.getNumOperands(); op++) {
                if (!i.getDefaultOperandRepresentation(op).contains("[")) {
                    continue;
                }
                boolean stack = false;
                long displacement = 0;
                for (Object o : i.getOpObjects(op)) {
                    if (o instanceof Register r) {
                        stack |= r.getName().equals("ESP") || r.getName().equals("EBP");
                    } else if (o instanceof Scalar s) {
                        displacement = s.getUnsignedValue();
                    }
                }
                if (!stack) {
                    max = Math.max(max, displacement);
                }
            }
        }
        return max;
    }

    private boolean usesScalar(Function f, long value) {
        for (Instruction i : listing.getInstructions(f.getBody(), true)) {
            for (int op = 0; op < i.getNumOperands(); op++) {
                for (Object o : i.getOpObjects(op)) {
                    if (o instanceof Scalar s && s.getUnsignedValue() == value) {
                        return true;
                    }
                }
            }
        }
        return false;
    }

    private List<Function> stringUsers(String text) {
        List<Function> users = new ArrayList<>();
        for (long at : findBytes((text + "\0").getBytes(StandardCharsets.US_ASCII), true)) {
            for (Function f : referencingFunctions(at)) {
                if (!users.contains(f)) {
                    users.add(f);
                }
            }
        }
        return users;
    }

    private List<Function> referencingFunctions(long address) {
        List<Function> users = new ArrayList<>();
        for (Reference r : getReferencesTo(toAddr(address))) {
            Function f = getFunctionContaining(r.getFromAddress());
            if (f != null && !users.contains(f)) {
                users.add(f);
            }
        }
        return users;
    }

    // Non-executable addresses whose address f takes (MOV reg, imm / PUSH imm references of type DATA).
    private Set<Long> addressTakenGlobals(Function f) {
        Set<Long> out = new LinkedHashSet<>();
        for (Instruction i : listing.getInstructions(f.getBody(), true)) {
            for (Reference r : i.getReferencesFrom()) {
                long to = r.getToAddress().getOffset();
                if (r.getReferenceType() == RefType.DATA && r.getToAddress().isMemoryAddress() && !isExec(to)) {
                    out.add(to);
                }
            }
        }
        return out;
    }

    private Set<Long> readGlobals(Function f) {
        Set<Long> out = new LinkedHashSet<>();
        for (Instruction i : listing.getInstructions(f.getBody(), true)) {
            out.addAll(readTargets(i));
        }
        return out;
    }

    private Set<Long> readTargets(Instruction i) {
        Set<Long> out = new LinkedHashSet<>();
        for (Reference r : i.getReferencesFrom()) {
            long to = r.getToAddress().getOffset();
            if (r.getReferenceType().isRead() && r.getToAddress().isMemoryAddress() && !isExec(to)) {
                out.add(to);
            }
        }
        return out;
    }

    // ---- code patterns ----

    // CanHandle: CMP [ESP+4], imm (or a register zeroed by XOR) then SETZ.
    private long comparedKey(Function f) {
        Set<Register> zero = new LinkedHashSet<>();
        for (Instruction i : listing.getInstructions(f.getBody(), true)) {
            String m = i.getMnemonicString();
            if (m.equals("XOR") && i.getNumOperands() == 2 && i.getOpObjects(0).length == 1
                    && i.getOpObjects(0)[0] instanceof Register a && i.getOpObjects(1).length == 1
                    && a.equals(i.getOpObjects(1)[0])) {
                zero.add(a);
            } else if (m.equals("CMP") && i.getNumOperands() == 2) {
                for (Object o : i.getOpObjects(1)) {
                    if (o instanceof Scalar s) {
                        return s.getUnsignedValue() & DWORD_MASK;
                    }
                    if (o instanceof Register r && zero.contains(r)) {
                        return 0;
                    }
                }
            }
        }
        return UNKNOWN_KEY;
    }

    // GetType: XOR EAX,EAX / MOV EAX,imm / PUSH imm + POP EAX, then RET.
    private long returnedConstant(Function f) {
        Long value = null;
        Long pushed = null;
        for (Instruction i : listing.getInstructions(f.getBody(), true)) {
            String m = i.getMnemonicString();
            Object[] dst = i.getNumOperands() > 0 ? i.getOpObjects(0) : new Object[0];
            boolean toEax = dst.length == 1 && dst[0] instanceof Register r && r.getName().equals("EAX");
            if (m.equals("XOR") && toEax && i.getOpObjects(1).length == 1 && dst[0].equals(i.getOpObjects(1)[0])) {
                value = 0L;
            } else if (m.equals("MOV") && toEax && i.getOpObjects(1).length == 1
                    && i.getOpObjects(1)[0] instanceof Scalar s) {
                value = s.getUnsignedValue() & DWORD_MASK;
            } else if (m.equals("PUSH") && dst.length == 1 && dst[0] instanceof Scalar s) {
                pushed = s.getUnsignedValue() & DWORD_MASK;
            } else if (m.equals("POP") && toEax && pushed != null) {
                value = pushed;
            } else if (m.equals("RET")) {
                return value == null ? UNKNOWN_KEY : value;
            }
        }
        return UNKNOWN_KEY;
    }

    // PUSH dword ptr [global] followed by a direct CALL: returns {global, callee entry}.
    private long[] globalPushedBeforeCall(Function f) {
        Long global = null;
        for (Instruction i : listing.getInstructions(f.getBody(), true)) {
            if (i.getMnemonicString().equals("PUSH")) {
                Long absolute = absoluteMemoryOperand(i, 0);
                if (absolute != null && !isExec(absolute) && memory.contains(toAddr(absolute))) {
                    global = absolute;
                }
            } else if (global != null && i.getFlowType().isCall() && i.getFlows().length > 0) {
                return new long[] {global, i.getFlows()[0].getOffset()};
            }
        }
        return null;
    }

    // Address of a [disp32] operand without registers; null otherwise.
    private static Long absoluteMemoryOperand(Instruction i, int op) {
        if (!i.getDefaultOperandRepresentation(op).contains("[")) {
            return null;
        }
        Long value = null;
        for (Object o : i.getOpObjects(op)) {
            if (o instanceof Register) {
                return null;
            }
            if (o instanceof Scalar s) {
                value = s.getUnsignedValue() & DWORD_MASK;
            } else if (o instanceof Address a) {
                value = a.getOffset();
            }
        }
        return value;
    }

    private Function singleCallerCallAfter(Function f, Function first) {
        boolean seen = false;
        for (Instruction i : listing.getInstructions(f.getBody(), true)) {
            if (!i.getFlowType().isCall() || i.getFlows().length == 0) {
                continue;
            }
            Function callee = functions.getFunctionAt(i.getFlows()[0]);
            if (seen && callee != null && callers(callee).size() == 1) {
                return callee;
            }
            seen |= callee != null && callee.equals(first);
        }
        return null;
    }

    // ---- naming helpers ----

    private void propose(Function f, String path, String evidence, boolean probable) {
        if (f == null) {
            anomalies.add("not found: `" + path + "`");
            return;
        }
        long address = f.getEntryPoint().getOffset();
        Proposal existing = proposals.get(address);
        if (existing != null) {
            if (!existing.path.equals(path)) {
                anomalies.add(hex(address) + " proposed as `" + existing.path + "` and `" + path + "`; kept the first");
            }
            return;
        }
        Proposal proposal = new Proposal();
        proposal.address = address;
        proposal.path = path;
        proposal.evidence = evidence;
        proposal.probable = probable;
        proposals.put(address, proposal);
    }

    // A slot shared with the base factory is named in the base class; a function referenced from more tables than
    // its class and base vtables is skipped (identical-code folding).
    private Map<Integer, Function> nameSlots(String cls, String baseCls, List<Slot> slots) {
        Map<Integer, Function> result = new LinkedHashMap<>();
        long vt = vtables.getOrDefault(cls, -1L);
        if (vt < 0) {
            anomalies.add("vftable of " + cls + " not found");
            return result;
        }
        long baseVt = baseCls == null ? -1 : vtables.getOrDefault(baseCls, -1L);
        for (Slot slot : slots) {
            Function f = slotFunction(vt, slot.offset());
            if (f == null) {
                anomalies.add(cls + " slot +" + hexShort(slot.offset()) + ": no function");
                continue;
            }
            result.put(slot.offset(), f);
            boolean inBase = baseVt >= 0 && f.equals(slotFunction(baseVt, slot.offset()));
            int allowed = inBase ? 2 : 1;
            int pointers = codePointerCounts.getOrDefault(f.getEntryPoint().getOffset(), 0);
            if (pointers > allowed) {
                shared.add(hex(f.getEntryPoint().getOffset()) + " " + cls + " +" + hexShort(slot.offset()) + " `"
                    + slot.name() + "`: " + pointers + " table pointers (identical-code folding), not named");
                continue;
            }
            propose(f, (inBase ? baseCls : cls) + SCOPE + slot.name(), cls + " vtable +" + hexShort(slot.offset())
                + ": " + slot.evidence(), slot.probable());
        }
        return result;
    }

    private void applyName(Proposal proposal) throws Exception {
        Function function = functions.getFunctionAt(toAddr(proposal.address));
        int split = proposal.path.lastIndexOf(SCOPE);
        String name = split < 0 ? proposal.path : proposal.path.substring(split + SCOPE.length());
        Namespace namespace = split < 0 ? currentProgram.getGlobalNamespace()
            : namespace(proposal.path.substring(0, split));
        Symbol symbol = function.getSymbol();
        if (symbol.getName().equals(name) && symbol.getParentNamespace().equals(namespace)) {
            proposal.status = "already named";
            if (hasStaleOwnComment(function, proposal)) {
                function.setComment(plateComment(proposal));
                count("plate comments refreshed");
            }
        } else if (symbol.getSource() == SourceType.DEFAULT) {
            try {
                symbol.setNameAndNamespace(name, namespace, SourceType.ANALYSIS);
            } catch (DuplicateNameException duplicate) {
                String unique = name + "_" + Long.toHexString(proposal.address);
                symbol.setNameAndNamespace(unique, namespace, SourceType.ANALYSIS);
                anomalies.add("`" + proposal.path + "` exists; " + hex(proposal.address) + " named `" + unique + "`");
            }
            proposal.status = "renamed";
            count("functions renamed");
        } else {
            proposal.status = "kept " + symbol.getName(true) + " (" + symbol.getSource() + ")";
            kept.add(hex(proposal.address) + " `" + symbol.getName(true) + "` (" + symbol.getSource()
                + "), proposed `" + proposal.path + "`");
        }
        boolean tagged = false;
        for (FunctionTag tag : function.getTags()) {
            tagged |= tag.getName().equals(TAG);
        }
        if (!tagged) {
            function.addTag(TAG);
            count("tags added");
        }
        String comment = function.getComment();
        if (comment == null || comment.isBlank()) {
            function.setComment(plateComment(proposal));
            count("plate comments set");
        }
    }

    private static String plateComment(Proposal proposal) {
        return COMMENT_PREFIX + TAG + " `" + proposal.path + "`" + (proposal.probable ? PROBABLE : "") + ": "
            + proposal.evidence;
    }

    // A single-line plate comment this script wrote for the current name, with outdated evidence. Comments from
    // other sources, or edited ones (more than one line), are never touched.
    private static boolean hasStaleOwnComment(Function function, Proposal proposal) {
        String comment = function.getComment();
        return comment != null && comment.startsWith(COMMENT_PREFIX + TAG + " `" + proposal.path + "`")
            && !comment.contains("\n") && !comment.equals(plateComment(proposal));
    }

    private void ensureTag() {
        if (functions.getFunctionTagManager().getFunctionTag(TAG) == null) {
            functions.getFunctionTagManager().createFunctionTag(TAG, TAG_COMMENT);
            count("tag created");
        }
    }

    private Namespace namespace(String path) throws Exception {
        Namespace namespace = currentProgram.getGlobalNamespace();
        for (String part : path.split(SCOPE)) {
            Namespace child = symbols.getNamespace(part, namespace);
            if (child == null) {
                child = symbols.createNameSpace(namespace, part, SourceType.ANALYSIS);
                count("namespaces created");
            }
            namespace = child;
        }
        return namespace;
    }

    private void label(long address, String name, String evidence, boolean apply) throws Exception {
        if (address < 0) {
            anomalies.add("global `" + name + "` not found");
            return;
        }
        Address a = toAddr(address);
        Symbol primary = symbols.getPrimarySymbol(a);
        String status;
        if (primary != null && primary.getName().equals(name)) {
            status = "already labelled";
        } else if (primary != null && primary.getSource() != SourceType.DEFAULT) {
            status = "kept `" + primary.getName(true) + "` (" + primary.getSource() + ")";
        } else if (apply) {
            Symbol created = symbols.createLabel(a, name, SourceType.ANALYSIS);
            created.setPrimary();
            count("labels created");
            status = "labelled";
        } else {
            status = "would label";
        }
        if (apply && (getPreComment(a) == null || getPreComment(a).isBlank())) {
            setPreComment(a, COMMENT_PREFIX + evidence);
            count("label comments set");
        }
        labels.add("| " + hex(address) + " | `" + name + "` | " + status + " | " + evidence + " |");
    }

    private void count(String change) {
        changes.merge(change, 1, Integer::sum);
    }

    // ---- types ----

    private void defineTypes(boolean apply) throws Exception {
        DataType u8 = ByteDataType.dataType;
        DataType i8 = SignedByteDataType.dataType;
        DataType u16 = UnsignedShortDataType.dataType;
        DataType u32 = UnsignedIntegerDataType.dataType;
        DataType i32 = IntegerDataType.dataType;
        DataType f32 = FloatDataType.dataType;
        DataType f64 = DoubleDataType.dataType;
        DataType u64 = QWordDataType.dataType;
        DataType bool = BooleanDataType.dataType;
        DataType voidPtr = new PointerDataType(VoidDataType.dataType, dtm);
        final int stringBuffer = 16;
        final int directions = 8;

        StructureDataType string = struct("MsvcString", 0x1c, "MSVC 2008 std::string: buffer holds a pointer when"
            + " capacity >= 16");
        field(string, 0x0, voidPtr, "allocator", null);
        field(string, 0x4, new ArrayDataType(CharDataType.dataType, stringBuffer, 1, dtm), "buffer", null);
        field(string, 0x14, u32, "size", null);
        field(string, 0x18, u32, "capacity", null);
        StructureDataType vector = struct("MsvcVector", 0x10, "MSVC 2008 std::vector (here of 8-byte shared_ptr"
            + " effect definitions unless noted)");
        field(vector, 0x0, voidPtr, "allocator", null);
        field(vector, 0x4, voidPtr, "first", null);
        field(vector, 0x8, voidPtr, "last", null);
        field(vector, 0xc, voidPtr, "end", null);
        StructureDataType map = struct("MsvcMap", 0xc, "MSVC 2008 std::map<unsigned, vector of definitions>");
        field(map, 0x0, voidPtr, "allocator", null);
        field(map, 0x4, voidPtr, "head", null);
        field(map, 0x8, u32, "size", null);

        StructureDataType particle = struct("UOParticleEffectDefinition", 0x44, "effect type 0");
        field(particle, 0x0, voidPtr, "vftable", null);
        field(particle, 0x4, string, "name", "string dictionary entry; '.ems' selects the Chaos effect");
        field(particle, 0x20, f32, "offset_x", "added to the effect x");
        field(particle, 0x24, f32, "offset_y", "subtracted from the effect z (vertical, probable)");
        field(particle, 0x28, f32, "camera_offset", "times -0.1286 along the camera vector (probable)");
        field(particle, 0x2c, f32, "scale", "node scale");
        field(particle, 0x30, u32, "attach_mode", "record u8 (1, 2, 0); UOObject::CreateComponents does not attach"
            + " the effect to the object when it is " + PARTICLE_ATTACH_PROJECTILE_ONLY + " (thrown models); 0 and 1"
            + " are treated alike");
        field(particle, 0x34, f32, "duration", "seconds; loop period or lifetime");
        field(particle, 0x38, bool, "loop", null);
        field(particle, 0x3c, u32, "field_3c", "record u8, 0 in the data; 1 fails the Nif init, non-zero fails Chaos;"
            + " no other reader");
        field(particle, 0x40, bool, "hued", "with a non-zero object hue the .nif becomes a UOHuedParticleEffectNif"
            + " (luminance remapped through Hues.dds); ignored for .ems");
        StructureDataType sound = struct("UOSoundEffectDefinition", 0xc, "effect type 1");
        field(sound, 0x0, voidPtr, "vftable", null);
        field(sound, 0x4, u32, "sound_id", null);
        field(sound, 0x8, f32, "value_08", "default 1.0; copied to UOSoundEffect +0x14, which no UOSoundEffect method"
            + " reads (Start computes its own distance volume)");
        StructureDataType roundRobin = struct("UORoundRobinEffectCollection", 0x18, "effect type 2");
        field(roundRobin, 0x0, voidPtr, "vftable", null);
        field(roundRobin, 0x4, vector, "children", null);
        field(roundRobin, 0x14, u32, "next_index", null);
        StructureDataType offset = struct("UOPositionOffsetDefinition", 0x44, "effect type 7");
        field(offset, 0x0, voidPtr, "vftable", null);
        field(offset, 0x4, new ArrayDataType(i32, directions, i32.getLength(), dtm), "x", "per direction");
        field(offset, 0x24, new ArrayDataType(i32, directions, i32.getLength(), dtm), "y", "per direction");
        StructureDataType wait = struct("UOWaitCollectionDefinition", 0x24, "effect type 10");
        field(wait, 0x0, voidPtr, "vftable", null);
        field(wait, 0x4, vector, "children", null);
        field(wait, 0x14, u32, "min_delay_ms", "default 2000");
        field(wait, 0x18, u32, "max_delay_ms", "default 4000");
        field(wait, 0x1c, bool, "aligned", "align the next fire time to a multiple of min_delay_ms");
        field(wait, 0x20, u32, "next_fire_ms", "runtime");
        StructureDataType lod = struct("UOEffectLODDefinition", 0x10, "effect type 11");
        field(lod, 0x0, voidPtr, "vftable", null);
        field(lod, 0x4, map, "levels", "LOD level -> children");
        StructureDataType flag = struct("UOEffectPropertyFlagDefinition", 0x30, "effect type 12");
        field(flag, 0x0, voidPtr, "vftable", null);
        field(flag, 0x4, bool, "mode", "0 flag mask test, 1 property value");
        field(flag, 0x8, u64, "mask", "mode 0");
        field(flag, 0x10, u32, "property", "mode 1: 2, 3, 7 (hue), 8 or a tile definition props1 key");
        field(flag, 0x14, vector, "default_children", "mode 1 fallback");
        field(flag, 0x24, map, "cases", "key -> children");
        StructureDataType quake = struct("UOEarthquakeEffectDefinition", 0x20, "effect type 15");
        field(quake, 0x0, voidPtr, "vftable", null);
        field(quake, 0x8, f64, "amplitude", "camera x displacement");
        field(quake, 0x10, f64, "duration", "seconds");
        field(quake, 0x18, f64, "frequency", "Hz (times 2pi)");
        StructureDataType randomEntry = struct("UORandomEffectEntry", 0x14, "weighted choice of type 16");
        field(randomEntry, 0x0, f32, "weight", null);
        field(randomEntry, 0x4, vector, "children", null);
        StructureDataType random = struct("UORandomEffectCollectionDefinition", 0x14, "effect type 16");
        field(random, 0x0, voidPtr, "vftable", null);
        field(random, 0x4, vector, "entries", "vector of UORandomEffectEntry");
        StructureDataType filter = struct("UOUserPrefEffectFilterDefinition", 0x18, "effect type 17");
        field(filter, 0x0, voidPtr, "vftable", null);
        field(filter, 0x4, u32, "mask", "user effect setting bits that suppress the children");
        field(filter, 0x8, vector, "children", null);
        StructureDataType projectile = struct(PROJECTILE_DEFINITION, 0x40, "built by UOEffects::MakeProjectileEffect"
            + " from a type 0 (moving) graphical effect record");
        field(projectile, 0x0, voidPtr, "vftable", null);
        field(projectile, 0x4, i32, "source_x", "packet source x (u16)");
        field(projectile, 0x8, i32, "source_y", "packet source y (u16)");
        field(projectile, 0xc, i32, "source_z", "packet source z (i8)");
        field(projectile, 0x10, i32, "target_x", "packet target x (u16)");
        field(projectile, 0x14, i32, "target_y", "packet target y (u16)");
        field(projectile, 0x18, i32, "target_z", "packet target z (i8)");
        field(projectile, 0x1c, f32, "speed", "packet speed byte; <= 0 -> 1.0");
        field(projectile, 0x20, u32, "item_id", "graphic of the moving item");
        field(projectile, 0x24, u32, "hue", "packet hue + 1, 0 when the packet hue is 0 (UOObject::SetHue)");
        field(projectile, 0x28, u32, "target_serial", null);
        field(projectile, 0x2c, u32, "source_serial", null);
        field(projectile, 0x30, bool, "explode", "packet explode byte & 1");
        field(projectile, 0x31, bool, "return_to_source", "(packet explode byte >> 1) & 1");
        field(projectile, 0x32, u16, "explode_effect", "0xC7 only: 0 -> classic tile effect 0x36CB, else collection"
            + " id explode_effect - 1 (no 9501/9502 special case)");
        field(projectile, 0x34, u16, "explode_sound", null);
        field(projectile, 0x38, u32, "render_mode", "0xC0/0xC7 render mode (UOObject::SetRenderMode)");
        field(projectile, 0x3c, bool, "suppress_tileart_effects", "packet effect == 9502"
            + " (UOObject::SetSuppressTileartEffects)");
        StructureDataType tile = struct(TILE_DEFINITION, 0x24, "built by UOEffects::MakeTileEffect from a type 2/3"
            + " graphical effect record with effect 0 or 9502");
        field(tile, 0x0, voidPtr, "vftable", null);
        field(tile, 0x4, u32, "item_id", null);
        field(tile, 0x8, u32, "hue", "packet hue + 1, 0 when the packet hue is 0");
        field(tile, 0xc, i32, "x", "packet source x (u16)");
        field(tile, 0x10, i32, "y", "packet source y (u16)");
        field(tile, 0x14, i32, "z", "packet source z (i8)");
        field(tile, 0x18, i32, "duration_ticks", "packet duration byte; UOTileEffect::Update counts it down");
        field(tile, 0x1c, bool, "suppress_tileart_effects", "packet effect == 9502");
        field(tile, 0x1d, bool, "on_mobile", "record type 3: attached to the source object, lifetime 200");
        field(tile, 0x20, u32, "render_mode", null);
        StructureDataType collection = struct(COLLECTION, 0x14, "EffectDefinitionCollection.uop entry");
        field(collection, 0x0, voidPtr, "vftable", null);
        field(collection, 0x4, vector, "definitions", null);

        // Byte-packed file records of the fixed-size bodies (after the u32 type key).
        StructureDataType particleRecord = packed("EffectParticleRecord", "type 0 body, 28 bytes", new Object[][] {
            {u32, "name_index", "string dictionary index"}, {f32, "offset_x", null}, {f32, "offset_y", null},
            {f32, "camera_offset", null}, {f32, "scale", null}, {u8, "attach_mode", null}, {f32, "duration", null},
            {u8, "loop", "tested != 0"}, {u8, "field_3c", null}, {u8, "hued", "tested != 0"}});
        StructureDataType soundRecord = packed("EffectSoundRecord", "type 1 body", new Object[][] {
            {u32, "sound_id", null}, {f32, "value_08", null}});
        StructureDataType offsetRecord = packed("EffectPositionOffsetRecord", "type 7 body", new Object[][] {
            {new ArrayDataType(i8, directions * 2, 1, dtm), "xy", "8 x {i8 x, i8 y}"}});
        StructureDataType quakeRecord = packed("EffectEarthquakeRecord", "type 15 body", new Object[][] {
            {f64, "amplitude", null}, {f64, "duration", null}, {f64, "frequency", null}});

        for (StructureDataType t : List.of(string, vector, map, particle, sound, roundRobin, offset, wait, lod, flag,
                quake, randomEntry, random, filter, projectile, tile, collection, particleRecord, soundRecord,
                offsetRecord, quakeRecord)) {
            DataType existing = dtm.getDataType(CATEGORY, t.getName());
            String status;
            if (existing != null && existing.isEquivalent(t)) {
                status = "unchanged";
            } else if (apply) {
                dtm.addDataType(t, DataTypeConflictHandler.REPLACE_HANDLER);
                status = existing == null ? "created" : "updated";
                count("types " + status);
            } else {
                status = existing == null ? "would create" : "would update";
            }
            typeLines.add("| `" + CATEGORY + "/" + t.getName() + "` | " + hexShort(t.getLength()) + " | " + status + " |");
        }
    }

    private StructureDataType struct(String name, int size, String description) {
        StructureDataType s = new StructureDataType(CATEGORY, name, size, dtm);
        s.setDescription(description);
        return s;
    }

    private StructureDataType packed(String name, String description, Object[][] fields) {
        StructureDataType s = new StructureDataType(CATEGORY, name, 0, dtm);
        s.setDescription(description + "; byte packed, little endian");
        for (Object[] f : fields) {
            s.add((DataType) f[0], (String) f[1], (String) f[2]);
        }
        return s;
    }

    private void field(StructureDataType s, int offset, DataType type, String name, String comment) {
        s.replaceAtOffset(offset, type, type.getLength(), name, comment);
    }

    // ---- program queries ----

    private void loadBlocks() throws Exception {
        for (MemoryBlock b : memory.getBlocks()) {
            if (!b.isInitialized() || b.getSize() > Integer.MAX_VALUE) {
                continue;
            }
            Block block = new Block();
            block.start = b.getStart().getOffset();
            block.bytes = new byte[(int) b.getSize()];
            b.getBytes(b.getStart(), block.bytes);
            block.exec = b.isExecute();
            blocks.add(block);
        }
        for (Block block : blocks) {
            if (block.exec) {
                continue;
            }
            for (int i = 0; i + POINTER_SIZE <= block.bytes.length; i += POINTER_SIZE) {
                long value = dword(block.bytes, i);
                if (isExec(value)) {
                    codePointerCounts.merge(value, 1, Integer::sum);
                }
            }
        }
    }

    private boolean isExec(long address) {
        for (Block block : blocks) {
            if (block.exec && address >= block.start && address < block.start + block.bytes.length) {
                return true;
            }
        }
        return false;
    }

    private static long dword(byte[] bytes, int i) {
        long value = 0;
        for (int k = POINTER_SIZE - 1; k >= 0; k--) {
            value = (value << BITS_PER_BYTE) | (bytes[i + k] & BYTE_MASK);
        }
        return value;
    }

    private long dwordAt(long address) {
        for (Block block : blocks) {
            long index = address - block.start;
            if (index >= 0 && index + POINTER_SIZE <= block.bytes.length) {
                return dword(block.bytes, (int) index);
            }
        }
        return -1;
    }

    private List<Long> findBytes(byte[] pattern, boolean dataOnly) {
        List<Long> found = new ArrayList<>();
        for (Block block : blocks) {
            if (dataOnly && block.exec) {
                continue;
            }
            byte[] b = block.bytes;
            outer:
            for (int i = 0; i + pattern.length <= b.length; i++) {
                for (int k = 0; k < pattern.length; k++) {
                    if (b[i + k] != pattern[k]) {
                        continue outer;
                    }
                }
                found.add(block.start + i);
            }
        }
        return found;
    }

    private List<Long> findDwords(long value) {
        List<Long> found = new ArrayList<>();
        for (Block block : blocks) {
            if (block.exec) {
                continue;
            }
            for (int i = 0; i + POINTER_SIZE <= block.bytes.length; i += POINTER_SIZE) {
                if (dword(block.bytes, i) == value) {
                    found.add(block.start + i);
                }
            }
        }
        return found;
    }

    private long rttiVtable(String cls) {
        byte[] name = (RTTI_PREFIX + cls + RTTI_SUFFIX + "\0").getBytes(StandardCharsets.US_ASCII);
        for (long nameAt : findBytes(name, true)) {
            long descriptor = nameAt - TYPE_DESCRIPTOR_NAME_OFFSET;
            for (long ref : findDwords(descriptor)) {
                long col = ref - COL_TYPE_DESCRIPTOR_OFFSET;
                if (dwordAt(col) != 0 || dwordAt(col + COL_OFFSET_FIELD) != 0) {
                    continue;
                }
                List<Long> uses = findDwords(col);
                if (uses.size() == 1) {
                    return uses.get(0) + POINTER_SIZE;
                }
            }
        }
        return -1;
    }

    private Function slotFunction(long vt, int offset) {
        long target = dwordAt(vt + offset);
        return target < 0 ? null : functions.getFunctionAt(toAddr(target));
    }

    // Functions referencing the vftable other than its own slot functions (constructors, registration).
    private List<Function> vtableWriters(String cls) {
        long vt = vtables.getOrDefault(cls, -1L);
        List<Function> writers = new ArrayList<>();
        if (vt < 0) {
            return writers;
        }
        Set<Function> slots = new LinkedHashSet<>();
        for (int offset = 0; codePointerCounts.containsKey(dwordAt(vt + offset)); offset += POINTER_SIZE) {
            Function f = slotFunction(vt, offset);
            if (f != null) {
                slots.add(f);
            }
        }
        for (Reference r : getReferencesTo(toAddr(vt))) {
            Function f = getFunctionContaining(r.getFromAddress());
            if (f != null && !slots.contains(f) && !writers.contains(f)) {
                writers.add(f);
            }
        }
        return writers;
    }

    private List<Function> calls(Function f) {
        List<Function> out = new ArrayList<>();
        for (Instruction i : listing.getInstructions(f.getBody(), true)) {
            if (!i.getFlowType().isCall()) {
                continue;
            }
            Address[] flows = i.getFlows();
            if (flows.length == 0) {
                continue;
            }
            Function callee = functions.getFunctionAt(flows[0]);
            if (callee != null) {
                out.add(callee.isThunk() ? callee.getThunkedFunction(true) : callee);
            }
        }
        return out;
    }

    private static List<Function> distinct(List<Function> list) {
        return new ArrayList<>(new LinkedHashSet<>(list));
    }

    private List<Function> callers(Function f) {
        List<Function> out = new ArrayList<>(f.getCallingFunctions(monitor));
        out.sort((a, b) -> a.getEntryPoint().compareTo(b.getEntryPoint()));
        return out;
    }

    private boolean callsNamed(Function f, String name) {
        for (Function c : distinct(calls(f))) {
            if (c.getName().equals(name)) {
                return true;
            }
        }
        return false;
    }

    private Function single(List<Function> list, String what) {
        if (list.size() == 1) {
            return list.get(0);
        }
        anomalies.add(what + ": " + list.size() + " candidates " + names(list));
        return null;
    }

    // Vtable offsets of virtual calls: MOV reg, [object]; optional ADD reg, imm; then CALL [reg + disp], or
    // MOV reg2, [reg + disp]; CALL reg2.
    private Set<Long> virtualCallOffsets(Function f) {
        Set<Long> offsets = new LinkedHashSet<>();
        Map<Register, Long> added = new HashMap<>();
        Map<Register, Long> loadedSlots = new HashMap<>();
        for (Instruction i : listing.getInstructions(f.getBody(), true)) {
            String m = i.getMnemonicString();
            Object[] dst = i.getNumOperands() > 0 ? i.getOpObjects(0) : new Object[0];
            Register reg = dst.length == 1 && dst[0] instanceof Register r ? r : null;
            if (m.equals("MOV") && reg != null) {
                Long slot = null;
                if (i.getNumOperands() > 1 && i.getDefaultOperandRepresentation(1).contains("[")) {
                    Register base = null;
                    long displacement = 0;
                    for (Object o : i.getOpObjects(1)) {
                        if (o instanceof Register r) {
                            base = r;
                        } else if (o instanceof Scalar s) {
                            displacement = s.getUnsignedValue();
                        }
                    }
                    if (base != null && added.containsKey(base)) {
                        slot = added.get(base) + displacement;
                    }
                }
                if (slot == null) {
                    loadedSlots.remove(reg);
                } else {
                    loadedSlots.put(reg, slot);
                }
                added.put(reg, 0L);
            } else if (i.getFlowType().isCall() && i.getFlows().length == 0 && reg != null
                    && !i.getDefaultOperandRepresentation(0).contains("[") && loadedSlots.containsKey(reg)) {
                offsets.add(loadedSlots.get(reg));
            } else if (m.equals("ADD") && reg != null && added.containsKey(reg) && i.getOpObjects(1).length == 1
                    && i.getOpObjects(1)[0] instanceof Scalar s) {
                added.put(reg, added.get(reg) + s.getUnsignedValue());
            } else if (i.getFlowType().isCall() && i.getFlows().length == 0
                    && i.getDefaultOperandRepresentation(0).contains("[")) {
                Register base = null;
                long displacement = 0;
                for (Object o : dst) {
                    if (o instanceof Register r) {
                        base = r;
                    } else if (o instanceof Scalar s) {
                        displacement = s.getUnsignedValue();
                    }
                }
                if (base != null) {
                    offsets.add(added.getOrDefault(base, 0L) + displacement);
                }
            }
        }
        return offsets;
    }

    private static boolean referencesAddress(Instruction i, long address) {
        for (int op = 0; op < i.getNumOperands(); op++) {
            for (Object o : i.getOpObjects(op)) {
                if (o instanceof Scalar s && (s.getUnsignedValue() & DWORD_MASK) == address) {
                    return true;
                }
                if (o instanceof Address a && a.getOffset() == address) {
                    return true;
                }
            }
        }
        return false;
    }

    private static long bodySize(Function f) {
        return f.getBody().getNumAddresses();
    }

    // ---- report ----

    private void report(PrintWriter out, boolean apply) {
        int probable = 0;
        for (Proposal p : proposals.values()) {
            probable += p.probable ? 1 : 0;
        }
        out.println("# effect code names");
        out.println();
        out.println("Program: " + currentProgram.getName());
        out.println("SHA-256: " + currentProgram.getExecutableSHA256());
        out.println("Mode: " + (apply ? "analysis and " + APPLY_FLAG : "analysis only"));
        out.println();
        out.println("| Item | Count |");
        out.println("|---|---:|");
        out.println("| Function names proposed | " + proposals.size() + " |");
        out.println("| of which probable | " + probable + " |");
        out.println("| Slots not named (shared code) | " + shared.size() + " |");
        out.println("| Global labels | " + labels.size() + " |");
        out.println("| Types | " + typeLines.size() + " |");
        out.println("| Kept non-default names | " + kept.size() + " |");
        out.println("| Anomalies | " + anomalies.size() + " |");
        out.println();
        out.println("Changes: " + (changes.isEmpty() ? "none" : changes.toString()));
        out.println();
        out.println("## Anchors");
        out.println();
        for (String a : anchors) {
            out.println("- " + a);
        }
        out.println();
        out.println("## Effect factories");
        out.println();
        out.println("| Key | Factory | vftable | CanHandle | Create | Definition | Definition vftable | CanHandle/GetType key |");
        out.println("|---:|---|---|---|---|---|---|---|");
        factoryRows.forEach(out::println);
        out.println();
        out.println("## Functions");
        out.println();
        out.println("| Address | Name | Status | Evidence |");
        out.println("|---|---|---|---|");
        for (Proposal p : proposals.values()) {
            out.println("| " + hex(p.address) + " | `" + p.path + "`" + (p.probable ? PROBABLE : "") + " | "
                + p.status + " | " + p.evidence.replace("|", "\\|") + " |");
        }
        out.println();
        out.println("## Global labels");
        out.println();
        out.println("| Address | Label | Status | Evidence |");
        out.println("|---|---|---|---|");
        labels.forEach(out::println);
        out.println();
        out.println("## Types");
        out.println();
        out.println("| Type | Size | Status |");
        out.println("|---|---:|---|");
        typeLines.forEach(out::println);
        section(out, "Other users of the readers (not renamed)", otherUsers);
        section(out, "Slots not named (identical-code folding)", shared);
        section(out, "Kept names (not renamed)", kept);
        section(out, "Anomalies", anomalies);
    }

    private static void section(PrintWriter out, String title, List<String> lines) {
        out.println();
        out.println("## " + title);
        out.println();
        if (lines.isEmpty()) {
            out.println("None.");
        }
        for (String line : lines) {
            out.println("- " + line);
        }
    }

    private static String names(List<Function> list) {
        List<String> out = new ArrayList<>();
        for (Function f : list) {
            out.add(f.getName(true) + "@" + f.getEntryPoint());
        }
        return out.toString();
    }

    private static String entry(Function f) {
        return f == null ? "not found" : "`" + f.getName(true) + "` @" + f.getEntryPoint();
    }

    private static String hex(long value) {
        return String.format("0x%08x", value);
    }

    private static String hexShort(long value) {
        return "0x" + Long.toString(value, HEX_RADIX);
    }

    private static String hexOrNone(long value) {
        return value < 0 ? "not found" : hex(value);
    }
}
