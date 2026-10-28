// Transfer Gamebryo 2.6 function names (DLLs analyzed with their PDBs) to the statically linked
// Gamebryo 2.3 code of the current program, by vtable slot correspondence and calibrated BSim
// similarity; with --apply, rename DEFAULT-named functions, tag them and add plate comments.
// @category ECResearch
import generic.lsh.vector.LSHVector;
import generic.lsh.vector.LSHVectorFactory;
import generic.lsh.vector.VectorCompare;
import ghidra.app.script.GhidraScript;
import ghidra.features.bsim.query.FunctionDatabase;
import ghidra.features.bsim.query.GenSignatures;
import ghidra.features.bsim.query.client.Configuration;
import ghidra.features.bsim.query.description.FunctionDescription;
import ghidra.framework.model.DomainFile;
import ghidra.framework.model.DomainFolder;
import ghidra.program.model.address.Address;
import ghidra.program.model.address.AddressSet;
import ghidra.program.model.listing.Function;
import ghidra.program.model.listing.FunctionTagManager;
import ghidra.program.model.listing.Instruction;
import ghidra.program.model.listing.Program;
import ghidra.program.model.mem.Memory;
import ghidra.program.model.mem.MemoryAccessException;
import ghidra.program.model.mem.MemoryBlock;
import ghidra.program.model.symbol.Namespace;
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
import java.util.HashMap;
import java.util.HashSet;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.TreeMap;
import java.util.TreeSet;

public class TransferGamebryoNames extends GhidraScript {
    private static final String APPLY_FLAG = "--apply";
    private static final String MATCHES_FLAG = "--matches";
    private static final String NO_BSIM_FLAG = "--no-bsim";
    private static final String NO_VTABLE_FLAG = "--no-vtable";
    private static final String SOURCES_FLAG = "--sources";
    private static final String RANGES_FLAG = "--ranges";
    private static final String MAX_SOURCES_FLAG = "--max-sources";
    private static final String MIN_SIM_FLAG = "--min-sim";
    private static final String MIN_SIG_FLAG = "--min-sig";
    private static final String MARGIN_FLAG = "--margin";
    // Without --sources, every program in this project folder is a source, in name order.
    private static final String DEFAULT_SOURCE_FOLDER = "/gamebryo26";
    // Gamebryo code in UOSA.exe, [start, end).
    private static final long[][] DEFAULT_RANGES = { { 0x00650000L, 0x00830000L }, { 0x00aa0000L, 0x00ac0000L } };
    private static final int RANGE_HEX_RADIX = 16;

    // BSim settings as in LocalBSimQueryScript; functions at or below the bound are not compared.
    private static final String BSIM_TEMPLATE = "medium_nosize";
    private static final double SELF_SIGNIFICANCE_BOUND = 15.0;
    // Acceptance defaults, chosen from the anchor calibration (see README).
    private static final double DEFAULT_MIN_SIMILARITY = 0.7;
    private static final double DEFAULT_MIN_SIGNIFICANCE = 30.0;
    // Best minus second-best similarity, on both the target side (different source name) and the
    // source side (different target).
    private static final double DEFAULT_MARGIN = 0.05;
    // Two sources this similar under different names are indistinguishable; neither is transferred.
    private static final double DUPLICATE_SOURCE_SIMILARITY = 0.97;
    private static final double[] CALIBRATION_BUCKETS = { 0.5, 0.6, 0.7, 0.75, 0.8, 0.85, 0.9, 0.95, 0.99 };
    private static final double[] SIGNIFICANCE_BUCKETS = { 0.0, 15.0, 20.0, 30.0, 40.0, 60.0, 100.0 };
    private static final double CALIBRATION_TARGET_AGREEMENT = 0.98;
    // Similarity table counts: pairs, agree, then both at the significance threshold, then both checked.
    private static final int CALIBRATION_COLUMNS = 6;
    private static final double PERCENT = 100.0;
    private static final int PROGRESS_INTERVAL = 250;

    private static final int POINTER_SIZE = 4;
    private static final long DWORD_MASK = 0xffffffffL;
    private static final int MAX_VTABLE_SLOTS = 512;
    // NiRefObject slot 0 is the destructor, which differs per class in a shared vtable family.
    private static final int DESTRUCTOR_SLOT = 0;
    private static final int GETRTTI_SLOT = 2;
    private static final int MAX_INITIALIZER_INSTRUCTIONS = 16;
    private static final int MAX_EXAMPLES = 8;
    private static final int UNSET = -1;

    private static final String SCOPE = "::";
    private static final String VFTABLE = "vftable";
    private static final String PDB_VFTABLE = "`vftable'";
    private static final String MANGLED_VFTABLE_PREFIX = "??_7";
    private static final String MANGLED_VFTABLE_SUFFIX = "@@6B@";
    private static final String MS_RTTI = "ms_RTTI";
    private static final String MANGLED_MS_RTTI = "?ms_RTTI@";
    private static final String MANGLED_SCOPE_END = "@@";
    private static final String MANGLED_SEPARATOR = "@";
    private static final String INITIALIZER_PREFIX = "`dynamic_initializer_for_'";
    private static final String INITIALIZER_SUFFIX = "::ms_RTTI''";
    private static final String GETRTTI = "GetRTTI";
    private static final String TAG_VTABLE = "GB26_VTABLE";
    private static final String TAG_BSIM = "GB26_BSIM";
    private static final String TAG_VTABLE_COMMENT = "Name transferred from a Gamebryo 2.6 vtable slot";
    private static final String TAG_BSIM_COMMENT = "Name transferred from Gamebryo 2.6 by BSim similarity";
    private static final String MANGLED_PREFIX = "?";
    private static final String ORDINAL_PREFIX = "Ordinal_";
    private static final String IMPORT_PREFIX = "__imp_";

    private static final class Settings {
        boolean apply;
        boolean bsim = true;
        boolean vtable = true;
        String output;
        String matches;
        // Project paths of the 2.6 programs; null means every program in DEFAULT_SOURCE_FOLDER.
        List<String> sources;
        List<long[]> ranges = new ArrayList<>(List.of(DEFAULT_RANGES));
        int maxSources = Integer.MAX_VALUE;
        double minSim = DEFAULT_MIN_SIMILARITY;
        double minSig = DEFAULT_MIN_SIGNIFICANCE;
        double margin = DEFAULT_MARGIN;
    }

    /** A namespace path element with the 2.6 symbol kind, so the path can be reproduced. */
    private record NsElem(String name, boolean isClass) {
    }

    /** One 2.6 name at an address: namespace path, method name, qualified name. */
    private record Alias(List<NsElem> path, String ns, String method, String full) {
    }

    private static final class Vtable {
        Program program;
        long start;
        final List<Long> slots = new ArrayList<>();
    }

    private static final class Class26 {
        String name;
        String parent;
        boolean hasRtti;
        Vtable vtable;
        List<String> chain;
    }

    private static final class ClassU {
        String name;
        String parent;
        boolean hasRtti;
        // Slot targets; for a shared (ambiguous) vtable family, the common slots with slot 0 unset.
        final List<Long> slots = new ArrayList<>();
        boolean shared;
        List<String> chain;
    }

    /** A proposed name for a UOSA function, with evidence. */
    private static final class Proposal {
        String name;
        Alias alias;
        String prototype = "";
        String evidence;
        int votes;
        // BSim only: the matched 2.6 function, for the call-graph check.
        Function source;
    }

    private static final class SourceFunction {
        Program program;
        Function function;
        String name;
        Alias alias;
        LSHVector vector;
        int bestTarget = UNSET;
        double bestSim = UNSET;
        double secondSim = UNSET;
    }

    private static final class TargetFunction {
        Function function;
        LSHVector vector;
        int best = UNSET;
        double bestSim = UNSET;
        double bestSig = UNSET;
        double secondDifferentSim = UNSET;
    }

    private final Settings settings = new Settings();
    private final Map<String, Integer> changes = new LinkedHashMap<>();
    private final List<String> notes = new ArrayList<>();
    private final Set<String> missingFunctions = new TreeSet<>();
    private final List<String> classLines = new ArrayList<>();
    private final Map<String, Integer> callgraphStats = new TreeMap<>();
    private final List<String> callgraphRejected = new ArrayList<>();
    // Vtable-class check: UOSA slot targets by Gamebryo class, slots of other labeled vtables, UOSA class
    // chains, and the 2.6 slot targets per source program. Empty with --no-vtable (check off).
    private final Map<Long, Set<String>> slotClassesU = new HashMap<>();
    private final Set<Long> foreignSlotsU = new HashSet<>();
    private final Map<String, List<String>> chainsU = new HashMap<>();
    private final Map<Program, Set<Long>> slots26 = new HashMap<>();
    private final List<String> classCheckRejected = new ArrayList<>();
    // Names the call-graph check knows: anchors and this run's final names.
    private final Map<Long, String> known = new HashMap<>();
    private final Map<Program, Map<Long, List<Alias>>> aliasCache = new HashMap<>();

