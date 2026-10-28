# Gamebryo 2.6 layouts for Ghidra

`GamebryoClasses.java` (sibling folder `ghidra-gamebryo-classes/`) creates one placeholder
structure per Gamebryo class in `UOSA.exe`, with the object size where `CreateObject` proves it
and only field 0 defined. `UOSA.exe` links Gamebryo 2.3 statically; no 2.3 symbols exist, but the
Gamebryo 2.6 SDK ships its DLLs with full PDBs. `ApplyGamebryoLayouts.java` copies the 2.6 member
layouts into those placeholders, only where the recovered sizes say the 2.6 layout fits.

| Path | Content |
|---|---|
| `ApplyGamebryoLayouts.java` | Ghidra script: report, and with `--apply` fill the structures |
| `results/uosa_gamebryo_layouts.md` | Report for the tested `UOSA.exe`: counts, class lists with sizes |
| `results/uosa_gamebryo_layouts_members.md` | The applied member layouts and the copied 2.6 types |

## Sources

The 2.6 SDK is not part of this repository; supply your own copy. Import into the same Ghidra
project, in the folder `/gamebryo26`, the release DLLs

- `sdk/Win32/DLL/NiMain26VC80R.dll`
- `sdk/Win32/DLL/NiDX9Renderer26VC80R.dll`
- `sdk/Win32/DLL/NiSystem26VC80R.dll`
- `sdk/Win32/DLL/NiAnimation26VC80R.dll`
- `sdk/Win32/DLL/NiParticle26VC80R.dll`
- `sdk/Win32/DLL/NiCollision26VC80R.dll`

and analyze each with its PDB `sdk/Win32/Lib/vc80/ReleaseDLL/<Lib>26VC80R.pdb`: in the GUI pick
the file in the analysis options of *PDB Universal*; headless, call
`PdbUniversalAnalyzer.setPdbFileOption(currentProgram, new File(pdbPath))` in a pre-script before
auto-analysis. Every program in `/gamebryo26` is used, so more libraries can be added the same
way. These six are the ones `UOSA.exe` names in its `Gamebryo Version 2.3.0.0` strings; it has no
such string for NiAnimationCompression or NiPhysXParticle. The source programs are opened
read-only.

Ghidra 12's PDB loader produces one flattened structure per class, e.g.
`/NiMain26VC80R.pdb/NiSourceTexture` (0x44): the full object size, the inherited regions (vptr,
base-class members) as `int _padding_` fields, then the class's own members at their offsets.
Nested types live in a category named after the class (`/NiMain26VC80R.pdb/NiTexture/FormatPrefs`).
There are no separate base-class, `_internal`, vftable or vbtable structures, so the own members
are the non-`_padding_` fields and the class's base layout comes from the `/Gamebryo` parent.

## Rules

Classes are read from the `/Gamebryo` structure descriptions that `GamebryoClasses.java` writes
(parent, exact size or bounds `>= ancestor, <= smallest sized descendant`).

- **2.6 type**: the structure of the same (nested) name in a `*.pdb` category. Every PDB that uses
  a class carries its type, so the DLL defining `<Class>::ms_RTTI` is preferred; all sources must
  agree on the layout, otherwise the class is rejected.
- **Exact**: a class with a `CreateObject` size gets the layout if the 2.6 size is equal.
- **Inferred**: a class without a size gets the layout if the 2.6 size lies within its bounds, at
  least one sized descendant with a 2.6 type has an equal size (if any has a 2.6 type), and no
  such fitting descendant has 2.6 own members inside the inferred size. Its description says
  `size inferred`.
- **No size evidence**: a class with neither a size nor a sized descendant has only a lower bound,
  which proves nothing; it is reported and not filled (`APPLY_UNBOUNDED` in the script changes
  that).
- **Consistency**: rejected if a UOSA ancestor's 2.6 size does not fit that ancestor, or the
  class's 2.6 own members start inside an ancestor's 2.6 or UOSA size. Size mismatches and
  rejections are listed with both sizes.
- **Re-check**: a layout an earlier run applied with an inferred size is evaluated again against
  the current sources (sized descendants that fit, and those of equal size). The report lists
  each one as consistent or contradicted; a contradicted layout is reported, not removed.
