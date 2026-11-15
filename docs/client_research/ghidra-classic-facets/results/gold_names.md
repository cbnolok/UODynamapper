# Classic facet names

Program: `gold_client.exe` (tested build UO Gold client.exe), SHA-256 `aa21bb3ea16b6c5e00837569466b052e978baa6e38fd945e2694af4f14fa28c2`.

Mode: `--apply` (changes: {labels created=6, plate comments set=31, functions renamed=25, tags added=25}).

| Address | Name | Checks | Result |
|---|---|---|---|
| `0x0056f918` | `UOClassicFacet::g_FacetTable` | referenced by 004f4780, first record name "Felucca" | named, comment set |
| `0x00757285` | `UOClassicFacet::g_SiegeRules` | referenced by 004ca750 | named, comment set |
| `0x005cd66c` | `UOClassicFacet::g_WrapWidth` | referenced by 004eab40 | named, comment set |
| `0x005cd670` | `UOClassicFacet::g_WrapHeight` | referenced by 004eab40 | named, comment set |
| `0x00768e60` | `UOClassicFacet::g_MapdifBlockMaps` | referenced by 004d1230 | named, comment set |
| `0x00768eb8` | `UOClassicFacet::g_StadifBlockMaps` | referenced by 004d1140 | named, comment set |
| `0x004f4780` | `UOClassicFacet::GetFacetName` | calls:0056f918 | named, tagged, comment set |
| `0x004f4800` | `UOClassicFacet::GetFacetWidth` | calls:0056f91c | named, tagged, comment set |
| `0x004f4830` | `UOClassicFacet::GetFacetHeight` | calls:0056f920 | named, tagged, comment set |
| `0x004f4860` | `UOClassicFacet::GetFacetWrapWidth` | calls:0056f924 | named, tagged, comment set |
| `0x004f4890` | `UOClassicFacet::GetFacetWrapHeight` | calls:0056f928 | named, tagged, comment set |
| `0x004f48c0` | `UOClassicFacet::GetFacetStaticsFileId` | calls:0056f92c | named, tagged, comment set |
| `0x004f48f0` | `UOClassicFacet::GetFacetStaidxFileId` | calls:0056f930 | named, tagged, comment set |
| `0x004f4920` | `UOClassicFacet::GetFacetMapdifFileId` | calls:0056f934 | named, tagged, comment set |
| `0x004f4950` | `UOClassicFacet::GetFacetStadifFileId` | calls:0056f938 | named, tagged, comment set |
| `0x004f4980` | `UOClassicFacet::GetFacetStadifiFileId` | calls:0056f93c | named, tagged, comment set |
| `0x004f49b0` | `UOClassicFacet::GetFacetMapFileId` | calls:0056f940 | named, tagged, comment set |
| `0x004f4610` | `UOClassicFacet::CurrentFacetHasFeluccaRules` | calls:0056f944, calls:00757285 | named, tagged, comment set |
| `0x004f47e0` | `UOClassicFacet::IsValidFacet` | calledby:004d1230 | named, tagged, comment set |
| `0x004ca750` | `UOClassicFacet::ApplyCharListFlags` | calls:00757285, calledby:00427d20 | named, tagged, comment set |
| `0x00427d20` | `UOClassicFacet::Handle_0xA9_CharList` | calls:004ca750, string:You have no characters on %s! | named, tagged, comment set |
| `0x0041ff90` | `UOClassicFacet::Handle_0xBF_GeneralInfo` | calls:004eab40, calls:004d0cb0 | named, tagged, comment set |
| `0x004eab40` | `UOClassicFacet::SetFacet` | calls:004f4800, calls:004f4860, calls:005cd66c, calledby:0041ff90 | named, tagged, comment set |
| `0x004d0cb0` | `UOClassicFacet::LoadDifLists` | string:mapdifl%d.mul, string:stadifl%d.mul, calledby:0041ff90 | named, tagged, comment set |
| `0x004d1230` | `UOClassicFacet::MapdifBlockLookup` | calls:00768e60, calls:004f47e0 | named, tagged, comment set |
| `0x004d1140` | `UOClassicFacet::StadifBlockLookup` | calls:00768eb8 | named, tagged, comment set |
| `0x004ea9d0` | `UOClassicFacet::GetLandGraphicAt` | calls:004d1230, calls:004f4920, calls:004f49b0 | named, tagged, comment set |
| `0x004ea700` | `UOClassicFacet::GetLandZAt` | calls:004d1230, calls:004f4920, calls:004f49b0 | named, tagged, comment set |
| `0x004e9df0` | `UOClassicFacet::BuildLandTileWrapped` | calls:005cd66c, calls:004d1230, calls:004ea700 | named, tagged, comment set |
| `0x004eaa70` | `UOClassicFacet::LoadLandBlock` | calls:004d1230, calls:004f4920, calls:004f49b0 | named, tagged, comment set |
| `0x004eaac0` | `UOClassicFacet::LoadStaticsBlock` | calls:004d1140, calls:004f4950, calls:004f4980 | named, tagged, comment set |
