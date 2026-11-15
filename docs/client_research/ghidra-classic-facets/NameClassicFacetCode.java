// Name the Classic client (client.exe) code around the facet table: the 48-byte facet records with their
// wrap rectangle, file ids and Felucca-rules flag, the second table copy selected by 0xA9 flag bit 15
// (map0x/1x/2x/5x), the 0xBF/0x18 map patch counts and the mapdifl/stadifl loaders, the dif lookups, and
// the code that applies the wrap rectangle (movement, distance, tile build, radar).
// Address-based: one entry set per tested build, selected by SHA-256, any other build is refused; every
// address is verified with stated structural checks first and a failed check is reported, not named.
// Findings: results/classic_facets.md.
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

public class NameClassicFacetCode extends GhidraScript {
    private static final String APPLY_FLAG = "--apply";
    // Script arguments are split on whitespace, so a path with spaces is passed with %20.
    private static final String ENCODED_SPACE = "%20";
    // SHA-256 of the tested client.exe builds (see README.md for the folders).
    private static final String CLASSIC_7_0_103_SHA256 = "4d857f2f5f8bf350bc50a87ee66b2236bfa480e712e9546c740ff04f9106690a";
    private static final String CLASSIC_7_0_20_SHA256 = "27eef2c0a4679f204b923f29dd1506bb2c7e649410054aefad680264bb504c9e";
    private static final String CLASSIC_6_0_14_3_SHA256 = "9104cea0ea14bd8ee81ee5aa111f1e3f39eca50ac33f818813202fadfdd3718d";
    private static final String GOLD_SHA256 = "aa21bb3ea16b6c5e00837569466b052e978baa6e38fd945e2694af4f14fa28c2";
    private static final String TAG = "UO_CLASSIC_FACET";
    private static final String FACET = "UOClassicFacet";
    private static final String COMMENT_PREFIX = "NameClassicFacetCode: ";
    private static final long UNSIGNED_INT_MASK = 0xffffffffL;
    private static final int HEX_RADIX = 16;
    // First record of every facet table.
    private static final String FIRST_FACET_NAME = "Felucca";
    // Returned by the distance functions when the two points are on opposite sides of the wrap width.
    private static final long NO_WRAP_DISTANCE = 9999;
    // Land graphic of the cells outside the wrap rectangle in the non-wrapping region.
    private static final long VOID_LAND_GRAPHIC = 0x244;
    // Radar colour index used for cells outside the map.
    private static final long RADAR_VOID_COLOR = 0x8001;

    private static final String STRING_NO_CHARACTERS = "You have no characters on %s!";
    private static final String STRING_MAPDIFL = "mapdifl%d.mul";
    private static final String STRING_STADIFL = "stadifl%d.mul";
    private static final String STRING_CRIMINAL = "This may flag you criminal!";

    private static final String RECORD = "48-byte record {char *name, u32 width, u32 height, u32 wrap_width, u32 wrap_height, u32 statics id, u32 staidx id, u32 mapdif id, u32 stadif id, u32 stadifi id, u32 map id, u32 felucca_rules}; ids index the client's file name table";

    // Checks: "calls:ADDR" (the function references ADDR: call, pointer, jump or data),
    // "calledby:ADDR" (the function at ADDR references this one), "string:TEXT" (references the string
    // TEXT), "const:HEX" (an operand equals HEX), "refs:N" (exactly N references to the function).
    private record Entry(String address, String name, String evidence, String... checks) {
    }

    // A data label: checked by a function that references it and, for a facet table, by the name of the
    // first record.
    private record Label(String address, String name, String evidence, String reader, String firstRecordName) {
    }

    private record Build(String label, String sha256, Label[] labels, Entry[] entries) {
    }

