// Cross-reference the Enhanced Client UI (ui_xref.tsv, written by extract_ui_xref.py) with UOSA.exe:
// UO Lua API call sites in the UI scripts (Lua -> C), engine strings naming SystemData/WindowData
// paths, event handlers, UI Lua globals and windows (C -> Lua), and the window XML schema
// (element dispatch -> parser functions), plus a verified UI texture loading chain. With --apply,
// append a UI call summary to the plate comments NameLuaFunctions wrote, tag UI_HOOK / UI_XML
// functions and name DEFAULT-named XML element parsers UOXml::Parse<Element>.
// @category ECResearch
import ghidra.app.script.GhidraScript;
import ghidra.program.model.address.Address;
import ghidra.program.model.listing.Function;
import ghidra.program.model.listing.FunctionTag;
import ghidra.program.model.listing.FunctionTagManager;
import ghidra.program.model.listing.Instruction;
import ghidra.program.model.mem.MemoryBlock;
import ghidra.program.model.symbol.FlowType;
import ghidra.program.model.symbol.Namespace;
import ghidra.program.model.symbol.SourceType;
import ghidra.program.model.symbol.Symbol;
import ghidra.program.model.symbol.SymbolTable;
import ghidra.util.exception.DuplicateNameException;
import java.io.File;
import java.io.PrintWriter;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardOpenOption;
import java.util.ArrayList;
import java.util.Collection;
import java.util.HashMap;
import java.util.HashSet;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.TreeMap;
import java.util.TreeSet;
import java.util.function.Predicate;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

public class CrossReferenceUi extends GhidraScript {
    private static final String APPLY_FLAG = "--apply";
    // Adds the per-call-site index and the per-string lists (paths, engine strings, globals, dispatch sites).
    private static final String DETAILS_FLAG = "--details";
    private static final String XREF_OPTION = "--xref";
    private static final String DEFAULT_XREF = "ui_xref.tsv";
    private static final String ENCODED_SPACE = "%20";
    private static final String TAG_API = "LUA_API";
    private static final String TAG_HOOK = "UI_HOOK";
    private static final String TAG_XML = "UI_XML";
    private static final String TAG_HOOK_COMMENT = "CrossReferenceUi: references UI Lua names "
        + "(SystemData/WindowData paths, event handlers, UI globals, windows or WindowData fields)";
    private static final String TAG_XML_COMMENT = "CrossReferenceUi: parses UI window XML elements";
    private static final String[] FOREIGN_TAG_PREFIXES = {"GB26_", "ZLIB_", "CRYPTOPP_", "TINYXML_", "FREETYPE_"};
    private static final String API_NAMESPACE = "UOLua";
    private static final String XML_NAMESPACE = "UOXml";
    private static final String XML_PREFIX = "Parse";
    private static final String SCOPE = "::";
    private static final String UI_LINE_PREFIX = "UI: ";
    private static final String LINE_BREAK = "\n";
    private static final String FIELD_SEPARATOR = "\t";
    private static final String TSV_COMMENT = "#";
    private static final String RESULT_ALL = "*";
    private static final String VARIADIC_MARK = "+";
    private static final String INDEX_MARK = "[]";
    private static final String SYSTEM_DATA = "SystemData.";
    private static final String WINDOW_DATA = "WindowData.";
    private static final String VFTABLE = "vftable";
    // The engine registers some API functions as `_Name`; Lua code outside the Default UI wraps them.
    private static final String WRAPPED_PREFIX = "_";
    // The plate comment NameLuaFunctions writes for a LUA_API function (a single line).
    private static final Pattern NAME_LUA_LINE = Pattern.compile("NameLuaFunctions: LUA_API `([^`]+)`: UO Lua API "
        + "`([^`]+)` registered at 0x[0-9a-f]{8} in (?:`[^`]*`|code outside functions) via [^;\\n]*"
        + "(?:; also registered as ([^;\\n]+))?(?:; [0-9]+ registrations)?");
    private static final String ALSO_SEPARATOR = ", ";

    private static final int POINTER_SIZE = 4;
    private static final int BYTE_MASK = 0xff;
    private static final int BITS_PER_BYTE = 8;
    private static final int OPCODE_PUSH_IMM = 0x68;
    private static final int OPCODE_LENGTH = 1;
    private static final int MAX_STRING_LENGTH = 512;
    private static final int PRINTABLE_FIRST = 0x20;
    private static final int PRINTABLE_LAST = 0x7e;
    // Instructions searched after a PUSH of a string for the call consuming it.
    private static final int MAX_CALL_LOOKAHEAD = 6;
    // Instructions searched after a tag compare call for the conditional branch on its result.
    private static final int MAX_JCC_LOOKAHEAD = 6;
    // Instructions scanned in the matching branch of a tag compare for the element handler call.
    private static final int MAX_BRANCH_INSTRUCTIONS = 32;
    // Minimum distinct attribute/child names of an element a parser must reference.
    private static final int MIN_XML_EVIDENCE = 2;
    // Element dispatches a function needs to be tagged as an XML dispatcher.
    private static final int MIN_DISPATCHES = 2;
    // Minimum distinct UI names (paths, events, globals, windows) for a UI_HOOK tag.
    private static final int MIN_HOOK_NAMES = 2;
    // UI Lua globals and window names shorter than this are too generic to match.
    private static final int MIN_HOOK_NAME_LENGTH = 5;
    // Field mapping: a C function referencing at least this many fields a UI table is read with.
    private static final int MIN_FIELD_MATCHES = 3;
    private static final double MIN_FIELD_SHARE = 0.25;
    private static final int MIN_FIELD_LENGTH = 3;
    private static final int PATH_TABLE_DEPTH = 2;
    private static final int MAX_CALLEES_REPORTED = 8;
    private static final int MAX_NAMES_SHOWN = 12;
    private static final int HEX_RADIX = 16;

    private static final String KIND_PATH = "path";
    private static final String KIND_EVENT = "event";
    private static final String KIND_GLOBAL = "global";
    private static final String KIND_WINDOW = "window";

    private static final class Block {
        long start;
        byte[] bytes;
        boolean exec;
    }

    private static final class Call {
        String name;
        String file;
        int line;
        int args;
        boolean variadic;
        String results;
        String enclosing;
    }

    private static final class PathUse {
        int reads;
        int writes;
        String file;
        int line;
    }

    private static final class Handler {
        String event;
        String function;
        String file;
        int line;
    }

    private static final class Dispatch {
        String element;
        long site;
        long dispatcher;
        long compare;
        long handler;
        Set<String> evidence;
    }

    private static final class Check {
        final String kind;
        final String value;

        Check(String kind, String value) {
            this.kind = kind;
            this.value = value;
        }
    }

    private static final class Step {
        final long address;
        final String role;
        final Check[] checks;

        Step(long address, String role, Check... checks) {
            this.address = address;
            this.role = role;
            this.checks = checks;
        }
    }

    private static final String CHECK_CALLS = "calls";
    private static final String CHECK_STRING = "string";
    private static final String CHECK_VFTABLE = "vftable";
    private static final String CHECK_SLOT = "slot";
    private static final String CHECK_NAME = "name";

