# UODynamapper

**Ultima Online Dynamic Map Renderer** — A dynamic map renderer for Ultima Online, built with Rust and Bevy. Plans are to recreate both classic and modern (Kingdom Reborn/Enhanced Client) visuals with a single codebase.
It's still in an early stage.  
Status: Land/terrain rendering supported.

![Current state of version 0.1](./docs/screenshot-0.1.webp)

---

## Configuration

Edit `assets/settings.toml` to set your Ultima Online installation directory:

```toml
[uo_paths]
installation_dir = "/path/to/your/uo"
```

Check other toml configuration files in the 'assets' folder.

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
> All keybindings are configurable, edit `assets/keybindings.toml` to customize.

See [docs/keybindings.md](docs/keybindings.md) for the complete list with descriptions.
