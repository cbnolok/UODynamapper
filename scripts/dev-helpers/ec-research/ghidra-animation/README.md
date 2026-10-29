# ghidra-animation

Names the code in `UOSA.exe` (Enhanced Client) that loads and plays mobile and item animations:

- the `AnimationSequence.uop` and `AnimationDefinition.uop` asset factories and parsers;
- the sequence action table, the group lookups (`FrameSetLookup`) and the frame mapping helpers;
- the mobile and tile animation components;
- the 0x6E / 0xE2 animation packet chain;
- the animation settings (`idleAnimation`, `frameSetRestriction`, `cacheSize`, `flying_animations`).

Findings: [results/uosa_animation.md](results/uosa_animation.md) has:

- the record layout of both files as the client reads them, field by field;
- the data verification;
- the run-time semantics: action resolution, directions and mirroring, frame numbering, timing, groups,
  packets and settings;
- how the files relate to `AnimationFrame*.uop`;
- the mismatches with the repo parsers and the proposed Rust changes.

## NameAnimationCode.java

GhidraScript, category `ECResearch`.

```
NameAnimationCode.java OUTPUT_FILE [--apply]
```

- `OUTPUT_FILE` is created with CREATE_NEW. If the file already exists, the run stops before making any change.
- Without `--apply`, the script only reports. With `--apply` it:
  - renames DEFAULT (`FUN_...`) functions with source ANALYSIS, in the RTTI class namespace or in
    `UOAnimations`;
  - adds the tag `UO_ANIMATION`;
  - sets a plate comment `NameAnimationCode: UO_ANIMATION <name> [(probable)]: <evidence>` when the comment
    is empty;
  - labels the settings globals `g_UOSettingIdleAnimation`, `g_UOSettingFrameSetRestriction`,
    `g_UOSettingCacheSize`, `g_UOSettingFlyingAnimations` and their run-time copies `g_UOIdleAnimation`,
    `g_UOFrameSetRestriction`, `g_UOFlyingAnimations`.
- New names may also go in the namespaces `UOObjectManager`, `UOPacket`, `UOSettings`, `FrameSetLookup`,
  `UOProjectileEffect` and `UOTileEffect`.
- Its own single-line plate comments are refreshed when the evidence text changes; comments edited by hand
  are kept.
- Names with another source type are never changed; they are listed under "Kept names".
- A second `--apply` reports `Changes: none`.

The script relies on names applied by the tileart and effects scripts (`UOAssets::GetAsset`,
`g_UOAssetManager`, `UOAssets::StreamRead`, `UOEffects::ParseEffectDefinitionCollection`,
`UOProjectileEffect::Init`, `UOTileEffect::Init`, the `Receive_0x70/0xC0/0xC7` handlers). Run those first.

Run it through ghidra-mcp:

```
run_ghidra_script(program="UOSA.exe", script_name="<abs path>/NameAnimationCode.java",
                  args="<scratch>/animation_report.md --apply", timeout_seconds=1800)
```

### How functions are found

The script uses no fixed function addresses. Each function is found from evidence that doesn't depend on
its address:

