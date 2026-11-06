// Name the code of the Enhanced Client (UOSA.exe) that draws a single Interface.uop gump (gumpart %08d.tga)
// outside the paperdoll: the gump art texture request shared by the Lua RequestGumpArt and the generic server
// gumps, the generic gump items that use it, the MythUi DynamicImage / ScreenTexture quad that draws it, the
// properties that quad carries (alpha blend, no alpha test) and the Gamebryo pixel format choice of its texture.
// Address-based: one entry set per tested file, selected by SHA-256; any other program is refused. Every
// address is verified with the stated checks first; a failed check is reported, not named.
// Findings: results/uosa_gump_art.md.
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

public class NameGumpArtCode extends GhidraScript {
    private static final String APPLY_FLAG = "--apply";
    // Script arguments are split on whitespace, so a path with spaces is passed with %20.
    private static final String ENCODED_SPACE = "%20";
    private static final String UOSA_SHA256 = "5a5e9dc536c836d1bf10b6f86bb88f61a6ae078f519d4060d3bc47aecbebad50";
    private static final String TAG = "UO_GUMP_ART";
    private static final String TAG_COMMENT = "Gump art: Interface.uop gumps drawn as UI textures (Lua RequestGumpArt,"
        + " generic gumps) and the UI quad state they are drawn with";
    private static final String COMMENT_PREFIX = "NameGumpArtCode: ";
    private static final String PROBABLE = " (probable)";
    private static final int HEX_RADIX = 16;

    private static final String GUMP_ART = "UOGumpArt";
    private static final String GENERIC_GUMP = "GenericGump";
    private static final String MYTH_UI = "MythUi";
    private static final String SCREEN_TEXTURE = "MythUi::ScreenTexture";
    private static final String DYNAMIC_IMAGE = "MythUi::DynamicImage";
    private static final String WINDOW = "MythUi::Window";

    // Checks: "calls:ADDR" (the function references ADDR: call, pointer, jump or data, e.g. a vtable store),
    // "calledby:ADDR" (the function at ADDR references this one), "ref:ADDR" (the data or code at ADDR
    // references this function, e.g. a vtable slot), "string:TEXT" (references a string that starts with
    // TEXT), "import:NAME" (references the external symbol NAME), "const:HEX" (an operand equals HEX).
    // The namespace may be nested with "::".
    private record Entry(String address, String namespace, String name, boolean probable, String evidence,
            String... checks) {
    }

    private record Build(String label, String sha256, Entry[] entries) {
    }

