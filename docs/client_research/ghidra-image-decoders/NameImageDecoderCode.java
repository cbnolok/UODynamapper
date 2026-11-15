// Name the UOSA.exe code that turns a texture request into a decoded image: the texture manager and its
// direct and asset-resolver paths, UOTextureData and its factory, the asset loader configuration that picks
// the resolver path per resource type, the assetmap.xml parsers, the UI gump and managed texture locations,
// the Gamebryo image readers (BMP, DDS, SGI, TGA) whose vtable slots were still unnamed or not functions,
// the texture failure callback, and the unreferenced D3DX loader. Address-based, so it refuses any other
// build; every address is verified with stated structural checks first and a failed check is reported, not
// named. Findings: results/uosa_image_decoders.md.
// @category ECResearch
import ghidra.app.script.GhidraScript;
import ghidra.program.model.address.Address;
import ghidra.program.model.data.StringDataInstance;
import ghidra.program.model.listing.Data;
import ghidra.program.model.listing.Function;
import ghidra.program.model.listing.FunctionTag;
import ghidra.program.model.listing.Instruction;
import ghidra.program.model.scalar.Scalar;
import ghidra.program.model.symbol.Namespace;
import ghidra.program.model.symbol.Reference;
import ghidra.program.model.symbol.SourceType;
import ghidra.program.model.symbol.Symbol;
import ghidra.program.model.symbol.SymbolTable;
import java.io.PrintWriter;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardOpenOption;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

public class NameImageDecoderCode extends GhidraScript {
    private static final String APPLY_FLAG = "--apply";
    // Script arguments are split on whitespace, so a path with spaces is passed with %20.
    private static final String ENCODED_SPACE = "%20";
    // SHA-256 of the UOSA.exe these addresses were traced in (same build as ghidra-source-names).
    private static final String TESTED_SHA256 = "5a5e9dc536c836d1bf10b6f86bb88f61a6ae078f519d4060d3bc47aecbebad50";
    private static final String TAG = "UO_IMAGE";
    private static final String TAG_COMMENT = "Texture request to image decoder (NameImageDecoderCode)";
    private static final String COMMENT_PREFIX = "NameImageDecoderCode: ";
    private static final String GLOBAL = "";
    private static final String VFTABLE = "vftable";
    private static final int POINTER_SIZE = 4;
    private static final long UNSIGNED_INT_MASK = 0xffffffffL;
    private static final int HEX_RADIX = 16;

    // Gamebryo image reader vftables (unlabelled; slots: 0 deleting destructor, 1 CanReadFile, 2 ReadFile,
    // 3 ReadHeader) and the texture manager vftable.
    private static final String BMP_READER_VTABLE = "00ccc5ec";
    private static final String DDS_READER_VTABLE = "00ccc5fc";
    private static final String SGI_READER_VTABLE = "00ccc634";
    private static final String TGA_READER_VTABLE = "00ccc654";
    private static final String TEXTURE_MANAGER_VTABLE = "00cb39b4";
    private static final int SLOT_CAN_READ_FILE = 1;
    private static final int SLOT_READ_FILE = 2;
    private static final int SLOT_READ_HEADER = 3;
    private static final int SLOT_REQUEST_TEXTURE = 1;
    private static final int SLOT_IS_DIRECT_LOAD = 2;
    private static final int SLOT_GET_ASSET_ID = 3;
    private static final int SLOT_FACTORY_CREATE = 0;
    // ".targa" string after the SGI reader vftable (the TGA reader's second extension).
    private static final String TARGA_EXTENSION_STRING = "00ccc64c";
    // Header magics and sizes the readers compare.
    private static final long BMP_MAGIC = 0x4d42L;
    private static final long DDS_MAGIC = 0x20534444L;
    private static final long DDS_HEADER_SIZE = 0x7cL;
    // Texture manager cache key type (FNV-1a of the lowercased path, type 0x11000000).
    private static final long TEXTURE_CACHE_KEY_TYPE = 0x11000000L;
    // NiTexture::RendererData::FILENOTFOUND, the only failure code the release Create passes.
    private static final long TEXTURE_FAILURE_FILE_NOT_FOUND = 0x80000005L;
    // g_UOAssetManager + 0x160 + type * 4: per resource type, 1 = packed (asset resolver), 0 = raw path.
    private static final long ASSET_FORMAT_ARRAY_OFFSET = 0x160L;
    // Offset of the last default-packed type (0x1a, TileArtEnhanced) in that array.
    private static final long LAST_DEFAULT_PACKED_OFFSET = 0x1c8L;

