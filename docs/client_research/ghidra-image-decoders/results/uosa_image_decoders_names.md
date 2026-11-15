# Image decoder names

Program: `UOSA.exe`, SHA-256 `5a5e9dc536c836d1bf10b6f86bb88f61a6ae078f519d4060d3bc47aecbebad50`.

Mode: `--apply` (changes: none).

| Address | Name | Checks | Result |
|---|---|---|---|
| `0x006d3140` | `NiBMPReader::CanReadFile` | vtslot:00ccc5ec:1 | already applied |
| `0x006d3160` | `NiBMPReader::ReadFile` | vtslot:00ccc5ec:2 | already applied |
| `0x006d2de0` | `NiBMPReader::ReadHeader` | vtslot:00ccc5ec:3, const:4d42 | already applied |
| `0x006d3760` | `NiDDSReader::NiDDSReader` | calls:00ccc5fc, calledby:00693cf0 | already applied |
| `0x006d3d20` | `NiDDSReader::CanReadFile` | vtslot:00ccc5fc:1 | already applied |
| `0x006d48f0` | `NiDDSReader::ReadFile` | vtslot:00ccc5fc:2, calls:0068cad0 | already applied |
| `0x006d3810` | `NiDDSReader::ReadHeader` | vtslot:00ccc5fc:3, const:20534444, const:7c | already applied |
| `0x006d4e80` | `NiSGIReader::NiSGIReader` | calls:00ccc634, calledby:00693cf0 | already applied |
| `0x006d5300` | `NiSGIReader::ReadFile` | vtslot:00ccc634:2 | already applied |
| `0x006d5040` | `NiSGIReader::ReadHeader` | vtslot:00ccc634:3 | already applied |
| `0x006d64e0` | `NiTGAReader::CanReadFile` | vtslot:00ccc654:1, calls:00ccc64c | already applied |
| `0x006d6520` | `NiTGAReader::ReadFile` | vtslot:00ccc654:2, calls:0047c760 | already applied |
| `0x0068da60` | `NiTexture::RendererData::DefaultTextureFailureCallback` | calls:0068ded0, calledby:00770b90 | already applied |
| `0x0067b1d0` | `NiD3DRenderer::Error` | calledby:00770b90 | already applied |
| `0x00aecc3d` | `LoadScratchTextureD3DX` | calls:00aecb87 | already applied |
| `0x0058d8e0` | `TextureManager::RequestTexture` | vtslot:00cb39b4:1, calls:0058d600, calls:0058d430 | already applied |
| `0x0058d600` | `TextureManager::LoadTextureDirect` | calledby:0058d8e0, calls:004ccc80, calls:00989fdb, const:11000000 | already applied |
| `0x0058d430` | `TextureManager::RequestTextureAsset` | calledby:0058d8e0, calls:00a6ced4, vftable:UOResourceRequest | already applied |
| `0x0058d270` | `SpriteTextureFileLocation::IsDirectLoad` | slot:SpriteTextureFileLocation:2, string:TileArtLegacy, const:160 | already applied |
| `0x0058d310` | `SpriteTextureFileLocation::GetAssetId` | slot:SpriteTextureFileLocation:3, calls:0058dbf0 | already applied |
| `0x004ccc80` | `UOTextureData::UOTextureData` | vftable:UOTextureData, calls:00989fdb, calls:0068a8b0 | already applied |
| `0x004cce20` | `UOTextureFactory::Create` | slot:UOTextureFactory:0, calls:004ccc80, calls:00a749ed | already applied |
| `0x00a6db3b` | `UOAssets::SetDefaultPackedTypes` | calledby:004a9150, const:1c8 | already applied |
| `0x00a6db93` | `UOAssets::LoadAssetLoaderConfig` | string:AssetLoaderConfig.xml, calls:00a6dadb, calledby:004a9150 | already applied |
| `0x00a6dadb` | `UOAssets::SetTypeFormat` | calls:00a6da73, calls:00a6da8a, const:160 | already applied |
| `0x00a6db20` | `UOAssets::SetAllTypesFormat` | calls:00a6dadb, calledby:00a6db93 | already applied |
| `0x00a6da73` | `UOAssets::HasRawMapping` | calledby:00a6dadb | already applied |
| `0x00a6da8a` | `UOAssets::HasPackedMapping` | calledby:00a6dadb | already applied |
| `0x00a74868` | `UOAssetMap::Load` | string:Data/assetmap.xml, calls:00a6d5e4 | already applied |
| `0x00a74200` | `UOAssetMap::ParsePatternMap` | string:patternMatch, string:suffix, string:padding | already applied |
| `0x00a74549` | `UOAssetMap::ParseBitmaskMap` | string:section, string:isDirectory | already applied |
| `0x00a73f5e` | `UOAssetMap::ParseSimpleMap` | string:resourcenumber, string:filename | already applied |
| `0x00519700` | `MakeGumpTextureLocation` | string:Data/Interface/Default/Textures/GumpArt/, string:%08d.tga | already applied |
| `0x00519570` | `MakeManagedTextureLocation` | string:Data/Interface/InterfaceCore/Textures/, string:missingFile_ID | already applied |
| `0x00911470` | `UITexture_CreateSourceTexture` | calls:0068a8b0 | already applied |
