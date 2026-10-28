# Gamebryo 2.6 layouts

Program: UOSA.exe
SHA-256: 5a5e9dc536c836d1bf10b6f86bb88f61a6ae078f519d4060d3bc47aecbebad50
Mode: --apply
Sources (/gamebryo26): `NiAnimation26VC80R.dll` (22363 data types), `NiCollision26VC80R.dll` (18420 data types), `NiDX9Renderer26VC80R.dll` (22240 data types), `NiMain26VC80R.dll` (17330 data types), `NiParticle26VC80R.dll` (25084 data types), `NiSystem26VC80R.dll` (9190 data types)

| Item | Count |
|---|---:|
| Class structures in `/Gamebryo` | 343 |
| Unfilled placeholders | 296 |
| Filled by an earlier run | 47 |
| Edited by the user (kept) | 0 |
| Other types in `/Gamebryo` | 1 |
| No 2.6 structure in the sources | 67 |
| Layout applied, exact size | 118 |
| Layout applied, inferred size | 25 |
| Size mismatches | 50 |
| No size evidence (no sized descendant) | 74 |
| Rejected by consistency checks | 9 |
| Earlier inferred layouts re-checked | 9 |
| Earlier inferred layouts contradicted | 0 |
| 2.6 types copied: structure | 26 |
| 2.6 types copied: stub (pointer target, no members) | 13 |
| 2.6 types copied: enum | 8 |
| 2.6 members: existing types reused | 9 |

## Exact size

UOSA size from `CreateObject` equals the 2.6 size.

