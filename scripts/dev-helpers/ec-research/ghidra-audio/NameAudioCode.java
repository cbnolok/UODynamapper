// Name the audio code of the Enhanced Client (UOSA.exe), Kingdom Reborn (UOKR.exe), the Classic clients
// (7.0.103.0 and UO Gold client.exe) and the Miles Sound System 7.2c files the EC ships (mss32.dll,
// miles/mssmp3.asi): the sound and music tables and their CSV readers, the play paths, the Miles file callbacks,
// the Miles WAV/MP3 acceptance code, the KR audio_*.csv parsers, the Classic Sound.def parser and the Classic
// digital music Config.txt reader.
// Address-based: one entry set per tested file, selected by SHA-256; any other program is refused. Every
// address is verified with the stated checks first; a failed check is reported, not named.
// Findings: results/*.md.
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

public class NameAudioCode extends GhidraScript {
    private static final String APPLY_FLAG = "--apply";
    // Script arguments are split on whitespace, so a path with spaces is passed with %20.
    private static final String ENCODED_SPACE = "%20";
    private static final String UOSA_SHA256 = "5a5e9dc536c836d1bf10b6f86bb88f61a6ae078f519d4060d3bc47aecbebad50";
    private static final String UOKR_SHA256 = "1b802e5c00f3bf89a771778b9c6c57406be0703808eeb4cd983e2f5baa348a43";
    private static final String CLASSIC_7_0_103_SHA256 = "4d857f2f5f8bf350bc50a87ee66b2236bfa480e712e9546c740ff04f9106690a";
    private static final String GOLD_SHA256 = "aa21bb3ea16b6c5e00837569466b052e978baa6e38fd945e2694af4f14fa28c2";
    private static final String MSS32_SHA256 = "b588b4791ff6803247962c661b6d3ff9a8625d0f60363a6fa53f61d401f0e74e";
    private static final String MSSMP3_SHA256 = "0fb784a9cc9826a9ec2412b7d71453b917af6c74442647f38061512e2c3a05bb";
    private static final String TAG = "UO_AUDIO";
    private static final String TAG_COMMENT = "Audio: sound/music tables, CSV and def parsers, play paths, Miles"
        + " file callbacks and WAV/MP3 acceptance";
    private static final String COMMENT_PREFIX = "NameAudioCode: ";
    private static final String PROBABLE = " (probable)";
    private static final int HEX_RADIX = 16;

    private static final String UO_AUDIO = "UOAudio";
    private static final String MYTH_AUDIO = "MythAudio";
    private static final String UO_CSV = "UOCsv";
    private static final String UOP = "UOP";
    private static final String CLASSIC_AUDIO = "UOClassicAudio";
    private static final String CLASSIC_DEFS = "UOClassicDefs";
    private static final String MILES = "Miles";
    private static final String MP3_ASI = "MilesMp3";

    // Checks: "calls:ADDR" (the function references ADDR: call, pointer, jump or data),
    // "calledby:ADDR" (the function at ADDR references this one), "ref:ADDR" (the data or code at ADDR
    // references this function, e.g. a vtable slot), "string:TEXT" (references a string that starts with
    // TEXT), "import:NAME" (references the external symbol NAME), "const:HEX" (an operand equals HEX).
    private record Entry(String address, String namespace, String name, boolean probable, String evidence,
            String... checks) {
    }

    private record Build(String label, String sha256, Entry[] entries) {
    }

