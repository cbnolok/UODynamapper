// Name the UOSA.exe code that decides which tileart texture item feeds which shader input: the material
// builder that turns a UOShaderDefinition into a NiTexturingProperty (item i -> shader map i, hue atlas ->
// base map), the two NiTexturingProperty setters it calls, and the per-shader code that reads the shader maps
// by fixed index (sprite HLSL samplers, sprite fixed-function stages, water and static terrain stages).
// None of these functions looks at a texture file name. Address-based, so it refuses any other build; every
// address is verified with stated structural checks first and a failed check is reported, not named.
// Findings: results/uosa_texture_roles.md.
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

public class NameTextureRoleCode extends GhidraScript {
    private static final String APPLY_FLAG = "--apply";
    // Script arguments are split on whitespace, so a path with spaces is passed with %20.
    private static final String ENCODED_SPACE = "%20";
    // SHA-256 of the UOSA.exe these addresses were traced in (same build as ghidra-source-names).
    private static final String TESTED_SHA256 = "5a5e9dc536c836d1bf10b6f86bb88f61a6ae078f519d4060d3bc47aecbebad50";
    private static final String TAG = "UO_TEXROLE";
    private static final String TAG_COMMENT = "Tileart texture item to shader input binding (NameTextureRoleCode)";
    private static final String COMMENT_PREFIX = "NameTextureRoleCode: ";
    private static final String GLOBAL = "";
    private static final String VFTABLE = "vftable";
    private static final int POINTER_SIZE = 4;
    private static final long UNSIGNED_INT_MASK = 0xffffffffL;
    private static final int HEX_RADIX = 16;

    // Shader vftables (MSVC RTTI: complete object locator -> type descriptor ".?AV<class>@@").
    private static final String SPRITE_HLSL_VTABLE = "00cb425c";
    private static final String SPRITE_FFP_VTABLE = "00cb3fbc";
    private static final String SPRITE_MOB_VTABLE = "00cb4064";
    private static final String WATER_VTABLE = "00cb46bc";
    private static final String STATIC_TERRAIN_VTABLE = "00cb446c";
    // Slot +0x1c binds the HLSL samplers; slot +0xa0 sets the fixed-function texture stages.
    private static final int SLOT_BIND_SAMPLERS = 0x1c / POINTER_SIZE;
    private static final int SLOT_SETUP_STAGES = 0xa0 / POINTER_SIZE;
    // Gamebryo vftables the NiTexturingProperty setters store in a new base map / shader map array.
    private static final long BASE_MAP_VTABLE = 0xcbf528L;
    private static final long SHADER_MAP_ARRAY_VTABLE = 0xcbf53cL;
    private static final String MATERIAL_BUILDER = "0045cd80";
    private static final String SET_SHADER_MAP = "00670bd0";
    private static final String SET_BASE_TEXTURE = "004169a0";
    private static final String BIND_MAP_TO_STAGE = "0058dfc0";
    private static final String HUE_ATLAS = "Build\\Hues\\Hues.dds";
    private static final String WATER_SHADER = "UOWaterShader";

    // Checks: "calls:ADDR" (the function references ADDR: call, pointer, jump or data),
    // "calledby:ADDR" (the function at ADDR references this one), "slot:CLASS:N" (vftable slot N of the
    // labelled CLASS::vftable holds this function), "vtslot:ADDR:N" (slot N of the table at ADDR holds this
    // function's address; with --apply a missing function there is created), "vftable:CLASS" (references
    // CLASS::vftable), "string:TEXT" (references the string TEXT), "const:HEX" (an operand equals HEX).
    private record Entry(String address, String namespace, String name, String evidence, String... checks) {
    }