    // classic_7.0.103.0 (the last 2D client in the tested set; UOP maps).
    private static final Label[] LABELS_7_0_103 = {
        new Label("006b7478", "g_FacetTable", "6 x " + RECORD + " (stride 0x3c); used when g_UseFacetVariants is 0", "005a0310", FIRST_FACET_NAME),
        new Label("006b75c8", "g_FacetTableVariants", "copy of g_FacetTable used when g_UseFacetVariants is 1: Felucca, Trammel, Ilshenar and Ter Mur take map0x/1x/2x/5xLegacyMUL.uop and statics/staidx 0x/1x/2x/5x; sizes, wrap, dif ids and flag equal", "005a0310", FIRST_FACET_NAME),
        new Label("00a58d7e", "g_UseFacetVariants", "u8, 0xA9 character-list flags bit 15 (0x8000), set by ApplyCharListFlags; selects g_FacetTableVariants in every table accessor", "005640b0", null),
        new Label("00a58d71", "g_SiegeRules", "u8, 0xA9 flags bit 2 (0x4); forces the Felucca-rules result to 1 on every facet", "005640b0", null),
        new Label("0074229c", "g_WrapWidth", "current facet wrap_width (+0xc), set by SetFacet", "00593eb0", null),
        new Label("007422a0", "g_WrapHeight", "current facet wrap_height (+0x10), set by SetFacet", "00593eb0", null),
        new Label("00aba9b5", "g_CurrentFacet", "u8 current facet index (0xBF/8)", "005a23c0", null),
    };

    private static final Entry[] ENTRIES_7_0_103 = {
        new Entry("005a0310", "FacetIndexFromName", "strstr of the name against each record name of the selected table copy; 6 when none matches",
            "calls:006b7478", "calls:006b75c8", "calls:00a58d7e"),
        new Entry("005a0380", "GetFacetWidth", "record +4 of the selected copy", "calls:006b747c", "calls:006b75cc"),
        new Entry("005a03c0", "GetFacetHeight", "record +8", "calls:006b7480", "calls:006b75d0"),
        new Entry("005a0400", "GetFacetWrapWidth", "record +0xc", "calls:006b7484", "calls:006b75d4"),
        new Entry("005a0440", "GetFacetWrapHeight", "record +0x10", "calls:006b7488", "calls:006b75d8"),
        new Entry("005a0480", "GetFacetStaticsFileId", "record +0x14 (statics file id)", "calls:006b748c", "calls:006b75dc"),
        new Entry("005a04c0", "GetFacetStaidxFileId", "record +0x18 (staidx file id)", "calls:006b7490", "calls:006b75e0"),
        new Entry("005a0500", "GetFacetMapFileId", "record +0x28 (map file id); the mapdif/stadif/stadifi ids +0x1c..+0x24 have no reader in this build", "calls:006b74a0", "calls:006b75f0"),
        new Entry("005a23c0", "CurrentFacetHasFeluccaRules", "1 when g_SiegeRules, else record +0x2c of the current facet; used for the 'This may flag you criminal!' confirmation",
            "calls:006b74a4", "calls:00a58d71", "calls:00aba9b5"),
        new Entry("005a2410", "GetFacetName", "record +0 of the selected copy", "const:6b7478", "const:6b75c8"),
        new Entry("0050acf0", "ConfirmCriminalAction", "yes/no prompt 'This may flag you criminal!' before an action on an innocent; the caller asks CurrentFacetHasFeluccaRules first",
            "string:" + STRING_CRIMINAL, "calledby:00488ad0"),
        new Entry("005640b0", "ApplyCharListFlags", "0xA9 u32 flags, one byte per bit at 0x00a58d70..0x00a58d7e; bit 2 g_SiegeRules, bit 15 g_UseFacetVariants",
            "calls:00a58d7e", "calls:00a58d71", "calledby:0047f460"),
        new Entry("0047f460", "Handle_0xA9_CharList", "characters, cities (index, name, area, 6 x u32), then u32 flags passed to ApplyCharListFlags",
            "calls:005640b0", "string:" + STRING_NO_CHARACTERS),
        new Entry("00477f80", "Handle_0xBF_GeneralInfo", "sub 8 SetFacet; sub 0x11 two maps' patch counts; sub 0x18 u32 map count then per map {u32 statics patches, u32 land patches} (up to 6), then ReloadDifLists",
            "calls:00593eb0", "calls:0056e8a0"),
        new Entry("00593eb0", "SetFacet", "width, height, block counts (>> 3), g_WrapWidth and g_WrapHeight of the facet",
            "calls:0074229c", "calls:007422a0", "calledby:00477f80"),
        new Entry("00593ba0", "WrappedTileDistance", "9999 when the points are on opposite sides of g_WrapWidth; inside the wrap region min(d, wrap - d) per axis; Chebyshev max",
            "calls:0074229c", "const:" + Long.toHexString(NO_WRAP_DISTANCE)),
        new Entry("00593c20", "WrappedTileDistanceXY", "WrappedTileDistance with the second point as (x, y)",
            "calls:0074229c", "const:" + Long.toHexString(NO_WRAP_DISTANCE)),
        new Entry("00596d50", "MovePlayerWrapped", "player x < g_WrapWidth: the new x and y wrap modulo the wrap rectangle and the world objects shift by it; else no wrap; then RebuildTilesAroundPlayer",
            "calls:0074229c", "calls:007422a0", "calls:00596840"),
        new Entry("00596840", "RebuildTilesAroundPlayer", "builds each missing cell around the player with BuildTileWrapped (player x < g_WrapWidth) or BuildTileUnwrapped; radar rebuild when open",
            "calls:00596710", "calls:005965f0", "calls:0052b180"),
        new Entry("00596710", "BuildTileWrapped", "cell coordinates wrapped modulo g_WrapWidth / g_WrapHeight before reading land and statics",
            "calledby:00596840", "calls:0074229c"),
        new Entry("005965f0", "BuildTileUnwrapped", "player outside the wrap region: cells with x < g_WrapWidth or y >= g_WrapHeight become land 0x244 (void)",
            "calledby:00596840", "const:" + Long.toHexString(VOID_LAND_GRAPHIC)),
        new Entry("0052b180", "Radar_RebuildAroundPlayer", "radar texture around the player; wrap region: coordinates wrapped; non-wrap region: x < g_WrapWidth, x >= width, y < 0 or y > height use colour 0x8001",
            "calls:0074229c", "const:" + Long.toHexString(RADAR_VOID_COLOR), "calledby:00596840"),
        new Entry("0056e8a0", "ReloadDifLists", "LoadStadifLists then LoadMapdifLists; called only from 0xBF/0x18", "calls:0056e5c0", "calls:0056e700", "calledby:00477f80"),
        new Entry("0056e5c0", "LoadStadifLists", "for each map: stadifl%d.mul u32 block numbers, entry i maps block -> stadif record i (a later duplicate wins), at most the 0xBF/0x18 statics patch count",
            "string:" + STRING_STADIFL, "calledby:0056e8a0"),
        new Entry("0056e700", "LoadMapdifLists", "mapdifl%d.mul, same as LoadStadifLists with the land patch count",
            "string:" + STRING_MAPDIFL, "calledby:0056e8a0"),
        new Entry("0056cf50", "StadifBlockLookup", "block -> stadif record from the stadifl map; no caller: statics patches are loaded but never applied", "refs:0"),
        new Entry("0056cfe0", "MapdifBlockLookup", "block -> mapdif record from the mapdifl map; no caller: land patches are loaded but never applied", "refs:0"),
        new Entry("0059ffb0", "ProbeLegacyMapUops", "bit mask of existing map%uLegacyMUL.uop, u = 0..15; map1 is not probed, bit 1 copies bit 0", "calledby:005a7360"),
    };

