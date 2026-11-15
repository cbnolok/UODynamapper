# Classic data file names

Program: `classic_7.0.103.0_client.exe` (tested file classic_7.0.103.0_client.exe (Classic Client 7.0.103.0)), SHA-256 `4d857f2f5f8bf350bc50a87ee66b2236bfa480e712e9546c740ff04f9106690a`.

Mode: `--apply` (changes: none).

| Address | Name | Checks | Result |
|---|---|---|---|
| `0x00576920` | `UOClassicDataFiles::HouseTables_LoadAll` | string:walls.txt, string:roof.txt, calls:00575bf0 | already applied |
| `0x00575bf0` | `UOClassicDataFiles::HouseTable_Load` | calls:0056f520, calls:00570480, calls:0056f870 | already applied |
| `0x0056f520` | `UOClassicDataFiles::WallsRow_Construct` | calledby:00575bf0, const:6b528c | already applied |
| `0x0056f580` | `UOClassicDataFiles::WallsRow_Parse` | ref:006b528c, string:%d %d %d %d %d %d %d %d %d %d %d %d %d %d %d %d %d %d | already applied |
| `0x00570480` | `UOClassicDataFiles::FloorsRow_Construct` | calledby:00575bf0, const:6b5444 | already applied |
| `0x0056f5f0` | `UOClassicDataFiles::FloorsRow_Parse` | ref:006b5444, string:%d %d %d %d %d %d %d %d %d %d %d %d %d %d %d %d %d %d | already applied |
| `0x00570520` | `UOClassicDataFiles::TeleportersRow_Construct` | calledby:00575bf0, const:6b5454 | already applied |
| `0x0056f660` | `UOClassicDataFiles::TeleportersRow_Parse` | ref:006b5454, string:%d %d %d %d %d %d %d %d %d %d %d %d %d %d %d %d %d %d | already applied |
| `0x005704d0` | `UOClassicDataFiles::DoorsRow_Construct` | calledby:00575bf0 | already applied |
| `0x0056f6d0` | `UOClassicDataFiles::DoorsRow_Parse` | string:%d %d %d %d %d %d %d %d %d %d | already applied |
| `0x0056f720` | `UOClassicDataFiles::MiscRow_Construct` | calledby:00575bf0 | already applied |
| `0x0056f770` | `UOClassicDataFiles::MiscRow_Parse` | string:%d %d %d %d %d %d %d %d %d %d %d %d | already applied |
| `0x0056f7c0` | `UOClassicDataFiles::StairsRow_Construct` | calledby:00575bf0 | already applied |
| `0x0056f810` | `UOClassicDataFiles::StairsRow_Parse` | string:%d %d %d %d %d %d %d %d %d %d %d %d %d %d %d | already applied |
| `0x0056f870` | `UOClassicDataFiles::RoofRow_Construct` | calledby:00575bf0 | already applied |
| `0x0056f8c0` | `UOClassicDataFiles::RoofRow_Parse` | string:%d %d %d %d %d %d %d %d %d %d %d %d %d %d %d %d %d %d %d %d | already applied |
| `0x00443770` | `UOClassicDataFiles::SuppInfo_Load` | string:suppinfo.txt, string:%s %d %d | already applied |
| `0x005252b0` | `UOClassicDataFiles::Professions_Load` | string:prof.txt, string:Begin, calls:0051dbe0 | already applied |
| `0x0051dbe0` | `UOClassicDataFiles::SkillId_FromName` | string:Mysticism, string:Throwing | already applied |
| `0x00589820` | `UOClassicDataFiles::KeyNamesAndMacros_Load` | string:keynames.txt, calls:00588950 | already applied |
| `0x00588950` | `UOClassicDataFiles::KeyNames_ReadLine` | calledby:00589820 | already applied |
| `0x0056a100` | `UOClassicDataFiles::Palette_Load` | string:palette.mul, calls:0056a970 | already applied |
| `0x0056a970` | `UOClassicDataFiles::AnimInfo_Load` | string:animinfo.mul | already applied |
