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
    private static final String ACTIVE_EFFECTS_LABEL = "g_UOActiveEffects";
    private static final String LOD_LABEL = "g_UOParticleLODLevel";
    private static final String FILTER_LABEL = "g_UOParticleFilter";
    private static final String USER_FILTER_LABEL = "g_UOUserEffectFilter";

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
        new Slot("UOHuedParticleEffectNif", 0x38, "Init", "(out, definition): Nif init with the object hue applied", true),
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

        defineTypes(apply);

        // 8. Apply names.
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
        field(particle, 0x40, bool, "hued", "apply the object hue (UOHuedParticleEffectNif)");
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
                quake, randomEntry, random, filter, collection, particleRecord, soundRecord, offsetRecord, quakeRecord)) {
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
