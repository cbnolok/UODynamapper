// Apply the prototypes, parameter / local / global names and the structure types described by a variable spec file
// (one per research area, next to its Name*Code.java) to the functions that script named and tagged. Functions are
// found by their full name and must carry the spec's tag; variables are found by role (parameter ordinal, stack
// offset, register defined at a function-relative offset, result of the n-th call to a named function), globals by
// the data reference of the instruction at a function-relative offset. No absolute addresses. Without --apply it
// only reports; a second --apply reports "Changes: none".
// @category ECResearch
import ghidra.app.decompiler.DecompInterface;
import ghidra.app.decompiler.DecompileResults;
import ghidra.app.script.GhidraScript;
import ghidra.app.util.NamespaceUtils;
import ghidra.program.model.address.Address;
import ghidra.program.model.data.ArrayDataType;
import ghidra.program.model.data.BuiltInDataTypeManager;
import ghidra.program.model.data.Category;
import ghidra.program.model.data.CategoryPath;
import ghidra.program.model.data.DataType;
import ghidra.program.model.data.DataTypeComponent;
import ghidra.program.model.data.DataTypeConflictHandler;
import ghidra.program.model.data.DataTypeManager;
import ghidra.program.model.data.PointerDataType;
import ghidra.program.model.data.Structure;
import ghidra.program.model.data.StructureDataType;
import ghidra.program.model.data.Undefined;
import ghidra.program.model.data.VoidDataType;
import ghidra.program.model.lang.PrototypeModel;
import ghidra.program.model.lang.Register;
import ghidra.program.model.listing.Data;
import ghidra.program.model.listing.Function;
import ghidra.program.model.listing.FunctionTag;
import ghidra.program.model.listing.Instruction;
import ghidra.program.model.listing.Parameter;
import ghidra.program.model.listing.ParameterImpl;
import ghidra.program.model.listing.ReturnParameterImpl;
import ghidra.program.model.listing.VariableStorage;
import ghidra.program.model.pcode.HighFunction;
import ghidra.program.model.pcode.HighFunctionDBUtil;
import ghidra.program.model.pcode.HighSymbol;
import ghidra.program.model.pcode.HighVariable;
import ghidra.program.model.pcode.PcodeOp;
import ghidra.program.model.pcode.PcodeOpAST;
import ghidra.program.model.pcode.Varnode;
import ghidra.program.model.scalar.Scalar;
import ghidra.program.model.symbol.Namespace;
import ghidra.program.model.symbol.RefType;
import ghidra.program.model.symbol.Reference;
import ghidra.program.model.symbol.SourceType;
import ghidra.program.model.symbol.Symbol;
import ghidra.program.model.symbol.SymbolType;
import java.io.PrintWriter;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardOpenOption;
import java.util.ArrayList;
import java.util.Iterator;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

public class ApplyVariableSpecs extends GhidraScript {
    private static final String APPLY_FLAG = "--apply";
    private static final String DEFAULT_PARAM = "-";
    private static final String KEEP_TYPE = "-";
    private static final String CUSTOM = "custom";
    private static final String THIS = "this";
    private static final String CDECL = "__cdecl";
    private static final String STDCALL = "__stdcall";
    private static final String THISCALL = "__thiscall";
    private static final String FASTCALL = "__fastcall";
    private static final String STACK_PREFIX = "stack:";
    private static final String REG_PREFIX = "reg:";
    private static final String CALL_PREFIX = "call:";
    private static final String REF_PREFIX = "ref:";
    private static final String OFFSET_SEPARATOR = "@";
    private static final String ORDINAL_SEPARATOR = "#";
    private static final int STACK_SLOT = 4;
    private static final int RETURN_ADDRESS_SIZE = 4;
    private static final int DECOMPILE_TIMEOUT_SECONDS = 120;
    private static final String PROLOG_NAME = "__EH_prolog";
    private static final String PROLOG_SCRATCH = "EAX";
    private static final String PROLOG_FIXUP = "EH_prolog";
    // Classic MSVC __EH_prolog: PUSH -1; PUSH EAX; MOV EAX,FS:[0]; PUSH EAX; MOV EAX,[ESP+0xc]; MOV FS:[0],ESP;
    // MOV [ESP+0xc],EBP; LEA EBP,[ESP+0xc]; PUSH EAX; RET. Callers start with MOV EAX,<handler>; CALL __EH_prolog.
    private static final String[] PROLOG_MNEMONICS = {"PUSH", "PUSH", "MOV", "PUSH", "MOV", "MOV", "MOV", "LEA", "PUSH", "RET"};
    private static final String SAVE_MNEMONIC = "PUSH";
    private static final String RESTORE_MNEMONIC = "POP";
    private static final String[] CALLEE_SAVED = {"EBX", "ESI", "EDI", "EBP"};
    private static final String[] GP_REGISTERS = {"EAX", "ECX", "EDX", "EBX", "ESI", "EDI", "EBP"};

    // ---- spec model ----

    private static final class FieldSpec {
        int offset;
        String type;
        String name;
        String comment;
    }

    private static final class TypeSpec {
        String path;
        int size;
        String description;
        final List<FieldSpec> fields = new ArrayList<>();
        int line;
    }

    private static final class ParamSpec {
        String name;
        String type;
        String storage;
    }

    private static final class VarSpec {
        String locator;
        String name;
        String type;
        int line;
    }

    private static final class FunctionSpec {
        String path;
        String convention;
        String returnType;
        boolean custom;
        final List<ParamSpec> params = new ArrayList<>();
        final List<VarSpec> locals = new ArrayList<>();
        final List<VarSpec> globals = new ArrayList<>();
        int line;
    }

