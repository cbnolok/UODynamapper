# Gamebryo 2.6 layouts: members

Program: UOSA.exe
Mode: --apply

## Classes

### `NiObject` (0x8, inferred)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `void *` | `vftable` | NiRefObject vptr |
| 0x4 | `uint` | `m_uiRefCount` | NiRefObject |

### `NiAVObjectPalette` (0x8, inferred)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiObject` | `base` |  |

### `NiDefaultAVObjectPalette` (0x20, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiAVObjectPalette` | `base` |  |
| 0x8 | `NiTStringPointerMap<NiAVObject_*,NiStringHashFunctor,NiStringEqualsFunctor>` | `m_kHash` |  |
| 0x1c | `NiAVObject *` | `m_pkScene` |  |

### `NiAccumulator` (0xc, inferred)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiObject` | `base` |  |
| 0x8 | `NiCamera *` | `m_pkCamera` |  |

### `NiBackToFrontAccumulator` (0x30, inferred)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiAccumulator` | `base` |  |
| 0xc | `NiTPointerList<NiRenderObject_*>` | `m_kItems` |  |
| 0x1c | `int` | `m_iNumItems` |  |
| 0x20 | `int` | `m_iMaxItems` |  |
| 0x24 | `NiRenderObject * *` | `m_ppkItems` |  |
| 0x28 | `float *` | `m_pfDepths` |  |
| 0x2c | `int` | `m_iCurrItem` |  |

### `NiAlphaAccumulator` (0x34, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiBackToFrontAccumulator` | `base` |  |
| 0x30 | `bool` | `m_bObserveNoSortHint` |  |
| 0x31 | `bool` | `m_bSortByClosestPoint` |  |

### `NiAdditionalGeometryData` (0x28, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiObject` | `base` |  |
| 0x8 | `uint` | `m_uiRevID` |  |
| 0xc | `ushort` | `m_usVertexCount` |  |
| 0x10 | `uint` | `m_uiDataStreamCount` |  |
| 0x14 | `NiAGDDataStream *` | `m_pkDataStreamEntries` |  |
| 0x18 | `NiTPrimitiveArray<NiAdditionalGeometryData::NiAGDDataBlock_*>` | `m_aDataBlocks` |  |

### `NiBSplineBasisData` (0x28, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiObject` | `base` |  |
| 0x8 | `NiBSplineBasis<float,3>` | `m_kBasisDegree3` |  |

### `NiBSplineData` (0x18, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiObject` | `base` |  |
| 0x8 | `float *` | `m_pafControlPoints` |  |
| 0xc | `short *` | `m_pasCompactControlPoints` |  |
| 0x10 | `uint` | `m_uiControlPointCount` |  |
| 0x14 | `uint` | `m_uiCompactControlPointCount` |  |

### `NiBoolData` (0x18, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiObject` | `base` |  |
| 0x8 | `uint` | `m_uiNumKeys` |  |
| 0xc | `NiBoolKey *` | `m_pkKeys` |  |
| 0x10 | `KeyType` | `m_eType` |  |
| 0x14 | `uchar` | `m_ucKeySize` |  |

### `NiColorData` (0x18, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiObject` | `base` |  |
| 0x8 | `uint` | `m_uiNumKeys` |  |
| 0xc | `NiColorKey *` | `m_pkKeys` |  |
| 0x10 | `KeyType` | `m_eType` |  |
| 0x14 | `uchar` | `m_ucKeySize` |  |

### `NiExtraData` (0xc, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiObject` | `base` |  |
| 0x8 | `NiFixedString` | `m_kName` |  |

### `NiBinaryExtraData` (0x14, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiExtraData` | `base` |  |
| 0xc | `char *` | `m_pcBinaryData` |  |
| 0x10 | `uint` | `m_uiSize` |  |

### `NiBooleanExtraData` (0x10, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiExtraData` | `base` |  |
| 0xc | `bool` | `m_bValue` |  |

### `NiColorExtraData` (0x1c, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiExtraData` | `base` |  |
| 0xc | `NiColorA` | `m_kColorA` |  |

### `NiFloatExtraData` (0x10, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiExtraData` | `base` |  |
| 0xc | `float` | `m_fValue` |  |

### `NiFloatsExtraData` (0x14, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiExtraData` | `base` |  |
| 0xc | `uint` | `m_uiSize` |  |
| 0x10 | `float *` | `m_pfValue` |  |

### `NiIntegerExtraData` (0x10, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiExtraData` | `base` |  |
| 0xc | `int` | `m_iValue` |  |

### `NiIntegersExtraData` (0x14, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiExtraData` | `base` |  |
| 0xc | `uint` | `m_uiSize` |  |
| 0x10 | `int *` | `m_piValue` |  |

### `NiStringExtraData` (0x10, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiExtraData` | `base` |  |
| 0xc | `NiFixedString` | `m_kString` |  |

### `NiStringsExtraData` (0x14, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiExtraData` | `base` |  |
| 0xc | `uint` | `m_uiSize` |  |
| 0x10 | `char * *` | `m_ppcValue` |  |