    private static final String ASSET_LOADER_CONFIG = "AssetLoaderConfig.xml";
    private static final String ASSET_MAP = "Data/assetmap.xml";
    private static final String GUMP_ART_DIRECTORY = "Data/Interface/Default/Textures/GumpArt/";
    private static final String MANAGED_TEXTURE_DIRECTORY = "Data/Interface/InterfaceCore/Textures/";

    // Checks: "calls:ADDR" (the function references ADDR: call, pointer, jump or data),
    // "calledby:ADDR" (the function at ADDR references this one), "slot:CLASS:N" (vftable slot N of the
    // labelled CLASS::vftable holds this function), "vtslot:ADDR:N" (slot N of the table at ADDR holds this
    // function's address; with --apply a missing function there is created), "vftable:CLASS" (references
    // CLASS::vftable), "string:TEXT" (references the string TEXT), "const:HEX" (an operand equals HEX).
    private record Entry(String address, String namespace, String name, String evidence, String... checks) {
    }

    private static final Entry[] ENTRIES = {
        // Gamebryo 2.6 image readers (NiDevImageConverter tries them in the order NIF, TGA, SGI, DDS, BMP and
        // asks each CanReadFile(extension); none looks at the content before that).
        new Entry("006d3140", "NiBMPReader", "CanReadFile",
            "BMP reader vftable slot 1: stricmp of the extension with \".bmp\"",
            "vtslot:" + BMP_READER_VTABLE + ":" + SLOT_CAN_READ_FILE),
        new Entry("006d3160", "NiBMPReader", "ReadFile",
            "BMP reader vftable slot 2: ReadHeader, then the padded rows from bfOffBits",
            "vtslot:" + BMP_READER_VTABLE + ":" + SLOT_READ_FILE),
        new Entry("006d2de0", "NiBMPReader", "ReadHeader",
            "BMP reader vftable slot 3: bfType must be 0x4D42; bfSize, the reserved fields and biSizeImage are read and ignored; biSize 4 rejected; bit count 4/8/24/32; compression 1/2 (RLE) rejected; negative height = top-down",
            "vtslot:" + BMP_READER_VTABLE + ":" + SLOT_READ_HEADER, "const:" + Long.toHexString(BMP_MAGIC)),
        new Entry("006d3760", "NiDDSReader", "NiDDSReader",
            "stores the DDS reader vftable 0x00ccc5fc; called by NiDevImageConverter::NiDevImageConverter",
            "calls:" + DDS_READER_VTABLE, "calledby:00693cf0"),
        new Entry("006d3d20", "NiDDSReader", "CanReadFile",
            "DDS reader vftable slot 1: stricmp of the extension with \".dds\"",
            "vtslot:" + DDS_READER_VTABLE + ":" + SLOT_CAN_READ_FILE),
        new Entry("006d48f0", "NiDDSReader", "ReadFile",
            "DDS reader vftable slot 2: ReadHeader, then the surfaces (Read24Bit / Read32Bit / compressed blocks) into NiPixelData",
            "vtslot:" + DDS_READER_VTABLE + ":" + SLOT_READ_FILE, "calls:0068cad0"),
        new Entry("006d3810", "NiDDSReader", "ReadHeader",
            "DDS reader vftable slot 3: magic \"DDS \" (also byte-swapped) and header size 0x7C, else fails",
            "vtslot:" + DDS_READER_VTABLE + ":" + SLOT_READ_HEADER, "const:" + Long.toHexString(DDS_MAGIC),
            "const:" + Long.toHexString(DDS_HEADER_SIZE)),
        new Entry("006d4e80", "NiSGIReader", "NiSGIReader",
            "stores the SGI reader vftable 0x00ccc634; called by NiDevImageConverter::NiDevImageConverter",
            "calls:" + SGI_READER_VTABLE, "calledby:00693cf0"),
        new Entry("006d5300", "NiSGIReader", "ReadFile",
            "SGI reader vftable slot 2",
            "vtslot:" + SGI_READER_VTABLE + ":" + SLOT_READ_FILE),
        new Entry("006d5040", "NiSGIReader", "ReadHeader",
            "SGI reader vftable slot 3",
            "vtslot:" + SGI_READER_VTABLE + ":" + SLOT_READ_HEADER),
        new Entry("006d64e0", "NiTGAReader", "CanReadFile",
            "TGA reader vftable slot 1: stricmp of the extension with \".tga\" or \".targa\"",
            "vtslot:" + TGA_READER_VTABLE + ":" + SLOT_CAN_READ_FILE, "calls:" + TARGA_EXTENSION_STRING),
        new Entry("006d6520", "NiTGAReader", "ReadFile",
            "TGA reader vftable slot 2: ReadHeader (no magic; image type 1/2/3/9/10/11 else fails), palette, then exactly height rows; bytes after the pixels (footer, padding) are never read; a 0x0 header gives a 1x1 NiPixelData whose texel is not written",
            "vtslot:" + TGA_READER_VTABLE + ":" + SLOT_READ_FILE, "calls:0047c760"),
        new Entry("0068da60", "NiTexture::RendererData", "DefaultTextureFailureCallback",
            "Gamebryo default texture failure callback (UO never sets another): a cached two-colour replacement image per failure code; FILENOTFOUND (0x80000005) is magenta",
            "calls:0068ded0", "calledby:00770b90"),
        new Entry("0067b1d0", "NiD3DRenderer", "Error",
            "NiD3DRenderer::Error(format, ...): an empty function (RET) in this build, so the texture failure messages of NiDX9SourceTextureData::Create are not logged",
            "calledby:00770b90"),
        new Entry("00aecc3d", GLOBAL, "LoadScratchTextureD3DX",
            "D3DXGetImageInfoFromFileInMemory, then D3DXCreateTextureFromFileInMemoryEx into D3DPOOL_SCRATCH; no code or data reference (content sniffing that UO never uses)",
            "calls:00aecb87"),

        // UO texture requests.
        new Entry("0058d8e0", "TextureManager", "RequestTexture",
            "TextureManager vftable slot 1 (location, out): the location's slot 2 (IsDirectLoad) picks LoadTextureDirect or RequestTextureAsset",
            "vtslot:" + TEXTURE_MANAGER_VTABLE + ":" + SLOT_REQUEST_TEXTURE, "calls:0058d600", "calls:0058d430"),
        new Entry("0058d600", "TextureManager", "LoadTextureDirect",
            "cache key FNV-1a of the lowercased location path with type 0x11000000; on a miss UOP::FileExists(path) and UOTextureData(path): the location's own path, extension included, is the file loaded",
            "calledby:0058d8e0", "calls:004ccc80", "calls:00989fdb",
            "const:" + Long.toHexString(TEXTURE_CACHE_KEY_TYPE)),
        new Entry("0058d430", "TextureManager", "RequestTextureAsset",
            "UOResourceRequest {location slot 3 (asset id), location slot 4 (resource type) << 24} to the asset manager; the assetmap packed pattern of the type gives the file name (Build/WorldArt/, Build/TerrainTexture/, Build/TileArtLegacy/, Build/TileArtEnhanced/ + %08d + .dds) and UOTextureFactory::Create loads it",
            "calledby:0058d8e0", "calls:00a6ced4", "vftable:UOResourceRequest"),
        new Entry("0058d270", "SpriteTextureFileLocation", "IsDirectLoad",
            "Sprite/WorldArt location vftable slot 2: true when the type is not packed (g_UOAssetManager+0x160+type*4 != 1) or the path holds WorldArt\\ref or none of WorldArt, Textures\\, TileArtLegacy, TileArtEnhanced",
            "slot:SpriteTextureFileLocation:" + SLOT_IS_DIRECT_LOAD, "string:TileArtLegacy",
            "const:" + Long.toHexString(ASSET_FORMAT_ARRAY_OFFSET)),
        new Entry("0058d310", "SpriteTextureFileLocation", "GetAssetId",
            "Sprite/WorldArt location vftable slot 3: atol of the file name part (\"3.tga\" -> 3, \"00000002_ankh.tga\" -> 2)",
            "slot:SpriteTextureFileLocation:" + SLOT_GET_ASSET_ID, "calls:0058dbf0"),
        new Entry("004ccc80", "UOTextureData", "UOTextureData",
            "(EAX = path) stores UOTextureData::vftable; UOP::FileExists(path) (logs when missing), then NiSourceTexture::Create(path, g_UOTextureLoadDirectHint = false), so the Gamebryo image converter decodes it",
            "vftable:UOTextureData", "calls:00989fdb", "calls:0068a8b0"),
        new Entry("004cce20", "UOTextureFactory", "Create",
            "UOTextureFactory vftable slot 0 (out, id, type, resolved path): UOTextureData(path); on failure logs the path TryResolvers gives",
            "slot:UOTextureFactory:" + SLOT_FACTORY_CREATE, "calls:004ccc80", "calls:00a749ed"),

        // Asset loader configuration: per resource type, packed (asset resolver) or raw (path as given).
        new Entry("00a6db3b", "UOAssets", "SetDefaultPackedTypes",
            "marks types 1, 3, 6, 8, 9, 0xb, 0xc, 0xd, 0x13, 0x14, 0x15, 0x16, 0x17 and 0x1a packed (g_UOAssetManager+0x160+type*4 = 1); called by UOAssets::Initialize before AssetLoaderConfig.xml, which no install ships",
            "calledby:004a9150", "const:" + Long.toHexString(LAST_DEFAULT_PACKED_OFFSET)),
        new Entry("00a6db93", "UOAssets", "LoadAssetLoaderConfig",
            "reads AssetLoaderConfig.xml <config><client><loader resourcetype=\"N|all\" format=\"raw|packed\"/>; absent from every shipped package, so the defaults stay",
            "string:" + ASSET_LOADER_CONFIG, "calls:00a6dadb", "calledby:004a9150"),
        new Entry("00a6dadb", "UOAssets", "SetTypeFormat",
            "(type, format 0 raw / 1 packed): stores 1 when the type has a mapping of that format (packed-only types 0x10, 0x12, 0x19 stay packed)",
            "calls:00a6da73", "calls:00a6da8a", "const:" + Long.toHexString(ASSET_FORMAT_ARRAY_OFFSET)),
        new Entry("00a6db20", "UOAssets", "SetAllTypesFormat",
            "SetTypeFormat for types 1..0x1a (loader resourcetype=\"all\")",
            "calls:00a6dadb", "calledby:00a6db93"),
        new Entry("00a6da73", "UOAssets", "HasRawMapping",
            "false for the packed-only types 0x10 (AnimationFrame) and 0x12 (Paperdoll)",
            "calledby:00a6dadb"),
        new Entry("00a6da8a", "UOAssets", "HasPackedMapping",
            "true for the types with a packed assetmap pattern (1, 3, 6, 8, 9, 0xb, 0xc, 0xd, 0x12..0x17, 0x19)",
            "calledby:00a6dadb"),
        new Entry("00a74868", "UOAssetMap", "Load",
            "reads Data/assetmap.xml (MainMisc.uop) and builds a mapping per patternmap / bitmaskmap / simplemap node; \"Node ... ignored - no valid mapping found.\"",
            "string:" + ASSET_MAP, "calls:00a6d5e4"),
        new Entry("00a74200", "UOAssetMap", "ParsePatternMap",
            "<patternmap resourcetype fileFormat directory prefix suffix padding patternMatch>: file name = directory + prefix + id padded to `padding` digits + suffix",
            "string:patternMatch", "string:suffix", "string:padding"),
        new Entry("00a74549", "UOAssetMap", "ParseBitmaskMap",
            "<bitmaskmap resourcetype fileFormat directory suffix> with <section prefix mask padding isDirectory patternMatch> children",
            "string:section", "string:isDirectory"),
        new Entry("00a73f5e", "UOAssetMap", "ParseSimpleMap",
            "<simplemap resourcetype resourcenumber fileFormat filename>: one fixed file name",
            "string:resourcenumber", "string:filename"),

        // UI texture locations.
        new Entry("00519700", GLOBAL, "MakeGumpTextureLocation",
            "gump id -> \"Data/Interface/Default/Textures/GumpArt/\" + \"%08d.tga\": the .tga name picks NiTGAReader",
            "string:" + GUMP_ART_DIRECTORY, "string:%08d.tga"),
        new Entry("00519570", GLOBAL, "MakeManagedTextureLocation",
            "ManagedTextures.xml file -> \"Data/Interface/InterfaceCore/Textures/\" + file name as written (its extension picks the reader); \"missingFile_ID\" otherwise",
            "string:" + MANAGED_TEXTURE_DIRECTORY, "string:missingFile_ID"),
        new Entry("00911470", GLOBAL, "UITexture_CreateSourceTexture",
            "UI <Texture file=...> path: NiSourceTexture::Create(file name as written, g_UOTextureLoadDirectHint)",
            "calls:0068a8b0"),
    };

