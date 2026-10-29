// Name the house customization piece code of UOSA.exe: the build/multicollection/housing.bin asset
// (UOCustomHousingPieces) factory and parser, the MultiCollection.uop multi factory and parser, the builder that
// turns the parsed pieces into the house customization tables, the piece queries and their filter, the
// WindowData.CustomizedHousing publisher and the tile / multi selection helpers. With --apply, rename DEFAULT
// functions, add the UO_HOUSING tag, plate comments and two global labels.
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
import java.util.LinkedHashMap;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.TreeMap;

public class NameHousingCode extends GhidraScript {
    private static final String APPLY_FLAG = "--apply";
    private static final String TAG = "UO_HOUSING";
    private static final String TAG_COMMENT = "housing.bin (UOCustomHousingPieces) and MultiCollection.uop assets,"
        + " house customization piece tables, WindowData.CustomizedHousing and piece selection";
    private static final String COMMENT_PREFIX = "NameHousingCode: ";
    private static final String PROBABLE = " (probable)";
    private static final String SCOPE = "::";

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

    // Classes (RTTI) and namespaces for new names.
    private static final String PIECES_FACTORY = "UOCustomHousingPiecesBinaryFactory";
    private static final String PIECES = "UOCustomHousingPieces";
    private static final String MULTI_FACTORY = "UOMultiCollectionBinaryFactory";
    private static final String MULTI = "UOMultiCollection";
    private static final String[] CLASSES = {PIECES_FACTORY, PIECES, MULTI_FACTORY, MULTI};
    private static final String HOUSE = "UOHouseCustomization";
    private static final String FILTER = "HousingPieceFilter";
    private static final String REQUEST = "UOResourceRequest";

    // Asset factory vtable slot and asset keys (type << 24).
    private static final int FACTORY_CREATE = 0x4;
    private static final long PIECES_ASSET_KEY = 0x14000000L;
    private static final long MULTI_ASSET_KEY = 0x13000000L;

    // Layout facts used as locators.
    private static final long MULTI_PART_SIZE = 0x1c;
    private static final long HOUSE_OBJECT_SIZE = 0x877b8L;
    private static final long HOUSE_SELECTION_IS_MULTI = 0x87704L;
    private static final int PIECE_LIST_READS = 3;
    private static final int PARSER_LOCAL_HELPERS = 5;
    private static final int LOCAL_HELPER_MAX_CALLERS = 2;
    private static final int BUILDER_LOCAL_CALLEES = 15;
    private static final int FILTER_HELPER_COUNT = 4;
    private static final int FILTER_HELPER_MAX_BYTES = 32;

    // Names applied by other scripts.
    private static final String STREAM_READ = "UOAssets::StreamRead";
    private static final String ASSET_MANAGER = "g_UOAssetManager";
    private static final String LUA_TILE_SELECTED = "UOLua::SetHousingTileSelected";
    private static final String LUA_MULTI_SELECTED = "UOLua::SetHousingMultiSelected";
    private static final String PREVIEW_TILE_STRING = "previewTile";

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

        piecesAsset();
        multiAsset();
        Function build = builder();
        selection();
        globals(build, apply);

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

    // ---- housing.bin: factory, parser and the parser's local helpers ----

