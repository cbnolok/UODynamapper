// Fill the /Gamebryo class placeholders of GamebryoClasses.java with member layouts from Gamebryo 2.6
// SDK PDB types, only where the recovered sizes prove the 2.6 layout fits.
// @category ECResearch
import ghidra.app.script.GhidraScript;
import ghidra.framework.model.DomainFile;
import ghidra.framework.model.DomainFolder;
import ghidra.program.model.data.Array;
import ghidra.program.model.data.ArrayDataType;
import ghidra.program.model.data.BitFieldDataType;
import ghidra.program.model.data.BuiltInDataType;
import ghidra.program.model.data.Category;
import ghidra.program.model.data.CategoryPath;
import ghidra.program.model.data.Composite;
import ghidra.program.model.data.DataType;
import ghidra.program.model.data.DataTypeComponent;
import ghidra.program.model.data.DataTypeConflictHandler;
import ghidra.program.model.data.DataTypeManager;
import ghidra.program.model.data.EnumDataType;
import ghidra.program.model.data.FunctionDefinition;
import ghidra.program.model.data.Pointer;
import ghidra.program.model.data.PointerDataType;
import ghidra.program.model.data.Structure;
import ghidra.program.model.data.StructureDataType;
import ghidra.program.model.data.TypeDef;
import ghidra.program.model.data.TypedefDataType;
import ghidra.program.model.data.Undefined;
import ghidra.program.model.data.Union;
import ghidra.program.model.data.UnionDataType;
import ghidra.program.model.data.VoidDataType;
import ghidra.program.model.listing.Program;
import ghidra.program.model.symbol.Namespace;
import ghidra.program.model.symbol.Symbol;
import ghidra.program.model.symbol.SymbolTable;
import java.io.PrintWriter;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardOpenOption;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.HashMap;
import java.util.HashSet;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.TreeMap;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

public class ApplyGamebryoLayouts extends GhidraScript {
    private static final String APPLY_FLAG = "--apply";
    private static final int REPORT_ARGS = 1;
    private static final int APPLY_ARGS = REPORT_ARGS + 1;
    private static final int POINTER_SIZE = 4;
    private static final int HEX_RADIX = 16;
    // Project folder holding the analyzed Gamebryo 2.6 SDK DLLs with their PDBs.
    private static final String SOURCE_FOLDER = "/gamebryo26";
    private static final String PDB_CATEGORY_SUFFIX = ".pdb";
    private static final String CATEGORY = "/Gamebryo";
    private static final String COPY_CATEGORY = "/Gamebryo26";
    private static final String SCOPE = "::";
    // Ghidra's PDB loader fills inherited regions (vptr, base members) with fields of this name.
    private static final String PADDING = "_padding_";
    // Every class GamebryoClasses.java finds derives from NiRefObject { vptr; m_uiRefCount }.
    private static final String REF_OBJECT = "NiRefObject";
    private static final String VFTABLE = "vftable";
    // Static NiRTTI member, defined only in the DLL that implements the class.
    private static final String RTTI_MEMBER = "ms_RTTI";
    private static final String BASE = "base";
    // The member layouts go to OUTPUT_FILE with this suffix before the extension.
    private static final String MEMBERS_SUFFIX = "_members";
    private static final String CLASS_PREFIX = "Gamebryo class ";
    private static final String LAYOUT_MARKER = "; layout from Gamebryo 2.6 ";
    private static final String INFERRED = "; size inferred: 2.6 size within the recorded bounds";
    private static final String UNBOUNDED = "; size inferred: no upper bound (no sized descendant)";
    // Classes without CreateObject and without a sized descendant have only a lower bound, which
    // proves nothing about the 2.6 layout; set to true to fill them anyway (marked UNBOUNDED).
    private static final boolean APPLY_UNBOUNDED = false;
    // GamebryoClasses.java description: name, parent, then exact size or bounds.
    private static final Pattern DESCRIPTION = Pattern.compile("Gamebryo class (\\S+), parent (\\S+); (?:"
        + "size from \\S+::CreateObject 0x[0-9a-f]+|size unknown, >= 0x([0-9a-f]+) \\([^)]*\\), "
        + "(?:<= 0x([0-9a-f]+) \\([^)]*\\)|no sized descendant))");
    private static final Pattern EXACT_SIZE = Pattern.compile("size from \\S+::CreateObject 0x[0-9a-f]+$");
    private static final int GROUP_NAME = 1;
    private static final int GROUP_PARENT = 2;
    private static final int GROUP_LOWER = 3;
    private static final int GROUP_UPPER = 4;
    private static final String NO_PARENT = "none";
    // Kinds of types copied to COPY_CATEGORY.
    private static final String STRUCTURES = "structure";
    private static final String UNIONS = "union";
    private static final String ENUMS = "enum";
    private static final String TYPEDEFS = "typedef";
    private static final String STUBS = "stub (pointer target, no members)";

    private enum State { PLACEHOLDER, APPLIED, EDITED }

    private enum Outcome { NONE, MISSING, MISMATCH, UNBOUNDED, REJECTED, EXACT, INFERRED }

    private static final class Source {
        String name;
        Program program;
        final List<Category> pdbRoots = new ArrayList<>();
    }

