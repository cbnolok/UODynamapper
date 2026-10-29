// Name the tileart.uop load path (asset manager, UOP archive, string dictionary), the
// UOObjectDefinition record parser and accessors, and code using decoded tile definitions;
// with --apply, rename DEFAULT functions, add the UO_TILEART tag, plate comments, /UOTileArt
// types, parameter types and global labels.
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
import ghidra.program.model.data.DataUtilities;
import ghidra.program.model.data.DataUtilities.ClearDataMode;
import ghidra.program.model.data.FloatDataType;
import ghidra.program.model.data.IntegerDataType;
import ghidra.program.model.data.PointerDataType;
import ghidra.program.model.data.QWordDataType;
import ghidra.program.model.data.StructureDataType;
import ghidra.program.model.data.UnsignedIntegerDataType;
import ghidra.program.model.data.UnsignedShortDataType;
import ghidra.program.model.data.VoidDataType;
import ghidra.program.model.lang.Register;
import ghidra.program.model.listing.Function;
import ghidra.program.model.listing.FunctionManager;
import ghidra.program.model.listing.FunctionTag;
import ghidra.program.model.listing.Data;
import ghidra.program.model.listing.Instruction;
import ghidra.program.model.listing.Listing;
import ghidra.program.model.mem.Memory;
import ghidra.program.model.mem.MemoryBlock;
import ghidra.program.model.scalar.Scalar;
import ghidra.program.model.symbol.Namespace;
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

public class NameTileartCode extends GhidraScript {
    private static final String APPLY_FLAG = "--apply";
    private static final String TAG = "UO_TILEART";
    private static final String TAG_COMMENT = "tileart.uop loading, UOObjectDefinition parsing and tile definition use";
    private static final String COMMENT_PREFIX = "NameTileartCode: ";
    private static final String PROBABLE = " (probable)";
    private static final String SCOPE = "::";
    private static final CategoryPath CATEGORY = new CategoryPath("/UOTileArt");

    // Anchors: strings, imports and named functions.
    private static final String TILEART_UOP = "tileart.uop";
    private static final String STRING_DICTIONARY_BIN = "Build/StringDictionary/string_dictionary.bin";
    private static final String RUNTIME_CLIENT = "Runtime\\client";
    private static final String TILEART_FORMAT = "%08d_TileArt";
    private static final String LEGACY_TILEART_FORMAT = "%08d_LegacyTileArt";
    private static final String TILEART_LEGACY_FOLDER = "TileArtLegacy\\";
    private static final String ZLIB_HEADER_ERROR = "incorrect header check";
    private static final String VIRTUAL_FILE_OPEN = "UOVirtualFileOpen";
    private static final String MEMCPY = "_memcpy";
    private static final String CREATE_FILE = "CreateFileA";

    // RTTI classes.
    private static final String OBJECT_DEFINITION = "UOObjectDefinition";
    private static final String OBJECT_DEFINITION_BASE = "UOObjectDefinitionBase";
    private static final String OBJECT_FACTORY = "UOObjectDefinitionBinaryFactory";
    private static final String BASE_FACTORY = "UOObjectDefinitionBinaryBaseFactory";
    private static final String SHADER_DEFINITION = "UOShaderDefinition";
    private static final String FILTER_BODY = "AppearanceFilterByOwnerBodyType";
    private static final String FILTER_GENDER = "AppearanceFilterByOwnerGender";
    private static final String FILTER_ANIMATION_BODY = "AnimationAppearanceFilterByOwnerBodyType";
    private static final String SPRITE_LOCATION = "SpriteTextureFileLocation";
    private static final String SIMPLE_LOCATION = "SimpleTextureFileLocation";
    private static final String MYP_INPUT_FILE = "MythMypInputFile";
    private static final String MEMORY_INPUT_FILE = "MythMemoryInputFile";
    // World objects and the components built from their tile definition.
    private static final String WORLD_OBJECT = "UOObject";
    private static final String COMPONENT_SPRITE = "UOObjectComponentSprite";
    private static final String COMPONENT_LIGHTING = "UOObjectComponentLighting";
    private static final String COMPONENT_EFFECTS = "UOObjectComponentEffects";

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

    // Asset type key of tile definitions in the asset registry (GetAsset argument).
    private static final long TILEART_ASSET_TYPE = 0x06000000L;
    // UseNewArt (flags2 bit 19), pushed before the HasFlags2 call in the sprite descriptor builder.
    private static final long USE_NEW_ART_FLAG = 0x80000L;
    private static final long FLOAT_ONE_BITS = 0x3f800000L;
    // zlib/lookup3 seed added to the key length in hashlittle2.
    private static final long LOOKUP3_SEED = 0xdeadbeefL;
    private static final int RECORD_VERSION = 4;
    // A thin wrapper or getter: functions at most this large.
    private static final int SMALL_FUNCTION = 0x40;
    // Inserter helpers of the sub-parsers have few callers; std:: helpers have many.
    private static final int MAX_HELPER_CALLERS = 4;
    private static final int IMPORT_SEARCH_DEPTH = 2;
    private static final int HASH_SEARCH_DEPTH = 3;
    private static final int TEXTURE_BLOCKS = 4;
    private static final int REPORT_STRINGS = 4;
    private static final String SHADER_PTR_TYPE = "UOShaderDefinitionSharedPtr";

    // Record parser sub-functions in call order (each calls the stream read helper directly).
    private static final String[] SUB_PARSERS = {
        "UOTileArt::ParseProperties1|props1: u8 count, {u8 key, u32 value} into the map at +0xc8",
        "UOTileArt::ParseProperties2|props2: u8 count, {u8 key, u32 value} into the map at +0xd4",
        "UOTileArt::ParseStackAliases|u32 count, {u32 amount, u32 tile id} into the lazily allocated map at +0xe0",
        "UOTileArt::ParseAppearanceFilters|u32 count, {i8 subtype 0/1/2, filter data}; other subtypes are an error",
        "UOTileArt::ParseSitting|u8 flag (0/1, else error); 1: six u32 into a 0x18 struct set via vtable +0x150",
        "UOTileArt::ParseRadarColor|four u8 r,g,b,a set via vtable +0xe0 (SetRadarColor)",
        "UOTileArt::ParseShaderBlock|u8 present; u8 field04, u32 shader name index, u8 item count, items, u32 n + i32 Technique, u32 n + f32 Tweaks",
        "UOTileArt::ParseEffects|u8 count of effect definitions, each parsed by the effect factory list",
    };
    private static final int SUB_PROPS1 = 0;
    private static final int SUB_PROPS2 = 1;
    private static final int SUB_STACK = 2;
    private static final int SUB_APPEARANCE = 3;
    private static final int SUB_SHADER = 6;
    private static final int SUB_EFFECTS = 7;

    // Tile id used when an object has no definition yet (cached-definition wrapper).
    private static final long FALLBACK_TILE_ID = 99999999L;
    // Paperdoll gump id offsets added to the appearance id (male / female).
    private static final long PAPERDOLL_GUMP_MALE = 50000L;
    private static final long PAPERDOLL_GUMP_FEMALE = 60000L;
    // UOObjectDefinition slots read only when a world object is built from its definition.
    private static final Set<Long> OBJECT_INIT_SLOTS = Set.of(0xb4L, 0xb8L);
    // Component Initialize(object, objectRef, definition, definitionRef) is vtable slot +4.
    private static final int COMPONENT_INITIALIZE_SLOT = 0x4;
    // UOObjectDefinition slots each component Initialize must call to be accepted.
    private static final Set<Long> SPRITE_INIT_SLOTS = Set.of(0x88L);
    private static final Set<Long> LIGHTING_INIT_SLOTS = Set.of(0x40L, 0x48L, 0x4cL);
    private static final long SLOT_GET_SITTING_DIRECTIONS = 0xe8L;
    // HasProperty2 / GetProperty2: only the paperdoll image builder calls both.
    private static final Set<Long> PROPERTY2_SLOTS = Set.of(0x6cL, 0x70L);
    private static final int WORLD_OBJECT_SET_DIRECTION = 0x44;
    // Virtual-call offsets found in the house placement check (bits 1, 2, 5; the bit 3 load jumps to the bit 2 CALL).
    private static final Set<Long> HOUSE_PLACEMENT_SLOTS = Set.of(0x100L, 0x104L, 0x110L);

    // UOObject vtable: world object state copied from its definition.
    private static final Slot[] WORLD_OBJECT_SLOTS = {
        new Slot(WORLD_OBJECT_SET_DIRECTION, "SetDirection", "reads the definition sitting direction mask"
            + " (GetSittingDirections) and snaps the requested direction to the nearest allowed one before"
            + " storing it at +0x2c", true),
        new Slot(0x6c, "SetBodyType", "stores +0x50; UOObject::InitFromDefinition passes"
            + " UOObjectDefinition::GetBodyType", false),
        new Slot(0x70, "GetBodyType", "returns +0x50", false),
        new Slot(0x74, "SetAlternateAnimation", "stores byte +0x54; UOObject::InitFromDefinition passes"
            + " UOObjectDefinitionBase::GetAlternateAnimation (tileart byte +0xe8)", false),
        new Slot(0x78, "GetAlternateAnimation", "returns byte +0x54; read on the item in the wearer's layer 2 to pick"
            + " the alternate entry of the animation action map", false),
        new Slot(0xb0, "GetFlags1", "returns the u64 at +0x38, copied from UOObjectDefinition::GetFlags1", true),
    };

    private record Slot(int offset, String name, String evidence, boolean probable) {
    }

