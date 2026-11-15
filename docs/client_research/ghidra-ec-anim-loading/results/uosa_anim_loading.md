# How UOSA.exe loads mobile animations, and where it stalls

Program: `UOSA.exe`, SHA-256 `5a5e9dc536c836d1bf10b6f86bb88f61a6ae078f519d4060d3bc47aecbebad50`. Names are those
applied by `NameEcAnimLoadingCode.java` (this folder), `NameAnimationCode.java` (`ghidra-animation`),
`NameAnimationFrameCode.java` (`ghidra-animation-frames`) and `NameUopTrace.java` (`ghidra-source-names`).

**V** = read in the code. **I** = inferred from the code, or from the code and the shipped data, without a
run of the client.

## Answer: what makes the official EC hitch

All of the work below runs on the main thread, inside the frame. None of it is queued to a worker thread.

1. **A new body/action loads one whole UOP entry synchronously in the frame (V).**
   - `UOAnimations::UpdateFrame` runs for each layer of each mobile every frame. When the layer's frame file
     lacks the needed (action, frame), it calls `LoadLayerFrameSet` inline.
   - `GetAnimationFrameSet` then calls `UOAssets::GetAsset`, and on a cache miss:
     `LoadAssetBlob` → `ReadPackagedFile` → `UOP::OpenFilePtr(path, 2)`.
   - `OpenFilePtr` passes `async = 0` to `UOVirtualFileOpen`. That path reads the compressed bytes with the
     archive's read, inflates the whole entry with zlib 1.2.2 `uncompress`, then copies it once more into the
     asset `Blob`.
   - The entry is the whole (body, action) file, holding all five directions. That is several times the
     frames one direction needs.
   - Measured inflate cost of one entry (table below): median 0.1 to 4 ms, p95 1 to 12 ms, maximum 57 ms.
     A cold read adds disk time, a seek per entry on an HDD.
   - A mobile entering the scene needs one entry per layer: body, mount and each worn item. Several such
     mobiles in one frame add up to tens of ms.
2. **Equipping a weapon loads up to six more whole entries just to test whether they exist (V).**
   `BuildLayers` → `CollectWeaponAttackActions` calls `GetAnimationFrameSet` for actions 9 to 14 of the
   weapon's appearance. Each call is a full load and inflate when the entry is not cached.
3. **A missing action can cost up to 52 lookups (V), but failures are cached (I).**
   - With `any_action`, `LoadLayerFrameSet` tries actions 0 to 0x33 until one exists. A missing UOP entry is
     only a hash lookup.
   - `StoreAsset` caches the null result as well: it has no null check (V). So the probe is not repeated
     while the entry stays cached (I).
4. **Every new composite frame is a CPU decode plus a new D3D9 texture, created during rendering (V).**
   - `MobAnimTriShape::RenderImmediate` (vtable +0x88, and the same for the shadow shape) calls
     `BuildFrameAtlas`. A miss in the composite cache does this, all synchronously:
     - allocates a power-of-two BGRA8888 `NiPixelData` and memsets it to 0;
     - RLE-decodes every layer of that one frame into it;
     - constructs a `MobAnimTexture`, whose constructor creates the renderer data at once:
       `NiDX9Renderer::CreateSourceTextureRendererData` →
       `NiDX9SourceTextureData::CreateD3DTexture` (`IDirect3DDevice9::CreateTexture`, usage 0,
       **`D3DPOOL_MANAGED`**, 1 level) → `CopyLevelToSurface` (`LockRect`, row-by-row memcpy, `UnlockRect`);
     - frees the CPU pixel data.
   - A composite is one frame of one mobile: body plus equipment, with hues and mirroring baked in. A
     walking mobile makes a new texture at every frame step, in each direction, the first time it is shown.
   - The managed-pool copy to video memory happens at the texture's first draw (I: D3D9 managed-pool
     behaviour).
5. **The composite cache is small, so busy scenes keep re-creating textures (V for the budget, I for the
   thrash).**
   - The cache is an LRU keyed by the layer records. Its budget is 10% of the "DiskCache" setting: **26 MB**
     at the default 256 MB.
   - It evicts least-recently-used textures until the new one fits. Each texture's size is
     width × height × 4 at the power-of-two size.
   - That budget holds about 830 composites of 64×128, 416 of 128×128, 104 of 256×256, or 26 of 512×512.
   - Thirty animated mobiles of 128×128 with 10 frames in each of 2 to 3 used directions need 600 to 900
     composites. Past the budget, frames that were drawn a second ago are created again: decode,
     CreateTexture, Lock and copy at every frame step.
