# udd-image-codecs

`udd-image-codecs` provides GPU texture codec support for the UODynamapper
conversion pipeline. It is feature-gated so that callers only compile what they
need.

## Features

### `bc7-encode` (default: off)

SIMD-assisted BC7 analytical encoder and decoder, translated and incrementally
optimized from the basis-universal implementation.

- Full BC7 mode support with analytical block encoding.
- SIMD acceleration via the `wide` crate (SSE2 / AVX2 / NEON-compatible).
- Parallel encoding via `rayon`.
- **BC7 RDO pass** (Rate-Distortion Optimization): deliberately allows tiny,
  bounded visual changes to make neighboring BC7 blocks more compressible by the
  outer package compressor (zstd). This keeps textures GPU-ready while improving
  disk size and streaming locality at no extra VRAM cost.

The encoder is inspired by:
- [basis-universal](https://github.com/BinomialLLC/basis_universal/tree/master/transcoder)
  for the analytical BC7 codec.
- [bc7enc_rdo](https://github.com/richgel999/bc7enc_rdo) for the RDO pass.

### `ktx2` (default: off)

KTX2 container writing via `libktx-rs`. Used by the `pack-radar` command in
`udd-pack` and other paths that need GPU-ready KTX2 output.

## Notes

- When neither feature is enabled the crate is essentially a stub; it compiles
  with no optional dependencies.
- The BC7 decoder is always available (no feature gate) for reading DDS/BC7
  blocks during inspection and conversion.
- See [docs/dev_wiki/impl_bc7/](../../docs/dev_wiki/impl_bc7/) for detailed
  technical analysis of the BC7 and RDO implementation.