    private static final class ClassInfo {
        String name;
        String parentName;
        ClassInfo parent;
        final List<ClassInfo> children = new ArrayList<>();
        Structure struct;
        int startLength;
        String original;
        State state;
        String editReason;
        long size = -1;
        long lower = POINTER_SIZE;
        long upper = -1;
        Structure source;
        String sourceName;
        String sourceConflict;
        int firstOwn;
        Outcome outcome = Outcome.NONE;
        String reason;
        String field0 = "";
        // The 2.6 layout as built this run (non-DB in report-only mode), for descendants' base field.
        DataType planned;

        boolean hasOwn() {
            return source != null && firstOwn < source.getLength();
        }

        boolean eligible() {
            return outcome == Outcome.EXACT || outcome == Outcome.INFERRED;
        }

        boolean hasLayout() {
            return eligible() && state != State.EDITED;
        }
    }

    private final Map<String, ClassInfo> byName = new TreeMap<>();
    private final List<String> others = new ArrayList<>();
    private final List<String> anomalies = new ArrayList<>();
    private final Map<String, Integer> changes = new LinkedHashMap<>();
    // Types created in COPY_CATEGORY by kind, and other member conversion counts.
    private final Map<String, Integer> copies = new LinkedHashMap<>();
    private final Map<String, Integer> notes = new LinkedHashMap<>();
    private final Set<String> valueTypes = new HashSet<>();
    private final Map<DataType, String> copiedTypes = new LinkedHashMap<>();
    private final Map<String, DataType> converted = new HashMap<>();
    private DataTypeManager dtm;
    private boolean apply;
    private Structure refObject;

    public void run() throws Exception {
        String[] args = getScriptArgs();
        apply = args.length == APPLY_ARGS && args[REPORT_ARGS].equals(APPLY_FLAG);
        if (args.length != REPORT_ARGS && !apply) {
            throw new IllegalArgumentException("Expected OUTPUT_FILE [" + APPLY_FLAG + "]");
        }
        dtm = currentProgram.getDataTypeManager();
        Category root = dtm.getCategory(new CategoryPath(CATEGORY));
        if (root == null) {
            throw new IllegalStateException("No " + CATEGORY + " category; run GamebryoClasses.java --apply first");
        }
        DomainFolder folder = state.getProject().getProjectData().getFolder(SOURCE_FOLDER);
        if (folder == null) {
            throw new IllegalStateException("No project folder " + SOURCE_FOLDER);
        }
        Path outputPath = Path.of(args[0]);
        String fileName = outputPath.getFileName().toString();
        int dot = fileName.lastIndexOf('.');
        Path membersPath = outputPath.resolveSibling(dot > 0 ? fileName.substring(0, dot) + MEMBERS_SUFFIX
            + fileName.substring(dot) : fileName + MEMBERS_SUFFIX);
        try (PrintWriter output = new PrintWriter(Files.newBufferedWriter(outputPath, StandardOpenOption.CREATE_NEW));
                PrintWriter members = new PrintWriter(Files.newBufferedWriter(membersPath,
                    StandardOpenOption.CREATE_NEW))) {
            List<Source> sources = new ArrayList<>();
            try {
                List<DomainFile> files = new ArrayList<>(List.of(folder.getFiles()));
                files.sort(Comparator.comparing(DomainFile::getName));
                for (DomainFile file : files) {
                    if (!Program.class.isAssignableFrom(file.getDomainObjectClass())) {
                        continue;
                    }
                    Source source = new Source();
                    source.name = file.getName();
                    // Read-only: the source programs are never modified.
                    source.program = (Program) file.getReadOnlyDomainObject(this, DomainFile.DEFAULT_VERSION, monitor);
                    for (Category category : source.program.getDataTypeManager().getRootCategory().getCategories()) {
                        if (category.getName().endsWith(PDB_CATEGORY_SUFFIX)) {
                            source.pdbRoots.add(category);
                        }
                    }
                    sources.add(source);
                }
                if (sources.isEmpty()) {
                    throw new IllegalStateException("No programs in " + SOURCE_FOLDER);
                }
                process(root, sources);
                report(output, sources);
                members(members);
            } finally {
                for (Source source : sources) {
                    source.program.release(this);
                }
            }
        }
    }

    private void process(Category root, List<Source> sources) throws Exception {
        collect(root, new ArrayList<>());
        for (ClassInfo info : byName.values()) {
            info.parent = info.parentName == null || info.parentName.equals(NO_PARENT) ? null : byName.get(info.parentName);
            if (info.parentName != null && !info.parentName.equals(NO_PARENT) && info.parent == null) {
                anomalies.add("`" + info.name + "` names parent `" + info.parentName + "`, which has no structure");
            }
            if (info.parent != null) {
                info.parent.children.add(info);
            }
        }
        refObject = findSource(sources, REF_OBJECT, null);
        if (refObject == null) {
            anomalies.add("No 2.6 `" + REF_OBJECT + "` structure; roots keep their vftable field only");
        }
        for (ClassInfo info : byName.values()) {
            if (info.state != State.EDITED) {
                info.source = findSource(sources, info.name, info);
            }
            if (info.source != null) {
                info.firstOwn = info.source.getLength();
                for (DataTypeComponent component : info.source.getDefinedComponents()) {
                    if (!isPadding(component)) {
                        info.firstOwn = Math.min(info.firstOwn, component.getOffset());
                    }
                }
            }
        }
        List<ClassInfo> ordered = preorder();
        for (ClassInfo info : ordered) {
            evaluate(info);
        }
        for (ClassInfo info : ordered) {
            if (info.eligible()) {
                collectOwnValueTypes(info.source);
            }
        }
        if (refObject != null) {
            collectOwnValueTypes(refObject);
        }
        for (ClassInfo info : ordered) {
            if (info.eligible()) {
                build(info);
            } else if (info.state == State.APPLIED) {
                anomalies.add("`" + info.name + "` was filled by an earlier run but no longer passes the checks; kept");
            }
        }
    }

