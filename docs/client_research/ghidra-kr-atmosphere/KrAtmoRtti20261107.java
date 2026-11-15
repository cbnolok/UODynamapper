// Probe (KR atmosphere research): list MSVC RTTI classes whose mangled name matches a regex, with their vtables,
// the vtable slots (function entry and name) and the functions that reference each vtable start (constructors,
// destructors). Read only.
// Args: OUTPUT_FILE NAME_REGEX MAX_SLOTS
// @category ECResearch
import ghidra.app.script.GhidraScript;
import ghidra.program.model.address.Address;
import ghidra.program.model.listing.Function;
import ghidra.program.model.mem.Memory;
import ghidra.program.model.mem.MemoryBlock;
import ghidra.program.model.symbol.Reference;
import java.io.FileWriter;
import java.io.PrintWriter;
import java.util.ArrayList;
import java.util.Collections;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.regex.Pattern;

public class KrAtmoRtti20261107 extends GhidraScript {
    private Memory memory;
    private final Map<Long, List<Long>> index = new HashMap<>();

    private void buildIndex() throws Exception {
        for (MemoryBlock block : memory.getBlocks()) {
            if (!block.isInitialized()) {
                continue;
            }
            byte[] data = new byte[(int) block.getSize()];
            block.getBytes(block.getStart(), data);
            long start = block.getStart().getOffset();
            for (int i = 0; i + 4 <= data.length; i += 4) {
                long value = (data[i] & 0xff) | ((data[i + 1] & 0xff) << 8) | ((data[i + 2] & 0xff) << 16)
                    | ((long) (data[i + 3] & 0xff) << 24);
                index.computeIfAbsent(value, k -> new ArrayList<>()).add(start + i);
            }
        }
    }

    private List<Long> refs(long value) {
        return index.getOrDefault(value, Collections.emptyList());
    }

    private boolean isCode(long value) {
        MemoryBlock block = memory.getBlock(toAddr(value));
        return block != null && block.isExecute();
    }

    @Override
    public void run() throws Exception {
        memory = currentProgram.getMemory();
        String[] args = getScriptArgs();
        Pattern pattern = Pattern.compile(args[1]);
        int maxSlots = Integer.parseInt(args[2]);
        buildIndex();
        try (PrintWriter out = new PrintWriter(new FileWriter(args[0]))) {
            for (MemoryBlock block : memory.getBlocks()) {
                if (!block.isInitialized()) {
                    continue;
                }
                byte[] data = new byte[(int) block.getSize()];
                block.getBytes(block.getStart(), data);
                long start = block.getStart().getOffset();
                for (int i = 0; i + 4 < data.length; i++) {
                    if (data[i] != '.' || data[i + 1] != '?' || data[i + 2] != 'A'
                            || (data[i + 3] != 'V' && data[i + 3] != 'U')) {
                        continue;
                    }
                    int j = i;
                    while (j < data.length && data[j] != 0) {
                        j++;
                    }
                    String name = new String(data, i, j - i, "ISO-8859-1");
                    if (!pattern.matcher(name).find()) {
                        i = j;
                        continue;
                    }
                    long typeDescriptor = start + i - 8;
                    out.println("== " + name + " TD " + Long.toHexString(typeDescriptor));
                    for (long ref : refs(typeDescriptor)) {
                        long col = ref - 12;
                        if (memory.getInt(toAddr(col)) != 0) {
                            continue;
                        }
                        int offset = memory.getInt(toAddr(col + 4));
                        for (long vref : refs(col)) {
                            long vtable = vref + 4;
                            out.printf("  COL %x off %d vtable %x%n", col, offset, vtable);
                            for (Reference r : getReferencesTo(toAddr(vtable))) {
                                Function fn = getFunctionContaining(r.getFromAddress());
                                out.printf("    vtable ref %s %s%n", r.getFromAddress(),
                                    fn == null ? "-" : fn.getEntryPoint() + " " + fn.getName(true));
                            }
                            for (int k = 0; k < maxSlots; k++) {
                                long f = memory.getInt(toAddr(vtable + 4L * k)) & 0xffffffffL;
                                if (k > 0 && !refs(vtable + 4L * k).isEmpty()) {
                                    break;
                                }
                                if (!isCode(f)) {
                                    break;
                                }
                                Function fn = getFunctionAt(toAddr(f));
                                out.printf("    slot %d (+0x%x) %x %s%n", k, 4 * k, f,
                                    fn == null ? "-" : fn.getName(true));
                            }
                        }
                    }
                    i = j;
                }
            }
        }
        println("done");
    }
}
