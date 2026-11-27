// Name the Classic client code (client.exe, PE version 7.0.102.3, Ghidra program classic_7.0.103.0_client.exe) that
// mounts tileart.uop and string_dictionary.uop, parses the tile definitions (version 4, the UOSA layout) and reads
// them: animation appearance and body type, the paperdoll appearance and the sitting data. tiledata.mul is loaded
// unconditionally beside them (LoadTiledata).
// Address-based: one entry set per tested file, selected by SHA-256; any other program is refused. Every
// address is verified with the stated checks first; a failed check is reported, not named.
// Findings: results/*.md.
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

public class NameClassicTileartCode extends GhidraScript {
    private static final String APPLY_FLAG = "--apply";
    // Script arguments are split on whitespace, so a path with spaces is passed with %20.
    private static final String ENCODED_SPACE = "%20";
    private static final String CLASSIC_7_0_103_SHA256 =
        "4d857f2f5f8bf350bc50a87ee66b2236bfa480e712e9546c740ff04f9106690a";
    private static final String TAG = "UO_CLASSIC_TILEART";
    private static final String TAG_COMMENT = "Classic tileart.uop: mounting, the definition parser and its readers"
        + " (appearance, body type, sitting)";
    private static final String COMMENT_PREFIX = "NameClassicTileartCode: ";
    private static final String PROBABLE = " (probable)";
    private static final int HEX_RADIX = 16;

    private static final String TILEART = "UOClassicTileArt";
    private static final String SITTING = "UOClassicSitting";
    private static final String FILES = "UOClassicFiles";

    // Checks: "calls:ADDR" (the function references ADDR: call, pointer, jump or data),
    // "calledby:ADDR" (the function at ADDR references this one), "ref:ADDR" (the data or code at ADDR
    // references this function, e.g. a vtable slot), "string:TEXT" (references a string that starts with
    // TEXT), "import:NAME" (references the external symbol NAME), "const:HEX" (an operand equals HEX).
    private record Entry(String address, String namespace, String name, boolean probable, String evidence,
            String... checks) {
    }

    private record Build(String label, String sha256, Entry[] entries) {
    }

