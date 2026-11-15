# Atmosphere names

Program: `UOSA.exe` (tested file UOSA.exe (Enhanced Client)), SHA-256 `5a5e9dc536c836d1bf10b6f86bb88f61a6ae078f519d4060d3bc47aecbebad50`.

Mode: `--apply`, third pass (changes: {functions renamed=10, tags added=10, plate comments set=10}); the first-pass and second-pass rows were already applied. `UOSA.exe` was saved.

| Address | Name | Checks | Result |
|---|---|---|---|
| `0x00429950` | `UOLightingPostProcessEffect::BuildCompositeQuad` | calledby:0042a360, string:Tweaks | already applied |
| `0x0042a360` | `UOLightingPostProcessEffect::CreateCompositeResources` (probable) | calls:00429950, string:Data\\Textures\\00000020_daymap | already applied |
| `0x0058f7f0` | `UOLightingShader::SetupTextureStages` | ref:00cb3cf4 | already applied |
| `0x00460c70` | `UOTerrain::ComputeVertexNormal` | calledby:00461ae0 | already applied |
| `0x00c0d610` | `UOShaderLibrary::RegisterShaders` (probable) | string:UOSpriteShaderMob | already applied |
| `0x00429070` | `UOAtmosphere::InitPostProcessEffects` | calls:004561d0 | named, tagged, comment set |
| `0x00456060` | `UOShadowPostProcessEffect::Constructor` | const:ca0434, calls:00c9b2bc | named, tagged, comment set |
| `0x004561d0` | `UOShadowPostProcessEffect::SetIntensity` | calledby:00429070 | named, tagged, comment set |
| `0x00456880` | `UOShadowPostProcessEffect::Render` | ref:00ca04f4, calls:00456350 | named, tagged, comment set |
| `0x00456350` | `UOShadowPostProcessEffect::BuildCompositeQuad` | calledby:00456880 | named, tagged, comment set |
| `0x005921b0` | `UOShadowShader::SetupTextureStages` | ref:00cb3ebc | named, tagged, comment set |
| `0x005937e0` | `UOSpriteShaderShadow::SetupTextureStages` | ref:00cb4254 | named, tagged, comment set |
| `0x00493c20` | `UOSettings::SetVideoDefaults` (probable) | calls:00e33588 | named, tagged, comment set |
| `0x00424a20` | `UOFadePostProcessEffect::Constructor` | const:c9ea4c, calledby:00425800 | named, tagged, comment set |
| `0x004250d0` | `UOFadePostProcessEffect::Render` (probable) | string:UOFadeShader | named, tagged, comment set |
