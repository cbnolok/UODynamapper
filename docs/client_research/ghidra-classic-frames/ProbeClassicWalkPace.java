// Read-only probe of how the Classic client (7.0.102.3 client.exe) paces a walking mobile's frames: every instruction
// whose memory operand has one of the given displacements off a register (a CMobile field), with its function and
// whether it writes the field. Used for the frame counter +0x1a0, the UOP frame time +0x238 and frame +0x220.
// Prints to the console; names nothing. Findings: results/classic_frames.md#walk-pace.
//
// ProbeClassicWalkPace.java HEX_DISPLACEMENT...
// run_ghidra_script(program="classic_7.0.103.0_client.exe", script_name="<abs path>/ProbeClassicWalkPace.java",
//                   args="1a0 238 220")
// @category ECResearch
import ghidra.app.script.GhidraScript;
import ghidra.program.model.lang.OperandType;
import ghidra.program.model.listing.Function;
import ghidra.program.model.listing.Instruction;
import ghidra.program.model.listing.InstructionIterator;
import ghidra.program.model.pcode.PcodeOp;
import ghidra.program.model.scalar.Scalar;
import java.util.HashSet;
import java.util.Set;

public class ProbeClassicWalkPace extends GhidraScript {
    private static final String CLASSIC_7_0_103_SHA256 =
        "4d857f2f5f8bf350bc50a87ee66b2236bfa480e712e9546c740ff04f9106690a";
    private static final int HEX_RADIX = 16;

    @Override
    public void run() throws Exception {
        String sha = currentProgram.getExecutableSHA256();
        if (!CLASSIC_7_0_103_SHA256.equalsIgnoreCase(sha)) {
            printerr("not classic 7.0.102.3 client.exe: " + sha);
            return;
        }
        Set<Long> wanted = new HashSet<>();
        for (String arg : getScriptArgs()) {
            wanted.add(Long.parseLong(arg, HEX_RADIX));
        }
        InstructionIterator it = currentProgram.getListing().getInstructions(true);
        while (it.hasNext() && !monitor.isCancelled()) {
            Instruction ins = it.next();
            for (int op = 0; op < ins.getNumOperands(); op++) {
                int type = ins.getOperandType(op);
                if ((type & OperandType.DYNAMIC) == 0) {
                    continue;
                }
                for (Object obj : ins.getOpObjects(op)) {
                    if (!(obj instanceof Scalar scalar) || !wanted.contains(scalar.getUnsignedValue())) {
                        continue;
                    }
                    boolean writes = false;
                    for (PcodeOp pcode : ins.getPcode()) {
                        if (pcode.getOpcode() == PcodeOp.STORE) {
                            writes = true;
                        }
                    }
                    Function f = getFunctionContaining(ins.getAddress());
                    println(String.format("%s %-28s %s %s", ins.getAddress(), f == null ? "-" : f.getName(),
                        writes ? "W" : "R", ins));
                }
            }
        }
    }
}
