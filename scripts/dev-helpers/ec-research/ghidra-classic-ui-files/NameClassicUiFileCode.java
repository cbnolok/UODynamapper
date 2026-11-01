// Name the code of the 2024 Classic client (7.0.103.0 client.exe) that reads and writes entitled.def, the
// desktop (desktop.nwb, desktop.uos), credits.dat, helpindx.dat, Data/assetmap.xml and the chat filter lists.
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

public class NameClassicUiFileCode extends GhidraScript {
    private static final String APPLY_FLAG = "--apply";
    // Script arguments are split on whitespace, so a path with spaces is passed with %20.
    private static final String ENCODED_SPACE = "%20";
    private static final String CLASSIC_7_0_103_SHA256 =
        "4d857f2f5f8bf350bc50a87ee66b2236bfa480e712e9546c740ff04f9106690a";
    private static final String TAG_FILES = "UO_CLASSIC_UI_FILES";
    private static final String TAG_FILES_COMMENT =
        "Classic entitled.def, desktop, credits.dat, helpindx.dat, assetmap.xml, chat filter lists";
    private static final String COMMENT_PREFIX = "NameClassicUiFileCode: ";
    private static final String PROBABLE = " (probable)";
    private static final int HEX_RADIX = 16;

    private static final String FILES = "UOClassicUiFiles";

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
        new Entry("005fb3c0", FILES, "Entitlement_LoadCache", false, "entitled.def opened rb at start-up: one"
            + " fread of 4 bytes (0 when the file is missing) handed to ClientFeatures_Set",
            "string:entitled.def", "calls:005641a0"),
        new Entry("0046f970", FILES, "Entitlement_OnFeaturesPacket", true, "reads a big-endian u32 from the"
            + " packet (the 0xB9 feature flags), hands it to ClientFeatures_Set and writes it to entitled.def"
            + " wb as 4 little-endian bytes: the file is a client-written cache", "string:entitled.def",
            "calls:005641a0"),
        new Entry("005641a0", FILES, "ClientFeatures_Set", true, "sets the client feature flags from a u32",
            "calledby:005fb3c0", "calledby:0046f970"),
        new Entry("004f81d0", FILES, "Desktop_LoadForCharacter", false, "loads Desktop/<account>/<shard>/"
            + "<character>/desktop.uos or Desktop/common/desktop.uos; the shipped desktop.nwb when the names"
            + " are empty or no saved desktop exists", "string:desktop.nwb", "string:desktop.uos",
            "calls:004f6bf0"),
        new Entry("004f6bf0", FILES, "Desktop_Load", false, "CGumpManager::LoadDesktop: a u32 serial (replaced"
            + " by the player's), then u8-tagged gump records to EOF; kinds 18 and >= 31 stop the read",
            "calledby:004f81d0"),
        new Entry("004e33b0", FILES, "Desktop_Save", true, "CGumpManager::SaveDesktop: writes desktop.uos and"
            + " rotates desktop.uo1..uo6, never desktop.nwb", "string:desktop.uo1"),
        new Entry("00560350", FILES, "Credits_Load", false, "credits.dat opened rt, lines read with"
            + " KeyNames_ReadLine; a first token \"!\" (strtok \" \\t\\n\\r\\f=\") makes a directive line: color,"
            + " defaultcolor, font, defaultfont, song; any other line is text", "string:credits.dat",
            "calls:00588950", "calls:00578e50"),
        new Entry("00578e50", FILES, "Credits_ParseColour", false, "#RRGGBB hex or a decimal (a leading '-'"
            + " keeps the raw atol value), converted to RGB555", "calledby:00560350"),
        new Entry("0055e120", FILES, "HelpIndex_Load", false, "helpindx.dat opened r, fgets 512: '#' comments;"
            + " else an optional '@', then tab fields: topic id and three cliloc numbers (all four nonzero or"
            + " the row is dropped), then linked topic ids", "string:helpindx.dat"),
        new Entry("004200ba", FILES, "AssetMap_Load", false, "TinyXML load of Data/assetmap.xml through"
            + " VirtualFile_Open in UOP-only mode (MainMisc.uop); the root must be <assetmap>; each child is"
            + " offered to the simplemap, patternmap and bitmaskmap handlers", "string:Data/assetmap.xml",
            "calls:00419a90"),
        new Entry("0041f9ac", FILES, "AssetMap_TrySimpleMap", false, "node name \"simplemap\" (case-sensitive)"
            + " then AssetMap_ParseSimpleMap", "string:simplemap", "calls:0041f829"),
        new Entry("0041f829", FILES, "AssetMap_ParseSimpleMap", false, "resourcetype (low byte), resourcenumber"
            + " (u32) else id (u64), filename, fileFormat (raw 0, packed 1, else raw)", "calledby:0041f9ac",
            "calls:00417454", "calls:00417509"),
        new Entry("0041fabc", FILES, "AssetMap_ParsePatternMap", false, "resourcetype, fileFormat, directory,"
            + " prefix, suffix, padding (u32), patternMatch (i32, != 0)", "calls:00417454", "calls:00417539",
            "string:patternMatch"),
        new Entry("0041fdd0", FILES, "AssetMap_ParseBitmaskMap", false, "resourcetype, fileFormat, directory,"
            + " suffix; section children: prefix, padding (u32), mask (hex), patternMatch and isDirectory (u32,"
            + " != 0)", "calls:00417498", "string:section"),
        new Entry("00417454", FILES, "XmlAttribute_GetU32", false, "strtoul base 10; the whole value must parse,"
            + " else the attribute counts as absent", "calledby:0041f829"),
        new Entry("00417539", FILES, "XmlAttribute_GetI32", false, "strtol base 10", "calledby:0041fabc"),
        new Entry("004175f6", FILES, "XmlAttribute_GetString", false, "the attribute's text", "calledby:0041f829"),
        new Entry("00417498", FILES, "XmlAttribute_ParseHexU32", false, "strtoul base 16", "calledby:0041fdd0"),
        new Entry("00417509", FILES, "XmlAttribute_ParseU64", false, "strtoui64 base 10", "calledby:0041f829"),
        new Entry("00429f35", FILES, "VirtualFile_Open", false, "mode in ECX (0 means 2): 1 disk only, 2 the"
            + " loaded UOP packages only (name hashed), otherwise disk then UOP", "calledby:0042a2e3"),
        new Entry("005a1480", FILES, "ChatFilterLists_Load", false, "uobscene.lst (words through"
            + " Uobscene_ReadWord, empty ones skipped, 30-unit slots, 120 slots before the next table),"
            + " ignore.lst and conf_ignore.lst (30-byte name + u32 serial records)", "string:uobscene.lst",
            "string:ignore.lst", "calls:005a13b0"),
        new Entry("005a13b0", FILES, "Uobscene_ReadWord", false, "UTF-16 units minus 13 to a CR or LF (CR LF and"
            + " LF CR are one end); no length limit (a global buffer); a last word without its end gets an"
            + " extra unit", "calledby:005a1480"),
        new Entry("0059ea00", FILES, "ChatFilterLists_Save", false, "list 0: uobscene.lst, every word's units"
            + " plus 13 then 0x17 (LF plus 13); list 1: ignore.lst and conf_ignore.lst", "string:uobscene.lst",
            "string:conf_ignore.lst"),
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
            throw new IllegalStateException("NameClassicUiFileCode names fixed addresses of the tested files (SHA-256 "
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
            output.println("# Classic UI file names");
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
