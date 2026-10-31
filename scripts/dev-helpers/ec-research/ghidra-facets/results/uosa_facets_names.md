# Facet names

Program: `UOSA.exe` (tested build UOSA.exe), SHA-256 `5a5e9dc536c836d1bf10b6f86bb88f61a6ae078f519d4060d3bc47aecbebad50`.

Mode: `--apply` (changes: {plate comments refreshed=1, functions renamed=6, tags added=6, plate comments set=6}).

| Address | Name | Checks | Result |
|---|---|---|---|
| `0x00da72a8` | `UOFacet::g_FacetDimensions` | first record {"Felucca", 7168, 4096}, width field read by 005dfef0, height field read by 005df210 | comment refreshed |
| `0x0083c82f` | `UOFacet::FacetDefinitionFactory_Create` | slot:UOFacetDefinitionBinaryFactory:1, calls:0083c933, calls:00a749ed | already applied |
| `0x0083c933` | `UOFacet::FacetDefinition_Read` | calledby:0083c82f, calls:0083c7a7, calls:0083ca5f, calls:00a6fe55 | already applied |
| `0x0083c7a7` | `UOFacet::FacetDefinition_ctor` | calledby:0083c933 | already applied |
| `0x0083ca5f` | `UOFacet::FacetDefinition_ReadRegions` | calledby:0083c933, calls:0083cb53, calls:00846a0d | already applied |
| `0x0083cb53` | `UOFacet::FacetDefinition_ReadRegionRects` | calledby:0083ca5f, calls:00846bc6 | already applied |
| `0x00846a0d` | `UOFacet::FacetDefinition_AddRegion` | calledby:0083ca5f, calls:00846936 | already applied |
| `0x00846936` | `UOFacet::FacetRegion_ctor` | calledby:00846a0d | already applied |
| `0x00846bc6` | `UOFacet::FacetDefinition_AddRegionRect` | calledby:0083cb53 | already applied |
| `0x008469cf` | `UOFacet::FacetDefinition_GetFacetId` | slot:UOFacetDefinition:3, refs:1 | already applied |
| `0x008469d7` | `UOFacet::FacetDefinition_GetNameCliloc` | slot:UOFacetDefinition:4, refs:1 | already applied |
| `0x008469e8` | `UOFacet::FacetDefinition_GetWidth` | slot:UOFacetDefinition:5, refs:1 | already applied |
| `0x008469ec` | `UOFacet::FacetDefinition_GetHeight` | slot:UOFacetDefinition:6, refs:1 | already applied |
| `0x008469fd` | `UOFacet::FacetDefinition_GetHeaderByte0` | slot:UOFacetDefinition:7, refs:1 | already applied |
| `0x00846a01` | `UOFacet::FacetDefinition_GetHeaderByte1` | slot:UOFacetDefinition:8, refs:1 | already applied |
| `0x00846a09` | `UOFacet::FacetDefinition_GetHeaderByte2` | slot:UOFacetDefinition:9, refs:1 | already applied |
| `0x00846a5d` | `UOFacet::FacetDefinition_GetRegionCount` | slot:UOFacetDefinition:10, refs:1 | already applied |
| `0x00846a65` | `UOFacet::FacetDefinition_GetRegionCliloc` | slot:UOFacetDefinition:11, refs:1 | already applied |
| `0x00846a8a` | `UOFacet::FacetDefinition_GetRegionColor` | slot:UOFacetDefinition:12, refs:1 | already applied |
| `0x00846ad7` | `UOFacet::FacetDefinition_GetRegionPhysicalFacet` | slot:UOFacetDefinition:13, refs:1 | already applied |
| `0x00846afc` | `UOFacet::FacetDefinition_GetRegionBounds` | slot:UOFacetDefinition:14, refs:1 | already applied |
| `0x00846c20` | `UOFacet::FacetDefinition_GetRegionRectCount` | slot:UOFacetDefinition:15, refs:1 | already applied |
| `0x00846c58` | `UOFacet::FacetDefinition_GetRegionRect` | slot:UOFacetDefinition:16, refs:1 | already applied |
| `0x005a5290` | `UOFacet::GetFacetDefinition` | calls:004cb9a0, calls:00a6ced4, calledby:005dfe80 | already applied |
| `0x004cb9a0` | `UOFacet::ResourceRequest_AddFacetDefinitionKey` | const:17000000, calledby:005a5290 | already applied |
| `0x005dfe80` | `UOFacet::GetFacetDimensions` | calls:005a5290, calledby:00534d90 | already applied |
| `0x0083d094` | `UOFacet::SectorFactory_Create` | slot:UOSectorInstanceBinaryFactory:1, calls:0083d19a, calls:00a749ed | already applied |
| `0x0083d19a` | `UOFacet::Sector_Read` | calledby:0083d094, calls:0083cf4a, calls:0083d29f, const:40 | already applied |
| `0x0083d29f` | `UOFacet::Sector_ReadSquare` | calledby:0083d19a, calls:00847c59, calls:0083d3a0, calls:0083d47d | already applied |
| `0x0083d3a0` | `UOFacet::Sector_ReadDelimiter` | calledby:0083d29f, calls:008480ca | already applied |
| `0x0083d47d` | `UOFacet::Sector_ReadStatic` | calledby:0083d29f, calls:00847ebf | already applied |
| `0x0083cf4a` | `UOFacet::SectorClientInstance_ctor` | vftable:UOSectorClientInstance, calledby:0083d19a | already applied |
| `0x00847c59` | `UOFacet::SectorClientInstance_AddSquare` | calledby:0083d29f | already applied |
| `0x00847c21` | `UOFacet::SectorClientInstance_GetSquare` | slot:UOSectorClientInstance:7, refs:1 | already applied |
| `0x00847d24` | `UOFacet::SectorClientInstance_GetNeighbourZAndLand` | slot:UOSectorClientInstance:4, refs:1 | already applied |
| `0x005df3b0` | `UOFacet::RequestSector` | const:9000000, calls:00a6cfc8, calledby:005e2f00 | already applied |
| `0x005df210` | `UOFacet::SectorIndexFromXY` | calls:00da72b0, calledby:005e2f00 | already applied |
| `0x005dfc40` | `UOFacet::QueueSectorChunks` | calls:00da72b0 | already applied |
| `0x005e2f00` | `UOFacet::BuildChunkFromSector` | calls:005df3b0, calls:005df210, const:3000000 | already applied |
| `0x00a6c9fc` | `UOFacet::SwapFacetPackageVariants` | string:facet0x.uop, string:facet5.uop, calls:0098a3fe, calledby:00615730 | already applied |
| `0x00615730` | `UOFacet::ApplyFacetVariantFlag` | calls:00a6c9fc, calledby:00612a80 | already applied |
| `0x00534b60` | `UOFacet::FacetDds_Open` | calledby:00534d10, calledby:00534d90 | already applied |
| `0x00534d10` | `UOFacet::FacetDds_GetMipCount` | calls:00534b60, calledby:005118c0 | already applied |
| `0x00534d90` | `UOFacet::FacetDds_CopyRegionBlocks` | calls:00534b60, calls:00487410, calls:005dfe80, calls:00535b30, calls:00535ca0, calls:00535d10 | already applied |
| `0x00535960` | `UOFacet::Radar_FindAreaAtLocation` | calls:005a5290, calledby:00510b40 | already applied |
| `0x00535b30` | `UOFacet::Radar_GetAreaColor` | calls:005a5290, calledby:00534d90 | already applied |
| `0x00535ca0` | `UOFacet::Radar_GetAreaRectCount` | calls:005a5290, calledby:00534d90 | already applied |
| `0x00535d10` | `UOFacet::Radar_GetAreaRect` | calls:005a5290, calledby:00534d90 | already applied |
| `0x00535ba0` | `UOFacet::Radar_IsLocationInArea` | calls:005a5290, calledby:00513ea0 | already applied |
| `0x00511bf0` | `UOFacet::Radar_GetMaxZoomForArea` | calls:005a5290, calledby:005140e0 | already applied |
| `0x00510b40` | `UOFacet::Radar_SetWindowSize` | calls:005a5290, calls:00535960, calledby:00512340 | already applied |
| `0x00510e60` | `UOFacet::Radar_SetView` | calls:005a5290, calls:005118c0, calledby:00512940 | already applied |
| `0x005118c0` | `UOFacet::Radar_Update` | calls:00534d10, calls:00511b10, calls:00535310 | already applied |
| `0x00511b10` | `UOFacet::Radar_RebuildTexture` | calls:00534d90, calledby:005118c0 | already applied |
| `0x00535310` | `UOFacet::Radar_OverlayStaticsInRect` | calls:00535500, calledby:005118c0 | already applied |
| `0x00535500` | `UOFacet::Radar_RecolorBlock` | calls:00487970, calls:00487410, calls:005bda40, calledby:00535310 | already applied |
| `0x005110c0` | `UOFacet::Radar_CollectObjectColors` | calls:005bda40, calledby:00511380 | already applied |
| `0x00487970` | `UOFacet::Dxt1_DecodeBlock` | calledby:00535500 | already applied |
| `0x00487410` | `UOFacet::Dxt1_EncodeBlock` | calledby:00535500, calledby:00534d90 | already applied |
| `0x005df060` | `UOFacet::SetPlayerChunkPosition` | calls:00da72ac, calls:00da72b0, calls:005dfef0 | named, tagged, comment set |
| `0x005dfef0` | `UOFacet::QueueNeighbourChunks` | calls:00da72ac, calls:00da72b0, calledby:005df060 | named, tagged, comment set |
| `0x005d3d90` | `UOFacet::LoadWaypoints` | calls:00da72ac, calls:00da72b0, calledby:005d2ef0 | named, tagged, comment set |
| `0x005df670` | `UOFacet::IsSameFeluccaTrammelHalf` | const:1400, calledby:005e2f00, calledby:005db3f0 | named, tagged, comment set |
| `0x005e94a0` | `UOFacet::Handle_0xBF_08_SetFacet` | calls:005dc720, calls:005e7c80, calledby:005e7ed0 | named, tagged, comment set |
| `0x005dc720` | `UOFacet::SetFacet` | calls:00cb6254, calledby:005e94a0 | named, tagged, comment set |
