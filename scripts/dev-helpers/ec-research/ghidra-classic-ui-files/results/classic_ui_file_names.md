# Classic UI file names

Program: `classic_7.0.103.0_client.exe` (tested file classic_7.0.103.0_client.exe (Classic Client 7.0.103.0)), SHA-256 `4d857f2f5f8bf350bc50a87ee66b2236bfa480e712e9546c740ff04f9106690a`.

Mode: `--apply` (changes: none).

| Address | Name | Checks | Result |
|---|---|---|---|
| `0x005fb3c0` | `UOClassicUiFiles::Entitlement_LoadCache` | string:entitled.def, calls:005641a0 | already applied |
| `0x0046f970` | `UOClassicUiFiles::Entitlement_OnFeaturesPacket` (probable) | string:entitled.def, calls:005641a0 | already applied |
| `0x005641a0` | `UOClassicUiFiles::ClientFeatures_Set` (probable) | calledby:005fb3c0, calledby:0046f970 | already applied |
| `0x004f81d0` | `UOClassicUiFiles::Desktop_LoadForCharacter` | string:desktop.nwb, string:desktop.uos, calls:004f6bf0 | already applied |
| `0x004f6bf0` | `UOClassicUiFiles::Desktop_Load` | calledby:004f81d0 | already applied |
| `0x004e33b0` | `UOClassicUiFiles::Desktop_Save` (probable) | string:desktop.uo1 | already applied |
| `0x00560350` | `UOClassicUiFiles::Credits_Load` | string:credits.dat, calls:00588950, calls:00578e50 | already applied |
| `0x00578e50` | `UOClassicUiFiles::Credits_ParseColour` | calledby:00560350 | already applied |
| `0x0055e120` | `UOClassicUiFiles::HelpIndex_Load` | string:helpindx.dat | already applied |
| `0x004200ba` | `UOClassicUiFiles::AssetMap_Load` | string:Data/assetmap.xml, calls:00419a90 | already applied |
| `0x0041f9ac` | `UOClassicUiFiles::AssetMap_TrySimpleMap` | string:simplemap, calls:0041f829 | already applied |
| `0x0041f829` | `UOClassicUiFiles::AssetMap_ParseSimpleMap` | calledby:0041f9ac, calls:00417454, calls:00417509 | already applied |
| `0x0041fabc` | `UOClassicUiFiles::AssetMap_ParsePatternMap` | calls:00417454, calls:00417539, string:patternMatch | already applied |
| `0x0041fdd0` | `UOClassicUiFiles::AssetMap_ParseBitmaskMap` | calls:00417498, string:section | already applied |
| `0x00417454` | `UOClassicUiFiles::XmlAttribute_GetU32` | calledby:0041f829 | already applied |
| `0x00417539` | `UOClassicUiFiles::XmlAttribute_GetI32` | calledby:0041fabc | already applied |
| `0x004175f6` | `UOClassicUiFiles::XmlAttribute_GetString` | calledby:0041f829 | already applied |
| `0x00417498` | `UOClassicUiFiles::XmlAttribute_ParseHexU32` | calledby:0041fdd0 | already applied |
| `0x00417509` | `UOClassicUiFiles::XmlAttribute_ParseU64` | calledby:0041f829 | already applied |
| `0x00429f35` | `UOClassicUiFiles::VirtualFile_Open` | calledby:0042a2e3 | already applied |
| `0x005a1480` | `UOClassicUiFiles::ChatFilterLists_Load` | string:uobscene.lst, string:ignore.lst, calls:005a13b0 | already applied |
| `0x005a13b0` | `UOClassicUiFiles::Uobscene_ReadWord` | calledby:005a1480 | already applied |
| `0x0059ea00` | `UOClassicUiFiles::ChatFilterLists_Save` | string:uobscene.lst, string:conf_ignore.lst | already applied |