    // The UI texture chain established by hand (see README); each step is re-verified on every run.
    private static final Step[] TEXTURE_CHAIN = {
        new Step(0x0096aebbL, "UI XML document root: `<Assets>`, `<Include>`, `<Scripts>`, `<Windows>`",
            new Check(CHECK_CALLS, "0x00956123")),
        new Step(0x00956123L, "`<Assets>` children: `<Texture>`, `<Font>`, `<TextureFont>`, `<Icon>`, `<MapIcon>`",
            new Check(CHECK_CALLS, "0x009573da"), new Check(CHECK_STRING, "TextureFont")),
        new Step(0x009573daL, "`<Texture name file>` and `<Slice>` children -> `MythUi::TextureDef`",
            new Check(CHECK_CALLS, "0x00949757"), new Check(CHECK_CALLS, "0x00957d2a")),
        new Step(0x00949757L, "`MythUi::TextureDef` constructor", new Check(CHECK_VFTABLE, "MythUi::TextureDef")),
        new Step(0x0090169cL, "UI asset loader: for each loaded interface, TextureDefs, fonts, icons "
            + "(`icon%06d`) and map icons -> textures in the UI texture registry",
            new Check(CHECK_STRING, "File not found: unable to load texture asset '%s'."),
            new Check(CHECK_CALLS, "0x009034e0"), new Check(CHECK_CALLS, "0x00911470"),
            new Check(CHECK_CALLS, "0x00989fdb")),
        new Step(0x009034e0L, "asset path: custom UI folder first (exists check mode 3), else the default folder",
            new Check(CHECK_CALLS, "0x00989fdb")),
        new Step(0x00989fdbL, "file exists: mode 1 disk, 2 mounted packages (default), 3 both",
            new Check(CHECK_CALLS, "0x0098afd4"), new Check(CHECK_CALLS, "0x0098d460")),
        new Step(0x00911470L, "`MythUi::Texture` from a file name: `NiSourceTexture::Create`, then wrap",
            new Check(CHECK_CALLS, "0x0068a8b0"), new Check(CHECK_CALLS, "0x00911701")),
        new Step(0x0068a8b0L, "`NiSourceTexture::Create(filename)`: pixel data is read later by the renderer "
            + "(`NiDX9SourceTextureData::LoadTextureFile`) through `NiFile` (the UOP-backed `UONdlFile`)",
            new Check(CHECK_VFTABLE, "NiSourceTexture")),
        new Step(0x00771af0L, "`NiDX9SourceTextureData::LoadTextureFile`", new Check(CHECK_NAME, "LoadTextureFile")),
        new Step(0x0058a850L, "`UONdlFile::FileCreateFunc` (NiFile factory)", new Check(CHECK_NAME, "FileCreateFunc")),
        new Step(0x0095fe77L, "`<DynamicImage texture=...>`: name looked up in the UI texture registry",
            new Check(CHECK_STRING, "Error Loading XML Definition %s: Ui Texture \"%s\" not found"),
            new Check(CHECK_CALLS, "0x00901100")),
        new Step(0x00901100L, "UI texture registry: exists(name)", new Check(CHECK_CALLS, "0x00901367")),
        new Step(0x008f4dc3L, "`DynamicImageSetTexture(window, name, x, y)`", new Check(CHECK_CALLS, "0x008f42da")),
        new Step(0x00519850L, "`ManagedTextures.xml`: `<Textures><Texture id file>` -> id -> file map "
            + "(`typeid` is never read); also creates `target_portrait`",
            new Check(CHECK_STRING, "Data/Interface/InterfaceCore/Textures/ManagedTextures.xml"),
            new Check(CHECK_CALLS, "0x0051db40"), new Check(CHECK_CALLS, "0x00900deb")),
        new Step(0x0051c750L, "`RequestTexture(id, w, h)`", new Check(CHECK_CALLS, "0x0051a950")),
        new Step(0x0051a950L, "managed texture request: id cache, file location, TextureManager, registered "
            + "as `%08d_Texture`", new Check(CHECK_STRING, "%08d_Texture"), new Check(CHECK_CALLS, "0x0051dad0"),
            new Check(CHECK_CALLS, "0x00519570"), new Check(CHECK_CALLS, "0x00900c72")),
        new Step(0x00519570L, "`ManagedTextureFileLocation(\"Data/Interface/InterfaceCore/Textures/\" + file)`",
            new Check(CHECK_VFTABLE, "ManagedTextureFileLocation"),
            new Check(CHECK_STRING, "Data/Interface/InterfaceCore/Textures/")),
        new Step(0x0058d8e0L, "`TextureManager` vftable slot 1: request by file location (sync or async)",
            new Check(CHECK_SLOT, "TextureManager:1"), new Check(CHECK_CALLS, "0x0058d600"),
            new Check(CHECK_CALLS, "0x0058d430")),
        new Step(0x0058d600L, "synchronous load: lower-cased path, FNV-1a keyed cache, `UOTextureData`",
            new Check(CHECK_CALLS, "0x00a70565"), new Check(CHECK_CALLS, "0x00a70665")),
        new Step(0x004ccc80L, "`UOTextureData(path)`: exists check, `NiSourceTexture::Create`",
            new Check(CHECK_VFTABLE, "UOTextureData"), new Check(CHECK_CALLS, "0x00989fdb"),
            new Check(CHECK_CALLS, "0x0068a8b0")),
        new Step(0x0051b650L, "`RequestTileArt(objType, w, h)`", new Check(CHECK_CALLS, "0x00519c50")),
        new Step(0x00519c50L, "tile art request: sprite file location, registered in the UI texture registry",
            new Check(CHECK_CALLS, "0x0058d1f0"), new Check(CHECK_CALLS, "0x00900c72")),
        new Step(0x0058d1f0L, "`SpriteTextureFileLocation`: `Textures\\`, `TileArtLegacy\\`, `TileArtEnhanced\\`",
            new Check(CHECK_VFTABLE, "SpriteTextureFileLocation"), new Check(CHECK_STRING, "TileArtLegacy\\")),
        new Step(0x0051cfd0L, "`RequestGumpArt(id)`", new Check(CHECK_CALLS, "0x0051af10")),
        new Step(0x0051af10L, "gump art request, registered in the UI texture registry",
            new Check(CHECK_CALLS, "0x00519700"), new Check(CHECK_CALLS, "0x00900c72")),
        new Step(0x00519700L, "gump file location under `Data/Interface/Default/Textures/GumpArt/`",
            new Check(CHECK_STRING, "Data/Interface/Default/Textures/GumpArt/")),
    };

    private static final String[] CAVEATS = {
        "String matching is exact: names built at run time, wide strings and substrings (`_WindowGetShowing` for "
            + "`WindowGetShowing`) are not found; strings reached through data tables have no code reference.",
        "The UI index covers `UserInterface/Default` only; the `InterfaceCore` Lua inside the packages (which "
            + "wraps the `_Name` registrations) is not included.",
        "Result counts come from a tokenizer, not a Lua parser: `0` is a statement call, `1` an expression; "
            + "calls through UI-defined names are left out.",
        "Field-name matches (C -> Lua tables) are by string only: a function referencing the field names of a UI "
            + "table is a candidate writer, not proof.",
        "Elements the Default UI never uses have no attribute list, so their handlers stay unnamed; elements "
            + "sharing a handler are reported, not named.",
        "Texture chain confidence: every listed step is checked statically; the step from `NiSourceTexture` to "
            + "`NiDX9SourceTextureData::LoadTextureFile` and `UONdlFile::FileCreateFunc` goes through renderer "
            + "virtual calls and is inferred (medium), the rest are direct calls, strings or vftables (high).",
    };

    private final List<Block> blocks = new ArrayList<>();
    private Block lastBlock;
    private final Map<String, Integer> changes = new LinkedHashMap<>();
    private final List<String> skipped = new ArrayList<>();
    private final List<String> kept = new ArrayList<>();
    private final List<String> anomalies = new ArrayList<>();

    // ui_xref.tsv
    private final Map<String, List<Call>> calls = new TreeMap<>();
    private final Map<String, String[]> definitions = new TreeMap<>();
    private final Map<String, PathUse> paths = new TreeMap<>();
    private final Map<String, int[]> elements = new TreeMap<>();
    private final Map<String, String> elementFirst = new HashMap<>();
    private final Map<String, Map<String, Integer>> childrenOf = new TreeMap<>();
    private final Map<String, Map<String, Integer>> attributesOf = new TreeMap<>();
    private final Map<String, Map<String, Integer>> eventValues = new TreeMap<>();
    private final Map<String, String[]> windows = new TreeMap<>();
    private final List<Handler> handlers = new ArrayList<>();

    // Strings and references.
    private final Map<String, List<Long>> stringAddresses = new HashMap<>();
    private final Map<Long, String> stringText = new HashMap<>();
    private final Map<Long, List<Long>> referenceSites = new HashMap<>();
    private final Map<Long, Function> functionCache = new HashMap<>();
    private boolean details;

    public void run() throws Exception {
        String[] args = getScriptArgs();
        if (args.length < 1) {
            throw new IllegalArgumentException("Expected OUTPUT_FILE [" + APPLY_FLAG + "] ["
                + DETAILS_FLAG + "] [" + XREF_OPTION + " TSV]");
        }
        boolean apply = false;
        String xrefPath = null;
        for (int i = 1; i < args.length; i++) {
            switch (args[i]) {
                case APPLY_FLAG -> apply = true;
                case DETAILS_FLAG -> details = true;
                case XREF_OPTION -> xrefPath = decodePath(args[++i]);
                default -> throw new IllegalArgumentException("Unknown argument " + args[i]);
            }
        }
        if (xrefPath == null) {
            xrefPath = new File(getSourceFile().getParentFile().getAbsolutePath(), DEFAULT_XREF).getPath();
        }
        if (!new File(xrefPath).isFile()) {
            throw new IllegalArgumentException("UI index " + xrefPath + " not found; run extract_ui_xref.py or pass "
                + XREF_OPTION);
        }
        // Created before any change, so an existing output file stops the run unchanged.
        try (PrintWriter output = new PrintWriter(Files.newBufferedWriter(Path.of(decodePath(args[0])),
                StandardOpenOption.CREATE_NEW))) {
            analyze(output, apply, xrefPath);
        }
        println("Wrote " + args[0] + (apply ? " (applied: " + changes + ")" : ""));
    }

