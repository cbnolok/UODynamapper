//! Frame rate as completed frames per wall-clock second.
//!
//! Bevy's `FrameTimeDiagnosticsPlugin::FPS` smooths the per-frame reciprocal `1 / dt`. That average
//! reads high whenever frame times vary, because short frames weigh more than long ones: frames of
//! 5 ms and 28 ms alternating are 60.6 frames per second, while their mean reciprocal reads 118.
//! Pipelined rendering makes this common. `time_system` takes `dt` from a timestamp sent by the
//! render thread and falls back to `Instant::now` when none has arrived, so single samples mix two
//! clocks. Counting frames over a window has neither problem, and pipelined rendering stays on.
use crate::prelude::*;
use bevy::prelude::*;
use std::time::{Duration, Instant};

/// How often the published rate refreshes. The window always closes on a frame boundary, so it is
/// also accurate below two frames per second.
const FRAME_RATE_WINDOW: Duration = Duration::from_millis(500);
const MILLISECONDS_PER_SECOND: f64 = 1000.0;

#[derive(Resource, Default)]
pub struct MeasuredFrameRate {
    window_start: Option<Instant>,
    frames_in_window: u32,
    frames_per_second: Option<f64>,
    total_frames: u64,
}

impl MeasuredFrameRate {
    /// Frames per second over the last closed window. `None` until the first window closes.
    pub fn frames_per_second(&self) -> Option<f64> {
        self.frames_per_second
    }

    /// Mean frame time over the last closed window, in milliseconds.
    pub fn frame_ms(&self) -> Option<f64> {
        self.frames_per_second.map(|fps| MILLISECONDS_PER_SECOND / fps)
    }

    /// Every frame counted so far, for checking the rate against an outside clock.
    pub fn total_frames(&self) -> u64 {
        self.total_frames
    }

    fn record_frame(&mut self, now: Instant) {
        self.total_frames += 1;
        let Some(window_start) = self.window_start else {
            self.window_start = Some(now);
            return;
        };
        self.frames_in_window += 1;
        let elapsed = now.saturating_duration_since(window_start);
        if elapsed >= FRAME_RATE_WINDOW {
            self.frames_per_second = Some(f64::from(self.frames_in_window) / elapsed.as_secs_f64());
            self.window_start = Some(now);
            self.frames_in_window = 0;
        }
    }
}

pub struct FrameRatePlugin {
    pub registered_by: &'static str,
}
impl_tracked_plugin!(FrameRatePlugin);

impl Plugin for FrameRatePlugin {
    fn build(&self, app: &mut App) {
        log_plugin_build(self);
        app.init_resource::<MeasuredFrameRate>().add_systems(Last, sys_measure_frame_rate);
    }
}

fn sys_measure_frame_rate(mut frame_rate: ResMut<MeasuredFrameRate>) {
    frame_rate.record_frame(Instant::now());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_frames(frame_rate: &mut MeasuredFrameRate, start: Instant, frame_times_ms: &[u64]) -> Instant {
        let mut now = start;
        for frame_ms in frame_times_ms {
            now += Duration::from_millis(*frame_ms);
            frame_rate.record_frame(now);
        }
        now
    }

    #[test]
    fn jittered_frames_report_the_real_rate_and_not_the_mean_reciprocal() {
        const JITTER_PAIR_MS: [u64; 2] = [5, 28];
        const PAIR_COUNT: usize = 40;
        let expected = 2.0 * MILLISECONDS_PER_SECOND / (JITTER_PAIR_MS[0] + JITTER_PAIR_MS[1]) as f64;
        let mean_reciprocal = JITTER_PAIR_MS.iter()
            .map(|ms| MILLISECONDS_PER_SECOND / *ms as f64).sum::<f64>() / JITTER_PAIR_MS.len() as f64;
        let frames: Vec<u64> = JITTER_PAIR_MS.iter().copied().cycle().take(2 * PAIR_COUNT).collect();

        let mut frame_rate = MeasuredFrameRate::default();
        let start = Instant::now();
        frame_rate.record_frame(start);
        run_frames(&mut frame_rate, start, &frames);

        // A window closes on a frame boundary, so it can differ from the long-run rate by one frame.
        let one_frame_in_a_window = expected * JITTER_PAIR_MS[1] as f64 / FRAME_RATE_WINDOW.as_millis() as f64;
        let measured = frame_rate.frames_per_second().expect("a window closed");
        assert!((measured - expected).abs() < one_frame_in_a_window, "measured {measured}, expected {expected}");
        assert!(mean_reciprocal > expected * 1.9, "the old reading was {mean_reciprocal}");
    }

    #[test]
    fn a_rate_below_the_window_length_is_still_exact() {
        const FRAME_MS: u64 = 1500;
        let mut frame_rate = MeasuredFrameRate::default();
        let start = Instant::now();
        frame_rate.record_frame(start);
        run_frames(&mut frame_rate, start, &[FRAME_MS, FRAME_MS]);

        let measured = frame_rate.frames_per_second().expect("a window closed");
        assert!((measured - MILLISECONDS_PER_SECOND / FRAME_MS as f64).abs() < 1e-9);
        assert!((frame_rate.frame_ms().unwrap() - FRAME_MS as f64).abs() < 1e-6);
        assert_eq!(frame_rate.total_frames(), 3);
    }

    #[test]
    fn nothing_is_published_before_the_first_window_closes() {
        let mut frame_rate = MeasuredFrameRate::default();
        let start = Instant::now();
        frame_rate.record_frame(start);
        run_frames(&mut frame_rate, start, &[1, 1, 1]);
        assert_eq!(frame_rate.frames_per_second(), None);
        assert_eq!(frame_rate.frame_ms(), None);
    }
}
