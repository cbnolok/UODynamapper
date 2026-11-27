// Name the Classic client code (7.0.102.3 client.exe) that picks and paces a mobile's movement actions: the mobile
// tick (step ticks, standing, frames in place, idle fidgets), the UOP move action from the AnimationSequence groups,
// and the mobtypes.txt queries the MUL action choice (GetMobileMulFrame) reads.
// Address-based: one entry set per tested file, selected by SHA-256; any other program is refused. Every
// address is verified with the stated checks first; a failed check is reported, not named. Names go in the global
// namespace, as the move code was first named there.
// Findings: results/classic_frames.md.
// @category ECResearch
import ghidra.app.script.GhidraScript;
import ghidra.program.model.address.Address;
import ghidra.program.model.data.StringDataInstance;
import ghidra.program.model.listing.Data;
import ghidra.program.model.listing.Function;
import ghidra.program.model.listing.FunctionTag;
import ghidra.program.model.listing.Instruction;
import ghidra.program.model.scalar.Scalar;
import ghidra.program.model.symbol.Reference;
import ghidra.program.model.symbol.SourceType;
import ghidra.program.model.symbol.Symbol;
import java.io.PrintWriter;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardOpenOption;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

public class NameClassicMoveCode extends GhidraScript {
    private static final String APPLY_FLAG = "--apply";
    // Script arguments are split on whitespace, so a path with spaces is passed with %20.
    private static final String ENCODED_SPACE = "%20";
    private static final String CLASSIC_7_0_103_SHA256 =
        "4d857f2f5f8bf350bc50a87ee66b2236bfa480e712e9546c740ff04f9106690a";
    private static final String TAG = "UO_CLASSIC_MOVES";
    private static final String TAG_COMMENT = "Classic mobile movement actions: the mobile tick, the UOP move action"
        + " and the mobtypes.txt queries of the MUL action choice";
    private static final String COMMENT_PREFIX = "NameClassicMoveCode: ";
    private static final String PROBABLE = " (probable)";
    private static final int HEX_RADIX = 16;

    // Checks: "calls:ADDR" (the function references ADDR: call, pointer, jump or data),
    // "calledby:ADDR" (the function at ADDR references this one), "ref:ADDR" (the data or code at ADDR
    // references this function, e.g. a vtable slot), "string:TEXT" (references a string that starts with
    // TEXT), "import:NAME" (references the external symbol NAME), "const:HEX" (an operand equals HEX).
    private record Entry(String address, String name, boolean probable, String evidence, String... checks) {
    }

    private record Build(String label, String sha256, Entry[] entries) {
    }

