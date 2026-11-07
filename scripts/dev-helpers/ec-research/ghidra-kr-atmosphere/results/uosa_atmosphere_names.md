# Atmosphere names

Program: `UOSA.exe` (tested file UOSA.exe (Enhanced Client)), SHA-256 `5a5e9dc536c836d1bf10b6f86bb88f61a6ae078f519d4060d3bc47aecbebad50`.

Mode: `--apply` (changes: {tag created=1, functions renamed=4, tags added=4, plate comments set=4}).

| Address | Name | Checks | Result |
|---|---|---|---|
| `0x00429950` | `UOLightingPostProcessEffect::BuildCompositeQuad` | calledby:0042a360, string:Tweaks | named, tagged, comment set |
| `0x0042a360` | `UOLightingPostProcessEffect::CreateCompositeResources` (probable) | calls:00429950, string:Data\\Textures\\00000020_daymap | named, tagged, comment set |
| `0x0058f7f0` | `UOLightingShader::SetupTextureStages` | ref:00cb3cf4 | named, tagged, comment set |
| `0x00460c70` | `UOTerrain::ComputeVertexNormal` | calledby:00461ae0 | named, tagged, comment set |