    // Classes from /Gamebryo structures; nested names live in subcategories.
    private void collect(Category category, List<String> scope) {
        for (DataType type : category.getDataTypes()) {
            if (type instanceof Pointer) {
                continue;
            }
            List<String> parts = new ArrayList<>(scope);
            parts.add(type.getName());
            String name = String.join(SCOPE, parts);
            String description = type.getDescription();
            if (!(type instanceof Structure struct) || description == null || !description.startsWith(CLASS_PREFIX)) {
                others.add(name);
                continue;
            }
            ClassInfo info = new ClassInfo();
            info.name = name;
            info.struct = struct;
            info.startLength = struct.getLength();
            int marker = description.indexOf(LAYOUT_MARKER);
            info.state = marker >= 0 ? State.APPLIED : State.PLACEHOLDER;
            info.original = marker >= 0 ? description.substring(0, marker) : description;
            Matcher matcher = DESCRIPTION.matcher(info.original);
            if (!matcher.matches() || !matcher.group(GROUP_NAME).equals(name)) {
                edited(info, "description changed");
            } else {
                info.parentName = matcher.group(GROUP_PARENT);
                if (EXACT_SIZE.matcher(info.original).find()) {
                    info.size = struct.getLength();
                } else {
                    info.lower = Long.parseLong(matcher.group(GROUP_LOWER), HEX_RADIX);
                    info.upper = matcher.group(GROUP_UPPER) != null ? Long.parseLong(matcher.group(GROUP_UPPER), HEX_RADIX) : -1;
                }
                if (info.state == State.PLACEHOLDER) {
                    checkPlaceholder(info);
                }
            }
            byName.put(name, info);
        }
        for (Category child : category.getCategories()) {
            List<String> parts = new ArrayList<>(scope);
            parts.add(child.getName());
            collect(child, parts);
        }
    }

    // A placeholder holds only field 0: `void *vftable` or the sized ancestor as `base`.
    private void checkPlaceholder(ClassInfo info) {
        DataTypeComponent[] components = info.struct.getDefinedComponents();
        if (components.length != 1 || components[0].getOffset() != 0) {
            edited(info, components.length + " defined components");
            return;
        }
        DataTypeComponent field = components[0];
        boolean vftable = VFTABLE.equals(field.getFieldName()) && field.getDataType() instanceof Pointer;
        boolean base = BASE.equals(field.getFieldName()) && field.getDataType() instanceof Structure
            && field.getDataType().getCategoryPath().isAncestorOrSelf(new CategoryPath(CATEGORY));
        if (!vftable && !base) {
            edited(info, "field 0 is `" + field.getFieldName() + "`");
            return;
        }
        int expected = info.size >= 0 ? (int) info.size : field.getLength();
        if (info.struct.getLength() != expected) {
            edited(info, "length " + hex(info.struct.getLength()));
        }
    }

    private static void edited(ClassInfo info, String reason) {
        info.state = State.EDITED;
        info.editReason = reason;
    }

    // The 2.6 class structure of the same (nested) name. Every PDB that uses a class has its type, so
    // prefer the DLL defining <Class>::ms_RTTI; all sources must agree on the member layout.
    private Structure findSource(List<Source> sources, String name, ClassInfo info) {
        String[] parts = name.split(SCOPE);
        Structure found = null;
        String foundIn = null;
        boolean foundHome = false;
        List<Structure> candidates = new ArrayList<>();
        List<String> candidateSources = new ArrayList<>();
        for (Source source : sources) {
            boolean home = definesRtti(source.program, parts);
            for (Category pdbRoot : source.pdbRoots) {
                CategoryPath category = pdbRoot.getCategoryPath();
                for (int i = 0; i < parts.length - 1; i++) {
                    category = new CategoryPath(category, parts[i]);
                }
                DataType type = source.program.getDataTypeManager().getDataType(category, parts[parts.length - 1]);
                if (!(type instanceof Structure struct) || struct.isNotYetDefined() || struct.getLength() == 0) {
                    continue;
                }
                candidates.add(struct);
                candidateSources.add(source.name);
                if (found == null || (home && !foundHome)) {
                    found = struct;
                    foundIn = source.name;
                    foundHome = home;
                }
            }
        }
        for (int i = 0; i < candidates.size() && info != null; i++) {
            if (!signature(candidates.get(i)).equals(signature(found))) {
                info.sourceConflict = foundIn + " " + hex(found.getLength()) + ", " + candidateSources.get(i) + " "
                    + hex(candidates.get(i).getLength());
                break;
            }
        }
        if (info != null) {
            info.sourceName = foundIn;
        }
        return found;
    }

    private static boolean definesRtti(Program program, String[] parts) {
        SymbolTable symbols = program.getSymbolTable();
        Namespace namespace = program.getGlobalNamespace();
        for (String part : parts) {
            namespace = symbols.getNamespace(part, namespace);
            if (namespace == null) {
                return false;
            }
        }
        for (Symbol symbol : symbols.getSymbols(RTTI_MEMBER, namespace)) {
            if (!symbol.isExternal()) {
                return true;
            }
        }
        return false;
    }

