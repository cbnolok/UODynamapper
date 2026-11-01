// Name the UOSA.exe (Enhanced Client) code on the path from "a mobile needs a body/action" to "its frame is a
// D3D9 texture": the asset manager cache (lookup, store, LRU sweep, eviction policy), the synchronous request
// loop, the cache-size setting, the per-frame composite texture cache, MobAnimTexture and the Gamebryo DX9
// source-texture creation it runs through.
// Address-based: one entry set per tested file, selected by SHA-256; any other program is refused. Every
// address is verified with the stated checks first; a failed check is reported, not named.
// Findings: results/uosa_anim_loading.md.
// @category ECResearch
import ghidra.app.script.GhidraScript;
import ghidra.program.model.address.Address;
import ghidra.program.model.data.StringDataInstance;
import ghidra.program.model.listing.Data;
import ghidra.program.model.listing.Function;
import ghidra.program.model.listing.FunctionTag;
import ghidra.program.model.listing.Instruction;
import ghidra.program.model.scalar.Scalar;
import ghidra.program.model.symbol.Namespace;
import ghidra.program.model.symbol.Reference;
import ghidra.program.model.symbol.SourceType;
import ghidra.program.model.symbol.Symbol;
import ghidra.program.model.symbol.SymbolTable;
import java.io.PrintWriter;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardOpenOption;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

public class NameEcAnimLoadingCode extends GhidraScript {
    private static final String APPLY_FLAG = "--apply";
    // Script arguments are split on whitespace, so a path with spaces is passed with %20.
    private static final String ENCODED_SPACE = "%20";
    private static final String UOSA_SHA256 = "5a5e9dc536c836d1bf10b6f86bb88f61a6ae078f519d4060d3bc47aecbebad50";
    private static final String TAG_LOADING = "UO_EC_ANIM_LOADING";
    private static final String TAG_LOADING_COMMENT =
        "EC mobile animation loading: asset cache, frame composite cache, MobAnimTexture, DX9 texture creation";
    private static final String COMMENT_PREFIX = "NameEcAnimLoadingCode: ";
    private static final String PROBABLE = " (probable)";
    private static final int HEX_RADIX = 16;

    private static final String ASSETS = "UOAssets";
    private static final String ANIMATIONS = "UOAnimations";
    private static final String APPLICATION = "UOApplication";
    private static final String SETTINGS = "UOSettings";
    private static final String MOB_TEXTURE = "MobAnimTexture";
    private static final String MOB_SHAPE = "MobAnimTriShape";
    private static final String MOB_SHADOW = "MobAnimTriShapeShadow";
    private static final String DX9_RENDERER = "NiDX9Renderer";
    private static final String DX9_TEXTURE_DATA = "NiDX9SourceTextureData";
    private static final String CLOCK = "UOClock";
    private static final String CACHE = "the asset cache (UOAssetManager +8: map, LRU list +0x28, bytes +0x40,"
        + " critical section +0xd4)";

    // Checks: "calls:ADDR" (the function references ADDR: call, pointer, jump or data),
    // "calledby:ADDR" (the function at ADDR references this one), "ref:ADDR" (the data or code at ADDR
    // references this function, e.g. a vtable slot), "string:TEXT" (references a string that starts with
    // TEXT), "import:NAME" (references the external symbol NAME), "const:HEX" (an operand equals HEX).
    private record Entry(String address, String namespace, String name, boolean probable, String evidence,
            String... checks) {
    }

    private record Build(String label, String sha256, String tag, String tagComment, Entry[] entries) {
    }