### `NiSwitchStringExtraData` (0x18, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiExtraData` | `base` |  |
| 0xc | `uint` | `m_uiSize` |  |
| 0x10 | `char * *` | `m_ppcValue` |  |
| 0x14 | `int` | `m_iIndex` |  |

### `NiTextKeyExtraData` (0x14, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiExtraData` | `base` |  |
| 0xc | `uint` | `m_uiNumKeys` |  |
| 0x10 | `NiTextKey *` | `m_pKeys` |  |

### `NiVectorExtraData` (0x1c, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiExtraData` | `base` |  |
| 0xc | `float[4]` | `m_afVector` |  |

### `NiVertWeightsExtraData` (0xc, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiExtraData` | `base` |  |

### `NiFloatData` (0x18, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiObject` | `base` |  |
| 0x8 | `uint` | `m_uiNumKeys` |  |
| 0xc | `NiFloatKey *` | `m_pkKeys` |  |
| 0x10 | `KeyType` | `m_eType` |  |
| 0x14 | `uchar` | `m_ucKeySize` |  |

### `NiInterpolator` (0xc, inferred)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiObject` | `base` |  |
| 0x8 | `float` | `m_fLastTime` |  |

### `NiBSplineInterpolator` (0x1c, inferred)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiInterpolator` | `base` |  |
| 0xc | `float` | `m_fStartTime` |  |
| 0x10 | `float` | `m_fEndTime` |  |
| 0x14 | `NiPointer<NiBSplineData>` | `m_spData` |  |
| 0x18 | `NiPointer<NiBSplineBasisData>` | `m_spBasisData` |  |

### `NiBSplineFloatInterpolator` (0x24, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiBSplineInterpolator` | `base` |  |
| 0x1c | `float` | `m_fFloatValue` |  |
| 0x20 | `uint` | `m_kFloatCPHandle` |  |

### `NiBSplineCompFloatInterpolator` (0x2c, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiBSplineFloatInterpolator` | `base` |  |
| 0x24 | `float[2]` | `m_afCompScalars` |  |

### `NiBSplinePoint3Interpolator` (0x2c, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiBSplineInterpolator` | `base` |  |
| 0x1c | `NiPoint3` | `m_kPoint3Value` |  |
| 0x28 | `uint` | `m_kPoint3CPHandle` |  |

### `NiBSplineCompPoint3Interpolator` (0x34, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiBSplinePoint3Interpolator` | `base` |  |
| 0x2c | `float[2]` | `m_afCompScalars` |  |

### `NiBSplineTransformInterpolator` (0x48, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiBSplineInterpolator` | `base` |  |
| 0x1c | `NiQuatTransform` | `m_kTransformValue` |  |
| 0x3c | `uint` | `m_kTransCPHandle` |  |
| 0x40 | `uint` | `m_kRotCPHandle` |  |
| 0x44 | `uint` | `m_kScaleCPHandle` |  |

### `NiBSplineCompTransformInterpolator` (0x60, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiBSplineTransformInterpolator` | `base` |  |
| 0x48 | `float[6]` | `m_afCompScalars` |  |

### `NiBlendInterpolator` (0x30, inferred)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiInterpolator` | `base` |  |
| 0xc | `uchar` | `m_uFlags` |  |
| 0xd | `uchar` | `m_ucArraySize` |  |
| 0xe | `uchar` | `m_ucInterpCount` |  |
| 0xf | `uchar` | `m_ucSingleIdx` |  |
| 0x10 | `char` | `m_cHighPriority` |  |
| 0x11 | `char` | `m_cNextHighPriority` |  |
| 0x14 | `InterpArrayItem *` | `m_pkInterpArray` |  |
| 0x18 | `NiInterpolator *` | `m_pkSingleInterpolator` |  |
| 0x1c | `float` | `m_fWeightThreshold` |  |
| 0x20 | `float` | `m_fSingleTime` |  |
| 0x24 | `float` | `m_fHighSumOfWeights` |  |
| 0x28 | `float` | `m_fNextHighSumOfWeights` |  |
| 0x2c | `float` | `m_fHighEaseSpinner` |  |

### `NiBlendBoolInterpolator` (0x34, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiBlendInterpolator` | `base` |  |
| 0x30 | `uchar` | `m_bBoolValue` |  |

### `NiBlendColorInterpolator` (0x44, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiBlendInterpolator` | `base` |  |
| 0x30 | `NiColorA` | `m_kColorValue` |  |
| 0x40 | `bool` | `m_bValidValue` |  |

### `NiBlendFloatInterpolator` (0x34, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiBlendInterpolator` | `base` |  |
| 0x30 | `float` | `m_fFloatValue` |  |

### `NiBlendPoint3Interpolator` (0x40, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiBlendInterpolator` | `base` |  |
| 0x30 | `NiPoint3` | `m_kPoint3Value` |  |
| 0x3c | `bool` | `m_bValidValue` |  |

### `NiBlendQuaternionInterpolator` (0x40, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiBlendInterpolator` | `base` |  |
| 0x30 | `NiQuaternion` | `m_kQuaternionValue` |  |

### `NiBlendTransformInterpolator` (0x30, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiBlendInterpolator` | `base` |  |