    private final Map<String, Integer> changes = new LinkedHashMap<>();

    @Override
    public void run() throws Exception {
        String[] args = getScriptArgs();
        if (args.length < 1 || args.length > 2 || args.length == 2 && !args[1].equals(APPLY_FLAG)) {
            throw new IllegalArgumentException("Expected OUTPUT_FILE [" + APPLY_FLAG + "]");
        }
        String sha = currentProgram.getExecutableSHA256();
        if (sha == null || !sha.equalsIgnoreCase(TESTED_SHA256)) {
            throw new IllegalStateException("NameImageDecoderCode names fixed addresses of the tested UOSA.exe (SHA-256 "
                + TESTED_SHA256 + "); " + currentProgram.getName() + " has SHA-256 " + sha + ". Nothing done.");
        }
        boolean apply = args.length == 2;
        String outputPath = args[0].replace(ENCODED_SPACE, " ");
        // Created before any change, so an existing output file stops the run unchanged.
        try (PrintWriter output = new PrintWriter(Files.newBufferedWriter(Path.of(outputPath),
                StandardOpenOption.CREATE_NEW))) {
            if (apply) {
                ensureTag();
            }
            List<String> rows = new ArrayList<>();
            for (Entry entry : ENTRIES) {
                rows.add(process(entry, apply));
            }
            output.println("# Image decoder names");
            output.println();
            output.println("Program: `" + currentProgram.getName() + "`, SHA-256 `" + sha + "`.");
            output.println();
            output.println("Mode: " + (apply ? "`--apply` (changes: " + (changes.isEmpty() ? "none" : changes) + ")"
                : "report only") + ".");
            output.println();
            output.println("| Address | Name | Checks | Result |");
            output.println("|---|---|---|---|");
            rows.forEach(output::println);
        }
        println("Wrote " + outputPath + (apply ? " (changes: " + changes + ")" : ""));
    }