    private static final Entry[] ENTRIES_UOSA = {
        new Entry("00a70565", ASSETS, "Cache_FindAndTouch", false, "looks the asset id up in " + CACHE
            + "; a hit stamps the entry (Cache_TouchEntry) and moves it to the LRU tail; first step of GetAsset",
            "calledby:00a6cfc8", "calls:00a70361"),
        new Entry("00a70361", ASSETS, "Cache_TouchEntry", false, "copies the entry's asset out, stores the"
            + " 100-us clock at entry +0x18 and counts a use at +0x20", "calledby:00a70565", "calls:0099a7c7"),
        new Entry("00a6d7b1", ASSETS, "StoreAsset", false, "GetAsset's last step on both the factory and the"
            + " blob path: Cache_Insert under the manager critical section +0x138, also for a null (failed)"
            + " result", "calledby:00a6cfc8", "calls:00a70665"),
        new Entry("00a70665", ASSETS, "Cache_Insert", false, "inserts or replaces the asset id in " + CACHE
            + " and links it at the LRU tail", "calledby:00a6d7b1"),
        new Entry("00a72988", ASSETS, "Factory_LoadsFromBlob", true, "finds the type's factory and returns its"
            + " vtable +0xc; true sends GetAsset to LoadAssetBlob (the UOP read); the AnimationFrame factory"
            + " returns 1", "calledby:00a6cfc8", "calls:00a729d0"),
        new Entry("00a6ced4", ASSETS, "LoadRequest", false, "runs a UOResourceRequest synchronously: GetAsset"
            + " for every (id, type) pair in it, in order", "calledby:00433a40", "calls:00a6cfc8"),
        new Entry("00a6d9dd", ASSETS, "Tick", false, "the asset manager's per-frame call from the main loop;"
            + " runs Cache_PeriodicSweep", "calledby:00482530", "calls:00a70b66"),
        new Entry("00a70b66", ASSETS, "Cache_PeriodicSweep", false, "at most once a second (more than 9999"
            + " 100-us ticks since +0x30): Cache_Sweep and a second pass, then restamps +0x30",
            "calls:00a70809", "calls:00a712b7", "const:2710"),
        new Entry("00a70809", ASSETS, "Cache_Sweep", false, "counts the sweep at +0x38 and runs"
            + " Cache_EvictEntries under the critical section", "calledby:00a70b66", "calls:00a70a7f"),
        new Entry("00a70a7f", ASSETS, "Cache_EvictEntries", false, "walks the LRU list from the oldest entry;"
            + " an unused entry (state 3) is dropped while Cache_ShouldEvict says so; the walk stops at the first"
            + " unused entry it keeps", "calledby:00a70809", "calls:00a70382", "calls:00a708d4"),
        new Entry("00a708d4", ASSETS, "Cache_ShouldEvict", false, "false for an entry used within +0x100 ticks"
            + " (10000 = 1 s); else true when +0xec (byte limit, 0 = none) is exceeded by +0x40, or when +0xf4 is"
            + " set and Cache_LowPhysicalMemory", "calls:00a7085e", "calls:00a7089d", "const:ec", "const:f4"),
        new Entry("00a7085e", ASSETS, "Cache_EntryRecentlyUsed", false, "true when the clock minus the entry's"
            + " last use (+0x18) is at most the manager's +0x100/+0x104", "calledby:00a708d4", "calls:0099a7c7",
            "const:100"),
        new Entry("00a7089d", ASSETS, "Cache_LowPhysicalMemory", false, "GlobalMemoryStatus: true when"
            + " dwAvailPhys is at most +0xf4 (0.9 x the cache size setting)", "import:GlobalMemoryStatus",
            "const:f4"),
        new Entry("00a70382", ASSETS, "Cache_EntryState", false, "0 no asset, 1 still referenced, 2 unused but"
            + " the asset's vtable +8 refuses, 3 unused and evictable", "calledby:00a70a7f", "calledby:00a70939"),
        new Entry("00a70939", ASSETS, "Cache_Statistics", true, "sums bytes (+0x34) and counts of the cache"
            + " entries by Cache_EntryState into four 64-bit pairs of arrays", "calls:00a70382"),
        new Entry("00a703ac", ASSETS, "Cache_Construct", false, "empty map and LRU list, critical section +0xd4,"
            + " +0xec/+0xf4 = 0, +0xf8 = 450000, +0x100 = 10000 (1 s)", "const:6ddd0", "const:2710"),
        new Entry("00a70f40", ASSETS, "Cache_SetMemoryThreshold", true, "sets +0xf4 to the larger of a value"
            + " derived from GlobalMemoryStatus and the argument; no callers", "import:GlobalMemoryStatus",
            "const:f4"),
        new Entry("0099a7c7", CLOCK, "Now100us", false, "QueryPerformanceCounter / (frequency / 10000) plus a"
            + " base: the time in 100-us ticks", "calls:0099a74f", "const:2710"),
        new Entry("00482530", APPLICATION, "MainLoop", true, "the frame loop: Sleep to the frame cap"
            + " (1000 / fps), then the per-frame updates, the asset manager Tick among them",
            "calls:00a6d9dd", "import:Sleep"),
        new Entry("0049a740", SETTINGS, "ApplyCacheSize", false, "asset manager +0xf4 = round(cache size MB"
            + " x 2^20 x 0.9); frame composite cache +0x1c = round(cache size x 0.1) MB",
            "calls:00e33624", "calls:00e37230", "const:f4"),
        new Entry("0043afe0", ANIMATIONS, "FrameAtlasCache_Find", false, "map lookup at the composite cache"
            + " +0x24, keyed by the layer records (frame entry, palette, mirror, offsets)", "calledby:00431a10"),
        new Entry("00431770", MOB_TEXTURE, "Construct", false, "NiSourceTexture subclass over the composite"
            + " NiPixelData: TRUE_COLOR_32, SMOOTH alpha, no mipmaps, static; creates the renderer data at once"
            + " (renderer vtable +0x100)", "calledby:00431a10", "calls:00c9f944"),
        new Entry("004328b0", MOB_SHAPE, "RenderImmediate", true, "MobAnimTriShape vtable +0x88: builds or"
            + " finds the frame composite (BuildFrameAtlas), sets the quad, then the renderer vtable +0x180",
            "ref:00c9fa34", "calls:00431a10"),
        new Entry("00432110", MOB_SHADOW, "RenderImmediate", true, "MobAnimTriShapeShadow vtable +0x88: the"
            + " same for the shadow quad", "ref:00c9fadc", "calls:00431a10"),
        new Entry("0075eac0", DX9_RENDERER, "CreateSourceTextureRendererData", false, "renderer vtable +0x100,"
            + " under the renderer critical section +0x70: NiDX9SourceTextureData creation",
            "ref:00cd4d48", "calls:00770b90"),
        new Entry("00771190", DX9_TEXTURE_DATA, "CreateD3DTexture", false, "IDirect3DDevice9::CreateTexture"
            + " (device vtable +0x5c), usage 0, pool 1 (D3DPOOL_MANAGED), one level without mipmaps",
            "calledby:00770b90"),
        new Entry("007712a0", DX9_TEXTURE_DATA, "CopyPixels", false, "copies every level of the converted pixel"
            + " data into the texture", "calledby:00770b90", "calls:00771380"),
        new Entry("00771380", DX9_TEXTURE_DATA, "CopyLevelToSurface", false, "GetSurfaceLevel, GetDesc, LockRect"
            + " (flags 0), row-by-row memcpy, UnlockRect", "calledby:007712a0"),
        new Entry("0043adf0", ANIMATIONS, "PushLayerRecord", false, "appends a 0x20-byte layer draw record"
            + " (vector growth on full)", "calledby:00436d40"),
    };

