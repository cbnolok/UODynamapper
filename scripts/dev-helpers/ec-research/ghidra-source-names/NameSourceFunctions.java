// Name statically linked open-source library functions from their sources: function pointer
// tables (TABLE), MSVC vtables (VTABLE), string literals unique to one source function (STRING)
// and unique call graph neighbours of named functions (CALLGRAPH); with --apply, rename DEFAULT
// functions, add tags and plate comments.
// @category ECResearch
import ghidra.app.script.GhidraScript;
import ghidra.program.model.address.Address;
import ghidra.program.model.listing.Function;
import ghidra.program.model.listing.FunctionTag;
import ghidra.program.model.listing.Instruction;
import ghidra.program.model.mem.MemoryBlock;
import ghidra.program.model.scalar.Scalar;
import ghidra.program.model.symbol.Namespace;
import ghidra.program.model.symbol.SourceType;
import ghidra.program.model.symbol.Symbol;
import ghidra.program.model.symbol.SymbolIterator;
import ghidra.program.model.symbol.SymbolTable;
import ghidra.util.exception.DuplicateNameException;
import java.io.PrintWriter;
import java.nio.charset.StandardCharsets;
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
import java.util.regex.Pattern;

public class NameSourceFunctions extends GhidraScript {
    private static final String APPLY_FLAG = "--apply";
    private static final String RANGE_OPTION = "--range";
    // Script arguments are split on whitespace, so a path with spaces is passed with %20.
    private static final String ENCODED_SPACE = "%20";
    private static final String TABLE = "TABLE";
    private static final String VTABLE = "VTABLE";
    private static final String STRING = "STRING";
    private static final String CALLGRAPH = "CALLGRAPH";
    // Method priority: a stronger method wins at one address.
    private static final List<String> METHODS = List.of(TABLE, VTABLE, STRING, CALLGRAPH);
    private static final String COMMENT_PREFIX = "NameSourceFunctions: ";
    private static final String SCOPE = "::";
    private static final String VFTABLE = "vftable";
    private static final Set<String> PURECALL_NAMES = Set.of("_purecall", "__purecall", "purecall");
    private static final String SCALAR_DELETING_DTOR = "`scalar_deleting_destructor'";
    private static final String VECTOR_DELETING_DTOR = "`vector_deleting_destructor'";
    private static final String LIBRARY_HEADER = "# Library: ";
    // Namespaces dropped when comparing class names (template arguments are written unqualified
    // in the map).
    private static final String STD_NAMESPACE = "std";
    private static final Pattern CLASS_KEY = Pattern.compile("\\b(class|struct|union|enum)[ _]");
    private static final Pattern NAME_NOISE = Pattern.compile("[\\s_]");

    private static final int POINTER_SIZE = 4;
    private static final int BYTE_MASK = 0xff;
    private static final int BITS_PER_BYTE = 8;
    private static final int MAX_STRING_LENGTH = 512;
    private static final int PRINTABLE_FIRST = 0x20;
    private static final int PRINTABLE_LAST = 0x7e;
    private static final int MAX_VTABLE_SLOTS = 512;
    // MSVC RTTI: CompleteObjectLocator {signature, offset, cdOffset, pTypeDescriptor,
    // pClassDescriptor}; ClassHierarchyDescriptor {signature, attributes, numBaseClasses,
    // pBaseClassArray}; BaseClassDescriptor {pTypeDescriptor, ...}; TypeDescriptor {pVFTable,
    // spare, name[]}.
    private static final int COL_OFFSET_FIELD = 4;
    private static final int COL_CLASS_DESCRIPTOR_FIELD = 16;
    private static final int CHD_BASE_COUNT_FIELD = 8;
    private static final int CHD_BASE_ARRAY_FIELD = 12;
    private static final int TYPE_DESCRIPTOR_NAME_FIELD = 8;
    private static final int MAX_BASE_CLASSES = 64;
    private static final String MANGLED_CLASS_PREFIX = ".?AV";
    private static final String MANGLED_STRUCT_PREFIX = ".?AU";
    private static final String MANGLED_SCOPE_END = "@@";
    private static final String MANGLED_SEPARATOR = "@";
    // A deleting destructor that tests bit 1 of its flags argument handles delete[] (vector).
    private static final int MAX_DTOR_INSTRUCTIONS = 12;
    private static final long VECTOR_DELETE_FLAG = 2;
    // STRING evidence: literal length and letter count, format specifiers not counted.
    private static final int MIN_STRING_LENGTH = 6;
    private static final int MIN_STRING_LETTERS = 4;
    private static final Pattern FORMAT_SPEC = Pattern.compile("%[-+ #0-9.]*[a-zA-Z%]");
    // TABLE evidence: checked fields (strings, pointers, constants) of an accepted table.
    private static final int MIN_TABLE_CHECKS = 2;
    private static final int MAX_ROUNDS = 64;
    private static final int HEX_RADIX = 16;

    private static final class Block {
        long start;
        byte[] bytes;
        boolean exec;
    }

    private static final class Field {
        int offset;
        int size;
        String kind;
        String value;
    }

    private static final class TableVariant {
        String name;
        int index;
        int size;
        String type;
        String conditions;
        final List<Field> fields = new ArrayList<>();
    }

    private static final class Slot {
        String owner;
        String method;
        boolean pure;
        boolean dtor;
        String signature;
    }

    private static final class VtVariant {
        String cls;
        int index;
        int count;
        String conditions;
        final List<Slot> slots = new ArrayList<>();
    }

    private static final class Lib {
        String name;
        String version;
        String tag;
        String path;
        final Map<String, Integer> definitions = new HashMap<>();
        final Set<String> inline = new HashSet<>();
        final List<String[]> strings = new ArrayList<>();
        final Map<String, Set<String>> calls = new HashMap<>();
        final Map<String, Set<String>> callers = new HashMap<>();
        final Map<String, List<TableVariant>> tables = new LinkedHashMap<>();
        final Map<String, List<VtVariant>> vtables = new LinkedHashMap<>();
        final Map<String, Set<String>> slotSignatures = new HashMap<>();
        final Set<String> namespaces = new TreeSet<>();
        final Set<String> globalClasses = new HashSet<>();

        String tag(String method) {
            return tag + "_" + method;
        }

        boolean overloaded(String name) {
            return definitions.getOrDefault(name, 0) > 1 || slotSignatures.getOrDefault(name, Set.of()).size() > 1;
        }

        boolean defined(String name) {
            return definitions.containsKey(name) || slotSignatures.containsKey(name);
        }
    }

    private static final class ProgramVtable {
        long address;
        Namespace namespace;
        String path;
        long colOffset = -1;
        final List<Long> slots = new ArrayList<>();
        final List<String> bases = new ArrayList<>();
    }

