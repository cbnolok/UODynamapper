# egui / eframe 0.37 upgrade notes (GUI tools)

Applies to the eframe-based tools (`lib/support/uot-gui`, `tools/*-gui`), not to `dynamapper`,
which uses its own egui through `bevy_egui` (a second egui version in the graph is expected).

## Silent feature regression: clipboard and links

The workspace pins `eframe` with `default-features = false`. Up to 0.34, eframe always
enabled `egui-winit/clipboard` and `egui-winit/links` itself. From 0.37 both are opt-in
eframe features (`clipboard`, `links`). Leaving them out still compiles cleanly, but
copy/paste and opening hyperlinks stop working at runtime. `clipboard` is enabled;
`links` is intentionally left off for now (maintainer decision).

`egui` itself also dropped `default_fonts` from its defaults in 0.37; eframe's
`default_fonts` feature re-enables it through feature unification.

## API breaks hit by this codebase (0.34 -> 0.37)

- `Panel::show_inside` / `CentralPanel::show_inside` are deprecated; `show(ui, ..)` now
  takes `&mut Ui` and is the direct replacement.
- `Id::new` is deprecated; `Id::unique` is the same hash (it is what `Id::new` forwards to).
- Id salts (`ui.make_persistent_id`, `Panel` constructors, ...) take `impl AsIdSalt`, which
  requires `Hash + Debug`. Generic helpers that forward an `impl Hash` salt need a
  `+ Debug` bound.

## Changelogs

The crates.io sources for egui/eframe do not ship `CHANGELOG.md`; read them from the
upstream repo (`CHANGELOG.md` and `crates/eframe/CHANGELOG.md`).
