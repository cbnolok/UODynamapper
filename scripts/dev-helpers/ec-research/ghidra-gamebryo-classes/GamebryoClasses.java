// Recover Gamebryo classes from NiRTTI records, GetRTTI vtable slots and NiStream loader
// registrations; with --apply, add namespaces, labels, function names and /Gamebryo structs.
// @category ECResearch
import ghidra.app.script.GhidraScript;
import ghidra.app.util.PseudoDisassembler;
import ghidra.program.model.address.Address;
import ghidra.program.model.address.AddressSet;
import ghidra.program.model.data.CategoryPath;
import ghidra.program.model.data.CharDataType;
import ghidra.program.model.data.DataType;
import ghidra.program.model.data.DataTypeConflictHandler;
import ghidra.program.model.data.DataTypeManager;
import ghidra.program.model.data.PointerDataType;
import ghidra.program.model.data.Structure;
import ghidra.program.model.data.StructureDataType;
import ghidra.program.model.data.Undefined;
import ghidra.program.model.data.VoidDataType;
import ghidra.program.model.listing.CodeUnit;
import ghidra.program.model.listing.Data;
import ghidra.program.model.listing.Function;
import ghidra.program.model.lang.Register;
import ghidra.program.model.listing.Instruction;
import ghidra.program.model.mem.MemoryBlock;
import ghidra.program.model.scalar.Scalar;
import ghidra.program.model.symbol.Namespace;
import ghidra.program.model.symbol.RefType;
import ghidra.program.model.symbol.Reference;
import ghidra.program.model.symbol.SourceType;
import ghidra.program.model.symbol.Symbol;
import ghidra.program.model.symbol.SymbolTable;
import ghidra.program.model.symbol.SymbolType;
import java.io.PrintWriter;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardOpenOption;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.HashMap;
import java.util.HashSet;
import java.util.LinkedHashMap;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.TreeMap;
import java.util.TreeSet;

public class GamebryoClasses extends GhidraScript {
    private static final String APPLY_FLAG = "--apply";
    private static final int REPORT_ARGS = 1;
    private static final int APPLY_ARGS = REPORT_ARGS + 1;
    private static final int POINTER_SIZE = 4;
    private static final long DWORD_MASK = 0xffffffffL;
    private static final int BYTE_MASK = 0xff;
    private static final int BITS_PER_BYTE = 8;
    // NiRTTI { const char *name; const NiRTTI *base; }
    private static final int RTTI_RECORD_SIZE = 2 * POINTER_SIZE;
    private static final int MIN_NAME_LENGTH = 2;
    private static final int MAX_NAME_LENGTH = 128;
    // Slots 0/1 are the NiRefObject destructor and DeleteThis virtuals.
    private static final int GETRTTI_SLOT = 2;
    private static final int OPCODE_MOV_EAX_IMM = 0xb8;
    private static final int OPCODE_RET = 0xc3;
    private static final int OPCODE_PUSH_IMM = 0x68;
    private static final int OPCODE_CALL_REL = 0xe8;
    private static final int OPCODE_LENGTH = 1;
    // Opcode byte followed by an imm32 or rel32.
    private static final int IMM_INSTRUCTION_LENGTH = OPCODE_LENGTH + POINTER_SIZE;
    private static final int GETRTTI_LENGTH = IMM_INSTRUCTION_LENGTH + OPCODE_LENGTH;
    private static final int MAX_VTABLE_SLOTS = 512;
    private static final int MIN_REGISTRATIONS = 8;
    private static final int MIN_UNREGISTRATIONS = 8;
    private static final int MAX_WALK_INSTRUCTIONS = 64;
    // CreateObject: MOV EAX, <size>; CALL <allocator> after at most a few prologue instructions.
    private static final int MAX_PROLOGUE_INSTRUCTIONS = 8;
    private static final String SIZE_REGISTER = "EAX";
    private static final String MOV = "MOV";
    private static final String SCOPE = "::";
    private static final String CATEGORY = "/Gamebryo";
    private static final String RTTI_STRUCT = "NiRTTI";
    private static final String VFTABLE = "vftable";
    private static final String GETRTTI = "GetRTTI";
    private static final String STREAM_CLASS = "NiStream";
    private static final String MEMORY_CLASS = "NiMemObject";
    private static final String DESCRIPTION_PREFIX = "Gamebryo class ";

    private static final class Block {
        long start;
        byte[] bytes;
        boolean exec;
    }

    private static final class ClassInfo {
        String name;
        long record;
        long baseRecord;
        ClassInfo parent;
        final List<ClassInfo> children = new ArrayList<>();
        final List<Long> getRtti = new ArrayList<>();
        final List<Long> vtables = new ArrayList<>();
        long vftable = -1;
        String vftableSource = "";
        Registration registration;
        DataType struct;
    }

    private static final class Registration {
        long site;
        String name;
        long createObject;
        long size = -1;
        long firstCall = -1;
        Set<Long> storedVtables = new LinkedHashSet<>();
    }

    // A GetRTTI-shaped function's pointer that is a later slot of another class's vtable.
    private static final class SlotUse {
        long function;
        ClassInfo returned;
        long pointer;
        long vtable;
        int slot;
        ClassInfo vtableClass;
        boolean dropped;
    }

    private final List<Block> blocks = new ArrayList<>();
    private Block lastBlock;
    private final List<String> anomalies = new ArrayList<>();
    private final Map<String, Integer> changes = new LinkedHashMap<>();
    private final List<String> kept = new ArrayList<>();
    private final List<String> removed = new ArrayList<>();