    private static String signature(Structure struct) {
        StringBuilder signature = new StringBuilder(hex(struct.getLength()));
        for (DataTypeComponent component : struct.getDefinedComponents()) {
            if (!isPadding(component)) {
                signature.append(';').append(component.getOffset()).append(':').append(component.getFieldName())
                    .append(':').append(component.getLength()).append(':').append(component.getDataType().getName());
            }
        }
        return signature.toString();
    }

    private List<ClassInfo> preorder() {
        List<ClassInfo> ordered = new ArrayList<>();
        Set<ClassInfo> visited = new HashSet<>();
        for (ClassInfo info : byName.values()) {
            if (info.parent == null) {
                preorder(info, ordered, visited);
            }
        }
        for (ClassInfo info : byName.values()) {
            if (!visited.contains(info)) {
                anomalies.add("`" + info.name + "` is in a parent cycle");
            }
        }
        return ordered;
    }

    private static void preorder(ClassInfo info, List<ClassInfo> ordered, Set<ClassInfo> visited) {
        if (!visited.add(info)) {
            return;
        }
        ordered.add(info);
        info.children.sort(Comparator.comparing(child -> child.name));
        for (ClassInfo child : info.children) {
            preorder(child, ordered, visited);
        }
    }

    private static boolean sizeFits(ClassInfo info) {
        long length = info.source.getLength();
        return info.size >= 0 ? length == info.size : length >= info.lower && (info.upper < 0 || length <= info.upper);
    }

    private void evaluate(ClassInfo info) {
        if (info.state == State.EDITED) {
            return;
        }
        if (info.source == null) {
            info.outcome = Outcome.MISSING;
            return;
        }
        if (info.sourceConflict != null) {
            reject(info, "2.6 sources disagree (" + info.sourceConflict + ")");
            return;
        }
        if (!sizeFits(info)) {
            info.outcome = Outcome.MISMATCH;
            return;
        }
        int length = info.source.getLength();
        for (ClassInfo ancestor = info.parent; ancestor != null; ancestor = ancestor.parent) {
            if (ancestor.source != null && ancestor.state != State.EDITED && !sizeFits(ancestor)) {
                reject(info, "ancestor `" + ancestor.name + "` does not fit (" + uosaSize(ancestor) + ", 2.6 "
                    + hex(ancestor.source.getLength()) + ")");
                return;
            }
            if (info.hasOwn() && ancestor.source != null && info.firstOwn < ancestor.source.getLength()) {
                reject(info, "2.6 own members start at " + hex(info.firstOwn) + ", inside 2.6 `" + ancestor.name
                    + "` (" + hex(ancestor.source.getLength()) + ")");
                return;
            }
            if (info.hasOwn() && ancestor.size >= 0 && info.firstOwn < ancestor.size) {
                reject(info, "2.6 own members start at " + hex(info.firstOwn) + ", inside `" + ancestor.name
                    + "` (" + hex(ancestor.size) + ")");
                return;
            }
        }
        if (info.size < 0) {
            // Without a sized descendant the bounds are only ">= ancestor"; that proves nothing.
            if (info.upper < 0 && !APPLY_UNBOUNDED) {
                info.outcome = Outcome.UNBOUNDED;
                return;
            }
            // Sized descendants that have a 2.6 type: at least one must fit, else the difference
            // may lie in this class.
            List<ClassInfo> sized = new ArrayList<>();
            sizedDescendants(info, sized);
            if (!sized.isEmpty() && sized.stream().noneMatch(ApplyGamebryoLayouts::sizeFits)) {
                reject(info, "none of the " + sized.size() + " sized descendants with a 2.6 type fits");
                return;
            }
            String conflict = descendantConflict(info, info, length);
            if (conflict != null) {
                reject(info, conflict);
                return;
            }
            if (info.startLength != length) {
                for (DataType parent : info.struct.getParents()) {
                    if (!(parent instanceof Pointer)) {
                        reject(info, "embedded by value in `" + parent.getPathName() + "`, would change size");
                        return;
                    }
                }
            }
        }
        info.outcome = info.size >= 0 ? Outcome.EXACT : Outcome.INFERRED;
    }

    private static void sizedDescendants(ClassInfo info, List<ClassInfo> sized) {
        for (ClassInfo child : info.children) {
            if (child.size >= 0 && child.source != null && child.state != State.EDITED) {
                sized.add(child);
            }
            sizedDescendants(child, sized);
        }
    }

    // Sized descendants whose 2.6 size fits must have their 2.6 own members after the inferred size.
    private static String descendantConflict(ClassInfo ancestor, ClassInfo info, int length) {
        for (ClassInfo child : info.children) {
            if (child.size >= 0 && child.source != null && child.state != State.EDITED && sizeFits(child)
                    && child.hasOwn() && child.firstOwn < length) {
                return "sized descendant `" + child.name + "` has 2.6 own members at " + hex(child.firstOwn)
                    + ", inside 2.6 `" + ancestor.name + "` (" + hex(length) + ")";
            }
            String conflict = descendantConflict(ancestor, child, length);
            if (conflict != null) {
                return conflict;
            }
        }
        return null;
    }

    private static void reject(ClassInfo info, String reason) {
        info.outcome = Outcome.REJECTED;
        info.reason = reason;
    }