| Class | Size | 2.6 source | Field 0 |
|---|---:|---|---|
| `NiDefaultAVObjectPalette` | 0x20 | `NiMain26VC80R.dll` | filled earlier |
| `NiAlphaAccumulator` | 0x34 | `NiMain26VC80R.dll` | filled earlier |
| `NiAdditionalGeometryData` | 0x28 | `NiMain26VC80R.dll` | filled earlier |
| `NiBSplineBasisData` | 0x28 | `NiAnimation26VC80R.dll` | applied; base `NiObject` |
| `NiBSplineData` | 0x18 | `NiAnimation26VC80R.dll` | applied; base `NiObject` |
| `NiBoolData` | 0x18 | `NiAnimation26VC80R.dll` | applied; base `NiObject` |
| `NiColorData` | 0x18 | `NiAnimation26VC80R.dll` | applied; base `NiObject` |
| `NiControllerSequence` | 0x7c | `NiAnimation26VC80R.dll` | applied; base `NiObject` |
| `NiExtraData` | 0xc | `NiMain26VC80R.dll` | filled earlier |
| `NiBinaryExtraData` | 0x14 | `NiMain26VC80R.dll` | filled earlier |
| `NiBooleanExtraData` | 0x10 | `NiMain26VC80R.dll` | filled earlier |
| `NiColorExtraData` | 0x1c | `NiMain26VC80R.dll` | filled earlier |
| `NiFloatExtraData` | 0x10 | `NiMain26VC80R.dll` | filled earlier |
| `NiFloatsExtraData` | 0x14 | `NiMain26VC80R.dll` | filled earlier |
| `NiIntegerExtraData` | 0x10 | `NiMain26VC80R.dll` | filled earlier |
| `NiIntegersExtraData` | 0x14 | `NiMain26VC80R.dll` | filled earlier |
| `NiStringExtraData` | 0x10 | `NiMain26VC80R.dll` | filled earlier |
| `NiStringsExtraData` | 0x14 | `NiMain26VC80R.dll` | filled earlier |
| `NiSwitchStringExtraData` | 0x18 | `NiMain26VC80R.dll` | filled earlier |
| `NiTextKeyExtraData` | 0x14 | `NiAnimation26VC80R.dll` | applied; base `NiExtraData` |
| `NiVectorExtraData` | 0x1c | `NiMain26VC80R.dll` | filled earlier |
| `NiVertWeightsExtraData` | 0xc | `NiMain26VC80R.dll` | filled earlier |
| `NiFloatData` | 0x18 | `NiAnimation26VC80R.dll` | applied; base `NiObject` |
| `NiBSplineFloatInterpolator` | 0x24 | `NiAnimation26VC80R.dll` | applied; base `NiBSplineInterpolator` |
| `NiBSplineCompFloatInterpolator` | 0x2c | `NiAnimation26VC80R.dll` | applied; base `NiBSplineFloatInterpolator` |
| `NiBSplinePoint3Interpolator` | 0x2c | `NiAnimation26VC80R.dll` | applied; base `NiBSplineInterpolator` |
| `NiBSplineCompPoint3Interpolator` | 0x34 | `NiAnimation26VC80R.dll` | applied; base `NiBSplinePoint3Interpolator` |
| `NiBSplineTransformInterpolator` | 0x48 | `NiAnimation26VC80R.dll` | applied; base `NiBSplineInterpolator` |
| `NiBSplineCompTransformInterpolator` | 0x60 | `NiAnimation26VC80R.dll` | applied; base `NiBSplineTransformInterpolator` |
| `NiBlendBoolInterpolator` | 0x34 | `NiAnimation26VC80R.dll` | applied; base `NiBlendInterpolator` |
| `NiBlendColorInterpolator` | 0x44 | `NiAnimation26VC80R.dll` | applied; base `NiBlendInterpolator` |
| `NiBlendFloatInterpolator` | 0x34 | `NiAnimation26VC80R.dll` | applied; base `NiBlendInterpolator` |
| `NiBlendPoint3Interpolator` | 0x40 | `NiAnimation26VC80R.dll` | applied; base `NiBlendInterpolator` |
| `NiBlendQuaternionInterpolator` | 0x40 | `NiAnimation26VC80R.dll` | applied; base `NiBlendInterpolator` |
| `NiBlendTransformInterpolator` | 0x30 | `NiAnimation26VC80R.dll` | applied; base `NiBlendInterpolator` |
| `NiBoolInterpolator` | 0x18 | `NiAnimation26VC80R.dll` | applied; base `NiKeyBasedInterpolator` |
| `NiBoolTimelineInterpolator` | 0x20 | `NiAnimation26VC80R.dll` | applied; base `NiBoolInterpolator` |
| `NiColorInterpolator` | 0x24 | `NiAnimation26VC80R.dll` | applied; base `NiKeyBasedInterpolator` |
| `NiFloatInterpolator` | 0x18 | `NiAnimation26VC80R.dll` | applied; base `NiKeyBasedInterpolator` |
| `NiPathInterpolator` | 0x5c | `NiAnimation26VC80R.dll` | applied; base `NiKeyBasedInterpolator` |
| `NiPoint3Interpolator` | 0x20 | `NiAnimation26VC80R.dll` | applied; base `NiKeyBasedInterpolator` |
| `NiQuaternionInterpolator` | 0x24 | `NiAnimation26VC80R.dll` | applied; base `NiKeyBasedInterpolator` |
| `NiTransformInterpolator` | 0x3c | `NiAnimation26VC80R.dll` | applied; base `NiKeyBasedInterpolator` |
| `NiLookAtInterpolator` | 0x44 | `NiAnimation26VC80R.dll` | applied; base `NiInterpolator` |
| `NiRangeLODData` | 0x28 | `NiMain26VC80R.dll` | filled earlier |
| `NiScreenLODData` | 0x30 | `NiMain26VC80R.dll` | filled earlier |
| `NiMorphData` | 0x18 | `NiAnimation26VC80R.dll` | applied; base `NiObject` |
| `NiAlphaProperty` | 0x1c | `NiMain26VC80R.dll` | filled earlier |
| `NiDitherProperty` | 0x1c | `NiMain26VC80R.dll` | filled earlier |
| `NiFogProperty` | 0x2c | `NiMain26VC80R.dll` | filled earlier |
| `NiMaterialProperty` | 0x5c | `NiMain26VC80R.dll` | filled earlier |
| `NiRendererSpecificProperty` | 0x18 | `NiMain26VC80R.dll` | filled earlier |
| `NiShadeProperty` | 0x1c | `NiMain26VC80R.dll` | filled earlier |
| `NiSpecularProperty` | 0x1c | `NiMain26VC80R.dll` | filled earlier |
| `NiStencilProperty` | 0x24 | `NiMain26VC80R.dll` | filled earlier |
| `NiVertexColorProperty` | 0x1c | `NiMain26VC80R.dll` | filled earlier |
| `NiWireframeProperty` | 0x1c | `NiMain26VC80R.dll` | filled earlier |
| `NiZBufferProperty` | 0x1c | `NiMain26VC80R.dll` | filled earlier |
| `NiSourceTexture` | 0x44 | `NiMain26VC80R.dll` | filled earlier |
| `NiSourceCubeMap` | 0x48 | `NiMain26VC80R.dll` | filled earlier |
| `NiPSysEmitterCtlrData` | 0x28 | `NiParticle26VC80R.dll` | applied; base `NiObject` |
| `NiPSysAgeDeathModifier` | 0x20 | `NiParticle26VC80R.dll` | applied; base `NiPSysModifier` |
| `NiPSysBombModifier` | 0x38 | `NiParticle26VC80R.dll` | applied; base `NiPSysModifier` |
| `NiPSysColliderManager` | 0x1c | `NiParticle26VC80R.dll` | applied; base `NiPSysModifier` |
| `NiPSysDragModifier` | 0x34 | `NiParticle26VC80R.dll` | applied; base `NiPSysModifier` |
| `NiPSysBoxEmitter` | 0x60 | `NiParticle26VC80R.dll` | applied; base `NiPSysVolumeEmitter` |
| `NiPSysCylinderEmitter` | 0x5c | `NiParticle26VC80R.dll` | applied; base `NiPSysVolumeEmitter` |
| `NiPSysSphereEmitter` | 0x58 | `NiParticle26VC80R.dll` | applied; base `NiPSysVolumeEmitter` |
| `NiPSysGravityModifier` | 0x3c | `NiParticle26VC80R.dll` | applied; base `NiPSysModifier` |
| `NiPSysGrowFadeModifier` | 0x28 | `NiParticle26VC80R.dll` | applied; base `NiPSysModifier` |
| `NiPSysMeshUpdateModifier` | 0x28 | `NiParticle26VC80R.dll` | applied; base `NiPSysModifier` |
| `NiPSysPositionModifier` | 0x18 | `NiParticle26VC80R.dll` | applied; base `NiPSysModifier` |
| `NiPSysRotationModifier` | 0x38 | `NiParticle26VC80R.dll` | applied; base `NiPSysModifier` |
| `NiPSysSpawnModifier` | 0x34 | `NiParticle26VC80R.dll` | applied; base `NiPSysModifier` |
| `NiPalette` | 0x24 | `NiMain26VC80R.dll` | filled earlier |
| `NiPersistentSrcTextureRendererData` | 0x8c | `NiMain26VC80R.dll` | filled earlier |
| `NiPixelData` | 0x70 | `NiMain26VC80R.dll` | filled earlier |
| `NiPosData` | 0x18 | `NiAnimation26VC80R.dll` | applied; base `NiObject` |
| `NiRotData` | 0x18 | `NiAnimation26VC80R.dll` | applied; base `NiObject` |
| `NiScreenPolygon` | 0x1c | `NiMain26VC80R.dll` | filled earlier |
| `NiShadowGenerator` | 0x68 | `NiMain26VC80R.dll` | filled earlier |
| `NiSkinData` | 0x48 | `NiMain26VC80R.dll` | filled earlier |
| `NiSkinPartition` | 0x10 | `NiMain26VC80R.dll` | filled earlier |
| `NiStringPalette` | 0x14 | `NiAnimation26VC80R.dll` | applied; base `NiObject` |
| `NiBoneLODController` | 0x68 | `NiAnimation26VC80R.dll` | applied; base `NiTimeController` |
| `NiGeomMorpherController` | 0x44 | `NiAnimation26VC80R.dll` | applied; base `NiInterpController` |
| `NiVisController` | 0x38 | `NiAnimation26VC80R.dll` | applied; base `NiBoolInterpController` |
| `NiColorExtraDataController` | 0x40 | `NiAnimation26VC80R.dll` | applied; base `NiExtraDataController` |
| `NiFloatExtraDataController` | 0x40 | `NiAnimation26VC80R.dll` | applied; base `NiExtraDataController` |
| `NiFloatsExtraDataController` | 0x48 | `NiAnimation26VC80R.dll` | applied; base `NiExtraDataController` |
| `NiFloatsExtraDataPoint3Controller` | 0x48 | `NiAnimation26VC80R.dll` | applied; base `NiExtraDataController` |
| `NiFlipController` | 0x54 | `NiAnimation26VC80R.dll` | applied; base `NiFloatInterpController` |
| `NiLightDimmerController` | 0x38 | `NiAnimation26VC80R.dll` | applied; base `NiFloatInterpController` |
| `NiTextureTransformController` | 0x50 | `NiAnimation26VC80R.dll` | applied; base `NiFloatInterpController` |
| `NiPSysModifierActiveCtlr` | 0x40 | `NiParticle26VC80R.dll` | applied; base `NiPSysModifierBoolCtlr` |
| `NiPSysAirFieldAirFrictionCtlr` | 0x40 | `NiParticle26VC80R.dll` | applied; base `NiPSysModifierFloatCtlr` |
| `NiPSysAirFieldInheritVelocityCtlr` | 0x40 | `NiParticle26VC80R.dll` | applied; base `NiPSysModifierFloatCtlr` |
| `NiPSysAirFieldSpreadCtlr` | 0x40 | `NiParticle26VC80R.dll` | applied; base `NiPSysModifierFloatCtlr` |
| `NiPSysEmitterDeclinationCtlr` | 0x40 | `NiParticle26VC80R.dll` | applied; base `NiPSysModifierFloatCtlr` |
| `NiPSysEmitterDeclinationVarCtlr` | 0x40 | `NiParticle26VC80R.dll` | applied; base `NiPSysModifierFloatCtlr` |
| `NiPSysEmitterInitialRadiusCtlr` | 0x40 | `NiParticle26VC80R.dll` | applied; base `NiPSysModifierFloatCtlr` |
| `NiPSysEmitterLifeSpanCtlr` | 0x40 | `NiParticle26VC80R.dll` | applied; base `NiPSysModifierFloatCtlr` |
| `NiPSysEmitterPlanarAngleCtlr` | 0x40 | `NiParticle26VC80R.dll` | applied; base `NiPSysModifierFloatCtlr` |
| `NiPSysEmitterPlanarAngleVarCtlr` | 0x40 | `NiParticle26VC80R.dll` | applied; base `NiPSysModifierFloatCtlr` |
| `NiPSysEmitterSpeedCtlr` | 0x40 | `NiParticle26VC80R.dll` | applied; base `NiPSysModifierFloatCtlr` |
| `NiPSysFieldAttenuationCtlr` | 0x40 | `NiParticle26VC80R.dll` | applied; base `NiPSysModifierFloatCtlr` |
| `NiPSysFieldMagnitudeCtlr` | 0x40 | `NiParticle26VC80R.dll` | applied; base `NiPSysModifierFloatCtlr` |
| `NiPSysFieldMaxDistanceCtlr` | 0x40 | `NiParticle26VC80R.dll` | applied; base `NiPSysModifierFloatCtlr` |
| `NiPSysGravityStrengthCtlr` | 0x40 | `NiParticle26VC80R.dll` | applied; base `NiPSysModifierFloatCtlr` |
| `NiPSysInitialRotAngleCtlr` | 0x40 | `NiParticle26VC80R.dll` | applied; base `NiPSysModifierFloatCtlr` |
| `NiPSysInitialRotAngleVarCtlr` | 0x40 | `NiParticle26VC80R.dll` | applied; base `NiPSysModifierFloatCtlr` |
| `NiPSysInitialRotSpeedCtlr` | 0x40 | `NiParticle26VC80R.dll` | applied; base `NiPSysModifierFloatCtlr` |
| `NiPSysInitialRotSpeedVarCtlr` | 0x40 | `NiParticle26VC80R.dll` | applied; base `NiPSysModifierFloatCtlr` |
| `NiLightColorController` | 0x3c | `NiAnimation26VC80R.dll` | applied; base `NiPoint3InterpController` |
| `NiMaterialColorController` | 0x3c | `NiAnimation26VC80R.dll` | applied; base `NiPoint3InterpController` |
| `NiTransformController` | 0x38 | `NiAnimation26VC80R.dll` | applied; base `NiSingleInterpController` |
| `NiPSysUpdateCtlr` | 0x34 | `NiParticle26VC80R.dll` | applied; base `NiTimeController` |
| `NiTransformData` | 0x38 | `NiAnimation26VC80R.dll` | applied; base `NiObject` |

