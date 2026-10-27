# Gamebryo 2.6 layouts

Program: UOSA.exe
SHA-256: 5a5e9dc536c836d1bf10b6f86bb88f61a6ae078f519d4060d3bc47aecbebad50
Mode: --apply
Sources (/gamebryo26): `NiDX9Renderer26VC80R.dll` (22240 data types), `NiMain26VC80R.dll` (17330 data types), `NiSystem26VC80R.dll` (9190 data types)

| Item | Count |
|---|---:|
| Class structures in `/Gamebryo` | 343 |
| Unfilled placeholders | 343 |
| Filled by an earlier run | 0 |
| Edited by the user (kept) | 0 |
| Other types in `/Gamebryo` | 1 |
| No 2.6 structure in the sources | 188 |
| Layout applied, exact size | 38 |
| Layout applied, inferred size | 9 |
| Size mismatches | 29 |
| No size evidence (no sized descendant) | 72 |
| Rejected by consistency checks | 7 |
| 2.6 types copied: structure | 20 |
| 2.6 types copied: stub (pointer target, no members) | 10 |
| 2.6 types copied: enum | 9 |
| 2.6 members: function pointers typed void * | 7 |

## Exact size

UOSA size from `CreateObject` equals the 2.6 size.

| Class | Size | 2.6 source | Field 0 |
|---|---:|---|---|
| `NiDefaultAVObjectPalette` | 0x20 | `NiMain26VC80R.dll` | applied; base `NiAVObjectPalette` |
| `NiAlphaAccumulator` | 0x34 | `NiMain26VC80R.dll` | applied; base `NiBackToFrontAccumulator` |
| `NiAdditionalGeometryData` | 0x28 | `NiMain26VC80R.dll` | applied; base `NiObject` |
| `NiExtraData` | 0xc | `NiMain26VC80R.dll` | applied; base `NiObject` |
| `NiBinaryExtraData` | 0x14 | `NiMain26VC80R.dll` | applied; base `NiExtraData` |
| `NiBooleanExtraData` | 0x10 | `NiMain26VC80R.dll` | applied; base `NiExtraData` |
| `NiColorExtraData` | 0x1c | `NiMain26VC80R.dll` | applied; base `NiExtraData` |
| `NiFloatExtraData` | 0x10 | `NiMain26VC80R.dll` | applied; base `NiExtraData` |
| `NiFloatsExtraData` | 0x14 | `NiMain26VC80R.dll` | applied; base `NiExtraData` |
| `NiIntegerExtraData` | 0x10 | `NiMain26VC80R.dll` | applied; base `NiExtraData` |
| `NiIntegersExtraData` | 0x14 | `NiMain26VC80R.dll` | applied; base `NiExtraData` |
| `NiStringExtraData` | 0x10 | `NiMain26VC80R.dll` | applied; base `NiExtraData` |
| `NiStringsExtraData` | 0x14 | `NiMain26VC80R.dll` | applied; base `NiExtraData` |
| `NiSwitchStringExtraData` | 0x18 | `NiMain26VC80R.dll` | applied; base `NiExtraData` |
| `NiVectorExtraData` | 0x1c | `NiMain26VC80R.dll` | applied; base `NiExtraData` |
| `NiVertWeightsExtraData` | 0xc | `NiMain26VC80R.dll` | applied; base `NiExtraData` |
| `NiRangeLODData` | 0x28 | `NiMain26VC80R.dll` | applied; base `NiLODData` |
| `NiScreenLODData` | 0x30 | `NiMain26VC80R.dll` | applied; base `NiLODData` |
| `NiAlphaProperty` | 0x1c | `NiMain26VC80R.dll` | applied; base `NiProperty` |
| `NiDitherProperty` | 0x1c | `NiMain26VC80R.dll` | applied; base `NiProperty` |
| `NiFogProperty` | 0x2c | `NiMain26VC80R.dll` | applied; base `NiProperty` |
| `NiMaterialProperty` | 0x5c | `NiMain26VC80R.dll` | applied; base `NiProperty` |
| `NiRendererSpecificProperty` | 0x18 | `NiMain26VC80R.dll` | applied; base `NiProperty` |
| `NiShadeProperty` | 0x1c | `NiMain26VC80R.dll` | applied; base `NiProperty` |
| `NiSpecularProperty` | 0x1c | `NiMain26VC80R.dll` | applied; base `NiProperty` |
| `NiStencilProperty` | 0x24 | `NiMain26VC80R.dll` | applied; base `NiProperty` |
| `NiVertexColorProperty` | 0x1c | `NiMain26VC80R.dll` | applied; base `NiProperty` |
| `NiWireframeProperty` | 0x1c | `NiMain26VC80R.dll` | applied; base `NiProperty` |
| `NiZBufferProperty` | 0x1c | `NiMain26VC80R.dll` | applied; base `NiProperty` |
| `NiSourceTexture` | 0x44 | `NiMain26VC80R.dll` | applied; base `NiTexture` |
| `NiSourceCubeMap` | 0x48 | `NiMain26VC80R.dll` | applied; base `NiSourceTexture` |
| `NiPalette` | 0x24 | `NiMain26VC80R.dll` | applied; base `NiObject` |
| `NiPersistentSrcTextureRendererData` | 0x8c | `NiMain26VC80R.dll` | applied; base `NiObject` |
| `NiPixelData` | 0x70 | `NiMain26VC80R.dll` | applied; base `NiObject` |
| `NiScreenPolygon` | 0x1c | `NiMain26VC80R.dll` | applied; base `NiObject` |
| `NiShadowGenerator` | 0x68 | `NiMain26VC80R.dll` | applied; base `NiObject` |
| `NiSkinData` | 0x48 | `NiMain26VC80R.dll` | applied; base `NiObject` |
| `NiSkinPartition` | 0x10 | `NiMain26VC80R.dll` | applied; base `NiObject` |

