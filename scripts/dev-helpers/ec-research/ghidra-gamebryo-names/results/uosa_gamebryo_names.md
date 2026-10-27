# Gamebryo 2.6 names in UOSA

Program: UOSA.exe
SHA-256: 5a5e9dc536c836d1bf10b6f86bb88f61a6ae078f519d4060d3bc47aecbebad50
Mode: analysis and --apply
Sources: `NiMain26VC80R.dll` (5786 functions), `NiDX9Renderer26VC80R.dll` (2568 functions), `NiSystem26VC80R.dll` (624 functions)
BSim target ranges: 0x00650000-0x00830000, 0x00aa0000-0x00ac0000
Thresholds: similarity >= 0.7, significance >= 30.0, margin 0.05, self-significance > 15.0, duplicate-source similarity 0.97

| Item | Count |
|---|---:|
| Vtable: 2.6 classes with a vtable | 323 |
| Vtable: 2.6 classes with NiRTTI parent data | 173 |
| Vtable: UOSA classes with a vtable | 348 |
| Vtable: UOSA shared vtable families (common slots only) | 1 |
| Vtable: Classes mapped (equal chain lengths) | 143 |
| Vtable: Mapped slots | 2272 |
| Vtable: Slots without a unique 2.6 method name (folded or pure) | 322 |
| Vtable: Slots whose UOSA target is not a function | 469 |
| Vtable: Slots whose UOSA target is also in a non-Gamebryo vtable | 1 |
| Vtable: UOSA functions named by vtables | 572 |
| Vtable: UOSA functions with conflicting vtable names | 0 |
| BSim: Source functions with one 2.6 name (not thunks/imports) | 6775 |
| BSim: Source functions skipped as ICF-folded (several names) | 1051 |
| BSim: Sources above the self-significance bound | 4880 |
| BSim: UOSA functions in the ranges | 6255 |
| BSim: Targets above the self-significance bound | 5702 |
| BSim: Rejected: not mutual best | 260 |
| BSim: Rejected: second-best source name within margin | 73 |
| BSim: Rejected: second-best target within margin | 24 |
| BSim: Rejected: significance below threshold | 251 |
| BSim: Rejected: similarity below threshold | 4730 |
| BSim: Accepted BSim names | 364 |
| Vtable and BSim agree | 40 |
| Vtable and BSim disagree (vtable kept) | 0 |
| BSim names a vtable gives to another function (dropped) | 0 |
| BSim call-graph check: all named callees agree | 55 |
| BSim call-graph check: no named callee | 257 |
| BSim call-graph check: no named callee agrees (dropped) | 7 |
| BSim call-graph check: some named callees agree | 5 |
| Final names from vtables | 572 |
| Final names from BSim | 317 |
| Existing names that also occur in 2.6 (anchors) | 257 |
| Anchors agreeing with the vtable name | 137 |
| Anchors disagreeing with the vtable name | 1 |
| Anchors agreeing with the BSim name | 1 |
| Anchors disagreeing with the BSim name | 0 |
| Other existing names with a transferred name (kept) | 0 |

Changes:

- function tag created: 2
- function renamed (GB26_VTABLE): 434
- function tag added: 750
- plate comment added: 750
- function renamed (GB26_BSIM): 316
- class namespace created: 71

## Calibration

Anchors: existing names that occur in 2.6 plus vtable names, restricted to BSim targets (439); 91 have a best match that is mutual and passes both margins at similarity >= 0.5. `Agree` means the best source has the anchor's name. Cumulative columns count the bucket and all higher ones.

By similarity (all significances | significance >= 30.0):

| Similarity | Pairs | Agree | Pairs (sig) | Agree (sig) | Cumulative (sig) | Cumulative rate (sig) |
|---|---:|---:|---:|---:|---:|---:|
| >= 0.99 | 13 | 12 | 10 | 10 | 10 | 100.000% |
| >= 0.95 | 1 | 1 | 1 | 1 | 11 | 100.000% |
| >= 0.9 | 7 | 7 | 7 | 7 | 18 | 100.000% |
| >= 0.85 | 7 | 7 | 7 | 7 | 25 | 100.000% |
| >= 0.8 | 4 | 4 | 4 | 4 | 29 | 100.000% |
| >= 0.75 | 5 | 5 | 5 | 5 | 34 | 100.000% |
| >= 0.7 | 10 | 10 | 7 | 7 | 41 | 100.000% |
| >= 0.6 | 20 | 20 | 17 | 17 | 58 | 100.000% |
| >= 0.5 | 24 | 24 | 18 | 18 | 76 | 100.000% |

