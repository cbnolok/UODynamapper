# Classic facet names

Program: `classic_7.0.103.0_client.exe` (tested build classic 7.0.103.0 client.exe), SHA-256 `4d857f2f5f8bf350bc50a87ee66b2236bfa480e712e9546c740ff04f9106690a`.

Mode: `--apply` (changes: {labels created=7, plate comments set=35, functions renamed=28, tags added=28}).

| Address | Name | Checks | Result |
|---|---|---|---|
| `0x006b7478` | `UOClassicFacet::g_FacetTable` | referenced by 005a0310, first record name "Felucca" | named, comment set |
| `0x006b75c8` | `UOClassicFacet::g_FacetTableVariants` | referenced by 005a0310, first record name "Felucca" | named, comment set |
| `0x00a58d7e` | `UOClassicFacet::g_UseFacetVariants` | referenced by 005640b0 | named, comment set |
| `0x00a58d71` | `UOClassicFacet::g_SiegeRules` | referenced by 005640b0 | named, comment set |
| `0x0074229c` | `UOClassicFacet::g_WrapWidth` | referenced by 00593eb0 | named, comment set |
| `0x007422a0` | `UOClassicFacet::g_WrapHeight` | referenced by 00593eb0 | named, comment set |
| `0x00aba9b5` | `UOClassicFacet::g_CurrentFacet` | referenced by 005a23c0 | named, comment set |
| `0x005a0310` | `UOClassicFacet::FacetIndexFromName` | calls:006b7478, calls:006b75c8, calls:00a58d7e | named, tagged, comment set |
| `0x005a0380` | `UOClassicFacet::GetFacetWidth` | calls:006b747c, calls:006b75cc | named, tagged, comment set |
| `0x005a03c0` | `UOClassicFacet::GetFacetHeight` | calls:006b7480, calls:006b75d0 | named, tagged, comment set |
| `0x005a0400` | `UOClassicFacet::GetFacetWrapWidth` | calls:006b7484, calls:006b75d4 | named, tagged, comment set |
| `0x005a0440` | `UOClassicFacet::GetFacetWrapHeight` | calls:006b7488, calls:006b75d8 | named, tagged, comment set |
| `0x005a0480` | `UOClassicFacet::GetFacetStaticsFileId` | calls:006b748c, calls:006b75dc | named, tagged, comment set |
| `0x005a04c0` | `UOClassicFacet::GetFacetStaidxFileId` | calls:006b7490, calls:006b75e0 | named, tagged, comment set |
| `0x005a0500` | `UOClassicFacet::GetFacetMapFileId` | calls:006b74a0, calls:006b75f0 | named, tagged, comment set |
| `0x005a23c0` | `UOClassicFacet::CurrentFacetHasFeluccaRules` | calls:006b74a4, calls:00a58d71, calls:00aba9b5 | named, tagged, comment set |
| `0x005a2410` | `UOClassicFacet::GetFacetName` | const:6b7478, const:6b75c8 | named, tagged, comment set |
| `0x0050acf0` | `UOClassicFacet::ConfirmCriminalAction` | string:This may flag you criminal!, calledby:00488ad0 | named, tagged, comment set |
| `0x005640b0` | `UOClassicFacet::ApplyCharListFlags` | calls:00a58d7e, calls:00a58d71, calledby:0047f460 | named, tagged, comment set |
| `0x0047f460` | `UOClassicFacet::Handle_0xA9_CharList` | calls:005640b0, string:You have no characters on %s! | named, tagged, comment set |
| `0x00477f80` | `UOClassicFacet::Handle_0xBF_GeneralInfo` | calls:00593eb0, calls:0056e8a0 | named, tagged, comment set |
| `0x00593eb0` | `UOClassicFacet::SetFacet` | calls:0074229c, calls:007422a0, calledby:00477f80 | named, tagged, comment set |
| `0x00593ba0` | `UOClassicFacet::WrappedTileDistance` | calls:0074229c, const:270f | named, tagged, comment set |
| `0x00593c20` | `UOClassicFacet::WrappedTileDistanceXY` | calls:0074229c, const:270f | named, tagged, comment set |
| `0x00596d50` | `UOClassicFacet::MovePlayerWrapped` | calls:0074229c, calls:007422a0, calls:00596840 | named, tagged, comment set |
| `0x00596840` | `UOClassicFacet::RebuildTilesAroundPlayer` | calls:00596710, calls:005965f0, calls:0052b180 | named, tagged, comment set |
| `0x00596710` | `UOClassicFacet::BuildTileWrapped` | calledby:00596840, calls:0074229c | named, tagged, comment set |
| `0x005965f0` | `UOClassicFacet::BuildTileUnwrapped` | calledby:00596840, const:244 | named, tagged, comment set |
| `0x0052b180` | `UOClassicFacet::Radar_RebuildAroundPlayer` | calls:0074229c, const:8001, calledby:00596840 | named, tagged, comment set |
| `0x0056e8a0` | `UOClassicFacet::ReloadDifLists` | calls:0056e5c0, calls:0056e700, calledby:00477f80 | named, tagged, comment set |
| `0x0056e5c0` | `UOClassicFacet::LoadStadifLists` | string:stadifl%d.mul, calledby:0056e8a0 | named, tagged, comment set |
| `0x0056e700` | `UOClassicFacet::LoadMapdifLists` | string:mapdifl%d.mul, calledby:0056e8a0 | named, tagged, comment set |
| `0x0056cf50` | `UOClassicFacet::StadifBlockLookup` | refs:0 | named, tagged, comment set |
| `0x0056cfe0` | `UOClassicFacet::MapdifBlockLookup` | refs:0 | named, tagged, comment set |
| `0x0059ffb0` | `UOClassicFacet::ProbeLegacyMapUops` | calledby:005a7360 | named, tagged, comment set |
