# UOP header names

Program: `UOSA.exe`, SHA-256 `5a5e9dc536c836d1bf10b6f86bb88f61a6ae078f519d4060d3bc47aecbebad50`.

Mode: report only.

| Address | Name | Checks | Result |
|---|---|---|---|
| `0x0098d044` | `UOP::MypArchive_ctor` | vftable:MypArchive, const:22, const:fd23ec43, calledby:0098a341 | already applied |
| `0x0098db38` | `UOP::MypArchive_EntryHashTable_Init` | calledby:0098d044 | already applied |
| `0x0098db81` | `UOP::MypArchive_EntryHashTable_Insert` | calledby:0098d044 | already applied |
| `0x0098d447` | `UOP::MypArchive_IsOpen` | slot:MypArchive:1 | already applied |
| `0x0098d53d` | `UOP::MypArchive_GetEntryTimestamp` | slot:MypArchive:2 | already applied |
| `0x0098d670` | `UOP::MypArchive_ReadEntryRange` | slot:MypArchive:5, calls:0098db06 | already applied |
| `0x0098d76e` | `UOP::MypArchive_GetHeaderRecord` | slot:MypArchive:6, calls:0098db06, const:800 | already applied |
| `0x0098da27` | `UOP::SwapHeaderRecordPrefix` | calledby:0098d76e | already applied |
| `0x0098d8d1` | `UOP::MypArchive_GetHeaderString` | slot:MypArchive:7 | already applied |
| `0x0098d93b` | `UOP::MypArchive_IsValidIndex` | slot:MypArchive:8 | already applied |
| `0x0098d978` | `UOP::MypArchive_GetEntryDataBlockHash` | calls:0098db06, calledby:0098a1f5 | already applied |
| `0x0098d995` | `UOP::MypArchive_GetEntryPayloadOffset` | calls:0098db06, calledby:0098e39d | already applied |
| `0x0098d9c0` | `UOP::MypArchive_CollectEntryTimestamps` | calls:0098d520 | already applied |
| `0x0098a1f5` | `UOP::GetFileDataBlockHash` | calls:0098d487, calls:0098d460, calls:0098d978 | already applied |
| `0x0098a125` | `UOP::GetFileTimestamp` | calls:0098d487, calls:0098d460 | already applied |
| `0x0098a29b` | `UOP::GetFileDecompressedSize` | calls:0098d487, calls:0098d503 | already applied |
| `0x0098a5a8` | `UOP::FindArchiveEntry` | calls:0098d487, calls:0098d460, calledby:0098e39d | already applied |
| `0x00989fdb` | `UOP::FileExists` | calls:0098d487, calls:0098d460, calls:0098afd4 | already applied |
| `0x00989da6` | `UOP::ReadFileRange` | calls:0098e31b, calls:0098d460 | already applied |
| `0x0098e31b` | `UOP::ReadLooseFileRange` | calledby:00989da6 | already applied |
| `0x0098e39d` | `UOP::OpenFileHandleAtPayload` | calls:0098a5a8, calls:0098d995 | already applied |
| `0x0088e39d` | `UOP::GetFileStamp` | calls:00989fdb, calls:0098a1f5 | already applied |
| `0x0088e3ca` | `AudioConfig_CollectFileStamps` | string:audio/audio_filelist.csv, calls:0088e39d | already applied |
| `0x009a310c` | `StringTable_BuildCache` | string:Called BuildCache() after loading!\n, calls:0098a1f5 | already applied |
