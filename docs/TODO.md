
# TODO

- Update systems log messages.
- Update CODE_OVERVIEW.md.
- Split the wgsl shader into multiple files.
- Further optimize uocf texmap loading (complete SIMD code).
- Move the default shader preset inside the toml file (fn setup_uniform_state and create_land_chunk_material)
- Adapt 'far' projection parameter to zoom level and window size.
- Hot reload settings and presets.
- Maybe load the full texture data in memory at startup, and keep a LRU cache only for art tiles in the texture atlas.
- Inject custom logger into uocf crate, so that it can use the same logging system as dynamapper.
- Tile Atlas Paging, LRU paging system for terrain metadata.
- Residency Synchronization: Add parent_map_id tracking to LCMesh  to ensure correct facet identification during world traversal and cleanup.
- Texture Array Expansion: Optimize TileAtlas  to dynamically resize its VRAM allocation as needed, reducing initial memory overhead while supporting large view distances.