# UOClient-stefanomerotta: Rendering Paths & Techniques — Reproduction Guide

> How this C# EC client renders terrain and statics, from CC tile ID to final pixel, and how to reproduce each technique in another engine.
>
> Third-party reconstruction reference, not the original EC client or its format
> contract. The executable-derived renderer/UDDP rework supersedes using these
> techniques as implementation requirements. Start with
> [the EC evidence map](EC_WORLD_SPACE_UV_EVIDENCE_MAP.md) for original-client
> contracts and current runtime behavior.

---

## 0. Coordinate System & Geometry Conventions

The world is a grid of tiles. Each tile has integer `(X, Y)` world coordinates and a `Z` height (signed byte, range -128..127).

**Vertex format** (for all terrain types):
```
struct TerrainVertex {
    Vector3 Position;    // (worldX, Z_height, worldY)  — Y-up, XZ-plane is ground
    ushort  TextureIndex; // = tile.Id, the EC canonical terrain type id
}
```

The vertex declaration passes `TextureIndex` as `TEXCOORD0` (a `float`, cast to `uint` in the shader). There are no explicit UV coordinates in the vertex — **all UVs are derived from world position in the shader**.

**Quad assembly**: Each tile is a quad formed by 4 corner vertices (topLeft, topRight, bottomLeft, bottomRight) using two triangles (TL→TR→BL, BL→TR→BR). Index buffers are 16-bit (`short`).

---

## 1. Runtime Data Flow Overview

```
terrain.bin                           terraindata.bin                    terraintextures.bin
  │ per tile: {Id, Z}                   │ 3 arrays indexed by Id          │ DDS textures by numeric id
  │                                     │                                 │
  ▼                                     ▼                                 ▼
TerrainFile.FillBlock()              TerrainsDataFile.Load()            TerrainTextureFile.ReadTexture()
  │                                     │                                 │
  ▼                                     ▼                                 ▼
TerrainBlock                         TerrainData                        Texture2D cache (per sub-renderer)
  │ .Tiles[65×65]                       │ .Solid[]/Liquid[]/Single[]       │
  │ .Indices[perType]                   │ .Types[id] → Solid/Liquid/Single │
  │                                     │                                 │
  └──────────────────┬──────────────────┘                                 │
                     ▼                                                    │
              TerrainRenderSystem                                         │
                     │                                                    │
         ┌───────────┼───────────┐                                        │
         ▼           ▼           ▼                                        │
    SolidSubR.   LiquidSubR.  SingleSubR.  ◄──────────────────────────────┘
         │           │           │
         ▼           ▼           ▼
    solid-terrain.fx  liquid-terrain.fx  single-terrain.fx
```

---

## 2. Block & Index Buffer Architecture

### Block loading (CPU, async thread)

1. `TerrainLoaderSystem.LoadBlock()` reads a 65×65 tile block (64×64 + 1 skirt row/col) from `terrain.bin`.
2. `TerrainBlock.Initialize()` calls `SetUpIndices()` which iterates every tile type id present in the block and builds **one index buffer per unique tile type**.

### How `SetUpIndices()` works — the key algorithm

For each unique tile type `typeId` in the block:

1. **Build a 64×64 bitmap** (`BitMapBlock64`). A bit is set at `(x,y)` if that cell OR any of its 8 neighbours has `tile.Id == typeId`. This expands the coverage by 1 tile in every direction.
2. For every set bit, emit a quad (2 triangles, 6 indices) referencing the 4 corner vertices of that cell.
3. Store the index array as `TerrainIndicesEntry { TileId = typeId, Indices = short[] }`.

**Why the neighbour expansion?** Because each terrain type is drawn as a separate pass with alpha blending. A tile of type Grass needs to cover not just its own cell but also the adjacent cells of other types, so that when Grass and Dirt overlap, the noise alpha mask can produce a smooth blend. Without neighbour expansion, there would be gaps at type boundaries.

### Result

Each block produces N index buffers (one per terrain type present). At draw time, the render system iterates these entries and dispatches to the correct sub-renderer based on `TerrainData.Types[entry.TileId]`.

---