### `NiKeyBasedInterpolator` (0xc, inferred)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiInterpolator` | `base` |  |

### `NiBoolInterpolator` (0x18, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiKeyBasedInterpolator` | `base` |  |
| 0xc | `uchar` | `m_bBoolValue` |  |
| 0x10 | `NiPointer<NiBoolData>` | `m_spBoolData` |  |
| 0x14 | `uint` | `m_uiLastIdx` |  |

### `NiBoolTimelineInterpolator` (0x20, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiBoolInterpolator` | `base` |  |
| 0x18 | `uint` | `m_uiPreviousIdx` |  |
| 0x1c | `bool` | `m_bPreviousValue` |  |

### `NiColorInterpolator` (0x24, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiKeyBasedInterpolator` | `base` |  |
| 0xc | `NiColorA` | `m_kColorValue` |  |
| 0x1c | `NiPointer<NiColorData>` | `m_spColorData` |  |
| 0x20 | `uint` | `m_uiLastIdx` |  |

### `NiFloatInterpolator` (0x18, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiKeyBasedInterpolator` | `base` |  |
| 0xc | `float` | `m_fFloatValue` |  |
| 0x10 | `NiPointer<NiFloatData>` | `m_spFloatData` |  |
| 0x14 | `uint` | `m_uiLastIdx` |  |

### `NiPathInterpolator` (0x5c, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiKeyBasedInterpolator` | `base` |  |
| 0xc | `ushort` | `m_uFlags` |  |
| 0x10 | `uint` | `m_uiLastPathIdx` |  |
| 0x14 | `uint` | `m_uiLastPctIdx` |  |
| 0x18 | `NiPointer<NiPosData>` | `m_spPathData` |  |
| 0x1c | `NiPointer<NiFloatData>` | `m_spPctData` |  |
| 0x20 | `float *` | `m_pfPartialLength` |  |
| 0x24 | `float` | `m_fTotalLength` |  |
| 0x28 | `float` | `m_fMaxBankAngle` |  |
| 0x2c | `float` | `m_fSmoothing` |  |
| 0x30 | `short` | `m_sFollowAxis` |  |
| 0x34 | `float` | `m_fMaxCurvature` |  |
| 0x38 | `BankDir` | `m_eBankDir` |  |
| 0x3c | `NiQuaternion` | `m_kRotation` |  |
| 0x4c | `NiPoint3` | `m_kPosition` |  |
| 0x58 | `float` | `m_fLastTime` |  |

### `NiPoint3Interpolator` (0x20, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiKeyBasedInterpolator` | `base` |  |
| 0xc | `NiPoint3` | `m_kPoint3Value` |  |
| 0x18 | `NiPointer<NiPosData>` | `m_spPoint3Data` |  |
| 0x1c | `uint` | `m_uiLastIdx` |  |

### `NiQuaternionInterpolator` (0x24, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiKeyBasedInterpolator` | `base` |  |
| 0xc | `NiQuaternion` | `m_kQuaternionValue` |  |
| 0x1c | `NiPointer<NiRotData>` | `m_spQuaternionData` |  |
| 0x20 | `uint` | `m_uiLastIdx` |  |

### `NiTransformInterpolator` (0x3c, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiKeyBasedInterpolator` | `base` |  |
| 0xc | `NiQuatTransform` | `m_kTransformValue` |  |
| 0x2c | `NiPointer<NiTransformData>` | `m_spData` |  |
| 0x30 | `uint` | `m_uiLastTransIdx` |  |
| 0x34 | `uint` | `m_uiLastRotIdx` |  |
| 0x38 | `uint` | `m_uiLastScaleIdx` |  |

### `NiLookAtInterpolator` (0x44, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiInterpolator` | `base` |  |
| 0xc | `ushort` | `m_uFlags` |  |
| 0x10 | `NiAVObject *` | `m_pkLookAt` |  |
| 0x14 | `NiFixedString` | `m_kLookAtName` |  |
| 0x18 | `NiQuatTransform` | `m_kTransformValue` |  |
| 0x38 | `NiPointer<NiInterpolator>[3]` | `m_aspInterpolators` |  |

### `NiLODData` (0x8, inferred)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiObject` | `base` |  |

### `NiRangeLODData` (0x28, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiLODData` | `base` |  |
| 0x8 | `NiPoint3` | `m_kCenter` |  |
| 0x14 | `NiPoint3` | `m_kWorldCenter` |  |
| 0x20 | `uint` | `m_uiNumRanges` |  |
| 0x24 | `Range *` | `m_pkRanges` |  |

### `NiScreenLODData` (0x30, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiLODData` | `base` |  |
| 0x8 | `NiBound` | `m_kBound` |  |
| 0x18 | `NiBound` | `m_kWorldBound` |  |
| 0x28 | `uint` | `m_uiNumProportions` |  |
| 0x2c | `float *` | `m_pfProportions` |  |

### `NiMorphData` (0x18, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiObject` | `base` |  |
| 0x8 | `uint` | `m_uiNumTargets` |  |
| 0xc | `uint` | `m_uiNumVertsPerTarget` |  |
| 0x10 | `MorphTarget *` | `m_aMorphTargets` |  |
| 0x14 | `bool` | `m_bRelativeTargets` |  |

