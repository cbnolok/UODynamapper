# BC7 RDO Optimization Notes

This file records local optimization attempts that were reverted after validation or benchmark review. Keep it short and evidence-focused so future agent runs do not repeat plausible but slower changes.

## Reverted Attempts

### Precomputed bounded-decoder color tables

Attempt:
- Precomputed per-subset/per-weight interpolated endpoint colors inside the bounded BC7 partition decoders.
- Applied first to the shared partitioned RGB helpers and mode 7, then retried as a lazy mode 1/mode 7 variant after the first pixel survived the early error bound.

Why it looked promising:
- `bc7_encode --rdo-stats` showed tens of millions of mode 7 decode trials for the `alpha_mobile_art` RDO benchmark case.
- The existing loops recompute endpoint interpolation per pixel.

Why it was reverted:
- Optimized benchmark passes regressed.
- The bounded decoders often exit early, so extra table setup is frequently wasted.
- The current per-pixel arithmetic is cheap enough that stack table construction and indexing lost against direct interpolation.

Practical guidance:
- Do not add upfront interpolation tables to bounded decoders unless profiling proves that most trials run deep into the 16-pixel loop.
- Preserve first-pixel early rejection as the cheapest path.

### Current-block BC7 segment cache

Attempt:
- Cached all current-block byte segments for valid `(len, offset)` pairs once per RDO block.
- Reused the cached segment values for original-block comparisons in the fixed, relative, and second-match candidate paths.

Why it looked promising:
- Candidate loops repeatedly compare previous segments against `(orig_bits >> shift) & segment_mask`.
- Avoiding repeated shifts and masks looked useful in the search hot path.

Why it was reverted:
- Focused correctness tests passed, but optimized benchmark timing was noisy and not convincingly better.
- A longer benchmark pass was essentially neutral to slightly worse on the tested cases.
- The extra stack data and setup cost did not pay for itself.

Practical guidance:
- Do not cache all current-block segments by default.
- Prefer optimizations that remove decode trials, reduce candidate count, or avoid work before the bounded decoder, rather than adding per-block setup.

## Benchmark Context

Commands used for these decisions:
- `cargo check --target-dir target/codex-test -p udd-image-codecs --features bc7-encode`
- `cargo test --target-dir target/codex-test -p udd-image-codecs rdo --features bc7-encode -- --nocapture`
- `cargo bench -p udd-image-codecs --features bc7-encode --bench bc7_encode -- --quick --rdo-case=rdo_default --rdo-stats`
- `cargo bench -p udd-image-codecs --features bc7-encode --bench bc7_encode -- --rdo-case=rdo_default --rdo-stats`

The release benchmark target may use a configured target directory outside the normal workspace sandbox.