    private static final Entry[] ENTRIES_UOSA = {
        new Entry("004b5600", UO_AUDIO, "InitAudio", false, "registers the Lua PlaySound, starts the sound system"
            + " (SoundSystem_Startup) and loads audio_sounds.csv and audio_music.csv",
            "string:PlaySound", "calls:008840e1", "calls:004b5180", "calls:004b53a0"),
        new Entry("004b5180", UO_AUDIO, "LoadSoundTable", false, "UOP data/audio/audio_sounds.csv: rows of int ID,"
            + " string FilePath (comma separated, double quotes toggle, no trimming); the row goes to"
            + " SoundTable_GetOrInsert(ID)", "string:audio_sounds.csv", "calls:009a28c7", "calls:009a2949",
            "calls:004b61c0"),
        new Entry("004b53a0", UO_AUDIO, "LoadMusicTable", false, "UOP data/audio/audio_music.csv rows into"
            + " MusicTable_GetOrInsert(ID)", "string:audio_music.csv", "calls:009a28c7", "calls:004b6290"),
        new Entry("004b61c0", UO_AUDIO, "SoundTable_GetOrInsert", false, "std::map<int, record> operator[]: an"
            + " unknown id inserts an empty record (its path becomes \"data/audio/\", whose load fails)",
            "calledby:004b5180", "calledby:004b5770"),
        new Entry("004b6290", UO_AUDIO, "MusicTable_GetOrInsert", false, "std::map<int, record> operator[] of the"
            + " music table", "calledby:004b53a0", "calledby:004b5a30"),
        new Entry("004b5770", UO_AUDIO, "PlaySoundById", false, "id in AX; mode 0 = 2D (id 0 ignored), 1 ="
            + " positional (gain 1 - d/24*0.9 clamped, failed handle reset for a retry), 2 = stop all;"
            + " path sprintf(\"%s/%s\", \"data/audio\", FilePath) in a 255-byte buffer; plays only the"
            + " audio_sounds.csv file (no legacysounds path exists in the client)", "string:data/audio",
            "calls:004b61c0", "calls:00884ab1", "calls:00884ff9", "calls:00885523"),
        new Entry("004b5a30", UO_AUDIO, "PlayMusicById", false, "music table path \"data/audio/\" + FilePath,"
            + " loaded and played through the sound system", "string:data/audio", "calls:004b6290",
            "calls:00884aeb", "calls:00884ff9"),
        new Entry("004b4f20", UO_AUDIO, "HandlePlaySoundPacket", false, "server sound: u16 id at +6 (0xFFFF"
            + " skipped) and the mode byte go to PlaySoundById unchanged (server id = audio_sounds.csv ID)",
            "calls:004b5770"),
        new Entry("008840e1", MYTH_AUDIO, "SoundSystem_Startup", false, "AIL_set_redist_directory(\"miles\"),"
            + " AIL_startup, sample handles, AIL_set_file_callbacks(MilesFileOpenCallback, ...)",
            "string:miles", "import:_AIL_set_redist_directory@4", "import:_AIL_set_file_callbacks@20"),
        new Entry("00885b67", MYTH_AUDIO, "MilesFileOpenCallback", false, "Miles file open callback: the name is"
            + " looked up only in the mounted UOP archives (UOP::FileExists) and sized with"
            + " UOP::GetFileDecompressedSize", "calls:00989fdb", "calls:0098a29b"),
        new Entry("00884ab1", MYTH_AUDIO, "SoundSystem_LoadSample", false, "returns a slot handle: -2 sound off,"
            + " -1 no free slot, -3 load request failed; the load itself is asynchronous",
            "calls:00884aeb", "calledby:004b5770"),
        new Entry("00884aeb", MYTH_AUDIO, "SoundSystem_RequestSampleFile", false, "queues an asynchronous file"
            + " load job (UOP::FileLoadJobRequest) for the sample name", "calls:00883c98",
            "calledby:00884ab1"),
        new Entry("00883c98", MYTH_AUDIO, "SampleLoadRequest_ctor", true, "constructor of the load job made by"
            + " SoundSystem_RequestSampleFile", "calledby:00884aeb"),
        new Entry("00884ff9", MYTH_AUDIO, "SoundSystem_PlaySample", false, "rejects handle - 1 >= 25000; a slot whose"
            + " load failed is logged and returns 0 (no sound, no fallback)", "calledby:004b5770",
            "calledby:004b5a30", "const:61a7"),
        new Entry("00898194", MYTH_AUDIO, "SampleSound_Start", false, "AIL_set_named_sample_file(handle, name,"
            + " data + 4, size, 0): Miles picks the decoder from the name extension (or an ID3v2 header);"
            + " the playback rate is the file's own, changed only for pitch",
            "import:_AIL_set_named_sample_file@20", "import:_AIL_start_sample@4"),
        new Entry("00898392", MYTH_AUDIO, "SampleSound_StartAtPosition", true, "as SampleSound_Start, then restores"
            + " a saved millisecond position", "import:_AIL_set_named_sample_file@20",
            "import:_AIL_sample_ms_position@12"),
        new Entry("009893e3", UO_CSV, "ReadLine", false, "line reader: stops at CR or LF and consumes CR, LF or"
            + " CRLF; at most 0x7fe characters", "calledby:00989551"),
        new Entry("00989551", UO_CSV, "NextLine", false, "ReadLine, then strips one trailing CR/LF",
            "calls:009893e3", "calledby:004b5180"),
        new Entry("009a28c7", UO_CSV, "ReadStringField", false, "comma separated; a double quote toggles quoting"
            + " and is dropped; no escapes and no whitespace trimming", "calledby:004b5180",
            "calledby:004b53a0"),
        new Entry("009a2949", UO_CSV, "ReadIntField", false, "ReadStringField then atol", "calls:00b6c91e",
            "calledby:004b5180"),
        new Entry("009899ef", UOP, "NormalizePath", false, "copies the name, '\\' -> '/', then tolower; no"
            + " trimming and no '//' collapse; used before HashFileName by every UOP lookup",
            "calls:00b6fd76", "calledby:00989fdb", "calledby:0098a29b"),
    };