### `NiObjectNET` (0x18, inferred)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiObject` | `base` |  |
| 0x8 | `NiFixedString` | `m_kName` |  |
| 0xc | `NiPointer<NiTimeController>` | `m_spControllers` |  |
| 0x10 | `NiExtraData * *` | `m_ppkExtra` |  |
| 0x14 | `ushort` | `m_usExtraDataSize` |  |
| 0x16 | `ushort` | `m_usMaxSize` |  |

### `NiProperty` (0x18, inferred)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiObjectNET` | `base` |  |

### `NiAlphaProperty` (0x1c, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiProperty` | `base` |  |
| 0x18 | `ushort` | `m_uFlags` |  |
| 0x1a | `uchar` | `m_ucAlphaTestRef` |  |

### `NiDitherProperty` (0x1c, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiProperty` | `base` |  |
| 0x18 | `ushort` | `m_uFlags` |  |

### `NiFogProperty` (0x2c, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiProperty` | `base` |  |
| 0x18 | `ushort` | `m_uFlags` |  |
| 0x1c | `float` | `m_fDepth` |  |
| 0x20 | `NiColor` | `m_kColor` |  |

### `NiMaterialProperty` (0x5c, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiProperty` | `base` |  |
| 0x18 | `int` | `m_iIndex` |  |
| 0x1c | `NiColor` | `m_amb` |  |
| 0x28 | `NiColor` | `m_diff` |  |
| 0x34 | `NiColor` | `m_spec` |  |
| 0x40 | `NiColor` | `m_emit` |  |
| 0x4c | `float` | `m_fShine` |  |
| 0x50 | `float` | `m_fAlpha` |  |
| 0x54 | `uint` | `m_uiRevID` |  |
| 0x58 | `void *` | `m_pvRendererData` |  |

### `NiRendererSpecificProperty` (0x18, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiProperty` | `base` |  |

### `NiShadeProperty` (0x1c, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiProperty` | `base` |  |
| 0x18 | `ushort` | `m_uFlags` |  |

### `NiSpecularProperty` (0x1c, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiProperty` | `base` |  |
| 0x18 | `ushort` | `m_uFlags` |  |

### `NiStencilProperty` (0x24, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiProperty` | `base` |  |
| 0x18 | `ushort` | `m_uFlags` |  |
| 0x1c | `uint` | `m_uiRef` |  |
| 0x20 | `uint` | `m_uiMask` |  |

### `NiVertexColorProperty` (0x1c, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiProperty` | `base` |  |
| 0x18 | `ushort` | `m_uFlags` |  |

### `NiWireframeProperty` (0x1c, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiProperty` | `base` |  |
| 0x18 | `ushort` | `m_uFlags` |  |

### `NiZBufferProperty` (0x1c, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiProperty` | `base` |  |
| 0x18 | `ushort` | `m_uFlags` |  |

### `NiTexture` (0x30, inferred)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiObjectNET` | `base` |  |
| 0x18 | `FormatPrefs` | `m_kFormatPrefs` |  |
| 0x24 | `RendererData *` | `m_pkRendererData` |  |
| 0x28 | `NiTexture *` | `m_pkPrev` |  |
| 0x2c | `NiTexture *` | `m_pkNext` |  |

### `NiSourceTexture` (0x44, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiTexture` | `base` |  |
| 0x30 | `NiFixedString` | `m_kFilename` |  |
| 0x34 | `NiFixedString` | `m_kPlatformFilename` |  |
| 0x38 | `NiPointer<NiPersistentSrcTextureRendererData>` | `m_spPersistentSrcRendererData` |  |
| 0x3c | `NiPointer<NiPixelData>` | `m_spSrcPixelData` |  |
| 0x40 | `bool` | `m_bStatic` |  |
| 0x41 | `bool` | `m_bLoadDirectToRendererHint` |  |
| 0x42 | `bool` | `m_bSrcRendererDataIsPersistent` |  |

### `NiSourceCubeMap` (0x48, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiSourceTexture` | `base` |  |
| 0x44 | `FaceID` | `m_eFace` |  |

### `NiPSysEmitterCtlrData` (0x28, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiObject` | `base` |  |
| 0x8 | `uint` | `m_uiNumBirthRateKeys` |  |
| 0xc | `NiFloatKey *` | `m_pkBirthRateKeys` |  |
| 0x10 | `KeyType` | `m_eBirthRateKeyType` |  |
| 0x14 | `uchar` | `m_ucBirthRateSize` |  |
| 0x18 | `uint` | `m_uiNumEmitterActiveKeys` |  |
| 0x1c | `NiBoolKey *` | `m_pkEmitterActiveKeys` |  |
| 0x20 | `KeyType` | `m_eEmitterActiveKeyType` |  |
| 0x24 | `uchar` | `m_ucEmitterActiveSize` |  |

### `NiPSysModifier` (0x18, inferred)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiObject` | `base` |  |
| 0x8 | `NiFixedString` | `m_kName` |  |
| 0xc | `uint` | `m_uiOrder` |  |
| 0x10 | `NiParticleSystem *` | `m_pkTarget` |  |
| 0x14 | `bool` | `m_bActive` |  |

