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

### `NiVectorExtraData` (0x1c, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiExtraData` | `base` |  |
| 0xc | `float[4]` | `m_afVector` |  |

### `NiVertWeightsExtraData` (0xc, exact)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiExtraData` | `base` |  |

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

## Types copied to `/Gamebryo26`

### `/Gamebryo26/NiTStringPointerMap<NiAVObject_*,NiStringHashFunctor,NiStringEqualsFunctor>` (0x14, structure)

### `/Gamebryo26/NiTPointerList<NiRenderObject_*>` (0x10, structure)

### `/Gamebryo26/NiRenderObject` (no size, stub (pointer target, no members))

### `/Gamebryo26/NiAdditionalGeometryData/NiAGDDataStream` (no size, stub (pointer target, no members))

### `/Gamebryo26/NiTPrimitiveArray<NiAdditionalGeometryData::NiAGDDataBlock_*>` (0x10, structure)

### `/Gamebryo26/NiFixedString` (0x4, structure)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `char *` | `m_kHandle` |  |

### `/Gamebryo26/NiColorA` (0x10, structure)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `float` | `r` |  |
| 0x4 | `float` | `g` |  |
| 0x8 | `float` | `b` |  |
| 0xc | `float` | `a` |  |

### `/Gamebryo26/NiPoint3` (0xc, structure)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `float` | `x` |  |
| 0x4 | `float` | `y` |  |
| 0x8 | `float` | `z` |  |

### `/Gamebryo26/NiRangeLODData/Range` (no size, stub (pointer target, no members))

### `/Gamebryo26/NiBound` (0x10, structure)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiPoint3` | `m_kCenter` |  |
| 0xc | `float` | `m_fRadius` |  |

### `/Gamebryo26/NiPointer<NiTimeController>` (0x4, structure)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiTimeController *` | `m_pObject` |  |

### `/Gamebryo26/NiColor` (0xc, structure)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `float` | `r` |  |
| 0x4 | `float` | `g` |  |
| 0x8 | `float` | `b` |  |

### `/Gamebryo26/NiTexture/FormatPrefs` (0xc, structure)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `PixelLayout` | `m_ePixelLayout` |  |
| 0x4 | `AlphaFormat` | `m_eAlphaFmt` |  |
| 0x8 | `MipFlag` | `m_eMipMapped` |  |

### `/Gamebryo26/NiTexture/FormatPrefs/PixelLayout` (0x4, enum)

### `/Gamebryo26/NiTexture/FormatPrefs/AlphaFormat` (0x4, enum)

### `/Gamebryo26/NiTexture/FormatPrefs/MipFlag` (0x4, enum)

### `/Gamebryo26/NiTexture/RendererData` (no size, stub (pointer target, no members))

### `/Gamebryo26/NiPointer<NiPersistentSrcTextureRendererData>` (0x4, structure)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiPersistentSrcTextureRendererData *` | `m_pObject` |  |

### `/Gamebryo26/NiPointer<NiPixelData>` (0x4, structure)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiPixelData *` | `m_pObject` |  |

### `/Gamebryo26/NiSourceCubeMap/FaceID` (0x4, enum)

### `/Gamebryo26/NiPalette/PaletteEntry` (no size, stub (pointer target, no members))

### `/Gamebryo26/NiPalette/RendererData` (no size, stub (pointer target, no members))

### `/Gamebryo26/NiPersistentSrcTextureRendererData/PlatformID` (0x4, enum)

### `/Gamebryo26/NiPointer<NiPalette>` (0x4, structure)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiPalette *` | `m_pObject` |  |

### `/Gamebryo26/NiPixelFormat` (0x44, structure)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `uchar` | `m_uFlags` |  |
| 0x1 | `uchar` | `m_ucBitsPerPixel` |  |
| 0x2 | `bool` | `m_bSRGBSpace` |  |
| 0x4 | `Format` | `m_eFormat` |  |
| 0x8 | `Tiling` | `m_eTiling` |  |
| 0xc | `uint` | `m_uiRendererHint` |  |
| 0x10 | `uint` | `m_uiExtraData` |  |
| 0x14 | `NiComponentSpec[4]` | `m_akComponents` |  |

### `/Gamebryo26/NiPixelFormat/Format` (0x4, enum)

### `/Gamebryo26/NiPixelFormat/Tiling` (0x4, enum)

### `/Gamebryo26/NiPixelFormat/NiComponentSpec` (0xc, structure)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `Component` | `m_eComponent` |  |
| 0x4 | `Representation` | `m_eRepresentation` |  |
| 0x8 | `uchar` | `m_ucBitsPerComponent` |  |
| 0x9 | `bool` | `m_bSigned` |  |

### `/Gamebryo26/NiPixelFormat/Component` (0x4, enum)

### `/Gamebryo26/NiPixelFormat/Representation` (0x4, enum)

### `/Gamebryo26/NiPointer<NiPropertyState>` (0x4, structure)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiPropertyState *` | `m_pObject` |  |

### `/Gamebryo26/NiPropertyState` (no size, stub (pointer target, no members))

### `/Gamebryo26/NiPoint2` (no size, stub (pointer target, no members))

### `/Gamebryo26/NiTPointerList<NiNode_*>` (0x10, structure)

### `/Gamebryo26/NiPointer<NiSkinPartition>` (0x4, structure)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiSkinPartition *` | `m_pObject` |  |

### `/Gamebryo26/NiTransform` (0x34, structure)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `NiMatrix3` | `m_Rotate` |  |
| 0x24 | `NiPoint3` | `m_Translate` |  |
| 0x30 | `float` | `m_fScale` |  |

### `/Gamebryo26/NiMatrix3` (0x24, structure)

| Offset | Type | Name | Comment |
|---:|---|---|---|
| 0x0 | `float[3][3]` | `m_pEntry` |  |

### `/Gamebryo26/NiSkinData/BoneData` (no size, stub (pointer target, no members))

### `/Gamebryo26/NiSkinPartition/Partition` (no size, stub (pointer target, no members))