| Evidence | Finds |
|---|---|
| RTTI vtables of `UOAnimationSequenceBinaryFactory` and `UOAnimationDefinitionBinaryFactory` | factory Create (+4); definition ReadFrame (+0x10) |
| Create callee that calls `UOEffects::ParseEffectDefinitionCollection` / that calls the constructor writing the `UOAnimationDefinition` vftable | `UOAnimationSequence::Parse`, `UOAnimationDefinition::Parse` |
| Constructors writing the `UOAnimationSequence`, `UOAnimationDefinition`, `UOAnimationFrameSet` and `FrameSetLookup` vftables | the four constructors |
| First two Parse callees first called after the sequence constructor; their shared first callee; the callee only AddAction has | `AddAction`, `AddAliasAction`, `HasAction`, `NextBaseFrame`; `FindAction` (other user of the map find) |
| Parse callees after ParseEffectDefinitionCollection; the map operator[] they share; the group map helpers around the FrameSetLookup constructor | `AddFrameEffects`, `AddSkippedFrame`, `ActionAt`, `GetGroup`, `GetOrCreateGroup`, `HasGroup` |
| Callers of FindAction that read `g_UOIdleAnimation` (GetDirectionFrames) or `g_UOFlyingAnimations` (the pair where one calls the other: ResolveAction, GetActionFps); the other ResolveAction caller; the remaining GetDirectionFrames callee | `GetDirectionFrames`, `ResolveAction`, `GetActionFps`, `GetFrameCount`, `IsStandAction` |
| IsStandAction's single-caller callee; the other caller of its map find | `FrameSetLookup::IsStandAction`, `FrameSetLookup::GetMoveAction` |
| Field displacements +0x48/+0x68/+0x88/+0xa8 indexed by direction | the four offset getters |
| ParseFrameSets callees: the one with two callers that also call the definition constructor; the helpers sharing its first callee; the writer of +0x38 | `AddFrameSet`, `GetFrameSet`, `SetStartFrameSets`, `SetNext`, `GetSharedTiming`, `SetFrameInterval` |
| Users of `g_UOAssetManager` with the scalar `type << 24` for 0x0c / 0x0b, and for 0x10 when they also call `UOAssets::GetAsset` | `GetAnimationSequence`, `GetAnimationDefinition`, `GetAnimationFrameSet` |
| Single caller of GetDirectionFrames | `BuildMobileAnimation` |
| Builder callee using `0x0c << 24`; builder callee calling GetSharedTiming; the other caller of that request function (must call HasGroup); the caller of the shared vector helper using `0x0b << 24` | `RequestAnimationSequence`, `SetFrameInterval`, `HasSeatedGroup`, `RequestAnimationDefinition` |
| RequestAnimationDefinition callers, classified by `UOProjectileEffect::Init`, `UOTileEffect::Init` or the tile component Initialize calling them | the three `LoadAnimationDefinition` functions |
| RTTI vtables of `UOObjectComponentAnimationMob` / `UOObjectComponentAnimationTile` (+4, +0x18, +0x5c) | component Initialize, Update, PlayAction |
| Users of the `idleAnimation`, `frameSetRestriction`, `cacheSize`, `flying_animations` strings; the 8-instruction copy window in the packet registration function | settings parser and the seven globals |
| Packet registration: `MOV [ESP+0x14], id; CALL register` paired with the preceding handler reference, confirmed by existing `Receive_` names | `Receive_0x6E`, `Receive_0xE2`, their `Read_` parsers and the component handlers |
| OnNewCharacterAnimation callee that calls HasGroup; the caller of GetTransitionAction that calls it | `SelectAnimationGroup`, `PlayTransitionAction` |
| The two readers of `g_UOFrameSetRestriction` (besides the copier): the one whose caller calls exactly it and GetGroup, and the other | `GetTypedAction` + `MapCharacterAnimationAction`, `GetTransitionAction` |
| Only user of the string `, mounted`; the caller of NextBaseFrame's map iterator step that UpdateFrame calls | `UpdateFrame`, `FrameToAction` |

## verify_animation.py

Read-only checker that decodes every entry of `AnimationSequence.uop` and `AnimationDefinition.uop` as the
client does and requires each entry to end exactly where its last field ends. It prints value
distributions per field, the entries whose stored id differs from their name, and the alias, group and
typed-action consistency checks. With `--frames` it also reads the `AnimationFrame*.uop` headers and checks
them against the sequences (frame counts, action ids, 1-based frame numbers). `--dump BODY` prints one
decoded sequence with the first frame of every direction.

```
C="<Enhanced Client folder>"
python3 -I verify_animation.py --sequence "$C/AnimationSequence.uop" \
    --definition "$C/AnimationDefinition.uop" --frames "$C" [--dump 400]
```

The script exits with code 1 and prints `FORMAT ERROR: ...` on the first violation: a read past the end of
an entry, trailing bytes, an unknown effect key or an entry name that is not `build/<type>/%08d.bin`.
Standard library only; no client data is stored.
