# UODynamapper

A dynamic map renderer for Ultima Online, built with Rust and Bevy. Plans are to recreate both classic and modern (Kingdom Reborn/Enhanced Client) visuals with a single codebase.
It's still in an early stage.  
Status: Land/terrain rendering supported.

![Current state of version 0.1](./docs/screenshot-0.1.webp)

---

## Workspace Components

This project is organized as a Cargo workspace with several specialized components:

### Applications & Tools
- **[dynamapper](dynamapper/)**: The main application.
- **[uddconv_cli](uddconv_cli/)**: Asset converter for UODynamapper. Packs classic `.mul` art and tiledata into optimized `.uddp` packages.
- **[uocf_cli](uocf_cli/)**: General-purpose UO tooling, providing:
  - `uoptool`: Utility for inspecting, hashing, brute-force find original string value from its hash, and modifying modern `.uop` package files.
  - `cc_uop_mul_converter`: Converter for switching between legacy `.mul`/`.idx` and modern `.uop` formats.

### Libraries
- **[uocf](uocf/)**: A parser library for core Ultima Online file formats (Map, Art, Tiledata, UOP) for Classic, Enhanced and Kingdom Reborn clients. Support for custom formats is being added (Michelangelo's `.uop`, `.vd` files).
- **[uddconv](uddconv/)**: Shared library for UODynamapper-specific asset conversion and runtime data loading.

---

## Features

- Classic isometric (actually orthogonal military projection) view.
- Perspective view.
- Free camera movement and rotation (it will be limited if art sprites rendering is enabled).
- Free "player" movement and teleport via click or teleport menu (supports different maps).
- Different shading styles.

---

## Configuration

Edit `assets/settings.toml` to set your Ultima Online installation directory:

```toml
[uo_paths]
installation_dir = "/path/to/your/uo"
```

Check other toml configuration files in the 'assets' folder.

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
> All keybindings are configurable, edit `assets/keybindings.toml` to customize.

See [docs/keybindings.md](docs/keybindings.md) for the complete list with descriptions.

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
