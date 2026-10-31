# Facet names

Program: `UOKR.exe` (tested build UOKR.exe), SHA-256 `1b802e5c00f3bf89a771778b9c6c57406be0703808eeb4cd983e2f5baa348a43`.

Mode: `--apply` (changes: {labels created=1, plate comments set=22, functions renamed=21, tags added=21}).

| Address | Name | Checks | Result |
|---|---|---|---|
| `0x00bfcc88` | `UOFacet::g_FacetDimensions` | first record {"Felucca", 7168, 4096}, width field read by 00589bd0, height field read by 00589ef0 | named, comment set |
| `0x007b6789` | `UOFacet::SectorFactory_Create` | slot:UOSectorInstanceBinaryFactory:1, calls:007b688f | named, tagged, comment set |
| `0x007b688f` | `UOFacet::Sector_Read` | calledby:007b6789, calls:007b663f, calls:007b6994, const:40 | named, tagged, comment set |
| `0x007b6994` | `UOFacet::Sector_ReadSquare` | calledby:007b688f, calls:007bfeae, calls:007b6ac9, calls:007b6ba6 | named, tagged, comment set |
| `0x007b6ac9` | `UOFacet::Sector_ReadDelimiter` | calledby:007b6994, calls:007c032b | named, tagged, comment set |
| `0x007b6ba6` | `UOFacet::Sector_ReadStatic` | calledby:007b6994, calls:007c0120 | named, tagged, comment set |
| `0x007b663f` | `UOFacet::SectorClientInstance_ctor` | calledby:007b688f | named, tagged, comment set |
| `0x007bfeae` | `UOFacet::SectorClientInstance_AddSquare` | calledby:007b6994 | named, tagged, comment set |
| `0x0058a080` | `UOFacet::RequestSector` | const:9000000, calledby:0058d890 | named, tagged, comment set |
| `0x00589ef0` | `UOFacet::SectorIndexFromXY` | calls:00bfcc90, calledby:0058d890 | named, tagged, comment set |
| `0x0058a860` | `UOFacet::QueueSectorChunks` | calls:00bfcc90 | named, tagged, comment set |
| `0x0058d890` | `UOFacet::BuildChunkFromSector` | calls:00589ef0, calls:0058a080 | named, tagged, comment set |
| `0x0058d880` | `UOFacet::ChunkLoadRequest_Execute` | slot:ChunkLoadRequest:1, calls:0058d890 | named, tagged, comment set |
| `0x00589bd0` | `UOFacet::SetPlayerChunkPosition` | calls:00bfcc8c, calls:00bfcc90 | named, tagged, comment set |
| `0x0057eed0` | `UOFacet::LoadWaypoints` | calls:00bfcc8c, calls:00bfcc90 | named, tagged, comment set |
| `0x004f8b30` | `UOFacet::FacetDds_GetMipOffset` | calls:00bfcc8c, calls:00bfcc90 | named, tagged, comment set |
| `0x004f88e0` | `UOFacet::FacetDds_Open` | string:%s%s%02d%s | named, tagged, comment set |
| `0x00457f60` | `UOFacet::StaticChunkBuildRequest_ctor` | string:StaticChunkBuildRequest(, vftable:StaticChunkBuildRequest | named, tagged, comment set |
| `0x004580c0` | `UOFacet::StaticChunkBuildRequest_Execute` | slot:StaticChunkBuildRequest:1 | named, tagged, comment set |
| `0x00458720` | `UOFacet::TerrainChunkBuildRequest_ctor` | string:TerrainChunkBuildRequest(, vftable:TerrainChunkBuildRequest | named, tagged, comment set |
| `0x00458890` | `UOFacet::TerrainChunkBuildRequest_Execute` | slot:TerrainChunkBuildRequest:1 | named, tagged, comment set |
| `0x006095b6` | `UOFacet::MountPackages` | string:StaticChunk.uop, string:TerrainChunk.uop, calls:00609552 | named, tagged, comment set |