    private void piecesAsset() {
        long factoryVt = vtables.get(PIECES_FACTORY);
        Function create = factoryVt < 0 ? null : slotFunction(factoryVt, FACTORY_CREATE);
        propose(create, PIECES_FACTORY + SCOPE + "Create", "asset factory vtable +4 (asset type 0x14,"
            + " build/multicollection/housing.bin): Parse, asset resolvers on failure", false);
        if (create == null) {
            return;
        }
        Function[] parseAndCtor = callerOfVtableWriter(create, PIECES);
        Function parse = parseAndCtor[0];
        Function ctor = parseAndCtor[1];
        propose(parse, PIECES + SCOPE + "Parse", "u32 tile type count x {u32 tile type, u32 piece count x {u32"
            + " category, u32 group, u32 feature mask, u32 tooltip cliloc, 3 x {u32 n, n x {u32 slot, u32 id}}"
            + " (solid tiles +0x10, multis +0x20, window tiles +0x30)}}; error unless the blob end is reached", false);
        propose(ctor, PIECES + SCOPE + PIECES, "writes the UOCustomHousingPieces vftable (object 0x10 bytes: +4"
            + " map<u32 tile type, vector<piece 0x40 bytes>>); called by Parse", false);
        if (parse == null || ctor == null) {
            return;
        }
        List<Function> local = new ArrayList<>();
        for (Function callee : calleesAfter(parse, ctor)) {
            if (callers(callee).size() <= LOCAL_HELPER_MAX_CALLERS && !isNamed(callee, STREAM_READ)) {
                local.add(callee);
            }
        }
        if (local.size() != PARSER_LOCAL_HELPERS || countCalls(parse, local.get(1)) != PIECE_LIST_READS
                || !callsNamed(local.get(1), STREAM_READ)) {
            anomalies.add("housing parser helpers: expected " + PARSER_LOCAL_HELPERS + " with the second called "
                + PIECE_LIST_READS + " times and reading the stream, found " + names(local));
            return;
        }
        propose(local.get(0), PIECES + SCOPE + "InitPiece", "constructs an empty 0x40-byte piece (+0 category, +4 group,"
            + " +8 feature mask, +0xc tooltip, +0x10/+0x20/+0x30 vector<{u32 slot, u32 id}>); first local helper"
            + " after the constructor in Parse", false);
        propose(local.get(1), PIECES + SCOPE + "ReadSlotList", "u32 n, n x {u32 slot, u32 id} into a piece"
            + " vector; called three times per piece by Parse", false);
        propose(local.get(2), PIECES + SCOPE + "PushPiece", "vector<piece>::push_back of the 0x40-byte piece;"
            + " third local helper in Parse", false);
        propose(local.get(3), PIECES + SCOPE + "TileTypeAt", "map<u32 tile type, vector<piece>> operator[] on"
            + " object +4; fourth local helper in Parse", false);
        propose(local.get(4), PIECES + SCOPE + "AssignPieces", "vector<piece> assignment of the pieces read for"
            + " one tile type into the map entry; fifth local helper in Parse", false);
    }

    // ---- MultiCollection.uop: factory, parser, part setter ----

    private void multiAsset() {
        long factoryVt = vtables.get(MULTI_FACTORY);
        Function create = factoryVt < 0 ? null : slotFunction(factoryVt, FACTORY_CREATE);
        propose(create, MULTI_FACTORY + SCOPE + "Create", "asset factory vtable +4 (asset type 0x13,"
            + " build/multicollection/%06d.bin): Parse, asset resolvers on failure", false);
        if (create == null) {
            return;
        }
        Function[] parseAndCtor = callerOfVtableWriter(create, MULTI);
        Function parse = parseAndCtor[0];
        Function ctor = parseAndCtor[1];
        propose(parse, MULTI + SCOPE + "Parse", "u32 multi id, u32 part count x {u16 tile, i16 x, i16 y, i16 z"
            + " (sign-extended), u8 not drawn, u8 flag2, u32 n, n x u32 string dictionary index (empty strings"
            + " dropped)}; error unless the blob end is reached", false);
        propose(ctor, MULTI + SCOPE + MULTI, "(id, part count): writes the UOMultiCollection vftable (object"
            + " 0x4c bytes: +4 id, +8 part count, +0xc parts[0x1c bytes], +0x2c first-part flag, +0x30/+0x3c"
            + " min/max xyz, +0x48 largest extent)", false);
        if (parse == null || ctor == null) {
            return;
        }
        Function setPart = singleCallerCallAfter(parse, ctor);
        if (setPart != null && !usesScalar(setPart, MULTI_PART_SIZE)) {
            anomalies.add("multi part setter candidate " + entry(setPart) + " does not use the part size");
            setPart = null;
        }
        propose(setPart, MULTI + SCOPE + "SetPart", "(EAX index, tile, x, y, z, not drawn, flag2, strings):"
            + " part +0 u16 tile, +2/+4/+6 i16 x/y/z, +8 u16 = 0, +0xa u8, +0xb u8, +0xc vector<string>; grows"
            + " the bounds; first single-caller callee after the constructor in Parse", false);
    }

    // ---- piece tables of the house customization ----

