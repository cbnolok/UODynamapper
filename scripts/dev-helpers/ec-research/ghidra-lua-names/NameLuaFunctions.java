// Name UO Lua API C functions (registration calls), Lua 5.1 library functions (luaL_Reg tables)
// and Lua/LuaPlus functions (unique source string literals); with --apply, rename DEFAULT
// functions, add tags and plate comments.
// @category ECResearch
import ghidra.app.script.GhidraScript;
import ghidra.app.util.PseudoDisassembler;
import ghidra.program.model.address.Address;
import ghidra.program.model.lang.Register;
import ghidra.program.model.listing.Function;
import ghidra.program.model.listing.FunctionTag;
import ghidra.program.model.listing.Instruction;
import ghidra.program.model.mem.MemoryBlock;
import ghidra.program.model.scalar.Scalar;
import ghidra.program.model.symbol.Namespace;
import ghidra.program.model.symbol.SourceType;
import ghidra.program.model.symbol.Symbol;
import ghidra.program.model.symbol.SymbolTable;
import ghidra.util.exception.DuplicateNameException;
import java.io.File;
import java.io.PrintWriter;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardOpenOption;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.HashSet;
import java.util.LinkedHashMap;
import java.util.LinkedList;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.TreeMap;
import java.util.TreeSet;
import java.util.regex.Matcher;
import java.util.regex.Pattern;
import java.util.stream.Stream;

public class NameLuaFunctions extends GhidraScript {
    private static final String APPLY_FLAG = "--apply";
    private static final String SOURCES_OPTION = "--sources";
    private static final String UI_OPTION = "--ui";
    private static final String RANGE_OPTION = "--range";
    private static final String DEFAULT_SOURCES = "lua51_sources.tsv";
    // Script arguments are split on whitespace, so a path with spaces is passed with %20.
    private static final String ENCODED_SPACE = "%20";
    private static final String TAG_API = "LUA_API";
    private static final String TAG_REG = "LUA_REG";
    private static final String TAG_STRING = "LUA_STRING";
    private static final String API_NAMESPACE = "UOLua";
    private static final String API_HELPER_NAME = "RegisterFunction";
    private static final String LUAPLUS_NAMESPACE = "LuaPlus";
    private static final String COMMENT_PREFIX = "NameLuaFunctions: ";
    private static final String SCOPE = "::";
    // The client registers some functions as _Name; the UI calls Name (a wrapper defined elsewhere).
    private static final String HIDDEN_PREFIX = "_";
    private static final String PREFERRED_VARIANT = "lua51-luaplus";

    private static final int POINTER_SIZE = 4;
    private static final long DWORD_MASK = 0xffffffffL;
    private static final int BYTE_MASK = 0xff;
    private static final int BITS_PER_BYTE = 8;
    private static final int OPCODE_PUSH_IMM = 0x68;
    private static final int OPCODE_CALL_REL = 0xe8;
    private static final int OPCODE_LENGTH = 1;
    private static final int IMM_INSTRUCTION_LENGTH = OPCODE_LENGTH + POINTER_SIZE;
    private static final int MAX_IDENTIFIER_LENGTH = 128;
    private static final int MAX_STRING_LENGTH = 512;
    private static final int PRINTABLE_FIRST = 0x20;
    private static final int PRINTABLE_LAST = 0x7e;
    // Registration survey: a PUSH of an identifier and a PUSH of code within this many bytes
    // before a CALL.
    private static final int SURVEY_WINDOW = 24;
    // Bytes decoded backwards from a call site to recover its pushed arguments.
    private static final int ARGUMENT_WINDOW = 48;
    private static final int API_ARGS = 3;
    private static final int MAX_ARGS = 4;
    private static final int MIN_API_REGISTRATIONS = 100;
    private static final int MIN_SECONDARY_SITES = 4;
    // A secondary helper needs this share of its sites passing a name and a code pointer.
    private static final double MIN_SECONDARY_SHARE = 0.5;
    private static final int MAX_TABLE_ENTRIES = 4096;
    // luaL_Reg { const char *name; lua_CFunction func; } arrays, {0, 0} terminated.
    private static final int REG_ENTRY_SIZE = 2 * POINTER_SIZE;
    private static final int MIN_REG_ENTRIES = 2;
    private static final double MIN_TABLE_SCORE = 0.5;
    // luaL_register call: the CALL must follow the PUSH of the table within this many bytes.
    private static final int REGISTER_CALL_WINDOW = 24;
    private static final int MAX_FORWARD_INSTRUCTIONS = 8;
    // Unrolled luaL_openlibs: PUSH luaopen_x ... PUSH "x" within this many bytes, all pairs
    // with at most OPEN_CLUSTER_GAP bytes between pairs, at least MIN_OPEN_SHARE of the source
    // library names found.
    private static final int OPEN_WINDOW = 24;
    private static final int OPEN_CLUSTER_GAP = 0x100;
    private static final String OPEN_PREFIX = "luaopen_";
    private static final double MIN_OPEN_SHARE = 0.5;
    // String evidence: literal length and letter count, format specifiers not counted.
    private static final int MIN_STRING_LENGTH = 6;
    private static final int MIN_STRING_LETTERS = 4;
    // Default LUA_STRING target range: the LUA_REG functions widened by this margin.
    private static final long RANGE_MARGIN = 0x20000;
    private static final int HEX_RADIX = 16;
    private static final Pattern IDENTIFIER = Pattern.compile("[A-Za-z_][A-Za-z0-9_]*");
    // UO API names may name a table field: Settings.LoadChatSettings.
    private static final Pattern API_NAME = Pattern.compile("[A-Za-z_][A-Za-z0-9_]*(\\.[A-Za-z_][A-Za-z0-9_]*)*");
    private static final String API_NAME_SEPARATOR = "\\.";
    private static final Pattern FORMAT_SPEC = Pattern.compile("%[-+ #0-9.]*[a-zA-Z%]");
    // CamelCase words of UO API names listed as texture/image related.
    private static final Set<String> TEXTURE_WORDS = Set.of("texture", "image", "icon", "art", "sprite",
        "portrait", "bitmap", "gump", "tile", "shader");
    private static final Pattern WORD_BREAK = Pattern.compile("(?<=[a-z0-9])(?=[A-Z])|[_.]");
    private static final Set<String> LUA_KEYWORDS = Set.of("and", "break", "do", "else", "elseif", "end",
        "false", "for", "function", "if", "in", "local", "nil", "not", "or", "repeat", "return", "then",
        "true", "until", "while");
    // Base globals set by base_open outside base_funcs.
    private static final Set<String> AUX_GLOBALS = Set.of("ipairs", "pairs", "newproxy", "_G", "_VERSION");

    private static final class Block {
        long start;
        byte[] bytes;
        boolean exec;
    }

    private static final class Arg {
        long value = -1;
        long tableBase = -1;
        long tableOffset;
        String text;
    }

    private static final class Registration {
        long site;
        String name;
        long function;
        String via;
    }

    private static final class RegTable {
        long address;
        final List<String> names = new ArrayList<>();
        final List<Long> functions = new ArrayList<>();
        final List<Long> refs = new ArrayList<>();
        String libname;
        long registerCallee = -1;
        String sourceKey;
        double score;
        boolean primary;
        String label;
    }

    private static final class SourceTable {
        String key;
        String file;
        String table;
        final Map<String, String> entries = new LinkedHashMap<>();
        String registeredBy;
        String libname;
        String callee;
    }

    private static final class Proposal {
        long address;
        String path;
        String tag;
        String evidence;

        Proposal(long address, String path, String tag, String evidence) {
            this.address = address;
            this.path = path;
            this.tag = tag;
            this.evidence = evidence;
        }
    }

    private static final class StringEvidence {
        String text;
        String sourceFunction;
        String files;
        final Set<Long> functions = new TreeSet<>();
        final Set<Long> orphanSites = new TreeSet<>();
        int dataRefs;
        int copies;
        String reason;
    }

    private final List<Block> blocks = new ArrayList<>();
    private Block lastBlock;
    private Block textBlock;
    private PseudoDisassembler pseudo;
    private final List<String> anomalies = new ArrayList<>();
    private final List<String> conflicts = new ArrayList<>();
    private final List<String> agreements = new ArrayList<>();
    private final List<String> kept = new ArrayList<>();
    private final Map<String, Integer> changes = new LinkedHashMap<>();
    private final Map<Long, List<Proposal>> proposals = new TreeMap<>();