    private void analyze(PrintWriter out, boolean apply, String xrefPath) throws Exception {
        loadXref(xrefPath);
        loadBlocks();

        // Names searched in the binary.
        Set<String> xmlNames = new HashSet<>(elements.keySet());
        for (Map<String, Integer> attributes : attributesOf.values()) {
            xmlNames.addAll(attributes.keySet());
        }
        Map<String, Set<String>> tableFields = tableFields();
        Set<String> fieldNames = new HashSet<>();
        tableFields.values().forEach(fieldNames::addAll);
        Map<String, String> hookKinds = hookNames(xmlNames);
        Set<String> chainStrings = new HashSet<>();
        for (Step step : TEXTURE_CHAIN) {
            for (Check check : step.checks) {
                if (check.kind.equals(CHECK_STRING)) {
                    chainStrings.add(check.value);
                }
            }
        }
        Set<String> wanted = new HashSet<>(xmlNames);
        wanted.addAll(hookKinds.keySet());
        wanted.addAll(fieldNames);
        wanted.addAll(chainStrings);
        wanted.addAll(calls.keySet());
        scanStrings(text -> wanted.contains(text) || text.startsWith(SYSTEM_DATA) || text.startsWith(WINDOW_DATA));
        Set<Long> targets = new HashSet<>(stringText.keySet());
        Map<String, Long> vftables = vftables();
        targets.addAll(vftables.values());
        scanReferences(targets);

        // 1. Lua -> C: UO Lua API call sites.
        List<String> apiLines = new ArrayList<>();
        List<String> apiIndex = new ArrayList<>();
        Map<Function, String> uiLines = new TreeMap<>((a, b) -> a.getEntryPoint().compareTo(b.getEntryPoint()));
        Set<String> apiNames = new HashSet<>();
        int[] apiCounts = apiCallSites(apiLines, apiIndex, uiLines, apiNames);
        List<String> notApi = new ArrayList<>();
        for (var entry : calls.entrySet()) {
            if (apiNames.contains(entry.getKey())) {
                continue;
            }
            List<Long> where = stringAddresses.getOrDefault(entry.getKey(), List.of());
            notApi.add("`" + entry.getKey() + "` " + entry.getValue().size() + " calls, e.g. "
                + entry.getValue().get(0).file + ":" + entry.getValue().get(0).line
                + (where.isEmpty() ? "; no such string in UOSA" : "; string at " + hexList(where) + " "
                + referencers(where))
                + (apiNames.contains(WRAPPED_PREFIX + entry.getKey()) ? "; `" + WRAPPED_PREFIX + entry.getKey()
                + "` is registered (wrapped outside the Default UI)" : ""));
        }

        // 2. C -> Lua.
        Map<Function, Set<String>> hookFunctions = new TreeMap<>((a, b) -> a.getEntryPoint().compareTo(b.getEntryPoint()));
        Map<String, Map<Long, Integer>> callees = new TreeMap<>();
        List<String> pathLines = new ArrayList<>();
        List<String> engineOnly = new ArrayList<>();
        List<String> eventLines = new ArrayList<>();
        List<String> globalLines = new ArrayList<>();
        hooks(hookKinds, hookFunctions, callees, pathLines, engineOnly, eventLines, globalLines);
        List<String> fieldLines = new ArrayList<>();
        fieldMapping(tableFields, fieldLines, hookFunctions);
        // A hook needs >= MIN_HOOK_NAMES names, one of them specific to the UI (a path, an event, a window or a
        // matched field table): plain Lua global names alone (`Scale`, `Opacity`) are too generic.
        hookFunctions.values().removeIf(names -> names.size() < MIN_HOOK_NAMES
            || names.stream().allMatch(name -> KIND_GLOBAL.equals(hookKinds.get(name))));
        List<String> uiOnlyTables = new ArrayList<>();
        for (var entry : paths.entrySet()) {
            String path = entry.getKey();
            if (path.split("\\.").length == PATH_TABLE_DEPTH && !path.contains(INDEX_MARK)
                    && !stringAddresses.containsKey(path)) {
                uiOnlyTables.add("`" + path + "`");
            }
        }

        // 3. XML schema -> parsers.
        Map<String, Set<Long>> tagAddresses = new TreeMap<>();
        for (String element : elements.keySet()) {
            tagAddresses.put(element, new TreeSet<>(stringAddresses.getOrDefault(element, List.of())));
        }
        Map<Function, Set<String>> xmlRefs = namesReferenced(xmlNames);
        List<Dispatch> dispatches = new ArrayList<>();
        Map<Long, Integer> compareCallees = new HashMap<>();
        for (var entry : tagAddresses.entrySet()) {
            for (long address : entry.getValue()) {
                for (long site : referenceSites.getOrDefault(address, List.of())) {
                    Dispatch dispatch = decodeDispatch(entry.getKey(), site, xmlRefs);
                    if (dispatch != null) {
                        dispatches.add(dispatch);
                        compareCallees.merge(dispatch.compare, 1, Integer::sum);
                    }
                }
            }
        }
        Map<Long, Set<String>> handlerElements = new TreeMap<>();
        Map<String, Set<Long>> elementHandlers = new TreeMap<>();
        Map<Long, Set<String>> dispatcherElements = new TreeMap<>();
        for (Dispatch dispatch : dispatches) {
            handlerElements.computeIfAbsent(dispatch.handler, key -> new TreeSet<>()).add(dispatch.element);
            elementHandlers.computeIfAbsent(dispatch.element, key -> new TreeSet<>()).add(dispatch.handler);
            dispatcherElements.computeIfAbsent(dispatch.dispatcher, key -> new TreeSet<>()).add(dispatch.element);
        }
        Map<Long, String> xmlNamesProposed = new TreeMap<>();
        List<String> sharedHandlers = new ArrayList<>();
        List<String> ambiguousElements = new ArrayList<>();
        for (var entry : handlerElements.entrySet()) {
            if (entry.getValue().size() == 1) {
                String element = entry.getValue().iterator().next();
                if (elementHandlers.get(element).size() == 1) {
                    xmlNamesProposed.put(entry.getKey(), XML_NAMESPACE + SCOPE + XML_PREFIX + sanitize(element));
                }
            } else {
                sharedHandlers.add(hex(entry.getKey()) + " " + label(entry.getKey()) + " handles "
                    + quoteAll(entry.getValue()));
            }
        }
        for (var entry : elementHandlers.entrySet()) {
            if (entry.getValue().size() > 1) {
                List<String> parts = new ArrayList<>();
                for (long handler : entry.getValue()) {
                    parts.add(hex(handler) + " " + label(handler));
                }
                ambiguousElements.add("`<" + entry.getKey() + ">`: " + String.join(", ", parts));
            }
        }
        List<String> directLines = new ArrayList<>();
        Map<Long, String> directNames = directParsers(xmlRefs, handlerElements.keySet(), elementHandlers, directLines);
        xmlNamesProposed.putAll(directNames);

        // Apply.
        if (apply) {
            applyComments(uiLines);
            for (var entry : hookFunctions.entrySet()) {
                if (entry.getValue().size() >= MIN_HOOK_NAMES) {
                    tag(entry.getKey(), TAG_HOOK, TAG_HOOK_COMMENT);
                }
            }
            Set<Long> xmlFunctions = new TreeSet<>(handlerElements.keySet());
            xmlFunctions.addAll(directNames.keySet());
            for (var entry : dispatcherElements.entrySet()) {
                if (entry.getValue().size() >= MIN_DISPATCHES) {
                    xmlFunctions.add(entry.getKey());
                }
            }
            for (long address : xmlFunctions) {
                Function function = functionAt(address);
                if (function != null) {
                    tag(function, TAG_XML, TAG_XML_COMMENT);
                }
            }
            for (var entry : xmlNamesProposed.entrySet()) {
                rename(entry.getKey(), entry.getValue());
            }
        }

        report(out, apply, xrefPath, apiCounts, apiLines, apiIndex, notApi, hookFunctions, callees, pathLines,
            engineOnly, eventLines, globalLines, fieldLines, uiOnlyTables, tagAddresses, dispatches,
            handlerElements, elementHandlers, dispatcherElements, compareCallees, xmlNamesProposed, sharedHandlers,
            ambiguousElements, directLines, xmlRefs, vftables);
    }

    // ---- ui_xref.tsv ----

    private void loadXref(String path) throws Exception {
        for (String line : Files.readAllLines(Path.of(path), StandardCharsets.UTF_8)) {
            if (line.isEmpty() || line.startsWith(TSV_COMMENT)) {
                continue;
            }
            String[] f = line.split(FIELD_SEPARATOR, -1);
            for (int i = 0; i < f.length; i++) {
                f[i] = unescape(f[i]);
            }
            switch (f[0]) {
                case "CALL" -> {
                    Call call = new Call();
                    call.name = f[1];
                    call.file = f[2];
                    call.line = Integer.parseInt(f[3]);
                    call.variadic = f[4].endsWith(VARIADIC_MARK);
                    call.args = Integer.parseInt(call.variadic ? f[4].substring(0, f[4].length() - 1) : f[4]);
                    call.results = f[5];
                    call.enclosing = f[6];
                    calls.computeIfAbsent(call.name, key -> new ArrayList<>()).add(call);
                }
                case "HANDLER" -> {
                    Handler handler = new Handler();
                    handler.event = f[2];
                    handler.function = f[3];
                    handler.file = f[4];
                    handler.line = Integer.parseInt(f[5]);
                    handlers.add(handler);
                }
                case "LUADEF" -> definitions.put(f[1], new String[] {f[2], f[3], f[4]});
                case "PATH" -> {
                    PathUse use = new PathUse();
                    use.reads = Integer.parseInt(f[2]);
                    use.writes = Integer.parseInt(f[3]);
                    use.file = f[4];
                    use.line = Integer.parseInt(f[5]);
                    paths.put(f[1], use);
                }
                case "XMLEL" -> {
                    elements.put(f[1], new int[] {Integer.parseInt(f[2])});
                    elementFirst.put(f[1], f[3] + ":" + f[4]);
                }
                case "XMLCHILD" -> childrenOf.computeIfAbsent(f[1], key -> new TreeMap<>()).put(f[2], Integer.parseInt(f[3]));
                case "XMLATTR" -> attributesOf.computeIfAbsent(f[1], key -> new TreeMap<>()).put(f[2], Integer.parseInt(f[3]));
                case "XMLVALUE" -> eventValues.computeIfAbsent(f[1] + "." + f[2], key -> new TreeMap<>())
                    .put(f[3], Integer.parseInt(f[4]));
                case "XMLWINDOW" -> windows.put(f[1], new String[] {f[2], f[3], f[4]});
                default -> anomalies.add("Unknown record `" + f[0] + "` in " + path);
            }
        }
    }