## Inferred size

No `CreateObject`; the 2.6 size lies within the recorded bounds (the upper bound is the smallest sized descendant), at least one sized descendant with a 2.6 type fits, and none of the fitting ones has 2.6 members inside this size.

| Class | UOSA bounds | 2.6 size | 2.6 source | Field 0 |
|---|---|---:|---|---|
| `NiObject` | >= 0x4, <= 0xc | 0x8 | `NiMain26VC80R.dll` | applied; vftable, NiRefObject members |
| `NiAVObjectPalette` | >= 0x4, <= 0x20 | 0x8 | `NiMain26VC80R.dll` | applied; base `NiObject` |
| `NiAccumulator` | >= 0x4, <= 0x34 | 0xc | `NiMain26VC80R.dll` | applied; base `NiObject` |
| `NiBackToFrontAccumulator` | >= 0x4, <= 0x34 | 0x30 | `NiMain26VC80R.dll` | applied; base `NiAccumulator` |
| `NiLODData` | >= 0x4, <= 0x28 | 0x8 | `NiMain26VC80R.dll` | applied; base `NiObject` |
| `NiObjectNET` | >= 0x4, <= 0x18 | 0x18 | `NiMain26VC80R.dll` | applied; base `NiObject` |
| `NiProperty` | >= 0x4, <= 0x18 | 0x18 | `NiMain26VC80R.dll` | applied; base `NiObjectNET` |
| `NiTexture` | >= 0x4, <= 0x44 | 0x30 | `NiMain26VC80R.dll` | applied; base `NiObjectNET` |
| `NiTimeController` | >= 0x4, <= 0x34 | 0x34 | `NiMain26VC80R.dll` | applied; base `NiObject` |

## Size mismatches

