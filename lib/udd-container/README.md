# udd-container

`udd-container` is the low-level infrastructure crate for `.uddp` and `.uddf`
package files. It handles file I/O, compression, hashing, and the binary
container layout.

## Responsibilities

- **Package reading**: `UddpReader` and `UddpReaderOptions` for memory-mapped
  package access and entry enumeration.
- **Package writing**: package image assembly and atomic rewrite.
- **Compression**: per-entry codec dispatch. Supported codecs: zstd,
  JPEG XL (for applicable texture data), and raw (uncompressed).
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
- `jpegxl-rs` links the system libjxl, or builds it from source with the `vendored-jpegxl` feature. libjxl's
  parallel decode and encode work runs on the caller's rayon pool, never on threads of its own.
- This crate has no dependency on `uocf` or any UO-specific logic; it is a
  pure container layer.
