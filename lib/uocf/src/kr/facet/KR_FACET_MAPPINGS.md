# Kingdom Reborn Facet Mappings

This directory contains the ID translation layer for Kingdom Reborn (KR) facet data. Because KR uses a different indexing system for terrain and statics compared to the classic client, these components allow the engine to map KR data back to classic IDs (and vice-versa).

## 1. The Dictionaries (`.bin` files)

These are binary serialized lookup tables used at runtime for high-performance translation:

*   **`TileDictionary.bin`**: A mapping of KR Land Tile IDs $\rightarrow$ Classic Land Tile IDs. It ensures that when you load a KR map, the engine knows which classic texture/properties to apply to the terrain.
*   **`StaticDictionary.bin`**: A whitelist of valid Classic Static IDs. During KR facet decoding, the engine uses this to filter out statics that don't have a known classic counterpart or to verify classic IDs stored within KR's complex format.

## 2. The Logic (`tile_mappings*.rs`)

*   **`tile_mappings_builder.rs`**: This is the **source of truth**. It contains massive hardcoded vectors and HashMaps (over 3,500 lines of code) defining the ID relationships. 
    *   It is used as a "compile-time generator" or by a separate utility to produce the `.bin` files. 
    *   Keeping this as a separate file prevents the main application from having to compile 164KB of hardcoded data every time, as loading the `.bin` files is much faster and lighter.
*   **`tile_mappings_loader.rs`**: This is the **runtime utility**. It provides the functions (`load_tile_dictionary`, `load_static_dictionary`) that actually read the `.bin` files from disk and reconstruct the in-memory HashMaps used by `facet_decoder.rs` and `facet_encoder.rs`.

## Summary Workflow

1.  **Define**: Map IDs manually in `tile_mappings_builder.rs`.
2.  **Serialize**: Generate `StaticDictionary.bin` and `TileDictionary.bin` (using the builder).
3.  **Load**: The app uses `tile_mappings_loader.rs` to load the `.bin` files into memory at startup.
4.  **Translate**: `facet_decoder.rs` uses these loaded maps to convert KR binary sectors into classic 8x8 blocks.
