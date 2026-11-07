// Name the code of Kingdom Reborn (UOKR.exe) and the Enhanced Client (UOSA.exe) that makes the scene look the
// way it does: the lighting post-process (night map, light sprites, gamma and the light level from the server),
// the projected sprite shadow post-process, the death post-process, their fixed-function shaders, the baked
// half-Lambert vertex shade of the terrain mesh and the Mythic factory of the unused Bloom / HDRBloom / Color
// effects.
// Address-based: one entry set per tested file, selected by SHA-256; any other program is refused. Every
// address is verified with the stated checks first; a failed check is reported, not named.
// Findings: results/uokr_atmosphere.md.
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

public class NameAtmosphereCode extends GhidraScript {
    private static final String APPLY_FLAG = "--apply";
    // Script arguments are split on whitespace, so a path with spaces is passed with %20.
    private static final String ENCODED_SPACE = "%20";
    private static final String UOSA_SHA256 = "5a5e9dc536c836d1bf10b6f86bb88f61a6ae078f519d4060d3bc47aecbebad50";
    private static final String UOKR_SHA256 = "1b802e5c00f3bf89a771778b9c6c57406be0703808eeb4cd983e2f5baa348a43";
    private static final String TAG = "UO_ATMOSPHERE";
    private static final String TAG_COMMENT = "Atmosphere: the light-level, night-map, gamma, shadow and death"
        + " post-processes, their shaders and the baked terrain shade";
    private static final String COMMENT_PREFIX = "NameAtmosphereCode: ";
    private static final String PROBABLE = " (probable)";
    private static final int HEX_RADIX = 16;

    private static final String ATMOSPHERE = "UOAtmosphere";
    private static final String LIGHTING = "UOLightingPostProcessEffect";
    private static final String SHADOW = "UOShadowPostProcessEffect";
    private static final String DEATH = "UODeathPostProcessEffect";
    private static final String LIGHTING_SHADER = "UOLightingShader";
    private static final String NIGHT_MAP_SHADER = "UONightMapShader";
    private static final String SHADOW_SHADER = "UOShadowShader";
    private static final String SPRITE_SHADOW_SHADER = "UOSpriteShaderShadow";
    private static final String DEATH_SHADER = "UODeathShader";
    private static final String TERRAIN = "UOTerrain";
    private static final String SETTINGS = "UOSettings";
    private static final String EFFECTS_MANAGER = "PostProcessEffectsManager";

    // Checks: "calls:ADDR" (the function references ADDR: call, pointer, jump or data),
    // "calledby:ADDR" (the function at ADDR references this one), "ref:ADDR" (the data or code at ADDR
    // references this function, e.g. a vtable slot), "string:TEXT" (references a string that starts with
    // TEXT), "const:HEX" (an operand equals HEX).
    private record Entry(String address, String namespace, String name, boolean probable, String evidence,
            String... checks) {
    }

    private record Build(String label, String sha256, Entry[] entries) {
    }

