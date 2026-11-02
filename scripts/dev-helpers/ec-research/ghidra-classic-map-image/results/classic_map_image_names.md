# Classic map image names

Program: `classic_7.0.103.0_client.exe` (tested file classic_7.0.103.0_client.exe (Classic Client 7.0.103.0)), SHA-256 `4d857f2f5f8bf350bc50a87ee66b2236bfa480e712e9546c740ff04f9106690a`.

Mode: `--apply` (changes: none).

| Address | Name | Checks | Result |
|---|---|---|---|
| `0x004c2d10` | `UOClassicMapImage::CourseGump_Construct` | string:Course gump, string:radarcol.mul | already applied |
| `0x004c1e70` | `UOClassicMapImage::CourseGump_SquareRegion` | calls:005a0400, calls:005a0440 | already applied |
| `0x004c28d0` | `UOClassicMapImage::CourseGump_DrawFacetImage` | calls:0056bf40, calledby:004c3240 | already applied |
| `0x004c3240` | `UOClassicMapImage::CourseGump_Redraw` (probable) | calls:004c28d0 | already applied |
| `0x0056bf40` | `UOClassicMapImage::MulFile_RecordPointer` | calledby:0056e000, calledby:004c28d0 | already applied |

# Classic map image names

Program: `gold_client.exe` (tested file gold_client.exe (UO Gold, 2004-11-11)), SHA-256 `aa21bb3ea16b6c5e00837569466b052e978baa6e38fd945e2694af4f14fa28c2`.

Mode: `--apply` (changes: none).

| Address | Name | Checks | Result |
|---|---|---|---|
| `0x004585c0` | `UOClassicMapImage::CourseGump_Construct` | string:Course gump, calls:00459be0 | already applied |
| `0x00459be0` | `UOClassicMapImage::CourseGump_Redraw` | string:multimap.rle | already applied |
