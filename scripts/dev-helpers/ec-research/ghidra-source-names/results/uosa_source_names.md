# Source library names in UOSA.exe

Program: `UOSA.exe`, SHA-256 `5a5e9dc536c836d1bf10b6f86bb88f61a6ae078f519d4060d3bc47aecbebad50`,
Ghidra 12.1.4. Maps: `maps/zlib-1.2.2.tsv`, `maps/freetype-2.3.5.tsv`,
`maps/tinyxml-2.5.3.tsv`, `maps/cryptopp-5.4.tsv`. Version evidence is in `README.md`.

## Summary

| Library | TABLE | VTABLE | STRING | CALLGRAPH | Total | UOP trace (by hand) |
|---|---:|---:|---:|---:|---:|---:|
| zlib 1.2.2 | 0 | 0 | 1 | 0 | 1 | 5 |
| FreeType 2.3.5 (+ zlib 1.1.4 as `ft_zlib::`) | 196 | 0 | 10 | 83 | 289 | 0 |
| TinyXML 2.5.3 | 0 | 84 | 0 | 0 | 84 | 0 |
| Crypto++ 5.4 | 0 | 294 | 15 | 0 | 309 | 0 |

`--apply` renamed 683 functions (84 created at table/vtable entries, 15 namespaces), with
7 agreements between methods, no conflicts, no existing names in the way and no anomalies;
a second `--apply` changed nothing. FreeType: 50 of 105 source tables found (the others
belong to modules not linked or have no anchor). Vtables: 11 of 11 TinyXML classes, 38
Crypto++ classes matched; `CryptoPP::ByteQueue::Walker` and `CryptoPP::DH2` rejected; 186
Crypto++ source classes have no primary vtable in the program. The CALLGRAPH count of the
report-only run is 56: the functions created by `--apply` add call edges.

Spot checks: `FT_Init_FreeType` `0x00ab551a` (not named by the tool: reached only as a
caller) returns 7 without memory and stores version 2.3.5; `FT_RoundFix` `0x00ab5593` is
`(x + 0x8000) & 0xffff0000` with the sign handling of the source; an earlier version of the
tool named UO functions `UOXml::ParseScripts` and the `FreeTypeCharRenderer` constructor
from the plain word "Script" and from a caller edge of the exported `FT_New_Size`, which is
why plain-word strings need two hits and callers are followed only from `static` functions.

## UOP decompression path

UOP entries are decompressed with **zlib 1.2.2 `uncompress` (direct `inflate`)**, not with
Crypto++ `ZlibDecompressor`/`Inflator` (their vtables are linked but not on this path).
The archive class is `MypArchive` (UOP files are Mythic "MYP" packages); its entry
record is 34 bytes (`0x22`): offset 8, header length 4, compressed size 4, decompressed
size 4, hash 8, Adler-32 4, compression flag 2 (0 stored, 1 zlib).

Synchronous open (`UOVirtualFileOpen(path, mode, 0)`):

1. `UONdlFile::UONdlFile` `0x005906f0` (Gamebryo file factory `UONdlFile::FileCreateFunc`
   `0x0058a850`; `UONdlFile::FileAccessFunc` `0x005908d0` checks existence through
   `0x00989fdb`; `UONdlFile::ReadFromMemory` `0x005905a0` then copies from the buffer) and
   51 other callers ->
2. `UOP::OpenFilePtr` `0x00989f55` ->
3. `UOVirtualFileOpen` `0x00989b72`: path key (`FUN_009899ef`, `FUN_0098d487`), archive list search (`FUN_0098d460`), then ->
4. `UOP::MythMypInputFile_ctor` `0x0098e5df` (buffer of the decompressed size) ->
5. `UOP::MypArchive_ReadEntryAlloc` `0x0098d6ed` -> `MypArchive::vftable` slot 3 ->
6. `UOP::MypArchive_ReadEntry` `0x0098d57c`: `UOP::MypArchive_ReadEntryRaw` `0x0098d61b`
   (slot 4: seek to offset + header length, read the compressed size), then flag 1 ->
   `UOP::Decompress` `0x0098d6d3`, flag 0 -> `memcpy` ->
7. `uncompress` `0x009a2530` -> `inflateInit_` `0x009a5bf0` (inflateInit2_ inlined,
   `zcalloc` `0x009a7690`, `inflateReset` `0x009a5ba0`), `inflate` `0x009a5d80`
   (`ZLIB_STRING`), `inflateEnd` `0x009a71e0`.

Asynchronous open (`UOVirtualFileOpen(path, mode, 1)`, also `UOP::ArchiveFileOpen`
`0x009898a6`): `UOP::MypArchive_GetEntryCompression` `0x0098d74c` == 1 ->
`UOP::FileLoadJob_ReadCompressed` `0x0098ce9f` reads the raw bytes into the job; later
`UOP::FileLoadJobRequest_Execute` `0x0098cd97` (`FileLoadJobRequest::vftable` slot 3,
folded with `MythAudio::SampleLoadRequest` slot 3) or `UOP::DecompressJobRequest_Execute`
`0x0098cf2c` (`FileLoadJobRequest::DecompressJobRequest::vftable` slot 3) allocate with tag
`"Decompress"`, call `UOP::Decompress` and wrap the result in
`UOP::MythMemoryInputFile_ctor` `0x0098e672`.

Entry helpers: `UOP::MypArchive_GetEntry` `0x0098db06` (copies the record; index from
`MypArchive` slot 8), `UOP::MypArchive_GetEntryCompressedSize` `0x0098d520` (+0x0c),
`UOP::MypArchive_GetEntryDecompressedSize` `0x0098d503` (+0x10). Other `uncompress`
callers, not UOP: `FUN_005a8380`, `FUN_005a8530` (both from `FUN_005a8070`),
`FUN_0062bf80`.

Named by `NameUopTrace.java` (first applied by an equivalent probe; DEFAULT names only,
plate comment with the evidence, every address verified by the checks listed in
`results/uosa_uop_trace.md`): 15 functions in
namespace `UOP` with tag `UOP_TRACE` (above), and the zlib functions `uncompress`,
`inflateInit_`, `inflateReset`, `inflateEnd`, `zcalloc` with tag `ZLIB_UOP_TRACE`
(structure identical to zlib 1.2.2: return codes, `Z_NEED_DICT`/`Z_BUF_ERROR` mapping,
state size `0x1ba8`, `wbits` 15, reset fields).

# Report of the `--apply` run

Program: `UOSA.exe`, SHA-256 `5a5e9dc536c836d1bf10b6f86bb88f61a6ae078f519d4060d3bc47aecbebad50`, 49357 functions.

- Map `zlib-1.2.2.tsv`: zlib 1.2.2, 102 functions, 28 unique strings, 1 tables, 0 vtable classes
- Map `freetype-2.3.5.tsv`: freetype 2.3.5, 1724 functions, 219 unique strings, 104 tables, 0 vtable classes
- Map `tinyxml-2.5.3.tsv`: tinyxml 2.5.3, 238 functions, 29 unique strings, 0 tables, 11 vtable classes
- Map `cryptopp-5.4.tsv`: cryptopp 5.4, 2826 functions, 320 unique strings, 0 tables, 226 vtable classes

Mode: `--apply` (changes: {functions created=84, functions renamed=683, tags added=683, plate comments set=683, namespaces created=15}). `_purecall`: `0x00b6b4f7`.

| Library | TABLE | VTABLE | STRING | CALLGRAPH | Total |
|---|---|---|---|---|---|
| zlib 1.2.2 | 0 | 0 | 1 | 0 | 1 |
| freetype 2.3.5 | 196 | 0 | 10 | 83 | 289 |
| tinyxml 2.5.3 | 0 | 84 | 0 | 0 | 84 |
| cryptopp 5.4 | 0 | 294 | 15 | 0 | 309 |

Names chosen per library and method (the strongest method at an address); 7 further method agreements.

## Tables (50 found of 105 source tables)

- freetype: `src/autofit/afmodule.c:autofit_module_class` at `0x00c934f0`, variant 0 (no conditions), 8 fields checked, 2 functions; anchor: string "autofitter"
- freetype: `src/autofit/afmodule.c:af_autofitter_service` at `0x00c934e0`, variant 0 (no conditions), 4 fields checked, 1 functions; anchor: pointer from `autofit_module_class`
- freetype: `src/bdf/bdfdrivr.c:bdf_services` at `0x00c838c4`, variant 0 (no conditions), 6 fields checked, 0 functions; anchor: string "bdf"
- freetype: `src/bdf/bdfdrivr.c:bdf_service_bdf` at `0x00c838bc`, variant 0 (no conditions), 2 fields checked, 2 functions; anchor: pointer from `bdf_services`
- freetype: `src/bdf/bdfdrivr.c:bdf_driver_class` at `0x00c83210`, variant 1 (FT_CONFIG_OPTION_OLD_INTERNALS), 22 fields checked, 8 functions; anchor: string "bdf"
- freetype: `src/cff/cffdrivr.c:cff_services` at `0x00c93944`, variant 0 (!FT_CONFIG_OPTION_NO_GLYPH_NAMES), 10 fields checked, 0 functions; anchor: string "xf86-driver-name"
- freetype: `src/cff/cffdrivr.c:cff_service_ps_info` at `0x00c938c4`, variant 0 (no conditions), 3 fields checked, 2 functions; anchor: pointer from `cff_services`
- freetype: `src/cff/cffdrivr.c:cff_service_glyph_dict` at `0x00c93938`, variant 0 (no conditions), 2 fields checked, 2 functions; anchor: pointer from `cff_services`
- freetype: `src/cff/cffdrivr.c:cff_driver_class` at `0x00c938d0`, variant 3 (FT_CONFIG_OPTION_OLD_INTERNALS,TT_CONFIG_OPTION_EMBEDDED_BITMAPS), 22 fields checked, 15 functions; anchor: string "cff"
- freetype: `src/cid/cidriver.c:cid_services` at `0x00c82e4c`, variant 0 (no conditions), 8 fields checked, 0 functions; anchor: string "postscript-font-name"
- freetype: `src/cid/cidriver.c:cid_service_ps_info` at `0x00c82e40`, variant 0 (no conditions), 3 fields checked, 1 functions; anchor: pointer from `cid_services`
- freetype: `src/cid/cidriver.c:t1cid_driver_class` at `0x00c826f0`, variant 1 (FT_CONFIG_OPTION_OLD_INTERNALS), 22 fields checked, 13 functions; anchor: string "t1cid"
- freetype: `src/pcf/pcfdrivr.c:pcf_services` at `0x00c92f78`, variant 0 (no conditions), 6 fields checked, 0 functions; anchor: string "bdf"
- freetype: `src/pcf/pcfdrivr.c:pcf_service_bdf` at `0x00c92f70`, variant 0 (no conditions), 2 fields checked, 2 functions; anchor: pointer from `pcf_services`
- freetype: `src/pcf/pcfdrivr.c:pcf_driver_class` at `0x00c92de0`, variant 1 (FT_CONFIG_OPTION_OLD_INTERNALS), 22 fields checked, 8 functions; anchor: string "pcf"
- freetype: `src/pfr/pfrdrivr.c:pfr_services` at `0x00c934c8`, variant 0 (no conditions), 6 fields checked, 0 functions; anchor: string "pfr-metrics"
- freetype: `src/pfr/pfrdrivr.c:pfr_metrics_service_rec` at `0x00c934bc`, variant 0 (no conditions), 3 fields checked, 3 functions; anchor: pointer from `pfr_services`
- freetype: `src/pfr/pfrdrivr.c:pfr_driver_class` at `0x00c933b8`, variant 1 (FT_CONFIG_OPTION_OLD_INTERNALS), 22 fields checked, 9 functions; anchor: string "pfr"
- freetype: `src/psaux/psauxmod.c:psaux_module_class` at `0x00c9316c`, variant 0 (no conditions), 8 fields checked, 0 functions; anchor: string "psaux"
- freetype: `src/pshinter/pshmod.c:pshinter_module_class` at `0x00c831ec`, variant 0 (no conditions), 8 fields checked, 2 functions; anchor: string "pshinter"
- freetype: `src/psnames/psmodule.c:pscmaps_services` at `0x00c926b0`, variant 0 (!FT_CONFIG_OPTION_NO_POSTSCRIPT_NAMES), 4 fields checked, 0 functions; anchor: string "postscript-cmaps"
- freetype: `src/psnames/psmodule.c:pscmaps_interface` at `0x00c83d00`, variant 1 (FT_CONFIG_OPTION_ADOBE_GLYPH_LIST,!FT_CONFIG_OPTION_NO_POSTSCRIPT_NAMES), 8 fields checked, 6 functions; anchor: pointer from `pscmaps_services`
- freetype: `src/psnames/psmodule.c:psnames_module_class` at `0x00c838dc`, variant 0 (!FT_CONFIG_OPTION_NO_POSTSCRIPT_NAMES), 8 fields checked, 1 functions; anchor: string "psnames"
- freetype: `src/raster/ftrend1.c:ft_raster1_renderer_class` at `0x00c9374c`, variant 0 (no conditions), 13 fields checked, 5 functions; anchor: string "raster1"
- freetype: `src/raster/ftraster.c:ft_standard_raster` at `0x00c93734`, variant 0 (no conditions), 5 fields checked, 5 functions; anchor: pointer from `ft_raster1_renderer_class`
- freetype: `src/raster/ftrend1.c:ft_raster5_renderer_class` at `0x00c93888`, variant 0 (no conditions), 13 fields checked, 5 functions; anchor: string "raster5"
- freetype: `src/sfnt/sfdriver.c:sfnt_services` at `0x00c82618`, variant 3 (TT_CONFIG_OPTION_BDF,TT_CONFIG_OPTION_POSTSCRIPT_NAMES), 12 fields checked, 0 functions; anchor: string "sfnt-table"
- freetype: `src/sfnt/sfdriver.c:sfnt_service_sfnt_table` at `0x00c825f4`, variant 0 (no conditions), 3 fields checked, 3 functions; anchor: pointer from `sfnt_services`
- freetype: `src/sfnt/sfdriver.c:sfnt_service_glyph_dict` at `0x00c82600`, variant 0 (TT_CONFIG_OPTION_POSTSCRIPT_NAMES), 2 fields checked, 1 functions; anchor: pointer from `sfnt_services`
- freetype: `src/sfnt/sfdriver.c:sfnt_service_bdf` at `0x00c82610`, variant 0 (TT_CONFIG_OPTION_BDF), 2 fields checked, 2 functions; anchor: pointer from `sfnt_services`
- freetype: `src/sfnt/sfdriver.c:sfnt_module_class` at `0x00c8227c`, variant 0 (no conditions), 8 fields checked, 1 functions; anchor: string "sfnt"
- freetype: `src/sfnt/sfdriver.c:sfnt_interface` at `0x00c822a0`, variant 7 (FT_CONFIG_OPTION_OLD_INTERNALS,TT_CONFIG_OPTION_EMBEDDED_BITMAPS,TT_CONFIG_OPTION_POSTSCRIPT_NAMES), 40 fields checked, 40 functions; anchor: pointer from `sfnt_module_class`
- freetype: `src/smooth/ftsmooth.c:ft_smooth_renderer_class` at `0x00c92fa8`, variant 0 (no conditions), 13 fields checked, 5 functions; anchor: string "smooth"
- freetype: `src/smooth/ftgrays.c:ft_grays_raster` at `0x00c92f90`, variant 0 (no conditions), 5 fields checked, 4 functions; anchor: pointer from `ft_smooth_renderer_class`
- freetype: `src/smooth/ftsmooth.c:ft_smooth_lcd_renderer_class` at `0x00c92fe4`, variant 0 (no conditions), 13 fields checked, 5 functions; anchor: string "smooth-lcd"
- freetype: `src/smooth/ftsmooth.c:ft_smooth_lcdv_renderer_class` at `0x00c93020`, variant 0 (no conditions), 13 fields checked, 5 functions; anchor: string "smooth-lcdv"
- freetype: `src/truetype/ttdriver.c:tt_services` at `0x00c82ef0`, variant 1 (TT_CONFIG_OPTION_GX_VAR_SUPPORT), 10 fields checked, 0 functions; anchor: string "xf86-driver-name"
- freetype: `src/truetype/ttdriver.c:tt_service_gx_multi_masters` at `0x00c82e6c`, variant 0 (TT_CONFIG_OPTION_GX_VAR_SUPPORT), 5 fields checked, 3 functions; anchor: pointer from `tt_services`
- freetype: `src/truetype/ttdriver.c:tt_driver_class` at `0x00c82e80`, variant 7 (FT_CONFIG_OPTION_OLD_INTERNALS,TT_CONFIG_OPTION_EMBEDDED_BITMAPS,TT_USE_BYTECODE_INTERPRETER), 22 fields checked, 14 functions; anchor: string "truetype"
- freetype: `src/type1/t1driver.c:t1_services` at `0x00c92da8`, variant 0 (!T1_CONFIG_OPTION_NO_AFM,!T1_CONFIG_OPTION_NO_MM_SUPPORT), 14 fields checked, 0 functions; anchor: string "postscript-font-name"
- freetype: `src/type1/t1driver.c:t1_service_glyph_dict` at `0x00c92d7c`, variant 0 (no conditions), 2 fields checked, 2 functions; anchor: pointer from `t1_services`
- freetype: `src/type1/t1driver.c:t1_service_ps_info` at `0x00c92d98`, variant 0 (no conditions), 3 fields checked, 3 functions; anchor: pointer from `t1_services`
- freetype: `src/type1/t1driver.c:t1_service_multi_masters` at `0x00c92d84`, variant 0 (!T1_CONFIG_OPTION_NO_MM_SUPPORT), 5 fields checked, 5 functions; anchor: pointer from `t1_services`
- freetype: `src/type1/t1driver.c:t1_driver_class` at `0x00c926c0`, variant 2 (FT_CONFIG_OPTION_OLD_INTERNALS,!T1_CONFIG_OPTION_NO_AFM), 22 fields checked, 15 functions; anchor: string "type1"
- freetype: `src/type42/t42drivr.c:t42_services` at `0x00c94a84`, variant 0 (no conditions), 10 fields checked, 0 functions; anchor: string "glyph-dict"
- freetype: `src/type42/t42drivr.c:t42_service_glyph_dict` at `0x00c94a6c`, variant 0 (no conditions), 2 fields checked, 2 functions; anchor: pointer from `t42_services`
- freetype: `src/type42/t42drivr.c:t42_service_ps_info` at `0x00c94a78`, variant 0 (no conditions), 3 fields checked, 3 functions; anchor: pointer from `t42_services`
- freetype: `src/type42/t42drivr.c:t42_driver_class` at `0x00c94758`, variant 3 (FT_CONFIG_OPTION_OLD_INTERNALS,TT_USE_BYTECODE_INTERPRETER), 22 fields checked, 14 functions; anchor: string "type42"
- freetype: `src/winfonts/winfnt.c:winfnt_services` at `0x00c82264`, variant 0 (no conditions), 6 fields checked, 0 functions; anchor: string "xf86-driver-name"
- freetype: `src/winfonts/winfnt.c:winfnt_driver_class` at `0x00c820c8`, variant 1 (FT_CONFIG_OPTION_OLD_INTERNALS), 22 fields checked, 8 functions; anchor: string "winfonts"

## Tables rejected

