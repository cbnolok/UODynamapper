// Name the AnimationFrame*.uop (AMOU) code of UOSA.exe (Enhanced Client) and UOKR.exe (Kingdom Reborn): the asset
// factory and the UOMobFrameSet constructor, the frame entry binary search, the palette builder and the hue table
// getter, the RLE decoders, the mask decoder, the layer bounds and the frame atlas. The same evidence finds the same
// functions in both clients; no fixed addresses are used.
// With --apply, rename DEFAULT functions, add the UO_ANIMATION_FRAME tag and plate comments.
// @category ECResearch
import ghidra.app.script.GhidraScript;
import ghidra.program.model.address.Address;
import ghidra.program.model.listing.Function;
import ghidra.program.model.listing.FunctionManager;
import ghidra.program.model.listing.FunctionTag;
import ghidra.program.model.listing.Instruction;
import ghidra.program.model.listing.Listing;
import ghidra.program.model.mem.Memory;
import ghidra.program.model.mem.MemoryBlock;
import ghidra.program.model.scalar.Scalar;
import ghidra.program.model.symbol.Namespace;
import ghidra.program.model.symbol.Reference;
import ghidra.program.model.symbol.SourceType;
import ghidra.program.model.symbol.Symbol;
import ghidra.program.model.symbol.SymbolTable;
import ghidra.util.exception.DuplicateNameException;
import java.io.PrintWriter;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardOpenOption;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Map;
import java.util.TreeMap;

public class NameAnimationFrameCode extends GhidraScript {
    private static final String APPLY_FLAG = "--apply";
    private static final String TAG = "UO_ANIMATION_FRAME";
    private static final String TAG_COMMENT = "AnimationFrame*.uop (AMOU) asset: factory, frame entry search, palette,"
        + " RLE decoders and atlas";
    private static final String COMMENT_PREFIX = "NameAnimationFrameCode: ";
    private static final String PROBABLE = " (probable)";
    private static final String SCOPE = "::";
    private static final String NAMESPACE = "UOAnimations";
    private static final String HUES = "UOHues";

    // MSVC RTTI: TypeDescriptor name at +8; CompleteObjectLocator {signature, offset, cdOffset, pTypeDescriptor}.
    private static final int TYPE_DESCRIPTOR_NAME_OFFSET = 8;
    private static final int COL_TYPE_DESCRIPTOR_OFFSET = 12;
    private static final int COL_OFFSET_FIELD = 4;
    private static final String RTTI_PREFIX = ".?AV";
    private static final String RTTI_SUFFIX = "@@";
    private static final int POINTER_SIZE = 4;
    private static final long DWORD_MASK = 0xffffffffL;
    private static final int BYTE_MASK = 0xff;
    private static final int BITS_PER_BYTE = 8;

    private static final String FRAME_FACTORY = "UOAnimationFrameSetFactory";
    private static final String MOB_FRAME_SET = "UOMobFrameSet";
    // Asset factory vtable: +4 Create(out, id, type, blob).
    private static final int FACTORY_CREATE = 0x4;

    // The RLE decoders blend with the packed red/blue mask and test run bytes against 0x80; hue 0x4001 is the
    // special (shadow) hue of the hue table getter.
    private static final long PACKED_RED_BLUE_MASK = 0xff00ff;
    private static final long RLE_RUN_FLAG = 0x80;
    private static final long SHADOW_HUE = 0x4001;
    // AnimationFrame header fields read by the frame entry search: +0x20 frames count, +0x24 frames offset.
    private static final String FRAMES_COUNT_OPERAND = "+ 0x20]";
    private static final String FRAMES_OFFSET_OPERAND = "+ 0x24]";
    private static final String HALVING_MNEMONIC = "SAR";
    private static final long HALVING_SHIFT = 1;
    // Small helpers only: the search and the hue table getter are under this many bytes.
    private static final long SMALL_FUNCTION_BYTES = 256;

    private static final class Proposal {
        long address;
        String path;
        String evidence;
        boolean probable;
        String status = "";
    }

    private static final class Block {
        long start;
        byte[] bytes;
        boolean exec;
    }