    private String tag;
    private final Map<String, TypeSpec> types = new LinkedHashMap<>();
    private final Map<String, String> classes = new LinkedHashMap<>();
    private final List<FunctionSpec> functionSpecs = new ArrayList<>();

    // ---- state ----

    private DataTypeManager dtm;
    private DecompInterface decompiler;
    private boolean apply;
    private final List<String> changes = new ArrayList<>();
    private final List<String> anomalies = new ArrayList<>();
    private final List<String> kept = new ArrayList<>();
    private final List<String> anchors = new ArrayList<>();
    private final List<String> rows = new ArrayList<>();
    private final List<String> typeRows = new ArrayList<>();
    private final List<String> globalRows = new ArrayList<>();
    private int paramsNamed;
    private int localsNamed;
    private int globalsNamed;
    private int prototypesSet;

    @Override
    public void run() throws Exception {
        String[] args = getScriptArgs();
        if (args.length < 2) {
            throw new IllegalArgumentException("Expected SPEC_FILE OUTPUT_FILE [" + APPLY_FLAG + "]");
        }
        for (int i = 2; i < args.length; i++) {
            if (args[i].equals(APPLY_FLAG)) {
                apply = true;
            } else {
                throw new IllegalArgumentException("Unknown argument " + args[i]);
            }
        }
        parse(Files.readAllLines(Path.of(args[0]), StandardCharsets.UTF_8));
        // Created before any change, so an existing output file stops the run unchanged.
        try (PrintWriter out = new PrintWriter(Files.newBufferedWriter(Path.of(args[1]), StandardOpenOption.CREATE_NEW))) {
            dtm = currentProgram.getDataTypeManager();
            decompiler = new DecompInterface();
            decompiler.openProgram(currentProgram);
            try {
                prolog();
                for (TypeSpec t : types.values()) {
                    ensureType(t);
                }
                for (Map.Entry<String, String> e : classes.entrySet()) {
                    ensureClass(e.getKey(), e.getValue());
                }
                Map<String, Function> tagged = taggedFunctions();
                for (FunctionSpec fs : functionSpecs) {
                    Function f = tagged.get(fs.path);
                    if (f == null) {
                        anomalies.add("line " + fs.line + ": no function `" + fs.path + "` with tag " + tag);
                        continue;
                    }
                    applyFunction(f, fs);
                }
                report(out, args[0], tagged.size());
            } finally {
                decompiler.dispose();
            }
        }
        println("Wrote " + args[1] + (apply ? " (applied: " + changes.size() + " changes)" : ""));
    }

    // ---- spec parsing ----

    private void parse(List<String> lines) {
        TypeSpec type = null;
        FunctionSpec function = null;
        for (int n = 0; n < lines.size(); n++) {
            String raw = lines.get(n);
            int hash = raw.indexOf('#');
            String line = (hash >= 0 ? raw.substring(0, hash) : raw).trim();
            if (line.isEmpty()) {
                continue;
            }
            int lineNo = n + 1;
            String[] head = line.split("\\s+", 2);
            String keyword = head[0];
            String rest = head.length > 1 ? head[1] : "";
            if (keyword.equals("end")) {
                if (type == null && function == null) {
                    throw new IllegalArgumentException("line " + lineNo + ": end without block");
                }
                type = null;
                function = null;
                continue;
            }
            if (type != null) {
                type.fields.add(field(rest.isEmpty() ? keyword : keyword + " " + rest, lineNo));
                continue;
            }
            if (function != null) {
                functionLine(function, keyword, rest, lineNo);
                continue;
            }
            switch (keyword) {
                case "tag":
                    tag = rest;
                    break;
                case "type": {
                    String[] desc = splitComment(rest);
                    String[] p = desc[0].split("\\s+");
                    if (p.length != 2) {
                        throw new IllegalArgumentException("line " + lineNo + ": type PATH SIZE [: description]");
                    }
                    type = new TypeSpec();
                    type.path = p[0];
                    type.size = Integer.decode(p[1]);
                    type.description = desc[1];
                    type.line = lineNo;
                    if (!type.path.startsWith("/")) {
                        throw new IllegalArgumentException("line " + lineNo + ": type path must be absolute");
                    }
                    types.put(type.path, type);
                    break;
                }
                case "class": {
                    String[] p = rest.split("\\s+");
                    if (p.length != 2) {
                        throw new IllegalArgumentException("line " + lineNo + ": class NAMESPACE STRUCT_PATH");
                    }
                    classes.put(p[0], p[1]);
                    break;
                }
                case "function":
                    function = new FunctionSpec();
                    function.path = rest;
                    function.line = lineNo;
                    functionSpecs.add(function);
                    break;
                default:
                    throw new IllegalArgumentException("line " + lineNo + ": unknown keyword " + keyword);
            }
        }
        if (tag == null) {
            throw new IllegalArgumentException("Spec has no tag line");
        }
        if (type != null || function != null) {
            throw new IllegalArgumentException("Spec ends inside a block");
        }
    }

    private static String[] splitComment(String s) {
        int colon = s.indexOf(" : ");
        return colon < 0 ? new String[] {s.trim(), ""} : new String[] {s.substring(0, colon).trim(), s.substring(colon + 3).trim()};
    }

    private static FieldSpec field(String s, int lineNo) {
        String[] c = splitComment(s);
        String[] p = c[0].split("\\s+");
        if (p.length != 3) {
            throw new IllegalArgumentException("line " + lineNo + ": field OFFSET TYPE NAME [: comment]");
        }
        FieldSpec f = new FieldSpec();
        f.offset = Integer.decode(p[0]);
        f.type = p[1];
        f.name = p[2];
        f.comment = c[1].isEmpty() ? null : c[1];
        return f;
    }

