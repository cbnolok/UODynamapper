---
name: renderer-reference-validation
description: Reproduce renderer defects, capture temporary frames with isolated settings, and compare actual output against supplied original-client reference images. Use for visual correctness checks of graphics, texture, material, geometry, and shader changes.
---

# Renderer reference validation

Build evidence from the actual renderer and original reference pixels. Separate observable mismatches from hypotheses about their cause; a plausible screenshot does not establish the original client's implementation.

## Establish a reproducible fixture

Read the repository's engineering and rendering guidance. Inspect current user changes before editing. Record the scene/map, coordinates, camera, zoom, resolution, asset sources/package versions, rendering preset, and relevant animation/effect settings.

Inspect the supplied reference images with the image-viewing tool. Check what each image actually depicts; file names and nearby screenshots do not prove matching coordinates or settings. Keep originals intact. Generated or retouched images cannot establish renderer fidelity.

Prepare an isolated temporary working/settings directory using the application's actual settings and cache precedence. Point it at the intended asset packages. Do not overwrite saved user preferences to obtain a test view. Give captures and logs descriptive names that identify the fixture and tested variant.

For KDE desktop placement, use the sibling [kde-x11-preview skill](../kde-x11-preview/SKILL.md) when the test must stay on a named desktop. Otherwise use the environment's appropriate isolated preview mechanism. Preserve any desktop constraint specified by the user.

## Capture the real output

Prefer an existing renderer-native screenshot mechanism. Wait for scene loading, atlas uploads, and shader compilation to settle before capture. Check the produced image and logs rather than assuming a fixed sleep produced a usable frame.

In this repository, first read the [renderer control commands](../../scripts/dev-helpers/renderer/README.md) and [control endpoint notes](../../docs/agent_notes/renderer_control_channel.md). Launch with `DYNAMAPPER_CONTROL_SOCKET` set to a new absolute path in the isolated run directory. The KDE preview helper inherits it. While the bounded preview runs, use a separate tool call from the repository root:

```sh
python3 scripts/dev-helpers/renderer/control.py --socket /tmp/my-preview/control.sock status
python3 scripts/dev-helpers/renderer/control.py --socket /tmp/my-preview/control.sock wait-ready
python3 scripts/dev-helpers/renderer/control.py --socket /tmp/my-preview/control.sock capture /tmp/my-preview/scene.png
```

Replace the sample paths with this run's actual paths. The capture parent must exist and the PNG destination must be new. `status.render` reports the effective preset and effects; readiness includes terrain and art queues but is not a GPU fence. App-only `key`, `text`, `pointer`, `click`, and `scroll` commands operate on inactive desktops without moving the real pointer or changing desktop focus. Use `reset-input` when finished. Follow the documented `wait-pick` workflow for picking checks.

For Bevy-specific temporary capture instrumentation, read [references/bevy-capture.md](references/bevy-capture.md). Remove diagnostic hooks before the production build unless a durable capture feature is part of the requested change.

Off-desktop XWayland windows can reject external `XGetImage`/ffmpeg capture with `BadMatch`. A direct renderer capture can work while the window remains on its assigned desktop. A newly created PNG may still be empty or incomplete; require the PNG signature and final IEND chunk before viewing. Keep baseline and changed captures separate.

When animation matters, capture more than one settled frame at recorded times or compare equivalent paused states. A single still cannot validate motion, reflection stability, or overlap flicker.

## Compare and diagnose

View the baseline, changed capture, and relevant original reference images. Compare the affected content, not just overall color or aesthetic similarity. Useful independent dimensions include:

- Selected texture/material and resource ownership.
- Authored art windows, orientation, repetition, and UV continuity.
- Geometry placement, height, occlusion, and depth behavior.
- Alpha coverage and composition across material/geometry boundaries.
- Lighting, reflections, animation, and postprocessing.

For texture/material defects, trace representative visible cells or objects through original source references, conversion, packed pixels/provenance, runtime lookup, and shader sampling. An improved runtime selector cannot change pixels already packed from a different source. Fix the earliest incorrect stage and recheck adjacent content families.

Use source data and preserved metadata to justify behavior. Name third-party reconstructions and shader guesses as such. Avoid coordinate-specific substitutions, arbitrary tinting, or disabling effects simply to conceal an unresolved source-selection problem.

Pixel-difference metrics help only when scene, framing, timing, and color handling are comparable. They do not replace visual inspection. A clean shader log and complete asset-slot coverage do not prove the right artwork was selected.

## Close the validation loop

Run the repository's required rendering presets and asset-source regressions. Revisit both the primary defect fixture and a previously accepted scene affected by shared code. Use focused structural tests for source selection, metadata, package conversion, and shader composition where they can prevent the observed failure.

Remove temporary instrumentation and debug assets, rebuild normally, and check the final diff. Keep user-provided references and unrelated edits untouched. Report the root cause, actual before/after evidence, checks performed, and visible limitations that remain. Record durable pipeline findings in the repository's appropriate engineering notes when its rules require that.
