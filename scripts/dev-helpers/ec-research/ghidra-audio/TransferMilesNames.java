// Transfer the function names of the Miles 7.2a SDK binaries (analysed with their PDBs in the project folder
// /miles-7.2a-sdk) to the Miles binaries the clients ship, by instruction comparison. The names come from the
// user's own SDK copy at run time: neither the names of internal functions nor anything else from the SDK is stored
// in this script. The report and the match list go outside the repository.
// Tiers, each only among the functions still unmatched and only when the key is unique on both sides:
//   1 strict: mnemonics, registers and constants, with addresses masked
//   2 normalized: mnemonics and registers, with every constant and address masked
//   3 mnemonics only, for bodies of at least MIN_MNEMONIC_INSTRUCTIONS
//   4 call position: the same call slot of an already matched pair, when the two bodies have the same normalized or
//     mnemonic key, or a mnemonic similarity of at least MIN_CALL_SIMILARITY
// Exported names shared by both sides are the seeds of tier 4 and the calibration of tiers 1 to 3.
// A pair that references strings on both sides with none in common is refused.
// @category ECResearch
import ghidra.app.script.GhidraScript;
import ghidra.framework.model.DomainFile;
import ghidra.program.model.address.Address;
import ghidra.program.model.data.StringDataInstance;
import ghidra.program.model.listing.Data;
import ghidra.program.model.listing.Function;
import ghidra.program.model.listing.FunctionTag;
import ghidra.program.model.listing.Instruction;
import ghidra.program.model.listing.Program;
import ghidra.program.model.mem.Memory;
import ghidra.program.model.scalar.Scalar;
import ghidra.program.model.symbol.Namespace;
import ghidra.program.model.symbol.Reference;
import ghidra.program.model.symbol.SourceType;
import ghidra.program.model.symbol.Symbol;
import java.io.PrintWriter;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardOpenOption;
import java.util.ArrayDeque;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.HashSet;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.TreeMap;

public class TransferMilesNames extends GhidraScript {
    private static final String APPLY_FLAG = "--apply";
    private static final String SOURCE_FLAG = "--source";
    private static final String NAMESPACE = "MilesSdk";
    private static final String TAG = "MILES_SDK_NAME";
    private static final String TAG_COMMENT = "Name transferred from the Miles 7.2a SDK by instruction comparison";
    private static final String COMMENT_PREFIX = "TransferMilesNames: tier ";
    private static final String DEFAULT_NAME_PREFIX = "FUN_";
    private static final String THUNK_DEFAULT_PREFIX = "thunk_FUN_";
    private static final String SCOPE = "::";
    private static final String FLAT_SCOPE = "__";
    private static final String CALL_FLOW = "CALL";
    private static final String MASK = "#";
    private static final int MIN_NORMALIZED_INSTRUCTIONS = 6;
    private static final int MIN_MNEMONIC_INSTRUCTIONS = 16;
    private static final int MIN_CALL_INSTRUCTIONS = 4;
    private static final double MIN_CALL_SIMILARITY = 0.85;
    private static final int TIER_STRICT = 1;
    private static final int TIER_NORMALIZED = 2;
    private static final int TIER_MNEMONIC = 3;
    private static final int TIER_CALL = 4;
    private static final int TIER_SEED = 0;

    private static final Map<String, String> SOURCES = Map.of(
        "b588b4791ff6803247962c661b6d3ff9a8625d0f60363a6fa53f61d401f0e74e", "/miles-7.2a-sdk/mss32.dll",
        "0fb784a9cc9826a9ec2412b7d71453b917af6c74442647f38061512e2c3a05bb", "/miles-7.2a-sdk/mssmp3.asi",
        "272f65a0b4cb84aef765ee80547341de1b1dd9d16907add2d784cd5ad04d155a", "/miles-7.2a-sdk/mss32.dll",
        "f6bbeccf925b1f2b3132143b5a289ea7041554e30d739560b9d5eadf1a427bdd", "/miles-7.2a-sdk/mssmp3.asi");