    public void run() throws Exception {
        String[] args = getScriptArgs();
        boolean apply = args.length == APPLY_ARGS && args[REPORT_ARGS].equals(APPLY_FLAG);
        if (args.length != REPORT_ARGS && !apply) {
            throw new IllegalArgumentException("Expected OUTPUT_FILE [" + APPLY_FLAG + "]");
        }
        for (MemoryBlock block : currentProgram.getMemory().getBlocks()) {
            if (!block.isInitialized() || block.getSize() > Integer.MAX_VALUE) {
                continue;
            }
            Block copy = new Block();
            copy.start = block.getStart().getOffset();
            copy.bytes = new byte[(int) block.getSize()];
            copy.exec = block.isExecute();
            block.getBytes(block.getStart(), copy.bytes);
            blocks.add(copy);
        }

        // 1. RTTI record candidates: { identifier string, 0 or aligned data pointer }.
        Map<Long, Long> candidates = new HashMap<>();
        for (Block block : blocks) {
            if (block.exec) {
                continue;
            }
            for (long address = alignUp(block.start); address + RTTI_RECORD_SIZE <= block.start + block.bytes.length;
                    address += POINTER_SIZE) {
                long base = u32(address + POINTER_SIZE);
                if (identifierAt(u32(address)) != null
                        && (base == 0 || (isData(base) && base % POINTER_SIZE == 0))) {
                    candidates.put(address, base);
                }
            }
        }
        // GetRTTI: MOV EAX, <record>; RET, pointed to from data (it is virtual).
        Map<Long, Long> shaped = new TreeMap<>();
        for (Block block : blocks) {
            if (!block.exec) {
                continue;
            }
            byte[] b = block.bytes;
            for (int i = 0; i + GETRTTI_LENGTH <= b.length; i++) {
                if ((b[i] & BYTE_MASK) == OPCODE_MOV_EAX_IMM && (b[i + IMM_INSTRUCTION_LENGTH] & BYTE_MASK) == OPCODE_RET) {
                    long record = le32(b, i + OPCODE_LENGTH);
                    if (candidates.containsKey(record)) {
                        shaped.put(block.start + i, record);
                    }
                }
            }
        }
        Map<Long, List<Long>> pointerSources = new HashMap<>();
        for (Block block : blocks) {
            if (block.exec) {
                continue;
            }
            for (long address = alignUp(block.start); address + POINTER_SIZE <= block.start + block.bytes.length;
                    address += POINTER_SIZE) {
                long value = u32(address);
                if (shaped.containsKey(value)) {
                    pointerSources.computeIfAbsent(value, key -> new ArrayList<>()).add(address);
                }
            }
        }
        Map<Long, ClassInfo> classes = new TreeMap<>();
        Map<Long, ClassInfo> getRttiOwner = new TreeMap<>();
        List<String> rejectedGetRtti = new ArrayList<>();
        for (var entry : shaped.entrySet()) {
            long function = entry.getKey();
            if (!pointerSources.containsKey(function)) {
                rejectedGetRtti.add(hex(function) + " returns " + hex(entry.getValue()) + " (`"
                    + identifierAt(u32(entry.getValue())) + "`)"
                    + (getFunctionAt(toAddr(function)) != null ? ", a function with no data pointer to it" : ""));
                continue;
            }
            ClassInfo info = classFor(classes, candidates, entry.getValue());
            info.getRtti.add(function);
            getRttiOwner.put(function, info);
        }
        SymbolTable symbols = currentProgram.getSymbolTable();
        List<SlotUse> reclassified = reclassifySlots(symbols, getRttiOwner, pointerSources);
        Set<Long> slotPointers = new HashSet<>();
        for (SlotUse use : reclassified) {
            slotPointers.add(use.pointer);
        }
        List<ClassInfo> work = new ArrayList<>(classes.values());
        while (!work.isEmpty()) {
            ClassInfo info = work.remove(work.size() - 1);
            if (info.baseRecord == 0 || info.parent != null) {
                continue;
            }
            if (!candidates.containsKey(info.baseRecord)) {
                anomalies.add("`" + info.name + "` (" + hex(info.record) + ") has base " + hex(info.baseRecord)
                    + ", which is not an RTTI record; treated as a root");
                continue;
            }
            boolean known = classes.containsKey(info.baseRecord);
            info.parent = classFor(classes, candidates, info.baseRecord);
            info.parent.children.add(info);
            if (!known) {
                work.add(info.parent);
            }
        }
        Map<String, ClassInfo> byName = new TreeMap<>();
        for (ClassInfo info : classes.values()) {
            if (info.getRtti.size() > 1) {
                anomalies.add("`" + info.name + "` has " + info.getRtti.size() + " GetRTTI functions: " + hexList(info.getRtti));
            }
            ClassInfo other = byName.putIfAbsent(info.name, info);
            if (other != null) {
                anomalies.add("Duplicate class name `" + info.name + "`: records " + hex(other.record) + " and "
                    + hex(info.record) + "; only the first is annotated");
            }
        }

        // 2. Vtables: a data pointer to GetRTTI at slot 2, with code pointers in slots 0 and 1. A slot
        // that is itself labeled or stored by code starts a vtable with GetRTTI in slot 0.
        Map<Long, ClassInfo> vtableRtti = new TreeMap<>();
        for (var entry : getRttiOwner.entrySet()) {
            for (long source : pointerSources.getOrDefault(entry.getKey(), List.of())) {
                if (slotPointers.contains(source)) {
                    continue;
                }
                long start = source - (long) GETRTTI_SLOT * POINTER_SIZE;
                if (startsLabeledItem(symbols, toAddr(source))) {
                    start = source;
                    anomalies.add("`" + entry.getValue().name + "::GetRTTI` is slot 0 of vtable " + hex(start)
                        + " (labeled or stored by code)");
                } else if (!isExec(u32(start)) || !isExec(u32(start + POINTER_SIZE))
                        || startsLabeledItem(symbols, toAddr(start + POINTER_SIZE))) {
                    anomalies.add("Pointer to `" + entry.getValue().name + "::GetRTTI` at " + hex(source)
                        + " is not in a vtable");
                    continue;
                }
                vtableRtti.put(start, entry.getValue());
                entry.getValue().vtables.add(start);
            }
        }
        Map<Long, Integer> slotCounts = new TreeMap<>();
        for (long start : vtableRtti.keySet()) {
            slotCounts.put(start, slotCount(start, vtableRtti.keySet()));
        }
        // Existing vftable labels from MSVC RTTI analysis or the user, by address.
        Map<Long, String> existingVftables = new TreeMap<>();
        for (Symbol symbol : symbols.getSymbols(VFTABLE)) {
            if (symbol.getSource() == SourceType.IMPORTED || symbol.getSource() == SourceType.USER_DEFINED) {
                existingVftables.put(symbol.getAddress().getOffset(), symbol.getParentNamespace().getName(true));
            }
        }

        // 3. Loader registrations: PUSH <code>; PUSH <class name>; CALL register.
        List<long[]> pairHits = new ArrayList<>();
        List<long[]> singleHits = new ArrayList<>();
        for (Block block : blocks) {
            if (!block.exec) {
                continue;
            }
            byte[] b = block.bytes;
            for (int i = 0; i + 2 * IMM_INSTRUCTION_LENGTH <= b.length; i++) {
                if ((b[i] & BYTE_MASK) != OPCODE_PUSH_IMM
                        || (b[i + IMM_INSTRUCTION_LENGTH] & BYTE_MASK) != OPCODE_CALL_REL) {
                    continue;
                }
                long string = le32(b, i + OPCODE_LENGTH);
                if (identifierAt(string) == null) {
                    continue;
                }
                long call = block.start + i + IMM_INSTRUCTION_LENGTH;
                long callee = (call + IMM_INSTRUCTION_LENGTH + (int) le32(b, i + IMM_INSTRUCTION_LENGTH + OPCODE_LENGTH)) & DWORD_MASK;
                singleHits.add(new long[] {call, string, callee});
                if (i >= IMM_INSTRUCTION_LENGTH && (b[i - IMM_INSTRUCTION_LENGTH] & BYTE_MASK) == OPCODE_PUSH_IMM) {
                    long code = le32(b, i - IMM_INSTRUCTION_LENGTH + OPCODE_LENGTH);
                    if (isExec(code)) {
                        pairHits.add(new long[] {call, code, string, callee});
                    }
                }
            }
        }
        Map<Long, Integer> pairCounts = new HashMap<>();
        for (long[] hit : pairHits) {
            if (byName.containsKey(identifierAt(hit[2]))) {
                pairCounts.merge(hit[3], 1, Integer::sum);
            }
        }
        long register = mostCommon(pairCounts, MIN_REGISTRATIONS);
        Map<Long, Integer> singleCounts = new HashMap<>();
        for (long[] hit : singleHits) {
            if (hit[2] != register && byName.containsKey(identifierAt(hit[1]))) {
                singleCounts.merge(hit[2], 1, Integer::sum);
            }
        }
        long unregister = mostCommon(singleCounts, MIN_UNREGISTRATIONS);

        List<Registration> registrations = new ArrayList<>();
        Set<Long> matchedSites = new HashSet<>();
        for (long[] hit : pairHits) {
            if (hit[3] != register) {
                continue;
            }
            Registration registration = new Registration();
            registration.site = hit[0];
            registration.createObject = hit[1];
            registration.name = identifierAt(hit[2]);
            registrations.add(registration);
            matchedSites.add(hit[0]);
        }
        List<String> unmatchedSites = new ArrayList<>();
        Set<Long> referencedSites = new HashSet<>();
        if (register >= 0) {
            for (Reference reference : getReferencesTo(toAddr(register))) {
                long from = reference.getFromAddress().getOffset();
                referencedSites.add(from);
                if (!matchedSites.contains(from)) {
                    Instruction previous = getInstructionBefore(reference.getFromAddress());
                    unmatchedSites.add(hex(from) + " (" + reference.getReferenceType()
                        + (previous != null ? ", previous instruction `" + previous + "`" : "") + ")");
                }
            }
        }
        Set<String> unregistered = new TreeSet<>();
        int unregisterSites = 0;
        for (long[] hit : singleHits) {
            if (unregister >= 0 && hit[2] == unregister) {
                unregistered.add(identifierAt(hit[1]));
                unregisterSites++;
            }
        }
        // CreateObject: MOV EAX, <size>; CALL <allocator>, decoded without changing the listing.
        PseudoDisassembler disassembler = new PseudoDisassembler(currentProgram);
        Map<Long, Integer> firstCalls = new HashMap<>();
        for (Registration registration : registrations) {
            decodeCreateObject(disassembler, registration);
            if (registration.firstCall >= 0) {
                firstCalls.merge(registration.firstCall, 1, Integer::sum);
            } else {
                anomalies.add("CreateObject " + hex(registration.createObject) + " for `" + registration.name
                    + "` has no MOV EAX, imm32; CALL in its first " + MAX_PROLOGUE_INSTRUCTIONS + " instructions");
            }
        }
        long allocator = mostCommon(firstCalls, 1);
        Map<String, Registration> registrationByName = new TreeMap<>();
        Map<String, List<Registration>> repeated = new TreeMap<>();
        for (Registration registration : registrations) {
            if (registration.firstCall >= 0 && registration.firstCall != allocator) {
                anomalies.add("CreateObject " + hex(registration.createObject) + " for `" + registration.name
                    + "` first calls " + hex(registration.firstCall) + ", not the allocator; size ignored");
                registration.size = -1;
            }
            collectVtables(disassembler, registration.createObject, vtableRtti.keySet(), registration.storedVtables,
                allocator, true);
            ClassInfo info = byName.get(registration.name);
            if (info == null) {
                List<String> sharing = new ArrayList<>();
                for (Registration other : registrations) {
                    if (other != registration && other.createObject == registration.createObject) {
                        sharing.add("`" + other.name + "`");
                    }
                }
                anomalies.add("Loader `" + registration.name + "` registered at " + hex(registration.site)
                    + " has no NiRTTI record of that name"
                    + (sharing.isEmpty() ? "" : "; its CreateObject is shared with " + String.join(", ", sharing)));
            } else if (info.registration != null) {
                repeated.computeIfAbsent(registration.name, key -> new ArrayList<>(List.of(info.registration)))
                    .add(registration);
            } else {
                info.registration = registration;
            }
            registrationByName.putIfAbsent(registration.name, registration);
        }
        for (var entry : repeated.entrySet()) {
            List<String> sites = new ArrayList<>();
            for (Registration registration : entry.getValue()) {
                sites.add(hex(registration.site) + " -> " + hex(registration.createObject));
            }
            anomalies.add("`" + entry.getKey() + "` is registered " + entry.getValue().size() + " times (site -> CreateObject): "
                + String.join(", ", sites) + "; the first is used");
        }
        for (String name : unregistered) {
            if (!registrationByName.containsKey(name)) {
                anomalies.add("`" + name + "` is unregistered but never registered");
            }
        }
        for (String name : registrationByName.keySet()) {
            if (unregister >= 0 && !unregistered.contains(name)) {
                anomalies.add("`" + name + "` is registered but never unregistered");
            }
        }

        // Vtable ownership.
        List<String> ambiguous = new ArrayList<>();
        for (ClassInfo info : classes.values()) {
            if (info.vtables.size() == 1) {
                info.vftable = info.vtables.get(0);
                info.vftableSource = "single vtable";
                continue;
            }
            if (info.vtables.isEmpty()) {
                continue;
            }
            List<Long> labeled = new ArrayList<>();
            List<Long> stored = new ArrayList<>();
            for (long vtable : info.vtables) {
                if (info.name.equals(existingVftables.get(vtable))) {
                    labeled.add(vtable);
                }
                if (info.registration != null && info.registration.storedVtables.contains(vtable)) {
                    stored.add(vtable);
                }
            }
            if (labeled.size() == 1) {
                info.vftable = labeled.get(0);
                info.vftableSource = "existing label";
            } else if (stored.size() == 1) {
                info.vftable = stored.get(0);
                info.vftableSource = "CreateObject";
            } else {
                StringBuilder line = new StringBuilder("`" + info.name + "::GetRTTI` is in " + info.vtables.size()
                    + " vtables:");
                for (long vtable : info.vtables) {
                    line.append(" ").append(hex(vtable)).append(" (").append(slotCounts.get(vtable)).append(" slots");
                    if (existingVftables.containsKey(vtable)) {
                        line.append(", `").append(existingVftables.get(vtable)).append("::vftable`");
                    }
                    line.append(")");
                }
                ambiguous.add(line.toString());
            }
        }
        for (ClassInfo info : classes.values()) {
            if (info.registration != null && !info.registration.storedVtables.isEmpty() && info.vftable >= 0
                    && !info.registration.storedVtables.contains(info.vftable)) {
                anomalies.add("`" + info.name + "::CreateObject` stores vtables "
                    + hexList(info.registration.storedVtables) + " but not the recovered vftable " + hex(info.vftable));
            }
        }

        // MSVC RTTI cross-check.
        List<String> mismatches = new ArrayList<>();
        List<String> inherited = new ArrayList<>();
        for (var entry : existingVftables.entrySet()) {
            long vtable = entry.getKey();
            String namespace = entry.getValue();
            ClassInfo owner = vtableRtti.get(vtable);
            ClassInfo named = byName.get(namespace);
            if (owner == null) {
                if (named != null) {
                    mismatches.add("`" + namespace + "::vftable` " + hex(vtable) + " has no NiRTTI GetRTTI at slot 2");
                }
            } else if (named == null) {
                inherited.add("`" + namespace + "::vftable` " + hex(vtable) + " uses `" + owner.name
                    + "::GetRTTI` (no NiRTTI record of its own)");
            } else if (named != owner) {
                mismatches.add("`" + namespace + "::vftable` " + hex(vtable) + " uses `" + owner.name
                    + "::GetRTTI`, but `" + namespace + "` has its own NiRTTI record");
            } else if (owner.vftable != vtable) {
                mismatches.add("`" + namespace + "::vftable` " + hex(vtable) + " differs from the recovered vftable "
                    + (owner.vftable >= 0 ? hex(owner.vftable) : "(none)"));
            }
        }

        List<ClassInfo> roots = new ArrayList<>();
        for (ClassInfo info : classes.values()) {
            info.children.sort(Comparator.comparing(child -> child.name));
            if (info.parent == null) {
                roots.add(info);
            }
        }
        roots.sort(Comparator.comparing(root -> root.name));
        List<ClassInfo> ordered = new ArrayList<>();
        Set<ClassInfo> visited = new HashSet<>();
        for (ClassInfo root : roots) {
            preorder(root, ordered, visited);
        }
        for (ClassInfo info : classes.values()) {
            if (!visited.contains(info)) {
                anomalies.add("`" + info.name + "` (" + hex(info.record) + ") is in a base cycle");
            }
        }

        if (apply) {
            annotate(ordered, byName, register, unregister, allocator);
            repair(classes, shaped, getRttiOwner, reclassified);
        }
        List<String> reclassifiedLines = new ArrayList<>();
        for (SlotUse use : reclassified) {
            reclassifiedLines.add(hex(use.function) + " returns `" + use.returned.name + "` " + hex(use.returned.record)
                + "; its pointer " + hex(use.pointer) + " is slot " + use.slot + " of `" + use.vtableClass.name
                + "` vtable " + hex(use.vtable) + " (not a vtable start at "
                + hex(use.pointer - (long) GETRTTI_SLOT * POINTER_SIZE) + ")"
                + (use.dropped ? "; no other pointer, so not a GetRTTI" : "; other pointers kept"));
        }

        try (PrintWriter output = new PrintWriter(Files.newBufferedWriter(Path.of(args[0]),
                StandardOpenOption.CREATE_NEW))) {
            int owned = 0;
            int sized = 0;
            for (ClassInfo info : classes.values()) {
                owned += info.vftable >= 0 ? 1 : 0;
                sized += size(info) >= 0 ? 1 : 0;
            }
            output.println("# Gamebryo classes");
            output.println();
            output.println("Program: " + currentProgram.getName());
            output.println("SHA-256: " + currentProgram.getExecutableSHA256());
            output.println("Mode: " + (apply ? "analysis and " + APPLY_FLAG : "analysis only"));
            output.println();
            output.println("| Item | Count |");
            output.println("|---|---:|");
            output.println("| RTTI record candidates (name pointer, base) | " + candidates.size() + " |");
            output.println("| GetRTTI-shaped functions | " + getRttiOwner.size() + " |");
            output.println("| Rejected `MOV EAX, record; RET` byte matches | " + rejectedGetRtti.size() + " |");
            output.println("| GetRTTI-shaped pointers reclassified as vtable slots | " + reclassified.size() + " |");
            output.println("| Validated classes | " + classes.size() + " |");
            output.println("| Roots | " + roots.size() + " |");
            output.println("| Vtables with a GetRTTI slot | " + vtableRtti.size() + " |");
            output.println("| Classes with a recovered vftable | " + owned + " |");
            output.println("| Classes with ambiguous vtables | " + ambiguous.size() + " |");
            output.println("| Existing vftable labels in the program (MSVC RTTI / user) | " + existingVftables.size() + " |");
            int undisassembled = 0;
            for (Registration registration : registrations) {
                undisassembled += referencedSites.contains(registration.site) ? 0 : 1;
            }
            output.println("| Loader registrations | " + registrations.size() + " |");
            output.println("| Registrations in code with no call reference (not disassembled) | " + undisassembled + " |");
            output.println("| Loader unregistrations | " + unregisterSites + " |");
            output.println("| Classes with object size | " + sized + " |");
            output.println();
            output.println("Detected functions:");
            output.println();
            output.println("- `" + STREAM_CLASS + "::RegisterLoader`: " + hexOrNone(register)
                + " (" + pairCounts.getOrDefault(register, 0) + " calls with a class name)");
            output.println("- `" + STREAM_CLASS + "::UnregisterLoader`: " + hexOrNone(unregister)
                + " (" + singleCounts.getOrDefault(unregister, 0) + " calls with a class name)");
            output.println("- `" + MEMORY_CLASS + "::operator_new` (inferred): " + hexOrNone(allocator)
                + " (first call of " + firstCalls.getOrDefault(allocator, 0) + " CreateObject functions)");
            output.println();
            output.println("Roots: " + String.join(", ", roots.stream().map(root -> "`" + root.name + "`").toList()));
            output.println();
            output.println("## Class tree");
            output.println();
            output.println("RTTI = NiRTTI record, vftable source in parentheses, size from CreateObject; unknown sizes"
                + " show the bounds from the nearest sized ancestor and the smallest sized descendant.");
            output.println();
            for (ClassInfo info : ordered) {
                output.println("  ".repeat(depth(info)) + "- " + describe(info, slotCounts, existingVftables));
            }
            section(output, "Ambiguous vtables", ambiguous);
            section(output, "MSVC RTTI mismatches", mismatches);
            section(output, "MSVC classes without their own NiRTTI", inherited);
            section(output, "Unmatched registration call sites", unmatchedSites);
            section(output, "Rejected GetRTTI byte matches", rejectedGetRtti);
            section(output, "GetRTTI-shaped vtable slots", reclassifiedLines);
            section(output, "Other anomalies", anomalies);
            if (apply) {
                output.println();
                output.println("## Applied changes");
                output.println();
                int total = 0;
                for (int count : changes.values()) {
                    total += count;
                }
                output.println("Total: " + total);
                output.println();
                for (var entry : changes.entrySet()) {
                    output.println("- " + entry.getKey() + ": " + entry.getValue());
                }
                section(output, "Removed earlier annotations", removed);
                section(output, "Kept existing", kept);
            }
        }
    }