    public void run() throws Exception {
        parseArgs(getScriptArgs());
        if (Files.exists(Path.of(settings.output))
                || (settings.matches != null && Files.exists(Path.of(settings.matches)))) {
            throw new IllegalArgumentException("Output file exists: " + settings.output + " / " + settings.matches);
        }
        List<Program> sources = new ArrayList<>();
        try {
            for (String path : settings.sources != null ? settings.sources : folderPrograms()) {
                DomainFile file = state.getProject().getProjectData().getFile(path);
                if (file == null) {
                    throw new IllegalArgumentException("No such project file: " + path);
                }
                sources.add((Program) file.getDomainObject(this, false, true, monitor));
            }
            transfer(sources);
        } finally {
            for (Program source : sources) {
                source.release(this);
            }
        }
    }

    private List<String> folderPrograms() {
        DomainFolder folder = state.getProject().getProjectData().getFolder(DEFAULT_SOURCE_FOLDER);
        if (folder == null) {
            throw new IllegalArgumentException("No project folder " + DEFAULT_SOURCE_FOLDER + "; use " + SOURCES_FLAG);
        }
        List<String> paths = new ArrayList<>();
        for (DomainFile file : folder.getFiles()) {
            if (Program.class.isAssignableFrom(file.getDomainObjectClass())) {
                paths.add(file.getPathname());
            }
        }
        if (paths.isEmpty()) {
            throw new IllegalArgumentException("No programs in " + DEFAULT_SOURCE_FOLDER);
        }
        paths.sort(null);
        return paths;
    }

    private void parseArgs(String[] args) {
        if (args.length < 1) {
            throw new IllegalArgumentException("Expected OUTPUT_FILE [" + APPLY_FLAG + "] [" + MATCHES_FLAG + " FILE] ["
                + NO_BSIM_FLAG + "] [" + NO_VTABLE_FLAG + "] [" + SOURCES_FLAG + " /a,/b] [" + RANGES_FLAG
                + " start-end,...] [" + MAX_SOURCES_FLAG + " N] [" + MIN_SIM_FLAG + " F] [" + MIN_SIG_FLAG + " F] ["
                + MARGIN_FLAG + " F]");
        }
        settings.output = args[0];
        for (int i = 1; i < args.length; i++) {
            String arg = args[i];
            switch (arg) {
                case APPLY_FLAG -> settings.apply = true;
                case NO_BSIM_FLAG -> settings.bsim = false;
                case NO_VTABLE_FLAG -> settings.vtable = false;
                case MATCHES_FLAG -> settings.matches = value(args, ++i, arg);
                case SOURCES_FLAG -> settings.sources = List.of(value(args, ++i, arg).split(","));
                case MAX_SOURCES_FLAG -> settings.maxSources = Integer.parseInt(value(args, ++i, arg));
                case MIN_SIM_FLAG -> settings.minSim = Double.parseDouble(value(args, ++i, arg));
                case MIN_SIG_FLAG -> settings.minSig = Double.parseDouble(value(args, ++i, arg));
                case MARGIN_FLAG -> settings.margin = Double.parseDouble(value(args, ++i, arg));
                case RANGES_FLAG -> {
                    settings.ranges = new ArrayList<>();
                    for (String range : value(args, ++i, arg).split(",")) {
                        String[] bounds = range.split("-");
                        settings.ranges.add(new long[] { Long.parseLong(bounds[0], RANGE_HEX_RADIX),
                            Long.parseLong(bounds[1], RANGE_HEX_RADIX) });
                    }
                }
                default -> throw new IllegalArgumentException("Unknown argument: " + arg);
            }
        }
    }

    private static String value(String[] args, int index, String flag) {
        if (index >= args.length) {
            throw new IllegalArgumentException(flag + " needs a value");
        }
        return args[index];
    }

    private void transfer(List<Program> sources) throws Exception {
        // 1. Vtable correspondence.
        Map<Long, Proposal> vtableNames = new TreeMap<>();
        Map<Long, List<String>> vtableConflicts = new TreeMap<>();
        List<String> vtableClasses = classLines;
        Map<String, Integer> vtableStats = new LinkedHashMap<>();
        if (settings.vtable) {
            vtableCorrespondence(sources, vtableNames, vtableConflicts, vtableClasses, vtableStats);
        }

        // 2. BSim.
        Map<Long, Proposal> bsimNames = new TreeMap<>();
        List<TargetFunction> targets = new ArrayList<>();
        List<SourceFunction> sourceFunctions = new ArrayList<>();
        Map<String, Integer> bsimStats = new LinkedHashMap<>();
        Map<Long, String> bsimRejected = new TreeMap<>();
        if (settings.bsim) {
            bsim(sources, targets, sourceFunctions, bsimNames, bsimRejected, bsimStats);
        }

        // 3. Merge: vtable beats BSim; never touch names we did not create.
        Set<String> allSourceNames = new HashSet<>();
        for (Program source : sources) {
            for (Function function : source.getFunctionManager().getFunctions(true)) {
                for (Alias alias : aliases(source, function.getEntryPoint().getOffset())) {
                    allSourceNames.add(alias.full());
                }
            }
        }
        // Existing names that also occur in 2.6 (not ours) are anchors.
        Map<Long, String> anchors = new TreeMap<>();
        for (Function function : currentProgram.getFunctionManager().getFunctions(true)) {
            if (function.getSymbol().getSource() == SourceType.DEFAULT || isOurs(function)) {
                continue;
            }
            String name = function.getName(true);
            if (allSourceNames.contains(name)) {
                anchors.put(function.getEntryPoint().getOffset(), name);
            }
        }
        Map<Long, Proposal> finalNames = new TreeMap<>(bsimNames);
        List<String> sourceDisagreements = new ArrayList<>();
        int sourceAgreements = 0;
        for (var entry : vtableNames.entrySet()) {
            Proposal bsim = bsimNames.get(entry.getKey());
            if (bsim != null) {
                if (bsim.name.equals(entry.getValue().name)) {
                    sourceAgreements++;
                } else {
                    sourceDisagreements.add(hex(entry.getKey()) + " vtable `" + entry.getValue().name + "` ("
                        + entry.getValue().evidence + "), BSim `" + bsim.name + "` (" + bsim.evidence + ")");
                }
            }
            finalNames.put(entry.getKey(), entry.getValue());
        }
        // A BSim name that a vtable already gives to another function is dropped.
        Map<String, Long> vtableByName = new HashMap<>();
        for (var entry : vtableNames.entrySet()) {
            vtableByName.putIfAbsent(entry.getValue().name, entry.getKey());
        }
        List<String> collisions = new ArrayList<>();
        for (var entry : bsimNames.entrySet()) {
            Long other = vtableByName.get(entry.getValue().name);
            if (other != null && other.longValue() != entry.getKey() && !vtableNames.containsKey(entry.getKey())) {
                collisions.add(hex(entry.getKey()) + " BSim `" + entry.getValue().name + "`, vtable gives it to "
                    + hex(other));
                finalNames.remove(entry.getKey());
            }
        }

        // Vtable-class check: a 2.6 virtual of class S matched to a UOSA vtable slot of Gamebryo classes none
        // of which is S or derives from S is dropped. Call-graph check: a BSim pair whose named UOSA callees
        // include none of the 2.6 function's callees is dropped.
        known.putAll(anchors);
        for (var entry : finalNames.entrySet()) {
            known.putIfAbsent(entry.getKey(), entry.getValue().name);
        }
        for (var entry : new ArrayList<>(bsimNames.entrySet())) {
            if (!finalNames.containsKey(entry.getKey()) || vtableNames.containsKey(entry.getKey())) {
                continue;
            }
            String classConflict = classCheck(entry.getKey(), entry.getValue().alias, entry.getValue().source);
            if (classConflict != null) {
                classCheckRejected.add(hex(entry.getKey()) + " `" + entry.getValue().name + "` ("
                    + entry.getValue().evidence + "): " + classConflict);
                finalNames.remove(entry.getKey());
                continue;
            }
            int[] callees = callgraphAgreement(entry.getKey(), entry.getValue().source);
            int checkable = callees[0];
            int agree = callees[1];
            if (checkable == 0) {
                callgraphStats.merge("no named callee", 1, Integer::sum);
            } else if (agree == checkable) {
                callgraphStats.merge("all named callees agree", 1, Integer::sum);
            } else if (agree > 0) {
                callgraphStats.merge("some named callees agree", 1, Integer::sum);
            } else {
                callgraphStats.merge("no named callee agrees (dropped)", 1, Integer::sum);
                callgraphRejected.add(hex(entry.getKey()) + " `" + entry.getValue().name + "` (" + entry.getValue().evidence
                    + "): " + checkable + " named callees, none called by the 2.6 function");
                finalNames.remove(entry.getKey());
            }
        }

        // 4. Compare with existing names (anchors) and calibrate BSim on them.
        FunctionTagManager tags = currentProgram.getFunctionManager().getFunctionTagManager();
        List<String> anchorAgreementsVtable = new ArrayList<>();
        List<String> anchorDisagreementsVtable = new ArrayList<>();
        List<String> anchorAgreementsBsim = new ArrayList<>();
        List<String> anchorDisagreementsBsim = new ArrayList<>();
        int existingOther = 0;
        for (var entry : finalNames.entrySet()) {
            Function function = getFunctionAt(toAddr(entry.getKey()));
            if (function.getSymbol().getSource() == SourceType.DEFAULT || isOurs(function)) {
                continue;
            }
            String existing = function.getName(true);
            boolean fromVtable = vtableNames.get(entry.getKey()) == entry.getValue();
            String line = hex(entry.getKey()) + " `" + existing + "` vs `" + entry.getValue().name + "` ("
                + entry.getValue().evidence + ")";
            if (!anchors.containsKey(entry.getKey())) {
                existingOther++;
            } else if (existing.equals(entry.getValue().name)) {
                (fromVtable ? anchorAgreementsVtable : anchorAgreementsBsim).add(line);
            } else {
                (fromVtable ? anchorDisagreementsVtable : anchorDisagreementsBsim).add(line);
            }
        }
        // Calibration set: anchors plus vtable names in the BSim target ranges.
        Map<Long, String> calibration = new TreeMap<>(anchors);
        for (var entry : vtableNames.entrySet()) {
            calibration.putIfAbsent(entry.getKey(), entry.getValue().name);
        }

        // 5. Apply.
        Map<Long, String> applied = new TreeMap<>();
        if (settings.apply) {
            ensureTag(tags, TAG_VTABLE, TAG_VTABLE_COMMENT);
            ensureTag(tags, TAG_BSIM, TAG_BSIM_COMMENT);
            for (var entry : finalNames.entrySet()) {
                boolean fromVtable = vtableNames.get(entry.getKey()) == entry.getValue();
                try {
                    applied.put(entry.getKey(), apply(entry.getKey(), entry.getValue(), fromVtable ? TAG_VTABLE : TAG_BSIM));
                } catch (Exception e) {
                    applied.put(entry.getKey(), "failed: " + e.getMessage());
                    count("rename failed");
                }
            }
        }

        writeReport(sources, vtableNames, vtableConflicts, vtableClasses, vtableStats, bsimNames, bsimRejected, bsimStats,
            targets, sourceFunctions, finalNames, sourceAgreements, sourceDisagreements, collisions, anchors,
            anchorAgreementsVtable, anchorDisagreementsVtable, anchorAgreementsBsim, anchorDisagreementsBsim,
            existingOther, calibration);
        if (settings.matches != null) {
            writeMatches(vtableNames, vtableConflicts, bsimNames, bsimRejected, finalNames, applied, sourceDisagreements,
                collisions, anchorDisagreementsVtable, anchorDisagreementsBsim);
        }
    }

