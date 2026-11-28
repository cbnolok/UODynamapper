# Wide z protocol extension (10-bit world z)

2026-11-28, on dev at 0bed0c0f. This is a proposal. Nothing is implemented, on either side.

Custom maps move to a 10-bit world z (-512..511): see
[`wide_z_range_cost.md`](../agent_notes/wide_z_range_cost.md), § "Formats for custom maps".

- Map cells and statics never travel in the protocol. Each client reads them from its own files.
- What does travel is the z of positions: items, mobiles, effects, targets, drops and moves. Today it is a signed byte.

This spec does four things:
- lists every packet that carries a z;
- defines a capability, negotiated at login, after which those fields become i16;
- sketches the Source-X (SphereServer) work;
- notes what this repo's future network layer has to branch on.

**Sources, read at fixed revisions.** The packet facts below come from these sources, not from memory, except where
§ "Unverified" says otherwise.
- **ClassicUO** `main` at ee79d7ebc1cc0e53ff84fe6cae17d389e2737f93 (2026-09-29). Paths are under
  `src/ClassicUO.Client/` and shortened to `cuo:`.
- **Source-X** `master` at e0cf4ed602dc0335f923b100b00d57522eed3da0 (2026-11-18). Paths are under `src/` and shortened
  to `sx:`.

Line numbers hold for those revisions.

**Conventions.**
- The protocol is big-endian.
- Offsets count from the packet id at byte 0.
- "var" marks a variable-length packet, whose length is a u16 at bytes 1-2.
- An 0xBF packet carries its subcommand as a u16 at bytes 3-4.
- "i8 at 16" means a signed byte at offset 16. "i16 at 15-16" means a big-endian signed short at offsets 15 and 16.

## 1. Packets that carry z today

### Server to client

Builders are in `sx:network/send.cpp`. Handlers are in `cuo:Network/PacketHandlers.cs`. Lengths come from
`cuo:Network/PacketsTable.cs`.

**z as a signed byte: these change.**