    private Memory memory;
    private Listing listing;
    private FunctionManager functions;
    private SymbolTable symbols;
    private final List<Block> blocks = new ArrayList<>();
    private final Map<String, Long> vtables = new LinkedHashMap<>();
    private final Map<Long, Proposal> proposals = new TreeMap<>();
    private final List<String> anomalies = new ArrayList<>();
    private final List<String> kept = new ArrayList<>();
    private final List<String> anchors = new ArrayList<>();
    private final Map<String, Integer> changes = new LinkedHashMap<>();

    @Override
    public void run() throws Exception {
        String[] args = getScriptArgs();
        if (args.length < 1) {
            throw new IllegalArgumentException("Expected OUTPUT_FILE [" + APPLY_FLAG + "]");
        }
        boolean apply = false;
        for (int i = 1; i < args.length; i++) {
            if (args[i].equals(APPLY_FLAG)) {
                apply = true;
            } else {
                throw new IllegalArgumentException("Unknown argument " + args[i]);
            }
        }
        // Created before any change, so an existing output file stops the run unchanged.
        try (PrintWriter output = new PrintWriter(Files.newBufferedWriter(Path.of(args[0]),
                StandardOpenOption.CREATE_NEW))) {
            analyze(output, apply);
        }
        println("Wrote " + args[0] + (apply ? " (applied: " + changes + ")" : ""));
    }

    private void analyze(PrintWriter out, boolean apply) throws Exception {
        memory = currentProgram.getMemory();
        listing = currentProgram.getListing();
        functions = currentProgram.getFunctionManager();
        symbols = currentProgram.getSymbolTable();
        loadBlocks();
        for (String cls : new String[] {FRAME_FACTORY, MOB_FRAME_SET}) {
            long vt = rttiVtable(cls);
            vtables.put(cls, vt);
            anchors.add("RTTI `" + cls + "` vftable " + hexOrNone(vt));
        }
        asset();
        decoders();
        search();

        if (apply) {
            ensureTag();
            for (Proposal proposal : proposals.values()) {
                applyName(proposal);
            }
        } else {
            for (Proposal proposal : proposals.values()) {
                Function f = functions.getFunctionAt(toAddr(proposal.address));
                proposal.status = f.getName(true).equals(proposal.path) ? "already named"
                    : f.getSymbol().getSource() == SourceType.DEFAULT ? "would rename"
                    : "kept " + f.getName(true) + " (" + f.getSymbol().getSource() + ")";
                if (proposal.status.startsWith("kept")) {
                    kept.add(hex(proposal.address) + " `" + f.getName(true) + "` (" + f.getSymbol().getSource()
                        + "), proposed `" + proposal.path + "`");
                }
            }
        }
        report(out, apply);
    }

    // ---- asset factory and UOMobFrameSet ----

    private void asset() {
        long factoryVt = vtables.get(FRAME_FACTORY);
        Function create = factoryVt < 0 ? null : slotFunction(factoryVt, FACTORY_CREATE);
        propose(create, FRAME_FACTORY + SCOPE + "Create", "asset factory vtable +4 (asset type 0x10): allocates a"
            + " UOMobFrameSet over the blob; a failed stream read hands the request to the next resolver", false);
        if (create == null) {
            return;
        }
        List<Function> ctors = new ArrayList<>();
        for (Function w : vtableWriters(MOB_FRAME_SET)) {
            if (realCalls(create).contains(w)) {
                ctors.add(w);
            }
        }
        propose(single(ctors, "UOMobFrameSet constructor"), MOB_FRAME_SET + SCOPE + MOB_FRAME_SET, "reads u32"
            + " signature (+4), version (+8), total size (+0xc) and checks none of them; keeps the blob", false);
    }

    // ---- RLE decoders, atlas, layer bounds, mask ----