    private void decodeCreateObject(PseudoDisassembler disassembler, Registration registration) {
        Address address = toAddr(registration.createObject);
        Instruction previous = null;
        try {
            for (int count = 0; count < MAX_PROLOGUE_INSTRUCTIONS; count++) {
                Instruction instruction = disassembler.disassemble(address);
                if (instruction.getFlowType().isCall()) {
                    Address[] flows = instruction.getFlows();
                    Register register = previous != null ? previous.getRegister(0) : null;
                    Scalar size = previous != null ? previous.getScalar(1) : null;
                    if (flows.length == 1 && register != null && previous.getMnemonicString().equals(MOV)
                            && register.getName().equals(SIZE_REGISTER) && size != null) {
                        registration.firstCall = flows[0].getOffset();
                        registration.size = size.getUnsignedValue();
                    }
                    return;
                }
                if (!instruction.getFlowType().isFallthrough()) {
                    return;
                }
                previous = instruction;
                address = instruction.getMaxAddress().next();
            }
        } catch (Exception undecodable) {
            // Left without a size; reported by the caller.
        }
    }

    private ClassInfo classFor(Map<Long, ClassInfo> classes, Map<Long, Long> candidates, long record) {
        return classes.computeIfAbsent(record, key -> {
            ClassInfo info = new ClassInfo();
            info.record = record;
            info.name = identifierAt(u32(record));
            info.baseRecord = candidates.get(record);
            return info;
        });
    }