    private Function builder() {
        Function publish = single(stringUsers(PREVIEW_TILE_STRING), "users of `" + PREVIEW_TILE_STRING + "`");
        propose(publish, HOUSE + SCOPE + "PublishCustomizedHousing", "WindowData.CustomizedHousing.Type[tile"
            + " type].Category[category + 1] = {previewTile, tooltip, entitlement, entitlementTID, Group[group +"
            + " 1] = {Solid[slot].id, Window[slot].id, Multi[slot] = {id, appearance}}}; sole user of"
            + " \"previewTile\"", false);
        Function init = publish == null ? null : single(callers(publish), "callers of PublishCustomizedHousing");
        propose(init, HOUSE + SCOPE + "Initialize", "clears the selection, BuildPieceTables, registers the"
            + " housing Lua functions, PublishCustomizedHousing; run when the play state connects and from a world"
            + " reset event handler; sole caller of PublishCustomizedHousing", false);
        if (init == null || realCalls(init).isEmpty()) {
            return null;
        }
        Function build = realCalls(init).get(0);
        List<Function> local = new ArrayList<>();
        for (Function callee : realCalls(build)) {
            if (callers(callee).size() == 1) {
                local.add(callee);
            }
        }
        if (local.size() != BUILDER_LOCAL_CALLEES || !usesScalar(local.get(0), PIECES_ASSET_KEY)) {
            anomalies.add("piece table builder " + entry(build) + ": expected " + BUILDER_LOCAL_CALLEES
                + " single-caller callees, the first using " + hex(PIECES_ASSET_KEY) + ", found " + names(local));
            return null;
        }
        propose(build, HOUSE + SCOPE + "BuildPieceTables", "requests housing.bin {0, 0x14000000} synchronously;"
            + " for tile types 1..7, categories min..max, groups min..max (expansion filter 0x6f3bff) fills the"
            + " house maps +0x87734 (tile type -> category -> group), +0x87750 (tile id -> {tile type, slot}) and"
            + " +0x8776c (multi id -> part offsets and tiles); first callee of Initialize", false);
        String[][] names = {
            {REQUEST + SCOPE + "AddCustomHousingPiecesKey", "pushes the asset key {0, 0x14000000} (housing.bin)"
                + " into the request"},
            {HOUSE + SCOPE + "TileTypeEntryAt", "house +0x87734 map operator[] (tile type)"},
            {PIECES + SCOPE + "GetCategoryRange", "(filter, out min, out max): min/max piece category of the"
                + " filter's tile type over pieces whose feature mask matches; false when none"},
            {HOUSE + SCOPE + "CategoryEntryAt", "category map operator[]; entry +0x1c tooltip, +0x20"
                + " entitlement, +0x24 entitlement cliloc"},
            {PIECES + SCOPE + "GetGroupRange", "(filter, out min, out max): min/max group of the filter's"
                + " category; false when the category has no piece"},
            {PIECES + SCOPE + "GetFeatureMask", "(filter, out mask): feature mask of the last piece of the"
                + " category (any group); entitlement = (server feature flags & mask) != 0"},
            {PIECES + SCOPE + "GetTooltip", "(filter, out cliloc): the nonzero tooltip of the category; false"
                + " (the first is kept) when two pieces have one"},
            {FILTER + SCOPE + "Describe", "\"filter for component/category/group/expansions/part\" text with the"
                + " expansion names (AOW, EP1, 8th, 9th, SA); built after a failed query, then discarded"},
            {HOUSE + SCOPE + "GroupEntryAt", "group map operator[]; entry +0 Solid, +0x1c Window, +0x38 Multi"
                + " maps<slot, id>"},
            {PIECES + SCOPE + "GetSolidTiles", "(filter, out vector<{slot, id}>): piece +0x10 lists of the"
                + " matching pieces"},
            {HOUSE + SCOPE + "SlotAt", "map<u32 slot, u32 id> operator[] (Solid, Window and Multi maps of a"
                + " group)"},
            {HOUSE + SCOPE + "TileLookupAt", "house +0x87750 map<u32 tile id, {tile type, slot}> operator[]"},
            {PIECES + SCOPE + "GetWindowTiles", "(filter, out vector<{slot, id}>): piece +0x30 lists"},
            {PIECES + SCOPE + "GetMultis", "(filter, out vector<{slot, multi id}>): piece +0x20 lists"},
            {HOUSE + SCOPE + "LoadMultiAppearance", "(house, multi id): requests the MultiCollection entry"
                + " {id, 0x13000000}; house +0x8776c entry = offsets (+0, 10 x i32 xyz) and tiles (+0x78, 10 x"
                + " u32) of the first 10 drawn parts (+0xa == 0)"},
        };
        for (int i = 0; i < BUILDER_LOCAL_CALLEES; i++) {
            propose(local.get(i), names[i][0], names[i][1] + "; single-caller callee " + (i + 1) + " of"
                + " BuildPieceTables", false);
        }
        filterHelpers(local.get(9));
        multiQueries(local.get(BUILDER_LOCAL_CALLEES - 1));
        return build;
    }

