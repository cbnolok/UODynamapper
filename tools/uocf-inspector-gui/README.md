# uocf-inspector-gui

`uocf-inspector-gui` is a graphical inspector for Ultima Online source formats.
It reads directly from Classic Client and Enhanced Client installation files
without requiring prior conversion, and is the primary tool for exploring raw
UO asset data during development and research.

## Views

| View | Content |
| ---- | ------- |
| UOP Explorer | Browse any `.uop` package with optional `.dic` dictionary resolution. |
| CC Art | Classic Client art tiles (`art.mul` / `artidx.mul`). |
| CC Tiledata | Classic Client `tiledata.mul` land and item tile metadata. |
| Tile Metadata | Combined CC tiledata and EC tileart metadata (switchable source). |
| Animations | Mobile animations from CC `anim*.mul` or EC `AnimationFrame*.uop`. |
| Anim Data | Classic Client `animdata.mul` animation sequence tables. |
| Gumps | CC or EC gump art (switchable source). |
| Multis | Multi-tile structures from CC or EC sources. |
| Hues | CC `hues.mul` or EC `hues.uop` hue tables (switchable source). |
| Clilocs | Classic Client `Cliloc.*` and Enhanced Client `LocalizedStrings.uop` localized string tables. |
| Terrain Definition | EC `TerrainDefinition.uop` material entries and texture references. |
| String Dictionary | EC `string_dictionary.uop` string table. |
| Sounds | Classic Client `sound.mul` / `soundidx.mul` audio entries. |

Most views support switching between Classic Client and Enhanced Client sources
where both exist.

## Binary

```
uocf-inspector-gui
```

## Notes

- Configure client paths in the application settings panel.
- Upscale preview is available for art tiles using the image-postprocess
  upscaling algorithms, including color-only vibrance, saturation, selective
  hue boosts, and local-contrast clarity passes.
- Audio playback is supported in the Sounds view.
- See [lib/uocf](../../lib/uocf/README.md) for the underlying parser library.