    private void preorder(ClassInfo info, List<ClassInfo> ordered, Set<ClassInfo> visited) {
        if (!visited.add(info)) {
            return;
        }
        ordered.add(info);
        for (ClassInfo child : info.children) {
            preorder(child, ordered, visited);
        }
    }

    private static int depth(ClassInfo info) {
        int depth = 0;
        for (ClassInfo parent = info.parent; parent != null; parent = parent.parent) {
            depth++;
        }
        return depth;
    }

    private static long size(ClassInfo info) {
        return info.registration != null ? info.registration.size : -1;
    }

    private static ClassInfo sizedAncestor(ClassInfo info) {
        for (ClassInfo parent = info.parent; parent != null; parent = parent.parent) {
            if (size(parent) >= 0) {
                return parent;
            }
        }
        return null;
    }

    private static ClassInfo smallestSizedDescendant(ClassInfo info) {
        ClassInfo best = null;
        List<ClassInfo> work = new ArrayList<>(info.children);
        while (!work.isEmpty()) {
            ClassInfo child = work.remove(work.size() - 1);
            if (size(child) >= 0 && (best == null || size(child) < size(best))) {
                best = child;
            }
            work.addAll(child.children);
        }
        return best;
    }

    private String bounds(ClassInfo info) {
        ClassInfo lower = sizedAncestor(info);
        ClassInfo upper = smallestSizedDescendant(info);
        return ">= " + (lower != null ? hexShort(size(lower)) + " (" + lower.name + ")" : hexShort(POINTER_SIZE) + " (vftable)")
            + ", " + (upper != null ? "<= " + hexShort(size(upper)) + " (" + upper.name + ")" : "no sized descendant");
    }