    private static String unescape(String field) {
        StringBuilder out = new StringBuilder();
        for (int i = 0; i < field.length(); i++) {
            char c = field.charAt(i);
            if (c == '\\' && i + 1 < field.length()) {
                char next = field.charAt(++i);
                out.append(next == 't' ? '\t' : next == 'n' ? '\n' : next);
            } else {
                out.append(c);
            }
        }
        return out.toString();
    }

    // Fields the UI reads from each engine table: `WindowData.PlayerStatus` -> CurrentHealth, ...
    private Map<String, Set<String>> tableFields() {
        Map<String, Set<String>> fields = new TreeMap<>();
        for (String path : paths.keySet()) {
            int split = path.lastIndexOf('.');
            String table = path.substring(0, split);
            String field = path.substring(split + 1);
            if (table.indexOf('.') < 0 || field.length() < MIN_FIELD_LENGTH || field.contains(INDEX_MARK)) {
                continue;
            }
            fields.computeIfAbsent(table, key -> new TreeSet<>()).add(field);
        }
        return fields;
    }

    // UI names C may use to reach into Lua, with their kind; XML schema words are left out.
    private Map<String, String> hookNames(Set<String> xmlNames) {
        Map<String, String> kinds = new HashMap<>();
        for (String path : paths.keySet()) {
            if (!path.contains(INDEX_MARK)) {
                kinds.put(path, KIND_PATH);
            }
        }
        for (Map<String, Integer> values : eventValues.values()) {
            for (String value : values.keySet()) {
                if (value.startsWith(SYSTEM_DATA)) {
                    continue;
                }
                kinds.putIfAbsent(value, eventValues.get("EventHandler.event") == values ? KIND_EVENT : KIND_GLOBAL);
            }
        }
        for (Handler handler : handlers) {
            if (!handler.event.startsWith(SYSTEM_DATA) && !handler.event.isEmpty()) {
                kinds.putIfAbsent(handler.event, KIND_EVENT);
            }
            kinds.putIfAbsent(handler.function, KIND_GLOBAL);
        }
        for (String name : definitions.keySet()) {
            if (name.length() >= MIN_HOOK_NAME_LENGTH) {
                kinds.putIfAbsent(name, KIND_GLOBAL);
            }
        }
        for (String name : windows.keySet()) {
            if (name.length() >= MIN_HOOK_NAME_LENGTH) {
                kinds.putIfAbsent(name, KIND_WINDOW);
            }
        }
        kinds.keySet().removeIf(xmlNames::contains);
        return kinds;
    }

    // ---- 1. Lua -> C ----

    private int[] apiCallSites(List<String> lines, List<String> index, Map<Function, String> uiLines,
            Set<String> apiNames) {
        int functions = 0;
        int called = 0;
        int sites = 0;
        for (Function function : currentProgram.getFunctionManager().getFunctions(true)) {
            if (!hasTag(function, TAG_API)) {
                continue;
            }
            functions++;
            List<String> luaNames = new ArrayList<>();
            String comment = function.getComment();
            String first = comment == null ? "" : comment.split(LINE_BREAK, -1)[0];
            Matcher matcher = NAME_LUA_LINE.matcher(first);
            if (matcher.matches()) {
                luaNames.add(matcher.group(2));
                if (matcher.group(3) != null) {
                    for (String other : matcher.group(3).split(ALSO_SEPARATOR)) {
                        luaNames.add(other.trim());
                    }
                }
            } else {
                String name = function.getName(true);
                luaNames.add((name.startsWith(API_NAMESPACE + SCOPE) ? name.substring(API_NAMESPACE.length()
                    + SCOPE.length()) : name).replace(SCOPE, "."));
            }
            apiNames.addAll(luaNames);
            List<Call> list = new ArrayList<>();
            for (String name : luaNames) {
                list.addAll(calls.getOrDefault(name, List.of()));
            }
            list.sort((a, b) -> a.file.equals(b.file) ? Integer.compare(a.line, b.line) : a.file.compareTo(b.file));
            String uiLine = list.isEmpty() ? null : uiLine(list);
            uiLines.put(function, uiLine);
            if (list.isEmpty()) {
                continue;
            }
            called++;
            sites += list.size();
            String defined = "";
            for (String name : luaNames) {
                if (definitions.containsKey(name)) {
                    defined = "; the UI also defines `" + name + "` (" + definitions.get(name)[1] + ":"
                        + definitions.get(name)[2] + ")";
                }
            }
            lines.add("`" + function.getName(true) + "` " + hex(function.getEntryPoint().getOffset()) + " `"
                + String.join("`, `", luaNames) + "`: " + uiLine.substring(UI_LINE_PREFIX.length()) + defined);
            StringBuilder entry = new StringBuilder("`" + String.join("`/`", luaNames) + "` "
                + hex(function.getEntryPoint().getOffset()) + ":");
            String file = null;
            for (Call call : list) {
                if (!call.file.equals(file)) {
                    file = call.file;
                    entry.append(file == null ? "" : ";").append(" ").append(file);
                }
                entry.append(" ").append(call.line).append(" (").append(call.enclosing).append(", ")
                    .append(call.args).append(call.variadic ? VARIADIC_MARK : "").append("->").append(call.results)
                    .append(")");
            }
            index.add(entry.toString().replace(": ;", ":"));
        }
        return new int[] {functions, called, sites};
    }

    private static String uiLine(List<Call> list) {
        Set<String> files = new TreeSet<>();
        int minArgs = Integer.MAX_VALUE;
        int maxArgs = Integer.MIN_VALUE;
        boolean variadic = false;
        int minResults = Integer.MAX_VALUE;
        int maxResults = Integer.MIN_VALUE;
        int forwarded = 0;
        Map<String, Integer> perFile = new TreeMap<>();
        for (Call call : list) {
            files.add(call.file);
            perFile.merge(call.file, 1, Integer::sum);
            minArgs = Math.min(minArgs, call.args);
            maxArgs = Math.max(maxArgs, call.args);
            variadic |= call.variadic;
            if (call.results.equals(RESULT_ALL)) {
                forwarded++;
            } else {
                int results = Integer.parseInt(call.results);
                minResults = Math.min(minResults, results);
                maxResults = Math.max(maxResults, results);
            }
        }
        String results = minResults == Integer.MAX_VALUE ? "all forwarded"
            : range(minResults, maxResults) + (forwarded > 0 ? " (" + forwarded + " forwarded)" : "");
        // Example: the first call in the file calling it most.
        String bestFile = list.get(0).file;
        for (var entry : perFile.entrySet()) {
            if (entry.getValue() > perFile.get(bestFile)) {
                bestFile = entry.getKey();
            }
        }
        Call example = null;
        for (Call call : list) {
            if (call.file.equals(bestFile)) {
                example = call;
                break;
            }
        }
        return UI_LINE_PREFIX + list.size() + (list.size() == 1 ? " call" : " calls") + " in " + files.size()
            + (files.size() == 1 ? " file" : " files") + ", args " + range(minArgs, maxArgs)
            + (variadic ? VARIADIC_MARK : "") + ", results " + results + "; e.g. " + example.file + ":" + example.line;
    }

    private static String range(int min, int max) {
        return min == max ? String.valueOf(min) : min + "-" + max;
    }

    private void applyComments(Map<Function, String> uiLines) {
        for (var entry : uiLines.entrySet()) {
            Function function = entry.getKey();
            String uiLine = entry.getValue();
            String comment = function.getComment();
            String[] lines = comment == null ? new String[0] : comment.split(LINE_BREAK, -1);
            Matcher matcher = lines.length == 0 ? null : NAME_LUA_LINE.matcher(lines[0]);
            boolean ours = matcher != null && matcher.matches() && expectedName(function, matcher.group(1))
                && (lines.length == 1 || (lines.length == 2 && lines[1].startsWith(UI_LINE_PREFIX)));
            if (!ours) {
                if (uiLine != null) {
                    skipped.add(hex(function.getEntryPoint().getOffset()) + " `" + function.getName(true)
                        + "`: plate comment is not the NameLuaFunctions one; not changed (" + uiLine + ")");
                }
                continue;
            }
            String wanted = uiLine == null ? lines[0] : lines[0] + LINE_BREAK + uiLine;
            if (wanted.equals(comment)) {
                continue;
            }
            function.setComment(wanted);
            count(lines.length == 1 ? "UI comment lines added" : uiLine == null ? "UI comment lines removed"
                : "UI comment lines updated");
        }
    }

