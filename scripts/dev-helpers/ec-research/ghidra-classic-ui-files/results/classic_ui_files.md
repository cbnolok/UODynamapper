# Classic UI files: what the 2024 client reads and writes

Program: `classic_7.0.103.0_client.exe` (SHA-256 `4d857f2f…690a`). The names are those of
`NameClassicUiFileCode.java`, in the namespace `UOClassicUiFiles`.

## entitled.def: a cache the client writes

- **Confirmed.** `Entitlement_LoadCache` (0x005fb3c0), called at start-up, opens the file `rb`. It does one
  `fread` of 4 bytes (0 when the file is missing) and passes the value to `ClientFeatures_Set` (0x005641a0).
- **Confirmed.** `Entitlement_OnFeaturesPacket` (0x0046f970), a packet handler, reads a big-endian `u32`, passes
  it to `ClientFeatures_Set`, and writes it to the file `wb` as 4 little-endian bytes.
- **Probable.** The packet is 0xB9, the feature flags.
- The shipped files hold 2 bytes (ML) or 4 bytes (7.0.20, 2024, fp).

## desktop.nwb: the saved-desktop format, read as the default

- **Confirmed.** `Desktop_LoadForCharacter` (0x004f81d0) loads
  `Desktop/<account>/<shard>/<character>/desktop.uos`, or `Desktop/common/desktop.uos`. When the names are empty,
  or no saved desktop exists, it loads `desktop.nwb`.
- **Confirmed.** `Desktop_Load` (0x004f6bf0) reads the file:
  - a `u32` serial, replaced by the player's;
  - then records to the end of the file, each a `u8` kind and a fixed payload.
- **Payload sizes** (bytes after the kind byte):

  | Kinds | Bytes |
  | --- | --- |
  | 0, 1, 6, 7, 8, 15, 20, 21, 25, 26, 29 | 9 |
  | 2, 3, 4, 30 | 13 |
  | 5 | 14 |
  | 9, 23, 27 | 8 |
  | 10 | 32 |
  | 11, 12, 13, 14, 19 | 24 |
  | 16 | 25 |
  | 17 | 17 |
  | 22, 28 | 10 |
  | 24 | 12 |

  Kind 18, or any kind from 31 up, stops the read.
- **Probable.** `Desktop_Save` (0x004e33b0) writes `desktop.uos` and `desktop.uo1`-`uo6`. No code writes
  `desktop.nwb`.
- The shipped file is identical in 7.0.20, 2024 and fp. It parses whole: serial 0x01e75001, then 12 records.

## credits.dat

- **Confirmed.** `Credits_Load` (0x00560350) opens the file `rt` and reads lines with `KeyNames_ReadLine`.
- **How a line is read:**
  - The first token is found with `strtok` and the delimiters `" \t\n\r\f="`.
  - When that token is `!`, the rest of the line (`strtok` with `" \t\n\r\f"`) holds keyword and value pairs.
  - The keywords are `color`, `defaultcolor` (which also sets the current colour), `font` (`atol`),
    `defaultfont` and `song`. Other tokens are skipped.
  - Every other line, empty ones included, is text.
- **Confirmed.** `Credits_ParseColour` (0x00578e50) reads one of two forms:
  - `#RRGGBB`;
  - a decimal, read as 0xRRGGBB. A leading `-` keeps the `atol` value as it is.
- The Gold client holds the same keyword strings.

## helpindx.dat

- **Confirmed.** `HelpIndex_Load` (0x0055e120) opens the file `r` and reads it with `fgets` into a 512-byte
  buffer.
- **How a line is read:**
  - A line starting with `#` is a comment.
  - Any other line is taken only when it is shorter than 511 bytes, its line feed included.
  - An optional leading `@` sets the topic's flag to 0; otherwise the flag is 1.
  - `strtok` splits the line at tabs (`atol`): the topic id, then three cliloc numbers. The row is kept only when
    all four are nonzero.
  - Any further fields are linked topic ids.
- **Probable.** One shipped row ends with a tab, so its line feed is read as one more field, the link 0.
  Another row has `92 `, which `atol` reads as 92.

## Data/assetmap.xml

- **Confirmed: the client reads the UOP copy.**
  - `AssetMap_Load` (0x004200ba) loads `Data/assetmap.xml` through 0x00419a90 and 0x0042a2e3, which set ECX to 0
    before `VirtualFile_Open` (0x00429f35).
  - `VirtualFile_Open` turns mode 0 into 2. The modes are 1 (disk only), 2 (the loaded UOP packages only, by
    name hash) and anything else (disk, then UOP).
  - So the 2024 client reads the `data/assetmap.xml` entry of `MainMisc.uop`, not the loose file.
- **Confirmed.** The document must have the root `<assetmap>`, or the client logs "Assetmap root node isn't
  <assetmap>". Each child element goes through the handlers, which match names case-sensitively. A child no
  handler takes is logged as "Node X ignored - no valid mapping found."
- **Confirmed.** `AssetMap_TrySimpleMap` / `AssetMap_ParseSimpleMap` read:
  - `resourcetype`, keeping only the low byte;
  - `resourcenumber` (`XmlAttribute_GetU32`), or, when it is absent, `id` (`XmlAttribute_ParseU64`);
  - `filename`;
  - `fileFormat`: `raw` gives 0, `packed` gives 1, anything else gives 0.
- **Confirmed.** `AssetMap_ParsePatternMap` reads `resourcetype`, `fileFormat`, `directory`, `prefix`, `suffix`,
  `padding` (`u32`) and `patternMatch` (`XmlAttribute_GetI32`, true when nonzero).
- **Confirmed.** `AssetMap_ParseBitmaskMap` reads `resourcetype`, `fileFormat`, `directory` and `suffix`. For
  each child named `section`, it reads:
  - `prefix`;
  - `padding` (`u32`);
  - `mask` (`XmlAttribute_ParseHexU32`, base 16);
  - `patternMatch` and `isDirectory` (`u32`, true when nonzero).
- **Confirmed.** `XmlAttribute_GetU32` uses `strtoul` base 10. When the value does not parse whole, the attribute
  counts as absent.

## The chat filter lists

- **Confirmed.** `Uobscene_ReadWord` (0x005a13b0) reads UTF-16 units, minus 13 each, up to a decoded CR or LF.
  - A CR LF or LF CR pair is one line end; CR CR is two line ends.
  - There is no length limit: the word goes into a global buffer.
  - When the last word has no line end, the reader adds one extra unit to it: the previous decoded unit, minus 13
    again.
- **Confirmed.** `ChatFilterLists_Load` (0x005a1480) skips empty words. It copies each word into a 30-unit slot
  of a table.
  - **Probable:** the table has 120 slots before the `conf_ignore.lst` serials (0xab7160 to 0xab8d80); there is
    no count check.
  - It reads `ignore.lst` and `conf_ignore.lst` as records of a 30-byte name and a `u32` serial.
- **Confirmed.** `ChatFilterLists_Save` (0x0059ea00) writes `uobscene.lst` as each word's units plus 13, then
  0x17 (LF plus 13). This is the shipped form. It writes the two ignore lists as the same records.