    private static final Entry[] ENTRIES_UOKR = {
        // World-viewer state: globals 0x00c4a010.. and their setters.
        new Entry("0041ecd0", ATMOSPHERE, "InitPostProcessEffects", false, "creates the lighting effect"
            + " (0x00c4a010) and the shadow effect (0x00c4a02c); light level 0x00c4a018 = 0.75, shadow intensity"
            + " 0x00c4a024 = 0.25, shadows enabled 0x00c4a014 from the settings, gamma 0x00c4a028 = 0.5; pushes"
            + " them into the effects", "calls:0041f190", "calls:0043f740", "calls:00c4a018", "calls:00c4a024"),
        new Entry("0041eef0", ATMOSPHERE, "SetLightLevel", false, "stores the light level (1 = full light) in"
            + " 0x00c4a018 and forwards it to UOLightingPostProcessEffect::SetLightLevel", "calls:0041f2e0",
            "calls:00c4a018"),
        new Entry("0041ef70", ATMOSPHERE, "SetNightSight", false, "stores the night-sight weight in 0x00c4a01c"
            + " and forwards it to UOLightingPostProcessEffect::SetNightSight", "calls:0041f370", "calls:00c4a01c"),
        new Entry("0041efe0", ATMOSPHERE, "SetGamma", false, "stores the gamma slider (0..1, default 0.5) in"
            + " 0x00c4a028 and forwards it to UOLightingPostProcessEffect::SetGamma", "calls:0041f440",
            "calls:00c4a028"),
        new Entry("0041f0c0", ATMOSPHERE, "SetShadowIntensity", false, "stores the shadow intensity (default"
            + " 0.25) in 0x00c4a024 and forwards it to UOShadowPostProcessEffect::SetIntensity", "calls:0043f870",
            "calls:00c4a024"),
        new Entry("0041f070", ATMOSPHERE, "SetShadowsEnabled", true, "stores the showShadows setting in"
            + " 0x00c4a014; the shadow effect is skipped while it is 0", "calls:00c4a014"),
        new Entry("0056e8a0", ATMOSPHERE, "OnGlobalLightLevel", true, "server light level L (0 = day): L < 31"
            + " gives SetLightLevel(1 - L * 0.032258064), i.e. 1 - L / 31", "calls:0041eef0", "const:1f"),
        new Entry("0056e940", ATMOSPHERE, "OnNightSight", true, "sets the lighting effect's night-sight weight"
            + " to 0 or 1", "calls:0041f370"),
        new Entry("0047e790", SETTINGS, "ApplyResolutionSettings", true, "reads SystemData.Settings.Resolution"
            + " (%s.gamma -> SetGamma, the shadow toggle -> SetShadowsEnabled)", "calls:0041efe0",
            "calls:0041f070"),

        // UOLightingPostProcessEffect, vtable 0x00b5126c.
        new Entry("0041f190", LIGHTING, LIGHTING, false, "constructor: vtable 0x00b5126c; light level +0x214 ="
            + " 0.5, night sight +0x218 = 0, gamma +0x21c = 0.5", "calls:00b5126c"),
        new Entry("0041f2b0", LIGHTING, "Initialize", true, "vtable 0x00b5126c slot 39", "ref:00b51308"),
        new Entry("0041f4e0", LIGHTING, "GetInputRenderTargetGroup", true, "vtable slot 46: returns +0x240, the"
            + " render target group of the scene input", "ref:00b51324"),
        new Entry("0041f4f0", LIGHTING, "GetInputTexture", true, "vtable slot 47: returns +0x220, the scene"
            + " input texture", "ref:00b51328"),
        new Entry("0041fe50", LIGHTING, "Render", false, "vtable slot 48: draws the night map (or the"
            + " night-sight map) into the light buffer +0x24c, adds the light sprites of the list +0x134, then"
            + " draws the composite quad into +0x100", "ref:00b5132c", "calls:0041f5c0", "calls:0041fab0"),
        new Entry("0041fe10", LIGHTING, "CreateRenderTargets", true, "vtable slot 57", "ref:00b51350"),
        new Entry("004205b0", LIGHTING, "DrawDebugOverlay", true, "vtable slot 59", "ref:00b51358"),
        new Entry("0041f5c0", LIGHTING, "BuildCompositeQuad", false, "screen quad over the scene (base) with the"
            + " light buffer (shader map 0) and the day map (shader map 1, unused by the stages); diffuse ="
            + " (c, c, c, light level) with c = 0.25 + 0.2 * (gamma - 0.5); UOLightingShader then gives"
            + " 4 * (c * level + light buffer * (1 - level)) * scene", "calledby:0041fe50", "calls:00b86f94",
            "calls:00b86fac"),
        new Entry("0041fab0", LIGHTING, "BuildNightMapQuad", false, "screen quad with the night map (or the"
            + " night-sight map) and UONightMapShader into the light buffer +0x24c", "calledby:0041fe50"),
        new Entry("0041f2e0", LIGHTING, "SetLightLevel", false, "writes +0x214", "calledby:0041eef0"),
        new Entry("0041f370", LIGHTING, "SetNightSight", false, "writes +0x218; 0 draws the night map, else"
            + " the night-sight map", "calledby:0041ef70"),
        new Entry("0041f440", LIGHTING, "SetGamma", false, "writes +0x21c", "calledby:0041efe0"),

        // UOShadowPostProcessEffect, vtable 0x00b525e4.
        new Entry("0043f740", SHADOW, SHADOW, false, "constructor: vtable 0x00b525e4, intensity +0x214 = 0.4"
            + " (set to 0.25 by InitPostProcessEffects)", "calls:00b525e4"),
        new Entry("0043f870", SHADOW, "SetIntensity", false, "writes +0x214", "calledby:0041f0c0"),
        new Entry("0043fe10", SHADOW, "Render", false, "clears the shadow buffer to white, draws the shadow"
            + " sprites (UOSpriteShaderShadow: texture * diffuse), then the composite scene * clamp(S + 1 - s)",
            "calls:0043f9f0"),
        new Entry("0043f9f0", SHADOW, "BuildCompositeQuad", false, "screen quad scene * shadow buffer with"
            + " UOShadowShader", "calledby:0043fe10"),

        // UODeathPostProcessEffect, vtable 0x00b5062c.
        new Entry("00412e20", DEATH, DEATH, false, "constructor: vtable 0x00b5062c", "calls:00b5062c"),

        // Fixed-function shaders (NiD3DShader), slot 40 = texture stage setup.
        new Entry("00543f60", LIGHTING_SHADER, LIGHTING_SHADER, false, "constructor: vtable 0x00b60bec, four"
            + " stages", "calls:00b60bec"),
        new Entry("005441c0", LIGHTING_SHADER, "SetupTextureStages", false, "vtable 0x00b60bec slot 40: stage 0"
            + " BLENDDIFFUSEALPHA(DIFFUSE, light buffer), stage 1 MODULATE4X(CURRENT, scene), stages 2 and 3"
            + " disabled", "ref:00b60c8c"),
        new Entry("00544440", NIGHT_MAP_SHADER, NIGHT_MAP_SHADER, false, "constructor: vtable 0x00b60c94",
            "calls:00b60c94"),
        new Entry("00544600", NIGHT_MAP_SHADER, "SetupTextureStages", false, "vtable 0x00b60c94 slot 40: the"
            + " night map with a world-anchored texture transform (scale f / 13 and 0.75 f / 13, rotated -45"
            + " degrees, translated by the camera)", "ref:00b60d34"),
        new Entry("00545e40", SHADOW_SHADER, "SetupTextureStages", false, "vtable 0x00b60dbc slot 40",
            "ref:00b60e5c"),
        new Entry("00546c70", SPRITE_SHADOW_SHADER, SPRITE_SHADOW_SHADER, false, "constructor: vtable"
            + " 0x00b611bc", "calls:00b611bc"),
        new Entry("00546e30", SPRITE_SHADOW_SHADER, "SetupTextureStages", false, "vtable 0x00b611bc slot 40:"
            + " MODULATE(texture, diffuse) into the shadow buffer", "ref:00b6125c"),
        new Entry("00413d20", DEATH_SHADER, "SetupTextureStages", false, "vtable 0x00b50764 slot 40:"
            + " DOTPRODUCT3 grey, then TFACTOR - CURRENT", "ref:00b50804"),

        // Terrain mesh: baked vertex shade.
        new Entry("0044b7f0", TERRAIN, "BuildTerrainMesh", false, "per 32x32 chunk: 4 vertices per cell;"
            + " vertex colour = grey (N . D + 1) * 0.5, alpha 1, with the constant D = (0, -0.7071, -0.7071) at"
            + " 0x00c0f3b8 and N from ComputeVertexNormal (flat ground 0.854)", "calls:00c0f3b8",
            "calls:004488a0"),
        new Entry("004488a0", TERRAIN, "ComputeVertexNormal", false, "normal of a cell corner from the corner"
            + " heights (three normalised triangle normals averaged); the height axis is y negated",
            "calledby:0044b7f0"),

        // Mythic post-process library.
        new Entry("00700b20", EFFECTS_MANAGER, "CreateEffectByType", true, "type 3 -> Bloom, 4 -> HDRBloom, 5 ->"
            + " Color; reached only through the helper 0x006f1300, which has no code or data reference",
            "calls:007130a0", "calls:0070f3d0"),
    };

