# udd-container

`udd-container` is the low-level infrastructure crate for `.uddp` and `.uddf`
package files. It handles file I/O, compression, hashing, and the binary
container layout.

## Responsibilities

- **Package reading**: `UddpReader` and `UddpReaderOptions` for memory-mapped
  package access and entry enumeration.
- **Package writing**: package image assembly and atomic rewrite.
- **Compression**: per-entry codec dispatch. Supported codecs: raw
  (uncompressed), zstd (plain or with a per-type dictionary), LZ4 HC, and the
  BC7 field split followed by zstd or LZ4 HC (`bc7_fields`, codecs 4 and 5,
  whose pages a reader can keep split and join per rectangle with
  `FieldsPage`). A package that names another codec, or sets a reserved
  locator bit, fails to open.
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
