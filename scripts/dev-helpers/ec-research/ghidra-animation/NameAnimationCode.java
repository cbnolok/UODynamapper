// Name the mobile/item animation code of UOSA.exe: the AnimationSequence.uop and AnimationDefinition.uop asset
// factories and parsers, the UOAnimationSequence / FrameSetLookup / UOAnimationDefinition / UOAnimationFrameSet
// methods that give the parsed fields their meaning, the asset getters, the mobile animation builder, the
// animation components' entry points, the 0x6E / 0xE2 animation packets, the animation settings, the
// AnimationFrame asset (RLE decoders, palette builder, frame atlas, layer frame-set loader) and the hue tables.
// With --apply, rename DEFAULT functions, add the UO_ANIMATION tag, plate comments and the setting labels.
// @category ECResearch
import ghidra.app.script.GhidraScript;
import ghidra.program.model.address.Address;
import ghidra.program.model.lang.Register;
import ghidra.program.model.listing.Data;
import ghidra.program.model.listing.Function;
import ghidra.program.model.listing.FunctionManager;
import ghidra.program.model.listing.FunctionTag;
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

public class NameAnimationCode extends GhidraScript {
    private static final String APPLY_FLAG = "--apply";
    private static final String TAG = "UO_ANIMATION";
    private static final String TAG_COMMENT = "AnimationSequence/AnimationDefinition assets, sequence lookups, mobile"
        + " animation builder, animation components and the 0x6E/0xE2 packets";
    private static final String COMMENT_PREFIX = "NameAnimationCode: ";
    private static final String PROBABLE = " (probable)";
    private static final String SCOPE = "::";
    private static final String NAMESPACE = "UOAnimations";

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
    // Instructions scanned after a setting load for the store into its run-time copy.
    private static final int COPY_WINDOW = 8;
    // Instructions scanned after a vtable slot load for the call through it.
    private static final int SLOT_CALL_WINDOW = 3;

    // Classes found through RTTI.
    private static final String SEQUENCE_FACTORY = "UOAnimationSequenceBinaryFactory";
    private static final String DEFINITION_FACTORY = "UOAnimationDefinitionBinaryFactory";
    private static final String SEQUENCE = "UOAnimationSequence";
    private static final String LOOKUP = "FrameSetLookup";
    private static final String DEFINITION = "UOAnimationDefinition";
    private static final String FRAME_SET = "UOAnimationFrameSet";
    private static final String MOB_COMPONENT = "UOObjectComponentAnimationMob";
    private static final String TILE_COMPONENT = "UOObjectComponentAnimationTile";
    private static final String FRAME_FACTORY = "UOAnimationFrameSetFactory";
    private static final String MOB_FRAME_SET = "UOMobFrameSet";
    private static final String OBJECT = "UOObject";
    private static final String[] CLASSES = {SEQUENCE_FACTORY, DEFINITION_FACTORY, SEQUENCE, LOOKUP, DEFINITION,
        FRAME_SET, MOB_COMPONENT, TILE_COMPONENT, FRAME_FACTORY, MOB_FRAME_SET, OBJECT};
    private static final String HUES = "UOHues";

    // Asset factory vtable: +4 Create(out, blob, ...). The definition factory adds +0x10 ReadFrame.
    private static final int FACTORY_CREATE = 0x4;
    private static final int DEFINITION_FACTORY_READ_FRAME = 0x10;
    // Animation component vtable (UOObjectComponentAnimation): +4 initialize from the owner, +0x18 per-frame
    // update, +0x5c play action.
    private static final int COMPONENT_INITIALIZE = 0x4;
    private static final int COMPONENT_UPDATE = 0x18;
    private static final int COMPONENT_PLAY_ACTION = 0x5c;
    // UOObjectComponentAnimationMob vtable +0x68: refreshes the movement action (GetMoveAction).
    private static final int COMPONENT_REFRESH_MOVE_ACTION = 0x68;
    // UOObject vtable +0xa0 / +0xa4: get / set the 0x77/0x78 mobile flags byte kept at object +0x6c.
    private static final int OBJECT_GET_MOBILE_FLAGS = 0xa0;
    private static final int OBJECT_SET_MOBILE_FLAGS = 0xa4;
    // UOObject vtable +0x78: the item's alternate animation flag (tileart definition +0xe8).
    private static final long OBJECT_GET_ALTERNATE_ANIMATION = 0x78;
    // UOObjectDefinition vtable: HasRace +0xc8, HasGender +0xcc, GetRace +0xd0, GetGender +0xd4.
    private static final long[] DEFINITION_RACE_GENDER_SLOTS = {0xc8, 0xcc, 0xd0, 0xd4};

    // Asset ids: {id, type << 24}.
    private static final int ASSET_TYPE_SHIFT = 24;
    private static final long DEFINITION_ASSET_TYPE = 0x0b;
    private static final long SEQUENCE_ASSET_TYPE = 0x0c;
    private static final long FRAME_ASSET_TYPE = 0x10;

    // Names given by other scripts, used as anchors.
    private static final String PARSE_COLLECTION = "UOEffects::ParseEffectDefinitionCollection";
    private static final String GET_ASSET = "UOAssets::GetAsset";
    private static final String ASSET_MANAGER = "g_UOAssetManager";
    private static final String PROJECTILE_INIT = "UOProjectileEffect::Init";
    private static final String TILE_EFFECT_INIT = "UOTileEffect::Init";
    private static final String RECEIVE_PREFIX = "Receive_0x";

    // Strings.
    private static final String MOUNTED_STRING = ", mounted";
    private static final String IDLE_SETTING = "idleAnimation";
    private static final String RESTRICTION_SETTING = "frameSetRestriction";
    private static final String CACHE_SETTING = "cacheSize";
    private static final String FLYING_SETTING = "flying_animations";
    private static final String HOUSE_SETTING = "display_all_house_contents";
    private static final String[] SETTINGS = {IDLE_SETTING, RESTRICTION_SETTING, CACHE_SETTING, FLYING_SETTING,
        HOUSE_SETTING};

    // Action record field displacements read by the per-direction offset getters (int[8] each).
    private static final long OFFSET_X_FIELD = 0x48;
    private static final long OFFSET_Y_FIELD = 0x68;
    private static final long EXTRA_X_FIELD = 0x88;
    private static final long EXTRA_Y_FIELD = 0xa8;
    // UOAnimationFrameSet +0x38: next frame set written by SetNext.
    private static final long FRAME_SET_NEXT_FIELD = 0x38;

    // AnimationFrame pixels: the RLE decoders blend with the packed red/blue mask and test run bytes against 0x80;
    // hue 0x4001 is the palette builder's special (shadow) hue; hue bitmaps are 256 entries wide.
    private static final long PACKED_RED_BLUE_MASK = 0xff00ff;
    private static final long RLE_RUN_FLAG = 0x80;
    private static final long SPECIAL_HUE = 0x4001;
    private static final long HUE_BITMAP_WIDTH = 0x100;
    private static final String HUE_BITMAP_STRING = "data\\definitions\\hues\\hue%04d.bmp";
    private static final String WEAR_STRING = "Not letting a %u(sq %u) wear a %u(sq %u)\n";
    private static final String GET_OBJECT_DEFINITION = "UOTileArt::GetObjectDefinition";
    // Equipment layers: 0x1a draw-order entries; layer 0x19 is the mount, layer 0x1e is skipped.
    private static final long EQUIPMENT_LAYER_COUNT = 0x1a;
    private static final long MOUNT_LAYER = 0x19;
    private static final long SKIPPED_LAYER = 0x1e;
    // Actions 9..14 (0xe) are the weapon attack actions collected for an equipped weapon.
    private static final long FIRST_ATTACK_ACTION = 0x9;
    private static final long LAST_ATTACK_ACTION = 0xe;
    private static final String READ_FILE = "UOAssets::ReadFileFromPath";

    private static final long PACKET_CHARACTER_ANIMATION = 0x6e;
    private static final long PACKET_NEW_CHARACTER_ANIMATION = 0xe2;

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
    private final List<Block> blocks = new ArrayList<>();
    private final Map<String, Long> vtables = new LinkedHashMap<>();
    private final Map<Long, Proposal> proposals = new TreeMap<>();
    private final List<String> anomalies = new ArrayList<>();
    private final List<String> kept = new ArrayList<>();
    private final List<String> labels = new ArrayList<>();
    private final List<String> anchors = new ArrayList<>();
    private final List<String> packetRows = new ArrayList<>();
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
        loadBlocks();
        for (String cls : CLASSES) {
            long vt = rttiVtable(cls);
            vtables.put(cls, vt);
            anchors.add("RTTI `" + cls + "` vftable " + hexOrNone(vt));
        }

        Map<String, Function> seq = sequenceAsset();
        Map<String, Function> def = definitionAsset();
        Map<String, Long> runtime = settings(apply);
        sequenceRuntime(seq, runtime);
        assets(seq, def);
        components(seq);
        frames();
        packets(seq, runtime);

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

    // ---- AnimationSequence.uop: factory, parser and the methods the parser calls ----