## Inferred size

No `CreateObject`; the 2.6 size lies within the recorded bounds (the upper bound is the smallest sized descendant), at least one sized descendant with a 2.6 type fits, and none of the fitting ones has 2.6 members inside this size.

| Class | UOSA bounds | 2.6 size | Sized descendants | 2.6 source | Field 0 |
|---|---|---:|---|---|---|
| `NiObject` | >= 0x4, <= 0xc | 0x8 | 118 of 167 fit, 0 of this size | `NiMain26VC80R.dll` | filled earlier |
| `NiAVObjectPalette` | >= 0x4, <= 0x20 | 0x8 | 1 of 1 fit, 0 of this size | `NiMain26VC80R.dll` | filled earlier |
| `NiAccumulator` | >= 0x4, <= 0x34 | 0xc | 1 of 1 fit, 0 of this size | `NiMain26VC80R.dll` | filled earlier |
| `NiBackToFrontAccumulator` | >= 0x4, <= 0x34 | 0x30 | 1 of 1 fit, 0 of this size | `NiMain26VC80R.dll` | filled earlier |
| `NiInterpolator` | >= 0x4, <= 0x18 | 0xc | 21 of 23 fit, 0 of this size | `NiAnimation26VC80R.dll` | applied; base `NiObject` |
| `NiBSplineInterpolator` | >= 0x4, <= 0x24 | 0x1c | 6 of 8 fit, 0 of this size | `NiAnimation26VC80R.dll` | applied; base `NiInterpolator` |
| `NiBlendInterpolator` | >= 0x4, <= 0x30 | 0x30 | 6 of 6 fit, 1 of this size | `NiAnimation26VC80R.dll` | applied; base `NiInterpolator` |
| `NiKeyBasedInterpolator` | >= 0x4, <= 0x18 | 0xc | 8 of 8 fit, 0 of this size | `NiAnimation26VC80R.dll` | applied; base `NiInterpolator` |
| `NiLODData` | >= 0x4, <= 0x28 | 0x8 | 2 of 2 fit, 0 of this size | `NiMain26VC80R.dll` | filled earlier |
| `NiObjectNET` | >= 0x4, <= 0x18 | 0x18 | 13 of 34 fit, 1 of this size | `NiMain26VC80R.dll` | filled earlier |
| `NiProperty` | >= 0x4, <= 0x18 | 0x18 | 11 of 12 fit, 1 of this size | `NiMain26VC80R.dll` | filled earlier |
| `NiTexture` | >= 0x4, <= 0x44 | 0x30 | 2 of 2 fit, 0 of this size | `NiMain26VC80R.dll` | filled earlier |
| `NiPSysModifier` | >= 0x4, <= 0x18 | 0x18 | 13 of 22 fit, 1 of this size | `NiParticle26VC80R.dll` | applied; base `NiObject` |
| `NiPSysEmitter` | >= 0x4, <= 0x58 | 0x50 | 3 of 4 fit, 0 of this size | `NiParticle26VC80R.dll` | applied; base `NiPSysModifier` |
| `NiPSysVolumeEmitter` | >= 0x4, <= 0x58 | 0x54 | 3 of 3 fit, 0 of this size | `NiParticle26VC80R.dll` | applied; base `NiPSysEmitter` |
| `NiTimeController` | >= 0x4, <= 0x34 | 0x34 | 33 of 37 fit, 1 of this size | `NiMain26VC80R.dll` | filled earlier |
| `NiInterpController` | >= 0x4, <= 0x38 | 0x34 | 31 of 33 fit, 0 of this size | `NiAnimation26VC80R.dll` | applied; base `NiTimeController` |
| `NiSingleInterpController` | >= 0x4, <= 0x38 | 0x38 | 30 of 31 fit, 3 of this size | `NiAnimation26VC80R.dll` | applied; base `NiInterpController` |
| `NiBoolInterpController` | >= 0x4, <= 0x38 | 0x38 | 1 of 1 fit, 1 of this size | `NiAnimation26VC80R.dll` | applied; base `NiSingleInterpController` |
| `NiExtraDataController` | >= 0x4, <= 0x40 | 0x40 | 4 of 4 fit, 2 of this size | `NiAnimation26VC80R.dll` | applied; base `NiSingleInterpController` |
| `NiFloatInterpController` | >= 0x4, <= 0x38 | 0x38 | 3 of 3 fit, 1 of this size | `NiAnimation26VC80R.dll` | applied; base `NiSingleInterpController` |
| `NiPSysModifierCtlr` | >= 0x4, <= 0x40 | 0x40 | 19 of 20 fit, 19 of this size | `NiParticle26VC80R.dll` | applied; base `NiSingleInterpController` |
| `NiPSysModifierBoolCtlr` | >= 0x4, <= 0x40 | 0x40 | 1 of 1 fit, 1 of this size | `NiParticle26VC80R.dll` | applied; base `NiPSysModifierCtlr` |
| `NiPSysModifierFloatCtlr` | >= 0x4, <= 0x40 | 0x40 | 18 of 18 fit, 18 of this size | `NiParticle26VC80R.dll` | applied; base `NiPSysModifierCtlr` |
| `NiPoint3InterpController` | >= 0x4, <= 0x3c | 0x38 | 2 of 2 fit, 0 of this size | `NiAnimation26VC80R.dll` | applied; base `NiSingleInterpController` |