    public void run() throws Exception {
        String[] args = getScriptArgs();
        if (args.length < 1) {
            throw new IllegalArgumentException("Expected OUTPUT_FILE [" + APPLY_FLAG + "] [" + SOURCES_OPTION
                + " TSV] [" + UI_OPTION + " DIR] [" + RANGE_OPTION + " 0xSTART-0xEND]");
        }
        boolean apply = false;
        String sourcesPath = null;
        String uiPath = null;
        long[] range = null;
        for (int i = 1; i < args.length; i++) {
            switch (args[i]) {
                case APPLY_FLAG -> apply = true;
                case SOURCES_OPTION -> sourcesPath = decodePath(args[++i]);
                case UI_OPTION -> uiPath = decodePath(args[++i]);
                case RANGE_OPTION -> range = parseRange(args[++i]);
                default -> throw new IllegalArgumentException("Unknown argument " + args[i]);
            }
        }
        if (sourcesPath == null) {
            sourcesPath = new File(getSourceFile().getParentFile().getAbsolutePath(), DEFAULT_SOURCES).getPath();
        }
        if (!new File(sourcesPath).isFile()) {
            throw new IllegalArgumentException("Source map " + sourcesPath + " not found; pass " + SOURCES_OPTION);
        }
        // Created before any change, so an existing output file stops the run unchanged.
        try (PrintWriter output = new PrintWriter(Files.newBufferedWriter(Path.of(args[0]),
                StandardOpenOption.CREATE_NEW))) {
            analyze(output, apply, sourcesPath, uiPath, range);
        }
        println("Wrote " + args[0] + (apply ? " (applied: " + changes + ")" : ""));
    }

    private void analyze(PrintWriter output, boolean apply, String sourcesPath, String uiPath, long[] range)
            throws Exception {
        loadBlocks();
        pseudo = new PseudoDisassembler(currentProgram);
        Sources sources = new Sources(sourcesPath);

        // 1. LUA_API: registration helper calls (and table-driven loops around them).
        Map<Long, Integer> survey = surveyRegistrationCalls();
        long helper = -1;
        int helperSurvey = 0;
        for (var entry : survey.entrySet()) {
            if (entry.getValue() > helperSurvey) {
                helper = entry.getKey();
                helperSurvey = entry.getValue();
            }
        }
        if (helperSurvey < MIN_API_REGISTRATIONS) {
            helper = -1;
        }
        List<Registration> apiRegistrations = new ArrayList<>();
        List<String> apiUnresolved = new ArrayList<>();
        int[] apiPositions = helper >= 0 ? argumentPositions(helper, API_ARGS) : null;
        if (apiPositions != null) {
            extractRegistrations(helper, API_ARGS, apiPositions, apiRegistrations, apiUnresolved, "helper");
        }

        // 2. LUA_REG: luaL_Reg tables, luaL_register calls and the unrolled luaL_openlibs.
        List<RegTable> tables = scanRegTables();
        Map<Long, Integer> registerCallees = new HashMap<>();
        for (RegTable table : tables) {
            findTableRegistration(table);
            if (table.registerCallee >= 0) {
                registerCallees.merge(table.registerCallee, 1, Integer::sum);
            }
        }
        long luaRegister = mostCommon(registerCallees);
        matchTables(tables, sources);
        Map<String, Long> openFunctions = new TreeMap<>();
        long openCluster = findOpenLibs(sources, tables, openFunctions);
        proposeRegNames(tables, sources, luaRegister, openFunctions, openCluster);

        long[] luaRange = range;
        if (luaRange == null) {
            long low = Long.MAX_VALUE;
            long high = -1;
            for (var entry : proposals.entrySet()) {
                for (Proposal proposal : entry.getValue()) {
                    if (proposal.tag.equals(TAG_REG)) {
                        low = Math.min(low, entry.getKey());
                        high = Math.max(high, entry.getKey());
                    }
                }
            }
            luaRange = high < 0 ? new long[] {0, DWORD_MASK}
                : new long[] {Math.max(0, low - RANGE_MARGIN), high + RANGE_MARGIN};
        }

        // Secondary registration helpers (LuaPlus LuaObject::Register-like) inside the Lua range.
        List<String> secondaryReport = new ArrayList<>();
        List<Registration> luaPlusRegistrations = new ArrayList<>();
        for (var entry : survey.entrySet()) {
            long callee = entry.getKey();
            if (callee == helper || entry.getValue() < MIN_SECONDARY_SITES) {
                continue;
            }
            int[] positions = argumentPositions(callee, MAX_ARGS);
            Function function = getFunctionAt(toAddr(callee));
            String name = function != null ? function.getName(true) : "no function";
            boolean inRange = callee >= luaRange[0] && callee <= luaRange[1];
            if (positions == null) {
                secondaryReport.add(hex(callee) + " `" + name + "`: " + entry.getValue()
                    + " survey hits, no consistent name/code argument positions; ignored");
                continue;
            }
            if (!inRange) {
                secondaryReport.add(hex(callee) + " `" + name + "`: name in argument " + (positions[0] + 1)
                    + ", code in argument " + (positions[1] + 1) + ", outside the Lua range; ignored");
                continue;
            }
            List<Registration> found = new ArrayList<>();
            extractRegistrations(callee, MAX_ARGS, positions, found, apiUnresolved, "LuaPlus helper");
            secondaryReport.add(hex(callee) + " `" + name + "`: " + found.size()
                + " registrations, name in argument " + (positions[0] + 1) + ", code in argument "
                + (positions[1] + 1) + "; named in namespace `" + LUAPLUS_NAMESPACE + "`");
            luaPlusRegistrations.addAll(found);
        }
        resolveDispatchers(luaPlusRegistrations, secondaryReport);
        Map<Long, List<Registration>> apiByFunction = proposeApiNames(apiRegistrations, API_NAMESPACE, helper,
            "UO Lua API");
        Map<Long, List<Registration>> luaPlusByFunction = proposeApiNames(luaPlusRegistrations, LUAPLUS_NAMESPACE,
            -1, "LuaPlus global");

        // Registered C functions are created before string matching, so strings they reference
        // count as referenced by a function and a second run sees the same functions.
        if (apply) {
            for (var entry : proposals.entrySet()) {
                if (hasProposal(entry.getKey(), TAG_API) || hasProposal(entry.getKey(), TAG_REG)) {
                    ensureFunction(entry.getKey(), entry.getValue().get(0).path);
                }
            }
        }

        // 3. LUA_STRING: unique source literals referenced by exactly one function.
        List<StringEvidence> stringEvidence = matchStrings(sources, luaRange);

        Map<Long, Proposal> chosen = resolve();
        if (apply) {
            annotate(chosen, helper);
        }

        UiNames ui = uiPath != null ? new UiNames(uiPath) : null;
        report(output, apply, sourcesPath, helper, helperSurvey, apiRegistrations, apiByFunction,
            apiUnresolved, secondaryReport, luaPlusByFunction, tables, luaRegister, openFunctions, openCluster,
            luaRange, range != null, stringEvidence, chosen, ui, sources);
    }

    // ---- Source map ----

    private final class Sources {
        final Map<String, SourceTable> tables = new LinkedHashMap<>();
        final Map<String, String> openLibs = new LinkedHashMap<>();
        final Map<String, Set<String>> functionsByFile = new HashMap<>();
        final Map<String, Integer> definitions = new HashMap<>();
        final List<String[]> strings = new ArrayList<>();
        final Set<String> baseGlobals = new TreeSet<>();
        String registerCallee;

        Sources(String path) throws Exception {
            Map<String, Set<String>> definitionVariants = new HashMap<>();
            for (String line : Files.readAllLines(Path.of(path), StandardCharsets.US_ASCII)) {
                if (line.startsWith("#") || line.isBlank()) {
                    continue;
                }
                String[] f = line.split("\t", -1);
                for (int i = 0; i < f.length; i++) {
                    f[i] = unescapeField(f[i]);
                }
                switch (f[0]) {
                    case "REG" -> {
                        SourceTable table = tables.computeIfAbsent(f[2] + ":" + f[3], key -> {
                            SourceTable created = new SourceTable();
                            created.key = key;
                            created.file = f[2];
                            created.table = f[3];
                            return created;
                        });
                        // The LuaPlus variant wins when the variants disagree.
                        if (f[1].equals(PREFERRED_VARIANT) || !table.entries.containsKey(f[4])) {
                            table.entries.put(f[4], f[5]);
                        }
                    }
                    case "REGISTER" -> {
                        SourceTable table = tables.get(f[2] + ":" + f[4]);
                        if (table != null && (table.registeredBy == null || f[1].equals(PREFERRED_VARIANT))) {
                            table.registeredBy = f[3];
                            table.libname = f[5];
                            table.callee = f[6];
                        }
                        registerCallee = f[6];
                    }
                    case "OPEN" -> openLibs.put(f[4], f[5]);
                    case "FUNC" -> {
                        functionsByFile.computeIfAbsent(f[2], key -> new HashSet<>()).add(f[3]);
                        definitionVariants.computeIfAbsent(f[1] + ":" + f[3], key -> new HashSet<>()).add(f[2]);
                        definitions.merge(f[1] + ":" + f[3], 1, Integer::sum);
                    }
                    case "STRING" -> strings.add(f);
                    default -> anomalies.add("Unknown source map record `" + f[0] + "`");
                }
            }
            for (SourceTable table : tables.values()) {
                if ("_G".equals(table.libname) || table.libname != null && table.libname.isEmpty()
                        && !table.entries.isEmpty() && table.registeredBy != null
                        && table.registeredBy.startsWith("luaopen_package")) {
                    baseGlobals.addAll(table.entries.keySet());
                }
            }
            baseGlobals.addAll(AUX_GLOBALS);
        }