    private Map<String, Function> sequenceAsset() {
        Map<String, Function> found = new HashMap<>();
        long factoryVt = vtables.get(SEQUENCE_FACTORY);
        Function create = factoryVt < 0 ? null : slotFunction(factoryVt, FACTORY_CREATE);
        propose(create, SEQUENCE_FACTORY + SCOPE + "Create", "asset factory vtable +4 (asset type 0x0c,"
            + " build/animationsequence/%08d.bin): UOAnimationSequence::Parse, asset resolvers on failure", false);
        if (create == null) {
            return found;
        }
        Function parse = null;
        for (Function callee : realCalls(create)) {
            if (callsNamed(callee, PARSE_COLLECTION)) {
                parse = callee;
            }
        }
        propose(parse, SEQUENCE + SCOPE + "Parse", "u32 id, 8x{i8,i8}, 8x{i16,i16}; u32 action count x {u32 action,"
            + " u32 frame count, i32 alias (-1 none), f32 fps, 8x{i8 x,i8 y}, 8x{i16,i16}, u32 n x {u32 frame, effect"
            + " collection}, u32 n x u32 skipped frame}; u32 group count x {i8 key, i8 parent, u32 default action,"
            + " u32 n x {i8 key, u32 action}, u32 n x {u8 move, u32 action, u32 alternate}, u32 n x {i8 type, u8 sub,"
            + " u32 m, m x u32 action}}", false);
        if (parse == null) {
            return found;
        }
        found.put("parse", parse);
        Function ctor = null;
        List<Function> writers = vtableWriters(SEQUENCE);
        for (Function callee : realCalls(parse)) {
            if (writers.contains(callee)) {
                ctor = callee;
            }
        }
        propose(ctor, SEQUENCE + SCOPE + SEQUENCE, "writes the UOAnimationSequence vftable (object 0xc4 bytes);"
            + " called by Parse", false);
        if (ctor == null) {
            return found;
        }
        List<Function> after = calleesAfter(parse, ctor);
        Function addAction = after.size() > 0 ? after.get(0) : null;
        Function addAlias = after.size() > 1 ? after.get(1) : null;
        Function hasAction = null;
        Function nextBase = null;
        if (addAction != null && addAlias != null) {
            List<Function> a = realCalls(addAction);
            List<Function> b = realCalls(addAlias);
            List<Function> extra = new ArrayList<>(a);
            extra.removeAll(b);
            if (!a.isEmpty() && !b.isEmpty() && a.get(0).equals(b.get(0)) && extra.size() == 1) {
                hasAction = a.get(0);
                nextBase = extra.get(0);
            } else {
                anomalies.add("action adders " + entry(addAction) + " / " + entry(addAlias) + " do not share their"
                    + " first callee or differ by more than one callee");
                addAction = null;
                addAlias = null;
            }
        }
        propose(addAction, SEQUENCE + SCOPE + "AddAction", "(id, frame count, fps) for alias == -1: record +0x20"
            + " frame count, +0x28 base frame = NextBaseFrame(), +0x2c fps; a duplicate id is an error unless the"
            + " existing record is empty; first new call after the constructor in Parse", false);
        propose(addAlias, SEQUENCE + SCOPE + "AddAliasAction", "(id, alias) for alias != -1: record +0x24 alias;"
            + " second new call after the constructor in Parse", false);
        propose(hasAction, SEQUENCE + SCOPE + "HasAction", "(action): find in the action map (+0xb8) != end;"
            + " first callee of both adders", false);
        propose(nextBase, SEQUENCE + SCOPE + "NextBaseFrame", "max(1, base + 5 * frame count) over the records"
            + " added so far: 5 stored directions per action, 1-based running frame index in file order; the callee"
            + " AddAction has and AddAliasAction lacks", false);
        found.put("hasAction", hasAction);
        found.put("nextBase", nextBase);

        Function addEffects = singleCallerCallAfter(parse, single(functionsNamed(PARSE_COLLECTION),
            PARSE_COLLECTION));
        propose(addEffects, SEQUENCE + SCOPE + "AddFrameEffects", "(action, frame, collection): record +0x30"
            + " map<u32 frame, effect collection>; first single-caller callee after ParseEffectDefinitionCollection"
            + " in Parse", false);
        Function addSkipped = addEffects == null ? null : singleCallerCallAfter(parse, addEffects);
        propose(addSkipped, SEQUENCE + SCOPE + "AddSkippedFrame", "(action, frame): record +0x3c map<u32 frame,"
            + " bool> = true; next single-caller callee in Parse", false);
        Function actionAt = null;
        if (addEffects != null && addSkipped != null) {
            List<Function> a = realCalls(addEffects);
            List<Function> b = realCalls(addSkipped);
            if (!a.isEmpty() && !b.isEmpty() && a.get(0).equals(b.get(0))) {
                actionAt = a.get(0);
            } else {
                anomalies.add("AddFrameEffects / AddSkippedFrame do not share their first callee");
            }
        }
        propose(actionAt, SEQUENCE + SCOPE + "ActionAt", "(action): action map (+0xb8) operator[] (inserts an"
            + " empty record when missing); first callee of AddFrameEffects and AddSkippedFrame", false);
        found.put("actionAt", actionAt);

        Function lookupCtor = null;
        Function getOrCreate = null;
        List<Function> lookupWriters = vtableWriters(LOOKUP);
        for (Function callee : after) {
            for (Function inner : realCalls(callee)) {
                if (lookupWriters.contains(inner)) {
                    getOrCreate = callee;
                    lookupCtor = inner;
                }
            }
        }
        propose(lookupCtor, LOOKUP + SCOPE + LOOKUP, "writes the FrameSetLookup vftable (0x30 bytes: +4 map<i8,"
            + " u32> transition actions, +0x10 map<u8, {action, alternate}> movement actions, +0x1c map<{i8, u8},"
            + " vector<u32>> typed actions, +0x28 parent, +0x2c default action)", false);
        propose(getOrCreate, SEQUENCE + SCOPE + "GetOrCreateGroup", "(i8 key): group map (+4) entry, creates a"
            + " FrameSetLookup when missing; Parse callee calling the FrameSetLookup constructor", false);
        Function getGroup = null;
        int index = getOrCreate == null ? -1 : after.indexOf(getOrCreate);
        if (index > 0) {
            getGroup = after.get(index - 1);
        }
        Function groupFind = getGroup == null ? null : singleCallee(getGroup);
        Function hasGroup = null;
        if (groupFind != null) {
            List<Function> others = callers(groupFind);
            others.remove(getGroup);
            hasGroup = single(others, "HasGroup (other caller of the group map find)");
        } else if (getGroup != null) {
            anomalies.add("GetGroup candidate " + entry(getGroup) + " has no single callee");
            getGroup = null;
        }
        propose(getGroup, SEQUENCE + SCOPE + "GetGroup", "(i8 key): FrameSetLookup of a group or the shared empty"
            + " one; Parse uses it to link a group's parent (i8 parent != -1); call before GetOrCreateGroup", false);
        propose(hasGroup, SEQUENCE + SCOPE + "HasGroup", "(i8 key): group map (+4) contains key; other caller of"
            + " the group map find", false);
        found.put("getGroup", getGroup);
        found.put("hasGroup", hasGroup);
        return found;
    }

    // ---- AnimationDefinition.uop ----

