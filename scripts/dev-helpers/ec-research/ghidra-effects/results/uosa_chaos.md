# UOSA.exe Chaos particle engine

The Enhanced Client (`UOSA.exe`, SHA-256 5a5e9dc536c836d1bf10b6f86bb88f61a6ae078f519d4060d3bc47aecbebad50,
Ghidra project `UOSA`) plays a particle effect whose name ends in `.ems` through `UOParticleEffectChaos`.
The effect drives "Chaos", a text-scripted particle engine. Its design follows the particle system of
Mason McCuskey's *Special Effects Game Programming with DirectX*: event sequences, min/max ranges,
`initial` / `at` / `final` / `fade so` events, step-based fades and emission by accumulated intervals.
Every rule below was checked in the binary; the book was only a starting hypothesis.

This document covers:

- the compiled representation of a `.ems` script;
- the run-time semantics (emission, spawn, integration, events);
- rendering;
- how `UOParticleEffectChaos` drives the engine;
- a proposed Rust design for a CPU simulator.

How a `.ems` file is found (`EffectTexture.uop`, `data/effects/<name>`) is described in
[uosa_effects.md](uosa_effects.md#particle-resource-resolution).

Conventions:

- Addresses are virtual addresses in `UOSA.exe`. The names are applied by `NameEffectCode.java` (tag
  `UO_EFFECTS`); prototypes and structures come from `effects_variables.spec`.
- **confirmed** means read in the assembly. **probable** means inferred from use or from one code path.
  **unknown** means not established.
- Every float is IEEE single precision unless noted. SSE arithmetic (`SUBSS`/`MULSS`/`ADDSS`) is used
  throughout, so an f32 simulator matches the rounding, except for `rand01` (see [RNG](#rng)).
- Survey numbers ("used N times") are from the 61 `.ems` files of `EffectTexture.uop` (135 sequences).

## Overview

| Object | RTTI class / address | Role |
|---|---|---|
| `ParticleSystemManager` | global `DAT_00e32da8` | caches compiled scripts by uppercase path, owns the particle pool, keeps the global time, updates active emitters |
| `EmitterGroup` | ctor `0x0089e820` | one per cached script; holds the template `Emitter` and up to N clones (grows by 10) |
| `Emitter` | vftable 0x00ce5460, 0x7c bytes | one compiled `particlesystem`: MinEmitTime, the sequences, per-sequence particle deques, emission state |
| `ParticleEventSequence` | vftable 0x00ce5d08, 0x11c bytes | one `eventsequence` block: properties, events, render settings |
| `ParticleEvent*` | 14 classes, 0x24/0x34/0x3c bytes | one event (`initial`, `at`, `final`, `fade so`) |
| `Particle` | 0xa8 bytes, pooled | a live particle |
| `EmitterEffect` | vftable 0x00ce5d2c | scene node handed to `UOParticleEffectChaos`; owns one emitter while it plays |
| `ParticleSet` | vftable 0x00ce5cf8 | the per-emitter renderer; one child per sequence |
| `ParticleBillboard` / `ParticleTriShape` | 0x00ce65ec / 0x00ce653c | quads for POINT and AXIAL |
| `MeshParticle` | 0x00ce660c | one `.nif` clone per particle for MESH |
| `EmitterParticle` | ctor `0x008c66c0` | one sub-`.ems` effect per particle for EMITTER |

All particles are rendered under the global `ParticleSystemRoot` node (`DAT_00e347c0`) in world space,
not under the effect's node. `UOEffects::InitParticleSystemRoot` 0x004216b0 (called by `UOWorldViewer::Init`
0x0046fa70 and `FUN_00470970`) attaches that node to render group 7, the group of the NIF effects, so Chaos
quads and NIF effects are sorted together (see "Draw order"). **confirmed**

## Script language

### Lexer (`ChaosScript::Tokenize` 0x008b1d50) — confirmed

A four-state machine (whitespace, word, quoted string, `//` comment):

- Delimiters are `( ) , " { }`. `=` is **not** a delimiter, so `a=1` is one token; the files always write
  `a = 1`.
- In the whitespace state, bytes ≥ 0x80 are skipped. Any other non-space byte starts a token: `"` starts a
  quoted string, `//` starts a comment, anything else starts a word, delimiters included.
- In the word state:
  - whitespace pushes the word;
  - a delimiter pushes the word, then pushes the delimiter as its own token;
  - `//` enters the comment state **without pushing the word** (`3.0// x` loses `3.0`);
  - any other byte is appended.
- A quoted string keeps both quotes and ends at the next `"`.
- Comments end at `\n`.
- Empty tokens are not pushed (`ChaosScript::AddToken` 0x008b1cf0).

Quirks a parser must reproduce:

- A delimiter typed after whitespace starts a *word*. If a word character follows it, the two merge:
  `XYZ (1, 2, 3)` gives the token `(1`, which is classified as `(` and then fails.
- `"` directly after a word: the word is pushed, and the quoted string starts with the word's text.

### Classifier (`ChaosScript::ClassifyToken` 0x008b15c0) — confirmed

A token whose first character is a digit or `-` is a number (type 1); `.5` and `+1` are not numbers.
Single characters map to `=` 2, `,` 3, `(` 4, `)` 5, `{` 6, `}` 7, and `"` 8 (a quoted token).

Otherwise the token is **uppercased in place** and tested with substring `contains`, in this order (first
match wins):

| Order | Substrings | Type |
|---|---|---|
| 1 | `STAYATEMITTER`, `CENTERED`, `SORT`, `LOCAL` | 0x26 bool property |
| 2 | `PARTICLESYSTEM` 9, `EVENTSEQUENCE` 0xa, `RANDOM` 0xb, `XYZ` 0xc, `RGBA` 0xe, `FADE` 0x21, `CELSETUP` 0xd, `INITIAL` 0x24, `FINAL` 0x25, `TEXTURE` 0xf, `TYPE` 0x11, `INHERITED` 0x10, `EMITTERNAME` 0x13, `MESHNAME` 0x14, `CELANIMATE` 0x15, `BURST` 0x16, `EMISSIONBOOST` 0x17, `COLLISIONCHECK` 0x18 | as listed |
| 3 | `POINT`, `MESH`, `BILLBOARD`, `EMITTER`, `LIGHT`, `TEST`, `AXIAL` | 0x19 type value |
| 4 | exactly `SO` | 0x22 |
| 5 | exactly `AT` | 0x23 |
| 6 | `ALPHA_` | 0x1a blend value |
| 7 | `SOURCEBLENDMODE`, `DESTBLENDMODE` | 0x29 blend property |
| 8 | `GRAVITY`, `EMITRADIUS`, `CHAOSLOCATION`, `CHAOSVELOCITY` | 0x28 vector property |
| 9 | `LIFETIME`, `EMITRATE`, `NUMPARTICLES`, `SCALEMULTIPLIER`, `VELOCITYMULTIPLIER`, `LOOPS` | 0x27 numeric property |
| 10 | `MINEMITTIME` 0x1b, `LOCATIONONLY` 0x1d | as listed |
| 11 | `SCALE`, `EVENTTIMER`, `THRUST`, `LENGTH` | 0x1e scalar event |
| 12 | `VELOCITY`, `ROTATION` | 0x1f vector event |
| 13 | `COLOR` | 0x20 colour event |
| — | anything else (`TRUE`, `ALPHA`, names) | 0x2a |

Consequences:

- Keywords are case-insensitive.
- Any word that *contains* a keyword is that keyword. For example `SORTED` is the bool property `SORT`.
- Type 0x1c is never produced.
- `ALPHA` alone is 0x2a, so the ALPHA event cannot be written ("Expecting particle property after time
  specifier!"). Its event class exists (`ParticleEventAlpha`), but no script can reach it.

### Grammar (`Emitter::CompileScript` 0x008a1160) — confirmed

```
script    := PARTICLESYSTEM "name" number '{' system* '}'
system    := MINEMITTIME '=' range              -> emitter+0x6c (last one wins; allowed between sequences)
           | LOCATIONONLY '=' bool              -> emitter+0x76 (never read at run time)
           | EVENTSEQUENCE "name" '{' seqitem* '}'
seqitem   := property | event                   (a property after the first event is an error)
range     := number | RANDOM '(' number ',' number ')'
event     := [FADE SO] (INITIAL | FINAL | AT range) eventkw '=' value
```

- The version number after the name is required but ignored (the data uses 0.00 to 4.00).
- Any error throws a `const char*` message ("Unexpected "%s" in Particle System Block!" and the like), and
  the whole script fails. `EmitterGroup::LoadEmitter` (probable) then logs "Can't compile emitter script
  for " and the effect plays nothing.
- `number` is converted with `atof` (`atol & 0xffff` for BURST), so `1.5abc` parses as 1.5.
- `RANDOM(a, b)` stores `min = a`, `max = b` as written, even when `a > b`. See
  [Sampling](#sampling-and-rng).

### Sequence properties (`ParticleEventSequence::CompileBlock` 0x0089ff00) — confirmed

| Keyword | Value | Stored at (sequence) | Default | Notes | Used |
|---|---|---|---|---|---:|
| `texture` | `"file.tga"` | +0x28 string | "" | quotes stripped; `_splitpath` name+ext; loaded from `Data/Effects/` | 126 |
| `meshname`, `emittername` | `"file"` | +0x28 (same string) | | the last of texture/meshname/emittername wins | 9 / 0 |
| `type` | `POINT`=1 `MESH`=2 `BILLBOARD`=3 `EMITTER`=4 `AXIAL`=5 `LIGHT`=6 `TEST`=8 | +0x74 | 0 | substring match (`ParseParticleType` 0x008b1320) | 135 |
| `sourceblendmode` / `destblendmode` | `ALPHA_*` | +0x70 / +0x6c | 0 | exact names, see [Blending](#blending) | 126 |
| `lifetime` | range | +0x78/+0x7c | 0, 0 | seconds; sampled per particle | 135 |
| `emitrate` | range | +0x80/+0x84 | 0, 0 | particles per second; **only max is used** | 125 |
| `numparticles` | range | +0xe8 u16 | 100 | `(int)max`; cap on live particles of this sequence | 67 |
| `loops` | range | +0xf4 i32 | -1 | `(int)max`; **never read at run time** | 0 |
| `scalemultiplier` | range | +0xf8 | 1.0 | `max`; sets flag bit 6; multiplies later SCALE events at parse time | 67 |
| `velocitymultiplier` | range | +0xfc | 1.0 | `max`; sets bit 7; multiplies later VELOCITY events (all 6 floats) at parse time | 0 |
| `gravity` | `XYZ(...)` | +0x88..+0x9c | 0 | acceleration, sampled every update | 16 |
| `chaoslocation` | `XYZ(...)` | +0xd0..+0xe4 | 0 | spawn offset; error with StayAtEmitter | 133 |
| `chaosvelocity` | `XYZ(...)` | +0xb8..+0xcc | 0 | added to the initial velocity; error with StayAtEmitter | 0 |
| `inherited` | `XYZ(...)` | +0xa0..+0xb4 | 0 | per-component factor on the emitter velocity; error with Local | 0 |
| `emitradius` | `XYZ(...)` | — | | **"Unknown sequence vector property!"**: unusable | 0 |
| `burst` | int range | +0x104/+0x108 u32 | 1, 1 | particles per emission step (`ParseIntRange` 0x008b09b0) | 63 |
| `emissionboost` | range | +0x10c/+0x110 | 0 | sets bit 11; extra emit rate per unit of emitter movement | 9 |
| `collisioncheck` | `(STOP\|REFLECT, range)` | +0x114/+0x118 | | bit 12, plus bit 13 STOP or bit 14 REFLECT; **hangs the client at run time** | 0 |
| `celanimate` | `CELSETUP(frames, uStep, vStep, cols, rows, fps, loop)` | +0x54.. | frames 1, cols 1, rows 1 | bit 9; bit 10 when `loop > 0`; each argument may be RANDOM (min is used) | 19 |
| `stayatemitter` | bool | bit 4 | false | error with Local | 0 |
| `local` | bool | bit 8 | false | error with StayAtEmitter | 13 |
| `centered`, `sort` | bool | bits 1, 2 | false | no run-time reader found (probable: no effect) | 0 |

- Bool values (`ParseBool` 0x008b1260): a token containing `1`, `T` or `Y` is true; otherwise one
  containing `0`, `F` or `N` is false; anything else is an error. Because the true test runs first, `NOT`
  is true (it contains `T`).
- Vectors (`ParseVector` 0x008b0ea0): `XYZ(x, y, z)`, each component a number or RANDOM. They are stored
  `[min.x, min.y, min.z, max.x, max.y, max.z]`.
- Colours (`ParseColor` 0x008b1050): `RGBA(r, g, b, a)` in 0..1. They are stored
  `[min.r, min.g, min.b, min.a, max.r, max.g, max.b, max.a]`.
- `CELSETUP` (`ParseCelSetup` 0x008b0b20) stores:
  - frames `(u16)` at +0x54;
  - uStep and vStep at +0x58 and +0x5c (raw floats: the cell size in UV units);
  - cols and rows `(u8)` at +0x60 and +0x61;
  - fps at +0x64;
  - `loop = arg > 0.0` at +0x68.

  `rows` is never read at run time.

### Events — confirmed

Time specifier (`ParseTime` 0x008b14e0):

| Form | Time range |
|---|---|
| `initial` | 0 (fade not allowed) |
| `at T` / `at RANDOM(a, b)` | (T, T) / (a, b) |
| `final` | (L, L), where L = the sequence's `lifetime` **max at that point of the script** |
| `fade so` prefix | sets is_fade (+0x14); "Fading is not supported on this event." if the class refuses |

Event keyword (`ParticleEventSequence::CreateEvent` 0x0089f9b0), checked by substring in this order:
SCALE, EVENTTIMER, REDCOLOR, GREENCOLOR, BLUECOLOR, ALPHA, COLOR, LENGTH, VELOCITYX, VELOCITYY,
VELOCITYZ, VELOCITY, THRUST, ROTATION.

| Event | Class | Size | Value | Particle field | Parse / Apply | Used (event lines) |
|---|---|---:|---|---|---|---:|
| `scale` | ParticleEventScale | 0x24 | range × scalemultiplier | size +0x8, step +0xc | 0x008af0d0 / 0x008af6f0 | 353 |
| `color` | ParticleEventColor | 0x3c | RGBA | colour +0x28, step +0x38 | 0x008af150 / 0x008af820 | 333 |
| `redcolor`/`greencolor`/`bluecolor` | ParticleEventRed/Green/BlueColor | 0x24 | range | one channel | 0x008af1d0.. / 0x008af980.. | 0 |
| `alpha` | ParticleEventAlpha | 0x24 | range | +0x34 | unreachable (see Classifier) | 0 |
| `velocity` | ParticleEventVelocity | 0x34 | XYZ × velocitymultiplier | velocity +0x48, step +0x54 | 0x008af310 / 0x008afe40 | 101 |
| `velocityx/y/z` | ParticleEventVelocityX/Y/Z | 0x24 | range | one component | 0x008af430.. / 0x008aff80.. | 0 |
| `thrust` | ParticleEventThrust | 0x24 | range | thrust +0x60, step +0x64; sets bit 5 | 0x008af520 / 0x008b0310 | 26 |
| `rotation` | ParticleEventRotation | 0x34 | XYZ degrees → radians (×π/180 at parse) | rotation +0x80, step +0x8c; sets bit 3 | 0x008af570 / 0x008b0450 | 115 |
| `length` | ParticleEventLength | 0x24 | range | length +0x98, step +0x9c | 0x008af650 / 0x008b05d0 | 62 |
| `eventtimer` | ParticleEventEventTimer | 0x24 | range | event timer +0x18 | 0x008af6a0 / 0x008b0720 | 0 |

VELOCITY, VELOCITYX/Y/Z and THRUST are errors in a StayAtEmitter sequence.

At the closing `}` of a sequence, three steps run:

1. `ParticleEventSequence::ResolveEventTimes` (0x008b79e0) samples each event's actual time (+0x10) **once,
   at compile time**. Every particle of the sequence shares it.
2. `ParticleEventSequence::SortEvents` (0x008b8cc0) sorts the events by actual time.
3. 0x008b7990 links each event's `next_fade` (+0x4) to the first later event of the same type id (+0x18)
   that has is_fade set.

## Compiled structures

Applied in Ghidra as `/UO/ChaosEmitter`, `/UO/ChaosEventSequence`, `/UO/ChaosParticleEvent`,
`/UO/ChaosParticle`, `/UO/ChaosRange`, `/UO/ChaosVectorRange`, `/UO/ChaosIntRange`, `/UO/ChaosCelSetup`
and `/UO/ChaosToken`.

### ParticleEventSequence (0x11c, ctor 0x008b7330)

| Offset | Field | Confidence |
|---|---|---|
| +0x4 | refcount | confirmed |
| +0x8 | flags: bit1 CENTERED, 2 SORT, 3 rotation used, 4 STAYATEMITTER, 5 thrust used, 6 scalemult set, 7 velmult set, 8 LOCAL, 9 CELANIMATE, 10 cel loop, 11 EMISSIONBOOST, 12 COLLISIONCHECK, 13 STOP, 14 REFLECT | confirmed (bit 5 probable from use) |
| +0xc | name (std::string, quoted) | confirmed |
| +0x28 | texture / mesh / emitter file (std::string) | confirmed |
| +0x44 | events (std::vector of ParticleEvent*) | confirmed |
| +0x54..+0x68 | cel setup | confirmed |
| +0x6c / +0x70 | dest / source blend | confirmed |
| +0x74 | particle type | confirmed |
| +0x78 .. +0xe4 | lifetime, emitrate, gravity, inherited, chaosvelocity, chaoslocation ranges | confirmed |
| +0xe8 | numparticles (u16) | confirmed |
| +0xec | particles emitted (counter) | confirmed |
| +0xf4 | loops | confirmed (no reader) |
| +0xf8 / +0xfc | scalemultiplier / velocitymultiplier | confirmed |
| +0x104 | burst (u32 min, max) | confirmed |
| +0x10c | emissionboost range | confirmed |
| +0x114 | collision absorption range | confirmed |

### ParticleEvent (base vftable 0x00ce5334)

The vtable slots are +0 dtor, +4 `Apply(particle)`, +8 SupportsFade and +0xc `Parse`.

| Offset | Field |
|---|---|
| +0x4 | next_fade: the next fade event of the same type |
| +0x8 / +0xc | time range |
| +0x10 | actual time (sampled at compile) |
| +0x14 | is_fade |
| +0x18 | type id: Scale 0, Color 1, Red 2, Green 3, Blue 4, Alpha 5, Velocity 6, VelocityX/Y/Z 7..9, Thrust 0xa, EventTimer 0xb, Rotation 0xc, Length 0xd |
| +0x1c | value: scalar (min, max); RGBA (min[4], max[4]); XYZ (min[3], max[3]) |

### Particle (0xa8, pooled by ParticleSystemManager)

| Offset | Field | Default (pool template) |
|---|---|---|
| +0x4 | iterator to the next event | |
| +0x8 / +0xc | size / step | 1.0 / 0 |
| +0x10 | lifetime | 1.0 (then sampled) |
| +0x14 | age | |
| +0x18 | event timer | 0 |
| +0x1c / +0x20 | birth time / last update time | |
| +0x24 | last dt | |
| +0x28 / +0x38 | RGBA / step | (1,1,1,1) / 0 |
| +0x48 / +0x54 | velocity / step | 0 |
| +0x60 / +0x64 | thrust / step | 0 |
| +0x68 / +0x74 | position / previous position | |
| +0x80 / +0x8c | rotation (radians) / step | 0 |
| +0x98 / +0x9c | length / step | 1.0 / 0 |
| +0xa4 | pool id | |

The pool allocator (0x008b83a0, pool in ESI) reuses released particles. `ParticleSystemManager::
ReleaseParticle` (0x008a2810) copies the template back, so every particle starts from these defaults.

### Emitter (0x7c, ctor 0x0089f210)

| Offset | Field |
|---|---|
| +0x4 | ParticleSet renderer |
| +0x8 | owner EmitterEffect (null when stopped) |
| +0x14 / +0x18 | sequences begin / end |
| +0x24 | per-sequence `std::deque<Particle*>` (stride 0x14) |
| +0x34 | per-sequence last emission time |
| +0x48 | last update time |
| +0x4c | previous owner world position |
| +0x58 | owner velocity |
| +0x64 | emission seconds remaining |
| +0x68 | infinite (duration ≤ 0) |
| +0x69 | first burst pending |
| +0x6a | one-shot |
| +0x6c | MinEmitTime range |
| +0x75 | skip first update |
| +0x76 | LocationOnly (unused) |
| +0x77 | started (IsActive) |
| +0x78 | manager |

Virtual slots:

| Slot | Name | Address |
|---|---|---|
| +0x0 | dtor | 0x0089f2e0 |
| +0x4 | Start | 0x0089f860 |
| +0x8 | ClearStarted | 0x0089e780 |
| +0xc | Stop | 0x008a1ce0 |
| +0x10 | Clone | 0x0089f460 |
| +0x14 | Update | 0x008a13c0 |
| +0x18 | Reset | 0x0089f940 |
| +0x1c | IsActive | 0x0089e7a0 |
| +0x20 | CompileScript | 0x008a1160 |
| +0x24 | CopyTo | 0x0089f710 |

## Sampling and RNG

### RNG — confirmed

There is one global MT19937 for the whole client:

| Function | Address |
|---|---|
| `MersenneTwister::NextU32` | 0x0099aa85 |
| `Twist` | 0x0099ac5a |
| `Seed` (init_genrand, 0x6c078965) | 0x0099ac2a |
| `SeedByArray` (0x19660d / 0x5d588b65) | 0x0099ab3e |

`SeedFromClock` (0x0099abff) is a CRT initializer: it seeds from a hash of `_time64()` and `clock()` at
startup. Other game systems draw from the same generator (effect Wait/Random collections, for example).
The sequence is therefore neither reproducible nor owned by Chaos, and a simulator may use any seed.

```
rand01() = (float)( (double)(int32)u [+ 2^32 if negative] * 2^-32 )   // u = NextU32()
```

The float rounding returns exactly 1.0 when `u ≥ 2^32 − 128`. **confirmed**

### Range sampling — confirmed

Scalar ranges (`ChaosScript::SampleFloatRange` 0x008a2050, also inlined in CreateParticle):

```
sample(min, max):
    if min == max: return min                 // no RNG draw
    r = rand01()
    if min < max: return min + (max - min) * r
    return r                                   // min > max: the bounds are ignored (quirk)
```

Vectors (`SampleVectorRange` 0x008aebe0) sample x, y, z independently, in that order, with the same rule.
Colours sample r, g, b, a the same way.

Integer ranges (`SampleIntRange` 0x0089cfe0, EAX = min, ECX = max), used for BURST:

```
if min == max: return min
r = rand01()
return ((max > min) ? trunc(min + (max - min) * r) : trunc(r)) & 0xffff
```

`max` is therefore **exclusive** (except when r = 1.0), so `burst = Random(1, 3)` gives 1 or 2. When
`min > max` the result is 0.

## Run time

### Time source — confirmed

- `UOEffects::UpdateActiveEffects(dt)` computes `ms = round(dt · 1000)` and calls
  `ParticleSystemManager::Update(manager, ms)` (0x008a7780).
- The manager does `time += ms · 0.001` (+0xc8, constant 0.001 at 0x00c9857c). It then calls
  `Emitter::Update(time, ms · 0.001)` for every emitter in the active deque (`DAT_00e347a0`).
- An emitter that is no longer active and has `CountParticles() == 0` (0x008a7240) is removed.

Time is therefore global, monotonic, quantized to whole milliseconds, and advanced once per rendered
frame. There is no fixed simulation step, only the sub-steps below.

### Emitter::Start (0x0089f860) — confirmed

```
Start(start_time, duration, not_looping):
    if started: return
    m = sample(MinEmitTime)                      // the system-level MINEMITTIME
    if not_looping and m > 0: one_shot = false; duration = m
    else:                     one_shot = not_looping
    remaining = duration; infinite = duration <= 0
    first_burst = true; started = true; skip_first_update = true
```

Resulting behaviour:

| Effect definition | MinEmitTime | Emission |
|---|---|---|
| loop | any | continuous for `duration` seconds (forever if ≤ 0) |
| not loop | > 0 | continuous for `sample(MinEmitTime)` seconds |
| not loop | ≤ 0 or absent | one burst |

Every case starts with one burst: `burst` particles per sequence. 56 of the 61 files set MinEmitTime.

### Emitter::Update (0x008a13c0) — confirmed

```
Update(time, dt):
    owner = this+0x8
    node_pos   = owner.world_translate            // NiAVObject +0x84
    node_vel   = owner.GetVelocity(dt)            // (world_pos - prev_world_pos) / dt, when dt > 0
    node_scale = owner.local_scale                // NiAVObject +0x5c
    node_rot   = owner.world_rotate               // 3x3, NiAVObject +0x60
    if skip_first_update:                         // the first frame after Start emits nothing
        skip_first_update = false; record prev pos/vel; goto end
    if started:
        remaining -= dt
        if first_burst or one_shot:
            for seq: n = SampleIntRange(burst); repeat n: CreateParticle(seq, ..., time)
                     last_emit[seq] = time
            first_burst = false
            if one_shot: Stop(pool, true, false); one_shot = false
        elif not infinite and remaining <= 0:
            Stop(pool, true, false)
        else:
            for seq:
                boost = bit11 ? |node_pos - prev_node_pos| * sample(emissionboost) : 0
                interval = max(1 / (emitrate.max + boost), 1/60)      // DAT_00d05310
                t = last_emit[seq] + interval
                while t <= time:
                    n = SampleIntRange(burst)
                    repeat n: if !CreateParticle(seq, ..., t): last_emit[seq] = time; break out
                    last_emit[seq] = t
                    if t - last_substep >= 1/30 and particles(seq) not empty:   // DAT_00d05210
                        SequenceUpdate(seq, t); last_substep = t
                    t += interval
end:
    for seq: SequenceUpdate(seq, time)
    copy the particles into the renderer buffers (see Rendering)
```

- The rate uses `emitrate` **max only**. A range such as `Random(10, 20)` emits 20 per second.
- The 1/60 s floor caps a sequence at 60 emission steps per second, each producing `burst` particles.
- With `emitrate = 0` and no boost the interval is infinite (1/0), so only the bursts emit.
- `Stop(pool, kill_stay_at_emitter = true, kill_all = false)` (0x008a1ce0) clears `started` and the owner,
  and releases only the particles of StayAtEmitter sequences. Other particles live out their lifetime.

### CreateParticle (`ParticleEventSequence::CreateParticle` 0x008b7ae0) — confirmed

Stack arguments are (seq, deque, node_pos, node_vel, node_rot, t, node_scale), with the pool in EDX.

```
if deque.size >= numparticles: return false
p = pool.alloc()                                   // template defaults
p.birth = p.last_update = t
p.lifetime = sample(lifetime)
loc = node_rot * (sample_vec(chaoslocation) * node_scale)
if !LOCAL: loc += node_pos                         // LOCAL particles are stored emitter-relative
p.pos = p.prev_pos = loc
for ev in events while ev.time == 0: ev.Apply(p)   // `initial` and `at 0`
v = p.velocity + sample_vec(chaosvelocity)
p.velocity = node_rot * v + node_vel ⊙ sample_vec(inherited)
p.size, p.size_step, p.velocity, p.velocity_step *= node_scale
p.next_event = first event with time != 0
seq.emitted += 1; deque.push_back(p)
```

- `node_rot * v` is a row-major product: `x' = R[0]·x + R[1]·y + R[2]·z`.
- The particle type does **not** change the spawn. Particles are not emitted from points of a mesh: MESH
  means "render a `.nif` per particle" (see Rendering).
- `centered`, `emitradius` and `locationonly` have no effect.

### Particle::Update (0x008b6fb0) — confirmed

The particle is in EDI and the node position in ECX. The stack holds (time, gravity*, &seq.flags).

```
p.age = time - p.birth
if p.age >= p.lifetime: return dead
dt = time - p.last_update; p.event_timer += dt; p.last_dt = dt
p.prev_pos = p.pos
if STAYATEMITTER: p.pos = node_pos
else:
    thrust_vec = 0
    if bit5 and p.velocity != 0:
        p.thrust += p.thrust_step * dt
        thrust_vec = normalize(p.velocity) * p.thrust
    p.velocity += (gravity + p.velocity_step + thrust_vec) * dt     // semi-implicit Euler
    p.pos += p.velocity * dt
if bit3: p.rotation += p.rotation_step * dt
p.color += p.color_step * dt                        // not clamped here
p.size  += p.size_step * dt
p.length += p.length_step * dt
p.last_update = time
```

- The velocity step from a `fade so` velocity event works like an acceleration added to gravity.
- There is no drag and no collision (see the quirks).
- Thrust is only integrated while the velocity is non-zero.

### SequenceUpdate (`ParticleEventSequence::Update` 0x008b7fb0) — confirmed

```
gravity = sample_vec(seq.gravity) ⊙ G            // sampled once per call, shared by all particles
for p in deque:
    if !Particle::Update(p, time, gravity): release p, erase
    else: RunEvents(seq, p)
```

- `G` is the global vector at `DAT_00de2810..18`. `FUN_004216b0` writes it from column 2 of the 3×3 at
  `DAT_00e37224+0x30`: the world up axis, **probable (0,0,1)**. Only the component of `gravity` along that
  axis survives.
- With COLLISIONCHECK (bit 12), the live branch jumps back without advancing the iterator or running
  events. The loop never ends, so a script that uses it hangs the client. **confirmed**; no data uses it.

### Events at run time — confirmed

`ParticleEventSequence::RunEvents` (0x008b7f40, particle in EDI):

```
it = p.next_event
while it != end and p.event_timer >= it.time:
    it.Apply(p)
    if the event changed p.event_timer (EVENTTIMER):
        it = (last event with time < new timer) + 1   // re-fires the latest passed event
    else: it += 1
p.next_event = it
```

`Apply` (vtable +4) is the same pattern for every event type, with V the particle field:

```
Apply(p):
    if !is_fade: V = sample(value)                 // per particle: one RNG draw per non-degenerate component
    if next_fade:
        target = sample(next_fade.value)           // sampled now, per particle
        span = next_fade.time - this.time; if span == 0: span = 1.0
        V_step = (target - V) / span
    elif type in {Thrust, Rotation, Length}: V_step = 0
    // Scale, Color, Red/Green/Blue, Alpha, Velocity(X/Y/Z): the step is kept
```

Consequences a simulator must reproduce:

- **Interpolation.** The fade is linear accumulation, `V += V_step · dt`, started when the event fires (at
  the first update after `event_timer` passes its time, not exactly at it). Values can overshoot or
  undershoot the target by up to one frame's worth of step.
- **A fade event does not set its value.** It only ends the previous ramp. If a later fade of the same
  type follows, it starts a new ramp from the current value. Otherwise Scale/Color/Velocity **keep
  ramping** at the old rate, while Thrust/Rotation/Length stop.
- **`final` fades** use the lifetime **max**. A particle whose lifetime is shorter dies before it reaches
  the final value. For `initial scale = 1`, `fade so final scale = 0` and lifetime `Random(1, 2)`, a
  particle living 1 s ends at about 0.5 or more.
- **Event times are shared.** `at Random(a, b)` is drawn once per compiled sequence (per cached script),
  not per particle.
- **EVENTTIMER quirk.** `EventTimer` Apply (0x008b0720) with `min > max` sets the timer to `rand01()` (the
  general sampling rule).
- An event whose time is 0 fires at spawn, before the velocity is rotated and scaled. Later events write
  world-space values directly. A non-initial VELOCITY event therefore ignores the node rotation and scale.

## Rendering

### Renderer selection (`ParticleSet::CreateSequenceRenderer` 0x008b3980) — confirmed

| Type | Renderer |
|---|---|
| 1 POINT, 5 AXIAL | `ParticleBillboard`: one `ParticleTriShape` with `numparticles` quads (4 vertices, 2 triangles each) |
| 2 MESH | `MeshParticle`: `numparticles` clones of the `.nif` named by `meshname` |
| 4 EMITTER | `EmitterParticle`: one sub-effect of the `.ems` named by `emittername` per particle |
| 3 BILLBOARD, 6 LIGHT, 8 TEST | no case: **nothing is rendered** |

The data uses only POINT (96), AXIAL (30) and MESH (9).

### Render state of quads — confirmed unless noted

| Property | Value |
|---|---|
| Texture | `NiTexturingProperty` base map, texture coordinate set 0, apply MODULATE (flags `& 0xfff5 \| 4`). `FUN_004169a0` creates the map with flags 0x3100 (WRAP_S_WRAP_T, BILERP); `CreateSequenceRenderer` then sets the filter to 2 = FILTER_TRILERP (`FUN_0041a590(map, 2)`, 0x008b3c52..0x008b3c5d): bilinear on the single-level textures, trilinear where the DDS stores mips. Loaded by `ParticleSystemManager::LoadTexture` 0x008a8630 (see "Texture loading"). |
| Blending | `NiAlphaProperty` (`FUN_00419f70`, flags 0xec then `& 0xfdff \| 1`, src << 1, dst << 5): blending on, src = sourceblendmode, dst = destblendmode, alpha test off, no-sorter clear (sorted) |
| Depth | `NiZBufferProperty` 0xd: test on (LESSEQUAL), write **off**. It is set on `ParticleSystemRoot` and on each shape. |
| Vertex colour | `NiVertexColorProperty` (ctor 0x00416bf0): flags `& 0xffd7 | 0x10`, i.e. source EMISSIVE and lighting E (probable enum mapping): unlit, the colour comes from the vertices |
| Culling | AXIAL adds `NiStencilProperty` draw mode BOTH (two-sided); POINT keeps the default back-face culling |
| Shader | "SimpleFFP" |
| Sorting | each sequence shape is one item of the render group 7 sort (see "Draw order"); inside a shape the quads are drawn in deque order (oldest first), with no per-quad sort |
| Hue | no hue path found in the Chaos code: Chaos effects are not hued (probable) |

### Blending

`ParseBlendMode` (0x008b1430) matches exact names from a table at 0x00c983c8 (stride 0x20, value at +0x1c).
The values are Gamebryo's `NiAlphaProperty::AlphaFunction`. The D3D mapping is the standard Gamebryo DX9
one (probable; the renderer was not traced):

| Name | Value | D3DBLEND | Used (src/dst pairs) |
|---|---:|---|---|
| ALPHA_ONE | 0 | ONE | ONE/ONE ×69 (additive) |
| ALPHA_ZERO | 1 | ZERO | ZERO/SRCCOLOR ×1 (modulate) |
| ALPHA_SRCCOLOR | 2 | SRCCOLOR | |
| ALPHA_INVSRCCOLOR | 3 | INVSRCCOLOR | |
| ALPHA_DESTCOLOR | 4 | DESTCOLOR | |
| ALPHA_INVDESTCOLOR | 5 | INVDESTCOLOR | |
| ALPHA_SRCALPHA | 6 | SRCALPHA | SRCALPHA/INVSRCALPHA ×56 (alpha blend) |
| ALPHA_INVSRCALPHA | 7 | INVSRCALPHA | |
| ALPHA_DESTALPHA | 8 | DESTALPHA | |
| ALPHA_INVDESTALPHA | 9 | INVDESTALPHA | |
| ALPHA_SRCALPHASAT | 10 | SRCALPHASAT | |

The default (no blend keyword) is 0/0, which is additive. The 9 MESH sequences set no blend mode. The
survey's 78 pairs of (0, 0) are those 9 plus 69 explicit ONE/ONE.

### Hand-off to the renderer (end of Emitter::Update) — confirmed

Each sequence has a render descriptor (stride 0x74) whose buffers receive, per live particle:

| Offset | Data |
|---|---|
| +4 | RGBA |
| +8 | size |
| +0xc | position, + node_pos when LOCAL |
| +0x10 | previous position, + node_pos when LOCAL |
| +0x14 | length × node local scale |
| +0x18 | rotation |
| +0x1c | lifetime |
| +0x20 | age |
| +0x24 | dt |

The descriptor also holds:

| Offset | Data |
|---|---|
| +0x28 | flags |
| +0x2c | type |
| +0x34 | cel frames |
| +0x38 / +0x3c | cel u / v step |
| +0x40 | cel columns |
| +0x44 | cel fps |
| +0x4c | node world rotation |
| +0x70 | velocity + node velocity |

LOCAL particles are therefore simulated relative to the emitter and drawn at the emitter's *current*
position: they follow a moving emitter. Non-LOCAL particles stay where they were spawned.

### Quad construction (`ParticleTriShape::BuildVertices` 0x008c4c30) — confirmed

- The template comes from `InitBuffers` (0x008c5c50).
- UVs per quad: v0 (0,1), v1 (0,0), v2 (1,0), v3 (1,1).
- Indices: (n, n+2, n+1) and (n, n+2, n+3).

Camera data comes from the global camera `DAT_00e321d0`:

- world rotate column 0 (+0x60, +0x6c, +0x78) is the view direction D;
- column 1 (+0x64, +0x70, +0x7c) is up U;
- column 2 (+0x68, +0x74, +0x80) is right R;
- the position is at +0x84.

Every quad uses `h = size · 0.5` (constant 0.5 at 0x00c95604).

POINT without rotation (flag bit 3 clear):

```
v0 = p - R·h - U·h     v1 = p - R·h + U·h     v2 = p + R·h + U·h     v3 = p + R·h - U·h
```

POINT with rotation (bit 3): the axes are replaced by columns 1 and 2 of `MakeRotation(-rot.y, D) ×
camera_rotate`. The quad spins in the screen plane by `rot.y` radians. Only the y component is used. The
sign convention is probable.

AXIAL (type 5): a quad stretched along the velocity, facing the camera:

```
vb = buffer velocity (particle velocity + node velocity); skip the particle when vb == 0
d    = normalize(vb)
view = normalize(cam_pos - p)
S    = h · cross(view, d)
front = p + d · length_scaled                      // length × node local scale
v0 = front - S   v1 = front + S   v2 = p + S   v3 = p - S
```

So u = 0 at the front (leading) end and u = 1 at the particle position. The quad points along the motion,
`length` units long and `size` wide.

Colours are clamped to [0, 1], and all four vertices share them.

### Draw order — confirmed

- `ParticleSystemRoot` is in render group 7 with the NIF effects (see `uosa_effects.md`, "Draw order"):
  one `NiAlphaAccumulator` sort, far to near by world bound centre · view direction, over the effect
  geometry and the Chaos shapes together.
- Each sequence's `ParticleTriShape` is one sort item. `ParticleSet::Update` 0x008b2de0 (cases 1 and 5)
  recomputes its bound every 0.033 s (`DAT_00d05214`): centre = the AABB centre of the live particle
  positions, radius = sqrt(max squared distance from it) + 0.01 (`DAT_00d05120`). It stores it through
  `ParticleBillboard::SetBound` 0x008c6300 (vtable +0x14: shape +0x1c..+0x28 and its data (+0xc0)
  +0x10..+0x1c) and updates the shape (`Update(0, 0)`). Before the first refresh the bound comes from the
  template quads (`ParticleTriShape::InitBuffers`, `NiBound::ComputeFromData` `FUN_0068e750`).
  `BuildVertices` does not touch it.
- Within a shape, quads keep the deque order (oldest first).

### Texture loading — confirmed

`ParticleSystemManager::LoadTexture` 0x008a8630 (only caller `CreateSequenceRenderer`) looks the name up in
the cache map (0x00e347e8 / 0x00e347ec); otherwise it calls `NiSourceTexture::Create` 0x0068a8b0 with
`NiSourceTexture_ms_kDefaultPrefs` 0x00dde184 = {PIX_DEFAULT, ALPHA_DEFAULT, MIP_DEFAULT} and the load-direct
hint `g_UOTextureLoadDirectHint` 0x00e31e9b (`.bss`, read only: false). `ms_bPreload` (0x00da7b79 = 1)
creates the renderer data at once. Without the hint, the texture goes through `NiDevImageConverter`, and
mipmapping is on only for `YES`, or `MIP_DEFAULT` with `NiTexture_ms_bMipmapByDefault` 0x00e31ea3 (false,
never written). `ConvertPixelData` keeps the stored levels and generates none
(`NiDX9SourceTextureData.cpp:375-378`, `NiDevImageConverter.cpp:450-475`). So a Chaos texture has exactly
the levels its file stores: the 16 single-level power-of-two Chaos-only textures have no mips in the client
either, and packing them without mips is correct.

### Cel animation (flag bit 9) — confirmed

`ParticleTriShape::SetCelTexCoords` (0x008c5970) runs per particle (CX = frame, EDI = cel descriptor):

```
frame = (u16) trunc(age * fps)
if loop: frame %= frames
frame = min(frame, frames - 1)
col = frame % cols;  row = frame / cols                // integer IDIV; `rows` is not used
u0 = col * uStep;    v0 = row * vStep
uv: v0 (u0, v0+vStep)  v1 (u0, v0)  v2 (u0+uStep, v0)  v3 (u0+uStep, v0+vStep)
```

The frame comes from the particle's age (not its event timer). Negative uStep or vStep selects a tiling
variant based on the particle length (`floor`, epsilon 0.002 at 0x00d05208). No data uses it.

Example: `CelSetup(4, 1, 0.25, 1, 4, 24, 1)` is a vertical strip of 4 frames at 24 fps, looping.

### MESH and EMITTER particles — confirmed in outline

- **MeshParticle** (ctor 0x008c6400) keeps `numparticles` clones of the `.nif`. For each live particle:
  - translate = position;
  - uniform scale = size;
  - rotation = node world rotation, × Euler(rotation) when bit 3 is set (0x0066b640).

  The clone's controllers are sub-stepped at 1/30 s and then updated with the **particle age**, so a mesh
  animation starts at the particle's birth; the controllers' own cycle types apply.
- **Mesh look** (confirmed): `ParticleSystemManager::LoadMesh` 0x008a8440 loads the `.nif` once (cached by
  name), runs `Update(0, 1)`, `UpdateProperties` and `UpdateEffects`, and returns a clone.
  `ParticleSet::UnshareAnimatedGeometry` 0x008b2820 gives each geometry whose properties have controllers
  its own copy of the geometry data. The particle colour and alpha are **not** written to the clone: the
  mesh is drawn with the NIF's own properties (material, vertex colours, alpha, z-buffer, textures) through
  `NiStandardMaterial`, like a NIF effect. `MeshParticle::SetActiveCount` 0x008c6520 culls (APP_CULLED,
  `NiAVObject` +0xac bit 0) the clones past the live count. Each geometry of each clone is its own sort
  item with its own bound.
- **EmitterParticle** (ctor 0x008c66c0) starts one `EmitterEffect` of the sub-script per particle and moves
  it with the particle. No data uses it.

## UOParticleEffectChaos (vftable 0x00c9e114)

See [uosa_effects.md](uosa_effects.md#runtime-semantics) for the IUOEffect contract.

| Method | Address | Behaviour |
|---|---|---|
| Init | 0x0041aff0 | Requires `definition+0x3c == 0`. Stores loop (+0x26) and duration (+0x28). `UOEffects::LoadChaosEmitterEffect` (0x00421d70) calls `ParticleSystemManager::LoadEmitterGroup` (0x008a7920), which uppercases the path, checks the cache, requires `.EMS` and compiles. It then takes the `_splitpath` file name and calls `CreateEmitterEffect` (0x008a7ca0). Node local scale = `fabs(definition scale)`. Node local rotation = the 9 floats at `DAT_00e37224+0x30` (the same global the Nif effect uses). Offset = the definition offsets along the camera axes, as for the Nif effect. |
| Start | 0x0041b210 | Resets fade (+0x2c) and elapsed (+0x20). Calls `EmitterEffect::Start(duration, !loop)` (0x008b9980), which takes an inactive emitter of the group (cloning 10 more when none is free) and calls `Emitter::Start(0, duration, !loop)`. Sets active (+0x24). |
| Stop | 0x0041b290 | Clears active and sets stopping (+0x25). `EmitterEffect::Stop` (0x008b9a10) calls `Emitter::Stop(pool, true, false)`. |
| Update | 0x0041b370 | `elapsed += dt`. Decrements the tick countdown (+0x3c). After Stop, advances the fade timer. Calls the node's Gamebryo `Update(elapsed)` while fade < 5.0 s (0x00c98590) and the effect is not paused. Emission itself runs from `ParticleSystemManager::Update`, not here. |
| IsFinished | 0x0041b2c0 | True when the countdown is 0 or the fade timer reached 5.0 s. While active, true once no emitter of the group is active, i.e. **when emission ends, not when the particles die**. (probable in detail) |

The effect's node only provides a moving transform (world position, rotation, local scale and velocity)
to the emitter. Particles live under `ParticleSystemRoot`. When the effect is released they keep
simulating and rendering until their lifetime ends, because Stop only clears the emitter's owner.

Definition fields on the Chaos path:

| Definition field | Chaos effect |
|---|---|
| name | `.ems` file, loaded through `Data/Effects/` |
| scale | `node_scale`: multiplies chaoslocation, size and velocity (with their steps) at spawn, and the AXIAL length |
| offsets | node position, so the spawn point |
| duration (s) | emission time when looping; ignored for non-looping systems that set MinEmitTime |
| loop | `not_looping = !loop` in Emitter::Start |
| `+0x3c` | must be 0 |

## Quirks (all confirmed unless noted)

1. `emitrate` uses its max; `burst` max is exclusive; `numparticles` and `loops` take `(int)max`.
2. `Random(a, b)` with `a > b` returns `rand01()` for floats and 0 for integers. The data has three such
   components: two `initial velocity` y components written `random(-1,-5)` (so the particles get y in
   [0, 1) instead of [-5, -1]), and one chaoslocation y `random(1,0.95)`.
3. Event times are sampled once per compiled sequence and shared by all particles.
4. `final` = lifetime max as parsed. `final` before any `lifetime` line means time 0, i.e. at spawn.
5. Fades without a later fade keep their step for Scale, Color and Velocity.
6. Multipliers apply at parse time, so they must precede the events. Properties after events are rejected.
7. Rotation is in degrees in the script and radians at run time. Billboards use only `rot.y`.
8. The ALPHA event and `emitradius` cannot be used. `loops`, `centered`, `sort` and `locationonly` are
   parsed but have no effect.
9. COLLISIONCHECK hangs the client.
10. BILLBOARD, LIGHT and TEST sequences render nothing.
11. The first `Emitter::Update` after Start only records the node position. The first burst comes one
    frame later.
12. Gravity is projected onto the global up axis `DAT_00de2810` (probable (0,0,1)).
13. Lexer: `//` after a word drops the word. A delimiter after whitespace merges with the following word.
    `=` needs surrounding whitespace.
14. A token is a number only if it starts with a digit or `-`.

## Proposed Rust design

The parser (`lib/uocf/src/enhanced/chaos_script.rs`, in progress elsewhere) should reproduce the lexer and
classifier quirks above. It should yield the compiled form, not the raw syntax tree.

```rust
// Engine constants, named after the binary's.
pub const MIN_EMIT_INTERVAL_S: f32 = 1.0 / 60.0;     // DAT_00d05310
pub const SUBSTEP_INTERVAL_S: f32 = 1.0 / 30.0;      // DAT_00d05210
pub const MS_PER_SECOND: f32 = 1000.0;               // time is quantized to whole ms
pub const DEFAULT_NUM_PARTICLES: u16 = 100;
pub const DEFAULT_BURST: (u32, u32) = (1, 1);
pub const DEGREES_TO_RADIANS: f32 = std::f32::consts::PI / 180.0;
pub const QUAD_HALF_EXTENT: f32 = 0.5;
pub const EFFECT_FADE_SECONDS: f32 = 5.0;            // UOParticleEffectChaos after Stop

pub struct Range { pub min: f32, pub max: f32 }        // sample(): see "Range sampling"
pub struct VecRange { pub min: [f32; 3], pub max: [f32; 3] }

pub enum ParticleType { Point = 1, Mesh = 2, Billboard = 3, Emitter = 4, Axial = 5, Light = 6, Test = 8 }

pub struct CelSetup { frames: u16, u_step: f32, v_step: f32, columns: u8, fps: f32, looping: bool }

pub enum EventKind { Scale, Color, Red, Green, Blue, Velocity, VelocityX, VelocityY, VelocityZ,
                     Thrust, EventTimer, Rotation, Length }

pub struct CompiledEvent { kind: EventKind, time: f32 /* resolved at compile */, is_fade: bool,
                           next_fade: Option<usize>, value_min: [f32; 4], value_max: [f32; 4] }

pub struct CompiledSequence { flags: SequenceFlags, particle_type: ParticleType, resource: String,
                              src_blend: AlphaFunction, dst_blend: AlphaFunction,
                              lifetime: Range, emit_rate_max: f32, gravity: VecRange,
                              inherited: VecRange, chaos_velocity: VecRange, chaos_location: VecRange,
                              num_particles: u16, burst: (u32, u32), emission_boost: Range,
                              cel: Option<CelSetup>, events: Vec<CompiledEvent> /* sorted */ }

pub struct ChaosSystem { min_emit_time: Range, sequences: Vec<CompiledSequence> }

pub trait ChaosRng { fn next_u32(&mut self) -> u32;
                     fn rand01(&mut self) -> f32 { /* (u as f64 * 2^-32) as f32 */ } }

pub struct EmitterInput { world_pos: [f32; 3], world_rot: [[f32; 3]; 3], local_scale: f32 }

pub struct EmitterState { /* Start/Update/Stop as in the pseudo-code; per-sequence Vec<Particle>,
                             last_emit, last_substep */ }
impl EmitterState {
    pub fn start(&mut self, duration_s: f32, not_looping: bool, rng: &mut impl ChaosRng);
    pub fn update(&mut self, time_s: f32, dt_s: f32, node: &EmitterInput, rng: &mut impl ChaosRng);
    pub fn stop(&mut self);
    pub fn is_active(&self) -> bool;
    pub fn particle_count(&self) -> usize;
    pub fn build_quads(&self, camera: &CameraFrame, out: &mut Vec<QuadVertex>);  // POINT/AXIAL + cel UVs
}
```

Implementation notes:

- Resolve the event times at compile time with the simulator's RNG. Recompile (or reseed) per cached
  script, not per effect instance.
- Drive `update` with millisecond-quantized time: `dt = round(frame_dt · 1000) / 1000`, accumulated into
  `time`.
- Keep particles of LOCAL sequences emitter-relative and add the current node position when building
  quads.
- Use f32 throughout. Compute `rand01` as `(u as f64 * 2^-32) as f32`.
- Group quads per sequence with that sequence's blend pair. Use depth test on, depth write off, unlit
  vertex colour × texture, and two-sided rendering for AXIAL.
- MESH particles need the `.nif` scene with its controllers evaluated at the particle age. That belongs to
  the NIF renderer (`lib/gamebryo-nif`), with the particle giving translate, uniform scale and rotation.
- For tests, `verify_chaos.py` replicates the lexer, classifier and compiler and reports per-file results.
  A Rust parser can be compared against its output. On the 61 shipped scripts it compiles all 135
  sequences. It finds 91 `at Random(...)` event times (shared per sequence), 9 emitrate ranges (only max
  used), 3 burst ranges (max exclusive), 3 reversed Random ranges and one fade that ends before the
  lifetime max, so the step persists. No lexer quirk is hit.

## Names and types applied in Ghidra

Applied in this round:

- `NameEffectCode.java`, step 8 `chaosEngine()`: 95 functions renamed, tagged and commented, with no
  anomalies; a second `--apply` reports `Changes: none`. It also creates the namespaces `ChaosScript` and
  `MersenneTwister`.
- `effects_variables.spec`: 9 types and 40 prototypes; a second run reports `Changes: none`.

The locators use no addresses:

| Evidence | Finds |
|---|---|
| RTTI vftables of Emitter, ParticleEventSequence, the 14 ParticleEvent classes, EmitterEffect, ParticleSet, ParticleTriShape, ParticleBillboard | Emitter slots +0..+0x24; event Apply (+4) / Parse (+0xc); EmitterEffect Start +0xa8, Stop +0xac, GetMinEmitTime +0xb4, GetVelocity +0xb8, GetEmitterGroup +0xbc; ParticleSet +0/+4/+8; ParticleTriShape Render +0x88, InitBuffers +0xa0, SetActiveCount +0xa4, BuildVertices +0xa8; ParticleBillboard Init +0x10 |
| Callees of Emitter::Update and Stop | SequenceUpdate, CreateParticle, SampleVectorRange, SampleInt/FloatRange (split by the 0xc00 x87 truncation constant), ReleaseParticle |
| Callees of SequenceUpdate with no other caller | Particle::Update (has callees), RunEvents |
| NextU32 callers/callee, `__time64`, 0x6c078965, 0x5d588b65 | MersenneTwister NextU32, Twist, Seed, SeedByArray, SeedFromClock |
| CompileScript callee calling `_isspace`; user of "EMITRADIUS" | Tokenize, ClassifyToken, AddToken |
| Unique strings of each parser | CompileSystemBlock, CompileBlock, CreateEvent, ParseCelSetup, ParseCollision, ParseVector, ParseColor, ParseBool, ParseParticleType, ParseBlendMode, ParseTime |
| Callees of the parsers calling `_atof` / `_atol` / Twist / themselves | ParseRange, ParseIntRange, ResolveEventTimes, SortEvents |
| "Can't compile emitter script for " and its caller chain | EmitterGroup::LoadEmitter (probable), CreateEmitter, ParticleSystemManager::LoadEmitterGroup |
| UOParticleEffectChaos::Init callees | UOEffects::LoadChaosEmitterEffect, ParticleSystemManager::CreateEmitterEffect, ParticleSet::CreateSequenceRenderer, renderer constructors |
| UpdateActiveEffects callee with virtual calls +0x14 and +0x1c | ParticleSystemManager::Update, Emitter::CountParticles |
| BuildVertices callee with BuildVertices as only caller | ParticleTriShape::SetCelTexCoords |

Round 7 (`drawOrderAndMaterials()` in `NameEffectCode.java`; table in `uosa_effects.md`, "Round 7") adds
`UOEffects::InitParticleSystemRoot` 0x004216b0 (probable), `ParticleSystemManager::LoadTexture` 0x008a8630
and `LoadMesh` 0x008a8440, `ParticleSet::UnshareAnimatedGeometry` 0x008b2820, `ParticleBillboard::SetBound`
0x008c6300 and `MeshParticle::SetActiveCount` 0x008c6520 (both with prototypes in `effects_variables.spec`),
`NiSourceTexture::Create` 0x0068a8b0 and the labels `NiSourceTexture_ms_kDefaultPrefs` and
`g_UOTextureLoadDirectHint`.

## Open questions

- **Gravity axis.** `DAT_00de2810` is copied from column 2 of the 3×3 at `DAT_00e37224+0x30`. That is also
  the matrix the effect node takes as its local rotation. Whether it is (0,0,1) in every view was not
  checked in a running client.
- **Rotation sign** of rotated POINT quads (`MakeRotation(-rot.y, D)`): probable, compare in the client.
- **Blend enum → D3D** mapping assumed from Gamebryo; the DX9 renderer was not traced.
- **Settled:** MeshParticle look (the NIF's own properties, no particle colour), quad texture address mode
  and filter, draw order (render group 7 sort, one item per sequence), texture mips.
- **CreateParticle pool** in EDX is read behind the cap check, so the spec leaves it out (see the spec
  comment). The behaviour is clear in the assembly.
- `EmitterEffect::GetMinEmitTime` (+0xb4) returns the first emitter's MinEmitTime max, or 1.0. Its caller
  was not identified.
