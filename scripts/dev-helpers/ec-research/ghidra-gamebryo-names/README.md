# Gamebryo 2.6 function names for Ghidra

`UOSA.exe` links Gamebryo 2.3 statically and has no symbols for it. The user-supplied
Gamebryo 2.6 SDK ships its DLLs with full PDBs. `TransferGamebryoNames.java` carries
function names from the 2.6 DLLs to the matching 2.3 code in two ways: by vtable slot
correspondence, and by BSim similarity calibrated on names the program already has. It
writes a report and, with `--apply`, renames the matched functions.

| Path | Content |
|---|---|
| `TransferGamebryoNames.java` | Ghidra script: report and match list, and with `--apply` renames |
| `results/uosa_gamebryo_names.md` | Report for the tested `UOSA.exe` (the `--apply` run) |
| `results/uosa_gamebryo_names.tsv` | Full match list for the same run |

The script expects the classes, `GetRTTI`s, `ms_RTTI` records and `vftable` labels from
[`../ghidra-gamebryo-classes`](../ghidra-gamebryo-classes) to be applied first.

## Preparing the 2.6 sources

Import each DLL into the same Ghidra project as `UOSA.exe`, in a `/gamebryo26` folder
(the default source paths), and analyze it with its PDB:

- DLL: `sdk/Win32/DLL/<Lib>26VC80R.dll`
- PDB: `sdk/Win32/Lib/vc80/ReleaseDLL/<Lib>26VC80R.pdb`