    private void build(ClassInfo info) throws Exception {
        if (info.state == State.APPLIED) {
            info.planned = info.struct;
            info.field0 = "already applied";
            return;
        }
        int length = info.source.getLength();
        StructureDataType def = new StructureDataType(info.struct.getCategoryPath(), info.struct.getName(), length, dtm);
        int start;
        ClassInfo base = null;
        for (ClassInfo ancestor = info.parent; ancestor != null; ancestor = ancestor.parent) {
            if (ancestor.hasLayout() && ancestor.planned != null) {
                base = ancestor;
                break;
            }
        }
        if (base != null) {
            DataType baseType = base.planned;
            String comment = base != info.parent ? "Direct parent: " + info.parent.name : null;
            def.replaceAtOffset(0, baseType, baseType.getLength(), BASE, comment);
            start = baseType.getLength();
            info.field0 = "base `" + base.name + "`" + (base != info.parent ? " (parent `" + info.parent.name
                + "` has no layout)" : "");
        } else if (info.parent == null && refObject != null && info.firstOwn >= refObject.getLength()) {
            def.replaceAtOffset(0, new PointerDataType(VoidDataType.dataType, dtm), POINTER_SIZE, VFTABLE,
                REF_OBJECT + " vptr");
            for (DataTypeComponent component : refObject.getDefinedComponents()) {
                if (!isPadding(component)) {
                    place(def, component, REF_OBJECT);
                }
            }
            start = refObject.getLength();
            info.field0 = "vftable, " + REF_OBJECT + " members";
        } else {
            DataTypeComponent field = info.struct.getDefinedComponents()[0];
            def.replaceAtOffset(0, field.getDataType(), field.getLength(), field.getFieldName(), field.getComment());
            start = field.getLength();
            info.field0 = "kept placeholder field 0";
        }
        if (info.hasOwn() && info.firstOwn < start) {
            reject(info, "2.6 own members start at " + hex(info.firstOwn) + ", inside field 0 (" + hex(start) + ")");
            return;
        }
        for (DataTypeComponent component : info.source.getDefinedComponents()) {
            if (!isPadding(component)) {
                place(def, component, null);
            }
        }
        String mismatch = verify(def, info.source, length);
        if (mismatch != null) {
            reject(info, "copied layout differs: " + mismatch);
            return;
        }
        String description = info.original + LAYOUT_MARKER + info.sourceName + " "
            + info.source.getPathName() + (info.outcome != Outcome.INFERRED ? "" : info.upper < 0 ? UNBOUNDED : INFERRED);
        def.setDescription(description);
        if (apply) {
            info.struct.replaceWith(def);
            info.struct.setDescription(description);
            info.planned = info.struct;
            count(changes, "class structures filled");
        } else {
            info.planned = def;
        }
    }

    private void place(Structure def, DataTypeComponent component, String comment) throws Exception {
        String text = comment != null ? comment : component.getComment();
        if (component.isBitFieldComponent()) {
            BitFieldDataType bitField = (BitFieldDataType) component.getDataType();
            def.insertBitFieldAt(component.getOffset(), component.getLength(), bitField.getBitOffset(),
                convertValue(bitField.getBaseDataType()), bitField.getDeclaredBitSize(), component.getFieldName(), text);
            count(notes, "bitfield members");
        } else if (component.getLength() > 0) {
            def.replaceAtOffset(component.getOffset(), convertValue(component.getDataType()), component.getLength(),
                component.getFieldName(), text);
        } else {
            count(notes, "zero-length members skipped");
        }
    }

    // Each non-padding 2.6 member must be at the same offset with the same name and length.
    private static String verify(Structure def, Structure source, int length) {
        if (def.getLength() != length) {
            return "length " + hex(def.getLength());
        }
        Set<String> placed = new HashSet<>();
        for (DataTypeComponent component : def.getDefinedComponents()) {
            placed.add(component.getOffset() + ":" + component.getFieldName() + ":" + component.getLength());
        }
        for (DataTypeComponent component : source.getDefinedComponents()) {
            if (!isPadding(component) && component.getLength() > 0 && !placed.contains(component.getOffset() + ":"
                    + component.getFieldName() + ":" + component.getLength())) {
                return "member at " + hex(component.getOffset());
            }
        }
        return null;
    }

    private static boolean isPadding(DataTypeComponent component) {
        return PADDING.equals(component.getFieldName());
    }

    // Composite types reached by value from the members; types reached only by pointer become stubs.
    private void collectOwnValueTypes(Structure source) {
        for (DataTypeComponent component : source.getDefinedComponents()) {
            if (!isPadding(component)) {
                collectValue(memberType(component));
            }
        }
    }

    private void collectValue(DataType type) {
        if (type instanceof TypeDef typedef) {
            collectValue(typedef.getBaseDataType());
        } else if (type instanceof Array array) {
            collectValue(array.getDataType());
        } else if (type instanceof Composite composite && classValue(type) == null
                && valueTypes.add(type.getPathName())) {
            for (DataTypeComponent component : composite.getDefinedComponents()) {
                if (!isPadding(component)) {
                    collectValue(memberType(component));
                }
            }
        }
    }

    private static DataType memberType(DataTypeComponent component) {
        return component.isBitFieldComponent() ? ((BitFieldDataType) component.getDataType()).getBaseDataType()
            : component.getDataType();
    }

