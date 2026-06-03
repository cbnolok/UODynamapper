# udd-assets

`udd-assets` provides runtime readers and package access helpers for converted
`.uddp` assets. It is used by `dynamapper` at runtime and by tools that need to
read already-converted packages (e.g. `uddp-inspector-gui`, `udd-conv` during
validation).

## Responsibilities

- Typed package wrappers for each runtime asset type: `MobileAnimCcPackage`,
  `MobileAnimEcPackage`, `GumpsPackage`, `AtlasCacheOptions`, and the tilemeta
  readers (`TileMetaLandTile`, `TileMetaItemTile`).
- Lazy/cached atlas page decoding with configurable cache options.
- BC7-aware texture decoding for atlas pages (via `udd-image-codecs`).
- KDL-based runtime asset configuration reading (`knuffel`).
- Helpers for locating and opening named entries by virtual path hash.

## Features

- `bc7-encode`: re-exports `udd-image-codecs/bc7-encode`. Needed only if the
  caller encodes BC7 data at runtime (uncommon).

## Notes

- This crate depends on `udd-container` for package I/O, `udd-image-codecs` for
  decoding, and `uocf` for tile metadata types shared with the source parsers.
- `dynamapper` is the primary consumer at runtime. Tools use it mainly for
  inspection and validation.
