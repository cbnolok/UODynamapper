# udd-container

`udd-container` is the low-level infrastructure crate for `.uddp` and `.uddf`
package files. It handles file I/O, compression, hashing, and the binary
container layout.

## Responsibilities

- **Package reading**: `UddpReader` and `UddpReaderOptions` for memory-mapped
  package access and entry enumeration.
- **Package writing**: package image assembly and atomic rewrite.
- **Compression**: per-entry codec dispatch. Supported codecs: zstd (plain or
  with a per-type dictionary) and raw (uncompressed). Codec id 3, lossless
  JPEG XL until its removal on 2026-11-06, stays reserved: a package that still
  uses it fails to open, naming the package, and has to be repacked with the
  current `udd-cli`.
- **Hashing**: `xxh64_virtual_path` for path-addressed entry lookup
  (xxHash-64 over the virtual path string).
- **Metadata helpers**: `unpack_codec`, `unpack_offset40`, `unpack_type`,
  `reconstruct_stored_size`, and `FileKey` for working with packed entry
  metadata fields.

## Format

The `.uddp` container format is documented in
[docs/dev_wiki/UDDP_FORMATS.md](../../docs/dev_wiki/UDDP_FORMATS.md).

## Notes

- `memmap2` is used for large package reads.
- This crate has no dependency on `uocf` or any UO-specific logic; it is a
  pure container layer.
