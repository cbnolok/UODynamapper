# BC7 alpha plane search

Modes 4 and 5 of BC7 can give one channel its own weight plane, which suits a sprite's edge: alpha falls from solid to
nothing while the colour stays put. `pack_bc7_rgba` rarely used that for alpha, for two reasons:

- `detect_dp_channel` offers a channel its own plane only when that channel's correlation with each of the others is
  under 0.35. On a sprite's edge the source holds transparent black under its transparent pixels, so alpha falls together with
  colour and the correlation is high, exactly where the plane helps most.
- Where the plane was offered, candidates were compared by the error `pack_mode4_or_5` estimates, not by decoding them.

Now, as its last step, `pack_bc7_rgba` tries alpha's own plane on every block whose alpha varies, decodes that block and the
search's best one through `decode_bc7_error_bounded`, and keeps the closer. Fully opaque blocks return to `pack_bc7_rgb` first and
solid blocks take the solid fast path, so neither changes. The work behind this is "fix C" in section 12 of
[the codec proposal](proposals/2026-11-01-mobile-anim-codec.md). "Fix A", the `encode_mode4` field order, landed first:
before it, mode 4 blocks decoded differently from what the search had scored.

`alpha_plane_is_kept_when_it_decodes_closer_on_sprite_edges` measures both blocks through the `dds` decoder rather than the encoder's own.
It failed before this change.

## What it changed

Real packs against the lossless package, sprite by sprite, fix A then this change. Both ran on 2026-11-05.

| Package | Sprites | Painted RGB dB | Alpha dB | Per-sprite RGB dB: median | p5 | min |
|---|---:|---:|---:|---:|---:|---:|
| `tex_art_cc --bc7` | 39196 | 28.02 → 33.04 | 31.08 → 57.22 | 29.01 → 35.61 | 20.34 → 25.96 | 13.12 → 13.62 |
| `tex_art_ec --art-bc7` | 12524 | 38.46 → 45.56 | 38.05 → 49.64 | 35.73 → 49.46 | 25.34 → 35.46 | 15.18 → 22.26 |

Without `--zstd` a BC7 page has a fixed size, so these files didn't change size: 239416483 and 84234006 bytes.

Whole animation set, SSIMULACRA2 on body mosaics, from the proposal's scratch copy of the same lines:

| Client | Median | Bodies at 80 or better | zstd 19 disk |
|---|---:|---:|---:|
| Enhanced | 69.99 → 85.40 | 163 → 712 of 926 | −0.7 % |
| Classic | 68.37 → 87.51 | 193 → 782 of 1022 | +2.6 % |

Encoding goes from about 0.05 to 0.14 worker seconds a megapixel on animation frames. Packing `tex_art_cc` went from 2 s to 3 s;
`tex_art_ec` stayed at 4 s.

## What it leaves

- Packages keep their old blocks until rebuilt from source: any built with `--bc7` before 2026-11-05. `--jxl` pages don't pass through
  this encoder.
- The comparison has no weight for what shows. BC3 still keeps edges closer on Enhanced: 0.211 levels of error against this
  encoder's 0.628.
- The colour step, which stores colour under transparent pixels, is a separate choice and is still undecided.
- The subset encoders still have the field bugs listed in section 12 as "bugs B". The converter's flags never reach them, but
  `benches/bc7_encode.rs` does, so comparisons made through those flags read wrong blocks.
- The BC7 RDO rows in `docs/TODO.md` ran before this change and should be measured again before anyone relies on them.