    private static final class Proposal {
        long address;
        String path;
        String tag;
        String method;
        Lib lib;
        String evidence;
        Namespace namespace;
    }

    private static final class StringEvidence {
        String text;
        Lib lib;
        String function;
        String file;
        final Set<Long> copies = new TreeSet<>();
        final Set<Long> functions = new TreeSet<>();
        final Set<Long> orphanSites = new TreeSet<>();
        int dataRefs;
        String reason;
    }

    private static final class TableHit {
        Lib lib;
        TableVariant variant;
        long base;
        int checks;
        String anchor;
        final Map<Integer, Long> pointers = new TreeMap<>();
    }

    private final List<Block> blocks = new ArrayList<>();
    private Block lastBlock;
    private final List<Lib> libs = new ArrayList<>();
    private final Map<String, List<Long>> copiesByText = new HashMap<>();
    private final Map<Long, List<Long>> dataPointers = new HashMap<>();
    private final List<ProgramVtable> programVtables = new ArrayList<>();
    private long purecall = -1;
    private final Map<Long, List<Proposal>> proposals = new TreeMap<>();
    private final List<TableHit> tableHits = new ArrayList<>();
    private final Set<String> tableVisits = new HashSet<>();
    private final List<String> tableRejects = new ArrayList<>();
    private final List<String> vtableMatches = new ArrayList<>();
    private final List<String> vtableRejects = new ArrayList<>();
    private final Map<String, Integer> vtableMissing = new TreeMap<>();
    private final List<StringEvidence> stringEvidence = new ArrayList<>();
    private final List<String> callgraphLog = new ArrayList<>();
    private final Map<String, Integer> alreadyNamed = new TreeMap<>();
    private final List<String> anomalies = new ArrayList<>();
    private final List<String> conflicts = new ArrayList<>();
    // Conflicts found by resolve(), which runs again after CALLGRAPH.
    private final List<String> resolveConflicts = new ArrayList<>();
    private final List<String> agreements = new ArrayList<>();
    private final List<String> kept = new ArrayList<>();
    private final Map<String, Integer> changes = new LinkedHashMap<>();

    public void run() throws Exception {
        String[] args = getScriptArgs();
        if (args.length < 2) {
            throw new IllegalArgumentException("Expected OUTPUT_FILE MAP.tsv [MAP.tsv...] [" + APPLY_FLAG + "] ["
                + RANGE_OPTION + " 0xSTART-0xEND]");
        }
        boolean apply = false;
        long[] range = null;
        List<String> maps = new ArrayList<>();
        for (int i = 1; i < args.length; i++) {
            switch (args[i]) {
                case APPLY_FLAG -> apply = true;
                case RANGE_OPTION -> range = parseRange(args[++i]);
                default -> maps.add(decodePath(args[i]));
            }
        }
        for (String map : maps) {
            libs.add(loadMap(map));
        }
        // Created before any change, so an existing output file stops the run unchanged.
        try (PrintWriter output = new PrintWriter(Files.newBufferedWriter(Path.of(decodePath(args[0])),
                StandardOpenOption.CREATE_NEW))) {
            analyze(output, apply, range);
        }
        println("Wrote " + args[0] + (apply ? " (applied: " + changes + ")" : ""));
    }

    private void analyze(PrintWriter output, boolean apply, long[] range) throws Exception {
        loadBlocks();
        purecall = findPurecall();
        indexStrings();
        indexDataPointers();

        // 1. TABLE: tables anchored by their strings, then the tables they point to.
        for (Lib lib : libs) {
            for (List<TableVariant> variants : lib.tables.values()) {
                anchorByStrings(lib, variants);
            }
        }
        // 2. VTABLE: RTTI vtables of the map's classes.
        collectVtables();
        matchVtables(apply);
        if (apply) {
            createProposedFunctions(TABLE);
        }
        // 3. STRING: unique literals referenced by exactly one function.
        matchStrings(range);
        // 4. TABLE again: tables anchored by functions named so far.
        anchorByFunctions();
        if (apply) {
            createProposedFunctions(TABLE);
        }
        Map<Long, Proposal> chosen = resolve();
        // 5. CALLGRAPH: unique unnamed neighbours of named functions.
        callgraph(chosen, range);
        chosen = resolve();
        if (apply) {
            for (Proposal proposal : chosen.values()) {
                apply(proposal);
            }
        }
        report(output, apply, chosen);
    }

    // ---- Maps ----

