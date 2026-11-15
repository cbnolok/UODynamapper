// Name the UOSA.exe code that reads TerrainDefinition.uop (asset type 3) and uses the definitions: the
// binary factory and record parser, the terrain mesh and layer loaders, and the per-cell land flag and
// material accessors; and the TerrainDefinition parser in UOKR.exe (Kingdom Reborn).
// Address-based: one entry set per tested build, selected by SHA-256, any other build is refused; every
// address is verified with stated structural checks first and a failed check is reported, not named.
// Findings: results/uosa_terrain.md.
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

public class NameTerrainCode extends GhidraScript {
    private static final String APPLY_FLAG = "--apply";
    // Script arguments are split on whitespace, so a path with spaces is passed with %20.
    private static final String ENCODED_SPACE = "%20";
    // SHA-256 of the UOSA.exe these addresses were traced in (same build as ghidra-uop-header).
    private static final String UOSA_SHA256 = "5a5e9dc536c836d1bf10b6f86bb88f61a6ae078f519d4060d3bc47aecbebad50";
    // SHA-256 of the UOKR.exe in "UO - Old KR".
    private static final String UOKR_SHA256 = "1b802e5c00f3bf89a771778b9c6c57406be0703808eeb4cd983e2f5baa348a43";
    private static final String TAG = "UO_TERRAIN";
    private static final String TERRAIN = "UOTerrain";
    private static final String COMMENT_PREFIX = "NameTerrainCode: ";
    private static final String VFTABLE = "vftable";
    private static final int POINTER_SIZE = 4;
    private static final long UNSIGNED_INT_MASK = 0xffffffffL;
    private static final int HEX_RADIX = 16;
    // Asset type key of TerrainDefinition (type 3 << 24, assetmap.xml).
    private static final long TERRAIN_DEFINITION_KEY = 0x03000000L;
    // Land flag bits the client tests: 0x40 impassable, 0x100 no land surface.
    private static final long FLAG_IMPASSABLE = 0x40;
    private static final long FLAG_NO_SURFACE = 0x100;

    // Checks: "calls:ADDR" (the function references ADDR: call, pointer, jump or data),
    // "calledby:ADDR" (the function at ADDR references this one), "slot:CLASS:N" (vftable slot N
    // of CLASS holds this function), "vftable:CLASS" (references CLASS::vftable),
    // "string:TEXT" (references the string TEXT), "const:HEX" (an operand equals HEX),
    // "refs:N" (exactly N references to the function).
    private record Entry(String address, String namespace, String name, String evidence, String... checks) {
    }

    private record Build(String label, String sha256, Entry[] entries) {
    }

    private static final String FACTORY = "UOTerrainDefinitionBinaryFactory";
    private static final String DEFINITION = "UOTerrainDefinitionBase";