    private static void functionLine(FunctionSpec fs, String keyword, String rest, int lineNo) {
        String[] p = rest.split("\\s+");
        switch (keyword) {
            case "proto":
                if (p.length < 2 || p.length > 3 || (p.length == 3 && !p[2].equals(CUSTOM))) {
                    throw new IllegalArgumentException("line " + lineNo + ": proto CONVENTION RETURN_TYPE [custom]");
                }
                fs.convention = p[0];
                fs.returnType = p[1];
                fs.custom = p.length == 3;
                break;
            case "param": {
                if (p.length < 2 || p.length > 3) {
                    throw new IllegalArgumentException("line " + lineNo + ": param NAME|- TYPE [@STORAGE]");
                }
                ParamSpec ps = new ParamSpec();
                ps.name = p[0];
                ps.type = p[1];
                ps.storage = p.length == 3 ? p[2].substring(1) : null;
                fs.params.add(ps);
                break;
            }
            case "local":
            case "global": {
                if (p.length < 2 || p.length > 3) {
                    throw new IllegalArgumentException("line " + lineNo + ": " + keyword + " LOCATOR NAME [TYPE]");
                }
                VarSpec v = new VarSpec();
                v.locator = p[0];
                v.name = p[1];
                v.type = p.length == 3 ? p[2] : KEEP_TYPE;
                v.line = lineNo;
                (keyword.equals("local") ? fs.locals : fs.globals).add(v);
                break;
            }
            default:
                throw new IllegalArgumentException("line " + lineNo + ": unknown function keyword " + keyword);
        }
    }

    // ---- prerequisite: the __EH_prolog call fixup ----

    // Without the EH_prolog call fixup (the helper sets up the EBP frame), every caller decompiles with unaff_EBP and
    // no parameters. Names the helper like the x86win.cspec fixup target and sets the fixup.
    private void prolog() throws Exception {
        Map<Address, Integer> callees = new LinkedHashMap<>();
        for (Function f : currentProgram.getFunctionManager().getFunctions(true)) {
            if (!hasTag(f)) {
                continue;
            }
            Instruction first = getInstructionAt(f.getEntryPoint());
            Instruction second = first == null ? null : first.getNext();
            if (first == null || second == null || !first.getMnemonicString().equals("MOV")
                    || !"EAX".equals(regName(first, 0)) || first.getScalar(1) == null || !second.getFlowType().isCall()) {
                continue;
            }
            for (Address a : second.getFlows()) {
                callees.merge(a, 1, Integer::sum);
            }
        }
        for (Map.Entry<Address, Integer> e : callees.entrySet()) {
            Function f = getFunctionAt(e.getKey());
            if (f == null || !isPrologBody(f)) {
                continue;
            }
            boolean named = f.getName().equals(PROLOG_NAME);
            if (!named && f.getSymbol().getSource() != SourceType.DEFAULT) {
                kept.add("prolog helper @" + f.getEntryPoint() + " keeps `" + f.getName() + "` (" + f.getSymbol().getSource()
                    + "); call fixup not set");
                continue;
            }
            if (!named) {
                changes.add("prolog helper " + f.getName() + " -> " + PROLOG_NAME);
                if (apply) {
                    f.setName(PROLOG_NAME, SourceType.USER_DEFINED);
                }
            }
            if (!PROLOG_FIXUP.equals(f.getCallFixup())) {
                changes.add("prolog helper call fixup " + PROLOG_FIXUP);
                if (apply) {
                    f.setCallFixup(PROLOG_FIXUP);
                }
            }
            anchors.add("prolog helper `" + PROLOG_NAME + "` @" + f.getEntryPoint() + ", call fixup " + PROLOG_FIXUP + ", "
                + e.getValue() + " tagged callers");
        }
    }

    private boolean isPrologBody(Function f) {
        Instruction i = getInstructionAt(f.getEntryPoint());
        for (int k = 0; k < PROLOG_MNEMONICS.length; k++, i = i == null ? null : i.getNext()) {
            if (i == null || !i.getMnemonicString().equals(PROLOG_MNEMONICS[k])) {
                return false;
            }
        }
        Instruction second = getInstructionAt(f.getEntryPoint()).getNext();
        return "EAX".equals(regName(second, 0));
    }

    private static String regName(Instruction i, int operand) {
        if (i.getNumOperands() <= operand) {
            return null;
        }
        Object[] objs = i.getOpObjects(operand);
        return objs.length == 1 && objs[0] instanceof Register ? ((Register) objs[0]).getName() : null;
    }

    // ---- types ----

    private void ensureType(TypeSpec t) throws Exception {
        int slash = t.path.lastIndexOf('/');
        CategoryPath cat = new CategoryPath(slash == 0 ? "/" : t.path.substring(0, slash));
        String name = t.path.substring(slash + 1);
        DataType existing = dtm.getDataType(t.path);
        if (existing != null && !(existing instanceof Structure)) {
            anomalies.add("line " + t.line + ": " + t.path + " exists and is not a structure");
            return;
        }
        if (existing == null && apply) {
            // Placeholder first, so that fields may point to it or to types declared later.
            existing = dtm.addDataType(new StructureDataType(cat, name, t.size, dtm), DataTypeConflictHandler.KEEP_HANDLER);
        }
        StructureDataType wanted = new StructureDataType(cat, name, t.size, dtm);
        wanted.setDescription(t.description);
        for (FieldSpec f : t.fields) {
            DataType ft = resolveType(f.type, "line " + t.line + " field " + f.name);
            if (ft == null) {
                return;
            }
            if (f.offset + ft.getLength() > t.size) {
                anomalies.add(t.path + "." + f.name + " ends past the structure size");
                return;
            }
            wanted.replaceAtOffset(f.offset, ft, ft.getLength(), f.name, f.comment);
        }
        if (existing != null && sameStructure((Structure) existing, wanted)) {
            typeRows.add("| `" + t.path + "` | " + hex(t.size) + " | " + t.fields.size() + " | unchanged |");
            return;
        }
        changes.add("type " + t.path);
        typeRows.add("| `" + t.path + "` | " + hex(t.size) + " | " + t.fields.size() + " | "
            + (existing == null ? "would create" : apply ? "created / updated" : "would update") + " |");
        if (apply) {
            ((Structure) existing).replaceWith(wanted);
            existing.setDescription(t.description);
        }
    }

