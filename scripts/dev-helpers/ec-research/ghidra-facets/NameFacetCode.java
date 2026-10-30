// Name the UOSA.exe code that reads facet sectors (facetN.uop, asset type 9), FacetDefinition records
// (facets.uop, asset type 23) and the facetNN.dds radar images, and the radar code that uses them.
// Address-based, so it refuses any other build; every address is verified with stated structural
// checks first and a failed check is reported, not named. Findings: results/uosa_facets.md.
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

public class NameFacetCode extends GhidraScript {
    private static final String APPLY_FLAG = "--apply";
    // Script arguments are split on whitespace, so a path with spaces is passed with %20.
    private static final String ENCODED_SPACE = "%20";
    // SHA-256 of the UOSA.exe these addresses were traced in (same build as ghidra-uop-header).
    private static final String TESTED_SHA256 = "5a5e9dc536c836d1bf10b6f86bb88f61a6ae078f519d4060d3bc47aecbebad50";
    private static final String TAG = "UO_FACET";
    private static final String FACET = "UOFacet";
    private static final String COMMENT_PREFIX = "NameFacetCode: ";
    private static final String VFTABLE = "vftable";
    private static final int POINTER_SIZE = 4;
    private static final long UNSIGNED_INT_MASK = 0xffffffffL;
    private static final int HEX_RADIX = 16;
    // Asset type keys (type << 24): TerrainDefinition 3, Sectors 9, FacetDefinition 23 (assetmap.xml).
    private static final long TERRAIN_DEFINITION_KEY = 0x03000000L;
    private static final long SECTOR_KEY = 0x09000000L;
    private static final long FACET_DEFINITION_KEY = 0x17000000L;
    // Squares per sector side; the sector reader loops 64 x 64.
    private static final long SECTOR_SIDE = 0x40;
    // Hard-coded facet table: 7 records {char *name, u32 width, u32 height}; code reads the width (+4) and
    // height (+8) fields only.
    private static final String FACET_TABLE = "00da72a8";
    private static final String FACET_TABLE_WIDTHS = "00da72ac";
    private static final String FACET_TABLE_HEIGHTS = "00da72b0";
    private static final String FIRST_FACET_NAME = "Felucca";
    private static final long FIRST_FACET_WIDTH = 7168;
    private static final long FIRST_FACET_HEIGHT = 4096;
    private static final int TABLE_WIDTH_OFFSET = 4;
    private static final int TABLE_HEIGHT_OFFSET = 8;

    // Checks: "calls:ADDR" (the function references ADDR: call, pointer, jump or data),
    // "calledby:ADDR" (the function at ADDR references this one), "slot:CLASS:N" (vftable slot N
    // of CLASS holds this function), "vftable:CLASS" (references CLASS::vftable),
    // "string:TEXT" (references the string TEXT), "const:HEX" (an operand equals HEX),
    // "refs:N" (exactly N references to the function, so a vftable getter is not shared with other classes).
    private record Entry(String address, String namespace, String name, String evidence, String... checks) {
    }

    private static final String FACET_DEFINITION = "UOFacetDefinition";
    private static final String FACET_DEFINITION_FACTORY = "UOFacetDefinitionBinaryFactory";
    private static final String SECTOR_FACTORY = "UOSectorInstanceBinaryFactory";
    private static final String SECTOR_INSTANCE = "UOSectorClientInstance";

