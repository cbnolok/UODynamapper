use bevy::prelude::*;
use bevy_egui::{
    EguiContext, EguiContextSettings, EguiFullOutput, EguiGlobalSettings, EguiInput,
    EguiPrimaryContextPass,
};

/// Plugin that provides reactive Egui rendering while the main application may remain continuous.
/// This optimizes CPU/GPU usage by skipping Egui passes when there is no input or repaint request.
pub struct EguiReactivePlugin;

impl Plugin for EguiReactivePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                sys_configure_egui_to_reactive,
                sys_run_egui_pass_reactively.after(sys_configure_egui_to_reactive),
                sys_toggle_egui_input_absorption,
            ),
        );
    }
}

/// One-time configuration for new Egui contexts.
/// Sets them to manual mode and increases max_passes to avoid flickering.
fn sys_configure_egui_to_reactive(
    mut query: Query<
        (
            &mut EguiContext,
            &mut EguiContextSettings,
        ),
        Added<EguiContextSettings>,
    >,
) {
    for (mut ctx, mut settings) in &mut query {
        settings.run_manually = true;

        // Increase max_passes to avoid initial flickering ("garbage") when dialogs open.
        // This allows egui to settle its layout in a single Bevy frame.
        ctx.get_mut().memory_mut(|mem| {
            mem.options.max_passes = std::num::NonZeroUsize::new(4).unwrap();
        });
    }
}

/// Manually runs the Egui pass only if there is a reason to (input or repaint request).
/// This allows Egui to be reactive even if WinitSettings are set to Continuous.
fn sys_run_egui_pass_reactively(world: &mut World) {
    let mut q = world.query::<(
        Entity,
        &mut EguiContext,
        &mut EguiInput,
        &mut EguiFullOutput,
        &EguiContextSettings,
    )>();

    // We'll collect the data to run to avoid double-borrowing World.
    let mut contexts_to_run = Vec::new();
    for (entity, mut ctx, mut input, _, settings) in q.iter_mut(world) {
        if settings.run_manually {
            // We must run the context every frame to provide the output required by bevy_egui's
            // output processing system, even if the world is continuous.
            contexts_to_run.push((entity, ctx.get_mut().clone(), input.take()));
        }
    }

    for (entity, ctx, input) in contexts_to_run {
        // Run the egui pass. This calls our UI systems registered in EguiPrimaryContextPass.
        let output = ctx.run(input, |_| {
            // IMPORTANT: This triggers the schedule where all our UI systems live.
            let _ = world.try_run_schedule(EguiPrimaryContextPass);
        });

        // Store the output so bevy_egui's output system can process it in PostUpdate.
        if let Ok((.., mut full_output, _)) = q.get_mut(world, entity) {
            **full_output = Some(output);
        }
    }
}

/// Dynamically toggles input absorption based on whether egui actually wants input.
fn sys_toggle_egui_input_absorption(
    mut egui_global_settings: ResMut<EguiGlobalSettings>,
    mut egui_contexts: bevy_egui::EguiContexts,
) {
    if let Ok(ctx) = egui_contexts.ctx_mut() {
        let wants_input =
            ctx.wants_pointer_input() || ctx.wants_keyboard_input() || ctx.is_using_pointer();
        if egui_global_settings.enable_absorb_bevy_input_system != wants_input {
            egui_global_settings.enable_absorb_bevy_input_system = wants_input;
        }
    }
}
