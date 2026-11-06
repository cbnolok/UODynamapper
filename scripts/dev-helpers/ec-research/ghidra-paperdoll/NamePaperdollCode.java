// Name the code of the Enhanced Client (UOSA.exe) and Kingdom Reborn (UOKR.exe) that reads Paperdoll.uop: the
// loader of its two AMOU frame sets (assets 0x40 and 0x41 of type 0x12, PaperdollFrameSet), the frame lookup by
// gender and id, the two paperdoll texture builders that use them, and the CPU draw of one gump layer of the
// gump paperdoll (UOPaperdoll::BuildImage), which applies the partial hue.
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

public class NamePaperdollCode extends GhidraScript {
    private static final String APPLY_FLAG = "--apply";
    // Script arguments are split on whitespace, so a path with spaces is passed with %20.
    private static final String ENCODED_SPACE = "%20";
    private static final String UOSA_SHA256 = "5a5e9dc536c836d1bf10b6f86bb88f61a6ae078f519d4060d3bc47aecbebad50";
    private static final String UOKR_SHA256 = "1b802e5c00f3bf89a771778b9c6c57406be0703808eeb4cd983e2f5baa348a43";
    private static final String TAG = "UO_PAPERDOLL";
    private static final String TAG_COMMENT = "Paperdoll: the Paperdoll.uop frame sets and the paperdoll textures"
        + " built from them";
    private static final String COMMENT_PREFIX = "NamePaperdollCode: ";
    private static final String PROBABLE = " (probable)";
    private static final int HEX_RADIX = 16;

    private static final String PAPERDOLL = "PaperdollTextures";
    private static final String EC_TEXTURE = "ECPaperdollTexture";
    private static final String CC_TEXTURE = "CCPaperdollTexture";
    private static final String GUMP_PAPERDOLL = "UOPaperdoll";

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
        new Entry("0050a2e0", PAPERDOLL, "LoadPaperdollFrameSets", false, "once: requests assets 0x40 and 0x41 of"
            + " type 0x12 (PaperdollFrameSet; Paperdoll.uop build/paperdoll/000001/00.bin and 01.bin) through"
            + " 0x00a6ced4 (GetAsset for each listed asset), keeps them in 0x00e33d3c (00.bin, male) and"
            + " 0x00e33d44 (01.bin, female); fails while either is missing", "const:12000000", "const:40", "const:41",
            "calls:00a6ced4"),
        new Entry("0050b780", PAPERDOLL, "FindPaperdollFrame", false, "UOAnimations::FindFrameEntry(frame set,"
            + " action = gender at +0x8c, number = id at +0x50, less 10000000 when above it); a miss falls back to"
            + " (action 0, number 400) of the male set, else fails; then BuildFramePalette(frame set, hue at"
            + " +0x54)", "calls:00430f60", "calls:0042b270", "const:190", "const:989680"),
        new Entry("0050b340", EC_TEXTURE, "Build", true, "vtable 0x00ca9298 slot 2: LoadPaperdollFrameSets, the"
            + " mobile's object definition (vtable +0xd4: gender) picks the female set (+0x88, action 1) or the"
            + " male one (action 0), FindPaperdollFrame, then the layer frames", "ref:00ca92a0", "calls:0050a2e0",
            "calls:0050b780"),
        new Entry("0050bfc0", CC_TEXTURE, "Build", true, "vtable 0x00ca92b8 slot 2: like the EC texture, the gender"
            + " from the object definition of id +0x50 (10000000 added below 10000001)", "ref:00ca92c0",
            "calls:0050a2e0", "calls:0050b780", "const:989680"),
        new Entry("0050b850", PAPERDOLL, "FindLayerFrame", false, "frame of a worn item: follows the object"
            + " definition's Appearance (property 8, definition id + 10000000) up to 5 levels, then"
            + " FindFrameEntry(set, action = gender, number = definition vtable +0xbc)", "calls:00430f60",
            "calls:005bda40", "const:989680"),
        new Entry("0050ba30", EC_TEXTURE, "GetLayerFrame", true, "vtable 0x00ca9298 slot 6: the item of a"
            + " layer, its Paperdoll property (11, definition id + 10000000), FindLayerFrame in the gender's set,"
            + " a miss retried in the male set with action 0; palette with the item's hue", "ref:00ca92b0",
            "calls:0050b850", "calls:0042b270"),
        new Entry("0050c550", CC_TEXTURE, "GetLayerFrame", true, "vtable 0x00ca92b8 slot 6: as the EC texture",
            "ref:00ca92d0", "calls:0050b850", "calls:0042b270"),
        new Entry("0050a240", PAPERDOLL, "ReleasePaperdollFrameSets", true, "clears the paperdoll texture cache"
            + " and both frame set pointers 0x00e33d3c and 0x00e33d44", "calls:0040c5f0"),
        new Entry("00509b30", GUMP_PAPERDOLL, "DrawGumpLayer", false, "(gump id, target NiPixelData, layer map,"
            + " layer, hue, partial, key r, g, b): CPU copy of one Interface.uop gump (%08d.tga) into the paperdoll"
            + " image; skips a 256x256 gump; the key colour defaults to the gump's first texel and texels whose"
            + " RGB equals it are skipped (alpha is not read); with a hue, a texel is hued when partial is false or"
            + " its bytes are exact greys (R == G and R == B, 0x00509d64 and 0x00509d6c), through"
            + " UOHues::GetHueTable(hue)[R], blended over the target by the entry's alpha byte when below 255",
            "calls:0051af10", "calls:00427ad0", "calledby:0050c790"),
    };

    private static final Entry[] ENTRIES_UOKR = {
        new Entry("004d9f80", PAPERDOLL, "LoadPaperdollFrameSets", false, "requests assets 0x40 and 0x41 of type"
            + " 0x12 (PaperdollFrameSet), keeps them in 0x00c93c78 and 0x00c93c8c", "const:12000000", "const:40",
            "const:41"),
    };

    private static final Build[] BUILDS = {
        new Build("UOSA.exe (Enhanced Client)", UOSA_SHA256, ENTRIES_UOSA),
        new Build("UOKR.exe (Kingdom Reborn)", UOKR_SHA256, ENTRIES_UOKR),
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
            throw new IllegalStateException("NamePaperdollCode names fixed addresses of the tested files (SHA-256 "
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
            output.println("# Paperdoll names");
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