| Class | UOSA size or bounds | 2.6 size |
|---|---|---:|
| `NiLinesData` | 0x48 | 0x40 |
| `NiParticlesData` | 0x58 | 0x50 |
| `NiTriShapeData` | 0x5c | 0x54 |
| `NiScreenElementsData` | 0x74 | 0x6c |
| `NiScreenGeometryData` | 0x74 | 0x6c |
| `NiTriShapeDynamicData` | 0x5c | 0x54 |
| `NiTriStripsData` | 0x54 | 0x4c |
| `NiCamera` | 0x130 | 0x124 |
| `NiScreenSpaceCamera` | 0x150 | 0x144 |
| `NiAmbientLight` | 0x110 | 0x10c |
| `NiDirectionalLight` | 0x11c | 0x118 |
| `NiPointLight` | 0x11c | 0x118 |
| `NiSpotLight` | 0x13c | 0x138 |
| `NiTextureEffect` | 0x17c | 0x178 |
| `NiLines` | 0xe4 | 0xd4 |
| `NiParticles` | 0xe4 | 0xd4 |
| `NiTriShape` | 0xe4 | 0xd4 |
| `NiScreenElements` | 0xe4 | 0xd4 |
| `NiScreenGeometry` | 0xe4 | 0xd4 |
| `NiTriStrips` | 0xe4 | 0xd4 |
| `NiNode` | 0xe0 | 0xdc |
| `NiBSPNode` | 0x100 | 0xfc |
| `NiBillboardNode` | 0xe8 | 0xe0 |
| `NiSortAdjustNode` | 0xe4 | 0xe0 |
| `NiSwitchNode` | 0x100 | 0xe4 |
| `NiLODNode` | 0x108 | 0xec |
| `NiTexturingProperty` | 0x34 | 0x30 |
| `NiScreenTexture` | 0x20 | 0x18 |
| `NiSkinInstance` | 0x30 | 0x18 |

## No size evidence

No `CreateObject` and no sized descendant, so the bounds are only a lower bound; not applied.