- **Excluded**: classes in the script's `EXCLUDED` map (class -> reason) are never filled, because
  the 2.3 code contradicts their 2.6 layout although the sizes fit. With `--apply`, a structure
  still exactly as this script filled it (same members, layout marker in the description) is
  restored to the `GamebryoClasses.java` placeholder; one edited since is reported and kept. The
  copied `/Gamebryo26` types it referenced are removed when this script created them (its
  description prefix) and nothing references them any more: no other type, defined data, function
  signature or variable. Categories left empty go too.

## Representation

- Field 0 is `base`, the `/Gamebryo` structure of the nearest ancestor that received a layout
  (the direct parent, or commented `Direct parent: ...` when that one has none). The bytes
  between that base and the class's own members stay undefined.
- A root (no parent) whose 2.6 own members start after `NiRefObject` gets `void *vftable` at 0
  and the `NiRefObject` members (`m_uiRefCount` at 4). Every class the sibling script finds
  derives from `NiRefObject`, since its vtable detection relies on that.
- Own members keep their 2.6 names and offsets; the description gets
  `; layout from Gamebryo 2.6 <DLL> <PDB type path>`.
- Member types are copied to `/Gamebryo26`, mirroring the PDB categories
  (`/NiMain26VC80R.pdb/NiTexture/FormatPrefs` becomes `/Gamebryo26/NiTexture/FormatPrefs`).
  Only types embedded by value are copied with members (structures, unions, enums, typedefs, and
  what they embed in turn). Pointers to Gamebryo classes point at the `/Gamebryo` structure;
  pointers to other composites point at an empty stub of the same name; function pointers become
  `void *`. Inherited regions of copied types stay undefined. Existing types at the same path are
  reused, never replaced.

Only placeholders exactly as `GamebryoClasses.java` created them (one field 0 `vftable` or
`base`, original description, matching length) are filled. Edited structures are kept and
reported; apart from excluded classes (see Rules), nothing is deleted. The layout marker in the description makes a second `--apply` run
change nothing.

## Running

Run `GamebryoClasses.java --apply` first. In the GUI, put this folder in the Script Manager's
script directories and run `ApplyGamebryoLayouts.java` (category `ECResearch`) with arguments
`OUTPUT_FILE [--apply]`. Through ghidra-mcp:

```text
run_ghidra_script(program="UOSA.exe",
  script_name="/absolute/repo/scripts/dev-helpers/ec-research/ghidra-gamebryo-layouts/ApplyGamebryoLayouts.java",
  args="/tmp/uosa-gamebryo-layouts.md --apply")
```

Headless, on the analyzed project:

```sh
/path/to/ghidra/support/analyzeHeadless /path/to/project-dir PROJECT \
  -process UOSA.exe -noanalysis \
  -scriptPath /absolute/repo/scripts/dev-helpers/ec-research/ghidra-gamebryo-layouts \
  -postScript ApplyGamebryoLayouts.java /tmp/uosa-gamebryo-layouts.md --apply
```

The report goes to `OUTPUT_FILE` and the member layouts to `OUTPUT_FILE` with `_members` before
the extension; both are created exclusively. Without `--apply` the program is not changed and the
members file shows the planned layouts.

## Limitations

- Equal sizes do not prove equal members: a 2.3 member can be renamed, retyped, reordered or sit
  in what is padding in 2.6. `NiSourceTexture` is an example: 0x44 in both, but UOSA's
  `CreateObject` (constructor inlined) stores a byte from a static at +0x43, which is tail padding
  in 2.6 and stays undefined. Verify an applied layout against the constructor stores (usually
  inlined in `CreateObject`), the `LoadBinary`/`SaveBinary` field order and the accessors before
  relying on a member name.
- The NiRTTI parent is not always the C++ parent: a base without its own NiRTTI is invisible to
  the sibling script. `NiPersistentSrcTextureRendererData` derives from `NiTexture::RendererData`
  in 2.6, so its bytes 0x08..0x58 stay undefined after the `NiObject` base.
- `NiControllerSequence` is excluded although its size is 0x7c in both versions: the UOSA
  constructor (`0x00799810`, called from `CreateObject` `0x0079d1f0`) stores an embedded array
  object (vptr, data pointer, 16-bit counts, grow-by 1) at +0x48, where 2.6 has the float
  `m_fWeightedLastTime`, and 2.3 has no `NiSequenceData`, `NiPoseBuffer` or `NiScratchPad` (no
  strings in the binary) that the 2.6 members point to. It stays a placeholder.