### `NiPSysAgeDeathModifier` (0x20, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiPSysModifier` | `base` |  |
| 0x18 | `bool` | `m_bSpawnOnDeath` |  |
| 0x1c | `NiPSysSpawnModifier *` | `m_pkSpawnModifier` |  |

### `NiPSysBombModifier` (0x38, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiPSysModifier` | `base` |  |
| 0x18 | `NiAVObject *` | `m_pkBombObj` |  |
| 0x1c | `NiPoint3` | `m_kBombAxis` |  |
| 0x28 | `float` | `m_fDecay` |  |
| 0x2c | `float` | `m_fDeltaV` |  |
| 0x30 | `DecayType` | `m_eDecayType` |  |
| 0x34 | `SymmType` | `m_eSymmType` |  |

### `NiPSysColliderManager` (0x1c, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiPSysModifier` | `base` |  |
| 0x18 | `NiPointer<NiPSysCollider>` | `m_spColliders` |  |

### `NiPSysDragModifier` (0x34, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiPSysModifier` | `base` |  |
| 0x18 | `NiAVObject *` | `m_pkDragObj` |  |
| 0x1c | `NiPoint3` | `m_kDragAxis` |  |
| 0x28 | `float` | `m_fPercentage` |  |
| 0x2c | `float` | `m_fRange` |  |
| 0x30 | `float` | `m_fFalloff` |  |

### `NiPSysEmitter` (0x50, inferred)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiPSysModifier` | `base` |  |
| 0x18 | `float` | `m_fSpeed` |  |
| 0x1c | `float` | `m_fSpeedVar` |  |
| 0x20 | `float` | `m_fDeclination` |  |
| 0x24 | `float` | `m_fDeclinationVar` |  |
| 0x28 | `float` | `m_fPlanarAngle` |  |
| 0x2c | `float` | `m_fPlanarAngleVar` |  |
| 0x30 | `NiColorA` | `m_kInitialColor` |  |
| 0x40 | `float` | `m_fInitialRadius` |  |
| 0x44 | `float` | `m_fRadiusVar` |  |
| 0x48 | `float` | `m_fLifeSpan` |  |
| 0x4c | `float` | `m_fLifeSpanVar` |  |

### `NiPSysVolumeEmitter` (0x54, inferred)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiPSysEmitter` | `base` |  |
| 0x50 | `NiAVObject *` | `m_pkEmitterObj` |  |

### `NiPSysBoxEmitter` (0x60, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiPSysVolumeEmitter` | `base` |  |
| 0x54 | `float` | `m_fEmitterWidth` |  |
| 0x58 | `float` | `m_fEmitterHeight` |  |
| 0x5c | `float` | `m_fEmitterDepth` |  |

### `NiPSysCylinderEmitter` (0x5c, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiPSysVolumeEmitter` | `base` |  |
| 0x54 | `float` | `m_fEmitterRadius` |  |
| 0x58 | `float` | `m_fEmitterHeight` |  |

### `NiPSysSphereEmitter` (0x58, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiPSysVolumeEmitter` | `base` |  |
| 0x54 | `float` | `m_fEmitterRadius` |  |

### `NiPSysGravityModifier` (0x3c, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiPSysModifier` | `base` |  |
| 0x18 | `NiAVObject *` | `m_pkGravityObj` |  |
| 0x1c | `NiPoint3` | `m_kGravityAxis` |  |
| 0x28 | `float` | `m_fDecay` |  |
| 0x2c | `float` | `m_fStrength` |  |
| 0x30 | `ForceType` | `m_eType` |  |
| 0x34 | `float` | `m_fTurbulence` |  |
| 0x38 | `float` | `m_fScale` |  |

### `NiPSysGrowFadeModifier` (0x28, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiPSysModifier` | `base` |  |
| 0x18 | `float` | `m_fGrowTime` |  |
| 0x1c | `ushort` | `m_usGrowGeneration` |  |
| 0x20 | `float` | `m_fFadeTime` |  |
| 0x24 | `ushort` | `m_usFadeGeneration` |  |

### `NiPSysMeshUpdateModifier` (0x28, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiPSysModifier` | `base` |  |
| 0x18 | `NiTObjectArray<NiPointer<NiAVObject>_>` | `m_kMeshes` |  |

### `NiPSysPositionModifier` (0x18, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiPSysModifier` | `base` |  |

### `NiPSysRotationModifier` (0x38, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiPSysModifier` | `base` |  |
| 0x18 | `float` | `m_fInitialRotSpeed` |  |
| 0x1c | `float` | `m_fInitialRotSpeedVar` |  |
| 0x20 | `float` | `m_fInitialRotAngle` |  |
| 0x24 | `float` | `m_fInitialRotAngleVar` |  |
| 0x28 | `NiPoint3` | `m_kInitialAxis` |  |
| 0x34 | `bool` | `m_bRandomInitialAxis` |  |
| 0x35 | `bool` | `m_bRandomRotSpeedSign` |  |

