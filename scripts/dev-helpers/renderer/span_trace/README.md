# Span trace

Finds where a hitch's time goes: which systems ran long in the frames that ran long. The renderer writes the
trace (`dynamapper/src/app/span_trace.rs`); these scripts run it and read it.

- **The trace** is on when `DYNAMAPPER_SPAN_TRACE` names a file. Without it, the app installs no trace layer and
  pays nothing. The file holds every span of at least 50 µs, as `thread start_us duration_us label`, and every
  frame's `update` span.
- **Bevy's per-system spans** need the `span_trace` cargo feature (`bevy/trace`). They cost every frame, so the
  normal builds go without them. Without the feature, the trace holds only the app's own spans and the backends'.

## A walk, traced

```sh
S=/home/claudio/rust-target/uodynamapper-scratch
cargo build --profile profiling -p dynamapper --features span_trace
cp /home/claudio/rust-target/uodynamapper/profiling/dynamapper "$S/dynamapper-trace"   # .cargo/config.toml target-dir
T="$S/trace" && mkdir -p "$T"
KEYLOG="$T/keys.log" DYNAMAPPER_SPAN_TRACE="$T/trace.txt" \
  python3 -I -B scripts/dev-helpers/renderer/span_trace/bench.py run "$T/bench.json" \
  --binary trace="$S/dynamapper-trace" --repetitions 1 --camera orthographic --style kr --motion walk
```

`bench.py` is the renderer's frame benchmark (`skills/renderer-reference-validation/scripts/frame_benchmark.py`,
same arguments). It also logs each key press and release to `$KEYLOG`, so the walks can be found in the trace. Each
launch rewrites the trace file, so trace one binary, one style and one repetition at a time. A launch walks once
per view: first the land, then Britain.

## Reading it

- `long_frames.py TRACE KEYLOG [WALK]`: the frames of one walk (default the last) that took over 33 ms. For each, it
  shows how busy the main and render apps were and the leaf spans that took the time. Then it sums those spans
  over all the long frames.
- `spikes.py TRACE KEYLOG [NAME,...]`: per walk, the leaf spans of at least 4 ms by label, without the GPU waits.
- `perf_callees.py PERF_TXT FUNCTION...`: once a span is known, where its time goes. Record with `perf record -g`
  on the same build, dump with `perf script -F tid,ip,sym > PERF_TXT`, and name the span's function. It lists the
  samples under that function by direct callee and by leaf.

Keep traces and perf data outside the repository: a walk's trace is about 13 MB, and its perf data hundreds.