Lowest similarity bucket with cumulative agreement >= 98.000% at significance >= 30.0: 0.5

By significance (similarity >= 0.5 | similarity >= 0.7):

| Significance | Pairs | Agree | Pairs (sim) | Agree (sim) |
|---|---:|---:|---:|---:|
| >= 100.0 | 13 | 13 | 8 | 8 |
| >= 60.0 | 26 | 26 | 16 | 16 |
| >= 40.0 | 23 | 23 | 12 | 12 |
| >= 30.0 | 14 | 14 | 5 | 5 |
| >= 20.0 | 10 | 9 | 5 | 4 |
| >= 15.0 | 3 | 3 | 1 | 1 |
| >= 0.0 | 2 | 2 | 0 | 0 |

## Calibration disagreements

1 in total:

- 0x006a4fc0 anchor `NiTimeController::ProcessClone`, best `NiLODNode::UpdateWorldData` sim=1.000 sig=21.380

## BSim names dropped by the call-graph check

7 in total:

- 0x00676840 `NiNode::PurgeRendererData` (bsim sim=0.763 sig=47.648 NiMain26VC80R.dll 1004f840): 1 named callees, none called by the 2.6 function
- 0x00680250 `NiTriShapeData::IsEqual` (bsim sim=0.859 sig=133.262 NiMain26VC80R.dll 100daaa0): 1 named callees, none called by the 2.6 function
- 0x0068d5b0 `NiPixelData::ConvertToOppositeEndian` (bsim sim=0.756 sig=149.173 NiMain26VC80R.dll 10055230): 1 named callees, none called by the 2.6 function
- 0x006d1a80 `NiRenderedCubeMap::Create` (bsim sim=0.702 sig=75.331 NiMain26VC80R.dll 1005b920): 1 named callees, none called by the 2.6 function
- 0x00736b40 `NiVSMShadowTechnique::PreparePostProcessData` (bsim sim=0.818 sig=142.123 NiMain26VC80R.dll 100cdaf0): 1 named callees, none called by the 2.6 function
- 0x00752650 `NiD3DDefaultShader::IsCurrentCombineMode` (bsim sim=0.907 sig=39.116 NiDX9Renderer26VC80R.dll 10001f20): 1 named callees, none called by the 2.6 function
- 0x007688a0 `NiD3DShaderProgramFactory::RegisterShaderCreator` (bsim sim=0.776 sig=50.313 NiDX9Renderer26VC80R.dll 1001d980): 1 named callees, none called by the 2.6 function

## Anchor disagreements (vtable)

1 in total:

- 0x00772270 `NiPersistentSrcTextureRendererData::GetRTTI` vs `NiDX9PersistentSrcTextureRendererData::GetStreamableRTTIName` (vtable NiDX9PersistentSrcTextureRendererData slot 14)

## Notes

6 in total:

- 2.6 `NiGPUProgramCache::NiGPUProgramDesc` has several vtables; the first is used
- 2.6 `NiGPUProgramCache::NiGPUProgramDesc` has several vtables; the first is used
- 2.6 `NiTFixedStringMap<NiFixedString>` has several vtables; the first is used
- UOSA `NiGPUProgramCache`: 2 vtables with its GetRTTI disagree; not mapped
- UOSA `NiShaderDeclaration`: 3 vtables with its GetRTTI disagree; not mapped
- `NiTriShapeData` vtable is shorter than its ancestor `NiObject`'s; not mapped


A second `--apply` run on the result reported `Changes: none` (750 functions `unchanged`).
Full match list: `uosa_gamebryo_names.tsv` (UOSA address, name, 2.6 name, source tag, apply status, evidence, 2.6 prototype; then conflicts, rejected candidates and the per-class vtable mapping).
