# UODynamapper Code Overview

This document provides a high-level overview of the codebase, intended to help new contributors understand the project structure, code flow, and key components.

## 1. Project Goal

The primary goal of this project is to create an interactive 2D map renderer for the Ultima Online client, with a focus on recreating both the classic terrain rendering style and mimicking the one of the *Kingdom Reborn* client.

## 2. Entry Point & Core Setup

The application entry point is in `dynamapper/src/main.rs`, but it immediately hands off control to `dynamapper/src/core.rs`.

### `core.rs`: The Heart of the Application

This file is responsible for building and configuring the Bevy `App`. Here's a breakdown of its key responsibilities:

1. **Configuration Loading**: It starts by loading settings from `config.toml` using the `settings` module. These settings control things like window size and debug options (e.g., wireframe rendering).

2. **Bevy Plugin Configuration**: It configures Bevy's `DefaultPlugins` with custom settings for:
    * **Window**: Sets the title, size, and resizability.
    * **Logging**: Configures a custom log format.
    * **Asset Handling**: Sets the `assets` directory as the root for loading assets.
    * **Rendering**: Enables specific `wgpu` features required for effects like wireframes.

3. **Plugin Registration**: This is where the application's functionality is assembled. It adds several types of plugins:
    * **Third-Party Plugins**: `WireframePlugin` for debugging meshes and `FramepacePlugin` for framerate limiting.
    * **Custom Engine Plugins**: The core logic of the application is modularized into custom plugins:
        * `ControlsPlugin`: Handles player input.
        * `RenderPlugin`: Manages all rendering logic, including setting up the scene, camera, and drawing the world.
        * `SettingsPlugin`: Manages application settings.
        * `TextureCachePlugin`: Caches land and item textures.
        * `UOFilesPlugin`: Responsible for loading data from the Ultima Online game files.

## 3. Application State Machine

The application's lifecycle is managed by a state machine defined in `core/app_states.rs`. The primary state is `AppState`, which controls the main flow:

* `AppState::StartupSetup`: The initial state where startup systems run.
* `AppState::AssetsLoading`: (Implied) A state for loading game assets.
* `AppState::InGame`: The main state where the game is running and interactive.

Transitions between these states are triggered by systems. For example, `advance_state_after_scene_setup_stage_2` moves the app from the startup/loading phase into the `InGame` state.

## 4. System Execution Order

Bevy's execution order is managed through **System Sets**. `core.rs` configures these sets to ensure a logical flow of operations during startup and each frame update.

* **`Startup` Schedule**: The startup sets are configured to run in a specific sequence:
    1. `StartupSysSet::First`: Initial setup.
    2. `StartupSysSet::LoadStartupUOFiles`: Loads essential UO files.
    3. `StartupSysSet::SetupSceneStage1` & `SetupSceneStage2`: Sets up the initial game scene (camera, player, world).
    4. `StartupSysSet::Done`: Finalizes startup.

* **`Update` Schedule**: In the main game loop, system sets ensure that player movement is processed before the camera is updated:
    1. `MovementSysSet::MovementActions`: Processes player input.
    2. `MovementSysSet::UpdateCamera`: Moves the camera to follow the player.

## 5. Code Flow Diagram

Here is a simplified diagram of the application flow from launch to the main game loop:

```text
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
            |--> Gather 8x8 tile metadata (ID, height)
            |--> Enqueue to TileAtlas in draw_mesh.rs
        |--> Draw mesh with land_base.wgsl shader
    |--> Evict Idle Blocks/Textures (sys_evict_map_blocks)
    |--> Render UI
    |--> (Loop)
```

## 6. Terrain Rendering Pipeline

The rendering of the game world, especially the terrain, is a core feature. Here's a high-level look at how it works:

1. **Chunk Management**: The world is divided into 8x8 tile chunks. The `RenderPlugin` determines visible chunks based on camera position.

2. **Paged Tile Metadata Atlas**: Terrain metadata is stored in a layered `Rg16Uint` texture array (Tile Metadata Atlas) instead of per-chunk uniforms. This architecture solves two primary problems:
    * **Material Churn**: By moving metadata to a global atlas, thousands of chunks can share a single material and bind group, virtually eliminating the CPU/GPU stalls caused by frequent uniform buffer updates.
    * **Scalability**: A paged system (using 2048x2048 layers) allows for massive maps (10,000x10,000+) while respecting GPU hardware limits on texture dimensions.

3. **Data Format & Packing**:
    The atlas uses a **4-byte-per-texel** (`Rg16Uint`) format to represent each world tile:
    * **R (16-bit)**: `tile_id` (0..65535).
    * **G (16-bit)**: Packed metadata:
        * **Low 8 bits**: `height_biased` (signed i8 height + 128 offset).
        * **High 8 bits**: `tex_size` flag (0 = small 64x64 texture array, 1 = big 128x128 array).

4. **Coordinate Mapping & Sampling**:
    * **CPU**: When a chunk is spawned, its 8x8 tile region is mapped to a logical page. The `TileAtlas` LRU cache assigns a physical layer and enqueues subregion updates via `queue.write_texture`.
    * **GPU**: The `land_base.wgsl` shader resolves world coordinates into `(layer, uv)` using `AtlasParams`. It uses `textureLoad` for deterministic integer lookups of IDs and heights.
    * **Neighborhood Sampling**: High-quality bicubic normals and slopes are calculated by reading neighboring texels directly from the atlas. Since the atlas is global/paged, sampling across chunk boundaries is seamless without requiring per-chunk padding.

5. **Uniform Management**:
    * Shared scene data (Lighting, Effects, Globals) are extracted into a shared bind group to further maximize batching efficiency.
    * The `LandCustomMaterial` remains slim, binding only the Tile Atlas handle and layout parameters.

6. **Optimization & Implementation Details**:
    * **Sequential I/O**: The map loader (`map.rs`) groups non-contiguous block requests into sequential ranges to minimize filesystem seeks and reads.
    * **Lazy Texture Residency**: `TexMap2D` lazy-loads land texture pixels from `.mul` files on-demand, caching them in an `Arc<Vec<u8>>` with 60s idle eviction.
    * **BC7 Alignment**: When uploading BC7-compressed textures via WGPU's `write_texture`, `bytes_per_row` is calculated as `(width + 3) / 4 * 16` to align with 4x4 pixel blocks.
    * **Idle Eviction**: An eviction system (`sys_evict_map_blocks`) checks for inactive `MapBlock`s and textures every 5 seconds, dropping data older than 60 seconds.