6. **The asset (blob) cache almost never evicts on a machine with free RAM (V), so stalls are first-sight
   costs, not refills.**
   - `UOAssets::Cache_PeriodicSweep` runs from the main loop at most once a second.
   - `Cache_ShouldEvict` keeps any entry used within the last second. Otherwise it evicts only when:
     - +0xec, a byte limit, is exceeded. Its only writer is the constructor, which sets it to 0, so this
       never fires; or
     - free physical memory (`GlobalMemoryStatus.dwAvailPhys`) is at most +0xf4, which is 0.9 × the cache
       setting (230 MB at the default).
   - The decompressed blobs of every (body, action) seen therefore stay resident until RAM gets tight.

There is no preloading of frames (V for the visible paths).
- `BuildMobileAnimation`'s `UOResourceRequest` holds only the sequence (type 0x0c000000).
- `UOAssets::LoadRequest` runs it synchronously, one `GetAsset` per entry.
- The only callers of `GetAnimationFrameSet` are `LoadLayerFrameSet` and `CollectWeaponAttackActions`.

The UOP layer has an asynchronous open (`UOVirtualFileOpen(path, mode, 1)`, `FileLoadJobRequest`,
`DecompressJobRequest`). The asset manager's path does not use it (V: `OpenFilePtr` passes 0).

**So:** the stutter when new mobiles or actions enter the view comes from (1), (2) and (4). Each is a
synchronous main-thread cost proportional to the data touched:
- inflating whole (body, action) entries that are much larger than the frames needed;
- per-frame CPU compositing and D3D9 texture creation at first display.

(5) turns (4) into a recurring cost in crowded scenes.

## Consequences for a new mobile_anim format

These are design implications, not findings.
- Make the unit of I/O and decompression small. Per (body, action, direction) or per frame, not the whole
  action, so a first sight touches only what is drawn.
- Make it decodable off the main thread without a sequential RLE pass per pixel row.
- Upload into long-lived atlases (array textures or atlas pages) instead of one texture per composite frame.
  Draw layers as separate quads, with hue in the shader, so equipment and hue changes don't force
  re-compositing.
- Allow a cheap existence test (an index) so that "does this action exist" never loads data.

## The path, step by step

| Step | Function | What it does | Mark |
|---|---|---|---|
| per frame | `UOApplication::MainLoop` `0x00482530` | the frame loop: Sleep to the fps cap, the updates, `UOAssets::Tick` | V (probable name) |
| per layer | `UOAnimations::UpdateFrame` `0x00436d40` | frame step; no entry for (action, frame) → `LoadLayerFrameSet` inline; palette rebuilt on hue or file change; layer record pushed (`PushLayerRecord` `0x0043adf0`) | V |
| layer build | `UOAnimations::BuildLayers` `0x004387f0` | layer setup, `CollectWeaponAttackActions` for weapons, `LoadLayerFrameSet` with any_action | V |
| file | `LoadLayerFrameSet` `0x00439b70` | `GetAnimationFrameSet(id, action)`; with any_action the first existing action of 0..0x33 | V |
| file | `GetAnimationFrameSet` `0x004316f0` | `GetAsset(id = body × 0x100 + action, type 0x10000000)` | V |
| cache | `UOAssets::GetAsset` `0x00a6cfc8` | `Cache_FindAndTouch` (hit: LRU tail, stamp); miss: `Factory_LoadsFromBlob` → `LoadAssetBlob`, `CreateAssetFromBlob`, then `StoreAsset` (also null); per-type QPC load stats and per-frame load count/time keyed to the frame counter `0x00e2e97c` | V |
| read | `ReadPackagedFile` `0x00a6d4b7` → `UOP::OpenFilePtr(path, 2)` → `UOVirtualFileOpen(path, 2, 0)` | synchronous open: `MythMypInputFile` with a buffer of the decompressed size, `MypArchive_ReadEntry` → `UOP::Decompress` → zlib `uncompress` | V |
| copy | `FUN_00a6fecf` | size (file vtable +0x28) and read (+0x38) into a new `Blob`: a second copy of the decompressed entry | V |
| asset | `UOAnimationFrameSetFactory::Create` `0x0084217e` | `UOMobFrameSet` over the blob | V |
| draw | `MobAnimTriShape::RenderImmediate` `0x004328b0`, `MobAnimTriShapeShadow::RenderImmediate` `0x00432110` | vtable +0x88: `BuildFrameAtlas`, quad, renderer vtable +0x180 | V (slot role I) |
| composite | `UOAnimations::BuildFrameAtlas` `0x00431a10` | `FrameAtlasCache_Find`; hit: move to front; miss: NiPixelData, memset, `DecodeFrameRle`/`DecodeFrameRleMirrored` per layer, `MobAnimTexture::Construct`, LRU eviction to the byte limit | V |
| texture | `MobAnimTexture::Construct` `0x00431770` | TRUE_COLOR_32, SMOOTH alpha, no mipmaps, static; renderer vtable +0x100 at once | V |
| D3D9 | `NiDX9Renderer::CreateSourceTextureRendererData` `0x0075eac0` → `NiDX9SourceTextureData` create `0x00770b90` | format pick, `NiImageConverter` conversion, `CreateD3DTexture` `0x00771190` (MANAGED, 1 level), `CopyPixels` `0x007712a0` → `CopyLevelToSurface` `0x00771380` (LockRect flags 0, memcpy, UnlockRect) | V |
| free | `NiSourceTexture` vtable +0x58 `0x0068ab80` | drops the CPU pixel data after creation | V |