## Earlier inferred layouts re-checked

Structures an earlier run filled with an inferred size, evaluated again against the current sources. A contradicted one is kept as it is and listed here and under Anomalies.

| Class | 2.6 size | Sized descendants | Result |
|---|---:|---|---|
| `NiObject` | 0x8 | 118 of 167 fit, 0 of this size | consistent |
| `NiAVObjectPalette` | 0x8 | 1 of 1 fit, 0 of this size | consistent |
| `NiAccumulator` | 0xc | 1 of 1 fit, 0 of this size | consistent |
| `NiBackToFrontAccumulator` | 0x30 | 1 of 1 fit, 0 of this size | consistent |
| `NiLODData` | 0x8 | 2 of 2 fit, 0 of this size | consistent |
| `NiObjectNET` | 0x18 | 13 of 34 fit, 1 of this size | consistent |
| `NiProperty` | 0x18 | 11 of 12 fit, 1 of this size | consistent |
| `NiTexture` | 0x30 | 2 of 2 fit, 0 of this size | consistent |
| `NiTimeController` | 0x34 | 33 of 37 fit, 1 of this size | consistent |

## Size mismatches

| Class | UOSA size or bounds | 2.6 size |
|---|---|---:|
| `NiLinesData` | 0x48 | 0x40 |
| `NiParticlesData` | 0x58 | 0x50 |
| `NiPSysData` | 0x64 | 0x5c |
| `NiMeshPSysData` | 0x80 | 0x6c |
| `NiTriShapeData` | 0x5c | 0x54 |
| `NiScreenElementsData` | 0x74 | 0x6c |
| `NiScreenGeometryData` | 0x74 | 0x6c |
| `NiTriShapeDynamicData` | 0x5c | 0x54 |
| `NiTriStripsData` | 0x54 | 0x4c |
| `NiBSplineColorInterpolator` | 0x34 | 0x30 |
| `NiBSplineCompColorInterpolator` | 0x3c | 0x38 |
| `NiCamera` | 0x130 | 0x124 |
| `NiScreenSpaceCamera` | 0x150 | 0x144 |
| `NiAmbientLight` | 0x110 | 0x10c |
| `NiDirectionalLight` | 0x11c | 0x118 |
| `NiPointLight` | 0x11c | 0x118 |
| `NiSpotLight` | 0x13c | 0x138 |
| `NiTextureEffect` | 0x17c | 0x178 |
| `NiLines` | 0xe4 | 0xd4 |
| `NiParticles` | 0xe4 | 0xd4 |
| `NiParticleSystem` | 0x134 | 0x124 |
| `NiMeshParticleSystem` | >= 0x134, no upper bound | 0x124 |
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
| `NiPSysPlanarCollider` | 0x100 | 0x40 |
| `NiPSysSphericalCollider` | 0xb0 | 0x24 |
| `NiPSysBoundUpdateModifier` | 0x20 | 0x1c |
| `NiPSysColorModifier` | 0x24 | 0x1c |
| `NiPSysMeshEmitter` | 0x84 | 0x74 |
| `NiPSysAirFieldModifier` | 0x68 | 0x54 |
| `NiPSysDragFieldModifier` | 0x40 | 0x3c |
| `NiPSysGravityFieldModifier` | 0x48 | 0x44 |
| `NiPSysRadialFieldModifier` | 0x34 | 0x30 |
| `NiPSysTurbulenceFieldModifier` | 0x3c | 0x30 |
| `NiPSysVortexFieldModifier` | 0x3c | 0x38 |
| `NiScreenTexture` | 0x20 | 0x18 |
| `NiSkinInstance` | 0x30 | 0x18 |
| `NiControllerManager` | 0x6c | 0xa0 |
| `NiMultiTargetTransformController` | 0x40 | 0x34 |
| `NiPSysEmitterCtlr` | 0x5c | 0x48 |
| `NiPSysResetOnLoopCtlr` | 0x38 | 0x34 |

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
| `NiCollisionData` | >= 0x4, no upper bound | 0x50 |
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
| `NiAlphaController` | >= 0x4, no upper bound | 0x38 |
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
| `NiGeometryData` | >= 0x4, <= 0x48 | 0x3c | none of the 9 sized descendants with a 2.6 type fits |
| `NiTriBasedGeomData` | >= 0x4, <= 0x54 | 0x40 | none of the 5 sized descendants with a 2.6 type fits |
| `NiAVObject` | >= 0x4, <= 0xe0 | 0xac | none of the 20 sized descendants with a 2.6 type fits |
| `NiDynamicEffect` | >= 0x4, <= 0x110 | 0xe0 | none of the 5 sized descendants with a 2.6 type fits |
| `NiLight` | >= 0x4, <= 0x110 | 0x10c | none of the 4 sized descendants with a 2.6 type fits |
| `NiGeometry` | >= 0x4, <= 0xe4 | 0xd4 | none of the 7 sized descendants with a 2.6 type fits |
| `NiTriBasedGeom` | >= 0x4, <= 0xe4 | 0xd4 | none of the 4 sized descendants with a 2.6 type fits |
| `NiPSysCollider` | >= 0x4, <= 0xb0 | 0x1c | none of the 2 sized descendants with a 2.6 type fits |
| `NiPSysFieldModifier` | >= 0x4, <= 0x34 | 0x2c | none of the 6 sized descendants with a 2.6 type fits |

