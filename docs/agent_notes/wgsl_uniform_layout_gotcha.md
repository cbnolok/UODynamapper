# WGSL uniform layout gotcha: don't end a shared uniform struct with `scalar + vec3`

> Status (2026-10-25): the root cause below was naga-oil composition, which Bevy 0.20
> removed (shaders are now WESL, see `bevy_0_20_port_notes.md`, so `.wgsl` paths below are
> now `.wesl`). Not re-tested under WESL; keep the `vec4` slot convention anyway.
>
> Status (2026-11-12): `kr_tonemap_params` is gone (its `.x` gain is folded into `exposure`), and
> `grade_warm_color`, `grade_cool_color` and `gloom_params` are `vec3` each followed by an explicit
> `f32` pad, the same `vec3 + scalar` pattern as `light_color` + `tone_curve`. That order (vec3 first)
> is fine; validated at runtime with no `failed to create shader module`. The struct now ends with a
> `vec4`, so the trailing `scalar + vec3` case below does not arise.

Discovered 2026-07-08 while adding a `kr_tonemap_gain` field to
`GlobalLightingUniforms` (task: expose the KR tonemap brightness knob).

## Symptom

Rust `cargo check`/`cargo test` pass, but at **runtime** wgpu rejects the
shaders with (repeated once per pipeline that imports the struct):

```
failed to create shader module: Validation Error ... Entry point fragment ...
invalid function call ... Argument 1 value [N] doesn't match the type [M]
```

The error points at a *function call* (e.g. the `tonemap_ec_kr_profile` call),
not at the struct — misleading. `cargo` never catches it because WGSL is only
validated by naga/naga-oil at runtime.

## Root cause

`GlobalLightingUniforms` is defined once in
`dynamapper/assets/shaders/world/common_bindings.wgsl` and imported (via
naga-oil composition) into the land shader (`world/land/main.wgsl`) and both art
shaders (`world/art/main.wgsl`, `world/art/ground.wgsl`). Appending a trailing

```wgsl
kr_tonemap_gain: f32,
_pad_kr_: vec3<f32>,
```

(mirrored on the Rust side as `f32` + glam `Vec3`) makes naga-oil mis-compose
the struct — every downstream module that takes the struct by value ends up
with a mismatched type, surfacing as the bogus "argument doesn't match type" on
unrelated calls.

## Fix / rule

Add new data to that struct as a **full `vec4<f32>` slot**, exactly like the
existing `grade_params` / `grade_extra` / `gloom_params` / `fog_params` blocks —
pack scalars into `.x/.y/.z/.w`. A trailing `vec4` composes fine; a trailing
`scalar + vec3` does not. Applied fix: `kr_tonemap_params: vec4<f32>` (`.x` =
gain), matching `Vec4` on the Rust side.

## How to catch this class of bug

**Run the app** — `timeout 40 cargo run -p dynamapper` — and grep stdout for
`failed to create shader module`. WGSL edits do **not** trigger a Rust rebuild
(shaders load from `assets/` at runtime), so iterating on shader/layout fixes is
fast once the binary is built. HEAD renders with 0 such errors, so any count > 0
is a regression you just introduced. There is a display available in this
environment (`DISPLAY=:1`), so the app does start and create pipelines.