    private String process(Entry entry, boolean apply) throws Exception {
        Address address = toAddr(Long.parseLong(entry.address(), HEX_RADIX));
        String qualified = (entry.namespace().isEmpty() ? "" : entry.namespace() + Namespace.DELIMITER) + entry.name();
        String row = "| `0x" + entry.address() + "` | `" + qualified + "` | " + checkList(entry) + " | ";
        List<String> result = new ArrayList<>();
        Function function = getFunctionAt(address);
        if (function == null) {
            if (!heldByTableSlot(entry, address)) {
                return row + "mismatch: no function at the address |";
            }
            if (getFunctionContaining(address) != null) {
                return row + "mismatch: the address is inside " + getFunctionContaining(address).getName(true) + " |";
            }
            if (!apply) {
                return row + "would create the function, then check and name it |";
            }
            disassemble(address);
            function = createFunction(address, null);
            if (function == null) {
                return row + "mismatch: no function could be created |";
            }
            count("functions created");
            result.add("function created");
        }
        List<String> failed = new ArrayList<>();
        for (String check : entry.checks()) {
            if (!check(function, check)) {
                failed.add(check);
            }
        }
        if (!failed.isEmpty()) {
            return row + "mismatch: " + String.join(", ", failed) + " failed; not named |";
        }
        Symbol symbol = function.getSymbol();
        boolean named = symbol.getName(true).equals(qualified);
        if (!named) {
            if (symbol.getSource() != SourceType.DEFAULT) {
                return row + "kept `" + symbol.getName(true) + "` (" + symbol.getSource() + ") |";
            }
            Namespace namespace = namespaceOrNull(entry.namespace());
            if (namespace != null && !currentProgram.getSymbolTable().getSymbols(entry.name(), namespace).isEmpty()) {
                return row + "conflict: `" + qualified + "` already names another symbol |";
            }
            if (apply) {
                if (namespace == null) {
                    namespace = createNamespace(entry.namespace());
                }
                symbol.setNameAndNamespace(entry.name(), namespace, SourceType.ANALYSIS);
                count("functions renamed");
            }
            result.add(apply ? "named" : "would name");
        }
        boolean tagged = false;
        for (FunctionTag tag : function.getTags()) {
            tagged |= tag.getName().equals(TAG);
        }
        if (!tagged) {
            if (apply) {
                function.addTag(TAG);
                count("tags added");
            }
            result.add(apply ? "tagged" : "would tag");
        }
        String plate = getPlateComment(address);
        String comment = COMMENT_PREFIX + TAG + ": " + entry.evidence();
        if (plate == null || plate.isBlank()) {
            if (apply) {
                setPlateComment(address, comment);
                count("plate comments set");
            }
            result.add(apply ? "comment set" : "would set comment");
        } else if (plate.startsWith(COMMENT_PREFIX) && !plate.equals(comment)) {
            // Our own comment from an earlier run with older evidence.
            if (apply) {
                setPlateComment(address, comment);
                count("plate comments refreshed");
            }
            result.add(apply ? "comment refreshed" : "would refresh comment");
        } else if (!plate.contains(entry.evidence())) {
            result.add("plate comment from another source left alone");
        }
        return row + (result.isEmpty() ? "already applied" : String.join(", ", result)) + " |";
    }