None.

## Vtables matched (49)

- tinyxml: `TiXmlAttribute` `0x00c175b4`, 3 slots (no conditions)
- tinyxml: `TiXmlBase` `0x00c174a8`, 3 slots (no conditions)
- tinyxml: `TiXmlComment` `0x00c174bc`, 18 slots (TIXML_USE_STL)
- tinyxml: `TiXmlDeclaration` `0x00c175c4`, 19 slots (TIXML_USE_STL)
- tinyxml: `TiXmlDocument` `0x00c1785c`, 18 slots (TIXML_USE_STL)
- tinyxml: `TiXmlElement` `0x00c177cc`, 18 slots (TIXML_USE_STL)
- tinyxml: `TiXmlNode` `0x00c1777c`, 18 slots (TIXML_USE_STL)
- tinyxml: `TiXmlPrinter` `0x00c17834`, 9 slots (no conditions)
- tinyxml: `TiXmlText` `0x00c1750c`, 18 slots (TIXML_USE_STL)
- tinyxml: `TiXmlUnknown` `0x00c1755c`, 18 slots (TIXML_USE_STL)
- tinyxml: `TiXmlVisitor` `0x00c1769c`, 9 slots (no conditions)
- cryptopp: `CryptoPP::Adler32` `0x00cb948c`, 18 slots (no conditions)
- cryptopp: `CryptoPP::AlgorithmParametersBase` `0x00cb90e0`, 4 slots (no conditions)
- cryptopp: `CryptoPP::ArraySink` `0x00cdfc6c`, 44 slots (no conditions)
- cryptopp: `CryptoPP::ArrayXorSink` `0x00ce29b4`, 44 slots (no conditions)
- cryptopp: `CryptoPP::AutoSeededRandomPool` `0x00cb7a6c`, 8 slots (OS_RNG_AVAILABLE)
- cryptopp: `CryptoPP::BERGeneralDecoder` `0x00ce262c`, 45 slots (no conditions)
- cryptopp: `CryptoPP::BERSequenceDecoder` `0x00ce0314`, 45 slots (no conditions)
- cryptopp: `CryptoPP::BitBucket` `0x00cdfbac`, 44 slots (no conditions)
- cryptopp: `CryptoPP::ByteQueue` `0x00ce24ac`, 44 slots (no conditions)
- cryptopp: `CryptoPP::ClassNullRNG` `0x00cdfd54`, 8 slots (no conditions)
- cryptopp: `CryptoPP::DERGeneralEncoder` `0x00ce26f4`, 44 slots (no conditions)
- cryptopp: `CryptoPP::DERSequenceEncoder` `0x00ce03dc`, 44 slots (no conditions)
- cryptopp: `CryptoPP::Deflator` `0x00ce18e4`, 50 slots (no conditions)
- cryptopp: `CryptoPP::FilterWithBufferedInput` `0x00ce1e8c`, 56 slots (no conditions)
- cryptopp: `CryptoPP::HashFilter` `0x00ce28e4`, 47 slots (no conditions)
- cryptopp: `CryptoPP::HashVerificationFilter` `0x00ce206c`, 56 slots (no conditions)
- cryptopp: `CryptoPP::Inflator` `0x00ce2dd4`, 53 slots (no conditions)
- cryptopp: `CryptoPP::Integer` `0x00ce01e8`, 4 slots (no conditions)
- cryptopp: `CryptoPP::KDF2_RNG` `0x00ce0584`, 8 slots (no conditions)
- cryptopp: `CryptoPP::LowFirstBitWriter` `0x00ce1814`, 47 slots (no conditions)
- cryptopp: `CryptoPP::MessageQueue` `0x00ce2f44`, 44 slots (no conditions)
- cryptopp: `CryptoPP::ModExpPrecomputation` `0x00cb7cdc`, 6 slots (no conditions)
- cryptopp: `CryptoPP::ModularArithmetic` `0x00cb7b54`, 27 slots (no conditions)
- cryptopp: `CryptoPP::MontgomeryRepresentation` `0x00cb7bc4`, 27 slots (no conditions)
- cryptopp: `CryptoPP::NonblockingRng` `0x00ce0b48`, 8 slots (NONBLOCKING_RNG_AVAILABLE,OS_RNG_AVAILABLE)
- cryptopp: `CryptoPP::NullNameValuePairs` `0x00cb90d4`, 2 slots (no conditions)
- cryptopp: `CryptoPP::OutputProxy` `0x00ce226c`, 44 slots (no conditions)
- cryptopp: `CryptoPP::PK_DefaultDecryptionFilter` `0x00cdfe4c`, 48 slots (no conditions)
- cryptopp: `CryptoPP::PK_DefaultEncryptionFilter` `0x00cdfd7c`, 48 slots (no conditions)
- cryptopp: `CryptoPP::ProxyFilter` `0x00ce232c`, 56 slots (no conditions)
- cryptopp: `CryptoPP::RandomPool` `0x00ce0bd8`, 8 slots (no conditions)
- cryptopp: `CryptoPP::SignatureVerificationFilter` `0x00ce216c`, 56 slots (no conditions)
- cryptopp: `CryptoPP::StreamTransformationFilter` `0x00ce1f7c`, 56 slots (no conditions)
- cryptopp: `CryptoPP::StringStore` `0x00ce0224`, 45 slots (no conditions)
- cryptopp: `CryptoPP::ZlibCompressor` `0x00cb94dc`, 50 slots (no conditions)
- cryptopp: `CryptoPP::ZlibDecompressor` `0x00ce0f9c`, 53 slots (no conditions)
- cryptopp: `CryptoPP::DL_FixedBasePrecomputation<CryptoPP::Integer>` `0x00cb7c9c`, 8 slots (no conditions)
- cryptopp: `CryptoPP::DL_GroupPrecomputation<CryptoPP::Integer>` `0x00cb7cc0`, 6 slots (no conditions)

## Vtables rejected

- `CryptoPP::ByteQueue::Walker` `0x00ce256c` (7 slots): variant 0 (no conditions): 44 slots in the source
- `CryptoPP::DH2` `0x00cb7fec` (19 slots): variant 0 (no conditions): slot 5 is not _purecall

## Vtables not in the program

- cryptopp: 186 source classes without a primary vtable

## Strings

- cryptopp: accepted: 29
- cryptopp: no code reference: 4
- cryptopp: not in the program: 238
- cryptopp: only plain word strings, fewer than 2: 7
- cryptopp: referenced by several functions: 2
- cryptopp: referenced from code outside functions: 5
- cryptopp: short or format-only: 35
- freetype: accepted: 28
- freetype: data references: 12
- freetype: not in the program: 122
- freetype: only plain word strings, fewer than 2: 4
- freetype: referenced by several functions: 7
- freetype: referenced from code outside functions: 2
- freetype: short or format-only: 40
- freetype: unique in several libraries: 4
- tinyxml: accepted: 7
- tinyxml: data references: 2
- tinyxml: only plain word strings, fewer than 2: 1
- tinyxml: short or format-only: 19
- zlib: accepted: 3
- zlib: not in the program: 9
- zlib: short or format-only: 12
- zlib: unique in several libraries: 4

## Call graph propagation

- freetype round 1: `0x00ab5593` `FT_RoundFix`, only unnamed source callee of `T1_Set_Var_Design` `0x00ad026c`
- freetype round 1: `0x00ab56a4` `FT_MulFix`, only unnamed source callee of `T1_Set_MM_Blend` `0x00ad00e0`
- freetype round 1: `0x00ab56d9` `FT_DivFix`, only unnamed source callee of `pfr_get_metrics` `0x00ac9734`
- freetype round 1: `0x00ab592b` `FT_GlyphLoader_CreateExtra`, only unnamed source callee of `tt_slot_init` `0x00ada683`
- freetype round 1: `0x00ab5ce0` `ft_service_list_lookup`, only unnamed source callee of `pfr_get_service` `0x00ac97a6`
- freetype round 1: `0x00ab5fd3` `FT_New_GlyphSlot`, only unnamed source callee of `T42_GlyphSlot_Init` `0x00abcd3e`
- freetype round 1: `0x00ab604d` `FT_Done_GlyphSlot`, only unnamed source callee of `T42_GlyphSlot_Done` `0x00abcd78`
- freetype round 1: `0x00ab722e` `FT_Match_Size`, only unnamed source callee of `tt_face_set_sbit_strike` `0x00ae4fcc`
- freetype round 1: `0x00ab737c` `FT_Select_Metrics`, only unnamed source callee of `PCF_Size_Select` `0x00acf3d7`
- freetype round 1: `0x00ab81ca` `FT_Get_Module`, only unnamed source callee of `T42_Driver_Init` `0x00abcc57`
- freetype round 1: `0x00ab89ba` `FT_Outline_Get_CBox`, only unnamed source callee of `ft_raster1_get_cbox` `0x00ac3a1c`
- freetype round 1: `0x00ab9836` `FT_Stream_ExtractFrame`, only unnamed source callee of `tt_face_load_kern` `0x00ae37cf`
- freetype round 1: `0x00ab9c1a` `FT_Stream_ReadFields`, only unnamed source callee of `tt_face_load_os2` `0x00ae1361`
- freetype round 1: `0x00aba232` `ft_mem_alloc`, only unnamed source callee of `gray_raster_new` `0x00acdce6`
- freetype round 1: `0x00aba389` `ft_mem_free`, only unnamed source callee of `ft_black_done` `0x00ac3888`
- freetype round 1: `0x00abcd87` `t42_glyphslot_clear`, only unnamed source callee of `T42_GlyphSlot_Load` `0x00abcdda`
- freetype round 1: `0x00abf956` `cff_size_get_globals_funcs`, only unnamed source callee of `cff_size_init` `0x00abf9b1`
- freetype round 1: `0x00ac6f48` `af_loader_init`, only unnamed source callee of `af_autofitter_init` `0x00ac77b0`
- freetype round 1: `0x00ac6fb5` `af_loader_done`, only unnamed source callee of `af_autofitter_done` `0x00ac77c1`
- freetype round 1: `0x00ac76e5` `af_loader_load_glyph`, only unnamed source callee of `af_autofitter_load_glyph` `0x00ac77d0`
- freetype round 1: `0x00ac8468` `pfr_glyph_init`, only unnamed source callee of `pfr_slot_init` `0x00ac92d5`
- freetype round 1: `0x00ac8480` `pfr_glyph_done`, only unnamed source callee of `pfr_slot_done` `0x00ac92f1`
- freetype round 1: `0x00acde16` `ft_smooth_render_generic`, only unnamed source callee of `ft_smooth_render` `0x00ace06f`
- freetype round 1: `0x00ace3e0` `pcf_find_property`, only unnamed source callee of `pcf_get_bdf_property` `0x00acf642`
- freetype round 1: `0x00ad1b7b` `T1_Size_Get_Globals_Funcs`, only unnamed source callee of `T1_Size_Init` `0x00ad1bd0`
- freetype round 1: `0x00ad2a87` `T1_Get_Kerning`, only unnamed source callee of `Get_Kerning` `0x00ad21b5`
- freetype round 1: `0x00ad4e16` `bdf_get_font_property`, only unnamed source callee of `bdf_get_bdf_property` `0x00ad573e`
- freetype round 1: `0x00ad5e24` `ps_hints_done`, only unnamed source callee of `ps_hinter_done` `0x00ad7d67`
- freetype round 1: `0x00ada1be` `tt_size_done_bytecode`, only unnamed source callee of `tt_size_done` `0x00ada4fb`
- freetype round 1: `0x00ada6ed` `TT_Done_Context`, only unnamed source callee of `tt_driver_done` `0x00ada66a`
- freetype round 1: `0x00adaa9d` `TT_New_Context`, only unnamed source callee of `tt_driver_init` `0x00ada650`
- freetype round 1: `0x00ae01e5` `cid_face_open`, only unnamed source caller of `cid_parse_dict` `0x00adfcc9`
- freetype round 1: `0x00ae0353` `cid_size_get_globals_funcs`, only unnamed source callee of `cid_size_done` `0x00ae0385`
- freetype round 1: `0x00ae0f61` `tt_face_load_generic_header`, only unnamed source callee of `tt_face_load_bhed` `0x00ae0fa5`
- freetype round 1: `0x00ae504a` `find_sbit_range`, only unnamed source callee of `tt_find_sbit_image` `0x00ae5127`
- freetype round 1: `0x00ae5a56` `load_post_names`, only unnamed source callee of `tt_face_get_ps_name` `0x00ae5b7d`
- freetype round 1: `0x00ae5c9a` `tt_face_load_bdf_props`, only unnamed source callee of `tt_face_find_bdf_prop` `0x00ae5dab`
- freetype round 1: `0x00ae73ef` `ft_zlib::huft_build`, only unnamed source callee of `ft_zlib::inflate_trees_dynamic` `0x00ae7800`
- freetype round 1: `0x00ae7fd4` `ft_zlib::inflate_blocks`, only unnamed source caller of `ft_zlib::inflate_trees_dynamic` `0x00ae7800`
- freetype round 2: `0x00ab5850` `FT_GlyphLoader_Rewind`, only unnamed source callee of `pfr_glyph_init` `0x00ac8468`
- freetype round 2: `0x00ab5e95` `ft_glyphslot_free_bitmap`, only unnamed source callee of `t42_glyphslot_clear` `0x00abcd87`
- freetype round 2: `0x00ab5f80` `ft_glyphslot_done`, only unnamed source callee of `FT_Done_GlyphSlot` `0x00ab604d`
- freetype round 2: `0x00ab7094` `FT_Done_Face`, only unnamed source callee of `T42_Face_Done` `0x00abcb00`
- freetype round 2: `0x00ab7328` `ft_recompute_scaled_metrics`, only unnamed source callee of `FT_Select_Metrics` `0x00ab737c`
- freetype round 2: `0x00ab73fd` `FT_Request_Metrics`, only unnamed source callee of `cid_size_request` `0x00ae03ed`
- freetype round 2: `0x00ab8a37` `FT_Outline_Translate`, only unnamed source callee of `ft_smooth_render_generic` `0x00acde16`
- freetype round 2: `0x00ab9725` `FT_Stream_Close`, only unnamed source callee of `PCF_Face_Done` `0x00acf18e`
- freetype round 2: `0x00ab9853` `FT_Stream_ReleaseFrame`, only unnamed source callee of `tt_face_load_bdf_props` `0x00ae5c9a`
- freetype round 2: `0x00ab986b` `FT_Stream_EnterFrame`, only unnamed source callee of `FT_Stream_ExtractFrame` `0x00ab9836`
- freetype round 2: `0x00aba26c` `ft_mem_qalloc`, only unnamed source callee of `ft_mem_alloc` `0x00aba232`
- freetype round 2: `0x00aba296` `ft_mem_realloc`, only unnamed source callee of `ps_unicodes_init` `0x00ad2d2e`
- freetype round 2: `0x00abeb19` `cff_index_get_sid_string`, only unnamed source callee of `cff_ps_get_font_info` `0x00abde82`
- freetype round 2: `0x00abf7da` `cff_font_done`, only unnamed source callee of `cff_face_done` `0x00ac028a`
- freetype round 2: `0x00ac7f91` `pfr_phy_font_done`, only unnamed source callee of `pfr_face_done` `0x00ac8fdf`
- freetype round 2: `0x00ad2f36` `hash_lookup`, only unnamed source callee of `bdf_get_font_property` `0x00ad4e16`
- freetype round 2: `0x00ad4c3e` `bdf_free_font`, only unnamed source callee of `BDF_Face_Done` `0x00ad50d4`
- freetype round 2: `0x00ad5c2e` `ps_dimension_done`, only unnamed source callee of `ps_hints_done` `0x00ad5e24`
- freetype round 2: `0x00ad9ca4` `tt_glyphzone_done`, only unnamed source callee of `tt_size_done_bytecode` `0x00ada1be`
- freetype round 2: `0x00ada263` `tt_size_init_bytecode`, only unnamed source caller of `tt_size_done_bytecode` `0x00ada1be`
- freetype round 2: `0x00ada514` `tt_size_reset`, only unnamed source callee of `tt_size_select` `0x00ad7e1b`
- freetype round 2: `0x00ada75d` `Init_Context`, only unnamed source callee of `TT_New_Context` `0x00adaa9d`
- freetype round 2: `0x00ae5493` `Load_SBit_Image`, only unnamed source caller of `find_sbit_range` `0x00ae504a`
- freetype round 2: `0x00ae5f60` `fnt_font_done`, only unnamed source callee of `FNT_Face_Done` `0x00ae6693`
- freetype round 2: `0x00ae88a2` `ft_zlib::inflate`, only unnamed source caller of `ft_zlib::inflate_blocks` `0x00ae7fd4`
- freetype round 3: `0x00ab58cb` `FT_GlyphLoader_Done`, only unnamed source callee of `ft_glyphslot_done` `0x00ab5f80`
- freetype round 3: `0x00ab58e6` `FT_GlyphLoader_Adjust_Points`, only unnamed source callee of `FT_GlyphLoader_CreateExtra` `0x00ab592b`
- freetype round 3: `0x00ab5e2a` `ft_glyphslot_init`, only unnamed source callee of `FT_New_GlyphSlot` `0x00ab5fd3`
- freetype round 3: `0x00ab72f5` `ft_synthesize_vertical_metrics`, only unnamed source callee of `FNT_Load_Glyph` `0x00ae6a0d`
- freetype round 3: `0x00ab8c00` `FT_Outline_Transform`, only unnamed source callee of `ft_smooth_transform` `0x00acdda7`
- freetype round 3: `0x00ab98fc` `FT_Stream_ExitFrame`, only unnamed source callee of `FT_Stream_ReadFields` `0x00ab9c1a`
- freetype round 3: `0x00aba2fb` `ft_mem_qrealloc`, only unnamed source callee of `ft_mem_realloc` `0x00aba296`
- freetype round 3: `0x00ad305b` `hash_free`, only unnamed source callee of `bdf_free_font` `0x00ad4c3e`
- freetype round 3: `0x00ada3f7` `tt_size_ready_bytecode`, only unnamed source caller of `tt_size_init_bytecode` `0x00ada263`
- freetype round 3: `0x00ae6063` `fnt_face_get_dll_font`, only unnamed source caller of `fnt_font_done` `0x00ae5f60`
- freetype round 3: `0x00ae7ef3` `ft_zlib::inflate_blocks_reset`, only unnamed source callee of `ft_zlib::inflate` `0x00ae88a2`
- freetype round 4: `0x00ab582b` `FT_GlyphLoader_New`, only unnamed source callee of `ft_glyphslot_init` `0x00ab5e2a`
- freetype round 4: `0x00ab586c` `FT_GlyphLoader_Reset`, only unnamed source callee of `FT_GlyphLoader_Done` `0x00ab58cb`
- freetype round 4: `0x00ab5ec9` `ft_glyphslot_set_bitmap`, only unnamed source callee of `BDF_Glyph_Load` `0x00ad5639`
- freetype round 4: `0x00ab8bb3` `FT_Vector_Transform`, only unnamed source callee of `FT_Outline_Transform` `0x00ab8c00`
- freetype round 4: `0x00ac41c0` `af_glyph_hints_done`, only unnamed source callee of `af_loader_done` `0x00ac6fb5`
- freetype round 4: `0x00ada0f5` `tt_size_run_prep`, only unnamed source callee of `tt_size_ready_bytecode` `0x00ada3f7`
- freetype round 5: `0x00ad21e1` `T1_Parse_Glyph_And_Get_Char_String`, only unnamed source callee of `T1_Load_Glyph` `0x00ad2338`
- freetype round 5: `0x00ae07b9` `cid_load_glyph`, only unnamed source callee of `cid_slot_load_glyph` `0x00ae0978`