    private static final Build[] BUILDS = {
        new Build("UOSA.exe (Enhanced Client)", UOSA_SHA256, TAG_LOADING, TAG_LOADING_COMMENT, ENTRIES_UOSA),
    };

    private final Map<String, Integer> changes = new LinkedHashMap<>();
    private String tag;
    private String tagComment;

    @Override
    public void run() throws Exception {
        String[] args = getScriptArgs();
        if (args.length < 1 || args.length > 2 || args.length == 2 && !args[1].equals(APPLY_FLAG)) {
            throw new IllegalArgumentException("Expected OUTPUT_FILE [" + APPLY_FLAG + "]");
        }
        String sha = currentProgram.getExecutableSHA256();
        Build build = null;
        for (Build candidate : BUILDS) {
            if (sha != null && sha.equalsIgnoreCase(candidate.sha256())) {
                build = candidate;
            }
        }
        if (build == null) {
            List<String> tested = new ArrayList<>();
            for (Build candidate : BUILDS) {
                tested.add(candidate.label() + " " + candidate.sha256());
            }
            throw new IllegalStateException("NameEcAnimLoadingCode names fixed addresses of the tested files (SHA-256 "
                + String.join(", ", tested) + "); " + currentProgram.getName() + " has SHA-256 " + sha
                + ". Nothing done.");
        }
        tag = build.tag();
        tagComment = build.tagComment();
        boolean apply = args.length == 2;
        String outputPath = args[0].replace(ENCODED_SPACE, " ");
        // Created before any change, so an existing output file stops the run unchanged.
        try (PrintWriter output = new PrintWriter(Files.newBufferedWriter(Path.of(outputPath),
                StandardOpenOption.CREATE_NEW))) {
            if (apply) {
                ensureTag();
            }
            List<String> rows = new ArrayList<>();
            for (Entry entry : build.entries()) {
                rows.add(process(entry, apply));
            }
            output.println("# UOSA.exe animation loading names");
            output.println();
            output.println("Program: `" + currentProgram.getName() + "` (tested file " + build.label()
                + "), SHA-256 `" + sha + "`.");
            output.println();
            output.println("Mode: " + (apply ? "`--apply` (changes: " + (changes.isEmpty() ? "none" : changes) + ")"
                : "report only") + ".");
            output.println();
            output.println("| Address | Name | Checks | Result |");
            output.println("|---|---|---|---|");
            rows.forEach(output::println);
        }
        println("Wrote " + outputPath + (apply ? " (changes: " + (changes.isEmpty() ? "none" : changes) + ")" : ""));
    }

