// Name the UOSA.exe code that sizes and places tileart sprites: the geometry builder called by
// UOTileArt::BuildSpriteDescriptor, the image-window quad builder (pixel unit 0.021427423 x descriptor scale),
// the map-position setter (4 legacy pixels per Z, origin (x + 0.5, -y - 1.5)), the floorf wrapper used for the
// quad extents and the viewport/frustum updater. Address-based, so it refuses any other build; every address is
// verified with stated structural checks first and a failed check is reported, not named.
// Findings: results/uosa_legacy_art_scale.md.
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

public class NameLegacyArtScaleCode extends GhidraScript {
    private static final String APPLY_FLAG = "--apply";
    // Script arguments are split on whitespace, so a path with spaces is passed with %20.
    private static final String ENCODED_SPACE = "%20";
    // SHA-256 of the UOSA.exe these addresses were traced in (same build as ghidra-source-names).
    private static final String TESTED_SHA256 = "5a5e9dc536c836d1bf10b6f86bb88f61a6ae078f519d4060d3bc47aecbebad50";
    private static final String TAG = "UO_LEGACYSCALE";
    private static final String TAG_COMMENT = "Tileart sprite sizing and placement (NameLegacyArtScaleCode)";
    private static final String COMMENT_PREFIX = "NameLegacyArtScaleCode: ";
    private static final String GLOBAL = "";
    private static final String VFTABLE = "vftable";
    private static final int POINTER_SIZE = 4;
    private static final long UNSIGNED_INT_MASK = 0xffffffffL;
    private static final int HEX_RADIX = 16;

    private static final String BUILD_SPRITE_DESCRIPTOR = "004592e0";
    private static final String BUILD_SPRITE_GEOMETRY = "0045a080";
    private static final String BUILD_IMAGE_WINDOW_QUAD = "00459b00";
    private static final String FLOORF = "004440b0";
    private static final String CRT_FLOOR = "00bd6d70";
    private static final String CREATE_CAMERA = "00471580";
    private static final String SET_CAMERA_FRUSTUM = "004712f0";
    // Float constants (address: value).
    private static final String CAMERA_QUAD_HALF_DIAGONAL = "00d0535c"; // 0.707105
    private static final String CAMERA_QUAD_HEIGHT = "00d05358"; // 1.4742099
    private static final String CAMERA_QUAD_MARGIN = "00d05354"; // 0.06
    private static final String SPRITE_PIXEL_UNIT = "00d051d8"; // 0.021427423 = sqrt(2) / 66
    private static final String LEGACY_PIXEL_UNIT = "00d05154"; // 0.032141134 = sqrt(2) / 44
    private static final String Z_UNIT = "00d050a0"; // 0.12856454 = 4 legacy pixels
    private static final String ONE_AND_HALF = "00c803d0"; // 1.5
    private static final String VIEWPORT_ASPECT = "00db9754"; // viewport height / width

    // Checks: "calls:ADDR" (the function references ADDR: call, pointer, jump or data),
    // "calledby:ADDR" (the function at ADDR references this one), "slot:CLASS:N" (vftable slot N of the
    // labelled CLASS::vftable holds this function), "vtslot:ADDR:N" (slot N of the table at ADDR holds this
    // function's address; with --apply a missing function there is created), "vftable:CLASS" (references
    // CLASS::vftable), "string:TEXT" (references the string TEXT), "const:HEX" (an operand equals HEX).
    private record Entry(String address, String namespace, String name, String evidence, String... checks) {
    }

    private static final Entry[] ENTRIES = {
        new Entry(BUILD_SPRITE_GEOMETRY, "UOTileArt", "BuildSpriteGeometry",
            "called at the end of BuildSpriteDescriptor (descriptor): flags bit 34 -> camera-plane quad from constants 0.707105, 1.4742099, 0.06 (descriptor +0x74..+0xa0); otherwise loads the primary texture of the selected block (missing texture fallback 0x00e33e94) and calls BuildImageWindowQuad(scale = descriptor +0x44, texture) on the window at descriptor +0; sitting split window: second quad shifted by viewer direction x 6 x -1e-5",
            "calledby:" + BUILD_SPRITE_DESCRIPTOR, "calls:" + BUILD_IMAGE_WINDOW_QUAD, "calls:" + CAMERA_QUAD_HALF_DIAGONAL,
            "calls:" + CAMERA_QUAD_HEIGHT, "calls:" + CAMERA_QUAD_MARGIN),
        new Entry(BUILD_IMAGE_WINDOW_QUAD, "UOTileArt", "BuildImageWindowQuad",
            "(scale, texture; EDI = window x0,y0,x1,y1,ox,oy; ESI = quad): k = 0.021427423; width floorf(|x1-x0| * scale) * k, height floorf(|y1-y0| * scale) * k; left ox * scale * k, bottom -oy * scale * k on the camera right / up axes (camera rotation at camera +0x2c); u = x / texture width, v = 1 - y / texture height; world pairs x - 0.5, z + 0.5",
            "calledby:" + BUILD_SPRITE_GEOMETRY, "calls:" + FLOORF, "calls:" + SPRITE_PIXEL_UNIT),
        new Entry("004598d0", GLOBAL, "SetSpriteMapPosition",
            "(x, y, z, ui): stores x, y, z at +0x4c/+0x50/+0x54; world: origin (x + 0.5, 0, -y - 1.5) moved along the viewer up column (viewer +0x24) by z * 0.12856454 (4 legacy pixels per Z); UI: legacy pixel 0.032141134 per pixel",
            "calls:" + Z_UNIT, "calls:" + LEGACY_PIXEL_UNIT, "calls:" + ONE_AND_HALF),
        new Entry(FLOORF, GLOBAL, "floorf",
            "float wrapper of the CRT floor(double) at 0x00bd6d70",
            "calls:" + CRT_FLOOR, "calledby:" + BUILD_IMAGE_WINDOW_QUAD),
        new Entry("00471390", "UOWorldViewer", "UpdateViewportFrustum",
            "computes the viewport fractions (0x00e33590..) and the height / width ratio 0x00db9754, then SetCameraFrustum (half-height = viewer +0x80 zoom x height fraction; orthographic, near 8, far 12)",
            "calledby:" + CREATE_CAMERA, "calls:" + SET_CAMERA_FRUSTUM, "calls:" + VIEWPORT_ASPECT),
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
            throw new IllegalStateException("NameLegacyArtScaleCode names fixed addresses of the tested UOSA.exe (SHA-256 "
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
            output.println("# Legacy art scale names");
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