    private static final Entry[] ENTRIES_UOKR = {
        new Entry("004951c0", UO_AUDIO, "InitAudio", false, "AudioSystem_Init(\"data/audio\"), then"
            + " LoadEffectsTable", "string:data/audio", "calls:00726bf4", "calls:004937e0"),
        new Entry("004937e0", UO_AUDIO, "LoadEffectsTable", false, "data/audio/audio_effects.csv: rows marker,"
            + " ID# (atol), sound structure name (a leading digit is read as an id, else a name lookup);"
            + " ';' rows are comments; ID# = server sound id + 1", "string:audio_effects.csv",
            "calls:00824bf0", "calls:0072773a"),
        new Entry("00726bf4", MYTH_AUDIO, "AudioSystem_Init", false, "Miles start-up and AudioSystem_LoadCsvFiles"
            + " of the given directory", "string:Audio system successfully initialized", "calls:0072821f"),
        new Entry("0072821f", MYTH_AUDIO, "AudioSystem_LoadCsvFiles", false, "\"<dir>/audio_\" + sounds.csv,"
            + " groups.csv, queues.csv, loops.csv", "string:audio_", "string:sounds.csv", "calls:0072836a",
            "calls:00728e1b", "calls:007291fd", "calls:00729606"),
        new Entry("0072836a", MYTH_AUDIO, "ParseSoundsCsv", false, "first field: '!' section, ';' comment, '#'"
            + " section row, empty/space/',' entry, other characters skip the row; entry fields type id, name,"
            + " filename, rate, volume, pan, offset, min range, max range, repeat, mono, polygroup, priority,"
            + " streamed; '*' = s_default value; an entry with an invalid field is skipped",
            "string:s_default", "string:Failed to read type ID", "calls:007264dc", "calls:0072ee63"),
        new Entry("00728e1b", MYTH_AUDIO, "ParseGroupsCsv", false, "audio_groups.csv: name, member list,"
            + " repeat interval", "string:Failed to open groups file!", "calledby:0072821f"),
        new Entry("007291fd", MYTH_AUDIO, "ParseQueuesCsv", false, "audio_queues.csv: name, member list,"
            + " interval, repeat", "string:Failed to open queues file!", "calledby:0072821f"),
        new Entry("00729606", MYTH_AUDIO, "ParseLoopsCsv", false, "audio_loops.csv: name, start, body and end"
            + " events, duration, interrupt flag, repeat", "string:Failed to open loops file!",
            "calledby:0072821f"),
        new Entry("007264dc", MYTH_AUDIO, "ParseSectionName", false, "upper-cased '!' section name: POLYGROUPS 0,"
            + " ROLLOFF FACTOR 1, DEFAULT PARAMETER 2, other 3", "string:POLYGROUPS",
            "string:ROLLOFF FACTOR", "string:DEFAULT PARAMETER"),
        new Entry("0072ee63", MYTH_AUDIO, "ParseFloatRange", false, "whitespace removed, '%', '-' and '*' become"
            + " spaces, trimmed; empty = -1 (default); \"%f %f\" or one atof for both ends",
            "string:%f %f", "calls:0072ed98", "calls:0072ec60"),
        new Entry("0072efbf", MYTH_AUDIO, "ParsePanRange", true, "as ParseFloatRange with \"%79s %79s\" tokens"
            + " converted by a helper", "string:%79s %79s", "calls:0072ed98"),
        new Entry("0072ede9", MYTH_AUDIO, "ParseIntRange", false, "as ParseFloatRange with \"%d %d\" or one atol",
            "string:%d %d", "calls:0072ec60"),
        new Entry("0072ed98", MYTH_AUDIO, "ReplaceCharWithSpace", false, "string in ESI: every given character"
            + " becomes a space", "calledby:0072ee63"),
        new Entry("0072ec60", MYTH_AUDIO, "TrimWhitespace", false, "string in EDI: trailing and leading iswspace"
            + " removed", "calledby:0072ee63"),
        new Entry("0072ecf1", MYTH_AUDIO, "RemoveWhitespace", false, "removes every iswspace character",
            "calledby:0072ee63"),
        new Entry("00729f4e", MYTH_AUDIO, "ResolveNameId", false, "name to id table (s_silence reserved); adds a"
            + " new name up to the table limit", "string:s_silence", "string:Unable to generate system ID"),
        new Entry("00824bf0", UO_CSV, "ReadStringField", false, "same reader as UOSA.exe: comma separated,"
            + " double quotes toggle, no trimming", "calledby:004937e0", "calledby:00662fa4"),
        new Entry("00662fa4", UO_CSV, "ReadField", false, "ReadStringField into a caller buffer",
            "calls:00824bf0", "calledby:0072836a"),
    };