    private void ensureTag() {
        if (currentProgram.getFunctionManager().getFunctionTagManager().getFunctionTag(tag) == null) {
            currentProgram.getFunctionManager().getFunctionTagManager().createFunctionTag(tag, tagComment);
            count("tag created");
        }
    }

    private Namespace namespace(String name) throws Exception {
        SymbolTable symbols = currentProgram.getSymbolTable();
        Namespace namespace = symbols.getNamespace(name, currentProgram.getGlobalNamespace());
        return namespace != null ? namespace
            : symbols.createNameSpace(currentProgram.getGlobalNamespace(), name, SourceType.ANALYSIS);
    }

    // Sets or refreshes our plate comment; "" when nothing is to do.
    private String plate(Address address, String comment, boolean apply) {
        String plate = getPlateComment(address);
        if (plate == null || plate.isBlank()) {
            if (apply) {
                setPlateComment(address, comment);
                count("plate comments set");
            }
            return apply ? "comment set" : "would set comment";
        }
        if (plate.startsWith(COMMENT_PREFIX) && !plate.equals(comment)) {
            // Our own comment from an earlier run with older evidence.
            if (apply) {
                setPlateComment(address, comment);
                count("plate comments refreshed");
            }
            return apply ? "comment refreshed" : "would refresh comment";
        }
        if (!plate.startsWith(COMMENT_PREFIX)) {
            return "plate comment from another source left alone";
        }
        return "";
    }

    private String process(Entry entry, boolean apply) throws Exception {
        Address address = toAddr(Long.parseLong(entry.address(), HEX_RADIX));
        String qualified = entry.namespace() + Namespace.DELIMITER + entry.name();
        String row = "| `0x" + entry.address() + "` | `" + qualified + "`" + (entry.probable() ? PROBABLE : "")
            + " | " + String.join(", ", entry.checks()) + " | ";
        Function function = getFunctionAt(address);
        if (function == null) {
            return row + "mismatch: no function at the address |";
        }
        List<String> failed = new ArrayList<>();
        for (String check : entry.checks()) {
            if (!check(function, check)) {
                failed.add(check);
            }
        }
        if (!failed.isEmpty()) {
            return row + "mismatch: " + String.join(", ", failed) + " failed; not named |";
        }
        List<String> result = new ArrayList<>();
        Symbol symbol = function.getSymbol();
        if (!symbol.getName(true).equals(qualified)) {
            if (symbol.getSource() != SourceType.DEFAULT && !isOwnEarlierName(function, symbol)) {
                return row + "kept `" + symbol.getName(true) + "` (" + symbol.getSource() + ") |";
            }
            String earlier = symbol.getSource() == SourceType.DEFAULT ? null : symbol.getName(true);
            Namespace existing = currentProgram.getSymbolTable().getNamespace(entry.namespace(),
                currentProgram.getGlobalNamespace());
            if (existing != null && !currentProgram.getSymbolTable().getSymbols(entry.name(), existing).isEmpty()) {
                return row + "conflict: `" + qualified + "` already names another symbol |";
            }
            if (apply) {
                symbol.setNameAndNamespace(entry.name(), namespace(entry.namespace()), SourceType.ANALYSIS);
                count(earlier == null ? "functions renamed" : "own earlier names corrected");
            }
            if (earlier == null) {
                result.add(apply ? "named" : "would name");
            } else {
                result.add((apply ? "renamed" : "would rename") + " from `" + earlier + "`");
            }
        }
        if (!hasTag(function)) {
            if (apply) {
                function.addTag(tag);
                count("tags added");
            }
            result.add(apply ? "tagged" : "would tag");
        }
        String comment = COMMENT_PREFIX + tag + " " + entry.name() + (entry.probable() ? PROBABLE : "") + ": "
            + entry.evidence();
        String plateResult = plate(address, comment, apply);
        if (!plateResult.isEmpty()) {
            result.add(plateResult);
        }
        return row + (result.isEmpty() ? "already applied" : String.join(", ", result)) + " |";
    }