## 3. Rendering Path A: Solid Terrain

**Used for**: Grass, dirt, rock, sand, snow, forest, jungle, all cliff variants, floor tiles, cave walls, etc.

### Data per type (from `SolidTerrainData`)

| Field | Description |
|---|---|
| `Texture0 { Id, Stretch }` | Primary detail texture + UV tiling scale |
| `Texture1 { Id, Stretch }` | Secondary detail texture for macro variation |
| `AlphaMask { Id, Stretch }` | Noise texture whose `.a` channel drives the blend |

All three are individual `Texture2D` objects loaded on demand from `terraintextures.bin` (DXT1/3/5 compressed).

### CPU-side setup per draw (`SolidSubRenderSystem.SetupDraw`)

```csharp
solid.TextureIndex = entry.TileId;              // "which type am I drawing?"
solid.Texture0 = GetTexture(data.Texture0.Id);  // bind primary texture
solid.Texture1 = GetTexture(data.Texture1.Id);  // bind secondary texture
solid.AlphaMask = GetTexture(data.AlphaMask.Id); // bind noise alpha
solid.Texture0Stretch = (int)data.Texture0.Stretch;
solid.Texture1Stretch = (int)data.Texture1.Stretch;
solid.AlphaMaskStretch = (int)data.AlphaMask.Stretch;
pass.Apply();
```

### Shader: `solid-terrain.fx`

**Vertex shader:**
```hlsl
// UV = world position / stretch factor — no explicit UVs, fully procedural
output.TexCoord.xy = position.xz / Texture0Stretch;  // primary UV
output.TexCoord.zw = position.xz / Texture1Stretch;  // secondary UV
output.AlphaMaskCoord.xy = position.xz / AlphaMaskStretch; // noise UV

// Per-vertex "ownership" flag: 1.0 if this vertex belongs to the
// terrain type currently being rendered, 0.0 if it's a neighbour tile
output.AlphaMaskCoord.z = (float)(TextureIndex <= TexIndex);
```

**Fragment shader:**
```hlsl
float4 color0 = Texture0.SampleLevel(sampler, texCoord.xy, 0);   // primary
float4 color1 = Texture1.SampleLevel(sampler, texCoord.zw, 0);   // secondary
float4 alpha  = AlphaMask.SampleLevel(sampler, alphaMask.xy, 0);  // noise

// Step 1: Blend two detail layers using noise alpha
float4 color = lerp(color0, color1, alpha.a);

// Step 2: Fade to black for non-owned tiles (alpha = 0 → transparent)
color = lerp(color, 0, 1 - alphaMaskCoord.z);
color.a = alphaMaskCoord.z;

return color;
```

### Blending across terrain types (how transitions work)

The client uses `BlendState.AlphaBlend` (standard `SrcAlpha, OneMinusSrcAlpha`). Terrain types are drawn **sequentially**, one type at a time, on the same geometry (with expanded index buffers):

```
1. Draw all Grass quads (including neighbour-expanded into Dirt tiles)
   → Grass pixels are alpha=1.0, non-Grass pixels are alpha=0.0
2. Draw all Dirt quads (including neighbour-expanded into Grass tiles)  
   → Dirt pixels are alpha=1.0, non-Dirt pixels are alpha=0.0
```

Where Grass and Dirt quads **overlap** (the expanded neighbours), both types render geometry. The noise alpha mask provides visual variation at the boundary. Because all vertices have the same world position and thus the same noise UV, the boundary is smooth and consistent from both sides.

**The inequality trick `TextureIndex <= TexIndex`**: This is a subtle ordering mechanism. If the currently-drawn type's id is ≤ the vertex's tile id, the vertex is considered "owned." This means lower-numbered types can render on top of higher-numbered types' geometry but not vice versa. This provides a deterministic layering order and prevents double-painting.

### How to reproduce in another engine

1. **Build per-type index buffers** with 1-cell expansion into neighbours.
2. **One draw call per terrain type per block.** Before each draw, bind the 3 textures for that type.
3. **Derive UVs from world position** in the vertex shader: `uv = worldPos.xz / stretch`.
4. **Use alpha blending** (`SrcAlpha, 1-SrcAlpha`) and draw types in id-order.
5. **Per-vertex ownership flag** via integer comparison on the tile id. This replaces the need for any transition tile data.

