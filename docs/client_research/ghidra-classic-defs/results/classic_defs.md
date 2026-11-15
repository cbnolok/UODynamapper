# Classic def files and the EC Interface XML loader

Read in `classic_7.0.103.0_client.exe` (the 2024 Classic client), `gold_client.exe` (UO Gold, bodyconv only) and
`UOSA.exe`, 2026-11-01. Names: [classic_defs_names.md](classic_defs_names.md),
[uosa_interface_xml_names.md](uosa_interface_xml_names.md).

## The `{targets}` def files

`UOClassicDefs::LoadDefFiles` loads, in order, `sound.def`, `gump.def`, `anim1.def` to `anim4.def`, `art.def`,
`texterr.def`, `body.def` and `corpse.def`; every one but `sound.def` is read by `DefTable_Load` (`0x00447220`)
and `DefTable_ParseLine` (`0x00446d90`). **Confirmed.**

- The file is opened in text mode (`fopen "r"`: CR LF reads as LF) and read with `fgets(buffer, 255)`, so a line
  of more than 254 bytes is read as two lines. The longest shipped line has 118 bytes.
- A line whose first non-whitespace byte is NUL or `#` is skipped. Any other line is
  `ID {T1, T2, ...} VALUE [# comment]`:
  - `ID` is read with `%hd`: a 16-bit id (`60759` is kept as the `u16` 60759).
  - The client then skips to the first `{` whatever lies between.
  - In the braces, non-digits before a target are skipped, so a `-` is too: `{-1}` (`anim2.def`) reads as
    target 1. Each target is `%hd`; after it only spaces and a `,` may come before the next one, or the `}`; any
    other byte drops the line. `{}` has no target.
  - After the `}` the client skips to the first digit or `-` (whatever lies between) and reads `VALUE` with `%d`
    (the hue of `body.def`, `corpse.def`, `art.def`, `texterr.def`, `gump.def`; 0 in `anim1/2.def`). A line
    without one makes the client read past the line; every shipped line has one.
  - After `VALUE` only whitespace, or `#` and a comment, may follow; anything else drops the line.
- The first line of an id wins (`Contains` before `Insert`).
- A remap picks one target with `rand() % count` (`DefTable_RemapRandom`).
- `art.def` (`ArtDefTable_Load`, `ArtDefTable_Insert`): an entry is kept only for an art id without an art
  record (`GetArtRecord` returns 0), so it supplies missing art.
- `body.def` (`BodyDefTable_Contains`): a body below 0x800 that `bodyconv.def` maps for an enabled expansion is
  not remapped by `body.def`.

## `bodyconv.def` and `Equipconv.def`

Own loaders, `BodyConv_Load` (`0x005a0060`) and `EquipConv_Load` (`0x00563cc0`). **Confirmed.**

- `fgets(buffer, 255)`, then the line is cut at its first `#`.
- `bodyconv.def`: `sscanf("%d %d %d %d %d")` (body, anim2, anim3, anim4, anim5); a line with fewer than 5 numbers
  is dropped, a sixth number is not read. A later line overwrites. The table has 0x800 rows indexed by the body
  without a check (the file says "The maximum value for an index is 2048"); shipped bodies go up to 1700.
- UO Gold reads `sscanf("%d %d %d %d")` (body, anim2, anim3, anim4) of every line not starting with NUL or `#`,
  without a count check. Its file has 4 numbers per line; the 2024 client drops such lines.
- `Equipconv.def`: `sscanf("%d %d %d %d %lx")` (body, equipment id, converted id, gump id, hue): **the hue is read
  as hexadecimal**. Fewer than 5 fields drop the line; a later line for the same body and equipment overwrites.
- The quoted comment lines of the shipped files (`"# low level..."`, from a spreadsheet export) read no number
  and are dropped.

## EC Interface documents

`UOXml::LoadInterfaceDocument` (`0x0090139f`) reads a document, `TiXmlDocument::Parse`, and when the document's
error flag is set logs `%s: unable to load '%s' due to error at line %d, column %d` and does not read it;
otherwise `UOXml::ParseInterfaceRoot` (`0x0096aebb`) reads its `Include`, `Scripts`, `Assets` and `Windows` and the
included documents are loaded in turn. KR has the same message. **Confirmed.**

TinyXML 2.5.3 (named by `NameSourceFunctions`) with no error flag for: attributes without whitespace between
them (`TiXmlElement::Parse`), an `&` starting no known entity (`GetEntity` returns it as a character), a `<` in a
quoted attribute value (`TiXmlAttribute::Parse`), a `<!--` inside a comment (`TiXmlComment::Parse` stops at the
first `-->`), an XML declaration after the start (`Identify`). An element still open at the end sets error 9
("Error reading end tag."), so the document is dropped. A duplicate attribute returns NULL without the error flag:
the document is silently cut there. **Confirmed.**

`startmenu.xml` (EC and KR) lacks its last `</Interface>`, so the client drops it. Nothing names it: none of the
6,515 EC `Interface.uop` entries (`Include`, `.mod`, Lua) mentions `StartMenu.xml`. **Confirmed** for the EC;
the KR loader has the same message, its references were not searched.
