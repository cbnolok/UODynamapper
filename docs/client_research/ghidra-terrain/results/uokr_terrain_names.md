# Terrain names

Program: `UOKR.exe` (tested build UOKR.exe), SHA-256 `1b802e5c00f3bf89a771778b9c6c57406be0703808eeb4cd983e2f5baa348a43`.

Mode: `--apply` (changes: {functions renamed=2, tags added=2, plate comments set=2}).

| Address | Name | Checks | Result |
|---|---|---|---|
| `0x007bf039` | `UOTerrain::TerrainDefinitionFactory_Create` | slot:UOTerrainDefinitionBinaryFactory:1, calls:007bf15a | named, tagged, comment set |
| `0x007bf15a` | `UOTerrain::TerrainDefinition_Read` | calledby:007bf039 | named, tagged, comment set |