    // UOObjectDefinition vtable (85 slots); a slot shared with UOObjectDefinitionBase is named there.
    private static final Slot[] OBJECT_SLOTS = {
        new Slot(0x0, "GetMemorySize", "returns base size + 0x20 + 8 per effect pointer", false),
        new Slot(0x4, "ScalarDeletingDestructor", "restores the vftable, frees through the allocator when flag&1", false),
        new Slot(0xc, "GetName", "returns c_str of the std::string at +0x4 (record name string index)", false),
        new Slot(0x10, "GetTileId", "returns +0x20 (record tile_id)", false),
        new Slot(0x14, "GetEnhancedImageWindow", "copies 6 ints from +0x24 (EC image window)", false),
        new Slot(0x18, "GetLegacyImageWindow", "copies 6 ints from +0x3c (legacy image window)", false),
        new Slot(0x1c, "HasWorldArt", "returns byte +0x54; set exactly when texture block 0 (WorldArt) is present", false),
        new Slot(0x20, "UsesEnhancedTileArt", "returns byte +0x55; selects block 2 (TileArtEnhanced) in the sprite descriptor builder", false),
        new Slot(0x24, "GetWorldArtShaderName", "name of shader block 0 (+0x9c)", false),
        new Slot(0x28, "GetLegacyShaderName", "name of shader block 1 (+0xa4)", false),
        new Slot(0x2c, "GetWorldArtScale", "returns float +0x58 (default 1.0); scale of the WorldArt sprite", false),
        new Slot(0x30, "GetLegacyArtScale", "returns float +0x5c (default 1.5); scale of the legacy/enhanced tile art", false),
        new Slot(0x34, "GetField60", "returns +0x60 (0 in all records)", false),
        new Slot(0x38, "HasLegacyArtData", "block 1 differs from the default or +0x5c != 1.5 or +0x60 set", true),
        new Slot(0x3c, "GetLightShaderName", "name of shader block 3 (+0xb4, *_light_window.tga)", false),
        new Slot(0x40, "GetLightScale", "returns float +0x6c (default 1.0, 2.0 on windows);"
            + " UOObjectComponentLighting::Initialize stores it as the light sprite scale (descriptor"
            + " +0x44)", false),
        new Slot(0x44, "GetLightField70", "returns +0x70 (0 in all records); no caller on a definition"
            + " was found", true),
        new Slot(0x48, "GetLightOffsetX", "returns float +0x74 (non-zero only with block 3);"
            + " UOObjectComponentLighting::Initialize rotates (0, y, x) by the view matrix and adds it"
            + " to the light sprite position", false),
        new Slot(0x4c, "GetLightOffsetY", "returns float +0x78 (non-zero only with block 3); second"
            + " component of the light sprite offset", false),
        new Slot(0x50, "HasLightData", "block 3 present or +0x6c..+0x78 differ from the defaults", true),
        new Slot(0x54, "HasFlags1", "(lo, hi) mask test against the u64 at +0x80", false),
        new Slot(0x58, "GetFlags1", "returns the u64 at +0x80", false),
        new Slot(0x5c, "HasFlags2", "(lo, hi) mask test against the u64 at +0x88", false),
        new Slot(0x60, "GetFlags2", "returns the u64 at +0x88", false),
        new Slot(0x64, "HasProperty1", "key lookup in the props1 map at +0xc8", false),
        new Slot(0x68, "GetProperty1", "value of a props1 key (0 when missing)", false),
        new Slot(0x6c, "HasProperty2", "key lookup in the props2 map at +0xd4", false),
        new Slot(0x70, "GetProperty2", "value of a props2 key (0 when missing)", false),
        new Slot(0x74, "HasFlags1OrProperties1", "flags1 non-zero or props1 non-empty", false),
        new Slot(0x78, "GetOldId", "returns +0x64 (default 0xffffff)", false),
        new Slot(0x7c, "GetRotation", "returns float +0x68 (0/90/180/270), used when flags1 bit 34 is set", true),
        new Slot(0x80, "GetPrimaryTextureLocation", "block 0 if HasWorldArt else block 1; checks UOWaterShader; builds a SpriteTextureFileLocation", false),
        new Slot(0x84, "GetPrimaryArtScale", "HasWorldArt ? +0x58 : +0x5c", false),
        new Slot(0x88, "GetFacing", "returns +0x90 (default 1); UOObjectComponentSprite::Initialize"
            + " stores it at component +0x2c", false),
        new Slot(0x8c, "GetEffectCount", "size of the shared_ptr vector at +0xf0", false),
        new Slot(0x90, "GetEffect", "shared_ptr copy of effect i", false),
        new Slot(0x94, "GetWorldArtShader", "shared_ptr at +0x9c (block 0)", false),
        new Slot(0x98, "GetLegacyShader", "shared_ptr at +0xa4 (block 1)", false),
        new Slot(0x9c, "GetEnhancedShader", "shared_ptr at +0xac (block 2)", false),
        new Slot(0xa0, "GetLightShader", "shared_ptr at +0xb4 (block 3)", false),
        new Slot(0xa4, "SetWorldArtShader", "parser stores block 0", false),
        new Slot(0xa8, "SetLegacyShader", "parser stores block 1", false),
        new Slot(0xac, "SetEnhancedShader", "parser stores block 2", false),
        new Slot(0xb0, "SetLightShader", "parser stores block 3", false),
        new Slot(0xb4, "GetBodyType", "returns +0xe4 (default 400 = human male body; 666 on gargoyle"
            + " items); copied to UOObject::SetBodyType; equipping refuses an appearance whose body type"
            + " differs from the wearer's body", false),
        new Slot(0xb8, "GetAlternateAnimation", "returns byte +0xe8; copied to UOObject::SetAlternateAnimation; for"
            + " the item in layer 2 it selects the alternate entry of the wearer's animation action map", false),
        new Slot(0xbc, "GetAppearance", "prop1 key 8 or tile id, then non-animation appearance filters for the owner body", false),
        new Slot(0xc0, "GetAnimationAppearance", "applies appearance filters of type 2", false),
        new Slot(0xc4, "GetGraphicForAmount", "last stack alias whose amount <= argument, default tile id", false),
        new Slot(0xc8, "HasRace", "props1 key 9 present", false),
        new Slot(0xcc, "HasGender", "props1 key 10 present", false),
        new Slot(0xd0, "GetRace", "props1 key 9, default 1", false),
        new Slot(0xd4, "GetGender", "props1 key 10, default 0", false),
        new Slot(0xd8, "HasRadarColor", "returns byte +0x98", false),
        new Slot(0xdc, "GetRadarColor", "returns the rgba at +0x94", false),
        new Slot(0xe0, "SetRadarColor", "stores rgba at +0x94, sets +0x98", false),
        new Slot(0xe4, "HasSitting", "sitting pointer +0xc0 non-null", false),
        new Slot(0xe8, "GetSittingDirections", "sitting field 0 (+0x0): direction mask 1 = north (dir"
            + " 0), 2 = east (dir 2), 4 = south (dir 4), 8 = west (dir 6); UOObject::SetDirection snaps"
            + " to the nearest allowed direction", false),
        new Slot(0xec, "GetSeatOffsetX", "sitting field 1: seated mobile sprite x offset (negated)", false),
        new Slot(0xf0, "GetSeatOffsetY", "sitting field 2: seated mobile sprite y offset (negated)", false),
        new Slot(0xf4, "GetSittingWindowShift", "sitting field 3: shifts the split column of the"
            + " chair-back overlay window for directions 1/8", false),
        new Slot(0xf8, "GetSittingField10", "sitting field 4; no caller on a definition was found", false),
        new Slot(0xfc, "GetSittingField14", "sitting field 5; no caller on a definition was found", false),
        new Slot(0x100, "HasHouseJoinBit1", "bit 1 of +0x7c; house customization: placeable on the"
            + " corner cell (x = 0, y = 0)", false),
        new Slot(0x104, "HasHouseJoinBit2", "bit 2 of +0x7c; house customization: placeable on the x = 0 edge", false),
        new Slot(0x108, "HasHouseJoinBit3", "bit 3 of +0x7c; house customization: placeable on the y = 0 edge", false),
        new Slot(0x10c, "HasHouseJoinBit4", "bit 4 of +0x7c; no caller on a definition was found", false),
        new Slot(0x110, "HasHouseJoinBit5", "bit 5 of +0x7c; house customization: placement also checks"
            + " the cell below", false),
        new Slot(0x114, "ConnectsWestA", "bit 6 of +0x7c; paired with bit 8 of the west neighbour", true),
        new Slot(0x118, "ConnectsWestB", "bit 7 of +0x7c; paired with bit 9 of the west neighbour", true),
        new Slot(0x11c, "ConnectsEastA", "bit 8 of +0x7c", true),
        new Slot(0x120, "ConnectsEastB", "bit 9 of +0x7c", true),
        new Slot(0x124, "ConnectsSouthA", "bit 12 of +0x7c", true),
        new Slot(0x128, "ConnectsSouthB", "bit 13 of +0x7c", true),
        new Slot(0x12c, "ConnectsNorthA", "bit 10 of +0x7c; paired with bit 12 of the north neighbour", true),
        new Slot(0x130, "ConnectsNorthB", "bit 11 of +0x7c; paired with bit 13 of the north neighbour", true),
        new Slot(0x134, "HasHouseJoinBit14", "bit 14 of +0x7c; house customization: marks the cell and"
            + " its +x/+y neighbours", false),
        new Slot(0x138, "HasHouseJoinBit15", "bit 15 of +0x7c; set only on roof tiles; no caller on a"
            + " definition was found", false),
        new Slot(0x13c, "GetSpriteDescriptor", "requests the definition and the cached sprite descriptor (legacy flag from settings +0x54)", false),
        new Slot(0x140, "HasFlags2OrProperties2", "flags2 non-zero or props2 non-empty", false),
        new Slot(0x144, "AddEffect", "push_back into the effect vector at +0xf0", false),
        new Slot(0x148, "GetHouseJoinBits", "returns +0x7c", false),
        new Slot(0x14c, "SetHouseJoinBits", "parser stores the record u32 at +0x7c", false),
        new Slot(0x150, "SetSitting", "stores the sitting shared_ptr at +0xc0", false),
    };