    private void filterHelpers(Function solid) {
        List<Function> calls = realCalls(solid);
        if (calls.size() <= FILTER_HELPER_COUNT) {
            anomalies.add("GetSolidTiles has too few callees: " + names(calls));
            return;
        }
        String[][] names = {
            {"MatchesTileType", "filter +0 tile type (0 = any)"},
            {"MatchesCategory", "filter +4 category (-1 = any)"},
            {"MatchesGroup", "filter +8 group (-1 = any)"},
            {"MatchesSlot", "filter +0x10 slot (0 = any); filter +0xc is the expansion mask"},
        };
        for (int i = 0; i < FILTER_HELPER_COUNT; i++) {
            Function f = calls.get(i + 1);
            if (f.getBody().getNumAddresses() > FILTER_HELPER_MAX_BYTES) {
                anomalies.add("filter helper candidate " + entry(f) + " is larger than " + FILTER_HELPER_MAX_BYTES
                    + " bytes");
                continue;
            }
            propose(f, FILTER + SCOPE + names[i][0], names[i][1] + "; callee " + (i + 2) + " of GetSolidTiles",
                false);
        }
    }

    private void multiQueries(Function load) {
        List<Function> keys = new ArrayList<>();
        List<Function> getters = new ArrayList<>();
        for (Function callee : realCalls(load)) {
            if (usesScalar(callee, MULTI_ASSET_KEY)) {
                keys.add(callee);
            } else if (usesScalar(callee, MULTI_PART_SIZE) && callers(callee).size() > 1) {
                getters.add(callee);
            }
        }
        Function key = single(keys, "callees of LoadMultiAppearance using " + hex(MULTI_ASSET_KEY));
        Function getPart = single(getters, "shared callees of LoadMultiAppearance using the part size");
        propose(key, REQUEST + SCOPE + "AddMultiCollectionKey", "pushes the asset key {multi id, 0x13000000}"
            + " into the request; callee of LoadMultiAppearance", false);
        propose(getPart, MULTI + SCOPE + "GetPart", "(ECX out xyz, EAX index, out tile, out u16 +8, out u8 +0xa"
            + " not drawn, out u8 +0xb, out strings, out ok); callee of LoadMultiAppearance using the part size",
            false);
    }

    // ---- selection of a palette piece (CustomizeHouseWindow) ----

    private void selection() {
        Function tileLua = single(functionsNamed(LUA_TILE_SELECTED), LUA_TILE_SELECTED);
        Function multiLua = single(functionsNamed(LUA_MULTI_SELECTED), LUA_MULTI_SELECTED);
        if (tileLua == null || multiLua == null) {
            return;
        }
        List<Function> own = new ArrayList<>();
        for (Function callee : realCalls(tileLua)) {
            if (callers(callee).size() == 1) {
                own.add(callee);
            }
        }
        Function select = single(own, "single-caller callees of " + LUA_TILE_SELECTED);
        propose(select, HOUSE + SCOPE + "SelectPieceById", "(tile id): tile id in the +0x87750 lookup -> SelectTile"
            + " (tile type, id, slot); else multi id in the +0x8776c map -> SelectMulti; called by"
            + " SetHousingTileSelected", false);
        if (select == null) {
            return;
        }
        Function tile = null;
        Function multi = null;
        List<Function> multiCalls = realCalls(multiLua);
        for (Function callee : realCalls(select)) {
            if (!writesField(callee, HOUSE_SELECTION_IS_MULTI)) {
                continue;
            }
            if (multiCalls.contains(callee)) {
                multi = multi == null ? callee : null;
            } else {
                tile = tile == null ? callee : null;
            }
        }
        propose(tile, HOUSE + SCOPE + "SelectTile", "(tile type, tile id, slot): rejects types 0 and 8; sets"
            + " +0x876fc type, +0x876f8 id, +0x87700 slot, +0x87704 multi = 0 and builds the cursor", false);
        propose(multi, HOUSE + SCOPE + "SelectMulti", "(multi id): sets +0x876f8 id, +0x876fc type 1 (stairs),"
            + " +0x87704 multi = 1 and builds the multi cursor; also called by SetHousingMultiSelected", false);
    }