    private static boolean sameStructure(Structure a, Structure b) {
        if (a.getLength() != b.getLength() || !String.valueOf(a.getDescription()).equals(String.valueOf(b.getDescription()))
                || a.getNumDefinedComponents() != b.getNumDefinedComponents()) {
            return false;
        }
        DataTypeComponent[] ca = a.getDefinedComponents();
        DataTypeComponent[] cb = b.getDefinedComponents();
        for (int i = 0; i < ca.length; i++) {
            if (ca[i].getOffset() != cb[i].getOffset() || ca[i].getLength() != cb[i].getLength()
                    || !String.valueOf(ca[i].getFieldName()).equals(String.valueOf(cb[i].getFieldName()))
                    || !String.valueOf(ca[i].getComment()).equals(String.valueOf(cb[i].getComment()))
                    || !sameType(ca[i].getDataType(), cb[i].getDataType())) {
                return false;
            }
        }
        return true;
    }

    private static boolean sameType(DataType a, DataType b) {
        return a.getDisplayName().equals(b.getDisplayName()) && a.isEquivalent(b);
    }

    // TYPE := BASE ['*'...] ['[' N ']']; BASE is an absolute path, a built-in name or a unique type name.
    private DataType resolveType(String token, String where) {
        String s = token;
        int count = -1;
        if (s.endsWith("]")) {
            int open = s.lastIndexOf('[');
            count = Integer.decode(s.substring(open + 1, s.length() - 1));
            s = s.substring(0, open);
        }
        int pointers = 0;
        while (s.endsWith("*")) {
            pointers++;
            s = s.substring(0, s.length() - 1);
        }
        DataType base;
        if (s.startsWith("/")) {
            base = dtm.getDataType(s);
            TypeSpec declared = types.get(s);
            if (base == null && declared != null && !apply) {
                // Dry run: a type of this spec that does not exist yet.
                int slash = s.lastIndexOf('/');
                base = new StructureDataType(new CategoryPath(s.substring(0, slash)), s.substring(slash + 1), declared.size, dtm);
            }
        } else {
            base = BuiltInDataTypeManager.getDataTypeManager().getDataType(new CategoryPath("/"), s);
            if (base == null) {
                List<DataType> found = new ArrayList<>();
                dtm.findDataTypes(s, found);
                String baseName = s;
                found.removeIf(d -> !d.getName().equals(baseName));
                if (found.size() > 1) {
                    anomalies.add(where + ": type name `" + s + "` is ambiguous, use a path");
                    return null;
                }
                base = found.isEmpty() ? null : found.get(0);
            }
        }
        if (base == null) {
            anomalies.add(where + ": unknown type `" + s + "`");
            return null;
        }
        DataType t = base;
        for (int i = 0; i < pointers; i++) {
            t = new PointerDataType(t, dtm);
        }
        if (count >= 0) {
            t = new ArrayDataType(t, count, t.getLength(), dtm);
        }
        return t;
    }

    // ---- classes ----

    // The auto `this` of a __thiscall function with dynamic storage takes the structure named like the function's
    // class namespace; a plain namespace gives `void *`. Converts the namespace to a class when needed and requires
    // that exactly the given structure carries that name.
    private void ensureClass(String namespace, String structPath) throws Exception {
        Namespace ns = currentProgram.getSymbolTable().getNamespace(namespace, currentProgram.getGlobalNamespace());
        if (ns == null) {
            anomalies.add("class " + namespace + ": namespace not found");
            return;
        }
        String structName = structPath.substring(structPath.lastIndexOf('/') + 1);
        if (!structName.equals(namespace)) {
            anomalies.add("class " + namespace + ": structure " + structPath + " has another name");
            return;
        }
        List<DataType> same = new ArrayList<>();
        dtm.findDataTypes(namespace, same);
        same.removeIf(d -> !(d instanceof Structure) || !d.getName().equals(namespace));
        boolean wantedExists = dtm.getDataType(structPath) != null;
        if (same.size() > (wantedExists ? 1 : 0)) {
            anomalies.add("class " + namespace + ": several structures named " + namespace + ": " + paths(same));
        }
        if (ns.getSymbol().getSymbolType() == SymbolType.CLASS) {
            return;
        }
        changes.add("namespace " + namespace + " -> class");
        if (apply) {
            NamespaceUtils.convertNamespaceToClass(ns);
        }
    }

    private static String paths(List<DataType> list) {
        List<String> out = new ArrayList<>();
        for (DataType d : list) {
            out.add(d.getPathName());
        }
        return out.toString();
    }

    // ---- functions ----

    private boolean hasTag(Function f) {
        for (FunctionTag t : f.getTags()) {
            if (t.getName().equals(tag)) {
                return true;
            }
        }
        return false;
    }

    private Map<String, Function> taggedFunctions() {
        Map<String, Function> out = new LinkedHashMap<>();
        for (Function f : currentProgram.getFunctionManager().getFunctions(true)) {
            if (hasTag(f)) {
                if (out.put(f.getName(true), f) != null) {
                    anomalies.add("two functions named `" + f.getName(true) + "` with tag " + tag);
                }
            }
        }
        return out;
    }