## Names from earlier runs (call graph seeds)

None.

## Conflicts (not named)

None.

## Existing names kept (disagreements)

None.

## Anomalies

None.

## ZLIB_STRING (1)

- `inflate` `0x009a5d80`: 3 unique strings of `inflate` (inflate.c): "header crc mismatch", "incorrect length check", "unknown header flags set"

## FREETYPE_TABLE (196)

- `ft_stub_set_char_sizes` `0x00ab845c`: `bdf_driver_class` +0x48 at `0x00c83210` (src/bdf/bdfdrivr.c)
- `ft_stub_set_pixel_sizes` `0x00ab84b7`: `bdf_driver_class` +0x4c at `0x00c83210` (src/bdf/bdfdrivr.c)
- `T42_Face_Init` `0x00abc5cd`: `t42_driver_class` +0x30 at `0x00c94758` (src/type42/t42drivr.c)
- `T42_Face_Done` `0x00abcb00`: `t42_driver_class` +0x34 at `0x00c94758` (src/type42/t42drivr.c)
- `T42_Driver_Init` `0x00abcc57`: `t42_driver_class` +0x18 at `0x00c94758` (src/type42/t42drivr.c)
- `T42_Driver_Done` `0x00abcc73`: `t42_driver_class` +0x1c at `0x00c94758` (src/type42/t42drivr.c)
- `T42_Size_Init` `0x00abcc74`: `t42_driver_class` +0x38 at `0x00c94758` (src/type42/t42drivr.c)
- `T42_Size_Request` `0x00abcca1`: `t42_driver_class` +0x60 at `0x00c94758` (src/type42/t42drivr.c)
- `T42_Size_Select` `0x00abccdb`: `t42_driver_class` +0x64 at `0x00c94758` (src/type42/t42drivr.c)
- `T42_Size_Done` `0x00abcd13`: `t42_driver_class` +0x3c at `0x00c94758` (src/type42/t42drivr.c)
- `T42_GlyphSlot_Init` `0x00abcd3e`: `t42_driver_class` +0x40 at `0x00c94758` (src/type42/t42drivr.c)
- `T42_GlyphSlot_Done` `0x00abcd78`: `t42_driver_class` +0x44 at `0x00c94758` (src/type42/t42drivr.c)
- `T42_GlyphSlot_Load` `0x00abcdda`: `t42_driver_class` +0x50 at `0x00c94758` (src/type42/t42drivr.c)
- `t42_get_glyph_name` `0x00abdbd4`: `t42_service_glyph_dict` +0x0 at `0x00c94a6c` (src/type42/t42drivr.c)
- `t42_get_name_index` `0x00abdbf6`: `t42_service_glyph_dict` +0x4 at `0x00c94a6c` (src/type42/t42drivr.c)
- `t42_ps_get_font_info` `0x00abdc7f`: `t42_service_ps_info` +0x0 at `0x00c94a78` (src/type42/t42drivr.c)
- `t42_ps_has_glyph_names` `0x00abdc99`: `t42_service_ps_info` +0x4 at `0x00c94a78` (src/type42/t42drivr.c)
- `t42_ps_get_font_private` `0x00abdc9d`: `t42_service_ps_info` +0x8 at `0x00c94a78` (src/type42/t42drivr.c)
- `T42_Get_Interface` `0x00abdcb7`: `t42_driver_class` +0x20 at `0x00c94758` (src/type42/t42drivr.c)
- `cff_get_kerning` `0x00abdcc7`: `cff_driver_class` +0x54 at `0x00c938d0` (src/cff/cffdrivr.c)
- `Load_Glyph` `0x00abdcf9`: `cff_driver_class` +0x50 at `0x00c938d0` (src/cff/cffdrivr.c)
- `cff_get_glyph_name` `0x00abdd40`: `cff_service_glyph_dict` +0x0 at `0x00c93938` (src/cff/cffdrivr.c)
- `cff_get_name_index` `0x00abdda9`: `cff_service_glyph_dict` +0x4 at `0x00c93938` (src/cff/cffdrivr.c)
- `cff_ps_has_glyph_names` `0x00abde70`: `cff_service_ps_info` +0x4 at `0x00c938c4` (src/cff/cffdrivr.c)
- `cff_ps_get_font_info` `0x00abde82`: `cff_service_ps_info` +0x0 at `0x00c938c4` (src/cff/cffdrivr.c)
- `cff_get_interface` `0x00abdfda`: `cff_driver_class` +0x20 at `0x00c938d0` (src/cff/cffdrivr.c)
- `cff_size_done` `0x00abf98e`: `cff_driver_class` +0x3c at `0x00c938d0` (src/cff/cffdrivr.c)
- `cff_size_init` `0x00abf9b1`: `cff_driver_class` +0x38 at `0x00c938d0` (src/cff/cffdrivr.c)
- `cff_size_select` `0x00abfb4d`: `cff_driver_class` +0x64 at `0x00c938d0` (src/cff/cffdrivr.c)
- `cff_size_request` `0x00abfb84`: `cff_driver_class` +0x60 at `0x00c938d0` (src/cff/cffdrivr.c)
- `cff_slot_done` `0x00abfbee`: `cff_driver_class` +0x44 at `0x00c938d0` (src/cff/cffdrivr.c)
- `cff_slot_init` `0x00abfbfd`: `cff_driver_class` +0x40 at `0x00c938d0` (src/cff/cffdrivr.c)
- `cff_face_init` `0x00abfc56`: `cff_driver_class` +0x30 at `0x00c938d0` (src/cff/cffdrivr.c)
- `cff_face_done` `0x00ac028a`: `cff_driver_class` +0x34 at `0x00c938d0` (src/cff/cffdrivr.c)
- `cff_driver_init` `0x00ac02c3`: `cff_driver_class` +0x18 at `0x00c938d0` (src/cff/cffdrivr.c)
- `cff_driver_done` `0x00ac02c6`: `cff_driver_class` +0x1c at `0x00c938d0` (src/cff/cffdrivr.c)
- `ft_black_new` `0x00ac3859`: `ft_standard_raster` +0x4 at `0x00c93734` (src/raster/ftraster.c)
- `ft_black_done` `0x00ac3888`: `ft_standard_raster` +0x14 at `0x00c93734` (src/raster/ftraster.c)
- `ft_black_reset` `0x00ac3897`: `ft_standard_raster` +0x8 at `0x00c93734` (src/raster/ftraster.c)
- `ft_black_set_mode` `0x00ac38d7`: `ft_standard_raster` +0xc at `0x00c93734` (src/raster/ftraster.c)
- `ft_black_render` `0x00ac38d8`: `ft_standard_raster` +0x10 at `0x00c93734` (src/raster/ftraster.c)
- `ft_raster1_init` `0x00ac3995`: `ft_raster1_renderer_class` +0x18 at `0x00c9374c` (src/raster/ftrend1.c)
- `ft_raster1_set_mode` `0x00ac39ba`: `ft_raster1_renderer_class` +0x34 at `0x00c9374c` (src/raster/ftrend1.c)
- `ft_raster1_transform` `0x00ac39d6`: `ft_raster1_renderer_class` +0x2c at `0x00c9374c` (src/raster/ftrend1.c)
- `ft_raster1_get_cbox` `0x00ac3a1c`: `ft_raster1_renderer_class` +0x30 at `0x00c9374c` (src/raster/ftrend1.c)
- `ft_raster1_render` `0x00ac3a45`: `ft_raster1_renderer_class` +0x28 at `0x00c9374c` (src/raster/ftrend1.c)
- `af_autofitter_init` `0x00ac77b0`: `autofit_module_class` +0x18 at `0x00c934f0` (src/autofit/afmodule.c)
- `af_autofitter_done` `0x00ac77c1`: `autofit_module_class` +0x1c at `0x00c934f0` (src/autofit/afmodule.c)
- `af_autofitter_load_glyph` `0x00ac77d0`: `af_autofitter_service` +0xc at `0x00c934e0` (src/autofit/afmodule.c)
- `pfr_face_done` `0x00ac8fdf`: `pfr_driver_class` +0x34 at `0x00c933b8` (src/pfr/pfrdrivr.c)
- `pfr_face_init` `0x00ac9016`: `pfr_driver_class` +0x30 at `0x00c933b8` (src/pfr/pfrdrivr.c)
- `pfr_slot_init` `0x00ac92d5`: `pfr_driver_class` +0x40 at `0x00c933b8` (src/pfr/pfrdrivr.c)
- `pfr_slot_done` `0x00ac92f1`: `pfr_driver_class` +0x44 at `0x00c933b8` (src/pfr/pfrdrivr.c)
- `pfr_slot_load` `0x00ac9303`: `pfr_driver_class` +0x50 at `0x00c933b8` (src/pfr/pfrdrivr.c)
- `pfr_face_get_kerning` `0x00ac94d3`: `pfr_metrics_service_rec` +0x4 at `0x00c934bc` (src/pfr/pfrdrivr.c)
- `pfr_get_kerning` `0x00ac96a3`: `pfr_driver_class` +0x54 at `0x00c933b8` (src/pfr/pfrdrivr.c)
- `pfr_get_advance` `0x00ac9700`: `pfr_metrics_service_rec` +0x8 at `0x00c934bc` (src/pfr/pfrdrivr.c)
- `pfr_get_metrics` `0x00ac9734`: `pfr_metrics_service_rec` +0x0 at `0x00c934bc` (src/pfr/pfrdrivr.c)
- `pfr_get_service` `0x00ac97a6`: `pfr_driver_class` +0x20 at `0x00c933b8` (src/pfr/pfrdrivr.c)
- `gray_raster_render` `0x00acdb4d`: `ft_grays_raster` +0x10 at `0x00c92f90` (src/smooth/ftgrays.c)
- `gray_raster_new` `0x00acdce6`: `ft_grays_raster` +0x4 at `0x00c92f90` (src/smooth/ftgrays.c)
- `gray_raster_done` `0x00acdd15`: `ft_grays_raster` +0x14 at `0x00c92f90` (src/smooth/ftgrays.c)
- `gray_raster_reset` `0x00acdd24`: `ft_grays_raster` +0x8 at `0x00c92f90` (src/smooth/ftgrays.c)
- `ft_smooth_init` `0x00acdd66`: `ft_smooth_renderer_class` +0x18 at `0x00c92fa8` (src/smooth/ftsmooth.c)
- `ft_smooth_set_mode` `0x00acdd8b`: `ft_smooth_renderer_class` +0x34 at `0x00c92fa8` (src/smooth/ftsmooth.c)
- `ft_smooth_transform` `0x00acdda7`: `ft_smooth_renderer_class` +0x2c at `0x00c92fa8` (src/smooth/ftsmooth.c)
- `ft_smooth_get_cbox` `0x00acdded`: `ft_smooth_renderer_class` +0x30 at `0x00c92fa8` (src/smooth/ftsmooth.c)
- `ft_smooth_render` `0x00ace06f`: `ft_smooth_renderer_class` +0x28 at `0x00c92fa8` (src/smooth/ftsmooth.c)
- `ft_smooth_render_lcd` `0x00ace091`: `ft_smooth_lcd_renderer_class` +0x28 at `0x00c92fe4` (src/smooth/ftsmooth.c)
- `ft_smooth_render_lcd_v` `0x00ace0b7`: `ft_smooth_lcdv_renderer_class` +0x28 at `0x00c93020` (src/smooth/ftsmooth.c)
- `PCF_Face_Done` `0x00acf18e`: `pcf_driver_class` +0x34 at `0x00c92de0` (src/pcf/pcfdrivr.c)
- `PCF_Face_Init` `0x00acf2b4`: `pcf_driver_class` +0x30 at `0x00c92de0` (src/pcf/pcfdrivr.c)
- `PCF_Size_Select` `0x00acf3d7`: `pcf_driver_class` +0x64 at `0x00c92de0` (src/pcf/pcfdrivr.c)
- `PCF_Size_Request` `0x00acf417`: `pcf_driver_class` +0x60 at `0x00c92de0` (src/pcf/pcfdrivr.c)
- `PCF_Glyph_Load` `0x00acf48e`: `pcf_driver_class` +0x50 at `0x00c92de0` (src/pcf/pcfdrivr.c)
- `pcf_get_bdf_property` `0x00acf642`: `pcf_service_bdf` +0x4 at `0x00c92f70` (src/pcf/pcfdrivr.c)
- `pcf_get_charset_id` `0x00acf679`: `pcf_service_bdf` +0x0 at `0x00c92f70` (src/pcf/pcfdrivr.c)
- `pcf_driver_requester` `0x00acf698`: `pcf_driver_class` +0x20 at `0x00c92de0` (src/pcf/pcfdrivr.c)
- `T1_Get_Multi_Master` `0x00acfd86`: `t1_service_multi_masters` +0x0 at `0x00c92d84` (src/type1/t1driver.c)
- `T1_Get_MM_Var` `0x00acff38`: `t1_service_multi_masters` +0xc at `0x00c92d84` (src/type1/t1driver.c)
- `T1_Set_MM_Blend` `0x00ad00e0`: `t1_service_multi_masters` +0x8 at `0x00c92d84` (src/type1/t1driver.c)
- `T1_Set_MM_Design` `0x00ad016c`: `t1_service_multi_masters` +0x4 at `0x00c92d84` (src/type1/t1driver.c)
- `T1_Set_Var_Design` `0x00ad026c`: `t1_service_multi_masters` +0x10 at `0x00c92d84` (src/type1/t1driver.c)
- `T1_Size_Done` `0x00ad1bad`: `t1_driver_class` +0x3c at `0x00c926c0` (src/type1/t1driver.c)
- `T1_Size_Init` `0x00ad1bd0`: `t1_driver_class` +0x38 at `0x00c926c0` (src/type1/t1driver.c)
- `T1_Size_Request` `0x00ad1c0d`: `t1_driver_class` +0x60 at `0x00c926c0` (src/type1/t1driver.c)
- `T1_GlyphSlot_Done` `0x00ad1c45`: `t1_driver_class` +0x44 at `0x00c926c0` (src/type1/t1driver.c)
- `T1_GlyphSlot_Init` `0x00ad1c54`: `t1_driver_class` +0x40 at `0x00c926c0` (src/type1/t1driver.c)
- `T1_Face_Done` `0x00ad1c8f`: `t1_driver_class` +0x34 at `0x00c926c0` (src/type1/t1driver.c)
- `T1_Face_Init` `0x00ad1dff`: `t1_driver_class` +0x30 at `0x00c926c0` (src/type1/t1driver.c)
- `T1_Driver_Init` `0x00ad20e8`: `t1_driver_class` +0x18 at `0x00c926c0` (src/type1/t1driver.c)
- `T1_Driver_Done` `0x00ad20eb`: `t1_driver_class` +0x1c at `0x00c926c0` (src/type1/t1driver.c)
- `t1_get_glyph_name` `0x00ad20ec`: `t1_service_glyph_dict` +0x0 at `0x00c92d7c` (src/type1/t1driver.c)
- `t1_get_name_index` `0x00ad210e`: `t1_service_glyph_dict` +0x4 at `0x00c92d7c` (src/type1/t1driver.c)
- `t1_ps_get_font_info` `0x00ad216d`: `t1_service_ps_info` +0x0 at `0x00c92d98` (src/type1/t1driver.c)
- `t1_ps_has_glyph_names` `0x00ad2187`: `t1_service_ps_info` +0x4 at `0x00c92d98` (src/type1/t1driver.c)
- `t1_ps_get_font_private` `0x00ad218b`: `t1_service_ps_info` +0x8 at `0x00c92d98` (src/type1/t1driver.c)
- `Get_Interface` `0x00ad21a5`: `t1_driver_class` +0x20 at `0x00c926c0` (src/type1/t1driver.c)
- `Get_Kerning` `0x00ad21b5`: `t1_driver_class` +0x54 at `0x00c926c0` (src/type1/t1driver.c)
- `T1_Load_Glyph` `0x00ad2338`: `t1_driver_class` +0x50 at `0x00c926c0` (src/type1/t1driver.c)
- `T1_Read_Metrics` `0x00ad28d5`: `t1_driver_class` +0x58 at `0x00c926c0` (src/type1/t1driver.c)
- `ps_unicode_value` `0x00ad2c29`: `pscmaps_interface` +0x0 at `0x00c83d00` (src/psnames/psmodule.c)
- `ps_unicodes_init` `0x00ad2d2e`: `pscmaps_interface` +0x4 at `0x00c83d00` (src/psnames/psmodule.c)
- `ps_unicodes_char_index` `0x00ad2e0c`: `pscmaps_interface` +0x8 at `0x00c83d00` (src/psnames/psmodule.c)
- `ps_unicodes_char_next` `0x00ad2e69`: `pscmaps_interface` +0xc at `0x00c83d00` (src/psnames/psmodule.c)
- `ps_get_macintosh_name` `0x00ad2eef`: `pscmaps_interface` +0x10 at `0x00c83d00` (src/psnames/psmodule.c)
- `ps_get_standard_strings` `0x00ad2f0a`: `pscmaps_interface` +0x14 at `0x00c83d00` (src/psnames/psmodule.c)
- `psnames_get_service` `0x00ad2f26`: `psnames_module_class` +0x20 at `0x00c838dc` (src/psnames/psmodule.c)
- `BDF_Face_Done` `0x00ad50d4`: `bdf_driver_class` +0x34 at `0x00c83210` (src/bdf/bdfdrivr.c)
- `BDF_Face_Init` `0x00ad5161`: `bdf_driver_class` +0x30 at `0x00c83210` (src/bdf/bdfdrivr.c)
- `BDF_Size_Select` `0x00ad5587`: `bdf_driver_class` +0x64 at `0x00c83210` (src/bdf/bdfdrivr.c)
- `BDF_Size_Request` `0x00ad55c2`: `bdf_driver_class` +0x60 at `0x00c83210` (src/bdf/bdfdrivr.c)
- `BDF_Glyph_Load` `0x00ad5639`: `bdf_driver_class` +0x50 at `0x00c83210` (src/bdf/bdfdrivr.c)
- `bdf_get_bdf_property` `0x00ad573e`: `bdf_service_bdf` +0x4 at `0x00c838bc` (src/bdf/bdfdrivr.c)
- `bdf_get_charset_id` `0x00ad5792`: `bdf_service_bdf` +0x0 at `0x00c838bc` (src/bdf/bdfdrivr.c)
- `bdf_driver_requester` `0x00ad57b1`: `bdf_driver_class` +0x20 at `0x00c83210` (src/bdf/bdfdrivr.c)
- `ps_hinter_done` `0x00ad7d67`: `pshinter_module_class` +0x1c at `0x00c831ec` (src/pshinter/pshmod.c)
- `ps_hinter_init` `0x00ad7d81`: `pshinter_module_class` +0x18 at `0x00c831ec` (src/pshinter/pshmod.c)
- `tt_get_kerning` `0x00ad7de9`: `tt_driver_class` +0x54 at `0x00c82e80` (src/truetype/ttdriver.c)
- `tt_size_select` `0x00ad7e1b`: `tt_driver_class` +0x64 at `0x00c82e80` (src/truetype/ttdriver.c)
- `tt_size_request` `0x00ad7e66`: `tt_driver_class` +0x60 at `0x00c82e80` (src/truetype/ttdriver.c)
- `Load_Glyph` `0x00ad7ec9`: `tt_driver_class` +0x50 at `0x00c82e80` (src/truetype/ttdriver.c)
- `tt_get_interface` `0x00ad7f0b`: `tt_driver_class` +0x20 at `0x00c82e80` (src/truetype/ttdriver.c)
- `tt_face_init` `0x00ad9dd2`: `tt_driver_class` +0x30 at `0x00c82e80` (src/truetype/ttdriver.c)
- `tt_face_done` `0x00ad9f5a`: `tt_driver_class` +0x34 at `0x00c82e80` (src/truetype/ttdriver.c)
- `tt_size_init` `0x00ada4df`: `tt_driver_class` +0x38 at `0x00c82e80` (src/truetype/ttdriver.c)
- `tt_size_done` `0x00ada4fb`: `tt_driver_class` +0x3c at `0x00c82e80` (src/truetype/ttdriver.c)
- `tt_driver_init` `0x00ada650`: `tt_driver_class` +0x18 at `0x00c82e80` (src/truetype/ttdriver.c)
- `tt_driver_done` `0x00ada66a`: `tt_driver_class` +0x1c at `0x00c82e80` (src/truetype/ttdriver.c)
- `tt_slot_init` `0x00ada683`: `tt_driver_class` +0x40 at `0x00c82e80` (src/truetype/ttdriver.c)
- `TT_Get_MM_Var` `0x00ade9b0`: `tt_service_gx_multi_masters` +0xc at `0x00c82e6c` (src/truetype/ttdriver.c)
- `TT_Set_MM_Blend` `0x00aded25`: `tt_service_gx_multi_masters` +0x8 at `0x00c82e6c` (src/truetype/ttdriver.c)
- `TT_Set_Var_Design` `0x00adee71`: `tt_service_gx_multi_masters` +0x10 at `0x00c82e6c` (src/truetype/ttdriver.c)
- `cid_slot_done` `0x00ae0309`: `t1cid_driver_class` +0x44 at `0x00c826f0` (src/cid/cidriver.c)
- `cid_slot_init` `0x00ae0318`: `t1cid_driver_class` +0x40 at `0x00c826f0` (src/cid/cidriver.c)
- `cid_size_done` `0x00ae0385`: `t1cid_driver_class` +0x3c at `0x00c826f0` (src/cid/cidriver.c)
- `cid_size_init` `0x00ae03a8`: `t1cid_driver_class` +0x38 at `0x00c826f0` (src/cid/cidriver.c)
- `cid_size_request` `0x00ae03ed`: `t1cid_driver_class` +0x60 at `0x00c826f0` (src/cid/cidriver.c)
- `cid_face_done` `0x00ae0421`: `t1cid_driver_class` +0x34 at `0x00c826f0` (src/cid/cidriver.c)
- `cid_face_init` `0x00ae0589`: `t1cid_driver_class` +0x30 at `0x00c826f0` (src/cid/cidriver.c)
- `cid_driver_init` `0x00ae0776`: `t1cid_driver_class` +0x18 at `0x00c826f0` (src/cid/cidriver.c)
- `cid_driver_done` `0x00ae0779`: `t1cid_driver_class` +0x1c at `0x00c826f0` (src/cid/cidriver.c)
- `cid_ps_get_font_info` `0x00ae078f`: `cid_service_ps_info` +0x0 at `0x00c82e40` (src/cid/cidriver.c)
- `cid_get_interface` `0x00ae07a9`: `t1cid_driver_class` +0x20 at `0x00c826f0` (src/cid/cidriver.c)
- `cid_slot_load_glyph` `0x00ae0978`: `t1cid_driver_class` +0x50 at `0x00c826f0` (src/cid/cidriver.c)
- `tt_face_goto_table` `0x00ae0c82`: `sfnt_interface` +0x0 at `0x00c822a0` (src/sfnt/sfdriver.c)
- `tt_face_load_font_dir` `0x00ae0dfa`: `sfnt_interface` +0x84 at `0x00c822a0` (src/sfnt/sfdriver.c)
- `tt_face_load_any` `0x00ae0f00`: `sfnt_service_sfnt_table` +0x0 at `0x00c825f4` (src/sfnt/sfdriver.c)
- `tt_face_load_head` `0x00ae0f79`: `sfnt_interface` +0x20 at `0x00c822a0` (src/sfnt/sfdriver.c)
- `tt_face_load_bhed` `0x00ae0fa5`: `sfnt_interface` +0x54 at `0x00c822a0` (src/sfnt/sfdriver.c)
- `tt_face_load_maxp` `0x00ae0fd1`: `sfnt_interface` +0x2c at `0x00c822a0` (src/sfnt/sfdriver.c)
- `tt_face_load_name` `0x00ae116d`: `sfnt_interface` +0x38 at `0x00c822a0` (src/sfnt/sfdriver.c)
- `tt_face_free_name` `0x00ae12aa`: `sfnt_interface` +0x3c at `0x00c822a0` (src/sfnt/sfdriver.c)
- `tt_face_load_cmap` `0x00ae131e`: `sfnt_interface` +0x28 at `0x00c822a0` (src/sfnt/sfdriver.c)
- `tt_face_load_os2` `0x00ae1361`: `sfnt_interface` +0x30 at `0x00c822a0` (src/sfnt/sfdriver.c)
- `tt_face_load_post` `0x00ae16e3`: `sfnt_interface` +0x34 at `0x00c822a0` (src/sfnt/sfdriver.c)
- `tt_face_load_pclt` `0x00ae171b`: `sfnt_interface` +0x50 at `0x00c822a0` (src/sfnt/sfdriver.c)
- `tt_face_load_gasp` `0x00ae1753`: `sfnt_interface` +0x4c at `0x00c822a0` (src/sfnt/sfdriver.c)
- `tt_face_load_hmtx` `0x00ae183f`: `sfnt_interface` +0x88 at `0x00c822a0` (src/sfnt/sfdriver.c)
- `tt_face_load_hhea` `0x00ae1a38`: `sfnt_interface` +0x24 at `0x00c822a0` (src/sfnt/sfdriver.c)
- `tt_face_get_metrics` `0x00ae1ba3`: `sfnt_interface` +0x9c at `0x00c822a0` (src/sfnt/sfdriver.c)
- `tt_face_load_kern` `0x00ae37cf`: `sfnt_interface` +0x48 at `0x00c822a0` (src/sfnt/sfdriver.c)
- `tt_face_get_kerning` `0x00ae39b7`: `sfnt_interface` +0x80 at `0x00c822a0` (src/sfnt/sfdriver.c)
- `sfnt_init_face` `0x00ae3ef6`: `sfnt_interface` +0x4 at `0x00c822a0` (src/sfnt/sfdriver.c)
- `sfnt_load_face` `0x00ae3fa1`: `sfnt_interface` +0x8 at `0x00c822a0` (src/sfnt/sfdriver.c)
- `sfnt_done_face` `0x00ae4431`: `sfnt_interface` +0xc at `0x00c822a0` (src/sfnt/sfdriver.c)
- `get_sfnt_table` `0x00ae4582`: `sfnt_service_sfnt_table` +0x4 at `0x00c825f4` (src/sfnt/sfdriver.c)
- `sfnt_table_info` `0x00ae4607`: `sfnt_service_sfnt_table` +0x8 at `0x00c825f4` (src/sfnt/sfdriver.c)
- `sfnt_get_glyph_name` `0x00ae4654`: `sfnt_service_glyph_dict` +0x0 at `0x00c82600` (src/sfnt/sfdriver.c)
- `sfnt_get_charset_id` `0x00ae4881`: `sfnt_service_bdf` +0x0 at `0x00c82610` (src/sfnt/sfdriver.c)
- `sfnt_get_interface` `0x00ae48da`: `sfnt_interface` +0x10 at `0x00c822a0` (src/sfnt/sfdriver.c)
- `tt_face_load_sfnt_header_stub` `0x00ae48ea`: `sfnt_interface` +0x18 at `0x00c822a0` (src/sfnt/sfdriver.c)
- `tt_face_load_directory_stub` `0x00ae48ee`: `sfnt_interface` +0x1c at `0x00c822a0` (src/sfnt/sfdriver.c)
- `tt_face_load_hdmx_stub` `0x00ae48f2`: `sfnt_interface` +0x40 at `0x00c822a0` (src/sfnt/sfdriver.c)
- `tt_face_free_hdmx_stub` `0x00ae48f6`: `sfnt_interface` +0x44 at `0x00c822a0` (src/sfnt/sfdriver.c)
- `tt_face_set_sbit_strike_stub` `0x00ae48f7`: `sfnt_interface` +0x58 at `0x00c822a0` (src/sfnt/sfdriver.c)
- `tt_face_load_sbit_stub` `0x00ae492f`: `sfnt_interface` +0x5c at `0x00c822a0` (src/sfnt/sfdriver.c)
- `tt_face_free_sbit_stub` `0x00ae4933`: `sfnt_interface` +0x6c at `0x00c822a0` (src/sfnt/sfdriver.c)
- `tt_face_load_charmap_stub` `0x00ae4934`: `sfnt_interface` +0x78 at `0x00c822a0` (src/sfnt/sfdriver.c)
- `tt_face_free_charmap_stub` `0x00ae4938`: `sfnt_interface` +0x7c at `0x00c822a0` (src/sfnt/sfdriver.c)
- `tt_face_load_eblc` `0x00ae4c97`: `sfnt_interface` +0x8c at `0x00c822a0` (src/sfnt/sfdriver.c)
- `tt_face_free_eblc` `0x00ae4f26`: `sfnt_interface` +0x90 at `0x00c822a0` (src/sfnt/sfdriver.c)
- `tt_face_set_sbit_strike` `0x00ae4fcc`: `sfnt_interface` +0x94 at `0x00c822a0` (src/sfnt/sfdriver.c)
- `tt_face_load_strike_metrics` `0x00ae4fe0`: `sfnt_interface` +0x98 at `0x00c822a0` (src/sfnt/sfdriver.c)
- `tt_find_sbit_image` `0x00ae5127`: `sfnt_interface` +0x60 at `0x00c822a0` (src/sfnt/sfdriver.c)
- `tt_load_sbit_metrics` `0x00ae5181`: `sfnt_interface` +0x64 at `0x00c822a0` (src/sfnt/sfdriver.c)
- `tt_face_load_sbit_image` `0x00ae570e`: `sfnt_interface` +0x68 at `0x00c822a0` (src/sfnt/sfdriver.c)
- `tt_face_free_ps_names` `0x00ae5ab9`: `sfnt_interface` +0x74 at `0x00c822a0` (src/sfnt/sfdriver.c)
- `tt_face_get_ps_name` `0x00ae5b7d`: `sfnt_interface` +0x70 at `0x00c822a0` (src/sfnt/sfdriver.c)
- `tt_face_find_bdf_prop` `0x00ae5dab`: `sfnt_service_bdf` +0x4 at `0x00c82610` (src/sfnt/sfdriver.c)
- `FNT_Face_Done` `0x00ae6693`: `winfnt_driver_class` +0x34 at `0x00c820c8` (src/winfonts/winfnt.c)
- `FNT_Face_Init` `0x00ae66bd`: `winfnt_driver_class` +0x30 at `0x00c820c8` (src/winfonts/winfnt.c)
- `FNT_Size_Select` `0x00ae6956`: `winfnt_driver_class` +0x64 at `0x00c820c8` (src/winfonts/winfnt.c)
- `FNT_Size_Request` `0x00ae6998`: `winfnt_driver_class` +0x60 at `0x00c820c8` (src/winfonts/winfnt.c)
- `FNT_Load_Glyph` `0x00ae6a0d`: `winfnt_driver_class` +0x50 at `0x00c820c8` (src/winfonts/winfnt.c)
- `winfnt_get_service` `0x00ae6bca`: `winfnt_driver_class` +0x20 at `0x00c820c8` (src/winfonts/winfnt.c)