    private String describe(ClassInfo info, Map<Long, Integer> slotCounts, Map<Long, String> existingVftables) {
        StringBuilder line = new StringBuilder("`" + info.name + "` RTTI " + hex(info.record) + ", GetRTTI "
            + hexList(info.getRtti) + ", vftable ");
        if (info.vftable >= 0) {
            line.append(hex(info.vftable)).append(" (").append(slotCounts.get(info.vftable)).append(" slots; ")
                .append(info.vftableSource);
            String label = existingVftables.get(info.vftable);
            if (label != null) {
                line.append("; `").append(label).append("::vftable`");
            }
            line.append(")");
        } else if (info.vtables.isEmpty()) {
            line.append("none");
        } else {
            line.append("ambiguous (").append(info.vtables.size()).append(")");
        }
        if (info.registration != null) {
            line.append(", CreateObject ").append(hex(info.registration.createObject));
        }
        long size = size(info);
        line.append(", size ").append(size >= 0 ? hexShort(size) : "? (" + bounds(info) + ")");
        return line.toString();
    }

    private void section(PrintWriter output, String title, List<String> lines) {
        output.println();
        output.println("## " + title);
        output.println();
        if (lines.isEmpty()) {
            output.println("None.");
        }
        for (String line : lines) {
            output.println("- " + line);
        }
    }

    private int slotCount(long start, Set<Long> vtableStarts) throws Exception {
        SymbolTable symbols = currentProgram.getSymbolTable();
        int count = 0;
        while (count < MAX_VTABLE_SLOTS) {
            long slot = start + (long) count * POINTER_SIZE;
            if (count > 0 && (vtableStarts.contains(slot) || startsLabeledItem(symbols, toAddr(slot)))) {
                break;
            }
            long target = u32(slot);
            if (!isExec(target)) {
                break;
            }
            // Code not disassembled yet still counts; data or the middle of an instruction does not.
            Address address = toAddr(target);
            Instruction containing = getInstructionContaining(address);
            if (getDataContaining(address) != null
                    || (containing != null && !containing.getMinAddress().equals(address))) {
                break;
            }
            count++;
        }
        return count;
    }

