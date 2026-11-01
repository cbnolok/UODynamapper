// Name the code of the 2024 Classic client (7.0.103.0 client.exe) that reads the custom-house tables (walls.txt,
// floors.txt, doors.txt, misc.txt, stairs.txt, teleprts.txt, roof.txt), suppinfo.txt, prof.txt, keynames.txt,
// palette.mul and animinfo.mul.
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

public class NameClassicTextTableCode extends GhidraScript {
    private static final String APPLY_FLAG = "--apply";
    // Script arguments are split on whitespace, so a path with spaces is passed with %20.
    private static final String ENCODED_SPACE = "%20";
    private static final String CLASSIC_7_0_103_SHA256 =
        "4d857f2f5f8bf350bc50a87ee66b2236bfa480e712e9546c740ff04f9106690a";
    private static final String TAG_FILES = "UO_CLASSIC_DATA_FILES";
    private static final String TAG_FILES_COMMENT =
        "Classic house tables, suppinfo.txt, prof.txt, keynames.txt, palette.mul, animinfo.mul";
    private static final String COMMENT_PREFIX = "NameClassicTextTableCode: ";
    private static final String PROBABLE = " (probable)";
    private static final int HEX_RADIX = 16;

    private static final String FILES = "UOClassicDataFiles";
    private static final String HOUSE_ROW = "a custom-house table row: ";
    private static final String HOUSE_PARSE = "sscanf of one custom-house table line; the row is kept when more"
        + " than ";

    // Checks: "calls:ADDR" (the function references ADDR: call, pointer, jump or data),
    // "calledby:ADDR" (the function at ADDR references this one), "ref:ADDR" (the data or code at ADDR
    // references this function, e.g. a vtable slot), "string:TEXT" (references a string that starts with
    // TEXT), "import:NAME" (references the external symbol NAME), "const:HEX" (an operand equals HEX).
    private record Entry(String address, String namespace, String name, boolean probable, String evidence,
            String... checks) {
    }

    private record Build(String label, String sha256, String tag, String tagComment, Entry[] entries) {
    }