    private static final Slot[] SHADER_SLOTS = {
        new Slot(0x0, "ScalarDeletingDestructor", "restores the vftable, frees when flag&1", false),
        new Slot(0x4, "GetTextureCount", "size of the shared_ptr vector at +0x24", false),
        new Slot(0x8, "GetTexture", "shared_ptr copy of texture item i", false),
        new Slot(0xc, "GetField04", "returns +0x4 (record byte, 0 in all records)", false),
        new Slot(0x10, "GetShaderName", "copies the std::string at +0x8", false),
        new Slot(0x14, "AddTexture", "push_back of a texture item", false),
        new Slot(0x18, "SetField04", "stores +0x4", false),
        new Slot(0x1c, "SetShaderName", "assigns the std::string at +0x8", false),
        new Slot(0x24, "GetTechniqueCount", "returns 4", false),
        new Slot(0x28, "GetTweakCount", "returns 4", false),
        new Slot(0x2c, "GetTechniqueValues", "returns &int[4] at +0x34", false),
        new Slot(0x30, "GetTweakValues", "returns &float[4] at +0x44", false),
        new Slot(0x34, "SetTechniqueValue", "int[4] at +0x34 [i] = v", false),
        new Slot(0x38, "SetTweakValue", "float[4] at +0x44 [i] = v", false),
    };

    private static final Slot[] FACTORY_SLOTS = {
        new Slot(0x4, "Create", "asset factory entry shared with UOObjectDefinitionBinaryFactory: NewInstance (vtable +0x10), then the record parser; error \"Trouble generating object definition asset\"", false),
        new Slot(0x10, "NewInstance", "allocates 0x100 bytes, base ctor, sets the UOObjectDefinition vftable", false),
    };

    private static final Slot[] BASE_FACTORY_SLOTS = {
        new Slot(0x10, "NewInstance", "allocates 0xf0 bytes and runs the UOObjectDefinitionBase constructor (no effect vector)", false),
    };

    // UOObjectDefinition slots used to find callers.
    private static final int SLOT_GET_SPRITE_DESCRIPTOR = 0x13c;
    private static final long SLOT_GET_GRAPHIC_FOR_AMOUNT = 0xc4L;
    // ConnectsWestA, ConnectsEastA, ConnectsNorthA: all tested only by the house tile connection check.
    private static final Set<Long> TILES_CONNECT_SLOTS = Set.of(0x114L, 0x11cL, 0x12cL);

    // UOObjectDefinitionBase offsets of shader blocks 1-3 -> label of the default shared_ptr stored there.
    private static final Map<Long, String> DEFAULT_SHADER_LABELS = Map.of(
        0xa4L, "g_UOTileArtDefaultLegacyShader",
        0xacL, "g_UOTileArtDefaultEnhancedShader",
        0xb4L, "g_UOTileArtDefaultLightShader");

    private static final int FILTER_APPLY_SLOT = 0x4;
    private static final int FILTER_TYPE_SLOT = 0xc;

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

    private static final class Usage {
        Function function;
        String via;
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
    private final Map<Long, Proposal> proposals = new TreeMap<>();
    private final List<String> anomalies = new ArrayList<>();
    private final List<String> kept = new ArrayList<>();
    // Names this script applied earlier and now replaces.
    private final List<String> corrected = new ArrayList<>();
    private final List<String> shared = new ArrayList<>();
    private final List<String> labels = new ArrayList<>();
    private final List<String> typeLines = new ArrayList<>();
    private final List<String> anchors = new ArrayList<>();
    private final Set<Long> defaultShaderGlobals = new LinkedHashSet<>();
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

        for (String cls : List.of(OBJECT_DEFINITION, OBJECT_DEFINITION_BASE, OBJECT_FACTORY, BASE_FACTORY, SHADER_DEFINITION,
                FILTER_BODY, FILTER_GENDER, FILTER_ANIMATION_BODY, SPRITE_LOCATION, SIMPLE_LOCATION,
                MYP_INPUT_FILE, MEMORY_INPUT_FILE, WORLD_OBJECT, COMPONENT_SPRITE, COMPONENT_LIGHTING,
                COMPONENT_EFFECTS)) {
            long vt = rttiVtable(cls);
            vtables.put(cls, vt);
            anchors.add("RTTI `" + cls + "` vftable " + hexOrNone(vt));
        }

        // 1. Asset manager and string dictionary.
        Function dictionaryLoad = single(functionsReferencingString(STRING_DICTIONARY_BIN), STRING_DICTIONARY_BIN);
        propose(dictionaryLoad, "UOStringDictionary::Load", "references \"" + STRING_DICTIONARY_BIN
            + "\"; reads u64 stamp, u32 count, u16 max length, then count x {u16 length, bytes} pushed into the"
            + " std::vector<std::string> at manager+0x150 (string 0 is empty)", false);
        Function streamRead = null;
        if (dictionaryLoad != null) {
            for (Function callee : distinct(calls(dictionaryLoad))) {
                if (bodySize(callee) <= SMALL_FUNCTION && callsNamed(callee, MEMCPY)) {
                    streamRead = callee;
                }
            }
            Function init = single(callers(dictionaryLoad), "caller of UOStringDictionary::Load");
            propose(init, "UOAssets::Initialize", "only caller of UOStringDictionary::Load; creates the asset"
                + " manager, mounts the UOP packages, reads AssetLoaderConfig and preloads asset types", true);
        }
        propose(streamRead, "UOAssets::StreamRead", "small callee of UOStringDictionary::Load calling _memcpy;"
            + " ECX = stream {+4 data, +8 size}, ESI = position, EDI = size; error on overrun", false);

        Function mount = single(functionsReferencingString(TILEART_UOP), TILEART_UOP);
        propose(mount, "UOAssets::MountPackages", "references \"tileart.uop\", \"string_dictionary.uop\" and the"
            + " other package names; mounts each through UOP::MountArchive", true);
        if (mount != null) {
            Function archive = null;
            for (Function callee : distinct(calls(mount))) {
                if (reachesImport(callee, CREATE_FILE, IMPORT_SEARCH_DEPTH)) {
                    archive = archive == null ? callee : archive;
                }
            }
            propose(archive, "UOP::MountArchive", "callee of UOAssets::MountPackages per package name that opens the"
                + " file (CreateFileA/GetFileSizeEx) and reads the UOP table", true);
        }

        // 2. UOP virtual files: hash, lookup, read and decompression.
        Function vopen = functionNamed(VIRTUAL_FILE_OPEN);
        anchors.add("`" + VIRTUAL_FILE_OPEN + "` " + (vopen == null ? "missing" : vopen.getEntryPoint()));
        Function hashFileName = null;
        if (vopen != null) {
            Function hash = null;
            for (Function f : calleeClosure(vopen, HASH_SEARCH_DEPTH)) {
                if (hasScalar(f, LOOKUP3_SEED)) {
                    hash = f;
                }
            }
            propose(hash, "UOP::HashLittle2", "Bob Jenkins lookup3 hashlittle2: seed 0xdeadbeef + length,"
                + " 12-byte blocks; computes the 64-bit UOP entry hash", false);
            for (Function callee : distinct(calls(vopen))) {
                for (Function inner : distinct(calls(callee))) {
                    if (hash != null && distinct(calls(inner)).contains(hash)) {
                        hashFileName = callee;
                        propose(inner, "UOP::HashString", "strlen of the key, then UOP::HashLittle2(key, len, &lo, &hi)", false);
                    }
                }
            }
            propose(hashFileName, "UOP::HashFileName", "callee of UOVirtualFileOpen: lowercases the path"
                + " (A-Z + 0x20) and returns the 64-bit lookup3 hash", false);
            if (hashFileName != null) {
                Set<Function> hashCallers = new LinkedHashSet<>(callers(hashFileName));
                for (Function callee : distinct(calls(vopen))) {
                    if (!callee.equals(hashFileName) && bodySize(callee) <= SMALL_FUNCTION
                            && new LinkedHashSet<>(callers(callee)).equals(hashCallers)) {
                        propose(callee, "UOP::FindEntryIndex", "called with the hash by every caller of"
                            + " UOP::HashFileName; returns the entry index or -1", false);
                        Function lookup = single(distinct(calls(callee)), "callee of UOP::FindEntryIndex");
                        propose(lookup, "UOP::LookupHash", "open-addressing table of {u32 lo, u32 hi, u32 index}"
                            + " (12 bytes), mask probe", false);
                    }
                }
            }
        }
        // zlib inflate -> uncompress -> small UOP wrapper -> entry reader (memcpy for stored entries). FreeType's
        // private zlib copy also references the string; only the chain whose reader calls two entry size
        // getters built on the entry copy function is the UOP one.
        Function inflate = null;
        for (Function f : functionsReferencingString(ZLIB_HEADER_ERROR)) {
            for (Function c : callers(f)) {
                for (Function wrapper : callers(c)) {
                    if (bodySize(wrapper) > SMALL_FUNCTION) {
                        continue;
                    }
                    for (Function reader : callers(wrapper)) {
                        List<Function> readerCalls = distinct(calls(reader));
                        if (!callsNamed(reader, MEMCPY) || readerCalls.isEmpty()) {
                            continue;
                        }
                        Function info = readerCalls.get(0);
                        List<Function> sizes = new ArrayList<>();
                        for (Function g : readerCalls) {
                            if (!g.equals(info) && distinct(calls(g)).contains(info)) {
                                sizes.add(g);
                            }
                        }
                        if (sizes.size() != 2) {
                            continue;
                        }
                        inflate = f;
                        propose(wrapper, "UOP::ZlibUncompress", "forwards to zlib uncompress " + c.getEntryPoint()
                            + " (inflate " + f.getEntryPoint() + " references \"incorrect header check\")", false);
                        propose(reader, "UOP::ReadEntryData", "reads the stored bytes via vtable +0x10;"
                            + " entry flag 1 -> UOP::ZlibUncompress, 0 -> memcpy", true);
                        propose(info, "UOP::GetEntryInfo", "copies the 0x22-byte UOP entry {i64 offset,"
                            + " u32 header, u32 compressed, u32 decompressed, u64 hash, u32 adler, u16 flag}", true);
                        propose(sizes.get(0), "UOP::GetEntryCompressedSize", "entry +0xc; sizes the"
                            + " read buffer in UOP::ReadEntryData", true);
                        propose(sizes.get(1), "UOP::GetEntryDecompressedSize", "entry +0x10; size of"
                            + " the output buffer", true);
                    }
                }
            }
        }
        anchors.add("zlib inflate (not named here): " + (inflate == null ? "missing" : inflate.getEntryPoint()));
        proposeVtableWriter(MYP_INPUT_FILE, "MythMypInputFile::MythMypInputFile", "UOP entry input file: entry"
            + " decompressed size, then MythMemoryInputFile base over the decompressed buffer", true);
        // MythMemoryInputFile has several constructors; name the one the UOP entry file calls.
        List<Function> mypCtors = vtableWriters(MYP_INPUT_FILE);
        List<Function> memoryCtors = new ArrayList<>();
        for (Function ctor : vtableWriters(MEMORY_INPUT_FILE)) {
            if (mypCtors.size() == 1 && distinct(calls(mypCtors.get(0))).contains(ctor)) {
                memoryCtors.add(ctor);
            }
        }
        propose(single(memoryCtors, "MythMemoryInputFile constructor called by MythMypInputFile"),
            "MythMemoryInputFile::MythMemoryInputFile", "writes the MythMemoryInputFile vftable, called by the"
            + " MythMypInputFile constructor: memory input file over the decompressed entry buffer", true);

