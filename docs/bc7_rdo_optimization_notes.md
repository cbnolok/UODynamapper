# BC7 RDO Optimization Notes

This file records local optimization attempts that were reverted after validation or benchmark review. Keep it short and evidence-focused so future agent runs do not repeat plausible but slower changes.

## Optimization Constraints And Objectives

Primary constraints:
- Do not compromise image quality or RDO decision quality for speed unless the maintainer explicitly asks for a quality/speed tradeoff.
- For a fixed parameter set, prefer exact-output optimizations: same accepted candidates, same block bytes, same checksums.
- If a change intentionally alters RDO search scope, thresholds, candidate ordering, tie handling, or defaults, treat it as a behavior change and validate quality separately from throughput.
- Keep sparse/mobile atlas behavior first-class. Transparent gutters and alpha-heavy content are important workloads, not edge cases.

CPU and memory objectives:
- Reduce candidate count and bounded decode count before micro-optimizing individual arithmetic instructions.
- Preserve early-exit behavior in bounded decoders. Do not move expensive setup ahead of a likely early rejection unless benchmark evidence supports it.
- Prefer branchless code in hot paths when it preserves semantics and does not add worse memory traffic.
- Reduce branch mispredictions by separating rare paths, hoisting invariant decisions, and keeping highly predictable checks outside inner loops.
- Choose the SIMD backend early, outside hot loops. Do not poll CPU feature support or dispatch between SIMD variants inside per-block/per-candidate paths.
- Make extensive use of SIMD where the operation maps cleanly and exactness can be preserved. Keep scalar fallbacks correct and measurable.
- Consider explicit loop unrolling for tiny fixed loops when it reduces bounds checks, branch overhead, or register shuffling. Validate because unrolling can increase code size and pressure instruction cache.
- Reason in terms of cache lines: avoid per-block tables, scratch buffers, or wider entries unless they remove enough hot work to pay for extra loads/stores.
- Prefer fixed-size stack storage over heap allocation for small hot-path state, as long as it does not inflate stack frames enough to hurt cache locality or recursion/thread scaling.
- Prefer compact bitwise operations over byte/slice manipulation when working with BC7 block fields, masks, selectors, and fixed-size segments.
- Avoid atomics, callbacks, allocation, dynamic dispatch, and repeated Rayon scheduling in per-candidate/per-block hot paths.

Validation expectations:
- Check correctness with focused RDO tests and stable fixture checksums.
- Benchmark with `bc7_encode --rdo-stats` when the change affects candidate loops or bounded decoders.
- Compare stats as well as throughput. A useful optimization should explain changes in candidates, decodes, hash skips, bounded exits, or mode mix.
- If a plausible optimization regresses, revert it and record the attempt below with enough detail to prevent repeat work.

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

### Partial max-trial-error branch hoist

Attempt:
- Split `max_trial_error` into a scaled helper and hoisted the `trial_error_scale > 0.0` flag once per block.
- Replaced the helper's internal branch with per-call-site conditionals before each bounded decode.

Why it looked promising:
- The max-trial-error calculation happens before nearly every bounded decode trial.
- The zero-scale case is block-invariant and should be rare in normal RDO settings.

Why it was reverted:
- Focused tests passed and checksums stayed stable, but the benchmark moved alpha-mobile and mixed-atlas down versus the current hash-probe baseline.
- The change did not actually remove the per-decode branch; it only moved it from the helper to each call site and added code size.

Practical guidance:
- Do not repeat this partial hoist.
- If optimizing this path, split the whole candidate search into scaled/unbounded outer paths or prove the zero-scale case can be eliminated for the relevant public parameters.

### Mode-history capacity pre-count

Attempt:
- After replacing the per-mode ring history with append-only histories, counted initial block modes and allocated each mode history to its expected final size.
- Intended to avoid `Vec` growth in alpha/mobile cases dominated by one BC7 mode.

Why it looked promising:
- The append-only history improves iteration but can grow beyond the initial lookback-sized capacity.
- Alpha-mobile is heavily Mode 7, so one history vector grows from the default 64-block capacity to most of the page.

Why it was reverted:
- Correctness tests passed and checksums stayed stable, but no-stats benchmark timing did not improve.
- The extra full pass over `block_modes` outweighed avoiding a small number of vector growth events on the tested 1024-block pages.

