# Texture Residency Architecture

This document explains the shared residency-planning layer under `dynamapper/src/core/texture_cache/residency.rs` and how terrain currently uses it.

---

## 1. Why This Exists

The project now has at least two distinct questions for texture collections:

- should a collection behave like an on-demand LRU cache?
- should it reserve fixed GPU layers for the full collection at startup?

Terrain texmaps already need this switch through `texmaps_preload_full_file`, and future texture families will likely need the same choice. The shared residency layer exists so those decisions are not reimplemented independently in each collection-specific module.

---

## 2. Shared Concepts

### 2.1 `TextureResidencyStrategy`

The strategy enum is intentionally small:

- `LruCache`: keep only a working set resident and rely on eviction / growth
- `PreloadFullCollection`: assign fixed layers to every planned texture ID at startup

This enum does not perform uploads by itself. It only decides which residency path the collection-specific cache should use.

### 2.2 `TextureResidencyPlan<Group>`

The plan is a collection-agnostic grouping of texture IDs.

- `Group` is chosen by the collection
- each bucket stores `Vec<u16>` texture IDs for one group
- insertion order is preserved so preload layer assignment is deterministic

Examples of possible group types:

- terrain: `LandTextureSize::{Small, Big}`
- art: separate arrays by source dimensions or alpha policy
- animations: split by body family, frame format, or upload target

### 2.3 `TextureResidencyGroupLayers<Group>`

This struct describes the layer policy for one group:

- `initial_layers`: normal LRU startup capacity
- `max_layers`: hard safety ceiling
- `debug_name`: human-readable assertion/log label

### 2.4 Shared Helpers

The residency module currently provides three reusable helpers:

1. `resolve_layer_allocations()`
   Chooses the final layer count for each group depending on the active strategy.

2. `visit_grouped_texture_ids()`
   Walks grouped texture IDs in a caller-defined stable group order.

3. `visit_grouped_layer_assignments()`
   Walks `(group, texture_id, layer)` tuples for deterministic preload layouts.

These helpers deliberately stop at planning and ordering. They do not know how bytes are decoded, compressed, or uploaded.

---

## 3. Terrain Integration

Terrain currently plugs into the shared layer as follows:

1. `land/texture_array.rs`
   Builds a `TextureResidencyPlan<LandTextureSize>` by scanning `TexMap2D`.

2. `land.rs`
   Defines the two terrain groups (`Small`, `Big`) and resolves startup layer counts through `resolve_layer_allocations()`.

3. `land/cache.rs`
   Uses the grouped traversal helpers to:
   - warm source texmap data in preload mode
   - assign permanent layer indices
   - schedule upload tasks in the same deterministic order

The land cache remains responsible for terrain-specific behavior:

- texmap byte lookup
- BC7 / RGBA encoding decisions
- GPU array ownership
- LRU bookkeeping and eviction rules

---

## 4. Design Boundary

The current abstraction boundary is intentional:

- shared layer: planning, grouping, deterministic ordering, layer budgeting
- collection layer: loading bytes, choosing formats, maintaining GPU/cache state

This keeps the shared code reusable without forcing every texture collection into a single monolithic cache implementation too early.

---

## 5. Expected Next Step

If more collections adopt the same execution model, the next extraction should be a generic grouped texture-array residency core that reuses:

- fixed-residency bookkeeping
- grouped LRU bookkeeping
- async upload scheduling patterns

At that point, collection-specific modules would mainly provide adapters for:

- group enumeration
- source data fetch
- upload encoding
- per-collection eviction rules
