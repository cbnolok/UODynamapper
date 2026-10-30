// Name the UOSA.exe code that reads UOP (MYP) package headers, entry tables, data block headers
// and data_block_hash, and the two users of data_block_hash (string table and audio caches).
// Address-based, so it refuses any other build; every address is verified with stated structural
// checks first and a failed check is reported, not named. Findings: results/uosa_uop_header.md.
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

public class NameUopHeaderCode extends GhidraScript {
    private static final String APPLY_FLAG = "--apply";
    // Script arguments are split on whitespace, so a path with spaces is passed with %20.
    private static final String ENCODED_SPACE = "%20";
    // SHA-256 of the UOSA.exe these addresses were traced in (same build as ghidra-source-names).
    private static final String TESTED_SHA256 = "5a5e9dc536c836d1bf10b6f86bb88f61a6ae078f519d4060d3bc47aecbebad50";
    private static final String TAG = "UO_UOP";
    private static final String UOP = "UOP";
    private static final String GLOBAL = "";
    private static final String COMMENT_PREFIX = "NameUopHeaderCode: ";
    private static final String VFTABLE = "vftable";
    private static final int POINTER_SIZE = 4;
    private static final long UNSIGNED_INT_MASK = 0xffffffffL;
    private static final int HEX_RADIX = 16;
    // UOP entry record size (offset 8, header length 4, compressed 4, decompressed 4, name hash 8,
    // data block hash 4, compression 2).
    private static final long ENTRY_RECORD_SIZE = 0x22;
    // Size of the stack buffer MypArchive_GetHeaderRecord skips non-matching records through.
    private static final long HEADER_SKIP_BUFFER_SIZE = 0x800;
    // UOP package header "misc" field; the archive constructor byte-swaps the header when it reads
    // the swapped value (big-endian package).
    private static final long PACKAGE_MISC = 0xfd23ec43L;
    private static final String STRING_TABLE_CACHE_ERROR = "Called BuildCache() after loading!\n";
    private static final String AUDIO_FILE_LIST = "audio/audio_filelist.csv";

    // Checks: "calls:ADDR" (the function references ADDR: call, pointer or jump),
    // "calledby:ADDR" (the function at ADDR references this one), "slot:CLASS:N" (vftable slot N
    // of CLASS holds this function), "vftable:CLASS" (references CLASS::vftable),
    // "string:TEXT" (references the string TEXT), "const:HEX" (an operand equals HEX).
    private record Entry(String address, String namespace, String name, String evidence, String... checks) {
    }

