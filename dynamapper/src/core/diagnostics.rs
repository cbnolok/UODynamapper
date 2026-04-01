use bevy::prelude::*;

// Low level GPU diagnostics, used during development
const DUMP_DIAGNOSTICS_INTERVAL_SEC: u64 = 0; // 0 = don't do it. If needed, 2 seconds is good.

pub fn add_diagnostics_plugins(app: &mut App) {
    app.add_plugins((
        bevy::diagnostic::FrameTimeDiagnosticsPlugin::default(),
        // `SystemInformationDiagnosticsPlugin` is intentionally NOT used here:
        // it fails to initialize with dynamic_linking enabled (emits a 'not supported'
        // warning and returns no data). We read CPU/RAM directly via `sysinfo` instead
        // (see `core/render/overlays/performance.rs`).

        // GPU pipeline statistics (vertex/fragment invocations, clipper primitives).
        // On Vulkan/DX12 also provides GPU elapsed time per pass.
        // NOTE: no public DiagnosticPath constants in 0.18 — paths are dynamic strings
        // like "render/{span_name}/fragment_shader_invocations".
        // Use LogDiagnosticsPlugin below to discover the exact span names.
        bevy::render::diagnostic::RenderDiagnosticsPlugin,
    ));

    if DUMP_DIAGNOSTICS_INTERVAL_SEC != 0 {
        app.add_plugins((
            // Temporary: dump all render/diagnostics to console every 2 seconds.
            // filter=None means log ALL diagnostics (FPS + render stats).
            // Once we know the exact span names, narrow to just the render ones.
            bevy::diagnostic::LogDiagnosticsPlugin {
                filter: None,
                wait_duration: std::time::Duration::from_secs(DUMP_DIAGNOSTICS_INTERVAL_SEC),
                ..default()
            },
        ));
    }
}

/// One-shot system (render world) that logs the active GPU preprocessing mode.
/// Helps verify whether Bevy is using full GPU culling + indirect draw.
pub fn sys_log_gpu_preprocessing_mode(
    support: Res<bevy::render::batching::gpu_preprocessing::GpuPreprocessingSupport>,
) {
    use bevy::render::batching::gpu_preprocessing::GpuPreprocessingMode;
    let mode_str = match support.max_supported_mode {
        GpuPreprocessingMode::None => "None (CPU-only, WebGL2 / no compute)",
        GpuPreprocessingMode::PreprocessingOnly => {
            "PreprocessingOnly (GPU uniforms, CPU draw calls)"
        }
        GpuPreprocessingMode::Culling => "Culling (full GPU frustum culling + multi_draw_indirect)",
    };
    bevy::log::info!("[GPU Batching] GpuPreprocessingMode = {mode_str}");
}
