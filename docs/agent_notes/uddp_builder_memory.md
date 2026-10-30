# UDDP builder memory

- `UddpBuilder` keeps every payload in memory until it builds, because type dictionaries are
  trained over all files. Packers that produce gigabytes of payloads (`mobile_anim_ec.uddp` is
  about 6.9 GB) should use `UddpStreamingBuilder`. It compresses each file when it is added and
  appends the stored bytes to `<output>.payloads.tmp`. `finish()` writes the head and copies the
  spilled payloads, and the output is byte-identical to `UddpBuilder` for the same files in the
  same order. It rejects `ZstdDict` and `Auto`. Use `add_owned_files` so a batch compresses in
  parallel.
- `UddpBuilder::into_file*` writes the package one payload at a time, so the whole image is never
  held in memory twice. `build()`/`build_with_progress()` (non-consuming) copy raw payloads.
  `udd_conv::package_progress::build_and_write_package*` take the builder by value and use
  `into_file`.
- `uocf` `UopPackage::unpack_file_by_hash_cached` keeps every loaded compressed payload in the
  package for its whole lifetime, with no bound. For a one-pass scan, use
  `unpack_file_by_hash_from_reader` with one reader from `open_payload_reader()`.