        // More than one definition of this name in one variant (C++ overloads).
        boolean overloaded(String function) {
            for (var entry : definitions.entrySet()) {
                if (entry.getKey().endsWith(":" + function) && entry.getValue() > 1) {
                    return true;
                }
            }
            return false;
        }
    }

    private static String unescapeField(String field) {
        StringBuilder out = new StringBuilder();
        for (int i = 0; i < field.length(); i++) {
            char c = field.charAt(i);
            if (c != '\\' || i + 1 >= field.length()) {
                out.append(c);
                continue;
            }
            char e = field.charAt(++i);
            switch (e) {
                case 't' -> out.append('\t');
                case 'n' -> out.append('\n');
                case 'r' -> out.append('\r');
                case 'x' -> {
                    out.append((char) Integer.parseInt(field.substring(i + 1, i + 3), HEX_RADIX));
                    i += 2;
                }
                default -> out.append(e);
            }
        }
        return out.toString();
    }

    // ---- LUA_API ----

    // Callees with PUSH <identifier string> and PUSH <code> shortly before the call.
    private Map<Long, Integer> surveyRegistrationCalls() {
        Map<Long, Integer> counts = new HashMap<>();
        byte[] b = textBlock.bytes;
        for (int j = SURVEY_WINDOW; j + IMM_INSTRUCTION_LENGTH <= b.length; j++) {
            if ((b[j] & BYTE_MASK) != OPCODE_CALL_REL) {
                continue;
            }
            long callee = callTarget(j);
            if (!isExec(callee)) {
                continue;
            }
            boolean name = false;
            boolean code = false;
            for (int i = j - SURVEY_WINDOW; i <= j - IMM_INSTRUCTION_LENGTH; i++) {
                if ((b[i] & BYTE_MASK) == OPCODE_PUSH_IMM) {
                    long value = le32(b, i + OPCODE_LENGTH);
                    name |= identifierAt(value) != null;
                    code |= isExec(value);
                }
            }
            if (name && code) {
                counts.merge(callee, 1, Integer::sum);
            }
        }
        return counts;
    }

    // {name argument index, code argument index} used by most call sites, or null.
    private int[] argumentPositions(long callee, int count) {
        Map<String, Integer> votes = new HashMap<>();
        List<Long> sites = callSites(callee);
        for (long site : sites) {
            List<Arg> args = pushedArgs(site, count);
            for (int n = 0; n < args.size(); n++) {
                for (int c = 0; c < args.size(); c++) {
                    if (n != c && identifierAt(args.get(n).value) != null && isExec(args.get(c).value)) {
                        votes.merge(n + ":" + c, 1, Integer::sum);
                    }
                }
            }
        }
        String best = null;
        int bestVotes = 0;
        for (var entry : votes.entrySet()) {
            if (entry.getValue() > bestVotes) {
                best = entry.getKey();
                bestVotes = entry.getValue();
            }
        }
        if (best == null || bestVotes < sites.size() * MIN_SECONDARY_SHARE) {
            return null;
        }
        String[] parts = best.split(":");
        return new int[] {Integer.parseInt(parts[0]), Integer.parseInt(parts[1])};
    }

    private void extractRegistrations(long callee, int count, int[] positions, List<Registration> out,
            List<String> unresolved, String via) {
        for (long site : callSites(callee)) {
            List<Arg> args = pushedArgs(site, count);
            Arg name = positions[0] < args.size() ? args.get(positions[0]) : null;
            Arg code = positions[1] < args.size() ? args.get(positions[1]) : null;
            if (name != null && apiNameAt(name.value) != null && code != null && isExec(code.value)) {
                out.add(registration(site, apiNameAt(name.value), code.value, via));
            } else if (name != null && name.tableBase >= 0) {
                // Table-driven loop: { func, name } or { name, func } entries ending with 0.
                long nameOffset = name.tableOffset;
                long codeOffset = nameOffset == 0 ? POINTER_SIZE : nameOffset - POINTER_SIZE;
                int entries = 0;
                for (long entry = name.tableBase; entries < MAX_TABLE_ENTRIES; entry += REG_ENTRY_SIZE, entries++) {
                    String text = apiNameAt(u32(entry + nameOffset));
                    long function = u32(entry + codeOffset);
                    if (text == null || !isExec(function)) {
                        break;
                    }
                    out.add(registration(site, text, function, via + " table " + hex(name.tableBase)));
                }
                if (entries == 0) {
                    unresolved.add(hex(site) + " (" + via + "): table " + hex(name.tableBase) + " has no entries");
                }
            } else {
                unresolved.add(hex(site) + " (" + via + "): arguments " + describe(args));
            }
        }
    }

    private static Registration registration(long site, String name, long function, String via) {
        Registration registration = new Registration();
        registration.site = site;
        registration.name = name;
        registration.function = function;
        registration.via = via;
        return registration;
    }

    // A code pointer registered under several names by a secondary helper is a dispatcher
    // (LuaPlus functor callback); the real function is the code immediate stored just before.
    private void resolveDispatchers(List<Registration> registrations, List<String> report) {
        Map<Long, Set<String>> names = new HashMap<>();
        for (Registration registration : registrations) {
            names.computeIfAbsent(registration.function, key -> new TreeSet<>()).add(registration.name);
        }
        for (Registration registration : registrations) {
            if (names.get(registration.function).size() < 2) {
                continue;
            }
            long dispatcher = registration.function;
            long target = -1;
            List<Instruction> window = decodeBefore(registration.site, ARGUMENT_WINDOW);
            for (Instruction instruction : window != null ? window : List.<Instruction>of()) {
                if (instruction.getMnemonicString().equals("MOV") && isMemory(instruction, 0)) {
                    Scalar scalar = instruction.getScalar(1);
                    if (scalar != null && isExec(scalar.getUnsignedValue()) && scalar.getUnsignedValue() != dispatcher) {
                        target = scalar.getUnsignedValue();
                    }
                }
            }
            if (target >= 0) {
                registration.function = target;
                registration.via += " (dispatcher " + hex(dispatcher) + ")";
            } else {
                registration.function = -1;
                report.add("`" + registration.name + "` at " + hex(registration.site) + " registers dispatcher "
                    + hex(dispatcher) + " without a recoverable target; not named");
            }
        }
        registrations.removeIf(registration -> registration.function < 0);
    }

    private Map<Long, List<Registration>> proposeApiNames(List<Registration> registrations, String namespace,
            long helper, String what) {
        Map<Long, List<Registration>> byFunction = new TreeMap<>();
        for (Registration registration : registrations) {
            byFunction.computeIfAbsent(registration.function, key -> new ArrayList<>()).add(registration);
        }
        for (var entry : byFunction.entrySet()) {
            List<Registration> list = entry.getValue();
            list.sort((a, b) -> Long.compare(a.site, b.site));
            Registration first = list.get(0);
            StringBuilder evidence = new StringBuilder(what + " `" + first.name + "` registered at "
                + hexShort(first.site) + " in " + functionLabel(first.site) + " via " + first.via);
            Set<String> others = new TreeSet<>();
            for (Registration registration : list) {
                if (!registration.name.equals(first.name)) {
                    others.add(registration.name);
                }
            }
            if (!others.isEmpty()) {
                evidence.append("; also registered as ").append(String.join(", ", others));
            }
            if (list.size() > 1) {
                evidence.append("; ").append(list.size()).append(" registrations");
            }
            List<String> parts = new ArrayList<>();
            for (String part : first.name.split(API_NAME_SEPARATOR)) {
                parts.add(sanitize(part));
            }
            propose(entry.getKey(), namespace + SCOPE + String.join(SCOPE, parts), TAG_API, evidence.toString());
        }
        return byFunction;
    }