    private void applyFunction(Function f, FunctionSpec fs) throws Exception {
        String where = "`" + fs.path + "`";
        String proto = fs.convention == null ? "unchanged" : prototype(f, fs, where);
        List<String> notes = new ArrayList<>();
        for (VarSpec v : fs.locals) {
            notes.add(local(f, v, where));
        }
        for (VarSpec v : fs.globals) {
            notes.add(global(f, v, where));
        }
        rows.add("| " + where + " | " + proto + " | " + (notes.isEmpty() ? "" : String.join("; ", notes)) + " |");
    }

    private String prototype(Function f, FunctionSpec fs, String where) throws Exception {
        PrototypeModel model = currentProgram.getCompilerSpec().getCallingConvention(fs.convention);
        if (model == null) {
            anomalies.add(where + ": unknown calling convention " + fs.convention);
            return "error";
        }
        DataType ret = resolveType(fs.returnType, where + " return");
        if (ret == null) {
            return "error";
        }
        Long purge = retPurge(f);
        if (purge == null) {
            anomalies.add(where + ": RET instructions disagree on the stack purge");
            return "error";
        }
        List<ParameterImpl> params = new ArrayList<>();
        int stackBytes = 0;
        for (ParamSpec p : fs.params) {
            DataType t = resolveType(p.type, where + " param " + p.name);
            if (t == null) {
                return "error";
            }
            String name = p.name.equals(DEFAULT_PARAM) ? null : p.name;
            if (fs.custom) {
                if (p.storage == null) {
                    anomalies.add(where + ": custom prototype needs @storage for " + p.name);
                    return "error";
                }
                VariableStorage storage = storage(p.storage, t);
                if (storage == null) {
                    anomalies.add(where + ": bad storage @" + p.storage);
                    return "error";
                }
                if (storage.isStackStorage()) {
                    stackBytes = Math.max(stackBytes, storage.getStackOffset() - RETURN_ADDRESS_SIZE + slots(t) * STACK_SLOT);
                }
                params.add(new ParameterImpl(name, t, storage, currentProgram));
            } else {
                if (p.storage != null) {
                    anomalies.add(where + ": @storage needs a custom prototype");
                    return "error";
                }
                params.add(new ParameterImpl(name, t, currentProgram));
            }
        }
        if (!fs.custom) {
            int registerParams = fs.convention.equals(FASTCALL) ? 2 : 0;
            for (int i = registerParams; i < params.size(); i++) {
                stackBytes += slots(params.get(i).getDataType()) * STACK_SLOT;
            }
        }
        boolean calleePops = fs.convention.equals(STDCALL) || fs.convention.equals(THISCALL) || fs.convention.equals(FASTCALL);
        long expected = calleePops ? stackBytes : 0;
        if (purge != expected) {
            anomalies.add(where + ": RET purges " + purge + " bytes, the spec's " + fs.convention + " stack parameters " + expected);
            return "error";
        }
        String inputs = registerInputs(f, fs);
        if (inputs.startsWith("!")) {
            anomalies.add(where + ": " + inputs.substring(1));
            return "error";
        }
        String text = fs.convention + " " + ret.getDisplayName() + " (" + describe(fs) + ")" + (fs.custom ? " custom" : "")
            + (inputs.isEmpty() ? "" : "; " + inputs);
        if (samePrototype(f, fs, ret, params)) {
            countParams(fs);
            return "unchanged: " + text;
        }
        changes.add("prototype " + fs.path);
        if (!apply) {
            return "would set: " + text;
        }
        ReturnParameterImpl returnVar = fs.custom
            ? new ReturnParameterImpl(ret, returnStorage(model, ret), currentProgram)
            : new ReturnParameterImpl(ret, currentProgram);
        if (!fs.custom && f.hasCustomVariableStorage()) {
            f.setCustomVariableStorage(false);
        }
        f.updateFunction(fs.convention, returnVar, params,
            fs.custom ? Function.FunctionUpdateType.CUSTOM_STORAGE : Function.FunctionUpdateType.DYNAMIC_STORAGE_FORMAL_PARAMS,
            true, SourceType.USER_DEFINED);
        prototypesSet++;
        countParams(fs);
        String thisCheck = checkThis(f, fs, where);
        return "set: " + text + thisCheck;
    }

    private void countParams(FunctionSpec fs) {
        for (ParamSpec p : fs.params) {
            if (!p.name.equals(DEFAULT_PARAM)) {
                paramsNamed++;
            }
        }
    }

    private String checkThis(Function f, FunctionSpec fs, String where) {
        if (fs.custom || !fs.convention.equals(THISCALL)) {
            return "";
        }
        Parameter p = f.getParameter(0);
        String want = classes.get(f.getParentNamespace().getName(true));
        String got = p == null ? "none" : p.getDataType().getPathName();
        if (want == null || !got.equals(want + " *")) {
            anomalies.add(where + ": `this` is " + got + (want == null ? " (no class line for its namespace)" : ", expected " + want + " *"));
        }
        return "";
    }

    private VariableStorage returnStorage(PrototypeModel model, DataType ret) throws Exception {
        return ret instanceof VoidDataType ? VariableStorage.VOID_STORAGE : model.getReturnLocation(ret, currentProgram);
    }

    private static int slots(DataType t) {
        return Math.max(1, (t.getLength() + STACK_SLOT - 1) / STACK_SLOT);
    }

    private VariableStorage storage(String s, DataType t) throws Exception {
        if (s.startsWith(STACK_PREFIX)) {
            return new VariableStorage(currentProgram, Integer.decode(s.substring(STACK_PREFIX.length())), t.getLength());
        }
        Register r = currentProgram.getRegister(s);
        return r == null ? null : new VariableStorage(currentProgram, r);
    }