    private static final Entry[] ENTRIES_UOSA = {
        // The gump art texture.
        new Entry("0051af10", GUMP_ART, "RequestGumpArtTexture", false, "(gump id & 0xffff, out UI texture name,"
            + " out width, out height; EAX out scale): cached by id with a reference count; on a miss"
            + " MakeGumpTextureLocation (Data/Interface/Default/Textures/GumpArt/%08d.tga) and"
            + " TextureManager::RequestTexture, so UOTextureData / NiSourceTexture::Create with prefs {PIX_DEFAULT,"
            + " ALPHA_DEFAULT, no mipmaps}; registers the texture as \"%08d_GumpArt\" with the UI texture list;"
            + " no colour key or alpha change. Callers: UOLua::RequestGumpArt, the generic gump items, the paperdoll",
            "calls:00519700", "calls:00900c72", "string:%08d_GumpArt"),
        new Entry("00900c72", MYTH_UI, "RegisterUiTexture", true, "(name, NiTexture): wraps the texture in a"
            + " MythUi::Texture and stores it under the name, so DynamicImage / Button textures can name it; called by"
            + " the four Request*Texture functions (gump art, tile art, legacy tile art)", "calls:0091165e",
            "calledby:0051af10", "calledby:00519c50"),
        new Entry("00911701", "MythUi::Texture", "Texture", false, "constructor: stores the MythUi::Texture vtable"
            + " and the NiTexture; no pixel change", "calls:00cee57c", "calledby:0091165e"),
        // Generic server gumps: the layout parser and the items that draw gump art.
        new Entry("004dada0", GENERIC_GUMP, "BuildLayout", true, "parses a generic gump's layout commands (gumppic,"
            + " tilepic, resizepic, picinpic, button, checkbox, radio, checkertrans, gumppictiled, ...) into"
            + " GGumpItem objects inside a GenericGumpTemplate window; called when a gump is created, rebuilt and on"
            + " UI reload", "string:gumppic", "string:checkertrans", "string:GenericGumpTemplate"),
        new Entry("004dded0", GENERIC_GUMP, "AddGumpPic", true, "gumppic: a GGumpItemGumpPic, or a"
            + " GGumpItemVirtue when the class argument is VirtueGumpItem", "calls:004e3600", "calls:004e7400",
            "string:VirtueGumpItem"),
        new Entry("004de1c0", GENERIC_GUMP, "AddResizePic", true, "resizepic: ids 0x7778, 0x805 and 0x806 make a"
            + " GGumpItemResizePic (one image), any other id a GGumpItemGumpPicResize (nine pieces id..id+8)",
            "calls:004e3db0", "calls:004e4050", "const:7778", "const:805", "const:806"),
        new Entry("004e2960", GENERIC_GUMP, "CreateImage", true, "a DynamicImage from GenericGumpImageTemplate"
            + " (type 0x2b) showing the named UI texture (DynamicImage::SetTexture); with a non-zero hue it sets the"
            + " custom shader UOSpriteUIShader with {hue} or {hue, second value} and the window tint (r, g, b, 255)"
            + " from 0x004279e0(hue), else the quad keeps the default SimpleFFP shader; no alpha or texture flags"
            + " are set", "string:GenericGumpImageTemplate",
            "string:UOSpriteUIShader", "calls:008f47d9", "calls:008f4bd3", "calls:008e58e6"),
        new Entry("004e2b80", GENERIC_GUMP, "CreateImageSlice", true, "picinpic: like CreateImage with a texture"
            + " offset (+0x2a8, +0x2ac) and size (+0x2b0, +0x2b4) inside the gump", "string:GenericGumpImageTemplate",
            "calls:008f47d9", "calledby:004e3b50"),
        new Entry("004e2cb0", GENERIC_GUMP, "CreateButton", true, "button, checkbox, radio: requests both gump ids"
            + " through RequestGumpArtTexture and makes a GenericGumpButtonTemplate button (type 0x31) with them as"
            + " the state textures", "string:GenericGumpButtonTemplate", "calls:0051af10", "calls:008fdd99"),
        new Entry("004e6c50", GENERIC_GUMP, "SetAlphaOfItemsInRect", true, "checkertrans: walks the gump's item"
            + " windows and multiplies the alpha of each one that lies inside the rectangle by the argument (0.5)",
            "calls:008e563b", "calledby:004e6b60"),
        new Entry("004e3600", "GGumpItemGumpPic", "GGumpItemGumpPic", false, "gumppic: RequestGumpArtTexture(id),"
            + " then CreateImage at (x, y) with the item's hue", "calls:00ca6ecc", "calls:0051af10",
            "calls:004e2960"),
        new Entry("004e3b50", "GGumpItemGumpPicInPic", "GGumpItemGumpPicInPic", false, "picinpic:"
            + " RequestGumpArtTexture(id), then CreateImageSlice", "calls:00ca6f24", "calls:0051af10",
            "calls:004e2b80"),
        new Entry("004e3db0", "GGumpItemResizePic", "GGumpItemResizePic", false, "resizepic of ids 0x7778, 0x805,"
            + " 0x806: one RequestGumpArtTexture image, its size clamped to the texture", "calls:00ca6ef8",
            "calls:0051af10", "calls:004e2960"),
        new Entry("004e4050", "GGumpItemGumpPicResize", "GGumpItemGumpPicResize", false, "resizepic: nine pieces"
            + " id..id+8 placed as corners, stretched edges and a stretched centre", "calls:00ca6f7c",
            "calls:004e42d0"),
        new Entry("004e42d0", "GGumpItemGumpPicResize", "AddPiece", true, "one resizepic piece: gump id = base +"
            + " index through RequestGumpArtTexture, CreateImage at (x, y), stretched when a size is given",
            "calls:0051af10", "calls:004e2960", "calledby:004e4050"),
        new Entry("004e71e0", "GGumpItemGumpPicTiled", "GGumpItemGumpPicTiled", false, "gumppictiled:"
            + " RequestGumpArtTexture(id), then CreateImage", "calls:00ca7108", "calls:0051af10", "calls:004e2960"),
        new Entry("004e7400", "GGumpItemVirtue", "GGumpItemVirtue", false, "gumppic of class VirtueGumpItem:"
            + " RequestGumpArtTexture(id), then CreateImage", "calls:00ca7160", "calls:0051af10", "calls:004e2960"),
        new Entry("004e6b60", "GGumpItemCheckerTrans", "GGumpItemCheckerTrans", false, "checkertrans: no texture;"
            + " SetAlphaOfItemsInRect(items, rect, 0.5: FLD of the float at 0x00c95604)", "calls:00ca70dc",
            "calls:004e6c50", "calls:00c95604"),
        new Entry("004e4a30", "GGumpItemButton", "GGumpItemButton", false, "button: CreateButton(up id, down id)",
            "calls:00ca6fd4", "calls:004e2cb0"),
        new Entry("004e4b50", "GGumpItemCheckBox", "GGumpItemCheckBox", false, "checkbox: CreateButton(off id, on"
            + " id)", "calls:00ca7000", "calls:004e2cb0"),
        new Entry("004e4da0", "GGumpItemRadioButton", "GGumpItemRadioButton", false, "radio: CreateButton(off id,"
            + " on id)", "calls:00ca702c", "calls:004e2cb0"),
        new Entry("004e6e40", "GGumpItemButtonTileArt", "GGumpItemButtonTileArt", false, "buttontileart:"
            + " CreateButton(gump ids), then CreateImage for the tile art", "calls:00ca7134", "calls:004e2cb0",
            "calls:004e2960"),
        // The MythUi quad.
        new Entry("008f47d9", DYNAMIC_IMAGE, "SetTexture", true, "stores the MythUi::Texture (+0x2a0), takes the"
            + " texture size and rebuilds the ScreenTexture", "calls:008f4826", "calledby:004e2960"),
        new Entry("008f4826", DYNAMIC_IMAGE, "RebuildScreenTexture", true, "new MythUi::ScreenTexture(1 quad) over"
            + " the image's rectangle; sets its stencil (clip) values, UVs and tint", "calls:00943724",
            "calledby:008f47d9"),
        new Entry("008f4bd3", DYNAMIC_IMAGE, "SetCustomShader", true, "the C++ side of DynamicImageSetCustomShader:"
            + " shader name and float parameters for the ScreenTexture", "calledby:008f54d8", "calledby:004e2960"),
        new Entry("008e58e6", WINDOW, "SetTintColor", true, "the C++ side of WindowSetTintColor (r, g, b, a)",
            "calledby:008e9ea5"),
        new Entry("008e563b", WINDOW, "SetAlpha", true, "the C++ side of WindowSetAlpha: alpha * base alpha,"
            + " clamped to 0..1, passed to the children", "calledby:008e9d2e"),
        new Entry("00943724", SCREEN_TEXTURE, "ScreenTexture", false, "constructor (quad count, texture): stores"
            + " the ScreenTexture vtable, then BuildGeometry", "calls:00cf5e74", "calls:00943cb6"),
        new Entry("00943cb6", SCREEN_TEXTURE, "BuildGeometry", true, "vertex, colour and UV arrays of the quads,"
            + " the triangle list, then AttachProperties", "calls:00944687", "calledby:00943724"),
        new Entry("00944687", SCREEN_TEXTURE, "AttachProperties", true, "shader: the custom one by name, else"
            + " SimpleFFP; NiZBufferProperty with test and write off (& 0xfffc); NiStencilProperty;"
            + " NiAlphaProperty with blend on (| 1), source SRCALPHA, destination INVSRCALPHA and the test bit"
            + " never set (0x009447e8..0x00944810); NiTexturingProperty with the base texture, apply MODULATE;"
            + " NiVertexColorProperty (source ambient and diffuse); NiMaterialProperty", "calls:00419f70",
            "calls:0041a470", "calls:00670490", "calls:00416bf0", "string:SimpleFFP"),
        new Entry("009d3300", "SimpleFFP", "SimpleFFP", false, "constructor of the default UI shader: one"
            + " fixed-function pass", "calls:00cfbdfc", "string:SimpleFFP"),
        // Gamebryo.
        new Entry("00419f70", "NiAlphaProperty", "NiAlphaProperty", false, "constructor (Gamebryo 2.6"
            + " NiAlphaProperty.h): vtable, flags 0xec = blend off, SRCALPHA / INVSRCALPHA, test off",
            "calls:00c9dfbc", "const:ec"),
        new Entry("0041a470", "NiZBufferProperty", "NiZBufferProperty", false, "constructor: vtable, flags 0xf",
            "calls:00c9e06c"),
        new Entry("0041a2b0", "NiStencilProperty", "NiStencilProperty", false, "constructor: vtable",
            "calls:00c9e014"),
        new Entry("00416bf0", "NiVertexColorProperty", "NiVertexColorProperty", false, "constructor: vtable, flags"
            + " 8", "calls:00c9d96c"),
        new Entry("00778160", "NiDX9TextureData", "FindMatchingPixelFormat", false, "Gamebryo 2.6"
            + " NiDX9TextureData.cpp: picks the device format from the source NiPixelData and the FormatPrefs; for"
            + " PIX_DEFAULT an RGB source (24-bit TGA) gets TEX_RGB888 (D3DFMT_X8R8G8B8) first, an RGBA source"
            + " (32-bit TGA) TEX_RGBA8888 (D3DFMT_A8R8G8B8)", "calledby:00770b90"),
    };