    private Map<String, Function> definitionAsset() {
        Map<String, Function> found = new HashMap<>();
        long factoryVt = vtables.get(DEFINITION_FACTORY);
        Function create = factoryVt < 0 ? null : slotFunction(factoryVt, FACTORY_CREATE);
        Function readFrame = factoryVt < 0 ? null : slotFunction(factoryVt, DEFINITION_FACTORY_READ_FRAME);
        propose(create, DEFINITION_FACTORY + SCOPE + "Create", "asset factory vtable +4 (asset type 0x0b,"
            + " build/animationdefinition/%08d.bin): Parse, asset resolvers on failure", false);
        propose(readFrame, DEFINITION_FACTORY + SCOPE + "ReadFrame", "vtable +0x10: u32 tile id, u8 effect count,"
            + " count x ParseEffectDefinition into a BinaryUOEffectDefinitionCollection, then adds the frame", false);
        if (create == null) {
            return found;
        }
        List<Function> defWriters = vtableWriters(DEFINITION);
        Function parse = null;
        Function ctor = null;
        for (Function callee : realCalls(create)) {
            for (Function inner : realCalls(callee)) {
                if (defWriters.contains(inner)) {
                    parse = callee;
                    ctor = inner;
                }
            }
        }
        propose(parse, DEFINITION + SCOPE + "Parse", "u32 id (+4), u32 start frame set (+8), u32 second frame set"
            + " (+0xc), f32 (+0x14), then ParseFrameSets; Create callee calling the UOAnimationDefinition"
            + " constructor", false);
        propose(ctor, DEFINITION + SCOPE + DEFINITION, "writes the UOAnimationDefinition vftable (0x48 bytes);"
            + " also used by the mobile animation builder", false);
        found.put("ctor", ctor);
        if (parse == null) {
            return found;
        }
        List<Function> setWriters = vtableWriters(FRAME_SET);
        Function parseSets = null;
        Function setCtor = null;
        for (Function callee : realCalls(parse)) {
            for (Function inner : realCalls(callee)) {
                if (setWriters.contains(inner)) {
                    parseSets = callee;
                    setCtor = inner;
                }
            }
        }
        propose(parseSets, DEFINITION + SCOPE + "ParseFrameSets", "u32 count x {u32 key, i32 next (-1 none), f32"
            + " frame interval, u32 frame count, frame count x factory ReadFrame}; adds each set, then sets the"
            + " start keys and links the next sets", false);
        propose(setCtor, FRAME_SET + SCOPE + FRAME_SET, "writes the UOAnimationFrameSet vftable (0x44 bytes)",
            false);
        found.put("setCtor", setCtor);
        if (parseSets == null || setCtor == null) {
            return found;
        }
        List<Function> after = calleesAfter(parseSets, setCtor);
        Function timing = after.isEmpty() ? null : after.get(0);
        if (timing != null && callers(timing).size() != 2) {
            anomalies.add("GetSharedTiming candidate " + entry(timing) + " has " + callers(timing).size() + " callers");
            timing = null;
        }
        propose(timing, FRAME_SET + SCOPE + "GetSharedTiming", "(f32 frame interval): interned timing object from a"
            + " map keyed by the interval; first call after the frame set constructor in ParseFrameSets", true);
        found.put("timing", timing);
        // AddFrameSet: called by ParseFrameSets and by one other function that builds a UOAnimationDefinition.
        Function addSet = null;
        for (Function callee : after) {
            List<Function> c = callers(callee);
            c.remove(parseSets);
            if (c.size() == 1 && !c.get(0).equals(parse) && ctor != null && realCalls(c.get(0)).contains(ctor)) {
                addSet = callee;
            }
        }
        propose(addSet, DEFINITION + SCOPE + "AddFrameSet", "(frame set): inserts by key into the map (+0x18);"
            + " called by ParseFrameSets and the mobile animation builder", false);
        found.put("addSet", addSet);
        Function mapKey = addSet == null || realCalls(addSet).isEmpty() ? null : realCalls(addSet).get(0);
        Function getSet = null;
        Function setStart = null;
        for (Function callee : after) {
            if (callee.equals(addSet) || mapKey == null || !realCalls(callee).contains(mapKey)) {
                continue;
            }
            if (callers(callee).size() == 1) {
                setStart = callee;
            } else {
                getSet = callee;
            }
        }
        propose(getSet, DEFINITION + SCOPE + "GetFrameSet", "(key): frame set map (+0x18) lookup; ParseFrameSets"
            + " callee sharing AddFrameSet's map helper, with other callers (the animation components)", false);
        propose(setStart, DEFINITION + SCOPE + "SetStartFrameSets", "(start, second, f32): both keys must exist;"
            + " +8 start, +0xc second, +0x14 value (>= 0); single-caller ParseFrameSets callee sharing AddFrameSet's"
            + " map helper", false);
        List<Function> linkers = new ArrayList<>();
        for (Function callee : after) {
            if (writesField(callee, FRAME_SET_NEXT_FIELD)) {
                linkers.add(callee);
            }
        }
        propose(single(linkers, "SetNext (writes +0x38)"), FRAME_SET + SCOPE + "SetNext", "frame set +0x38 next"
            + " frame set (shared_ptr +0x3c), from the i32 next key", false);
        found.put("addSet", addSet);
        return found;
    }

    // ---- settings: idleAnimation, frameSetRestriction, flying_animations ----

    private Map<String, Long> settings(boolean apply) throws Exception {
        Map<String, Long> runtime = new HashMap<>();
        Map<Long, String> stringAt = new HashMap<>();
        for (String s : SETTINGS) {
            for (long at : findBytes((s + "\0").getBytes(StandardCharsets.US_ASCII), true)) {
                stringAt.put(at, s);
            }
        }
        Function parser = null;
        for (long at : stringAt.keySet()) {
            if (!stringAt.get(at).equals(RESTRICTION_SETTING)) {
                continue;
            }
            for (Function f : referencingFunctions(at)) {
                if (callsNamed(f, "_strtoul")) {
                    parser = f;
                }
            }
        }
        propose(parser, "UOSettings" + SCOPE + "ParseOptimizationSettings", "reads idleAnimation (bool),"
            + " frameSetRestriction (u32), cacheSize (u32), flying_animations (bool) and display_all_house_contents"
            + " from the settings file (Lua SystemData.Settings.Optimization); user of the strings that calls"
            + " _strtoul", true);
        if (parser == null) {
            return runtime;
        }
        Map<String, Long> source = new LinkedHashMap<>();
        String pending = null;
        for (Instruction i : listing.getInstructions(parser.getBody(), true)) {
            for (Reference r : i.getReferencesFrom()) {
                long to = r.getToAddress().getOffset();
                if (!r.getToAddress().isMemoryAddress() || isExec(to)) {
                    continue;
                }
                if (stringAt.containsKey(to)) {
                    pending = stringAt.get(to);
                } else if (pending != null && !source.containsKey(pending)) {
                    source.put(pending, to);
                    pending = null;
                }
            }
        }
        String[][] sourceLabels = {
            {IDLE_SETTING, "g_UOSettingIdleAnimation", "settings idleAnimation (bool, default 1)"},
            {RESTRICTION_SETTING, "g_UOSettingFrameSetRestriction", "settings frameSetRestriction (u32, combo"
                + " 'Animation' 0..2)"},
            {CACHE_SETTING, "g_UOSettingCacheSize", "settings cacheSize (u32)"},
            {FLYING_SETTING, "g_UOSettingFlyingAnimations", "settings flying_animations (bool, Lua"
                + " bEnableFlyingAnimation, default 1); also read directly by SelectAnimationGroup"},
        };
        String[][] runtimeLabels = {
            {IDLE_SETTING, "g_UOIdleAnimation", "run-time copy of idleAnimation: 0 -> stand actions show one frame"},
            {RESTRICTION_SETTING, "g_UOFrameSetRestriction", "run-time copy of frameSetRestriction: < 2 -> typed"
                + " action variant 0; 0 -> sub type ignored (0xff)"},
            {FLYING_SETTING, "g_UOFlyingAnimations", "run-time copy of flying_animations: 0 -> action 19 resolves"
                + " to 0, actions 61..63 use the fps of action - 60"},
        };
        for (String[] l : sourceLabels) {
            label(source.getOrDefault(l[0], -1L), l[1], l[2], apply);
        }
        Set<Function> copiers = null;
        Map<String, Long> copies = new HashMap<>();
        for (String[] l : runtimeLabels) {
            Long src = source.get(l[0]);
            Map<Function, Long> found = src == null ? Map.of() : copiedTo(src);
            if (copiers == null) {
                copiers = new LinkedHashSet<>(found.keySet());
            } else {
                copiers.retainAll(found.keySet());
            }
            for (Map.Entry<Function, Long> e : found.entrySet()) {
                copies.put(l[0] + "@" + e.getKey().getEntryPoint(), e.getValue());
            }
        }
        Function copier = copiers == null ? null : single(new ArrayList<>(copiers), "settings copier");
        if (copier != null) {
            anchors.add("settings copied to their run-time globals in " + entry(copier) + " (not renamed; it also"
                + " registers the game packet handlers)");
            for (String[] l : runtimeLabels) {
                Long dst = copies.get(l[0] + "@" + copier.getEntryPoint());
                runtime.put(l[0], dst);
                label(dst == null ? -1 : dst, l[1], l[2], apply);
            }
            runtime.put("copier", copier.getEntryPoint().getOffset());
        }
        return runtime;
    }

    // MOV reg, [source] followed within a few instructions by MOV [global], reg (global != source).
    private Map<Function, Long> copiedTo(long source) {
        Map<Function, Long> found = new LinkedHashMap<>();
        for (Reference r : getReferencesTo(toAddr(source))) {
            if (!r.getReferenceType().isRead()) {
                continue;
            }
            Instruction i = listing.getInstructionAt(r.getFromAddress());
            if (i == null || !i.getMnemonicString().equals("MOV") || i.getOpObjects(0).length != 1
                    || !(i.getOpObjects(0)[0] instanceof Register reg)) {
                continue;
            }
            Instruction n = i.getNext();
            for (int k = 0; k < COPY_WINDOW && n != null; k++, n = n.getNext()) {
                if (!n.getMnemonicString().equals("MOV")) {
                    continue;
                }
                Long dst = absoluteMemoryOperand(n, 0);
                Object[] src = n.getOpObjects(1);
                if (dst != null && dst != source && src.length == 1 && reg.equals(src[0])) {
                    Function f = getFunctionContaining(i.getAddress());
                    if (f != null) {
                        found.put(f, dst);
                    }
                    break;
                }
            }
        }
        return found;
    }

    // ---- sequence lookups used at run time ----