    // The /Gamebryo class of a 2.6 structure, or null.
    private ClassInfo classOf(DataType type) {
        if (!(type instanceof Structure)) {
            return null;
        }
        String[] path = type.getCategoryPath().getPathElements();
        if (path.length == 0 || !path[0].endsWith(PDB_CATEGORY_SUFFIX)) {
            return null;
        }
        List<String> parts = new ArrayList<>(List.of(path).subList(1, path.length));
        parts.add(type.getName());
        return byName.get(String.join(SCOPE, parts));
    }

    // A /Gamebryo class usable by value: its layout is applied and its size equals the 2.6 one.
    private DataType classValue(DataType type) {
        ClassInfo info = classOf(type);
        return info != null && info.hasLayout() && info.startLength == type.getLength() ? info.struct : null;
    }

    private DataType convertValue(DataType type) throws Exception {
        if (type == null) {
            return DataType.DEFAULT;
        }
        if (type instanceof Pointer pointer) {
            DataType target = pointer.getDataType();
            DataType referenced = target != null ? convertReference(target) : null;
            // Default-size pointers stay plain `T *` rather than an explicitly sized `T *32`.
            return pointer.getLength() == dtm.getDataOrganization().getPointerSize()
                ? new PointerDataType(referenced, dtm) : new PointerDataType(referenced, pointer.getLength(), dtm);
        }
        if (type instanceof Array array) {
            return new ArrayDataType(convertValue(array.getDataType()), array.getNumElements(), array.getElementLength(), dtm);
        }
        if (type instanceof BuiltInDataType) {
            return type.clone(dtm);
        }
        DataType classType = classValue(type);
        if (classType != null) {
            return classType;
        }
        String key = type.getPathName();
        DataType done = converted.get(key);
        if (done != null) {
            return done;
        }
        CategoryPath category = copyCategory(type.getCategoryPath());
        DataType existing = dtm.getDataType(category, type.getName());
        if (existing != null) {
            converted.put(key, existing);
            count(notes, "existing types reused");
            return existing;
        }
        String description = "Gamebryo 2.6 " + key;
        if (type instanceof TypeDef typedef) {
            TypedefDataType copy = new TypedefDataType(category, type.getName(), convertValue(typedef.getBaseDataType()), dtm);
            copy.setDescription(description);
            return added(key, copy, TYPEDEFS);
        }
        if (type instanceof ghidra.program.model.data.Enum source) {
            EnumDataType copy = new EnumDataType(category, type.getName(), type.getLength(), dtm);
            for (String name : source.getNames()) {
                copy.add(name, source.getValue(name), source.getComment(name));
            }
            copy.setDescription(description);
            return added(key, copy, ENUMS);
        }
        if (type instanceof Structure source) {
            StructureDataType shell = new StructureDataType(category, type.getName(), type.getLength(), dtm);
            shell.setDescription(description + (hasPadding(source) ? "; inherited regions left undefined" : ""));
            // Register before the members so self-references through pointers resolve to it.
            Structure target = (Structure) added(key, shell, STRUCTURES);
            Structure def = apply ? new StructureDataType(category, type.getName(), type.getLength(), dtm) : target;
            for (DataTypeComponent component : source.getDefinedComponents()) {
                if (!isPadding(component)) {
                    place(def, component, null);
                }
            }
            String mismatch = verify(def, source, type.getLength());
            if (mismatch != null) {
                anomalies.add("Copy of `" + key + "` differs: " + mismatch);
            }
            if (apply) {
                target.replaceWith(def);
                target.setDescription(shell.getDescription());
            }
            return target;
        }
        if (type instanceof Union source) {
            UnionDataType shell = new UnionDataType(category, type.getName(), dtm);
            shell.setDescription(description);
            Union target = (Union) added(key, shell, UNIONS);
            Union def = apply ? new UnionDataType(category, type.getName(), dtm) : target;
            for (DataTypeComponent component : source.getDefinedComponents()) {
                if (component.isBitFieldComponent()) {
                    BitFieldDataType bitField = (BitFieldDataType) component.getDataType();
                    def.addBitField(convertValue(bitField.getBaseDataType()), bitField.getDeclaredBitSize(),
                        component.getFieldName(), component.getComment());
                } else {
                    def.add(convertValue(component.getDataType()), component.getLength(), component.getFieldName(),
                        component.getComment());
                }
            }
            if (apply) {
                target.replaceWith(def);
                target.setDescription(description);
            }
            return target;
        }
        anomalies.add("Unsupported 2.6 type `" + key + "` (" + type.getClass().getSimpleName() + "); left undefined");
        return Undefined.getUndefinedDataType(type.getLength());
    }

    // Pointer targets: /Gamebryo classes, stubs for composites not needed by value, no function types.
    private DataType convertReference(DataType target) throws Exception {
        ClassInfo info = classOf(target);
        if (info != null) {
            return info.struct;
        }
        if (target instanceof FunctionDefinition) {
            count(notes, "function pointers typed void *");
            return VoidDataType.dataType;
        }
        DataType underlying = target;
        while (underlying instanceof TypeDef typedef) {
            underlying = typedef.getBaseDataType();
        }
        if (underlying instanceof Composite && !valueTypes.contains(underlying.getPathName())) {
            ClassInfo underlyingClass = classOf(underlying);
            return underlyingClass != null ? underlyingClass.struct : stub(underlying);
        }
        return convertValue(target);
    }