    private static final Build[] BUILDS = {
        new Build("UOSA.exe (Enhanced Client)", UOSA_SHA256, ENTRIES_UOSA),
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
            throw new IllegalStateException("NameGumpArtCode names fixed addresses of the tested files (SHA-256 "
                + String.join(", ", tested) + "); " + currentProgram.getName() + " has SHA-256 " + sha
                + ". Nothing done.");
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
            for (Entry entry : build.entries()) {
                rows.add(process(entry, apply));
            }
            output.println("# Gump art names");
            output.println();
            output.println("Program: `" + currentProgram.getName() + "` (tested file " + build.label()
                + "), SHA-256 `" + sha + "`.");
            output.println();
            output.println("Mode: " + (apply ? "`--apply` (changes: " + (changes.isEmpty() ? "none" : changes) + ")"
                : "report only") + ".");
            output.println();
            output.println("| Address | Name | Checks | Result |");
            output.println("|---|---|---|---|");
            rows.forEach(output::println);
        }
        println("Wrote " + outputPath + (apply ? " (changes: " + (changes.isEmpty() ? "none" : changes) + ")" : ""));
    }

    private void ensureTag() {
        if (currentProgram.getFunctionManager().getFunctionTagManager().getFunctionTag(TAG) == null) {
            currentProgram.getFunctionManager().getFunctionTagManager().createFunctionTag(TAG, TAG_COMMENT);
            count("tag created");
        }
    }

    // The namespace of a "::" path, or null when a level is missing.
    private Namespace findNamespace(String path) {
        SymbolTable symbols = currentProgram.getSymbolTable();
        Namespace namespace = currentProgram.getGlobalNamespace();
        for (String part : path.split(Namespace.DELIMITER)) {
            namespace = symbols.getNamespace(part, namespace);
            if (namespace == null) {
                return null;
            }
        }
        return namespace;
    }

    private Namespace namespace(String path) throws Exception {
        SymbolTable symbols = currentProgram.getSymbolTable();
        Namespace namespace = currentProgram.getGlobalNamespace();
        for (String part : path.split(Namespace.DELIMITER)) {
            Namespace next = symbols.getNamespace(part, namespace);
            namespace = next != null ? next : symbols.createNameSpace(namespace, part, SourceType.ANALYSIS);
        }
        return namespace;
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
        String row = "| `0x" + entry.address() + "` | `" + qualified + "`" + (entry.probable() ? PROBABLE : "")
            + " | " + String.join(", ", entry.checks()) + " | ";
        Function function = getFunctionAt(address);
        if (function == null) {
            return row + "mismatch: no function at the address |";
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
        List<String> result = new ArrayList<>();
        Symbol symbol = function.getSymbol();
        if (!symbol.getName(true).equals(qualified)) {
            if (symbol.getSource() != SourceType.DEFAULT && !isOwnEarlierName(function, symbol)) {
                return row + "kept `" + symbol.getName(true) + "` (" + symbol.getSource() + ") |";
            }
            String earlier = symbol.getSource() == SourceType.DEFAULT ? null : symbol.getName(true);
            Namespace existing = findNamespace(entry.namespace());
            if (existing != null && !currentProgram.getSymbolTable().getSymbols(entry.name(), existing).isEmpty()) {
                return row + "conflict: `" + qualified + "` already names another symbol |";
            }
            if (apply) {
                symbol.setNameAndNamespace(entry.name(), namespace(entry.namespace()), SourceType.ANALYSIS);
                count(earlier == null ? "functions renamed" : "own earlier names corrected");
            }
            if (earlier == null) {
                result.add(apply ? "named" : "would name");
            } else {
                result.add((apply ? "renamed" : "would rename") + " from `" + earlier + "`");
            }
        }
        if (!hasTag(function)) {
            if (apply) {
                function.addTag(TAG);
                count("tags added");
            }
            result.add(apply ? "tagged" : "would tag");
        }
        String comment = COMMENT_PREFIX + TAG + " " + entry.name() + (entry.probable() ? PROBABLE : "") + ": "
            + entry.evidence();
        String plateResult = plate(address, comment, apply);
        if (!plateResult.isEmpty()) {
            result.add(plateResult);
        }
        return row + (result.isEmpty() ? "already applied" : String.join(", ", result)) + " |";
    }

    private boolean hasTag(Function function) {
        for (FunctionTag tag : function.getTags()) {
            if (tag.getName().equals(TAG)) {
                return true;
            }
        }
        return false;
    }

    // A name this script set in an earlier run (source ANALYSIS, our tag and our plate comment): it may be
    // corrected when the entry at the address changed.
    private boolean isOwnEarlierName(Function function, Symbol symbol) {
        String plate = getPlateComment(function.getEntryPoint());
        return hasTag(function) && symbol.getSource() == SourceType.ANALYSIS
            && plate != null && plate.startsWith(COMMENT_PREFIX);
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
            case "ref" -> {
                boolean found = false;
                for (Reference reference : getReferencesFrom(toAddr(Long.parseLong(value, HEX_RADIX)))) {
                    found |= reference.getToAddress().equals(function.getEntryPoint());
                }
                yield found;
            }
            case "string" -> referencesString(function, value);
            case "import" -> referencesExternal(function, value);
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

    private boolean referencesString(Function function, String prefix) {
        for (Address from : function.getBody().getAddresses(true)) {
            for (Reference reference : getReferencesFrom(from)) {
                Data data = getDataAt(reference.getToAddress());
                if (data != null && StringDataInstance.isString(data)) {
                    String text = StringDataInstance.getStringDataInstance(data).getStringValue();
                    if (text != null && text.startsWith(prefix)) {
                        return true;
                    }
                }
            }
        }
        return false;
    }

    private boolean referencesExternal(Function function, String name) {
        for (Address from : function.getBody().getAddresses(true)) {
            for (Reference reference : getReferencesFrom(from)) {
                if (!reference.getToAddress().isExternalAddress()) {
                    continue;
                }
                Symbol target = currentProgram.getSymbolTable().getPrimarySymbol(reference.getToAddress());
                if (target != null && target.getName().equals(name)) {
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
