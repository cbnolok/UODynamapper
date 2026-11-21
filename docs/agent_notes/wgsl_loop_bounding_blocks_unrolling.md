# naga's loop bounding stops the driver from unrolling compute loops

Discovered 2026-11-21 while porting the SPAN upscaler (`dynamapper/src/render/mobiles/upscale.rs`).

## Symptom

A tiled matrix-product kernel (`assets/shaders/upscale/conv.wgsl`, copied from `assets/shaders/rife/conv.wgsl`)
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

## Open

The RIFE kernel (`render/mobiles/rife.rs`, about 230 GMAC/s) has the same structure and creates its modules with
the default checks; it should gain similarly.