**Alternative for GPU-driven approaches**: Instead of N draw calls per block, pack all type definitions into a uniform buffer array / SSBO and do the type dispatch + texture selection in the fragment shader. The index buffer expansion technique still applies.

---

## 4. Rendering Path B: Liquid Terrain (Animated)

**Used for**: Water, lava, swamp water, acid, stars.

### Data per type (from `LiquidTerrainData`)

| Field | Description |
|---|---|
| `Texture0 { Id, Stretch }` | Albedo/diffuse texture (the "look" of the liquid) |
| `Normal { Id, Stretch }` | Normal map (scrolled over time for animation) |
| `Static { Id, Stretch }` | (unused at runtime in the shader — fallback for non-animated path) |
| `Speed` | Flow/wind speed (`WindForce` shader param) |
| `WaveHeight` | Amplitude of UV perturbation |
| `Type` flags | `FollowCenter` bit selects the shader technique |

### Animation mechanism

There is **no per-frame texture swap**. The animation is entirely procedural:

1. A **normal map** is scrolled along a wind direction by accumulating `Time * WindForce` into the normal map UV.
2. The normal map's RG channels (interpreted as a 2D perturbation vector) are used to **offset the albedo texture UV** by `WaveHeight * (normal.rg - 0.5) * 2.0`.
3. This creates a continuously moving, refraction-like distortion effect.

### CPU-side update per frame (`LiquidSubRenderSystem.Update`)

```csharp
liquid.Time = (float)gameTime.TotalGameTime.TotalMilliseconds / 100.0f;
liquid.Center = new Vector2(camera.Target.X, camera.Target.Z);
liquid.WindDirection = new(0, 1);  // hardcoded: south
```

### CPU-side setup per draw

```csharp
liquid.TextureIndex = entry.TileId;
liquid.WaveHeight = data.WaveHeight;
liquid.WindForce = data.Speed != 0 ? data.Speed : 0.1f;
liquid.FollowCenter = data.Type.Has(TerrainTileType.FollowCenter);  // selects technique
liquid.Normal = GetTexture(data.Normal.Id);
liquid.Texture0 = GetTexture(data.Texture0.Id);
liquid.Texture0Stretch = (int)data.Texture0.Stretch;
liquid.NormalStretch = (int)data.Normal.Stretch;
```

### Shader: `liquid-terrain.fx`

**Two HLSL techniques** (selected by `FollowCenter` flag):

#### Technique "Main" (absolute UV) — used for lava, acid

```hlsl
// Vertex shader
output.TexCoord.xy = position.xz / Texture0Stretch;   // absolute world UV
output.TexCoord.zw = GetNormalTextCoord(position);      // scrolled normal UV
output.AlphaMask = (float)(TextureIndex <= TexIndex);   // ownership flag
```

#### Technique "FollowCenter" (camera-relative UV) — used for water, swamp

```hlsl
// Vertex shader — UV follows camera, hides tiling near the player
output.TexCoord.xy = ((Center - position.xz) / Texture0Stretch) - 0.5;
output.TexCoord.zw = GetNormalTextCoord(position);
output.AlphaMask = (float)(TextureIndex <= TexIndex);
```

#### Normal UV computation (shared)

```hlsl
float2 GetNormalTextCoord(float4 position) {
    float2 windDir = normalize(WindDirection);
    float3 perpDir = cross(float3(windDir, 0), float3(0, 0, 1));

    float xDot = dot(position.xz, perpDir.xy);
    float yDot = dot(position.xz, windDir);

    yDot -= Time * WindForce;   // ◄── THIS IS THE ANIMATION: scroll along wind

    return float2(xDot / NormalStretch, yDot / NormalStretch);
}
```

#### Fragment shader (shared by both techniques)

