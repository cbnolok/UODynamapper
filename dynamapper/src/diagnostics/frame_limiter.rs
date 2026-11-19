use crate::prelude::*;
use bevy::{
    app::MainScheduleOrder,
    ecs::schedule::ScheduleLabel,
    prelude::*,
};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Limiter {
    Off,
    Manual(Duration),
}

impl Limiter {
    pub fn from_framerate(fps: f64) -> Self {
        if fps > 0.0 {
            Duration::try_from_secs_f64(fps.recip()).map_or(Self::Off, Self::Manual)
        } else {
            Self::Off
        }
    }
}

/// The limiter the performance settings ask for: off while vsync paces the frames.
pub fn frame_limiter_for_options(frame_limit_enabled: bool, target_fps: u32, vsync: bool) -> Limiter {
    if frame_limit_enabled && !vsync {
        Limiter::from_framerate(target_fps as f64)
    } else {
        Limiter::Off
    }
}

pub fn frame_limiter_for_settings(settings: &crate::settings::Settings) -> Limiter {
    frame_limiter_for_options(
        settings.app.performance.frame_limit_enabled,
        settings.app.performance.target_fps,
        settings.graphics.vsync,
    )
}

/// Applies the saved performance settings to the limiter whenever the settings change. The preferences dialog
/// registers it.
pub fn sys_apply_performance_settings(
    settings: Res<crate::settings::Settings>,
    mut frame_limiter: ResMut<FrameLimiterSettings>,
) {
    if settings.is_changed() {
        log_system_add_update::<crate::ui::dialogs::preferences::PreferencesDialogPlugin>(fname!());
        frame_limiter.limiter = frame_limiter_for_settings(settings.as_ref());
    }
}

#[derive(Resource)]
pub struct FrameLimiterSettings {
    pub limiter: Limiter,
}

impl Default for FrameLimiterSettings {
    fn default() -> Self {
        Self {
            limiter: Limiter::Off,
        }
    }
}

/// Runs right after `Last`, so the limiter sleep is the final step of each main-world update.
#[derive(ScheduleLabel, Debug, Clone, PartialEq, Eq, Hash)]
struct FrameLimit;

pub struct FrameLimiterPlugin {
    pub registered_by: &'static str,
}
impl_tracked_plugin!(FrameLimiterPlugin);

impl Plugin for FrameLimiterPlugin {
    fn build(&self, app: &mut App) {
        log_plugin_build(self);
        app.init_resource::<FrameLimiterSettings>()
            .init_schedule(FrameLimit)
            .add_systems(FrameLimit, sys_frame_limiter_sleep);
        app.world_mut()
            .resource_mut::<MainScheduleOrder>()
            .insert_after(Last, FrameLimit);
    }
}

#[derive(Default)]
struct FrameLimiterState {
    frame_time: Duration,
    deadline: Option<Instant>,
}

/// Sleeps in the main world at the end of `Last` (in `FrameLimit`) until the next frame deadline.
/// This also throttles the pipelined render thread, since it waits on extract.
/// Deadlines advance by one frame time; if a frame is late by more than a full frame
/// (or the limiter just changed), the deadline resyncs to now instead of bursting to catch up.
fn sys_frame_limiter_sleep(
    settings: Res<FrameLimiterSettings>,
    mut state: Local<FrameLimiterState>,
) {
    let Limiter::Manual(frame_time) = settings.limiter else {
        state.deadline = None;
        return;
    };

    let now = Instant::now();
    let deadline = match state.deadline {
        Some(prev) if state.frame_time == frame_time => {
            let next = prev + frame_time;
            if now > next + frame_time { now } else { next }
        }
        _ => now,
    };
    state.frame_time = frame_time;
    state.deadline = Some(deadline);

    if deadline > now {
        spin_sleep::sleep(deadline - now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_framerate_rejects_non_positive_fps() {
        assert_eq!(Limiter::from_framerate(0.0), Limiter::Off);
        assert_eq!(Limiter::from_framerate(-30.0), Limiter::Off);
        assert_eq!(Limiter::from_framerate(f64::NAN), Limiter::Off);
    }

    #[test]
    fn from_framerate_converts_fps_to_frame_time() {
        const FPS: f64 = 50.0;
        const FRAME_TIME_MS: u64 = 20;
        assert_eq!(
            Limiter::from_framerate(FPS),
            Limiter::Manual(Duration::from_millis(FRAME_TIME_MS))
        );
    }
}