        // 3. Asset manager lookup: GetObjectDefinition pushes the tile art type key.
        List<Function> typePushers = functionsUsingScalar(TILEART_ASSET_TYPE);
        Function accessor = null;
        for (Function f : typePushers) {
            if (accessor == null || callers(f).size() > callers(accessor).size()) {
                accessor = f;
            }
        }
        anchors.add("Immediate 0x6000000 (tile art asset type) in: " + names(typePushers));
        propose(accessor, "UOTileArt::GetObjectDefinition", "uses asset type 0x06000000 and calls"
            + " UOAssets::GetAsset(manager, out, tileId, 0x06000000, 0); most-called such function", false);
        Function getAsset = null;
        long manager = -1;
        if (accessor != null) {
            getAsset = callAfterScalar(accessor, TILEART_ASSET_TYPE);
            List<Long> data = dataRefs(accessor);
            manager = data.isEmpty() ? -1 : data.get(0);
        }
        propose(getAsset, "UOAssets::GetAsset", "called by UOTileArt::GetObjectDefinition after loading 0x06000000;"
            + " resolve path, load blob, run the type's binary factory", false);
        label(manager, "g_UOAssetManager", "asset manager read by UOTileArt::GetObjectDefinition"
            + " (registry +0x110, string dictionary vector +0x150)", apply);
        Function loadBlob = null;
        if (getAsset != null) {
            for (Function callee : distinct(calls(getAsset))) {
                Function readPackaged = null;
                Function readPath = null;
                for (Function inner : distinct(calls(callee))) {
                    if (dictionaryLoad != null && distinct(calls(dictionaryLoad)).contains(inner)
                            && vopen != null && reachesFunction(inner, vopen, IMPORT_SEARCH_DEPTH)) {
                        readPackaged = inner;
                    } else if (functionsReferencingString(RUNTIME_CLIENT).contains(inner)) {
                        readPath = inner;
                    }
                }
                if (readPackaged != null) {
                    loadBlob = callee;
                    List<Function> openers = new ArrayList<>();
                    for (Function opener : distinct(calls(readPackaged))) {
                        if (distinct(calls(opener)).contains(vopen)) {
                            openers.add(opener);
                        }
                    }
                    propose(single(openers, "UOVirtualFileOpen wrapper called by ReadPackagedFile"),
                        "UOP::OpenVirtualFileOwned", "called by UOAssets::ReadPackagedFile; wraps"
                        + " UOVirtualFileOpen(path, mode, 0) into an owning pointer", true);
                    propose(readPackaged, "UOAssets::ReadPackagedFile", "called by UOStringDictionary::Load and"
                        + " UOAssets::LoadAssetBlob; reads a whole UOP virtual file into a Blob", true);
                    propose(readPath, "UOAssets::ReadFileFromPath", "UOAssets::LoadAssetBlob branch for non-packaged"
                        + " assets; reads into a Blob, reports a missing file unless the cwd ends in \"Runtime\\client\"", true);
                }
            }
            propose(loadBlob, "UOAssets::LoadAssetBlob", "callee of UOAssets::GetAsset calling"
                + " UOAssets::ReadPackagedFile (location type 1) or UOAssets::ReadFileFromPath", true);
            if (loadBlob != null) {
                // ResolveAsset: shared by GetAsset and LoadAssetBlob; its resolver loop makes one CALL [reg].
                for (Function callee : distinct(calls(loadBlob))) {
                    if (!distinct(calls(getAsset)).contains(callee)) {
                        continue;
                    }
                    for (Function inner : distinct(calls(callee))) {
                        if (bodySize(inner) <= SMALL_FUNCTION && indirectCallOffsets(inner).equals(Set.of(0L))) {
                            propose(callee, "UOAssets::ResolveAsset", "called by GetAsset and LoadAssetBlob; asks"
                                + " the resolver list, error with the type byte (key >> 24) on failure", true);
                            propose(inner, "UOAssets::TryResolvers", "walks the resolver list until one"
                                + " vtable +0 call succeeds", true);
                        }
                    }
                }
            }
            List<Function> creators = new ArrayList<>();
            for (Function callee : distinct(calls(getAsset))) {
                // Few callers: excludes the widely shared shared_ptr release helpers (dispose is also +4).
                if (indirectCallOffsets(callee).equals(Set.of((long) POINTER_SIZE))
                        && !callee.equals(loadBlob) && callers(callee).size() <= MAX_HELPER_CALLERS) {
                    creators.add(callee);
                }
            }
            if (creators.size() == 1) {
                propose(creators.get(0), "UOAssets::CreateAssetFromBlob", "callee of GetAsset whose only indirect"
                    + " call is factory vtable +4 (Create)", true);
            } else {
                anomalies.add("UOAssets::CreateAssetFromBlob: " + creators.size() + " candidates " + names(creators));
            }
        }

        // 4. UOObjectDefinition binary factory, record parser and sub-parsers.
        Map<Integer, Function> factorySlots = nameSlots(OBJECT_FACTORY, BASE_FACTORY, FACTORY_SLOTS);
        nameSlots(BASE_FACTORY, null, BASE_FACTORY_SLOTS);
        Function create = factorySlots.get(FACTORY_SLOTS[0].offset());
        Function newInstance = factorySlots.get(FACTORY_SLOTS[1].offset());
        Function parser = null;
        if (create != null && streamRead != null) {
            for (Function callee : distinct(calls(create))) {
                if (distinct(calls(callee)).contains(streamRead)) {
                    parser = callee;
                }
            }
        }
        propose(parser, "UOTileArt::ParseObjectDefinition", "callee of UOObjectDefinitionBinaryFactory::Create;"
            + " u16 version (must be " + RECORD_VERSION + "), fixed fields, sub-parsers, 4 shader blocks, effects", false);
        Function allocator = null;
        if (newInstance != null) {
            List<Function> instanceCalls = distinct(calls(newInstance));
            allocator = instanceCalls.isEmpty() ? null : instanceCalls.get(0);
            for (Function callee : instanceCalls) {
                if (referencesAddress(callee, vtables.get(OBJECT_DEFINITION_BASE))) {
                    propose(callee, "UOObjectDefinitionBase::UOObjectDefinitionBase", "called by NewInstance; sets"
                        + " the UOObjectDefinitionBase vftable and the field defaults (1.0, 1.5, 0xffffff, facing 1,"
                        + " body type 400, default shader blocks 1-3)", false);
                    labelDefaultShaders(callee, apply);
                }
            }
        }
        List<Function> subParsers = new ArrayList<>();
        if (parser != null) {
            for (Function callee : distinct(calls(parser))) {
                if (!callee.equals(streamRead) && distinct(calls(callee)).contains(streamRead)) {
                    subParsers.add(callee);
                }
            }
        }
        if (subParsers.size() == SUB_PARSERS.length) {
            for (int i = 0; i < SUB_PARSERS.length; i++) {
                String[] parts = SUB_PARSERS[i].split("\\|");
                propose(subParsers.get(i), parts[0], "call " + (i + 1) + " of " + SUB_PARSERS.length
                    + " stream-reading callees of UOTileArt::ParseObjectDefinition; " + parts[1], false);
            }
            proposeHelper(subParsers.get(SUB_PROPS1), streamRead, "UOObjectDefinitionBase::SetProperty1",
                "map insert called by UOTileArt::ParseProperties1");
            proposeHelper(subParsers.get(SUB_PROPS2), streamRead, "UOObjectDefinitionBase::SetProperty2",
                "map insert called by UOTileArt::ParseProperties2");
            proposeHelper(subParsers.get(SUB_STACK), streamRead, "UOObjectDefinitionBase::AddStackAlias",
                "creates the alias map on first use and inserts {amount, tile id}");
            nameAppearance(subParsers.get(SUB_APPEARANCE), streamRead);
            Function shaderParser = subParsers.get(SUB_SHADER);
            Function itemCtor = callAfter(shaderParser, allocator);
            propose(itemCtor, "UOTileArtTextureItem::Init", "called after allocating 0x2c bytes in"
                + " UOTileArt::ParseShaderBlock: std::string path +0, u32 +0x1c, f32 stretch +0x20, u32 +0x24,"
                + " u32 +0x28", true);
            Function effects = subParsers.get(SUB_EFFECTS);
            for (Function callee : distinct(calls(effects))) {
                if (!callee.equals(streamRead) && distinct(calls(callee)).contains(streamRead)) {
                    propose(callee, "UOEffects::ParseEffectDefinition", "per effect: u32 type key dispatched to the"
                        + " factory list (CanHandle +0, Create +8); same format as EffectDefinitionCollection.uop", true);
                }
            }
            List<Long> effectData = dataRefs(effects);
            label(effectData.isEmpty() ? -1 : effectData.get(0), "g_UOEffectDefinitionFactories",
                "effect factory list passed by UOTileArt::ParseEffects", apply);
        } else {
            anomalies.add("record sub-parsers: expected " + SUB_PARSERS.length + ", found " + subParsers.size()
                + " " + names(subParsers));
        }