| Class | UOSA bounds | 2.6 size |
|---|---|---:|
| `NiDX92DBufferData` | >= 0x4, no upper bound | 0x18 |
| `NiDX9DepthStencilBufferData` | >= 0x4, no upper bound | 0x18 |
| `NiDX9AdditionalDepthStencilBufferData` | >= 0x4, no upper bound | 0x1c |
| `NiDX9Direct3DDepthStencilBufferData` | >= 0x4, no upper bound | 0x18 |
| `NiDX9ImplicitDepthStencilBufferData` | >= 0x4, no upper bound | 0x18 |
| `NiDX9SwapChainDepthStencilBufferData` | >= 0x4, no upper bound | 0x1c |
| `NiDX9Direct3DBufferData` | >= 0x4, no upper bound | 0x18 |
| `NiDX9OnscreenBufferData` | >= 0x4, no upper bound | 0x50 |
| `NiDX9ImplicitBufferData` | >= 0x4, no upper bound | 0x54 |
| `NiDX9SwapChainBufferData` | >= 0x4, no upper bound | 0x54 |
| `NiDX9TextureBufferData` | >= 0x4, no upper bound | 0x1c |
| `NiGPUProgram` | >= 0x4, no upper bound | 0xc |
| `NiD3DShaderProgram` | >= 0x4, no upper bound | 0x2c |
| `NiD3DPixelShader` | >= 0x4, no upper bound | 0x30 |
| `NiD3DHLSLPixelShader` | >= 0x4, no upper bound | 0x3c |
| `NiD3DVertexShader` | >= 0x4, no upper bound | 0x38 |
| `NiD3DHLSLVertexShader` | >= 0x4, no upper bound | 0x44 |
| `NiGPUProgramCache` | >= 0x4, no upper bound | 0x4c |
| `NiMaterial` | >= 0x4, no upper bound | 0xc |
| `NiFragmentMaterial` | >= 0x4, no upper bound | 0x3b8 |
| `NiDirectionalShadowWriteMaterial` | >= 0x4, no upper bound | 0x3c4 |
| `NiPointShadowWriteMaterial` | >= 0x4, no upper bound | 0x3c4 |
| `NiSpotShadowWriteMaterial` | >= 0x4, no upper bound | 0x3c4 |
| `NiStandardMaterial` | >= 0x4, no upper bound | 0x3c0 |
| `NiVSMBlurMaterial` | >= 0x4, no upper bound | 0x3bc |
| `NiSingleShaderMaterial` | >= 0x4, no upper bound | 0x18 |
| `NiMaterialNode` | >= 0x4, no upper bound | 0x30 |
| `NiMaterialFragmentNode` | >= 0x4, no upper bound | 0x44 |
| `NiMaterialResourceConsumerNode` | >= 0x4, no upper bound | 0x34 |
| `NiMaterialResourceProducerNode` | >= 0x4, no upper bound | 0x34 |
| `Ni2DBuffer` | >= 0x4, no upper bound | 0x14 |
| `NiDepthStencilBuffer` | >= 0x4, no upper bound | 0x14 |
| `NiCollisionObject` | >= 0x4, no upper bound | 0xc |
| `NiSCMExtraData` | >= 0xc, no upper bound | 0x30 |
| `NiDX9Direct3DTexture` | >= 0x4, no upper bound | 0x38 |
| `NiRenderedTexture` | >= 0x4, no upper bound | 0x34 |
| `NiRenderedCubeMap` | >= 0x4, no upper bound | 0x50 |
| `NiDX9PersistentSrcTextureRendererData` | >= 0x8c, no upper bound | 0x8c |
| `NiRenderTargetGroup` | >= 0x4, no upper bound | 0x34 |
| `NiRenderer` | >= 0x4, no upper bound | 0xe0 |
| `NiDX9Renderer` | >= 0x4, no upper bound | 0x9d0 |
| `NiShaderDeclaration` | >= 0x4, no upper bound | 0x18 |
| `NiTask` | >= 0x4, no upper bound | 0xc |
| `NiParallelUpdateTaskManager::SignalTask` | >= 0x4, no upper bound | 0xc |
| `NiTaskManager` | >= 0x4, no upper bound | 0x6c |
| `NiParallelUpdateTaskManager` | >= 0x4, no upper bound | 0xa0 |
| `NiShaderTimeController` | >= 0x4, no upper bound | 0x34 |
| `NiRenderClick` | >= 0x4, no upper bound | 0x50 |
| `NiViewRenderClick` | >= 0x4, no upper bound | 0x84 |
| `NiShadowRenderClick` | >= 0x4, no upper bound | 0x90 |
| `NiRenderClickValidator` | >= 0x4, no upper bound | 0x8 |
| `NiShadowClickValidator` | >= 0x4, no upper bound | 0x20 |
| `NiRenderListProcessor` | >= 0x4, no upper bound | 0x8 |
| `NiMaterialSwapProcessor` | >= 0x4, no upper bound | 0x10 |
| `NiRenderView` | >= 0x4, no upper bound | 0x20 |
| `Ni3DRenderView` | >= 0x4, no upper bound | 0x3c |
| `NiScreenFillingRenderView` | >= 0x4, no upper bound | 0x38 |
| `NiShader` | >= 0x4, no upper bound | 0x3c |
| `NiD3DShaderInterface` | >= 0x4, no upper bound | 0x48 |
| `NiD3DShader` | >= 0x4, no upper bound | 0x74 |
| `NiD3DDefaultShader` | >= 0x4, no upper bound | 0x100 |
| `NiDX9ErrorShader` | >= 0x4, no upper bound | 0x78 |
| `NiDX9FragmentShader` | >= 0x4, no upper bound | 0x8c |
| `NiDX9ShadowWriteShader` | >= 0x4, no upper bound | 0x8c |
| `NiShader::NiShaderInstanceDescriptor` | >= 0x4, no upper bound | 0x4 |
| `NiFragmentShaderInstanceDescriptor` | >= 0x4, no upper bound | 0x18 |
| `NiShadowClickGenerator` | >= 0x4, no upper bound | 0x1c |
| `NiDefaultShadowClickGenerator` | >= 0x4, no upper bound | 0x60 |
| `NiShadowMap` | >= 0x4, no upper bound | 0x10 |
| `NiShadowCubeMap` | >= 0x4, no upper bound | 0x28 |
| `NiShadowTechnique` | >= 0x4, no upper bound | 0x50 |
| `NiVSMShadowTechnique` | >= 0x4, no upper bound | 0x70 |

