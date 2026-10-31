# Terrain names

Program: `UOSA.exe` (tested build UOSA.exe), SHA-256 `5a5e9dc536c836d1bf10b6f86bb88f61a6ae078f519d4060d3bc47aecbebad50`.

Mode: `--apply` (changes: {functions renamed=23, tags added=23, plate comments set=23}).

| Address | Name | Checks | Result |
|---|---|---|---|
| `0x00845b04` | `UOTerrain::TerrainDefinitionFactory_Create` | slot:UOTerrainDefinitionBinaryFactory:1, calls:00845c25 | named, tagged, comment set |
| `0x004cc5f0` | `UOTerrain::TerrainDefinitionFactory_CreateDefault` | slot:UOTerrainDefinitionBinaryFactory:4, calls:0083ac8f | named, tagged, comment set |
| `0x00845c25` | `UOTerrain::TerrainDefinition_Read` | calledby:00845b04, calls:00845da6, calls:008456a3, calls:008459c0 | named, tagged, comment set |
| `0x00845da6` | `UOTerrain::TerrainDefinition_ReadAliases` | calledby:00845c25, calls:0084596f | named, tagged, comment set |
| `0x0084596f` | `UOTerrain::TerrainDefinition_AddAlias` | calledby:00845da6 | named, tagged, comment set |
| `0x008459c0` | `UOTerrain::TerrainDefinition_SetShader` | calledby:00845c25 | named, tagged, comment set |
| `0x008459fe` | `UOTerrain::TerrainDefinition_Alloc` | calls:0083ac8f | named, tagged, comment set |
| `0x0083ac8f` | `UOTerrain::TerrainDefinition_ctor` | calledby:004cc5f0, calledby:008459fe | named, tagged, comment set |
| `0x0083ad1e` | `UOTerrain::TerrainDefinition_dtor` | calledby:0083ad02 | named, tagged, comment set |
| `0x0083ad02` | `UOTerrain::TerrainDefinition_ScalarDeletingDtor` | slot:UOTerrainDefinitionBase:1, calls:0083ad1e | named, tagged, comment set |
| `0x0083ac78` | `UOTerrain::TerrainDefinition_MemSize` | slot:UOTerrainDefinitionBase:0 | named, tagged, comment set |
| `0x004cc990` | `UOTerrain::GetTerrainDefinition` | calls:004cb460, calledby:00461ae0 | named, tagged, comment set |
| `0x004cb460` | `UOTerrain::ResourceRequest_AddTerrainDefinitionKey` | const:3000000, calledby:004cc990 | named, tagged, comment set |
| `0x00461ae0` | `UOTerrain::BuildTerrainMesh` | calls:004cc990, calls:00461800, calls:00463b60, calledby:005dbc00 | named, tagged, comment set |
| `0x00461800` | `UOTerrain::LoadTerrainLayer` | calls:00461060, calls:004613b0, calledby:00461ae0 | named, tagged, comment set |
| `0x00461060` | `UOTerrain::BuildTerrainCoverage` | calledby:00461800 | named, tagged, comment set |
| `0x004613b0` | `UOTerrain::LoadTerrainLayerTexture` | calledby:00461800 | named, tagged, comment set |
| `0x00463b60` | `UOTerrain::AddNoDrawTerrain` | calledby:00461ae0 | named, tagged, comment set |
| `0x004027b0` | `UOTerrain::LoadLegacyTerrainMap` | calledby:00403050 | named, tagged, comment set |
| `0x00403410` | `UOTerrain::LegacyTerrainMap_Lookup` | calledby:005e2f00 | named, tagged, comment set |
| `0x005de730` | `UOTerrain::GetLandFlags` | calledby:005f3b20 | named, tagged, comment set |
| `0x005de840` | `UOTerrain::GetLandMaterial` | calledby:005dc160 | named, tagged, comment set |
| `0x005f3b20` | `UOTerrain::CollectSurfaces` | calls:005de730, const:40, const:100 | named, tagged, comment set |