    // ---- LUA_REG ----

    private List<RegTable> scanRegTables() {
        List<RegTable> tables = new ArrayList<>();
        for (Block block : blocks) {
            if (block.exec) {
                continue;
            }
            long end = block.start + block.bytes.length;
            for (long address = alignUp(block.start); address + REG_ENTRY_SIZE <= end; address += POINTER_SIZE) {
                if (!regEntry(address) || regEntry(address - REG_ENTRY_SIZE)) {
                    continue;
                }
                RegTable table = new RegTable();
                table.address = address;
                long entry = address;
                while (regEntry(entry) && table.names.size() < MAX_TABLE_ENTRIES) {
                    table.names.add(identifierAt(u32(entry)));
                    table.functions.add(u32(entry + POINTER_SIZE));
                    entry += REG_ENTRY_SIZE;
                }
                if (table.names.size() >= MIN_REG_ENTRIES && u32(entry) == 0 && u32(entry + POINTER_SIZE) == 0) {
                    tables.add(table);
                }
            }
        }
        return tables;
    }

    private boolean regEntry(long address) {
        return identifierAt(u32(address)) != null && isExec(u32(address + POINTER_SIZE));
    }

    // PUSH <table> ... PUSH <libname> ... CALL luaL_register.
    private void findTableRegistration(RegTable table) {
        byte[] b = textBlock.bytes;
        for (int i = 0; i + IMM_INSTRUCTION_LENGTH <= b.length; i++) {
            if ((b[i] & BYTE_MASK) != OPCODE_PUSH_IMM || le32(b, i + OPCODE_LENGTH) != table.address) {
                continue;
            }
            long push = textBlock.start + i;
            table.refs.add(push);
            Instruction instruction = decode(push + IMM_INSTRUCTION_LENGTH);
            Arg libname = null;
            for (int count = 0; instruction != null && count < MAX_FORWARD_INSTRUCTIONS; count++) {
                long at = instruction.getAddress().getOffset();
                if (at - push > REGISTER_CALL_WINDOW) {
                    break;
                }
                if (instruction.getFlowType().isCall()) {
                    Address[] flows = instruction.getFlows();
                    if (flows.length == 1 && table.registerCallee < 0) {
                        table.registerCallee = flows[0].getOffset();
                        table.libname = libname == null ? null : libname.value == 0 ? ""
                            : stringAt(libname.value);
                    }
                    break;
                }
                if (libname == null && instruction.getMnemonicString().equals("PUSH")) {
                    libname = operandArg(instruction, 0, decodeBefore(push, ARGUMENT_WINDOW), Integer.MAX_VALUE);
                }
                instruction = decode(at + instruction.getLength());
            }
        }
    }

    private void matchTables(List<RegTable> tables, Sources sources) {
        Map<String, RegTable> primary = new HashMap<>();
        for (RegTable table : tables) {
            Set<String> names = new HashSet<>(table.names);
            for (SourceTable source : sources.tables.values()) {
                Set<String> union = new HashSet<>(names);
                union.addAll(source.entries.keySet());
                Set<String> common = new HashSet<>(names);
                common.retainAll(source.entries.keySet());
                double score = (double) common.size() / union.size();
                if (score > table.score) {
                    table.score = score;
                    table.sourceKey = source.key;
                }
            }
            if (table.score < MIN_TABLE_SCORE) {
                table.sourceKey = null;
                continue;
            }
            RegTable best = primary.get(table.sourceKey);
            if (best == null || table.score > best.score) {
                primary.put(table.sourceKey, table);
            }
        }
        for (RegTable table : primary.values()) {
            table.primary = true;
        }
        for (RegTable table : tables) {
            SourceTable source = table.sourceKey != null ? sources.tables.get(table.sourceKey) : null;
            table.label = table.libname != null && !table.libname.isEmpty() ? table.libname
                : source != null ? source.table : null;
        }
    }

    // Unrolled luaL_openlibs: PUSH luaopen_x; ...; PUSH "x"; CALL lua_pushstring. Library names
    // come from linit.c and, for libraries missing there, from the luaL_register calls.
    private long findOpenLibs(Sources sources, List<RegTable> tables, Map<String, Long> out) {
        Map<String, String> names = new LinkedHashMap<>(sources.openLibs);
        for (RegTable table : tables) {
            if (table.libname != null && !table.libname.isEmpty() && !names.containsKey(table.libname)
                    && !sources.openLibs.containsValue(OPEN_PREFIX + table.libname)) {
                names.put(table.libname, OPEN_PREFIX + table.libname);
            }
        }
        byte[] b = textBlock.bytes;
        TreeMap<Long, Object[]> pairs = new TreeMap<>();
        for (int i = 0; i + IMM_INSTRUCTION_LENGTH <= b.length; i++) {
            if ((b[i] & BYTE_MASK) != OPCODE_PUSH_IMM) {
                continue;
            }
            String text = stringAt(le32(b, i + OPCODE_LENGTH));
            if (text == null || !names.containsKey(text)) {
                continue;
            }
            for (int j = Math.max(0, i - OPEN_WINDOW); j < i; j++) {
                if ((b[j] & BYTE_MASK) == OPCODE_PUSH_IMM && isExec(le32(b, j + OPCODE_LENGTH))) {
                    pairs.put(textBlock.start + i, new Object[] {text, le32(b, j + OPCODE_LENGTH)});
                }
            }
        }
        long best = -1;
        Map<String, Long> bestLibs = new TreeMap<>();
        long start = -1;
        long previous = -1;
        Map<String, Long> libs = new TreeMap<>();
        for (var entry : pairs.entrySet()) {
            if (previous < 0 || entry.getKey() - previous > OPEN_CLUSTER_GAP) {
                start = entry.getKey();
                libs = new TreeMap<>();
            }
            libs.put((String) entry.getValue()[0], (Long) entry.getValue()[1]);
            previous = entry.getKey();
            if (libs.size() > bestLibs.size()) {
                best = start;
                bestLibs = libs;
            }
        }
        int known = 0;
        for (String lib : bestLibs.keySet()) {
            known += sources.openLibs.containsKey(lib) ? 1 : 0;
        }
        if (known < sources.openLibs.size() * MIN_OPEN_SHARE) {
            return -1;
        }
        for (var lib : bestLibs.entrySet()) {
            out.put(names.get(lib.getKey()), lib.getValue());
        }
        return best;
    }

    private void proposeRegNames(List<RegTable> tables, Sources sources, long luaRegister,
            Map<String, Long> openFunctions, long openCluster) {
        for (RegTable table : tables) {
            SourceTable source = table.sourceKey != null ? sources.tables.get(table.sourceKey) : null;
            String where = "luaL_Reg " + hexShort(table.address) + (table.libname != null
                ? " (registered as `" + (table.libname.isEmpty() ? "NULL" : table.libname) + "`)" : "");
            if (source == null && table.label == null) {
                continue;
            }
            Set<String> fileFunctions = source != null ? sources.functionsByFile.getOrDefault(source.file, Set.of())
                : Set.of();
            for (int i = 0; i < table.names.size(); i++) {
                String entry = table.names.get(i);
                long function = table.functions.get(i);
                String cName = source != null ? source.entries.get(entry) : null;
                String evidence;
                String path;
                if (cName != null && table.primary) {
                    path = cName;
                    evidence = where + " entry `" + entry + "` = `" + source.table + "` (" + source.file
                        + ") score " + score(table.score);
                } else if (cName == null && table.primary && fileFunctions.contains(entry)) {
                    path = entry;
                    evidence = where + " entry `" + entry + "`, not in `" + source.table
                        + "` but a function of that name exists in " + source.file;
                } else if (table.label != null) {
                    path = sanitize(table.label) + SCOPE + sanitize(entry);
                    evidence = where + " entry `" + entry + "`" + (source != null ? " (table matches `"
                        + source.table + "` score " + score(table.score) + (table.primary ? "" : ", not the best match")
                        + ")" : " (no source table)");
                } else {
                    continue;
                }
                // A function already named from a better table keeps that name.
                if (!table.primary && hasProposal(function, TAG_REG)) {
                    continue;
                }
                propose(function, path, TAG_REG, evidence);
            }
            if (table.primary && source != null && source.registeredBy != null && !table.refs.isEmpty()) {
                Function function = getFunctionContaining(toAddr(table.refs.get(0)));
                if (function != null) {
                    propose(function.getEntryPoint().getOffset(), source.registeredBy, TAG_REG, "registers "
                        + where + " (`" + source.table + "` is registered by `" + source.registeredBy + "` in "
                        + source.file + ")");
                } else if (!openFunctions.containsKey(source.registeredBy)) {
                    anomalies.add("Reference to " + where + " at " + hex(table.refs.get(0))
                        + " is not inside a function (source: `" + source.registeredBy + "`)");
                }
            }
        }
        if (luaRegister >= 0 && sources.registerCallee != null) {
            propose(luaRegister, sources.registerCallee, TAG_REG,
                "called with luaL_Reg tables after PUSH <table>; PUSH <libname>");
        }
        for (var entry : openFunctions.entrySet()) {
            boolean listed = sources.openLibs.containsValue(entry.getKey());
            propose(entry.getValue(), entry.getKey(), TAG_REG, "pushed before its library name in the unrolled"
                + " luaL_openlibs at " + hexShort(openCluster) + (listed ? " (linit.c `lualibs`)"
                : " (library not in linit.c; name follows the luaopen_<libname> convention)"));
        }
    }