    private void decoders() {
        List<Function> decoders = new ArrayList<>();
        for (Function f : scalarUsersInProgram(PACKED_RED_BLUE_MASK)) {
            if (calls(f).isEmpty() && usesScalar(f, RLE_RUN_FLAG)) {
                decoders.add(f);
            }
        }
        List<Function> atlases = new ArrayList<>();
        for (Function d : decoders) {
            for (Function caller : callers(d)) {
                int both = 0;
                for (Function other : decoders) {
                    both += realCalls(caller).contains(other) ? 1 : 0;
                }
                if (both == decoders.size() && decoders.size() > 1 && !atlases.contains(caller)) {
                    atlases.add(caller);
                }
            }
        }
        Function atlas = single(atlases, "frame atlas builder (caller of every RLE decoder)");
        propose(atlas, NAMESPACE + SCOPE + "BuildFrameAtlas", "draws the layers (body, equipment) in order into a"
            + " zeroed 32-bit NiPixelData of power-of-2 sides (at least 4, aspect at most 8:1); layer +0xc mirrored ->"
            + " mirrored decoder; palette **(layer +0x10)", false);
        if (atlas == null) {
            return;
        }
        Function normal = null;
        Function mirrored = null;
        for (Function d : decoders) {
            if (callers(d).size() > 1) {
                normal = d;
            } else {
                mirrored = d;
            }
        }
        propose(normal, NAMESPACE + SCOPE + "DecodeFrameRle", "(EAX frame entry, palette, canvas, pitch): byte c <"
            + " 0x80 skips c pixels (carrying over rows); else flag byte F: F>>4 edge pixel, c-0x80 solid pixels, F&15"
            + " edge pixel; edge = per-channel (src*f + dst*(16-f))>>4; a coded run is written past the row end in"
            + " memory, so it must stay in its row", false);
        propose(mirrored, NAMESPACE + SCOPE + "DecodeFrameRleMirrored", "same RLE, columns written right to left"
            + " from -x0-1 (x -> -1-x)", false);
        List<Function> bounds = new ArrayList<>();
        for (Function c : realCalls(atlas)) {
            boolean shared = !c.isExternal() && calls(c).isEmpty();
            for (Function caller : callers(atlas)) {
                shared &= realCalls(caller).contains(c);
            }
            if (shared && !decoders.contains(c)) {
                bounds.add(c);
            }
        }
        propose(single(bounds, "layer bounds helper"), NAMESPACE + SCOPE + "GetLayerBounds", "union of the layers'"
            + " frame boxes (mirrored: [-x1, -x0]) minus the layer offsets +0x18/+0x1c, 1-pixel margin", true);
        if (normal != null) {
            List<Function> masks = new ArrayList<>();
            for (Function caller : callers(normal)) {
                if (caller.equals(atlas)) {
                    continue;
                }
                for (Function c : realCalls(caller)) {
                    if (!c.equals(normal) && calls(c).isEmpty() && usesScalar(c, RLE_RUN_FLAG)
                            && !usesScalar(c, PACKED_RED_BLUE_MASK) && !masks.contains(c)) {
                        masks.add(c);
                    }
                }
            }
            propose(single(masks, "RLE mask decoder"), NAMESPACE + SCOPE + "DecodeFrameMask", "same RLE into a"
                + " 1-byte-per-pixel mask; every coded pixel (solid and edge) is marked", true);
        }
    }

    // ---- frame entry search, palette builder, hue table getter ----

    private void search() {
        List<Function> searches = new ArrayList<>();
        List<Function> palettes = new ArrayList<>();
        List<Function> hueTables = new ArrayList<>();
        for (Function s : functions.getFunctions(true)) {
            if (!isEntrySearch(s)) {
                continue;
            }
            for (Function caller : callers(s)) {
                for (Function p : realCalls(caller)) {
                    for (Function h : realCalls(p)) {
                        if (h.getBody().getNumAddresses() < SMALL_FUNCTION_BYTES && usesScalar(h, SHADOW_HUE)) {
                            addOnce(searches, s);
                            addOnce(palettes, p);
                            addOnce(hueTables, h);
                        }
                    }
                }
            }
        }
        propose(single(searches, "frame entry search"), NAMESPACE + SCOPE + "FindFrameEntry", "(EAX frame file,"
            + " u16 action, u16 number) binary search over the +0x20 entries at +0x24, unsigned (action, number)"
            + " ascending; returns the 16-byte entry or 0", false);
        propose(single(palettes, "palette builder"), NAMESPACE + SCOPE + "BuildFramePalette", "(ECX hue, EAX frame"
            + " file, out): colours_count (+0x18) 4-byte entries at +0x1c -> opaque 32-bit words; with a hue table"
            + " and byte 3 == 0 the table entry replaces the colour (EC: indexed by byte 0; KR: by (b0+b1+b2)/3)",
            false);
        propose(single(hueTables, "hue table getter"), HUES + SCOPE + "GetHueTable", "hue -> table: 0x4001 shadow"
            + " key, 0x8000 masked, 0x4000 alone none", false);
    }