    // Checks as one table cell; a newline in a string check is shown escaped.
    private static String checkList(Entry entry) {
        return String.join(", ", entry.checks()).replace("\n", "\\n");
    }

    // A "vtslot" check of the entry holds the address, so a function may be created there.
    private boolean heldByTableSlot(Entry entry, Address address) {
        for (String check : entry.checks()) {
            if (check.startsWith("vtslot:") && check(null, check, address)) {
                return true;
            }
        }
        return false;
    }

    private boolean check(Function function, String check) {
        return check(function, check, function.getEntryPoint());
    }

    private boolean check(Function function, String check, Address entryPoint) {
        String[] parts = check.split(":", 2);
        String value = parts[1];
        return switch (parts[0]) {
            case "calls" -> references(function, toAddr(Long.parseLong(value, HEX_RADIX)));
            case "calledby" -> {
                Function caller = getFunctionAt(toAddr(Long.parseLong(value, HEX_RADIX)));
                yield caller != null && references(caller, entryPoint);
            }
            case "slot" -> {
                int split = value.lastIndexOf(':');
                boolean found = false;
                for (Address vftable : vftables(value.substring(0, split))) {
                    found |= tableSlotHolds(vftable, Integer.parseInt(value.substring(split + 1)), entryPoint);
                }
                yield found;
            }
            case "vtslot" -> {
                int split = value.lastIndexOf(':');
                yield tableSlotHolds(toAddr(Long.parseLong(value.substring(0, split), HEX_RADIX)),
                    Integer.parseInt(value.substring(split + 1)), entryPoint);
            }
            case "vftable" -> {
                boolean found = false;
                for (Address vftable : vftables(value)) {
                    found |= references(function, vftable);
                }
                yield found;
            }
            case "string" -> referencesString(function, value);
            case "const" -> hasConstant(function, Long.parseLong(value, HEX_RADIX));
            default -> throw new IllegalArgumentException("Unknown check " + check);
        };
    }