## FREETYPE_STRING (10)

- `raccess_guess_linux_netatalk` `0x00ab94d5`: 1 unique string of `raccess_guess_linux_netatalk` (src/base/ftrfork.c): ".AppleDouble/"
- `t42_parser_init` `0x00abce91`: 1 unique string of `t42_parser_init` (src/type42/t42parse.c): "%!PS-TrueTypeFont"
- `T1_New_Parser` `0x00acf774`: 2 unique strings of `T1_New_Parser` (src/type1/t1parse.c): "%!FontType", "%!PS-AdobeFont"
- `_bdf_parse_glyphs` `0x00ad3ca6`: 7 unique strings of `_bdf_parse_glyphs` (src/bdf/bdflib.c): "BITMAP", "DWIDTH", "ENCODING", "ENDCHAR", "ENDFONT", "STARTCHAR", "SWIDTH"
- `_bdf_parse_properties` `0x00ad4418`: 2 unique strings of `_bdf_parse_properties` (src/bdf/bdflib.c): "ENDPROPERTIES", "_XFREE86_GLYPH_RANGES"
- `_bdf_parse_start` `0x00ad45c6`: 3 unique strings of `_bdf_parse_start` (src/bdf/bdflib.c): "FONTBOUNDINGBOX", "STARTFONT", "STARTPROPERTIES"
- `cid_parser_new` `0x00adf830`: 3 unique strings of `cid_parser_new` (src/cid/cidparse.c): "%!PS-Adobe-3.0 Resource-CIDFont", "/sfnts", "StartData"
- `cid_parse_dict` `0x00adfcc9`: 1 unique string of `cid_parse_dict` (src/cid/cidload.c): "%ADOBeginFontDict"
- `ft_zlib::inflate_trees_bits` `0x00ae7784`: 2 unique strings of `ft_zlib::inflate_trees_bits` (src/gzip/inftrees.c): "incomplete dynamic bit lengths tree", "oversubscribed dynamic bit lengths tree"
- `ft_zlib::inflate_trees_dynamic` `0x00ae7800`: 5 unique strings of `ft_zlib::inflate_trees_dynamic` (src/gzip/inftrees.c): "empty distance tree with lengths", "incomplete distance tree", "incomplete literal/length tree", "oversubscribed distance tree", "oversubscribed literal/length tree"

## FREETYPE_CALLGRAPH (83)

