// Decompile explicitly supplied function entries in an imported Ghidra program.
// @category ECResearch
import ghidra.app.script.GhidraScript;
import ghidra.app.decompiler.DecompInterface;
import ghidra.program.model.listing.Function;
import java.io.PrintWriter;
import java.nio.file.Files;
import java.nio.file.Path;

public class DecompileFunctions extends GhidraScript {
    public void run() throws Exception {
        String[] args = getScriptArgs();
        if (args.length < 2) {
            throw new IllegalArgumentException("Expected OUTPUT_FILE ENTRY_VA [ENTRY_VA ...]");
        }
        DecompInterface decompiler = new DecompInterface();
        if (!decompiler.openProgram(currentProgram)) {
            throw new IllegalStateException(decompiler.getLastMessage());
        }
        try (PrintWriter output = new PrintWriter(Files.newBufferedWriter(Path.of(args[0]),
                java.nio.file.StandardOpenOption.CREATE_NEW))) {
            output.println("// Program: " + currentProgram.getName());
            output.println("// SHA-256: " + currentProgram.getExecutableSHA256());
            for (int index = 1; index < args.length; index++) {
                var address = toAddr(args[index]);
                Function function = getFunctionAt(address);
                if (function == null) {
                    if (getFunctionContaining(address) != null) {
                        throw new IllegalArgumentException("Address is inside another function: " + address);
                    }
                    disassemble(address);
                    function = createFunction(address, "inspect_" + address);
                }
                if (function == null) {
                    throw new IllegalArgumentException("Cannot create function at " + address);
                }
                var result = decompiler.decompileFunction(function, 30, monitor);
                output.println("// Entry: " + address);
                output.println(result.decompileCompleted()
                    ? result.getDecompiledFunction().getC() : "// " + result.getErrorMessage());
                output.flush();
                monitor.checkCancelled();
            }
        } finally {
            decompiler.dispose();
        }
    }
}