    // ---------------------------------------------------------------- vtable correspondence

    private void vtableCorrespondence(List<Program> sources, Map<Long, Proposal> names, Map<Long, List<String>> conflicts,
            List<String> classLines, Map<String, Integer> stats) throws Exception {
        Map<String, Class26> classes26 = new TreeMap<>();
        for (Program source : sources) {
            collectClasses26(source, classes26);
        }
        for (Class26 info : classes26.values()) {
            info.chain = chain26(classes26, info);
        }
        Map<String, ClassU> classesU = collectClassesU(classes26.keySet());
        Set<Long> foreignSlots = foreignVtableSlots(classesU);
        foreignSlotsU.addAll(foreignSlots);
        for (ClassU classU : classesU.values()) {
            chainsU.put(classU.name, classU.chain);
            for (Long slot : classU.slots) {
                if (slot != null) {
                    slotClassesU.computeIfAbsent(slot, key -> new HashSet<>()).add(classU.name);
                }
            }
        }
        stats.put("2.6 classes with a vtable", count(classes26.values(), c -> c.vtable != null));
        stats.put("2.6 classes with NiRTTI parent data", count(classes26.values(), c -> c.hasRtti));
        stats.put("UOSA classes with a vtable", count(classesU.values(), c -> !c.slots.isEmpty()));
        stats.put("UOSA shared vtable families (common slots only)", count(classesU.values(), c -> c.shared));

        // Method name per (introducing class, slot): the intersection of the candidate method names
        // over every 2.6 class that has the slot, so ICF-folded functions resolve to the slot's method.
        Map<String, Set<String>> slotMethods = new HashMap<>();
        for (Class26 info : classes26.values()) {
            if (info.vtable == null) {
                continue;
            }
            for (int slot = 0; slot < info.vtable.slots.size(); slot++) {
                Set<String> methods = new TreeSet<>();
                for (Alias alias : chainAliases(info, slot)) {
                    methods.add(alias.method());
                }
                if (methods.isEmpty()) {
                    continue;
                }
                String key = introducer(classes26, info, slot) + "#" + slot;
                Set<String> current = slotMethods.get(key);
                if (current == null) {
                    slotMethods.put(key, methods);
                } else {
                    current.retainAll(methods);
                }
            }
        }

        Map<Long, Map<String, Proposal>> votes = new TreeMap<>();
        int mappedClasses = 0;
        int mappedSlots = 0;
        int unresolved = 0;
        int noFunction = 0;
        int foreign = 0;
        for (ClassU classU : classesU.values()) {
            Class26 class26 = classes26.get(classU.name);
            if (class26 == null || class26.vtable == null || classU.slots.isEmpty()) {
                continue;
            }
            // Deepest A on both chains such that every class root..A has equal vtable lengths.
            String deepest = null;
            int length = 0;
            List<String> chainU = classU.chain;
            List<String> chain26 = class26.chain;
            for (int i = 0; i < chainU.size() && i < chain26.size(); i++) {
                ClassU ancestorU = classesU.get(chainU.get(i));
                Class26 ancestor26 = classes26.get(chain26.get(i));
                if (!chainU.get(i).equals(chain26.get(i)) || ancestorU == null || ancestor26 == null
                        || ancestor26.vtable == null || ancestorU.slots.isEmpty()
                        || ancestorU.slots.size() != ancestor26.vtable.slots.size()) {
                    break;
                }
                deepest = chainU.get(i);
                length = ancestorU.slots.size();
            }
            // A vtable shorter than its ancestor's was cut short (label or code-stored address inside).
            if (classU.slots.size() < length || class26.vtable.slots.size() < length) {
                notes.add("`" + classU.name + "` vtable is shorter than its ancestor `" + deepest + "`'s; not mapped");
                deepest = null;
            }
            classLines.add("`" + classU.name + "`: UOSA " + classU.slots.size() + " slots, 2.6 "
                + class26.vtable.slots.size() + ", chain " + String.join(" > ", chainU)
                + (deepest == null ? ", not mapped" : ", slots [0, " + length + ") via `" + deepest + "`")
                + (classU.shared ? " (shared vtable family)" : ""));
            if (deepest == null) {
                continue;
            }
            mappedClasses++;
            for (int slot = 0; slot < length; slot++) {
                Long target = classU.slots.get(slot);
                if (target == null) {
                    continue;
                }
                mappedSlots++;
                Set<String> methods = slotMethods.get(introducer(classes26, class26, slot) + "#" + slot);
                Alias chosen = null;
                if (methods != null && methods.size() == 1) {
                    String method = methods.iterator().next();
                    // The alias in the class nearest to C wins.
                    List<Alias> candidates = chainAliases(class26, slot);
                    for (int depth = class26.chain.size() - 1; depth >= 0 && chosen == null; depth--) {
                        for (Alias alias : candidates) {
                            if (alias.method().equals(method) && alias.ns().equals(class26.chain.get(depth))) {
                                chosen = alias;
                                break;
                            }
                        }
                    }
                }
                if (chosen == null) {
                    unresolved++;
                    continue;
                }
                Function function = getFunctionAt(toAddr(target));
                if (function == null) {
                    missingFunctions.add(hex(target) + " `" + chosen.full() + "` (" + classU.name + " slot " + slot + ")");
                    noFunction++;
                    continue;
                }
                if (foreignSlots.contains(target)) {
                    foreign++;
                    continue;
                }
                Proposal proposal = votes.computeIfAbsent(target, key -> new TreeMap<>()).get(chosen.full());
                if (proposal == null) {
                    proposal = new Proposal();
                    proposal.name = chosen.full();
                    proposal.alias = chosen;
                    proposal.prototype = prototype(class26.vtable.program, class26.vtable.slots.get(slot), chosen);
                    proposal.evidence = "vtable " + classU.name + " slot " + slot;
                    votes.get(target).put(chosen.full(), proposal);
                }
                proposal.votes++;
            }
        }
        for (var entry : votes.entrySet()) {
            if (entry.getValue().size() == 1) {
                Proposal proposal = entry.getValue().values().iterator().next();
                if (proposal.votes > 1) {
                    proposal.evidence += " (+" + (proposal.votes - 1) + " more slots)";
                }
                names.put(entry.getKey(), proposal);
            } else {
                List<String> conflict = new ArrayList<>();
                for (Proposal proposal : entry.getValue().values()) {
                    conflict.add("`" + proposal.name + "` (" + proposal.evidence + ", " + proposal.votes + " slots)");
                }
                conflicts.put(entry.getKey(), conflict);
            }
        }
        stats.put("Classes mapped (equal chain lengths)", mappedClasses);
        stats.put("Mapped slots", mappedSlots);
        stats.put("Slots without a unique 2.6 method name (folded or pure)", unresolved);
        stats.put("Slots whose UOSA target is not a function", noFunction);
        stats.put("Slots whose UOSA target is also in a non-Gamebryo vtable", foreign);
        stats.put("UOSA functions named by vtables", names.size());
        stats.put("UOSA functions with conflicting vtable names", conflicts.size());
    }

