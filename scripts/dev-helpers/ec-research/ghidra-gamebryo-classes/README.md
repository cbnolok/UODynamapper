# Gamebryo classes for Ghidra

`UOSA.exe` is built on Gamebryo. Ghidra's MSVC RTTI analyzer names only the few
classes compiled with C++ RTTI, but every class derived from `NiObject` (and a few other
Gamebryo roots) carries an engine `NiRTTI` record, and every streamable class registers a
loader with `NiStream`. `GamebryoClasses.java` recovers the class tree, vtables, loader
functions and object sizes from those, and can annotate the program with them.

| Path | Content |
|---|---|
| `GamebryoClasses.java` | Ghidra script: report, and with `--apply` annotations |
| `results/uosa_gamebryo_classes.md` | Report for the tested `UOSA.exe` |

## What it recovers

- **NiRTTI records**: 8-byte `{ const char *name; const NiRTTI *base; }` pairs at 4-byte
  alignment in initialized non-executable memory. The name must be a NUL-terminated C++
  identifier (`Outer::Inner` allowed) of bounded length, the base 0 or another candidate.
  A candidate becomes a class only if a `GetRTTI` returns it or it is an ancestor of one.
  Records with base 0 are roots.
- **GetRTTI**: the 6-byte virtual `MOV EAX, <record>; RET`, found by byte scan and
  accepted only when a data pointer refers to it.
- **Vtables**: a data pointer to `GetRTTI` at slot 2 (slots 0/1 are the `NiRefObject`
  destructor and `DeleteThis`), with code pointers in slots 0 and 1. If the slot is itself
  labeled or stored by code, the vtable starts there with `GetRTTI` at slot 0. A class
  without its own RTTI inherits its parent's `GetRTTI`, so one `GetRTTI` can be in several
  vtables. A single vtable is the class's `vftable`; several are disambiguated by an
  existing `<Class>::vftable` label (MSVC RTTI or user), then by the one vtable stored by
  the class's `CreateObject` or the constructors it calls; otherwise all are reported as
  ambiguous. The length ends at a non-code pointer, the next labeled or code-stored
  address, another detected vtable, or a maximum.
- **Loader registrations**: `PUSH <CreateObject>; PUSH <class name>; CALL`, found by byte
  scan so call sites in code Ghidra did not disassemble count too. The callee with the
  most such calls naming a class is `NiStream::RegisterLoader`; the callee with the most
  `PUSH <class name>; CALL` calls otherwise is `NiStream::UnregisterLoader`. Other calls
  to the register function are reported, not guessed.
- **Object sizes**: each `CreateObject` starts with `MOV EAX, <size>; CALL <allocator>`
  after a few prologue instructions, decoded with Ghidra's pseudo-disassembler so the
  listing is not changed. The most common first call is the allocator.
- **MSVC cross-check**: existing `vftable` labels are compared with the recovered
  vtables; MSVC classes without their own NiRTTI are listed with the `GetRTTI` they use.

## Running

In the GUI, put this folder in the Script Manager's script directories and run
`GamebryoClasses.java` (category `ECResearch`) with arguments `OUTPUT_FILE [--apply]`.
Through ghidra-mcp:

```text
run_ghidra_script(program="UOSA.exe",
  script_name="/absolute/repo/scripts/dev-helpers/ec-research/ghidra-gamebryo-classes/GamebryoClasses.java",
  args="/tmp/uosa-gamebryo-classes.md")
```

Headless, on an existing analyzed project:

```sh
/path/to/ghidra/support/analyzeHeadless /path/to/project-dir PROJECT \
  -process UOSA.exe -noanalysis \
  -scriptPath /absolute/repo/scripts/dev-helpers/ec-research/ghidra-gamebryo-classes \
  -postScript GamebryoClasses.java /tmp/uosa-gamebryo-classes.md
```

The output is created exclusively. Add `--apply` to annotate; headless runs save the
program unless `-readOnly` is given. Without `--apply` the program is not changed.

## What `--apply` changes

- Class namespaces per class (nested for `Outer::Inner`), reusing existing ones.
- Labels `<Class>::ms_RTTI` on records, typed as `/Gamebryo/NiRTTI`
  `{ char *name; NiRTTI *base; }`. Only undefined bytes or plain pointers are replaced.
- Function names `<Class>::GetRTTI`, `<Class>::CreateObject`,
  `NiStream::RegisterLoader`, `NiStream::UnregisterLoader` and
  `NiMemObject::operator_new`. Functions are created where missing.
- Labels `<Class>::vftable` for unambiguous vtables that have no MSVC or user label.
- One structure per class in `/Gamebryo`. With a known size it has that size and field 0
  is the nearest ancestor with a known size (`base`, commented with the direct parent if
  different), or `void *vftable` when no ancestor size is known. Without a known size only
  field 0 is set, and the description gives the bounds: the nearest sized ancestor and the
  smallest sized descendant.

New names use source type `ANALYSIS`. Only `DEFAULT` and `ANALYSIS` names are renamed, so
user and imported names are kept and reported. Existing structures with the same path are
never modified; the report lists them as kept and flags a size different from the
recovered one, so fields added by hand survive later runs. A second `--apply` run reports
zero changes.

## Limitations

- Sizes are known only for streamable classes, which have a `CreateObject`. Abstract and
  non-streamable classes get bounds instead.
- Classes without their own RTTI share their parent's `GetRTTI`; their vtables stay
  unassigned unless a label or `CreateObject` decides, and are listed as ambiguous.
- `NiStream::RegisterLoader`, `NiStream::UnregisterLoader` and especially
  `NiMemObject::operator_new` are inferred names (the allocator takes its size in `EAX`;
  it may be Gamebryo's `NiMalloc`/`NiNew` path rather than a literal `operator new`).
- Vtable lengths are heuristic: a code-stored address or a label inside a vtable cuts it
  short, and an adjacent unlabeled table can extend it.
- Only virtual functions reached through these patterns are covered; non-virtual member
  functions, constructors and field layouts beyond field 0 are not recovered.

## Tested result

Ghidra 12.1.4, `UOSA.exe` SHA-256
`5a5e9dc536c836d1bf10b6f86bb88f61a6ae078f519d4060d3bc47aecbebad50`: 343 classes (60 with
non-`Ni` names) under 15 roots (`NiObject` plus `NiRefObject`-derived roots such as
`NiShader`, `NiMaterial`, `NiRenderView`), 340 vftables, 181 loader registrations (59 in
code Ghidra had not disassembled) and 171 object sizes; `NiSourceTexture` is `0x44`
bytes with vftable `0x00c9f774` (24 slots), `NiPixelData` `0x70`. The recovered vtables
agree with all MSVC RTTI `vftable` labels. The report also lists the three ambiguous
`GetRTTI`s, the loader aliases (old names such as `NiKeyframeController` registered with
a current class's `CreateObject`) and the two register calls without the push pattern.