    // ---- LUA_STRING ----

    private List<StringEvidence> matchStrings(Sources sources, long[] range) {
        Map<String, StringEvidence> wanted = new LinkedHashMap<>();
        List<StringEvidence> all = new ArrayList<>();
        for (String[] record : sources.strings) {
            StringEvidence evidence = new StringEvidence();
            evidence.text = record[5];
            evidence.sourceFunction = record[3];
            evidence.files = record[2];
            all.add(evidence);
            String letters = FORMAT_SPEC.matcher(evidence.text).replaceAll("").replaceAll("[^A-Za-z]", "");
            if (evidence.text.length() < MIN_STRING_LENGTH || letters.length() < MIN_STRING_LETTERS) {
                evidence.reason = "short or format-only";
                continue;
            }
            wanted.put(evidence.text, evidence);
        }
        // Copies of each wanted literal: NUL-terminated, starting after a NUL.
        Map<Long, StringEvidence> copies = new HashMap<>();
        for (Block block : blocks) {
            if (block.exec) {
                continue;
            }
            byte[] b = block.bytes;
            for (int i = 0; i < b.length; i++) {
                if (i > 0 && b[i - 1] != 0 || b[i] == 0) {
                    continue;
                }
                int end = i;
                while (end < b.length && b[end] != 0 && end - i < MAX_STRING_LENGTH) {
                    end++;
                }
                if (end < b.length && b[end] == 0) {
                    StringEvidence evidence = wanted.get(new String(b, i, end - i, StandardCharsets.ISO_8859_1));
                    if (evidence != null) {
                        copies.put(block.start + i, evidence);
                        evidence.copies++;
                    }
                }
                i = end;
            }
        }
        // References: any 4-byte immediate in code, aligned pointers in data.
        for (Block block : blocks) {
            byte[] b = block.bytes;
            int step = block.exec ? 1 : POINTER_SIZE;
            for (int i = 0; i + POINTER_SIZE <= b.length; i += step) {
                StringEvidence evidence = copies.get(le32(b, i));
                if (evidence == null) {
                    continue;
                }
                if (!block.exec) {
                    evidence.dataRefs++;
                    continue;
                }
                long site = block.start + i;
                Function function = getFunctionContaining(toAddr(site));
                if (function != null) {
                    evidence.functions.add(function.getEntryPoint().getOffset());
                } else {
                    evidence.orphanSites.add(site);
                }
            }
        }
        // Per string: exactly one referencing function, inside the range, nothing else.
        Map<Long, Map<String, List<StringEvidence>>> byTarget = new TreeMap<>();
        for (StringEvidence evidence : wanted.values()) {
            if (evidence.copies == 0) {
                evidence.reason = "not in the program";
            } else if (evidence.dataRefs > 0) {
                evidence.reason = evidence.dataRefs + " data references";
            } else if (!evidence.orphanSites.isEmpty()) {
                evidence.reason = "referenced from code outside functions at " + hexList(evidence.orphanSites);
            } else if (evidence.functions.size() != 1) {
                evidence.reason = evidence.functions.isEmpty() ? "no code reference"
                    : "referenced by " + evidence.functions.size() + " functions";
            } else {
                long target = evidence.functions.iterator().next();
                if (target < range[0] || target > range[1]) {
                    evidence.reason = "only referencing function " + hex(target) + " is outside the range";
                    continue;
                }
                byTarget.computeIfAbsent(target, key -> new TreeMap<>())
                    .computeIfAbsent(evidence.sourceFunction, key -> new ArrayList<>()).add(evidence);
            }
        }
        // Conflicts: one function with several source names, one source name in several functions.
        Map<String, Set<Long>> targetsByName = new HashMap<>();
        for (var entry : byTarget.entrySet()) {
            for (String name : entry.getValue().keySet()) {
                targetsByName.computeIfAbsent(name, key -> new TreeSet<>()).add(entry.getKey());
            }
        }
        for (var entry : byTarget.entrySet()) {
            Map<String, List<StringEvidence>> names = entry.getValue();
            if (names.size() > 1) {
                conflicts.add("LUA_STRING: " + hex(entry.getKey()) + " references unique strings of "
                    + String.join(", ", names.keySet()) + "; not named");
                for (List<StringEvidence> list : names.values()) {
                    list.forEach(evidence -> evidence.reason = "function also matches another source function");
                }
                continue;
            }
            String name = names.keySet().iterator().next();
            List<StringEvidence> list = names.get(name);
            Set<Long> targets = targetsByName.get(name);
            if (targets.size() > 1 && !sources.overloaded(name)) {
                conflicts.add("LUA_STRING: `" + name + "` strings are referenced by " + hexList(targets)
                    + "; not named");
                list.forEach(evidence -> evidence.reason = "source function matches several functions");
                continue;
            }
            List<String> quoted = new ArrayList<>();
            for (StringEvidence evidence : list) {
                evidence.reason = "accepted";
                quoted.add("\"" + printable(evidence.text) + "\"");
            }
            propose(entry.getKey(), name, TAG_STRING, list.size() + " unique string" + (list.size() > 1 ? "s" : "")
                + " of `" + name + "` (" + list.get(0).files + "): " + String.join(", ", quoted));
        }
        return all;
    }

    // ---- Resolution and annotation ----

    private void propose(long address, String path, String tag, String evidence) {
        for (Proposal existing : proposals.getOrDefault(address, List.of())) {
            if (existing.tag.equals(tag) && existing.path.equals(path)) {
                return;
            }
        }
        proposals.computeIfAbsent(address, key -> new ArrayList<>()).add(new Proposal(address, path, tag, evidence));
    }

    private boolean hasProposal(long address, String tag) {
        for (Proposal proposal : proposals.getOrDefault(address, List.of())) {
            if (proposal.tag.equals(tag)) {
                return true;
            }
        }
        return false;
    }

    // LUA_REG beats LUA_API beats LUA_STRING at one address; a name proposed for two addresses
    // by different sources keeps the stronger one.
    private Map<Long, Proposal> resolve() {
        List<String> order = List.of(TAG_REG, TAG_API, TAG_STRING);
        Map<Long, Proposal> chosen = new TreeMap<>();
        for (var entry : proposals.entrySet()) {
            List<Proposal> list = new ArrayList<>(entry.getValue());
            list.sort((a, b) -> Integer.compare(order.indexOf(a.tag), order.indexOf(b.tag)));
            Proposal best = list.get(0);
            for (Proposal other : list) {
                if (other.tag.equals(best.tag) && !other.path.equals(best.path)) {
                    conflicts.add(other.tag + ": " + hex(entry.getKey()) + " named `" + best.path + "`, also `"
                        + other.path + "` (" + other.evidence + ")");
                } else if (other.tag.equals(best.tag)) {
                    continue;
                } else if (other.path.equals(best.path)) {
                    agreements.add(hex(entry.getKey()) + " `" + best.path + "`: " + best.tag + " and " + other.tag);
                } else {
                    conflicts.add(hex(entry.getKey()) + ": " + best.tag + " `" + best.path + "` vs " + other.tag
                        + " `" + other.path + "`; kept " + best.tag);
                }
            }
            chosen.put(entry.getKey(), best);
        }
        Map<String, Long> byPath = new HashMap<>();
        for (Proposal proposal : new ArrayList<>(chosen.values())) {
            Long previous = byPath.putIfAbsent(proposal.path, proposal.address);
            if (previous == null) {
                continue;
            }
            Proposal first = chosen.get(previous);
            if (first.tag.equals(TAG_API) && proposal.tag.equals(TAG_API)) {
                conflicts.add("LUA_API: `" + proposal.path + "` names both " + hex(previous) + " and "
                    + hex(proposal.address));
                continue;
            }
            Proposal loser = order.indexOf(proposal.tag) < order.indexOf(first.tag) ? first : proposal;
            Proposal winner = loser == first ? proposal : first;
            conflicts.add("`" + proposal.path + "` proposed for " + hex(winner.address) + " (" + winner.tag + ") and "
                + hex(loser.address) + " (" + loser.tag + "); " + hex(loser.address) + " not named");
            chosen.remove(loser.address);
            byPath.put(proposal.path, winner.address);
        }
        return chosen;
    }