    /** Aliases of a class's slot target whose namespace is the class or one of its 2.6 ancestors. */
    private List<Alias> chainAliases(Class26 info, int slot) {
        List<Alias> result = new ArrayList<>();
        Set<String> chain = new HashSet<>(info.chain);
        for (Alias alias : aliases(info.vtable.program, info.vtable.slots.get(slot))) {
            if (chain.contains(alias.ns())) {
                result.add(alias);
            }
        }
        return result;
    }

    /** The topmost ancestor whose vtable already has the slot (the class that introduced it). */
    private static String introducer(Map<String, Class26> classes, Class26 info, int slot) {
        for (String name : info.chain) {
            Class26 ancestor = classes.get(name);
            if (ancestor != null && ancestor.vtable != null && ancestor.vtable.slots.size() > slot) {
                return name;
            }
        }
        return info.name;
    }

    private void collectClasses26(Program program, Map<String, Class26> classes) throws Exception {
        SymbolTable symbols = program.getSymbolTable();
        Map<Long, Vtable> vtables = new HashMap<>();
        for (Symbol symbol : symbols.getAllSymbols(false)) {
            String name = symbol.getName();
            String className = null;
            if (name.equals(PDB_VFTABLE)) {
                className = symbol.getParentNamespace().getName(true);
            } else if (name.startsWith(MANGLED_VFTABLE_PREFIX) && name.endsWith(MANGLED_VFTABLE_SUFFIX)) {
                className = demangleScope(name.substring(MANGLED_VFTABLE_PREFIX.length(),
                    name.length() - MANGLED_VFTABLE_SUFFIX.length()));
            }
            if (className == null) {
                continue;
            }
            long start = symbol.getAddress().getOffset();
            Vtable vtable = vtables.computeIfAbsent(start, key -> readVtable(program, key));
            slots26.computeIfAbsent(program, key -> new HashSet<>()).addAll(vtable.slots);
            Class26 info = classes.computeIfAbsent(className, key -> new Class26());
            info.name = className;
            if (info.vtable != null && (info.vtable.program != program || info.vtable.start != start)) {
                notes.add("2.6 `" + className + "` has several vtables; the first is used");
                continue;
            }
            info.vtable = vtable;
        }
        // Parents from the NiRTTI dynamic initializers: PUSH <base ms_RTTI>; PUSH <name>; MOV ECX, <ms_RTTI>.
        for (Function function : program.getFunctionManager().getFunctions(true)) {
            // Ghidra splits the name at "::", so the class part may be a namespace.
            String name = function.getName(true);
            if (!name.startsWith(INITIALIZER_PREFIX) || !name.endsWith(INITIALIZER_SUFFIX)) {
                continue;
            }
            String className = name.substring(INITIALIZER_PREFIX.length(), name.length() - INITIALIZER_SUFFIX.length());
            String parent = null;
            int seen = 0;
            for (Instruction instruction : program.getListing().getInstructions(function.getBody(), true)) {
                if (seen++ >= MAX_INITIALIZER_INSTRUCTIONS) {
                    break;
                }
                for (Reference reference : instruction.getReferencesFrom()) {
                    for (Symbol symbol : symbols.getSymbols(reference.getToAddress())) {
                        String rtti = rttiClass(symbol);
                        if (rtti != null && !rtti.equals(className)) {
                            parent = rtti;
                        }
                    }
                }
            }
            Class26 info = classes.computeIfAbsent(className, key -> new Class26());
            info.name = className;
            if (info.hasRtti && !java.util.Objects.equals(info.parent, parent)) {
                notes.add("2.6 `" + className + "` has NiRTTI initializers with different parents");
            }
            info.hasRtti = true;
            info.parent = parent;
        }
    }

    /** Class of an ms_RTTI symbol (demangled label, mangled public or import pointer), else null. */
    private static String rttiClass(Symbol symbol) {
        String name = symbol.getName();
        if (name.equals(MS_RTTI)) {
            return symbol.getParentNamespace().getName(true);
        }
        int start = name.indexOf(MANGLED_MS_RTTI);
        if (start < 0) {
            return null;
        }
        int end = name.indexOf(MANGLED_SCOPE_END, start);
        return end < 0 ? null : demangleScope(name.substring(start + MANGLED_MS_RTTI.length(), end));
    }

    /** "Inner@Outer" -> "Outer::Inner"; null for templates and other special names. */
    private static String demangleScope(String mangled) {
        if (mangled.isEmpty() || mangled.contains("?") || mangled.contains("$")) {
            return null;
        }
        String[] parts = mangled.split(MANGLED_SEPARATOR);
        StringBuilder result = new StringBuilder();
        for (int i = parts.length - 1; i >= 0; i--) {
            result.append(parts[i]);
            if (i > 0) {
                result.append(SCOPE);
            }
        }
        return result.toString();
    }

    private static List<String> chain26(Map<String, Class26> classes, Class26 info) {
        List<String> chain = new ArrayList<>();
        Set<String> seen = new HashSet<>();
        for (Class26 current = info; current != null && seen.add(current.name);
                current = current.hasRtti && current.parent != null ? classes.get(current.parent) : null) {
            chain.add(0, current.name);
        }
        return chain;
    }

    /** UOSA classes: NiRTTI classes (ms_RTTI labels) and labeled vtables of classes the 2.6 programs have. */
    private Map<String, ClassU> collectClassesU(Set<String> names26) throws Exception {
        SymbolTable symbols = currentProgram.getSymbolTable();
        Map<String, ClassU> classes = new TreeMap<>();
        Map<Long, String> recordClass = new HashMap<>();
        for (Symbol symbol : symbols.getSymbols(MS_RTTI)) {
            recordClass.put(symbol.getAddress().getOffset(), symbol.getParentNamespace().getName(true));
        }
        for (var entry : recordClass.entrySet()) {
            ClassU info = classes.computeIfAbsent(entry.getValue(), key -> new ClassU());
            info.name = entry.getValue();
            info.hasRtti = true;
            long base = u32(currentProgram, entry.getKey() + POINTER_SIZE);
            info.parent = base == 0 ? null : recordClass.get(base);
            if (base != 0 && info.parent == null) {
                notes.add("UOSA `" + info.name + "` ms_RTTI base " + hex(base) + " is not a labeled record");
            }
        }
        for (Symbol symbol : symbols.getSymbols(VFTABLE)) {
            String name = symbol.getParentNamespace().getName(true);
            if (!classes.containsKey(name) && !names26.contains(name)) {
                continue;
            }
            ClassU info = classes.computeIfAbsent(name, key -> new ClassU());
            info.name = name;
            if (!info.slots.isEmpty()) {
                notes.add("UOSA `" + name + "` has several vftable labels; not mapped");
                info.slots.clear();
                info.shared = true;
                continue;
            }
            info.slots.addAll(readVtable(currentProgram, symbol.getAddress().getOffset()).slots);
        }
        // A NiRTTI class without a vtable label: every vtable with its GetRTTI in slot 2 belongs to it or to
        // a subclass without its own NiRTTI. If they all agree on the slots of the shortest one (bar the
        // destructor), that is the class's vtable layout.
        for (ClassU info : classes.values()) {
            if (!info.slots.isEmpty() || info.shared || !info.hasRtti) {
                continue;
            }
            Function getRtti = null;
            for (Symbol symbol : symbols.getSymbols(GETRTTI)) {
                if (symbol.getParentNamespace().getName(true).equals(info.name)) {
                    getRtti = getFunctionAt(symbol.getAddress());
                }
            }
            if (getRtti == null) {
                continue;
            }
            List<List<Long>> family = new ArrayList<>();
            for (Reference reference : getReferencesTo(getRtti.getEntryPoint())) {
                if (getInstructionAt(reference.getFromAddress()) != null) {
                    continue;
                }
                long start = reference.getFromAddress().getOffset() - (long) GETRTTI_SLOT * POINTER_SIZE;
                List<Long> slots = readVtable(currentProgram, start).slots;
                if (slots.size() > GETRTTI_SLOT && slots.get(GETRTTI_SLOT) == getRtti.getEntryPoint().getOffset()) {
                    family.add(slots);
                }
            }
            if (family.isEmpty()) {
                continue;
            }
            int shortest = Integer.MAX_VALUE;
            for (List<Long> slots : family) {
                shortest = Math.min(shortest, slots.size());
            }
            boolean agree = true;
            for (int slot = DESTRUCTOR_SLOT + 1; slot < shortest && agree; slot++) {
                for (List<Long> slots : family) {
                    agree &= slots.get(slot).equals(family.get(0).get(slot));
                }
            }
            if (!agree) {
                notes.add("UOSA `" + info.name + "`: " + family.size() + " vtables with its GetRTTI disagree; not mapped");
                continue;
            }
            info.slots.addAll(family.get(0).subList(0, shortest));
            info.slots.set(DESTRUCTOR_SLOT, null);
            info.shared = family.size() > 1;
        }
        for (ClassU info : classes.values()) {
            List<String> chain = new ArrayList<>();
            Set<String> seen = new HashSet<>();
            for (ClassU current = info; current != null && seen.add(current.name);
                    current = current.parent == null ? null : classes.get(current.parent)) {
                chain.add(0, current.name);
            }
            info.chain = chain;
        }
        return classes;
    }