### `NiPSysSpawnModifier` (0x34, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiPSysModifier` | `base` |  |
| 0x18 | `ushort` | `m_usNumSpawnGenerations` |  |
| 0x1c | `float` | `m_fPercentageSpawned` |  |
| 0x20 | `ushort` | `m_usMinNumToSpawn` |  |
| 0x22 | `ushort` | `m_usMaxNumToSpawn` |  |
| 0x24 | `float` | `m_fSpawnSpeedChaos` |  |
| 0x28 | `float` | `m_fSpawnDirChaos` |  |
| 0x2c | `float` | `m_fLifeSpan` |  |
| 0x30 | `float` | `m_fLifeSpanVar` |  |

### `NiPalette` (0x24, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiObject` | `base` |  |
| 0x8 | `bool` | `m_bAlpha` |  |
| 0xc | `uint` | `m_uiEntries` |  |
| 0x10 | `uint` | `m_uiRevID` |  |
| 0x14 | `PaletteEntry *` | `m_pkPalette` |  |
| 0x18 | `RendererData *` | `m_pkRendererData` |  |
| 0x1c | `NiPalette *` | `m_pkPrev` |  |
| 0x20 | `NiPalette *` | `m_pkNext` |  |

### `NiPersistentSrcTextureRendererData` (0x8c, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiObject` | `base` |  |
| 0x58 | `uchar *` | `m_pucPixels` |  |
| 0x5c | `uint *` | `m_puiWidth` |  |
| 0x60 | `uint *` | `m_puiHeight` |  |
| 0x64 | `uint *` | `m_puiOffsetInBytes` |  |
| 0x68 | `uint` | `m_uiPadOffsetInBytes` |  |
| 0x6c | `uint` | `m_uiMipmapLevels` |  |
| 0x70 | `uint` | `m_uiPixelStride` |  |
| 0x74 | `uint` | `m_uiFaces` |  |
| 0x78 | `PlatformID` | `m_eTargetPlatform` |  |
| 0x7c | `uchar *` | `m_pucPristinePixels` |  |
| 0x80 | `uint` | `m_uiPristineMaxOffsetInBytes` |  |
| 0x84 | `uint` | `m_uiPristinePadOffsetInBytes` |  |
| 0x88 | `NiPointer<NiPalette>` | `m_spPalette` |  |

### `NiPixelData` (0x70, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiObject` | `base` |  |
| 0x8 | `NiPixelFormat` | `m_kPixelFormat` |  |
| 0x4c | `NiPointer<NiPalette>` | `m_spPalette` |  |
| 0x50 | `uchar *` | `m_pucPixels` |  |
| 0x54 | `uint *` | `m_puiWidth` |  |
| 0x58 | `uint *` | `m_puiHeight` |  |
| 0x5c | `uint *` | `m_puiOffsetInBytes` |  |
| 0x60 | `uint` | `m_uiMipmapLevels` |  |
| 0x64 | `uint` | `m_uiPixelStride` |  |
| 0x68 | `uint` | `m_uiRevID` |  |
| 0x6c | `uint` | `m_uiFaces` |  |

### `NiPosData` (0x18, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiObject` | `base` |  |
| 0x8 | `uint` | `m_uiNumKeys` |  |
| 0xc | `NiPosKey *` | `m_pkKeys` |  |
| 0x10 | `KeyType` | `m_eType` |  |
| 0x14 | `uchar` | `m_ucKeySize` |  |

### `NiRotData` (0x18, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiObject` | `base` |  |
| 0x8 | `uint` | `m_uiNumKeys` |  |
| 0xc | `NiRotKey *` | `m_pkKeys` |  |
| 0x10 | `KeyType` | `m_eType` |  |
| 0x14 | `uchar` | `m_ucKeySize` |  |

### `NiScreenPolygon` (0x1c, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiObject` | `base` |  |
| 0x8 | `NiPointer<NiPropertyState>` | `m_spPropertyState` |  |
| 0xc | `ushort` | `m_usVertices` |  |
| 0x10 | `NiPoint3 *` | `m_pkVertex` |  |
| 0x14 | `NiPoint2 *` | `m_pkTexture` |  |
| 0x18 | `NiColorA *` | `m_pkColor` |  |

### `NiShadowGenerator` (0x68, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiObject` | `base` |  |
| 0x8 | `ushort` | `m_uFlags` |  |
| 0xc | `NiShadowTechnique *` | `m_pkShadowTechnique` |  |
| 0x10 | `NiTPointerList<NiNode_*>` | `m_kUnaffectedCasterList` |  |
| 0x20 | `NiTPointerList<NiNode_*>` | `m_kUnaffectedReceiverList` |  |
| 0x30 | `void *` | `m_pfnSMTextureRetrievalCallback` |  |
| 0x34 | `void *` | `m_pfnSMTextureAssignmentCallback` |  |
| 0x38 | `void *` | `m_pfnOwnedSMCallback` |  |
| 0x3c | `void *` | `m_pfnSMCountCallback` |  |
| 0x40 | `void *` | `m_pfnSMConstantDataAssignmentCallback` |  |
| 0x44 | `void *` | `m_pfnSMConstantDataRetrievalCallback` |  |
| 0x48 | `void *` | `m_pfnExtraDataCleanupCallback` |  |
| 0x4c | `NiDynamicEffect *` | `m_pkDynEffect` |  |
| 0x50 | `void *` | `m_pvExtraData` |  |
| 0x54 | `float` | `m_fDepthBias` |  |
| 0x58 | `float` | `m_fNearClippingDist` |  |
| 0x5c | `float` | `m_fFarClippingDist` |  |
| 0x60 | `float` | `m_fDirLightFrustumWidth` |  |
| 0x64 | `ushort` | `m_usSizeHint` |  |