    private void sequenceRuntime(Map<String, Function> seq, Map<String, Long> runtime) {
        Function hasAction = seq.get("hasAction");
        Function find = hasAction == null ? null : singleCallee(hasAction);
        Function findAction = null;
        if (find != null) {
            List<Function> others = callers(find);
            others.remove(hasAction);
            findAction = single(others, "FindAction (other caller of the action map find)");
        }
        propose(findAction, SEQUENCE + SCOPE + "FindAction", "(action): action record pointer from the map (+0xb8);"
            + " other caller of the map find HasAction uses", false);
        if (findAction == null) {
            return;
        }
        Long idle = runtime.get(IDLE_SETTING);
        Long flying = runtime.get(FLYING_SETTING);
        Long restriction = runtime.get(RESTRICTION_SETTING);
        Function directionFrames = null;
        List<Function> flyingReaders = new ArrayList<>();
        for (Function caller : callers(findAction)) {
            if (idle != null && readsGlobal(caller, idle)) {
                directionFrames = caller;
            }
            if (flying != null && readsGlobal(caller, flying)) {
                flyingReaders.add(caller);
            }
        }
        Function resolve = null;
        Function fps = null;
        if (flyingReaders.size() == 2) {
            Function a = flyingReaders.get(0);
            Function b = flyingReaders.get(1);
            resolve = realCalls(a).contains(b) ? b : realCalls(b).contains(a) ? a : null;
            fps = resolve == null ? null : resolve.equals(a) ? b : a;
        }
        if (resolve == null) {
            anomalies.add("FindAction callers reading g_UOFlyingAnimations: " + names(flyingReaders));
        }
        propose(resolve, SEQUENCE + SCOPE + "ResolveAction", "(action): alias -1 -> the action; alias != -1 ->"
            + " the alias when that record has alias -1 (one level only); action 19 is treated as an alias of 0 when"
            + " g_UOFlyingAnimations is 0; a missing action, missing target or alias chain logs and falls back to the"
            + " lowest action id with alias -1", false);
        propose(fps, SEQUENCE + SCOPE + "GetActionFps", "(action): record +0x2c fps of the resolved action, 16.0"
            + " when missing; actions 61..63 use action - 60 when g_UOFlyingAnimations is 0; frame interval = 1/fps",
            false);
        propose(directionFrames, SEQUENCE + SCOPE + "GetDirectionFrames", "(EAX out mirrored; sequence, action, dir, out"
            + " count) -> first frame: resolved action; dir < 3 -> mirrored, dir = 6 - dir; first = base + (dir - 3)"
            + " * frame count; count = frame count, or 1 for stand actions when g_UOIdleAnimation is 0; missing -> 0", false);
        Function frameCount = null;
        if (resolve != null) {
            List<Function> others = new ArrayList<>();
            for (Function caller : callers(resolve)) {
                if (!caller.equals(fps) && !caller.equals(directionFrames) && realCalls(caller).contains(findAction)) {
                    others.add(caller);
                }
            }
            frameCount = single(others, "GetFrameCount (other caller of ResolveAction)");
        }
        propose(frameCount, SEQUENCE + SCOPE + "GetFrameCount", "(action): record +0x20 frame count of the resolved"
            + " action", false);
        Function isStand = null;
        if (directionFrames != null) {
            List<Function> rest = new ArrayList<>(realCalls(directionFrames));
            rest.remove(resolve);
            rest.remove(hasAction);
            rest.remove(findAction);
            isStand = single(rest, "IsStandAction (remaining callee of GetDirectionFrames)");
        }
        propose(isStand, SEQUENCE + SCOPE + "IsStandAction", "(action): any group's FrameSetLookup::IsStandAction;"
            + " the remaining callee of GetDirectionFrames", false);
        Function lookupStand = null;
        if (isStand != null) {
            List<Function> own = new ArrayList<>();
            for (Function callee : realCalls(isStand)) {
                if (callers(callee).size() == 1) {
                    own.add(callee);
                }
            }
            lookupStand = single(own, "FrameSetLookup::IsStandAction (single-caller callee)");
        }
        propose(lookupStand, LOOKUP + SCOPE + "IsStandAction", "(action): movement map (+0x10) key 0 (stand) action"
            + " or alternate == action", false);
        Function moveFind = lookupStand == null ? null : singleCallee(lookupStand);
        Function moveAction = null;
        if (moveFind != null) {
            List<Function> others = callers(moveFind);
            others.remove(lookupStand);
            moveAction = single(others, "GetMoveAction (other caller of the movement map find)");
        }
        propose(moveAction, LOOKUP + SCOPE + "GetMoveAction", "(u8 move 0 stand / 1 walk / 2 run, bool alternate):"
            + " movement map (+0x10) entry action or alternate, searching the parent chain (+0x28); else default"
            + " action (+0x2c)", false);

        List<Function> restrictionReaders = new ArrayList<>();
        if (restriction != null) {
            restrictionReaders = referencingFunctions(restriction);
            Long copier = runtime.get("copier");
            restrictionReaders.removeIf(f -> copier != null && f.getEntryPoint().getOffset() == copier);
        }
        Function getGroup = seq.get("getGroup");
        Function typed = null;
        Function transition = null;
        Function mapPacket = null;
        // MapCharacterAnimationAction calls exactly GetGroup and GetTypedAction.
        for (Function reader : restrictionReaders) {
            for (Function caller : callers(reader)) {
                List<Function> c = realCalls(caller);
                if (getGroup != null && c.size() == 2 && c.contains(getGroup)) {
                    typed = reader;
                    mapPacket = caller;
                }
            }
        }
        for (Function reader : restrictionReaders) {
            if (!reader.equals(typed)) {
                transition = reader;
            }
        }
        if (restrictionReaders.size() != 2) {
            anomalies.add("g_UOFrameSetRestriction readers: " + names(restrictionReaders));
        }
        propose(typed, LOOKUP + SCOPE + "GetTypedAction", "(i8 type, u8 sub, u32 variant): typed map (+0x1c) at"
            + " (type, sub), then (type, 0xff), then the parent chain; list[variant % size]; else default action;"
            + " g_UOFrameSetRestriction < 2 -> variant 0, == 0 -> sub 0xff", false);
        propose(transition, LOOKUP + SCOPE + "GetTransitionAction", "(i8 key, out action): transition map (+4) at"
            + " key, searching the parent chain; keys 0/1 (state flag set/clear) + 4 when flying", true);
        propose(mapPacket, SEQUENCE + SCOPE + "MapCharacterAnimationAction", "(0x6E action): 200..269 -> GetTypedAction(group"
            + " 1, type 11, sub 0xff, variant = the packet action) when the sequence has groups; else 200..209 -> 16,"
            + " 210..269 -> 17; other actions pass through", false);
        seq.put("typed", typed);
        seq.put("transition", transition);
        seq.put("mapPacket", mapPacket);
        seq.put("resolve", resolve);
        seq.put("fps", fps);
        seq.put("directionFrames", directionFrames);
        seq.put("frameCount", frameCount);
    }

    // ---- asset getters, the mobile builder and the frame asset loader ----