    // A class has one NiRTTI and one GetRTTI. Of several candidates, one whose pointers are all later slots
    // of another class's vtable (contiguous code pointers back to that vtable's start, nothing starting in
    // between) is a virtual of that vtable, such as a getter returning the base class's NiRTTI. Repeated
    // until no pointer changes, since a dropped pointer no longer starts a vtable.
    private List<SlotUse> reclassifySlots(SymbolTable symbols, Map<Long, ClassInfo> getRttiOwner,
            Map<Long, List<Long>> pointerSources) {
        List<SlotUse> uses = new ArrayList<>();
        Set<Long> slotPointers = new HashSet<>();
        boolean changed = true;
        while (changed) {
            changed = false;
            Map<Long, Long> startPointers = new HashMap<>();
            for (long function : getRttiOwner.keySet()) {
                for (long source : pointerSources.get(function)) {
                    long start = slotPointers.contains(source) ? -1 : vtableStart(symbols, source);
                    if (start >= 0) {
                        startPointers.put(start, source);
                    }
                }
            }
            for (var entry : new ArrayList<>(getRttiOwner.entrySet())) {
                long function = entry.getKey();
                ClassInfo info = entry.getValue();
                if (info.getRtti.size() < 2) {
                    continue;
                }
                List<SlotUse> found = new ArrayList<>();
                int remaining = 0;
                for (long source : pointerSources.get(function)) {
                    if (slotPointers.contains(source)) {
                        continue;
                    }
                    SlotUse use = enclosingVtable(symbols, source, startPointers);
                    ClassInfo vtableClass = use != null ? getRttiOwner.get(u32(use.vtable + (long) GETRTTI_SLOT * POINTER_SIZE)) : null;
                    if (vtableClass == null || vtableClass == info) {
                        remaining++;
                        continue;
                    }
                    use.function = function;
                    use.returned = info;
                    use.vtableClass = vtableClass;
                    found.add(use);
                }
                for (SlotUse use : found) {
                    use.dropped = remaining == 0;
                    slotPointers.add(use.pointer);
                    uses.add(use);
                    changed = true;
                }
                if (!found.isEmpty() && remaining == 0) {
                    info.getRtti.remove(function);
                    getRttiOwner.remove(function);
                }
            }
        }
        return uses;
    }

    // The vtable start a GetRTTI pointer implies (see step 2), or -1.
    private long vtableStart(SymbolTable symbols, long source) {
        if (startsLabeledItem(symbols, toAddr(source))) {
            return source;
        }
        long start = source - (long) GETRTTI_SLOT * POINTER_SIZE;
        return isExec(u32(start)) && isExec(u32(start + POINTER_SIZE))
            && !startsLabeledItem(symbols, toAddr(start + POINTER_SIZE)) ? start : -1;
    }

    // The vtable whose slot past GETRTTI_SLOT a pointer is, walking back over code pointers to the nearest
    // other vtable start with nothing starting in between; null if there is none. This script's own
    // vftable labels do not count as starts, as an earlier run may have put one at the pointer's start.
    private SlotUse enclosingVtable(SymbolTable symbols, long pointer, Map<Long, Long> startPointers) {
        for (int back = 1; back < MAX_VTABLE_SLOTS; back++) {
            long slot = pointer - (long) back * POINTER_SIZE;
            if (startsLabeledItem(symbols, toAddr(slot + POINTER_SIZE), true) || !isExec(u32(slot))) {
                return null;
            }
            Long other = startPointers.get(slot);
            if (other != null && other != pointer) {
                if (back <= GETRTTI_SLOT || u32(slot + (long) GETRTTI_SLOT * POINTER_SIZE) != u32(other)) {
                    return null;
                }
                SlotUse use = new SlotUse();
                use.pointer = pointer;
                use.vtable = slot;
                use.slot = back;
                return use;
            }
        }
        return null;
    }

    private boolean startsLabeledItem(SymbolTable symbols, Address address) {
        return startsLabeledItem(symbols, address, false);
    }

    // A named label, or an address taken as a value (a vtable store), that is not an offcut into a
    // data item starting earlier such as vftable[23]. Direct calls through a slot only read it.
    private boolean startsLabeledItem(SymbolTable symbols, Address address, boolean ignoreOwnVftables) {
        Data containing = getDataContaining(address);
        if (containing != null && !containing.getMinAddress().equals(address)) {
            return false;
        }
        for (Symbol symbol : symbols.getSymbols(address)) {
            if (symbol.getSource() != SourceType.DEFAULT && !(ignoreOwnVftables && ownVftable(symbol))) {
                return true;
            }
        }
        for (Reference reference : getReferencesTo(address)) {
            if (reference.getReferenceType() == RefType.DATA) {
                return true;
            }
        }
        return false;
    }

    // A vftable label as this script creates it; MSVC RTTI labels are IMPORTED.
    private static boolean ownVftable(Symbol symbol) {
        return symbol.getSource() == SourceType.ANALYSIS && symbol.getName().equals(VFTABLE);
    }

    // Vtable immediates stored by a CreateObject and by the constructors it calls.
    private void collectVtables(PseudoDisassembler disassembler, long start, Set<Long> vtables, Set<Long> found,
            long allocator, boolean followCalls) {
        List<Long> callees = new ArrayList<>();
        Instruction instruction = pseudo(disassembler, toAddr(start));
        for (int count = 0; instruction != null && count < MAX_WALK_INSTRUCTIONS; count++) {
            for (int operand = 0; operand < instruction.getNumOperands(); operand++) {
                for (Object object : instruction.getOpObjects(operand)) {
                    long value = object instanceof Scalar scalar ? scalar.getUnsignedValue()
                        : object instanceof Address address ? address.getOffset() : -1;
                    if (vtables.contains(value)) {
                        found.add(value);
                    }
                }
            }
            if (instruction.getFlowType().isCall()) {
                for (Address target : instruction.getFlows()) {
                    if (target.getOffset() != allocator) {
                        callees.add(target.getOffset());
                    }
                }
            }
            if (instruction.getFlowType().isTerminal()) {
                break;
            }
            Address next = instruction.getMaxAddress().next();
            instruction = next != null ? pseudo(disassembler, next) : null;
        }
        if (followCalls) {
            for (long callee : callees) {
                collectVtables(disassembler, callee, vtables, found, allocator, false);
            }
        }
    }

    private static Instruction pseudo(PseudoDisassembler disassembler, Address address) {
        try {
            return disassembler.disassemble(address);
        } catch (Exception undecodable) {
            return null;
        }
    }

