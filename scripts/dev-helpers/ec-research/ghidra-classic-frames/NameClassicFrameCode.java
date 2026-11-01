// Name the Classic client code (7.0.103.0 client.exe) that reads and draws mobile animation frames: the anim*.mul
// and the AnimationFrame*.uop (build/animationlegacyframe) frame lookups, the AMOU frame-entry search, and the
// Classic RLE frame drawers (runs of palette indices, a 0x7FFF7FFF end marker).
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

public class NameClassicFrameCode extends GhidraScript {
    private static final String APPLY_FLAG = "--apply";
    // Script arguments are split on whitespace, so a path with spaces is passed with %20.
    private static final String ENCODED_SPACE = "%20";
    private static final String CLASSIC_7_0_103_SHA256 =
        "4d857f2f5f8bf350bc50a87ee66b2236bfa480e712e9546c740ff04f9106690a";
    private static final String TAG = "UO_CLASSIC_FRAMES";
    private static final String TAG_COMMENT = "Classic animation frames: anim*.mul and animationlegacyframe UOP"
        + " frame lookups and the RLE frame drawers";
    private static final String COMMENT_PREFIX = "NameClassicFrameCode: ";
    private static final String PROBABLE = " (probable)";
    private static final int HEX_RADIX = 16;

    private static final String ANIM = "UOClassicAnim";
    private static final String FILES = "UOClassicFiles";
    private static final String MOB_FRAME_SET = "UOMobFrameSet";
    private static final String FRAME_SET_FACTORY = "UOAnimationFrameSetFactory";

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
        new Entry("00449c90", ANIM, "DrawFrameRle", false, "draws a Classic RLE frame: each run header u32 has the"
            + " length in bits 0..11, the y offset in bits 12..21 and the x offset in bits 22..31 (both signed 10-bit,"
            + " relative to the anchor), then that many palette indices; 0x7FFF7FFF ends the frame. Every index of a"
            + " run is drawn as palette[index] (no transparent index or colour); stops at the first run below the clip"
            + " box, so runs must come in ascending rows; mirrored frames read each run backwards at -x - length."
            + " Hue modes: 0x3fff bits = hue + 1, 0x8000 = only grey colours ((c & 0x3ff) == c >> 5) take the hue",
            "const:7fff7fff", "const:fff", "calledby:00452ad0"),
        new Entry("00449c30", ANIM, "DarkenPixel", true, "DrawFrameRle's modes with hue bit 0x4000 (every pixel"
            + " without 0x8000, the greys with it): darkens the destination pixel by level 0x17 - (colour >> 10),"
            + " clamped at 0, through the table at 0x00788fd8 (0x20 words a level); the pixel is drawn whatever its"
            + " colour, 0 included", "calledby:00449c90"),
        new Entry("00449aa0", ANIM, "DrawFrameShadowRle", false, "the mobile shadow: walks the same runs, keeps the"
            + " rows with an even y offset at y / 2, skews x by the row, and halves the pixels under each run"
            + " (>> 1 & 0x3def); stops at the first run below the clip box", "const:7fff7fff", "const:3def"),
        new Entry("0044a4c0", ANIM, "FrameRleHitTest", false, "whether a point is on a frame pixel: walks the runs,"
            + " returns 0 at the first run below the point's row (runs in ascending rows), 1 when a run of that row"
            + " covers it", "const:7fff7fff", "calledby:0044fd50"),
        new Entry("005d4dc0", ANIM, "DrawFrameRleToBuffer", true, "draws a frame's runs into a 64 x 128 buffer"
            + " (anchor 32, 80) with the hue modes of DrawFrameRle", "const:7fff7fff", "const:50"),
        new Entry("00543500", ANIM, "GetMulFrame", false, "anim*.mul: the index entry of the animation"
            + " (ReadMulIndexEntry), its 256 RGB555 palette words copied as they are (no bit masked), then frame N:"
            + " N < the u32 frame count at +0x200, the u32 offset at +0x204 + 4N not 0 (0 = no frame), the frame"
            + " at +0x200 + offset: i16 anchor x, i16 anchor y, i16 width, i16 height, then the runs",
            "calls:0056c170"),
        new Entry("00543e50", ANIM, "GetUopFrame", false, "AnimationFrame*.uop (build/animationlegacyframe): the"
            + " entry's frame by (action, frame number) through FindFrameEntry; the frame data at entry + its"
            + " u32 at +0x0c: 256 RGB555 palette words copied as they are, i16 anchor x, anchor y, width, height"
            + " at +0x200..+0x206, the runs at +0x208; a width or height of 0 is no frame", "calls:00542c40",
            "calls:00543890"),
        new Entry("00542c40", ANIM, "FindFrameEntry", false, "binary search of the AMOU frame table (count at"
            + " header +0x20, table at header + the u32 at +0x24, 16-byte entries) by u16 action at +0 then u16"
            + " frame number at +2: the table must be sorted by (action, frame number). Reads nothing else of the"
            + " header or the entry boxes", "calledby:00543e50"),
        new Entry("005bc880", ANIM, "GetMobileFrame", false, "a mobile's current frame: the anim*.mul frame"
            + " (GetMulFrame) when 0x005978a0(body) is false, else the AnimationFrame*.uop one (GetUopFrame)",
            "calls:00543500", "calls:00543e50", "const:6000"),
        new Entry("00452ad0", ANIM, "DrawMobile", true, "draws a mobile and its equipment layers: GetMobileFrame"
            + " for each, mirrored anchor x = width - anchor x, DrawFrameRle", "calls:005bc880", "calls:00449c90"),
        new Entry("0056c170", FILES, "ReadMulIndexEntry", true, "the 12-byte index entry N of a MUL pair (u32"
            + " lookup, length, extra); a negative lookup is no entry; returns the data pointer", "calledby:00543500"),
        new Entry("00543890", ANIM, "GetUopFrameSet", true, "the frame set asset of a body and action"
            + " (CUOAssetAnimation, then the UOMobFrameSet of the AnimationFrame*.uop entry)", "calledby:00543e50"),
        new Entry("00413752", MOB_FRAME_SET, "Construct", false, "sets the UOMobFrameSet vtable 0x006db10c, keeps"
            + " the entry stream and reads three u32 of its header", "const:6db10c", "calledby:0041385a"),
        new Entry("0041385a", FRAME_SET_FACTORY, "CreateAsset", false, "UOAnimationFrameSetFactory vtable 0x006db0f8"
            + " slot 1: a UOMobFrameSet from the archive entry; \"Failed to get animation frame set ...\"",
            "ref:006db0fc", "string:Failed to get animation frame set"),
    };

    private static final Build[] BUILDS = {
        new Build("classic_7.0.103.0_client.exe (Classic Client 7.0.103.0)", CLASSIC_7_0_103_SHA256,
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
            throw new IllegalStateException("NameClassicFrameCode names fixed addresses of the tested files (SHA-256 "
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
            output.println("# Classic animation frame names");
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