### `NiSkinData` (0x48, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiObject` | `base` |  |
| 0x8 | `NiPointer<NiSkinPartition>` | `m_spSkinPartition` |  |
| 0xc | `NiTransform` | `m_kRootParentToSkin` |  |
| 0x40 | `BoneData *` | `m_pkBoneData` |  |
| 0x44 | `uint` | `m_uiBones` |  |

### `NiSkinPartition` (0x10, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiObject` | `base` |  |
| 0x8 | `uint` | `m_uiPartitions` |  |
| 0xc | `Partition *` | `m_pkPartitions` |  |

### `NiStringPalette` (0x14, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiObject` | `base` |  |
| 0x8 | `char *` | `m_pcBuffer` |  |
| 0xc | `uint` | `m_uiAllocatedSize` |  |
| 0x10 | `uint` | `m_uiEndOfBuffer` |  |

### `NiTimeController` (0x34, inferred)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiObject` | `base` |  |
| 0x8 | `ushort` | `m_uFlags` |  |
| 0xc | `float` | `m_fFrequency` |  |
| 0x10 | `float` | `m_fPhase` |  |
| 0x14 | `float` | `m_fLoKeyTime` |  |
| 0x18 | `float` | `m_fHiKeyTime` |  |
| 0x1c | `float` | `m_fStartTime` |  |
| 0x20 | `float` | `m_fLastTime` |  |
| 0x24 | `float` | `m_fWeightedLastTime` |  |
| 0x28 | `float` | `m_fScaledTime` |  |
| 0x2c | `NiObjectNET *` | `m_pkTarget` |  |
| 0x30 | `NiPointer<NiTimeController>` | `m_spNext` |  |

### `NiBoneLODController` (0x68, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiTimeController` | `base` |  |
| 0x34 | `int` | `m_iLOD` |  |
| 0x38 | `uint` | `m_uiNumLODs` |  |
| 0x3c | `NiTPrimitiveArray<NiTPrimitiveSet<NiNode_*>_*>` | `m_kBoneArray` |  |
| 0x4c | `NiTPrimitiveArray<NiTPrimitiveSet<NiBoneLODController::SkinInfo_*>_*>` | `m_kSkinArray` |  |
| 0x5c | `NiTPrimitiveSet<NiTriBasedGeom_*>` | `m_kSkinSet` |  |

### `NiInterpController` (0x34, inferred)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiTimeController` | `base` |  |

### `NiGeomMorpherController` (0x44, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiInterpController` | `base` |  |
| 0x34 | `ushort` | `m_uFlags` |  |
| 0x38 | `NiPointer<NiMorphData>` | `m_spMorphData` |  |
| 0x3c | `InterpItem *` | `m_pkInterpItems` |  |
| 0x40 | `bool` | `m_bAlwaysUpdate` |  |

### `NiSingleInterpController` (0x38, inferred)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiInterpController` | `base` |  |
| 0x34 | `NiPointer<NiInterpolator>` | `m_spInterpolator` |  |

### `NiBoolInterpController` (0x38, inferred)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiSingleInterpController` | `base` |  |

### `NiVisController` (0x38, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiBoolInterpController` | `base` |  |

### `NiExtraDataController` (0x40, inferred)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiSingleInterpController` | `base` |  |
| 0x38 | `NiFixedString` | `m_kExtraDataName` |  |
| 0x3c | `NiPointer<NiExtraData>` | `m_spExtraData` |  |

### `NiColorExtraDataController` (0x40, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiExtraDataController` | `base` |  |

### `NiFloatExtraDataController` (0x40, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiExtraDataController` | `base` |  |

### `NiFloatsExtraDataController` (0x48, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiExtraDataController` | `base` |  |
| 0x40 | `int` | `m_iFloatsExtraDataIndex` |  |
| 0x44 | `NiFixedString` | `m_kCtlrID` |  |

### `NiFloatsExtraDataPoint3Controller` (0x48, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiExtraDataController` | `base` |  |
| 0x40 | `int` | `m_iFloatsExtraDataIndex` |  |
| 0x44 | `NiFixedString` | `m_kCtlrID` |  |

### `NiFloatInterpController` (0x38, inferred)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiSingleInterpController` | `base` |  |

### `NiFlipController` (0x54, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiFloatInterpController` | `base` |  |
| 0x38 | `NiTObjectArray<NiPointer<NiTexture>_>` | `m_kTextures` |  |
| 0x48 | `uint` | `m_uiIndex` |  |
| 0x4c | `uint` | `m_uiAffectedMap` |  |
| 0x50 | `char *` | `m_pcCtlrID` |  |

