# Classic facet names

Program: `classic_7.0.20.0_client.exe` (tested build classic 7.0.20.0 client.exe), SHA-256 `27eef2c0a4679f204b923f29dd1506bb2c7e649410054aefad680264bb504c9e`.

Mode: `--apply` (changes: {labels created=4, plate comments set=19, functions renamed=15, tags added=15}).

| Address | Name | Checks | Result |
|---|---|---|---|
| `0x00685088` | `UOClassicFacet::g_FacetTable` | referenced by 00591f50, first record name "Felucca" | named, comment set |
| `0x006851d8` | `UOClassicFacet::g_FacetTableVariants` | referenced by 00591f50, first record name "Felucca" | named, comment set |
| `0x00a1bcde` | `UOClassicFacet::g_UseFacetVariants` | referenced by 00559c20 | named, comment set |
| `0x00a1bcd1` | `UOClassicFacet::g_SiegeRules` | referenced by 00559c20 | named, comment set |
| `0x00591f50` | `UOClassicFacet::FacetIndexFromName` | calls:00685088, calls:006851d8, calls:00a1bcde | named, tagged, comment set |
| `0x00591fc0` | `UOClassicFacet::GetFacetWidth` | calls:0068508c, calls:006851dc | named, tagged, comment set |
| `0x00592000` | `UOClassicFacet::GetFacetHeight` | calls:00685090, calls:006851e0 | named, tagged, comment set |
| `0x00592040` | `UOClassicFacet::GetFacetWrapWidth` | calls:00685094, calls:006851e4 | named, tagged, comment set |
| `0x00592080` | `UOClassicFacet::GetFacetWrapHeight` | calls:00685098, calls:006851e8 | named, tagged, comment set |
| `0x005920c0` | `UOClassicFacet::GetFacetStaticsFileId` | calls:0068509c, calls:006851ec | named, tagged, comment set |
| `0x00592100` | `UOClassicFacet::GetFacetStaidxFileId` | calls:006850a0, calls:006851f0 | named, tagged, comment set |
| `0x00592140` | `UOClassicFacet::GetFacetMapFileId` | calls:006850b0, calls:00685200 | named, tagged, comment set |
| `0x00593ed0` | `UOClassicFacet::CurrentFacetHasFeluccaRules` | calls:006850b4, calls:00685204, calls:00a1bcd1 | named, tagged, comment set |
| `0x00593f20` | `UOClassicFacet::GetFacetName` | const:685088, const:6851d8 | named, tagged, comment set |
| `0x00559c20` | `UOClassicFacet::ApplyCharListFlags` | calls:00a1bcde, calls:00a1bcd1, calledby:0047dfd0 | named, tagged, comment set |
| `0x0047dfd0` | `UOClassicFacet::Handle_0xA9_CharList` | calls:00559c20 | named, tagged, comment set |
| `0x00585de0` | `UOClassicFacet::SetFacet` | calls:00591fc0, calls:00592000, calls:00592040, calls:00592080, calledby:00479440 | named, tagged, comment set |
| `0x00479440` | `UOClassicFacet::Handle_0xBF_GeneralInfo` | calls:00585de0 | named, tagged, comment set |
| `0x00561850` | `UOClassicFacet::MapdifBlockLookup` | refs:0 | named, tagged, comment set |
