// Parse Direct3D 9 / D3DX9 headers into a 32-bit Windows Ghidra data type archive.
// @category ECResearch
import ghidra.app.script.GhidraScript;
import ghidra.app.util.cparser.C.CParserUtils;
import ghidra.framework.Application;
import ghidra.program.model.data.DataTypeManager;
import ghidra.program.model.data.FileDataTypeManager;
import java.io.File;
import java.util.Arrays;

public class CreateD3D9Archive extends GhidraScript {
    private static final String LANGUAGE_ID = "x86:LE:32:default";
    private static final String COMPILER_ID = "windows";
    private static final String BASE_ARCHIVE = "typeinfo/win32/windows_vs12_32.gdt";
    private static final String[] HEADERS = { "d3d9_prelude.h", "d3d9.h", "d3dx9.h" };
    // CINTERFACE selects the C vtable layout; __i386__ enables the headers' 4-byte packing.
    private static final String[] DEFINES = { "-DCINTERFACE", "-D__i386__" };

    public void run() throws Exception {
        String[] args = getScriptArgs();
        if (args.length < 2) {
            throw new IllegalArgumentException("Expected OUTPUT_GDT INCLUDE_DIR [INCLUDE_DIR ...]");
        }
        File output = new File(args[0]);
        if (output.exists()) {
            throw new IllegalArgumentException("Output already exists: " + output);
        }
        File baseFile = Application.getModuleDataFile("Base", BASE_ARCHIVE).getFile(false);
        FileDataTypeManager base = FileDataTypeManager.openFileArchive(baseFile, false);
        try {
            // Types equivalent to ones in the base archive are replaced by the base archive's.
            DataTypeManager[] openTypes = { base };
            String[] includePaths = Arrays.copyOfRange(args, 1, args.length);
            FileDataTypeManager archive = CParserUtils.parseHeaderFiles(openTypes, HEADERS,
                includePaths, DEFINES, output.getAbsolutePath(), LANGUAGE_ID, COMPILER_ID,
                monitor);
            println("Data types: " + archive.getDataTypeCount(true));
            archive.save();
            archive.close();
        } finally {
            base.close();
        }
    }
}