    private String describe(FunctionSpec fs) {
        List<String> out = new ArrayList<>();
        for (ParamSpec p : fs.params) {
            out.add(p.type + " " + p.name + (p.storage == null ? "" : "@" + p.storage));
        }
        return String.join(", ", out);
    }

    private boolean samePrototype(Function f, FunctionSpec fs, DataType ret, List<ParameterImpl> params) {
        if (f.hasCustomVariableStorage() != fs.custom || !fs.convention.equals(f.getCallingConventionName())
                || f.getSignatureSource() == SourceType.DEFAULT || !sameType(f.getReturnType(), ret)) {
            return false;
        }
        List<Parameter> current = new ArrayList<>();
        for (Parameter p : f.getParameters()) {
            if (fs.custom || !p.isAutoParameter()) {
                current.add(p);
            }
        }
        if (current.size() != params.size()) {
            return false;
        }
        for (int i = 0; i < params.size(); i++) {
            Parameter c = current.get(i);
            ParameterImpl w = params.get(i);
            boolean nameOk = w.getName() == null || w.getName().isEmpty() ? c.getSource() == SourceType.DEFAULT : c.getName().equals(w.getName());
            if (!nameOk || !sameType(c.getDataType(), w.getDataType())
                    || (fs.custom && !c.getVariableStorage().equals(w.getVariableStorage()))) {
                return false;
            }
        }
        return true;
    }

    // The stack bytes the function's RET instructions pop (null when they disagree, 0 without RET).
    private Long retPurge(Function f) {
        Long purge = null;
        for (Instruction i : currentProgram.getListing().getInstructions(f.getBody(), true)) {
            if (!i.getMnemonicString().startsWith("RET")) {
                continue;
            }
            Scalar s = i.getNumOperands() > 0 ? i.getScalar(0) : null;
            long v = s == null ? 0 : s.getUnsignedValue();
            if (purge != null && purge != v) {
                return null;
            }
            purge = v;
        }
        return purge == null ? 0L : purge;
    }

    // Register parameters must be read before written (decompiler input varnodes). Returns "!<error>", or a note listing
    // the general purpose registers the decompiler sees as inputs that the prototype does not cover.
    private String registerInputs(Function f, FunctionSpec fs) {
        HighFunction hf = decompile(f);
        List<String> inputs = new ArrayList<>();
        // A function the decompiler cannot finish (e.g. under its current prototype) is checked by instructions only.
        Iterator<PcodeOpAST> ops = hf == null ? java.util.Collections.emptyIterator() : hf.getPcodeOps();
        while (ops.hasNext()) {
            for (Varnode v : ops.next().getInputs()) {
                if (v.isInput() && v.isRegister()) {
                    Register r = currentProgram.getRegister(v.getAddress(), v.getSize());
                    Register base = r == null ? null : r.getBaseRegister();
                    String name = base == null ? null : base.getName();
                    if (name != null && List.of(GP_REGISTERS).contains(name) && !inputs.contains(name)) {
                        inputs.add(name);
                    }
                }
            }
        }
        List<String> declared = new ArrayList<>();
        if (fs.custom) {
            for (ParamSpec p : fs.params) {
                Register r = p.storage == null ? null : currentProgram.getRegister(p.storage);
                if (r != null) {
                    declared.add(r.getBaseRegister().getName());
                }
            }
        } else if (fs.convention.equals(THISCALL)) {
            declared.add("ECX");
        } else if (fs.convention.equals(FASTCALL)) {
            declared.add("ECX");
            declared.add("EDX");
        }
        for (String d : declared) {
            if (!inputs.contains(d) && !readAtEntry(f, d)) {
                return "!register parameter " + d + " is not read before written";
            }
        }
        inputs.removeAll(declared);
        if (hf == null) {
            return "decompilation failed, registers checked by instructions";
        }
        return inputs.isEmpty() ? "" : "other register inputs " + inputs;
    }

    // Fallback for a register the decompiler does not report as an input (e.g. stored into an argument slot whose
    // address is taken): the entry block, followed by fall-through, reads it before any instruction writes it, or
    // hands it unchanged to a callee whose prototype takes it. The prolog helper call is stepped over, and so is any
    // call for a callee-saved register (EBX, ESI, EDI, EBP). A PUSH is a
    // save, not a use, when the function also POPs that register (callee-saved register); otherwise it pushes an
    // argument and counts as a read.
    private boolean readAtEntry(Function f, String name) {
        Register reg = currentProgram.getRegister(name);
        boolean restored = restores(f, reg);
        for (Instruction i = getInstructionAt(f.getEntryPoint()); i != null && f.getBody().contains(i.getAddress()); i = i.getNext()) {
            if (i.getMnemonicString().equals(SAVE_MNEMONIC) && restored) {
                continue; // saving a callee-saved register is not a use
            }
            for (Object o : i.getInputObjects()) {
                if (o instanceof Register && ((Register) o).getBaseRegister().equals(reg)) {
                    return true;
                }
            }
            for (Object o : i.getResultObjects()) {
                if (o instanceof Register && ((Register) o).getBaseRegister().equals(reg)) {
                    return false;
                }
            }
            if (i.getFlowType().isCall()) {
                Function callee = calledFunction(i);
                if (callee != null && passesRegister(callee, reg)) {
                    return true; // handed on unchanged to a callee that takes it
                }
                if (callee != null && callee.getName().equals(PROLOG_NAME) && !reg.getName().equals(PROLOG_SCRATCH)) {
                    continue; // the prolog helper only uses EAX
                }
                if (List.of(CALLEE_SAVED).contains(reg.getName())) {
                    continue; // a call preserves EBX, ESI, EDI and EBP
                }
                return false;
            }
            if (!i.getFlowType().isFallthrough()) {
                return false;
            }
        }
        return false;
    }