    private boolean references(Function function, Address target) {
        for (Address from : function.getBody().getAddresses(true)) {
            for (Reference reference : getReferencesFrom(from)) {
                if (reference.getToAddress().equals(target)) {
                    return true;
                }
            }
        }
        return false;
    }

    private boolean referencesString(Function function, String text) {
        for (Address from : function.getBody().getAddresses(true)) {
            for (Reference reference : getReferencesFrom(from)) {
                Data data = getDataAt(reference.getToAddress());
                if (data != null && StringDataInstance.isString(data)
                        && text.equals(StringDataInstance.getStringDataInstance(data).getStringValue())) {
                    return true;
                }
            }
        }
        return false;
    }

    private boolean hasConstant(Function function, long value) {
        for (Instruction instruction : currentProgram.getListing().getInstructions(function.getBody(), true)) {
            for (int i = 0; i < instruction.getNumOperands(); i++) {
                for (Object object : instruction.getOpObjects(i)) {
                    if (object instanceof Scalar scalar && scalar.getUnsignedValue() == value) {
                        return true;
                    }
                }
            }
        }
        return false;
    }

    private boolean tableSlotHolds(Address table, int index, Address target) {
        try {
            long pointer = getInt(table.add((long) index * POINTER_SIZE)) & UNSIGNED_INT_MASK;
            return pointer == target.getOffset();
        } catch (Exception unreadable) {
            return false;
        }
    }