## Caches and limits

| Cache | Key | Budget | Eviction | Mark |
|---|---|---|---|---|
| Asset cache (`g_UOAssetManager`, `0x00e37220`) | asset id and type | no byte limit in practice (+0xec = 0) | sweep at most once a second from `UOAssets::Tick`; entries used within 1 s kept; evicts unused entries only while free physical RAM ≤ 0.9 × cache setting | V |
| Frame composite cache (`0x00e37230`, 0x30 bytes) | the layer records of one frame (frame entry, palette, mirror, offsets) | 0.1 × cache setting MB (26 MB at 256) | LRU on insert, until used + new ≤ limit | V |
| Cache setting `g_UOSettingCacheSize` `0x00e33624` | | default 256 MB; 32 MB under 512 MB RAM, 64 MB under 1 GB; settings slider "DiskCache" 0..1024 | `UOSettings::ApplyCacheSize` `0x0049a740` | V |

`UOAssets::Cache_Construct` also sets +0xf8 = 450000 (45 s in clock ticks). Its use was not traced.
`UOAssets::Cache_SetMemoryThreshold` (`0x00a70f40`) has no callers.

## Measurements: AnimationFrame entries

The table covers every entry of the shipped EC `AnimationFrame1..6.uop` (sizes decompressed). The inflate
times use one core of an AMD Ryzen AI 9 365, **boost off, EPP power**, through Python's zlib 1.3.2 (the client
links 1.2.2). The files were in the page cache, so there is no disk time. Each time is the best of 5 for one
entry. The scripts are kept outside the repository.

| File | Entries | zlib | Median | Median inflate | p95 | p95 inflate | Max | Max inflate | Whole file |
|---|---|---|---|---|---|---|---|---|---|
| AnimationFrame1 | 5836 | 5833 | 24 KB | 0.15 ms | 117 KB | 1.12 ms | 966 KB | 10.97 ms | 218 MB |
| AnimationFrame2 | 14197 | 14165 | 15 KB | 0.09 ms | 115 KB | 1.28 ms | 4023 KB | 41.11 ms | 649 MB |
| AnimationFrame3 | 8050 | 8017 | 17 KB | 0.11 ms | 333 KB | 3.09 ms | 4912 KB | 56.82 ms | 598 MB |
| AnimationFrame4 | 2575 | 2575 | 98 KB | 0.97 ms | 457 KB | 4.38 ms | 7105 KB | 18.59 ms | 418 MB |
| AnimationFrame5 | 455 | 455 | 434 KB | 4.15 ms | 1177 KB | 11.53 ms | 1746 KB | 9.02 ms | 215 MB |
| AnimationFrame6 | 764 | 764 | 237 KB | 2.32 ms | 611 KB | 6.28 ms | 1866 KB | 5.68 ms | 205 MB |

Inflating every entry of a file runs at 55 to 83 MB/s on this machine. The CPUs of the EC's time were
slower.
