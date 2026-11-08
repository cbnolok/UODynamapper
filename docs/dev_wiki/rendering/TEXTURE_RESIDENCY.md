# Texture Residency Architecture

Texture residency determines which decoded texture data is available to the
GPU, when it is uploaded, and when it can be evicted. Keep that policy separate
from package decoding and from the format selected for each payload.

A collection can use a bounded working set with eviction, or reserve fixed
layers for a known collection. The shared residency planning helpers describe
groups, layer budgets and deterministic assignments; collection-specific code
owns loading, uploads and cache state. The helpers do not themselves make data
resident. The terrain cache currently selects an LRU strategy; the old
`texmaps_preload_full_file` setting is not its current control.

Preserve bounded memory and predictable upload scheduling as collections evolve.
Do not assume a particular folder layout or force every asset family into one
cache design. Texture formats are selected per use: RGBA and block-compressed
formats have different fidelity, alpha and hardware support requirements. BC1
(DXT1) and BC7 are used in the project; ASTC and ETC2 are mobile candidates
for future evaluation, not an established runtime requirement. See the
[animation format research](../../agent_notes/uddp_anim_format_requirements.md)
and [UDDP formats](../UDDP_FORMATS.md) for package-specific decisions.