    private static final Entry[] ENTRIES = {
        // FacetDefinition (facets.uop, build/facetdefinition/%08d.bin).
        new Entry("0083c82f", FACET, "FacetDefinitionFactory_Create",
            "UOFacetDefinitionBinaryFactory slot 1: FacetDefinition_Read of the asset blob; on failure TryResolvers",
            "slot:" + FACET_DEFINITION_FACTORY + ":1", "calls:0083c933", "calls:00a749ed"),
        new Entry("0083c933", FACET, "FacetDefinition_Read",
            "StreamRead u8 facet id (+4), u32 name cliloc (+8), u32 width (+0xc), u32 height (+0x10), three u8 (+0x14..+0x16, 0x40 0x40 0x01 in all records; their getters have no caller), then FacetDefinition_ReadRegions",
            "calledby:0083c82f", "calls:0083c7a7", "calls:0083ca5f", "calls:00a6fe55"),
        new Entry("0083c7a7", FACET, "FacetDefinition_ctor",
            "allocates the UOFacetDefinition object (vftable 0x00cdeb14, region vector +0x1c)",
            "calledby:0083c933"),
        new Entry("0083ca5f", FACET, "FacetDefinition_ReadRegions",
            "u32 region count; per region u32 index (stored at region +0, no getter), u32 cliloc, u8 r, u8 g, u8 b, u8 physical facet (0xff none), then FacetDefinition_ReadRegionRects",
            "calledby:0083c933", "calls:0083cb53", "calls:00846a0d"),
        new Entry("0083cb53", FACET, "FacetDefinition_ReadRegionRects",
            "u32 rect count, then rects of 4 x u32 (x1, y1, x2, y2; inclusive)",
            "calledby:0083ca5f", "calls:00846bc6"),
        new Entry("00846a0d", FACET, "FacetDefinition_AddRegion",
            "appends a 0x1c-byte region {u32 index, u32 cliloc, u8 rgb[3], u8 physical facet, rect vector +0x10}",
            "calledby:0083ca5f", "calls:00846936"),
        new Entry("00846936", FACET, "FacetRegion_ctor",
            "region record constructor",
            "calledby:00846a0d"),
        new Entry("00846bc6", FACET, "FacetDefinition_AddRegionRect",
            "appends a 16-byte rect to the last region",
            "calledby:0083cb53"),
        new Entry("008469cf", FACET, "FacetDefinition_GetFacetId",
            "UOFacetDefinition slot 3: u8 +4",
            "slot:" + FACET_DEFINITION + ":3", "refs:1"),
        new Entry("008469d7", FACET, "FacetDefinition_GetNameCliloc",
            "UOFacetDefinition slot 4: u32 +8 (UORadarGetFacetLabel)",
            "slot:" + FACET_DEFINITION + ":4", "refs:1"),
        new Entry("008469e8", FACET, "FacetDefinition_GetWidth",
            "UOFacetDefinition slot 5: u32 +0xc (UORadarGetFacetDimensions, GetFacetDimensions)",
            "slot:" + FACET_DEFINITION + ":5", "refs:1"),
        new Entry("008469ec", FACET, "FacetDefinition_GetHeight",
            "UOFacetDefinition slot 6: u32 +0x10",
            "slot:" + FACET_DEFINITION + ":6", "refs:1"),
        new Entry("008469fd", FACET, "FacetDefinition_GetHeaderByte0",
            "UOFacetDefinition slot 7: u8 +0x14 (0x40 in every record); no caller found",
            "slot:" + FACET_DEFINITION + ":7", "refs:1"),
        new Entry("00846a01", FACET, "FacetDefinition_GetHeaderByte1",
            "UOFacetDefinition slot 8: u8 +0x15 (0x40 in every record); no caller found",
            "slot:" + FACET_DEFINITION + ":8", "refs:1"),
        new Entry("00846a09", FACET, "FacetDefinition_GetHeaderByte2",
            "UOFacetDefinition slot 9: u8 +0x16 (0x01 in every record); no caller found",
            "slot:" + FACET_DEFINITION + ":9", "refs:1"),
        new Entry("00846a5d", FACET, "FacetDefinition_GetRegionCount",
            "UOFacetDefinition slot 10: region count",
            "slot:" + FACET_DEFINITION + ":10", "refs:1"),
        new Entry("00846a65", FACET, "FacetDefinition_GetRegionCliloc",
            "UOFacetDefinition slot 11: region(i) +4 (UORadarGetAreaLabel)",
            "slot:" + FACET_DEFINITION + ":11", "refs:1"),
        new Entry("00846a8a", FACET, "FacetDefinition_GetRegionColor",
            "UOFacetDefinition slot 12: region(i) rgb +8..+0xa (radar background colour)",
            "slot:" + FACET_DEFINITION + ":12", "refs:1"),
        new Entry("00846ad7", FACET, "FacetDefinition_GetRegionPhysicalFacet",
            "UOFacetDefinition slot 13: region(i) u8 +0xb, the facet whose coordinates the rects use (0xff: this facet)",
            "slot:" + FACET_DEFINITION + ":13", "refs:1"),
        new Entry("00846afc", FACET, "FacetDefinition_GetRegionBounds",
            "UOFacetDefinition slot 14: min x1/y1 and max x2/y2 over the rects of region(i)",
            "slot:" + FACET_DEFINITION + ":14", "refs:1"),
        new Entry("00846c20", FACET, "FacetDefinition_GetRegionRectCount",
            "UOFacetDefinition slot 15: rect count of region(i)",
            "slot:" + FACET_DEFINITION + ":15", "refs:1"),
        new Entry("00846c58", FACET, "FacetDefinition_GetRegionRect",
            "UOFacetDefinition slot 16: rect j of region(i)",
            "slot:" + FACET_DEFINITION + ":16", "refs:1"),
        new Entry("005a5290", FACET, "GetFacetDefinition",
            "(out, facet id): asset (facet id, 0x17000000) through a UOResourceRequest and g_UOAssetManager; the only FacetDefinition accessor (14 callers, all radar/map code)",
            "calls:004cb9a0", "calls:00a6ced4", "calledby:005dfe80"),
        new Entry("004cb9a0", FACET, "ResourceRequest_AddFacetDefinitionKey",
            "pushes the asset key {id, 0x17000000} (FacetDefinition) onto a request key vector",
            "const:" + Long.toHexString(FACET_DEFINITION_KEY), "calledby:005a5290"),
        new Entry("005dfe80", FACET, "GetFacetDimensions",
            "width and height of a facet from its FacetDefinition (slots 5, 6)",
            "calls:005a5290", "calledby:00534d90"),
        // Sectors (facetN.uop, build/sectors/facet_%02d/%08d.bin).
        new Entry("0083d094", FACET, "SectorFactory_Create",
            "UOSectorInstanceBinaryFactory slot 1: Sector_Read of the asset blob; on failure (ok false) TryResolvers",
            "slot:" + SECTOR_FACTORY + ":1", "calls:0083d19a", "calls:00a749ed"),
        new Entry("0083d19a", FACET, "Sector_Read",
            "StreamRead u8 facet, u16 sector id, SectorClientInstance_ctor, then Sector_ReadSquare for x 0..63 (outer), y 0..63 (inner); any read failure returns null; bytes left over set ok false",
            "calledby:0083d094", "calls:0083cf4a", "calls:0083d29f", "const:" + Long.toHexString(SECTOR_SIDE)),
        new Entry("0083d29f", FACET, "Sector_ReadSquare",
            "u8 z, u16 land id, SectorClientInstance_AddSquare, u8 delimiter count + Sector_ReadDelimiter each, u8 static count + Sector_ReadStatic each",
            "calledby:0083d19a", "calls:00847c59", "calls:0083d3a0", "calls:0083d47d"),
        new Entry("0083d3a0", FACET, "Sector_ReadDelimiter",
            "u8 direction (> 7 sets ok false: the sector is rejected), i8 z, u32 land id; direction -> (dx, dy): 0 (-1,0) 1 (-1,-1) 2 (0,-1) 3 (1,0) 4 (1,1) 5 (0,1) 6 (-1,1) 7 (1,-1)",
            "calledby:0083d29f", "calls:008480ca"),
        new Entry("0083d47d", FACET, "Sector_ReadStatic",
            "u32 graphic, u8 z, u32 hue, added to the square",
            "calledby:0083d29f", "calls:00847ebf"),
        new Entry("0083cf4a", FACET, "SectorClientInstance_ctor",
            "stores UOSectorClientInstance::vftable, facet (+4) and sector id (+8)",
            "vftable:" + SECTOR_INSTANCE, "calledby:0083d19a"),
        new Entry("00847c59", FACET, "SectorClientInstance_AddSquare",
            "allocates a 0x48-byte square, stores the land id, inserts it into the (x, y) map",
            "calledby:0083d29f"),
        new Entry("00847c21", FACET, "SectorClientInstance_GetSquare",
            "UOSectorClientInstance slot 7: square (x, y) from the map, null when absent",
            "slot:" + SECTOR_INSTANCE + ":7", "refs:1"),
        new Entry("00847d24", FACET, "SectorClientInstance_GetNeighbourZAndLand",
            "UOSectorClientInstance slot 4: z and land id of (x, y); outside 0..63 the border square's delimiter in that direction",
            "slot:" + SECTOR_INSTANCE + ":4", "refs:1"),
        new Entry("005df3b0", FACET, "RequestSector",
            "GetAsset(sector << 6 | facet & 0x3f, 0x09000000); the only sector asset request",
            "const:" + Long.toHexString(SECTOR_KEY), "calls:00a6cfc8", "calledby:005e2f00"),
        new Entry("005df210", FACET, "SectorIndexFromXY",
            "sector = ceil(height / 64) * (x >> 6) + (y >> 6) with the height from the facet table (column-major), and the 32-aligned chunk offset in the sector",
            "calls:" + FACET_TABLE_HEIGHTS, "calledby:005e2f00"),
        new Entry("005dfc40", FACET, "QueueSectorChunks",
            "sector index -> origin x = (index / rows) * 64, y = (index % rows) * 64; queues its 2 x 2 chunks of 32",
            "calls:" + FACET_TABLE_HEIGHTS),
        new Entry("005e2f00", FACET, "BuildChunkFromSector",
            "32 x 32 chunk: RequestSector, rejects a null sector or one whose id or facet differs from the request (chunk not built), GetSquare per cell (null = empty), TerrainDefinition (0x03000000) per land id, neighbours via slot 4",
            "calls:005df3b0", "calls:005df210", "const:" + Long.toHexString(TERRAIN_DEFINITION_KEY)),
        // Package variants: MountPackages mounts facet0..6.uop; this swaps in facet0x/1x/2x/5x.uop.
        new Entry("00a6c9fc", FACET, "SwapFacetPackageVariants",
            "flag set: unmount facetN.uop and mount facetNx.uop for N in 0, 1, 2, 5; flag clear: the reverse",
            "string:facet0x.uop", "string:facet5.uop", "calls:0098a3fe", "calledby:00615730"),
        new Entry("00615730", FACET, "ApplyFacetVariantFlag",
            "0xA9 character-list flags bit 15 (0x8000) toggles SwapFacetPackageVariants when it differs from the mounted state (+0xab)",
            "calls:00a6c9fc", "calledby:00612a80"),
        // Radar: facetNN.dds and the live overlay.
        new Entry("00534b60", FACET, "FacetDds_Open",
            "memory-maps <base dir>facet%02d.dds (\"%s%s%02d%s\", \"facet\", \".dds\") into a cache keyed by facet",
            "calledby:00534d10", "calledby:00534d90"),
        new Entry("00534d10", FACET, "FacetDds_GetMipCount",
            "DDS header +0x1c (mipMapCount) of facet%02d.dds; 0 when the file is missing",
            "calls:00534b60", "calledby:005118c0"),
        new Entry("00534d90", FACET, "FacetDds_CopyRegionBlocks",
            "fills the radar texture with the region colour as 8-byte DXT1 blocks, then copies the DXT1 blocks of the chosen mip (data at +0x80) inside the region rects",
            "calls:00534b60", "calls:00487410", "calls:005dfe80", "calls:00535b30", "calls:00535ca0", "calls:00535d10"),
        new Entry("00535960", FACET, "Radar_FindAreaAtLocation",
            "region index of a location: scans the current facet's regions, then the other facets for regions whose physical facet is the current one",
            "calls:005a5290", "calledby:00510b40"),
        new Entry("00535b30", FACET, "Radar_GetAreaColor",
            "FacetDefinition region colour (slot 12)",
            "calls:005a5290", "calledby:00534d90"),
        new Entry("00535ca0", FACET, "Radar_GetAreaRectCount",
            "FacetDefinition region rect count (slot 15)",
            "calls:005a5290", "calledby:00534d90"),
        new Entry("00535d10", FACET, "Radar_GetAreaRect",
            "FacetDefinition region rect (slot 16)",
            "calls:005a5290", "calledby:00534d90"),
        new Entry("00535ba0", FACET, "Radar_IsLocationInArea",
            "inclusive rect test x1 <= x <= x2, y1 <= y <= y2 over the region rects",
            "calls:005a5290", "calledby:00513ea0"),
        new Entry("00511bf0", FACET, "Radar_GetMaxZoomForArea",
            "zoom limit from the region bounds (slot 14)",
            "calls:005a5290", "calledby:005140e0"),
        new Entry("00510b40", FACET, "Radar_SetWindowSize",
            "creates the radar_texture NiPixelData (sizes rounded up to a power of two), finds the area and sets the view",
            "calls:005a5290", "calls:00535960", "calledby:00512340"),
        new Entry("00510e60", FACET, "Radar_SetView",
            "sets facet, area and centre; a region physical facet other than 0xff switches the radar facet",
            "calls:005a5290", "calls:005118c0", "calledby:00512940"),
        new Entry("005118c0", FACET, "Radar_Update",
            "zoom -> mip level (clamped below the DDS mip count), rebuilds the texture when the view changed, then the static overlay",
            "calls:00534d10", "calls:00511b10", "calls:00535310"),
        new Entry("00511b10", FACET, "Radar_RebuildTexture",
            "FacetDds_CopyRegionBlocks for the current facet, area, mip and window",
            "calls:00534d90", "calledby:005118c0"),
        new Entry("00535310", FACET, "Radar_OverlayStaticsInRect",
            "for each 4 x 4 block of the dirty rect not yet patched: Radar_RecolorBlock (statics near the player only within 24 tiles)",
            "calls:00535500", "calledby:005118c0"),
        new Entry("00535500", FACET, "Radar_RecolorBlock",
            "decodes the DXT1 block, sets pixels from collected object colours and from statics (not NoDraw or HoverOver; Wall or Roof above, Surface at or above the highest so far, from z -128) using the tileart radar colour, re-encodes",
            "calls:00487970", "calls:00487410", "calls:005bda40", "calledby:00535310"),
        new Entry("005110c0", FACET, "Radar_CollectObjectColors",
            "per tile the tileart radar colour of the highest object seen, and the dirty rect",
            "calls:005bda40", "calledby:00511380"),
        new Entry("00487970", FACET, "Dxt1_DecodeBlock",
            "8-byte DXT1 block to 16 RGBA pixels",
            "calledby:00535500"),
        new Entry("00487410", FACET, "Dxt1_EncodeBlock",
            "16 RGBA pixels to an 8-byte DXT1 block (min/max endpoints, nearest of 4 colours)",
            "calledby:00535500", "calledby:00534d90"),
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
            throw new IllegalStateException("NameFacetCode names fixed addresses of the tested UOSA.exe (SHA-256 "
                + TESTED_SHA256 + "); " + currentProgram.getName() + " has SHA-256 " + sha + ". Nothing done.");
        }
        boolean apply = args.length == 2;
        String outputPath = args[0].replace(ENCODED_SPACE, " ");
        // Created before any change, so an existing output file stops the run unchanged.
        try (PrintWriter output = new PrintWriter(Files.newBufferedWriter(Path.of(outputPath),
                StandardOpenOption.CREATE_NEW))) {
            List<String> rows = new ArrayList<>();
            rows.add(processTable(apply));
            for (Entry entry : ENTRIES) {
                rows.add(process(entry, apply));
            }
            output.println("# Facet names");
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

    // The facet dimension table: checked by its first record and the code reading its fields, then labelled.
    private String processTable(boolean apply) throws Exception {
        String name = "g_FacetDimensions";
        String qualified = FACET + Namespace.DELIMITER + name;
        String checks = "first record {\"" + FIRST_FACET_NAME + "\", " + FIRST_FACET_WIDTH + ", " + FIRST_FACET_HEIGHT
            + "}, width field read by 005dfef0, height field read by 005df210";
        String row = "| `0x" + FACET_TABLE + "` | `" + qualified + "` | " + checks + " | ";
        Address table = toAddr(Long.parseLong(FACET_TABLE, HEX_RADIX));
        Address namePointer = toAddr(getInt(table) & UNSIGNED_INT_MASK);
        Data nameData = getDataAt(namePointer);
        boolean recordOk = nameData != null && StringDataInstance.isString(nameData)
            && FIRST_FACET_NAME.equals(StringDataInstance.getStringDataInstance(nameData).getStringValue())
            && (getInt(table.add(TABLE_WIDTH_OFFSET)) & UNSIGNED_INT_MASK) == FIRST_FACET_WIDTH
            && (getInt(table.add(TABLE_HEIGHT_OFFSET)) & UNSIGNED_INT_MASK) == FIRST_FACET_HEIGHT;
        Function widthReader = getFunctionAt(toAddr(0x005dfef0L));
        Function heightReader = getFunctionAt(toAddr(0x005df210L));
        boolean readersOk = widthReader != null && heightReader != null
            && references(widthReader, toAddr(Long.parseLong(FACET_TABLE_WIDTHS, HEX_RADIX)))
            && references(heightReader, toAddr(Long.parseLong(FACET_TABLE_HEIGHTS, HEX_RADIX)));
        if (!recordOk || !readersOk) {
            return row + "mismatch: " + (recordOk ? "" : "first record ") + (readersOk ? "" : "readers ") + "failed; not named |";
        }
        List<String> result = new ArrayList<>();
        SymbolTable symbols = currentProgram.getSymbolTable();
        Symbol primary = symbols.getPrimarySymbol(table);
        boolean named = primary != null && primary.getName(true).equals(qualified);
        if (!named) {
            if (primary != null && primary.getSource() != SourceType.DEFAULT) {
                return row + "kept `" + primary.getName(true) + "` (" + primary.getSource() + ") |";
            }
            if (apply) {
                Namespace namespace = facetNamespace();
                symbols.createLabel(table, name, namespace, SourceType.ANALYSIS).setPrimary();
                count("labels created");
            }
            result.add(apply ? "named" : "would name");
        }
        String comment = COMMENT_PREFIX + TAG + ": 7 x {char *name, u32 width, u32 height}: Felucca and Trammel 7168 x 4096, Ilshenar 2304 x 1600, Malas 2560 x 2048, Tokuno 1448 x 1448, TerMur 1280 x 4096, Test 256 x 256";
        result.add(plate(table, comment, apply));
        return row + String.join(", ", result.stream().filter(s -> !s.isEmpty()).toList()).replaceFirst("^$", "already applied") + " |";
    }

    private Namespace facetNamespace() throws Exception {
        SymbolTable symbols = currentProgram.getSymbolTable();
        Namespace namespace = symbols.getNamespace(FACET, currentProgram.getGlobalNamespace());
        return namespace != null ? namespace
            : symbols.createNameSpace(currentProgram.getGlobalNamespace(), FACET, SourceType.ANALYSIS);
    }

    // Sets or refreshes our plate comment; "" when nothing is to do.
    private String plate(Address address, String comment, boolean apply) {
        String plate = getPlateComment(address);
        if (plate == null || plate.isBlank()) {
            if (apply) {
                setPlateComment(address, comment);
                count("plate comments set");
            }
            return apply ? "comment set" : "would set comment";
        }
        if (plate.startsWith(COMMENT_PREFIX) && !plate.equals(comment)) {
            // Our own comment from an earlier run with older evidence.
            if (apply) {
                setPlateComment(address, comment);
                count("plate comments refreshed");
            }
            return apply ? "comment refreshed" : "would refresh comment";
        }
        if (!plate.startsWith(COMMENT_PREFIX)) {
            return "plate comment from another source left alone";
        }
        return "";
    }

    private String process(Entry entry, boolean apply) throws Exception {
        Address address = toAddr(Long.parseLong(entry.address(), HEX_RADIX));
        String qualified = entry.namespace() + Namespace.DELIMITER + entry.name();
        String row = "| `0x" + entry.address() + "` | `" + qualified + "` | ";
        Function function = getFunctionAt(address);
        if (function == null) {
            return row + checkList(entry) + " | mismatch: no function at the address |";
        }
        List<String> failed = new ArrayList<>();
        for (String check : entry.checks()) {
            if (!check(function, check)) {
                failed.add(check);
            }
        }
        row += checkList(entry) + " | ";
        if (!failed.isEmpty()) {
            return row + "mismatch: " + String.join(", ", failed) + " failed; not named |";
        }
        List<String> result = new ArrayList<>();
        Symbol symbol = function.getSymbol();
        boolean named = symbol.getName(true).equals(qualified);
        if (!named) {
            if (symbol.getSource() != SourceType.DEFAULT) {
                return row + "kept `" + symbol.getName(true) + "` (" + symbol.getSource() + ") |";
            }
            Namespace namespace = currentProgram.getSymbolTable().getNamespace(entry.namespace(),
                currentProgram.getGlobalNamespace());
            if (namespace != null && !currentProgram.getSymbolTable().getSymbols(entry.name(), namespace).isEmpty()) {
                return row + "conflict: `" + qualified + "` already names another symbol |";
            }
            if (apply) {
                symbol.setNameAndNamespace(entry.name(), facetNamespace(), SourceType.ANALYSIS);
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
        String plateResult = plate(address, COMMENT_PREFIX + TAG + ": " + entry.evidence(), apply);
        if (!plateResult.isEmpty()) {
            result.add(plateResult);
        }
        return row + (result.isEmpty() ? "already applied" : String.join(", ", result)) + " |";
    }

    private static String checkList(Entry entry) {
        return String.join(", ", entry.checks());
    }

    private boolean check(Function function, String check) {
        String[] parts = check.split(":", 2);
        String value = parts[1];
        return switch (parts[0]) {
            case "calls" -> references(function, toAddr(Long.parseLong(value, HEX_RADIX)));
            case "calledby" -> {
                Function caller = getFunctionAt(toAddr(Long.parseLong(value, HEX_RADIX)));
                yield caller != null && references(caller, function.getEntryPoint());
            }
            case "slot" -> {
                int split = value.lastIndexOf(':');
                yield slot(value.substring(0, split), Integer.parseInt(value.substring(split + 1)),
                    function.getEntryPoint());
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
            case "refs" -> getReferencesTo(function.getEntryPoint()).length == Integer.parseInt(value);
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

    // Any vftable label of the class (one per base subobject) whose slot holds the target.
    private boolean slot(String className, int index, Address target) {
        for (Address vftable : vftables(className)) {
            try {
                long pointer = getInt(vftable.add((long) index * POINTER_SIZE)) & UNSIGNED_INT_MASK;
                if (pointer == target.getOffset()) {
                    return true;
                }
            } catch (Exception unreadable) {
                // Not a readable table; try the next label.
            }
        }
        return false;
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

    private void count(String what) {
        changes.merge(what, 1, Integer::sum);
    }
}
