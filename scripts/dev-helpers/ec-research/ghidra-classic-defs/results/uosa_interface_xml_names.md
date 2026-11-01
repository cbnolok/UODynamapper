# Classic def and Interface XML names

Program: `UOSA.exe` (tested file UOSA.exe (Enhanced Client)), SHA-256 `5a5e9dc536c836d1bf10b6f86bb88f61a6ae078f519d4060d3bc47aecbebad50`.

Mode: `--apply` (changes: {tag created=1, functions renamed=2, tags added=2, plate comments set=2}).

| Address | Name | Checks | Result |
|---|---|---|---|
| `0x0090139f` | `UOXml::LoadInterfaceDocument` | string:%s: unable to load, calls:0096aebb | named, tagged, comment set |
| `0x0096aebb` | `UOXml::ParseInterfaceRoot` | string:Include, calledby:0090139f | named, tagged, comment set |
