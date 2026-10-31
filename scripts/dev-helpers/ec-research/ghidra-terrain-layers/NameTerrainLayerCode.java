// Name the UOSA.exe terrain layer code: the factory that picks the layer class from the TerrainDefinition shader
// name, the three layer classes (constructors, Draw methods, the shared four-stage / two-stage setups) and the
// UOBaseTerrainLayer accessors (maps, techniques, tweaks, material id), plus the WorldArt location helper and the
// pass texture-stage getter. Address-based, so it refuses any other build; every address is verified with stated
// structural checks first and a failed check is reported, not named.
// Findings: results/uosa_terrain_layers.md.
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

public class NameTerrainLayerCode extends GhidraScript {
    private static final String APPLY_FLAG = "--apply";
    // Script arguments are split on whitespace, so a path with spaces is passed with %20.
    private static final String ENCODED_SPACE = "%20";
    // SHA-256 of the UOSA.exe these addresses were traced in (same build as ghidra-source-names).
    private static final String TESTED_SHA256 = "5a5e9dc536c836d1bf10b6f86bb88f61a6ae078f519d4060d3bc47aecbebad50";
    private static final String TAG = "UO_TERRAINLAYER";
    private static final String TAG_COMMENT = "Terrain layer classes and texture-stage roles (NameTerrainLayerCode)";
    private static final String COMMENT_PREFIX = "NameTerrainLayerCode: ";
    private static final String GLOBAL = "";
    private static final String VFTABLE = "vftable";
    private static final int POINTER_SIZE = 4;
    private static final long UNSIGNED_INT_MASK = 0xffffffffL;
    private static final int HEX_RADIX = 16;

    private static final String LOAD_TERRAIN_LAYER = "00461800";
    private static final String LOAD_TERRAIN_LAYER_TEXTURE = "004613b0";
    private static final String CREATE_LAYER = "004616e0";
    private static final String BIND_MAP_TO_STAGE = "0058dfc0";
    private static final String GET_PASS_STAGE = "00596930";
    private static final String DEFAULT_LAYER_VFTABLE = "00cb466c";
    private static final String WATER_LAYER_VFTABLE = "00ca0a5c";
    private static final String BUMP_LAYER_VFTABLE = "00ca0a0c";
    private static final String BASE_LAYER_VFTABLE = "00cb4624";
    // vftable slots (index = byte offset / 4).
    private static final int SLOT_DRAW = 1;
    private static final int SLOT_GET_MATERIAL_ID = 2;
    private static final int SLOT_SET_MATERIAL_ID = 3;
    private static final int SLOT_ADD_MAP = 4;
    private static final int SLOT_SET_MAP = 5;
    private static final int SLOT_GET_MAP = 6;
    private static final int SLOT_MAP_COUNT = 7;
    private static final int SLOT_ADD_TECHNIQUE = 8;
    private static final int SLOT_SET_TECHNIQUE = 9;
    private static final int SLOT_GET_TECHNIQUE = 10;
    private static final int SLOT_TECHNIQUE_COUNT = 11;
    private static final int SLOT_ADD_TWEAK = 12;
    private static final int SLOT_SET_TWEAK = 13;
    private static final int SLOT_GET_TWEAK = 14;
    private static final int SLOT_TWEAK_COUNT = 15;
    private static final int SLOT_SETUP_FOUR_STAGES = 16;
    private static final int SLOT_SETUP_TWO_STAGES = 17;
    // D3DTEXOPCAPS bits tested by the draws (renderer caps +0x90).
    private static final String CAP_BUMPENVMAP = "200000";
    private static final String CAP_BUMPENVMAPLUMINANCE = "400000";
    // D3DTSS_TCI_CAMERASPACEREFLECTIONVECTOR.
    private static final String TCI_REFLECTION = "30000";

    // Checks: "calls:ADDR" (the function references ADDR: call, pointer, jump or data),
    // "calledby:ADDR" (the function at ADDR references this one), "slot:CLASS:N" (vftable slot N of the
    // labelled CLASS::vftable holds this function), "vtslot:ADDR:N" (slot N of the table at ADDR holds this
    // function's address; with --apply a missing function there is created), "vftable:CLASS" (references
    // CLASS::vftable), "string:TEXT" (references the string TEXT), "const:HEX" (an operand equals HEX).
    private record Entry(String address, String namespace, String name, String evidence, String... checks) {
    }

    private static String vt(String table, int slot) {
        return "vtslot:" + table + ":" + slot;
    }