## Rejected by consistency checks

| Class | UOSA size or bounds | 2.6 size | Reason |
|---|---|---:|---|
| `NiGeometryData` | >= 0x4, <= 0x48 | 0x3c | none of the 7 sized descendants with a 2.6 type fits |
| `NiTriBasedGeomData` | >= 0x4, <= 0x54 | 0x40 | none of the 5 sized descendants with a 2.6 type fits |
| `NiAVObject` | >= 0x4, <= 0xe0 | 0xac | none of the 19 sized descendants with a 2.6 type fits |
| `NiDynamicEffect` | >= 0x4, <= 0x110 | 0xe0 | none of the 5 sized descendants with a 2.6 type fits |
| `NiLight` | >= 0x4, <= 0x110 | 0x10c | none of the 4 sized descendants with a 2.6 type fits |
| `NiGeometry` | >= 0x4, <= 0xe4 | 0xd4 | none of the 6 sized descendants with a 2.6 type fits |
| `NiTriBasedGeom` | >= 0x4, <= 0xe4 | 0xd4 | none of the 4 sized descendants with a 2.6 type fits |

## No 2.6 structure in the sources

`ShaderOverrideSorter`, `UOShaderOverrideSorter`, `NiBSplineBasisData`, `NiBSplineData`, `NiBoolData`, `NiCollisionData`, `NiColorData`, `NiControllerSequence`, `MythicShadowGenerator`, `NiTextKeyExtraData`, `NiFloatData`, `NiPSysData`, `NiMeshPSysData`, `NiInterpolator`, `NiBSplineInterpolator`, `NiBSplineColorInterpolator`, `NiBSplineCompColorInterpolator`, `NiBSplineFloatInterpolator`, `NiBSplineCompFloatInterpolator`, `NiBSplinePoint3Interpolator`, `NiBSplineCompPoint3Interpolator`, `NiBSplineTransformInterpolator`, `NiBSplineCompTransformInterpolator`, `NiBlendInterpolator`, `NiBlendAccumTransformInterpolator`, `NiBlendBoolInterpolator`, `NiBlendColorInterpolator`, `NiBlendFloatInterpolator`, `NiBlendPoint3Interpolator`, `NiBlendQuaternionInterpolator`, `NiBlendTransformInterpolator`, `NiKeyBasedInterpolator`, `NiBoolInterpolator`, `NiBoolTimelineInterpolator`, `NiColorInterpolator`, `NiFloatInterpolator`, `NiPathInterpolator`, `NiPoint3Interpolator`, `NiQuaternionInterpolator`, `NiTransformInterpolator`, `NiLookAtInterpolator`, `NiMorphData`, `MythicAmbientLight`, `MythicDirectionalLight`, `MythicPointLight`, `NiParticleSystem`, `NiMeshParticleSystem`, `CachingTriShape`, `FoliageTriShape`, `MobAnimTriShape`, `MobAnimTriShapeShadow`, `ParticleTriShape`, `SpriteTriShape`, `TerrainTriShape`, `UOCollisionNode`, `UOObjectCollisionNode`, `UOStencilCollisionNode`, `UOTerrainCollisionNode`, `WeaponTrailTriShape`, `Effect`, `DynamicEffect`, `WeaponTrailEffect`, `EmitterEffect`, `GenericShaderTexturingProperty`, `UOTerrainTexturingProperty`, `MobAnimTexture`, `NiPSysCollider`, `NiPSysPlanarCollider`, `NiPSysSphericalCollider`, `NiPSysEmitterCtlrData`, `NiPSysModifier`, `NiPSysAgeDeathModifier`, `NiPSysBombModifier`, `NiPSysBoundUpdateModifier`, `NiPSysColliderManager`, `NiPSysColorModifier`, `NiPSysDragModifier`, `NiPSysEmitter`, `NiPSysMeshEmitter`, `NiPSysVolumeEmitter`, `NiPSysBoxEmitter`, `NiPSysCylinderEmitter`, `NiPSysSphereEmitter`, `NiPSysFieldModifier`, `NiPSysAirFieldModifier`, `NiPSysDragFieldModifier`, `NiPSysGravityFieldModifier`, `NiPSysRadialFieldModifier`, `NiPSysTurbulenceFieldModifier`, `NiPSysVortexFieldModifier`, `NiPSysGravityModifier`, `NiPSysGrowFadeModifier`, `NiPSysMeshUpdateModifier`, `NiPSysPositionModifier`, `NiPSysRotationModifier`, `NiPSysSpawnModifier`, `NiPosData`, `NiRotData`, `NiStringPalette`, `NiGeomMorpherUpdateTask`, `NiPSysUpdateTask`, `NiBoneLODController`, `NiControllerManager`, `NiFloatController`, `NiRollController`, `NiInterpController`, `NiGeomMorpherController`, `NiMultiTargetTransformController`, `NiSingleInterpController`, `NiBoolInterpController`, `NiVisController`, `NiExtraDataController`, `NiColorExtraDataController`, `NiFloatExtraDataController`, `NiFloatsExtraDataController`, `NiFloatsExtraDataPoint3Controller`, `NiFloatInterpController`, `NiAlphaController`, `NiFlipController`, `NiLightDimmerController`, `NiTextureTransformController`, `NiPSysModifierCtlr`, `NiPSysEmitterCtlr`, `NiPSysModifierBoolCtlr`, `NiPSysModifierActiveCtlr`, `NiPSysModifierFloatCtlr`, `NiPSysAirFieldAirFrictionCtlr`, `NiPSysAirFieldInheritVelocityCtlr`, `NiPSysAirFieldSpreadCtlr`, `NiPSysEmitterDeclinationCtlr`, `NiPSysEmitterDeclinationVarCtlr`, `NiPSysEmitterInitialRadiusCtlr`, `NiPSysEmitterLifeSpanCtlr`, `NiPSysEmitterPlanarAngleCtlr`, `NiPSysEmitterPlanarAngleVarCtlr`, `NiPSysEmitterSpeedCtlr`, `NiPSysFieldAttenuationCtlr`, `NiPSysFieldMagnitudeCtlr`, `NiPSysFieldMaxDistanceCtlr`, `NiPSysGravityStrengthCtlr`, `NiPSysInitialRotAngleCtlr`, `NiPSysInitialRotAngleVarCtlr`, `NiPSysInitialRotSpeedCtlr`, `NiPSysInitialRotSpeedVarCtlr`, `NiPoint3InterpController`, `NiLightColorController`, `NiMaterialColorController`, `NiTransformController`, `NiLookAtController`, `NiPSysResetOnLoopCtlr`, `NiPSysUpdateCtlr`, `NiPathController`, `NiTransformData`, `MythicBaseShader`, `ColorShader`, `DepthOnlyShader`, `LightingShader`, `AtlasTerrainShader`, `GameTerrainShader`, `GenericShader`, `WarhammerCharacterShader`, `WarhammerFixtureBlendShader`, `WarhammerFixtureShader`, `NewWaterShader`, `ShadowMapShader`, `SimpleFFP`, `UIDecalShader`, `UiShader`, `VCAlphaTextureBlendShader`, `UOBaseShader`, `UODeathShaderFFP`, `UODeathShaderHLSL`, `UOEffectsShaderFFP`, `UOEffectsShaderHLSL`, `UOFadeShader`, `UOLightingShader`, `UONightMapShader`, `UOShadowShader`, `UOSpriteShaderDepthOnly`, `UOSpriteShaderFFP`, `UOSpriteShaderMob`, `UOSpriteShaderHLSL`, `UOSpriteShaderShadow`, `UOSpriteUIShaderFFP`, `UOSpriteUIShaderHLSL`, `UOStaticTerrainShader`, `UOTerrainShader`, `UOWaterShader`

## Edited by the user (kept)

None.

## Other types in /Gamebryo

`NiRTTI`

## Anomalies

None.

## Applied changes

- class structures filled: 47
- 2.6 types copied to `/Gamebryo26`: 39