    private void assets(Map<String, Function> seq, Map<String, Function> def) {
        List<Function> managerUsers = new ArrayList<>();
        for (Function f : functionsReferencingSymbol(ASSET_MANAGER)) {
            managerUsers.add(f);
        }
        List<Function> seqGetters = new ArrayList<>();
        List<Function> defGetters = new ArrayList<>();
        List<Function> frameLoaders = new ArrayList<>();
        for (Function f : managerUsers) {
            if (usesScalar(f, SEQUENCE_ASSET_TYPE << ASSET_TYPE_SHIFT)) {
                seqGetters.add(f);
            }
            if (usesScalar(f, DEFINITION_ASSET_TYPE << ASSET_TYPE_SHIFT)) {
                defGetters.add(f);
            }
            if (usesScalar(f, FRAME_ASSET_TYPE << ASSET_TYPE_SHIFT) && callsNamed(f, GET_ASSET)) {
                frameLoaders.add(f);
            }
        }
        propose(single(seqGetters, "AnimationSequence getter"), NAMESPACE + SCOPE + "GetAnimationSequence",
            "(out, id): synchronous asset request {id, 0x0c000000} through g_UOAssetManager", false);
        propose(single(defGetters, "AnimationDefinition getter"), NAMESPACE + SCOPE + "GetAnimationDefinition",
            "(out, id): synchronous asset request {id, 0x0b000000} through g_UOAssetManager", false);
        propose(single(frameLoaders, "AnimationFrame loader"), NAMESPACE + SCOPE + "GetAnimationFrameSet",
            "(body, action): UOAssets::GetAsset(id = body * 0x100 + action, type 0x10000000):"
            + " build/animationframe/%06d/%02d.bin", false);

        Function directionFrames = seq.get("directionFrames");
        Function build = directionFrames == null ? null : single(callers(directionFrames), "mobile animation builder");
        propose(build, NAMESPACE + SCOPE + "BuildMobileAnimation", "(object): sequence id = GetBodyType (vtable"
            + " +0x70); cache map keyed by body -> {definition, sequence}; requests the sequence (status 1 aborts);"
            + " builds a UOAnimationDefinition with one frame set per action and direction: frames base + i,"
            + " interval 1/fps", false);
        if (build == null) {
            return;
        }
        Function requestSeq = null;
        for (Function callee : realCalls(build)) {
            if (usesScalar(callee, SEQUENCE_ASSET_TYPE << ASSET_TYPE_SHIFT)) {
                requestSeq = callee;
            }
        }
        propose(requestSeq, NAMESPACE + SCOPE + "RequestAnimationSequence", "(request, id): pushes {id, 0x0c000000}"
            + " into a UOResourceRequest", false);
        Function setInterval = null;
        Function timing = def.get("timing");
        for (Function callee : realCalls(build)) {
            if (timing != null && realCalls(callee).contains(timing)) {
                setInterval = callee;
            }
        }
        propose(setInterval, FRAME_SET + SCOPE + "SetFrameInterval", "(f32 seconds): GetSharedTiming, stored at"
            + " +0x30/+0x34; the builder passes 1/GetActionFps", false);
        if (requestSeq == null) {
            return;
        }
        List<Function> others = callers(requestSeq);
        others.remove(build);
        Function seated = single(others, "other RequestAnimationSequence caller");
        Function hasGroup = seq.get("hasGroup");
        if (seated != null && hasGroup != null && !realCalls(seated).contains(hasGroup)) {
            anomalies.add(entry(seated) + " does not call HasGroup");
            seated = null;
        }
        propose(seated, NAMESPACE + SCOPE + "HasSeatedGroup", "(object): unmounted mobile, requests its sequence and"
            + " returns HasGroup(6) (the seated group)", true);
        Function vectorGrow = singleCallee(requestSeq);
        Function requestDef = null;
        if (vectorGrow != null) {
            List<Function> found = new ArrayList<>();
            for (Function caller : callers(vectorGrow)) {
                if (usesScalar(caller, DEFINITION_ASSET_TYPE << ASSET_TYPE_SHIFT)) {
                    found.add(caller);
                }
            }
            requestDef = single(found, "RequestAnimationDefinition");
        }
        propose(requestDef, NAMESPACE + SCOPE + "RequestAnimationDefinition", "(request, id): pushes {id,"
            + " 0x0b000000} into a UOResourceRequest", false);
        if (requestDef == null) {
            return;
        }
        long tileVt = vtables.get(TILE_COMPONENT);
        Function tileInit = tileVt < 0 ? null : slotFunction(tileVt, COMPONENT_INITIALIZE);
        for (Function caller : callers(requestDef)) {
            List<String> callerNames = new ArrayList<>();
            for (Function c : callers(caller)) {
                callerNames.add(c.getName(true));
            }
            if (callerNames.contains(PROJECTILE_INIT)) {
                propose(caller, "UOProjectileEffect" + SCOPE + "LoadAnimationDefinition", "(item id): synchronous"
                    + " AnimationDefinition request; found -> animated tile component, else a static one", false);
            } else if (callerNames.contains(TILE_EFFECT_INIT)) {
                propose(caller, "UOTileEffect" + SCOPE + "LoadAnimationDefinition", "(item id): synchronous"
                    + " AnimationDefinition request; found -> animated tile component, else a static one", false);
            } else if (tileInit != null && realCalls(tileInit).contains(caller)) {
                propose(caller, TILE_COMPONENT + SCOPE + "LoadAnimationDefinition", "(item id): synchronous"
                    + " AnimationDefinition request for the tile animation component", false);
            } else {
                anomalies.add("unclassified RequestAnimationDefinition caller " + entry(caller));
            }
        }
    }

    // ---- animation components ----

    private void components(Map<String, Function> seq) {
        long mobVt = vtables.get(MOB_COMPONENT);
        long tileVt = vtables.get(TILE_COMPONENT);
        if (mobVt >= 0) {
            propose(slotFunction(mobVt, COMPONENT_INITIALIZE), MOB_COMPONENT + SCOPE + "Initialize", "vtable +4:"
                + " BuildMobileAnimation for the owner, then selects the group and the starting action", true);
            Function update = slotFunction(mobVt, COMPONENT_UPDATE);
            Function frameCount = seq.get("frameCount");
            if (update != null && frameCount != null && !realCalls(update).contains(frameCount)) {
                anomalies.add(MOB_COMPONENT + " +0x18 does not call GetFrameCount");
            }
            propose(update, MOB_COMPONENT + SCOPE + "Update", "vtable +0x18 (dt): advances the frame time; frames in"
                + " the action's skipped set are passed over", true);
            propose(slotFunction(mobVt, COMPONENT_PLAY_ACTION), MOB_COMPONENT + SCOPE + "PlayAction", "vtable +0x5c"
                + " (out, action): called with the mapped 0x6E action and the 0xE2 typed action", true);
            Function actionAt = seq.get("actionAt");
            if (update != null && actionAt != null) {
                Function skipped = null;
                for (Function callee : realCalls(update)) {
                    if (realCalls(callee).contains(actionAt)) {
                        skipped = callee;
                    }
                }
                propose(skipped, SEQUENCE + SCOPE + "IsFrameSkipped", "(action, frame): record +0x3c map lookup;"
                    + " the mob Update advances past such frames without showing them", false);
            }
        }
        if (tileVt >= 0) {
            propose(slotFunction(tileVt, COMPONENT_INITIALIZE), TILE_COMPONENT + SCOPE + "Initialize", "vtable +4:"
                + " loads the AnimationDefinition of the item; start set (+8), second set (+0xc), value (+0x14)",
                true);
            propose(slotFunction(tileVt, COMPONENT_UPDATE), TILE_COMPONENT + SCOPE + "Update", "vtable +0x18 (dt)",
                true);
            propose(slotFunction(tileVt, COMPONENT_PLAY_ACTION), TILE_COMPONENT + SCOPE + "PlayAction", "vtable"
                + " +0x5c (out, frame set key)", true);
        }

        Function mobFrame = single(stringUsers(MOUNTED_STRING), "user of '" + MOUNTED_STRING + "'");
        propose(mobFrame, MOB_COMPONENT + SCOPE + "UpdateFrame", "maps the global frame to (action, frame) with"
            + " FrameToAction; per-direction offsets for the mount/attachments: action i8 x/y (-128 -> 0) plus the"
            + " i16 pair (header unless the action's is not -0x8000), y - 15; a PositionOffset effect of the frame"
            + " overrides the i8 offsets", true);
        Function actionAt = seq.get("actionAt");
        if (mobFrame == null || actionAt == null) {
            return;
        }
        Map<Long, String> getters = Map.of(OFFSET_X_FIELD, "GetOffsetX", OFFSET_Y_FIELD, "GetOffsetY",
            EXTRA_X_FIELD, "GetExtraOffsetX", EXTRA_Y_FIELD, "GetExtraOffsetY");
        for (Function callee : realCalls(mobFrame)) {
            if (!realCalls(callee).contains(actionAt)) {
                continue;
            }
            long field = indexedDisplacement(callee);
            if (getters.containsKey(field)) {
                propose(callee, SEQUENCE + SCOPE + getters.get(field), "(action, dir): record +" + hexShort(field)
                    + "[dir]", false);
            } else {
                propose(callee, SEQUENCE + SCOPE + "GetFrameEffects", "(action, frame): effect collection of the"
                    + " frame from the record +0x30 map (empty when none)", false);
            }
        }
        // FrameToAction walks the action map with the same iterator step as NextBaseFrame.
        Function nextBase = seq.get("nextBase");
        Function iterator = nextBase == null ? null : singleCallee(nextBase);
        Function frameToAction = null;
        if (iterator != null) {
            for (Function caller : callers(iterator)) {
                if (realCalls(mobFrame).contains(caller)) {
                    frameToAction = caller;
                }
            }
        }
        propose(frameToAction, SEQUENCE + SCOPE + "FrameToAction", "(global frame, out action, out frame): the"
            + " record whose base <= frame; local frame = frame - base + 1 (the AnimationFrame entry's second u16)",
            false);
    }

    // ---- AnimationFrame asset, pixel decoding, palette and hue tables ----