```hlsl
float4 normal = Normal.SampleLevel(sampler, normalCoord, 0);
float2 perturbation = WaveHeight * (normal.rg - 0.5f) * 2.0f;

float4 color = Texture0.SampleLevel(sampler, texCoord + perturbation, 0);

// Fade non-liquid tiles (same ownership mechanism as solid terrain)
color = lerp(color, 0, 1 - alphaMask);
color.a = alphaMask;
return color;
```

### How to reproduce

1. **Two textures**: an albedo and a normal map (both seamless/tileable).
2. **Scroll the normal map UV** by `time * speed` along a wind direction vector each frame.
3. **Perturb the albedo UV** using the sampled normal's RG as a displacement vector, scaled by `waveHeight`.
4. **For camera-relative UV** (the `FollowCenter` path): compute albedo UV as `(cameraPos - worldPos) / stretch - 0.5`. This makes the texture appear to "stick" under the camera, hiding the repetition visible from the player's viewpoint.
5. **Use the same ownership/alpha mechanism** as solid terrain for type transitions.

---

## 5. Rendering Path C: Single Terrain (Texture2DArray)

**Used for**: Ornate tiles (terra cotta patterns, blue slate ornate, etc.) — tiles where a specific small repeating pattern needs to be rendered exactly, not blended.

### Data per type (from `SingleTerrainData`)

| Field | Description |
|---|---|
| `TextureId` | Index into `terraintextures.bin` for a small DXT5 texture (e.g., 64×64) |

There is no blending, no secondary texture, no alpha mask.

### CPU-side setup (once at startup)

All single-terrain textures are loaded into a **`Texture2DArray`** (one layer per canonical type id):

```csharp
textureArray = new TextureArray(device, 64, 64, terrainData.Length, SurfaceFormat.Dxt5);
for (int i = 0; i < terrainData.Length; i++) {
    int textureId = terrainData[i].TextureId;
    textureFile.FillTexture(textureId, buffer, out byte dxtFormat);
    textureArray.Add(i, buffer);  // layer index = canonical type id
}
single.Texture0 = textureArray;  // bind once, never changes
```

### Shader: `single-terrain.fx`

```hlsl
Texture2DArray Texture0;

// Vertex shader
output.TexCoord = float3(position.xz, TexIndex);
    // .xy = world position (used directly as UV — 1:1 texel-to-tile mapping)
    // .z  = texture array layer index (= canonical tile type id)

// Fragment shader
float4 color = Texture0.SampleLevel(sampler, TexCoord, 0);
return color;
```

No alpha blending, no ownership test, no special handling. The `position.xz` directly indexes the texture atlas layer at the world coordinate, giving an exact 1:1 tile pattern.

### How to reproduce

1. Pack all single-tile textures into a `Texture2DArray` (or equivalent atlas).
2. In the vertex shader, pass the world position XZ as UV and the tile type id as the array layer.
3. Sample with no blending. Draw opaque, no alpha.

---

## 6. Rendering Path D: Statics (Sprites)

**Used for**: All static objects placed on the map — trees, walls, furniture, etc.

### Data flow

Statics use a completely separate pipeline from terrain:

1. `StaticsLoaderSystem` reads statics from `statics.bin` (per-block lists of `{Id, Color, Z}`).
2. For each static, it looks up `StaticData[Id]` which provides texture atlas coordinates (EC or CC).
3. All sprites in a block are packed into a **texture atlas** (8192×8192, DXT5) using `TextureBuffer` + `TexturePacker`.
4. Geometry is billboarded quads (4 vertices per sprite), rotated 45° to face the isometric camera.

### Vertex format (statics)

```
struct StaticsVertex {
    Vector3 Position;     // world position of the tile this static sits on
    Vector4 Bounds;       // .xy = billboard offset, .zw = atlas UV
    float   DepthOffset;  // manual depth adjustment for sorting
}
```

### CPU-side render setup

```csharp
statics.Rotation = Matrix.CreateRotationY(MathHelper.ToRadians(45));
statics.Texture0 = block.Texture;        // atlas
statics.TextureSize = new(block.TextureWidth, block.TextureHeight);
```

### Shader: `statics.fx`

