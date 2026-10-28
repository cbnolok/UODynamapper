# Gamebryo 2.6 names in UOSA

Program: UOSA.exe
SHA-256: 5a5e9dc536c836d1bf10b6f86bb88f61a6ae078f519d4060d3bc47aecbebad50
Mode: analysis and --apply
Sources: `NiAnimation26VC80R.dll` (3738 functions), `NiCollision26VC80R.dll` (1183 functions), `NiDX9Renderer26VC80R.dll` (2568 functions), `NiMain26VC80R.dll` (5786 functions), `NiParticle26VC80R.dll` (2640 functions), `NiSystem26VC80R.dll` (624 functions)
BSim target ranges: 0x00650000-0x00830000, 0x00aa0000-0x00ac0000
Thresholds: similarity >= 0.7, significance >= 30.0, margin 0.05, self-significance > 15.0, duplicate-source similarity 0.97

| Item | Count |
|---|---:|
| Vtable: 2.6 classes with a vtable | 590 |
| Vtable: 2.6 classes with NiRTTI parent data | 387 |
| Vtable: UOSA classes with a vtable | 349 |
| Vtable: UOSA shared vtable families (common slots only) | 1 |
| Vtable: Classes mapped (equal chain lengths) | 265 |
| Vtable: Mapped slots | 4929 |
| Vtable: Slots without a unique 2.6 method name (folded or pure) | 538 |
| Vtable: Slots whose UOSA target is not a function | 1370 |
| Vtable: Slots whose UOSA target is also in a non-Gamebryo vtable | 1 |
| Vtable: UOSA functions named by vtables | 1073 |
| Vtable: UOSA functions with conflicting vtable names | 0 |
| BSim: Source functions with one 2.6 name (not thunks/imports) | 12142 |
| BSim: Source functions skipped as ICF-folded (several names) | 1673 |
| BSim: Sources above the self-significance bound | 8433 |
| BSim: UOSA functions in the ranges | 6274 |
| BSim: Targets above the self-significance bound | 5716 |
| BSim: Rejected: not mutual best | 342 |
| BSim: Rejected: second-best source name within margin | 102 |
| BSim: Rejected: second-best target within margin | 30 |
| BSim: Rejected: significance below threshold | 317 |
| BSim: Rejected: similarity below threshold | 4497 |
| BSim: Accepted BSim names | 428 |
| Vtable and BSim agree | 46 |
| Vtable and BSim disagree (vtable kept) | 2 |
| BSim names a vtable gives to another function (dropped) | 0 |
| BSim vtable-class check: 2.6 virtual of an unrelated class (dropped) | 1 |
| BSim call-graph check: all named callees agree | 65 |
| BSim call-graph check: no named callee | 302 |
| BSim call-graph check: no named callee agrees (dropped) | 7 |
| BSim call-graph check: some named callees agree | 5 |
| Final names from vtables | 1073 |
| Final names from BSim | 372 |
| Existing names that also occur in 2.6 (anchors) | 476 |
| Anchors agreeing with the vtable name | 258 |
| Anchors disagreeing with the vtable name | 0 |
| Anchors agreeing with the BSim name | 1 |
| Anchors disagreeing with the BSim name | 0 |
| Other existing names with a transferred name (kept) | 0 |

Changes:

- function renamed (GB26_VTABLE): 380
- function tag added: 436
- plate comment added: 436
- class namespace created: 15
- function renamed (GB26_BSIM): 56
- namespace created: 1

## Calibration

Anchors: existing names that occur in 2.6 plus vtable names, restricted to BSim targets (911); 111 have a best match that is mutual and passes both margins at similarity >= 0.5. `Agree` means the best source has the anchor's name. Cumulative columns count the bucket and all higher ones. `Checked` columns keep only the pairs that also pass the vtable-class and call-graph checks, as accepted names must.

By similarity (all significances | significance >= 30.0):

| Similarity | Pairs | Agree | Pairs (sig) | Agree (sig) | Cumulative (sig) | Cumulative rate (sig) | Checked (sig) | Agree checked (sig) | Cumulative rate checked (sig) |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| >= 0.99 | 13 | 12 | 10 | 10 | 10 | 100.000% | 10 | 10 | 100.000% |
| >= 0.95 | 3 | 3 | 3 | 3 | 13 | 100.000% | 2 | 2 | 100.000% |
| >= 0.9 | 8 | 8 | 8 | 8 | 21 | 100.000% | 7 | 7 | 100.000% |
| >= 0.85 | 7 | 7 | 7 | 7 | 28 | 100.000% | 5 | 5 | 100.000% |
| >= 0.8 | 6 | 6 | 6 | 6 | 34 | 100.000% | 4 | 4 | 100.000% |
| >= 0.75 | 6 | 6 | 6 | 6 | 40 | 100.000% | 4 | 4 | 100.000% |
| >= 0.7 | 14 | 10 | 9 | 7 | 49 | 95.918% | 4 | 4 | 100.000% |
| >= 0.6 | 23 | 22 | 20 | 19 | 69 | 95.652% | 14 | 13 | 98.000% |
| >= 0.5 | 31 | 30 | 23 | 23 | 92 | 96.739% | 17 | 17 | 98.507% |