    private static final Entry[] ENTRIES = {
        new Entry(CREATE_LAYER, "UOTerrain", "CreateTerrainLayerForShader",
            "(shader name): compares the TerrainDefinition shader name only: UOWaterTerrainLayer or UOWaterShader -> UOWaterTerrainLayer, UOBumpMapTerrainLayer -> UOBumpMapTerrainLayer, UODefaultTerrainLayer, UOTerrainShader or anything else -> UODefaultTerrainLayer. No texture path is looked at",
            "calledby:" + LOAD_TERRAIN_LAYER, "calls:00597170", "calls:00460190", "calls:004600f0",
            "string:UOWaterTerrainLayer", "string:UODefaultTerrainLayer", "string:UOBumpMapTerrainLayer"),
        new Entry("00597170", "UODefaultTerrainLayer", "UODefaultTerrainLayer",
            "constructor (0x38-byte UOBaseTerrainLayer, 8 map slots reserved)",
            "calledby:" + CREATE_LAYER, "vftable:UODefaultTerrainLayer"),
        new Entry("00460190", "UOWaterTerrainLayer", "UOWaterTerrainLayer",
            "constructor", "calledby:" + CREATE_LAYER, "vftable:UOWaterTerrainLayer"),
        new Entry("004600f0", "UOBumpMapTerrainLayer", "UOBumpMapTerrainLayer",
            "constructor", "calledby:" + CREATE_LAYER, "vftable:UOBumpMapTerrainLayer"),
        new Entry("00597210", "UODefaultTerrainLayer", "Draw",
            "(renderer pass, pass index): maps 0..3 (0 = generated coverage, i + 1 = texture item i), stages 0..3; >= 4 blend stages -> SetupFourStages(pass, map0..3, stage0..3), else SetupTwoStages(pass, map0, map1, stage0, stage1). Item 3 is never read",
            vt(DEFAULT_LAYER_VFTABLE, SLOT_DRAW)),
        new Entry("005974f0", "UODefaultTerrainLayer", "SetupFourStages",
            "stage0 item 1 SELECTARG1 TEXTURE -> TEMP; stage1 item 2 colour CURRENT, alpha TEXTURE (blend mask); stage2 item 0 BLENDCURRENTALPHA(TEXTURE, TEMP) = item0 * a + item1 * (1 - a); stage3 coverage MODULATE(CURRENT, DIFFUSE), alpha DIFFUSE on pass 0 else coverage TEXTURE. Shared by UOBumpMapTerrainLayer",
            vt(DEFAULT_LAYER_VFTABLE, SLOT_SETUP_FOUR_STAGES), vt(BUMP_LAYER_VFTABLE, SLOT_SETUP_FOUR_STAGES),
            "calls:" + BIND_MAP_TO_STAGE),
        new Entry("00597b50", "UODefaultTerrainLayer", "SetupTwoStages",
            "fewer than 4 blend stages: stage0 item 0 MODULATE(TEXTURE, DIFFUSE); stage1 coverage, colour CURRENT, alpha DIFFUSE on pass 0 else coverage TEXTURE. Item 0 is the only colour drawn. Shared by UOBumpMapTerrainLayer",
            vt(DEFAULT_LAYER_VFTABLE, SLOT_SETUP_TWO_STAGES), vt(BUMP_LAYER_VFTABLE, SLOT_SETUP_TWO_STAGES),
            "calls:" + BIND_MAP_TO_STAGE),
        new Entry("00597f00", "UOBumpMapTerrainLayer", "Draw",
            "maps 0..5: items 0..2 as UODefaultTerrainLayer (SetupFourStages / SetupTwoStages); with >= 6 stages and BUMPENVMAPLUMINANCE: stage4 item 3 BUMPENVMAPLUMINANCE (identity matrix, lscale 1, loffset 0), stage5 item 4 at CAMERASPACEREFLECTIONVECTOR, ADD(TEXTURE, CURRENT), alpha CURRENT",
            vt(BUMP_LAYER_VFTABLE, SLOT_DRAW), "calls:" + BIND_MAP_TO_STAGE, "const:" + CAP_BUMPENVMAPLUMINANCE,
            "const:" + TCI_REFLECTION),
        new Entry("005988a0", "UOWaterTerrainLayer", "Draw",
            "maps 0..3, needs items 0..2; >= 4 stages: stage0 item 0 BUMPENVMAP (identity matrix), alpha item 0, UV scrolled by time x tweaks 0/1; stage1 item 2 SELECTARG1 TEXTURE, alpha CURRENT, coordinates by technique 0 (1 own uv set, 2 set 0, 3 camera normal, 4 sphere map, else camera reflection vector; table 0x005996b4); stage2 coverage alpha, disabled on pass 0. Fallback: item 1 scrolled, then coverage. Item 3, technique 1 and tweaks 2/3 unused here",
            vt(WATER_LAYER_VFTABLE, SLOT_DRAW), "calls:" + BIND_MAP_TO_STAGE, "calls:" + GET_PASS_STAGE,
            "const:" + CAP_BUMPENVMAP, "const:" + TCI_REFLECTION),
        new Entry(GET_PASS_STAGE, GLOBAL, "GetTerrainPassTextureStage",
            "(EAX = stage index, EDX = renderer pass): stage object from the pass stage array (+0x24, count +0x2a), or null; the default and bump Draw methods inline the same lookup",
            "calledby:005988a0"),
        new Entry("004603f0", "UOTerrain", "MakeWorldArtTextureLocation",
            "(item path): WorldArtTextureFileLocation for \"Data\\\\WorldArt\\\\\" + path; no name tests",
            "calledby:" + LOAD_TERRAIN_LAYER_TEXTURE, "string:Data\\WorldArt\\"),
        new Entry("004600d0", "UOBaseTerrainLayer", "GetMaterialId", "returns the material id (sort key, comparator 0x005996d0)",
            vt(BASE_LAYER_VFTABLE, SLOT_GET_MATERIAL_ID)),
        new Entry("004600e0", "UOBaseTerrainLayer", "SetMaterialId", "stores the material id",
            vt(BASE_LAYER_VFTABLE, SLOT_SET_MATERIAL_ID)),
        new Entry("004601e0", "UOWaterTerrainLayer", "SetMaterialId", "stores material id x 1000 (water layers sort after the others)",
            vt(WATER_LAYER_VFTABLE, SLOT_SET_MATERIAL_ID)),
        new Entry("00596f30", "UOBaseTerrainLayer", "AddMap", "appends a texture map (LoadTerrainLayer: coverage first, then item i at i + 1)",
            vt(BASE_LAYER_VFTABLE, SLOT_ADD_MAP)),
        new Entry("00596f80", "UOBaseTerrainLayer", "SetMap", "replaces map i", vt(BASE_LAYER_VFTABLE, SLOT_SET_MAP)),
        new Entry("00596fd0", "UOBaseTerrainLayer", "GetMap", "map i or null", vt(BASE_LAYER_VFTABLE, SLOT_GET_MAP)),
        new Entry("00597000", "UOBaseTerrainLayer", "MapCount", "number of maps", vt(BASE_LAYER_VFTABLE, SLOT_MAP_COUNT)),
        new Entry("00597020", "UOBaseTerrainLayer", "AddTechnique", "appends a technique value",
            vt(BASE_LAYER_VFTABLE, SLOT_ADD_TECHNIQUE)),
        new Entry("00597070", "UOBaseTerrainLayer", "SetTechnique", "replaces technique value i",
            vt(BASE_LAYER_VFTABLE, SLOT_SET_TECHNIQUE)),
        new Entry("00597090", "UOBaseTerrainLayer", "GetTechnique", "technique value i",
            vt(BASE_LAYER_VFTABLE, SLOT_GET_TECHNIQUE)),
        new Entry("005970a0", "UOBaseTerrainLayer", "TechniqueCount", "number of technique values",
            vt(BASE_LAYER_VFTABLE, SLOT_TECHNIQUE_COUNT)),
        new Entry("005970c0", "UOBaseTerrainLayer", "AddTweak", "appends a tweak float", vt(BASE_LAYER_VFTABLE, SLOT_ADD_TWEAK)),
        new Entry("00597120", "UOBaseTerrainLayer", "SetTweak", "replaces tweak i", vt(BASE_LAYER_VFTABLE, SLOT_SET_TWEAK)),
        new Entry("00597140", "UOBaseTerrainLayer", "GetTweak", "tweak float i", vt(BASE_LAYER_VFTABLE, SLOT_GET_TWEAK)),
        new Entry("00597150", "UOBaseTerrainLayer", "TweakCount", "number of tweak floats",
            vt(BASE_LAYER_VFTABLE, SLOT_TWEAK_COUNT)),
    };