    private List<Address> vftables(String className) {
        List<Address> found = new ArrayList<>();
        SymbolTable table = currentProgram.getSymbolTable();
        for (Symbol symbol : table.getSymbols(VFTABLE)) {
            if (symbol.getName(true).equals(className + Namespace.DELIMITER + VFTABLE)) {
                found.add(symbol.getAddress());
            }
        }
        return found;
    }

    private Namespace namespaceOrNull(String path) {
        Namespace namespace = currentProgram.getGlobalNamespace();
        if (path.isEmpty()) {
            return namespace;
        }
        for (String part : path.split(Namespace.DELIMITER)) {
            namespace = currentProgram.getSymbolTable().getNamespace(part, namespace);
            if (namespace == null) {
                return null;
            }
        }
        return namespace;
    }

    private Namespace createNamespace(String path) throws Exception {
        Namespace namespace = currentProgram.getGlobalNamespace();
        for (String part : path.split(Namespace.DELIMITER)) {
            Namespace child = currentProgram.getSymbolTable().getNamespace(part, namespace);
            if (child == null) {
                child = currentProgram.getSymbolTable().createNameSpace(namespace, part, SourceType.ANALYSIS);
                count("namespaces created");
            }
            namespace = child;
        }
        return namespace;
    }

    private void ensureTag() {
        if (currentProgram.getFunctionManager().getFunctionTagManager().getFunctionTag(TAG) == null) {
            currentProgram.getFunctionManager().getFunctionTagManager().createFunctionTag(TAG, TAG_COMMENT);
            count("tag created");
        }
    }

    private void count(String what) {
        changes.merge(what, 1, Integer::sum);
    }
}
