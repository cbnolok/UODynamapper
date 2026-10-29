# EC terrain ownership cache

`texture_cache/land/coverage/cache` precomputes the existing fragment shader's
sorted material stack and four normalized ownership-mask values at each bilinear
corner of a native 32×32 map chunk. This moves reconstruction out of the fragment loop;
it does not replace the recovered ownership rules or the terrain shading policy.
The uncached implementation remains in `land/coverage.wesl` for missing,
unsettled and uncached neighborhoods. `land/main.wesl` keeps its shading calls.

## Exact inputs and representation

The builder uses raw map cells, original per-material parameters, effective EC
image availability, and the existing chunk's first/last retained passes. It
preserves canonical base aliases, first row-major non-base aliases, stable signed
key ordering, the 49-pass key limit, positive-exponent exterior-edge membership,
and the original cardinal/diagonal mask bytes. Point and bilinear filtering stay
in the shader. CPU normalization uses the same constant reciprocal as the original
shader, retaining exact zero/one values. Storing byte masks and renormalizing them
in the cached shader produced repeatable precision differences in native captures.
A 34×34 raw halo supplies native chunk-border ownership.

Each corner stores one header and two integer RGBA texels for each of at most
17 layers: the original packed layer and four normalized mask values stored as
float bits. One extra row identifies the chunk and map. Binding 118 is a
1120×33×16 RGBA32Uint array, 9,461,760 bytes once allocated. CC-only sessions retain a one-texel sentinel.
Binding 117's chunk LUT contains `[first, last, cache_slot, map]` in RGBA16Uint;
65535 means no published slot. The shader checks chunk/map identity and the
current raw payload before using a cache entry.

## Scheduling and coherence

The bounded cache follows the camera viewport-center ray's ground intersection,
including perspective and free-camera movement. Sixteen native chunks cover a
4×4 region, nearest first, with at most two completed bakes per frame. The builder
reads already cached native map blocks; it performs no file I/O. A missing
in-world halo block defers baking. Existing entries need no rebake after CPU
block eviction because loaded map contents and source parameters are immutable.

An entry is published only after its entire raw halo agrees with the resident
metadata atlas and all writes for that metadata revision have been submitted.
Mapping changes invalidate residency just as pixel changes do. Map changes clear
entries; changed chunk pass limits invalidate the corresponding LUT entry.
No image or material mutation occurs while the cache is idle.

The metadata uploader stages in `First`, before `Update` writes newer data.
Therefore a staged batch carries its own revision and image identity: extraction
must not label it with the current CPU revision. Budget backlogs retain their
order. Missing GPU images defer uploads instead of discarding them. Replacing an
atlas image drops its predecessor's queued and staged writes; they must never
target the replacement's layers. The shared submitted revision advances only
after the full batch backlog is issued. This is queue submission ordering, not
a GPU completion fence or an arbitrary settling delay.

The opt-in local `status` command exposes CPU/submitted metadata revisions,
pending writes and published chunk count. Those fields establish metadata
readiness, not color-texture residency or shader compilation completion.

## Regression coverage

The explicit Vulkan test compares every layer and bit-identical alpha against
the production uncached shader at 9,216 positions. Its fixture includes native edges, aliases,
signed keys, point masks, liquid masks, missing images, unsupported materials,
excluded passes and the maximum 17-layer footprint. Removing the uncached lookup
records proves that it actually exercises cache hits. Poisoned cache entries
with wrong chunk, map or raw-owner guards prove fallback behavior.

CPU regressions cover full-halo residency, source-mode changes, atlas eviction,
staged-versus-current revisions and atlas replacement with a pending backlog.
For native images and measured gains, see `renderer_release_performance.md`.