    private void annotate(List<ClassInfo> ordered, Map<String, ClassInfo> byName, long register, long unregister,
            long allocator) throws Exception {
        DataTypeManager dtm = currentProgram.getDataTypeManager();
        CategoryPath category = new CategoryPath(CATEGORY);
        DataType rtti = dtm.getDataType(category, RTTI_STRUCT);
        if (rtti == null) {
            StructureDataType record = new StructureDataType(category, RTTI_STRUCT, 0, dtm);
            record.add(new PointerDataType(CharDataType.dataType, dtm), "name", null);
            Structure added = (Structure) dtm.addDataType(record, DataTypeConflictHandler.DEFAULT_HANDLER);
            added.add(new PointerDataType(added, dtm), "base", null);
            rtti = added;
            count("structures created");
        } else if (rtti.getLength() != RTTI_RECORD_SIZE) {
            anomalies.add("Existing `" + CATEGORY + "/" + RTTI_STRUCT + "` is " + rtti.getLength()
                + " bytes; not applied to records");
            rtti = null;
        }
        for (ClassInfo info : ordered) {
            if (byName.get(info.name) != info) {
                continue;
            }
            Namespace namespace = classNamespace(info.name);
            label(info.record, namespace, "ms_RTTI");
            if (rtti != null) {
                applyRecord(info, rtti);
            }
            for (long function : info.getRtti) {
                nameFunction(function, namespace, GETRTTI);
            }
            if (info.vftable >= 0) {
                boolean existing = false;
                for (Symbol symbol : currentProgram.getSymbolTable().getSymbols(toAddr(info.vftable))) {
                    existing |= symbol.getName().equals(VFTABLE) && symbol.getSource() != SourceType.ANALYSIS;
                }
                if (!existing) {
                    label(info.vftable, namespace, VFTABLE);
                }
            }
            if (info.registration != null) {
                nameFunction(info.registration.createObject, namespace, "CreateObject");
            }
            createStruct(info, dtm, category);
        }
        if (register >= 0) {
            nameFunction(register, classNamespace(STREAM_CLASS), "RegisterLoader");
        }
        if (unregister >= 0) {
            nameFunction(unregister, classNamespace(STREAM_CLASS), "UnregisterLoader");
        }
        if (allocator >= 0) {
            nameFunction(allocator, classNamespace(MEMORY_CLASS), "operator_new");
        }
    }

    // Undo names an earlier run gave on wrong evidence: an ANALYSIS <Class>::GetRTTI on an untagged
    // GetRTTI-shaped function returning that class that no longer qualifies returns to its default
    // name, and an ANALYSIS vftable label at the start a reclassified pointer implied is deleted.
    private void repair(Map<Long, ClassInfo> classes, Map<Long, Long> shaped, Map<Long, ClassInfo> getRttiOwner,
            List<SlotUse> reclassified) {
        SymbolTable symbols = currentProgram.getSymbolTable();
        List<Symbol> named = new ArrayList<>();
        symbols.getSymbols(GETRTTI).forEach(named::add);
        for (Symbol symbol : named) {
            long offset = symbol.getAddress().getOffset();
            Function function = getFunctionAt(symbol.getAddress());
            ClassInfo returned = shaped.containsKey(offset) ? classes.get(shaped.get(offset)) : null;
            if (symbol.getSymbolType() != SymbolType.FUNCTION || symbol.getSource() != SourceType.ANALYSIS
                    || getRttiOwner.containsKey(offset) || returned == null || function == null
                    || !function.getTags().isEmpty() || !symbol.getParentNamespace().getName(true).equals(returned.name)) {
                continue;
            }
            String old = symbol.getName(true);
            // Returns the function to its default name, or to another label at its address.
            symbols.removeSymbolSpecial(symbol);
            removed.add("Function " + hex(offset) + " `" + old + "` renamed to `" + function.getName(true) + "` ("
                + function.getSymbol().getSource() + ")");
            count("GetRTTI names removed");
        }
        Set<Long> vftables = new HashSet<>();
        for (ClassInfo info : classes.values()) {
            vftables.add(info.vftable);
        }
        for (SlotUse use : reclassified) {
            long start = use.pointer - (long) GETRTTI_SLOT * POINTER_SIZE;
            if (vftables.contains(start)) {
                continue;
            }
            for (Symbol symbol : symbols.getSymbols(toAddr(start))) {
                if (ownVftable(symbol)) {
                    removed.add("Label " + hex(start) + " `" + symbol.getName(true) + "` deleted (not a vtable start)");
                    symbol.delete();
                    count("vftable labels removed");
                }
            }
        }
    }

    private void count(String change) {
        changes.merge(change, 1, Integer::sum);
    }

    private static boolean renamable(SourceType source) {
        return source == SourceType.DEFAULT || source == SourceType.ANALYSIS;
    }

    // Nested names such as NiShader::NiShaderInstanceDescriptor become nested class namespaces.
    private Namespace classNamespace(String name) throws Exception {
        SymbolTable symbols = currentProgram.getSymbolTable();
        Namespace namespace = currentProgram.getGlobalNamespace();
        for (String part : name.split(SCOPE)) {
            Namespace child = symbols.getNamespace(part, namespace);
            if (child == null) {
                child = symbols.createClass(namespace, part, SourceType.ANALYSIS);
                count("namespaces created");
            }
            namespace = child;
        }
        return namespace;
    }

    private void label(long offset, Namespace namespace, String name) throws Exception {
        SymbolTable symbols = currentProgram.getSymbolTable();
        Address address = toAddr(offset);
        for (Symbol symbol : symbols.getSymbols(address)) {
            if (symbol.getName().equals(name) && symbol.getParentNamespace().equals(namespace)) {
                return;
            }
        }
        Symbol primary = symbols.getPrimarySymbol(address);
        Symbol created = symbols.createLabel(address, name, namespace, SourceType.ANALYSIS);
        if (primary == null || renamable(primary.getSource())) {
            created.setPrimary();
        }
        count("labels created");
    }

    private void nameFunction(long offset, Namespace namespace, String name) throws Exception {
        Address address = toAddr(offset);
        Function function = getFunctionAt(address);
        if (function == null) {
            if (getInstructionAt(address) == null) {
                disassemble(address);
            }
            function = createFunction(address, null);
            if (function == null) {
                anomalies.add("Could not create function " + hex(offset) + " for `" + namespace.getName() + "::" + name + "`");
                return;
            }
            count("functions created");
        }
        Symbol symbol = function.getSymbol();
        if (symbol.getName().equals(name) && symbol.getParentNamespace().equals(namespace)) {
            return;
        }
        if (!renamable(symbol.getSource())) {
            kept.add("Function " + hex(offset) + " `" + symbol.getName(true) + "` (" + symbol.getSource()
                + "), not renamed to `" + namespace.getName() + "::" + name + "`");
            return;
        }
        symbol.setNameAndNamespace(name, namespace, SourceType.ANALYSIS);
        count("functions renamed");
    }

