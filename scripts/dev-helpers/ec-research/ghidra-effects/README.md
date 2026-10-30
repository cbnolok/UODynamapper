# ghidra-effects

Names the code in `UOSA.exe` (Enhanced Client) that decodes effect definitions. These are the binary
effect records that close every `tileart.uop` record and that make up the entries of
`EffectDefinitionCollection.uop`. The script also names the runtime effects that those definitions
instantiate, the effect-id lookup, the active-effect list and the 0x70/0xC0/0xC7 graphical-effect packet
chain that selects collection entries.

Findings: [results/uosa_effects.md](results/uosa_effects.md) has:

- the factory/key table;
- the wire format of every effect type, with field meanings;
- the data verification;
- how the format relates to `EffectDefinitionCollection.uop`;
- the run-time use of the collection: load path, packet keying, composite semantics, particle resource
  resolution, and tileart vs collection effects;
- a Rust parser specification and proposed Rust changes.

[results/uosa_chaos.md](results/uosa_chaos.md) covers the Chaos particle engine behind `.ems` effects
(`UOParticleEffectChaos`):

- the script language: lexer, classifier and grammar, with their quirks;
- the compiled structures;
- emission, spawn, integration and event/fade semantics;
- the RNG and the time source;
- quad and cel rendering and the render state;
- a proposed Rust simulator design.

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
  - labels `g_UOEffectDefinitionFactories`, `g_UOActiveEffects`, `g_UOParticleFilter`,
    `g_UOParticleLODLevel` and `g_UOUserEffectFilter`.
- New names may also go in the namespaces `UOObjectManager`, `UOPacket` and `UOWaypoints`, and in RTTI
  class namespaces of runtime effects.
- Its own single-line plate comments are refreshed when the evidence text changes; comments edited by hand
  are kept.
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
| RTTI vtables of `UOParticleEffectNif`, `UOParticleEffectChaos`, `UOHuedParticleEffectNif`, `UOSoundEffect`, `UOPositionOffset`, `UOEarthquakeEffect`, `UOEffectContainer`, `UOEffectContainerDefinition`, `UOEffectCollectionDefinition`, `UOProjectileEffect`, `UOTileEffect`, `UOObjectComponentEffects`, `BinaryUOEffectDefinitionCollection` | runtime effect slots |
| Only user of the string `Missing Effect Definition %d\n` | `UOEffects::InstantiateEffectById` |
| Its callee whose single callee makes the Instantiate (+8) virtual call; the address that callee takes, shared with two other functions | `InstantiateAndRegisterByValue`, `InstantiateAndRegister`, `g_UOActiveEffects`, `UpdateActiveEffects` (the one with an Update +0x14 call), `ReleaseActiveEffects` |
| Callers of the lookup: scalar 10001; callee of `UOProjectileEffect` +0x14; virtual call +0x30; the remaining one | `ShowTargetIndicator`, `UOProjectileEffect::SpawnExplosion`, `PlayEffectAtTile`, `UOWaypoints::SetWaypointEffect` (probable) |
| Single caller of PlayEffectAtTile; its callees that read a global it also reads first, or write the `UOProjectileEffectDefinition` / `UOTileEffectDefinition` vftables | `PlayGraphicalEffect`, `IsParticleFiltered` + `g_UOParticleFilter`, `MakeProjectileEffect`, `MakeTileEffect` |
| The three callers of PlayGraphicalEffect, ordered by the largest packet field offset read; their single callers; the call before the copier | `PlayGraphicalEffect_0x70/0xC0/0xC7`, `UOObjectManager::Receive_0x..._*`, `UOPacket::Read_0x..._*` |
| The single global read by the LOD and UserPreference Instantiates | `g_UOParticleLODLevel`, `g_UOUserEffectFilter` |
| Functions that write all the derived factory vtables | registration function (reported with its order, not renamed) |
| Chaos engine (step 8, `chaosEngine()`): RTTI vftables of `Emitter`, `ParticleEventSequence`, the 14 `ParticleEvent*` classes, `EmitterEffect`, `ParticleSet`, `ParticleTriShape`, `ParticleBillboard`; callees of their slots; the MT19937 constants 0x6c078965 / 0x5d588b65 and `__time64`; the callee of `CompileScript` that calls `_isspace`; unique parser strings; the "Can't compile emitter script for " caller chain; callees of `UOParticleEffectChaos::Init` and `UpdateActiveEffects` | 95 functions in `Emitter`, `EmitterGroup`, `ParticleEventSequence`, `Particle`, `ParticleEvent*`, `ChaosScript`, `MersenneTwister`, `ParticleSystemManager`, `EmitterEffect`, `ParticleSet`, `ParticleTriShape`, `ParticleBillboard`, `MeshParticle`, `EmitterParticle`, `UOEffects::LoadChaosEmitterEffect` (see [the table in uosa_chaos.md](results/uosa_chaos.md#names-and-types-applied-in-ghidra)) |

Vtable slot functions referenced from more tables than their class and base vtables (identical-code
folding) are skipped and reported. In this build that applies to 6 `UOParticleEffectNif` slots.

The report also lists other callers of the readers (not renamed):

- `FUN_0083c0ba`, an asset that embeds effect collections;
- `FUN_004b93d0`, which reads u32 id, u8 count and that many effects;
- `UOTileArt::ParseEffects`.

## effects_variables.spec

Applied by `../ApplyVariableSpecs.java` after `NameEffectCode.java` and the tileart spec (see
[the shared description](../README.md#variable-names-prototypes-and-structures-applyvariablespecsjava)):

```
run_ghidra_script(program="UOSA.exe", script_name="<abs path>/ApplyVariableSpecs.java",
                  args="<abs path>/ghidra-effects/effects_variables.spec <scratch>/effects_vars.md --apply")
```

- Adds `/UO/MsvcList` and the runtime effect types `/UO/UOEffectContainer`, `UOSoundEffect` and
  `UOEarthquakeEffect`, and makes 14 namespaces (the effect definitions and those runtime effects) classes
  whose `this` is their `/UOEffects` or `/UO` structure.
- Adds the Chaos types `/UO/ChaosEmitter`, `ChaosEventSequence`, `ChaosParticleEvent`, `ChaosParticle`,
  `ChaosRange`, `ChaosIntRange`, `ChaosVectorRange`, `ChaosCelSetup` and `ChaosToken`.
- Covers 201 functions (433 named parameters, 5 globals), 40 of them in the Chaos engine. Most Chaos
  helpers pass objects in registers, so their prototypes are custom. `UOEffects::ParseEffectDefinition` is
  in the tileart spec.
- The header of the spec lists the conventions: IUOEffect methods take an `ok` byte pointer first, factory
  `Create` keeps the factory in ECX and takes (out, blob, position, ok), a factory `Init` receives the
  definition shared_ptr as `definition` + `definition_refcount`.

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

## verify_chaos.py

This read-only reference implementation of the Chaos script compiler follows
[results/uosa_chaos.md](results/uosa_chaos.md). It reproduces the lexer, the classifier and the
compiler, including their quirks. It compiles every file that starts with `PARTICLESYSTEM` under the given
paths, for example the `.ems` payloads extracted from `EffectTexture.uop` to a scratch folder. It prints
aggregate counts:

- versions, keys and particle types;
- blend pairs and events;
- value forms and cel setups;
- quirk hits.

```
python3 -I verify_chaos.py <folder with extracted EffectTexture.uop entries> [--dump]
```

The script exits with code 1 and prints `FAIL <file>: <message>` for any script the client would reject.

