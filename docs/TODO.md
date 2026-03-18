
# TODO

- Split the wgsl shader into multiple files.
- Adapt 'far' projection parameter to zoom level and window size.
- Hot reload settings and presets.
- Maybe load the full texture data in memory at startup, and keep a LRU cache only for art tiles in the texture atlas.
- Inject custom logger into uocf crate, so that it can use the same logging system as dynamapper.
- Reduce idle CPU further by making more render/cache systems fully event-driven.
- Reuse temporary buffers and collections in chunk visibility, map-block loading, and atlas upload paths.
- Optimize UO map block acquisition and in-memory storage (faster caches, less cloning, background prefetch).
- Consider memory-mapped map/texture file access for lower seek overhead at large zoom-out levels.