    private static final class Info {
        Function function;
        String strict;
        String normalized;
        String mnemonics;
        int instructions;
        Set<String> strings = new HashSet<>();
        List<Address> calls = new ArrayList<>();
    }

    private static final class Match {
        Info source;
        Info target;
        int tier;
        Match(Info source, Info target, int tier) {
            this.source = source;
            this.target = target;
            this.tier = tier;
        }
    }

    private final Map<Address, Match> byTarget = new TreeMap<>();
    private final Map<Address, Match> bySource = new TreeMap<>();
    private final int[] counts = new int[TIER_CALL + 1];
    private int stringRefusals;

    @Override
    public void run() throws Exception {
        String[] args = getScriptArgs();
        boolean apply = false;
        String output = null;
        String sourcePath = SOURCES.get(currentProgram.getExecutableSHA256());
        for (int i = 0; i < args.length; i++) {
            if (args[i].equals(APPLY_FLAG)) {
                apply = true;
            }
            else if (args[i].equals(SOURCE_FLAG)) {
                sourcePath = args[++i];
            }
            else {
                output = args[i];
            }
        }
        if (output == null || sourcePath == null) {
            throw new IllegalArgumentException("usage: OUTPUT_FILE [--apply]; this program's SHA-256 is not in the"
                + " table: " + currentProgram.getExecutableSHA256());
        }
        DomainFile file = state.getProject().getProjectData().getFile(sourcePath);
        Program source = (Program) file.getReadOnlyDomainObject(this, DomainFile.DEFAULT_VERSION, monitor);
        try (PrintWriter out = new PrintWriter(Files.newBufferedWriter(Path.of(output),
                StandardOpenOption.CREATE_NEW, StandardOpenOption.WRITE))) {
            transfer(source, out, apply);
        }
        finally {
            source.release(this);
        }
    }

