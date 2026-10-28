# UOP trace names

Program: `UOSA.exe`, SHA-256 `5a5e9dc536c836d1bf10b6f86bb88f61a6ae078f519d4060d3bc47aecbebad50`.

Mode: report only.

| Address | Name | Tag | Checks | Result |
|---|---|---|---|---|
| `0x00989f55` | `UOP::OpenFilePtr` | UOP_TRACE | calls:00989b72 | already applied |
| `0x009898a6` | `UOP::ArchiveFileOpen` | UOP_TRACE | calls:0098e5df, calls:0098d74c, calls:0098ce9f | already applied |
| `0x0098e5df` | `UOP::MythMypInputFile_ctor` | UOP_TRACE | vftable:MythMypInputFile, calls:0098d6ed, calls:0098e672 | already applied |
| `0x0098e672` | `UOP::MythMemoryInputFile_ctor` | UOP_TRACE | vftable:MythMemoryInputFile | already applied |
| `0x0098d6ed` | `UOP::MypArchive_ReadEntryAlloc` | UOP_TRACE | calls:0098db06, calledby:0098e5df | already applied |
| `0x0098d57c` | `UOP::MypArchive_ReadEntry` | UOP_TRACE | slot:MypArchive:3, calls:0098d6d3 | already applied |
| `0x0098d61b` | `UOP::MypArchive_ReadEntryRaw` | UOP_TRACE | slot:MypArchive:4, calls:0098db06 | already applied |
| `0x0098db06` | `UOP::MypArchive_GetEntry` | UOP_TRACE | const:22, calledby:0098d503 | already applied |
| `0x0098d503` | `UOP::MypArchive_GetEntryDecompressedSize` | UOP_TRACE | calls:0098db06, calledby:0098e5df | already applied |
| `0x0098d520` | `UOP::MypArchive_GetEntryCompressedSize` | UOP_TRACE | calls:0098db06, calledby:0098d57c | already applied |
| `0x0098d74c` | `UOP::MypArchive_GetEntryCompression` | UOP_TRACE | calls:0098db06, calledby:00989b72 | already applied |
| `0x0098ce9f` | `UOP::FileLoadJob_ReadCompressed` | UOP_TRACE | calls:0098d503, calls:0098d520, calledby:00989b72 | already applied |
| `0x0098cd97` | `UOP::FileLoadJobRequest_Execute` | UOP_TRACE | slot:FileLoadJobRequest:3, string:Decompress, calls:0098d6d3 | already applied |
| `0x0098cf2c` | `UOP::DecompressJobRequest_Execute` | UOP_TRACE | slot:FileLoadJobRequest::DecompressJobRequest:3, string:Decompress, calls:0098d6d3 | already applied |
| `0x0098d6d3` | `UOP::Decompress` | UOP_TRACE | calls:009a2530, calledby:0098d57c | already applied |
| `0x009a2530` | `uncompress` | ZLIB_UOP_TRACE | calls:009a5bf0, calls:009a5d80, calls:009a71e0 | already applied |
| `0x009a5bf0` | `inflateInit_` | ZLIB_UOP_TRACE | const:1ba8, calls:009a7690, calls:009a5ba0 | already applied |
| `0x009a5ba0` | `inflateReset` | ZLIB_UOP_TRACE | calledby:009a5bf0 | already applied |
| `0x009a71e0` | `inflateEnd` | ZLIB_UOP_TRACE | calledby:009a2530 | already applied |
| `0x009a7690` | `zcalloc` | ZLIB_UOP_TRACE | calledby:009a5bf0 | already applied |