### `NiLightDimmerController` (0x38, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiFloatInterpController` | `base` |  |

### `NiTextureTransformController` (0x50, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiFloatInterpController` | `base` |  |
| 0x38 | `uint` | `m_uiLastIdx` |  |
| 0x3c | `Map *` | `m_pkMap` |  |
| 0x40 | `bool` | `m_bShaderMap` |  |
| 0x44 | `uint` | `m_uiMapIndex` |  |
| 0x48 | `TransformMember` | `m_eMember` |  |
| 0x4c | `char *` | `m_pcCtlrID` |  |

### `NiPSysModifierCtlr` (0x40, inferred)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiSingleInterpController` | `base` |  |
| 0x38 | `NiFixedString` | `m_kModifierName` |  |
| 0x3c | `NiPSysModifier *` | `m_pkModifier` |  |

### `NiPSysModifierBoolCtlr` (0x40, inferred)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiPSysModifierCtlr` | `base` |  |

### `NiPSysModifierActiveCtlr` (0x40, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiPSysModifierBoolCtlr` | `base` |  |

### `NiPSysModifierFloatCtlr` (0x40, inferred)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiPSysModifierCtlr` | `base` |  |

### `NiPSysAirFieldAirFrictionCtlr` (0x40, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiPSysModifierFloatCtlr` | `base` |  |

### `NiPSysAirFieldInheritVelocityCtlr` (0x40, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiPSysModifierFloatCtlr` | `base` |  |

### `NiPSysAirFieldSpreadCtlr` (0x40, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiPSysModifierFloatCtlr` | `base` |  |

### `NiPSysEmitterDeclinationCtlr` (0x40, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiPSysModifierFloatCtlr` | `base` |  |

### `NiPSysEmitterDeclinationVarCtlr` (0x40, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiPSysModifierFloatCtlr` | `base` |  |

### `NiPSysEmitterInitialRadiusCtlr` (0x40, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiPSysModifierFloatCtlr` | `base` |  |

### `NiPSysEmitterLifeSpanCtlr` (0x40, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiPSysModifierFloatCtlr` | `base` |  |

### `NiPSysEmitterPlanarAngleCtlr` (0x40, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiPSysModifierFloatCtlr` | `base` |  |

### `NiPSysEmitterPlanarAngleVarCtlr` (0x40, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiPSysModifierFloatCtlr` | `base` |  |

### `NiPSysEmitterSpeedCtlr` (0x40, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiPSysModifierFloatCtlr` | `base` |  |

### `NiPSysFieldAttenuationCtlr` (0x40, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiPSysModifierFloatCtlr` | `base` |  |

### `NiPSysFieldMagnitudeCtlr` (0x40, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiPSysModifierFloatCtlr` | `base` |  |

### `NiPSysFieldMaxDistanceCtlr` (0x40, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiPSysModifierFloatCtlr` | `base` |  |

### `NiPSysGravityStrengthCtlr` (0x40, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiPSysModifierFloatCtlr` | `base` |  |

### `NiPSysInitialRotAngleCtlr` (0x40, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiPSysModifierFloatCtlr` | `base` |  |

### `NiPSysInitialRotAngleVarCtlr` (0x40, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiPSysModifierFloatCtlr` | `base` |  |

### `NiPSysInitialRotSpeedCtlr` (0x40, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiPSysModifierFloatCtlr` | `base` |  |

### `NiPSysInitialRotSpeedVarCtlr` (0x40, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiPSysModifierFloatCtlr` | `base` |  |

### `NiPoint3InterpController` (0x38, inferred)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiSingleInterpController` | `base` |  |

### `NiLightColorController` (0x3c, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiPoint3InterpController` | `base` |  |
| 0x38 | `ushort` | `m_uFlags` |  |

### `NiMaterialColorController` (0x3c, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiPoint3InterpController` | `base` |  |
| 0x38 | `ushort` | `m_uFlags` |  |

### `NiTransformController` (0x38, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiSingleInterpController` | `base` |  |

### `NiPSysUpdateCtlr` (0x34, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiTimeController` | `base` |  |

### `NiTransformData` (0x38, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiObject` | `base` |  |
| 0x8 | `uint` | `m_uiNumRotKeys` |  |
| 0xc | `NiRotKey *` | `m_pkRotKeys` |  |
| 0x10 | `KeyType` | `m_eRotType` |  |
| 0x14 | `uchar` | `m_ucRotSize` |  |
| 0x18 | `uint` | `m_uiNumPosKeys` |  |
| 0x1c | `NiPosKey *` | `m_pkPosKeys` |  |
| 0x20 | `KeyType` | `m_ePosType` |  |
| 0x24 | `uchar` | `m_ucPosSize` |  |
| 0x28 | `uint` | `m_uiNumScaleKeys` |  |
| 0x2c | `NiFloatKey *` | `m_pkScaleKeys` |  |
| 0x30 | `KeyType` | `m_eScaleType` |  |
| 0x34 | `uchar` | `m_ucScaleSize` |  |

## Types copied to `/Gamebryo26`

None in this run.