    private static final Entry[] ENTRIES = {
        new Entry("0098d044", UOP, "MypArchive_ctor",
            "stores MypArchive::vftable, reads the 0x28-byte package header (misc 0xFD23EC43 or byte-swapped -> big-endian flag +0x5c; magic not checked; version 4..5 else no table), then the block chain: entries with offset 0 skipped, others copied (34 bytes) and inserted by name hash until file_count",
            "vftable:MypArchive", "const:" + Long.toHexString(ENTRY_RECORD_SIZE), "const:" + Long.toHexString(PACKAGE_MISC),
            "calledby:0098a341"),
        new Entry("0098db38", UOP, "MypArchive_EntryHashTable_Init",
            "allocates the open-addressing table of {u64 name hash, u32 index} (12 bytes per slot) for file_count entries",
            "calledby:0098d044"),
        new Entry("0098db81", UOP, "MypArchive_EntryHashTable_Insert",
            "inserts {name hash, index} with linear probing; a duplicate name hash is not inserted (first entry wins)",
            "calledby:0098d044"),
        new Entry("0098d447", UOP, "MypArchive_IsOpen",
            "MypArchive::vftable slot 1: stream present and its size is not 0",
            "slot:MypArchive:1"),
        new Entry("0098d53d", UOP, "MypArchive_GetEntryTimestamp",
            "MypArchive::vftable slot 2: data block header record type 3 (u64 FILETIME) via slot 6; only callers 0x0098a125 and 0x0098d9c0 are unreferenced",
            "slot:MypArchive:2"),
        new Entry("0098d670", UOP, "MypArchive_ReadEntryRange",
            "MypArchive::vftable slot 5: reads (pos, len) of the stored payload at entry offset + header length + pos",
            "slot:MypArchive:5", "calls:0098db06"),
        new Entry("0098d76e", UOP, "MypArchive_GetHeaderRecord",
            "MypArchive::vftable slot 6 (index, u16 type, u16 *len): type 4 returns the whole data block header; otherwise walks {u16 type, u16 len, data} records and returns the first of the type; called only by slots 2 (type 3) and 7 (type 1), never with type 4",
            "slot:MypArchive:6", "calls:0098db06", "const:" + Long.toHexString(HEADER_SKIP_BUFFER_SIZE)),
        new Entry("0098da27", UOP, "SwapHeaderRecordPrefix",
            "byte-swaps the u16 type and u16 length of a data block header record (big-endian package)",
            "calledby:0098d76e"),
        new Entry("0098d8d1", UOP, "MypArchive_GetHeaderString",
            "MypArchive::vftable slot 7: copies data block header record type 1 (string) into a caller buffer via slot 6; no caller found",
            "slot:MypArchive:7"),
        new Entry("0098d93b", UOP, "MypArchive_IsValidIndex",
            "MypArchive::vftable slot 8: stream open and index below the table's entry count",
            "slot:MypArchive:8"),
        new Entry("0098d978", UOP, "MypArchive_GetEntryDataBlockHash",
            "entry record field +0x1c (data_block_hash); the client never computes it",
            "calls:0098db06", "calledby:0098a1f5"),
        new Entry("0098d995", UOP, "MypArchive_GetEntryPayloadOffset",
            "entry offset + data block header length (where the stored payload starts)",
            "calls:0098db06", "calledby:0098e39d"),
        new Entry("0098d9c0", UOP, "MypArchive_CollectEntryTimestamps",
            "for every entry with a compressed size, adds {index, timestamp (slot 2) or 0} to a container; unreferenced (probable)",
            "calls:0098d520"),
        new Entry("0098a1f5", UOP, "GetFileDataBlockHash",
            "hashes the path, searches the mounted archives (last mounted first) and returns the entry's data_block_hash",
            "calls:0098d487", "calls:0098d460", "calls:0098d978"),
        new Entry("0098a125", UOP, "GetFileTimestamp",
            "loose file time (mode 1/3) or the entry's type-3 header record via MypArchive slot 2; unreferenced",
            "calls:0098d487", "calls:0098d460"),
        new Entry("0098a29b", UOP, "GetFileDecompressedSize",
            "hashes the path, searches the mounted archives and returns the entry's decompressed size",
            "calls:0098d487", "calls:0098d503"),
        new Entry("0098a5a8", UOP, "FindArchiveEntry",
            "hashes the path and returns the archive and entry index of the last mounted archive holding it",
            "calls:0098d487", "calls:0098d460", "calledby:0098e39d"),
        new Entry("00989fdb", UOP, "FileExists",
            "mode 2: name hash in a mounted archive; mode 1: loose file (FindFirstFileA); mode 3: either",
            "calls:0098d487", "calls:0098d460", "calls:0098afd4"),
        new Entry("00989da6", UOP, "ReadFileRange",
            "reads (pos, len) of a loose file or, through MypArchive slot 5, of an archived entry's stored payload",
            "calls:0098e31b", "calls:0098d460"),
        new Entry("0098e31b", UOP, "ReadLooseFileRange",
            "CreateFileA, SetFilePointer(pos), ReadFile(len) of a loose file",
            "calledby:00989da6"),
        new Entry("0098e39d", UOP, "OpenFileHandleAtPayload",
            "opens a loose file, or the package file positioned at entry offset + header length (stored payload), as a Win32 handle",
            "calls:0098a5a8", "calls:0098d995"),
        new Entry("0088e39d", UOP, "GetFileStamp",
            "{data_block_hash, exists} of a path: FileExists then GetFileDataBlockHash",
            "calls:00989fdb", "calls:0098a1f5"),
        new Entry("0088e3ca", GLOBAL, "AudioConfig_CollectFileStamps",
            "file stamps (data_block_hash) of audio_filelist(.csv, _vo.csv) and the sounds/groups/queues/loops CSVs, passed by the caller 0x0088db6a to the audio cache loader 0x0088e519, which reloads the cache only when its header matches (probable)",
            "string:" + AUDIO_FILE_LIST, "calls:0088e39d"),
        new Entry("009a310c", GLOBAL, "StringTable_BuildCache",
            "keeps the string table cache when its stored data_block_hash of the table and fallback table match GetFileDataBlockHash, else rebuilds it",
            "string:" + STRING_TABLE_CACHE_ERROR, "calls:0098a1f5"),
    };

    private final Map<String, Integer> changes = new LinkedHashMap<>();

    @Override
    public void run() throws Exception {
        String[] args = getScriptArgs();
        if (args.length < 1 || args.length > 2 || args.length == 2 && !args[1].equals(APPLY_FLAG)) {
            throw new IllegalArgumentException("Expected OUTPUT_FILE [" + APPLY_FLAG + "]");
        }
        String sha = currentProgram.getExecutableSHA256();
        if (!TESTED_SHA256.equalsIgnoreCase(sha)) {
            throw new IllegalStateException("NameUopHeaderCode names fixed addresses of the tested UOSA.exe (SHA-256 "
                + TESTED_SHA256 + "); " + currentProgram.getName() + " has SHA-256 " + sha + ". Nothing done.");
        }
        boolean apply = args.length == 2;
        String outputPath = args[0].replace(ENCODED_SPACE, " ");
        // Created before any change, so an existing output file stops the run unchanged.
        try (PrintWriter output = new PrintWriter(Files.newBufferedWriter(Path.of(outputPath),
                StandardOpenOption.CREATE_NEW))) {
            List<String> rows = new ArrayList<>();
            for (Entry entry : ENTRIES) {
                rows.add(process(entry, apply));
            }
            output.println("# UOP header names");
            output.println();
            output.println("Program: `" + currentProgram.getName() + "`, SHA-256 `" + sha + "`.");
            output.println();
            output.println("Mode: " + (apply ? "`--apply` (changes: " + (changes.isEmpty() ? "none" : changes) + ")"
                : "report only") + ".");
            output.println();
            output.println("| Address | Name | Checks | Result |");
            output.println("|---|---|---|---|");
            rows.forEach(output::println);
        }
        println("Wrote " + outputPath + (apply ? " (changes: " + changes + ")" : ""));
    }