## No 2.6 structure in the sources

`ShaderOverrideSorter`, `UOShaderOverrideSorter`, `MythicShadowGenerator`, `NiBlendAccumTransformInterpolator`, `MythicAmbientLight`, `MythicDirectionalLight`, `MythicPointLight`, `CachingTriShape`, `FoliageTriShape`, `MobAnimTriShape`, `MobAnimTriShapeShadow`, `ParticleTriShape`, `SpriteTriShape`, `TerrainTriShape`, `UOCollisionNode`, `UOObjectCollisionNode`, `UOStencilCollisionNode`, `UOTerrainCollisionNode`, `WeaponTrailTriShape`, `Effect`, `DynamicEffect`, `WeaponTrailEffect`, `EmitterEffect`, `GenericShaderTexturingProperty`, `UOTerrainTexturingProperty`, `MobAnimTexture`, `NiGeomMorpherUpdateTask`, `NiPSysUpdateTask`, `NiFloatController`, `NiRollController`, `NiLookAtController`, `NiPathController`, `MythicBaseShader`, `ColorShader`, `DepthOnlyShader`, `LightingShader`, `AtlasTerrainShader`, `GameTerrainShader`, `GenericShader`, `WarhammerCharacterShader`, `WarhammerFixtureBlendShader`, `WarhammerFixtureShader`, `NewWaterShader`, `ShadowMapShader`, `SimpleFFP`, `UIDecalShader`, `UiShader`, `VCAlphaTextureBlendShader`, `UOBaseShader`, `UODeathShaderFFP`, `UODeathShaderHLSL`, `UOEffectsShaderFFP`, `UOEffectsShaderHLSL`, `UOFadeShader`, `UOLightingShader`, `UONightMapShader`, `UOShadowShader`, `UOSpriteShaderDepthOnly`, `UOSpriteShaderFFP`, `UOSpriteShaderMob`, `UOSpriteShaderHLSL`, `UOSpriteShaderShadow`, `UOSpriteUIShaderFFP`, `UOSpriteUIShaderHLSL`, `UOStaticTerrainShader`, `UOTerrainShader`, `UOWaterShader`

## Edited by the user (kept)

None.

## Other types in /Gamebryo

`NiRTTI`

## Anomalies

None.

## Applied changes

- class structures filled: 96
- 2.6 types copied to `/Gamebryo26`: 47
