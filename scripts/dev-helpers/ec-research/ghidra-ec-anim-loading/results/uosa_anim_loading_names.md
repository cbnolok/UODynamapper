# UOSA.exe animation loading names

Program: `UOSA.exe` (tested file UOSA.exe (Enhanced Client)), SHA-256 `5a5e9dc536c836d1bf10b6f86bb88f61a6ae078f519d4060d3bc47aecbebad50`.

Mode: `--apply` (changes: {tag created=1, functions renamed=29, tags added=29, plate comments set=29}).

| Address | Name | Checks | Result |
|---|---|---|---|
| `0x00a70565` | `UOAssets::Cache_FindAndTouch` | calledby:00a6cfc8, calls:00a70361 | named, tagged, comment set |
| `0x00a70361` | `UOAssets::Cache_TouchEntry` | calledby:00a70565, calls:0099a7c7 | named, tagged, comment set |
| `0x00a6d7b1` | `UOAssets::StoreAsset` | calledby:00a6cfc8, calls:00a70665 | named, tagged, comment set |
| `0x00a70665` | `UOAssets::Cache_Insert` | calledby:00a6d7b1 | named, tagged, comment set |
| `0x00a72988` | `UOAssets::Factory_LoadsFromBlob` (probable) | calledby:00a6cfc8, calls:00a729d0 | named, tagged, comment set |
| `0x00a6ced4` | `UOAssets::LoadRequest` | calledby:00433a40, calls:00a6cfc8 | named, tagged, comment set |
| `0x00a6d9dd` | `UOAssets::Tick` | calledby:00482530, calls:00a70b66 | named, tagged, comment set |
| `0x00a70b66` | `UOAssets::Cache_PeriodicSweep` | calls:00a70809, calls:00a712b7, const:2710 | named, tagged, comment set |
| `0x00a70809` | `UOAssets::Cache_Sweep` | calledby:00a70b66, calls:00a70a7f | named, tagged, comment set |
| `0x00a70a7f` | `UOAssets::Cache_EvictEntries` | calledby:00a70809, calls:00a70382, calls:00a708d4 | named, tagged, comment set |
| `0x00a708d4` | `UOAssets::Cache_ShouldEvict` | calls:00a7085e, calls:00a7089d, const:ec, const:f4 | named, tagged, comment set |
| `0x00a7085e` | `UOAssets::Cache_EntryRecentlyUsed` | calledby:00a708d4, calls:0099a7c7, const:100 | named, tagged, comment set |
| `0x00a7089d` | `UOAssets::Cache_LowPhysicalMemory` | import:GlobalMemoryStatus, const:f4 | named, tagged, comment set |
| `0x00a70382` | `UOAssets::Cache_EntryState` | calledby:00a70a7f, calledby:00a70939 | named, tagged, comment set |
| `0x00a70939` | `UOAssets::Cache_Statistics` (probable) | calls:00a70382 | named, tagged, comment set |
| `0x00a703ac` | `UOAssets::Cache_Construct` | const:6ddd0, const:2710 | named, tagged, comment set |
| `0x00a70f40` | `UOAssets::Cache_SetMemoryThreshold` (probable) | import:GlobalMemoryStatus, const:f4 | named, tagged, comment set |
| `0x0099a7c7` | `UOClock::Now100us` | calls:0099a74f, const:2710 | named, tagged, comment set |
| `0x00482530` | `UOApplication::MainLoop` (probable) | calls:00a6d9dd, import:Sleep | named, tagged, comment set |
| `0x0049a740` | `UOSettings::ApplyCacheSize` | calls:00e33624, calls:00e37230, const:f4 | named, tagged, comment set |
| `0x0043afe0` | `UOAnimations::FrameAtlasCache_Find` | calledby:00431a10 | named, tagged, comment set |
| `0x00431770` | `MobAnimTexture::Construct` | calledby:00431a10, calls:00c9f944 | named, tagged, comment set |
| `0x004328b0` | `MobAnimTriShape::RenderImmediate` (probable) | ref:00c9fa34, calls:00431a10 | named, tagged, comment set |
| `0x00432110` | `MobAnimTriShapeShadow::RenderImmediate` (probable) | ref:00c9fadc, calls:00431a10 | named, tagged, comment set |
| `0x0075eac0` | `NiDX9Renderer::CreateSourceTextureRendererData` | ref:00cd4d48, calls:00770b90 | named, tagged, comment set |
| `0x00771190` | `NiDX9SourceTextureData::CreateD3DTexture` | calledby:00770b90 | named, tagged, comment set |
| `0x007712a0` | `NiDX9SourceTextureData::CopyPixels` | calledby:00770b90, calls:00771380 | named, tagged, comment set |
| `0x00771380` | `NiDX9SourceTextureData::CopyLevelToSurface` | calledby:007712a0 | named, tagged, comment set |
| `0x0043adf0` | `UOAnimations::PushLayerRecord` | calledby:00436d40 | named, tagged, comment set |