        // 5. Vtable slots.
        Map<Integer, Function> objectSlots = nameSlots(OBJECT_DEFINITION, OBJECT_DEFINITION_BASE, OBJECT_SLOTS);
        nameSlots(SHADER_DEFINITION, null, SHADER_SLOTS);

        // 6. Sprite descriptors and texture locations.
        Function spriteSlot = objectSlots.get(SLOT_GET_SPRITE_DESCRIPTOR);
        if (spriteSlot != null) {
            for (Function callee : distinct(calls(spriteSlot))) {
                if (typePushers.contains(callee)) {
                    propose(callee, "UOTileArt::RequestObjectDefinition", "uses asset type 0x06000000 and"
                        + " requests the definition through the asset manager", true);
                } else if (distinct(calls(callee)).stream().anyMatch(g -> pushesValue(g, USE_NEW_ART_FLAG))) {
                    nameSpriteCache(callee);
                }
            }
        }
        proposeStringUser(TILEART_FORMAT, "UOTileArt::RequestTileArtTexture", "formats \"%08d_TileArt\"; uses the"
            + " full texture size when HasFlags1(0, 4) (flags1 bit 34), else the sprite descriptor window", true);
        proposeStringUser(LEGACY_TILEART_FORMAT, "UOTileArt::RequestLegacyTileArtTexture",
            "formats \"%08d_LegacyTileArt\" for the legacy tile art", true);
        for (Function f : functionsReferencingString(TILEART_LEGACY_FOLDER)) {
            boolean fromPath = referencesAddress(f, vtables.get(SIMPLE_LOCATION));
            propose(f, SPRITE_LOCATION + SCOPE + (fromPath ? "ConstructFromPath" : "CopyConstruct"),
                "sets the SpriteTextureFileLocation vftable and classifies the path: \"Textures\\\" 8,"
                + " \"TileArtLegacy\\\" 0x16, \"TileArtEnhanced\\\" 0x1a", true);
        }

        // 7. Users of GetObjectDefinition.
        List<Usage> usages = new ArrayList<>();
        if (accessor != null) {
            List<Function> amountUsers = new ArrayList<>();
            List<Function> joinUsers = new ArrayList<>();
            for (Function f : callers(accessor)) {
                Usage usage = new Usage();
                usage.function = f;
                usage.via = "GetObjectDefinition";
                usages.add(usage);
                Set<Long> offsets = indirectCallOffsets(f);
                if (offsets.contains(SLOT_GET_GRAPHIC_FOR_AMOUNT)) {
                    amountUsers.add(f);
                }
                if (offsets.containsAll(TILES_CONNECT_SLOTS)) {
                    joinUsers.add(f);
                }
            }
            if (amountUsers.size() == 1) {
                propose(amountUsers.get(0), "UOTileArt::GetGraphicForAmount", "only GetObjectDefinition caller using"
                    + " vtable +0xc4 (GetGraphicForAmount); used by the item quantity/objectType UI", true);
            } else {
                anomalies.add("GetGraphicForAmount users: " + names(amountUsers));
            }
            if (joinUsers.size() == 1) {
                propose(joinUsers.get(0), "UOHouseCustomization::TilesConnect", "(dx, dy): tests the house join"
                    + " bits of a tile against the neighbour (6/8, 7/9 along x; 10/12, 11/13 along y)", true);
            } else {
                anomalies.add("TilesConnect candidates: " + names(joinUsers));
            }
        }

        // 7b. World objects and components built from a definition.
        if (accessor != null) {
            nameWorldObjectCode(accessor);
        }

        // 8. Types.
        defineTypes(apply);
        for (long global : defaultShaderGlobals) {
            applyGlobalType(global, SHADER_PTR_TYPE, apply);
        }

        // 9. Apply names.
        if (apply) {
            ensureTag();
            for (Proposal proposal : proposals.values()) {
                applyName(proposal);
            }
        } else {
            for (Proposal proposal : proposals.values()) {
                Function f = functions.getFunctionAt(toAddr(proposal.address));
                proposal.status = f.getName(true).equals(proposal.path) ? "already named"
                    : f.getSymbol().getSource() == SourceType.DEFAULT ? "would rename"
                    : isOwnName(f) ? "would rename from " + f.getName(true)
                    : "kept " + f.getName(true) + " (" + f.getSymbol().getSource() + ")";
                if (proposal.status.startsWith("would rename from")) {
                    corrected.add(hex(proposal.address) + " `" + f.getName(true) + "` -> `" + proposal.path + "`"
                        + (isOwnCommentText(f.getComment(), f.getName(true)) ? "" : " (plate comment kept)"));
                }
                if (proposal.status.startsWith("kept")) {
                    kept.add(hex(proposal.address) + " `" + f.getName(true) + "` (" + f.getSymbol().getSource()
                        + "), proposed `" + proposal.path + "`");
                }
            }
        }
        report(out, apply, usages);
    }

    private void nameWorldObjectCode(Function accessor) {
        Map<Integer, Function> objectSlots = nameSlots(WORLD_OBJECT, null, WORLD_OBJECT_SLOTS);
        List<Function> users = callers(accessor);

        Function sprite = componentInitialize(COMPONENT_SPRITE, SPRITE_INIT_SLOTS);
        propose(sprite, COMPONENT_SPRITE + SCOPE + "Initialize", COMPONENT_SPRITE + " vtable +"
            + hexShort(COMPONENT_INITIALIZE_SLOT) + "; reads the definition sprite descriptor, sitting window and"
            + " GetFacing (stored at component +0x2c)", false);
        Function lighting = componentInitialize(COMPONENT_LIGHTING, LIGHTING_INIT_SLOTS);
        propose(lighting, COMPONENT_LIGHTING + SCOPE + "Initialize", COMPONENT_LIGHTING + " vtable +"
            + hexShort(COMPONENT_INITIALIZE_SLOT) + "; definition is the third argument: GetLightScale -> light"
            + " sprite scale, (0, GetLightOffsetY, GetLightOffsetX) rotated by the view matrix -> light sprite"
            + " position", false);

        List<Function> creators = new ArrayList<>(vtableWriters(COMPONENT_LIGHTING));
        creators.retainAll(vtableWriters(COMPONENT_EFFECTS));
        Function creator = single(creators, "writer of both component vftables");
        propose(creator, WORLD_OBJECT + SCOPE + "CreateComponents", "writes the " + COMPONENT_LIGHTING + " and "
            + COMPONENT_EFFECTS + " vftables; adds the lighting component only when GetLightShaderName is set", true);

        List<Function> cached = new ArrayList<>();
        for (Function f : users) {
            if (hasScalar(f, FALLBACK_TILE_ID)) {
                cached.add(f);
            }
        }
        Function objectDefinition = single(cached, "GetObjectDefinition caller with the fallback tile id");
        propose(objectDefinition, WORLD_OBJECT + SCOPE + "GetObjectDefinition", "caches the definition of tile id"
            + " +0x8 at +0x44; falls back to tile " + FALLBACK_TILE_ID + " when the definition is missing", true);
        if (objectDefinition != null) {
            List<Function> init = new ArrayList<>();
            for (Function f : callers(objectDefinition)) {
                if (indirectCallOffsets(f).containsAll(OBJECT_INIT_SLOTS)) {
                    init.add(f);
                }
            }
            propose(single(init, "UOObject::GetObjectDefinition caller reading GetBodyType and GetAlternateAnimation"),
                WORLD_OBJECT + SCOPE + "InitFromDefinition", "copies GetFlags1, GetBodyType (SetBodyType),"
                + " GetAlternateAnimation (SetAlternateAnimation) and property 3 (height) from the definition",
                true);
        }

        Function setDirection = objectSlots.get(WORLD_OBJECT_SET_DIRECTION);
        if (setDirection != null) {
            List<Function> snaps = new ArrayList<>();
            for (Function callee : distinct(calls(setDirection))) {
                List<Function> calleeCallers = callers(callee);
                if (!callee.equals(accessor) && calleeCallers.size() <= MAX_HELPER_CALLERS && calleeCallers.stream()
                        .allMatch(c -> indirectCallOffsets(c).contains(SLOT_GET_SITTING_DIRECTIONS))) {
                    snaps.add(callee);
                }
            }
            propose(single(snaps, "SetDirection helper"), WORLD_OBJECT + SCOPE + "NearestAllowedDirection",
                "called with the sitting direction mask by every caller (UOObject::SetDirection and the mobile"
                + " animation code); returns the allowed direction (mask 1/2/4/8 = 0/2/4/6) nearest the requested one",
                true);
        }

        List<Function> paperdoll = new ArrayList<>();
        List<Function> placement = new ArrayList<>();
        for (Function f : users) {
            Set<Long> offsets = indirectCallOffsets(f);
            if (offsets.containsAll(PROPERTY2_SLOTS)) {
                paperdoll.add(f);
            }
            if (HOUSE_PLACEMENT_SLOTS.equals(offsets)) {
                placement.add(f);
            }
        }
        propose(single(paperdoll, "HasProperty2/GetProperty2 user"), "UOPaperdoll::BuildImage",
            "only caller of HasProperty2/GetProperty2: per equipped layer, appearance tile = properties2 key 11, else"
            + " key 8, else properties1 key 11, else key 8 (+10000000); draws gump id + " + PAPERDOLL_GUMP_FEMALE
            + " (female) or + " + PAPERDOLL_GUMP_MALE + "; HasFlags2 bit 18 (PartialHue) selects partial hue", true);
        propose(single(placement, "house placement check"), "UOHouseCustomization::IsPlacementAllowed",
            "calls only HasHouseJoinBit1/2/3/5 (the bit 3 load shares the bit 2 CALL): cell (0, 0) needs bit 1,"
            + " x = 0 needs bit 2, y = 0 needs bit 3; bit 5 adds the support check of the cell below", true);
    }

