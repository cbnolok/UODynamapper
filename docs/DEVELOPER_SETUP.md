# First-time developer setup

Start with [the user quick start](../README.md#quick-start) if you already have the application and tools archives.

## Build from source

Open a shell at the repository root with Rust and Cargo installed. Install `just` for the release, profiling, and conversion recipes. The repository's [Cargo configuration](../.cargo/config.toml) contains a checkout-local `target-dir` and Linux linker settings. Adjust the local target path and use the configured compiler and linker on your machine. The [release workflow](../.github/workflows/release.yml) lists the build dependencies used on each platform.

```sh
cargo build --locked -p dynamapper -p udd-conv-gui
```

## Runtime packages and settings

Build runtime packages from local Classic and Enhanced Client data with `udd-conv-gui` or `udd-cli pack`. A Classic-only world can use Classic packages. For the Kingdom Reborn sections, the converters can use the bundled KR delta with Enhanced Client data or take data from a local Kingdom Reborn installation. Point Dynamapper at the resulting UDDP directory. See the [asset pipeline guide](dev_wiki/ASSET_PIPELINE.md) for examples and package details.

The bundled shaders, fonts, and settings are in [dynamapper/assets/](../dynamapper/assets/). Configure the package directory in [runtime_assets.toml](../dynamapper/assets/settings/runtime_assets.toml): the default `uddp/` path is relative to the working directory. Settings saved by the app override the bundled settings and are stored in a `cache/` directory alongside the resolved assets directory. Likewise a copy of a file of `assets/runtime_specs/` at the same path under `cache/runtime_specs/` (such as `cache/runtime_specs/paperdoll/PaperdollOverrides.kdl`, which the paperdoll windows' right-click menu writes) is read in its place.

The application looks for an `assets/` directory in the working directory, the Cargo crate directory, or next to the executable. The package directory can be elsewhere; it need not be inside the assets tree. The conversion guide and [package format reference](dev_wiki/UDDP_FORMATS.md) describe the available packages.

Keybindings are configurable; see [the default keybindings](keybindings.md).

To build and run a specific workspace binary from source, use the generic Just recipes so
the linker flags, Cargo flags, and toolchain match the corresponding build:

```sh
just run-bin-release udd-cli --help
just run-bin-release-nightly udd-cli --help
just run-bin-profile dynamapper
just run-bin-profile-nightly dynamapper
```

Arguments after the binary name go directly to the program; no extra `--` separator is needed.
Conversion workflows use the Python or shell wrappers in `scripts/uddconv/`, which launch
`udd-cli` through `just run-bin-release`:

```sh
python3 scripts/uddconv/uddconv.py all --ccdir "/path/to/Classic" --ecdir "/path/to/Enhanced" --output-dir /path/to/uddp
python3 scripts/uddconv/uddconv.py animations --ccdir "/path/to/Classic" --output-dir /path/to/uddp
```

See the [engineering guide](../ENGINEERING.md) for workspace layout and validation.