    private boolean hasTag(Function function) {
        for (FunctionTag candidate : function.getTags()) {
            if (candidate.getName().equals(tag)) {
                return true;
            }
        }
        return false;
    }

    // A name this script set in an earlier run (source ANALYSIS, our tag and our plate comment): it may be
    // corrected when the entry at the address changed.
    private boolean isOwnEarlierName(Function function, Symbol symbol) {
        String plate = getPlateComment(function.getEntryPoint());
        return hasTag(function) && symbol.getSource() == SourceType.ANALYSIS
            && plate != null && plate.startsWith(COMMENT_PREFIX);
    }

    private boolean check(Function function, String check) {
        String[] parts = check.split(":", 2);
        String value = parts[1];
        return switch (parts[0]) {
            case "calls" -> references(function, toAddr(Long.parseLong(value, HEX_RADIX)));
            case "calledby" -> {
                Function caller = getFunctionAt(toAddr(Long.parseLong(value, HEX_RADIX)));
                yield caller != null && references(caller, function.getEntryPoint());
            }
            case "ref" -> {
                boolean found = false;
                for (Reference reference : getReferencesFrom(toAddr(Long.parseLong(value, HEX_RADIX)))) {
                    found |= reference.getToAddress().equals(function.getEntryPoint());
                }
                yield found;
            }
            case "string" -> referencesString(function, value);
            case "import" -> referencesExternal(function, value);
            case "const" -> hasConstant(function, Long.parseLong(value, HEX_RADIX));
            default -> throw new IllegalArgumentException("Unknown check " + check);
        };
    }

    private boolean references(Function function, Address target) {
        for (Address from : function.getBody().getAddresses(true)) {
            for (Reference reference : getReferencesFrom(from)) {
                if (reference.getToAddress().equals(target)) {
                    return true;
                }
            }
        }
        return false;
    }

    private boolean referencesString(Function function, String prefix) {
        for (Address from : function.getBody().getAddresses(true)) {
            for (Reference reference : getReferencesFrom(from)) {
                Data data = getDataAt(reference.getToAddress());
                if (data != null && StringDataInstance.isString(data)) {
                    String text = StringDataInstance.getStringDataInstance(data).getStringValue();
                    if (text != null && text.startsWith(prefix)) {
                        return true;
                    }
                }
            }
        }
        return false;
    }

    private boolean referencesExternal(Function function, String name) {
        for (Address from : function.getBody().getAddresses(true)) {
            for (Reference reference : getReferencesFrom(from)) {
                if (!reference.getToAddress().isExternalAddress()) {
                    continue;
                }
                Symbol target = currentProgram.getSymbolTable().getPrimarySymbol(reference.getToAddress());
                if (target != null && target.getName().equals(name)) {
                    return true;
                }
            }
        }
        return false;
    }

    private boolean hasConstant(Function function, long value) {
        for (Instruction instruction : currentProgram.getListing().getInstructions(function.getBody(), true)) {
            for (int i = 0; i < instruction.getNumOperands(); i++) {
                for (Object object : instruction.getOpObjects(i)) {
                    if (object instanceof Scalar scalar && scalar.getUnsignedValue() == value) {
                        return true;
                    }
                }
            }
        }
        return false;
    }

    private void count(String what) {
        changes.merge(what, 1, Integer::sum);
    }
}