    private boolean restores(Function f, Register reg) {
        for (Instruction i : currentProgram.getListing().getInstructions(f.getBody(), true)) {
            if (i.getMnemonicString().equals(RESTORE_MNEMONIC)) {
                for (Object o : i.getResultObjects()) {
                    if (o instanceof Register && ((Register) o).getBaseRegister().equals(reg)) {
                        return true;
                    }
                }
            }
        }
        return false;
    }

    private Function calledFunction(Instruction i) {
        for (Reference r : i.getReferencesFrom()) {
            if (r.getReferenceType().isCall()) {
                return getFunctionAt(r.getToAddress());
            }
        }
        return null;
    }

    private static boolean passesRegister(Function callee, Register reg) {
        for (Parameter p : callee.getParameters()) {
            Register r = p.getVariableStorage().getRegister();
            if (r != null && r.getBaseRegister().equals(reg)) {
                return true;
            }
        }
        return false;
    }

    private HighFunction decompile(Function f) {
        DecompileResults r = decompiler.decompileFunction(f, DECOMPILE_TIMEOUT_SECONDS, monitor);
        return r == null ? null : r.getHighFunction();
    }

    // ---- locals ----

    private String local(Function f, VarSpec v, String where) throws Exception {
        HighFunction hf = decompile(f);
        if (hf == null) {
            anomalies.add(where + " line " + v.line + ": decompilation failed");
            return v.name + " error";
        }
        HighSymbol sym = findLocal(f, hf, v.locator);
        if (sym == null) {
            anomalies.add(where + " line " + v.line + ": no variable at " + v.locator);
            return v.name + " not found";
        }
        if (sym.isParameter()) {
            anomalies.add(where + " line " + v.line + ": " + v.locator + " is the parameter " + sym.getName());
            return v.name + " error";
        }
        DataType t = v.type.equals(KEEP_TYPE) ? null : resolveType(v.type, where + " local " + v.name);
        if (!v.type.equals(KEEP_TYPE) && t == null) {
            return v.name + " error";
        }
        boolean nameOk = sym.getName().equals(v.name);
        boolean typeOk = t == null || sameType(sym.getDataType(), t);
        localsNamed++;
        if (nameOk && typeOk) {
            return v.name;
        }
        changes.add("local " + fs(where) + "." + v.name);
        if (apply) {
            // A stored local of that name that the decompiler no longer uses (its storage was merged into another
            // variable after a prototype change) would block the rename.
            for (ghidra.program.model.listing.Variable stale : f.getLocalVariables()) {
                if (stale.getName().equals(v.name) && !stale.getVariableStorage().equals(sym.getStorage())) {
                    anomalies.add(where + " line " + v.line + ": removed the stale local " + v.name + " at " + stale.getVariableStorage());
                    f.removeVariable(stale);
                }
            }
            HighFunctionDBUtil.updateDBVariable(sym, v.name, t, SourceType.USER_DEFINED);
            return v.name + " (renamed " + sym.getName() + ")";
        }
        return v.name + " (would rename " + sym.getName() + ")";
    }

    private static String fs(String where) {
        return where.replace("`", "");
    }

    private HighSymbol findLocal(Function f, HighFunction hf, String locator) {
        if (locator.startsWith(STACK_PREFIX)) {
            int offset = Integer.decode(locator.substring(STACK_PREFIX.length()));
            Iterator<HighSymbol> it = hf.getLocalSymbolMap().getSymbols();
            while (it.hasNext()) {
                HighSymbol s = it.next();
                VariableStorage st = s.getStorage();
                if (!s.isParameter() && st != null && st.isStackStorage() && st.getStackOffset() == offset) {
                    return s;
                }
            }
            return null;
        }
        if (locator.startsWith(REG_PREFIX)) {
            String[] p = locator.substring(REG_PREFIX.length()).split(OFFSET_SEPARATOR);
            Register reg = currentProgram.getRegister(p[0]);
            Address at = f.getEntryPoint().add(Long.decode(p[1]));
            Iterator<PcodeOpAST> ops = hf.getPcodeOps(at);
            while (ops.hasNext()) {
                Varnode out = ops.next().getOutput();
                if (out != null && out.isRegister() && reg != null && out.getAddress().equals(reg.getAddress())
                        && out.getHigh() != null && out.getHigh().getSymbol() != null) {
                    return out.getHigh().getSymbol();
                }
            }
            return null;
        }
        if (locator.startsWith(CALL_PREFIX)) {
            String[] p = locator.substring(CALL_PREFIX.length()).split(ORDINAL_SEPARATOR);
            int ordinal = p.length > 1 ? Integer.parseInt(p[1]) : 1;
            int seen = 0;
            Iterator<PcodeOpAST> ops = hf.getPcodeOps();
            List<PcodeOpAST> calls = new ArrayList<>();
            while (ops.hasNext()) {
                PcodeOpAST op = ops.next();
                if (op.getOpcode() == PcodeOp.CALL) {
                    Function callee = getFunctionAt(op.getInput(0).getAddress());
                    if (callee != null && (callee.getName(true).equals(p[0]) || callee.getName().equals(p[0]))) {
                        calls.add(op);
                    }
                }
            }
            calls.sort((a, b) -> a.getSeqnum().getTarget().compareTo(b.getSeqnum().getTarget()));
            for (PcodeOpAST op : calls) {
                if (++seen == ordinal) {
                    Varnode out = op.getOutput();
                    HighVariable hv = out == null ? null : out.getHigh();
                    return hv == null ? null : hv.getSymbol();
                }
            }
            return null;
        }
        throw new IllegalArgumentException("Unknown local locator " + locator);
    }