Lowest similarity bucket with cumulative agreement >= 98.000% at significance >= 30.0: 0.75; checked pairs only: 0.5

By significance (similarity >= 0.5 | similarity >= 0.7):

| Significance | Pairs | Agree | Pairs (sim) | Agree (sim) |
|---|---:|---:|---:|---:|
| >= 100.0 | 17 | 16 | 10 | 10 |
| >= 60.0 | 32 | 31 | 19 | 18 |
| >= 40.0 | 27 | 27 | 14 | 14 |
| >= 30.0 | 16 | 15 | 6 | 5 |
| >= 20.0 | 11 | 10 | 5 | 4 |
| >= 15.0 | 5 | 3 | 2 | 1 |
| >= 0.0 | 3 | 2 | 1 | 0 |

## Calibration disagreements

7 in total:

- 0x006a4900 anchor `NiTimeController::OnPreDisplay`, best `NiTimeController::ComputeScaledTime` sim=0.681 sig=102.850; passes the checks
- 0x006a4fc0 anchor `NiTimeController::ProcessClone`, best `NiLODNode::UpdateWorldData` sim=1.000 sig=21.380; dropped by the checks: 2.6 virtual of `NiLODNode`, UOSA slot of 5 classes not derived from it (NiInterpController, NiPSysResetOnLoopCtlr, NiPSysUpdateCtlr, NiShaderTimeController, NiTimeController)
- 0x006d8780 anchor `NiDitherProperty::IsEqual`, best `NiLightColorController::IsEqual` sim=0.702 sig=18.397; dropped by the checks: 2.6 virtual of `NiLightColorController`, UOSA slot of 1 classes not derived from it (NiDitherProperty)
- 0x007d1d50 anchor `NiSingleInterpController::IsEqual`, best `NiFloatInterpolator::IsEqual` sim=0.716 sig=33.016; dropped by the checks: 2.6 virtual of `NiFloatInterpolator`, UOSA slot of 1 classes not derived from it (NiSingleInterpController)
- 0x0080a0d0 anchor `NiPSysResetOnLoopCtlr::Update`, best `NiPSResetOnLoopCtlr::Update` sim=0.505 sig=19.996; dropped by the checks: 2.6 virtual of `NiPSResetOnLoopCtlr`, UOSA slot of 1 classes not derived from it (NiPSysResetOnLoopCtlr)
- 0x00810830 anchor `NiObject::ProcessClone`, best `NiPSMeshParticleSystem::OnVisible` sim=0.720 sig=12.733; dropped by the checks: 2.6 virtual of `NiPSMeshParticleSystem`, UOSA slot of 1 classes not derived from it (NiMeshPSysData)
- 0x0081b7d0 anchor `NiPSysEmitterCtlrData::IsEqual`, best `NiTransformData::IsEqual` sim=0.730 sig=70.972; dropped by the checks: 2.6 virtual of `NiTransformData`, UOSA slot of 1 classes not derived from it (NiPSysEmitterCtlrData)

## Calibration agreements the checks would drop

28 in total; first 8:

- 0x00667630 `NiGeometry::ProcessClone` sim=0.821 sig=131.667: no named callee agrees
- 0x006715e0 `NiTexturingProperty::LinkObject` sim=0.621 sig=155.691: no named callee agrees
- 0x006719a0 `NiTexturingProperty::RegisterStreamables` sim=0.878 sig=88.253: no named callee agrees
- 0x00671a30 `NiTexturingProperty::SaveBinary` sim=0.694 sig=77.779: no named callee agrees
- 0x00671bf0 `NiTexturingProperty::IsEqual` sim=0.661 sig=98.117: no named callee agrees
- 0x00679ff0 `NiVertexColorProperty::LoadBinary` sim=0.544 sig=23.851: no named callee agrees
- 0x0067a120 `NiVertexColorProperty::IsEqual` sim=0.743 sig=37.139: no named callee agrees
- 0x0067df10 `NiZBufferProperty::LoadBinary` sim=0.521 sig=17.185: no named callee agrees

