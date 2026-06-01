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

### Fixed-path distance-cost row hoist

Attempt:
- In the fixed-offset RDO path, loaded the `normal_match_bits` and `normal_trial_lambda` arrays once per previous block.
- Indexed those arrays inside the match-length loop instead of calling the small per-length accessors.

Why it looked promising:
- The fixed-offset search loops over many lengths and offsets for the same previous block.
- Hoisting row lookup removes repeated vector indexing through tiny accessor calls.

Why it was reverted:
- Correctness tests passed, but the benchmark did not support the change.
- It slightly improved the mode1-heavy land case, but regressed the alpha-mobile and mixed-atlas cases in the same run.
- The compiler likely already handled the tiny accessors well enough, and the extra local references did not improve the generated hot path consistently.

Practical guidance:
- Do not repeat this exact row-hoist unless assembly or a stronger benchmark shows a target-specific gain.
- Prefer changes that reduce candidate work or decode trials over reshuffling cheap layout accessors.

### Fixed-path pre-copy mode rejection

Attempt:
- For fixed-offset candidates at offset zero, checked `bc7_first_byte_has_mode(prev_segment as u8, bc7_mode)` before constructing the copied trial block.
- This is equivalent because the copied segment supplies the first byte when `ofs == 0`.

Why it looked promising:
- It can skip `bc7_copy_segment_bits_from_segment` for candidates that would be rejected immediately by the mode check.

Why it was reverted:
- The default benchmark cases reported `unsupported_modes=0`, so the new branch did not skip any work.
- Benchmark timing was neutral to slightly worse.

Practical guidance:
- Do not add this branch to the default fixed path unless a workload shows meaningful unsupported offset-zero trials.

### Analytical Mode 1 weight descriptor table

Attempt:
- Replaced the Mode 1 encode weight-packing loop with a compile-time table containing partition subset and bit offset per pixel.
- Preserved the original unmasked weight write behavior, so benchmark checksums stayed stable.

Why it looked promising:
- The RDO bounded Mode 1 decoder benefited from a similar descriptor-table approach.
- The encode loop also performs per-pixel partition lookup and anchor offset adjustment.

Why it was reverted:
- The BC7 encode benchmark did not show a win; scalar and wide timings moved sideways to slightly worse in the tested run.

Practical guidance:
- Do not assume the RDO decoder descriptor win transfers to analytical encode packing.
- Revisit only with assembly evidence or a benchmark case where Mode 1 encode packing is isolated as a measurable bottleneck.

### Bounded decoder endpoint-delta interpolation

Attempt:
- Precomputed endpoint deltas (`hi - lo`) once in the bounded BC7 decoders.
- Replaced repeated `interpolate_bc7(lo, hi, weight)` calls with `interpolate_bc7_delta(lo, delta, weight)` in Mode 1, Mode 4, Mode 5, Mode 6, Mode 7, and the shared partitioned RGB helpers.
- Preserved the same per-pixel bounded-exit checks and produced stable benchmark checksums.

Why it looked promising:
- RDO stats showed Mode 1 and Mode 7 bounded decoders dominate the default fixture cases.
- Avoiding repeated endpoint subtraction inside those loops looked cheaper than full palette predecode and kept early exits intact.

Why it was reverted:
- Focused RDO tests passed, but the release RDO benchmark regressed against the immediately preceding hash-table baseline.
- The alpha-mobile and mixed-atlas cases were both slower, which are the important sparse/mobile-style workloads.
- The extra local arrays and helper plumbing likely increased register pressure enough to outweigh the saved subtractions.

Practical guidance:
- Do not reintroduce broad endpoint-delta arrays in the bounded decoders without assembly evidence.
- If revisiting interpolation, prefer a narrower mode-specific change with measured register pressure and the same bounded-exit behavior.

### Mode 7 bounded decoder palette predecode

Attempt:
- Precomputed two Mode 7 subset palettes with four RGBA colors each.
- Replaced per-pixel interpolation in `decode_bc7_mode7_error_bounded` with palette lookups.
- Kept the same pixel order and the same bounded-error exit after each pixel, so checksums and focused Mode 7 tests stayed stable.

Why it looked promising:
- Alpha-mobile RDO stats showed Mode 7 dominates the bounded decode count.
- Mode 7 uses only 2-bit weights, so an 8-color palette can reduce repeated interpolation in full-block decodes.

Why it was reverted:
- The RDO benchmark regressed badly on all cases after two consecutive runs.
- Alpha-mobile dropped to roughly `451-454 blk/s` in those runs, much worse than the current baseline.
- The likely cause is that most bounded decodes exit early, so paying to build all subset palette entries upfront is wasted work and increases register/cache pressure.

Practical guidance:
- Do not precompute full Mode 7 palettes in the bounded decoder.
- Mode 7 optimization should preserve lazy per-pixel work or first measure average bounded-exit depth before moving work ahead of the exit checks.

## Benchmark Context

Commands used for these decisions:
- `cargo check --target-dir target/codex-test -p udd-image-codecs --features bc7-encode`
- `cargo test --target-dir target/codex-test -p udd-image-codecs rdo --features bc7-encode -- --nocapture`
- `cargo bench -p udd-image-codecs --features bc7-encode --bench bc7_encode -- --quick --rdo-case=rdo_default --rdo-stats`
- `cargo bench -p udd-image-codecs --features bc7-encode --bench bc7_encode -- --rdo-case=rdo_default --rdo-stats`

The release benchmark target may use a configured target directory outside the normal workspace sandbox.