    // NameLuaFunctions' path is the name, or the name with an _<address> suffix on a clash.
    private static boolean expectedName(Function function, String path) {
        String name = function.getName(true);
        return name.equals(path) || name.equals(path + "_" + Long.toHexString(function.getEntryPoint().getOffset()));
    }

    // ---- 2. C -> Lua ----

    private void hooks(Map<String, String> kinds, Map<Function, Set<String>> hookFunctions,
            Map<String, Map<Long, Integer>> callees, List<String> pathLines, List<String> engineOnly,
            List<String> eventLines, List<String> globalLines) {
        Map<String, List<String>> handlersByEvent = new TreeMap<>();
        for (Handler handler : handlers) {
            handlersByEvent.computeIfAbsent(handler.event, key -> new ArrayList<>()).add(handler.function);
        }
        List<String> texts = new ArrayList<>(stringAddresses.keySet());
        texts.sort(null);
        for (String text : texts) {
            String kind = kinds.get(text);
            boolean enginePath = text.startsWith(SYSTEM_DATA) || text.startsWith(WINDOW_DATA);
            if (kind == null && !enginePath) {
                continue;
            }
            List<Long> addresses = stringAddresses.get(text);
            Set<String> users = new TreeSet<>();
            Map<Long, Integer> consumer = new TreeMap<>();
            for (long address : addresses) {
                for (long site : referenceSites.getOrDefault(address, List.of())) {
                    Function function = functionContaining(site);
                    if (function == null) {
                        continue;
                    }
                    users.add(function.getName(true) + " " + hex(function.getEntryPoint().getOffset()));
                    long callee = consumingCall(site);
                    consumer.merge(callee, 1, Integer::sum);
                    if (kind != null) {
                        hookFunctions.computeIfAbsent(function, key -> new TreeSet<>()).add(text);
                        callees.computeIfAbsent(kind, key -> new HashMap<>()).merge(callee, 1, Integer::sum);
                    }
                }
            }
            String where = hexList(addresses) + (users.isEmpty() ? " (no code references)" : " in "
                + String.join(", ", users)) + calleeNote(consumer);
            if (kind == null) {
                engineOnly.add("`" + text + "` " + where);
                continue;
            }
            switch (kind) {
                case KIND_PATH -> {
                    PathUse use = paths.get(text);
                    List<String> registered = handlersByEvent.getOrDefault(text, List.of());
                    pathLines.add("`" + text + "` " + where + "; UI reads " + use.reads + ", writes " + use.writes
                        + " (" + use.file + ":" + use.line + ")" + (registered.isEmpty() ? ""
                        : "; handlers " + quoteAll(new TreeSet<>(registered))));
                }
                case KIND_EVENT -> {
                    Integer xml = eventValues.getOrDefault("EventHandler.event", Map.of()).get(text);
                    List<String> registered = handlersByEvent.getOrDefault(text, List.of());
                    eventLines.add("`" + text + "` " + where + "; XML handlers " + (xml == null ? 0 : xml)
                        + ", Lua registrations " + registered.size());
                }
                default -> {
                    String[] definition = definitions.get(text);
                    String[] window = windows.get(text);
                    String origin = window != null ? "window `<" + window[0] + ">` " + window[1] + ":" + window[2]
                        : definition != null ? "Lua " + definition[0] + " " + definition[1] + ":" + definition[2]
                        : "event handler function";
                    globalLines.add("`" + text + "` (" + origin + ") " + where);
                }
            }
        }
    }

    private String calleeNote(Map<Long, Integer> consumer) {
        List<String> parts = new ArrayList<>();
        for (var entry : consumer.entrySet()) {
            if (entry.getKey() >= 0) {
                parts.add(label(entry.getKey()) + (entry.getValue() > 1 ? " x" + entry.getValue() : ""));
            }
        }
        return parts.isEmpty() ? "" : "; passed to " + String.join(", ", parts);
    }

    // Tables the UI reads fields of, matched against C functions referencing those field names.
    private void fieldMapping(Map<String, Set<String>> tableFields, List<String> lines,
            Map<Function, Set<String>> hookFunctions) {
        Map<String, Set<Function>> fieldUsers = new HashMap<>();
        for (Set<String> fields : tableFields.values()) {
            for (String field : fields) {
                if (fieldUsers.containsKey(field)) {
                    continue;
                }
                Set<Function> users = new HashSet<>();
                for (long address : stringAddresses.getOrDefault(field, List.of())) {
                    for (long site : referenceSites.getOrDefault(address, List.of())) {
                        Function function = functionContaining(site);
                        if (function != null) {
                            users.add(function);
                        }
                    }
                }
                fieldUsers.put(field, users);
            }
        }
        for (var entry : tableFields.entrySet()) {
            Set<String> fields = entry.getValue();
            Map<Function, Set<String>> matched = new HashMap<>();
            for (String field : fields) {
                for (Function function : fieldUsers.get(field)) {
                    matched.computeIfAbsent(function, key -> new TreeSet<>()).add(field);
                }
            }
            int needed = Math.max(MIN_FIELD_MATCHES, (int) Math.ceil(MIN_FIELD_SHARE * fields.size()));
            List<Function> winners = new ArrayList<>();
            for (var match : matched.entrySet()) {
                if (match.getValue().size() >= needed) {
                    winners.add(match.getKey());
                }
            }
            if (winners.isEmpty()) {
                continue;
            }
            winners.sort((a, b) -> matched.get(b).size() != matched.get(a).size()
                ? Integer.compare(matched.get(b).size(), matched.get(a).size())
                : a.getEntryPoint().compareTo(b.getEntryPoint()));
            List<String> parts = new ArrayList<>();
            for (Function function : winners) {
                parts.add(function.getName(true) + " " + hex(function.getEntryPoint().getOffset()) + " ("
                    + matched.get(function).size() + ")");
                hookFunctions.computeIfAbsent(function, key -> new TreeSet<>()).add(entry.getKey() + ".*");
            }
            lines.add("`" + entry.getKey() + "` (" + fields.size() + " fields read by the UI): "
                + String.join(", ", parts));
        }
    }

    // ---- 3. XML ----

    private Map<Function, Set<String>> namesReferenced(Set<String> names) {
        Map<Function, Set<String>> refs = new HashMap<>();
        for (String name : names) {
            for (long address : stringAddresses.getOrDefault(name, List.of())) {
                for (long site : referenceSites.getOrDefault(address, List.of())) {
                    Function function = functionContaining(site);
                    if (function != null) {
                        refs.computeIfAbsent(function, key -> new HashSet<>()).add(name);
                    }
                }
            }
        }
        return refs;
    }

    private Set<String> level1(String element) {
        Set<String> names = new TreeSet<>(attributesOf.getOrDefault(element, Map.of()).keySet());
        names.addAll(childrenOf.getOrDefault(element, Map.of()).keySet());
        return names;
    }

    private Set<String> level2(String element) {
        Set<String> names = level1(element);
        for (String child : childrenOf.getOrDefault(element, Map.of()).keySet()) {
            names.addAll(level1(child));
        }
        names.remove(element);
        return names;
    }

    // A tag compare `PUSH "Tag"; ...; CALL compare; ...; Jcc` and the first call in a branch of it
    // whose function (with its direct callees) references the element's attributes and children.
    private Dispatch decodeDispatch(String element, long site, Map<Function, Set<String>> xmlRefs) throws Exception {
        if (!isPushImmediate(site)) {
            return null;
        }
        Instruction instruction = getInstructionAt(toAddr(site - OPCODE_LENGTH));
        Instruction compare = null;
        for (int i = 0; i < MAX_CALL_LOOKAHEAD && instruction != null; i++) {
            instruction = instruction.getNext();
            if (instruction != null && instruction.getFlowType().isCall()) {
                compare = instruction;
                break;
            }
        }
        if (compare == null) {
            return null;
        }
        Instruction branch = null;
        instruction = compare;
        for (int i = 0; i < MAX_JCC_LOOKAHEAD && instruction != null; i++) {
            instruction = instruction.getNext();
            if (instruction == null || instruction.getFlowType().isCall()) {
                return null;
            }
            if (instruction.getFlowType().isConditional() && instruction.getFlowType().isJump()) {
                branch = instruction;
                break;
            }
        }
        if (branch == null || branch.getFlows().length == 0) {
            return null;
        }
        long compareCallee = callTarget(compare);
        Set<String> wanted = level2(element);
        Set<String> direct = level1(element);
        for (Address start : new Address[] {branch.getFallThrough(), branch.getFlows()[0]}) {
            Instruction current = start == null ? null : getInstructionAt(start);
            for (int i = 0; i < MAX_BRANCH_INSTRUCTIONS && current != null; i++) {
                FlowType flow = current.getFlowType();
                if (flow.isCall()) {
                    long callee = callTarget(current);
                    Function function = callee < 0 || callee == compareCallee ? null : functionAt(callee);
                    if (function != null) {
                        Set<String> own = xmlRefs.getOrDefault(function, Set.of());
                        Set<String> evidence = new TreeSet<>(own);
                        evidence.retainAll(wanted);
                        for (Function sub : function.getCalledFunctions(monitor)) {
                            Set<String> subNames = new HashSet<>(xmlRefs.getOrDefault(sub, Set.of()));
                            subNames.retainAll(wanted);
                            evidence.addAll(subNames);
                        }
                        boolean anchored = false;
                        for (String name : own) {
                            anchored |= direct.contains(name);
                        }
                        if (anchored && evidence.size() >= MIN_XML_EVIDENCE) {
                            Dispatch dispatch = new Dispatch();
                            dispatch.element = element;
                            dispatch.site = site;
                            Function dispatcher = functionContaining(site);
                            dispatch.dispatcher = dispatcher == null ? -1 : dispatcher.getEntryPoint().getOffset();
                            dispatch.compare = compareCallee;
                            dispatch.handler = callee;
                            dispatch.evidence = evidence;
                            return dispatch;
                        }
                    }
                } else if (flow.isJump() && !flow.isConditional() || flow.isTerminal()) {
                    break;
                }
                current = current.getNext();
            }
        }
        return null;
    }