    private static final Entry[] UOSA_ENTRIES = {
        // Factory and parser (TerrainDefinition.uop, build/terraindefinition/%d.bin).
        new Entry("00845b04", TERRAIN, "TerrainDefinitionFactory_Create",
            "UOTerrainDefinitionBinaryFactory slot 1: TerrainDefinition_Read of the asset blob",
            "slot:" + FACTORY + ":1", "calls:00845c25"),
        new Entry("004cc5f0", TERRAIN, "TerrainDefinitionFactory_CreateDefault",
            "UOTerrainDefinitionBinaryFactory slot 4: an empty definition (constructor defaults)",
            "slot:" + FACTORY + ":4", "calls:0083ac8f"),
        new Entry("00845c25", TERRAIN, "TerrainDefinition_Read",
            "u32 name (string_dictionary index, +0x14), u32 id (+0x30), f32 +0x34 (0 in all, no reader), f32 +0x38 (coverage exponent, default 1.0), f32 +0x3c (0 in all, no reader), ReadAliases, ParseShaderBlock, SetShader",
            "calledby:00845b04", "calls:00845da6", "calls:008456a3", "calls:008459c0"),
        new Entry("00845da6", TERRAIN, "TerrainDefinition_ReadAliases",
            "u32 count; per alias u32 subtype, u32 legacy land id (no reader), u64 flags",
            "calledby:00845c25", "calls:0084596f"),
        new Entry("0084596f", TERRAIN, "TerrainDefinition_AddAlias",
            "appends a 0x20-byte alias {+0 subtype, +4 legacy id, +8 flags lo, +0xc flags hi} to the vector at +8",
            "calledby:00845da6"),
        new Entry("008459c0", TERRAIN, "TerrainDefinition_SetShader",
            "stores the parsed shader block shared_ptr at +0x40/+0x44",
            "calledby:00845c25"),
        new Entry("008459fe", TERRAIN, "TerrainDefinition_Alloc",
            "allocates a 0x48-byte definition and runs the constructor",
            "calls:0083ac8f"),
        new Entry("0083ac8f", TERRAIN, "TerrainDefinition_ctor",
            "vftable, empty name, id 0, floats 0 / 1.0 (DAT_00c981c8) / 0, empty alias vector, null shader",
            "calledby:004cc5f0", "calledby:008459fe"),
        new Entry("0083ad1e", TERRAIN, "TerrainDefinition_dtor",
            "releases the shader and the alias vector",
            "calledby:0083ad02"),
        new Entry("0083ad02", TERRAIN, "TerrainDefinition_ScalarDeletingDtor",
            "UOTerrainDefinitionBase slot 1",
            "slot:" + DEFINITION + ":1", "calls:0083ad1e"),
        new Entry("0083ac78", TERRAIN, "TerrainDefinition_MemSize",
            "UOTerrainDefinitionBase slot 0",
            "slot:" + DEFINITION + ":0"),
        // Lookup and use.
        new Entry("004cc990", TERRAIN, "GetTerrainDefinition",
            "(out, id): asset (id, 0x03000000) through a UOResourceRequest",
            "calls:004cb460", "calledby:00461ae0"),
        new Entry("004cb460", TERRAIN, "ResourceRequest_AddTerrainDefinitionKey",
            "pushes the asset key {id, 0x03000000} onto a request key vector",
            "const:" + Long.toHexString(TERRAIN_DEFINITION_KEY), "calledby:004cc990"),
        new Entry("00461ae0", TERRAIN, "BuildTerrainMesh",
            "per chunk: GetTerrainDefinition per cell material; alias subtype 0 flags hi bit 1 (flag bit 33) -> no-draw set (AddNoDrawTerrain); the definition shader -> LoadTerrainLayer; dereferences the subtype-0 alias without a null check",
            "calls:004cc990", "calls:00461800", "calls:00463b60", "calledby:005dbc00"),
        new Entry("00461800", TERRAIN, "LoadTerrainLayer",
            "BuildTerrainCoverage with the definition +0x38 exponent; +0x38 == 0.0 clears bit 0x100 of the layer property, else sets it; LoadTerrainLayerTexture per texture item",
            "calls:00461060", "calls:004613b0", "calledby:00461ae0"),
        new Entry("00461060", TERRAIN, "BuildTerrainCoverage",
            "32 x 32 coverage NiPixelData from neighbour material equality; exponent > 0 shapes the falloff (pow-like FUN_00bd6860)",
            "calledby:00461800"),
        new Entry("004613b0", TERRAIN, "LoadTerrainLayerTexture",
            "texture item: source kind (+0x1c), stretch (+0x20) -> UV scale 1 / stretch",
            "calledby:00461800"),
        new Entry("00463b60", TERRAIN, "AddNoDrawTerrain",
            "adds a material to the chunk no-draw set",
            "calledby:00461ae0"),
        new Entry("004027b0", TERRAIN, "LoadLegacyTerrainMap",
            "LegacyTerrain map load (legacy land id -> material, subtype)",
            "calledby:00403050"),
        new Entry("00403410", TERRAIN, "LegacyTerrainMap_Lookup",
            "legacy land id -> (material, subtype); used by BuildChunkFromSector",
            "calledby:005e2f00"),
        new Entry("005de730", TERRAIN, "GetLandFlags",
            "64-bit alias flags of the cell (+0x10/+0x14); 0 when the subtype has no alias",
            "calledby:005f3b20"),
        new Entry("005de840", TERRAIN, "GetLandMaterial",
            "material (TerrainDefinition id) of the cell (+0x18)",
            "calledby:005dc160"),
        new Entry("005f3b20", TERRAIN, "CollectSurfaces",
            "walk surfaces: land impassable when flags & 0x40; no land surface when flags & 0x100; adds 0x2000000 for param 3",
            "calls:005de730", "const:" + Long.toHexString(FLAG_IMPASSABLE), "const:" + Long.toHexString(FLAG_NO_SURFACE)),
    };

