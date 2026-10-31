# Terrain layer names

Program: `UOSA.exe`, SHA-256 `5a5e9dc536c836d1bf10b6f86bb88f61a6ae078f519d4060d3bc47aecbebad50`.

Mode: `--apply` (changes: {tag created=1, functions renamed=26, tags added=26, plate comments set=26}).

| Address | Name | Checks | Result |
|---|---|---|---|
| `0x004616e0` | `UOTerrain::CreateTerrainLayerForShader` | calledby:00461800, calls:00597170, calls:00460190, calls:004600f0, string:UOWaterTerrainLayer, string:UODefaultTerrainLayer, string:UOBumpMapTerrainLayer | named, tagged, comment set |
| `0x00597170` | `UODefaultTerrainLayer::UODefaultTerrainLayer` | calledby:004616e0, vftable:UODefaultTerrainLayer | named, tagged, comment set |
| `0x00460190` | `UOWaterTerrainLayer::UOWaterTerrainLayer` | calledby:004616e0, vftable:UOWaterTerrainLayer | named, tagged, comment set |
| `0x004600f0` | `UOBumpMapTerrainLayer::UOBumpMapTerrainLayer` | calledby:004616e0, vftable:UOBumpMapTerrainLayer | named, tagged, comment set |
| `0x00597210` | `UODefaultTerrainLayer::Draw` | vtslot:00cb466c:1 | named, tagged, comment set |
| `0x005974f0` | `UODefaultTerrainLayer::SetupFourStages` | vtslot:00cb466c:16, vtslot:00ca0a0c:16, calls:0058dfc0 | named, tagged, comment set |
| `0x00597b50` | `UODefaultTerrainLayer::SetupTwoStages` | vtslot:00cb466c:17, vtslot:00ca0a0c:17, calls:0058dfc0 | named, tagged, comment set |
| `0x00597f00` | `UOBumpMapTerrainLayer::Draw` | vtslot:00ca0a0c:1, calls:0058dfc0, const:400000, const:30000 | named, tagged, comment set |
| `0x005988a0` | `UOWaterTerrainLayer::Draw` | vtslot:00ca0a5c:1, calls:0058dfc0, calls:00596930, const:200000, const:30000 | named, tagged, comment set |
| `0x00596930` | `GetTerrainPassTextureStage` | calledby:005988a0 | named, tagged, comment set |
| `0x004603f0` | `UOTerrain::MakeWorldArtTextureLocation` | calledby:004613b0, string:Data\WorldArt\ | named, tagged, comment set |
| `0x004600d0` | `UOBaseTerrainLayer::GetMaterialId` | vtslot:00cb4624:2 | named, tagged, comment set |
| `0x004600e0` | `UOBaseTerrainLayer::SetMaterialId` | vtslot:00cb4624:3 | named, tagged, comment set |
| `0x004601e0` | `UOWaterTerrainLayer::SetMaterialId` | vtslot:00ca0a5c:3 | named, tagged, comment set |
| `0x00596f30` | `UOBaseTerrainLayer::AddMap` | vtslot:00cb4624:4 | named, tagged, comment set |
| `0x00596f80` | `UOBaseTerrainLayer::SetMap` | vtslot:00cb4624:5 | named, tagged, comment set |
| `0x00596fd0` | `UOBaseTerrainLayer::GetMap` | vtslot:00cb4624:6 | named, tagged, comment set |
| `0x00597000` | `UOBaseTerrainLayer::MapCount` | vtslot:00cb4624:7 | named, tagged, comment set |
| `0x00597020` | `UOBaseTerrainLayer::AddTechnique` | vtslot:00cb4624:8 | named, tagged, comment set |
| `0x00597070` | `UOBaseTerrainLayer::SetTechnique` | vtslot:00cb4624:9 | named, tagged, comment set |
| `0x00597090` | `UOBaseTerrainLayer::GetTechnique` | vtslot:00cb4624:10 | named, tagged, comment set |
| `0x005970a0` | `UOBaseTerrainLayer::TechniqueCount` | vtslot:00cb4624:11 | named, tagged, comment set |
| `0x005970c0` | `UOBaseTerrainLayer::AddTweak` | vtslot:00cb4624:12 | named, tagged, comment set |
| `0x00597120` | `UOBaseTerrainLayer::SetTweak` | vtslot:00cb4624:13 | named, tagged, comment set |
| `0x00597140` | `UOBaseTerrainLayer::GetTweak` | vtslot:00cb4624:14 | named, tagged, comment set |
| `0x00597150` | `UOBaseTerrainLayer::TweakCount` | vtslot:00cb4624:15 | named, tagged, comment set |