    // ---- globals ----

    private void globals(Function build, boolean apply) throws Exception {
        Set<Long> written = new LinkedHashSet<>();
        List<Function> allocators = functionsUsingScalar(HOUSE_OBJECT_SIZE);
        Function creator = single(allocators, "functions using the house object size " + hex(HOUSE_OBJECT_SIZE));
        if (creator != null) {
            for (Instruction i : listing.getInstructions(creator.getBody(), true)) {
                Long target = i.getMnemonicString().equals("MOV") ? absoluteMemoryOperand(i, 0) : null;
                if (target != null) {
                    written.add(target);
                }
            }
        }
        long house = written.size() == 1 ? written.iterator().next() : -1;
        if (creator != null && house < 0) {
            anomalies.add(entry(creator) + " writes " + written.size() + " globals, expected 1");
            creator = null;
        }
        propose(creator, HOUSE + SCOPE + "Create", "allocates the 0x877b8-byte house customization object once"
            + " and stores it in g_UOHouseCustomization", false);
        if (creator != null) {
            List<Function> own = new ArrayList<>();
            for (Function callee : realCalls(creator)) {
                if (callers(callee).size() == 1) {
                    own.add(callee);
                }
            }
            propose(single(own, "single-caller callees of the house creator"), HOUSE + SCOPE + HOUSE,
                "constructor of the 0x877b8-byte house customization object; called by Create", false);
        }
        label(house, "g_UOHouseCustomization", "UOHouseCustomization object (0x877b8 bytes); written by"
            + " UOHouseCustomization::Create", apply);

        Set<Long> read = new LinkedHashSet<>();
        long manager = symbolAddress(ASSET_MANAGER);
        if (build != null) {
            for (Instruction i : listing.getInstructions(build.getBody(), true)) {
                Long source = i.getMnemonicString().equals("MOV") ? absoluteMemoryOperand(i, 1) : null;
                if (source != null && source != manager) {
                    read.add(source);
                }
            }
        }
        long flags = read.size() == 1 ? read.iterator().next() : -1;
        if (build != null && flags < 0) {
            anomalies.add("BuildPieceTables reads " + read.size() + " globals besides " + ASSET_MANAGER + ", expected 1");
        }
        label(flags, "g_UOServerFeatureFlags", "0xB9 feature flags; BuildPieceTables: category entitlement ="
            + " (flags & piece feature mask) != 0", apply);
    }

    private long symbolAddress(String name) {
        for (Symbol s : symbols.getGlobalSymbols(name)) {
            return s.getAddress().getOffset();
        }
        anomalies.add("global `" + name + "` not found");
        return -1;
    }

    private List<Function> functionsUsingScalar(long value) {
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

    // The callee of f calling a writer of cls's vftable, and that writer; {null, null} when not exactly one.
    private Function[] callerOfVtableWriter(Function f, String cls) {
        List<Function> writers = vtableWriters(cls);
        Function caller = null;
        Function writer = null;
        int found = 0;
        for (Function callee : realCalls(f)) {
            for (Function inner : realCalls(callee)) {
                if (writers.contains(inner)) {
                    caller = callee;
                    writer = inner;
                    found++;
                }
            }
        }
        if (found != 1) {
            anomalies.add(entry(f) + ": " + found + " callees call a writer of the " + cls + " vftable");
            return new Function[] {null, null};
        }
        return new Function[] {caller, writer};
    }

    private int countCalls(Function f, Function target) {
        int n = 0;
        for (Function callee : calls(f)) {
            n += callee.equals(target) ? 1 : 0;
        }
        return n;
    }

    private static boolean isNamed(Function f, String path) {
        return f.getName(true).equals(path);
    }

    // ---- naming ----

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
        out.println("# housing code names");
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

    private static String hexOrNone(long value) {
        return value < 0 ? "not found" : hex(value);
    }
}