    // classic_7.0.20.0 (MUL maps; the variant copy swaps Felucca and Trammel only).
    private static final Label[] LABELS_7_0_20 = {
        new Label("00685088", "g_FacetTable", "6 x " + RECORD + " (stride 0x38); used when g_UseFacetVariants is 0", "00591f50", FIRST_FACET_NAME),
        new Label("006851d8", "g_FacetTableVariants", "copy used when g_UseFacetVariants is 1: Felucca takes map0x/staidx0x/statics0x.mul and Trammel map1x/staidx1x/statics1x.mul; everything else equal", "00591f50", FIRST_FACET_NAME),
        new Label("00a1bcde", "g_UseFacetVariants", "u8, 0xA9 flags bit 15 (0x8000), set by ApplyCharListFlags", "00559c20", null),
        new Label("00a1bcd1", "g_SiegeRules", "u8, 0xA9 flags bit 2 (0x4)", "00559c20", null),
    };

    private static final Entry[] ENTRIES_7_0_20 = {
        new Entry("00591f50", "FacetIndexFromName", "strstr against each record name of the selected copy; 6 when none matches",
            "calls:00685088", "calls:006851d8", "calls:00a1bcde"),
        new Entry("00591fc0", "GetFacetWidth", "record +4 of the selected copy", "calls:0068508c", "calls:006851dc"),
        new Entry("00592000", "GetFacetHeight", "record +8", "calls:00685090", "calls:006851e0"),
        new Entry("00592040", "GetFacetWrapWidth", "record +0xc", "calls:00685094", "calls:006851e4"),
        new Entry("00592080", "GetFacetWrapHeight", "record +0x10", "calls:00685098", "calls:006851e8"),
        new Entry("005920c0", "GetFacetStaticsFileId", "record +0x14", "calls:0068509c", "calls:006851ec"),
        new Entry("00592100", "GetFacetStaidxFileId", "record +0x18", "calls:006850a0", "calls:006851f0"),
        new Entry("00592140", "GetFacetMapFileId", "record +0x28; the dif ids +0x1c..+0x24 have no reader in this build", "calls:006850b0", "calls:00685200"),
        new Entry("00593ed0", "CurrentFacetHasFeluccaRules", "1 when g_SiegeRules, else record +0x2c of the current facet", "calls:006850b4", "calls:00685204", "calls:00a1bcd1"),
        new Entry("00593f20", "GetFacetName", "record +0 of the selected copy", "const:685088", "const:6851d8"),
        new Entry("00559c20", "ApplyCharListFlags", "0xA9 u32 flags, one byte per bit; bit 2 g_SiegeRules, bit 15 g_UseFacetVariants",
            "calls:00a1bcde", "calls:00a1bcd1", "calledby:0047dfd0"),
        new Entry("0047dfd0", "Handle_0xA9_CharList", "characters, cities, then u32 flags passed to ApplyCharListFlags", "calls:00559c20"),
        new Entry("00585de0", "SetFacet", "width, height, wrap width and wrap height of the facet from the accessors",
            "calls:00591fc0", "calls:00592000", "calls:00592040", "calls:00592080", "calledby:00479440"),
        new Entry("00479440", "Handle_0xBF_GeneralInfo", "sub 8 SetFacet, sub 0x18 map patch counts, then ReloadDifLists",
            "calls:00585de0", "calls:005625b0"),
        new Entry("005625b0", "ReloadDifLists", "LoadStadifLists then LoadMapdifLists; called only from 0xBF/0x18",
            "calls:00562300", "calls:00562440", "calledby:00479440"),
        new Entry("00562300", "LoadStadifLists", "for each map: stadifl%d.mul u32 block numbers into the stadif map array 0x00a2e2c8, at most the 0xBF/0x18 statics patch count",
            "string:" + STRING_STADIFL, "calls:00a2e2c8", "calledby:005625b0"),
        new Entry("00562440", "LoadMapdifLists", "mapdifl%d.mul into the mapdif map array 0x00a2e310, same as LoadStadifLists with the land patch count",
            "string:" + STRING_MAPDIFL, "calls:00a2e310", "calledby:005625b0"),
        // An earlier run named 0x00561850 MapdifBlockLookup; it reads the stadif map array (Ghidra made no function at
        // the mapdif lookup 0x005618e0, created with create_function before naming).
        new Entry("00561850", "StadifBlockLookup", "block -> stadif record from the stadifl map array 0x00a2e2c8; no caller: statics patches are loaded but never applied",
            "calls:00a2e2c8", "refs:0"),
        new Entry("005618e0", "MapdifBlockLookup", "block -> mapdif record from the mapdifl map array 0x00a2e310; no caller: land patches are loaded but never applied",
            "calls:00a2e310", "refs:0"),
    };