    private void frames() {
        long factoryVt = vtables.get(FRAME_FACTORY);
        Function create = factoryVt < 0 ? null : slotFunction(factoryVt, FACTORY_CREATE);
        propose(create, FRAME_FACTORY + SCOPE + "Create", "asset factory vtable +4 (asset type 0x10): allocates a"
            + " UOMobFrameSet over the blob", false);
        if (create != null) {
            List<Function> ctors = new ArrayList<>();
            for (Function w : vtableWriters(MOB_FRAME_SET)) {
                if (realCalls(create).contains(w)) {
                    ctors.add(w);
                }
            }
            propose(single(ctors, "UOMobFrameSet constructor"), MOB_FRAME_SET + SCOPE + MOB_FRAME_SET, "reads u32"
                + " signature (+4), version (+8), total size (+0xc); keeps the blob (+0x10)", false);
        }

        // The two RLE decoders: no calls, packed red/blue blend mask, 0x80 run test; the atlas builder calls both.
        List<Function> decoders = new ArrayList<>();
        for (Function f : scalarUsersInProgram(PACKED_RED_BLUE_MASK)) {
            if (calls(f).isEmpty() && usesScalar(f, RLE_RUN_FLAG)) {
                decoders.add(f);
            }
        }
        List<Function> atlases = new ArrayList<>();
        for (Function d : decoders) {
            for (Function caller : callers(d)) {
                int both = 0;
                for (Function other : decoders) {
                    both += realCalls(caller).contains(other) ? 1 : 0;
                }
                if (both == decoders.size() && decoders.size() > 1 && !atlases.contains(caller)) {
                    atlases.add(caller);
                }
            }
        }
        Function atlas = single(atlases, "frame atlas builder (caller of every RLE decoder)");
        propose(atlas, NAMESPACE + SCOPE + "BuildFrameAtlas", "draws the layers (body, equipment) in order into a"
            + " zeroed BGRA8888 NiPixelData (power-of-2 size); layer +0xc mirrored -> mirrored decoder; palette"
            + " **(layer +0x10)", false);
        if (atlas == null) {
            return;
        }
        Function normal = null;
        Function mirrored = null;
        for (Function d : decoders) {
            if (callers(d).size() > 1) {
                normal = d;
            } else {
                mirrored = d;
            }
        }
        propose(normal, NAMESPACE + SCOPE + "DecodeFrameRle", "(EAX frame entry, palette, canvas, pitch): byte c <"
            + " 0x80 skips c pixels; else flag byte F: F>>4 edge pixel, c-0x80 solid pixels, F&15 edge pixel; edge"
            + " = per-channel (src*f + dst*(16-f))>>4", false);
        propose(mirrored, NAMESPACE + SCOPE + "DecodeFrameRleMirrored", "same RLE, columns written right to left"
            + " from -x0-1 (x -> -1-x)", false);
        List<Function> bounds = new ArrayList<>();
        for (Function c : realCalls(atlas)) {
            boolean shared = !c.isExternal() && calls(c).isEmpty();
            for (Function caller : callers(atlas)) {
                shared &= realCalls(caller).contains(c);
            }
            if (shared && !decoders.contains(c)) {
                bounds.add(c);
            }
        }
        propose(single(bounds, "layer bounds helper"), NAMESPACE + SCOPE + "GetLayerBounds", "union of the layers'"
            + " frame boxes (mirrored: [-x1, -x0]) minus the layer offsets +0x18/+0x1c, 1-pixel margin", true);
        if (normal != null) {
            List<Function> masks = new ArrayList<>();
            for (Function caller : callers(normal)) {
                if (caller.equals(atlas)) {
                    continue;
                }
                for (Function c : realCalls(caller)) {
                    if (!c.equals(normal) && calls(c).isEmpty() && usesScalar(c, RLE_RUN_FLAG)
                            && !usesScalar(c, PACKED_RED_BLUE_MASK)) {
                        masks.add(c);
                    }
                }
            }
            propose(single(masks, "RLE mask decoder"), NAMESPACE + SCOPE + "DecodeFrameMask", "same RLE into a"
                + " 1-byte-per-pixel mask; every coded pixel (solid and edge) is marked", true);
        }

        Function updateFrame = proposed(MOB_COMPONENT + SCOPE + "UpdateFrame");
        Function getFrameSet = proposed(NAMESPACE + SCOPE + "GetAnimationFrameSet");
        Function frameToAction = proposed(SEQUENCE + SCOPE + "FrameToAction");
        if (updateFrame == null) {
            return;
        }
        List<Function> palettes = new ArrayList<>();
        List<Function> loaders = new ArrayList<>();
        for (Function c : realCalls(updateFrame)) {
            if (usesScalar(c, SPECIAL_HUE)) {
                palettes.add(c);
            }
            if (getFrameSet != null && realCalls(c).contains(getFrameSet)) {
                loaders.add(c);
            }
        }
        Function palette = single(palettes, "palette builder (UpdateFrame callee using 0x4001)");
        propose(palette, NAMESPACE + SCOPE + "BuildFramePalette", "(ECX hue, EAX blob): entry [b0,b1,b2,b3] ->"
            + " A=0xff, R=b0, G=b1, B=b2; with a hue table and b3 == 0: table[b0] (b3 != 0 keeps the colour);"
            + " hue 0x4001 (shadow): black; inv = 255 - trunc(b0*.34 + b2*.07 + b1*.59); over the palette min, max"
            + " and integer mean of inv: x = 0 when mean-min or max-mean is 0, 0.5 at the mean, below it"
            + " (inv-min)/(mean-min)*0.8*0.5, above it t = (inv-mean)/(max-mean), ((1-t)*0.2 + t + 1)*0.5;"
            + " alpha = trunc(x*130 + 60); output BGRA", false);
        if (palette != null) {
            List<Function> tables = new ArrayList<>();
            for (Function c : realCalls(palette)) {
                if (usesScalar(c, SPECIAL_HUE)) {
                    tables.add(c);
                }
            }
            propose(single(tables, "hue table getter"), HUES + SCOPE + "GetHueTable", "(ECX hue): 0x4001 -> 0x9c3;"
                + " no 0xc000 bits -> hue-1; else (hue & 0x3fff)-1 unless only 0x4000 is set; map lookup of 256"
                + " u32 entries", false);
        }
        Function loader = single(loaders, "layer frame-set loader (UpdateFrame callee calling GetAnimationFrameSet)");
        propose(loader, NAMESPACE + SCOPE + "LoadLayerFrameSet", "(owner, out, id, any-action flag, sequence, ...):"
            + " id >= 10000000 loses 10000000; GetAnimationFrameSet(id, action); flag set and missing -> the first"
            + " of actions 0..0x33 that exists", false);
        if (loader != null && frameToAction != null) {
            List<Function> actions = new ArrayList<>();
            for (Function c : realCalls(loader)) {
                if (realCalls(c).contains(frameToAction)) {
                    actions.add(c);
                }
            }
            propose(single(actions, "layer action helper"), NAMESPACE + SCOPE + "GetLayerAction", "frame-set time"
                + " -> global frame -> FrameToAction: the action whose frames the layer loads", true);
        }
        Function layers = single(stringUsers(WEAR_STRING), "user of the wear-check string");
        propose(layers, MOB_COMPONENT + SCOPE + "BuildLayers", "body layer frames id = GetAppearance (definition"
            + " +0xbc) of the object type, overridden by GetAnimationAppearance (+0xc0); none -> body 0; ghost bodies"
            + " 402/403/607/608/694/695 and objects with mobile flag 0x80 (GetMobileFlags) get hue 0x3e3; equipment"
            + " needs body_type == sequence stored id", true);
        if (layers != null) {
            equipment(layers);
        }
        statusHue(palette);

        Function hueLoad = single(stringUsers(HUE_BITMAP_STRING), "user of the hue bitmap path");
        propose(hueLoad, HUES + SCOPE + "LoadHueTables", "reads data\\definitions\\hues\\huenames.csv and"
            + " hue%04d.bmp; key = hue number - 1", false);
        if (hueLoad != null) {
            List<Function> bitmaps = new ArrayList<>();
            for (Function c : realCalls(hueLoad)) {
                if (callsNamed(c, READ_FILE) && usesScalar(c, HUE_BITMAP_WIDTH)) {
                    bitmaps.add(c);
                }
            }
            propose(single(bitmaps, "hue bitmap reader"), HUES + SCOPE + "LoadHueBitmap", "(path): 32-bpp BMP,"
                + " width 256, height 2; the second row (+0x400) gives the 256 entries, B,G,R,A -> A<<24|B<<16|G<<8|R"
                + " (RGBA from the low byte)", false);
        }
    }

    // ---- equipment layers, mobile flags and the shadow hue selector ----

