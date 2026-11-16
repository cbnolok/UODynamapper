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

## Regression harness

[`scripts/visual_harness.py`](scripts/visual_harness.py) captures the fixed views of [`fixtures.toml`](fixtures.toml) with a given binary and compares them with a baseline. It applies the preview rules itself: Desktop 4 through the exact-PID preview helper, private copies of the settings and cache, the shared preview lock held per launch, positioning and capture through the control socket, and a discarded first capture after a launch and after every runtime switch. It also freezes water, lava and KR fog drift, and repeats each capture until two consecutive frames agree, at most four times. Run it with `python3 -I -B` from any checkout. `--tree` names the checkout whose `dynamapper/assets` the runs copy; the default is the script's own.

For every visual change:

1. Before the change, build and record a baseline: `visual_harness.py record before-<topic> --binary <old dynamapper>`. A baseline from the same commit can be reused.
2. Make the change and build.
3. Run `visual_harness.py compare before-<topic> after-<topic> --binary <new dynamapper>`. It captures the label unless it exists and writes `summary.json` and sheets (baseline, current, |diff| ×4, client reference) under `--root`. The exit status is 0 when nothing changed beyond each fixture's tolerance, 1 on changes and 2 on errors.
4. View the sheets of the changed fixtures, and attach the summary and the sheets that show the intended change to the report. An intended change is still listed as changed; say why it is expected.
5. Add a fixture for every visual bug fixed: the smallest view that showed it, with `guards` naming the bug and the fix commit.

The default selection is the quick tier: 52 captures in about 4 launches, about 3 minutes when the preview lock is free (24 launches and 5 to 10 minutes with `--fresh-launch`). Use `--tier full` (218 captures in about 12 launches, about 13 minutes; 74 launches and 16 to 22 minutes with `--fresh-launch`) before a milestone or after shared shader, lighting or atlas changes. Two runs of one binary match exactly except lava at night, which varies by up to 3 levels; a capture whose consecutive frames never agree is marked unstable in the manifest and the compare output. Narrow a run with `--only <name or tag glob>`, `--style`, `--source art/land` and `--time`. `list -v` shows the fixtures and the launches they need. `sheet <label>` draws a label's captures with the styles side by side. `clean --keep N` drops old labels. `self-test` checks the parser, the settings rewriting and the diff math without a GUI.

By default one launch per map goes from fixture to fixture and switches the style, time of day, art and land sources, zoom and allowlisted settings through the control socket (`preset`, `sources`, `set`; see [the control channel note](../../docs/agent_notes/renderer_control_channel.md#runtime-switches)), the same code path as the UI. It holds the preview lock for about `--chunk-seconds` (60 by default) and then closes, so other agents' previews get their turn, and the next launch goes on. `--fresh-launch` restores one launch per style, time, source, map and settings set. A fixture with an override the socket cannot switch (a `preset` override, another graphics key, the camera) always gets launches of its own, as does every capture with a binary built before the switch commands. Both modes give the same pixels within tolerance, and so does a launch of one fixture alone: the night glows no longer depend on the views the launch visited before (2026-11-16, full tier at night, live, `--fresh-launch` and one launch per capture of the keep and Britain lamp-post fixtures, at most 2 levels apart). The manifest records the mode of the label and of each launch.

Each capture stores the collected statics' digest (`status.art.digest`), which does not depend on the loading order. When the baseline and the label come from one binary (a live run against a `--fresh-launch` one), compare marks a capture changed if the digests differ, even when the pixels stay within tolerance. `--reload-land-check` also sends `reload_land` after each capture, dropping the loaded map blocks and the cached statics as 60 idle seconds would, and reports a problem when the statics rebuilt from scratch differ from the cached ones. It would have caught the client shadows that lost their land after idling (2026-11-16) and costs about a second per capture.

Labels default to `/home/claudio/rust-target/uodynamapper-scratch/visual-baselines/`, outside the repository. A label stores the tree commit, the binary hash and a hash of each fixture's view. A fixture edited after the baseline reports `stale` instead of a diff. Captures are lossless WebP by default; `--format jpeg` is leaner for throwaway runs.

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
