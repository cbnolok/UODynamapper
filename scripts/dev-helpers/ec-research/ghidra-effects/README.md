# ghidra-effects

Names the code in `UOSA.exe` (Enhanced Client) that decodes effect definitions. These are the binary
effect records that close every `tileart.uop` record and that make up the entries of
`EffectDefinitionCollection.uop`. The script also names the runtime effects that those definitions
instantiate.

Findings: [results/uosa_effects.md](results/uosa_effects.md) has:

- the factory/key table;
- the wire format of every effect type, with field meanings;
- the data verification;
- how the format relates to `EffectDefinitionCollection.uop`;
- a Rust parser specification.

## NameEffectCode.java

GhidraScript, category `ECResearch`.

```
NameEffectCode.java OUTPUT_FILE [--apply]
```

- `OUTPUT_FILE` is created with CREATE_NEW. If the file already exists, the run stops before making any change.
- Without `--apply`, the script only reports. With `--apply` it:
  - renames DEFAULT (`FUN_...`) functions with source ANALYSIS, in the RTTI class namespace or in `UOEffects`;
  - adds the tag `UO_EFFECTS`;
  - sets a plate comment `NameEffectCode: UO_EFFECTS <name> [(probable)]: <evidence>` when the comment is empty;
  - creates or updates the `/UOEffects` structure types (definition classes, wire records, MSVC containers);
  - labels `g_UOEffectDefinitionFactories`.
- Names with another source type are never changed; they are listed under "Kept names".
- A second `--apply` reports `Changes: none`.

Run it through ghidra-mcp:

```
run_ghidra_script(program="UOSA.exe", script_name="<abs path>/NameEffectCode.java",
                  args="<scratch>/effects_report.md --apply", timeout_seconds=1800)
```

### How functions are found

The script uses no fixed addresses and no address fallbacks. Each function is found from evidence that
doesn't depend on its address:

| Evidence | Finds |
|---|---|
| RTTI vtables (`.?AV<class>@@` -> type descriptor -> complete object locator -> vftable) of the ten `*BinaryFactory` classes | factory slots: CanHandle +0, Create +8, destructor +0x10, NewInstance +0x14, Init +0x18 |
| `CMP key, imm` (or a zeroed register) in CanHandle; constant returned by the definition's GetType (+0xc) | type key of each factory; a mismatch is reported as an anomaly |
| RTTI vtables of the ten definition classes | destructor +0, Instantiate +8, GetType +0xc |
| Small callee of the Particle Create that calls `_memcpy` | `UOAssets::StreamRead` (anchor only; named by UO_TILEART) |
| `PUSH [global]; CALL f` in the RoundRobin Create, where `f` makes virtual calls at +0 and +8 | `UOEffects::ParseEffectDefinition` and `g_UOEffectDefinitionFactories` |
| Common callee of the Wait, PropertyFlag, Random and UserPreference Creates that calls ParseEffectDefinition | `UOEffects::ParseEffectList` |
| Callee of the LOD Create that calls ParseEffectList | `UOEffects::ParseEffectMap` |
| RTTI `UOEffectDefinitionCollectionBinaryFactory` +4, then its callee that calls ParseEffectDefinition | collection Create, `UOEffects::ParseEffectDefinitionCollection`, `AddDefinition` (probable) |
| RTTI vtables of `UOParticleEffectNif`, `UOParticleEffectChaos`, `UOHuedParticleEffectNif`, `UOSoundEffect`, `UOPositionOffset`, `UOEarthquakeEffect`, `BinaryUOEffectDefinitionCollection` | runtime effect slots |
| Functions that write all the derived factory vtables | registration function (reported with its order, not renamed) |

Vtable slot functions referenced from more tables than their class and base vtables (identical-code
folding) are skipped and reported. In this build that applies to 5 `UOParticleEffectNif` slots.

The report also lists other callers of the readers (not renamed):

- `FUN_0083c0ba`, an asset that embeds effect collections;
- `FUN_004b93d0`, which reads u32 id, u8 count and that many effects;
- `UOTileArt::ParseEffects`.

## verify_effects.py

This read-only checker decodes every effect in `tileart.uop`, and optionally in
`EffectDefinitionCollection.uop`. It skips the known tileart record fields, then requires each record and
each collection entry to end exactly where its last effect ends. It prints:

- counts per type key;
- distributions of the field values;
- the particle names shared by both files.

```
C="<Enhanced Client folder>"
python3 -I verify_effects.py --tileart "$C/tileart.uop" \
    --collection "$C/EffectDefinitionCollection.uop" --strings "$C/string_dictionary.uop" [--dump]
```

The script exits with code 1 and prints `FORMAT ERROR: ...` on the first violation. A violation is an
unknown key, a read past the end of the data, trailing bytes, a bad tileart version, or a Wait/PropertyFlag
flag that is not 0 or 1.