    private boolean isEntrySearch(Function f) {
        if (f.getBody().getNumAddresses() >= SMALL_FUNCTION_BYTES || !calls(f).isEmpty()) {
            return false;
        }
        boolean count = false;
        boolean offset = false;
        boolean halve = false;
        for (Instruction i : listing.getInstructions(f.getBody(), true)) {
            String text = i.toString();
            count |= text.contains(FRAMES_COUNT_OPERAND);
            offset |= text.contains(FRAMES_OFFSET_OPERAND);
            if (i.getMnemonicString().equals(HALVING_MNEMONIC) && i.getNumOperands() == 2) {
                for (Object o : i.getOpObjects(1)) {
                    halve |= o instanceof Scalar s && s.getUnsignedValue() == HALVING_SHIFT;
                }
            }
        }
        return count && offset && halve;
    }

    private static void addOnce(List<Function> list, Function f) {
        if (!list.contains(f)) {
            list.add(f);
        }
    }

    // ---- proposals ----

    private void propose(Function f, String path, String evidence, boolean probable) {
        if (f == null) {
            anomalies.add("not found: `" + path + "`");
            return;
        }
        long address = f.getEntryPoint().getOffset();
        Proposal existing = proposals.get(address);
        if (existing != null) {
            if (!existing.path.equals(path)) {
                anomalies.add(hex(address) + " proposed as `" + existing.path + "` and `" + path + "`; kept the first");
            }
            return;
        }
        Proposal proposal = new Proposal();
        proposal.address = address;
        proposal.path = path;
        proposal.evidence = evidence;
        proposal.probable = probable;
        proposals.put(address, proposal);
    }

    private void applyName(Proposal proposal) throws Exception {
        Function function = functions.getFunctionAt(toAddr(proposal.address));
        int split = proposal.path.lastIndexOf(SCOPE);
        String name = split < 0 ? proposal.path : proposal.path.substring(split + SCOPE.length());
        Namespace namespace = split < 0 ? currentProgram.getGlobalNamespace()
            : namespace(proposal.path.substring(0, split));
        Symbol symbol = function.getSymbol();
        if (symbol.getName().equals(name) && symbol.getParentNamespace().equals(namespace)) {
            proposal.status = "already named";
        } else if (symbol.getSource() == SourceType.DEFAULT) {
            try {
                symbol.setNameAndNamespace(name, namespace, SourceType.ANALYSIS);
            } catch (DuplicateNameException duplicate) {
                String unique = name + "_" + Long.toHexString(proposal.address);
                symbol.setNameAndNamespace(unique, namespace, SourceType.ANALYSIS);
                anomalies.add("`" + proposal.path + "` exists; " + hex(proposal.address) + " named `" + unique + "`");
            }
            proposal.status = "renamed";
            count("functions renamed");
        } else {
            proposal.status = "kept " + symbol.getName(true) + " (" + symbol.getSource() + ")";
            kept.add(hex(proposal.address) + " `" + symbol.getName(true) + "` (" + symbol.getSource()
                + "), proposed `" + proposal.path + "`");
        }
        boolean tagged = false;
        for (FunctionTag tag : function.getTags()) {
            tagged |= tag.getName().equals(TAG);
        }
        if (!tagged) {
            function.addTag(TAG);
            count("tags added");
        }
        String comment = function.getComment();
        if (comment == null || comment.isBlank()) {
            function.setComment(COMMENT_PREFIX + TAG + " `" + proposal.path + "`"
                + (proposal.probable ? PROBABLE : "") + ": " + proposal.evidence);
            count("plate comments set");
        }
    }