    private void equipment(Function layers) {
        Function getFrameSet = proposed(NAMESPACE + SCOPE + "GetAnimationFrameSet");
        List<Function> serials = new ArrayList<>();
        List<Function> attacks = new ArrayList<>();
        for (Function c : realCalls(layers)) {
            if (usesScalar(c, MOUNT_LAYER) && usesScalar(c, SKIPPED_LAYER)) {
                serials.add(c);
            }
            if (getFrameSet != null && realCalls(c).contains(getFrameSet) && usesScalar(c, FIRST_ATTACK_ACTION)
                    && usesScalar(c, LAST_ATTACK_ACTION)) {
                attacks.add(c);
            }
        }
        propose(single(serials, "layer item getter (BuildLayers callee using layers 0x19 and 0x1e)"), NAMESPACE
            + SCOPE + "GetDrawLayerItemSerial", "(EAX object, draw-order index, out is-mount): layer = object"
            + " +0xf8[index]; out = layer == 0x19 (mount); layer 0x1e -> 0; else the u64 serial of the layer item"
            + " (object +8 + layer*8)", true);
        propose(single(attacks, "attack action collector (BuildLayers callee, actions 9..14)"), MOB_COMPONENT
            + SCOPE + "CollectWeaponAttackActions", "(EAX weapon item, component, ...): clears the set at component"
            + " +0xe0, then adds every action 9..14 whose frame set exists for the weapon appearance (+0xbc,"
            + " overridden by +0xc0); called when an item with tiledata flag 2 (weapon) changes", false);

        List<Function> orders = new ArrayList<>();
        for (Function f : functionsNamed(GET_OBJECT_DEFINITION)) {
            for (Function caller : callers(f)) {
                boolean all = usesScalar(caller, EQUIPMENT_LAYER_COUNT);
                for (long slot : DEFINITION_RACE_GENDER_SLOTS) {
                    all &= callsThroughSlot(caller, slot);
                }
                if (all) {
                    orders.add(caller);
                }
            }
        }
        propose(single(orders, "equipment draw-order builder (race/gender slots and 0x1a layers)"), NAMESPACE + SCOPE
            + "BuildEquipmentLayerOrder", "(object): the 26-entry layer draw orders at +0xf8 and +0x160 from the"
            + " object definition's race and gender (HasRace +0xc8 / GetRace +0xd0, default 1; HasGender +0xcc /"
            + " GetGender +0xd4, default 0)", true);

        long mobVt = vtables.get(MOB_COMPONENT);
        Function refresh = mobVt < 0 ? null : slotFunction(mobVt, COMPONENT_REFRESH_MOVE_ACTION);
        propose(refresh, MOB_COMPONENT + SCOPE + "RefreshMoveAction", "vtable +0x68: GetMoveAction(group, 0,"
            + " GetLayer2AlternateAnimation()); on a change in group 6 (seated) keeps the chair object at +0xb4 and"
            + " turns to its allowed sitting directions", true);
        if (refresh != null) {
            List<Function> alternates = new ArrayList<>();
            for (Function c : realCalls(refresh)) {
                if (callsThroughSlot(c, OBJECT_GET_ALTERNATE_ANIMATION)) {
                    alternates.add(c);
                }
            }
            propose(single(alternates, "layer-2 alternate animation getter"), MOB_COMPONENT + SCOPE
                + "GetLayer2AlternateAnimation", "(ECX component): equipment component (6) serial of layer 2 ->"
                + " object vtable +0x78 GetAlternateAnimation; false when there is no such item", false);
        }

        long objectVt = vtables.get(OBJECT);
        if (objectVt >= 0) {
            propose(slotFunction(objectVt, OBJECT_GET_MOBILE_FLAGS), OBJECT + SCOPE + "GetMobileFlags", "vtable"
                + " +0xa0: returns +0x6c, the flags byte of 0x77/0x78; BuildLayers forces hue 0x3e3 when bit 0x80"
                + " (hidden) is set", true);
            propose(slotFunction(objectVt, OBJECT_SET_MOBILE_FLAGS), OBJECT + SCOPE + "SetMobileFlags", "vtable"
                + " +0xa4 (flags): stores +0x6c and notifies change 7 (vtable +0x1c) when it differs", true);
        }
    }

    // The other users of hue 0x4001: the bonded-status handler sets it, the object updater keeps it.
    private void statusHue(Function palette) {
        Function table = proposed(HUES + SCOPE + "GetHueTable");
        List<Function> called = new ArrayList<>();
        List<Function> registered = new ArrayList<>();
        for (Function f : scalarUsersInProgram(SPECIAL_HUE)) {
            if (f.equals(palette) || f.equals(table)) {
                continue;
            }
            (callers(f).isEmpty() ? registered : called).add(f);
        }
        Function handler = single(registered, "registered user of hue 0x4001 (bonded status handler)");
        propose(handler, "UOMobileStatus" + SCOPE + "OnBondedStatus", "0xBF sub-command 0x19 callback: reads the"
            + " packet, stores its flag byte at mobile status (component 5) +0x6d and, when set, SetHue(0x4001)"
            + " (vtable +0x54) so the bonded pet draws as a shadow", true);
        if (handler != null) {
            List<Function> calls = realCalls(handler);
            propose(calls.isEmpty() ? null : calls.get(0), "UOPacket" + SCOPE + "Read_0xBF_19_BondedStatus",
                "first call of the bonded-status handler: serial and the flag byte", true);
        }
        propose(single(called, "called user of hue 0x4001 (object hue updater)"), "UOObjectManager" + SCOPE
            + "ApplyGraphicAndHue", "(..., graphic, hue, ...): graphic 0x2006 (corpse) -> id + 10000000; when the"
            + " mobile status +0x6d byte is set the hue becomes 0x4001; SetHue (vtable +0x54) when it changed;"
            + " called by the 0x77/0x78-style object packets", true);
    }

    // MOV reg, [reg + displacement] followed within SLOT_CALL_WINDOW instructions by CALL reg.
    private boolean callsThroughSlot(Function f, long displacement) {
        for (Instruction i : listing.getInstructions(f.getBody(), true)) {
            if (!i.getMnemonicString().equals("MOV") || i.getNumOperands() != 2 || i.getOpObjects(0).length != 1
                    || !i.getDefaultOperandRepresentation(1).contains("[")) {
                continue;
            }
            boolean match = false;
            for (Object o : i.getOpObjects(1)) {
                match |= o instanceof Scalar sc && sc.getUnsignedValue() == displacement;
            }
            Instruction n = i.getNext();
            for (int k = 0; match && n != null && k < SLOT_CALL_WINDOW; k++, n = n.getNext()) {
                if (n.getMnemonicString().equals("CALL") && n.getOpObjects(0).length == 1
                        && i.getOpObjects(0)[0].equals(n.getOpObjects(0)[0])) {
                    return true;
                }
            }
        }
        return false;
    }

    // ---- 0x6E and 0xE2 ----

    private void packets(Map<String, Function> seq, Map<String, Long> runtime) {
        Long copierAt = runtime.get("copier");
        Function registrar = copierAt == null ? null : functions.getFunctionAt(toAddr(copierAt));
        if (registrar == null) {
            anomalies.add("packet registration function not found");
            return;
        }
        Map<Long, Function> handlers = new TreeMap<>();
        Function last = null;
        for (Instruction i : listing.getInstructions(registrar.getBody(), true)) {
            if (i.getFlowType().isCall()) {
                continue;
            }
            for (Reference r : i.getReferencesFrom()) {
                Function target = r.getReferenceType().isFlow() ? null : functions.getFunctionAt(r.getToAddress());
                if (target != null) {
                    last = target;
                }
            }
            if (i.getMnemonicString().equals("MOV") && i.getDefaultOperandRepresentation(0).contains("ESP")
                    && i.getOpObjects(1).length == 1 && i.getOpObjects(1)[0] instanceof Scalar s) {
                Instruction n = i.getNext();
                if (n != null && n.getFlowType().isCall() && last != null) {
                    handlers.put(s.getUnsignedValue(), last);
                    last = null;
                }
            }
        }
        int confirmed = 0;
        for (Map.Entry<Long, Function> e : handlers.entrySet()) {
            String name = e.getValue().getName();
            if (name.startsWith(RECEIVE_PREFIX)) {
                String expected = RECEIVE_PREFIX + String.format("%02X", e.getKey()) + "_";
                if (name.startsWith(expected)) {
                    confirmed++;
                } else {
                    anomalies.add("registration pairs packet " + hexShort(e.getKey()) + " with " + entry(e.getValue()));
                }
            }
        }
        anchors.add("packet registration in " + entry(registrar) + ": " + handlers.size() + " handlers paired, "
            + confirmed + " confirmed by existing Receive_ names");

        Function h6e = handlers.get(PACKET_CHARACTER_ANIMATION);
        Function mapPacket = seq.get("mapPacket");
        propose(h6e, "UOObjectManager" + SCOPE + "Receive_0x6E_CharacterAnimation", "UOObjectManager packet callback"
            + " for 0x6E: parses the packet, finds the mobile, OnCharacterAnimation", false);
        if (h6e != null) {
            List<Function> calls = realCalls(h6e);
            propose(calls.isEmpty() ? null : calls.get(0), "UOPacket" + SCOPE + "Read_0x6E_CharacterAnimation",
                "u32 serial, u16 action, u16 frame count, u16 repeat count, u8 forward, u8 repeat, u8 delay (only"
                + " the action is used); first call of the handler", false);
            Function apply = null;
            for (Function callee : calls) {
                if (mapPacket != null && realCalls(callee).contains(mapPacket)) {
                    apply = callee;
                }
            }
            propose(apply, MOB_COMPONENT + SCOPE + "OnCharacterAnimation", "(packet): PlayAction (+0x5c) of"
                + " MapCharacterAnimationAction(u16 action); frame count, repeat, direction and delay are ignored",
                false);
        }
        Function hE2 = handlers.get(PACKET_NEW_CHARACTER_ANIMATION);
        Function typed = seq.get("typed");
        propose(hE2, "UOObjectManager" + SCOPE + "Receive_0xE2_NewCharacterAnimation", "UOObjectManager packet"
            + " callback for 0xE2: parses the packet, finds the mobile, OnNewCharacterAnimation", false);
        if (hE2 != null) {
            List<Function> calls = realCalls(hE2);
            propose(calls.isEmpty() ? null : calls.get(0), "UOPacket" + SCOPE + "Read_0xE2_NewCharacterAnimation",
                "u32 serial, u16 type, u16 sub type, u8 variant; first call of the handler", false);
            Function apply = null;
            for (Function callee : calls) {
                if (typed != null && realCalls(callee).contains(typed)) {
                    apply = callee;
                }
            }
            propose(apply, MOB_COMPONENT + SCOPE + "OnNewCharacterAnimation", "(packet): group ="
                + " SelectAnimationGroup, action = FrameSetLookup::GetTypedAction(type, sub, variant) (type 0 in war"
                + " mode may use the weapon's attack action), then PlayAction (+0x5c)", false);
            Function hasGroup = seq.get("hasGroup");
            Function select = null;
            if (apply != null && hasGroup != null) {
                for (Function callee : realCalls(apply)) {
                    if (realCalls(callee).contains(hasGroup)) {
                        select = callee;
                    }
                }
            }
            propose(select, NAMESPACE + SCOPE + "SelectAnimationGroup", "(object): 2 mounted (when the group"
                + " exists), 4/5 by a mobile state, 3 flying (setting flying_animations and component state), 0 war"
                + " mode, 6 seated, else 1 peace", false);
            if (select != null && typed != null) {
                Function transition = seq.get("transition");
                if (transition != null) {
                    List<Function> found = new ArrayList<>();
                    for (Function caller : callers(transition)) {
                        if (realCalls(caller).contains(select)) {
                            found.add(caller);
                        }
                    }
                    propose(single(found, "transition action player"), MOB_COMPONENT + SCOPE
                        + "PlayTransitionAction", "(key): GetTransitionAction(SelectAnimationGroup(), key) ->"
                        + " PlayAction (+0x5c); keys from the state-flag (+0x5c) and flying-mode changes", true);
                }
            }
        }
    }