    private final Map<String, Integer> changes = new LinkedHashMap<>();

    @Override
    public void run() throws Exception {
        String[] args = getScriptArgs();
        if (args.length < 1 || args.length > 2 || args.length == 2 && !args[1].equals(APPLY_FLAG)) {
            throw new IllegalArgumentException("Expected OUTPUT_FILE [" + APPLY_FLAG + "]");
        }
        String sha = currentProgram.getExecutableSHA256();
        if (sha == null || !sha.equalsIgnoreCase(TESTED_SHA256)) {
            throw new IllegalStateException("NameTerrainLayerCode names fixed addresses of the tested UOSA.exe (SHA-256 "
                + TESTED_SHA256 + "); " + currentProgram.getName() + " has SHA-256 " + sha + ". Nothing done.");
        }
        boolean apply = args.length == 2;
        String outputPath = args[0].replace(ENCODED_SPACE, " ");
        // Created before any change, so an existing output file stops the run unchanged.
        try (PrintWriter output = new PrintWriter(Files.newBufferedWriter(Path.of(outputPath),
                StandardOpenOption.CREATE_NEW))) {
            if (apply) {
                ensureTag();
            }
            List<String> rows = new ArrayList<>();
            for (Entry entry : ENTRIES) {
                rows.add(process(entry, apply));
            }
            output.println("# Terrain layer names");
            output.println();
            output.println("Program: `" + currentProgram.getName() + "`, SHA-256 `" + sha + "`.");
            output.println();
            output.println("Mode: " + (apply ? "`--apply` (changes: " + (changes.isEmpty() ? "none" : changes) + ")"
                : "report only") + ".");
            output.println();
            output.println("| Address | Name | Checks | Result |");
            output.println("|---|---|---|---|");
            rows.forEach(output::println);
        }
        println("Wrote " + outputPath + (apply ? " (changes: " + changes + ")" : ""));
    }