    private static final Entry[] ENTRIES = {
        new Entry(MATERIAL_BUILDER, GLOBAL, "BuildShaderDefinitionMaterial",
            "UOShaderDefinition -> NiShader + NiTexturingProperty: RetrieveShader(GetShaderName, GetField04 = implementation index) (0x0045da8d), fallback UOSpriteShader; texture item 0 -> SetShaderMap(0) (0x0045e0a5), items 1..n -> SetShaderMap(i) in a loop from 1 (0x0045e145, 0x0045e5ac); uv_set, stretch and source_kind set the map, the path only goes to the texture loader (missing.tga fallback); Build\\Hues\\Hues.dds -> SetBaseTexture (0x0045e666); NiAlphaProperty when the shader name is UOWaterShader (0x0045e79f). No test of the texture file name",
            "string:" + WATER_SHADER, "string:" + HUE_ATLAS, "calls:" + SET_SHADER_MAP, "calls:" + SET_BASE_TEXTURE),
        new Entry(SET_SHADER_MAP, "NiTexturingProperty", "SetShaderMap",
            "(EAX = property, index, map): creates the shader map array at +0x30 (vftable 0xcbf53c) and stores the map at the index (count ushort +0xa, data +4)",
            "calledby:" + MATERIAL_BUILDER, "const:" + Long.toHexString(SHADER_MAP_ARRAY_VTABLE)),
        new Entry(SET_BASE_TEXTURE, "NiTexturingProperty", "SetBaseTexture",
            "(EAX = property, texture): creates the base map ([+0x20][0], vftable 0xcbf528) when absent and sets its texture; the material builder passes the hue atlas",
            "calledby:" + MATERIAL_BUILDER, "const:" + Long.toHexString(BASE_MAP_VTABLE)),
        new Entry("005940e0", "UOSpriteShaderHLSL", "BindSamplers",
            "UOSpriteShaderHLSL vftable slot +0x1c: shader map 0 -> SpriteSampler (+0x120), base map -> HueSampler (+0x140, HAS_HUE_TEX +0x180), shader map 2 -> HueMaskSampler (+0x160, HAS_HUEMASK_TEX +0x1a0); shader map 1 is not read",
            "vtslot:" + SPRITE_HLSL_VTABLE + ":" + SLOT_BIND_SAMPLERS),
        new Entry("00593b30", "UOSpriteShaderHLSL", "LoadPixelShader",
            "loads UOSprite.psh and attaches the SpriteSampler, HueSampler, HueMaskSampler and the two HAS_* constants by parameter name",
            "string:UOSprite.psh"),
        new Entry("00592a00", "UOSpriteShaderFFP", "SetupTextureStages",
            "UOSpriteShaderFFP (and UOSpriteShaderMob) vftable slot +0xa0: shader map 0 -> stage 0 (0x00592af2) colour and alpha MODULATE(texture, diffuse); shader map 1, if present -> stage 1 (0x00592d0f) MODULATE(texture, current); else stage 1 disabled",
            "vtslot:" + SPRITE_FFP_VTABLE + ":" + SLOT_SETUP_STAGES, "vtslot:" + SPRITE_MOB_VTABLE + ":" + SLOT_SETUP_STAGES,
            "calls:" + BIND_MAP_TO_STAGE),
        new Entry(BIND_MAP_TO_STAGE, GLOBAL, "BindShaderMapToTextureStage",
            "(EAX = map, stage): texture of the map (+8) to the stage, texture coordinate index from the map flags, filter and clamp mode, texture transform (+0xc)",
            "calledby:00592a00"),
        new Entry("0059bbd0", WATER_SHADER, "SetupTextureStages",
            "UOWaterShader vftable slot +0xa0: reads shader maps 0..3 by fixed index (0x0059bbe2-0x0059bc3f); Technique and Tweaks looked up by name",
            "vtslot:" + WATER_VTABLE + ":" + SLOT_SETUP_STAGES, "string:Technique", "string:Tweaks"),
        new Entry("00595a00", "UOStaticTerrainShader", "SetupTextureStages",
            "UOStaticTerrainShader vftable slot +0xa0: reads shader maps 0..3 by fixed index (0x00595a0f-0x00595a76)",
            "vtslot:" + STATIC_TERRAIN_VTABLE + ":" + SLOT_SETUP_STAGES),
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
            throw new IllegalStateException("NameTextureRoleCode names fixed addresses of the tested UOSA.exe (SHA-256 "
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
            output.println("# Texture role names");
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
