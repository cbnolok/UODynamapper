# UODynamapper Code Overview

This document provides a high-level overview of the codebase, intended to help new contributors understand the project structure, code flow, and key components.

## 1. Project Goal

The primary goal of this project is to create an interactive 2D map renderer for the Ultima Online client, with a focus on recreating both the classic terrain rendering style and mimicking the one of the *Kingdom Reborn* client.

## 2. Entry Point & Core Setup

The application entry point is in `dynamapper/src/main.rs`, but it immediately hands off control to `dynamapper/src/core.rs`.

### `core.rs`: The Heart of the Application

This file is responsible for building and configuring the Bevy `App`. Here's a breakdown of its key responsibilities:

1.  **Configuration Loading**: It starts by loading settings from `config.toml` using the `settings` module. These settings control things like window size and debug options (e.g., wireframe rendering).

2.  **Bevy Plugin Configuration**: It configures Bevy's `DefaultPlugins` with custom settings for:
    *   **Window**: Sets the title, size, and resizability.
    *   **Logging**: Configures a custom log format.
    *   **Asset Handling**: Sets the `assets` directory as the root for loading assets.
    *   **Rendering**: Enables specific `wgpu` features required for effects like wireframes.

3.  **Plugin Registration**: This is where the application's functionality is assembled. It adds several types of plugins:
    *   **Third-Party Plugins**: `WireframePlugin` for debugging meshes and `FramepacePlugin` for framerate limiting.
    *   **Custom Engine Plugins**: The core logic of the application is modularized into custom plugins:
        *   `ControlsPlugin`: Handles player input.
        *   `RenderPlugin`: Manages all rendering logic, including setting up the scene, camera, and drawing the world.
        *   `SettingsPlugin`: Manages application settings.
        *   `TextureCachePlugin`: Caches land and item textures.
        *   `UOFilesPlugin`: Responsible for loading data from the Ultima Online game files.

## 3. Application State Machine

The application's lifecycle is managed by a state machine defined in `core/app_states.rs`. The primary state is `AppState`, which controls the main flow:

*   `AppState::StartupSetup`: The initial state where startup systems run.
*   `AppState::AssetsLoading`: (Implied) A state for loading game assets.
*   `AppState::InGame`: The main state where the game is running and interactive.

Transitions between these states are triggered by systems. For example, `advance_state_after_scene_setup_stage_2` moves the app from the startup/loading phase into the `InGame` state.

## 4. System Execution Order

Bevy's execution order is managed through **System Sets**. `core.rs` configures these sets to ensure a logical flow of operations during startup and each frame update.

*   **`Startup` Schedule**: The startup sets are configured to run in a specific sequence:
    1.  `StartupSysSet::First`: Initial setup.
    2.  `StartupSysSet::LoadStartupUOFiles`: Loads essential UO files.
    3.  `StartupSysSet::SetupSceneStage1` & `SetupSceneStage2`: Sets up the initial game scene (camera, player, world).
    4.  `StartupSysSet::Done`: Finalizes startup.

*   **`Update` Schedule**: In the main game loop, system sets ensure that player movement is processed before the camera is updated:
    1.  `MovementSysSet::MovementActions`: Processes player input.
    2.  `MovementSysSet::UpdateCamera`: Moves the camera to follow the player.

## 5. Code Flow Diagram

Here is a simplified diagram of the application flow from launch to the main game loop:

```
[ main() ]
    |
    v
[ core::run_bevy_app() ]
    |--> Load settings from config.toml
    |--> Create Bevy App
    |--> Add & Configure Bevy Plugins (Window, Assets, etc.)
    |--> Add Custom Plugins (Render, Controls, UOFiles, etc.)
    |--> Set Initial State: AppState::StartupSetup
    |
    v
--- STARTUP PHASE ---
[ PreStartup ] -> advance_state_after_init_core()
    |
    v
[ Startup ]
    |--> Load UO Files (UOFilesPlugin)
    |--> Setup Scene (RenderPlugin)
    |--> Setup Land Mesh (setup_land_mesh system)
    |--> advance_state_after_scene_setup_stage_2()
    |--> Transition to AppState::InGame
    |
    v
--- MAIN GAME LOOP (Update Schedule) ---
[ AppState::InGame ]
    |--> Process Player Input (ControlsPlugin)
    |--> Update Player Position
    |--> Update Camera Position
    |--> Render World (RenderPlugin)
        |--> Identify visible chunks
        |--> For each chunk:
            |--> create_land_chunk_material() in draw_chunk_mesh.rs
            |--> Prepare uniforms (height data, lighting, etc.)
            |--> Draw mesh with land_base.wgsl shader
    |--> Render UI
    |--> (Loop)
```

## 6. Terrain Rendering Pipeline

The rendering of the game world, especially the terrain, is a core feature. Here's a high-level look at how it works:

1.  **Chunk Management**: The world is divided into 8x8 tile chunks. The `RenderPlugin` contains logic to determine which chunks are visible to the camera.

2.  **Mesh Generation**: For each visible chunk that doesn't have a mesh yet, the `sys_draw_spawned_land_chunks` system in `draw_chunk_mesh.rs` is called.

3.  **Material Creation**: This system calls `create_land_chunk_material`, which is the bridge between the Rust code and the shader. This function is responsible for:
    *   Gathering the height and texture data for a **13x13 tile area** (the 8x8 chunk + a 2-tile border).
    *   Packing this data into uniform buffers (`LandUniform`, `LightingUniforms`, etc.).
    *   Creating a new `LandCustomMaterial` with this data.

4.  **Drawing**: Bevy then draws the chunk's mesh using this custom material. The GPU executes the `land_base.wgsl` shader, which uses the uniform data to displace the mesh vertices and calculate the final color for each pixel, resulting in the stylized terrain.
