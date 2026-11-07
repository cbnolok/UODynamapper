# Temporary Bevy capture

Use this only after checking the project's current Bevy API and existing screenshot features. The following API shape was exercised with Bevy 0.20; the repository source/build is the authority for its current version.

A screenshot can be requested from the application without making its window active:

```rust
commands
    .spawn(bevy::render::view::screenshot::Screenshot::primary_window())
    .observe(bevy::render::view::screenshot::save_to_disk(capture_path.clone()));
```

Register a one-shot request at a known settled point. A frame counter is a temporary convenience, not proof that streaming has completed: inspect readiness or logs and the final image. Unfocused frame-rate throttling can make a fixed frame count take much longer than expected.

Keep any environment-variable capture hook confined to the experiment unless the user requested a durable feature. Write down the exact temporary diff so it can be removed without touching unrelated code. After visual checks, remove the hook, verify the diff contains no capture instrumentation, and make a normal build.

The save observer may create the destination file before encoding finishes. Wait for a PNG signature and final IEND chunk; then use the image-viewing tool to confirm it decodes and contains the intended scene. A file-existence check alone previously observed zero-byte, unfinished captures.

Use an isolated assets/settings root and fresh output path for each run. Do not change the primary window's visibility, active desktop, or the user's saved settings just to make an external screenshot utility work.