- `FT_RoundFix` `0x00ab5593`: only unnamed source callee of `T1_Set_Var_Design` `0x00ad026c`
- `FT_MulFix` `0x00ab56a4`: only unnamed source callee of `T1_Set_MM_Blend` `0x00ad00e0`
- `FT_DivFix` `0x00ab56d9`: only unnamed source callee of `pfr_get_metrics` `0x00ac9734`
- `FT_GlyphLoader_New` `0x00ab582b`: only unnamed source callee of `ft_glyphslot_init` `0x00ab5e2a`
- `FT_GlyphLoader_Rewind` `0x00ab5850`: only unnamed source callee of `pfr_glyph_init` `0x00ac8468`
- `FT_GlyphLoader_Reset` `0x00ab586c`: only unnamed source callee of `FT_GlyphLoader_Done` `0x00ab58cb`
- `FT_GlyphLoader_Done` `0x00ab58cb`: only unnamed source callee of `ft_glyphslot_done` `0x00ab5f80`
- `FT_GlyphLoader_Adjust_Points` `0x00ab58e6`: only unnamed source callee of `FT_GlyphLoader_CreateExtra` `0x00ab592b`
- `FT_GlyphLoader_CreateExtra` `0x00ab592b`: only unnamed source callee of `tt_slot_init` `0x00ada683`
- `ft_service_list_lookup` `0x00ab5ce0`: only unnamed source callee of `pfr_get_service` `0x00ac97a6`
- `ft_glyphslot_init` `0x00ab5e2a`: only unnamed source callee of `FT_New_GlyphSlot` `0x00ab5fd3`
- `ft_glyphslot_free_bitmap` `0x00ab5e95`: only unnamed source callee of `t42_glyphslot_clear` `0x00abcd87`
- `ft_glyphslot_set_bitmap` `0x00ab5ec9`: only unnamed source callee of `BDF_Glyph_Load` `0x00ad5639`
- `ft_glyphslot_done` `0x00ab5f80`: only unnamed source callee of `FT_Done_GlyphSlot` `0x00ab604d`
- `FT_New_GlyphSlot` `0x00ab5fd3`: only unnamed source callee of `T42_GlyphSlot_Init` `0x00abcd3e`
- `FT_Done_GlyphSlot` `0x00ab604d`: only unnamed source callee of `T42_GlyphSlot_Done` `0x00abcd78`
- `FT_Done_Face` `0x00ab7094`: only unnamed source callee of `T42_Face_Done` `0x00abcb00`
- `FT_Match_Size` `0x00ab722e`: only unnamed source callee of `tt_face_set_sbit_strike` `0x00ae4fcc`
- `ft_synthesize_vertical_metrics` `0x00ab72f5`: only unnamed source callee of `FNT_Load_Glyph` `0x00ae6a0d`
- `ft_recompute_scaled_metrics` `0x00ab7328`: only unnamed source callee of `FT_Select_Metrics` `0x00ab737c`
- `FT_Select_Metrics` `0x00ab737c`: only unnamed source callee of `PCF_Size_Select` `0x00acf3d7`
- `FT_Request_Metrics` `0x00ab73fd`: only unnamed source callee of `cid_size_request` `0x00ae03ed`
- `FT_Get_Module` `0x00ab81ca`: only unnamed source callee of `T42_Driver_Init` `0x00abcc57`
- `FT_Outline_Get_CBox` `0x00ab89ba`: only unnamed source callee of `ft_raster1_get_cbox` `0x00ac3a1c`
- `FT_Outline_Translate` `0x00ab8a37`: only unnamed source callee of `ft_smooth_render_generic` `0x00acde16`
- `FT_Vector_Transform` `0x00ab8bb3`: only unnamed source callee of `FT_Outline_Transform` `0x00ab8c00`
- `FT_Outline_Transform` `0x00ab8c00`: only unnamed source callee of `ft_smooth_transform` `0x00acdda7`
- `FT_Stream_Close` `0x00ab9725`: only unnamed source callee of `PCF_Face_Done` `0x00acf18e`
- `FT_Stream_ExtractFrame` `0x00ab9836`: only unnamed source callee of `tt_face_load_kern` `0x00ae37cf`
- `FT_Stream_ReleaseFrame` `0x00ab9853`: only unnamed source callee of `tt_face_load_bdf_props` `0x00ae5c9a`
- `FT_Stream_EnterFrame` `0x00ab986b`: only unnamed source callee of `FT_Stream_ExtractFrame` `0x00ab9836`
- `FT_Stream_ExitFrame` `0x00ab98fc`: only unnamed source callee of `FT_Stream_ReadFields` `0x00ab9c1a`
- `FT_Stream_ReadFields` `0x00ab9c1a`: only unnamed source callee of `tt_face_load_os2` `0x00ae1361`
- `ft_mem_alloc` `0x00aba232`: only unnamed source callee of `gray_raster_new` `0x00acdce6`
- `ft_mem_qalloc` `0x00aba26c`: only unnamed source callee of `ft_mem_alloc` `0x00aba232`
- `ft_mem_realloc` `0x00aba296`: only unnamed source callee of `ps_unicodes_init` `0x00ad2d2e`
- `ft_mem_qrealloc` `0x00aba2fb`: only unnamed source callee of `ft_mem_realloc` `0x00aba296`
- `ft_mem_free` `0x00aba389`: only unnamed source callee of `ft_black_done` `0x00ac3888`
- `t42_glyphslot_clear` `0x00abcd87`: only unnamed source callee of `T42_GlyphSlot_Load` `0x00abcdda`
- `cff_index_get_sid_string` `0x00abeb19`: only unnamed source callee of `cff_ps_get_font_info` `0x00abde82`
- `cff_font_done` `0x00abf7da`: only unnamed source callee of `cff_face_done` `0x00ac028a`
- `cff_size_get_globals_funcs` `0x00abf956`: only unnamed source callee of `cff_size_init` `0x00abf9b1`
- `af_glyph_hints_done` `0x00ac41c0`: only unnamed source callee of `af_loader_done` `0x00ac6fb5`
- `af_loader_init` `0x00ac6f48`: only unnamed source callee of `af_autofitter_init` `0x00ac77b0`
- `af_loader_done` `0x00ac6fb5`: only unnamed source callee of `af_autofitter_done` `0x00ac77c1`
- `af_loader_load_glyph` `0x00ac76e5`: only unnamed source callee of `af_autofitter_load_glyph` `0x00ac77d0`
- `pfr_phy_font_done` `0x00ac7f91`: only unnamed source callee of `pfr_face_done` `0x00ac8fdf`
- `pfr_glyph_init` `0x00ac8468`: only unnamed source callee of `pfr_slot_init` `0x00ac92d5`
- `pfr_glyph_done` `0x00ac8480`: only unnamed source callee of `pfr_slot_done` `0x00ac92f1`
- `ft_smooth_render_generic` `0x00acde16`: only unnamed source callee of `ft_smooth_render` `0x00ace06f`
- `pcf_find_property` `0x00ace3e0`: only unnamed source callee of `pcf_get_bdf_property` `0x00acf642`
- `T1_Size_Get_Globals_Funcs` `0x00ad1b7b`: only unnamed source callee of `T1_Size_Init` `0x00ad1bd0`
- `T1_Parse_Glyph_And_Get_Char_String` `0x00ad21e1`: only unnamed source callee of `T1_Load_Glyph` `0x00ad2338`
- `T1_Get_Kerning` `0x00ad2a87`: only unnamed source callee of `Get_Kerning` `0x00ad21b5`
- `hash_lookup` `0x00ad2f36`: only unnamed source callee of `bdf_get_font_property` `0x00ad4e16`
- `hash_free` `0x00ad305b`: only unnamed source callee of `bdf_free_font` `0x00ad4c3e`
- `bdf_free_font` `0x00ad4c3e`: only unnamed source callee of `BDF_Face_Done` `0x00ad50d4`
- `bdf_get_font_property` `0x00ad4e16`: only unnamed source callee of `bdf_get_bdf_property` `0x00ad573e`
- `ps_dimension_done` `0x00ad5c2e`: only unnamed source callee of `ps_hints_done` `0x00ad5e24`
- `ps_hints_done` `0x00ad5e24`: only unnamed source callee of `ps_hinter_done` `0x00ad7d67`
- `tt_glyphzone_done` `0x00ad9ca4`: only unnamed source callee of `tt_size_done_bytecode` `0x00ada1be`
- `tt_size_run_prep` `0x00ada0f5`: only unnamed source callee of `tt_size_ready_bytecode` `0x00ada3f7`
- `tt_size_done_bytecode` `0x00ada1be`: only unnamed source callee of `tt_size_done` `0x00ada4fb`
- `tt_size_init_bytecode` `0x00ada263`: only unnamed source caller of `tt_size_done_bytecode` `0x00ada1be`
- `tt_size_ready_bytecode` `0x00ada3f7`: only unnamed source caller of `tt_size_init_bytecode` `0x00ada263`
- `tt_size_reset` `0x00ada514`: only unnamed source callee of `tt_size_select` `0x00ad7e1b`
- `TT_Done_Context` `0x00ada6ed`: only unnamed source callee of `tt_driver_done` `0x00ada66a`
- `Init_Context` `0x00ada75d`: only unnamed source callee of `TT_New_Context` `0x00adaa9d`
- `TT_New_Context` `0x00adaa9d`: only unnamed source callee of `tt_driver_init` `0x00ada650`
- `cid_face_open` `0x00ae01e5`: only unnamed source caller of `cid_parse_dict` `0x00adfcc9`
- `cid_size_get_globals_funcs` `0x00ae0353`: only unnamed source callee of `cid_size_done` `0x00ae0385`
- `cid_load_glyph` `0x00ae07b9`: only unnamed source callee of `cid_slot_load_glyph` `0x00ae0978`
- `tt_face_load_generic_header` `0x00ae0f61`: only unnamed source callee of `tt_face_load_bhed` `0x00ae0fa5`
- `find_sbit_range` `0x00ae504a`: only unnamed source callee of `tt_find_sbit_image` `0x00ae5127`
- `Load_SBit_Image` `0x00ae5493`: only unnamed source caller of `find_sbit_range` `0x00ae504a`
- `load_post_names` `0x00ae5a56`: only unnamed source callee of `tt_face_get_ps_name` `0x00ae5b7d`
- `tt_face_load_bdf_props` `0x00ae5c9a`: only unnamed source callee of `tt_face_find_bdf_prop` `0x00ae5dab`
- `fnt_font_done` `0x00ae5f60`: only unnamed source callee of `FNT_Face_Done` `0x00ae6693`
- `fnt_face_get_dll_font` `0x00ae6063`: only unnamed source caller of `fnt_font_done` `0x00ae5f60`
- `ft_zlib::huft_build` `0x00ae73ef`: only unnamed source callee of `ft_zlib::inflate_trees_dynamic` `0x00ae7800`
- `ft_zlib::inflate_blocks_reset` `0x00ae7ef3`: only unnamed source callee of `ft_zlib::inflate` `0x00ae88a2`
- `ft_zlib::inflate_blocks` `0x00ae7fd4`: only unnamed source caller of `ft_zlib::inflate_trees_dynamic` `0x00ae7800`
- `ft_zlib::inflate` `0x00ae88a2`: only unnamed source caller of `ft_zlib::inflate_blocks` `0x00ae7fd4`

## TINYXML_VTABLE (84)

- `TiXmlNode::ToElement` `0x00467230`: slot 6 of `TiXmlComment::vftable` `0x00c174bc` and 5 more
- `TiXmlNode::ToComment` `0x00467240`: slot 8 of `TiXmlDeclaration::vftable` `0x00c175c4` and 5 more
- `TiXmlNode::ToUnknown` `0x00467250`: slot 10 of `TiXmlComment::vftable` `0x00c174bc` and 5 more
- `TiXmlNode::ToText` `0x00467260`: slot 12 of `TiXmlComment::vftable` `0x00c174bc` and 5 more
- `TiXmlNode::ToDeclaration` `0x00467270`: slot 14 of `TiXmlComment::vftable` `0x00c174bc` and 5 more
- `TiXmlNode::ToElement` `0x00467280`: slot 5 of `TiXmlComment::vftable` `0x00c174bc` and 5 more
- `TiXmlNode::ToComment` `0x00467290`: slot 7 of `TiXmlDeclaration::vftable` `0x00c175c4` and 5 more
- `TiXmlNode::ToUnknown` `0x004672a0`: slot 9 of `TiXmlComment::vftable` `0x00c174bc` and 5 more
- `TiXmlNode::ToText` `0x004672b0`: slot 11 of `TiXmlComment::vftable` `0x00c174bc` and 5 more
- `TiXmlNode::ToDeclaration` `0x004672c0`: slot 13 of `TiXmlComment::vftable` `0x00c174bc` and 5 more
- `TiXmlDocument::ToDocument` `0x00467320`: slot 4 of `TiXmlDocument::vftable` `0x00c1785c`
- `TiXmlDocument::ToDocument` `0x00467330`: slot 3 of `TiXmlDocument::vftable` `0x00c1785c`
- `TiXmlDocument::`scalar_deleting_destructor'` `0x00467340`: slot 0 of `TiXmlDocument::vftable` `0x00c1785c`
- `TiXmlNode::ToDocument` `0x0048bb00`: slot 4 of `TiXmlComment::vftable` `0x00c174bc` and 5 more
- `TiXmlNode::ToDocument` `0x0048bb10`: slot 3 of `TiXmlComment::vftable` `0x00c174bc` and 5 more
- `TiXmlText::ToText` `0x0048bb70`: slot 12 of `TiXmlText::vftable` `0x00c1750c`
- `TiXmlText::ToText` `0x0048bb80`: slot 11 of `TiXmlText::vftable` `0x00c1750c`
- `TiXmlText::`scalar_deleting_destructor'` `0x0048bb90`: slot 0 of `TiXmlText::vftable` `0x00c1750c`
- `TiXmlDeclaration::Print` `0x0048bc10`: slot 1 of `TiXmlDeclaration::vftable` `0x00c175c4`
- `TiXmlDeclaration::ToDeclaration` `0x0048bc30`: slot 14 of `TiXmlDeclaration::vftable` `0x00c175c4`
- `TiXmlDeclaration::ToDeclaration` `0x0048bc40`: slot 13 of `TiXmlDeclaration::vftable` `0x00c175c4`
- `TiXmlDeclaration::`scalar_deleting_destructor'` `0x0048bc50`: slot 0 of `TiXmlDeclaration::vftable` `0x00c175c4`
- `TiXmlVisitor::VisitEnter` `0x00520e90`: slot 2 of `TiXmlVisitor::vftable` `0x00c1769c`
- `TiXmlVisitor::VisitExit` `0x00520ea0`: slot 4 of `TiXmlVisitor::vftable` `0x00c1769c`
- `TiXmlVisitor::VisitEnter` `0x00520eb0`: slot 1 of `TiXmlVisitor::vftable` `0x00c1769c`
- `TiXmlVisitor::VisitExit` `0x00520ec0`: slot 3 of `TiXmlVisitor::vftable` `0x00c1769c`
- `TiXmlVisitor::Visit` `0x00520ed0`: slot 8 of `TiXmlVisitor::vftable` `0x00c1769c`
- `TiXmlVisitor::Visit` `0x00520ee0`: slot 7 of `TiXmlVisitor::vftable` `0x00c1769c`
- `TiXmlVisitor::Visit` `0x00520ef0`: slot 6 of `TiXmlVisitor::vftable` `0x00c1769c`
- `TiXmlVisitor::Visit` `0x00520f00`: slot 5 of `TiXmlVisitor::vftable` `0x00c1769c`
- `TiXmlVisitor::`scalar_deleting_destructor'` `0x00520f10`: slot 0 of `TiXmlVisitor::vftable` `0x00c1769c`
- `TiXmlPrinter::`scalar_deleting_destructor'` `0x00521000`: slot 0 of `TiXmlPrinter::vftable` `0x00c17834`
- `TiXmlBase::`scalar_deleting_destructor'` `0x00b63fd9`: slot 0 of `TiXmlBase::vftable` `0x00c174a8`
- `TiXmlComment::ToComment` `0x00b6406c`: slot 8 of `TiXmlComment::vftable` `0x00c174bc`
- `TiXmlComment::ToComment` `0x00b6406f`: slot 7 of `TiXmlComment::vftable` `0x00c174bc`
- `TiXmlComment::`scalar_deleting_destructor'` `0x00b64072`: slot 0 of `TiXmlComment::vftable` `0x00c174bc`
- `TiXmlUnknown::ToUnknown` `0x00b640f6`: slot 10 of `TiXmlUnknown::vftable` `0x00c1755c`
- `TiXmlUnknown::ToUnknown` `0x00b640f9`: slot 9 of `TiXmlUnknown::vftable` `0x00c1755c`
- `TiXmlUnknown::`scalar_deleting_destructor'` `0x00b640fc`: slot 0 of `TiXmlUnknown::vftable` `0x00c1755c`
- `TiXmlAttribute::Print` `0x00b644fd`: slot 1 of `TiXmlAttribute::vftable` `0x00c175b4`
- `TiXmlAttribute::`scalar_deleting_destructor'` `0x00b647f7`: slot 0 of `TiXmlAttribute::vftable` `0x00c175b4`
- `TiXmlUnknown::Parse` `0x00b64d6d`: slot 2 of `TiXmlUnknown::vftable` `0x00c1755c`
- `TiXmlComment::Parse` `0x00b64e21`: slot 2 of `TiXmlComment::vftable` `0x00c174bc`
- `TiXmlAttribute::Parse` `0x00b64f02`: slot 2 of `TiXmlAttribute::vftable` `0x00c175b4`
- `TiXmlText::Parse` `0x00b65048`: slot 2 of `TiXmlText::vftable` `0x00c1750c`
- `TiXmlDeclaration::Parse` `0x00b6518c`: slot 2 of `TiXmlDeclaration::vftable` `0x00c175c4`
- `TiXmlDocument::Parse` `0x00b653a3`: slot 2 of `TiXmlDocument::vftable` `0x00c1785c`
- `TiXmlElement::Parse` `0x00b65534`: slot 2 of `TiXmlElement::vftable` `0x00c177cc`
- `TiXmlDocument::StreamIn` `0x00b65a0d`: slot 17 of `TiXmlDocument::vftable` `0x00c1785c`
- `TiXmlUnknown::StreamIn` `0x00b65afd`: slot 17 of `TiXmlUnknown::vftable` `0x00c1755c`
- `TiXmlComment::StreamIn` `0x00b65b60`: slot 17 of `TiXmlComment::vftable` `0x00c174bc`
- `TiXmlText::StreamIn` `0x00b65bed`: slot 17 of `TiXmlText::vftable` `0x00c1750c`
- `TiXmlDeclaration::StreamIn` `0x00b65c98`: slot 17 of `TiXmlDeclaration::vftable` `0x00c175c4`
- `TiXmlElement::StreamIn` `0x00b65cfb`: slot 17 of `TiXmlElement::vftable` `0x00c177cc`
- `TiXmlElement::Accept` `0x00b6610a`: slot 16 of `TiXmlElement::vftable` `0x00c177cc`
- `TiXmlDocument::Print` `0x00b661a3`: slot 1 of `TiXmlDocument::vftable` `0x00c1785c`
- `TiXmlDocument::Accept` `0x00b661d3`: slot 16 of `TiXmlDocument::vftable` `0x00c1785c`
- `TiXmlComment::Accept` `0x00b6620e`: slot 16 of `TiXmlComment::vftable` `0x00c174bc`
- `TiXmlText::Accept` `0x00b6621d`: slot 16 of `TiXmlText::vftable` `0x00c1750c`
- `TiXmlDeclaration::Accept` `0x00b6622c`: slot 16 of `TiXmlDeclaration::vftable` `0x00c175c4`
- `TiXmlUnknown::Accept` `0x00b6623b`: slot 16 of `TiXmlUnknown::vftable` `0x00c1755c`
- `TiXmlPrinter::VisitEnter` `0x00b662e8`: slot 2 of `TiXmlPrinter::vftable` `0x00c17834`
- `TiXmlPrinter::VisitExit` `0x00b662ed`: slot 4 of `TiXmlPrinter::vftable` `0x00c17834`
- `TiXmlElement::Print` `0x00b6673f`: slot 1 of `TiXmlElement::vftable` `0x00c177cc`
- `TiXmlComment::Print` `0x00b66951`: slot 1 of `TiXmlComment::vftable` `0x00c174bc`
- `TiXmlUnknown::Print` `0x00b66995`: slot 1 of `TiXmlUnknown::vftable` `0x00c1755c`
- `TiXmlNode::`scalar_deleting_destructor'` `0x00b66dfa`: slot 0 of `TiXmlNode::vftable` `0x00c1777c`
- `TiXmlElement::ToElement` `0x00b66e5d`: slot 6 of `TiXmlElement::vftable` `0x00c177cc`
- `TiXmlElement::ToElement` `0x00b66e60`: slot 5 of `TiXmlElement::vftable` `0x00c177cc`
- `TiXmlElement::`scalar_deleting_destructor'` `0x00b67026`: slot 0 of `TiXmlElement::vftable` `0x00c177cc`
- `TiXmlText::Print` `0x00b671dc`: slot 1 of `TiXmlText::vftable` `0x00c1750c`
- `TiXmlElement::Clone` `0x00b676fb`: slot 15 of `TiXmlElement::vftable` `0x00c177cc`
- `TiXmlDocument::Clone` `0x00b67992`: slot 15 of `TiXmlDocument::vftable` `0x00c1785c`
- `TiXmlComment::Clone` `0x00b67bc3`: slot 15 of `TiXmlComment::vftable` `0x00c174bc`
- `TiXmlText::Clone` `0x00b67c2f`: slot 15 of `TiXmlText::vftable` `0x00c1750c`
- `TiXmlDeclaration::Print` `0x00b67d97`: slot 18 of `TiXmlDeclaration::vftable` `0x00c175c4`
- `TiXmlDeclaration::Clone` `0x00b67f20`: slot 15 of `TiXmlDeclaration::vftable` `0x00c175c4`
- `TiXmlUnknown::Clone` `0x00b67f7b`: slot 15 of `TiXmlUnknown::vftable` `0x00c1755c`
- `TiXmlPrinter::VisitEnter` `0x00b68026`: slot 1 of `TiXmlPrinter::vftable` `0x00c17834`
- `TiXmlPrinter::VisitExit` `0x00b680e2`: slot 3 of `TiXmlPrinter::vftable` `0x00c17834`
- `TiXmlPrinter::Visit` `0x00b6814d`: slot 7 of `TiXmlPrinter::vftable` `0x00c17834`
- `TiXmlPrinter::Visit` `0x00b68254`: slot 8 of `TiXmlPrinter::vftable` `0x00c17834`
- `TiXmlPrinter::Visit` `0x00b68284`: slot 6 of `TiXmlPrinter::vftable` `0x00c17834`
- `TiXmlPrinter::Visit` `0x00b682d8`: slot 5 of `TiXmlPrinter::vftable` `0x00c17834`

## CRYPTOPP_VTABLE (294)