Practical guidance:
- Keep append-only history iteration, but do not pre-count mode capacities for current page sizes.
- Revisit only if much larger RDO chunks show allocator growth as a measurable cost.

### Precomputed shifted segment masks

Attempt:
- Added a `len`/offset table of shifted BC7 segment destination masks.
- Reused the shifted mask for original-block comparisons and segment copies in relative, fixed, and second-match paths.

Why it looked promising:
- Candidate loops repeatedly compute `segment_mask << shift` and compare `(orig_bits >> shift) & segment_mask`.
- A small static table could replace per-candidate mask shifts with a cache-resident load and use `orig_bits & dst_mask`.

Why it was reverted:
- Correctness tests passed and checksums stayed stable, but benchmark results were mixed.
- Mixed/opaque cases improved slightly, while alpha-mobile moved down in both stats and no-stats runs.
- The extra table load/cache footprint did not clearly beat the direct shift/mask arithmetic for the mobile-heavy target.

Practical guidance:
- Do not add a broad shifted-mask table for all RDO copy paths.
- Revisit only if a narrower path, such as second-match only or relative only, shows a clear workload-specific gain.

### Stack-allocated RDO hash table

Attempt:
- Replaced the fixed 8192-entry duplicate-candidate hash table allocation with a stack array.
- Kept lookup behavior and candidate order unchanged.

Why it looked promising:
- The table size is fixed and local to each RDO pass/chunk.
- Avoiding heap allocation matches the stack-preference objective for fixed-size hot-path state.

Why it was reverted:
- Correctness tests passed and checksums stayed stable, but benchmark behavior was mixed.
- Opaque and alpha-mobile no-stats runs were neutral to slightly positive, while mixed-atlas regressed enough to reject the change.
- The larger stack frame likely hurt cache/stack locality more than the removed allocation helped.

Practical guidance:
- Do not move the 8192-entry RDO hash table to the stack by default.
- Stack storage is still preferred for small fixed hot-path state, but validate larger arrays carefully against cache and stack-frame costs.

### Rolling fixed-offset bit windows

Attempt:
- In the normal fixed-offset RDO path, replaced repeated `ofs * 8` plus variable `u128` shifts with rolling `prev_bits`/`orig_bits` windows shifted by one byte per offset.
- Kept the same candidate order, hash-before-original-compare order, accepted candidates, stats, and output checksums.

Why it looked promising:
- The default fixed-offset path performs many segment extractions for each length and previous block.
- Rolling windows should replace repeated variable shifts and offset multiplication with cheaper fixed shifts.

Why it was reverted:
- Focused tests passed and benchmark stats/checksums stayed stable, but no-stats throughput dropped versus the immediately preceding committed descriptor-table baseline.
- The nested control flow needed to advance windows once per offset likely increased branch pressure and register pressure enough to erase the arithmetic savings.

Practical guidance:
- Do not repeat the broad rolling-window rewrite for the normal fixed-offset path.
- If revisiting this idea, isolate a narrower path or use generated length/offset-specialized code that preserves straight-line control flow.

### Grouped relative candidates by source offset

Attempt:
- Replaced the flat relative candidate list with source-offset runs plus destination candidates.
- Preserved the existing `src_ofs` then `dst_ofs` candidate order and loaded the previous segment lazily once per source run after the rate check passed.

Why it looked promising:
- Relative movement can test multiple destination offsets for the same previous-block source segment.
- Reusing `(prev_bits >> src_shift) & segment_mask` should reduce repeated variable `u128` shifts without adding current-block segment caches.

Why it was reverted:
- Focused RDO tests passed and relative-mix checksums stayed stable, but the no-stats `rdo_relative_mix` benchmark regressed versus a temporary `HEAD` baseline.
- Baseline was roughly `1511/1293/3721 blk/s` for opaque/alpha/mixed; grouped runs were roughly `1421/1262/3566 blk/s`.
- The extra nested slices and lazy-load branch cost more than the saved source-segment shifts.

Practical guidance:
- Keep the flat relative candidate layout for now.
- If revisiting relative movement, prefer length-specialized hashing or a flatter grouped representation that does not add an inner lazy-load branch.

