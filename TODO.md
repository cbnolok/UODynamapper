
# TODO

- Update systems log messages.
- Update CODE_OVERVIEW.md.
- Split the wgsl shader into multiple files.
- Investigate the lag occurring while loading new chunks (related to the loading/updating of the texture array?).
- Add a global uniform buffer (not per mesh material) with lighting info.
- Further optimize uocf texmap loading (complete SIMD code).
- Move the default shader preset inside the toml file (fn setup_uniform_state and create_land_chunk_material)
- Adapt 'far' projection parameter to zoom level and window size.
- Hot reload settings and presets.
- Maybe load the full texture data in memory at startup, and keep a LRU cache only for art tiles in the texture atlas.
- Inject custom logger into uocf crate.
