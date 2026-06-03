# uop-dict-populator-gui

`uop-dict-populator-gui` is a graphical tool for building and expanding UOP hash
dictionaries (`.dic` files). A `.dic` file maps 64-bit UOP path hashes back to
their original virtual file path strings, which is required for meaningful
package inspection and extraction.

## Features

- Load an existing `.dic` dictionary or start from scratch.
- Point the tool at a directory of `.uop` packages to search.
- Define candidate path templates in TOML: the tool expands them over a
  configured numeric range and checks each candidate hash against the packages.
- Run the search in the background with live progress and logging.
- Save the expanded dictionary back to a `.dic` file compatible with MPE
  (Mythic Package Editor) and the `uocf-inspector-gui`.

## TOML template format

```toml
[Texture.uop]
candidates = ["build/worldart/{:08}.dds"]
range = [0, 10000]

[LegacyTexture.uop]
candidates = ["build/tileartlegacy/{:08}.dds"]
range = [0, 10000]
```

Each section key is the target `.uop` filename. `candidates` is a list of
printf-style format strings where `{:08}` is substituted with the numeric index.
`range` sets the inclusive `[start, end]` bounds.

## Binary

```
uop-dict-populator-gui
```

## Notes

- For CLI-based dictionary population, use `uop-dict-populator-cli` from
  [`uocf-cli`](../uocf-cli/README.md).
- The shared `tools/_shared_assets/Dictionary.dic` is a pre-built community
  dictionary. Load it as a starting point and expand from there.
