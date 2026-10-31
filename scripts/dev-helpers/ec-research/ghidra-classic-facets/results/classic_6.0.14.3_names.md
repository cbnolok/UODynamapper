# Classic facet names

Program: `classic_6.0.14.3_client.exe` (tested build classic 6.0.14.3 client.exe), SHA-256 `9104cea0ea14bd8ee81ee5aa111f1e3f39eca50ac33f818813202fadfdd3718d`.

Mode: `--apply` (changes: {labels created=1, plate comments set=15, functions renamed=14, tags added=14}).

| Address | Name | Checks | Result |
|---|---|---|---|
| `0x005a6db0` | `UOClassicFacet::g_FacetTable` | referenced by 00503400, first record name "Felucca" | named, comment set |
| `0x00503400` | `UOClassicFacet::FacetIndexFromName` | calls:005a6db0 | named, tagged, comment set |
| `0x00503440` | `UOClassicFacet::GetFacetWidth` | calls:005a6db4 | named, tagged, comment set |
| `0x00503470` | `UOClassicFacet::GetFacetHeight` | calls:005a6db8 | named, tagged, comment set |
| `0x005034a0` | `UOClassicFacet::GetFacetWrapWidth` | calls:005a6dbc | named, tagged, comment set |
| `0x005034d0` | `UOClassicFacet::GetFacetWrapHeight` | calls:005a6dc0 | named, tagged, comment set |
| `0x00503500` | `UOClassicFacet::GetFacetStaticsFileId` | calls:005a6dc4 | named, tagged, comment set |
| `0x00503530` | `UOClassicFacet::GetFacetStaidxFileId` | calls:005a6dc8 | named, tagged, comment set |
| `0x00503560` | `UOClassicFacet::GetFacetMapFileId` | calls:005a6dd8 | named, tagged, comment set |
| `0x005047d0` | `UOClassicFacet::CurrentFacetHasFeluccaRules` | calls:005a6ddc | named, tagged, comment set |
| `0x004face0` | `UOClassicFacet::SetFacet` | calls:00503440, calls:00503470, calls:005034a0, calls:005034d0 | named, tagged, comment set |
| `0x004e0eb0` | `UOClassicFacet::LoadStadifLists` | string:stadifl%d.mul, calledby:004e1160 | named, tagged, comment set |
| `0x004e0ff0` | `UOClassicFacet::LoadMapdifLists` | string:mapdifl%d.mul, calledby:004e1160 | named, tagged, comment set |
| `0x004e1160` | `UOClassicFacet::ReloadDifLists` | calls:004e0eb0, calls:004e0ff0, calledby:00430ae0 | named, tagged, comment set |
| `0x00430ae0` | `UOClassicFacet::Handle_0xBF_GeneralInfo` | calls:004e1160 | named, tagged, comment set |
