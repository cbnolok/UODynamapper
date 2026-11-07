# Atmosphere names

Program: `UOSA.exe` (tested file UOSA.exe (Enhanced Client)), SHA-256 `5a5e9dc536c836d1bf10b6f86bb88f61a6ae078f519d4060d3bc47aecbebad50`.

Mode: `--apply`, second pass (changes: {functions renamed=1, tags added=1, plate comments set=1}); a follow-up `--apply` corrected the shader-model wording of the `0x00c0d610` comment (changes: {plate comments refreshed=1}). A third `--apply` reported `changes: none`.

| Address | Name | Checks | Result |
|---|---|---|---|
| `0x00429950` | `UOLightingPostProcessEffect::BuildCompositeQuad` | calledby:0042a360, string:Tweaks | already applied |
| `0x0042a360` | `UOLightingPostProcessEffect::CreateCompositeResources` (probable) | calls:00429950, string:Data\\Textures\\00000020_daymap | already applied |
| `0x0058f7f0` | `UOLightingShader::SetupTextureStages` | ref:00cb3cf4 | already applied |
| `0x00460c70` | `UOTerrain::ComputeVertexNormal` | calledby:00461ae0 | already applied |
| `0x00c0d610` | `UOShaderLibrary::RegisterShaders` (probable) | string:UOSpriteShaderMob | named, tagged, comment set |
