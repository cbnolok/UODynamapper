# naga's loop bounding stops the driver from unrolling compute loops

Discovered 2026-11-21 while porting the SPAN upscaler (now `lib/support/uot-gpu-compute/src/upscale.rs`).

## Symptom

A tiled matrix-product kernel (`shaders/upscale/conv.wgsl`, copied from `shaders/rife/conv.wgsl`, now in `lib/support/uot-gpu-compute/`)
ran at about 270 GMAC/s on the Radeon 880M (RADV), far below what its register tile should give.

## Cause

`device.create_shader_module` uses `ShaderRuntimeChecks::checked()`, which includes `force_loop_bounding`: naga
adds a hidden counter and a break to every loop, so that a loop the driver might assume finite cannot hang. The
counter makes every loop look data dependent, so ACO does not unroll the constant-trip inner loops. The ISA
(`RADV_DEBUG=shaders,shaderstats`) showed each FMA of the micro-tile followed by a counter update and a branch.

## Fix

Create the module with the bounding off and the bounds checks kept:

```rust
let checks = wgpu::ShaderRuntimeChecks { force_loop_bounding: false, ..wgpu::ShaderRuntimeChecks::checked() };
unsafe { device.create_shader_module_trusted(descriptor, checks) }
```

It is sound only when every loop in the shader provably ends (constant or uniform bounds, positive steps). The SPAN
network went from 105 ms to about 29 ms at 256x256 (3.3x to 3.6x across sizes); the unrolled kernel shows 768
`v_fmac` per k-step, no scratch, 128 VGPRs. Turning `bounds_checks` off as well gained about 2%, not worth it.

`network_shader_module` in `uot-gpu-compute` (`src/lib.rs`) does this; it is `unsafe`, and each caller states why its shader's
loops end.

## RIFE

The RIFE modules (conv.wgsl, pixel.wgsl, and the frame steps now in uot-gpu-compute's rife/frames.wgsl) use it too since
2026-11-21. One t went from 2.83 / 3.77 / 9.22 / 35.3 ms to 1.41 / 2.01 / 4.80 / 17.2 ms at 64² / 128² / 256² / 576×512,
about 2x rather than the upscaler's 3.5x: the small sizes are mostly fixed per-dispatch cost, and the pixel steps did
not change (the block inputs of blocks 2 to 4, which warp the frames, take 2.6 ms of the 15 at 576×512). Split-K is
still needed: without it one t at 64² takes 8.6 ms; doubling `TARGET_WORKGROUPS` to 192 was 2-10% slower at 128² and up.