    // ---- globals ----

    private String global(Function f, VarSpec v, String where) throws Exception {
        if (!v.locator.startsWith(REF_PREFIX)) {
            throw new IllegalArgumentException("Unknown global locator " + v.locator);
        }
        Address at = f.getEntryPoint().add(Long.decode(v.locator.substring(REF_PREFIX.length())));
        Instruction ins = getInstructionAt(at);
        Address target = null;
        if (ins != null) {
            for (Reference r : ins.getReferencesFrom()) {
                RefType rt = r.getReferenceType();
                if (r.isMemoryReference() && !rt.isFlow() && getFunctionAt(r.getToAddress()) == null) {
                    if (target != null && !target.equals(r.getToAddress())) {
                        target = null;
                        break;
                    }
                    target = r.getToAddress();
                }
            }
        }
        if (target == null) {
            anomalies.add(where + " line " + v.line + ": no single data reference at " + v.locator);
            return v.name + " not found";
        }
        String status;
        Symbol sym = currentProgram.getSymbolTable().getPrimarySymbol(target);
        if (sym != null && sym.getName().equals(v.name)) {
            status = "named";
        } else if (sym != null && sym.getSource() != SourceType.DEFAULT && sym.getSource() != SourceType.ANALYSIS) {
            kept.add(v.name + ": " + target + " keeps `" + sym.getName() + "` (" + sym.getSource() + ")");
            globalRows.add("| `" + v.name + "` | " + where + " " + v.locator + " | kept `" + sym.getName() + "` |");
            return v.name + " kept as " + sym.getName();
        } else {
            changes.add("global " + v.name);
            status = apply ? "labeled" : "would label";
            if (apply) {
                Symbol s = currentProgram.getSymbolTable().createLabel(target, v.name, currentProgram.getGlobalNamespace(), SourceType.USER_DEFINED);
                s.setPrimary();
            }
        }
        globalsNamed++;
        if (!v.type.equals(KEEP_TYPE)) {
            DataType t = resolveType(v.type, where + " global " + v.name);
            if (t != null) {
                status += globalType(target, t, v.name);
            }
        }
        globalRows.add("| `" + v.name + "` | " + where + " " + v.locator + " | " + status + " |");
        return v.name;
    }

    private String globalType(Address target, DataType t, String name) throws Exception {
        Data d = getDataAt(target);
        if (d != null && sameType(d.getDataType(), t)) {
            return "";
        }
        Data containing = getDataContaining(target);
        if (containing != null && containing.getMinAddress().compareTo(target) < 0 && containing.isDefined()) {
            anomalies.add("global " + name + " lies inside data at " + containing.getMinAddress());
            return ", type not set";
        }
        Address end = target.add(t.getLength() - 1);
        for (Data other = getDataAt(target); other != null && other.getMinAddress().compareTo(end) <= 0; other = getDataAfter(other)) {
            if (other.isDefined() && !Undefined.isUndefined(other.getDataType())) {
                if (!other.getMinAddress().equals(target)) {
                    anomalies.add("global " + name + " overlaps data at " + other.getMinAddress());
                    return ", type not set";
                }
            }
        }
        changes.add("global type " + name);
        if (apply) {
            clearListing(target, end);
            createData(target, t);
            return ", typed " + t.getDisplayName();
        }
        return ", would type " + t.getDisplayName();
    }

    // ---- report ----

    private void report(PrintWriter out, String spec, int taggedCount) {
        out.println("# variable names: " + tag);
        out.println();
        out.println("Program: " + currentProgram.getName());
        out.println("SHA-256: " + currentProgram.getExecutableSHA256());
        out.println("Spec: " + Path.of(spec).getFileName());
        out.println("Mode: " + (apply ? "analysis and " + APPLY_FLAG : "analysis only"));
        out.println();
        out.println("| Item | Count |");
        out.println("|---|---:|");
        out.println("| Functions with tag " + tag + " | " + taggedCount + " |");
        out.println("| Functions in the spec | " + functionSpecs.size() + " |");
        out.println("| Prototypes set in this run | " + prototypesSet + " |");
        out.println("| Named parameters | " + paramsNamed + " |");
        out.println("| Named locals | " + localsNamed + " |");
        out.println("| Named globals | " + globalsNamed + " |");
        out.println("| Types | " + types.size() + " |");
        out.println("| Anomalies | " + anomalies.size() + " |");
        out.println();
        out.println("Changes: " + (changes.isEmpty() ? "none" : changes.size() + " " + changes));
        out.println();
        out.println("## Types");
        out.println();
        out.println("| Type | Size | Fields | Status |");
        out.println("|---|---:|---:|---|");
        typeRows.forEach(out::println);
        out.println();
        out.println("## Functions");
        out.println();
        out.println("| Function | Prototype | Locals and globals |");
        out.println("|---|---|---|");
        rows.forEach(out::println);
        out.println();
        out.println("## Globals");
        out.println();
        out.println("| Name | Locator | Status |");
        out.println("|---|---|---|");
        globalRows.forEach(out::println);
        section(out, "Anchors", anchors);
        section(out, "Kept names", kept);
        section(out, "Anomalies", anomalies);
    }

    private static void section(PrintWriter out, String title, List<String> lines) {
        out.println();
        out.println("## " + title);
        out.println();
        if (lines.isEmpty()) {
            out.println("None.");
        }
        for (String line : lines) {
            out.println("- " + line);
        }
    }

    private static String hex(long value) {
        return String.format("0x%x", value);
    }
}
