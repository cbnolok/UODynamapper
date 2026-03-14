# UODynamapper

**Ultima Online Dynamic Map Renderer** — A highly configurable 2D client/dynamic map renderer for Ultima Online, built with Rust and Bevy. Recreate both classic and modern (Kingdom Reborn) visuals with a single codebase.

![Current state of version 0.1](./docs/screenshot-0.1.webp)

---

## Features

### Rendering
- ✅ **Three Shader Modes**: Classic 2D (faceted), Enhanced Classic (smooth), KR-like (full effects)
- ✅ **Paged Tile Atlas**: GPU-driven terrain metadata supporting massive maps (10,000×10,000+ tiles)
- ✅ **BC7 Texture Compression**: 8× VRAM reduction (~160MB → ~20MB)
- ✅ **Hot-Reload Shaders**: Automatic shader reloading for rapid iteration
- ✅ **Multi-Map Support**: Auto-discovers and renders all map planes (map0.mul through map5.mul)
- ✅ **Dynamic Chunk Loading**: Lazy loading with 60s idle eviction for low memory footprint

### User Interface
- ✅ **Performance Overlay**: Real-time FPS, CPU%, and RAM usage (top-right)
- ✅ **Player Position Display**: Live coordinates (top-left)
- ✅ **System Messages**: In-game log with severity-based colors and icons (bottom-left)
- ✅ **Terrain Shader Controls** (`F3`): Runtime adjustment of lighting, fog, color grading
- ✅ **Teleport Dialog** (`Ctrl+G`): Instant teleport to any coordinate
- ✅ **Keybindings Help** (`F1`): Quick reference overlay
- ✅ **Options Menu** (`F2`): General settings

### Controls
- ✅ **WASD Movement**: Navigate the map in all four directions
- ✅ **Vertical Movement**: PageUp/PageDown for altitude changes
- ✅ **Camera Zoom**: Scroll wheel to zoom in/out
- ✅ **Fullscreen Toggle**: F11 or Alt+Enter
- ✅ **Configurable Keybindings**: Fully customizable via `assets/keybindings.toml`

### Performance
- ✅ **Idle Eviction**: Automatic cleanup of inactive data after 60s
- ✅ **Power Saving Mode**: Reactive low-power mode when window is unfocused
- ✅ **Optimized Builds**: Release profile with LTO, stripping, and size optimization
- ✅ **Fast Linking**: CI uses `mold` linker for 3–5× faster builds

---

## Getting Started

### Prerequisites

- **Rust Toolchain**: Install from [rustup.rs](https://rustup.rs)
- **Ultima Online Files**: Required game data (map.mul, art.mul, tiledata.mul)

### Installation

```bash
# Clone the repository
git clone https://github.com/your-username/UODynamapper.git
cd UODynamapper

# Build the project
cargo build

# Run in debug mode
cargo run

# Run in release mode (optimized)
cargo run --release
```

### Configuration

Edit `assets/settings.toml` to set your Ultima Online installation directory:

```toml
[uo_paths]
installation_dir = "/path/to/your/uo"
```

---

## Documentation

| Document | Description |
|----------|-------------|
| **[docs/PROJECT_OVERVIEW.md](docs/PROJECT_OVERVIEW.md)** | High-level project summary, quick start, and current status |
| **[docs/CONTRIBUTORS_GUIDE.md](docs/CONTRIBUTORS_GUIDE.md)** | Quick file reference, common workflows, and troubleshooting |
| **[docs/CODE_OVERVIEW.md](docs/CODE_OVERVIEW.md)** | Low-level architecture details and design choices |
| **[docs/keybindings.md](docs/keybindings.md)** | Complete list of keyboard shortcuts |
| **[docs/TODO.md](docs/TODO.md)** | Planned features and future improvements |
| **[GEMINI.md](GEMINI.md)** | AI agent instructions and best practices |

**Recommended Reading Order**:
1. **New users**: Start with [PROJECT_OVERVIEW.md](docs/PROJECT_OVERVIEW.md)
2. **Contributors**: Read [CONTRIBUTORS_GUIDE.md](docs/CONTRIBUTORS_GUIDE.md) for quick reference
3. **Deep dive**: Consult [CODE_OVERVIEW.md](docs/CODE_OVERVIEW.md) for architecture details

---

## Keybindings

| Key | Action |
|-----|--------|
| `W` / `A` / `S` / `D` | Move player (NW / SW / SE / NE) |
| `PageUp` / `PageDown` | Increase / Decrease altitude |
| `Scroll Wheel` | Zoom in / out |
| `F1` | Keybindings help overlay |
| `F2` | Options menu |
| `F3` | Terrain shader controls |
| `Ctrl+G` | Teleport dialog |
| `F11` / `Alt+Enter` | Toggle fullscreen |
| `Esc` | Close dialogs |

> [!NOTE]
> All keybindings are configurable! Edit `assets/keybindings.toml` to customize.

See [docs/keybindings.md](docs/keybindings.md) for the complete list with descriptions.

---

## Screenshots

### Classic 2D Mode
Faceted normals, geometric look, Gouraud (per-vertex) lighting.

### Enhanced Classic Mode
Smooth bicubic normals, per-fragment lighting, subtle fill light.

### KR-like Mode
Full effects: rim lighting, specular highlights, procedural fog, color grading, tonemapping.

---

## Technical Highlights

- **Language**: Rust (Edition 2024)
- **Engine**: Bevy v0.18.1
- **Shaders**: WGSL (WESL + naga-oil)
- **GPU API**: wgpu
- **Texture Compression**: BC7 via `intel_tex_2`

### Architecture

- **Paged Tile Metadata Atlas**: Layered `Rg16Uint` texture array for terrain data
- **Uniform-Driven Shaders**: Shared bind groups for minimal material churn
- **LRU Cache**: Dynamic layer management with idle eviction
- **Modular Plugins**: Controls, Rendering, Settings, Texture Cache, UO Files, UI Overlays

---

## Development

### Build Commands

```bash
cargo build              # Debug build
cargo build --release    # Release build (optimized)
cargo run                # Run debug build
cargo run --release      # Run release build
cargo clippy             # Lint with Clippy
cargo fmt                # Format code with rustfmt
```

### Hot-Reload Workflow

1. Make shader changes in `assets/shaders/worldmap/land_base.wgsl`
2. Bevy automatically hot-reloads the shader
3. Use `F3` dialog to test different rendering modes in real-time
4. Verify changes across all three presets (Classic, Enhanced, KR-like)

---

## Roadmap

### Planned Features

- [ ] Split WGSL shader into multiple files for better organization
- [ ] SIMD optimizations for texture loading in `uocf` crate
- [ ] Move default shader preset to TOML configuration
- [ ] Adaptive far projection based on zoom level
- [ ] Hot-reload settings and shader presets without restart
- [ ] Dynamic texture array expansion (resize at runtime)

See [docs/TODO.md](docs/TODO.md) for the complete roadmap.

---

## License

This project is licensed under the MIT License — see the [LICENSE](LICENSE) file for details.

---

## Acknowledgments

- **Ultima Online**: Created by Richard Garriott / Origin Systems
- **Bevy Engine**: A refreshing simple data-driven game engine built in Rust
- **wgpu**: Cross-platform, safe, future-proof GPU API

---

**Last Updated**: sabato 14 marzo 2026  
**Bevy Version**: 0.18.1  
**Rust Edition**: 2024