    // ---- naming ----

    private Function proposed(String path) {
        for (Proposal p : proposals.values()) {
            if (p.path.equals(path)) {
                return functions.getFunctionAt(toAddr(p.address));
            }
        }
        return null;
    }

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

    // ---- memory and RTTI ----

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

    // Functions referencing the vftable (constructors, destructors), excluding the functions in its slots.
    private List<Function> vtableWriters(String cls) {
        long vt = vtables.getOrDefault(cls, -1L);
        List<Function> writers = new ArrayList<>();
        if (vt < 0) {
            return writers;
        }
        for (Reference r : getReferencesTo(toAddr(vt))) {
            Function f = getFunctionContaining(r.getFromAddress());
            if (f != null && !writers.contains(f)) {
                writers.add(f);
            }
        }
        return writers;
    }

    // ---- call graph ----

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

    // Target of the MSVC SEH prolog helper: MOV EAX, imm (handler) followed by CALL at the entry.
    private Function sehPrologTarget(Function f) {
        Instruction first = listing.getInstructionAt(f.getEntryPoint());
        if (first == null || !first.getMnemonicString().equals("MOV") || first.getOpObjects(0).length != 1
                || !(first.getOpObjects(0)[0] instanceof Register r) || !r.getName().equals("EAX")) {
            return null;
        }
        Instruction next = first.getNext();
        if (next == null || !next.getFlowType().isCall() || next.getFlows().length == 0) {
            return null;
        }
        return functions.getFunctionAt(next.getFlows()[0]);
    }

    // Distinct callees in call order, without the SEH prolog helper.
    private List<Function> realCalls(Function f) {
        List<Function> out = new ArrayList<>(new LinkedHashSet<>(calls(f)));
        Function seh = sehPrologTarget(f);
        if (seh != null) {
            out.remove(seh);
        }
        return out;
    }

    private Function singleCallee(Function f) {
        List<Function> c = realCalls(f);
        return c.size() == 1 ? c.get(0) : null;
    }

    // Distinct callees (without the SEH helper) first called after the first call to target.
    private List<Function> calleesAfter(Function f, Function target) {
        List<Function> out = new ArrayList<>();
        Set<Function> before = new LinkedHashSet<>();
        Function seh = sehPrologTarget(f);
        boolean seen = false;
        for (Function callee : calls(f)) {
            seen |= callee.equals(target);
            if (!seen) {
                before.add(callee);
            } else if (!callee.equals(seh) && !callee.equals(target) && !before.contains(callee)
                    && !out.contains(callee)) {
                out.add(callee);
            }
        }
        return out;
    }

    private Function singleCallerCallAfter(Function f, Function first) {
        if (first == null) {
            return null;
        }
        boolean seen = false;
        for (Function callee : calls(f)) {
            if (seen && callers(callee).size() == 1) {
                return callee;
            }
            seen |= callee.equals(first);
        }
        return null;
    }

    private List<Function> callers(Function f) {
        List<Function> out = new ArrayList<>(f.getCallingFunctions(monitor));
        out.sort((a, b) -> a.getEntryPoint().compareTo(b.getEntryPoint()));
        return out;
    }

    private boolean callsNamed(Function f, String name) {
        for (Function c : realCalls(f)) {
            if (c.getName(true).equals(name) || c.getName().equals(name)) {
                return true;
            }
        }
        return false;
    }

    private List<Function> functionsNamed(String path) {
        List<Function> out = new ArrayList<>();
        for (Function f : functions.getFunctions(true)) {
            if (f.getName(true).equals(path)) {
                out.add(f);
            }
        }
        return out;
    }

    private List<Function> functionsReferencingSymbol(String name) {
        List<Function> out = new ArrayList<>();
        for (Symbol s : symbols.getGlobalSymbols(name)) {
            for (Function f : referencingFunctions(s.getAddress().getOffset())) {
                if (!out.contains(f)) {
                    out.add(f);
                }
            }
        }
        if (out.isEmpty()) {
            anomalies.add("no users of `" + name + "`");
        }
        return out;
    }

    private Function single(List<Function> list, String what) {
        if (list.size() == 1) {
            return list.get(0);
        }
        anomalies.add(what + ": " + list.size() + " candidates " + names(list));
        return null;
    }

    // ---- instruction patterns ----

    private boolean usesScalar(Function f, long value) {
        for (Instruction i : listing.getInstructions(f.getBody(), true)) {
            for (int op = 0; op < i.getNumOperands(); op++) {
                for (Object o : i.getOpObjects(op)) {
                    if (o instanceof Scalar s && (s.getUnsignedValue() & DWORD_MASK) == value) {
                        return true;
                    }
                }
            }
        }
        return false;
    }

    // Functions containing an instruction with the scalar operand, over the whole program.
    private List<Function> scalarUsersInProgram(long value) {
        List<Function> out = new ArrayList<>();
        for (Instruction i : listing.getInstructions(true)) {
            for (int op = 0; op < i.getNumOperands(); op++) {
                for (Object o : i.getOpObjects(op)) {
                    if (o instanceof Scalar s && (s.getUnsignedValue() & DWORD_MASK) == value) {
                        Function f = getFunctionContaining(i.getAddress());
                        if (f != null && !out.contains(f)) {
                            out.add(f);
                        }
                    }
                }
            }
        }
        return out;
    }

    private boolean readsGlobal(Function f, long address) {
        for (Instruction i : listing.getInstructions(f.getBody(), true)) {
            for (Reference r : i.getReferencesFrom()) {
                if (r.getReferenceType().isRead() && r.getToAddress().getOffset() == address) {
                    return true;
                }
            }
        }
        return false;
    }

    // Displacement of the first [base + index*4 + disp] operand (per-direction int[8] fields).
    private long indexedDisplacement(Function f) {
        for (Instruction i : listing.getInstructions(f.getBody(), true)) {
            for (int op = 0; op < i.getNumOperands(); op++) {
                String text = i.getDefaultOperandRepresentation(op);
                if (!text.contains("[") || !text.contains("*0x4")) {
                    continue;
                }
                for (Object o : i.getOpObjects(op)) {
                    if (o instanceof Scalar s && s.getUnsignedValue() != POINTER_SIZE) {
                        return s.getUnsignedValue();
                    }
                }
            }
        }
        return -1;
    }

    // MOV [reg + disp], ... with a non-stack base register.
    private boolean writesField(Function f, long displacement) {
        for (Instruction i : listing.getInstructions(f.getBody(), true)) {
            if (!i.getMnemonicString().equals("MOV") || i.getNumOperands() < 1) {
                continue;
            }
            String text = i.getDefaultOperandRepresentation(0);
            if (!text.contains("[") || text.contains("ESP") || text.contains("EBP")) {
                continue;
            }
            for (Object o : i.getOpObjects(0)) {
                if (o instanceof Scalar s && s.getUnsignedValue() == displacement) {
                    return true;
                }
            }
        }
        return false;
    }

    private List<Function> stringUsers(String text) {
        List<Function> users = new ArrayList<>();
        for (long at : findBytes((text + "\0").getBytes(StandardCharsets.US_ASCII), true)) {
            Data d = listing.getDataAt(toAddr(at));
            if (d == null || !d.hasStringValue()) {
                continue;
            }
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

    // ---- report ----

    private void report(PrintWriter out, boolean apply) {
        int probable = 0;
        for (Proposal p : proposals.values()) {
            probable += p.probable ? 1 : 0;
        }
        out.println("# animation code names");
        out.println();
        out.println("Program: " + currentProgram.getName());
        out.println("SHA-256: " + currentProgram.getExecutableSHA256());
        out.println("Mode: " + (apply ? "analysis and " + APPLY_FLAG : "analysis only"));
        out.println();
        out.println("| Item | Count |");
        out.println("|---|---:|");
        out.println("| Function names proposed | " + proposals.size() + " |");
        out.println("| of which probable | " + probable + " |");
        out.println("| Global labels | " + labels.size() + " |");
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