    private static final Entry[] ENTRIES_CLASSIC_7_0_103 = {
        new Entry("00447bc0", CLASSIC_DEFS, "LoadDefFiles", false, "sound.def, gump.def, anim1-4.def, art.def,"
            + " texterr.def, body.def, corpse.def (no music.def)", "string:sound.def", "string:gump.def",
            "calls:00447220"),
        new Entry("00447220", CLASSIC_DEFS, "DefTable_Load", false, "fopen \"r\", fgets 255 bytes per line,"
            + " DefTable_ParseLine; inserts only ids not already present (first line wins)",
            "calls:0062f52a", "calls:0062e93f", "calls:00446d90"),
        new Entry("00446d90", CLASSIC_DEFS, "DefTable_ParseLine", false, "'#'/empty skipped; \"%hd\" id; scans to"
            + " '{'; each target: non-digits skipped (a '-' too, so -1 reads as 1), \"%hd\", then only ','"
            + " or '}' may follow; after '}' a \"%d\" third field is required; then only whitespace or '#'."
            + " A line breaking a rule is dropped", "calls:0062e8bf", "calls:0062ef33",
            "calledby:00447220"),
        new Entry("00448620", CLASSIC_DEFS, "SoundDefTable_Insert", false, "sound table vtable +8: adds the def"
            + " entry only when the sound slot is missing (offset -1 or length < 0x29)", "ref:0069711c",
            "calls:0056e000"),
        new Entry("00445dd0", CLASSIC_DEFS, "DefTable_Contains", false, "def table vtable +4", "ref:00697118"),
        new Entry("00446fa0", CLASSIC_DEFS, "DefTable_RemapRandom", false, "returns one of the def targets,"
            + " chosen with rand() % count", "calls:0059e950", "calledby:005fa840"),
        new Entry("005fa840", CLASSIC_AUDIO, "PlaySound", false, "remaps through the def table when it holds"
            + " the id; ids above 0x683 rejected; 8 voices; the slot data must be longer than 0x28 bytes;"
            + " optional random pitch around 22048 Hz", "calls:00446fa0", "calls:0056e000",
            "calls:005fa510", "const:683"),
        new Entry("005fa510", CLASSIC_AUDIO, "Voice_Start", false, "SoundVoice_SetData, SetPan, SetVolume,"
            + " SetFrequency (0 = buffer rate), Play", "string:DirectSound SetFrequency failed.",
            "calls:005f9c70"),
        new Entry("005f9c70", CLASSIC_AUDIO, "Voice_SetData", false, "copies slot data + 0x28, length - 0x28:"
            + " the 40-byte header is skipped unread", "calls:0056e000", "calledby:005fa510",
            "const:28"),
        new Entry("005f9fb0", CLASSIC_AUDIO, "Voice_CreateBuffer", false, "DirectSound secondary buffer: PCM mono"
            + " 16-bit 22048 Hz (avg 0xAC40 bytes/s), 0x10000 bytes", "const:5620", "const:ac40"),
        new Entry("005aee50", CLASSIC_AUDIO, "DigitalMusic_LoadConfig", false, "MUSIC\\DIGITAL\\CONFIG.TXT:"
            + " fgets 0x200, trailing characters below '!' trimmed, '#'/empty skipped, strtok \" \": id then"
            + " every further token stored (last one wins)", "string:MUSIC\\DIGITAL\\CONFIG.TXT",
            "calls:00631394"),
        new Entry("005ad3f0", CLASSIC_AUDIO, "DigitalMusic_Play", false, "name,flag split at the first ',';"
            + " \".mp3\" added when the last 4 characters hold no '.'; the extension must be mp3 (stricmp);"
            + " loops when the flag is exactly \"loop\" or \"lop\" (case-sensitive)",
            "string:MUSIC\\DIGITAL\\", "calls:006b7c08", "calls:0063130e"),
    };