    private void transfer(Program source, PrintWriter out, boolean apply) throws Exception {
        Map<Address, Info> sourceInfo = collect(source);
        Map<Address, Info> targetInfo = collect(currentProgram);
        int sourceNamed = 0;
        for (Info info : sourceInfo.values()) {
            if (isNamed(info.function)) {
                sourceNamed++;
            }
        }
        seedExports(sourceInfo, targetInfo);
        int calibrated = 0;
        int calibratedWrong = 0;
        // Calibration: the seeds are set aside, then the key tiers must find the same pairs on their own.
        Map<Address, Match> seeds = new TreeMap<>(byTarget);
        byTarget.clear();
        bySource.clear();
        matchByKey(sourceInfo, targetInfo, TIER_STRICT);
        matchByKey(sourceInfo, targetInfo, TIER_NORMALIZED);
        matchByKey(sourceInfo, targetInfo, TIER_MNEMONIC);
        for (Map.Entry<Address, Match> seed : seeds.entrySet()) {
            Match found = byTarget.get(seed.getKey());
            if (found != null) {
                if (found.source.function.getEntryPoint().equals(seed.getValue().source.function.getEntryPoint())) {
                    calibrated++;
                }
                else {
                    calibratedWrong++;
                    out.println("calibration conflict at " + seed.getKey() + ": export says "
                        + seed.getValue().source.function.getName() + ", key tier says "
                        + found.source.function.getName());
                }
            }
        }
        // Exports win over key tiers, and they anchor the call tier.
        for (Match seed : seeds.values()) {
            Match oldTarget = byTarget.remove(seed.target.function.getEntryPoint());
            if (oldTarget != null) {
                bySource.remove(oldTarget.source.function.getEntryPoint());
            }
            Match oldSource = bySource.remove(seed.source.function.getEntryPoint());
            if (oldSource != null) {
                byTarget.remove(oldSource.target.function.getEntryPoint());
            }
            record(seed);
        }
        matchByCalls(sourceInfo, targetInfo);
        java.util.Arrays.fill(counts, 0);
        for (Match m : byTarget.values()) {
            counts[m.tier]++;
        }
        out.println("# TransferMilesNames report");
        out.println();
        out.println("source " + source.getDomainFile().getPathname() + " functions " + sourceInfo.size()
            + " with a name " + sourceNamed);
        out.println("target " + currentProgram.getDomainFile().getPathname() + " sha256 "
            + currentProgram.getExecutableSHA256() + " functions " + targetInfo.size());
        out.println("seeds (same export on both sides) " + seeds.size() + "; found again by tiers 1-3 " + calibrated
            + ", in conflict " + calibratedWrong);
        out.println("pairs refused for disjoint strings " + stringRefusals);
        out.println("tier 0 seed " + counts[TIER_SEED] + ", 1 strict " + counts[TIER_STRICT] + ", 2 normalized "
            + counts[TIER_NORMALIZED] + ", 3 mnemonic " + counts[TIER_MNEMONIC] + ", 4 call position "
            + counts[TIER_CALL]);
        int nameable = 0;
        int renamed = 0;
        int kept = 0;
        int noName = 0;
        int transactionChanges = 0;
        Namespace ns = apply ? currentProgram.getSymbolTable().getOrCreateNameSpace(
            currentProgram.getGlobalNamespace(), NAMESPACE, SourceType.USER_DEFINED) : null;
        out.println();
        out.println("target\tsource\ttier\tsource name\tstate");
        for (Match m : byTarget.values()) {
            Function s = m.source.function;
            Function t = m.target.function;
            String state;
            if (!isNamed(s)) {
                noName++;
                state = "source has no name";
            }
            else {
                nameable++;
                String name = cleanName(s);
                if (t.getName().equals(name)) {
                    state = "already";
                }
                else if (isDefault(t)) {
                    state = "renamed";
                    renamed++;
                    if (apply) {
                        t.setParentNamespace(ns);
                        t.setName(name, SourceType.USER_DEFINED);
                        FunctionTag tag = currentProgram.getFunctionManager().getFunctionTagManager()
                            .getFunctionTag(TAG);
                        if (tag == null) {
                            currentProgram.getFunctionManager().getFunctionTagManager().createFunctionTag(TAG,
                                TAG_COMMENT);
                        }
                        t.addTag(TAG);
                        if (t.getComment() == null) {
                            t.setComment(COMMENT_PREFIX + m.tier);
                        }
                        transactionChanges++;
                    }
                }
                else {
                    state = "kept " + t.getName(true);
                    kept++;
                }
            }
            out.println(t.getEntryPoint() + "\t" + s.getEntryPoint() + "\t" + m.tier + "\t" + cleanName(s) + "\t"
                + state);
        }
        String summary = "matched " + byTarget.size() + " of " + targetInfo.size() + " target functions; with a source"
            + " name " + nameable + "; to rename " + renamed + "; kept (already named otherwise) " + kept
            + "; source has no name " + noName + "; changes: " + (renamed == 0 ? "none" : "" + renamed)
            + (apply ? " (applied " + transactionChanges + ")" : " (report only)");
        out.println();
        out.println(summary);
        println(summary);
        println("seeds " + seeds.size() + " found again " + calibrated + " conflicts " + calibratedWrong
            + " string refusals " + stringRefusals + " tiers 0/1/2/3/4 " + counts[TIER_SEED] + "/"
            + counts[TIER_STRICT] + "/" + counts[TIER_NORMALIZED] + "/" + counts[TIER_MNEMONIC] + "/"
            + counts[TIER_CALL] + "; source functions " + sourceInfo.size() + " named " + sourceNamed);
    }

    private static boolean isDefault(Function f) {
        return f.getSymbol().getSource() == SourceType.DEFAULT || f.getName().startsWith(DEFAULT_NAME_PREFIX)
            || f.getName().startsWith(THUNK_DEFAULT_PREFIX);
    }

