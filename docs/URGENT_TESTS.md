# Urgent Unit Test Plan

Based on the current codebase and the 4-Phase roadmap, the following unit tests are considered **critical/urgent** to prevent regressions and core architectural bugs.

---

## 1. Metadata Indirection & Bit-Packing (Top Priority)
The `uvec4` metadata texture is the single point of failure for the entire Phase 1-3 rendering. Any bit-shift error will result in garbled textures or misplaced assets.

- **[NEW] `test_metadata_packing_roundtrip`**: 
  - Define a struct `TileMetadata` in Rust that mirrors the shader fields.
  - Implement `pack()` and `unpack()` in Rust (using identical logic to the future shader).
  - Assert that `unpack(pack(data)) == data` for thousands of randomized inputs, especially covering:
    - Max values for `layer_idx` (e.g., 2048 layers).
    - Max/Min signed values for `offset_x/y` (-128 to 127).
    - Bitmask flags.

---

## 2. UDDP v2 & mmap Safety
Since UDDP v2 relies on 4KB page alignment and memory mapping, any off-by-one error in byte offsets will cause `Zstd` decompression failures or segmentation faults.

- **[NEW] `test_uddp_alignment_verification`**:
  - Mock a UDDP file with randomized block sizes.
  - Assert that every `data_block_address` in the generated file is a multiple of 4096.
- **[NEW] `test_mmap_block_access`**:
  - Use `memmap2` to map a test UDDP.
  - Verify that `UopPackage` can correctly read a block from the middle of the file without loading the entire file into a `Vec<u8>`.

---

## 3. Isometric Coordinate & Depth Math
The "Z-Order" of UO is notorious for corner cases. We must validate the mathematical model before committing it to a shader.

- **[NEW] `test_iso_depth_sorting`**:
  - Verify that the formula `depth = (x + y) * 22.0 - z * 4.0` creates a consistent order.
  - Assert: `depth(x+1, y, z) > depth(x, y, z)`.
  - Assert: `depth(x, y+1, z) > depth(x, y, z)`.
  - Assert: `depth(x, y, z+1) < depth(x, y, z)`.
- **[NEW] `test_subtile_precision_bias`**:
  - Verify that adding the `0.001` epsilon for paperdoll layers (Phase 4) doesn't swap depth order when two mobiles are on adjacent tiles.

---

## 4. SIMD RLE Decoders (anim.mul / art.mul)
RLE decoding is a high-risk area for buffer overflows.

- **[NEW] `fuzz_rle_decoder`**:
  - Create a test bench that feeds the `decode_static_tile` with various RLE sequences.
  - Include "malformed" RLE (e.g., runs that exceed the proclaimed width).
  - Ensure the decoder returns an `Err` instead of panicking or writing out of bounds.

---

## 5. Hue Palette Integration
- **[NEW] `test_hue_lookup_logic`**:
  - Mock the 1024x32 palette texture.
  - Verify that a given `(color_ramp, hue_id)` pair correctly retrieves the expected RGBA value from a known test palette.
