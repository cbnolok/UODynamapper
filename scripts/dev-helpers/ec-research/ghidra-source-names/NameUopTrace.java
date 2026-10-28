// Name the UOP read/decompress path of the tested UOSA.exe (traced by hand, see README):
// MypArchive entry access and the zlib 1.2.2 uncompress/inflateInit_/inflateReset/inflateEnd/
// zcalloc it calls. Address-based, so it refuses any other build; every address is verified
// with stated structural checks first and a failed check is reported, not named.
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

public class NameUopTrace extends GhidraScript {
    private static final String APPLY_FLAG = "--apply";
    // Script arguments are split on whitespace, so a path with spaces is passed with %20.
    private static final String ENCODED_SPACE = "%20";
    private static final String TESTED_SHA256 = "5a5e9dc536c836d1bf10b6f86bb88f61a6ae078f519d4060d3bc47aecbebad50";
    private static final String UOP_TAG = "UOP_TRACE";
    private static final String ZLIB_TAG = "ZLIB_UOP_TRACE";
    private static final String UOP = "UOP";
    private static final String GLOBAL = "";
    // Plate comments written by this script; the original probe wrote the same evidence.
    private static final String COMMENT_PREFIX = "NameUopTrace: ";
    private static final String VFTABLE = "vftable";
    private static final int POINTER_SIZE = 4;
    private static final long UNSIGNED_INT_MASK = 0xffffffffL;
    private static final int HEX_RADIX = 16;
    // UOP entry record size (offset 8, header 4, compressed 4, decompressed 4, hash 8, adler 4, flag 2).
    private static final long ENTRY_RECORD_SIZE = 0x22;
    // zlib 1.2.2 sizeof(struct inflate_state) in this build.
    private static final long INFLATE_STATE_SIZE = 0x1ba8;
    private static final String DECOMPRESS_TAG_STRING = "Decompress";

    // Checks: "calls:ADDR" (the function references ADDR: call, pointer or jump),
    // "calledby:ADDR" (the function at ADDR references this one), "slot:CLASS:N" (vftable slot N
    // of CLASS holds this function), "vftable:CLASS" (references CLASS::vftable),
    // "string:TEXT" (references the string TEXT), "const:HEX" (an operand equals HEX).
    private record Entry(String address, String namespace, String name, String tag, String evidence,
            String... checks) {
    }