    private static final Entry[] ENTRIES_GOLD = {
        new Entry("004f7940", CLASSIC_AUDIO, "DigitalMusic_LoadConfig", false, "MUSIC\\DIGITAL\\CONFIG.TXT"
            + " reader, as in 7.0.103", "string:MUSIC\\DIGITAL\\CONFIG.TXT"),
        new Entry("004f8860", CLASSIC_AUDIO, "DigitalMusic_Play", false, "loops when the flag is \"lop\" or"
            + " \"loop\", as in 7.0.103", "string:MUSIC\\DIGITAL\\", "calls:005ce108", "calls:004f9920"),
    };

    private static final Entry[] ENTRIES_MSS32 = {
        new Entry("2112b670", MILES, "SetNamedSampleFile_Impl", false, "AIL_set_named_sample_file: a valid ID3v2"
            + " header at the start is skipped and the name treated as .mp3; then the ASI provider whose"
            + " \"Input file types\" match the extension; no provider: .wav/.voc go to SetSampleFile_Impl,"
            + " anything else fails", "string:Couldn't find provider for this file type",
            "calledby:21114bd0", "calls:21114a40"),
        new Entry("2112b4c0", MILES, "SetSampleFile_Impl", false, "AIL_set_sample_file: \"Creative\" = VOC,"
            + " \"OggS\" (Speex), else AIL_WAV_info: PCM and 4-bit IMA ADPCM native, other wave tags need an"
            + " ASI codec", "string:Creative", "string:Invalid or missing data block.",
            "calls:2111d1f0"),
        new Entry("21129cc0", MILES, "WavInfo_Impl", false, "AIL_WAV_info: \"WAVE\" at +8; chunk walk from +12"
            + " by size + (size & 1) + 8 up to the RIFF size; \"fmt \" fields, then \"data\" from the first"
            + " chunk; WAVE_FORMAT_EXTENSIBLE needs KSDATAFORMAT_SUBTYPE_PCM", "string:Not a WAV file.",
            "string:Not KSDATAFORMAT_SUBTYPE_PCM.", "calledby:2111d1f0"),
        new Entry("2112b3c0", MILES, "InitSampleFromWavInfo", false, "AIL_set_sample_info: PCM 8 or 16 bits;"
            + " AIL_set_sample_playback_rate(file rate), so the mixer resamples any rate",
            "string:Unsupported wave file format.", "calls:21115510"),
        new Entry("21109d00", MILES, "StrnicmpN", false, "case-insensitive compare of N characters (chunk ids,"
            + " extensions)", "calledby:21129cc0", "calledby:2112b4c0"),
    };

    private static final Entry[] ENTRIES_MSSMP3 = {
        new Entry("26f07080", MP3_ASI, "SeekFrameHeader", false, "scans up to 8 KiB for an 11-bit sync; skips"
            + " inline ID3v1 \"TAG\" and ID3v2 (with footer); Layer III only; bitrate index 15 and rate index"
            + " 3 rejected; later headers must match version, layer and rate (bitrate may change: VBR)",
            "string:MPEG Layer 1 files not supported", "string:MPEG audio header not found"),
    };

    private static final Build[] BUILDS = {
        new Build("UOSA.exe (Enhanced Client)", UOSA_SHA256, ENTRIES_UOSA),
        new Build("UOKR.exe (Kingdom Reborn)", UOKR_SHA256, ENTRIES_UOKR),
        new Build("classic 7.0.103.0 client.exe", CLASSIC_7_0_103_SHA256, ENTRIES_CLASSIC_7_0_103),
        new Build("UO Gold client.exe", GOLD_SHA256, ENTRIES_GOLD),
        new Build("EC mss32.dll (Miles 7.2c)", MSS32_SHA256, ENTRIES_MSS32),
        new Build("EC miles/mssmp3.asi", MSSMP3_SHA256, ENTRIES_MSSMP3),
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
            throw new IllegalStateException("NameAudioCode names fixed addresses of the tested files (SHA-256 "
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
            output.println("# Audio names");
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
