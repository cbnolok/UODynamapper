# Phase 2: UDDP/UDDF Streaming, Compression, and Live Editing

Phase 2 defines the runtime I/O architecture and the edit persistence model. The goal is constant frame pacing under movement and edits, not just high average throughput.

Hard targets:
- No main-thread blocking on package reads or decode.
- Batched partial texture writes per frame (never per-object immediate writes).
- Predictable RAM/VRAM residency under long sessions.

## 1. Package Architecture

## 1.1 UDDP Container (Package)

UDDP is the multi-entry runtime container based on the UOP-style organization.

Recommended logical order:
1. Header
2. Dictionary Section Index
3. Dictionary Blobs
4. Entry Index / Bookkeeping
5. Data Blocks

All sections should be explicitly offset-addressable.

Mandatory per-entry fields:
- `content_id: u16`
- `codec_bits: u16`
- `data_offset: u64`
- `compressed_size: u32`
- `raw_size: u32`
- `checksum/hash: u32 or u64`

## 1.2 UDDF Wrapper (Single Payload)

UDDF wraps one logical payload for tooling and modular artifacts.

Recommended header fields:
- `magic = "UDDF"`
- `version: u32`
- `flags: u64/u96/u128`
- `payload_format: u32`
- `payload_offset: u64`
- `payload_size: u64`

Alignment:
- Align `payload_offset` to 256 bytes minimum (4 KB preferred for file mapping friendliness).

## 1.3 Compression Field Encoding (`u16`)

Packed enum layout (mutually exclusive fields, not bitmask composition):
- bits `0..3`: base compression (`0..15`)
- bits `4..6`: custom compression (`0..7`)
- bits `7..15`: content id (`0..511`)

Decode helpers:
- `base = flags & 0x000F`
- `custom = (flags >> 4) & 0x0007`
- `content_id = (flags >> 7) & 0x01FF`

`content_id` drives postprocess path (for example swizzle/unshuffle).

## 2. Compression Policy by Payload Class

## 2.1 Tiny Payloads (~1-8 KB)

- Use Zstd + per-content dictionary.
- Dictionaries are generated at save time, stored in dictionary sections.
- Decompression uses prepared decoder dictionaries cached at package open.

## 2.2 Mid Payloads (~16 KB, e.g. 64x64 map chunks)

- Default Zstd level 1-3, no dictionary by default.
- Evaluate LZ4 when decode latency dominates and ratio is acceptable.

## 2.3 Precompressed GPU Payloads (BC7)

- Do not apply secondary compression unless benchmark proves clear gain.
- Preferred path: store raw BC7 blocks for direct GPU upload readiness.

## 2.4 Practical Rule

- Dictionaries are mandatory for very small repetitive payload families.
- Dictionaries are optional for medium chunks.
- Dictionaries are generally pointless for already compressed BC formats.

## 3. Chunking and Transport Unit

Streaming transport unit is decoupled from legacy 8x8 logical map block.

Baseline transport unit:
- `64x64` tiles.

Rationale:
- Better I/O and decode amortization than 8x8.
- Better frustum granularity than 128x128 in memory-sensitive scenarios.
- Better upload scheduling control under strict per-frame budgets.

## 4. Runtime Streaming Pipeline

## 4.1 Demand Generation

Input:
- Camera frustum intersection against tile grid.
- Prefetch ring around visible footprint.

Priority tiers:
1. Visible now.
2. Near ring.
3. Far ring.

## 4.2 Async Read/Decode

- Use Bevy task pools for all reads and decode work.
- Resolve entry metadata from in-memory package index.
- Decode based on `codec_bits + content_id` pipeline.
- Emit decoded payload descriptors into upload queue.

No direct GPU calls from gameplay systems.

## 4.3 Upload Scheduler (Mandatory Design)

Upload queue item fields should include:
- destination texture id
- array layer
- origin `(x, y)`
- extent `(w, h)`
- pixel format
- source bytes handle
- priority

Per-frame flush behavior:
- group by destination texture/layer
- coalesce adjacent writes when valid
- apply hard cap (`max_ops_per_frame`, `max_bytes_per_frame`)
- carry over remainder by priority

This is required to avoid API/driver overhead from many small `queue.write_texture` calls.

## 5. RAM/VRAM Caching

## 5.1 Decoded RAM Cache

- LRU keyed by logical chunk/content key.
- Hysteresis timer before eviction.
- Prevent immediate eviction/reload oscillation near camera boundaries.

## 5.2 VRAM Residency Cache

- LRU for atlas slots/pages/layers.
- Eviction is bookkeeping-first; overwrite on next allocation.
- Optional periodic defrag path if fragmentation exceeds threshold.

## 5.3 Required Telemetry

- RAM cache hit/miss.
- VRAM residency hit/miss.
- Evictions/s.
- Queue backlog depth.
- Upload bytes/frame.

## 6. Live Editing Model

## 6.1 Runtime Mutation

- Edits modify metadata texture/region via minimal partial write.
- Dirty region tracker records changed coordinates and affected LOD regions.

## 6.2 Mipmap Consistency

- Base-level edit invalidates higher mips in affected region.
- Rebuild with compute or targeted CPU path, bounded by frame budget.

## 6.3 Persistence Strategy

- Runtime delta map keyed by tile coordinate stores edits.
- Read path merges base UDDP data + in-memory delta.
- Save path either:
	- writes patch/delta file, or
	- rebuilds package offline on explicit command.

## 7. Performance Budgets and Acceptance

## 7.1 Budget Controls

Configurable hard limits:
- `decode_jobs_max_in_flight`
- `upload_max_ops_per_frame`
- `upload_max_bytes_per_frame`
- `prefetch_radius_tiles`

## 7.2 Acceptance Criteria

- No sustained frame-time spikes during normal movement on GTX 1060 baseline profile.
- Stable frame pacing during continuous camera sweep with mixed visible/prefetch pressure.
- Queue backlogs converge after transient movement spikes.

## 8. Platform Constraints

- Core path: Bevy + wgpu + WGSL only.
- No reliance on advanced vendor-only runtime features.
- Design must remain portable across baseline desktop GPUs at or above GTX 1060 class.