    private DataType stub(DataType type) {
        String key = type.getPathName();
        DataType done = converted.get(key);
        if (done != null) {
            return done;
        }
        CategoryPath category = copyCategory(type.getCategoryPath());
        DataType existing = dtm.getDataType(category, type.getName());
        if (existing != null) {
            converted.put(key, existing);
            count(notes, "existing types reused");
            return existing;
        }
        DataType stub = type instanceof Union ? new UnionDataType(category, type.getName(), dtm)
            : new StructureDataType(category, type.getName(), 0, dtm);
        stub.setDescription("Gamebryo 2.6 " + key + "; referenced by pointer only, members not copied");
        return added(key, stub, STUBS);
    }

    private DataType added(String key, DataType type, String kind) {
        DataType result = apply ? dtm.addDataType(type, DataTypeConflictHandler.KEEP_HANDLER) : type;
        converted.put(key, result);
        copiedTypes.put(result, kind);
        count(copies, kind);
        return result;
    }

    private static boolean hasPadding(Structure source) {
        for (DataTypeComponent component : source.getDefinedComponents()) {
            if (isPadding(component)) {
                return true;
            }
        }
        return false;
    }

    // /<Lib>.pdb/A/B -> /Gamebryo26/A/B; other categories keep their path below /Gamebryo26.
    private static CategoryPath copyCategory(CategoryPath category) {
        String[] path = category.getPathElements();
        int first = path.length > 0 && path[0].endsWith(PDB_CATEGORY_SUFFIX) ? 1 : 0;
        CategoryPath result = new CategoryPath(COPY_CATEGORY);
        for (int i = first; i < path.length; i++) {
            result = new CategoryPath(result, path[i]);
        }
        return result;
    }

    private void report(PrintWriter output, List<Source> sources) {
        List<ClassInfo> ordered = preorder();
        Map<Outcome, List<ClassInfo>> byOutcome = new LinkedHashMap<>();
        for (Outcome outcome : Outcome.values()) {
            byOutcome.put(outcome, new ArrayList<>());
        }
        List<ClassInfo> editedList = new ArrayList<>();
        int placeholders = 0;
        int alreadyApplied = 0;
        for (ClassInfo info : ordered) {
            if (info.state == State.EDITED) {
                editedList.add(info);
                continue;
            }
            placeholders += info.state == State.PLACEHOLDER ? 1 : 0;
            alreadyApplied += info.state == State.APPLIED ? 1 : 0;
            byOutcome.get(info.outcome).add(info);
        }
        output.println("# Gamebryo 2.6 layouts");
        output.println();
        output.println("Program: " + currentProgram.getName());
        output.println("SHA-256: " + currentProgram.getExecutableSHA256());
        output.println("Mode: " + (apply ? APPLY_FLAG : "report only"));
        output.println("Sources (" + SOURCE_FOLDER + "): " + String.join(", ", sources.stream().map(source -> "`"
            + source.name + "` (" + source.program.getDataTypeManager().getDataTypeCount(false) + " data types)").toList()));
        output.println();
        output.println("| Item | Count |");
        output.println("|---|---:|");
        output.println("| Class structures in `" + CATEGORY + "` | " + ordered.size() + " |");
        output.println("| Unfilled placeholders | " + placeholders + " |");
        output.println("| Filled by an earlier run | " + alreadyApplied + " |");
        output.println("| Edited by the user (kept) | " + editedList.size() + " |");
        output.println("| Other types in `" + CATEGORY + "` | " + others.size() + " |");
        output.println("| No 2.6 structure in the sources | " + byOutcome.get(Outcome.MISSING).size() + " |");
        output.println("| Layout applied, exact size | " + byOutcome.get(Outcome.EXACT).size() + " |");
        output.println("| Layout applied, inferred size | " + byOutcome.get(Outcome.INFERRED).size() + " |");
        output.println("| Size mismatches | " + byOutcome.get(Outcome.MISMATCH).size() + " |");
        output.println("| No size evidence (no sized descendant) | " + byOutcome.get(Outcome.UNBOUNDED).size() + " |");
        output.println("| Rejected by consistency checks | " + byOutcome.get(Outcome.REJECTED).size() + " |");
        for (var entry : copies.entrySet()) {
            output.println("| 2.6 types " + (apply ? "copied" : "to copy") + ": " + entry.getKey() + " | " + entry.getValue() + " |");
        }
        for (var entry : notes.entrySet()) {
            output.println("| 2.6 members: " + entry.getKey() + " | " + entry.getValue() + " |");
        }
        String status = apply ? "applied" : "would apply";
        output.println();
        output.println("## Exact size");
        output.println();
        output.println("UOSA size from `CreateObject` equals the 2.6 size.");
        output.println();
        output.println("| Class | Size | 2.6 source | Field 0 |");
        output.println("|---|---:|---|---|");
        for (ClassInfo info : byOutcome.get(Outcome.EXACT)) {
            output.println("| `" + info.name + "` | " + hex(info.size) + " | `" + info.sourceName + "` | "
                + field0(info, status) + " |");
        }
        output.println();
        output.println("## Inferred size");
        output.println();
        output.println("No `CreateObject`; the 2.6 size lies within the recorded bounds (the upper bound is the smallest"
            + " sized descendant), at least one sized descendant with a 2.6 type fits, and none of the fitting ones"
            + " has 2.6 members inside this size.");
        output.println();
        output.println("| Class | UOSA bounds | 2.6 size | 2.6 source | Field 0 |");
        output.println("|---|---|---:|---|---|");
        for (ClassInfo info : byOutcome.get(Outcome.INFERRED)) {
            output.println("| `" + info.name + "` | " + uosaSize(info) + " | " + hex(info.source.getLength()) + " | `"
                + info.sourceName + "` | " + field0(info, status) + " |");
        }
        output.println();
        output.println("## Size mismatches");
        output.println();
        output.println("| Class | UOSA size or bounds | 2.6 size |");
        output.println("|---|---|---:|");
        for (ClassInfo info : byOutcome.get(Outcome.MISMATCH)) {
            output.println("| `" + info.name + "` | " + uosaSize(info) + " | " + hex(info.source.getLength()) + " |");
        }
        output.println();
        output.println("## No size evidence");
        output.println();
        output.println("No `CreateObject` and no sized descendant, so the bounds are only a lower bound; not applied.");
        output.println();
        output.println("| Class | UOSA bounds | 2.6 size |");
        output.println("|---|---|---:|");
        for (ClassInfo info : byOutcome.get(Outcome.UNBOUNDED)) {
            output.println("| `" + info.name + "` | " + uosaSize(info) + " | " + hex(info.source.getLength()) + " |");
        }
        output.println();
        output.println("## Rejected by consistency checks");
        output.println();
        output.println("| Class | UOSA size or bounds | 2.6 size | Reason |");
        output.println("|---|---|---:|---|");
        for (ClassInfo info : byOutcome.get(Outcome.REJECTED)) {
            output.println("| `" + info.name + "` | " + uosaSize(info) + " | " + hex(info.source.getLength()) + " | "
                + info.reason + " |");
        }
        output.println();
        output.println("## No 2.6 structure in the sources");
        output.println();
        output.println(names(byOutcome.get(Outcome.MISSING)));
        output.println();
        output.println("## Edited by the user (kept)");
        output.println();
        if (editedList.isEmpty()) {
            output.println("None.");
        }
        for (ClassInfo info : editedList) {
            output.println("- `" + info.name + "`: " + info.editReason);
        }
        output.println();
        output.println("## Other types in " + CATEGORY);
        output.println();
        output.println(others.isEmpty() ? "None." : String.join(", ", others.stream().map(name -> "`" + name + "`").toList()));
        output.println();
        output.println("## Anomalies");
        output.println();
        if (anomalies.isEmpty()) {
            output.println("None.");
        }
        for (String anomaly : anomalies) {
            output.println("- " + anomaly);
        }
        if (apply) {
            int total = 0;
            for (int value : copies.values()) {
                total += value;
            }
            output.println();
            output.println("## Applied changes");
            output.println();
            output.println("- class structures filled: " + changes.getOrDefault("class structures filled", 0));
            output.println("- 2.6 types copied to `" + COPY_CATEGORY + "`: " + total);
        }
    }

