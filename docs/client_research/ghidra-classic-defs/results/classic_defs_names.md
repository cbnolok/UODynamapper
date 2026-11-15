# Classic def and Interface XML names

Program: `classic_7.0.103.0_client.exe` (tested file classic_7.0.103.0_client.exe (Classic Client 7.0.103.0)), SHA-256 `4d857f2f5f8bf350bc50a87ee66b2236bfa480e712e9546c740ff04f9106690a`.

Mode: `--apply` (changes: {tag created=1, functions renamed=6, tags added=6, plate comments set=6}).

| Address | Name | Checks | Result |
|---|---|---|---|
| `0x005a0060` | `UOClassicDefs::BodyConv_Load` | string:bodyconv.def, string:%d %d %d %d %d | named, tagged, comment set |
| `0x00563cc0` | `UOClassicDefs::EquipConv_Load` | string:Equipconv.def, string:%d %d %d %d %lx | named, tagged, comment set |
| `0x004473d0` | `UOClassicDefs::ArtDefTable_Load` | calls:00447220, calls:00446fa0, const:40000 | named, tagged, comment set |
| `0x004485d0` | `UOClassicDefs::ArtDefTable_Insert` | calls:005d7c00 | named, tagged, comment set |
| `0x004485a0` | `UOClassicDefs::DefTable_Insert` (probable) | calls:00448490 | named, tagged, comment set |
| `0x00445e10` | `UOClassicDefs::BodyDefTable_Contains` | const:800 | named, tagged, comment set |