### Relative-path length-specialized hashing

Attempt:
- Kept the flat relative candidate layout, but moved the `len` dispatch outside the per-candidate loop with a const-generic relative helper.
- Replaced `hash_hsieh_bc7_segment(prev_segment, len, dst_ofs)` with `hash_hsieh_bc7_segment_fixed::<LEN>(...)`.
- Preserved candidate order, stats, accepted candidates, and output checksums.

Why it looked promising:
- The fixed-offset path benefited from length-specializing the common candidate loop.
- Relative RDO performs many hash checks, so removing per-candidate length dispatch looked transferable without adding the grouped-layout branch cost.

Why it was reverted:
- Focused tests passed and checksums stayed stable, but `rdo_relative_mix` no-stats benchmarks were mixed and regressed alpha/mobile.
- Fresh `HEAD` baseline was roughly `1512/1343/3874 blk/s` for opaque/alpha/mixed; specialized relative was roughly `1507/1288/3948 blk/s`.
- The mixed-atlas gain did not justify the alpha/mobile regression.

Practical guidance:
- Do not length-specialize the full relative candidate loop with the current helper shape.
- If revisiting, use profile data to target only modes or workloads where relative movement is actually selected and avoid increasing code size for alpha-heavy pages.

### Single-pass block standard deviation

Attempt:
- Rewrote `compute_block_max_std_dev` from a channel-first loop over the 16 pixels to one pass accumulating RGBA sums and squared sums together.
- Preserved the same variance formula and output checksums.

Why it looked promising:
- The original code scans each 4x4 block four times, once per channel.
- One pass should be more cache-friendly and reduce loop overhead before candidate search begins.

Why it was reverted:
- Focused tests passed and RDO checksums stayed stable, but both stats and no-stats default benchmarks regressed.
- The array-based accumulators likely added register pressure and indexing overhead, while the original tiny channel loop is easy for LLVM to optimize.

Practical guidance:
- Keep the simple channel-first `compute_block_max_std_dev` loop.
- If revisiting, try a fully scalar unrolled accumulator only with assembly or benchmark evidence, not an indexed `[u32; 4]` implementation.

### Mode-specialized fixed-path bounded decode

Attempt:
- Specialized the common fixed-offset RDO helper by both match length and BC7 mode.
- Replaced the per-candidate generic trusted-mode decoder dispatch with a const-mode decoder helper.
- Left relative, REP0/continuation, and second-match paths on the generic decoder.

Why it looked promising:
- The default fixed path performs millions of bounded decode trials.
- The block mode is known for the whole block, so dispatching on mode for every decoded candidate should be avoidable.

Why it was reverted:
- Focused tests passed and stats/checksums stayed identical.
- Stats runs improved, but no-stats benchmark repeats did not beat the current accepted baseline overall.
- The added mode x length specialization likely increased code size and instruction-cache pressure enough to offset the removed per-candidate mode match.

Practical guidance:
- Do not add broad mode-specialized copies of the fixed candidate helper.
- If revisiting, target one dominant mode-specific decoder path at a time, or use assembly/perf evidence to prove the mode dispatch is still a real bottleneck.

### Unrolled Mode 1 and Mode 7 bounded decoder loops

Attempt:
- Manually unrolled the per-pixel loops in `decode_bc7_mode1_error_bounded` and `decode_bc7_mode7_error_bounded`.
- Preserved the same pixel order and the same bounded-error early exit after every pixel.

Why it looked promising:
- Mode 1 dominates opaque land RDO decode trials and Mode 7 dominates alpha/mobile decode trials.
- Removing loop control and bounds/index bookkeeping looked like a direct fit for the manual-unroll objective.

Why it was reverted:
- Focused tests passed and checksums stayed stable, but default RDO stats/no-stats benchmarks regressed overall.
- Mode 7-only unroll helped one no-stats alpha/mixed run, but combining Mode 1 and Mode 7 reduced opaque and alpha enough to reject the pass.
- The extra instruction footprint appears more expensive than the loop overhead LLVM already handles well.

Practical guidance:
- Keep the compact Mode 1/Mode 7 pixel loops.
- Avoid broad decoder-loop unrolling unless perf data shows a specific loop branch or bounds check survived optimization.

### Post-rate-check max trial error helper