    /** Slot addresses of labeled vtables that have no Gamebryo GetRTTI in slot 2 and are not mapped classes. */
    private Set<Long> foreignVtableSlots(Map<String, ClassU> classes) throws Exception {
        Set<Long> gamebryoGetRtti = new HashSet<>();
        SymbolTable symbols = currentProgram.getSymbolTable();
        for (Symbol symbol : symbols.getSymbols(GETRTTI)) {
            if (classes.containsKey(symbol.getParentNamespace().getName(true))) {
                gamebryoGetRtti.add(symbol.getAddress().getOffset());
            }
        }
        Set<Long> targets = new HashSet<>();
        for (Symbol symbol : symbols.getSymbols(VFTABLE)) {
            if (classes.containsKey(symbol.getParentNamespace().getName(true))) {
                continue;
            }
            List<Long> slots = readVtable(currentProgram, symbol.getAddress().getOffset()).slots;
            if (slots.size() > GETRTTI_SLOT && gamebryoGetRtti.contains(slots.get(GETRTTI_SLOT))) {
                continue;
            }
            targets.addAll(slots);
        }
        return targets;
    }

    /** Code pointers from the start until a non-code pointer, the next symbol, or the maximum. */
    private Vtable readVtable(Program program, long start) {
        Vtable vtable = new Vtable();
        vtable.program = program;
        vtable.start = start;
        SymbolTable symbols = program.getSymbolTable();
        Memory memory = program.getMemory();
        for (int slot = 0; slot < MAX_VTABLE_SLOTS; slot++) {
            long address = start + (long) slot * POINTER_SIZE;
            Address slotAddress = addr(program, address);
            if (slot > 0 && startsItem(program, symbols, slotAddress)) {
                break;
            }
            long value;
            try {
                value = u32(program, address);
            } catch (MemoryAccessException e) {
                break;
            }
            MemoryBlock block = memory.getBlock(addr(program, value));
            if (block == null || !block.isExecute() || !block.isInitialized()) {
                break;
            }
            vtable.slots.add(value);
        }
        return vtable;
    }

    /**
     * A slot address starts another item if it has a real label or its address is taken. Dynamic labels
     * from READ references (devirtualized calls through the slot) do not end a vtable.
     */
    private static boolean startsItem(Program program, SymbolTable symbols, Address address) {
        for (Symbol symbol : symbols.getSymbols(address)) {
            if (!symbol.isDynamic()) {
                return true;
            }
        }
        for (Reference reference : program.getReferenceManager().getReferencesTo(address)) {
            if (!reference.getReferenceType().isRead()) {
                return true;
            }
        }
        return false;
    }

    // ---------------------------------------------------------------- BSim

    private void bsim(List<Program> sources, List<TargetFunction> targets, List<SourceFunction> sourceFunctions,
            Map<Long, Proposal> names, Map<Long, String> rejected, Map<String, Integer> stats) throws Exception {
        LSHVectorFactory factory = FunctionDatabase.generateLSHVectorFactory();
        Configuration config = FunctionDatabase.loadConfigurationTemplate(BSIM_TEMPLATE);
        factory.set(config.weightfactory, config.idflookup, config.info.settings);

        int folded = 0;
        int sourceCandidates = 0;
        for (Program source : sources) {
            List<Function> functions = new ArrayList<>();
            for (Function function : source.getFunctionManager().getFunctions(true)) {
                if (functions.size() >= settings.maxSources) {
                    break;
                }
                SourceType type = function.getSymbol().getSource();
                if (function.isThunk() || function.isExternal()
                        || (type != SourceType.IMPORTED && type != SourceType.USER_DEFINED)) {
                    continue;
                }
                List<Alias> aliases = aliases(source, function.getEntryPoint().getOffset());
                if (aliases.size() != 1) {
                    folded++;
                    continue;
                }
                String name = aliases.get(0).full();
                if (name.contains("$") || name.contains("@")) {
                    continue;
                }
                functions.add(function);
            }
            sourceCandidates += functions.size();
            monitor.setMessage("BSim signatures: " + source.getName());
            Map<Long, LSHVector> vectors = signatures(source, functions, factory);
            for (Function function : functions) {
                LSHVector vector = vectors.get(function.getEntryPoint().getOffset());
                if (vector == null || factory.getSelfSignificance(vector) <= SELF_SIGNIFICANCE_BOUND) {
                    continue;
                }
                SourceFunction entry = new SourceFunction();
                entry.program = source;
                entry.function = function;
                entry.alias = aliases(source, function.getEntryPoint().getOffset()).get(0);
                entry.name = entry.alias.full();
                entry.vector = vector;
                sourceFunctions.add(entry);
            }
        }
        AddressSet ranges = new AddressSet();
        for (long[] range : settings.ranges) {
            ranges.add(toAddr(range[0]), toAddr(range[1] - 1));
        }
        List<Function> targetFunctions = new ArrayList<>();
        for (Function function : currentProgram.getFunctionManager().getFunctions(ranges, true)) {
            if (!function.isThunk() && !function.isExternal()) {
                targetFunctions.add(function);
            }
        }
        monitor.setMessage("BSim signatures: " + currentProgram.getName());
        Map<Long, LSHVector> targetVectors = signatures(currentProgram, targetFunctions, factory);
        for (Function function : targetFunctions) {
            LSHVector vector = targetVectors.get(function.getEntryPoint().getOffset());
            if (vector != null && factory.getSelfSignificance(vector) > SELF_SIGNIFICANCE_BOUND) {
                TargetFunction entry = new TargetFunction();
                entry.function = function;
                entry.vector = vector;
                targets.add(entry);
            }
        }
        stats.put("Source functions with one 2.6 name (not thunks/imports)", sourceCandidates);
        stats.put("Source functions skipped as ICF-folded (several names)", folded);
        stats.put("Sources above the self-significance bound", sourceFunctions.size());
        stats.put("UOSA functions in the ranges", targetFunctions.size());
        stats.put("Targets above the self-significance bound", targets.size());

        // All pairs: best and second-best per target (second with a different name) and per source.
        Map<String, Integer> nameIds = new HashMap<>();
        int[] sourceNameIds = new int[sourceFunctions.size()];
        for (int s = 0; s < sourceFunctions.size(); s++) {
            sourceNameIds[s] = nameIds.computeIfAbsent(sourceFunctions.get(s).name, key -> nameIds.size());
        }
        VectorCompare compare = new VectorCompare();
        monitor.initialize(targets.size());
        for (int t = 0; t < targets.size(); t++) {
            monitor.checkCancelled();
            if (t % PROGRESS_INTERVAL == 0) {
                monitor.setMessage("BSim compare " + t + "/" + targets.size());
                monitor.setProgress(t);
            }
            TargetFunction target = targets.get(t);
            int bestNameId = UNSET;
            double secondDifferent = UNSET;
            for (int s = 0; s < sourceFunctions.size(); s++) {
                SourceFunction source = sourceFunctions.get(s);
                double sim = target.vector.compare(source.vector, compare);
                if (sim > target.bestSim) {
                    if (bestNameId != UNSET && bestNameId != sourceNameIds[s]) {
                        secondDifferent = Math.max(secondDifferent, target.bestSim);
                    }
                    target.bestSim = sim;
                    target.bestSig = factory.calculateSignificance(compare);
                    target.best = s;
                    bestNameId = sourceNameIds[s];
                } else if (sourceNameIds[s] != bestNameId) {
                    secondDifferent = Math.max(secondDifferent, sim);
                }
                if (sim > source.bestSim) {
                    source.secondSim = source.bestSim;
                    source.bestSim = sim;
                    source.bestTarget = t;
                } else if (sim > source.secondSim) {
                    source.secondSim = sim;
                }
            }
            target.secondDifferentSim = secondDifferent;
        }

        int accepted = 0;
        Map<String, Integer> reasons = new TreeMap<>();
        for (int t = 0; t < targets.size(); t++) {
            TargetFunction target = targets.get(t);
            String reason = rejection(targets, sourceFunctions, t, settings.minSim, settings.minSig);
            if (reason == null) {
                SourceFunction source = sourceFunctions.get(target.best);
                // Different names with near-identical vectors cannot be told apart.
                for (SourceFunction other : sourceFunctions) {
                    if (!other.name.equals(source.name)
                            && source.vector.compare(other.vector, compare) >= DUPLICATE_SOURCE_SIMILARITY) {
                        reason = "source duplicated under another name (`" + other.name + "`)";
                        break;
                    }
                }
            }
            long address = target.function.getEntryPoint().getOffset();
            if (reason != null) {
                if (target.best != UNSET && target.bestSim >= settings.minSim) {
                    rejected.put(address, "`" + sourceFunctions.get(target.best).name + "` sim="
                        + format(target.bestSim) + " sig=" + format(target.bestSig) + ": " + reason);
                }
                reasons.merge(reason.replaceAll(" \\(`.*", ""), 1, Integer::sum);
                continue;
            }
            SourceFunction source = sourceFunctions.get(target.best);
            Proposal proposal = new Proposal();
            proposal.name = source.name;
            proposal.alias = source.alias;
            proposal.prototype = prototype(source.program, source.function.getEntryPoint().getOffset(), source.alias);
            proposal.source = source.function;
            proposal.evidence = "bsim sim=" + format(target.bestSim) + " sig=" + format(target.bestSig) + " "
                + source.program.getName() + " " + source.function.getEntryPoint();
            names.put(address, proposal);
            accepted++;
        }
        for (var entry : reasons.entrySet()) {
            stats.put("Rejected: " + entry.getKey(), entry.getValue());
        }
        stats.put("Accepted BSim names", accepted);
    }

