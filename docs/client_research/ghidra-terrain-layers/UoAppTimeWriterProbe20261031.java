// GhidraScript probe (read-only). Lists the UOApplication vftable methods, and their direct callees, that
// store to [reg + 0x58]: the float UOWaterTerrainLayer::Draw (0x005988a0) reads as `t` from
// *(0x00e3722c) + 0x58. Run on UOSA.exe; prints to the console only.
// Result (2026-10-31): FUN_00482530 (vftable slot 1, the main loop) at 0x00482542 and 0x0048277e.
import ghidra.app.script.GhidraScript;
import ghidra.program.model.address.Address;
import ghidra.program.model.listing.*;
import ghidra.program.model.symbol.*;
import java.util.*;

public class UoAppTimeWriterProbe20261031 extends GhidraScript {
    private static final int VTABLE_SLOTS = 40;
    private static final String TARGET_OPERAND = "+ 0x58]";

    public void run() throws Exception {
        Symbol vt = null;
        for (Symbol s : currentProgram.getSymbolTable().getSymbols("vftable")) {
            if (s.getParentNamespace().getName().equals("UOApplication")) { vt = s; }
        }
        if (vt == null) { println("no UOApplication::vftable"); return; }
        Listing listing = currentProgram.getListing();
        Set<Function> seen = new LinkedHashSet<>();
        List<Function> todo = new ArrayList<>();
        for (int i = 0; i < VTABLE_SLOTS; i++) {
            Address slot = vt.getAddress().add(4L * i);
            Address target = toAddr(getInt(slot) & 0xffffffffL);
            Function f = getFunctionAt(target);
            if (f == null) { continue; }
            println("slot " + i + " -> " + f.getName() + " @ " + target);
            todo.add(f);
        }
        List<Function> expanded = new ArrayList<>(todo);
        for (Function f : todo) { expanded.addAll(f.getCalledFunctions(monitor)); }
        for (Function f : expanded) {
            if (!seen.add(f)) { continue; }
            for (Instruction ins : listing.getInstructions(f.getBody(), true)) {
                String m = ins.getMnemonicString();
                if (!(m.startsWith("FST") || m.startsWith("MOV"))) { continue; }
                if (ins.getDefaultOperandRepresentation(0).contains(TARGET_OPERAND)) {
                    println("store " + f.getName() + " @ " + ins.getAddress() + ": " + ins);
                }
            }
        }
    }
}