    // Member layouts of the filled classes and the types copied by this run.
    private void members(PrintWriter output) {
        output.println("# Gamebryo 2.6 layouts: members");
        output.println();
        output.println("Program: " + currentProgram.getName());
        output.println("Mode: " + (apply ? APPLY_FLAG : "report only (planned layouts)"));
        output.println();
        output.println("## Classes");
        for (ClassInfo info : preorder()) {
            if (info.hasLayout() && info.planned instanceof Structure struct) {
                output.println();
                output.println("### `" + info.name + "` (" + hex(struct.getLength()) + ", "
                    + (info.outcome == Outcome.INFERRED ? "inferred" : "exact") + ")");
                output.println();
                layout(output, struct);
            }
        }
        output.println();
        output.println("## Types copied to `" + COPY_CATEGORY + "`");
        if (copiedTypes.isEmpty()) {
            output.println();
            output.println("None in this run.");
        }
        for (var entry : copiedTypes.entrySet()) {
            DataType type = entry.getKey();
            output.println();
            output.println("### `" + type.getPathName() + "` (" + (type.isNotYetDefined() ? "no size" : hex(type.getLength()))
                + ", " + entry.getValue() + ")");
            if (type instanceof Composite composite && composite.getNumDefinedComponents() > 0) {
                output.println();
                layout(output, composite);
            }
        }
    }

    private static void layout(PrintWriter output, Composite composite) {
        output.println("| Offset | Type | Name | Comment |");
        output.println("|---:|---|---|---|");
        for (DataTypeComponent component : composite.getDefinedComponents()) {
            output.println("| " + hex(component.getOffset()) + " | `" + component.getDataType().getDisplayName() + "` | "
                + (component.getFieldName() != null ? "`" + component.getFieldName() + "`" : "") + " | "
                + (component.getComment() != null ? component.getComment() : "") + " |");
        }
    }

    private static String field0(ClassInfo info, String status) {
        return info.state == State.APPLIED ? "filled earlier" : status + "; " + info.field0;
    }

    private static String names(List<ClassInfo> infos) {
        return infos.isEmpty() ? "None." : String.join(", ", infos.stream().map(info -> "`" + info.name + "`").toList());
    }

    private static String uosaSize(ClassInfo info) {
        if (info.size >= 0) {
            return hex(info.size);
        }
        return ">= " + hex(info.lower) + ", " + (info.upper >= 0 ? "<= " + hex(info.upper) : "no upper bound");
    }

    private static void count(Map<String, Integer> counts, String key) {
        counts.merge(key, 1, Integer::sum);
    }

    private static String hex(long value) {
        return String.format("0x%x", value);
    }
}
