# Atmosphere names

Program: `UOKR.exe` (tested file UOKR.exe (Kingdom Reborn)), SHA-256 `1b802e5c00f3bf89a771778b9c6c57406be0703808eeb4cd983e2f5baa348a43`.

Mode: `--apply` (changes: {tag created=1, functions renamed=37, tags added=37, plate comments set=37}).

| Address | Name | Checks | Result |
|---|---|---|---|
| `0x0041ecd0` | `UOAtmosphere::InitPostProcessEffects` | calls:0041f190, calls:0043f740, calls:00c4a018, calls:00c4a024 | named, tagged, comment set |
| `0x0041eef0` | `UOAtmosphere::SetLightLevel` | calls:0041f2e0, calls:00c4a018 | named, tagged, comment set |
| `0x0041ef70` | `UOAtmosphere::SetNightSight` | calls:0041f370, calls:00c4a01c | named, tagged, comment set |
| `0x0041efe0` | `UOAtmosphere::SetGamma` | calls:0041f440, calls:00c4a028 | named, tagged, comment set |
| `0x0041f0c0` | `UOAtmosphere::SetShadowIntensity` | calls:0043f870, calls:00c4a024 | named, tagged, comment set |
| `0x0041f070` | `UOAtmosphere::SetShadowsEnabled` (probable) | calls:00c4a014 | named, tagged, comment set |
| `0x0056e8a0` | `UOAtmosphere::OnGlobalLightLevel` (probable) | calls:0041eef0, const:1f | named, tagged, comment set |
| `0x0056e940` | `UOAtmosphere::OnNightSight` (probable) | calls:0041f370 | named, tagged, comment set |
| `0x0047e790` | `UOSettings::ApplyResolutionSettings` (probable) | calls:0041efe0, calls:0041f070 | named, tagged, comment set |
| `0x0041f190` | `UOLightingPostProcessEffect::UOLightingPostProcessEffect` | calls:00b5126c | named, tagged, comment set |
| `0x0041f2b0` | `UOLightingPostProcessEffect::Initialize` (probable) | ref:00b51308 | named, tagged, comment set |
| `0x0041f4e0` | `UOLightingPostProcessEffect::GetInputRenderTargetGroup` (probable) | ref:00b51324 | named, tagged, comment set |
| `0x0041f4f0` | `UOLightingPostProcessEffect::GetInputTexture` (probable) | ref:00b51328 | named, tagged, comment set |
| `0x0041fe50` | `UOLightingPostProcessEffect::Render` | ref:00b5132c, calls:0041f5c0, calls:0041fab0 | named, tagged, comment set |
| `0x0041fe10` | `UOLightingPostProcessEffect::CreateRenderTargets` (probable) | ref:00b51350 | named, tagged, comment set |
| `0x004205b0` | `UOLightingPostProcessEffect::DrawDebugOverlay` (probable) | ref:00b51358 | named, tagged, comment set |
| `0x0041f5c0` | `UOLightingPostProcessEffect::BuildCompositeQuad` | calledby:0041fe50, calls:00b86f94, calls:00b86fac | named, tagged, comment set |
| `0x0041fab0` | `UOLightingPostProcessEffect::BuildNightMapQuad` | calledby:0041fe50 | named, tagged, comment set |
| `0x0041f2e0` | `UOLightingPostProcessEffect::SetLightLevel` | calledby:0041eef0 | named, tagged, comment set |
| `0x0041f370` | `UOLightingPostProcessEffect::SetNightSight` | calledby:0041ef70 | named, tagged, comment set |
| `0x0041f440` | `UOLightingPostProcessEffect::SetGamma` | calledby:0041efe0 | named, tagged, comment set |
| `0x0043f740` | `UOShadowPostProcessEffect::UOShadowPostProcessEffect` | calls:00b525e4 | named, tagged, comment set |
| `0x0043f870` | `UOShadowPostProcessEffect::SetIntensity` | calledby:0041f0c0 | named, tagged, comment set |
| `0x0043fe10` | `UOShadowPostProcessEffect::Render` | calls:0043f9f0 | named, tagged, comment set |
| `0x0043f9f0` | `UOShadowPostProcessEffect::BuildCompositeQuad` | calledby:0043fe10 | named, tagged, comment set |
| `0x00412e20` | `UODeathPostProcessEffect::UODeathPostProcessEffect` | calls:00b5062c | named, tagged, comment set |
| `0x00543f60` | `UOLightingShader::UOLightingShader` | calls:00b60bec | named, tagged, comment set |
| `0x005441c0` | `UOLightingShader::SetupTextureStages` | ref:00b60c8c | named, tagged, comment set |
| `0x00544440` | `UONightMapShader::UONightMapShader` | calls:00b60c94 | named, tagged, comment set |
| `0x00544600` | `UONightMapShader::SetupTextureStages` | ref:00b60d34 | named, tagged, comment set |
| `0x00545e40` | `UOShadowShader::SetupTextureStages` | ref:00b60e5c | named, tagged, comment set |
| `0x00546c70` | `UOSpriteShaderShadow::UOSpriteShaderShadow` | calls:00b611bc | named, tagged, comment set |
| `0x00546e30` | `UOSpriteShaderShadow::SetupTextureStages` | ref:00b6125c | named, tagged, comment set |
| `0x00413d20` | `UODeathShader::SetupTextureStages` | ref:00b50804 | named, tagged, comment set |
| `0x0044b7f0` | `UOTerrain::BuildTerrainMesh` | calls:00c0f3b8, calls:004488a0 | named, tagged, comment set |
| `0x004488a0` | `UOTerrain::ComputeVertexNormal` | calledby:0044b7f0 | named, tagged, comment set |
| `0x00700b20` | `PostProcessEffectsManager::CreateEffectByType` (probable) | calls:007130a0, calls:0070f3d0 | named, tagged, comment set |
