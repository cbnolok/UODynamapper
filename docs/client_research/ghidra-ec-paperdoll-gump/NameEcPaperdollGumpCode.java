// Name the code of the Enhanced Client (UOSA.exe) that picks the gump drawn for each worn item of the old-style gump
// paperdoll (LegacyPaperdollTexture, UOPaperdoll::BuildImage): the mobile and body resolution, the layer list and its
// hard-coded draw order with the item-specific moves, the appearance lookup of a layer, and the equipconv.csv table.
// Address-based: one entry set for the tested file, selected by SHA-256; any other program is refused. Every address is
// verified with the stated checks first; a failed check is reported, not named.
// Findings: results/uosa_paperdoll_gump.md.
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

public class NameEcPaperdollGumpCode extends GhidraScript {
    private static final String APPLY_FLAG = "--apply";
    // Script arguments are split on whitespace, so a path with spaces is passed with %20.
    private static final String ENCODED_SPACE = "%20";
    private static final String UOSA_SHA256 = "5a5e9dc536c836d1bf10b6f86bb88f61a6ae078f519d4060d3bc47aecbebad50";
    private static final String TAG = "UO_PAPERDOLL_GUMP";
    private static final String TAG_COMMENT = "Gump paperdoll: the gump, draw order and hue of each worn item of the"
        + " Enhanced Client's old-style paperdoll";
    private static final String COMMENT_PREFIX = "NameEcPaperdollGumpCode: ";
    private static final String PROBABLE = " (probable)";
    private static final int HEX_RADIX = 16;

    private static final String PAPERDOLL = "PaperdollTextures";
    private static final String LEGACY_TEXTURE = "LegacyPaperdollTexture";
    private static final String GUMP_PAPERDOLL = "UOPaperdoll";
    private static final String GAME_DATA = "UOGameData";

    // Checks: "calls:ADDR" (the function references ADDR: call, pointer, jump or data),
    // "calledby:ADDR" (the function at ADDR references this one), "ref:ADDR" (the data or code at ADDR
    // references this function, e.g. a vtable slot), "string:TEXT" (references a string that starts with
    // TEXT), "import:NAME" (references the external symbol NAME), "const:HEX" (an operand equals HEX).
    private record Entry(String address, String namespace, String name, boolean probable, String evidence,
            String... checks) {
    }

    private record Build(String label, String sha256, Entry[] entries) {
    }