    private String process(Entry entry, boolean apply) throws Exception {
        Address address = toAddr(Long.parseLong(entry.address(), HEX_RADIX));
        String qualified = (entry.namespace().isEmpty() ? "" : entry.namespace() + Namespace.DELIMITER) + entry.name();
        String row = "| `0x" + entry.address() + "` | `" + qualified + "` | " + checkList(entry) + " | ";
        List<String> result = new ArrayList<>();
        Function function = getFunctionAt(address);
        if (function == null) {
            if (!heldByTableSlot(entry, address)) {
                return row + "mismatch: no function at the address |";
            }
            if (getFunctionContaining(address) != null) {
                return row + "mismatch: the address is inside " + getFunctionContaining(address).getName(true) + " |";
            }
            if (!apply) {
                return row + "would create the function, then check and name it |";
            }
            disassemble(address);
            function = createFunction(address, null);
            if (function == null) {
                return row + "mismatch: no function could be created |";
            }
            count("functions created");
            result.add("function created");
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
        Symbol symbol = function.getSymbol();
        boolean named = symbol.getName(true).equals(qualified);
        if (!named) {
            if (symbol.getSource() != SourceType.DEFAULT) {
                return row + "kept `" + symbol.getName(true) + "` (" + symbol.getSource() + ") |";
            }
            Namespace namespace = namespaceOrNull(entry.namespace());
            if (namespace != null && !currentProgram.getSymbolTable().getSymbols(entry.name(), namespace).isEmpty()) {
                return row + "conflict: `" + qualified + "` already names another symbol |";
            }
            if (apply) {
                if (namespace == null) {
                    namespace = createNamespace(entry.namespace());
                }
                symbol.setNameAndNamespace(entry.name(), namespace, SourceType.ANALYSIS);
                count("functions renamed");
            }
            result.add(apply ? "named" : "would name");
        }
        boolean tagged = false;
        for (FunctionTag tag : function.getTags()) {
            tagged |= tag.getName().equals(TAG);
        }
        if (!tagged) {
            if (apply) {
                function.addTag(TAG);
                count("tags added");
            }
            result.add(apply ? "tagged" : "would tag");
        }
        String plate = getPlateComment(address);
        String comment = COMMENT_PREFIX + TAG + ": " + entry.evidence();
        if (plate == null || plate.isBlank()) {
            if (apply) {
                setPlateComment(address, comment);
                count("plate comments set");
            }
            result.add(apply ? "comment set" : "would set comment");
        } else if (plate.startsWith(COMMENT_PREFIX) && !plate.equals(comment)) {
            // Our own comment from an earlier run with older evidence.
            if (apply) {
                setPlateComment(address, comment);
                count("plate comments refreshed");
            }
            result.add(apply ? "comment refreshed" : "would refresh comment");
        } else if (!plate.contains(entry.evidence())) {
            result.add("plate comment from another source left alone");
        }
        return row + (result.isEmpty() ? "already applied" : String.join(", ", result)) + " |";
    }

    // Checks as one table cell; a newline in a string check is shown escaped.
    private static String checkList(Entry entry) {
        return String.join(", ", entry.checks()).replace("\n", "\\n");
    }

    // A "vtslot" check of the entry holds the address, so a function may be created there.
    private boolean heldByTableSlot(Entry entry, Address address) {
        for (String check : entry.checks()) {
            if (check.startsWith("vtslot:") && check(null, check, address)) {
                return true;
            }
        }
        return false;
    }

    private boolean check(Function function, String check) {
        return check(function, check, function.getEntryPoint());
    }

    private boolean check(Function function, String check, Address entryPoint) {
        String[] parts = check.split(":", 2);
        String value = parts[1];
        return switch (parts[0]) {
            case "calls" -> references(function, toAddr(Long.parseLong(value, HEX_RADIX)));
            case "calledby" -> {
                Function caller = getFunctionAt(toAddr(Long.parseLong(value, HEX_RADIX)));
                yield caller != null && references(caller, entryPoint);
            }
            case "slot" -> {
                int split = value.lastIndexOf(':');
                boolean found = false;
                for (Address vftable : vftables(value.substring(0, split))) {
                    found |= tableSlotHolds(vftable, Integer.parseInt(value.substring(split + 1)), entryPoint);
                }
                yield found;
            }
            case "vtslot" -> {
                int split = value.lastIndexOf(':');
                yield tableSlotHolds(toAddr(Long.parseLong(value.substring(0, split), HEX_RADIX)),
                    Integer.parseInt(value.substring(split + 1)), entryPoint);
            }
            case "vftable" -> {
                boolean found = false;
                for (Address vftable : vftables(value)) {
                    found |= references(function, vftable);
                }
                yield found;
            }
            case "string" -> referencesString(function, value);
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

    private boolean referencesString(Function function, String text) {
        for (Address from : function.getBody().getAddresses(true)) {
            for (Reference reference : getReferencesFrom(from)) {
                Data data = getDataAt(reference.getToAddress());
                if (data != null && StringDataInstance.isString(data)
                        && text.equals(StringDataInstance.getStringDataInstance(data).getStringValue())) {
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

    private boolean tableSlotHolds(Address table, int index, Address target) {
        try {
            long pointer = getInt(table.add((long) index * POINTER_SIZE)) & UNSIGNED_INT_MASK;
            return pointer == target.getOffset();
        } catch (Exception unreadable) {
            return false;
        }
    }

    private List<Address> vftables(String className) {
        List<Address> found = new ArrayList<>();
        SymbolTable table = currentProgram.getSymbolTable();
        for (Symbol symbol : table.getSymbols(VFTABLE)) {
            if (symbol.getName(true).equals(className + Namespace.DELIMITER + VFTABLE)) {
                found.add(symbol.getAddress());
            }
        }
        return found;
    }

    private Namespace namespaceOrNull(String path) {
        Namespace namespace = currentProgram.getGlobalNamespace();
        if (path.isEmpty()) {
            return namespace;
        }
        for (String part : path.split(Namespace.DELIMITER)) {
            namespace = currentProgram.getSymbolTable().getNamespace(part, namespace);
            if (namespace == null) {
                return null;
            }
        }
        return namespace;
    }

    private Namespace createNamespace(String path) throws Exception {
        Namespace namespace = currentProgram.getGlobalNamespace();
        for (String part : path.split(Namespace.DELIMITER)) {
            Namespace child = currentProgram.getSymbolTable().getNamespace(part, namespace);
            if (child == null) {
                child = currentProgram.getSymbolTable().createNameSpace(namespace, part, SourceType.ANALYSIS);
                count("namespaces created");
            }
            namespace = child;
        }
        return namespace;
    }

    private void ensureTag() {
        if (currentProgram.getFunctionManager().getFunctionTagManager().getFunctionTag(TAG) == null) {
            currentProgram.getFunctionManager().getFunctionTagManager().createFunctionTag(TAG, TAG_COMMENT);
            count("tag created");
        }
    }

    private void count(String what) {
        changes.merge(what, 1, Integer::sum);
    }
}