- `CryptoPP::Clonable::Clone` `0x0060a540`: slot 1 of `CryptoPP::Adler32::vftable` `0x00cb948c` and 29 more
- `CryptoPP::Algorithm::AlgorithmName` `0x0060a610`: slot 2 of `CryptoPP::Adler32::vftable` `0x00cb948c` and 24 more
- `CryptoPP::HashTransformation::Final` `0x0060a640`: slot 5 of `CryptoPP::Adler32::vftable` `0x00cb948c`
- `CryptoPP::HashTransformation::CalculateDigest` `0x0060a660`: slot 11 of `CryptoPP::Adler32::vftable` `0x00cb948c`
- `CryptoPP::HashTransformation::Verify` `0x0060a690`: slot 12 of `CryptoPP::Adler32::vftable` `0x00cb948c`
- `CryptoPP::HashTransformation::VerifyDigest` `0x0060a6b0`: slot 13 of `CryptoPP::Adler32::vftable` `0x00cb948c`
- `CryptoPP::HashTransformation::CalculateTruncatedDigest` `0x0060a6e0`: slot 15 of `CryptoPP::Adler32::vftable` `0x00cb948c`
- `CryptoPP::HashTransformation::VerifyTruncatedDigest` `0x0060a710`: slot 17 of `CryptoPP::Adler32::vftable` `0x00cb948c`
- `CryptoPP::BufferedTransformation::CreatePutSpace` `0x0060a740`: slot 3 of `CryptoPP::BERGeneralDecoder::vftable` `0x00ce262c` and 16 more
- `CryptoPP::BufferedTransformation::CanModifyInput` `0x0060a750`: slot 4 of `CryptoPP::ArraySink::vftable` `0x00cdfc6c` and 23 more
- `CryptoPP::BufferedTransformation::PutModifiable2` `0x0060a760`: slot 6 of `CryptoPP::ArraySink::vftable` `0x00cdfc6c` and 17 more
- `CryptoPP::BufferedTransformation::IsolatedMessageSeriesEnd` `0x0060a770`: slot 9 of `CryptoPP::ArraySink::vftable` `0x00cdfc6c` and 19 more
- `CryptoPP::BufferedTransformation::SetAutoSignalPropagation` `0x0060a780`: slot 13 of `CryptoPP::ArraySink::vftable` `0x00cdfc6c` and 17 more
- `CryptoPP::BufferedTransformation::GetAutoSignalPropagation` `0x0060a790`: slot 14 of `CryptoPP::ArraySink::vftable` `0x00cdfc6c` and 17 more
- `CryptoPP::BufferedTransformation::GetNextMessageSeries` `0x0060a7a0`: slot 28 of `CryptoPP::ArraySink::vftable` `0x00cdfc6c` and 23 more
- `CryptoPP::BufferedTransformation::NumberOfMessagesInThisSeries` `0x0060a7b0`: slot 29 of `CryptoPP::ArraySink::vftable` `0x00cdfc6c` and 22 more
- `CryptoPP::BufferedTransformation::NumberOfMessageSeries` `0x0060a7c0`: slot 30 of `CryptoPP::ArraySink::vftable` `0x00cdfc6c` and 22 more
- `CryptoPP::BufferedTransformation::Attachable` `0x0060a7d0`: slot 39 of `CryptoPP::ArraySink::vftable` `0x00cdfc6c` and 10 more
- `CryptoPP::BufferedTransformation::AttachedTransformation` `0x0060a7e0`: slot 41 of `CryptoPP::ArraySink::vftable` `0x00cdfc6c` and 10 more
- `CryptoPP::BufferedTransformation::AttachedTransformation` `0x0060a7f0`: slot 40 of `CryptoPP::ArraySink::vftable` `0x00cdfc6c` and 10 more
- `CryptoPP::BufferedTransformation::Detach` `0x0060a800`: slot 42 of `CryptoPP::ArraySink::vftable` `0x00cdfc6c` and 10 more
- `CryptoPP::ASN1Object::BEREncode` `0x0060aac0`: slot 3 of `CryptoPP::Integer::vftable` `0x00ce01e8`
- `CryptoPP::AutoSeededRandomPool::`scalar_deleting_destructor'` `0x0060ac80`: slot 0 of `CryptoPP::AutoSeededRandomPool::vftable` `0x00cb7a6c`
- `CryptoPP::ModularArithmetic::Clone` `0x0060aef0`: slot 23 of `CryptoPP::ModularArithmetic::vftable` `0x00cb7b54`
- `CryptoPP::ModularArithmetic::IsMontgomeryRepresentation` `0x0060af50`: slot 24 of `CryptoPP::ModularArithmetic::vftable` `0x00cb7b54`
- `CryptoPP::ModularArithmetic::ConvertIn` `0x0060af60`: slot 25 of `CryptoPP::ModularArithmetic::vftable` `0x00cb7b54`
- `CryptoPP::ModularArithmetic::ConvertOut` `0x0060af80`: slot 26 of `CryptoPP::ModularArithmetic::vftable` `0x00cb7b54`
- `CryptoPP::ModularArithmetic::Equal` `0x0060afa0`: slot 1 of `CryptoPP::ModularArithmetic::vftable` `0x00cb7b54` and 1 more
- `CryptoPP::ModularArithmetic::Identity` `0x0060afb0`: slot 2 of `CryptoPP::ModularArithmetic::vftable` `0x00cb7b54` and 1 more
- `CryptoPP::ModularArithmetic::Double` `0x0060afc0`: slot 6 of `CryptoPP::ModularArithmetic::vftable` `0x00cb7b54` and 1 more
- `CryptoPP::ModularArithmetic::MultiplicativeIdentity` `0x0060afd0`: slot 14 of `CryptoPP::ModularArithmetic::vftable` `0x00cb7b54`
- `CryptoPP::ModularArithmetic::Multiply` `0x0060afe0`: slot 15 of `CryptoPP::ModularArithmetic::vftable` `0x00cb7b54`
- `CryptoPP::ModularArithmetic::Square` `0x0060b040`: slot 17 of `CryptoPP::ModularArithmetic::vftable` `0x00cb7b54`
- `CryptoPP::ModularArithmetic::IsUnit` `0x0060b0a0`: slot 13 of `CryptoPP::ModularArithmetic::vftable` `0x00cb7b54` and 1 more
- `CryptoPP::ModularArithmetic::MultiplicativeInverse` `0x0060b110`: slot 16 of `CryptoPP::ModularArithmetic::vftable` `0x00cb7b54`
- `CryptoPP::ModularArithmetic::Divide` `0x0060b150`: slot 18 of `CryptoPP::ModularArithmetic::vftable` `0x00cb7b54` and 1 more
- `CryptoPP::ModularArithmetic::`scalar_deleting_destructor'` `0x0060b180`: slot 0 of `CryptoPP::ModularArithmetic::vftable` `0x00cb7b54`
- `CryptoPP::MontgomeryRepresentation::Clone` `0x0060b220`: slot 23 of `CryptoPP::MontgomeryRepresentation::vftable` `0x00cb7bc4`
- `CryptoPP::MontgomeryRepresentation::IsMontgomeryRepresentation` `0x0060b250`: slot 24 of `CryptoPP::MontgomeryRepresentation::vftable` `0x00cb7bc4`
- `CryptoPP::MontgomeryRepresentation::ConvertIn` `0x0060b260`: slot 25 of `CryptoPP::MontgomeryRepresentation::vftable` `0x00cb7bc4`
- `CryptoPP::MontgomeryRepresentation::MultiplicativeIdentity` `0x0060b2c0`: slot 14 of `CryptoPP::MontgomeryRepresentation::vftable` `0x00cb7bc4`
- `CryptoPP::MontgomeryRepresentation::CascadeExponentiate` `0x0060b320`: slot 20 of `CryptoPP::MontgomeryRepresentation::vftable` `0x00cb7bc4`
- `CryptoPP::MontgomeryRepresentation::SimultaneousExponentiate` `0x0060b350`: slot 21 of `CryptoPP::MontgomeryRepresentation::vftable` `0x00cb7bc4`
- `CryptoPP::MontgomeryRepresentation::`scalar_deleting_destructor'` `0x0060b3e0`: slot 0 of `CryptoPP::MontgomeryRepresentation::vftable` `0x00cb7bc4`
- `CryptoPP::ModExpPrecomputation::NeedConversions` `0x0060b590`: slot 0 of `CryptoPP::ModExpPrecomputation::vftable` `0x00cb7cdc`
- `CryptoPP::ModExpPrecomputation::ConvertIn` `0x0060b5a0`: slot 1 of `CryptoPP::ModExpPrecomputation::vftable` `0x00cb7cdc`
- `CryptoPP::ModExpPrecomputation::ConvertOut` `0x0060b5c0`: slot 2 of `CryptoPP::ModExpPrecomputation::vftable` `0x00cb7cdc`
- `CryptoPP::ModExpPrecomputation::GetGroup` `0x0060b5e0`: slot 3 of `CryptoPP::ModExpPrecomputation::vftable` `0x00cb7cdc`
- `CryptoPP::ModExpPrecomputation::BERDecodeElement` `0x0060b5f0`: slot 4 of `CryptoPP::ModExpPrecomputation::vftable` `0x00cb7cdc`
- `CryptoPP::ModExpPrecomputation::DEREncodeElement` `0x0060b610`: slot 5 of `CryptoPP::ModExpPrecomputation::vftable` `0x00cb7cdc`
- `CryptoPP::Bufferless<BufferedTransformation>::IsolatedFlush` `0x006159a0`: slot 8 of `CryptoPP::ByteQueue::vftable` `0x00ce24ac` and 2 more
- `CryptoPP::AbstractGroup<Integer>::InversionIsFast` `0x00615cf0`: slot 5 of `CryptoPP::ModularArithmetic::vftable` `0x00cb7b54` and 1 more
- `CryptoPP::AbstractRing<Integer>::MultiplicativeGroup` `0x00615d10`: slot 22 of `CryptoPP::ModularArithmetic::vftable` `0x00cb7b54` and 1 more
- `CryptoPP::DL_GroupPrecomputation<Integer>::NeedConversions` `0x00615e90`: slot 0 of `CryptoPP::DL_GroupPrecomputation<CryptoPP::Integer>::vftable` `0x00cb7cc0`
- `CryptoPP::DL_GroupPrecomputation<Integer>::ConvertIn` `0x00615ea0`: slot 1 of `CryptoPP::DL_GroupPrecomputation<CryptoPP::Integer>::vftable` `0x00cb7cc0`
- `CryptoPP::DL_GroupPrecomputation<Integer>::ConvertOut` `0x00615ec0`: slot 2 of `CryptoPP::DL_GroupPrecomputation<CryptoPP::Integer>::vftable` `0x00cb7cc0`
- `CryptoPP::NullNameValuePairs::GetVoidValue` `0x0063a040`: slot 1 of `CryptoPP::NullNameValuePairs::vftable` `0x00cb90d4`
- `CryptoPP::AlgorithmParametersBase::`scalar_deleting_destructor'` `0x0063a410`: slot 0 of `CryptoPP::AlgorithmParametersBase::vftable` `0x00cb90e0`
- `CryptoPP::NullNameValuePairs::`scalar_deleting_destructor'` `0x0063b790`: slot 0 of `CryptoPP::NullNameValuePairs::vftable` `0x00cb90d4`
- `CryptoPP::HashTransformation::CreateUpdateSpace` `0x0063c870`: slot 4 of `CryptoPP::Adler32::vftable` `0x00cb948c`
- `CryptoPP::HashTransformation::Restart` `0x0063c880`: slot 6 of `CryptoPP::Adler32::vftable` `0x00cb948c`
- `CryptoPP::HashTransformation::BlockSize` `0x0063c890`: slot 8 of `CryptoPP::Adler32::vftable` `0x00cb948c`
- `CryptoPP::HashTransformation::OptimalBlockSize` `0x0063c8a0`: slot 9 of `CryptoPP::Adler32::vftable` `0x00cb948c`
- `CryptoPP::HashTransformation::OptimalDataAlignment` `0x0063c8b0`: slot 10 of `CryptoPP::Adler32::vftable` `0x00cb948c`
- `CryptoPP::Filter::Attachable` `0x0063c8c0`: slot 39 of `CryptoPP::Deflator::vftable` `0x00ce18e4` and 12 more
- `CryptoPP::Filter::ShouldPropagateMessageEnd` `0x0063c8d0`: slot 45 of `CryptoPP::Deflator::vftable` `0x00ce18e4` and 12 more
- `CryptoPP::Filter::ShouldPropagateMessageSeriesEnd` `0x0063c8e0`: slot 46 of `CryptoPP::Deflator::vftable` `0x00ce18e4` and 12 more
- `CryptoPP::Adler32::DigestSize` `0x0063c920`: slot 7 of `CryptoPP::Adler32::vftable` `0x00cb948c`
- `CryptoPP::Adler32::`scalar_deleting_destructor'` `0x0063c940`: slot 0 of `CryptoPP::Adler32::vftable` `0x00cb948c`
- `CryptoPP::ZlibCompressor::`scalar_deleting_destructor'` `0x0063c9d0`: slot 0 of `CryptoPP::ZlibCompressor::vftable` `0x00cb94dc`
- `CryptoPP::ArraySink::`scalar_deleting_destructor'` `0x0084b5e0`: slot 0 of `CryptoPP::ArraySink::vftable` `0x00cdfc6c`
- `CryptoPP::BitBucket::`scalar_deleting_destructor'` `0x0084b687`: slot 0 of `CryptoPP::BitBucket::vftable` `0x00cdfbac`
- `CryptoPP::RandomNumberGenerator::GenerateBit` `0x0084b9d4`: slot 4 of `CryptoPP::AutoSeededRandomPool::vftable` `0x00cb7a6c` and 4 more
- `CryptoPP::RandomNumberGenerator::GenerateBlock` `0x0084b9de`: slot 6 of `CryptoPP::ClassNullRNG::vftable` `0x00cdfd54`
- `CryptoPP::RandomNumberGenerator::GenerateWord32` `0x0084ba07`: slot 5 of `CryptoPP::AutoSeededRandomPool::vftable` `0x00cb7a6c` and 4 more
- `CryptoPP::RandomNumberGenerator::DiscardBytes` `0x0084ba6a`: slot 7 of `CryptoPP::AutoSeededRandomPool::vftable` `0x00cb7a6c` and 4 more
- `CryptoPP::ClassNullRNG::AlgorithmName` `0x0084ba85`: slot 2 of `CryptoPP::ClassNullRNG::vftable` `0x00cdfd54`
- `CryptoPP::ClassNullRNG::GenerateByte` `0x0084baa1`: slot 3 of `CryptoPP::ClassNullRNG::vftable` `0x00cdfd54`
- `CryptoPP::ClassNullRNG::`scalar_deleting_destructor'` `0x0084bb33`: slot 0 of `CryptoPP::ClassNullRNG::vftable` `0x00cdfd54`
- `CryptoPP::HashTransformation::TruncatedVerify` `0x0084bb5a`: slot 16 of `CryptoPP::Adler32::vftable` `0x00cb948c`
- `CryptoPP::BufferedTransformation::Initialize` `0x0084bca2`: slot 10 of `CryptoPP::ArraySink::vftable` `0x00cdfc6c` and 9 more
- `CryptoPP::BufferedTransformation::Flush` `0x0084bcae`: slot 11 of `CryptoPP::ArraySink::vftable` `0x00cdfc6c` and 9 more
- `CryptoPP::BufferedTransformation::MessageSeriesEnd` `0x0084bcbe`: slot 12 of `CryptoPP::ArraySink::vftable` `0x00cdfc6c` and 9 more
- `CryptoPP::BufferedTransformation::ChannelCreatePutSpace` `0x0084bcca`: slot 33 of `CryptoPP::ArraySink::vftable` `0x00cdfc6c` and 23 more
- `CryptoPP::BufferedTransformation::ChannelPut2` `0x0084bd15`: slot 34 of `CryptoPP::ArraySink::vftable` `0x00cdfc6c` and 19 more
- `CryptoPP::BufferedTransformation::ChannelPutModifiable2` `0x0084bd51`: slot 35 of `CryptoPP::ArraySink::vftable` `0x00cdfc6c` and 22 more
- `CryptoPP::BufferedTransformation::ChannelFlush` `0x0084bd7d`: slot 36 of `CryptoPP::ArraySink::vftable` `0x00cdfc6c` and 20 more
- `CryptoPP::BufferedTransformation::ChannelMessageSeriesEnd` `0x0084bdb6`: slot 37 of `CryptoPP::ArraySink::vftable` `0x00cdfc6c` and 19 more
- `CryptoPP::BufferedTransformation::MaxRetrievable` `0x0084bdec`: slot 15 of `CryptoPP::ArraySink::vftable` `0x00cdfc6c` and 19 more
- `CryptoPP::BufferedTransformation::AnyRetrievable` `0x0084be25`: slot 16 of `CryptoPP::ArraySink::vftable` `0x00cdfc6c` and 19 more
- `CryptoPP::BufferedTransformation::Get` `0x0084be5d`: slot 18 of `CryptoPP::ArraySink::vftable` `0x00cdfc6c` and 20 more
- `CryptoPP::BufferedTransformation::Get` `0x0084be8b`: slot 17 of `CryptoPP::ArraySink::vftable` `0x00cdfc6c` and 20 more
- `CryptoPP::BufferedTransformation::Peek` `0x0084beef`: slot 20 of `CryptoPP::ArraySink::vftable` `0x00cdfc6c` and 20 more
- `CryptoPP::BufferedTransformation::Peek` `0x0084bf1d`: slot 19 of `CryptoPP::ArraySink::vftable` `0x00cdfc6c` and 20 more
- `CryptoPP::BufferedTransformation::Skip` `0x0084bf7a`: slot 21 of `CryptoPP::ArraySink::vftable` `0x00cdfc6c` and 23 more
- `CryptoPP::BufferedTransformation::TotalBytesRetrievable` `0x0084bfc2`: slot 22 of `CryptoPP::ArraySink::vftable` `0x00cdfc6c` and 22 more
- `CryptoPP::BufferedTransformation::NumberOfMessages` `0x0084bfe7`: slot 23 of `CryptoPP::ArraySink::vftable` `0x00cdfc6c` and 19 more
- `CryptoPP::BufferedTransformation::AnyMessages` `0x0084c015`: slot 24 of `CryptoPP::ArraySink::vftable` `0x00cdfc6c` and 23 more
- `CryptoPP::BufferedTransformation::GetNextMessage` `0x0084c041`: slot 25 of `CryptoPP::ArraySink::vftable` `0x00cdfc6c` and 19 more
- `CryptoPP::BufferedTransformation::SkipMessages` `0x0084c066`: slot 26 of `CryptoPP::ArraySink::vftable` `0x00cdfc6c` and 23 more
- `CryptoPP::BufferedTransformation::SkipAll` `0x0084c158`: slot 27 of `CryptoPP::ArraySink::vftable` `0x00cdfc6c` and 23 more
- `CryptoPP::BufferedTransformation::SetRetrievalChannel` `0x0084c237`: slot 38 of `CryptoPP::ArraySink::vftable` `0x00cdfc6c` and 23 more
- `CryptoPP::BufferedTransformation::Attach` `0x0084c395`: slot 43 of `CryptoPP::ArraySink::vftable` `0x00cdfc6c` and 23 more
- `CryptoPP::PK_DefaultEncryptionFilter::Put2` `0x0084c4ab`: slot 5 of `CryptoPP::PK_DefaultEncryptionFilter::vftable` `0x00cdfd7c`
- `CryptoPP::PK_DefaultEncryptionFilter::`scalar_deleting_destructor'` `0x0084c5f9`: slot 0 of `CryptoPP::PK_DefaultEncryptionFilter::vftable` `0x00cdfd7c`
- `CryptoPP::PK_DefaultDecryptionFilter::Put2` `0x0084c6ec`: slot 5 of `CryptoPP::PK_DefaultDecryptionFilter::vftable` `0x00cdfe4c`
- `CryptoPP::PK_DefaultDecryptionFilter::`scalar_deleting_destructor'` `0x0084c8a8`: slot 0 of `CryptoPP::PK_DefaultDecryptionFilter::vftable` `0x00cdfe4c`
- `CryptoPP::Unflushable<Filter>::Flush` `0x0084cc09`: slot 11 of `CryptoPP::PK_DefaultDecryptionFilter::vftable` `0x00cdfe4c` and 1 more
- `CryptoPP::Unflushable<Filter>::IsolatedFlush` `0x0084cc25`: slot 8 of `CryptoPP::PK_DefaultDecryptionFilter::vftable` `0x00cdfe4c` and 1 more
- `CryptoPP::Unflushable<Filter>::ChannelFlush` `0x0084cc2a`: slot 36 of `CryptoPP::PK_DefaultDecryptionFilter::vftable` `0x00cdfe4c` and 1 more
- `CryptoPP::Unflushable<Filter>::InputBufferIsEmpty` `0x0084ccbc`: slot 47 of `CryptoPP::PK_DefaultDecryptionFilter::vftable` `0x00cdfe4c` and 1 more
- `CryptoPP::Bufferless<Sink>::IsolatedFlush` `0x0084cf55`: slot 8 of `CryptoPP::ArraySink::vftable` `0x00cdfc6c` and 2 more
- `CryptoPP::BitBucket::IsolatedInitialize` `0x0084cf5a`: slot 7 of `CryptoPP::BitBucket::vftable` `0x00cdfbac`
- `CryptoPP::BitBucket::Put2` `0x0084cf5d`: slot 5 of `CryptoPP::BitBucket::vftable` `0x00cdfbac`
- `CryptoPP::BitBucket::AlgorithmName` `0x0084cf62`: slot 2 of `CryptoPP::BitBucket::vftable` `0x00cdfbac`
- `CryptoPP::Sink::CopyRangeTo2` `0x0084cf7e`: slot 32 of `CryptoPP::ArraySink::vftable` `0x00cdfc6c` and 3 more
- `CryptoPP::Sink::TransferTo2` `0x0084cf83`: slot 31 of `CryptoPP::ArraySink::vftable` `0x00cdfc6c` and 3 more
- `CryptoPP::BufferedTransformation::IsolatedInitialize` `0x0084cf93`: slot 7 of `CryptoPP::LowFirstBitWriter::vftable` `0x00ce1814` and 2 more
- `CryptoPP::ByteQueue::MaxRetrievable` `0x0084d75e`: slot 15 of `CryptoPP::ByteQueue::vftable` `0x00ce24ac` and 2 more
- `CryptoPP::ByteQueue::AnyRetrievable` `0x0084d763`: slot 16 of `CryptoPP::ByteQueue::vftable` `0x00ce24ac` and 2 more
- `CryptoPP::StringStore::`scalar_deleting_destructor'` `0x0084d7e3`: slot 0 of `CryptoPP::StringStore::vftable` `0x00ce0224`
- `CryptoPP::BERGeneralDecoder::StoreInitialize` `0x0084d90d`: slot 44 of `CryptoPP::BERGeneralDecoder::vftable` `0x00ce262c` and 1 more
- `CryptoPP::BERSequenceDecoder::`scalar_deleting_destructor'` `0x0084d92f`: slot 0 of `CryptoPP::BERSequenceDecoder::vftable` `0x00ce0314`
- `CryptoPP::DERSequenceEncoder::`scalar_deleting_destructor'` `0x0084d970`: slot 0 of `CryptoPP::DERSequenceEncoder::vftable` `0x00ce03dc`
- `CryptoPP::Integer::`scalar_deleting_destructor'` `0x00851670`: slot 0 of `CryptoPP::Integer::vftable` `0x00ce01e8`
- `CryptoPP::Integer::DEREncode` `0x008520c9`: slot 2 of `CryptoPP::Integer::vftable` `0x00ce01e8`
- `CryptoPP::Integer::BERDecode` `0x0085215d`: slot 1 of `CryptoPP::Integer::vftable` `0x00ce01e8`
- `CryptoPP::KDF2_RNG::GenerateByte` `0x00852721`: slot 3 of `CryptoPP::KDF2_RNG::vftable` `0x00ce0584`
- `CryptoPP::KDF2_RNG::GenerateBlock` `0x00852735`: slot 6 of `CryptoPP::KDF2_RNG::vftable` `0x00ce0584`
- `CryptoPP::KDF2_RNG::`scalar_deleting_destructor'` `0x00852773`: slot 0 of `CryptoPP::KDF2_RNG::vftable` `0x00ce0584`
- `CryptoPP::ModularArithmetic::Add` `0x00854863`: slot 3 of `CryptoPP::ModularArithmetic::vftable` `0x00cb7b54` and 1 more
- `CryptoPP::ModularArithmetic::Accumulate` `0x00854921`: slot 8 of `CryptoPP::ModularArithmetic::vftable` `0x00cb7b54` and 1 more
- `CryptoPP::ModularArithmetic::Subtract` `0x008549a1`: slot 7 of `CryptoPP::ModularArithmetic::vftable` `0x00cb7b54` and 1 more
- `CryptoPP::ModularArithmetic::Reduce` `0x00854a45`: slot 9 of `CryptoPP::ModularArithmetic::vftable` `0x00cb7b54` and 1 more
- `CryptoPP::ModularArithmetic::Inverse` `0x00854aab`: slot 4 of `CryptoPP::ModularArithmetic::vftable` `0x00cb7b54` and 1 more
- `CryptoPP::ModularArithmetic::CascadeExponentiate` `0x00854b16`: slot 20 of `CryptoPP::ModularArithmetic::vftable` `0x00cb7b54`
- `CryptoPP::ModularArithmetic::SimultaneousExponentiate` `0x00854c09`: slot 21 of `CryptoPP::ModularArithmetic::vftable` `0x00cb7b54`
- `CryptoPP::MontgomeryRepresentation::Multiply` `0x00854db9`: slot 15 of `CryptoPP::MontgomeryRepresentation::vftable` `0x00cb7bc4`
- `CryptoPP::MontgomeryRepresentation::Square` `0x00854e37`: slot 17 of `CryptoPP::MontgomeryRepresentation::vftable` `0x00cb7bc4`
- `CryptoPP::MontgomeryRepresentation::ConvertOut` `0x00854e9f`: slot 26 of `CryptoPP::MontgomeryRepresentation::vftable` `0x00cb7bc4`
- `CryptoPP::MontgomeryRepresentation::MultiplicativeInverse` `0x00854f2b`: slot 16 of `CryptoPP::MontgomeryRepresentation::vftable` `0x00cb7bc4`
- `CryptoPP::Store::NumberOfMessages` `0x00856b35`: slot 23 of `CryptoPP::BERGeneralDecoder::vftable` `0x00ce262c` and 2 more
- `CryptoPP::AutoSignaling<InputRejecting<BufferedTransformation>>::GetAutoSignalPropagation` `0x00856b3e`: slot 14 of `CryptoPP::BERGeneralDecoder::vftable` `0x00ce262c` and 2 more
- `CryptoPP::AutoSignaling<InputRejecting<BufferedTransformation>>::SetAutoSignalPropagation` `0x00856b42`: slot 13 of `CryptoPP::BERGeneralDecoder::vftable` `0x00ce262c` and 2 more
- `CryptoPP::Store::IsolatedInitialize` `0x00856b4c`: slot 7 of `CryptoPP::BERGeneralDecoder::vftable` `0x00ce262c` and 2 more
- `CryptoPP::InputRejecting<BufferedTransformation>::ChannelMessageSeriesEnd` `0x00856b58`: slot 37 of `CryptoPP::BERGeneralDecoder::vftable` `0x00ce262c` and 2 more
- `CryptoPP::InputRejecting<BufferedTransformation>::ChannelPut2` `0x00856bfd`: slot 34 of `CryptoPP::BERGeneralDecoder::vftable` `0x00ce262c` and 2 more
- `CryptoPP::InputRejecting<BufferedTransformation>::IsolatedMessageSeriesEnd` `0x00856c1b`: slot 9 of `CryptoPP::BERGeneralDecoder::vftable` `0x00ce262c` and 2 more
- `CryptoPP::InputRejecting<BufferedTransformation>::IsolatedFlush` `0x00856c39`: slot 8 of `CryptoPP::BERGeneralDecoder::vftable` `0x00ce262c` and 2 more
- `CryptoPP::InputRejecting<BufferedTransformation>::Put2` `0x00856c3e`: slot 5 of `CryptoPP::BERGeneralDecoder::vftable` `0x00ce262c` and 2 more
- `CryptoPP::AbstractGroup<Integer>::ScalarMultiply` `0x00858f3a`: slot 10 of `CryptoPP::ModularArithmetic::vftable` `0x00cb7b54` and 1 more
- `CryptoPP::AbstractGroup<Integer>::CascadeScalarMultiply` `0x00858f85`: slot 11 of `CryptoPP::ModularArithmetic::vftable` `0x00cb7b54` and 1 more
- `CryptoPP::AbstractGroup<Integer>::SimultaneousMultiply` `0x0085937a`: slot 12 of `CryptoPP::ModularArithmetic::vftable` `0x00cb7b54` and 1 more
- `CryptoPP::AbstractRing<Integer>::Exponentiate` `0x008596c5`: slot 19 of `CryptoPP::ModularArithmetic::vftable` `0x00cb7b54` and 1 more
- `CryptoPP::NonblockingRng::`scalar_deleting_destructor'` `0x0085e1b7`: slot 0 of `CryptoPP::NonblockingRng::vftable` `0x00ce0b48`
- `CryptoPP::NonblockingRng::GenerateByte` `0x0085e1db`: slot 3 of `CryptoPP::NonblockingRng::vftable` `0x00ce0b48`
- `CryptoPP::NonblockingRng::GenerateBlock` `0x0085e1ef`: slot 6 of `CryptoPP::NonblockingRng::vftable` `0x00ce0b48`
- `CryptoPP::RandomPool::`scalar_deleting_destructor'` `0x00861c2b`: slot 0 of `CryptoPP::RandomPool::vftable` `0x00ce0bd8`
- `CryptoPP::RandomPool::GenerateByte` `0x00861eb1`: slot 3 of `CryptoPP::AutoSeededRandomPool::vftable` `0x00cb7a6c` and 1 more
- `CryptoPP::RandomPool::GenerateBlock` `0x00861ed0`: slot 6 of `CryptoPP::AutoSeededRandomPool::vftable` `0x00cb7a6c` and 1 more
- `CryptoPP::AlgorithmParametersBase::GetVoidValue` `0x008636c1`: slot 1 of `CryptoPP::AlgorithmParametersBase::vftable` `0x00cb90e0`
- `CryptoPP::Adler32::Update` `0x00864cb7`: slot 3 of `CryptoPP::Adler32::vftable` `0x00cb948c`
- `CryptoPP::Adler32::TruncatedFinal` `0x00864d64`: slot 14 of `CryptoPP::Adler32::vftable` `0x00cb948c`
- `CryptoPP::ZlibDecompressor::GetLog2WindowSize` `0x00864f4f`: slot 47 of `CryptoPP::ZlibDecompressor::vftable` `0x00ce0f9c`
- `CryptoPP::ZlibDecompressor::MaxPrestreamHeaderSize` `0x00864f56`: slot 48 of `CryptoPP::ZlibDecompressor::vftable` `0x00ce0f9c`
- `CryptoPP::ZlibDecompressor::MaxPoststreamTailSize` `0x00864f5a`: slot 51 of `CryptoPP::ZlibDecompressor::vftable` `0x00ce0f9c`
- `CryptoPP::ZlibCompressor::WritePrestreamHeader` `0x00864fe2`: slot 47 of `CryptoPP::ZlibCompressor::vftable` `0x00cb94dc`
- `CryptoPP::ZlibCompressor::ProcessUncompressedData` `0x00865052`: slot 48 of `CryptoPP::ZlibCompressor::vftable` `0x00cb94dc`
- `CryptoPP::ZlibCompressor::WritePoststreamTail` `0x0086505d`: slot 49 of `CryptoPP::ZlibCompressor::vftable` `0x00cb94dc`
- `CryptoPP::ZlibDecompressor::`scalar_deleting_destructor'` `0x0086512b`: slot 0 of `CryptoPP::ZlibDecompressor::vftable` `0x00ce0f9c`
- `CryptoPP::ZlibDecompressor::ProcessPrestreamHeader` `0x008651d7`: slot 49 of `CryptoPP::ZlibDecompressor::vftable` `0x00ce0f9c`
- `CryptoPP::ZlibDecompressor::ProcessDecompressedData` `0x008652df`: slot 50 of `CryptoPP::ZlibDecompressor::vftable` `0x00ce0f9c`
- `CryptoPP::ZlibDecompressor::ProcessPoststreamTail` `0x00865309`: slot 52 of `CryptoPP::ZlibDecompressor::vftable` `0x00ce0f9c`
- `CryptoPP::AutoSignaling<Filter>::SetAutoSignalPropagation` `0x008653a7`: slot 13 of `CryptoPP::Inflator::vftable` `0x00ce2dd4` and 1 more
- `CryptoPP::AutoSignaling<Filter>::GetAutoSignalPropagation` `0x008653b1`: slot 14 of `CryptoPP::Inflator::vftable` `0x00ce2dd4` and 1 more
- `CryptoPP::Deflator::WritePrestreamHeader` `0x0086557a`: slot 47 of `CryptoPP::Deflator::vftable` `0x00ce18e4`
- `CryptoPP::Deflator::ProcessUncompressedData` `0x0086557b`: slot 48 of `CryptoPP::Deflator::vftable` `0x00ce18e4`
- `CryptoPP::Deflator::WritePoststreamTail` `0x0086557e`: slot 49 of `CryptoPP::Deflator::vftable` `0x00ce18e4`
- `CryptoPP::LowFirstBitWriter::`scalar_deleting_destructor'` `0x008655c2`: slot 0 of `CryptoPP::LowFirstBitWriter::vftable` `0x00ce1814`
- `CryptoPP::Deflator::`scalar_deleting_destructor'` `0x00865cd8`: slot 0 of `CryptoPP::Deflator::vftable` `0x00ce18e4`
- `CryptoPP::Deflator::IsolatedInitialize` `0x00865ef8`: slot 7 of `CryptoPP::Deflator::vftable` `0x00ce18e4` and 1 more
- `CryptoPP::Deflator::Put2` `0x008669a2`: slot 5 of `CryptoPP::Deflator::vftable` `0x00ce18e4` and 1 more
- `CryptoPP::Deflator::IsolatedFlush` `0x00866a74`: slot 8 of `CryptoPP::Deflator::vftable` `0x00ce18e4` and 1 more
- `CryptoPP::FilterWithBufferedInput::Put2` `0x00868e1a`: slot 5 of `CryptoPP::FilterWithBufferedInput::vftable` `0x00ce1e8c` and 4 more
- `CryptoPP::FilterWithBufferedInput::PutModifiable2` `0x00868e35`: slot 6 of `CryptoPP::FilterWithBufferedInput::vftable` `0x00ce1e8c` and 4 more
- `CryptoPP::FilterWithBufferedInput::InitializeDerivedAndReturnNewSizes` `0x00868e50`: slot 47 of `CryptoPP::FilterWithBufferedInput::vftable` `0x00ce1e8c` and 2 more
- `CryptoPP::FilterWithBufferedInput::InitializeDerived` `0x00868e5f`: slot 48 of `CryptoPP::FilterWithBufferedInput::vftable` `0x00ce1e8c` and 4 more
- `CryptoPP::FilterWithBufferedInput::NextPutSingle` `0x00868e62`: slot 50 of `CryptoPP::FilterWithBufferedInput::vftable` `0x00ce1e8c` and 4 more
- `CryptoPP::FilterWithBufferedInput::NextPutModifiable` `0x00868e65`: slot 52 of `CryptoPP::FilterWithBufferedInput::vftable` `0x00ce1e8c` and 2 more
- `CryptoPP::FilterWithBufferedInput::FlushDerived` `0x00868e6d`: slot 54 of `CryptoPP::FilterWithBufferedInput::vftable` `0x00ce1e8c` and 4 more
- `CryptoPP::FilterWithBufferedInput::NextPut` `0x00868e91`: slot 55 of `CryptoPP::FilterWithBufferedInput::vftable` `0x00ce1e8c` and 4 more
- `CryptoPP::HashVerificationFilter::AlgorithmName` `0x00868f16`: slot 2 of `CryptoPP::HashVerificationFilter::vftable` `0x00ce206c`
- `CryptoPP::SignatureVerificationFilter::AlgorithmName` `0x00868fa1`: slot 2 of `CryptoPP::SignatureVerificationFilter::vftable` `0x00ce216c`
- `CryptoPP::OutputProxy::CreatePutSpace` `0x0086902b`: slot 3 of `CryptoPP::OutputProxy::vftable` `0x00ce226c`
- `CryptoPP::OutputProxy::Put2` `0x0086903d`: slot 5 of `CryptoPP::OutputProxy::vftable` `0x00ce226c`
- `CryptoPP::OutputProxy::PutModifiable2` `0x0086906e`: slot 6 of `CryptoPP::OutputProxy::vftable` `0x00ce226c`
- `CryptoPP::OutputProxy::Initialize` `0x0086909f`: slot 10 of `CryptoPP::OutputProxy::vftable` `0x00ce226c`
- `CryptoPP::OutputProxy::Flush` `0x008690ba`: slot 11 of `CryptoPP::OutputProxy::vftable` `0x00ce226c`
- `CryptoPP::OutputProxy::MessageSeriesEnd` `0x008690d6`: slot 12 of `CryptoPP::OutputProxy::vftable` `0x00ce226c`
- `CryptoPP::OutputProxy::ChannelPut2` `0x008690f2`: slot 34 of `CryptoPP::OutputProxy::vftable` `0x00ce226c`
- `CryptoPP::OutputProxy::ChannelPutModifiable2` `0x00869129`: slot 35 of `CryptoPP::OutputProxy::vftable` `0x00ce226c`
- `CryptoPP::OutputProxy::ChannelFlush` `0x00869160`: slot 36 of `CryptoPP::OutputProxy::vftable` `0x00ce226c`
- `CryptoPP::OutputProxy::ChannelMessageSeriesEnd` `0x0086917f`: slot 37 of `CryptoPP::OutputProxy::vftable` `0x00ce226c`
- `CryptoPP::OutputProxy::`scalar_deleting_destructor'` `0x0086919e`: slot 0 of `CryptoPP::OutputProxy::vftable` `0x00ce226c`
- `CryptoPP::Filter::NewDefaultAttachment` `0x0086925d`: slot 44 of `CryptoPP::Deflator::vftable` `0x00ce18e4` and 12 more
- `CryptoPP::Filter::AttachedTransformation` `0x00869291`: slot 41 of `CryptoPP::Deflator::vftable` `0x00ce18e4` and 12 more
- `CryptoPP::Filter::AttachedTransformation` `0x008692ac`: slot 40 of `CryptoPP::Deflator::vftable` `0x00ce18e4` and 12 more
- `CryptoPP::Filter::Detach` `0x008692c7`: slot 42 of `CryptoPP::Deflator::vftable` `0x00ce18e4` and 12 more
- `CryptoPP::Filter::CopyRangeTo2` `0x00869300`: slot 32 of `CryptoPP::Deflator::vftable` `0x00ce18e4` and 12 more
- `CryptoPP::Filter::TransferTo2` `0x0086932b`: slot 31 of `CryptoPP::Deflator::vftable` `0x00ce18e4` and 12 more
- `CryptoPP::Filter::Initialize` `0x0086933a`: slot 10 of `CryptoPP::Deflator::vftable` `0x00ce18e4` and 12 more
- `CryptoPP::Filter::Flush` `0x0086935d`: slot 11 of `CryptoPP::Deflator::vftable` `0x00ce18e4` and 10 more
- `CryptoPP::Filter::MessageSeriesEnd` `0x0086939a`: slot 12 of `CryptoPP::Deflator::vftable` `0x00ce18e4` and 12 more
- `CryptoPP::FilterWithBufferedInput::`scalar_deleting_destructor'` `0x0086995a`: slot 0 of `CryptoPP::FilterWithBufferedInput::vftable` `0x00ce1e8c`
- `CryptoPP::FilterWithBufferedInput::IsolatedInitialize` `0x00869a3d`: slot 7 of `CryptoPP::FilterWithBufferedInput::vftable` `0x00ce1e8c` and 4 more
- `CryptoPP::FilterWithBufferedInput::IsolatedFlush` `0x00869ab9`: slot 8 of `CryptoPP::FilterWithBufferedInput::vftable` `0x00ce1e8c` and 3 more
- `CryptoPP::FilterWithBufferedInput::NextPutMultiple` `0x00869df3`: slot 51 of `CryptoPP::FilterWithBufferedInput::vftable` `0x00ce1e8c`
- `CryptoPP::ProxyFilter::`scalar_deleting_destructor'` `0x00869ed2`: slot 0 of `CryptoPP::ProxyFilter::vftable` `0x00ce232c`
- `CryptoPP::ProxyFilter::IsolatedFlush` `0x00869eef`: slot 8 of `CryptoPP::ProxyFilter::vftable` `0x00ce232c`
- `CryptoPP::ProxyFilter::NextPutMultiple` `0x00869f82`: slot 51 of `CryptoPP::ProxyFilter::vftable` `0x00ce232c`
- `CryptoPP::ProxyFilter::NextPutModifiable` `0x00869f9d`: slot 52 of `CryptoPP::ProxyFilter::vftable` `0x00ce232c`
- `CryptoPP::ArraySink::Put2` `0x00869fb8`: slot 5 of `CryptoPP::ArraySink::vftable` `0x00cdfc6c`
- `CryptoPP::ArraySink::CreatePutSpace` `0x0086a000`: slot 3 of `CryptoPP::ArraySink::vftable` `0x00cdfc6c`
- `CryptoPP::ArraySink::IsolatedInitialize` `0x0086a021`: slot 7 of `CryptoPP::ArraySink::vftable` `0x00cdfc6c` and 1 more
- `CryptoPP::ArrayXorSink::Put2` `0x0086a09f`: slot 5 of `CryptoPP::ArrayXorSink::vftable` `0x00ce29b4`
- `CryptoPP::StreamTransformationFilter::`scalar_deleting_destructor'` `0x0086a20a`: slot 0 of `CryptoPP::StreamTransformationFilter::vftable` `0x00ce1f7c`
- `CryptoPP::StreamTransformationFilter::FirstPut` `0x0086a23d`: slot 49 of `CryptoPP::StreamTransformationFilter::vftable` `0x00ce1f7c`
- `CryptoPP::StreamTransformationFilter::NextPutMultiple` `0x0086a275`: slot 51 of `CryptoPP::StreamTransformationFilter::vftable` `0x00ce1f7c`
- `CryptoPP::StreamTransformationFilter::NextPutModifiable` `0x0086a31a`: slot 52 of `CryptoPP::StreamTransformationFilter::vftable` `0x00ce1f7c`
- `CryptoPP::StreamTransformationFilter::LastPut` `0x0086a351`: slot 53 of `CryptoPP::StreamTransformationFilter::vftable` `0x00ce1f7c`
- `CryptoPP::HashFilter::IsolatedInitialize` `0x0086a6bb`: slot 7 of `CryptoPP::HashFilter::vftable` `0x00ce28e4`
- `CryptoPP::HashFilter::Put2` `0x0086a705`: slot 5 of `CryptoPP::HashFilter::vftable` `0x00ce28e4`
- `CryptoPP::HashVerificationFilter::`scalar_deleting_destructor'` `0x0086a86b`: slot 0 of `CryptoPP::HashVerificationFilter::vftable` `0x00ce206c`
- `CryptoPP::HashVerificationFilter::InitializeDerivedAndReturnNewSizes` `0x0086a8ca`: slot 47 of `CryptoPP::HashVerificationFilter::vftable` `0x00ce206c`
- `CryptoPP::HashVerificationFilter::FirstPut` `0x0086a925`: slot 49 of `CryptoPP::HashVerificationFilter::vftable` `0x00ce206c`
- `CryptoPP::HashVerificationFilter::NextPutMultiple` `0x0086a97d`: slot 51 of `CryptoPP::HashVerificationFilter::vftable` `0x00ce206c`
- `CryptoPP::HashVerificationFilter::LastPut` `0x0086a9b7`: slot 53 of `CryptoPP::HashVerificationFilter::vftable` `0x00ce206c`
- `CryptoPP::SignatureVerificationFilter::`scalar_deleting_destructor'` `0x0086ac24`: slot 0 of `CryptoPP::SignatureVerificationFilter::vftable` `0x00ce216c`
- `CryptoPP::SignatureVerificationFilter::InitializeDerivedAndReturnNewSizes` `0x0086ac7c`: slot 47 of `CryptoPP::SignatureVerificationFilter::vftable` `0x00ce216c`
- `CryptoPP::SignatureVerificationFilter::FirstPut` `0x0086ace0`: slot 49 of `CryptoPP::SignatureVerificationFilter::vftable` `0x00ce216c`
- `CryptoPP::SignatureVerificationFilter::NextPutMultiple` `0x0086ad61`: slot 51 of `CryptoPP::SignatureVerificationFilter::vftable` `0x00ce216c`
- `CryptoPP::SignatureVerificationFilter::LastPut` `0x0086ad9b`: slot 53 of `CryptoPP::SignatureVerificationFilter::vftable` `0x00ce216c`
- `CryptoPP::Store::GetNextMessage` `0x0086ae9a`: slot 25 of `CryptoPP::BERGeneralDecoder::vftable` `0x00ce262c` and 2 more
- `CryptoPP::StringStore::StoreInitialize` `0x0086af02`: slot 44 of `CryptoPP::StringStore::vftable` `0x00ce0224`
- `CryptoPP::StringStore::TransferTo2` `0x0086af9e`: slot 31 of `CryptoPP::StringStore::vftable` `0x00ce0224`
- `CryptoPP::StringStore::CopyRangeTo2` `0x0086afe2`: slot 32 of `CryptoPP::StringStore::vftable` `0x00ce0224`
- `CryptoPP::CustomSignalPropagation<Sink>::IsolatedInitialize` `0x0086b2f4`: slot 7 of `CryptoPP::OutputProxy::vftable` `0x00ce226c`
- `CryptoPP::CustomFlushPropagation<Sink>::IsolatedFlush` `0x0086b2f7`: slot 8 of `CryptoPP::OutputProxy::vftable` `0x00ce226c`
- `CryptoPP::ByteQueue::`scalar_deleting_destructor'` `0x0086c8a0`: slot 0 of `CryptoPP::ByteQueue::vftable` `0x00ce24ac`
- `CryptoPP::ByteQueue::IsolatedInitialize` `0x0086ca31`: slot 7 of `CryptoPP::ByteQueue::vftable` `0x00ce24ac` and 2 more
- `CryptoPP::ByteQueue::Put2` `0x0086cadd`: slot 5 of `CryptoPP::ByteQueue::vftable` `0x00ce24ac` and 2 more
- `CryptoPP::ByteQueue::Get` `0x0086ccab`: slot 18 of `CryptoPP::ByteQueue::vftable` `0x00ce24ac` and 2 more
- `CryptoPP::ByteQueue::Get` `0x0086ccfa`: slot 17 of `CryptoPP::ByteQueue::vftable` `0x00ce24ac` and 2 more
- `CryptoPP::ByteQueue::Peek` `0x0086cd39`: slot 20 of `CryptoPP::ByteQueue::vftable` `0x00ce24ac` and 2 more
- `CryptoPP::ByteQueue::Peek` `0x0086cd69`: slot 19 of `CryptoPP::ByteQueue::vftable` `0x00ce24ac` and 2 more
- `CryptoPP::ByteQueue::TransferTo2` `0x0086cda1`: slot 31 of `CryptoPP::ByteQueue::vftable` `0x00ce24ac` and 2 more
- `CryptoPP::ByteQueue::CopyRangeTo2` `0x0086ce97`: slot 32 of `CryptoPP::ByteQueue::vftable` `0x00ce24ac` and 2 more
- `CryptoPP::ByteQueue::CreatePutSpace` `0x0086cfba`: slot 3 of `CryptoPP::ByteQueue::vftable` `0x00ce24ac` and 2 more
- `CryptoPP::BERGeneralDecoder::`scalar_deleting_destructor'` `0x0086df61`: slot 0 of `CryptoPP::BERGeneralDecoder::vftable` `0x00ce262c`
- `CryptoPP::BERGeneralDecoder::TransferTo2` `0x0086e0f1`: slot 31 of `CryptoPP::BERGeneralDecoder::vftable` `0x00ce262c` and 1 more
- `CryptoPP::BERGeneralDecoder::CopyRangeTo2` `0x0086e145`: slot 32 of `CryptoPP::BERGeneralDecoder::vftable` `0x00ce262c` and 1 more
- `CryptoPP::DERGeneralEncoder::`scalar_deleting_destructor'` `0x0086e1f6`: slot 0 of `CryptoPP::DERGeneralEncoder::vftable` `0x00ce26f4`
- `CryptoPP::HashFilter::AlgorithmName` `0x0087395e`: slot 2 of `CryptoPP::HashFilter::vftable` `0x00ce28e4`
- `CryptoPP::HashFilter::CreatePutSpace` `0x00873978`: slot 3 of `CryptoPP::HashFilter::vftable` `0x00ce28e4`
- `CryptoPP::HashFilter::`scalar_deleting_destructor'` `0x00873980`: slot 0 of `CryptoPP::HashFilter::vftable` `0x00ce28e4`
- `CryptoPP::ArrayXorSink::CreatePutSpace` `0x008739df`: slot 3 of `CryptoPP::ArrayXorSink::vftable` `0x00ce29b4`
- `CryptoPP::ArrayXorSink::`scalar_deleting_destructor'` `0x008739eb`: slot 0 of `CryptoPP::ArrayXorSink::vftable` `0x00ce29b4`
- `CryptoPP::Bufferless<Filter>::IsolatedFlush` `0x008742dd`: slot 8 of `CryptoPP::HashFilter::vftable` `0x00ce28e4`
- `CryptoPP::Inflator::GetLog2WindowSize` `0x0087469a`: slot 47 of `CryptoPP::Inflator::vftable` `0x00ce2dd4`
- `CryptoPP::Inflator::MaxPrestreamHeaderSize` `0x0087469e`: slot 48 of `CryptoPP::Inflator::vftable` `0x00ce2dd4`
- `CryptoPP::Inflator::ProcessPrestreamHeader` `0x008746a1`: slot 49 of `CryptoPP::Inflator::vftable` `0x00ce2dd4`
- `CryptoPP::Inflator::ProcessDecompressedData` `0x008746a2`: slot 50 of `CryptoPP::Inflator::vftable` `0x00ce2dd4`
- `CryptoPP::Inflator::MaxPoststreamTailSize` `0x008746c0`: slot 51 of `CryptoPP::Inflator::vftable` `0x00ce2dd4`
- `CryptoPP::Inflator::ProcessPoststreamTail` `0x008746c3`: slot 52 of `CryptoPP::Inflator::vftable` `0x00ce2dd4`
- `CryptoPP::Inflator::`scalar_deleting_destructor'` `0x00874c72`: slot 0 of `CryptoPP::Inflator::vftable` `0x00ce2dd4`
- `CryptoPP::Inflator::IsolatedInitialize` `0x00874c8e`: slot 7 of `CryptoPP::Inflator::vftable` `0x00ce2dd4` and 1 more
- `CryptoPP::Inflator::Put2` `0x00874e8e`: slot 5 of `CryptoPP::Inflator::vftable` `0x00ce2dd4` and 1 more
- `CryptoPP::Inflator::IsolatedFlush` `0x00874f6b`: slot 8 of `CryptoPP::Inflator::vftable` `0x00ce2dd4` and 1 more
- `CryptoPP::MessageQueue::IsolatedInitialize` `0x00877002`: slot 7 of `CryptoPP::MessageQueue::vftable` `0x00ce2f44`
- `CryptoPP::MessageQueue::Put2` `0x0087703c`: slot 5 of `CryptoPP::MessageQueue::vftable` `0x00ce2f44`
- `CryptoPP::MessageQueue::IsolatedFlush` `0x008770b2`: slot 8 of `CryptoPP::MessageQueue::vftable` `0x00ce2f44`
- `CryptoPP::MessageQueue::IsolatedMessageSeriesEnd` `0x008770b7`: slot 9 of `CryptoPP::MessageQueue::vftable` `0x00ce2f44`
- `CryptoPP::MessageQueue::MaxRetrievable` `0x008770d1`: slot 15 of `CryptoPP::MessageQueue::vftable` `0x00ce2f44`
- `CryptoPP::MessageQueue::AnyRetrievable` `0x008770f3`: slot 16 of `CryptoPP::MessageQueue::vftable` `0x00ce2f44`
- `CryptoPP::MessageQueue::TotalBytesRetrievable` `0x00877120`: slot 22 of `CryptoPP::MessageQueue::vftable` `0x00ce2f44`
- `CryptoPP::MessageQueue::NumberOfMessages` `0x00877128`: slot 23 of `CryptoPP::MessageQueue::vftable` `0x00ce2f44`
- `CryptoPP::MessageQueue::NumberOfMessagesInThisSeries` `0x0087712d`: slot 29 of `CryptoPP::MessageQueue::vftable` `0x00ce2f44`
- `CryptoPP::MessageQueue::NumberOfMessageSeries` `0x0087714a`: slot 30 of `CryptoPP::MessageQueue::vftable` `0x00ce2f44`
- `CryptoPP::MessageQueue::`scalar_deleting_destructor'` `0x00877252`: slot 0 of `CryptoPP::MessageQueue::vftable` `0x00ce2f44`
- `CryptoPP::MessageQueue::CopyRangeTo2` `0x0087726e`: slot 32 of `CryptoPP::MessageQueue::vftable` `0x00ce2f44`
- `CryptoPP::MessageQueue::TransferTo2` `0x008772d7`: slot 31 of `CryptoPP::MessageQueue::vftable` `0x00ce2f44`
- `CryptoPP::MessageQueue::GetNextMessage` `0x0087734a`: slot 25 of `CryptoPP::MessageQueue::vftable` `0x00ce2f44`
- `CryptoPP::AutoSignaling<BufferedTransformation>::SetAutoSignalPropagation` `0x00877718`: slot 13 of `CryptoPP::MessageQueue::vftable` `0x00ce2f44`
- `CryptoPP::AutoSignaling<BufferedTransformation>::GetAutoSignalPropagation` `0x00877722`: slot 14 of `CryptoPP::MessageQueue::vftable` `0x00ce2f44`