    /** Named UOSA callees of a BSim target, and how many of them the 2.6 function (or a folded alias) calls. */
    private int[] callgraphAgreement(long address, Function source) {
        Set<String> sourceCallees = new HashSet<>();
        Program sourceProgram = source.getProgram();
        for (Function callee : source.getCalledFunctions(monitor)) {
            sourceCallees.add(callee.getName(true));
            for (Alias alias : aliases(sourceProgram, callee.getEntryPoint().getOffset())) {
                sourceCallees.add(alias.full());
            }
            Function thunked = callee.isThunk() ? callee.getThunkedFunction(true) : null;
            if (thunked != null) {
                sourceCallees.add(thunked.getName(true));
            }
        }
        int checkable = 0;
        int agree = 0;
        for (Function callee : getFunctionAt(toAddr(address)).getCalledFunctions(monitor)) {
            Function resolved = callee.isThunk() && callee.getThunkedFunction(true) != null
                ? callee.getThunkedFunction(true) : callee;
            String name = known.get(callee.getEntryPoint().getOffset());
            if (name == null) {
                name = known.get(resolved.getEntryPoint().getOffset());
            }
            if (name == null) {
                continue;
            }
            checkable++;
            agree += sourceCallees.contains(name) ? 1 : 0;
        }
        return new int[] { checkable, agree };
    }

    /**
     * Why a BSim pair contradicts the vtables, or null: the 2.6 function is a vtable slot implemented by
     * class S, the UOSA function is a slot only of Gamebryo vtables, and none of their classes is S or
     * derives from S. Pairs without vtable evidence on either side pass.
     */
    private String classCheck(long address, Alias alias, Function source) {
        Set<String> classes = slotClassesU.get(address);
        if (classes == null || foreignSlotsU.contains(address)
                || !slots26.getOrDefault(source.getProgram(), Set.of()).contains(source.getEntryPoint().getOffset())) {
            return null;
        }
        for (String name : classes) {
            if (chainsU.getOrDefault(name, List.of()).contains(alias.ns())) {
                return null;
            }
        }
        List<String> sample = new ArrayList<>(new TreeSet<>(classes));
        return "2.6 virtual of `" + alias.ns() + "`, UOSA slot of " + classes.size() + " classes not derived from it ("
            + String.join(", ", sample.subList(0, Math.min(sample.size(), MAX_EXAMPLES))) + ")";
    }

    /** Why a target's best match is not accepted under the given thresholds, or null. */
    private String rejection(List<TargetFunction> targets, List<SourceFunction> sources, int t, double minSim,
            double minSig) {
        TargetFunction target = targets.get(t);
        if (target.best == UNSET) {
            return "no candidate";
        }
        if (target.bestSim < minSim) {
            return "similarity below threshold";
        }
        if (target.bestSig < minSig) {
            return "significance below threshold";
        }
        SourceFunction source = sources.get(target.best);
        if (source.bestTarget != t) {
            return "not mutual best";
        }
        if (target.bestSim - target.secondDifferentSim < settings.margin) {
            return "second-best source name within margin";
        }
        if (source.bestSim - source.secondSim < settings.margin) {
            return "second-best target within margin";
        }
        return null;
    }

    private Map<Long, LSHVector> signatures(Program program, List<Function> functions, LSHVectorFactory factory)
            throws Exception {
        Map<Long, LSHVector> vectors = new HashMap<>();
        if (functions.isEmpty()) {
            return vectors;
        }
        GenSignatures generator = new GenSignatures(false);
        try {
            generator.setVectorFactory(factory);
            generator.openProgram(program, null, null, null, null, null);
            generator.scanFunctions(functions.iterator(), functions.size(), monitor);
            var iterator = generator.getDescriptionManager().listAllFunctions();
            while (iterator.hasNext()) {
                FunctionDescription description = iterator.next();
                if (description.getSignatureRecord() != null) {
                    vectors.put(description.getAddress(), description.getSignatureRecord().getLSHVector());
                }
            }
        } finally {
            generator.dispose();
        }
        return vectors;
    }

    // ---------------------------------------------------------------- names and apply

    /** Distinct 2.6 names at an address (PDB function names and ICF-folded labels; no mangled or ordinal ones). */
    private List<Alias> aliases(Program program, long address) {
        Map<Long, List<Alias>> cache = aliasCache.computeIfAbsent(program, key -> new HashMap<>());
        List<Alias> cached = cache.get(address);
        if (cached != null) {
            return cached;
        }
        Map<String, Alias> result = new LinkedHashMap<>();
        for (Symbol symbol : program.getSymbolTable().getSymbols(addr(program, address))) {
            String name = symbol.getName();
            SymbolType type = symbol.getSymbolType();
            if (symbol.getSource() == SourceType.DEFAULT || (type != SymbolType.FUNCTION && type != SymbolType.LABEL)
                    || name.startsWith(MANGLED_PREFIX) || name.startsWith(ORDINAL_PREFIX)
                    || name.startsWith(IMPORT_PREFIX)) {
                continue;
            }
            List<NsElem> path = new ArrayList<>();
            for (Namespace ns = symbol.getParentNamespace(); ns != null && !ns.isGlobal(); ns = ns.getParentNamespace()) {
                path.add(0, new NsElem(ns.getName(), ns.getSymbol().getSymbolType() == SymbolType.CLASS));
            }
            String nsName = symbol.getParentNamespace().isGlobal() ? "" : symbol.getParentNamespace().getName(true);
            String full = nsName.isEmpty() ? name : nsName + SCOPE + name;
            result.putIfAbsent(full, new Alias(path, nsName, name, full));
        }
        List<Alias> list = new ArrayList<>(result.values());
        cache.put(address, list);
        return list;
    }

    private static String prototype(Program program, long address, Alias alias) {
        Function function = program.getFunctionManager().getFunctionAt(addr(program, address));
        if (function == null) {
            return "";
        }
        String own = function.getName(true);
        String signature = function.getSignature().getPrototypeString(true);
        return own.equals(alias.full()) ? signature : "2.6 code folded (ICF) with `" + own + "`: " + signature;
    }

    private boolean isOurs(Function function) {
        return function.getTags().stream().anyMatch(tag -> tag.getName().equals(TAG_VTABLE)
            || tag.getName().equals(TAG_BSIM));
    }

    private void ensureTag(FunctionTagManager tags, String name, String comment) {
        if (tags.getFunctionTag(name) == null) {
            tags.createFunctionTag(name, comment);
            count("function tag created");
        }
    }

    private String apply(long address, Proposal proposal, String tag) throws Exception {
        Function function = getFunctionAt(toAddr(address));
        String status;
        if (function.getSymbol().getSource() == SourceType.DEFAULT) {
            Namespace namespace = currentProgram.getGlobalNamespace();
            SymbolTable symbols = currentProgram.getSymbolTable();
            for (NsElem elem : proposal.alias.path()) {
                Namespace next = symbols.getNamespace(elem.name(), namespace);
                if (next == null) {
                    next = elem.isClass() ? symbols.createClass(namespace, elem.name(), SourceType.ANALYSIS)
                        : symbols.createNameSpace(namespace, elem.name(), SourceType.ANALYSIS);
                    count(elem.isClass() ? "class namespace created" : "namespace created");
                }
                namespace = next;
            }
            function.getSymbol().setNameAndNamespace(proposal.alias.method(), namespace, SourceType.ANALYSIS);
            count("function renamed (" + tag + ")");
            status = "renamed";
        } else if (isOurs(function) && function.getName(true).equals(proposal.name)) {
            status = "unchanged";
        } else {
            return function.getName(true).equals(proposal.name) ? "kept (already named)" : "kept (existing name)";
        }
        if (function.getTags().stream().noneMatch(t -> t.getName().equals(tag))) {
            function.addTag(tag);
            count("function tag added");
        }
        String plate = function.getComment();
        if (plate == null || plate.isBlank()) {
            function.setComment("Gamebryo 2.6: " + proposal.name + "\n" + proposal.prototype + "\n" + proposal.evidence);
            count("plate comment added");
        }
        return status;
    }