    // classic_6.0.14.3 (single table, Trammel on map1/statics1/staidx1).
    private static final Label[] LABELS_6_0_14_3 = {
        new Label("005a6db0", "g_FacetTable", "5 x " + RECORD + " (stride 0x38); Trammel uses map1/statics1/staidx1; no variant copy", "00503400", FIRST_FACET_NAME),
    };

    private static final Entry[] ENTRIES_6_0_14_3 = {
        new Entry("00503400", "FacetIndexFromName", "strstr against each record name", "calls:005a6db0"),
        new Entry("00503440", "GetFacetWidth", "record +4", "calls:005a6db4"),
        new Entry("00503470", "GetFacetHeight", "record +8", "calls:005a6db8"),
        new Entry("005034a0", "GetFacetWrapWidth", "record +0xc", "calls:005a6dbc"),
        new Entry("005034d0", "GetFacetWrapHeight", "record +0x10", "calls:005a6dc0"),
        new Entry("00503500", "GetFacetStaticsFileId", "record +0x14", "calls:005a6dc4"),
        new Entry("00503530", "GetFacetStaidxFileId", "record +0x18", "calls:005a6dc8"),
        new Entry("00503560", "GetFacetMapFileId", "record +0x28; the dif ids +0x1c..+0x24 have no reader in this build", "calls:005a6dd8"),
        new Entry("005047d0", "CurrentFacetHasFeluccaRules", "record +0x2c of the current facet (Siege override)", "calls:005a6ddc"),
        new Entry("004face0", "SetFacet", "width, height, wrap width and wrap height from the accessors",
            "calls:00503440", "calls:00503470", "calls:005034a0", "calls:005034d0"),
        new Entry("004e0eb0", "LoadStadifLists", "stadifl%d.mul for 5 maps, at most the 0xBF/0x18 statics patch count; the lists have no reader",
            "string:" + STRING_STADIFL, "calledby:004e1160"),
        new Entry("004e0ff0", "LoadMapdifLists", "mapdifl%d.mul for 5 maps, at most the 0xBF/0x18 land patch count; the lists have no reader",
            "string:" + STRING_MAPDIFL, "calledby:004e1160"),
        new Entry("004e1160", "ReloadDifLists", "LoadStadifLists then LoadMapdifLists", "calls:004e0eb0", "calls:004e0ff0", "calledby:00430ae0"),
        new Entry("00430ae0", "Handle_0xBF_GeneralInfo", "sub 0x11 and 0x18 patch counts, then ReloadDifLists", "calls:004e1160"),
    };