    private static final Entry[] ENTRIES = {
        new Entry("00989f55", UOP, "OpenFilePtr", UOP_TAG,
            "wraps UOVirtualFileOpen(path, mode, 0) into an owning pointer; 51 callers (UONdlFile ctor, CSV tables, fonts)",
            "calls:00989b72"),
        new Entry("009898a6", UOP, "ArchiveFileOpen", UOP_TAG,
            "archive-only twin of UOVirtualFileOpen: hash path, search the MYP archive list, MythMypInputFile or deferred read",
            "calls:0098e5df", "calls:0098d74c", "calls:0098ce9f"),
        new Entry("0098e5df", UOP, "MythMypInputFile_ctor", UOP_TAG,
            "stores MythMypInputFile::vftable; buffer of the entry's decompressed size filled by ReadEntryAlloc",
            "vftable:MythMypInputFile", "calls:0098d6ed", "calls:0098e672"),
        new Entry("0098e672", UOP, "MythMemoryInputFile_ctor", UOP_TAG,
            "stores MythInputFile then MythMemoryInputFile vftables over (buffer, size)",
            "vftable:MythMemoryInputFile"),
        new Entry("0098d6ed", UOP, "MypArchive_ReadEntryAlloc", UOP_TAG,
            "allocates the entry's decompressed size and calls MypArchive vftable slot 3 (ReadEntry)",
            "calls:0098db06", "calledby:0098e5df"),
        new Entry("0098d57c", UOP, "MypArchive_ReadEntry", UOP_TAG,
            "MypArchive::vftable slot 3: raw read (slot 4), flag 1 -> Decompress (zlib uncompress), flag 0 -> memcpy",
            "slot:MypArchive:3", "calls:0098d6d3"),
        new Entry("0098d61b", UOP, "MypArchive_ReadEntryRaw", UOP_TAG,
            "MypArchive::vftable slot 4: seek to entry offset + header length, read the compressed length",
            "slot:MypArchive:4", "calls:0098db06"),
        new Entry("0098db06", UOP, "MypArchive_GetEntry", UOP_TAG,
            "copies one 34-byte (0x22) entry record: offset 8, header length 4, compressed 4, decompressed 4, hash 8, adler 4, flag 2",
            "const:" + Long.toHexString(ENTRY_RECORD_SIZE), "calledby:0098d503"),
        new Entry("0098d503", UOP, "MypArchive_GetEntryDecompressedSize", UOP_TAG, "entry record field +0x10",
            "calls:0098db06", "calledby:0098e5df"),
        new Entry("0098d520", UOP, "MypArchive_GetEntryCompressedSize", UOP_TAG, "entry record field +0x0c",
            "calls:0098db06", "calledby:0098d57c"),
        new Entry("0098d74c", UOP, "MypArchive_GetEntryCompression", UOP_TAG,
            "entry record field +0x20 (0 stored, 1 zlib)",
            "calls:0098db06", "calledby:00989b72"),
        new Entry("0098ce9f", UOP, "FileLoadJob_ReadCompressed", UOP_TAG,
            "async open of a compressed entry: reads the raw bytes into the job (+0x384, sizes +0x388/+0x38c) for DecompressJobRequest",
            "calls:0098d503", "calls:0098d520", "calledby:00989b72"),
        new Entry("0098cd97", UOP, "FileLoadJobRequest_Execute", UOP_TAG,
            "FileLoadJobRequest::vftable slot 3 (folded with MythAudio::SampleLoadRequest slot 3): open, else Decompress the pending buffer",
            "slot:FileLoadJobRequest:3", "string:" + DECOMPRESS_TAG_STRING, "calls:0098d6d3"),
        new Entry("0098cf2c", UOP, "DecompressJobRequest_Execute", UOP_TAG,
            "FileLoadJobRequest::DecompressJobRequest::vftable slot 3: Decompress the pending buffer into a MythMemoryInputFile",
            "slot:FileLoadJobRequest::DecompressJobRequest:3", "string:" + DECOMPRESS_TAG_STRING, "calls:0098d6d3"),
        new Entry("0098d6d3", UOP, "Decompress", UOP_TAG, "only call: zlib uncompress (0x009a2530)",
            "calls:009a2530", "calledby:0098d57c"),
        new Entry("009a2530", GLOBAL, "uncompress", ZLIB_TAG,
            "zlib 1.2.2 uncompress: inflateInit_, inflate(Z_FINISH), Z_NEED_DICT/Z_BUF_ERROR -> Z_DATA_ERROR, inflateEnd",
            "calls:009a5bf0", "calls:009a5d80", "calls:009a71e0"),
        new Entry("009a5bf0", GLOBAL, "inflateInit_", ZLIB_TAG,
            "zlib 1.2.2 inflateInit_ with inflateInit2_ inlined: default zcalloc/zcfree, state 0x1ba8 bytes, wrap 1, wbits 15, inflateReset",
            "const:" + Long.toHexString(INFLATE_STATE_SIZE), "calls:009a7690", "calls:009a5ba0"),
        new Entry("009a5ba0", GLOBAL, "inflateReset", ZLIB_TAG,
            "zlib 1.2.2 inflateReset: totals 0, adler 1, mode HEAD, codes pointers",
            "calledby:009a5bf0"),
        new Entry("009a71e0", GLOBAL, "inflateEnd", ZLIB_TAG, "zlib 1.2.2 inflateEnd: ZFREE window then state",
            "calledby:009a2530"),
        new Entry("009a7690", GLOBAL, "zcalloc", ZLIB_TAG,
            "zlib zcalloc: malloc(items * size); default zalloc of inflateInit_",
            "calledby:009a5bf0"),
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
            throw new IllegalStateException("NameUopTrace names fixed addresses of the tested UOSA.exe (SHA-256 "
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
            output.println("# UOP trace names");
            output.println();
            output.println("Program: `" + currentProgram.getName() + "`, SHA-256 `" + sha + "`.");
            output.println();
            output.println("Mode: " + (apply ? "`--apply` (changes: " + (changes.isEmpty() ? "none" : changes) + ")"
                : "report only") + ".");
            output.println();
            output.println("| Address | Name | Tag | Checks | Result |");
            output.println("|---|---|---|---|---|");
            rows.forEach(output::println);
        }
        println("Wrote " + outputPath + (apply ? " (changes: " + changes + ")" : ""));
    }

    private String process(Entry entry, boolean apply) throws Exception {
        Address address = toAddr(Long.parseLong(entry.address(), HEX_RADIX));
        String qualified = (entry.namespace().isEmpty() ? "" : entry.namespace() + Namespace.DELIMITER) + entry.name();
        String row = "| `0x" + entry.address() + "` | `" + qualified + "` | " + entry.tag() + " | ";
        Function function = getFunctionAt(address);
        if (function == null) {
            return row + String.join(", ", entry.checks()) + " | mismatch: no function at the address |";
        }
        List<String> failed = new ArrayList<>();
        for (String check : entry.checks()) {
            if (!check(function, check)) {
                failed.add(check);
            }
        }
        row += String.join(", ", entry.checks()) + " | ";
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
            tagged |= tag.getName().equals(entry.tag());
        }
        if (!tagged) {
            if (apply) {
                function.addTag(entry.tag());
                count("tags added");
            }
            result.add(apply ? "tagged" : "would tag");
        }
        String plate = getPlateComment(address);
        if (plate == null || plate.isBlank()) {
            if (apply) {
                setPlateComment(address, COMMENT_PREFIX + entry.tag() + ": " + entry.evidence());
                count("plate comments set");
            }
            result.add(apply ? "comment set" : "would set comment");
        } else if (!plate.contains(entry.evidence())) {
            result.add("plate comment from another source left alone");
        }
        return row + (result.isEmpty() ? "already applied" : String.join(", ", result)) + " |";
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