    private Lib loadMap(String path) throws Exception {
        Lib lib = new Lib();
        lib.path = path;
        Map<String, TableVariant> tableByKey = new HashMap<>();
        Map<String, VtVariant> vtableByKey = new HashMap<>();
        for (String line : Files.readAllLines(Path.of(path), StandardCharsets.US_ASCII)) {
            if (line.startsWith(LIBRARY_HEADER)) {
                String[] parts = line.substring(LIBRARY_HEADER.length()).trim().split(" ", 2);
                lib.name = parts[0];
                lib.version = parts.length > 1 ? parts[1] : "";
                lib.tag = parts[0].toUpperCase().replaceAll("[^A-Z0-9]", "");
                continue;
            }
            if (line.startsWith("#") || line.isBlank()) {
                continue;
            }
            String[] f = line.split("\t", -1);
            for (int i = 0; i < f.length; i++) {
                f[i] = unescapeField(f[i]);
            }
            switch (f[0]) {
                case "FUNC" -> {
                    lib.definitions.merge(f[2], 1, Integer::sum);
                    if (f[3].contains("i")) {
                        lib.inline.add(f[2]);
                    }
                }
                case "STRING" -> lib.strings.add(new String[] {f[1], f[2], f[3]});
                case "CALL" -> {
                    lib.calls.computeIfAbsent(f[2], key -> new TreeSet<>()).add(f[3]);
                    lib.callers.computeIfAbsent(f[3], key -> new TreeSet<>()).add(f[2]);
                }
                case "TABLE" -> {
                    TableVariant variant = new TableVariant();
                    variant.name = f[1];
                    variant.index = Integer.parseInt(f[2]);
                    variant.size = Integer.parseInt(f[3]);
                    variant.type = f[4];
                    variant.conditions = f[5];
                    tableByKey.put(f[1] + "\t" + f[2], variant);
                    lib.tables.computeIfAbsent(f[1], key -> new ArrayList<>()).add(variant);
                }
                case "FIELD" -> {
                    Field field = new Field();
                    field.offset = Integer.parseInt(f[3]);
                    field.size = Integer.parseInt(f[4]);
                    field.kind = f[5];
                    field.value = f[6];
                    tableByKey.get(f[1] + "\t" + f[2]).fields.add(field);
                }
                case "VTABLE" -> {
                    VtVariant variant = new VtVariant();
                    variant.cls = f[1];
                    variant.index = Integer.parseInt(f[2]);
                    variant.count = Integer.parseInt(f[3]);
                    variant.conditions = f[4];
                    vtableByKey.put(f[1] + "\t" + f[2], variant);
                    lib.vtables.computeIfAbsent(f[1], key -> new ArrayList<>()).add(variant);
                    int scope = topLevelScope(f[1]);
                    if (scope >= 0) {
                        lib.namespaces.add(f[1].substring(0, f[1].indexOf(SCOPE)));
                    } else {
                        lib.globalClasses.add(f[1]);
                    }
                }
                case "SLOT" -> {
                    Slot slot = new Slot();
                    int split = topLevelScope(f[4]);
                    slot.owner = f[4].substring(0, split);
                    slot.method = f[4].substring(split + SCOPE.length());
                    slot.pure = f[5].contains("p");
                    slot.dtor = f[5].contains("d");
                    slot.signature = f[6];
                    vtableByKey.get(f[1] + "\t" + f[2]).slots.add(slot);
                    if (!slot.dtor) {
                        lib.slotSignatures.computeIfAbsent(f[4], key -> new HashSet<>()).add(f[6]);
                    }
                }
                default -> throw new IllegalArgumentException("Unknown record " + f[0] + " in " + path);
            }
        }
        if (lib.name == null) {
            throw new IllegalArgumentException(path + " has no " + LIBRARY_HEADER.trim() + " header");
        }
        return lib;
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

    // Index of the last "::" outside template brackets, or -1.
    private static int topLevelScope(String path) {
        int depth = 0;
        int found = -1;
        for (int i = 0; i + 1 < path.length(); i++) {
            char c = path.charAt(i);
            if (c == '<') {
                depth++;
            } else if (c == '>') {
                depth--;
            } else if (depth == 0 && c == ':' && path.charAt(i + 1) == ':') {
                found = i;
                i++;
            }
        }
        return found;
    }

    private static List<String> splitPath(String path) {
        List<String> parts = new ArrayList<>();
        int depth = 0;
        int start = 0;
        for (int i = 0; i < path.length(); i++) {
            char c = path.charAt(i);
            if (c == '<') {
                depth++;
            } else if (c == '>') {
                depth--;
            } else if (depth == 0 && c == ':' && i + 1 < path.length() && path.charAt(i + 1) == ':') {
                parts.add(path.substring(start, i));
                start = i + 2;
                i++;
            }
        }
        parts.add(path.substring(start));
        return parts;
    }

    // ---- Program indexes ----

    private void indexStrings() {
        Set<String> wanted = new HashSet<>();
        for (Lib lib : libs) {
            for (String[] string : lib.strings) {
                wanted.add(string[2]);
            }
            for (List<TableVariant> variants : lib.tables.values()) {
                for (TableVariant variant : variants) {
                    for (Field field : variant.fields) {
                        if (field.kind.equals("str")) {
                            wanted.add(field.value);
                        }
                    }
                }
            }
        }
        // NUL-terminated copies starting after a NUL.
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
                    String text = new String(b, i, end - i, StandardCharsets.ISO_8859_1);
                    if (wanted.contains(text)) {
                        copiesByText.computeIfAbsent(text, key -> new ArrayList<>()).add(block.start + i);
                    }
                }
                i = end;
            }
        }
    }

    // Aligned pointers in data to code or to a wanted string copy.
    private void indexDataPointers() {
        Set<Long> copies = new HashSet<>();
        copiesByText.values().forEach(copies::addAll);
        for (Block block : blocks) {
            if (block.exec) {
                continue;
            }
            byte[] b = block.bytes;
            for (int i = 0; i + POINTER_SIZE <= b.length; i += POINTER_SIZE) {
                long value = le32(b, i);
                if (copies.contains(value) || isExec(value)) {
                    dataPointers.computeIfAbsent(value, key -> new ArrayList<>()).add(block.start + i);
                }
            }
        }
    }

    private long findPurecall() {
        for (String name : PURECALL_NAMES) {
            for (Function function : getGlobalFunctions(name)) {
                return function.getEntryPoint().getOffset();
            }
        }
        anomalies.add("No `_purecall` function found; pure vtable slots are not checked");
        return -1;
    }

    // ---- TABLE ----

    private void anchorByStrings(Lib lib, List<TableVariant> variants) {
        for (TableVariant variant : variants) {
            for (Field field : variant.fields) {
                if (!field.kind.equals("str")) {
                    continue;
                }
                for (long copy : copiesByText.getOrDefault(field.value, List.of())) {
                    for (long pointer : dataPointers.getOrDefault(copy, List.of())) {
                        tryTable(lib, variant.name, pointer - field.offset, "string \"" + printable(field.value) + "\"");
                    }
                }
                break;
            }
        }
    }

    private void anchorByFunctions() {
        for (int round = 0; round < MAX_ROUNDS; round++) {
            int before = tableHits.size();
            for (Lib lib : libs) {
                for (var entry : lib.tables.entrySet()) {
                    if (hasTableHit(lib, entry.getKey())) {
                        continue;
                    }
                    for (TableVariant variant : entry.getValue()) {
                        for (Field field : variant.fields) {
                            if (!field.kind.equals("func")) {
                                continue;
                            }
                            long address = proposedAddress(lib, field.value);
                            if (address < 0) {
                                continue;
                            }
                            for (long pointer : dataPointers.getOrDefault(address, List.of())) {
                                tryTable(lib, entry.getKey(), pointer - field.offset, "function `" + field.value + "`");
                            }
                        }
                    }
                }
            }
            if (tableHits.size() == before) {
                break;
            }
        }
    }

    private boolean hasTableHit(Lib lib, String name) {
        for (TableHit hit : tableHits) {
            if (hit.lib == lib && hit.variant.name.equals(name)) {
                return true;
            }
        }
        return false;
    }

    // The single address a source function is proposed at, or -1.
    private long proposedAddress(Lib lib, String name) {
        Set<Long> found = new TreeSet<>();
        for (var entry : proposals.entrySet()) {
            for (Proposal proposal : entry.getValue()) {
                if (proposal.lib == lib && proposal.path.equals(name)) {
                    found.add(entry.getKey());
                }
            }
        }
        return found.size() == 1 && !lib.overloaded(name) ? found.iterator().next() : -1;
    }

    // Validate every variant of a table at base; accept the one with most checked fields.
    private void tryTable(Lib lib, String name, long base, String anchor) {
        if (!tableVisits.add(lib.name + "\t" + name + "\t" + base)) {
            return;
        }
        List<TableVariant> variants = lib.tables.get(name);
        if (variants == null) {
            return;
        }
        TableHit best = null;
        boolean ambiguous = false;
        for (TableVariant variant : variants) {
            TableHit hit = checkTable(lib, variant, base);
            if (hit == null || hit.checks < MIN_TABLE_CHECKS) {
                continue;
            }
            if (best == null || hit.checks > best.checks) {
                best = hit;
                ambiguous = false;
            } else if (hit.checks == best.checks && !sameFunctions(hit, best)) {
                ambiguous = true;
            }
        }
        if (best == null) {
            return;
        }
        if (ambiguous) {
            tableRejects.add("`" + name + "` at " + hex(base) + ": several variants fit with different functions");
            return;
        }
        best.anchor = anchor;
        tableHits.add(best);
        for (Field field : best.variant.fields) {
            long value = best.pointers.getOrDefault(field.offset, -1L);
            if (field.kind.equals("func")) {
                propose(value, field.value, lib, TABLE, "`" + shortTable(name) + "` +0x"
                    + Integer.toHexString(field.offset) + " at " + hex(base) + " (" + fileOf(name) + ")", null);
            } else if (field.kind.equals("ref")) {
                tryTable(lib, field.value, value, "pointer from `" + shortTable(name) + "`");
            }
        }
    }

    private static boolean sameFunctions(TableHit a, TableHit b) {
        Map<Long, String> left = new TreeMap<>();
        Map<Long, String> right = new TreeMap<>();
        for (Field field : a.variant.fields) {
            if (field.kind.equals("func")) {
                left.put(a.pointers.get(field.offset), field.value);
            }
        }
        for (Field field : b.variant.fields) {
            if (field.kind.equals("func")) {
                right.put(b.pointers.get(field.offset), field.value);
            }
        }
        return left.equals(right);
    }

    private TableHit checkTable(Lib lib, TableVariant variant, long base) {
        Block block = blockOf(base);
        if (block == null || block.exec || base + variant.size > block.start + block.bytes.length) {
            return null;
        }
        TableHit hit = new TableHit();
        hit.lib = lib;
        hit.variant = variant;
        hit.base = base;
        for (Field field : variant.fields) {
            long at = base + field.offset;
            switch (field.kind) {
                case "str" -> {
                    long pointer = u32(at);
                    if (!field.value.equals(stringAt(pointer))) {
                        return null;
                    }
                }
                case "func" -> {
                    long pointer = u32(at);
                    if (!isExec(pointer)) {
                        return null;
                    }
                    hit.pointers.put(field.offset, pointer);
                }
                case "ref" -> {
                    long pointer = u32(at);
                    Block target = pointer > 0 ? blockOf(pointer) : null;
                    if (target == null || target.exec) {
                        return null;
                    }
                    hit.pointers.put(field.offset, pointer);
                }
                case "null", "int" -> {
                    long expected = field.kind.equals("null") ? 0 : Long.parseLong(field.value);
                    if (readUnsigned(at, field.size) != (expected & sizeMask(field.size))) {
                        return null;
                    }
                }
                default -> {
                    continue;
                }
            }
            hit.checks++;
        }
        return hit;
    }

    private static String shortTable(String name) {
        return name.substring(name.indexOf(':') + 1);
    }

    private static String fileOf(String name) {
        return name.substring(0, name.indexOf(':'));
    }

    // ---- VTABLE ----

    private void collectVtables() {
        SymbolTable symbols = currentProgram.getSymbolTable();
        SymbolIterator iterator = symbols.getSymbols(VFTABLE);
        while (iterator.hasNext()) {
            Symbol symbol = iterator.next();
            ProgramVtable vtable = new ProgramVtable();
            vtable.address = symbol.getAddress().getOffset();
            vtable.namespace = symbol.getParentNamespace();
            vtable.path = vtable.namespace.getName(true);
            for (int i = 0; i < MAX_VTABLE_SLOTS; i++) {
                long at = vtable.address + (long) i * POINTER_SIZE;
                if (i > 0 && symbols.getPrimarySymbol(toAddr(at)) != null) {
                    break;
                }
                long value = u32(at);
                if (!isExec(value)) {
                    break;
                }
                vtable.slots.add(value);
            }
            long locator = u32(vtable.address - POINTER_SIZE);
            if (locator > 0 && blockOf(locator) != null && !isExec(locator)) {
                vtable.colOffset = u32(locator + COL_OFFSET_FIELD);
                long hierarchy = u32(locator + COL_CLASS_DESCRIPTOR_FIELD);
                long count = u32(hierarchy + CHD_BASE_COUNT_FIELD);
                long array = u32(hierarchy + CHD_BASE_ARRAY_FIELD);
                for (int k = 0; k < count && k < MAX_BASE_CLASSES && array > 0; k++) {
                    long descriptor = u32(u32(array + (long) k * POINTER_SIZE));
                    String name = descriptor > 0 ? stringAt(descriptor + TYPE_DESCRIPTOR_NAME_FIELD) : null;
                    if (name != null) {
                        vtable.bases.add(name);
                    }
                }
            }
            programVtables.add(vtable);
        }
    }

    private String canonical(String name, Lib lib) {
        String text = CLASS_KEY.matcher(name).replaceAll("");
        for (String namespace : lib.namespaces) {
            text = text.replace(namespace + SCOPE, "");
        }
        text = text.replace(STD_NAMESPACE + SCOPE, "");
        return NAME_NOISE.matcher(text).replaceAll("");
    }

    // A vtable of the library's own classes or of a class derived from one.
    private boolean owned(ProgramVtable vtable, Lib lib) {
        if (lib.vtables.isEmpty()) {
            return false;
        }
        for (String namespace : lib.namespaces) {
            if (vtable.path.startsWith(namespace + SCOPE)) {
                return true;
            }
        }
        for (String base : vtable.bases) {
            for (String namespace : lib.namespaces) {
                if (base.contains(MANGLED_SEPARATOR + namespace + MANGLED_SCOPE_END)) {
                    return true;
                }
            }
            for (String cls : lib.globalClasses) {
                if (base.equals(MANGLED_CLASS_PREFIX + cls + MANGLED_SCOPE_END)
                        || base.equals(MANGLED_STRUCT_PREFIX + cls + MANGLED_SCOPE_END)) {
                    return true;
                }
            }
        }
        return lib.globalClasses.contains(vtable.path);
    }

    private void matchVtables(boolean apply) {
        // Functions referenced by vtables of classes outside every library: possibly folded
        // (/OPT:ICF) with library functions, so not named from a library vtable.
        Map<Long, String> foreign = new HashMap<>();
        for (ProgramVtable vtable : programVtables) {
            boolean libraryVtable = false;
            for (Lib lib : libs) {
                libraryVtable |= owned(vtable, lib);
            }
            if (!libraryVtable) {
                for (long slot : vtable.slots) {
                    foreign.putIfAbsent(slot, vtable.path);
                }
            }
        }
        for (Lib lib : libs) {
            Map<String, List<ProgramVtable>> byClass = new HashMap<>();
            for (ProgramVtable vtable : programVtables) {
                byClass.computeIfAbsent(canonical(vtable.path, lib), key -> new ArrayList<>()).add(vtable);
            }
            // target -> name -> evidence
            Map<Long, Map<String, List<String>>> names = new TreeMap<>();
            Map<Long, Namespace> namespaces = new HashMap<>();
            for (var entry : lib.vtables.entrySet()) {
                List<ProgramVtable> candidates = new ArrayList<>();
                List<ProgramVtable> all = byClass.getOrDefault(canonical(entry.getKey(), lib), List.of());
                String scope = topLevelScope(entry.getKey()) >= 0 ? splitPath(entry.getKey()).get(0) + SCOPE : "";
                for (ProgramVtable vtable : all) {
                    if (!vtable.path.startsWith(scope)) {
                        continue;
                    }
                    if (vtable.colOffset == 0 || vtable.colOffset < 0 && all.size() == 1) {
                        candidates.add(vtable);
                    }
                }
                if (candidates.isEmpty()) {
                    vtableMissing.merge(lib.name, 1, Integer::sum);
                    continue;
                }
                for (ProgramVtable vtable : candidates) {
                    List<VtVariant> fitting = new ArrayList<>();
                    List<String> reasons = new ArrayList<>();
                    for (VtVariant variant : entry.getValue()) {
                        String reason = checkVtable(variant, vtable);
                        if (reason == null) {
                            fitting.add(variant);
                        } else {
                            reasons.add("variant " + variant.index + " (" + conditionText(variant.conditions) + "): "
                                + reason);
                        }
                    }
                    if (fitting.isEmpty()) {
                        vtableRejects.add("`" + vtable.path + "` " + hex(vtable.address) + " (" + vtable.slots.size()
                            + " slots): " + String.join("; ", reasons));
                        continue;
                    }
                    if (fitting.size() > 1 && !sameSlots(fitting)) {
                        vtableRejects.add("`" + vtable.path + "` " + hex(vtable.address) + ": " + fitting.size()
                            + " variants fit with different slots");
                        continue;
                    }
                    VtVariant variant = fitting.get(0);
                    vtableMatches.add(lib.name + ": `" + vtable.path + "` " + hex(vtable.address) + ", "
                        + vtable.slots.size() + " slots (" + conditionText(variant.conditions) + ")");
                    for (int i = 0; i < variant.slots.size(); i++) {
                        Slot slot = variant.slots.get(i);
                        long target = vtable.slots.get(i);
                        if (slot.pure) {
                            continue;
                        }
                        if (apply) {
                            ensureFunction(target, slot.owner + SCOPE + slot.method);
                        }
                        String method = slot.dtor ? deletingDestructor(target) : slot.method;
                        String path = slot.owner + SCOPE + method;
                        names.computeIfAbsent(target, key -> new TreeMap<>())
                            .computeIfAbsent(path, key -> new ArrayList<>())
                            .add("slot " + i + " of `" + vtable.path + "::vftable` " + hex(vtable.address));
                        if (canonical(slot.owner, lib).equals(canonical(vtable.path, lib))) {
                            namespaces.put(target, vtable.namespace);
                        }
                    }
                }
            }
            for (var entry : names.entrySet()) {
                long target = entry.getKey();
                if (entry.getValue().size() > 1) {
                    conflicts.add(lib.tag(VTABLE) + ": " + hex(target) + " is in slots of "
                        + String.join(", ", entry.getValue().keySet()) + " (folded); not named");
                    continue;
                }
                String path = entry.getValue().keySet().iterator().next();
                List<String> evidence = entry.getValue().get(path);
                if (foreign.containsKey(target)) {
                    conflicts.add(lib.tag(VTABLE) + ": " + hex(target) + " `" + path + "` is also in `"
                        + foreign.get(target) + "::vftable` (no " + lib.name + " base; folded); not named");
                    continue;
                }
                propose(target, path, lib, VTABLE, evidence.get(0) + (evidence.size() > 1 ? " and "
                    + (evidence.size() - 1) + " more" : ""), namespaces.get(target));
            }
        }
    }

    private String checkVtable(VtVariant variant, ProgramVtable vtable) {
        if (variant.count != vtable.slots.size()) {
            return variant.count + " slots in the source";
        }
        if (purecall < 0) {
            return null;
        }
        for (int i = 0; i < variant.slots.size(); i++) {
            boolean pure = vtable.slots.get(i) == purecall;
            if (pure != variant.slots.get(i).pure) {
                return "slot " + i + (pure ? " is _purecall" : " is not _purecall");
            }
        }
        return null;
    }

    private static boolean sameSlots(List<VtVariant> variants) {
        List<String> first = slotNames(variants.get(0));
        for (VtVariant variant : variants) {
            if (!slotNames(variant).equals(first)) {
                return false;
            }
        }
        return true;
    }

    private static List<String> slotNames(VtVariant variant) {
        List<String> names = new ArrayList<>();
        for (Slot slot : variant.slots) {
            names.add(slot.owner + SCOPE + slot.method + (slot.pure ? "=0" : ""));
        }
        return names;
    }

    // `vector_deleting_destructor' when the code tests the delete[] flag, else scalar.
    private String deletingDestructor(long address) {
        Instruction instruction = getInstructionAt(toAddr(address));
        for (int i = 0; instruction != null && i < MAX_DTOR_INSTRUCTIONS; i++) {
            if (instruction.getMnemonicString().equals("TEST")) {
                for (int operand = 0; operand < instruction.getNumOperands(); operand++) {
                    Scalar scalar = instruction.getScalar(operand);
                    if (scalar != null && scalar.getUnsignedValue() == VECTOR_DELETE_FLAG) {
                        return VECTOR_DELETING_DTOR;
                    }
                }
            }
            if (instruction.getFlowType().isTerminal()) {
                break;
            }
            instruction = getInstructionAfter(instruction);
        }
        return SCALAR_DELETING_DTOR;
    }

    private static String conditionText(String conditions) {
        return conditions.isEmpty() ? "no conditions" : conditions;
    }

    // ---- STRING ----

    private void matchStrings(long[] range) {
        Map<String, List<StringEvidence>> byText = new HashMap<>();
        for (Lib lib : libs) {
            for (String[] string : lib.strings) {
                StringEvidence evidence = new StringEvidence();
                evidence.lib = lib;
                evidence.file = string[0];
                evidence.function = string[1];
                evidence.text = string[2];
                stringEvidence.add(evidence);
                String letters = FORMAT_SPEC.matcher(evidence.text).replaceAll("").replaceAll("[^A-Za-z]", "");
                if (evidence.text.length() < MIN_STRING_LENGTH || letters.length() < MIN_STRING_LETTERS) {
                    evidence.reason = "short or format-only";
                    continue;
                }
                byText.computeIfAbsent(evidence.text, key -> new ArrayList<>()).add(evidence);
            }
        }
        Map<Long, StringEvidence> copies = new HashMap<>();
        for (var entry : byText.entrySet()) {
            if (entry.getValue().size() > 1) {
                for (StringEvidence evidence : entry.getValue()) {
                    evidence.reason = "unique in several libraries";
                }
                continue;
            }
            StringEvidence evidence = entry.getValue().get(0);
            for (long copy : copiesByText.getOrDefault(evidence.text, List.of())) {
                evidence.copies.add(copy);
                copies.put(copy, evidence);
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
        for (StringEvidence evidence : stringEvidence) {
            if (evidence.reason != null) {
                continue;
            }
            if (evidence.copies.isEmpty()) {
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
                if (range != null && (target < range[0] || target > range[1])) {
                    evidence.reason = "referencing function " + hex(target) + " is outside the range";
                    continue;
                }
                byTarget.computeIfAbsent(target, key -> new TreeMap<>())
                    .computeIfAbsent(evidence.lib.name + "\t" + evidence.function, key -> new ArrayList<>())
                    .add(evidence);
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
                conflicts.add("STRING: " + hex(entry.getKey()) + " references unique strings of `"
                    + String.join("`, `", names.keySet()).replace("\t", ":") + "`; not named");
                for (List<StringEvidence> list : names.values()) {
                    list.forEach(evidence -> evidence.reason = "function also matches another source function");
                }
                continue;
            }
            String key = names.keySet().iterator().next();
            List<StringEvidence> list = names.get(key);
            StringEvidence first = list.get(0);
            Set<Long> targets = targetsByName.get(key);
            if (targets.size() > 1 && !first.lib.overloaded(first.function)) {
                conflicts.add(first.lib.tag(STRING) + ": `" + first.function + "` strings are referenced by "
                    + hexList(targets) + "; not named");
                list.forEach(evidence -> evidence.reason = "source function matches several functions");
                continue;
            }
            List<String> quoted = new ArrayList<>();
            for (StringEvidence evidence : list) {
                evidence.reason = "accepted";
                quoted.add("\"" + printable(evidence.text) + "\"");
            }
            propose(entry.getKey(), first.function, first.lib, STRING, list.size() + " unique string"
                + (list.size() > 1 ? "s" : "") + " of `" + first.function + "` (" + first.file + "): "
                + String.join(", ", quoted), null);
        }
    }

    // ---- CALLGRAPH ----

    // A named function's single unnamed source callee (caller) is its single DEFAULT-named
    // program callee (caller), when its other source callees (callers) are named and are its
    // program callees (callers).
    private void callgraph(Map<Long, Proposal> chosen, long[] range) throws Exception {
        for (Lib lib : libs) {
            if (lib.calls.isEmpty()) {
                continue;
            }
            Map<String, Long> placed = new HashMap<>();
            Map<Long, String> placedAt = new HashMap<>();
            Set<String> ambiguousNames = new HashSet<>();
            for (Proposal proposal : chosen.values()) {
                if (proposal.lib == lib) {
                    place(placed, placedAt, ambiguousNames, proposal.path, proposal.address);
                }
            }
            // Names of an earlier --apply run count as placed, so a second run proposes nothing new.
            for (Function function : currentProgram.getFunctionManager().getFunctions(true)) {
                long address = function.getEntryPoint().getOffset();
                if (placedAt.containsKey(address) || function.getSymbol().getSource() == SourceType.DEFAULT) {
                    continue;
                }
                for (FunctionTag tag : function.getTags()) {
                    if (tag.getName().startsWith(lib.tag + "_") && lib.defined(function.getName(true))) {
                        place(placed, placedAt, ambiguousNames, function.getName(true), address);
                        alreadyNamed.merge(lib.name, 1, Integer::sum);
                        break;
                    }
                }
            }
            for (int round = 0; round < MAX_ROUNDS; round++) {
                Map<Long, Map<String, String>> found = new TreeMap<>();
                for (var entry : new ArrayList<>(placed.entrySet())) {
                    String name = entry.getKey();
                    if (ambiguousNames.contains(name)) {
                        continue;
                    }
                    Function function = getFunctionAt(toAddr(entry.getValue()));
                    if (function == null) {
                        continue;
                    }
                    neighbour(lib, name, function.getCalledFunctions(monitor), lib.calls.get(name), placed, placedAt,
                        "callee", found);
                    neighbour(lib, name, function.getCallingFunctions(monitor), lib.callers.get(name), placed,
                        placedAt, "caller", found);
                }
                Map<String, Set<Long>> byName = new HashMap<>();
                for (var entry : found.entrySet()) {
                    for (String name : entry.getValue().keySet()) {
                        byName.computeIfAbsent(name, key -> new TreeSet<>()).add(entry.getKey());
                    }
                }
                int added = 0;
                for (var entry : found.entrySet()) {
                    long address = entry.getKey();
                    Map<String, String> names = entry.getValue();
                    if (names.size() > 1) {
                        conflicts.add(lib.tag(CALLGRAPH) + ": " + hex(address) + " fits `"
                            + String.join("`, `", names.keySet()) + "`; not named");
                        continue;
                    }
                    String name = names.keySet().iterator().next();
                    if (byName.get(name).size() > 1) {
                        conflicts.add(lib.tag(CALLGRAPH) + ": `" + name + "` fits " + hexList(byName.get(name))
                            + "; not named");
                        continue;
                    }
                    if (range != null && (address < range[0] || address > range[1])) {
                        continue;
                    }
                    propose(address, name, lib, CALLGRAPH, names.get(name), null);
                    callgraphLog.add(lib.name + " round " + (round + 1) + ": " + hex(address) + " `" + name + "`, "
                        + names.get(name));
                    place(placed, placedAt, ambiguousNames, name, address);
                    added++;
                }
                if (added == 0) {
                    break;
                }
            }
        }
    }

    private static void place(Map<String, Long> placed, Map<Long, String> placedAt, Set<String> ambiguous,
            String name, long address) {
        Long previous = placed.putIfAbsent(name, address);
        if (previous != null && previous != address) {
            ambiguous.add(name);
        }
        placedAt.put(address, name);
    }

    private void neighbour(Lib lib, String name, Set<Function> programSide, Set<String> sourceSide,
            Map<String, Long> placed, Map<Long, String> placedAt, String relation, Map<Long, Map<String, String>> found) {
        if (sourceSide == null) {
            return;
        }
        List<String> open = new ArrayList<>();
        Set<Long> programAddresses = new HashSet<>();
        for (Function function : programSide) {
            programAddresses.add(function.getEntryPoint().getOffset());
        }
        for (String other : sourceSide) {
            if (lib.definitions.getOrDefault(other, 0) != 1 || lib.inline.contains(other)) {
                continue;
            }
            Long address = placed.get(other);
            if (address == null) {
                open.add(other);
            } else if (!programAddresses.contains(address)) {
                return;
            }
        }
        if (open.size() != 1) {
            return;
        }
        List<Long> candidates = new ArrayList<>();
        for (Function function : programSide) {
            long address = function.getEntryPoint().getOffset();
            if (function.getSymbol().getSource() == SourceType.DEFAULT && !function.isThunk()
                    && !placedAt.containsKey(address) && !proposals.containsKey(address)) {
                candidates.add(address);
            }
        }
        if (candidates.size() != 1) {
            return;
        }
        found.computeIfAbsent(candidates.get(0), key -> new TreeMap<>())
            .putIfAbsent(open.get(0), "only unnamed source " + relation + " of `" + name + "` "
                + hex(placed.get(name)));
    }

    // ---- Resolution and annotation ----

    private void propose(long address, String path, Lib lib, String method, String evidence, Namespace namespace) {
        if (address <= 0) {
            return;
        }
        String tag = lib.tag(method);
        for (Proposal existing : proposals.getOrDefault(address, List.of())) {
            if (existing.tag.equals(tag) && existing.path.equals(path)) {
                return;
            }
        }
        Proposal proposal = new Proposal();
        proposal.address = address;
        proposal.path = path;
        proposal.tag = tag;
        proposal.method = method;
        proposal.lib = lib;
        proposal.evidence = evidence;
        proposal.namespace = namespace;
        proposals.computeIfAbsent(address, key -> new ArrayList<>()).add(proposal);
    }

    // The strongest method wins at one address; a non-overloaded name proposed for two
    // addresses keeps the stronger one (neither on a tie).
    private Map<Long, Proposal> resolve() {
        Map<Long, Proposal> chosen = new TreeMap<>();
        resolveConflicts.clear();
        agreements.clear();
        for (var entry : proposals.entrySet()) {
            List<Proposal> list = new ArrayList<>(entry.getValue());
            list.sort((a, b) -> Integer.compare(METHODS.indexOf(a.method), METHODS.indexOf(b.method)));
            Proposal best = list.get(0);
            boolean contested = false;
            for (Proposal other : list) {
                if (other == best) {
                    continue;
                }
                if (other.path.equals(best.path)) {
                    agreements.add(hex(entry.getKey()) + " `" + best.path + "`: " + best.tag + " and " + other.tag);
                } else if (other.method.equals(best.method)) {
                    contested = true;
                    resolveConflicts.add(other.tag + ": " + hex(entry.getKey()) + " `" + best.path + "` or `"
                        + other.path + "`; not named");
                } else {
                    resolveConflicts.add(hex(entry.getKey()) + ": " + best.tag + " `" + best.path + "` vs "
                        + other.tag + " `" + other.path + "`; kept " + best.tag);
                }
            }
            if (!contested) {
                chosen.put(entry.getKey(), best);
            }
        }
        Map<String, List<Proposal>> byPath = new HashMap<>();
        for (Proposal proposal : chosen.values()) {
            byPath.computeIfAbsent(proposal.lib.name + "\t" + proposal.path, key -> new ArrayList<>()).add(proposal);
        }
        for (List<Proposal> list : byPath.values()) {
            Proposal first = list.get(0);
            if (list.size() < 2 || first.lib.overloaded(first.path)) {
                continue;
            }
            list.sort((a, b) -> Integer.compare(METHODS.indexOf(a.method), METHODS.indexOf(b.method)));
            boolean tie = list.get(1).method.equals(list.get(0).method);
            List<Long> addresses = new ArrayList<>();
            list.forEach(proposal -> addresses.add(proposal.address));
            resolveConflicts.add("`" + first.path + "` proposed for " + hexList(addresses) + " ("
                + list.get(0).tag + (tie ? "" : " kept at " + hex(list.get(0).address)) + ")");
            for (int i = tie ? 0 : 1; i < list.size(); i++) {
                chosen.remove(list.get(i).address);
            }
        }
        return chosen;
    }

    private void createProposedFunctions(String method) {
        for (var entry : proposals.entrySet()) {
            for (Proposal proposal : entry.getValue()) {
                if (proposal.method.equals(method)) {
                    ensureFunction(entry.getKey(), proposal.path);
                    break;
                }
            }
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
        List<String> parts = splitPath(proposal.path);
        String name = parts.get(parts.size() - 1);
        Namespace namespace = proposal.namespace != null ? proposal.namespace
            : namespace(parts.subList(0, parts.size() - 1));
        Symbol symbol = function.getSymbol();
        if (!(symbol.getName().equals(name) && symbol.getParentNamespace().equals(namespace))) {
            if (symbol.getSource() != SourceType.DEFAULT) {
                // Names from other sources (other tools, imports, users) are kept and listed.
                kept.add(hex(proposal.address) + " `" + symbol.getName(true) + "` (" + symbol.getSource()
                    + "), not renamed to `" + proposal.path + "` (" + proposal.tag + ")");
                return;
            }
            try {
                symbol.setNameAndNamespace(name, namespace, SourceType.ANALYSIS);
            } catch (DuplicateNameException duplicate) {
                String unique = name + "_" + Long.toHexString(proposal.address);
                symbol.setNameAndNamespace(unique, namespace, SourceType.ANALYSIS);
                anomalies.add("`" + proposal.path + "` already exists; " + hex(proposal.address) + " named `"
                    + unique + "`");
            }
            count("functions renamed");
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
            function.setComment(COMMENT_PREFIX + proposal.tag + " `" + proposal.path + "` (" + proposal.lib.name + " "
                + proposal.lib.version + "): " + proposal.evidence);
            count("plate comments set");
        }
    }

    private Namespace namespace(List<String> parts) throws Exception {
        SymbolTable symbols = currentProgram.getSymbolTable();
        Namespace namespace = currentProgram.getGlobalNamespace();
        for (String part : parts) {
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

    // ---- Report ----

    private void report(PrintWriter output, boolean apply, Map<Long, Proposal> chosen) {
        output.println("# Source library function names in " + currentProgram.getName());
        output.println();
        output.println("Program: `" + currentProgram.getName() + "`, SHA-256 `" + currentProgram.getExecutableSHA256()
            + "`, " + currentProgram.getFunctionManager().getFunctionCount() + " functions.");
        output.println();
        for (Lib lib : libs) {
            output.println("- Map `" + Path.of(lib.path).getFileName() + "`: " + lib.name + " " + lib.version + ", "
                + lib.definitions.size() + " functions, " + lib.strings.size() + " unique strings, "
                + lib.tables.size() + " tables, " + lib.vtables.size() + " vtable classes");
        }
        output.println();
        output.println("Mode: " + (apply ? "`--apply` (changes: " + (changes.isEmpty() ? "none" : changes) + ")"
            : "report only") + ". `_purecall`: " + (purecall >= 0 ? hex(purecall) : "not found") + ".");
        output.println();
        output.println("| Library | " + String.join(" | ", METHODS) + " | Total |");
        output.println("|---|" + "---|".repeat(METHODS.size() + 1));
        for (Lib lib : libs) {
            StringBuilder row = new StringBuilder("| " + lib.name + " " + lib.version + " |");
            int total = 0;
            for (String method : METHODS) {
                int n = 0;
                for (Proposal proposal : chosen.values()) {
                    n += proposal.lib == lib && proposal.method.equals(method) ? 1 : 0;
                }
                total += n;
                row.append(" ").append(n).append(" |");
            }
            output.println(row + " " + total + " |");
        }
        output.println();
        output.println("Names chosen per library and method (the strongest method at an address); "
            + agreements.size() + " further method agreements.");

        List<String> tables = new ArrayList<>();
        for (TableHit hit : tableHits) {
            int functions = 0;
            for (Field field : hit.variant.fields) {
                functions += field.kind.equals("func") ? 1 : 0;
            }
            tables.add(hit.lib.name + ": `" + hit.variant.name + "` at " + hex(hit.base) + ", variant "
                + hit.variant.index + " (" + conditionText(hit.variant.conditions) + "), " + hit.checks
                + " fields checked, " + functions + " functions; anchor: " + hit.anchor);
        }
        int sourceTables = 0;
        for (Lib lib : libs) {
            sourceTables += lib.tables.size();
        }
        section(output, "Tables (" + tableHits.size() + " found of " + sourceTables + " source tables)", tables);
        section(output, "Tables rejected", tableRejects);

        List<String> missing = new ArrayList<>();
        vtableMissing.forEach((lib, n) -> missing.add(lib + ": " + n + " source classes without a primary vtable"));
        section(output, "Vtables matched (" + vtableMatches.size() + ")", vtableMatches);
        section(output, "Vtables rejected", vtableRejects);
        section(output, "Vtables not in the program", missing);

        Map<String, Integer> reasons = new TreeMap<>();
        for (StringEvidence evidence : stringEvidence) {
            String reason = evidence.reason == null ? "unresolved" : evidence.reason
                .replaceAll("^\\d+ data", "data").replaceAll("^referenced by \\d+ functions", "referenced by several functions")
                .replaceAll(" at `0x.*", "").replaceAll("function `0x.*", "function outside the range");
            reasons.merge(evidence.lib.name + ": " + reason, 1, Integer::sum);
        }
        List<String> stringLines = new ArrayList<>();
        reasons.forEach((reason, n) -> stringLines.add(reason + ": " + n));
        section(output, "Strings", stringLines);
        section(output, "Call graph propagation", callgraphLog);
        List<String> previous = new ArrayList<>();
        alreadyNamed.forEach((lib, n) -> previous.add(lib + ": " + n + " functions already named and tagged"));
        section(output, "Names from earlier runs (call graph seeds)", previous);
        List<String> allConflicts = new ArrayList<>(conflicts);
        allConflicts.addAll(resolveConflicts);
        section(output, "Conflicts (not named)", allConflicts);
        section(output, "Existing names kept (disagreements)", kept);
        section(output, "Anomalies", anomalies);

        for (Lib lib : libs) {
            for (String method : METHODS) {
                List<String> lines = new ArrayList<>();
                for (Proposal proposal : chosen.values()) {
                    if (proposal.lib == lib && proposal.method.equals(method)) {
                        lines.add("`" + proposal.path + "` " + hex(proposal.address) + ": " + proposal.evidence);
                    }
                }
                if (!lines.isEmpty()) {
                    section(output, lib.tag(method) + " (" + lines.size() + ")", lines);
                }
            }
        }
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

    // ---- Memory ----

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
        return readUnsigned(address, POINTER_SIZE);
    }

    // Little-endian unsigned value of size bytes, or -1 outside initialized memory.
    private long readUnsigned(long address, int size) {
        Block block = address < 0 ? null : blockOf(address);
        if (block == null || address + size > block.start + block.bytes.length) {
            return -1;
        }
        int index = (int) (address - block.start);
        long value = 0;
        for (int i = size - 1; i >= 0; i--) {
            value = (value << BITS_PER_BYTE) | (block.bytes[index + i] & BYTE_MASK);
        }
        return value;
    }

    private static long sizeMask(int size) {
        return size >= Long.BYTES ? -1L : (1L << (size * BITS_PER_BYTE)) - 1;
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
            if (text.length() >= MAX_STRING_LENGTH) {
                return null;
            }
            text.append((char) c);
        }
        return null;
    }

    private static String printable(String text) {
        StringBuilder out = new StringBuilder();
        for (char c : text.toCharArray()) {
            out.append(c == '\n' ? "\\n" : c == '\t' ? "\\t" : c == '`' ? "'" : c < PRINTABLE_FIRST || c > PRINTABLE_LAST
                ? String.format("\\x%02x", (int) c) : String.valueOf(c));
        }
        return out.toString();
    }

    private static String decodePath(String path) {
        return path.replace(ENCODED_SPACE, " ");
    }

    private static long[] parseRange(String text) {
        String[] parts = text.split("-");
        return new long[] {Long.decode(parts[0]), Long.decode(parts[1])};
    }

    private static String hex(long value) {
        return String.format("`0x%08x`", value);
    }

    private static String hexList(Iterable<Long> values) {
        List<String> parts = new ArrayList<>();
        for (long value : values) {
            parts.add(hex(value));
        }
        return parts.isEmpty() ? "none" : String.join(", ", parts);
    }
}