    // UO Gold client.exe (2004-11-11): Felucca and Trammel share map0/statics0, Trammel adds the dif files.
    private static final Label[] LABELS_GOLD = {
        new Label("0056f918", "g_FacetTable", "5 x " + RECORD + " (stride 0x38); Felucca and Trammel 6144 x 4096 on map0/statics0/staidx0, Trammel with mapdif1/stadif1/stadifi1", "004f4780", FIRST_FACET_NAME),
        new Label("00757285", "g_SiegeRules", "u8, 0xA9 flags bit 2 (0x4)", "004ca750", null),
        new Label("005cd66c", "g_WrapWidth", "current facet wrap_width, set by SetFacet", "004eab40", null),
        new Label("005cd670", "g_WrapHeight", "current facet wrap_height, set by SetFacet", "004eab40", null),
        new Label("00768e60", "g_MapdifBlockMaps", "5 x std::map block -> mapdif record, filled by LoadDifLists", "004d1230", null),
        new Label("00768eb8", "g_StadifBlockMaps", "5 x std::map block -> stadif record, filled by LoadDifLists", "004d1140", null),
    };

    private static final Entry[] ENTRIES_GOLD = {
        new Entry("004f4780", "GetFacetName", "record +0", "calls:0056f918"),
        new Entry("004f4800", "GetFacetWidth", "record +4", "calls:0056f91c"),
        new Entry("004f4830", "GetFacetHeight", "record +8", "calls:0056f920"),
        new Entry("004f4860", "GetFacetWrapWidth", "record +0xc", "calls:0056f924"),
        new Entry("004f4890", "GetFacetWrapHeight", "record +0x10", "calls:0056f928"),
        new Entry("004f48c0", "GetFacetStaticsFileId", "record +0x14", "calls:0056f92c"),
        new Entry("004f48f0", "GetFacetStaidxFileId", "record +0x18", "calls:0056f930"),
        new Entry("004f4920", "GetFacetMapdifFileId", "record +0x1c", "calls:0056f934"),
        new Entry("004f4950", "GetFacetStadifFileId", "record +0x20", "calls:0056f938"),
        new Entry("004f4980", "GetFacetStadifiFileId", "record +0x24", "calls:0056f93c"),
        new Entry("004f49b0", "GetFacetMapFileId", "record +0x28", "calls:0056f940"),
        new Entry("004f4610", "CurrentFacetHasFeluccaRules", "1 when g_SiegeRules, else record +0x2c of the current facet", "calls:0056f944", "calls:00757285"),
        new Entry("004f47e0", "IsValidFacet", "facet index < 5", "calledby:004d1230"),
        new Entry("004ca750", "ApplyCharListFlags", "0xA9 u32 flags bits 0..7 only (no bit 15 in Gold); bit 2 g_SiegeRules", "calls:00757285", "calledby:00427d20"),
        new Entry("00427d20", "Handle_0xA9_CharList", "characters, cities, u32 flags to ApplyCharListFlags", "calls:004ca750", "string:" + STRING_NO_CHARACTERS),
        new Entry("0041ff90", "Handle_0xBF_GeneralInfo", "sub 8 SetFacet; sub 0x11 two maps; sub 0x18 u32 map count then per map {u32 statics patches, u32 land patches} (up to 5), then LoadDifLists",
            "calls:004eab40", "calls:004d0cb0"),
        new Entry("004eab40", "SetFacet", "width, height, g_WrapWidth and g_WrapHeight from the accessors",
            "calls:004f4800", "calls:004f4860", "calls:005cd66c", "calledby:0041ff90"),
        new Entry("004d0cb0", "LoadDifLists", "per map stadifl%d.mul then mapdifl%d.mul: u32 block numbers, entry i maps block -> dif record i (a later duplicate wins), at most the 0xBF/0x18 count",
            "string:" + STRING_MAPDIFL, "string:" + STRING_STADIFL, "calledby:0041ff90"),
        new Entry("004d1230", "MapdifBlockLookup", "block -> mapdif record of the facet, -1 when not patched", "calls:00768e60", "calls:004f47e0"),
        new Entry("004d1140", "StadifBlockLookup", "block -> stadif record of the facet, -1 when not patched", "calls:00768eb8"),
        new Entry("004ea9d0", "GetLandGraphicAt", "land graphic of (x, y): mapdif record when MapdifBlockLookup >= 0, else the map block; 0x8001 outside",
            "calls:004d1230", "calls:004f4920", "calls:004f49b0"),
        new Entry("004ea700", "GetLandZAt", "land z of (x, y) wrapped by the full width and height, from the mapdif record or the map block",
            "calls:004d1230", "calls:004f4920", "calls:004f49b0"),
        new Entry("004e9df0", "BuildLandTileWrapped", "(x, y) wrapped by g_WrapWidth / g_WrapHeight, land from mapdif or map",
            "calls:005cd66c", "calls:004d1230", "calls:004ea700"),
        new Entry("004eaa70", "LoadLandBlock", "map block or its mapdif record", "calls:004d1230", "calls:004f4920", "calls:004f49b0"),
        new Entry("004eaac0", "LoadStaticsBlock", "staidx/statics block or its stadifi/stadif record", "calls:004d1140", "calls:004f4950", "calls:004f4980"),
    };