    private static final Entry[] ENTRIES_UOSA = {
        new Entry("0050c790", GUMP_PAPERDOLL, "BuildImage", false, "LegacyPaperdollTexture (vtable 0x00ca92d8) slot"
            + " 2: ResolvePaperdollMobile; the body (+0x50, less 10000000) and its hue (+0x54; bodies 987 and 1253"
            + " hue 1002); SelectPaperdollBodyGumpForBody; the canvas is the body gump's size and the body is drawn"
            + " at 0,0 with partial true; GetLayers (slot 5); per layer except backpack, mount and (ghost) robe: the"
            + " item's graphic (layer 15: its display graphic when set), its definition, props2 Paperdoll (11) else"
            + " props2 Appearance (8) giving the number and the definition at 10000000 + value, else the item's"
            + " own id and definition; the equipconv.csv row (number, body) replaces number and definition (then"
            + " props2 11/8, props1 11/8), gump and hue; no final definition draws nothing; partial ="
            + " HasFlags2(0x40000) of the final definition; gump = the row's, else number + 50000, a female body"
            + " first trying number + 60000", "ref:00ca92e0", "calls:0050aef0", "calls:0050d1c0", "calls:00404220",
            "calls:00509b30", "calls:005bda40", "const:c350", "const:2710", "const:40000"),
        new Entry("0050aef0", PAPERDOLL, "ResolvePaperdollMobile", true, "shared by the EC (0x0050b340) and the"
            + " gump (0x0050c790) builds: the mobile of serial +0x48, its graphic to +0x50 and hue to +0x54; a body"
            + " whose definition has Paperdoll (11) or Appearance (8) takes it, other bodies the race table"
            + " 0x00c9b12c [race * 2 + female] (400/401, 605/606, 666/667), + 10000000", "calledby:0050c790",
            "calledby:0050b340", "calls:005b8ad0", "calls:005bda40", "const:989680", "const:c9b12c"),
        new Entry("0050d1c0", GUMP_PAPERDOLL, "SelectPaperdollBodyGumpForBody", false, "body -> body gump, ghost"
            + " and female flags: 400 12, 401 13, 402/403 ghosts, 605 14, 606 15, 607/608 ghosts, 666 666, 667 665,"
            + " 694/695 ghosts, 744 12, 745 13, 987 by the player's gender, 1253 51253; any other body 12 or 13 by the"
            + " player's gender", "calledby:0050c790", "calls:0042eff0", "const:29a", "const:3db", "const:4e5",
            "const:c835"),
        new Entry("0050d010", LEGACY_TEXTURE, "GetLayers", false, "vtable 0x00ca92d8 slot 5: the graphic of each"
            + " worn layer, ordered by BuildLegacyLayerOrder, and the layers in that order", "ref:00ca92ec",
            "calls:00501340"),
        new Entry("00501340", GUMP_PAPERDOLL, "BuildLegacyLayerOrder", false, "the draw order of the gump paperdoll,"
            + " bottom first: table 0x00c9b688 (20 5 4 3 24 19 13 17 8 14 15 7 23 22 12 10 11 16 18 6 1 2 21 9 25),"
            + " 0x00c9b66c (arms after the face) when the arms appearance is 975, 528, 947 or 989 or the chest's is"
            + " 538, 0x00c9b6a4 (chest second) when the chest's is 921 to 925; then moves keyed to legs, feet, waist,"
            + " robe, cape, head and neck appearances", "calledby:0050d010", "calls:005012c0", "calls:00501240",
            "calls:005099f0", "const:c9b688", "const:c9b66c", "const:c9b6a4"),
        new Entry("005012c0", GUMP_PAPERDOLL, "MoveLayerNextTo", true, "moves layer b next to layer a of the order:"
            + " before a when b came after it, after a when b came before it", "calledby:00501340",
            "calls:00b6b9d0"),
        new Entry("00501240", GUMP_PAPERDOLL, "MoveLayerAfter", true, "moves layer b right after layer a of the"
            + " order", "calledby:00501340", "calls:00b6b9d0"),
        new Entry("005099f0", GUMP_PAPERDOLL, "GetLayerItemAppearance", false, "props1 Appearance (8) of the"
            + " definition of a layer's graphic; 0 without a definition", "calledby:00501340", "calls:005bda40"),
        new Entry("00403050", GAME_DATA, "LoadGameData", true, "\"Loading game data...\": loads the data/gamedata"
            + " tables, among them equipconv.csv", "string:Loading game data", "calls:00402d80"),
        new Entry("00402d80", GAME_DATA, "LoadEquipConvCsv", false, "data/gamedata/equipconv.csv: skips the header"
            + " line; five integers per row, body, item, convert, gump, hue, into the map at 0x00e32cf4 keyed"
            + " (item, body); the file is in no shipped package, so the map stays empty",
            "string:data/gamedata/equipconv.csv", "calledby:00403050", "calls:00404150"),
        new Entry("00404220", GAME_DATA, "FindEquipConv", false, "find (item, body) in the equipconv map; its end"
            + " (0x00e32cf4) when absent", "calledby:0050c790", "calls:004084f0"),
        new Entry("004084f0", GAME_DATA, "EquipConvLowerBound", true, "lower bound of a key in the equipconv map",
            "calledby:00404220"),
        new Entry("00404150", GAME_DATA, "InsertEquipConv", true, "inserts a row into the equipconv map",
            "calledby:00402d80"),
    };

    private static final Build[] BUILDS = {
        new Build("UOSA.exe 4.0.102.3 (Enhanced Client)", UOSA_SHA256, ENTRIES_UOSA),
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
            throw new IllegalStateException("NameEcPaperdollGumpCode names fixed addresses of the tested files (SHA-256 "
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
            output.println("# Gump paperdoll names");
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