- Copied by-value types (`NiPixelFormat`, `NiTransform`, ...) are 2.6 layouts checked only by the
  size of the class embedding them.
- Only classes of the imported libraries can be filled; UO/Mythic classes and classes from
  Gamebryo libraries not imported are reported as having no 2.6 type.
- There is no undo besides Ghidra's own. Besides the restore of excluded classes and the removal
  of their unreferenced copies, the script never deletes or replaces existing types.

## Tested result

Ghidra 12.1.4, `UOSA.exe` SHA-256
`5a5e9dc536c836d1bf10b6f86bb88f61a6ae078f519d4060d3bc47aecbebad50`, sources NiMain,
NiDX9Renderer, NiSystem, NiAnimation, NiParticle and NiCollision 2.6 (VC80 release DLLs): of 343
classes, 117 filled with exact size and 25 inferred (`NiObject` 0x8, `NiObjectNET` 0x18,
`NiTexture` 0x30, `NiTimeController` 0x34, `NiInterpolator` 0xc, `NiPSysModifier` 0x18, ...);
50 size mismatches (e.g. `NiNode` 0xe0 vs 0xdc, the geometry data classes by 8,
`NiParticleSystem` 0x134 vs 0x124, `NiControllerManager` 0x6c vs 0xa0); 9 rejected (`NiAVObject`
and its geometry/light/effect bases, `NiPSysCollider`, `NiPSysFieldModifier`: no sized descendant
fits); 74 without size evidence (renderer, shader, material and render-view classes,
`NiCollisionData`); 67 without a 2.6 type (UO/Mythic classes and 2.3 classes dropped by 2.6, such
as `NiFloatController`, `NiPathController`, `NiLookAtController`); `NiControllerSequence`
excluded. The six-source run extended an earlier one with three sources (38 exact, 9 inferred):
it filled 96 more structures and copied 47 types (26 structures, 8 enums, 13 stubs), and the 9
earlier inferred layouts all stay consistent. The committed report is the following run, which
restored `NiControllerSequence` to its placeholder and removed the 11 copied types only it used
(`NiAnimState`, `NiTimeController/CycleType`, six `NiPointer<...>`, the
`NiTPrimitiveArray<NiSequenceData::ActivationCallback_*>` and the `NiPoseBuffer`,
`NiScratchPad`, `NiSequenceData` stubs) with the emptied category `/Gamebryo26/NiTimeController`.
`NiTimeController` (0x34) is confirmed by `NiPSysUpdateCtlr`, exactly 0x34 with no own members;
33 of its 37 sized descendants fit. A second `--apply` run reports zero changes.

The texture path checks out against the binary: the inlined `NiTexture` constructor at
`0x00430960` writes the `NiRefObject` vptr and count, clears `NiObjectNET`'s name, controllers,
extra-data pointer and the two 16-bit counts at 0x08..0x16, then stores 6, 3, 2 at 0x18..0x20
(`PIX_DEFAULT`, `ALPHA_DEFAULT`, `MIP_DEFAULT` of the copied `FormatPrefs` enums) and clears
`m_pkRendererData` at 0x24. `NiSourceTexture::CreateObject` (`0x0068abb0`) clears 0x30..0x3c
(the two `NiFixedString` handles and two `NiPointer`s) and sets the bools at 0x40 (`m_bStatic`,
1 as in 2.6), 0x41 and 0x42.

The animation path checks out as well: the `NiFlipController` constructor (`0x0079fbe0`)
initializes an embedded array at 0x38 and clears 0x48..0x50 (`m_kTextures`, `m_uiIndex`,
`m_uiAffectedMap`, `m_pcCtlrID`); `NiTextureTransformController` (`0x007b15d0`) writes 0x38..0x4c
(`m_uiLastIdx`, `m_pkMap`, `m_bShaderMap`, `m_uiMapIndex`, `m_eMember`, `m_pcCtlrID`);
`NiTransformInterpolator` (`0x007b2110`) sets `m_fLastTime` at 0x8, the `NiQuatTransform` at
0x0c..0x28, `m_spData` at 0x2c (reference count incremented) and the three key indices at
0x30..0x38.