## Vtable and BSim disagreements

2 in total:

- 0x007d1d50 vtable `NiSingleInterpController::IsEqual` (vtable NiSingleInterpController slot 8), BSim `NiFloatInterpolator::IsEqual` (bsim sim=0.716 sig=33.016 NiAnimation26VC80R.dll 1003d520)
- 0x0081b7d0 vtable `NiPSysEmitterCtlrData::IsEqual` (vtable NiPSysEmitterCtlrData slot 8), BSim `NiTransformData::IsEqual` (bsim sim=0.730 sig=70.972 NiAnimation26VC80R.dll 10034790)

## BSim names dropped by the vtable-class check

1 in total:

- 0x006cbee0 `NiGPUProgramCache::NiGPUProgramDesc::`scalar_deleting_destructor'` (bsim sim=0.724 sig=31.277 NiMain26VC80R.dll 10024ad0): 2.6 virtual of `NiGPUProgramCache::NiGPUProgramDesc`, UOSA slot of 1 classes not derived from it (NiDepthStencilBuffer)

## BSim names dropped by the call-graph check

7 in total:

- 0x00676840 `NiNode::PurgeRendererData` (bsim sim=0.763 sig=47.648 NiMain26VC80R.dll 1004f840): 1 named callees, none called by the 2.6 function
- 0x00680250 `NiTriShapeData::IsEqual` (bsim sim=0.859 sig=133.262 NiMain26VC80R.dll 100daaa0): 1 named callees, none called by the 2.6 function
- 0x0068d5b0 `NiPixelData::ConvertToOppositeEndian` (bsim sim=0.756 sig=149.173 NiMain26VC80R.dll 10055230): 1 named callees, none called by the 2.6 function
- 0x006d1a80 `NiRenderedCubeMap::Create` (bsim sim=0.702 sig=75.331 NiMain26VC80R.dll 1005b920): 1 named callees, none called by the 2.6 function
- 0x00736b40 `NiVSMShadowTechnique::PreparePostProcessData` (bsim sim=0.818 sig=142.123 NiMain26VC80R.dll 100cdaf0): 1 named callees, none called by the 2.6 function
- 0x00752650 `NiD3DDefaultShader::IsCurrentCombineMode` (bsim sim=0.907 sig=39.116 NiDX9Renderer26VC80R.dll 10001f20): 1 named callees, none called by the 2.6 function
- 0x007688a0 `NiD3DShaderProgramFactory::RegisterShaderCreator` (bsim sim=0.776 sig=50.313 NiDX9Renderer26VC80R.dll 1001d980): 1 named callees, none called by the 2.6 function

## Notes

11 in total; first 8:

- 2.6 `NiTListBase<NiTPointerAllocator<unsigned_int>,unsigned_int>` has several vtables; the first is used
- 2.6 `NiTPointerListBase<NiTPointerAllocator<unsigned_int>,unsigned_int>` has several vtables; the first is used
- 2.6 `NiGPUProgramCache::NiGPUProgramDesc` has several vtables; the first is used
- 2.6 `NiGPUProgramCache::NiGPUProgramDesc` has several vtables; the first is used
- 2.6 `NiTFixedStringMap<NiFixedString>` has several vtables; the first is used
- 2.6 `NiTLargeArray<NiPointer<NiObject>,NiTNewInterface<NiPointer<NiObject>_>_>` has several vtables; the first is used
- 2.6 `NiTArray<NiFixedString,NiTNewInterface<NiFixedString>_>` has several vtables; the first is used
- 2.6 `NiTLargeArray<NiPointer<NiObject>,NiTNewInterface<NiPointer<NiObject>_>_>` has several vtables; the first is used


This run extended an earlier `--apply` (751 functions renamed from NiMain, NiDX9Renderer and NiSystem) with NiAnimation, NiParticle and NiCollision; status `unchanged` marks names from the earlier run. The vtable-class check now drops the earlier BSim name at `0x006cbee0` (`NiGPUProgramCache::NiGPUProgramDesc` destructor in a `NiDepthStencilBuffer` vtable); the script never removes names, so it stays in the program.
A second `--apply` run on the result reported `Changes: none` (1185 functions `unchanged`).
Full match list: `uosa_gamebryo_names.tsv` (UOSA address, name, 2.6 name, source tag, apply status, evidence, 2.6 prototype; then conflicts, rejected candidates and the per-class vtable mapping).