    private static final Entry[] UOKR_ENTRIES = {
        new Entry("007bf039", TERRAIN, "TerrainDefinitionFactory_Create",
            "UOTerrainDefinitionBinaryFactory slot 1: TerrainDefinition_Read of the asset blob",
            "slot:" + FACTORY + ":1", "calls:007bf15a"),
        new Entry("007bf15a", TERRAIN, "TerrainDefinition_Read",
            "EC layout with 12-byte aliases {u32 subtype, u32 legacy id, u32 flags} (EC: u64 flags)",
            "calledby:007bf039"),
    };

    private static final Build[] BUILDS = {
        new Build("UOSA.exe", UOSA_SHA256, UOSA_ENTRIES),
        new Build("UOKR.exe", UOKR_SHA256, UOKR_ENTRIES),
    };

    private final Map<String, Integer> changes = new LinkedHashMap<>();

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
            throw new IllegalStateException("NameTerrainCode names fixed addresses of the tested builds (SHA-256 "
                + String.join(", ", tested) + "); " + currentProgram.getName() + " has SHA-256 " + sha + ". Nothing done.");
        }
        boolean apply = args.length == 2;
        String outputPath = args[0].replace(ENCODED_SPACE, " ");
        // Created before any change, so an existing output file stops the run unchanged.
        try (PrintWriter output = new PrintWriter(Files.newBufferedWriter(Path.of(outputPath),
                StandardOpenOption.CREATE_NEW))) {
            List<String> rows = new ArrayList<>();
            for (Entry entry : build.entries()) {
                rows.add(process(entry, apply));
            }
            output.println("# Terrain names");
            output.println();
            output.println("Program: `" + currentProgram.getName() + "` (tested build " + build.label() + "), SHA-256 `" + sha + "`.");
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
        String row = "| `0x" + entry.address() + "` | `" + qualified + "` | ";
        Function function = getFunctionAt(address);
        if (function == null) {
            return row + checkList(entry) + " | mismatch: no function at the address |";
        }
        List<String> failed = new ArrayList<>();
        for (String check : entry.checks()) {
            if (!check(function, check)) {
                failed.add(check);
            }
        }
        row += checkList(entry) + " | ";
        if (!failed.isEmpty()) {
            return row + "mismatch: " + String.join(", ", failed) + " failed; not named |";
        }
        List<String> result = new ArrayList<>();
        Symbol symbol = function.getSymbol();
        boolean named = symbol.getName(true).equals(qualified);
        if (!named) {
            if (symbol.getSource() != SourceType.DEFAULT) {
                return row + "kept `" + symbol.getName(true) + "` (" + symbol.getSource() + ") |";
            }
            Namespace namespace = currentProgram.getSymbolTable().getNamespace(entry.namespace(),
                currentProgram.getGlobalNamespace());
            if (namespace != null && !currentProgram.getSymbolTable().getSymbols(entry.name(), namespace).isEmpty()) {
                return row + "conflict: `" + qualified + "` already names another symbol |";
            }
            if (apply) {
                symbol.setNameAndNamespace(entry.name(), terrainNamespace(), SourceType.ANALYSIS);
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
        String plateResult = plate(address, COMMENT_PREFIX + TAG + ": " + entry.evidence(), apply);
        if (!plateResult.isEmpty()) {
            result.add(plateResult);
        }
        return row + (result.isEmpty() ? "already applied" : String.join(", ", result)) + " |";
    }

    private static String checkList(Entry entry) {
        return String.join(", ", entry.checks());
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
            case "slot" -> {
                int split = value.lastIndexOf(':');
                yield slot(value.substring(0, split), Integer.parseInt(value.substring(split + 1)),
                    function.getEntryPoint());
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
            case "refs" -> getReferencesTo(function.getEntryPoint()).length == Integer.parseInt(value);
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

    // Any vftable label of the class (one per base subobject) whose slot holds the target.
    private boolean slot(String className, int index, Address target) {
        for (Address vftable : vftables(className)) {
            try {
                long pointer = getInt(vftable.add((long) index * POINTER_SIZE)) & UNSIGNED_INT_MASK;
                if (pointer == target.getOffset()) {
                    return true;
                }
            } catch (Exception unreadable) {
                // Not a readable table; try the next label.
            }
        }
        return false;
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

    private Namespace terrainNamespace() throws Exception {
        SymbolTable symbols = currentProgram.getSymbolTable();
        Namespace namespace = symbols.getNamespace(TERRAIN, currentProgram.getGlobalNamespace());
        return namespace != null ? namespace
            : symbols.createNameSpace(currentProgram.getGlobalNamespace(), TERRAIN, SourceType.ANALYSIS);
    }

    private void count(String what) {
        changes.merge(what, 1, Integer::sum);
    }
}