    private String process(Entry entry, boolean apply) throws Exception {
        Address address = toAddr(Long.parseLong(entry.address(), HEX_RADIX));
        String qualified = (entry.namespace().isEmpty() ? "" : entry.namespace() + Namespace.DELIMITER) + entry.name();
        String row = "| `0x" + entry.address() + "` | `" + qualified + "` | ";
        Function function = getFunctionAt(address);
        if (function == null) {
            return row + checkList(entry) + " | mismatch: no function at the address |";
        }
        List<String> failed = new ArrayList<>();
        for (String check : entry.checks()) {
            if (!check(function, check)) {
                failed.add(check);
            }
        }
        row += checkList(entry) + " | ";
        if (!failed.isEmpty()) {
            return row + "mismatch: " + String.join(", ", failed) + " failed; not named |";
        }
        List<String> result = new ArrayList<>();
        Symbol symbol = function.getSymbol();
        boolean named = symbol.getName(true).equals(qualified);
        if (!named) {
            if (symbol.getSource() != SourceType.DEFAULT) {
                return row + "kept `" + symbol.getName(true) + "` (" + symbol.getSource() + ") |";
            }
            Namespace namespace = entry.namespace().isEmpty() ? currentProgram.getGlobalNamespace()
                : currentProgram.getSymbolTable().getNamespace(entry.namespace(), currentProgram.getGlobalNamespace());
            if (namespace != null && !currentProgram.getSymbolTable().getSymbols(entry.name(), namespace).isEmpty()) {
                return row + "conflict: `" + qualified + "` already names another symbol |";
            }
            if (apply) {
                if (namespace == null) {
                    namespace = currentProgram.getSymbolTable().createNameSpace(currentProgram.getGlobalNamespace(),
                        entry.namespace(), SourceType.ANALYSIS);
                }
                symbol.setNameAndNamespace(entry.name(), namespace, SourceType.ANALYSIS);
                count("functions renamed");
            }
            result.add(apply ? "named" : "would name");
        }
        boolean tagged = false;
        for (FunctionTag tag : function.getTags()) {
            tagged |= tag.getName().equals(TAG);
        }
        if (!tagged) {
            if (apply) {
                function.addTag(TAG);
                count("tags added");
            }
            result.add(apply ? "tagged" : "would tag");
        }
        String plate = getPlateComment(address);
        if (plate == null || plate.isBlank()) {
            if (apply) {
                setPlateComment(address, COMMENT_PREFIX + TAG + ": " + entry.evidence());
                count("plate comments set");
            }
            result.add(apply ? "comment set" : "would set comment");
        } else if (!plate.contains(entry.evidence())) {
            result.add("plate comment from another source left alone");
        }
        return row + (result.isEmpty() ? "already applied" : String.join(", ", result)) + " |";
    }

    // Checks as one table cell; a newline in a string check is shown escaped.
    private static String checkList(Entry entry) {
        return String.join(", ", entry.checks()).replace("\n", "\\n");
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
            case "slot" -> {
                int split = value.lastIndexOf(':');
                yield slot(value.substring(0, split), Integer.parseInt(value.substring(split + 1)),
                    function.getEntryPoint());
            }
            case "vftable" -> {
                boolean found = false;
                for (Address vftable : vftables(value)) {
                    found |= references(function, vftable);
                }
                yield found;
            }
            case "string" -> referencesString(function, value);
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

    private boolean referencesString(Function function, String text) {
        for (Address from : function.getBody().getAddresses(true)) {
            for (Reference reference : getReferencesFrom(from)) {
                Data data = getDataAt(reference.getToAddress());
                if (data != null && StringDataInstance.isString(data)
                        && text.equals(StringDataInstance.getStringDataInstance(data).getStringValue())) {
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

    // Any vftable label of the class (one per base subobject) whose slot holds the target.
    private boolean slot(String className, int index, Address target) {
        for (Address vftable : vftables(className)) {
            try {
                long pointer = getInt(vftable.add((long) index * POINTER_SIZE)) & UNSIGNED_INT_MASK;
                if (pointer == target.getOffset()) {
                    return true;
                }
            } catch (Exception unreadable) {
                // Not a readable table; try the next label.
            }
        }
        return false;
    }

    private List<Address> vftables(String className) {
        List<Address> found = new ArrayList<>();
        SymbolTable table = currentProgram.getSymbolTable();
        for (Symbol symbol : table.getSymbols(VFTABLE)) {
            if (symbol.getName(true).equals(className + Namespace.DELIMITER + VFTABLE)) {
                found.add(symbol.getAddress());
            }
        }
        return found;
    }

    private void count(String what) {
        changes.merge(what, 1, Integer::sum);
    }
}