    private void annotate(Map<Long, Proposal> chosen, long helper) throws Exception {
        if (helper >= 0) {
            Proposal helperName = new Proposal(helper, API_NAMESPACE + SCOPE + API_HELPER_NAME, TAG_API,
                "UO Lua API registration helper (callee of most PUSH <name>; PUSH <function> calls); inferred name");
            if (!chosen.containsKey(helper)) {
                apply(helperName);
            }
        }
        for (Proposal proposal : chosen.values()) {
            apply(proposal);
        }
    }

    private Function ensureFunction(long offset, String path) {
        Address address = toAddr(offset);
        Function function = getFunctionAt(address);
        if (function != null) {
            return function;
        }
        Function containing = getFunctionContaining(address);
        if (containing != null) {
            anomalies.add(hex(offset) + " (`" + path + "`) was inside `" + containing.getName(true)
                + "`; function created there");
        }
        if (getInstructionAt(address) == null) {
            disassemble(address);
        }
        function = createFunction(address, null);
        if (function == null) {
            anomalies.add("Could not create function " + hex(offset) + " for `" + path + "`");
            return null;
        }
        count("functions created");
        return function;
    }

    private void apply(Proposal proposal) throws Exception {
        Function function = ensureFunction(proposal.address, proposal.path);
        if (function == null) {
            return;
        }
        int split = proposal.path.lastIndexOf(SCOPE);
        String name = split < 0 ? proposal.path : proposal.path.substring(split + SCOPE.length());
        Namespace namespace = split < 0 ? currentProgram.getGlobalNamespace()
            : namespace(proposal.path.substring(0, split));
        Symbol symbol = function.getSymbol();
        if (!(symbol.getName().equals(name) && symbol.getParentNamespace().equals(namespace))) {
            if (symbol.getSource() == SourceType.DEFAULT) {
                try {
                    symbol.setNameAndNamespace(name, namespace, SourceType.ANALYSIS);
                } catch (DuplicateNameException duplicate) {
                    String unique = name + "_" + Long.toHexString(proposal.address);
                    symbol.setNameAndNamespace(unique, namespace, SourceType.ANALYSIS);
                    anomalies.add("`" + proposal.path + "` already exists; " + hex(proposal.address) + " named `"
                        + unique + "`");
                }
                count("functions renamed");
            } else {
                kept.add(hex(proposal.address) + " `" + symbol.getName(true) + "` (" + symbol.getSource()
                    + "), not renamed to `" + proposal.path + "` (" + proposal.tag + ")");
            }
        }
        boolean tagged = false;
        for (FunctionTag tag : function.getTags()) {
            tagged |= tag.getName().equals(proposal.tag);
        }
        if (!tagged) {
            function.addTag(proposal.tag);
            count("tags added");
        }
        String comment = function.getComment();
        if (comment == null || comment.isBlank()) {
            function.setComment(COMMENT_PREFIX + proposal.tag + " `" + proposal.path + "`: " + proposal.evidence);
            count("plate comments set");
        }
    }