    // ---------------------------------------------------------------- report

    private void writeReport(List<Program> sources, Map<Long, Proposal> vtableNames, Map<Long, List<String>> vtableConflicts,
            List<String> vtableClasses, Map<String, Integer> vtableStats, Map<Long, Proposal> bsimNames,
            Map<Long, String> bsimRejected, Map<String, Integer> bsimStats, List<TargetFunction> targets,
            List<SourceFunction> sourceFunctions, Map<Long, Proposal> finalNames, int sourceAgreements,
            List<String> sourceDisagreements, List<String> collisions, Map<Long, String> anchors,
            List<String> anchorAgreementsVtable, List<String> anchorDisagreementsVtable, List<String> anchorAgreementsBsim,
            List<String> anchorDisagreementsBsim, int existingOther, Map<Long, String> calibration) throws Exception {
        try (PrintWriter output = new PrintWriter(Files.newBufferedWriter(Path.of(settings.output),
                StandardOpenOption.CREATE_NEW))) {
            output.println("# Gamebryo 2.6 names in UOSA");
            output.println();
            output.println("Program: " + currentProgram.getName());
            output.println("SHA-256: " + currentProgram.getExecutableSHA256());
            output.println("Mode: " + (settings.apply ? "analysis and " + APPLY_FLAG : "analysis only"));
            StringBuilder sourceLine = new StringBuilder();
            for (Program source : sources) {
                sourceLine.append(sourceLine.length() == 0 ? "" : ", ").append("`").append(source.getName()).append("` (")
                    .append(source.getFunctionManager().getFunctionCount()).append(" functions)");
            }
            output.println("Sources: " + sourceLine);
            StringBuilder rangeLine = new StringBuilder();
            for (long[] range : settings.ranges) {
                rangeLine.append(rangeLine.length() == 0 ? "" : ", ").append(hex(range[0])).append("-").append(hex(range[1]));
            }
            output.println("BSim target ranges: " + rangeLine);
            output.println("Thresholds: similarity >= " + settings.minSim + ", significance >= " + settings.minSig
                + ", margin " + settings.margin + ", self-significance > " + SELF_SIGNIFICANCE_BOUND
                + ", duplicate-source similarity " + DUPLICATE_SOURCE_SIMILARITY);
            output.println();
            output.println("| Item | Count |");
            output.println("|---|---:|");
            for (var entry : vtableStats.entrySet()) {
                output.println("| Vtable: " + entry.getKey() + " | " + entry.getValue() + " |");
            }
            for (var entry : bsimStats.entrySet()) {
                output.println("| BSim: " + entry.getKey() + " | " + entry.getValue() + " |");
            }
            int vtableFinal = 0;
            for (var entry : finalNames.entrySet()) {
                if (vtableNames.get(entry.getKey()) == entry.getValue()) {
                    vtableFinal++;
                }
            }
            output.println("| Vtable and BSim agree | " + sourceAgreements + " |");
            output.println("| Vtable and BSim disagree (vtable kept) | " + sourceDisagreements.size() + " |");
            output.println("| BSim names a vtable gives to another function (dropped) | " + collisions.size() + " |");
            output.println("| BSim vtable-class check: 2.6 virtual of an unrelated class (dropped) | "
                + classCheckRejected.size() + " |");
            for (var entry : callgraphStats.entrySet()) {
                output.println("| BSim call-graph check: " + entry.getKey() + " | " + entry.getValue() + " |");
            }
            output.println("| Final names from vtables | " + vtableFinal + " |");
            output.println("| Final names from BSim | " + (finalNames.size() - vtableFinal) + " |");
            output.println("| Existing names that also occur in 2.6 (anchors) | " + anchors.size() + " |");
            output.println("| Anchors agreeing with the vtable name | " + anchorAgreementsVtable.size() + " |");
            output.println("| Anchors disagreeing with the vtable name | " + anchorDisagreementsVtable.size() + " |");
            output.println("| Anchors agreeing with the BSim name | " + anchorAgreementsBsim.size() + " |");
            output.println("| Anchors disagreeing with the BSim name | " + anchorDisagreementsBsim.size() + " |");
            output.println("| Other existing names with a transferred name (kept) | " + existingOther + " |");
            output.println();
            if (!changes.isEmpty()) {
                output.println("Changes:");
                output.println();
                for (var entry : changes.entrySet()) {
                    output.println("- " + entry.getKey() + ": " + entry.getValue());
                }
                output.println();
            } else if (settings.apply) {
                output.println("Changes: none");
                output.println();
            }

            if (!targets.isEmpty()) {
                writeCalibration(output, targets, sourceFunctions, calibration);
            }
            examples(output, "Vtable conflicts (not named)", conflictLines(vtableConflicts), vtableConflicts.size());
            examples(output, "Vtable and BSim disagreements", sourceDisagreements, sourceDisagreements.size());
            examples(output, "BSim names dropped for a vtable collision", collisions, collisions.size());
            examples(output, "BSim names dropped by the vtable-class check", classCheckRejected, classCheckRejected.size());
            examples(output, "BSim names dropped by the call-graph check", callgraphRejected, callgraphRejected.size());
            examples(output, "Anchor disagreements (vtable)", anchorDisagreementsVtable, anchorDisagreementsVtable.size());
            examples(output, "Anchor disagreements (BSim)", anchorDisagreementsBsim, anchorDisagreementsBsim.size());
            examples(output, "Notes", notes, notes.size());
        }
    }

