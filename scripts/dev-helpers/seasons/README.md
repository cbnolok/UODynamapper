# Season profiles

`generate_season_profiles.py` writes the `spring`, `summer`, `fall`, `winter` and `desolation` profiles of
Dynamapper's bundled route file, `dynamapper/assets/runtime_specs/land/LandRoutes.kdl`, from the Classic client's
season tables. Its docstring has the command. The tables come from ClassicUO's `src/Game/Managers/SeasonManager.cs`
at commit `5889a2d6fb35ba5e7f06b7b1d6d70ad1ded30b01` (2021), the last version that held them in code; later versions
read a `seasons.txt`. OrionUO (`OrionUO.cpp` at `0377275cab9147725a6c74553191d6dd7df27935`) holds the same tables.

| Season | Land ids | Static ids |
|---|---|---|
| spring | 0 | 17 |
| summer | 0 | 0 |
| fall | 0 | 27 |
| winter | 31 | 0 |
| desolation | 0 | 72 |

## The Classic client

Checked read-only in Ghidra on `classic_7.0.103.0_client.exe` (project `UOSA`, `/classic`; see
[ghidra-classic-facets](../ec-research/ghidra-classic-facets/README.md) for the import). Every table entry above
matches the client's.

- `DAT_00708be0` holds the season: 0 spring, 1 summer, 2 fall, 3 winter, 4 desolation (set by `FUN_0045c620`).
- `FUN_0045bfa0` fills eight identity tables of 0xFFDC entries; it changes only the winter land table at
  `0x008891d8` (the 31 land ids). The other tables stay identity.
- `FUN_0045c0d0` looks a land id up in winter only; the land getter `FUN_00594670` is its only caller.
- `FUN_0045c5a0` switches a static's graphic: spring `FUN_0045c130`, fall `FUN_0045c250`, desolation `FUN_0045c3d0`.
  Desolation applies only while the player's x (`DAT_00afb6d4 + 0x24`, a signed 16-bit compare:
  `CMP word ptr [EAX + 0x24],0x1800; JGE`) is below 0x1800, strictly, and only with a player object. No map is
  checked: the rule holds on every map, and only Felucca and Trammel are wider than 0x1800. Elsewhere the client
  reports summer (`FUN_0045c5e0`, read by the radar, the object list and the season packet handler). Summer and
  winter change no static. The script writes the rule as the desolation profile's `when player-x-below=0x1800`, which
  Dynamapper follows from the player's position.
- When it is evaluated: per statics block. `FUN_00595240` keeps the 8x8 blocks of a 7x7-block window around the
  player and builds only the blocks entering it (at each step, through `RebuildTilesAroundPlayer`), switching their
  statics with the player's x of that moment; blocks already built keep their graphics until they leave the window.
  The world is cleared (`FUN_00470370`) and built again only on a map change (`SetFacet`) and when the season
  packet (`FUN_004704c0`) changes the reported season; a teleport (`FUN_00476290`) builds the blocks missing around
  the new position, the others leaving as they fall out of the window. The client has no hysteresis or other guard at
  the line. Dynamapper switches the whole
  profile at once when the player crosses the line.
- The static switch has one caller, `FUN_005b00f0`, called only by `FUN_00595240`, which builds the statics of the
  blocks around the player from `staidx`/`statics`. `FUN_005b00f0` stores the switched graphic and reads the tile
  data of that graphic, so a switched static takes the new item's tile data too. Multis are built elsewhere and are
  not switched (ClassicUO switches them; the client does not).
- `FUN_0045d3a0` builds winter snow edges (land 0x5BF-0x5E2 by neighbour masks); nothing references it.

Land drawing (`FUN_0044fa70` from the land art, `FUN_0044c180` from the texmap) skips land id 2 (`NODRAW`) unless
the debug flag `DAT_0075a8ac` is set. Ids 0 and 1 are drawn. ClassicUO skips ids up to 2.

## The Enhanced and Kingdom Reborn clients

`UOSA.exe` and `UOKR.exe` hold no such tables: none of the desolation, fall and spring target ids (0x1BAE, 0x1B9C,
0x1B22, 0x1B1F, 0x0D3F) occurs as a constant, the winter targets occur in `UOSA.exe` only in unrelated code (inflate,
CRT locale data), and neither binary nor the Enhanced client's UserInterface scripts name a season. Their seasons
change no land or static graphic that these probes could find.
