// Probe (KR atmosphere research): decompile the functions at the given entry addresses into a text file.
// Read only: an address without a function is reported, never created.
// Args: OUTPUT_FILE ENTRY [ENTRY...]
// @category ECResearch
import ghidra.app.decompiler.DecompInterface;
import ghidra.app.script.GhidraScript;
import ghidra.program.model.address.Address;
import ghidra.program.model.listing.Function;
import java.io.FileWriter;
import java.io.PrintWriter;

public class KrAtmoDecompile20261107 extends GhidraScript {
    private static final int DECOMPILE_TIMEOUT_SECONDS = 60;

    @Override
    public void run() throws Exception {
        String[] args = getScriptArgs();
        DecompInterface decompiler = new DecompInterface();
        if (!decompiler.openProgram(currentProgram)) {
            throw new IllegalStateException(decompiler.getLastMessage());
        }
        try (PrintWriter out = new PrintWriter(new FileWriter(args[0]))) {
            out.println("// Program: " + currentProgram.getName());
            out.println("// SHA-256: " + currentProgram.getExecutableSHA256());
            for (int i = 1; i < args.length; i++) {
                Address address = toAddr(Long.parseLong(args[i].replace("0x", ""), 16));
                Function function = getFunctionContaining(address);
                if (function == null) {
                    out.println("// " + address + ": no function");
                    continue;
                }
                var result = decompiler.decompileFunction(function, DECOMPILE_TIMEOUT_SECONDS, monitor);
                out.println("// Entry: " + function.getEntryPoint() + " (asked " + address + ") " + function.getName(true));
                out.println(result.decompileCompleted()
                    ? result.getDecompiledFunction().getC() : "// " + result.getErrorMessage());
                out.flush();
            }
        } finally {
            decompiler.dispose();
        }
        println("done");
    }
}