    private void ensureTag() {
        if (functions.getFunctionTagManager().getFunctionTag(TAG) == null) {
            functions.getFunctionTagManager().createFunctionTag(TAG, TAG_COMMENT);
            count("tag created");
        }
    }

    private Namespace namespace(String path) throws Exception {
        Namespace namespace = currentProgram.getGlobalNamespace();
        for (String part : path.split(SCOPE)) {
            Namespace child = symbols.getNamespace(part, namespace);
            if (child == null) {
                child = symbols.createNameSpace(namespace, part, SourceType.ANALYSIS);
                count("namespaces created");
            }
            namespace = child;
        }
        return namespace;
    }

    private void count(String change) {
        changes.merge(change, 1, Integer::sum);
    }

    // ---- memory and RTTI ----

    private void loadBlocks() throws Exception {
        for (MemoryBlock b : memory.getBlocks()) {
            if (!b.isInitialized() || b.getSize() > Integer.MAX_VALUE) {
                continue;
            }
            Block block = new Block();
            block.start = b.getStart().getOffset();
            block.bytes = new byte[(int) b.getSize()];
            b.getBytes(b.getStart(), block.bytes);
            block.exec = b.isExecute();
            blocks.add(block);
        }
    }

    private static long dword(byte[] bytes, int i) {
        long value = 0;
        for (int k = POINTER_SIZE - 1; k >= 0; k--) {
            value = (value << BITS_PER_BYTE) | (bytes[i + k] & BYTE_MASK);
        }
        return value;
    }

    private long dwordAt(long address) {
        for (Block block : blocks) {
            long index = address - block.start;
            if (index >= 0 && index + POINTER_SIZE <= block.bytes.length) {
                return dword(block.bytes, (int) index);
            }
        }
        return -1;
    }

    private List<Long> findBytes(byte[] pattern) {
        List<Long> found = new ArrayList<>();
        for (Block block : blocks) {
            if (block.exec) {
                continue;
            }
            byte[] b = block.bytes;
            outer:
            for (int i = 0; i + pattern.length <= b.length; i++) {
                for (int k = 0; k < pattern.length; k++) {
                    if (b[i + k] != pattern[k]) {
                        continue outer;
                    }
                }
                found.add(block.start + i);
            }
        }
        return found;
    }

    private List<Long> findDwords(long value) {
        List<Long> found = new ArrayList<>();
        for (Block block : blocks) {
            if (block.exec) {
                continue;
            }
            for (int i = 0; i + POINTER_SIZE <= block.bytes.length; i += POINTER_SIZE) {
                if (dword(block.bytes, i) == value) {
                    found.add(block.start + i);
                }
            }
        }
        return found;
    }

    private long rttiVtable(String cls) {
        byte[] name = (RTTI_PREFIX + cls + RTTI_SUFFIX + "\0").getBytes(StandardCharsets.US_ASCII);
        for (long nameAt : findBytes(name)) {
            long descriptor = nameAt - TYPE_DESCRIPTOR_NAME_OFFSET;
            for (long ref : findDwords(descriptor)) {
                long col = ref - COL_TYPE_DESCRIPTOR_OFFSET;
                if (dwordAt(col) != 0 || dwordAt(col + COL_OFFSET_FIELD) != 0) {
                    continue;
                }
                List<Long> uses = findDwords(col);
                if (uses.size() == 1) {
                    return uses.get(0) + POINTER_SIZE;
                }
            }
        }
        return -1;
    }

    private Function slotFunction(long vt, int offset) {
        long target = dwordAt(vt + offset);
        return target < 0 ? null : functions.getFunctionAt(toAddr(target));
    }

    // Functions referencing the vftable (constructors, destructors).
    private List<Function> vtableWriters(String cls) {
        long vt = vtables.getOrDefault(cls, -1L);
        List<Function> writers = new ArrayList<>();
        if (vt < 0) {
            return writers;
        }
        for (Reference r : getReferencesTo(toAddr(vt))) {
            Function f = getFunctionContaining(r.getFromAddress());
            if (f != null && !writers.contains(f)) {
                writers.add(f);
            }
        }
        return writers;
    }

    // ---- call graph ----

