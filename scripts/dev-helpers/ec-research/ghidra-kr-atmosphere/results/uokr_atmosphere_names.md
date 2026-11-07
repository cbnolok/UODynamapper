# Atmosphere names

Program: `UOKR.exe` (tested file UOKR.exe (Kingdom Reborn)), SHA-256 `1b802e5c00f3bf89a771778b9c6c57406be0703808eeb4cd983e2f5baa348a43`.

Mode: `--apply`, second pass (changes: {plate comments refreshed=1, functions renamed=25, tags added=25, plate comments set=25}); a follow-up `--apply` renamed `0x005477d0` to match the UOSA name (changes: {own earlier names corrected=1, plate comments refreshed=1}); its row is the follow-up's. A third `--apply` reported `changes: none`.

| Address | Name | Checks | Result |
|---|---|---|---|
| `0x0041ecd0` | `UOAtmosphere::InitPostProcessEffects` | calls:0041f190, calls:0043f740, calls:00c4a018, calls:00c4a024 | already applied |
| `0x0041eef0` | `UOAtmosphere::SetLightLevel` | calls:0041f2e0, calls:00c4a018 | already applied |
| `0x0041ef70` | `UOAtmosphere::SetNightSight` | calls:0041f370, calls:00c4a01c | already applied |
| `0x0041efe0` | `UOAtmosphere::SetGamma` | calls:0041f440, calls:00c4a028 | already applied |
| `0x0041f0c0` | `UOAtmosphere::SetShadowIntensity` | calls:0043f870, calls:00c4a024 | already applied |
| `0x0041f070` | `UOAtmosphere::SetShadowsEnabled` (probable) | calls:00c4a014 | already applied |
| `0x0056e8a0` | `UOAtmosphere::OnGlobalLightLevel` (probable) | calls:0041eef0, const:1f | already applied |
| `0x0056e940` | `UOAtmosphere::OnNightSight` (probable) | calls:0041f370 | already applied |
| `0x0047e790` | `UOSettings::ApplyResolutionSettings` (probable) | calls:0041efe0, calls:0041f070 | already applied |
| `0x0041f190` | `UOLightingPostProcessEffect::UOLightingPostProcessEffect` | calls:00b5126c | already applied |
| `0x0041f2b0` | `UOLightingPostProcessEffect::Initialize` (probable) | ref:00b51308 | already applied |
| `0x0041f4e0` | `UOLightingPostProcessEffect::GetInputRenderTargetGroup` (probable) | ref:00b51324 | already applied |
| `0x0041f4f0` | `UOLightingPostProcessEffect::GetInputTexture` (probable) | ref:00b51328 | already applied |
| `0x0041fe50` | `UOLightingPostProcessEffect::Render` | ref:00b5132c, calls:0041f5c0, calls:0041fab0 | comment refreshed |
| `0x0041fe10` | `UOLightingPostProcessEffect::CreateRenderTargets` (probable) | ref:00b51350 | already applied |
| `0x004205b0` | `UOLightingPostProcessEffect::DrawDebugOverlay` (probable) | ref:00b51358 | already applied |
| `0x0041f5c0` | `UOLightingPostProcessEffect::BuildCompositeQuad` | calledby:0041fe50, calls:00b86f94, calls:00b86fac | already applied |
| `0x0041fab0` | `UOLightingPostProcessEffect::BuildNightMapQuad` | calledby:0041fe50 | already applied |
| `0x0041f2e0` | `UOLightingPostProcessEffect::SetLightLevel` | calledby:0041eef0 | already applied |
| `0x0041f370` | `UOLightingPostProcessEffect::SetNightSight` | calledby:0041ef70 | already applied |
| `0x0041f440` | `UOLightingPostProcessEffect::SetGamma` | calledby:0041efe0 | already applied |
| `0x0043f740` | `UOShadowPostProcessEffect::UOShadowPostProcessEffect` | calls:00b525e4 | already applied |
| `0x0043f870` | `UOShadowPostProcessEffect::SetIntensity` | calledby:0041f0c0 | already applied |
| `0x0043fe10` | `UOShadowPostProcessEffect::Render` | calls:0043f9f0 | already applied |
| `0x0043f9f0` | `UOShadowPostProcessEffect::BuildCompositeQuad` | calledby:0043fe10 | already applied |
| `0x00412e20` | `UODeathPostProcessEffect::UODeathPostProcessEffect` | calls:00b5062c | already applied |
| `0x00543f60` | `UOLightingShader::UOLightingShader` | calls:00b60bec | already applied |
| `0x005441c0` | `UOLightingShader::SetupTextureStages` | ref:00b60c8c | already applied |
| `0x00544440` | `UONightMapShader::UONightMapShader` | calls:00b60c94 | already applied |
| `0x00544600` | `UONightMapShader::SetupTextureStages` | ref:00b60d34 | already applied |
| `0x00545e40` | `UOShadowShader::SetupTextureStages` | ref:00b60e5c | already applied |
| `0x00546c70` | `UOSpriteShaderShadow::UOSpriteShaderShadow` | calls:00b611bc | already applied |
| `0x00546e30` | `UOSpriteShaderShadow::SetupTextureStages` | ref:00b6125c | already applied |
| `0x00413d20` | `UODeathShader::SetupTextureStages` | ref:00b50804 | already applied |
| `0x0044b7f0` | `UOTerrain::BuildTerrainMesh` | calls:00c0f3b8, calls:004488a0 | already applied |
| `0x004488a0` | `UOTerrain::ComputeVertexNormal` | calledby:0044b7f0 | already applied |
| `0x00700b20` | `PostProcessEffectsManager::CreateEffectByType` (probable) | calls:007130a0, calls:0070f3d0 | already applied |
| `0x00459360` | `UOWorldViewer::UOWorldViewer` | calls:00b4f2e4 | named, tagged, comment set |
| `0x0045b510` | `UOWorldViewer::SetDefaultFrustum` (probable) | calls:00b4f2e4, calls:00b86ebc | named, tagged, comment set |
| `0x004454c0` | `UOSprite::BuildSpriteBatch` | calls:00440910, calls:004408a0, calledby:00438ee0 | named, tagged, comment set |
| `0x00442430` | `UOSprite::BuildSpriteQuad` | calledby:00442890 | named, tagged, comment set |
| `0x004422b0` | `UOSprite::PlaceSprite` (probable) | calledby:00438a80 | named, tagged, comment set |
| `0x00441ce0` | `UOSprite::SetSpriteHueColour` (probable) | calledby:00436320 | named, tagged, comment set |
| `0x00440910` | `UOSprite::GetSpriteAlphaProperty` | calledby:004454c0 | named, tagged, comment set |
| `0x004408a0` | `UOSprite::GetVertexColourProperty` | calledby:004454c0 | named, tagged, comment set |
| `0x00546fa0` | `UOSpriteShaderHLSL::LoadVertexShader` | string:UOSprite.vsh, calledby:005476c0 | named, tagged, comment set |
| `0x00547010` | `UOSpriteShaderHLSL::LoadPixelShader` | string:UOSprite.psh, calledby:005476c0 | named, tagged, comment set |
| `0x005476c0` | `UOSpriteShaderHLSL::Initialize` (probable) | ref:00b61274, calls:00547010, calls:00546fa0 | named, tagged, comment set |
| `0x005477d0` | `UOSpriteShaderHLSL::BindSamplers` | ref:00b61280 | renamed from `UOSpriteShaderHLSL::SelectPermutation`, comment refreshed |
| `0x005499e0` | `UODefaultTerrainLayer::Draw` (probable) | ref:00b61678 | named, tagged, comment set |
| `0x00549c30` | `UODefaultTerrainLayer::SetupFourStages` | ref:00b616b4 | named, tagged, comment set |
| `0x00549d90` | `UODefaultTerrainLayer::SetupTwoStages` | ref:00b616b8 | named, tagged, comment set |
| `0x0044b180` | `UOTerrain::LoadTerrainLayerTexture` (probable) | calledby:0044b530 | named, tagged, comment set |
| `0x00548d10` | `UOStaticTerrainShader::SetupTextureStages` | ref:00b61514 | named, tagged, comment set |
| `0x00438a80` | `UOObjectComponentLighting::PlaceLightSprite` | ref:00b5222c, calls:004422b0 | named, tagged, comment set |
| `0x00438ee0` | `UOObjectComponentLighting::BuildLightBatch` | calls:004454c0 | named, tagged, comment set |
| `0x00418420` | `UORainEffect::LoadParticles` (probable) | ref:00b50c00, string:00070006_Rain.nif | named, tagged, comment set |
| `0x00418cd0` | `UOSnowEffect::AttachToRenderGroup` (probable) | ref:00b617e8 | named, tagged, comment set |
| `0x00417de0` | `UOStencilEffect::Update` (probable) | ref:00b50bc8, calls:00416db0, calls:00417480 | named, tagged, comment set |
| `0x00416db0` | `UOStencilEffect::BuildFirstCircleShape` (probable) | string:Data\\Textures\\00000016_circle_of_trans | named, tagged, comment set |
| `0x00417480` | `UOStencilEffect::BuildSecondCircleShape` (probable) | string:Data\\Textures\\00000016_circle_of_trans | named, tagged, comment set |
| `0x0073025b` | `figleaf::ParseSkyDefinition` (probable) | calledby:00730fde | named, tagged, comment set |