    private static boolean isNamed(Function f) {
        return !isDefault(f);
    }

    private static String cleanName(Function f) {
        return f.getName(true).replace(SCOPE, FLAT_SCOPE).replace(' ', '_');
    }

    private Map<Address, Info> collect(Program program) throws Exception {
        Map<Address, Info> result = new LinkedHashMap<>();
        Memory memory = program.getMemory();
        long imageLow = memory.getMinAddress().getOffset();
        long imageHigh = memory.getMaxAddress().getOffset();
        for (Function f : program.getFunctionManager().getFunctions(true)) {
            if (f.isThunk() || f.isExternal()) {
                continue;
            }
            Info info = new Info();
            info.function = f;
            StringBuilder strict = new StringBuilder();
            StringBuilder normalized = new StringBuilder();
            StringBuilder mnemonics = new StringBuilder();
            for (Instruction ins : program.getListing().getInstructions(f.getBody(), true)) {
                info.instructions++;
                String mnemonic = ins.getMnemonicString();
                mnemonics.append(mnemonic).append(';');
                strict.append(mnemonic);
                normalized.append(mnemonic);
                for (int op = 0; op < ins.getNumOperands(); op++) {
                    StringBuilder strictOp = new StringBuilder();
                    StringBuilder normOp = new StringBuilder();
                    for (Object o : ins.getOpObjects(op)) {
                        if (o instanceof Scalar scalar) {
                            long v = scalar.getUnsignedValue();
                            strictOp.append(v >= imageLow && v <= imageHigh ? MASK : Long.toHexString(v));
                            normOp.append(MASK);
                        }
                        else if (o instanceof Address) {
                            strictOp.append(MASK);
                            normOp.append(MASK);
                        }
                        else {
                            strictOp.append(o);
                            normOp.append(o);
                        }
                        strictOp.append(',');
                        normOp.append(',');
                    }
                    strict.append(' ').append(strictOp);
                    normalized.append(' ').append(normOp);
                }
                strict.append(';');
                normalized.append(';');
                boolean isCall = ins.getFlowType().isCall();
                for (Reference r : ins.getReferencesFrom()) {
                    Address to = r.getToAddress();
                    if (isCall && r.getReferenceType().isCall()) {
                        Function callee = resolve(program, to);
                        if (callee != null) {
                            info.calls.add(callee.getEntryPoint());
                        }
                    }
                    else if (r.getReferenceType().isData()) {
                        Data data = program.getListing().getDataAt(to);
                        if (data != null && data.hasStringValue()) {
                            info.strings.add(StringDataInstance.getStringDataInstance(data).getStringValue());
                        }
                    }
                }
            }
            info.strict = strict.toString();
            info.normalized = normalized.toString();
            info.mnemonics = mnemonics.toString();
            result.put(f.getEntryPoint(), info);
        }
        return result;
    }

    private static Function resolve(Program program, Address to) {
        Function callee = program.getFunctionManager().getFunctionAt(to);
        while (callee != null && callee.isThunk()) {
            Function next = callee.getThunkedFunction(false);
            if (next == null) {
                break;
            }
            callee = next;
        }
        return callee;
    }

    private static String exportName(Function f) {
        for (Symbol s : f.getProgram().getSymbolTable().getSymbols(f.getEntryPoint())) {
            String n = s.getName();
            if (s.isExternalEntryPoint() && (n.startsWith("_AIL") || n.startsWith("AIL_") || n.startsWith("_RIB")
                || n.startsWith("RIB_"))) {
                return s.getName();
            }
        }
        return null;
    }

    private void seedExports(Map<Address, Info> sourceInfo, Map<Address, Info> targetInfo) {
        Map<String, Info> sourceExports = new HashMap<>();
        for (Info info : sourceInfo.values()) {
            String name = exportName(info.function);
            if (name != null) {
                sourceExports.put(name, info);
            }
        }
        for (Info info : targetInfo.values()) {
            String name = exportName(info.function);
            Info other = name == null ? null : sourceExports.get(name);
            if (other != null) {
                record(new Match(other, info, TIER_SEED));
            }
        }
    }