    private static final Build[] BUILDS = {
        new Build("classic 7.0.103.0 client.exe", CLASSIC_7_0_103_SHA256, LABELS_7_0_103, ENTRIES_7_0_103),
        new Build("classic 7.0.20.0 client.exe", CLASSIC_7_0_20_SHA256, LABELS_7_0_20, ENTRIES_7_0_20),
        new Build("classic 6.0.14.3 client.exe", CLASSIC_6_0_14_3_SHA256, LABELS_6_0_14_3, ENTRIES_6_0_14_3),
        new Build("UO Gold client.exe", GOLD_SHA256, LABELS_GOLD, ENTRIES_GOLD),
    };

    private final Map<String, Integer> changes = new LinkedHashMap<>();

    @Override
    public void run() throws Exception {
        String[] args = getScriptArgs();
        if (args.length < 1 || args.length > 2 || args.length == 2 && !args[1].equals(APPLY_FLAG)) {
            throw new IllegalArgumentException("Expected OUTPUT_FILE [" + APPLY_FLAG + "]");
        }
        String sha = currentProgram.getExecutableSHA256();
        Build build = null;
        for (Build candidate : BUILDS) {
            if (sha != null && sha.equalsIgnoreCase(candidate.sha256())) {
                build = candidate;
            }
        }
        if (build == null) {
            List<String> tested = new ArrayList<>();
            for (Build candidate : BUILDS) {
                tested.add(candidate.label() + " " + candidate.sha256());
            }
            throw new IllegalStateException("NameClassicFacetCode names fixed addresses of the tested builds (SHA-256 "
                + String.join(", ", tested) + "); " + currentProgram.getName() + " has SHA-256 " + sha + ". Nothing done.");
        }
        boolean apply = args.length == 2;
        String outputPath = args[0].replace(ENCODED_SPACE, " ");
        // Created before any change, so an existing output file stops the run unchanged.
        try (PrintWriter output = new PrintWriter(Files.newBufferedWriter(Path.of(outputPath),
                StandardOpenOption.CREATE_NEW))) {
            List<String> rows = new ArrayList<>();
            for (Label label : build.labels()) {
                rows.add(processLabel(label, apply));
            }
            for (Entry entry : build.entries()) {
                rows.add(process(entry, apply));
            }
            output.println("# Classic facet names");
            output.println();
            output.println("Program: `" + currentProgram.getName() + "` (tested build " + build.label() + "), SHA-256 `" + sha + "`.");
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

    private String processLabel(Label label, boolean apply) throws Exception {
        String qualified = FACET + Namespace.DELIMITER + label.name();
        String checks = "referenced by " + label.reader()
            + (label.firstRecordName() == null ? "" : ", first record name \"" + label.firstRecordName() + "\"");
        String row = "| `0x" + label.address() + "` | `" + qualified + "` | " + checks + " | ";
        Address address = toAddr(Long.parseLong(label.address(), HEX_RADIX));
        Function reader = getFunctionAt(toAddr(Long.parseLong(label.reader(), HEX_RADIX)));
        boolean readerOk = reader != null && references(reader, address);
        boolean recordOk = true;
        if (label.firstRecordName() != null) {
            Data nameData = getDataAt(toAddr(getInt(address) & UNSIGNED_INT_MASK));
            recordOk = nameData != null && StringDataInstance.isString(nameData)
                && label.firstRecordName().equals(StringDataInstance.getStringDataInstance(nameData).getStringValue());
        }
        if (!readerOk || !recordOk) {
            return row + "mismatch: " + (readerOk ? "" : "reader ") + (recordOk ? "" : "first record ") + "failed; not named |";
        }
        List<String> result = new ArrayList<>();
        SymbolTable symbols = currentProgram.getSymbolTable();
        Symbol primary = symbols.getPrimarySymbol(address);
        boolean named = primary != null && primary.getName(true).equals(qualified);
        if (!named) {
            if (primary != null && primary.getSource() != SourceType.DEFAULT) {
                return row + "kept `" + primary.getName(true) + "` (" + primary.getSource() + ") |";
            }
            if (apply) {
                symbols.createLabel(address, label.name(), facetNamespace(), SourceType.ANALYSIS).setPrimary();
                count("labels created");
            }
            result.add(apply ? "named" : "would name");
        }
        String plateResult = plate(address, COMMENT_PREFIX + TAG + ": " + label.evidence(), apply);
        if (!plateResult.isEmpty()) {
            result.add(plateResult);
        }
        return row + (result.isEmpty() ? "already applied" : String.join(", ", result)) + " |";
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
        String qualified = FACET + Namespace.DELIMITER + entry.name();
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
            if (symbol.getSource() != SourceType.DEFAULT && !isOwnEarlierName(function, symbol)) {
                return row + "kept `" + symbol.getName(true) + "` (" + symbol.getSource() + ") |";
            }
            String earlier = symbol.getSource() == SourceType.DEFAULT ? null : symbol.getName(true);
            Namespace namespace = currentProgram.getSymbolTable().getNamespace(FACET,
                currentProgram.getGlobalNamespace());
            if (namespace != null && !currentProgram.getSymbolTable().getSymbols(entry.name(), namespace).isEmpty()) {
                return row + "conflict: `" + qualified + "` already names another symbol |";
            }
            if (apply) {
                symbol.setNameAndNamespace(entry.name(), facetNamespace(), SourceType.ANALYSIS);
                count(earlier == null ? "functions renamed" : "own earlier names corrected");
            }
            if (earlier == null) {
                result.add(apply ? "named" : "would name");
            } else {
                result.add((apply ? "renamed" : "would rename") + " from `" + earlier + "`");
            }
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

    // A name this script set in an earlier run (source ANALYSIS in our namespace, our tag and our plate comment): it
    // may be corrected when the entry at the address changed.
    private boolean isOwnEarlierName(Function function, Symbol symbol) {
        boolean tagged = false;
        for (FunctionTag tag : function.getTags()) {
            tagged |= tag.getName().equals(TAG);
        }
        String plate = getPlateComment(function.getEntryPoint());
        return tagged && symbol.getSource() == SourceType.ANALYSIS
            && symbol.getParentNamespace().getName().equals(FACET)
            && plate != null && plate.startsWith(COMMENT_PREFIX);
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

    private void count(String what) {
        changes.merge(what, 1, Integer::sum);
    }
}