    // Functions reading an element's tag and its attributes/children directly (no dispatch call).
    private Map<Long, String> directParsers(Map<Function, Set<String>> xmlRefs, Set<Long> dispatchHandlers,
            Map<String, Set<Long>> elementHandlers, List<String> lines) {
        Map<String, Set<Function>> byElement = new TreeMap<>();
        Map<Function, Set<String>> evidenceOf = new HashMap<>();
        for (var entry : xmlRefs.entrySet()) {
            Function function = entry.getKey();
            if (dispatchHandlers.contains(function.getEntryPoint().getOffset())) {
                continue;
            }
            Set<String> qualifying = new TreeSet<>();
            for (String element : elements.keySet()) {
                if (!entry.getValue().contains(element)) {
                    continue;
                }
                Set<String> evidence = new TreeSet<>(entry.getValue());
                evidence.retainAll(level2(element));
                if (evidence.size() >= MIN_XML_EVIDENCE) {
                    qualifying.add(element);
                    evidenceOf.computeIfAbsent(function, key -> new TreeSet<>()).addAll(evidence);
                }
            }
            Set<String> outermost = new TreeSet<>(qualifying);
            for (String element : qualifying) {
                outermost.removeAll(childrenOf.getOrDefault(element, Map.of()).keySet());
            }
            for (String element : outermost) {
                byElement.computeIfAbsent(element, key -> new TreeSet<>((a, b) -> a.getEntryPoint()
                    .compareTo(b.getEntryPoint()))).add(function);
            }
        }
        Map<Long, String> names = new TreeMap<>();
        Map<Function, Integer> elementCount = new HashMap<>();
        for (Set<Function> functions : byElement.values()) {
            for (Function function : functions) {
                elementCount.merge(function, 1, Integer::sum);
            }
        }
        for (var entry : byElement.entrySet()) {
            String element = entry.getKey();
            List<String> parts = new ArrayList<>();
            for (Function function : entry.getValue()) {
                parts.add(function.getName(true) + " " + hex(function.getEntryPoint().getOffset()) + " ("
                    + String.join(", ", evidenceOf.get(function)) + ")");
            }
            boolean unique = entry.getValue().size() == 1 && !elementHandlers.containsKey(element);
            Function only = entry.getValue().iterator().next();
            if (unique && elementCount.get(only) == 1) {
                names.put(only.getEntryPoint().getOffset(), XML_NAMESPACE + SCOPE + XML_PREFIX + sanitize(element));
            }
            lines.add("`<" + element + ">`: " + String.join("; ", parts) + (unique && elementCount.get(only) == 1
                ? " -> named" : ""));
        }
        return names;
    }

    // ---- Apply helpers ----

    private void tag(Function function, String name, String comment) {
        for (FunctionTag tag : function.getTags()) {
            if (tag.getName().equals(name)) {
                return;
            }
        }
        if (foreign(function)) {
            kept.add(hex(function.getEntryPoint().getOffset()) + " `" + function.getName(true)
                + "` is a library function; not tagged " + name);
            return;
        }
        function.addTag(name);
        count(name + " tags added");
        FunctionTagManager manager = currentProgram.getFunctionManager().getFunctionTagManager();
        FunctionTag tag = manager.getFunctionTag(name);
        if (tag != null && (tag.getComment() == null || tag.getComment().isBlank())) {
            tag.setComment(comment);
            count("tag comments set");
        }
    }

    private void rename(long address, String path) throws Exception {
        Function function = functionAt(address);
        if (function == null) {
            anomalies.add("No function at " + hex(address) + " for `" + path + "`");
            return;
        }
        int split = path.lastIndexOf(SCOPE);
        String name = path.substring(split + SCOPE.length());
        Namespace namespace = namespace(path.substring(0, split));
        Symbol symbol = function.getSymbol();
        if (symbol.getName().equals(name) && symbol.getParentNamespace().equals(namespace)) {
            return;
        }
        if (symbol.getSource() != SourceType.DEFAULT || foreign(function)) {
            kept.add(hex(address) + " `" + symbol.getName(true) + "` (" + symbol.getSource() + "), not renamed to `"
                + path + "`");
            return;
        }
        try {
            symbol.setNameAndNamespace(name, namespace, SourceType.ANALYSIS);
        } catch (DuplicateNameException duplicate) {
            String unique = name + "_" + Long.toHexString(address);
            symbol.setNameAndNamespace(unique, namespace, SourceType.ANALYSIS);
            anomalies.add("`" + path + "` already exists; " + hex(address) + " named `" + unique + "`");
        }
        count("functions renamed");
    }

    private Namespace namespace(String path) throws Exception {
        SymbolTable symbols = currentProgram.getSymbolTable();
        Namespace namespace = currentProgram.getGlobalNamespace();
        for (String part : path.split(SCOPE)) {
            Namespace child = symbols.getNamespace(part, namespace);
            if (child == null) {
                child = symbols.createNameSpace(namespace, part, SourceType.ANALYSIS);
                count("namespaces created");
            }
            namespace = child;
        }
        return namespace;
    }

    private void count(String change) {
        changes.merge(change, 1, Integer::sum);
    }

    // Functions other scripts name (Gamebryo and statically linked libraries) are left alone.
    private static boolean foreign(Function function) {
        for (FunctionTag tag : function.getTags()) {
            for (String prefix : FOREIGN_TAG_PREFIXES) {
                if (tag.getName().startsWith(prefix)) {
                    return true;
                }
            }
        }
        return false;
    }

    private static boolean hasTag(Function function, String name) {
        for (FunctionTag tag : function.getTags()) {
            if (tag.getName().equals(name)) {
                return true;
            }
        }
        return false;
    }

    // ---- Report ----