    private Function componentInitialize(String cls, Set<Long> requiredSlots) {
        long vt = vtables.getOrDefault(cls, -1L);
        Function f = vt < 0 ? null : slotFunction(vt, COMPONENT_INITIALIZE_SLOT);
        if (f == null || !indirectCallOffsets(f).containsAll(requiredSlots)) {
            anomalies.add(cls + " Initialize slot does not call UOObjectDefinition slots " + requiredSlots);
            return null;
        }
        return f;
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

    private void proposeHelper(Function parent, Function streamRead, String path, String evidence) {
        List<Function> found = new ArrayList<>();
        for (Function callee : distinct(calls(parent))) {
            if (!callee.equals(streamRead) && callers(callee).size() <= MAX_HELPER_CALLERS
                    && !distinct(calls(callee)).contains(streamRead)) {
                found.add(callee);
            }
        }
        if (found.size() == 1) {
            propose(found.get(0), path, evidence, false);
        } else {
            anomalies.add("`" + path + "`: " + found.size() + " candidates " + names(found));
        }
    }

    private void proposeStringUser(String text, String path, String evidence, boolean probable) {
        propose(single(functionsReferencingString(text), text), path, evidence, probable);
    }

    private void proposeVtableWriter(String cls, String path, String evidence, boolean probable) {
        List<Function> writers = vtableWriters(cls);
        if (writers.size() == 1) {
            propose(writers.get(0), path, "writes the " + cls + " vftable; " + evidence, probable);
        } else {
            anomalies.add("`" + path + "`: " + writers.size() + " vftable writers " + names(writers));
        }
    }

    private void nameAppearance(Function appearance, Function streamRead) {
        String[][] filters = {
            {FILTER_BODY, "UOTileArt::ParseAppearanceFilterByOwnerBodyType", "subtype 0: u32 n + n x {u32 body, u32 appearance}"},
            {FILTER_GENDER, "UOTileArt::ParseAppearanceFilterByOwnerGender", "subtype 1: u8 gender + u32 appearance"},
            {FILTER_ANIMATION_BODY, "UOTileArt::ParseAnimationAppearanceFilterByOwnerBodyType",
                "subtype 2: u32 n + n x {u32 body, u32 animation appearance}"},
        };
        List<Function> parsers = new ArrayList<>();
        for (Function callee : distinct(calls(appearance))) {
            if (!callee.equals(streamRead) && distinct(calls(callee)).contains(streamRead)) {
                parsers.add(callee);
            }
        }
        proposeHelper(appearance, streamRead, "UOObjectDefinitionBase::AddAppearanceFilter",
            "creates the filter vector at +0xbc on first use and appends a filter");
        Map<Function, Integer> entryAdders = new HashMap<>();
        for (String[] filter : filters) {
            List<Function> ctors = vtableWriters(filter[0]);
            Function match = null;
            for (Function p : parsers) {
                for (Function callee : distinct(calls(p))) {
                    if (ctors.contains(callee)) {
                        match = p;
                        propose(callee, filter[0] + SCOPE + filter[0], "writes the " + filter[0]
                            + " vftable; called by " + filter[1], false);
                    }
                }
            }
            propose(match, filter[1], "calls the " + filter[0] + " constructor; " + filter[2], false);
            if (match != null) {
                for (Function callee : distinct(calls(match))) {
                    if (!callee.equals(streamRead) && callers(callee).size() <= MAX_HELPER_CALLERS
                            && !ctors.contains(callee) && !vtableWriters(FILTER_BODY).contains(callee)) {
                        entryAdders.merge(callee, 1, Integer::sum);
                    }
                }
            }
            nameFilterSlot(filter[0], FILTER_TYPE_SLOT, "GetFilterType", "returns the filter subtype"
                + " (GetAppearance skips type 2)", false);
            nameFilterSlot(filter[0], FILTER_APPLY_SLOT, "Apply", "matches the owner and replaces the appearance"
                + " id; GetAppearance stops at the first match", true);
        }
        for (var entry : entryAdders.entrySet()) {
            if (entry.getValue() > 1) {
                propose(entry.getKey(), FILTER_BODY + SCOPE + "AddBodyMapping", "inserts {body, appearance};"
                    + " shared by the subtype 0 and 2 parsers", true);
            }
        }
    }

    private void nameFilterSlot(String cls, int offset, String name, String evidence, boolean probable) {
        long vt = vtables.getOrDefault(cls, -1L);
        if (vt < 0) {
            return;
        }
        Function f = slotFunction(vt, offset);
        if (f != null && codePointerCounts.getOrDefault(f.getEntryPoint().getOffset(), 0) == 1) {
            propose(f, cls + SCOPE + name, "vtable +" + hexShort(offset) + "; " + evidence, probable);
        }
    }

    private void nameSpriteCache(Function cache) {
        propose(cache, "UOTileArt::GetCachedSpriteDescriptor", "called by UOObjectDefinition::GetSpriteDescriptor;"
            + " per tile id map under a critical section; builds 0x120-byte descriptors on a miss", true);
        for (Function callee : distinct(calls(cache))) {
            if (pushesValue(callee, USE_NEW_ART_FLAG)) {
                propose(callee, "UOTileArt::BuildSpriteDescriptor", "(useLegacy, def): HasFlags2(0x80000 UseNewArt)"
                    + " -> mode 1 block 0, WorldArt scale, EC window (+1, 44x44 without world art); else"
                    + " UsesEnhancedTileArt -> mode 2 block 2, legacy window x1.5; else mode 0 block 1; foliage or"
                    + " flags1 bit 35 -> alpha; flags1 bit 34 -> rotation; sitting 1/8 shift the window", true);
            } else if (storesScalar(callee, FLOAT_ONE_BITS) && distinct(calls(callee)).isEmpty()) {
                propose(callee, "UOTileArt::SpriteDescriptor_Init", "zeroes the 0x120-byte descriptor, sets scales"
                    + " to 1.0", true);
            }
        }
    }

    // A function referenced from more tables than its class and base vtables is skipped (identical-code folding).
    private Map<Integer, Function> nameSlots(String cls, String baseCls, Slot[] slots) {
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

    private void labelDefaultShaders(Function ctor, boolean apply) throws Exception {
        Map<Long, String> byOffset = DEFAULT_SHADER_LABELS;
        // MOV reg, [global] followed by MOV [obj + off], reg.
        Map<Register, Long> loaded = new HashMap<>();
        for (Instruction i : listing.getInstructions(ctor.getBody(), true)) {
            if (!i.getMnemonicString().equals("MOV")) {
                continue;
            }
            Object[] dst = i.getOpObjects(0);
            Object[] src = i.getOpObjects(1);
            if (dst.length == 1 && dst[0] instanceof Register reg) {
                Long global = null;
                for (Reference r : i.getReferencesFrom()) {
                    if (r.isMemoryReference() && !isExec(r.getToAddress().getOffset())) {
                        global = r.getToAddress().getOffset();
                    }
                }
                if (global != null && src.length == 1 && src[0] instanceof Address) {
                    loaded.put(reg, global);
                } else {
                    loaded.remove(reg);
                }
            } else if (src.length == 1 && src[0] instanceof Register reg && loaded.containsKey(reg)) {
                for (Object o : dst) {
                    if (o instanceof Scalar s && byOffset.containsKey(s.getUnsignedValue())) {
                        defaultShaderGlobals.add(loaded.get(reg));
                        label(loaded.get(reg), byOffset.get(s.getUnsignedValue()), "default shader shared_ptr stored"
                            + " at UOObjectDefinitionBase +" + hexShort(s.getUnsignedValue()) + " by the constructor;"
                            + " HasLegacyArtData compares block 1 with it", apply);
                    }
                }
            }
        }
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
        } else if (isOwnName(function)) {
            String old = symbol.getName(true);
            String oldComment = function.getComment();
            symbol.setNameAndNamespace(name, namespace, SourceType.ANALYSIS);
            proposal.status = "renamed from " + old;
            corrected.add(hex(proposal.address) + " `" + old + "` -> `" + proposal.path + "`");
            count("own names corrected");
            if (isOwnCommentText(oldComment, old)) {
                function.setComment(plateComment(proposal));
                count("plate comments refreshed");
            }
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

    // A name this script applied earlier: source ANALYSIS, tag UO_TILEART and a plate comment written by this
    // script for exactly the current name. Names from other sources, tags or comments are never touched.
    private boolean isOwnName(Function function) {
        Symbol symbol = function.getSymbol();
        if (symbol.getSource() != SourceType.ANALYSIS) {
            return false;
        }
        boolean tagged = false;
        for (FunctionTag tag : function.getTags()) {
            tagged |= tag.getName().equals(TAG);
        }
        String comment = function.getComment();
        return tagged && comment != null && comment.startsWith(ownCommentHead(symbol.getName(true)));
    }

    // The comment is still the script's own text: its head for that name and one line, nothing added.
    private static boolean isOwnCommentText(String comment, String path) {
        return comment != null && comment.startsWith(ownCommentHead(path)) && !comment.contains("\n");
    }

    private static String ownCommentHead(String path) {
        return COMMENT_PREFIX + TAG + " `" + path + "`";
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
        DataType u16 = UnsignedShortDataType.dataType;
        DataType u32 = UnsignedIntegerDataType.dataType;
        DataType i32 = IntegerDataType.dataType;
        DataType f32 = FloatDataType.dataType;
        DataType u64 = QWordDataType.dataType;
        DataType bool = BooleanDataType.dataType;
        DataType voidPtr = new PointerDataType(VoidDataType.dataType, dtm);
        final int stringBuffer = 16;
        final int windowInts = 6;
        final int shaderValues = 4;

        StructureDataType string = struct("MsvcString", 0x1c, "MSVC 2008 std::string: buffer holds a pointer when"
            + " capacity >= 16");
        field(string, 0x0, voidPtr, "allocator", null);
        field(string, 0x4, new ArrayDataType(CharDataType.dataType, stringBuffer, 1, dtm), "buffer", null);
        field(string, 0x14, u32, "size", null);
        field(string, 0x18, u32, "capacity", null);
        StructureDataType vector = struct("MsvcVector", 0x10, "MSVC 2008 std::vector");
        field(vector, 0x0, voidPtr, "allocator", null);
        field(vector, 0x4, voidPtr, "first", null);
        field(vector, 0x8, voidPtr, "last", null);
        field(vector, 0xc, voidPtr, "end", null);
        StructureDataType map = struct("MsvcMap", 0xc, "MSVC 2008 std::map");
        field(map, 0x0, voidPtr, "allocator", null);
        field(map, 0x4, voidPtr, "head", null);
        field(map, 0x8, u32, "size", null);

        StructureDataType window = struct("UOTileArtImageWindow", 0x18, "image window (6 x i32) of a tile art"
            + " record; the sprite builder treats x_end/y_end as inclusive");
        String[] windowNames = {"x_start", "y_start", "x_end", "y_end", "x_offset", "y_offset"};
        for (int i = 0; i < windowInts; i++) {
            field(window, i * POINTER_SIZE, i32, windowNames[i], null);
        }
        StructureDataType sitting = struct("UOTileArtSitting", 0x18, "sitting record: 6 x u32 (file and memory)");
        String[] sittingNames = {"directions", "seat_offset_x", "seat_offset_y", "window_shift",
            "field_10", "field_14"};
        String[] sittingComments = {"direction mask: 1 north, 2 east, 4 south, 8 west",
            "subtracted from the seated mobile sprite x", "subtracted from the seated mobile sprite y",
            "shifts the chair-back overlay split for directions 1 and 8", "not read by the client",
            "not read by the client"};
        for (int i = 0; i < sittingNames.length; i++) {
            field(sitting, i * POINTER_SIZE, u32, sittingNames[i], sittingComments[i]);
        }
        StructureDataType item = struct("UOTileArtTextureItem", 0x2c, "texture item of a shader block");
        field(item, 0x0, string, "path", "string dictionary entry, e.g. Data\\WorldArt\\...");
        field(item, 0x1c, u32, "source_kind", "record u8; 0 in all records; material builder branches on 0/2");
        field(item, 0x20, f32, "stretch", "repetition; != 1.0 selects wrap addressing");
        field(item, 0x24, u32, "sampler_index", "probable: 0..3 per block, noise texture 1");
        field(item, 0x28, u32, "uv_set", "ORed into the sampler flags (UV set)");
        StructureDataType shader = struct("UOShaderDefinition", 0x54, "shader block of a tile art record");
        field(shader, 0x0, voidPtr, "vftable", null);
        field(shader, 0x4, u32, "field_04", "record u8; 0 in all records");
        field(shader, 0x8, string, "name", "UOSpriteShader, UOStaticTerrainShader, UOWaterShader");
        field(shader, 0x24, vector, "textures", "vector of shared_ptr<UOTileArtTextureItem>");
        field(shader, 0x34, new ArrayDataType(i32, shaderValues, i32.getLength(), dtm), "technique", null);
        field(shader, 0x44, new ArrayDataType(f32, shaderValues, f32.getLength(), dtm), "tweaks", null);
        StructureDataType shaderPtr = struct(SHADER_PTR_TYPE, 0x8, "boost::shared_ptr");
        field(shaderPtr, 0x0, new PointerDataType(shader, dtm), "px", null);
        field(shaderPtr, 0x4, voidPtr, "pn", null);
        StructureDataType sittingPtr = struct("UOTileArtSittingSharedPtr", 0x8, "boost::shared_ptr");
        field(sittingPtr, 0x0, new PointerDataType(sitting, dtm), "px", null);
        field(sittingPtr, 0x4, voidPtr, "pn", null);

        StructureDataType def = struct("UOObjectDefinition", 0x100, "decoded tileart.uop record"
            + " (UOObjectDefinitionBase 0xf0 + effect vector)");
        field(def, 0x0, voidPtr, "vftable", null);
        field(def, 0x4, string, "name", null);
        field(def, 0x20, u32, "tile_id", null);
        field(def, 0x24, window, "enhanced_window", null);
        field(def, 0x3c, window, "legacy_window", null);
        field(def, 0x54, bool, "has_world_art", "block 0 present");
        field(def, 0x55, bool, "uses_enhanced_tileart", "0 in all records");
        field(def, 0x58, f32, "world_art_scale", "default 1.0");
        field(def, 0x5c, f32, "legacy_art_scale", "default 1.5");
        field(def, 0x60, u32, "field_60", "0 in all records");
        field(def, 0x64, u32, "old_id", "default 0xffffff");
        field(def, 0x68, f32, "rotation", "degrees, used with flags1 bit 34");
        field(def, 0x6c, f32, "light_scale", "light sprite scale");
        field(def, 0x70, u32, "light_field_70", "0 in all records; not read by the client");
        field(def, 0x74, f32, "light_offset_x", "light sprite offset, rotated by the view matrix");
        field(def, 0x78, f32, "light_offset_y", "light sprite offset, rotated by the view matrix");
        field(def, 0x7c, u32, "house_join_bits", "house customization connection bits");
        field(def, 0x80, u64, "flags1", null);
        field(def, 0x88, u64, "flags2", null);
        field(def, 0x90, u32, "facing", "default 1; copied to the sprite component +0x2c");
        field(def, 0x94, u32, "radar_rgba", null);
        field(def, 0x98, bool, "has_radar_color", null);
        String[] blockNames = {"world_art_shader", "legacy_shader", "enhanced_shader", "light_shader"};
        for (int i = 0; i < TEXTURE_BLOCKS; i++) {
            field(def, 0x9c + i * shaderPtr.getLength(), shaderPtr, blockNames[i], null);
        }
        field(def, 0xbc, voidPtr, "appearance_filters", "vector of filter pointers, allocated on first use");
        field(def, 0xc0, sittingPtr, "sitting", null);
        field(def, 0xc8, map, "properties1", null);
        field(def, 0xd4, map, "properties2", null);
        field(def, 0xe0, voidPtr, "stack_aliases", "map amount -> tile id, allocated on first use");
        field(def, 0xe4, u32, "body_type", "default 400 (human male), 666 gargoyle; must match the wearer's body");
        field(def, 0xe8, bool, "alternate_animation", "layer 2 item: wearer uses the alternate animation actions");
        field(def, 0xf0, vector, "effects", "vector of shared_ptr effect definitions");

        StructureDataType record = new StructureDataType(CATEGORY, "TileArtRecordFixed", 0, dtm);
        record.setDescription("tileart.uop record (version 4), fixed part before props1; byte packed");
        Object[][] recordFields = {
            {u16, "version", "must be 4"}, {u32, "name_index", "string dictionary index (0 = empty)"},
            {u32, "tile_id", null}, {u8, "has_world_art", null}, {u8, "uses_enhanced_tileart", null},
            {f32, "world_art_scale", null}, {f32, "legacy_art_scale", null}, {u32, "field_60", null},
            {u32, "old_id", null}, {f32, "rotation", null}, {u32, "body_type", null}, {u8, "alternate_animation", null},
            {f32, "light_scale", null}, {u32, "light_field_70", null}, {f32, "light_offset_x", null},
            {f32, "light_offset_y", null}, {u32, "house_join_bits", "set via vtable +0x14c"},
            {u64, "flags1", null}, {u64, "flags2", null}, {u32, "facing", null},
            {window, "enhanced_window", null}, {window, "legacy_window", null},
        };
        for (Object[] f : recordFields) {
            record.add((DataType) f[0], (String) f[1], (String) f[2]);
        }
        StructureDataType dictionary = new StructureDataType(CATEGORY, "StringDictionaryHeader", 0, dtm);
        dictionary.setDescription("string_dictionary.bin header, followed by count x {u16 length, bytes}");
        dictionary.add(u64, "stamp", "probable FILETIME build stamp");
        dictionary.add(u32, "count", "string 0 is empty");
        dictionary.add(u16, "max_length", "reader allocates max_length + 1");

        for (StructureDataType t : List.of(string, vector, map, window, sitting, item, shader, shaderPtr, sittingPtr,
                def, record, dictionary)) {
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

    private void field(StructureDataType s, int offset, DataType type, String name, String comment) {
        s.replaceAtOffset(offset, type, type.getLength(), name, comment);
    }

    private void applyGlobalType(long address, String typeName, boolean apply) throws Exception {
        DataType type = dtm.getDataType(CATEGORY, typeName);
        Address a = toAddr(address);
        Data data = getDataAt(a);
        String status;
        if (type != null && data != null && data.getDataType().isEquivalent(type)) {
            status = "unchanged";
        } else if (!apply) {
            status = "would apply";
        } else {
            try {
                DataUtilities.createData(currentProgram, a, type, type.getLength(), false,
                    ClearDataMode.CLEAR_ALL_UNDEFINED_CONFLICT_DATA);
                status = "applied";
                count("global types applied");
            } catch (Exception conflict) {
                status = "not applied: " + conflict.getMessage();
                anomalies.add(hex(address) + " `" + typeName + "` " + status);
            }
        }
        typeLines.add("| " + hex(address) + " as `" + typeName + "` | " + hexShort(POINTER_SIZE * 2) + " | " + status + " |");
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

    private Set<Function> calleeClosure(Function f, int depth) {
        Set<Function> seen = new LinkedHashSet<>();
        List<Function> current = List.of(f);
        for (int d = 0; d <= depth; d++) {
            List<Function> next = new ArrayList<>();
            for (Function g : current) {
                if (seen.add(g) && !g.isExternal()) {
                    next.addAll(distinct(calls(g)));
                }
            }
            current = next;
        }
        return seen;
    }

    private boolean reachesImport(Function f, String name, int depth) {
        for (Function g : calleeClosure(f, depth)) {
            for (Instruction i : listing.getInstructions(g.getBody(), true)) {
                if (!i.getFlowType().isCall()) {
                    continue;
                }
                for (Reference r : i.getReferencesFrom()) {
                    Symbol s = symbols.getPrimarySymbol(r.getToAddress());
                    if (s != null && s.getName().contains(name)) {
                        return true;
                    }
                    Data data = getDataAt(r.getToAddress());
                    if (data != null && data.getValue() instanceof Address target) {
                        Symbol t = symbols.getPrimarySymbol(target);
                        if (t != null && t.getName().equals(name)) {
                            return true;
                        }
                    }
                }
                Address[] flows = i.getFlows();
                for (Address flow : flows) {
                    Function callee = functions.getFunctionAt(flow);
                    if (callee != null && callee.getName().equals(name)) {
                        return true;
                    }
                }
            }
        }
        return false;
    }

    private boolean reachesFunction(Function f, Function target, int depth) {
        return calleeClosure(f, depth).contains(target);
    }

    private boolean callsNamed(Function f, String name) {
        for (Function c : distinct(calls(f))) {
            if (c.getName().equals(name)) {
                return true;
            }
        }
        return false;
    }

    private Function functionNamed(String name) {
        for (Function f : functions.getFunctions(true)) {
            if (f.getName().equals(name)) {
                return f;
            }
        }
        return null;
    }

    private List<Function> functionsReferencingString(String text) {
        List<Function> out = new ArrayList<>();
        byte[] pattern = (text + "\0").getBytes(StandardCharsets.US_ASCII);
        for (long at : findBytes(pattern, true)) {
            if (previousByte(at) != 0) {
                continue;
            }
            for (Reference r : getReferencesTo(toAddr(at))) {
                Function f = getFunctionContaining(r.getFromAddress());
                if (f != null && !out.contains(f)) {
                    out.add(f);
                }
            }
        }
        return out;
    }

    private int previousByte(long address) {
        for (Block block : blocks) {
            long index = address - 1 - block.start;
            if (index >= 0 && index < block.bytes.length) {
                return block.bytes[(int) index] & BYTE_MASK;
            }
        }
        return 0;
    }

    private Function single(List<Function> list, String what) {
        if (list.size() == 1) {
            return list.get(0);
        }
        anomalies.add(what + ": " + list.size() + " candidates " + names(list));
        return null;
    }

    private List<Function> functionsUsingScalar(long value) {
        List<Function> out = new ArrayList<>();
        for (Instruction i : listing.getInstructions(true)) {
            if (monitor.isCancelled()) {
                break;
            }
            if (hasScalarOperand(i, value)) {
                Function f = getFunctionContaining(i.getAddress());
                if (f != null && !out.contains(f)) {
                    out.add(f);
                }
            }
        }
        return out;
    }

    private static boolean isScalar(Instruction i, long value) {
        Object[] objects = i.getOpObjects(0);
        return objects.length == 1 && objects[0] instanceof Scalar s && (s.getUnsignedValue() & DWORD_MASK) == value;
    }

    private boolean pushesValue(Function f, long value) {
        for (Instruction i : listing.getInstructions(f.getBody(), true)) {
            if (i.getMnemonicString().equals("PUSH") && isScalar(i, value)) {
                return true;
            }
        }
        return false;
    }

    private boolean hasScalar(Function f, long value) {
        for (Instruction i : listing.getInstructions(f.getBody(), true)) {
            if (hasScalarOperand(i, value)) {
                return true;
            }
        }
        return false;
    }

    private static boolean hasScalarOperand(Instruction i, long value) {
        for (int op = 0; op < i.getNumOperands(); op++) {
            for (Object o : i.getOpObjects(op)) {
                if (o instanceof Scalar s && (s.getUnsignedValue() & DWORD_MASK) == value) {
                    return true;
                }
            }
        }
        return false;
    }

    private boolean storesScalar(Function f, long value) {
        for (Instruction i : listing.getInstructions(f.getBody(), true)) {
            if (i.getMnemonicString().equals("MOV") && i.getNumOperands() == 2) {
                for (Object o : i.getOpObjects(1)) {
                    if (o instanceof Scalar s && (s.getUnsignedValue() & DWORD_MASK) == value) {
                        return true;
                    }
                }
            }
        }
        return false;
    }

    private Function callAfterScalar(Function f, long value) {
        boolean pushed = false;
        for (Instruction i : listing.getInstructions(f.getBody(), true)) {
            if (hasScalarOperand(i, value)) {
                pushed = true;
            } else if (pushed && i.getFlowType().isCall() && i.getFlows().length > 0) {
                return functions.getFunctionAt(i.getFlows()[0]);
            }
        }
        return null;
    }

    private Function callAfter(Function f, Function first) {
        boolean seen = false;
        for (Instruction i : listing.getInstructions(f.getBody(), true)) {
            if (!i.getFlowType().isCall() || i.getFlows().length == 0) {
                continue;
            }
            Function callee = functions.getFunctionAt(i.getFlows()[0]);
            if (seen) {
                return callee;
            }
            seen = callee != null && callee.equals(first);
        }
        return null;
    }

    // Offsets of virtual calls: CALL [reg + off], or MOV reg, [reg2 + off] followed by CALL reg.
    private Set<Long> indirectCallOffsets(Function f) {
        Set<Long> offsets = new LinkedHashSet<>();
        Map<Register, Long> loaded = new HashMap<>();
        for (Instruction i : listing.getInstructions(f.getBody(), true)) {
            if (i.getMnemonicString().equals("MOV") && i.getNumOperands() == 2) {
                Object[] dst = i.getOpObjects(0);
                if (dst.length == 1 && dst[0] instanceof Register reg) {
                    Long offset = memoryOffset(i, 1);
                    if (offset != null) {
                        loaded.put(reg, offset);
                    } else {
                        loaded.remove(reg);
                    }
                }
                continue;
            }
            if (!i.getFlowType().isCall() || i.getFlows().length > 0) {
                continue;
            }
            Long offset = memoryOffset(i, 0);
            Object[] objects = i.getOpObjects(0);
            if (offset != null) {
                offsets.add(offset);
            } else if (objects.length == 1 && objects[0] instanceof Register reg && loaded.containsKey(reg)) {
                offsets.add(loaded.get(reg));
            }
        }
        return offsets;
    }

    // Displacement of a [reg + off] operand (0 for [reg]); null for other operands.
    private static Long memoryOffset(Instruction i, int op) {
        if (!i.getDefaultOperandRepresentation(op).contains("[")) {
            return null;
        }
        boolean register = false;
        long offset = 0;
        for (Object o : i.getOpObjects(op)) {
            register |= o instanceof Register;
            if (o instanceof Scalar s) {
                offset = s.getUnsignedValue();
            }
        }
        return register ? offset : null;
    }

    private List<Long> dataRefs(Function f) {
        List<Long> out = new ArrayList<>();
        for (Instruction i : listing.getInstructions(f.getBody(), true)) {
            for (Reference r : i.getReferencesFrom()) {
                Address to = r.getToAddress();
                if (!r.isMemoryReference() || r.getReferenceType().isFlow() || isExec(to.getOffset())
                        || !memory.contains(to)) {
                    continue;
                }
                Data data = getDataAt(to);
                if ((data == null || !data.hasStringValue()) && !out.contains(to.getOffset())) {
                    out.add(to.getOffset());
                }
            }
        }
        return out;
    }

    private boolean referencesAddress(Function f, Long address) {
        if (address == null || address < 0) {
            return false;
        }
        for (Instruction i : listing.getInstructions(f.getBody(), true)) {
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
        }
        return false;
    }

    private static long bodySize(Function f) {
        return f.getBody().getNumAddresses();
    }

    // ---- report ----

    private void report(PrintWriter out, boolean apply, List<Usage> usages) {
        int probable = 0;
        for (Proposal p : proposals.values()) {
            probable += p.probable ? 1 : 0;
        }
        out.println("# tileart code names");
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
        out.println("| Own names corrected | " + corrected.size() + " |");
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
        out.println();
        out.println("## Users of UOTileArt::GetObjectDefinition");
        out.println();
        out.println("Indirect call offsets may belong to other objects; strings are the first few referenced.");
        out.println();
        out.println("| Function | Virtual call offsets (UOObjectDefinition slot name if any) | Strings |");
        out.println("|---|---|---|");
        for (Usage u : usages) {
            List<String> strings = new ArrayList<>();
            for (Instruction i : listing.getInstructions(u.function.getBody(), true)) {
                for (Reference r : i.getReferencesFrom()) {
                    Data d = getDataAt(r.getToAddress());
                    if (d != null && d.hasStringValue() && strings.size() < REPORT_STRINGS) {
                        strings.add("`" + String.valueOf(d.getValue()).replace("|", "\\|") + "`");
                    }
                }
            }
            List<String> offsets = new ArrayList<>();
            for (long o : indirectCallOffsets(u.function)) {
                String slot = "";
                for (Slot candidate : OBJECT_SLOTS) {
                    slot = candidate.offset() == o ? " " + candidate.name() : slot;
                }
                offsets.add("+" + hexShort(o) + slot);
            }
            out.println("| " + u.function.getName(true) + " @" + u.function.getEntryPoint() + " | "
                + String.join(" ", offsets) + " | " + String.join(", ", strings) + " |");
        }
        section(out, "Slots not named (identical-code folding)", shared);
        section(out, "Own names corrected (applied earlier by this script)", corrected);
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