    private void applyRecord(ClassInfo info, DataType rtti) throws Exception {
        Address start = toAddr(info.record);
        Address end = start.add(RTTI_RECORD_SIZE - 1);
        Data existing = getDataAt(start);
        if (existing != null && existing.getDataType().isEquivalent(rtti)) {
            return;
        }
        // Replace only undefined bytes or plain pointers inside the record.
        for (CodeUnit unit : currentProgram.getListing().getCodeUnits(new AddressSet(start, end), true)) {
            boolean replaceable = unit instanceof Data data && unit.getMinAddress().compareTo(start) >= 0
                && unit.getMaxAddress().compareTo(end) <= 0
                && (!data.isDefined() || data.isPointer() || Undefined.isUndefined(data.getDataType()));
            if (!replaceable) {
                kept.add("Data at " + hex(info.record) + " (`" + info.name + "::ms_RTTI`): `" + unit + "`");
                return;
            }
        }
        Data containing = getDataContaining(start);
        if (containing != null && containing.getMinAddress().compareTo(start) < 0) {
            kept.add("Data at " + hex(info.record) + " is inside `" + containing + "`");
            return;
        }
        currentProgram.getListing().clearCodeUnits(start, end, false);
        createData(start, rtti);
        count("NiRTTI records typed");
    }

    private void createStruct(ClassInfo info, DataTypeManager dtm, CategoryPath root) {
        long size = size(info);
        // Outer classes of nested names become subcategories.
        String[] parts = info.name.split(SCOPE);
        CategoryPath category = root;
        for (int i = 0; i < parts.length - 1; i++) {
            category = new CategoryPath(category, parts[i]);
        }
        String structName = parts[parts.length - 1];
        String path = category.getPath() + CategoryPath.DELIMITER_STRING + structName;
        DataType existing = dtm.getDataType(category, structName);
        String parent = info.parent != null ? info.parent.name : "none";
        String description = DESCRIPTION_PREFIX + info.name + ", parent " + parent + "; " + (size >= 0
            ? "size from " + info.name + "::CreateObject " + hexShort(info.registration.createObject)
            : "size unknown, " + bounds(info));
        if (existing != null) {
            info.struct = existing;
            // Only descriptions this script wrote are compared; text appended by other tools is kept.
            String old = existing.getDescription();
            boolean stale = old != null && old.startsWith(DESCRIPTION_PREFIX) && !old.startsWith(description);
            kept.add("Structure `" + path + "` (" + hexShort(existing.getLength()) + " bytes)"
                + (size >= 0 && existing.getLength() != size ? " **size differs from recovered " + hexShort(size) + "**" : "")
                + (stale ? " **description differs from recovered: " + description + "**" : ""));
            return;
        }
        ClassInfo ancestor = sizedAncestor(info);
        DataType base = ancestor != null ? ancestor.struct : null;
        int length = size >= 0 ? (int) size : base != null ? base.getLength() : POINTER_SIZE;
        if (base != null && base.getLength() > length) {
            anomalies.add("`" + info.name + "` (" + hexShort(length) + ") is smaller than its ancestor structure `"
                + ancestor.name + "` (" + hexShort(base.getLength()) + "); used a vftable field instead");
            base = null;
        }
        if (length < POINTER_SIZE) {
            anomalies.add("`" + info.name + "` size " + hexShort(length) + " is too small; no structure created");
            return;
        }
        StructureDataType struct = new StructureDataType(category, structName, length, dtm);
        if (base != null) {
            struct.replaceAtOffset(0, base, base.getLength(), "base",
                ancestor != info.parent ? "Direct parent: " + parent : null);
        } else {
            struct.replaceAtOffset(0, new PointerDataType(VoidDataType.dataType, dtm), POINTER_SIZE, VFTABLE, null);
        }
        struct.setDescription(description);
        info.struct = dtm.addDataType(struct, DataTypeConflictHandler.DEFAULT_HANDLER);
        count("structures created");
    }

    private static long mostCommon(Map<Long, Integer> counts, int minimum) {
        long best = -1;
        int bestCount = minimum - 1;
        for (var entry : counts.entrySet()) {
            if (entry.getValue() > bestCount) {
                best = entry.getKey();
                bestCount = entry.getValue();
            }
        }
        return best;
    }

    private Block blockOf(long address) {
        if (lastBlock != null && address >= lastBlock.start && address < lastBlock.start + lastBlock.bytes.length) {
            return lastBlock;
        }
        for (Block block : blocks) {
            if (address >= block.start && address < block.start + block.bytes.length) {
                lastBlock = block;
                return block;
            }
        }
        return null;
    }

    private long u32(long address) {
        Block block = blockOf(address);
        if (block == null || address + POINTER_SIZE > block.start + block.bytes.length) {
            return -1;
        }
        return le32(block.bytes, (int) (address - block.start));
    }

    private static long le32(byte[] bytes, int index) {
        long value = 0;
        for (int i = POINTER_SIZE - 1; i >= 0; i--) {
            value = (value << BITS_PER_BYTE) | (bytes[index + i] & BYTE_MASK);
        }
        return value;
    }

    private boolean isExec(long address) {
        Block block = address < 0 ? null : blockOf(address);
        return block != null && block.exec;
    }

    private boolean isData(long address) {
        Block block = address < 0 ? null : blockOf(address);
        return block != null && !block.exec;
    }

    private static long alignUp(long address) {
        return (address + POINTER_SIZE - 1) / POINTER_SIZE * POINTER_SIZE;
    }

    // A NUL-terminated C++ identifier (optionally Outer::Inner) of bounded length in non-executable
    // memory, or null.
    private String identifierAt(long address) {
        Block block = address <= 0 ? null : blockOf(address);
        if (block == null || block.exec) {
            return null;
        }
        int index = (int) (address - block.start);
        StringBuilder name = new StringBuilder();
        while (index < block.bytes.length && name.length() <= MAX_NAME_LENGTH) {
            char c = (char) (block.bytes[index++] & BYTE_MASK);
            if (c == 0) {
                return name.length() >= MIN_NAME_LENGTH && validScopes(name.toString()) ? name.toString() : null;
            }
            boolean letter = (c >= 'A' && c <= 'Z') || (c >= 'a' && c <= 'z') || c == '_';
            boolean digit = c >= '0' && c <= '9';
            if (!letter && !digit && c != ':') {
                return null;
            }
            name.append(c);
        }
        return null;
    }

    private static boolean validScopes(String name) {
        for (String part : name.split(SCOPE, -1)) {
            if (part.isEmpty() || part.indexOf(':') >= 0 || Character.isDigit(part.charAt(0))) {
                return false;
            }
        }
        return true;
    }

    private static String hex(long value) {
        return String.format("`0x%08x`", value);
    }

    private static String hexShort(long value) {
        return String.format("0x%x", value);
    }

    private static String hexOrNone(long value) {
        return value >= 0 ? hex(value) : "not found";
    }

    private static String hexList(Iterable<Long> values) {
        List<String> parts = new ArrayList<>();
        for (long value : values) {
            parts.add(hex(value));
        }
        return parts.isEmpty() ? "none" : String.join(", ", parts);
    }
}
