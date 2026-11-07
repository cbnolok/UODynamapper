// Headless helper (pass 3): decompiles the functions at the given addresses without auto-analysis, for use while
// the GUI's MCP bridge is busy. Args: OUTPUT_FILE ADDRESS [ADDRESS ...] (hex, no 0x). Read only on the program.
// @category UODynamapper

import ghidra.app.cmd.disassemble.DisassembleCommand;
import ghidra.app.cmd.function.CreateFunctionCmd;
import ghidra.app.decompiler.DecompInterface;
import ghidra.app.decompiler.DecompileResults;
import ghidra.app.script.GhidraScript;
import ghidra.program.model.address.Address;
import ghidra.program.model.address.AddressSet;
import ghidra.program.model.listing.Function;
import ghidra.program.model.symbol.Reference;

import java.io.PrintWriter;
import java.nio.file.Files;
import java.nio.file.Path;

public class KrShadowHeadlessDecompile20261107 extends GhidraScript {
    private static final int HEX_RADIX = 16;
    private static final int DECOMPILE_TIMEOUT_SECONDS = 120;

    @Override
    protected void run() throws Exception {
        String[] args = getScriptArgs();
        DecompInterface decompiler = new DecompInterface();
        decompiler.openProgram(currentProgram);
        try (PrintWriter out = new PrintWriter(Files.newBufferedWriter(Path.of(args[0])))) {
            for (int i = 1; i < args.length; i++) {
                Address address = toAddr(Long.parseLong(args[i], HEX_RADIX));
                new DisassembleCommand(new AddressSet(address), null, true).applyTo(currentProgram, monitor);
                Function function = getFunctionAt(address);
                if (function == null) {
                    new CreateFunctionCmd(address).applyTo(currentProgram, monitor);
                    function = getFunctionAt(address);
                }
                out.println("// ===== " + args[i] + (function == null ? " (no function)" : " " + function.getName(true)));
                if (function == null) {
                    continue;
                }
                for (Reference reference : getReferencesTo(address)) {
                    Function caller = getFunctionContaining(reference.getFromAddress());
                    out.println("// ref from " + reference.getFromAddress() + " " + reference.getReferenceType()
                        + (caller == null ? "" : " in " + caller.getName(true) + " @ " + caller.getEntryPoint()));
                }
                DecompileResults results = decompiler.decompileFunction(function, DECOMPILE_TIMEOUT_SECONDS, monitor);
                out.println(results.decompileCompleted() ? results.getDecompiledFunction().getC()
                    : "// failed: " + results.getErrorMessage());
            }
        } finally {
            decompiler.dispose();
        }
    }
}
