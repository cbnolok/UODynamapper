// Probe (KR atmosphere research): print 32-bit values at given addresses as hex, float and int. Read only.
// Args: OUTPUT_FILE ADDRESS [ADDRESS...]
// @category ECResearch
import ghidra.app.script.GhidraScript;
import ghidra.program.model.address.Address;
import java.io.FileWriter;
import java.io.PrintWriter;

public class KrAtmoFloats20261107 extends GhidraScript {
    @Override
    public void run() throws Exception {
        String[] args = getScriptArgs();
        try (PrintWriter out = new PrintWriter(new FileWriter(args[0]))) {
            for (int i = 1; i < args.length; i++) {
                Address a = toAddr(Long.parseLong(args[i].replace("0x", ""), 16));
                boolean init = currentProgram.getMemory().getBlock(a).isInitialized();
                int v = init ? currentProgram.getMemory().getInt(a) : 0;
                out.printf("%s init=%b hex=%08x float=%s int=%d%n", a, init, v, Float.intBitsToFloat(v), v);
            }
        }
        println("done");
    }
}