    private Namespace namespace(String path) throws Exception {
        SymbolTable symbols = currentProgram.getSymbolTable();
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

    // ---- UI scripts ----

    private final class UiNames {
        final Set<String> identifiers = new HashSet<>();
        final Map<String, Integer> called = new TreeMap<>();
        final Set<String> defined = new HashSet<>();
        int files;

        UiNames(String root) throws Exception {
            Pattern token = Pattern.compile("[A-Za-z_][A-Za-z0-9_]*|==|~=|<=|>=|\\.\\.\\.|\\.\\.|\\S");
            try (Stream<Path> paths = Files.walk(Path.of(root))) {
                for (Path path : (Iterable<Path>) paths.filter(p -> p.toString().toLowerCase().endsWith(".lua"))::iterator) {
                    files++;
                    String code = stripLua(new String(Files.readAllBytes(path), StandardCharsets.ISO_8859_1));
                    List<String> tokens = new ArrayList<>();
                    Matcher matcher = token.matcher(code);
                    while (matcher.find()) {
                        tokens.add(matcher.group());
                    }
                    scan(tokens);
                }
            }
        }

        private void scan(List<String> t) {
            for (int i = 0; i < t.size(); i++) {
                String s = t.get(i);
                if (!IDENTIFIER.matcher(s).matches() || LUA_KEYWORDS.contains(s)) {
                    continue;
                }
                String previous = i > 0 ? t.get(i - 1) : "";
                String next = i + 1 < t.size() ? t.get(i + 1) : "";
                if (previous.equals(".") || previous.equals(":")) {
                    continue;
                }
                identifiers.add(s);
                if (previous.equals("function") || previous.equals("local") || previous.equals("for")
                        || next.equals("=") || previous.equals(",") && definitionList(t, i)) {
                    defined.add(s);
                }
                if (previous.equals("function") && next.equals("(") || previous.equals("function")
                        && (next.equals(".") || next.equals(":"))) {
                    // Parameters of the definition are locals.
                    int j = i + 1;
                    while (j < t.size() && !t.get(j).equals("(")) {
                        j++;
                    }
                    for (j++; j < t.size() && !t.get(j).equals(")"); j++) {
                        defined.add(t.get(j));
                    }
                }
                if (previous.equals("function") && next.equals("(") && i >= 2 && t.get(i - 2).equals("=")) {
                    continue;
                }
                if (next.equals("(") && !previous.equals("function")) {
                    called.merge(s, 1, Integer::sum);
                }
            }
        }

        // NAME in "local a, NAME" or "for k, NAME in" or "a, NAME = ...".
        private boolean definitionList(List<String> t, int i) {
            for (int j = i - 1; j >= 0; j--) {
                String s = t.get(j);
                if (s.equals("local") || s.equals("for")) {
                    return true;
                }
                if (!s.equals(",") && !IDENTIFIER.matcher(s).matches()) {
                    return false;
                }
            }
            return false;
        }
    }

    // Lua source with comments and string contents removed.
    private static String stripLua(String code) {
        StringBuilder out = new StringBuilder();
        int n = code.length();
        for (int i = 0; i < n; i++) {
            char c = code.charAt(i);
            if (code.startsWith("--", i)) {
                int level = longBracket(code, i + 2);
                int end = level >= 0 ? code.indexOf("]" + "=".repeat(level) + "]", i) : code.indexOf('\n', i);
                i = end < 0 ? n : end + (level >= 0 ? level + 1 : 0);
                out.append(' ');
            } else if (c == '"' || c == '\'') {
                int j = i + 1;
                while (j < n && code.charAt(j) != c && code.charAt(j) != '\n') {
                    j += code.charAt(j) == '\\' ? 2 : 1;
                }
                i = j;
                out.append(" \"\" ");
            } else if (c == '[' && longBracket(code, i) >= 0) {
                int level = longBracket(code, i);
                int end = code.indexOf("]" + "=".repeat(level) + "]", i);
                i = end < 0 ? n : end + level + 1;
                out.append(" \"\" ");
            } else {
                out.append(c);
            }
        }
        return out.toString();
    }

    // Level of a Lua long bracket [==[ at index, or -1.
    private static int longBracket(String code, int index) {
        if (index >= code.length() || code.charAt(index) != '[') {
            return -1;
        }
        int level = 0;
        int i = index + 1;
        while (i < code.length() && code.charAt(i) == '=') {
            level++;
            i++;
        }
        return i < code.length() && code.charAt(i) == '[' ? level : -1;
    }

    // ---- Report ----

    private void report(PrintWriter out, boolean apply, String sourcesPath, long helper, int helperSurvey,
            List<Registration> api, Map<Long, List<Registration>> apiByFunction, List<String> apiUnresolved,
            List<String> secondary, Map<Long, List<Registration>> luaPlusByFunction, List<RegTable> tables,
            long luaRegister, Map<String, Long> openFunctions, long openCluster, long[] range, boolean rangeGiven,
            List<StringEvidence> strings, Map<Long, Proposal> chosen, UiNames ui, Sources sources) {
        Map<String, Set<Long>> apiByName = new TreeMap<>(String.CASE_INSENSITIVE_ORDER);
        for (Registration registration : api) {
            apiByName.computeIfAbsent(registration.name, key -> new TreeSet<>()).add(registration.function);
        }
        int undefined = 0;
        for (long function : apiByFunction.keySet()) {
            undefined += getFunctionAt(toAddr(function)) == null ? 1 : 0;
        }
        Map<String, Integer> byTag = new TreeMap<>();
        for (Proposal proposal : chosen.values()) {
            byTag.merge(proposal.tag, 1, Integer::sum);
        }
        int matchedTables = 0;
        for (RegTable table : tables) {
            matchedTables += table.sourceKey != null ? 1 : 0;
        }
        int accepted = 0;
        for (StringEvidence evidence : strings) {
            accepted += "accepted".equals(evidence.reason) ? 1 : 0;
        }
        out.println("# Lua function names");
        out.println();
        out.println("Program: " + currentProgram.getName());
        out.println("SHA-256: " + currentProgram.getExecutableSHA256());
        out.println("Source map: `" + new File(sourcesPath).getName() + "`");
        out.println("Mode: " + (apply ? "analysis and " + APPLY_FLAG : "analysis only"));
        out.println();
        out.println("| Item | Count |");
        out.println("|---|---:|");
        out.println("| UO API registrations (helper calls and table entries) | " + api.size() + " |");
        out.println("| Distinct UO API names | " + apiByName.size() + " |");
        out.println("| Distinct UO API C functions | " + apiByFunction.size() + " |");
        out.println("| UO API C functions not yet defined as functions | " + undefined + " |");
        out.println("| Unresolved helper call sites | " + apiUnresolved.size() + " |");
        out.println("| LuaPlus global registrations (secondary helpers) | " + luaPlusByFunction.size() + " functions |");
        out.println("| luaL_Reg tables found | " + tables.size() + " |");
        out.println("| luaL_Reg tables matched to a source table | " + matchedTables + " |");
        out.println("| Source strings considered / accepted | " + strings.size() + " / " + accepted + " |");
        for (var entry : byTag.entrySet()) {
            out.println("| Functions named by " + entry.getKey() + " | " + entry.getValue() + " |");
        }
        out.println("| Conflicts | " + conflicts.size() + " |");
        out.println("| Agreements between sources | " + agreements.size() + " |");
        out.println();
        if (apply) {
            out.println("Changes: " + (changes.isEmpty() ? "none" : changes.toString()));
            out.println();
        }
        out.println("Detected functions:");
        out.println();
        out.println("- UO API registration helper (`" + API_NAMESPACE + SCOPE + API_HELPER_NAME + "`, inferred): "
            + hexOrNone(helper) + " (" + helperSurvey + " survey hits)");
        out.println("- `luaL_register`: " + hexOrNone(luaRegister));
        out.println("- Unrolled `luaL_openlibs` cluster: " + hexOrNone(openCluster));
        out.println("- LUA_STRING target range: " + hex(range[0]) + "-" + hex(range[1])
            + (rangeGiven ? " (given)" : " (LUA_REG functions +/- " + hexShort(RANGE_MARGIN) + ")"));
        long low = Long.MAX_VALUE;
        long high = -1;
        for (Proposal proposal : chosen.values()) {
            if (!proposal.tag.equals(TAG_API)) {
                low = Math.min(low, proposal.address);
                high = Math.max(high, proposal.address);
            }
        }
        out.println("- Range of named Lua/LuaPlus functions: " + (high < 0 ? "none" : hex(low) + "-" + hex(high)));

        section(out, "Other registration-like helpers", secondary);
        section(out, "Unresolved registration sites", apiUnresolved);

        List<String> duplicates = new ArrayList<>();
        for (var entry : apiByFunction.entrySet()) {
            Set<String> names = new TreeSet<>();
            entry.getValue().forEach(registration -> names.add(registration.name));
            if (names.size() > 1) {
                duplicates.add(hex(entry.getKey()) + " registered as " + String.join(", ", names));
            }
        }
        for (var entry : apiByName.entrySet()) {
            if (entry.getValue().size() > 1) {
                duplicates.add("`" + entry.getKey() + "` registered for " + hexList(entry.getValue()));
            }
        }
        section(out, "UO API: one function under several names, one name for several functions", duplicates);

        if (ui != null) {
            List<String> lines = new ArrayList<>();
            int used = 0;
            List<String> unused = new ArrayList<>();
            for (String name : apiByName.keySet()) {
                if (ui.identifiers.contains(name)) {
                    used++;
                } else {
                    unused.add(name);
                }
            }
            Set<String> known = new HashSet<>(apiByName.keySet());
            for (List<Registration> list : luaPlusByFunction.values()) {
                list.forEach(registration -> known.add(registration.name));
            }
            for (RegTable table : tables) {
                if ("_G".equals(table.libname)) {
                    known.addAll(table.names);
                }
            }
            known.addAll(sources.baseGlobals);
            List<String> missing = new ArrayList<>();
            for (var entry : ui.called.entrySet()) {
                if (!known.contains(entry.getKey()) && !ui.defined.contains(entry.getKey())) {
                    String hidden = HIDDEN_PREFIX + entry.getKey();
                    missing.add("`" + entry.getKey() + "` (" + entry.getValue() + " calls" + (apiByName.containsKey(hidden)
                        ? "; `" + hidden + "` is registered" : "") + ")");
                }
            }
            lines.add(ui.files + " Lua files scanned");
            lines.add(used + " of " + apiByName.size() + " registered UO API names appear in the UI scripts");
            lines.add(unused.size() + " registered names not referenced by the UI: " + String.join(", ", unused));
            lines.add(missing.size() + " global names called by the UI but neither registered nor defined in the"
                + " scripts: " + String.join(", ", missing));
            section(out, "Validation against the UI scripts", lines);
        }

        List<String> textureNames = new ArrayList<>();
        for (var entry : apiByName.entrySet()) {
            boolean texture = false;
            for (String word : WORD_BREAK.split(entry.getKey())) {
                texture |= TEXTURE_WORDS.contains(word.toLowerCase());
            }
            if (texture) {
                textureNames.add("`" + entry.getKey() + "` " + hexList(entry.getValue()));
            }
        }
        section(out, "Texture, image, icon and art related UO API names", textureNames);

        List<String> apiLines = new ArrayList<>();
        for (var entry : apiByName.entrySet()) {
            Set<Long> sites = new TreeSet<>();
            for (Registration registration : api) {
                if (registration.name.equals(entry.getKey())) {
                    sites.add(registration.site);
                }
            }
            apiLines.add("`" + entry.getKey() + "` " + hexList(entry.getValue()) + " (registered at " + hexList(sites) + ")");
        }
        section(out, "UO API names", apiLines);

        List<String> luaPlusLines = new ArrayList<>();
        for (var entry : luaPlusByFunction.entrySet()) {
            for (Registration registration : entry.getValue()) {
                luaPlusLines.add("`" + registration.name + "` " + hex(entry.getKey()) + " (at " + hex(registration.site)
                    + ", " + registration.via + ")");
            }
        }
        section(out, "LuaPlus global registrations", luaPlusLines);

        List<String> tableLines = new ArrayList<>();
        for (RegTable table : tables) {
            SourceTable source = table.sourceKey != null ? sources.tables.get(table.sourceKey) : null;
            tableLines.add(hex(table.address) + " " + table.names.size() + " entries, libname "
                + (table.libname == null ? "unknown" : table.libname.isEmpty() ? "NULL" : "`" + table.libname + "`")
                + ", registered at " + hexList(table.refs) + ": " + (source != null ? "`" + source.table + "` ("
                + source.file + ") score " + score(table.score) + (table.primary ? "" : " (not the best match)")
                : "no source table") + "; entries " + String.join(" ", table.names));
        }
        section(out, "luaL_Reg tables", tableLines);

        List<String> openLines = new ArrayList<>();
        openFunctions.forEach((name, address) -> openLines.add("`" + name + "` " + hex(address)));
        section(out, "Unrolled luaL_openlibs", openLines);

        Map<String, List<String>> named = new TreeMap<>();
        for (Proposal proposal : chosen.values()) {
            if (!proposal.tag.equals(TAG_API)) {
                named.computeIfAbsent(proposal.tag, key -> new ArrayList<>()).add("`" + proposal.path + "` "
                    + hex(proposal.address) + ": " + proposal.evidence);
            }
        }
        for (var entry : named.entrySet()) {
            List<String> lines = entry.getValue();
            lines.sort(String::compareTo);
            section(out, "Lua function names (" + entry.getKey() + ")", lines);
        }

        Map<String, Integer> reasons = new TreeMap<>();
        for (StringEvidence evidence : strings) {
            String reason = evidence.reason == null ? "unknown" : evidence.reason.replaceAll(" at .*| 0x\\w+.*|\\d+ ", "");
            reasons.merge(reason, 1, Integer::sum);
        }
        List<String> reasonLines = new ArrayList<>();
        reasons.forEach((reason, n) -> reasonLines.add(reason + ": " + n));
        section(out, "LUA_STRING candidate outcomes", reasonLines);

        section(out, "Conflicts", conflicts);
        section(out, "Agreements between sources", agreements);
        section(out, "Kept names (not DEFAULT)", kept);
        section(out, "Anomalies", anomalies);
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

    // ---- Instruction decoding ----

    // Instructions ending at site: the listing if the site is disassembled, otherwise the
    // pseudo-disassembly from the lowest start in the window that lands exactly on the site.
    private List<Instruction> decodeBefore(long site, int window) {
        Instruction real = getInstructionAt(toAddr(site));
        if (real != null) {
            LinkedList<Instruction> list = new LinkedList<>();
            for (Instruction p = real.getPrevious(); p != null && p.getAddress().getOffset() >= site - window;
                    p = p.getPrevious()) {
                list.addFirst(p);
            }
            return list;
        }
        for (long start = site - window; start < site; start++) {
            List<Instruction> list = new ArrayList<>();
            long address = start;
            while (address < site) {
                Instruction instruction = decode(address);
                if (instruction == null) {
                    break;
                }
                list.add(instruction);
                address += instruction.getLength();
            }
            if (address == site) {
                return list;
            }
        }
        return null;
    }

    private Instruction decode(long address) {
        try {
            return pseudo.disassemble(toAddr(address));
        } catch (Exception undecodable) {
            return null;
        }
    }

    // Arguments of the call at site: [0] is the last push (first argument).
    private List<Arg> pushedArgs(long site, int count) {
        List<Arg> args = new ArrayList<>();
        List<Instruction> window = decodeBefore(site, ARGUMENT_WINDOW);
        if (window == null) {
            return args;
        }
        for (int k = window.size() - 1; k >= 0 && args.size() < count; k--) {
            Instruction instruction = window.get(k);
            if (instruction.getFlowType().isCall() || instruction.getFlowType().isJump()
                    || instruction.getFlowType().isTerminal()) {
                break;
            }
            if (instruction.getMnemonicString().equals("PUSH")) {
                args.add(operandArg(instruction, 0, window, k));
            }
        }
        return args;
    }

    // Value of an operand: an immediate, a register loaded with an immediate earlier in the
    // window, or a memory operand [reg + offset] with reg loaded with an immediate.
    private Arg operandArg(Instruction instruction, int operand, List<Instruction> window, int index) {
        Arg arg = new Arg();
        arg.text = instruction.getDefaultOperandRepresentation(operand);
        Object[] objects = instruction.getOpObjects(operand);
        if (!isMemory(instruction, operand)) {
            if (objects.length == 1 && objects[0] instanceof Scalar scalar) {
                arg.value = scalar.getUnsignedValue();
            } else if (objects.length == 1 && objects[0] instanceof Register register) {
                arg.value = registerConstant(register, window, index);
            }
            return arg;
        }
        Register base = null;
        long offset = 0;
        for (Object object : objects) {
            if (object instanceof Register register) {
                base = base == null ? register : null;
            } else if (object instanceof Scalar scalar) {
                offset = scalar.getSignedValue();
            }
        }
        long baseValue = base != null ? registerConstant(base, window, index) : -1;
        if (baseValue >= 0) {
            arg.tableBase = baseValue;
            arg.tableOffset = offset;
        }
        return arg;
    }

    private long registerConstant(Register register, List<Instruction> window, int index) {
        if (window == null) {
            return -1;
        }
        for (int q = Math.min(index, window.size()) - 1; q >= 0; q--) {
            Instruction write = window.get(q);
            Register target = write.getRegister(0);
            if (target == null || !target.contains(register) && !register.contains(target)) {
                continue;
            }
            if (write.getMnemonicString().equals("MOV") && !isMemory(write, 1) && write.getScalar(1) != null) {
                return write.getScalar(1).getUnsignedValue();
            }
            if (write.getMnemonicString().equals("PUSH") || write.getMnemonicString().equals("CMP")
                    || write.getMnemonicString().equals("TEST")) {
                continue;
            }
            return -1;
        }
        return -1;
    }

    private static boolean isMemory(Instruction instruction, int operand) {
        return instruction.getDefaultOperandRepresentation(operand).contains("[");
    }

    private List<Long> callSites(long target) {
        List<Long> sites = new ArrayList<>();
        byte[] b = textBlock.bytes;
        for (int j = 0; j + IMM_INSTRUCTION_LENGTH <= b.length; j++) {
            if ((b[j] & BYTE_MASK) == OPCODE_CALL_REL && callTarget(j) == target) {
                sites.add(textBlock.start + j);
            }
        }
        return sites;
    }

    private long callTarget(int index) {
        return (textBlock.start + index + IMM_INSTRUCTION_LENGTH + (int) le32(textBlock.bytes, index + OPCODE_LENGTH))
            & DWORD_MASK;
    }

    // ---- Memory helpers ----

    private void loadBlocks() throws Exception {
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
            if (copy.exec && (textBlock == null || copy.bytes.length > textBlock.bytes.length)) {
                textBlock = copy;
            }
        }
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
        Block block = address < 0 ? null : blockOf(address);
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
        Block block = address <= 0 ? null : blockOf(address);
        return block != null && block.exec;
    }

    private static long alignUp(long address) {
        return (address + POINTER_SIZE - 1) / POINTER_SIZE * POINTER_SIZE;
    }

    // A NUL-terminated printable string (possibly empty) in non-executable memory, or null.
    private String stringAt(long address) {
        Block block = address <= 0 ? null : blockOf(address);
        if (block == null || block.exec) {
            return null;
        }
        StringBuilder text = new StringBuilder();
        for (int index = (int) (address - block.start); index < block.bytes.length; index++) {
            int c = block.bytes[index] & BYTE_MASK;
            if (c == 0) {
                return text.toString();
            }
            if (c < PRINTABLE_FIRST || c > PRINTABLE_LAST || text.length() >= MAX_STRING_LENGTH) {
                return null;
            }
            text.append((char) c);
        }
        return null;
    }

    // A Lua-style global name: a non-empty identifier of bounded length.
    private String identifierAt(long address) {
        String text = stringAt(address);
        return text != null && text.length() <= MAX_IDENTIFIER_LENGTH && IDENTIFIER.matcher(text).matches() ? text : null;
    }

    private String apiNameAt(long address) {
        String text = stringAt(address);
        return text != null && text.length() <= MAX_IDENTIFIER_LENGTH && API_NAME.matcher(text).matches() ? text : null;
    }

    private String functionLabel(long address) {
        Function function = getFunctionContaining(toAddr(address));
        return function != null ? "`" + function.getName(true) + "`" : "code outside functions";
    }

    private static String sanitize(String name) {
        return name.replaceAll("[^A-Za-z0-9_]", "_");
    }

    private static String printable(String text) {
        StringBuilder out = new StringBuilder();
        for (char c : text.toCharArray()) {
            out.append(c == '\n' ? "\\n" : c == '\t' ? "\\t" : c == '`' ? "'" : c < PRINTABLE_FIRST || c > PRINTABLE_LAST
                ? String.format("\\x%02x", (int) c) : String.valueOf(c));
        }
        return out.toString();
    }

    private static long mostCommon(Map<Long, Integer> counts) {
        long best = -1;
        int bestCount = 0;
        for (var entry : counts.entrySet()) {
            if (entry.getValue() > bestCount) {
                best = entry.getKey();
                bestCount = entry.getValue();
            }
        }
        return best;
    }

    private static String describe(List<Arg> args) {
        List<String> parts = new ArrayList<>();
        for (Arg arg : args) {
            parts.add("`" + arg.text + "`");
        }
        return parts.isEmpty() ? "not decoded" : String.join(", ", parts);
    }

    private static String decodePath(String path) {
        return path.replace(ENCODED_SPACE, " ");
    }

    private static long[] parseRange(String text) {
        String[] parts = text.split("-");
        return new long[] {Long.decode(parts[0]), Long.decode(parts[1])};
    }

    private static String score(double score) {
        return String.format("%.2f", score);
    }

    private static String hex(long value) {
        return String.format("`0x%08x`", value);
    }

    private static String hexShort(long value) {
        return String.format("0x%08x", value);
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