```hlsl
static const float3 TileTranslation = float3(0.5f, 0, 1.5f);

VertexShaderOutput MainVS(VertexShaderInput input) {
    // Billboard offset rotated to face camera
    float3 billboard = mul(float3(input.Bounds.xy, 0), Rotation);
    float4 translatedPos = float4(input.Position.xyz + TileTranslation + billboard,
                                   input.Position.w);

    // Depth computed from un-billboarded position for correct occlusion
    float depth = mul(float4(input.Position.xyz + TileTranslation
                              + float3(0, input.DepthOffset, 0),
                              input.Position.w), WorldViewProjection).z;

    output.Position = mul(translatedPos, WorldViewProjection);
    output.TexCoord = float3(input.Bounds.zw / TextureSize, depth);
    return output;
}
```

Two fragment passes:

| Pass | Purpose | Alpha test |
|---|---|---|
| P0 (opaque) | Draw solid pixels | `clip(color.a - 0.0001)` — discard transparent |
| P1 (transparent) | Draw semi-transparent pixels | `clip(0.5 - color.a)` — discard opaque; `color.rgb *= color.a * 2` |

### How to reproduce

1. **Atlas-pack** all visible static sprites into large atlases.
2. **Billboard** each sprite toward the camera using a rotation matrix in the vertex shader.
3. **Custom depth**: compute depth from the un-billboarded base position (+ DepthOffset), not from the billboarded position, so that tall sprites don't clip through terrain.
4. **Two-pass alpha**: first draw opaque pixels with alpha test, then draw semi-transparent pixels with additive-like modulation.

---

## 7. The Full Draw Order

```
MainGame.Draw():
  1. Clear(DarkSlateBlue)
  2. Set RasterizerState: CullCounterClockwise, Solid
  3. Set BlendState: AlphaBlend

  4. TerrainRenderSystem.Update():
     a. Update liquid uniforms (Time, Center, WindDirection)
     b. Update solid/single WVP matrices
     c. For each loaded TerrainBlock:
        For each TerrainIndicesEntry (one per terrain type in this block):
          switch (terrainData.Types[entry.TileId]):
            case Liquid: bind liquid textures+params → Apply → DrawIndexed
            case Solid:  bind solid textures+params  → Apply → DrawIndexed
            case Single: (textures pre-bound)        → Apply → DrawIndexed

  5. StaticsRenderSystem.Update():
     For each loaded StaticsBlock:
       Bind atlas texture
       Apply statics shader pass 0 (opaque) → DrawIndexed
       // (pass 1 transparent — implied by technique)

  6. Wireframe debug overlay (optional)
```

### Key observation: no explicit draw order between terrain types within a block

Terrain types are drawn in the order they appear in `TerrainBlock.Indices[]`, which is the order `SetUpIndices()` encounters them scanning the tile grid. The `TextureIndex <= TexIndex` ownership test plus alpha blending provides the compositing order automatically:
- Lower-numbered types render first but paint `alpha=1` only on their own tiles.
- Higher-numbered types render later, overwriting with their own `alpha=1` pixels.
- In the overlap zone (neighbour-expanded quads), both types contribute, and the noise mask creates the boundary.

---

## 8. Summary: What You Need to Reproduce Each Path

### Solid terrain
- 3 tileable textures per type (primary, secondary, noise alpha)
- Per-type stretch values (UV = worldPos / stretch)
- Per-type index buffers with 1-cell neighbour expansion
- Sequential draw with alpha blend, ownership flag via tile id comparison

### Liquid terrain  
- 2 tileable textures per type (albedo, normal map)
- Per-type Speed, WaveHeight, FollowCenter flag
- Time uniform updated every frame
- Normal UV scrolled by `time * speed` along wind direction
- Albedo UV perturbed by sampled normal displacement × waveHeight
- Optional camera-relative UV for FollowCenter types

### Single terrain
- 1 small texture per type, all packed into a Texture2DArray
- UV = world position (1:1 mapping), array layer = tile type id
- No blending, no alpha

### Statics
- Per-block texture atlas packed at load time
- Billboarded quads with 45° Y-rotation
- Custom depth from base position + offset
- Two-pass alpha (opaque then transparent)