| Id | Packet | Length | z today | Sphere builder: line, z write | ClassicUO handler: line, z read |
|---|---|---|---|---|---|
| 0x1A | world item, old form | var | i8 after y and the optional direction or light byte | `PacketItemWorld` 471, 529 | `UpdateItem` 779, 844 |
| 0x20 | player update | 19 | i8 at 18 | `PacketPlayerUpdate` 721, 744 | `UpdatePlayer` 1157, 1173 |
| 0x21 | move reject | 8 | i8 at 7 | `PacketMovementRej` 757, 769 | `DenyWalk` 1178, 1190 |
| 0x23 | drag animation | 26 | i8 at 16 (source) and 25 (target) | `PacketDragAnimation` 797, 817-848 | `DragAnimation` 1219, 1228 and 1232 |
| 0x70 | effect | 28 | i8 at 16 (source) and 21 (target) | `writeBasicEffect` 1986, 2035 and 2038; `writeBasicEffectLocation` 2095, 2117 and 2128 | `GraphicEffect` 2435, 2467 and 2470 |
| 0xC0 | hued effect | 36 | as 0x70 (hue at 28-31, render mode at 32-35) | as 0x70, plus `writeHuedEffect` | `GraphicEffect` |
| 0xC7 | particle effect | 49 | as 0x70 (0xC0's fields, then effect, explode effect, explode sound, serial, layer and a u16, at 36-48) | as 0x70 | `GraphicEffect` |
| 0x77 | mobile moving | 17 | i8 at 11 | `PacketCharacterMove` 2385, 2400 | `UpdateCharacter` 2787, 2805 |
| 0x78 | mobile incoming | var | i8 at 13; the equipment list after it has no z | `PacketCharacter` 2417, 2435 | `UpdateObject` 2827, 2838 |
| 0xE5 | waypoint add | var | i8 at 11 (serial 3-6, x 7-8, y 9-10, map 12, type 13-14, u16 15-16, cliloc 17-20, name) | `PacketWaypointAdd` 5138, 5153 | `DisplayWaypoint` 5616, 5621 |
| 0xF3 | world item, SA form | 24; 26 from 7.0.9.0 | i8 at 19 | `PacketItemWorldNew` 5257, 5304. The mobile form is at 5316, 5334 | `UpdateItemSA` 5702, 5718 |
| 0xF7 | packet container | var | holds 0xF3 records of 26 bytes each | `PacketContainer` 5442, 5486 and 5512. It is never constructed | `PacketList` 5917, which calls `UpdateItemSA` per record |

Notes on the table:
- **0x1A and 0xF3.** Sphere sends 0x1A only to clients older than 7.0.0.0, and for corpses, whose direction 0xF3
  doesn't show (`sx:game/clients/CClientMsg.cpp:351-356`).
- **0x23.** Sphere never sends it to 7.0.0.0+ or Enhanced clients (`PacketDragAnimation::canSendTo`, send.cpp:852).
  It builds the packet once per drag and broadcasts it (`sx:game/chars/CCharAct.cpp:680-681`, `UpdateCanSee`), where
  every other z packet is built for its own recipient.

**z already 16 bits wide: the layout doesn't change.**

| Id | Packet | Length | z today | Sphere | ClassicUO |
|---|---|---|---|---|---|
| 0x1B | login confirm | 37 | 16-bit slot at 15-16 (serial 1-4, body 9-10, x 11-12, y 13-14, direction 17) | `PacketPlayerStart` 624. Writes `0x00` then the i8 (636-637): **not sign-extended** | `EnterWorld` 883: `(sbyte)ReadUInt16BE()` (894), keeping the low byte |
| 0x54 | sound | 12 | i16 at 10-11 | `PacketPlaySound` 1550, sign-extended (1559) | `PlaySoundEffect` 2193 reads a short and doesn't use it |
| 0x76 | server change | 16 | i16 at 5-6 | `PacketZoneChange` 2361, sign-extended (2367) | no handler; the length table only skips it |
| 0xF6 | boat moving | var | ship at 14-15, then i16 per object (serial, x, y, z: 10 bytes each) | `PacketMoveShip` 5402, sign-extended (5415, 5428) | `BoatMoving` 5754 reads a u16, then casts to `sbyte` (5801-5904) |
| 0x38 | pathfind | 7 | u16 at 5-6 | never sent | `Pathfinding` 2075 reads a `ushort`, so a negative z is misread |
| 0x6C | target cursor | 19 | byte 15 unknown, i8 at 16 | `PacketAddTarget` 1753 writes zeros after the flags | `TargetCursor` 359 reads nothing after the flags |
| 0xA9 | character list | var | i32 per start location, from 7.0.13 | send.cpp:3409 | |

**z relative to something else: no change.**

| Id | Packet | z | Sphere | ClassicUO |
|---|---|---|---|---|
| 0x99 | multi placement cursor | u16 at 24-25, relative to the cursor (the multi's dz). Length 26, or 30 from 7.0.9.0 (hue at 26-29) | `PacketAddTarget` 1772, 1803 | `MultiPlacement` 3313 |
| 0xD8 | custom house design | per component: i8 relative to the foundation (plane mode 0), or derived from the plane, `(planeZ - 1) % 4 * 20 + 7` (modes 1 and 2) | `PacketHouseDesign` 4806; the stairs buffer stores `(byte)z` (4899) | `CustomHouse` 5256 sums `(sbyte)(item.Z + z)` (5143, 5178, 5235) |
| 0xBF.0x20 | house customization begin and end | i8 at 16 (serial 5-8, type 9, graphic 10-11, x 12-13, y 14-15). Sphere sends 0xFF sentinels | `PacketHouseBeginCustomise` 4431, `PacketHouseEndCustomise` 4458 | `ExtendedCommand` case 0x20, 4584-4590 |

0xD2 and 0xD3, the UO:3D forms of 0x77 and 0x78, are out of scope:
- Sphere declares them but never sends them.
- ClassicUO decodes them with the 0x77 and 0x78 handlers.
- If a server ever sends them, they follow the 0x77 and 0x78 rule.

**Checked, no z:**
- 0x15, 0x1D, 0x22, 0x24, 0x25, 0x2E, 0x2F, 0x3C, 0x56, 0x89 and 0x97;
- 0x90 and 0xF5 (maps), 0xBA (quest arrow), 0xF0 (Krrios);
- speech;
- the 0xBF subcommands 0x08, 0x1D and 0x22.

### Client to server

Receivers are in `sx:network/receive.cpp`. Writers are in `cuo:Network/OutgoingPackets.cs`.

| Id | Packet | Length | z today | Sphere receiver | ClassicUO writer |
|---|---|---|---|---|---|
| 0x08 | drop item | 14; 15 from 6.0.1.7, KR and EC (grid byte at 10) | i8 at 9 (serial 1-4, x 5-6, y 7-8, container last) | `PacketItemDropReq`: `getExpectedLength` 414 returns 14 or 15; `onReceive` 426 reads a byte, and `CPointMap(..., (int8_t)z, ...)` at 452 | `Send_DropRequest_Old` 547 and `Send_DropRequest` 589, `WriteInt8(z)` |
| 0x6C | target response | 19 | ClassicUO: `WriteUInt16BE((ushort)z)` at 15-16, so the sign extends. Sphere: `skip(1)`, then the byte at 16 | `PacketTarget::onReceive` 1078, 1094-1095 | `Send_TargetObject` 1686, `Send_TargetXYZ` 1734 |

**z wider than a byte or relative: no change.**
- **0xD7, house design:** i32, relative to the house; Sphere casts it to `char` (3801, 4016, 4052).
- **0xF0, EC and KR movement:** x, y and z as i32; Sphere skips them.
- **0xF4, crash report:** an i8 z, which Sphere only logs.

**Checked, no z:**
- 0x02, 0x07, 0x13, 0x56;
- the 0xBF subcommands;
- 0x00, 0x8D and 0xF8 (character creation picks a start location by index).

### Where the sources disagree

| Packet | Disagreement | Effect |
|---|---|---|
| 0x1B | Sphere writes `0x00, i8`. A decoder that reads i16 sees z = -5 as 251. ClassicUO keeps the low byte | Correct today only because ClassicUO truncates. Narrow mode must keep the low-byte read |
| 0x6C, client to server | ClassicUO writes a sign-extended 16-bit z at 15-16. Sphere skips byte 15 and reads byte 16 | Equivalent within the i8 range. Wide mode reads 15-16 as i16, which is already what ClassicUO writes |
| 0x38 | ClassicUO reads z as unsigned | A ClassicUO bug. Sphere never sends 0x38 |
| 0xF6 | Sphere sends i16. ClassicUO truncates to `sbyte` | Fine for i8 values. A wide-capable client must keep the i16 |
| 0xD8 | ClassicUO sums the foundation z and the relative z into an `sbyte` | Overflows on wide maps. Sum in i16 |
| 0x76 | ClassicUO has no handler | Not a z problem |

Two disagreements don't involve z: the meaning of 0xF3 bytes 10 and 20 (Sphere: direction and light; ClassicUO:
graphic increment and direction), and 0x20 bytes 15-16 (Sphere writes 0; ClassicUO names it server id).

## 2. Negotiation

### Choice: a new 0xBF subcommand, offered by the client before it picks a character

**Why 0xBF.** The 0xBF extended command is the protocol's own extension point:
- OSI added subcommands up to 0x33 this way (`EXTDATA_WheelBoatMove = 0x33`, `sx:common/sphereproto.h:273`).
- ClassicUO already uses a private subcommand, 0xBEEF, for plugin traffic (`cuo:Network/OutgoingPackets.cs:4565`;
  handled at PacketHandlers.cs:4724).

Both sides frame 0xBF by its length, so an unknown subcommand never desyncs the stream:
- **ClassicUO** logs `Unhandled 0xBF - sub` and continues (PacketHandlers.cs:4731).
- **Sphere** drops a subcommand that has no handler (receive.cpp:2546-2575). It also drops every 0xBF that arrives
  before a character is attached (receive.cpp:2552). The game loop ignores the handler's return value
  (`sx:network/CNetworkInput.cpp:381`), so the drop is silent and the connection stays up.

**Subcommand number.** The proposal is **0x00E0**. It must stay below 0x100, the size of Sphere's handler table
(`m_extended[NETWORK_PACKETCOUNT]`, `sx:network/CPacketManager.h:15` and `:22`, and `getExtendedHandler`,
CPacketManager.cpp:223). It is far from OSI's range. The value is an open question.

**Why before world entry.**
- Sphere builds the whole world burst while it processes 0x5D (play character). `Setup_Play` →
  `Setup_Start` → `addPlayerStart` sends 0x1B, and the objects in view follow (`sx:game/clients/CClientMsg.cpp:2911`,
  `:2795`, `:1465`). Builders write their bytes in the constructor.
- So the encoding must be settled before the server reads 0x5D.
- Before 0x5D, the server sends only 0xB9 (features) and 0xA9 (character list) (`Setup_ListReq`,
  CClientMsg.cpp:2989, 3047-3048). Neither carries a z that changes.

**Why three messages.** The three messages are an offer, an accept and a commit.
- The connection switches encoding at the commit, which the client sends just before 0x5D.
- Both sides see it at the same point in the stream. No timing assumption is needed.
- A two-message scheme (offer, then accept, switching at the accept) breaks in one case. Say the accept arrives after
  the client stops waiting for it: the client goes narrow while the server is already wide.
- Sphere's per-priority output queues also let packets of different priorities overtake each other
  (`sx:network/CNetworkOutput.cpp:75-113`). Stream order between the accept and the world burst is therefore not
  guaranteed by construction.

### Messages

All three use 0xBF subcommand `PROTOCOL_EXT_SUBCOMMAND` = 0x00E0. A u8 `op` follows the subcommand.

**OFFER, client to server, 12 bytes:**

| Offset | Size | Field |
|---|---|---|
| 0 | 1 | 0xBF |
| 1-2 | 2 | length = `PROTOCOL_EXT_OFFER_LEN` = 12 |
| 3-4 | 2 | 0x00E0 |
| 5 | 1 | op = `PROTOCOL_EXT_OP_OFFER` = 1 |
| 6-7 | 2 | extension version the client speaks, `PROTOCOL_EXT_VERSION` = 1 |
| 8-11 | 4 | offered capabilities: bit 0 = `PROTOCOL_EXT_CAP_WIDE_Z`. Other bits are 0 |

**ACCEPT, server to client, 16 bytes:**

| Offset | Size | Field |
|---|---|---|
| 0-4 | 5 | 0xBF, length = `PROTOCOL_EXT_ACCEPT_LEN` = 16, 0x00E0 |
| 5 | 1 | op = `PROTOCOL_EXT_OP_ACCEPT` = 2 |
| 6-7 | 2 | version: the lower of the client's and the server's |
| 8-11 | 4 | accepted capabilities: offered ∩ enabled on this server. 0 is a refusal |
| 12-13 | 2 | i16 `z_min`: the lowest z the server will send or accept (version 1: -512) |
| 14-15 | 2 | i16 `z_max`: the highest (version 1: 511) |

**COMMIT, client to server, 12 bytes:** the OFFER's layout with op = `PROTOCOL_EXT_OP_COMMIT` = 3. It echoes the
version and the accepted capabilities from the ACCEPT.

The bounds in the ACCEPT let a later version change the range without changing the layout: a 9-bit server, or a
future full i16. Whether to keep them is an open question.

### Sequence

1. The client connects to the game server and sends the seed and 0x91 (game login).
2. Right after 0x91, in the same write if it likes, the client sends OFFER. It does this only when the shard's
   profile enables the extension (see "Against servers that don't know it").
3. The server processes 0x91: the account logs in, and 0xB9 and 0xA9 are queued. Then it processes OFFER.
   - Conditions: an account is logged in, no character is attached, and the server has the feature enabled.
   - It stores `accepted` as pending in the connection state and replies with ACCEPT.
   - If the feature is disabled, it replies with ACCEPT and `accepted` = 0, so the client doesn't wait.
4. The player picks a character. When the client sends 0x5D:
   - **If it holds an ACCEPT with `accepted` ≠ 0:** it first sends COMMIT and switches its own encoding. Server
     packets still in flight at that moment carry no z that changes (0xB9, 0xA9), so the switch is safe.
   - **Otherwise:** it sends plain 0x5D and stays narrow.
   - A client that logs in automatically waits up to `PROTOCOL_EXT_ACCEPT_TIMEOUT` (proposed: 5 s) for the ACCEPT.
     A person picking a character never waits, because the ACCEPT arrives with the character list.
5. On COMMIT, the server checks that the committed capabilities are a subset of the pending ones and that no
   character is attached yet. Then it switches the connection to the wide encoding, in both directions. The 0x5D
   that follows, and everything after it, use the new encoding.

The server ignores:
- an OFFER or COMMIT that arrives after a character is attached;
- a COMMIT without a prior ACCEPT;
- a COMMIT whose capabilities aren't a subset of the ACCEPT's.

A repeated OFFER before 0x5D gets a fresh ACCEPT.

The encoding depends only on the connection's committed flag, never on the map. Decoders need no map knowledge.

Sphere's commented-out "linger" branch in `Setup_ListReq` (CClientMsg.cpp:3022-3034) would jump from 0x91
straight into the world. If it is ever re-enabled, a client relogging into a lingering character enters the world
before its OFFER is read, and its connection stays narrow.

### Rejected alternatives

- **A bit in 0x5D's client-flags field (offset 37):**
  - ClassicUO already writes its protocol flags there (`Send_SelectCharacter`, OutgoingPackets.cs:496).
  - Sphere skips the field (receive.cpp:957, "0x3f - Orion, Classic, ClassicUO, 0xff - Enhanced client").
  - A bit there would overload an expansion field, and there is no channel for the server's answer.
- **Bits in 0xB9 or 0xA9:** the server would have to set them for every client, so vanilla clients would see changed
  packets.
- **A suffix on the 0xBD version string:** servers parse that string to pick packet forms (Sphere's
  `isClientVersionNumber` gates), so a suffix risks breaking those parsers. It also has no answer channel.
- **Krrios 0xF0:** Sphere overloads it with EC movement (receive.cpp:4495).
- **A new top-level packet id:** Sphere discards the rest of the receive buffer on an unknown id
  (`sx:network/CNetworkInput.cpp:397-409`). That would swallow a 0x5D sent in the same write.
- **Negotiating after world entry:** the first world burst would go out narrow. The server would have to resend the
  view, and the switch would race the output queues.

### Against servers and clients that don't know it

**A vanilla client never sees a changed packet.** It never sends OFFER, so its connection is never committed. The
server never sends it an ACCEPT either, because ACCEPT only answers an OFFER.

**Our client against a server that doesn't know the extension:**
- **Vanilla Sphere** drops OFFER silently (verified above). The client gets no ACCEPT and stays narrow.
- **RunUO-family servers** (RunUO, ServUO, ModernUO) are not verified; this is from memory. They register 0xBF as an
  in-game packet and disconnect a client that sends one before a mobile is attached.
- So the client must not probe blindly. It sends OFFER only when the shard's profile says the server supports the
  extension. Whether to probe automatically is an open question.

**Third-party tools in the stream** (proxies or sniffers with their own length tables) desync on a committed
connection, because fixed lengths change. Sphere's own packet recording and its script packet filters see the new
layouts (see § 4).

### Mixed clients on one server

The server encodes per connection:
- **Committed connections** get i16, clamped to [`WIDE_Z_MIN`, `WIDE_Z_MAX`].
- **The others** get i8, clamped to [`NARROW_Z_MIN`, `NARROW_Z_MAX`].

**On narrow maps** (every official facet and any custom map without wide z), clamping never fires if the server keeps
each map's z inside the map's own range (§ 4, "Per-map range"). Wide-capable and vanilla clients mix freely there.

**On a wide map, a vanilla client can't be served correctly:**
- Objects above 127 or below -128 pile up at the clamp. The draw order among them is lost.
- Its terrain and statics come from its own i8 map files, which cannot hold the wide map. So mobiles float or sink
  relative to what it draws.
- Its walk prediction runs on those same files, so the server rejects its moves (0x21).
- Its target responses carry clamped z, so line-of-sight and placement checks fail.

**Recommendation:** mark maps as wide in the server config and refuse narrow connections on them:
- at login, when the character stands on a wide map;
- and on any move into a wide map.

Clamping stays as a safety net. The alternative of hiding objects outside the i8 range from vanilla clients still
leaves them with wrong terrain. Which to choose is an open question.

## 3. Wire format after COMMIT

### Bounds

| Constant | Value | Meaning |
|---|---|---|
| `WIDE_Z_MIN` | -512 | lowest z on a committed connection (10-bit two's complement) |
| `WIDE_Z_MAX` | 511 | highest |
| `NARROW_Z_MIN` | -128 | lowest z on a vanilla connection (i8) |
| `NARROW_Z_MAX` | 127 | highest |

The wire type is i16 big-endian, and values stay within the bounds the ACCEPT announced. A later version can lift the
limit without changing the layout.

### Layouts

**The rule:** every field that is an i8 z today becomes an i16 z at the same offset. The fields after it shift by
one byte. Fields that are already 16 bits wide keep their offset and become i16 sign-extended. Relative z fields
don't change.

**Server to client, changed:**

| Id | Length: narrow → wide | Wide z offsets, and the fields that move |
|---|---|---|
| 0x1A | var, +1 | z i16 in its current position; the hue and flags after it shift by 1 |
| 0x1B | 37 → 37 | z i16 at 15-16, **sign-extended** (narrow mode keeps Sphere's `0x00, i8`) |
| 0x20 | 19 → 20 | z 18-19 |
| 0x21 | 8 → 9 | z 7-8 |
| 0x23 | 26 → 28 | source z 16-17, target serial 18-21, x 22-23, y 24-25, z 26-27 |
| 0x70 | 28 → 30 | source z 16-17, target x 18-19, y 20-21, z 22-23, speed 24, duration 25, u16 26-27, fixed direction 28, explode 29 |
| 0xC0 | 36 → 38 | as 0x70, then hue 30-33 and render mode 34-37 |
| 0xC7 | 49 → 51 | as 0xC0, then effect 38-39, explode effect 40-41, explode sound 42-43, serial 44-47, layer 48, u16 49-50 |
| 0x77 | 17 → 18 | z 11-12, direction 13, hue 14-15, flags 16, notoriety 17 |
| 0x78 | var, +1 | z 13-14, direction 15, hue 16-17, flags 18, notoriety 19, equipment unchanged |
| 0xE5 | var, +1 | z 11-12, map 13, type 14-15, u16 16-17, cliloc 18-21, name |
| 0xF3 | 24 → 25; 26 → 27 | z 19-20, then byte 21, hue 22-23, flags 24, and the u16 at 25-26 from 7.0.9.0 |
| 0xF7 | var | each 0xF3 record grows to 27 bytes |
| 0x6C | 19 → 19 | i16 z at 15-16, replacing the unknown byte and the i8; Sphere writes 0 |

**Server to client, unchanged but read as i16:** 0x54 (10-11), 0x76 (5-6), 0xF6 (14-15 and per object), 0x38 (5-6).

**Client to server:**

| Id | Length: narrow → wide | Wide z |
|---|---|---|
| 0x08 | 14 → 15; 15 → 16 | z 9-10, then [grid 11], then the container |
| 0x6C | 19 → 19 | z i16 at 15-16, which ClassicUO's writer already produces |

**No packet needs a form that can't grow.**
- Only committed connections see the wide layouts.
- Every fixed length above changes together in both sides' length tables.
- The two packets with a 16-bit z slot keep their length: 0x1B, and 0x6C in both directions.
- The relative fields (0x99, 0xD8, 0xBF.0x20) don't need to grow, and 0xD7's are already i32:
  - house components stay within a few planes of their foundation (at most `(4 - 1) * 20 + 7` = 67 above it);
  - a multi's dz is already 16 bits.
- 0xF4 (crash report) stays i8. A committed client clamps the value; it is diagnostics only.

Other ways to carry the high bits were considered and rejected:
- spare high bits in other fields: 0x1A's x and y have a few, but 0x70 and 0x77 have none, so each packet would need
  its own trick;
- a side packet carrying only the high byte.

Either way, each object would get a second decode path.

### Validation

- **Client-to-server values** outside [`z_min`, `z_max`], or outside the map's range: the server **rejects** the
  action. It doesn't clamp, because a clamped z acts on a location the player didn't pick.
  - **0x08:** the drop is refused, and the item goes back to where it was (`Event_Item_Drop_Fail`, the path that
    refuses quest items, `sx:game/clients/CClientEvent.cpp:308`).
  - **0x6C:** treated as a cancelled target. The receiver passes x = -1 and serial 0, so `Event_Target` takes its
    cancel branch (CClientEvent.cpp:2482-2486).
- **Server-to-client values** outside the announced bounds: the client clamps to the bounds and logs one warning per
  connection. This is a server bug, and dropping the object would lose state.
- **The server never puts a value outside its bounds on the wire.** The encoder clamps as a last line of defence.

## 4. Source-X implementation sketch

### Negotiation and connection state

- **`sx:common/sphereproto.h`:**
  - add `EXTDATA_ProtocolExt = 0xE0` to `EXTDATA_TYPE` (217-274). `EXTDATA_QTY` is unused, so the gap is harmless;
  - add the constants: `PROTOEXT_VERSION`, `PROTOEXT_OP_OFFER`, `PROTOEXT_OP_ACCEPT`, `PROTOEXT_OP_COMMIT`,
    `PROTOEXT_CAP_WIDEZ`, `PROTOEXT_OFFER_LEN`, `PROTOEXT_ACCEPT_LEN`;
  - add the bounds `UO_WIRE_Z_MIN_WIDE`, `UO_WIRE_Z_MAX_WIDE`, `UO_WIRE_Z_MIN_NARROW` and `UO_WIRE_Z_MAX_NARROW`.
- **`sx:network/CNetState.h`:** add `dword m_protoExtPending`, `dword m_protoExtCaps` and
  `bool isWideZ() const`. Clear them in `CNetState::clear` (CNetState.cpp:44).
- **`sx:network/receive.cpp:2546` (`PacketExtendedCommand::onReceive`):** read the length and subcommand before the
  `GetChar() == nullptr` test. Let `EXTDATA_ProtocolExt` through when `GetAccount() != nullptr`.
- **New `PacketProtocolExt` receiver (receive.cpp and receive.h):**
  - check `packetLength`;
  - read `op`, the version and the capabilities;
  - apply the OFFER and COMMIT rules of § 2.
- **Registration:** `registerExtended(EXTDATA_ProtocolExt, new PacketProtocolExt())` in `CPacketManager.cpp` (next
  to 110).
- **New `PacketProtocolExtAccept` builder (send.cpp and send.h):**
  `PacketExtended(EXTDATA_ProtocolExt, PROTOEXT_ACCEPT_LEN, PRI_HIGHEST)` (the constructor is at send.cpp:3944).
- **`sphere.ini`:** a global `WideZ=0|1`, default 0. Add a bool member in `CServerConfig`, plus an `RC_` enum entry
  and a key table entry, following `USEPACKETPRIORITY` (CServerConfig.cpp:338, 731 and 1026).

### Per-map range

- `MAPx=MaxX,MaxY,SectorSize,MapReadID,MapSendID` gains an optional sixth field: 1 = wide z. The parser is
  `CUOMapList::Load`, `sx:game/uo_files/CUOMapList.cpp:77`; `ppCmd[5]` becomes `ppCmd[6]`.
  - A wide map's range is -511..511, mirroring today's ±127.
  - A narrow map keeps -127..127.
- `CPointBase::IsValidPoint` and `IsCharValid` (`sx:common/CPointBase.cpp:269-296`) already look up the map for x
  and y. They take z's bounds from the map instead of `UO_SIZE_Z`.
- Script setters (Z, P, MOREZ, MOREP) then reject or clamp z to the map's range. The choice follows each setter's
  existing behaviour for invalid points.
- Refuse narrow connections on wide maps:
  - in `CClient::Setup_Start` (CClientMsg.cpp:2795), before `addPlayerStart`, return a login error;
  - in `CChar::MoveToChar` (`sx:game/chars/CCharAct.cpp:5307`), reject a map change into a wide map for a narrow
    connection.

### Packet encoders and decoders

- **Helpers:**
  - `PacketSend::writeZ(const CNetState*, int16 z)` writes i16 when the connection is wide, else a clamped byte;
  - `Packet::readZ(const CNetState*)` mirrors it.
  - Each z-carrying builder above calls `writeZ(target->GetNetState(), ...)` in place of `writeByte(...m_z)`. These
    are 0x1A, 0x20, 0x21, 0x23, 0x70, 0xC0, 0xC7, 0x77, 0x78, 0xE5, 0xF3 and 0xF7, about 15 call sites.
  - The builders' initial sizes are buffer hints (`PacketSend(id, len, prio)`). The sent length is what was written
    (`Packet::sync` and `trim`, packet.cpp:683-695), so fixed packets grow by themselves.
- **0x1B** (send.cpp:636-637): `writeInt16(z)` sign-extended when wide. Narrow mode keeps `0x00, i8`.
- **0x6C, server to client** (1753): `writeInt16(0)` at 15-16 when wide. Narrow mode stays as it is.
- **0x23:** `CChar::UpdateDrag` builds one packet for every viewer. Build a second, wide instance for committed
  viewers, or build per viewer. 0x23 never reaches 7.0.0.0+ clients anyway.
- **0x08:** `PacketItemDropReq::getExpectedLength` (receive.cpp:414) adds 1 when wide; `onReceive` (426) uses `readZ`.
- **0x6C, client to server:** `PacketTarget::onReceive` (1078) reads i16 at 15-16 when wide.
- **Validation:** both receivers range-check as in § 3 before building the `CPointMap`.

### Widening the world z

Sphere stores z as `int8` almost everywhere. In the full `src/`:
- `m_z` appears 427 times in 37 files;
- about 135 narrow declarations or casts of z sit in about 30 files.

The work:

- **`CPointBase`** (`sx:common/CPointBase.h:35-38`): `int8 m_z` → `int16`. The struct grows from 6 to 8 bytes.
  - The "Do NOT change these datatypes" comment ties the layout to the MUL files. Check that no code reads a
    `CPointBase` straight from file bytes.
  - `m_z` also doubles as an equipped item's layer (`GetEquipLayer`). That still fits.
- **The `CItem` type union** (`sx:game/items/CItem.h:160`, "4+4+6 = 14 bytes") is the delicate part.
  - Its `m_itNormal.m_morep` and the `m_pntOpen`, `m_ptMark` and `m_pt` members are `CPointBase`.
  - 9 type structs alias the MOREZ byte at offset 12: `m_poison_skill`, `m_pattern`, `m_facing_dir`, `m_iDist` and
    others.
  - 4 type structs alias MOREM at offset 13: `m_map`, `m_foodval` and `m_nonused`.
  - With an i16 z, offset 13 becomes z's high byte and MOREM moves to 14. Every alias must be re-laid, and the union
    grows to 16 bytes.
- **Bounds:** `UO_SIZE_Z` and `UO_SIZE_MIN_Z` (`sx:game/uo_files/uofiles_macros.h:28-29`, 127 and -127) have 42 uses
  in 11 files.
  - They serve as validity bounds and as "none" sentinels (`m_Top.m_z = UO_SIZE_Z`, `m_Bottom.m_z = UO_SIZE_MIN_Z`,
    `sx:common/CServerMap.cpp:54-85`).
  - Both roles become 511 and -511, or the per-map range.
  - One use is a *height*: `CItemBase.cpp:834` returns `UO_SIZE_Z` as the height of an item without tiledata
    (`height_t` is `uchar`). It needs its own constant.
- **Narrow types:**
  - `CServerMapBlocker::m_z`, and `CServerMapBlockingState`'s `m_z` and `m_zClimb`, constructors and `CheckTile*`
    parameters;
  - `CWorldMap::GetHeightPoint` and `GetHeightPoint2` (they return `char`);
  - `CItem::GetFixZ` and `CChar::GetFixZ`;
  - `CObjBaseTemplate::GetTopZ`, `GetUnkZ`, `SetUnkZ` and `SetTopZ`;
  - the line-of-sight code in `CCharLOS.cpp`: `(char)minimum(iTotalZ, UO_SIZE_Z)` at 133 and 682, and the
    `(char)dz` casts;
  - the multi movement bounds in `CCMultiMovable.cpp:285`, `:287` and `:1226`.
- **Parsing:**
  - `CPointBase.cpp:852` (`m_z = (char)iVal`) and `:1008` (`Str_ToI8`);
  - `CObjBase.cpp:3052` (`SetUnkZ((char)...)`);
  - `CItem.cpp:3483` (MOREZ via `GetArgCVal`).
  - All become 16-bit with a range check.
- **Script-visible packing:** `CCharAct.cpp:5361` packs `((uchar)ptOld.m_z << 16) | ptOld.m_map` into ENVIRONCHANGE's
  ARGN3. A wide z needs a 16-bit field there, and scripts that unpack it change.
- **Formats:** about ten `%hhd` format strings print z.
- **Unchanged:**
  - `CUOMultiItemRec` already stores `short m_dz`;
  - house design z (`CItemMultiCustom`, `int8` arguments, `GetPlaneZ`) is relative and stays `int8`;
  - saves are text (`P=x,y,z,map`), so the save format doesn't change.

### Map data

`CUOMapMeter` (u16 id and `int8 m_z`, 3 bytes) and `CUOStaticItemRec` (`char m_z`, 7 bytes) mirror the MUL records.
They stay for MUL maps. A wide map needs wide records and a reader for whichever source format is chosen (the wide-z
note's open question 3). That choice is shared with this repo.

Sphere doesn't stream map data to clients: its 0x3F receiver only logs, and it has no UltimaLive support. So nothing
else on the wire changes.

### Script surface

- **A read-only client property `WIDEZ` (0 or 1),** so that the following can branch on it:
  - `PACKETxx` and `OUTPACKETxx` packet filters (`xPacketFilter` at CClientEvent.cpp:3291, `xOutPacketFilter` at
    3341), which see raw bytes;
  - `SENDPACKET` (CClient.cpp:1522).
- **Z, P, MOREZ and MOREP** accept the map's range. **DZ** (multi components) is already 16 bits.

### Size

| Area | Files | Lines, rough |
|---|---|---|
| Negotiation: enum, receiver, builder, connection state, ini key, dispatcher | 8 | 120 |
| Packet encoders and decoders, validation | 4 (send and receive, .cpp and .h) | 120 |
| Per-map range and the wide-map refusal | 4 | 50 |
| Core widening: CPointBase, the CItem union, bounds, accessors, LOS, blocking state, parsing, formats | about 35 | 300-500, mostly mechanical; the union re-layout needs care |
| Wide map and statics reader | depends on the source format | 200-400 |

That is about 40 files and 800-1200 changed lines. The packets and negotiation are about a quarter of it.

## 5. Client side: this repo's future network layer

The repo has no packet code yet. The only socket is the local JSON control channel, whose `Teleport` and
`SpawnMobile` carry `z: i8` (`dynamapper/src/input/dev_control.rs:35`, `:56`). It needs i16 once the renderer goes
wide. When the network layer is written:

- **Per-connection state:** a `committed_caps` set at COMMIT, plus the ACCEPT's `z_min` and `z_max`. Every z decoder
  and encoder takes it.
- **Constants:** `WIDE_Z_MIN`, `WIDE_Z_MAX`, `NARROW_Z_MIN`, `NARROW_Z_MAX` (i16), `PROTOCOL_EXT_SUBCOMMAND`,
  `PROTOCOL_EXT_VERSION`, `PROTOCOL_EXT_OP_*`, `PROTOCOL_EXT_CAP_WIDE_Z`, `PROTOCOL_EXT_OFFER_LEN`,
  `PROTOCOL_EXT_ACCEPT_LEN` and `PROTOCOL_EXT_ACCEPT_TIMEOUT`.
- **The length table depends on the connection.**
  - It already depends on the client version presented: 0x08 is 14 or 15, 0x99 is 26 or 30, 0xF3 is 24 or 26
    (`cuo:Network/PacketsTable.cs:316-321` and `:361-372`).
  - The version presented also decides which forms Sphere sends: 0x1A or 0xF3, and whether 0x23 arrives at all.
  - On a committed connection, the fixed lengths of 0x20, 0x21, 0x23, 0x70, 0xC0, 0xC7, 0x77 and 0xF3 also change
    (§ 3).
- **Decoders that branch on the capability:** 0x1A, 0x1B, 0x20, 0x21, 0x23, 0x70, 0xC0, 0xC7, 0x77, 0x78, 0xE5,
  0xF3, and 0xF7 per record.
- **Narrow 0x1B:** read the 16-bit slot and keep the low byte as an i8 (ClassicUO's reading). Wide 0x1B: i16.
- **Always i16, whatever the mode:** 0x54, 0x76, 0xF6, 0x38. Don't truncate as ClassicUO does.
- **Relative values, summed in i16:**
  - 0xD8 components: foundation z + relative z, or the plane formula;
  - 0x99's dz.
- **Encoders that branch:**
  - 0x08: i8 or i16 at 9, and the length;
  - 0x6C: always write a sign-extended 16-bit z at 15-16. Narrow servers read byte 16, wide ones read 15-16.
- **Validation:** clamp incoming z to the announced bounds and warn once. Never send a z outside the bounds.
- **Entity positions** are stored as i16 from the start, so nothing narrows between the decoder and the renderer.
- **Out of scope:** UltimaLive (0x3F and 0x40, map and statics streaming). ClassicUO implements it in
  `cuo:Game/UltimaLive.cs`, and its records carry an i8 z. If server-streamed wide maps are ever wanted, it needs a
  wide variant of its own.

## Unverified

These are from memory and were not checked against source:

- **RunUO, ServUO and ModernUO:**
  - 0xBF is registered as an in-game packet, so one sent before a mobile is attached disconnects the client;
  - unknown subcommands that arrive in game are traced and ignored.
- **OSI clients:** how they handle an unknown 0xBF subcommand, and a non-zero high byte in 0x1B's z slot. Neither
  matters as long as only committed connections see the new forms and narrow 0x1B keeps Sphere's bytes.

## Open questions for the user

1. **The subcommand number:** 0x00E0 is proposed. It must be below 0x100 for Sphere's table.
2. **Names:** the `sphere.ini` key (`WideZ`) and the per-map flag (a sixth `MAPx` field).
3. **Vanilla clients on wide maps:** refuse (recommended), hide the objects outside the i8 range, or only clamp.
4. **Probing:** should our client probe servers automatically? Blind probing is unsafe on RunUO-family servers, from
   memory. The alternative is a per-shard setting.
5. **Bounds in the ACCEPT:** keep them, which allows 9-bit servers and a later i16 without a layout change, or fix the
   range in the version.
6. **Edge packets:** widen 0xF4 (crash report), and 0x38, which Sphere never sends? Both are proposed as unchanged.
7. **The custom map source format:** the wide-z note's question 3. Sphere needs a reader for it too.
8. **ClassicUO:** should it be patched to speak the extension as well? Its handlers would need the same branches, and
   its 0xF6 and 0xD8 truncations would need fixing.