    private void report(PrintWriter out, boolean apply, String xrefPath, int[] apiCounts, List<String> apiLines,
            List<String> apiIndex, List<String> notApi, Map<Function, Set<String>> hookFunctions,
            Map<String, Map<Long, Integer>> callees, List<String> pathLines, List<String> engineOnly,
            List<String> eventLines, List<String> globalLines, List<String> fieldLines, List<String> uiOnlyTables,
            Map<String, Set<Long>> tagAddresses, List<Dispatch> dispatches, Map<Long, Set<String>> handlerElements,
            Map<String, Set<Long>> elementHandlers, Map<Long, Set<String>> dispatcherElements,
            Map<Long, Integer> compareCallees, Map<Long, String> xmlNamesProposed, List<String> sharedHandlers,
            List<String> ambiguousElements, List<String> directLines, Map<Function, Set<String>> xmlRefs,
            Map<String, Long> vftables) throws Exception {
        int uiCalls = 0;
        for (List<Call> list : calls.values()) {
            uiCalls += list.size();
        }
        List<Function> hookList = new ArrayList<>();
        for (var entry : hookFunctions.entrySet()) {
            if (entry.getValue().size() >= MIN_HOOK_NAMES) {
                hookList.add(entry.getKey());
            }
        }
        int attributeCount = 0;
        for (Map<String, Integer> attributes : attributesOf.values()) {
            attributeCount += attributes.size();
        }
        out.println("# UI cross-reference: " + currentProgram.getName());
        out.println();
        out.println("Generated by `CrossReferenceUi.java` " + (apply ? "with" : "without") + " `" + APPLY_FLAG
            + "`, " + (details ? "with" : "without") + " `" + DETAILS_FLAG + "`, from `" + new File(xrefPath).getName()
            + "` (written by `extract_ui_xref.py`).");
        out.println();
        out.println("## Summary");
        out.println();
        out.println("- UI calls to non-UI functions: " + uiCalls + " sites of " + calls.size() + " names");
        out.println("- " + TAG_API + " functions: " + apiCounts[0] + "; called from the UI: " + apiCounts[1] + " ("
            + apiCounts[2] + " call sites); names called but not UO Lua API: " + notApi.size());
        out.println("- C -> Lua: " + pathLines.size() + " SystemData/WindowData paths used by the UI found as "
            + "strings, " + engineOnly.size() + " engine-only paths, " + eventLines.size() + " event names, "
            + globalLines.size() + " UI globals/windows, " + fieldLines.size() + " UI tables matched by field names; "
            + hookList.size() + " functions with >= " + MIN_HOOK_NAMES + " UI names, one UI-specific (" + TAG_HOOK + ")");
        out.println("- XML: " + elements.size() + " elements, " + attributeCount + " element/attribute pairs; "
            + dispatches.size() + " tag dispatch sites in " + dispatcherElements.size() + " functions, "
            + handlerElements.size() + " handlers (" + sharedHandlers.size() + " shared), " + ambiguousElements.size()
            + " elements with several handlers; " + xmlNamesProposed.size() + " parser names proposed");
        out.println("- Changes: " + (apply ? changes.isEmpty() ? "none" : changes.toString() : "report only"));

        out.println();
        out.println("## 1. Lua -> C: UO Lua API calls from the UI");
        out.println();
        out.println("Plate comment line format: `UI: <calls> in <files>, args <min-max>[+], results <min-max> "
            + "(<n> forwarded); e.g. <file>:<line>`. `+`: the last argument is a call or `...` (may expand). "
            + "Results: values assigned (0 = statement, 1 = expression); forwarded = `return F()` or last "
            + "argument/table item.");
        section(out, "API functions called from the UI", apiLines);
        detailSection(out, "Call site index (line (enclosing function, args->results))", apiIndex);
        section(out, "Called by the UI but not a UO Lua API function", notApi);
        section(out, "Plate comments not changed", skipped);

        out.println();
        out.println("## 2. C -> Lua");
        out.println();
        out.println("C reaches Lua state by dotted path strings (`SystemData.*`, `WindowData.*`), by event "
            + "names it fires on windows, by window names it creates or toggles, and by table field names it "
            + "writes. `passed to` names the call taking the string right after it is pushed.");
        List<String> calleeLines = new ArrayList<>();
        for (var entry : callees.entrySet()) {
            List<Map.Entry<Long, Integer>> sorted = new ArrayList<>(entry.getValue().entrySet());
            sorted.sort((a, b) -> Integer.compare(b.getValue(), a.getValue()));
            List<String> parts = new ArrayList<>();
            for (int i = 0; i < Math.min(MAX_CALLEES_REPORTED, sorted.size()); i++) {
                long callee = sorted.get(i).getKey();
                parts.add((callee < 0 ? "no call" : label(callee) + " " + hex(callee)) + " x" + sorted.get(i).getValue());
            }
            calleeLines.add(entry.getKey() + ": " + String.join(", ", parts));
        }
        section(out, "Calls consuming UI name strings", calleeLines);
        detailSection(out, "SystemData/WindowData paths used by the UI and present in UOSA", pathLines);
        detailSection(out, "SystemData/WindowData strings the UI never uses", engineOnly);
        section(out, "UI tables without a whole-path string (filled field by field or by RegisterWindowData)",
            uiOnlyTables.isEmpty() ? List.of() : List.of(String.join(", ", uiOnlyTables)));
        section(out, "Event names", eventLines);
        detailSection(out, "UI Lua globals, handler functions and window names in UOSA", globalLines);
        section(out, "UI tables matched to C functions by field names (fields referenced)", fieldLines);
        List<String> hookLines = new ArrayList<>();
        for (Function function : hookList) {
            List<String> names = new ArrayList<>(hookFunctions.get(function));
            hookLines.add(function.getName(true) + " " + hex(function.getEntryPoint().getOffset()) + ": "
                + names.size() + " names: " + quoteAll(names.subList(0, Math.min(MAX_NAMES_SHOWN, names.size())))
                + (names.size() > MAX_NAMES_SHOWN ? ", ..." : ""));
        }
        section(out, TAG_HOOK + " functions", hookLines);

        out.println();
        out.println("## 3. XML schema -> parsers");
        out.println();
        out.println("Per element: UI uses, tag string addresses, dispatch sites (dispatcher -> handler) and "
            + "attributes with UI counts; `+` the handler (or its callees) references the attribute string, "
            + "`=` the string exists in UOSA, `-` it does not.");
        List<String> compareLines = new ArrayList<>();
        for (var entry : compareCallees.entrySet()) {
            compareLines.add(label(entry.getKey()) + " " + hex(entry.getKey()) + " x" + entry.getValue());
        }
        section(out, "Tag compare calls", compareLines);
        List<String> elementLines = new ArrayList<>();
        List<String> missingAttributes = new ArrayList<>();
        List<String> sortedElements = new ArrayList<>(elements.keySet());
        sortedElements.sort((a, b) -> elements.get(b)[0] != elements.get(a)[0]
            ? Integer.compare(elements.get(b)[0], elements.get(a)[0]) : a.compareTo(b));
        for (String element : sortedElements) {
            Set<Long> handlerSet = elementHandlers.getOrDefault(element, Set.of());
            Set<String> handled = new HashSet<>();
            for (long handler : handlerSet) {
                Function function = functionAt(handler);
                if (function != null) {
                    handled.addAll(xmlRefs.getOrDefault(function, Set.of()));
                    for (Function sub : function.getCalledFunctions(monitor)) {
                        handled.addAll(xmlRefs.getOrDefault(sub, Set.of()));
                    }
                }
            }
            List<String> dispatchParts = new ArrayList<>();
            Map<Long, Integer> sitesPerHandler = new TreeMap<>();
            for (Dispatch dispatch : dispatches) {
                if (dispatch.element.equals(element)) {
                    sitesPerHandler.merge(dispatch.handler, 1, Integer::sum);
                    if (details) {
                        dispatchParts.add(hexShort(dispatch.site) + " " + label(dispatch.dispatcher) + " -> "
                            + label(dispatch.handler) + " " + hex(dispatch.handler));
                    }
                }
            }
            if (!details) {
                for (var entry : sitesPerHandler.entrySet()) {
                    dispatchParts.add(label(entry.getKey()) + " " + hex(entry.getKey()) + " (" + entry.getValue()
                        + (entry.getValue() == 1 ? " site)" : " sites)"));
                }
            }
            List<String> attributeParts = new ArrayList<>();
            for (var attribute : attributesOf.getOrDefault(element, Map.of()).entrySet()) {
                String name = attribute.getKey();
                String mark = handled.contains(name) ? "+" : stringAddresses.containsKey(name) ? "=" : "-";
                attributeParts.add(name + mark + attribute.getValue());
                if (mark.equals("-")) {
                    missingAttributes.add("`" + element + "." + name + "` (" + attribute.getValue() + ")");
                }
            }
            Set<Long> tags = tagAddresses.get(element);
            elementLines.add("`<" + element + ">` " + elements.get(element)[0] + " (" + elementFirst.get(element)
                + "): tag " + (tags.isEmpty() ? "not in UOSA" : hexList(tags)) + "; dispatch: "
                + (dispatchParts.isEmpty() ? "none decoded" : String.join(", ", dispatchParts)) + "; attributes: "
                + (attributeParts.isEmpty() ? "none" : String.join(" ", attributeParts)) + "; children: "
                + String.join(" ", childrenOf.getOrDefault(element, Map.of()).keySet()));
        }
        section(out, "Elements", elementLines);
        section(out, "Attributes whose name is not a string in UOSA (never read by name)", missingAttributes);
        List<String> nameLines = new ArrayList<>();
        for (var entry : xmlNamesProposed.entrySet()) {
            nameLines.add(hex(entry.getKey()) + " " + label(entry.getKey()) + " -> `" + entry.getValue() + "`");
        }
        section(out, "Parser names (" + XML_NAMESPACE + SCOPE + XML_PREFIX + "<Element>, DEFAULT names only)",
            nameLines);
        section(out, "Handlers shared by several elements (not named)", sharedHandlers);
        section(out, "Elements with several handlers (not named)", ambiguousElements);
        List<String> dispatcherLines = new ArrayList<>();
        for (var entry : dispatcherElements.entrySet()) {
            dispatcherLines.add(hex(entry.getKey()) + " " + label(entry.getKey()) + ": " + quoteAll(entry.getValue()));
        }
        section(out, "Dispatchers (elements compared)", dispatcherLines);
        section(out, "Direct parsers (tag and >= " + MIN_XML_EVIDENCE + " attribute/child names, no dispatch call)",
            directLines);

        out.println();
        out.println("## 4. UI texture chain");
        out.println();
        out.println("XML `<Texture name file>` (`Textures/Textures.xml` via `Interface.xml` `<Include>`) -> "
            + "`<Assets>` parser -> `MythUi::TextureDef` -> UI asset loader -> path resolution (custom UI "
            + "folder, then packages/disk) -> `NiSourceTexture::Create` -> UI texture registry by name, used by "
            + "`texture=` attributes and `DynamicImageSetTexture`/`ButtonSetTexture`. Engine textures "
            + "(`RequestTexture`, `RequestTileArt`, `RequestGumpArt`) go through a texture file location and "
            + "the `TextureManager` cache instead, and are registered in the same UI texture registry under a "
            + "generated name returned to Lua. Each step is re-checked:");
        List<String> chainLines = new ArrayList<>();
        for (Step step : TEXTURE_CHAIN) {
            List<String> results = new ArrayList<>();
            int passed = 0;
            for (Check check : step.checks) {
                boolean ok = verify(step.address, check, vftables);
                passed += ok ? 1 : 0;
                results.add(check.kind + " " + check.value.replace("\n", "\\n") + (ok ? " ok" : " FAILED"));
            }
            chainLines.add(hex(step.address) + " " + label(step.address) + ": " + step.role + " ["
                + (passed == step.checks.length ? "verified" : passed + "/" + step.checks.length) + ": "
                + String.join("; ", results) + "]");
        }
        section(out, "Steps", chainLines);

        section(out, "Not renamed or tagged (named already, or library functions)", kept);
        section(out, "Anomalies", anomalies);
        section(out, "Caveats", List.of(CAVEATS));
    }