Attempt:
- Replaced hot `max_trial_error(...)` calls with a helper used only after rate checks had already proven `best_t > trial_bits_times_lambda`.
- Removed the redundant `.max(0.0)` clamp from those hot call sites while preserving the zero-scale `u64::MAX` behavior.
- Kept output checksums and RDO stats stable.

Why it looked promising:
- `max_trial_error` runs before every bounded decode trial.
- The hot call sites already skip candidates where the trial rate is not below the current best score, so the error budget delta should be positive.

Why it was reverted:
- Focused tests passed, but stats/no-stats default RDO benchmarks were below the accepted baseline.
- The clamp removal was likely optimized well already or lost in noise next to the subsequent bounded decoder work.

Practical guidance:
- Keep the existing `max_trial_error` helper shape for now.
- If revisiting budget calculation, split the whole search by `trial_error_scale > 0.0` instead of shaving one clamp at individual call sites.

### Unchecked RDO hash-table slot

Attempt:
- Replaced the hot `hash_table[hs as usize & hash_mask]` probe in `rdo_hash_seen` with `get_unchecked_mut`.
- Added debug assertions that the table length is a power of two and that `hash_mask == hash_table.len() - 1`.
- Kept the same hash entry encoding and overwrite semantics.

Why it looked promising:
- The hash table is fixed at 8192 entries, so the masked hash index is provably in range.
- The helper is called in the candidate loops for relative, fixed, and specialized fixed-match search.
- It was a narrow unsafe change with no image-quality or feature impact.

Why it was reverted:
- Focused tests passed and checksums stayed stable, but repeated no-stats default RDO benchmarks regressed versus the accepted baseline.
- The safe indexing bounds check was likely optimized well, or the unsafe version inhibited code generation enough to lose.

Practical guidance:
- Do not replace `rdo_hash_seen` indexing with unchecked access without new assembly/perf evidence.
- Prefer unsafe only where benchmark data shows a real hot bounds check survived optimization.

### Unchecked distance-cost table accessors

Attempt:
- Replaced the hot `DistanceCostLayout` vector/array indexing in `normal_bits`, `normal_match_bits`, `normal_trial_lambda`, and `relative_bits` with `get_unchecked`.
- Added debug assertions for the established invariants: previous-block deltas are within the lookback-derived table, and match lengths are emitted only from `3..=16` loops.
- Kept the distance-cost table layout, candidate order, scoring, and decode decisions unchanged.

Why it looked promising:
- These accessors are called from fixed, relative, and second-match RDO search loops.
- The bounds are guaranteed by `recent_from(first_block_to_check)` and by the precomputed table length.
- The change avoided unsafe decoder rewrites and targeted only small cache-resident cost tables.

Why it was reverted:
- Focused compile/tests passed and output checksums stayed stable, but the no-stats default RDO benchmark regressed on opaque, alpha/mobile, and mixed fixtures.
- LLVM likely already eliminated or hid most of these bounds checks, while unchecked access added no useful instruction-cache or branch benefit.

Practical guidance:
- Keep the safe `DistanceCostLayout` accessors unless assembly evidence shows remaining checks in the final hot loops.
- Unsafe indexing should not be assumed faster for tiny table lookups; validate it against no-stats throughput before keeping it.

### Relative-RDO minimum-rate row skip

Attempt:
- Added a per-length bitmask of relative distance indices used by `RelativeCandidateLayout`.
- Precomputed the minimum possible relative trial lambda for each `(block_delta, len)` row.
- Skipped an entire relative candidate length row when that minimum rate was already `>= best_t`, preserving the same condition used by the existing per-candidate rate skip.

Why it looked promising:
- Relative rows can contain many source/destination candidates for each previous block and match length.
- A row-level skip should avoid candidate iteration, segment extraction, hashing, hash-table probes, and decode attempts once `best_t` is tight.
- The exactness invariant is strong: if the minimum possible rate cannot beat `best_t`, no candidate in that row can beat it.

Why it was reverted:
- Focused compile/tests passed and relative RDO checksums stayed stable.
- Same-machine baseline benchmarking from a temporary worktree showed the patch regressed `rdo_relative`: the added precompute/table access cost was higher than the saved inner-loop work on the benchmark fixtures.
- The existing per-candidate rate skip is already cheap, and many relative rows still need to be visited.