    private static final Entry[] ENTRIES_CLASSIC_7_0_103 = {
        new Entry("005a4a00", "GameLoop_Tick", false, "calls TickMobiles when more than 75 ms passed since the last"
            + " call (compared with 0x4c)", "calls:005bc160", "const:4c"),
        new Entry("005bc160", "TickMobiles", false, "the mobile tick: while a step runs, the frame counter +0x1a0"
            + " advances once a tick and +0x19c counts the step's ticks down (CMobile_StepTicks); +0x1a4 is the run"
            + " bit (queued direction & 0x80); more than 2000 ms after the last step (+0x1ac) +0x1a8 (standing) is"
            + " set and the counter reset to 3, at once for IsStandAtOnceBody; otherwise the counter advances while"
            + " CMobile_AnimatesInPlace; a flier (+0xcc & 4) with no step ticks left stands at once while the flying"
            + " gate DAT_00742cf8 is off, and in peace with nothing playing (+0x21c) its +0x1ac moves back 15000 ms;"
            + " then CMobile_TryIdleFidget", "calledby:005a4a00", "calls:005b2310", "calls:005b2f20", "calls:005b2e80",
            "calls:005bbf20", "const:7d0"),
        new Entry("005b2310", "CMobile_StepTicks", false, "ticks a step lasts: the animinfo.mul byte of the body"
            + " (0x00a59e30 walk, 0x00a59e31 run with +0x1a4; 2 bytes a body; a body above 1999 reads body 0x32);"
            + " mounted (+0x204), the walk byte + (+0x214 / 11) * 2", "calledby:005bc160"),
        new Entry("005b2f20", "IsStandAtOnceBody", false, "bodies 0x50 and 0x51 stand as soon as they stop",
            "calledby:005bc160", "const:50", "const:51"),
        new Entry("005b2e80", "CMobile_AnimatesInPlace", false, "the frame counter runs while not stepping: a flier"
            + " (+0xcc & 4 with the flying gate), bodies 0xd 0xf 0x10 0x23e 0x2b4 0x57e 0x593 0x597 0x598, 0x599"
            + " standing, or, for a body from 400, a layer-2 item with tile flag 0x800000 (bodies below 400 return"
            + " before that test)", "calledby:005bc160", "const:800000"),
        new Entry("005bbf20", "CMobile_TryIdleFidget", false, "the idle fidget: nothing when +0x208 is 1, +0x188 is"
            + " set, the count n at DAT_00afb744 is above 20 or DAT_00a58bbc is set; more than 15000 ms after the"
            + " last step (+0x1ac), r = rand % (n * 4 + 120) below 2 plays a fidget (CMobile_PlayAnimation, action and"
            + " frame count): monster 18 (r 0) or 17, 5 frames; sea monster 4 with 20 or 3 with 15; animal 10 with 3"
            + " (r 0 without MobTypes_Flag2) or 9 with 5; human or equipment 6 (r 0) or 5, 5 frames; a UOP body"
            + " plays a type-5 action of its sequence group (CMobile_SetUopAction)",
            "calledby:005bc160", "const:3a98", "const:78", "calls:005b6270", "calls:005b6580", "calls:005b6cc0",
            "calls:005976b0", "calls:00597780"),
        new Entry("005b6e40", "CMobile_UpdateUopMoveAction", false, "a UOP body with no server animation: movement 0"
            + " standing (+0x1a8), else 1 walk or 2 run (+0x1a4); action = FrameSetLookup_GetMoveAction(group of"
            + " CMobile_UopSequenceGroup, movement, alternate from the tile definition of the layer-2 item +0x12c);"
            + " CMobile_SetUopAction when it differs from +0x228; the counter +0x1a0 wraps at the frame count +0x224",
            "calledby:005441e0", "calls:00408114", "calls:005b6580"),
        new Entry("005b6270", "CMobile_UopSequenceGroup", false, "the sequence group: 2 mounted when the set has"
            + " group 2 and +0x1fc is set; a flier (gate and +0xcc & 4) 4 in war (+0x1b0), else 5; 0 in war unless"
            + " vtable +0xc0; 3 for a running MobTypes_CanFly body; 6 when FUN_005d6d30; else 1 peace",
            "calledby:005bbf20", "calls:00597960", "calls:0044d250"),
        new Entry("005b6580", "CMobile_SetUopAction", false, "sets the action +0x228 and resets the frame +0x220"
            + " and the counter +0x1a0; a UOP body takes its frame count +0x224 and frame time +0x238 = 1000 / fps"
            + " from the sequence, any other the frame time 62.0 ms (0x42780000)", "calledby:005b6e40",
            "calledby:005b6cc0"),
        new Entry("005b6cc0", "CMobile_PlayAnimation", false, "a server-style animation (action, frame count,"
            + " repeat count, forward, repeat, delay); sets +0x21c (playing)", "calledby:005bbf20", "calls:005b6580"),
        new Entry("00408114", "FrameSetLookup_GetMoveAction", false, "the move action of a sequence group: the"
            + " movement entry of the first group of the parent chain that has one (the alternate action when asked),"
            + " else the default action of the chain's last group", "calledby:005b6e40"),
        new Entry("00597ae0", "MobTypes_IsPeopleGroup", false, "mobtypes.txt flag 0x400: a monster in the people"
            + " layout (every movement action 0)", "calledby:0044e8e0", "const:800"),
        new Entry("00597a00", "MobTypes_IsLowGroup", false, "mobtypes.txt flag 0x40: a monster in the animal layout",
            "calledby:0044e8e0", "const:800"),
        new Entry("005979d0", "MobTypes_IsLowGroupExtended", false, "mobtypes.txt flag 0x20: an animal takes the"
            + " monster rules", "calledby:0044e8e0", "const:800"),
        new Entry("00597960", "MobTypes_CanFly", false, "mobtypes.txt flag 0x8, only while the flying gate"
            + " DAT_00742cf8 is set: a monster runs with action 19", "calledby:0044e8e0", "calledby:005b6270",
            "const:8", "const:800"),
        new Entry("00597870", "MobTypes_IdleAt8", false, "mobtypes.txt flag 0x4: a monster stands with action 8",
            "calledby:0044e8e0", "const:800"),
        new Entry("005977e0", "MobTypes_IsHuman", false, "mobtypes.txt type 3 (HUMAN): the only type that takes the"
            + " mounted actions", "calledby:0044e8e0", "const:3", "const:800"),
        new Entry("00597780", "MobTypes_Flag2", false, "mobtypes.txt flag 0x2: picks the fidget and attack variant",
            "calledby:005bbf20", "const:2", "const:800"),
        new Entry("005976b0", "MobTypes_GroupType", false, "the mobtypes.txt type of a body; 4 (EQUIPMENT) without a"
            + " row", "calledby:0044e8e0", "calledby:005bbf20", "const:800"),
        new Entry("0044d250", "MobTypes_EnsureTable", false, "allocates the 0x800-row mobtypes.txt table on first"
            + " use", "calledby:005b6270", "calledby:005bbf20", "const:6000"),
    };

    private static final Build[] BUILDS = {
        new Build("classic_7.0.103.0_client.exe (Classic Client 7.0.102.3)", CLASSIC_7_0_103_SHA256,
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
            throw new IllegalStateException("NameClassicMoveCode names fixed addresses of the tested files (SHA-256 "
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
            output.println("# Classic movement action names");
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
        String qualified = entry.name();
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
            if (!currentProgram.getSymbolTable().getSymbols(entry.name(), currentProgram.getGlobalNamespace())
                    .isEmpty()) {
                return row + "conflict: `" + qualified + "` already names another symbol |";
            }
            if (apply) {
                symbol.setNameAndNamespace(entry.name(), currentProgram.getGlobalNamespace(), SourceType.ANALYSIS);
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