    private static final Entry[] ENTRIES_CLASSIC_7_0_103 = {
        new Entry("00576920", FILES, "HouseTables_LoadAll", false, "loads walls.txt (kind 1), floors.txt (2),"
            + " doors.txt (3), misc.txt (5), stairs.txt (4), teleprts.txt (6) and roof.txt (7) through"
            + " HouseTable_Load", "string:walls.txt", "string:roof.txt", "calls:00575bf0"),
        new Entry("00575bf0", FILES, "HouseTable_Load", false, "opens the table in text mode, fgets 255 bytes"
            + " a line, skips lines 1 and 2, makes the kind's row for every later line and keeps it when the"
            + " row's Parse accepts the line, else deletes it", "calls:0056f520", "calls:00570480",
            "calls:0056f870"),
        new Entry("0056f520", FILES, "WallsRow_Construct", false, HOUSE_ROW + "walls.txt, vtable 0x6b528c",
            "calledby:00575bf0", "const:6b528c"),
        new Entry("0056f580", FILES, "WallsRow_Parse", false, HOUSE_PARSE + "16 of 18 numbers are read",
            "ref:006b528c", "string:%d %d %d %d %d %d %d %d %d %d %d %d %d %d %d %d %d %d"),
        new Entry("00570480", FILES, "FloorsRow_Construct", false, HOUSE_ROW + "floors.txt, vtable 0x6b5444",
            "calledby:00575bf0", "const:6b5444"),
        new Entry("0056f5f0", FILES, "FloorsRow_Parse", false, HOUSE_PARSE + "16 of 18 numbers are read",
            "ref:006b5444", "string:%d %d %d %d %d %d %d %d %d %d %d %d %d %d %d %d %d %d"),
        new Entry("00570520", FILES, "TeleportersRow_Construct", false, HOUSE_ROW + "teleprts.txt, vtable"
            + " 0x6b5454", "calledby:00575bf0", "const:6b5454"),
        new Entry("0056f660", FILES, "TeleportersRow_Parse", false, HOUSE_PARSE + "16 of 18 numbers are read",
            "ref:006b5454", "string:%d %d %d %d %d %d %d %d %d %d %d %d %d %d %d %d %d %d"),
        new Entry("005704d0", FILES, "DoorsRow_Construct", false, HOUSE_ROW + "doors.txt", "calledby:00575bf0"),
        new Entry("0056f6d0", FILES, "DoorsRow_Parse", false, HOUSE_PARSE + "8 of 10 numbers are read",
            "string:%d %d %d %d %d %d %d %d %d %d"),
        new Entry("0056f720", FILES, "MiscRow_Construct", false, HOUSE_ROW + "misc.txt", "calledby:00575bf0"),
        new Entry("0056f770", FILES, "MiscRow_Parse", false, HOUSE_PARSE + "10 of 12 numbers are read",
            "string:%d %d %d %d %d %d %d %d %d %d %d %d"),
        new Entry("0056f7c0", FILES, "StairsRow_Construct", false, HOUSE_ROW + "stairs.txt", "calledby:00575bf0"),
        new Entry("0056f810", FILES, "StairsRow_Parse", false, HOUSE_PARSE + "13 of 15 numbers are read",
            "string:%d %d %d %d %d %d %d %d %d %d %d %d %d %d %d"),
        new Entry("0056f870", FILES, "RoofRow_Construct", false, HOUSE_ROW + "roof.txt", "calledby:00575bf0"),
        new Entry("0056f8c0", FILES, "RoofRow_Parse", false, HOUSE_PARSE + "17 of 20 numbers are read",
            "string:%d %d %d %d %d %d %d %d %d %d %d %d %d %d %d %d %d %d %d %d"),
        new Entry("00443770", FILES, "SuppInfo_Load", false, "suppinfo.txt: fgets 255 bytes, lines 1 and 2"
            + " skipped, sscanf \"%s\" into 32 bytes then 15 \"%d\" (the tile, then 14 flags kept as != 0); a"
            + " line with fewer than all 16 is dropped", "string:suppinfo.txt", "string:%s %d %d"),
        new Entry("005252b0", FILES, "Professions_Load", false, "prof.txt: fgets 255 bytes; a line holding"
            + " \"Begin\" starts a profession, one holding \"End\" adds it; else strtok keywords (case-insensitive)"
            + " Name, TrueName (strtok to the next quote), Desc, NameId, DescId, TopLevel, Gump, Type, Children,"
            + " Skill, Stat Str/Dex/Int; others are ignored", "string:prof.txt", "string:Begin",
            "calls:0051dbe0"),
        new Entry("0051dbe0", FILES, "SkillId_FromName", false, "case-insensitive skill name to id 0 (Alchemy)"
            + " to 57 (Throwing); -1 for any other name", "string:Mysticism", "string:Throwing"),
        new Entry("00589820", FILES, "KeyNamesAndMacros_Load", false, "keynames.txt: 89 lines read with"
            + " KeyNames_ReadLine, the name of scan codes 0 to 88 (MapVirtualKeyA); then macros2d.txt",
            "string:keynames.txt", "calls:00588950"),
        new Entry("00588950", FILES, "KeyNames_ReadLine", false, "fgetc to a CR or LF (CR LF and LF CR are one"
            + " line end, CR CR two), at most 510 bytes kept", "calledby:00589820"),
        new Entry("0056a100", FILES, "Palette_Load", false, "palette.mul: fread 0x300 bytes, 256 RGB triples"
            + " (the 8-bit desktop palette); then AnimInfo_Load", "string:palette.mul", "calls:0056a970"),
        new Entry("0056a970", FILES, "AnimInfo_Load", false, "animinfo.mul: 2000 two-byte entries read a byte"
            + " at a time; a zero byte in an entry sets it to 4, 2 and reports the file corrupted; without the"
            + " file every entry is 4, 2", "string:animinfo.mul"),
    };

    private static final Build[] BUILDS = {
        new Build("classic_7.0.103.0_client.exe (Classic Client 7.0.103.0)", CLASSIC_7_0_103_SHA256, TAG_FILES,
            TAG_FILES_COMMENT, ENTRIES_CLASSIC_7_0_103),
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
            throw new IllegalStateException("NameClassicTextTableCode names fixed addresses of the tested files (SHA-256 "
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
            output.println("# Classic data file names");
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