    private boolean verify(long address, Check check, Map<String, Long> vftables) {
        Function function = functionAt(address);
        if (function == null) {
            return false;
        }
        switch (check.kind) {
            case CHECK_CALLS -> {
                long target = Long.decode(check.value);
                for (Function callee : function.getCalledFunctions(monitor)) {
                    if (callee.getEntryPoint().getOffset() == target) {
                        return true;
                    }
                }
                return false;
            }
            case CHECK_STRING -> {
                for (long string : stringAddresses.getOrDefault(check.value, List.of())) {
                    for (long site : referenceSites.getOrDefault(string, List.of())) {
                        if (function.getBody().contains(toAddr(site))) {
                            return true;
                        }
                    }
                }
                return false;
            }
            case CHECK_VFTABLE -> {
                Long table = vftables.get(check.value);
                if (table == null) {
                    return false;
                }
                for (long site : referenceSites.getOrDefault(table, List.of())) {
                    if (function.getBody().contains(toAddr(site))) {
                        return true;
                    }
                }
                return false;
            }
            case CHECK_SLOT -> {
                String[] parts = check.value.split(":");
                Long table = vftables.get(parts[0]);
                return table != null && u32(table + (long) POINTER_SIZE * Integer.parseInt(parts[1])) == address;
            }
            case CHECK_NAME -> {
                return function.getName(true).contains(check.value);
            }
            default -> {
                return false;
            }
        }
    }

    // A section listed only with --details; otherwise its size.
    private void detailSection(PrintWriter output, String title, List<String> lines) {
        if (details) {
            section(output, title, lines);
            return;
        }
        output.println();
        output.println("### " + title);
        output.println();
        output.println(lines.size() + " entries; run with `" + DETAILS_FLAG + "` to list them.");
    }

    private void section(PrintWriter output, String title, List<String> lines) {
        output.println();
        output.println("### " + title);
        output.println();
        if (lines.isEmpty()) {
            output.println("None.");
        }
        for (String line : lines) {
            output.println("- " + line);
        }
    }

    // ---- Strings, references, instructions ----

    private void scanStrings(Predicate<String> wanted) {
        for (Block block : blocks) {
            if (block.exec) {
                continue;
            }
            int i = 0;
            while (i < block.bytes.length) {
                int j = i;
                while (j < block.bytes.length && printable(block.bytes[j]) && j - i <= MAX_STRING_LENGTH) {
                    j++;
                }
                if (j < block.bytes.length && block.bytes[j] == 0 && j > i) {
                    String text = new String(block.bytes, i, j - i, StandardCharsets.US_ASCII);
                    if (wanted.test(text)) {
                        stringAddresses.computeIfAbsent(text, key -> new ArrayList<>()).add(block.start + i);
                        stringText.put(block.start + i, text);
                    }
                }
                i = j + 1;
            }
        }
    }

    private static boolean printable(byte value) {
        int c = value & BYTE_MASK;
        return c >= PRINTABLE_FIRST && c <= PRINTABLE_LAST;
    }

    // Raw scan of code for 32-bit immediates equal to a target (strings are pushed or moved as imm32).
    private void scanReferences(Set<Long> targets) {
        for (long target : targets) {
            referenceSites.put(target, new ArrayList<>());
        }
        for (Block block : blocks) {
            if (!block.exec) {
                continue;
            }
            for (int p = 0; p + POINTER_SIZE <= block.bytes.length; p++) {
                List<Long> sites = referenceSites.get(le32(block.bytes, p));
                if (sites != null) {
                    sites.add(block.start + p);
                }
            }
        }
    }

    private Map<String, Long> vftables() {
        Map<String, Long> tables = new HashMap<>();
        for (Symbol symbol : currentProgram.getSymbolTable().getSymbols(VFTABLE)) {
            tables.putIfAbsent(symbol.getParentNamespace().getName(true), symbol.getAddress().getOffset());
        }
        return tables;
    }

    private boolean isPushImmediate(long site) {
        Block block = blockOf(site - OPCODE_LENGTH);
        if (block == null || !block.exec || (block.bytes[(int) (site - OPCODE_LENGTH - block.start)] & BYTE_MASK)
                != OPCODE_PUSH_IMM) {
            return false;
        }
        Instruction push = getInstructionAt(toAddr(site - OPCODE_LENGTH));
        return push != null && push.getLength() == OPCODE_LENGTH + POINTER_SIZE;
    }

    // The callee of the first call after a PUSH of the string at site, or -1.
    private long consumingCall(long site) {
        if (!isPushImmediate(site)) {
            return -1;
        }
        Instruction instruction = getInstructionAt(toAddr(site - OPCODE_LENGTH));
        for (int i = 0; i < MAX_CALL_LOOKAHEAD && instruction != null; i++) {
            instruction = instruction.getNext();
            if (instruction != null && instruction.getFlowType().isCall()) {
                return callTarget(instruction);
            }
        }
        return -1;
    }

    private static long callTarget(Instruction call) {
        Address[] flows = call.getFlows();
        return flows.length == 0 ? -1 : flows[0].getOffset();
    }

    private Function functionContaining(long site) {
        return functionCache.computeIfAbsent(site, key -> getFunctionContaining(toAddr(key)));
    }

    private Function functionAt(long address) {
        return address < 0 ? null : getFunctionAt(toAddr(address));
    }

    private String referencers(List<Long> addresses) {
        Set<String> users = new TreeSet<>();
        for (long address : addresses) {
            for (long site : referenceSites.getOrDefault(address, List.of())) {
                Function function = functionContaining(site);
                if (function != null) {
                    users.add(function.getName(true) + " " + hex(function.getEntryPoint().getOffset()));
                }
            }
        }
        return users.isEmpty() ? "(no code references)" : "in " + String.join(", ", users);
    }

    private String label(long address) {
        Function function = functionAt(address);
        if (function == null) {
            function = address < 0 ? null : getFunctionContaining(toAddr(address));
        }
        return function == null ? "?" : "`" + function.getName(true) + "`";
    }

    // ---- Memory helpers ----

    private void loadBlocks() throws Exception {
        for (MemoryBlock block : currentProgram.getMemory().getBlocks()) {
            if (!block.isInitialized() || block.getSize() > Integer.MAX_VALUE) {
                continue;
            }
            Block copy = new Block();
            copy.start = block.getStart().getOffset();
            copy.bytes = new byte[(int) block.getSize()];
            copy.exec = block.isExecute();
            block.getBytes(block.getStart(), copy.bytes);
            blocks.add(copy);
        }
    }

    private Block blockOf(long address) {
        if (lastBlock != null && address >= lastBlock.start && address < lastBlock.start + lastBlock.bytes.length) {
            return lastBlock;
        }
        for (Block block : blocks) {
            if (address >= block.start && address < block.start + block.bytes.length) {
                lastBlock = block;
                return block;
            }
        }
        return null;
    }

    private long u32(long address) {
        Block block = address < 0 ? null : blockOf(address);
        if (block == null || address + POINTER_SIZE > block.start + block.bytes.length) {
            return -1;
        }
        return le32(block.bytes, (int) (address - block.start));
    }

    private static long le32(byte[] bytes, int index) {
        long value = 0;
        for (int i = POINTER_SIZE - 1; i >= 0; i--) {
            value = (value << BITS_PER_BYTE) | (bytes[index + i] & BYTE_MASK);
        }
        return value;
    }

    // ---- Formatting ----

    private static String sanitize(String name) {
        return name.replaceAll("[^A-Za-z0-9_]", "_");
    }

    private static String quoteAll(Collection<String> names) {
        List<String> parts = new ArrayList<>();
        for (String name : names) {
            parts.add("`" + name + "`");
        }
        return String.join(", ", parts);
    }

    private static String decodePath(String path) {
        return path.replace(ENCODED_SPACE, " ");
    }

    private static String hex(long value) {
        return String.format("`0x%08x`", value);
    }

    private static String hexShort(long value) {
        return String.format("0x%08x", value);
    }

    private static String hexList(Iterable<Long> values) {
        List<String> parts = new ArrayList<>();
        for (long value : values) {
            parts.add(hex(value));
        }
        return parts.isEmpty() ? "none" : String.join(", ", parts);
    }
}