    private List<Function> calls(Function f) {
        List<Function> out = new ArrayList<>();
        for (Instruction i : listing.getInstructions(f.getBody(), true)) {
            if (!i.getFlowType().isCall()) {
                continue;
            }
            Address[] flows = i.getFlows();
            if (flows.length == 0) {
                continue;
            }
            Function callee = functions.getFunctionAt(flows[0]);
            if (callee != null) {
                out.add(callee.isThunk() ? callee.getThunkedFunction(true) : callee);
            }
        }
        return out;
    }

    private List<Function> realCalls(Function f) {
        return new ArrayList<>(new LinkedHashSet<>(calls(f)));
    }

    private List<Function> callers(Function f) {
        List<Function> out = new ArrayList<>(f.getCallingFunctions(monitor));
        out.sort((a, b) -> a.getEntryPoint().compareTo(b.getEntryPoint()));
        return out;
    }

    private Function single(List<Function> list, String what) {
        if (list.size() == 1) {
            return list.get(0);
        }
        anomalies.add(what + ": " + list.size() + " candidates " + names(list));
        return null;
    }

    // ---- instruction patterns ----

    private boolean usesScalar(Function f, long value) {
        for (Instruction i : listing.getInstructions(f.getBody(), true)) {
            for (int op = 0; op < i.getNumOperands(); op++) {
                for (Object o : i.getOpObjects(op)) {
                    if (o instanceof Scalar s && (s.getUnsignedValue() & DWORD_MASK) == value) {
                        return true;
                    }
                }
            }
        }
        return false;
    }

    private List<Function> scalarUsersInProgram(long value) {
        List<Function> out = new ArrayList<>();
        for (Instruction i : listing.getInstructions(true)) {
            for (int op = 0; op < i.getNumOperands(); op++) {
                for (Object o : i.getOpObjects(op)) {
                    if (o instanceof Scalar s && (s.getUnsignedValue() & DWORD_MASK) == value) {
                        Function f = getFunctionContaining(i.getAddress());
                        if (f != null && !out.contains(f)) {
                            out.add(f);
                        }
                    }
                }
            }
        }
        return out;
    }

    // ---- report ----

    private void report(PrintWriter out, boolean apply) {
        out.println("# AnimationFrame code names");
        out.println();
        out.println("Program: " + currentProgram.getName());
        out.println("SHA-256: " + currentProgram.getExecutableSHA256());
        out.println("Mode: " + (apply ? "analysis and " + APPLY_FLAG : "analysis only"));
        out.println();
        out.println("| Item | Count |");
        out.println("|---|---:|");
        out.println("| Function names proposed | " + proposals.size() + " |");
        out.println("| Kept non-default names | " + kept.size() + " |");
        out.println("| Anomalies | " + anomalies.size() + " |");
        out.println();
        out.println("Changes: " + (changes.isEmpty() ? "none" : changes.toString()));
        out.println();
        out.println("## Anchors");
        out.println();
        for (String a : anchors) {
            out.println("- " + a);
        }
        out.println();
        out.println("## Functions");
        out.println();
        out.println("| Address | Name | Status | Evidence |");
        out.println("|---|---|---|---|");
        for (Proposal p : proposals.values()) {
            out.println("| " + hex(p.address) + " | `" + p.path + "`" + (p.probable ? PROBABLE : "") + " | "
                + p.status + " | " + p.evidence.replace("|", "\\|") + " |");
        }
        section(out, "Kept names (not renamed)", kept);
        section(out, "Anomalies", anomalies);
    }

    private static void section(PrintWriter out, String title, List<String> lines) {
        out.println();
        out.println("## " + title);
        out.println();
        if (lines.isEmpty()) {
            out.println("None.");
        }
        for (String line : lines) {
            out.println("- " + line);
        }
    }

    private static String names(List<Function> list) {
        List<String> out = new ArrayList<>();
        for (Function f : list) {
            out.add(f.getName(true) + "@" + f.getEntryPoint());
        }
        return out.toString();
    }

    private static String hex(long value) {
        return String.format("0x%08x", value);
    }

    private static String hexOrNone(long value) {
        return value < 0 ? "not found" : hex(value);
    }
}