    private void writeCalibration(PrintWriter output, List<TargetFunction> targets, List<SourceFunction> sources,
            Map<Long, String> calibration) {
        Map<Long, Integer> index = new HashMap<>();
        for (int t = 0; t < targets.size(); t++) {
            index.put(targets.get(t).function.getEntryPoint().getOffset(), t);
        }
        // Anchor targets whose best match passes mutual best and both margins at the calibration floor.
        List<Integer> candidates = new ArrayList<>();
        List<Boolean> agrees = new ArrayList<>();
        // Why the vtable-class or call-graph check would drop the pair, or null if it passes both.
        List<String> dropped = new ArrayList<>();
        for (var entry : calibration.entrySet()) {
            Integer t = index.get(entry.getKey());
            if (t != null && rejection(targets, sources, t, CALIBRATION_BUCKETS[0], SIGNIFICANCE_BUCKETS[0]) == null) {
                candidates.add(t);
                SourceFunction source = sources.get(targets.get(t).best);
                agrees.add(source.name.equals(entry.getValue()));
                String reason = classCheck(entry.getKey(), source.alias, source.function);
                if (reason == null) {
                    int[] callees = callgraphAgreement(entry.getKey(), source.function);
                    reason = callees[0] > 0 && callees[1] == 0 ? "no named callee agrees" : null;
                }
                dropped.add(reason);
            }
        }
        output.println("## Calibration");
        output.println();
        output.println("Anchors: existing names that occur in 2.6 plus vtable names, restricted to BSim targets ("
            + count(calibration.keySet(), index::containsKey) + "); " + candidates.size() + " have a best match that"
            + " is mutual and passes both margins at similarity >= " + CALIBRATION_BUCKETS[0] + ". `Agree` means the"
            + " best source has the anchor's name. Cumulative columns count the bucket and all higher ones. `Checked`"
            + " columns keep only the pairs that also pass the vtable-class and call-graph checks, as accepted names"
            + " must.");
        output.println();
        output.println("By similarity (all significances | significance >= " + settings.minSig + "):");
        output.println();
        output.println("| Similarity | Pairs | Agree | Pairs (sig) | Agree (sig) | Cumulative (sig) | Cumulative rate (sig)"
            + " | Checked (sig) | Agree checked (sig) | Cumulative rate checked (sig) |");
        output.println("|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|");
        int buckets = CALIBRATION_BUCKETS.length;
        int[][] table = new int[buckets][CALIBRATION_COLUMNS];
        for (int i = 0; i < candidates.size(); i++) {
            TargetFunction target = targets.get(candidates.get(i));
            int bucket = bucketOf(CALIBRATION_BUCKETS, target.bestSim);
            table[bucket][0]++;
            table[bucket][1] += agrees.get(i) ? 1 : 0;
            if (target.bestSig >= settings.minSig) {
                table[bucket][2]++;
                table[bucket][3] += agrees.get(i) ? 1 : 0;
                if (dropped.get(i) == null) {
                    table[bucket][4]++;
                    table[bucket][5] += agrees.get(i) ? 1 : 0;
                }
            }
        }
        String[] lines = new String[buckets];
        int cumulativePairs = 0;
        int cumulativeAgree = 0;
        int cumulativeChecked = 0;
        int cumulativeAgreeChecked = 0;
        double suggested = UNSET;
        double suggestedChecked = UNSET;
        for (int bucket = buckets - 1; bucket >= 0; bucket--) {
            cumulativePairs += table[bucket][2];
            cumulativeAgree += table[bucket][3];
            cumulativeChecked += table[bucket][4];
            cumulativeAgreeChecked += table[bucket][5];
            double cumulativeRate = rate(cumulativeAgree, cumulativePairs);
            double cumulativeRateChecked = rate(cumulativeAgreeChecked, cumulativeChecked);
            if (cumulativePairs > 0 && cumulativeRate >= CALIBRATION_TARGET_AGREEMENT * PERCENT) {
                suggested = CALIBRATION_BUCKETS[bucket];
            }
            if (cumulativeChecked > 0 && cumulativeRateChecked >= CALIBRATION_TARGET_AGREEMENT * PERCENT) {
                suggestedChecked = CALIBRATION_BUCKETS[bucket];
            }
            lines[bucket] = "| >= " + CALIBRATION_BUCKETS[bucket] + " | " + table[bucket][0] + " | " + table[bucket][1]
                + " | " + table[bucket][2] + " | " + table[bucket][3] + " | " + cumulativePairs + " | "
                + format(cumulativeRate) + "% | " + table[bucket][4] + " | " + table[bucket][5] + " | "
                + format(cumulativeRateChecked) + "% |";
        }
        for (int bucket = buckets - 1; bucket >= 0; bucket--) {
            output.println(lines[bucket]);
        }
        output.println();
        output.println("Lowest similarity bucket with cumulative agreement >= " + format(CALIBRATION_TARGET_AGREEMENT
            * PERCENT) + "% at significance >= " + settings.minSig + ": "
            + (suggested == UNSET ? "none" : String.valueOf(suggested)) + "; checked pairs only: "
            + (suggestedChecked == UNSET ? "none" : String.valueOf(suggestedChecked)));
        output.println();
        output.println("By significance (similarity >= " + CALIBRATION_BUCKETS[0] + " | similarity >= " + settings.minSim
            + "):");
        output.println();
        output.println("| Significance | Pairs | Agree | Pairs (sim) | Agree (sim) |");
        output.println("|---|---:|---:|---:|---:|");
        int sigBuckets = SIGNIFICANCE_BUCKETS.length;
        int[][] sigTable = new int[sigBuckets][4];
        for (int i = 0; i < candidates.size(); i++) {
            TargetFunction target = targets.get(candidates.get(i));
            int bucket = bucketOf(SIGNIFICANCE_BUCKETS, target.bestSig);
            sigTable[bucket][0]++;
            sigTable[bucket][1] += agrees.get(i) ? 1 : 0;
            if (target.bestSim >= settings.minSim) {
                sigTable[bucket][2]++;
                sigTable[bucket][3] += agrees.get(i) ? 1 : 0;
            }
        }
        for (int bucket = sigBuckets - 1; bucket >= 0; bucket--) {
            output.println("| >= " + SIGNIFICANCE_BUCKETS[bucket] + " | " + sigTable[bucket][0] + " | "
                + sigTable[bucket][1] + " | " + sigTable[bucket][2] + " | " + sigTable[bucket][3] + " |");
        }
        output.println();
        List<String> wrong = new ArrayList<>();
        List<String> lost = new ArrayList<>();
        for (int i = 0; i < candidates.size(); i++) {
            if (agrees.get(i) && dropped.get(i) != null) {
                TargetFunction target = targets.get(candidates.get(i));
                lost.add(hex(target.function.getEntryPoint().getOffset()) + " `" + sources.get(target.best).name
                    + "` sim=" + format(target.bestSim) + " sig=" + format(target.bestSig) + ": " + dropped.get(i));
            }
            if (!agrees.get(i)) {
                TargetFunction target = targets.get(candidates.get(i));
                long address = target.function.getEntryPoint().getOffset();
                wrong.add(hex(address) + " anchor `" + calibration.get(address) + "`, best `"
                    + sources.get(target.best).name + "` sim=" + format(target.bestSim) + " sig="
                    + format(target.bestSig) + (dropped.get(i) != null ? "; dropped by the checks: " + dropped.get(i)
                    : "; passes the checks"));
            }
        }
        examples(output, "Calibration disagreements", wrong, wrong.size());
        examples(output, "Calibration agreements the checks would drop", lost, lost.size());
    }

    private static int bucketOf(double[] bounds, double value) {
        int bucket = bounds.length - 1;
        while (bucket > 0 && value < bounds[bucket]) {
            bucket--;
        }
        return bucket;
    }

    private static List<String> conflictLines(Map<Long, List<String>> conflicts) {
        List<String> lines = new ArrayList<>();
        for (var entry : conflicts.entrySet()) {
            lines.add(hex(entry.getKey()) + ": " + String.join("; ", entry.getValue()));
        }
        return lines;
    }

    private static void examples(PrintWriter output, String title, List<String> lines, int total) {
        if (lines.isEmpty()) {
            return;
        }
        output.println("## " + title);
        output.println();
        output.println(total + " in total" + (total > MAX_EXAMPLES ? "; first " + MAX_EXAMPLES + ":" : ":"));
        output.println();
        for (int i = 0; i < lines.size() && i < MAX_EXAMPLES; i++) {
            output.println("- " + lines.get(i));
        }
        output.println();
    }

    private void writeMatches(Map<Long, Proposal> vtableNames, Map<Long, List<String>> vtableConflicts,
            Map<Long, Proposal> bsimNames, Map<Long, String> bsimRejected, Map<Long, Proposal> finalNames,
            Map<Long, String> applied, List<String> sourceDisagreements, List<String> collisions,
            List<String> anchorDisagreementsVtable, List<String> anchorDisagreementsBsim) throws Exception {
        try (PrintWriter output = new PrintWriter(Files.newBufferedWriter(Path.of(settings.matches),
                StandardOpenOption.CREATE_NEW))) {
            output.println("# address\tcurrent name\ttransferred name\tsource\tapply status\tevidence\tprototype");
            for (var entry : finalNames.entrySet()) {
                Function function = getFunctionAt(toAddr(entry.getKey()));
                Proposal proposal = entry.getValue();
                output.println(hex(entry.getKey()) + "\t" + function.getName(true) + "\t" + proposal.name + "\t"
                    + (vtableNames.get(entry.getKey()) == proposal ? TAG_VTABLE : TAG_BSIM) + "\t"
                    + applied.getOrDefault(entry.getKey(), "-") + "\t" + proposal.evidence + "\t"
                    + proposal.prototype.replace('\n', ' '));
            }
            section(output, "vtable conflicts", conflictLines(vtableConflicts));
            section(output, "vtable slots whose UOSA target is not a function", new ArrayList<>(missingFunctions));
            section(output, "vtable classes", classLines);
            section(output, "vtable/BSim disagreements", sourceDisagreements);
            section(output, "BSim names dropped for vtable collisions", collisions);
            section(output, "BSim names dropped by the vtable-class check", classCheckRejected);
            section(output, "BSim names dropped by the call-graph check", callgraphRejected);
            section(output, "anchor disagreements (vtable)", anchorDisagreementsVtable);
            section(output, "anchor disagreements (BSim)", anchorDisagreementsBsim);
            List<String> rejectedLines = new ArrayList<>();
            for (var entry : bsimRejected.entrySet()) {
                rejectedLines.add(hex(entry.getKey()) + "\t" + entry.getValue());
            }
            section(output, "BSim candidates above the similarity threshold that were rejected", rejectedLines);
        }
    }

    private static void section(PrintWriter output, String title, List<String> lines) {
        output.println();
        output.println("# " + title + ": " + lines.size());
        for (String line : lines) {
            output.println(line);
        }
    }

    // ---------------------------------------------------------------- helpers

    private void count(String change) {
        changes.merge(change, 1, Integer::sum);
    }

    private static <T> int count(Iterable<T> items, java.util.function.Predicate<T> predicate) {
        int result = 0;
        for (T item : items) {
            if (predicate.test(item)) {
                result++;
            }
        }
        return result;
    }

    private static Address addr(Program program, long offset) {
        return program.getAddressFactory().getDefaultAddressSpace().getAddress(offset);
    }

    private static long u32(Program program, long offset) throws MemoryAccessException {
        return program.getMemory().getInt(addr(program, offset)) & DWORD_MASK;
    }

    private static double rate(int part, int total) {
        return total == 0 ? 0 : PERCENT * part / total;
    }

    private static String format(double value) {
        return String.format("%.3f", value);
    }

    private static String hex(long value) {
        return String.format("0x%08x", value);
    }
}
