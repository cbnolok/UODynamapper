// Light sprite probe (2026-11-12). Read only: prints to the console, changes nothing.
// Args: decomp ADDR... | callers ADDR... | getters OFF... (hex field offsets read by FLD [ECX+off]; RET)
//       | disasm ADDR COUNT | bytes HEX (each match and the functions that reference it)
// @category ECResearch
import ghidra.app.script.GhidraScript;
import ghidra.app.decompiler.DecompInterface;
import ghidra.program.model.address.Address;
import ghidra.program.model.listing.Function;
import ghidra.program.model.listing.Instruction;
import ghidra.program.model.symbol.Reference;
import java.util.HashSet;
import java.util.Set;

public class LightSpriteProbe20261112 extends GhidraScript {
    public void run() throws Exception {
        String[] args = getScriptArgs();
        String mode = args[0];
        if (mode.equals("decomp")) {
            DecompInterface decompiler = new DecompInterface();
            decompiler.openProgram(currentProgram);
            for (int i = 1; i < args.length; i++) {
                Function function = getFunctionContaining(toAddr(args[i]));
                if (function == null) { println("// no function at " + args[i]); continue; }
                var result = decompiler.decompileFunction(function, 60, monitor);
                println("// ==== " + function.getName() + " @ " + function.getEntryPoint());
                println(result.decompileCompleted() ? result.getDecompiledFunction().getC() : result.getErrorMessage());
            }
            decompiler.dispose();
        } else if (mode.equals("callers")) {
            for (int i = 1; i < args.length; i++) {
                Address target = toAddr(args[i]);
                println("// refs to " + target);
                for (Reference ref : getReferencesTo(target)) {
                    Function f = getFunctionContaining(ref.getFromAddress());
                    println("  " + ref.getFromAddress() + " " + ref.getReferenceType() + " in "
                        + (f == null ? "?" : f.getName() + " @ " + f.getEntryPoint()));
                }
            }
        } else if (mode.equals("getters")) {
            Set<Long> offsets = new HashSet<>();
            for (int i = 1; i < args.length; i++) offsets.add(Long.decode(args[i]));
            for (Function f : currentProgram.getFunctionManager().getFunctions(true)) {
                Instruction first = getInstructionAt(f.getEntryPoint());
                if (first == null) continue;
                Instruction second = first.getNext();
                if (second == null || !second.getMnemonicString().startsWith("RET")) continue;
                String text = first.toString();
                if (!text.startsWith("FLD float ptr [ECX + ") && !text.startsWith("MOV EAX,dword ptr [ECX + ")) continue;
                String imm = text.substring(text.lastIndexOf('+') + 2, text.length() - 1);
                if (offsets.contains(Long.decode(imm))) {
                    println(f.getEntryPoint() + " " + text);
                    for (Reference ref : getReferencesTo(f.getEntryPoint())) println("    ref " + ref.getFromAddress());
                }
            }
        } else if (mode.equals("bytes")) {
            byte[] pattern = new byte[args[1].length() / 2];
            for (int i = 0; i < pattern.length; i++)
                pattern[i] = (byte) Integer.parseInt(args[1].substring(2 * i, 2 * i + 2), 16);
            Address at = currentProgram.getMinAddress();
            while (true) {
                at = currentProgram.getMemory().findBytes(at, pattern, null, true, monitor);
                if (at == null) break;
                println("match " + at);
                for (Reference ref : getReferencesTo(at)) {
                    Function f = getFunctionContaining(ref.getFromAddress());
                    println("    ref " + ref.getFromAddress() + " in " + (f == null ? "?" : f.getName() + " @ " + f.getEntryPoint()));
                }
                at = at.add(1);
            }
        } else if (mode.equals("disasm")) {
            Instruction ins = getInstructionAt(toAddr(args[1]));
            int count = Integer.parseInt(args[2]);
            for (int i = 0; i < count && ins != null; i++, ins = ins.getNext()) println(ins.getAddress() + "  " + ins);
        }
    }
}
