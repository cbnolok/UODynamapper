// Report which imported functions have a matching function signature in Ghidra's
// windows_vs12_32.gdt or in the supplied extra data type archives.
// @category ECResearch
import ghidra.app.script.GhidraScript;
import ghidra.framework.Application;
import ghidra.program.model.data.DataType;
import ghidra.program.model.data.FileDataTypeManager;
import ghidra.program.model.data.FunctionDefinition;
import ghidra.program.model.symbol.ExternalLocation;
import ghidra.program.model.symbol.Symbol;
import java.io.File;
import java.io.PrintWriter;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardOpenOption;
import java.util.ArrayList;
import java.util.HashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.TreeMap;
import java.util.TreeSet;

public class ImportSignatureCoverage extends GhidraScript {
    private static final String BASE_ARCHIVE = "typeinfo/win32/windows_vs12_32.gdt";

    public void run() throws Exception {
        String[] args = getScriptArgs();
        if (args.length < 1) {
            throw new IllegalArgumentException("Expected OUTPUT_FILE [EXTRA_GDT ...]");
        }
        List<File> archives = new ArrayList<>();
        archives.add(Application.getModuleDataFile("Base", BASE_ARCHIVE).getFile(false));
        for (int index = 1; index < args.length; index++) {
            archives.add(new File(args[index]));
        }
        Set<String> signatures = new HashSet<>();
        for (File archive : archives) {
            FileDataTypeManager dtm = FileDataTypeManager.openFileArchive(archive, false);
            try {
                var all = dtm.getAllDataTypes();
                while (all.hasNext()) {
                    DataType dt = all.next();
                    if (dt instanceof FunctionDefinition) {
                        signatures.add(dt.getName());
                    }
                }
            } finally {
                dtm.close();
            }
        }

        Map<String, Set<String>> covered = new TreeMap<>();
        Map<String, Set<String>> missing = new TreeMap<>();
        // Without analysis most imports are external labels, not yet external functions.
        var externals = currentProgram.getExternalManager();
        for (Symbol symbol : currentProgram.getSymbolTable().getExternalSymbols()) {
            ExternalLocation location = externals.getExternalLocation(symbol);
            if (location == null) {
                continue;
            }
            String library = location.getLibraryName().toUpperCase();
            String name = symbol.getName();
            (signatures.contains(name) ? covered : missing)
                .computeIfAbsent(library, key -> new TreeSet<>()).add(name);
        }
        Set<String> libraries = new TreeSet<>(covered.keySet());
        libraries.addAll(missing.keySet());

        try (PrintWriter output = new PrintWriter(Files.newBufferedWriter(Path.of(args[0]),
                StandardOpenOption.CREATE_NEW))) {
            output.println("# Import signature coverage");
            output.println();
            output.println("Program: " + currentProgram.getName());
            output.println("SHA-256: " + currentProgram.getExecutableSHA256());
            output.println("Archives:");
            for (File archive : archives) {
                output.println("- " + archive.getName());
            }
            output.println();
            output.println("| Library | Imports | With signature | Without |");
            output.println("|---|---:|---:|---:|");
            for (String library : libraries) {
                int have = covered.getOrDefault(library, Set.of()).size();
                int lack = missing.getOrDefault(library, Set.of()).size();
                output.println("| " + library + " | " + (have + lack) + " | " + have + " | " + lack + " |");
            }
            for (var entry : missing.entrySet()) {
                output.println();
                output.println("## " + entry.getKey() + " without signature");
                output.println();
                for (String name : entry.getValue()) {
                    output.println("- `" + name + "`");
                }
            }
        }
    }
}