## CRYPTOPP_STRING (15)

- `CryptoPP::SimpleKeyingInterface::ThrowIfResynchronizable` `0x0084b813`: 1 unique string of `CryptoPP::SimpleKeyingInterface::ThrowIfResynchronizable` (cryptlib.cpp): ": this object requires an IV"
- `CryptoPP::SimpleKeyingInterface::ThrowIfInvalidIV` `0x0084b877`: 1 unique string of `CryptoPP::SimpleKeyingInterface::ThrowIfInvalidIV` (cryptlib.cpp): ": this object cannot use a null IV"
- `CryptoPP::StreamTransformation::ProcessLastBlock` `0x0084b96a`: 1 unique string of `CryptoPP::StreamTransformation::ProcessLastBlock` (cryptlib.cpp): "StreamTransformation: this object does't support a special last block"
- `CryptoPP::HashTransformation::ThrowIfInvalidTruncatedSize` `0x0084bbb8`: 2 unique strings of `CryptoPP::HashTransformation::ThrowIfInvalidTruncatedSize` (cryptlib.cpp): " byte digest to ", "HashTransformation: can't truncate a "
- `CryptoPP::Integer::GenerateRandomNoThrow` `0x0085279a`: 4 unique strings of `CryptoPP::Integer::GenerateRandomNoThrow` (integer.cpp): "BitLength", "Integer: invalid EquivalentTo and/or Mod argument", "Integer: invalid RandomNumberType argument", "Integer: missing Max argument"
- `CryptoPP::MontgomeryRepresentation::MontgomeryRepresentation` `0x00854d0a`: 1 unique string of `CryptoPP::MontgomeryRepresentation::MontgomeryRepresentation` (integer.cpp): "MontgomeryRepresentation: Montgomery representation requires an odd modulus"
- `CryptoPP::RandomPool::TransferTo2` `0x00861dd3`: 1 unique string of `CryptoPP::RandomPool::TransferTo2` (randpool.cpp): "RandomPool: nonblocking transfer is not implemented by this object"
- `CryptoPP::CipherModeBase::GetNextIV` `0x0086379c`: 1 unique string of `CryptoPP::CipherModeBase::GetNextIV` (modes.cpp): "CipherModeBase: GetNextIV() must be called on an encryption object"
- `CryptoPP::CBC_CTS_Encryption::ProcessLastBlock` `0x00863af6`: 1 unique string of `CryptoPP::CBC_CTS_Encryption::ProcessLastBlock` (modes.cpp): "CBC_Encryption: message is too short for ciphertext stealing"
- `CryptoPP::Deflator::SetDeflateLevel` `0x008660fc`: 1 unique string of `CryptoPP::Deflator::SetDeflateLevel` (zdeflate.cpp): " is an invalid deflate level"
- `CryptoPP::StreamTransformationFilter::StreamTransformationFilter` `0x0086a12c`: 1 unique string of `CryptoPP::StreamTransformationFilter::StreamTransformationFilter` (filters.cpp): "StreamTransformationFilter: PKCS_PADDING and ONE_AND_ZEROS_PADDING cannot be used with "
- `CryptoPP::RandomNumberStore::TransferTo2` `0x0086b092`: 1 unique string of `CryptoPP::RandomNumberStore::TransferTo2` (filters.cpp): "RandomNumberStore: nonblocking transfer is not implemented by this object"
- `CryptoPP::ByteQueue::UndoLazyPut` `0x0086cc3f`: 1 unique string of `CryptoPP::ByteQueue::UndoLazyPut` (queue.cpp): "ByteQueue: size specified for UndoLazyPut is too large"
- `CryptoPP::TF_EncryptorBase::Encrypt` `0x008741c8`: 1 unique string of `CryptoPP::TF_EncryptorBase::Encrypt` (pubkey.cpp): ": message too long for this public key"
- `CryptoPP::HuffmanDecoder::Initialize` `0x0087477f`: 4 unique strings of `CryptoPP::HuffmanDecoder::Initialize` (zinflate.cpp): "code length exceeds maximum", "codes incomplete", "codes oversubscribed", "null code"
