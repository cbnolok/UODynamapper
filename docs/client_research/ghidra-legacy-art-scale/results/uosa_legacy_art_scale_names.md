# Legacy art scale names

Program: `UOSA.exe`, SHA-256 `5a5e9dc536c836d1bf10b6f86bb88f61a6ae078f519d4060d3bc47aecbebad50`.

Mode: `--apply` (changes: {tag created=1, functions renamed=5, tags added=5, plate comments set=5}).

| Address | Name | Checks | Result |
|---|---|---|---|
| `0x0045a080` | `UOTileArt::BuildSpriteGeometry` | calledby:004592e0, calls:00459b00, calls:00d0535c, calls:00d05358, calls:00d05354 | named, tagged, comment set |
| `0x00459b00` | `UOTileArt::BuildImageWindowQuad` | calledby:0045a080, calls:004440b0, calls:00d051d8 | named, tagged, comment set |
| `0x004598d0` | `SetSpriteMapPosition` | calls:00d050a0, calls:00d05154, calls:00c803d0 | named, tagged, comment set |
| `0x004440b0` | `floorf` | calls:00bd6d70, calledby:00459b00 | named, tagged, comment set |
| `0x00471390` | `UOWorldViewer::UpdateViewportFrustum` | calledby:00471580, calls:004712f0, calls:00db9754 | named, tagged, comment set |
