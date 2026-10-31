# Classic static translucency

## Original executable evidence

Read-only inspection of `classic_7.0.103.0_client.exe` (SHA-256
`4d857f2f5f8bf350bc50a87ee66b2236bfa480e712e9546c740ff04f9106690a`)
locates ordinary object art dispatch at `0x0044e3a0`. The tiledata array pointer
is `0x00afb748`, with 0x30-byte in-memory records. The dispatcher retains flags
`0x60008`: Translucent 0x8, Foliage 0x20000 and PartialHue 0x40000. Ordinary
Translucent art selects `0x005d8000`; Foliage+Translucent selects `0x005d8050`.
PartialHue keeps the same weighting distinction. Hue and explicit object overrides have
additional paths and must not be mistaken for the ordinary flag decision.

The two targets are valid code labels without defined Ghidra functions. Bounded
`objdump` disassembly resolves their instructions without creating functions or
changing the shared Ghidra program. At `0x005d8000`, the loop reads source from
EDI and destination from ECX, then performs this integer operation per pixel:

```
output = ((((destination & M2) + (source & M2)) >> 1)
          + (source & M1)) >> 1;
output |= 1;
```

`0x005d8050` swaps source and destination in that expression. The format masks
at `0x007469f0`/`0x007469f4` clear one/two low bits per channel respectively
(the RGB555 setup writes 0x7bde/0x739c). The continuous-weight equivalents are
3/4 source + 1/4 destination, and 1/4 source + 3/4 destination for Foliage.
The original also truncates 5-bit channels and forces the packed low bit;
GPU alpha blending with those weights does not reproduce that quantization
or establish identical gamma-space composition.

The discovery route was: scan constants and tiledata pointer references in the
mapped PE, recover the complete art dispatcher, follow its selected code
labels, and inspect bounded assembly. Searching only for 0xb2 or float 0.7
did not identify this path. The existing `pe_inspect.py` helper maps file
offsets to virtual addresses; executable code was never run.

For comparison, the independently maintained
[ClassicUO ProcessAlpha](https://github.com/ClassicUO/ClassicUO/blob/main/src/ClassicUO.Client/Game/Scenes/GameSceneDrawingSorting.cs)
uses 178/255 for Translucent. That implementation is useful comparative evidence,
but its constant is not the original blitter's weighting.

## Renderer implications and scope

Before this repair, the collector emitted sprite instance opacity 1. It now
carries the selected world/legacy source into collection and applies the original
weight only to ordinary legacy-source sprites, including flat CC art. The
collector and picker share the opacity rule. `art/main.wesl` uses image alpha
multiplied by instance opacity for pass admission, routing explicit translucency
through the blended pass. Merely changing instance color would have left those
texels in the opaque pass. Explicit translucency also bypasses the existing
fractional-edge RGB adjustment, preserving straight color for standard alpha
blending. Projected shadow opacity receives the same multiplier.

EC authored material surfaces already compose image masks and keep their
existing path. New world-art pixels do not inherit this legacy opacity rule.
Flags, raw Z, graphic identity and movement metadata remain intact.

## Source and native validation

The map-0 `(1445,1790)` isolated wave-like mark belongs to graphic 6061 at
`(1447,1790,-5)`, packed CC slot 22445. App-only probes at screen `(675,505)`
identify that owner; the earlier `(650,450)` probe hit neighboring water and
must not be used as ownership evidence. Original UOP art 6061 contains the
detail and exactly matches its packed 44x44 RGBA pixels. Its raw CC flags are
524481, without Translucent. A radius-18 census contains water statics
6038..6064 and no wave 8099..8138. No coordinate exception, generic fill or
animation-frame change is justified by this mark.

Positive translucency fixtures are graphic 8109 at `(5423,1490,0)` and 8134 at
`(5423,1491,1)`, both hue 33. Here the historically named wave sprites are red
dungeon effects. Their original CC source RGBA also matches packed pixels.
The current EC-world source for these owners is a separate 8x8 placeholder;
legacy opacity must not be applied to that source.

Twenty-four matched native frames from eight verified Desktop-4 runs use the
same mandatory-trim EC art/tilemeta pair on both sides of the opacity change.
Classic CC, Enhanced EC-land/CC-art and Enhanced CC each change 3700 pixels by
more than one channel unit at the positive fixture (maximum channel differences
83, 96 and 96). Their coast and bridge `(2834,987)` controls are byte-identical.
KR EC-art controls are byte-identical at all three fixtures. These establish
source-gated opacity and pass reachability, not exact original 5-bit color
composition. The 98 focused art tests pass; two adapter-dependent tests remain
ignored. Native runs compile and exercise the production WESL passes.

Research artifacts are under `/tmp/uodynamapper-ec-followup/water-alpha-*`,
`water-classic-art-blitters.txt`, `water-mark-source-pixels/` and
`water-classic-source-pixels/`. Matching-pair captures use `trim-water-*`.