Practical guidance:
- Do not add row-level relative minimum-rate tables by default.
- If revisiting relative pruning, target dynamic cases with high observed `rate_skips` and validate against same-state baseline worktrees, not old benchmark anchors.

### Interior-specialized ultrasmooth erosion

Attempt:
- Added an interior fast path for `erode_ultrasmooth_mask_at` and `median_erode_ultrasmooth_mask_at`.
- Interior blocks used fixed 3x3 neighbor offsets and skipped the boundary checks inside the `dx/dy` loops.
- Border blocks kept the existing boundary-aware loop, preserving exact mask semantics.

Why it looked promising:
- The ultrasmooth mask prepass runs one erosion plus 32 median-like erosion passes.
- Most blocks in large pages are interior, so removing repeated boundary tests looked like a branch-reduction win.

Why it was reverted:
- Focused compile/tests passed and checksums stayed stable.
- Same-state benchmarking was mixed and too small to justify the extra code: tiny gains on opaque/alpha, but a loss on mixed.
- The fixed-offset path checks all nine neighbors even when an early false neighbor could have ended the original loop, so lower branch count did not translate into consistent throughput.

Practical guidance:
- Keep the compact boundary-aware erosion helpers for now.
- If revisiting, measure mask-shape distributions first; an interior path may only help when most active mask cells have all neighbors true.

### Linear-index ultrasmooth flood fill

Attempt:
- Changed the ultrasmooth-region flood fill from `(x, y)` tuple stacks/components to linear block indices.
- Replaced the small neighbor tuple loop with explicit left/right/up/down index pushes through an inlined helper.
- Kept the same 4-connected components and size threshold decisions.

Why it looked promising:
- Linear indices halve the stack/component element width and avoid repeated `x + y * blocks_x` reconstruction.
- Explicit neighbor branches should avoid iterating a tiny tuple array for every popped component cell.

Why it was reverted:
- Focused compile/tests passed and checksums stayed stable.
- The no-stats default RDO benchmark regressed sharply on the opaque fixture and did not improve the alpha/mixed fixtures enough to justify the change.
- The extra `%` and `/` needed to recover `x`/`y` per popped index likely cost more than the tuple storage saved.

Practical guidance:
- Keep the tuple-based flood fill unless the traversal is redesigned to avoid per-cell division.
- If revisiting, consider row-aware runs or a queue that carries edge flags, not a simple linear-index stack.

### Reused seed-pass smooth scales

Attempt:
- Moved smooth-scale parameter derivation before the ultrasmooth seed pass.
- Changed the ultrasmooth prepass to compute and store per-block fallback smooth scales for luma-in-range blocks while it already had each block's max stddev.
- Overwrote only surviving large ultrasmooth components with the adjusted ultrasmooth scale and removed the later scale-adjustment pass.

Why it looked promising:
- The seed pass computes max stddev for many blocks, and the main RDO loop otherwise recomputes the same stddev for non-ultrasmooth blocks.
- Reusing those values should remove duplicated `sqrt`/channel scans and one post-pass over the scale vector.

Why it was reverted:
- Focused compile/tests passed and checksums stayed stable.
- Same-state default RDO benchmarking regressed on all fixtures. The stored scale vector increased memory traffic and branch work enough to outweigh the avoided fallback stddev calculations.

Practical guidance:
- Do not cache per-block fallback smooth scales in the ultrasmooth vector by default.
- If revisiting, first measure how many blocks both pass the luma gate and reach the fallback path; broad caching is too expensive on the current fixtures.

## Benchmark Context

Commands used for these decisions:
- `cargo check --target-dir target/codex-test -p udd-image-codecs --features bc7-encode`
- `cargo test --target-dir target/codex-test -p udd-image-codecs rdo --features bc7-encode -- --nocapture`
- `cargo bench -p udd-image-codecs --features bc7-encode --bench bc7_encode -- --quick --rdo-case=rdo_default --rdo-stats`
- `cargo bench -p udd-image-codecs --features bc7-encode --bench bc7_encode -- --rdo-case=rdo_default --rdo-stats`

The release benchmark target may use a configured target directory outside the normal workspace sandbox.