    private static final Entry[] ENTRIES_CLASSIC_7_0_103 = {
        new Entry("00417ec4", FILES, "MountUopPackages", true, "mounts a fixed list of UOP packages when present"
            + " (facet0-6, tileart.uop, string_dictionary.uop, AnimationDefinition, ...); a missing file is skipped",
            "string:tileart.uop", "string:string_dictionary.uop"),
        new Entry("005f0e20", TILEART, "InitAssetSystem", true, "mounts the UOP packages, loads"
            + " Build/StringDictionary/string_dictionary.bin and registers the asset factories by type; type 6 is"
            + " the tile definition factory", "calls:00417ec4", "calls:00411a1e",
            "string:Build/StringDictionary/string_dictionary.bin"),
        new Entry("00411a1e", TILEART, "TileDefinitionFactory_Construct", true, "UOObjectDefinitionBinaryBaseFactory,"
            + " registered for asset type 6 (tileart.uop)", "calledby:005f0e20"),
        new Entry("00411b1d", TILEART, "TileDefinitionFactory_Create", true, "factory vtable 0x006dafd4 +4: reads the"
            + " asset and parses it with ParseTileDefinition", "ref:006dafd8", "calls:00411c6e"),
        new Entry("00411c6e", TILEART, "ParseTileDefinition", false, "a tileart.uop record: u16 version, compared with"
            + " 4, then the UOSA version 4 fixed fields, properties, stack aliases, appearance filters, sitting, radar"
            + " colour, four shader blocks and effects; everything is kept on the definition", "const:4",
            "calls:00412376", "calls:00412410", "calls:004122e5", "calls:0041256f", "calls:00412683",
            "calls:00412818", "calls:004124aa"),
        new Entry("00412376", TILEART, "ParseTileProperties1", false, "u8 count, count x {u8 key, u32 value}, stored"
            + " as properties 1", "calledby:00411c6e"),
        new Entry("00412410", TILEART, "ParseTileProperties2", false, "u8 count, count x {u8 key, u32 value}, stored"
            + " as properties 2", "calledby:00411c6e"),
        new Entry("004122e5", TILEART, "ParseTileStackAliases", false, "u32 count, count x {u32 amount, u32 id}",
            "calledby:00411c6e"),
        new Entry("0041256f", TILEART, "ParseTileAppearanceFilters", false, "u32 count of appearance filters (by body"
            + " type, by gender, animation by body type)", "calledby:00411c6e",
            "string:Invalid animation appearance filter type"),
        new Entry("00412683", TILEART, "ParseTileSitting", false, "u8 0 or 1, then six i32 sitting words",
            "calledby:00411c6e", "string:The sitting data count is not 1 or 0"),
        new Entry("00412818", TILEART, "ParseTileRadarColor", false, "four bytes, stored through the definition's"
            + " vtable +0xe0; never read back in this client", "calledby:00411c6e"),
        new Entry("004124aa", TILEART, "ParseTileEffects", true, "u8 count of effect definitions, added through the"
            + " definition's vtable +0x144", "calledby:00411c6e"),
        new Entry("00406001", TILEART, "GetTileDefinition", false, "the definition of a tile id: asset key id |"
            + " 6 << 24 (tileart.uop); null when the package or the record is missing", "const:6000000"),
        new Entry("005f0780", TILEART, "RequestTileDefinitionUnused", true, "a second type 6 asset request; nothing"
            + " references it", "const:6000000"),
        new Entry("00543000", TILEART, "GetAnimationAppearance", true, "definition vtable +0xc0 (the animation"
            + " appearance of a tile for a body placeholder); used by the paperdoll gumps", "calls:00406001"),
        new Entry("005435d0", TILEART, "GetAppearance", true, "definition vtable +0xbc (the appearance of a tile for a"
            + " body placeholder); used by the paperdoll gumps", "calls:00406001"),
        new Entry("005d6810", SITTING, "GetSeatGraphic", false, "the object's graphic when its tile definition has"
            + " sitting data (vtable +0xe4), else 0", "calls:00406001"),
        new Entry("005d6d30", SITTING, "FindSeat", false, "a human mobile (body 400..1999) is seated when an item at"
            + " |dz| < 2 on its cell has sitting data and the direction mask (+0xe8) allows it; keeps the seat"
            + " graphic", "calls:005d6810"),
        new Entry("005d6930", SITTING, "DrawSeatOverlay", true, "seated frame buffer: y shifted by sitting word 5"
            + " (+0xfc) for directions 0 and 6, else word 4 (+0xf8); for directions 0 and 6 the seat item is redrawn"
            + " over the mobile, clipped at its centre moved by 1 when word 3 (+0xf4) is non-zero (3 for 0x121a-0x121b,"
            + " no overlay for 0x2de5-0x2de6 and 0x9c57..0x9c62)", "calledby:005d6f50", "calls:00406001",
            "const:2de6"),
        new Entry("005d6f50", SITTING, "DrawSeatedMobile", true, "draws a seated MUL-frame mobile", "calls:005d6930",
            "calls:005d5620"),
        new Entry("005d5620", SITTING, "SnapSeatedDirection", true, "snaps a seated mobile's direction to the seat's"
            + " direction mask (sitting word 0)", "calledby:005d6f50"),
        new Entry("005d5450", SITTING, "HitTestSeatedMobile", true, "hit test of a seated mobile's 64x128 frame buffer"
            + " with the same y shift as DrawSeatOverlay (word 5 for directions 0 and 6, else word 4)",
            "calls:00406001"),
        new Entry("005b2ce0", FILES, "LoadTiledata", false, "reads tiledata.mul: 0x4000 land entries and the statics"
            + " (0x30 bytes each in memory, up to 0x2ff940 bytes); unconditional", "const:2ff940"),
    };

    private static final Build[] BUILDS = {
        new Build("classic_7.0.103.0_client.exe (Classic Client, PE version 7.0.102.3)", CLASSIC_7_0_103_SHA256,
            ENTRIES_CLASSIC_7_0_103),
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
            throw new IllegalStateException("NameClassicTileartCode names fixed addresses of the tested files (SHA-256 "
                + String.join(", ", tested) + "); " + currentProgram.getName() + " has SHA-256 " + sha
                + ". Nothing done.");
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
            for (Entry entry : build.entries()) {
                rows.add(process(entry, apply));
            }
            output.println("# Classic tileart names");
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
        if (currentProgram.getFunctionManager().getFunctionTagManager().getFunctionTag(TAG) == null) {
            currentProgram.getFunctionManager().getFunctionTagManager().createFunctionTag(TAG, TAG_COMMENT);
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
                function.addTag(TAG);
                count("tags added");
            }
            result.add(apply ? "tagged" : "would tag");
        }
        String comment = COMMENT_PREFIX + TAG + " " + entry.name() + (entry.probable() ? PROBABLE : "") + ": "
            + entry.evidence();
        String plateResult = plate(address, comment, apply);
        if (!plateResult.isEmpty()) {
            result.add(plateResult);
        }
        return row + (result.isEmpty() ? "already applied" : String.join(", ", result)) + " |";
    }

    private boolean hasTag(Function function) {
        for (FunctionTag tag : function.getTags()) {
            if (tag.getName().equals(TAG)) {
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