    private static final Entry[] ENTRIES_UOSA = {
        new Entry("00429950", LIGHTING, "BuildCompositeQuad", false, "as UOKR 0x0041f5c0: diffuse (c, c, c,"
            + " light level +0x21c) with c = 0.25 + 0.2 * (gamma +0x224 - 0.5); shader maps 0 (light buffer) and"
            + " 1 (day map)", "calledby:0042a360", "string:Tweaks"),
        new Entry("0042a360", LIGHTING, "CreateCompositeResources", true, "loads"
            + " Data\\Textures\\00000020_daymap.tga and builds the composite quad", "calls:00429950",
            "string:Data\\Textures\\00000020_daymap"),
        new Entry("0058f7f0", LIGHTING_SHADER, "SetupTextureStages", false, "as UOKR 0x005441c0: stage 0"
            + " BLENDDIFFUSEALPHA(DIFFUSE, light buffer), stage 1 MODULATE4X(CURRENT, scene), stages 2 and 3"
            + " disabled", "ref:00cb3cf4"),
        new Entry("00460c70", TERRAIN, "ComputeVertexNormal", false, "as UOKR 0x004488a0; BuildTerrainMesh"
            + " shades each vertex grey (N . D + 1) * 0.5 with D = (0, -0.7071, -0.7071) at 0x00dc18b8",
            "calledby:00461ae0"),
    };

    private static final Build[] BUILDS = {
        new Build("UOKR.exe (Kingdom Reborn)", UOKR_SHA256, ENTRIES_UOKR),
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
            throw new IllegalStateException("NameAtmosphereCode names fixed addresses of the tested files (SHA-256 "
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
            output.println("# Atmosphere names");
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

    private Namespace namespace(String name) throws Exception {
        SymbolTable symbols = currentProgram.getSymbolTable();
        Namespace namespace = symbols.getNamespace(name, currentProgram.getGlobalNamespace());
        return namespace != null ? namespace
            : symbols.createNameSpace(currentProgram.getGlobalNamespace(), name, SourceType.ANALYSIS);
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
            + " | " + String.join(", ", entry.checks()).replace("\\", "\\\\") + " | ";
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
            Namespace existing = currentProgram.getSymbolTable().getNamespace(entry.namespace(),
                currentProgram.getGlobalNamespace());
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
