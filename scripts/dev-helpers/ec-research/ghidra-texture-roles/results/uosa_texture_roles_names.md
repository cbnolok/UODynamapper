# Texture role names

Program: `UOSA.exe`, SHA-256 `5a5e9dc536c836d1bf10b6f86bb88f61a6ae078f519d4060d3bc47aecbebad50`.

Mode: `--apply` (changes: {tag created=1, functions renamed=9, tags added=9, plate comments set=9}).

| Address | Name | Checks | Result |
|---|---|---|---|
| `0x0045cd80` | `BuildShaderDefinitionMaterial` | string:UOWaterShader, string:Build\Hues\Hues.dds, calls:00670bd0, calls:004169a0 | named, tagged, comment set |
| `0x00670bd0` | `NiTexturingProperty::SetShaderMap` | calledby:0045cd80, const:cbf53c | named, tagged, comment set |
| `0x004169a0` | `NiTexturingProperty::SetBaseTexture` | calledby:0045cd80, const:cbf528 | named, tagged, comment set |
| `0x005940e0` | `UOSpriteShaderHLSL::BindSamplers` | vtslot:00cb425c:7 | named, tagged, comment set |
| `0x00593b30` | `UOSpriteShaderHLSL::LoadPixelShader` | string:UOSprite.psh | named, tagged, comment set |
| `0x00592a00` | `UOSpriteShaderFFP::SetupTextureStages` | vtslot:00cb3fbc:40, vtslot:00cb4064:40, calls:0058dfc0 | named, tagged, comment set |
| `0x0058dfc0` | `BindShaderMapToTextureStage` | calledby:00592a00 | named, tagged, comment set |
| `0x0059bbd0` | `UOWaterShader::SetupTextureStages` | vtslot:00cb46bc:40, string:Technique, string:Tweaks | named, tagged, comment set |
| `0x00595a00` | `UOStaticTerrainShader::SetupTextureStages` | vtslot:00cb446c:40 | named, tagged, comment set |