    private String key(Info info, int tier) {
        return switch (tier) {
            case TIER_STRICT -> info.instructions >= MIN_NORMALIZED_INSTRUCTIONS ? info.strict : null;
            case TIER_NORMALIZED -> info.instructions >= MIN_NORMALIZED_INSTRUCTIONS ? info.normalized : null;
            default -> info.instructions >= MIN_MNEMONIC_INSTRUCTIONS ? info.mnemonics : null;
        };
    }

    private void matchByKey(Map<Address, Info> sourceInfo, Map<Address, Info> targetInfo, int tier) {
        Map<String, List<Info>> sources = group(sourceInfo, bySource, tier);
        Map<String, List<Info>> targets = group(targetInfo, byTarget, tier);
        for (Map.Entry<String, List<Info>> e : sources.entrySet()) {
            List<Info> other = targets.get(e.getKey());
            if (other != null && other.size() == 1 && e.getValue().size() == 1) {
                accept(e.getValue().get(0), other.get(0), tier);
            }
        }
    }

    private Map<String, List<Info>> group(Map<Address, Info> infos, Map<Address, Match> taken, int tier) {
        Map<String, List<Info>> result = new HashMap<>();
        for (Info info : infos.values()) {
            if (!taken.containsKey(info.function.getEntryPoint())) {
                String key = key(info, tier);
                if (key != null) {
                    result.computeIfAbsent(key, k -> new ArrayList<>()).add(info);
                }
            }
        }
        return result;
    }

    private boolean accept(Info source, Info target, int tier) {
        if (!source.strings.isEmpty() && !target.strings.isEmpty()) {
            Set<String> common = new HashSet<>(source.strings);
            common.retainAll(target.strings);
            if (common.isEmpty()) {
                stringRefusals++;
                return false;
            }
        }
        record(new Match(source, target, tier));
        return true;
    }

    private void record(Match m) {
        byTarget.put(m.target.function.getEntryPoint(), m);
        bySource.put(m.source.function.getEntryPoint(), m);
    }

    private void matchByCalls(Map<Address, Info> sourceInfo, Map<Address, Info> targetInfo) {
        ArrayDeque<Match> queue = new ArrayDeque<>(byTarget.values());
        while (!queue.isEmpty()) {
            Match m = queue.poll();
            if (m.source.calls.size() != m.target.calls.size()) {
                continue;
            }
            for (int i = 0; i < m.source.calls.size(); i++) {
                Info s = sourceInfo.get(m.source.calls.get(i));
                Info t = targetInfo.get(m.target.calls.get(i));
                if (s == null || t == null || bySource.containsKey(s.function.getEntryPoint())
                    || byTarget.containsKey(t.function.getEntryPoint()) || s.instructions < MIN_CALL_INSTRUCTIONS) {
                    continue;
                }
                boolean similar = s.normalized.equals(t.normalized) || s.mnemonics.equals(t.mnemonics)
                    || similarity(s.mnemonics, t.mnemonics) >= MIN_CALL_SIMILARITY;
                if (similar && accept(s, t, TIER_CALL)) {
                    queue.add(bySource.get(s.function.getEntryPoint()));
                }
            }
        }
    }

    // Longest common subsequence of the mnemonic lists over the longer list.
    private static double similarity(String a, String b) {
        String[] x = a.split(";");
        String[] y = b.split(";");
        int[] previous = new int[y.length + 1];
        int[] current = new int[y.length + 1];
        for (String mx : x) {
            for (int j = 1; j <= y.length; j++) {
                current[j] = mx.equals(y[j - 1]) ? previous[j - 1] + 1 : Math.max(previous[j], current[j - 1]);
            }
            int[] swap = previous;
            previous = current;
            current = swap;
        }
        return (double) previous[y.length] / Math.max(x.length, y.length);
    }
}