for `<Lib>` = `NiMain`, `NiDX9Renderer` and `NiSystem`. Set the PDB with
`PdbUniversalAnalyzer.setPdbFileOption(program, pdbFile)` (in a pre-script, or through
the analyzer's options in the GUI) before running auto-analysis. The script opens these
programs read-only and releases them at the end; it never modifies them.

## Method

**Vtables.** In 2.6, classes come from the `vftable` labels (demangled and mangled) and
parents from the `dynamic_initializer_for_'X::ms_RTTI''` functions, which push the base
class's `ms_RTTI` before calling `NiRTTI::NiRTTI`. In UOSA, classes and parents come from
the `ms_RTTI` records and `vftable` labels. A UOSA vtable ends at a non-code pointer or at
a slot with its own label or a non-read reference; dynamic `PTR_FUN_` labels from
devirtualized reads are ignored. For each class present in both versions, the chain from
the root is walked to the deepest ancestor `A` such that every class on root..`A` has the
same vtable length in both; slots `[0, L)`, with `L` the length of `A`, are mapped. A slot
with no unique 2.6 name (pure virtual, or several names, see below) is skipped. When one
UOSA function gets different names from different slots, it is a conflict and is not
named.

`NiObject` has no `vftable` label in UOSA. All vtables whose slot 2 is
`NiObject::GetRTTI` agree on their first slots, and those common slots are used as
NiObject's vtable (a "shared family").

**ICF folding.** The 2.6 linker folded identical functions, so one address carries many
labels (`NiObject::AddViewerStrings`, `NiObject::LinkObject`, ...). For a slot, the names
at the 2.6 target are intersected over all classes that inherit that slot from the same
introducing class, and the alias whose namespace is nearest in the class's 2.6 chain is
used. The prototype is taken from the chosen alias; the plate comment names the other
folded functions.

**BSim.** As in Ghidra's `LocalBSimQueryScript`: `medium_nosize` signatures, cosine
similarity with `VectorCompare`, and only vectors with self-significance above 15.
Sources are non-thunk 2.6 functions with exactly one name (folded ones are skipped);
targets are the UOSA functions in the Gamebryo ranges. All pairs are compared. A pair is
accepted only if:

- it is the best match for both functions,
- similarity and significance are at or above the thresholds,
- the next different source name for the target, and the next target for the source,
  are below the best by at least the margin,
- no near-identical source vector (similarity >= 0.97) carries another name.

**Call-graph check.** For an accepted BSim pair, the named callees of the UOSA function
(existing names and names from this run) are compared with the callees of the 2.6
function, including their folded aliases. If at least one callee is named and none of
them occurs in the 2.6 function, the pair is dropped.

**Merging.** Vtable names win over BSim names. A BSim name that a vtable gives to another
function is dropped. All disagreements between the two sources and with existing names
are listed.

**Calibration.** Existing names that also occur in 2.6 (the `GetRTTI`/`CreateObject`
names and other existing names) and the vtable names are anchors.
For each anchor that is a BSim target and whose best match passes the mutual-best and
margin tests at a low floor, the report tabulates how often the best source has the
anchor's name, by similarity and by significance bucket.

## Running

In the GUI, put this folder in the Script Manager's script directories and run
`TransferGamebryoNames.java` (category `ECResearch`). Arguments:

```text
OUTPUT_FILE [--apply] [--matches TSV_FILE] [--no-bsim] [--no-vtable]
            [--sources /path/a,/path/b] [--ranges hexstart-hexend,...] [--max-sources N]
            [--min-sim F] [--min-sig F] [--margin F]
```

Defaults: the three `/gamebryo26` DLLs, ranges `00650000-00830000,00aa0000-00ac0000`,
similarity 0.7, significance 30, margin 0.05. `--max-sources` limits the BSim sources for
quick tests. Through ghidra-mcp:

```text
run_ghidra_script(program="UOSA.exe",
  script_name="/absolute/repo/scripts/dev-helpers/ec-research/ghidra-gamebryo-names/TransferGamebryoNames.java",
  args="/tmp/uosa-gamebryo-names.md --matches /tmp/uosa-gamebryo-names.tsv")
```

Headless, on an existing analyzed project:

```sh
/path/to/ghidra/support/analyzeHeadless /path/to/project-dir PROJECT \
  -process UOSA.exe -noanalysis \
  -scriptPath /absolute/repo/scripts/dev-helpers/ec-research/ghidra-gamebryo-names \
  -postScript TransferGamebryoNames.java /tmp/uosa-gamebryo-names.md
```

Both outputs are created exclusively. A full run takes about 40 to 70 s, mostly BSim
signature generation. Without `--apply` the program is not changed; headless runs save it
unless `-readOnly` is given.

The match list has one row per transferred name: UOSA address, current name, 2.6 name,
source (`GB26_VTABLE` or `GB26_BSIM`), apply status, evidence and 2.6 prototype. Sections
after it (`# ...` lines) list vtable conflicts, slots whose target is not a function,
the per-class vtable mapping, disagreements, dropped BSim names and the rejected BSim
candidates above the similarity threshold with the reason.

## What `--apply` changes

- Only functions with a `DEFAULT` (`FUN_...`) name are renamed, to the 2.6 method name in
  the 2.6 namespace path. Missing namespaces are created (classes for classes). Names and
  namespaces use source type `ANALYSIS`.
- Tag `GB26_VTABLE` or `GB26_BSIM` on each renamed function.
- A plate comment, only if the function has none: `Gamebryo 2.6: <name>`, the 2.6
  prototype, and the evidence (vtable class and slot, or BSim similarity, significance
  and source address).

Prototypes are not applied: 2.3 and 2.6 signatures differ (for example `CreateObject`).
Functions that already have another name are kept and listed. A second `--apply` run
reports `Changes: none`.

## Limitations

- Only classes whose vtable lengths match along the whole chain are mapped; a class that
  gained or lost a virtual between 2.3 and 2.6 is mapped only up to the last matching
  ancestor (`NiTexture` and `NiSourceTexture` only up to `NiObjectNET`), and shader and
  material classes are not mapped at all.
- 469 mapped slots point at code Ghidra has no function for. They are listed but not
  named; creating functions is out of scope.
- Folded 2.6 functions cannot be BSim sources, and small functions fall below the
  self-significance bound, so short accessors are named only through vtables.
- BSim names are calibrated on 76 anchor pairs, biased toward vtable methods and
  hand-named texture-path code. Lower similarity buckets agreed fully too, but 0.7 is used
  to stay well inside the tested range. A wrong BSim name is still possible where 2.3 and
  2.6 code diverged; the evidence is in each plate comment.
- A 2.3 virtual can differ in meaning from the 2.6 virtual in the same slot even when the
  vtable lengths match. Slot 14 of `NiDX9PersistentSrcTextureRendererData` (`0x00772270`)
  is named `GetStreamableRTTIName`, which in 2.6 is
  `bool GetStreamableRTTIName(char *, unsigned int)`, but the 2.3 function is a getter
  returning an `NiRTTI *` (the base class's `ms_RTTI`). The name marks the slot; check the
  body before relying on the 2.6 prototype.

## Tested result

Ghidra 12.1.4, `UOSA.exe` SHA-256
`5a5e9dc536c836d1bf10b6f86bb88f61a6ae078f519d4060d3bc47aecbebad50`, with the three 2.6
DLLs above: 143 classes mapped by vtable (2272 slots), naming 572 functions with no
conflicts; BSim accepted 364 pairs, of which 40 agree with a vtable name, none disagree,
and 7 were dropped by the call-graph check, leaving 317 BSim names. `--apply` renamed 751
functions (435 vtable, 316 BSim; the rest had names already) and created 71 class
namespaces; a second run changed nothing. The committed report is a refresh run after the
classes script dropped the misidentified `GetRTTI` at `0x00772270`: it renamed that
function only, to `NiDX9PersistentSrcTextureRendererData::GetStreamableRTTIName`.
Calibration: 76 anchor pairs at significance >= 30 all agree, from similarity 0.5 up; the
only miss in the floor set is `NiTimeController::ProcessClone` at `0x006a4fc0` (best
`NiLODNode::UpdateWorldData`, similarity 1.0, significance 21.4), which the significance
threshold excludes. Of the 256 anchors, 137 agree with the vtable name and none disagree.

On the texture path, the vtables name `NiSourceTexture::LoadBinary`, `LinkObject`,
`PostLinkObject`, `SaveBinary`, `IsEqual` and `GetViewerStrings`, and BSim names
`NiFile::Seek`/`FileRead`/`FileWrite`/`GetFileSize`, `NiPath::ConvertToAbsolute`
(and agrees with the existing `NiPath::Standardize`), `NiPixelData::NiPixelData`,
`NiSourceTexture::CreateRendererData`, `NiDevImageConverter::GenerateMipmapLevels`,
`NiDX9SourceTextureData::CreateSurfFromRendererData` and
`NiDX9TextureData::InitializeFromD3DTexture`.
