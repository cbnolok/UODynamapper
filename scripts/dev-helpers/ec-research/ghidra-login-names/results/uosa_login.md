# UOSA.exe login and network code

`UOSA.exe` 4.0.102.3, SHA-256 `5a5e9dc536c836d1bf10b6f86bb88f61a6ae078f519d4060d3bc47aecbebad50`
(a TOL client). All names below are the ones applied by `NameLoginCode.java`. The generated
report, with every name, locator, handler registration and the full packet length table,
follows this analysis.

## Login flow

| Step | UI / Lua | C++ | Network |
|---|---|---|---|
| Login | `UOLua::TryLogin` | `LoginManager::LoginAccount` (0x00613510) -> `LoginManager::ConnectToLoginServer` (0x00610330) | connect the login connection; `0xEF` seed (unencrypted); install the login cipher; `0x80` account login |
| Shard list | (event) | `UOPacket::Handle_0xA8_BritanniaList` (0x00611f10) | fills `SystemData.Login.shardList` |
| Shard select | `UOLua::SelectShard` (or automatic in 0xA8) | `LoginManager::SendSelectedShard` (0x00613790) | `0xA0` (2-byte shard index) |
| Relay | - | `UOPacket::Handle_0x8C_UserServer` (0x00612670) -> `LoginManager::Disconnect` (login part), `LoginManager::ConnectToGameServer` (0x00613e20) | connect the game connection; 4-byte auth key as the seed; install the game ciphers and Huffman; `0x91` game login (`LoginManager::SendGameLogin`) |
| Characters | (event) | `Handle_0xB9_Features`, `Handle_0xA9_CitiesAndChars`, `Handle_0x86_AllCharacters` | |
| Character select | `UOLua::SelectCharacter` | `LoginManager::SelectCharacter` (0x00611050) | play-character packet with the local IPv4 |
| Delete / create | `UOLua::DeleteCharacter`, char creation | `LoginManager::DeleteCharacter`, `LoginManager::CreateCharacter` | result in `Handle_0x85_ChangeCharResult` |
| World | (event) | `UOPacket::Handle_0x1B_LoginConfirm` (0x00611950) | "Login succeeded to shard %s with character %s"; sends `0xC8` |
| Cancel | `UOLua::UndoLogin` | `LoginManager::Disconnect` (0x006105d0) | closes both connections (vtable +0x50) |

The UO Lua functions read the `LoginManager` singleton from `g_LoginManager` (0x00e37304).
The login UI scripts are not in `UserInterface/Default`; they are most probably packed in
`Interface.uop`.

`UOLua::SetLoginServer` and `UOLua::GetLoginServers` work on `g_LoginConfig` (0x00e37338).
The dormant `LoginConfig::LoadLoginServers` (0x006052a0) contains loaders for
`LoginServersDefault.xml` / `LoginServersUser.xml`, but has no incoming references in
the examined build. The only write to the global is cleanup at 0x004aab6d, which clears
it (`FUN_004aa830`). It stays null; placing XML files in the client folder does not
activate it. `LoginConfig::SetInfoForLogin` (0x006054b0) resolves a selected server and
stores the IP at +0x9c and the port at +0xa0 only if that dormant configuration is used.

### Login address choice (`LoginManager::LoginAccount`)

1. If `g_LoginConfig` is set, the address is `LoginConfig` +0x9c/+0xa0.
2. Otherwise, if app object (`DAT_00e3722c`) +0x1c9 is set, the address is the internal test
   address `172.16.17.41:7775` (0x0061360a).
3. Otherwise, `rand() & 1` picks the host: odd gives `g_DefaultLoginHost1`
   ("107.23.85.115", 0x00cb78cc); even gives `g_DefaultLoginHost2` ("107.23.176.74",
   0x00cb78dc). The host is resolved by `UONet::ResolveHostIPv4`. A second `rand() & 1`
   picks the port: 7775 when odd, 7776 when even.

The address struct keeps the IPv4 and the port in host order. The window title is set to
"UOSA - %s " (account), and later "UOSA - %s (%s) " (account, shard).

## Packet registry, framing and dispatch

- **Length table.** `UONet::PacketRegistry_RegisterAll` (0x005f8630) registers a
  `{dynamic flag, fixed length, name}` entry for each packet id, through
  `UONet::PacketRegistry_FindOrInsert` (0x005fd050). It runs 262 registrations for 251 ids;
  11 ids are registered twice and the last registration wins. The full table is in the
  report below. Login-relevant lengths:
  - fixed: 0x80=62, 0x82=2, 0x85=2, 0x8C=11, 0x91=65, 0xA0=3, 0xB9=5, 0x1B=37, 0x53=2,
    0x5D=73, 0xC8=2, 0xEE=10, 0xEF=21;
  - variable: 0x86, 0xA8, 0xA9, 0xE3, 0xE4.
- **Send.** `UONetMessageSend::UONetMessageSend(buffer, id, flags)` (0x00601700) looks the
  id up with `UONet::PacketRegistry_Find` (0x005fd3c0). It writes the id byte, then a
  big-endian length word for variable packets. Fields are appended with
  `UONet::NetMessage_AppendBytes` (0x00646630). The message is sent through connection
  vtable +0x38, which runs the send transform chain; vtable +0x40 sends without it (used
  for the 0xEF seed).
- **Receive.** `BaseConnection::OnReceive` (vtable slot 5, 0x005ff2d0) reads the socket and
  runs the receive transform chain (decrypt, then decompress). It then calls
  `BaseConnection::ProcessReceivedPackets` (0x005ff5b0), which loops:
  - `UONet::ExtractPacket` (0x005ff6d0): reads the id, takes the fixed length or the
    `ntohs` word at +1, and cuts a `UONetMessageReceive`.
  - `BaseConnection::DispatchPacket` (0x005ff1b0): asks the connection filter (vtable
    +0x2c), then `UONet::PacketHandlerMap_Dispatch` (0x006442f0).
- **Handlers.** They are registered with `UONet::PacketHandlerMap_Register` (0x00644290;
  this = connection +0x24, args packet id and callback). `LoginManager::RegisterPacketHandlers`
  (0x0060ee10) registers:
  - on `g_LoginServerConnection` (0x00e39954): 0xA8, 0x82, 0x8C;
  - on `g_GameServerConnection` (0x00e3994c): 0x82, 0xB9, 0xA9, 0x86, 0x1B, 0xC8, 0x53,
    0x85, 0xEE.

  0xE3 is registered separately in `FUN_00614380` (not renamed).
- **Filter.** `GameServerConnection::FilterPacket` (0x00600720) only lets 0xE3 through
  while the connection state (+0x80) is 1. Any other packet in that state calls
  `LoginManager::Disconnect`. In state 2 everything passes.

| Id | Dir | Connection | Function(s) |
|---|---|---|---|
| 0xEF | C->S | login | `UOLogin::SendSeedAndInstallLoginCipher`, `UOPacket::Write_0xEF_Seed` |
| 0x80 | C->S | login | built in `LoginManager::ConnectToLoginServer` (30-byte account, 30-byte password, 1 byte) |
| 0xA8 | S->C | login | `UOPacket::Handle_0xA8_BritanniaList`, `UOPacket::Read_0xA8_BritanniaList` |
| 0xA0 | C->S | login | `UOPacket::Build_0xA0_SelectServer` |
| 0x82 | S->C | both | `UOPacket::Handle_0x82_LoginFailed` |
| 0x8C | S->C | login | `UOPacket::Handle_0x8C_UserServer` |
| 0x91 | C->S | game | `LoginManager::SendGameLogin` (probable) |
| 0xB9 | S->C | game | `UOPacket::Handle_0xB9_Features` (probable; dword into `g_ServerFeatureFlags`) |
| 0xA9 | S->C | game | `UOPacket::Handle_0xA9_CitiesAndChars` |
| 0x86 | S->C | game | `UOPacket::Handle_0x86_AllCharacters` |
| 0x1B | S->C | game | `UOPacket::Handle_0x1B_LoginConfirm` |
| 0xC8 | S->C | game | `UOPacket::Handle_0xC8_ViewRange` (probable) |
| 0x53 | S->C | game | `UOPacket::Handle_0x53_ErrorCode` (probable; disconnects unless the code is 7) |
| 0x85 | S->C | game | `UOPacket::Handle_0x85_ChangeCharResult` |
| 0xEE | S->C | game | `UOPacket::Handle_0xEE_Unknown` (first word stored in LoginManager +0xa8) |
| 0xE3 | S->C | game | `UOPacket::Handle_0xE3_EncryptionRequest` (probable) |
| 0xE4 | C->S | game | `UOPacket::Build_0xE4_EncryptionResponse` |

## Encryption and compression

- **Seed and version (0xEF).** `UOLogin::SendSeedAndInstallLoginCipher` (0x00600ae0) sends
  five dwords: the seed (the local IPv4 from `UONet::GetLocalIPv4`, 0x00646880), then
  version 67, 0, 102, 0. The EC reports major 0x43 = 67, against file version
  4.0.102.3.
- **Login cipher, client to server.** `UOLoginCipher_C2S_Encrypt` is the classic UO login
  LFSR.
  - `::Init` (0x0063c720) builds the keys from the seed:
    - `k0 = ((~s ^ 0x1357) << 16) | ((s ^ 0xffffaaaa) & 0xffff)`;
    - `k1 = ((s ^ 0x43210000) >> 16) | ((~s ^ 0xabcdffff) & 0xffff0000)`;
    - +0xc/+0x10 hold `s ^ 0xa8612ae5` and `~s ^ 0xfbb33e92`, which this transform does
      not use.
  - `::Transform` (0x0063c7b0) does, per byte:
    `b ^= k0; k1 = (((k1 >> 1 | k0 << 31) ^ 0x1b92eb9c) >> 1 | k0 << 31) ^ 0x1b92eb9d;
    k0 = ((k0 >> 1) | (old_k1 << 31)) ^ 0xb91e227f`.
  - So the version keys are 0x1B92EB9D / 0xB91E227F. ECLO's 0xB5E2127F (another client
    version) is not in this build.
- **Game cipher (classic).** The seed is the 4-byte auth key from 0x8C, sent first on the
  game connection.
  - `UOGameCipher_C2S_Encrypt::Init` (0x00639ea0) makes a Twofish key from the seed (4
    copies) and encrypts a 256-byte pad 0..255 in place (pad +0x114c, counter +0x124c).
  - `::EncryptByte` (0x00639f90) XORs one byte with the pad and re-encrypts the pad every
    256 bytes.
  - `UOGameCipher_S2C_Decrypt::Init` (0x0063be10) builds the same pad and keeps a 16-byte
    digest of it (probably MD5: `FUN_009a876a`/`FUN_009a87fa`).
  - `::Transform` (0x0063be90) XORs each incoming byte with `digest[counter % 16]`.
- **Compression.** `UOHuffmanCoding_S2C_Decompress::Transform` (0x0063c410) does the
  classic server-to-client Huffman decompression.
- **KR/EC key exchange (0xE3/0xE4).** `UOPacket::Handle_0xE3_EncryptionRequest` uses
  Crypto++ `DH_Domain<DL_GroupParameters_GFP_DefaultSafePrime>`, SHA256 and
  `AutoSeededRandomPool`. It sends 0xE4 and sets the game connection state to 2. It then
  installs three transforms:
  - `UOGameCipher2_C2S_Encrypt` (`::Init` 0x0063a810);
  - `UOGameCipher2_S2C_Decrypt` (`::Init` 0x0063b900);
  - Huffman.

  Both Cipher2 classes share slot 0 `CryptoPPAdapter::Transform` (0x006390b0). The
  symmetric cipher behind the adapters was not identified.
- **Transform chains.** Ciphers are appended with `UONet::Connection_AddSendTransform`
  (0x005feeb0) and `UONet::Connection_AddReceiveTransform` (0x005fef40).

## ECLO patch sites

ECLO reads the client version, finds byte patterns and rewrites immediates and strings.
All patterns still match exactly where ECLO expects them in this build:

| Patch | Site | What ECLO changes | Status |
|---|---|---|---|
| PORT1 `C7 44 24 14 ?? ?? ?? ?? 66 C7 44 24 18` | 0x00613612, imm16 at 0x00613617 (7775) | port of the internal test address 172.16.17.41 | matches once; this branch only runs when app +0x1c9 is set, and ECLO leaves the test IP alone, so this patch has no effect on a normal login |
| PORT2 `89 ?? 24 14 66 C7 44 24 18`, first match after PORT1 | 0x006136bd, imm16 at 0x006136c2 (7775) | hardcoded-host port when `rand()` is odd | valid (the pattern matches 9 times; the 8 later matches are packet builders, but ECLO takes the first one after PORT1) |
| PORT3 (TOL) `75 07 66 C7 44 24 18` after PORT2 | 0x006136c6, imm16 at 0x006136cb (7776) | hardcoded-host port when `rand()` is even | valid |
| IP `0A 00 "UOSA - %s " 00 00` + 2 x 16-byte slots | 0x00cb78cc, 0x00cb78dc | both hardcoded login hosts | valid |
| ENC1 `30 45 00 83 83 4C 12 00 00 01 5D 5B` | 0x00639ff9 in `UOGameCipher_C2S_Encrypt::EncryptByte` | NOP the XOR | matches once; commented out in ECLO, not applied |
| ENC2 `30 10 83 46 18 01 8B 45 0C ...` | 0x0063becb in `UOGameCipher_S2C_Decrypt::Transform` | NOP the XOR | matches once; not applied |
| ENC3 `30 08 8B 4E 04 8B 46 08 ...` | 0x0063c7da in `UOLoginCipher_C2S_Encrypt::Transform` | NOP the XOR | matches once; not applied |

Limits of the address and encryption patches:

- The config branch would bypass the built-in address if `g_LoginConfig` were populated,
  but the examined build never populates it. The earlier inference that XML file
  presence alone bypassed the patches was incorrect.
- The encryption NOPs would not cover the 0xE3/0xE4 Crypto++ path, if a server uses it.

## Uncertain points

- "Probable" names:
  - `0xB9` features: the dword goes into `g_ServerFeatureFlags`; its readers were not followed.
  - `0xC8` view range.
  - `0x53` error code.
  - `0xEE`: unknown meaning.
  - `0xE3` encryption request.
  - `SendGameLogin` (0x91).
  - `SetCredentials`.
  - `AutoLoginFromConfig`.
  - The transform-chain and dispatch helpers.
  - The `OnDisconnected` and `FilterPacket` slots.
- The S2C digest is believed to be MD5 from its shape (16 bytes, init/update calls) and
  from the classic client. The digest functions were not checked against constants.
- The connection vtable slots +0x34 (connect), +0x38 (send), +0x40 (send without
  transforms), +0x4c and +0x50 (close) were identified from their use and are not renamed.
- Other managers register their packet handlers through their own wrappers. Those were
  not scanned.

---

## Generated report (second `--apply`, no changes)

Program: UOSA.exe
SHA-256: 5a5e9dc536c836d1bf10b6f86bb88f61a6ae078f519d4060d3bc47aecbebad50 (tested build)
Mode: apply
Changes: {}

### Names

| Address | Name | Tag | Current | Locator |
|---|---|---|---|---|
| 005296e0 | (anchor `luaTryLogin`) | - | UOLua::TryLogin | `str:LuaTryLogin: logging in account` |
| 00529ba0 | (anchor `luaSelectShard`) | - | UOLua::SelectShard | `str:LuaSelectShard: requesting shard` |
| 00529df0 | (anchor `luaSelectCharacter`) | - | UOLua::SelectCharacter | `str:LuaSelectCharacter: requesting c` |
| 0052a050 | (anchor `luaDeleteCharacter`) | - | UOLua::DeleteCharacter | `str:LuaDeleteCharacter: Deleteing ch` |
| 00613510 | `LoginManager::LoginAccount` | UO_LOGIN | LoginManager::LoginAccount | `str:LoginManager::LoginAccount: UI Login started` |
| 00610280 | `LoginManager::SetCredentials` (probable) | UO_LOGIN | LoginManager::SetCredentials | `call:loginAccount:2` |
| 00613710 | `LoginManager::AutoLoginFromConfig` (probable) | UO_LOGIN | LoginManager::AutoLoginFromConfig | `othercaller:setCredentials:loginAccount` |
| 00610330 | `LoginManager::ConnectToLoginServer` | UO_LOGIN | LoginManager::ConnectToLoginServer | `call:loginAccount:14` |
| 00646880 | `UONet::GetLocalIPv4` | UO_NET | UONet::GetLocalIPv4 | `call:connectLogin:2` |
| 00646920 | `UONet::ResolveHostIPv4` | UO_NET | UONet::ResolveHostIPv4 | `call:loginAccount:9` |
| 00600ae0 | `UOLogin::SendSeedAndInstallLoginCipher` | UO_NET | UOLogin::SendSeedAndInstallLoginCipher | `call:connectLogin:4` |
| 00601700 | `UONetMessageSend::UONetMessageSend` | UO_NET | UONetMessageSend::UONetMessageSend | `call:sendSeed:1` |
| 00646630 | `UONet::NetMessage_AppendBytes` (probable) | UO_NET | UONet::NetMessage_AppendBytes | `call:netMessageSend:2` |
| 006008f0 | `UOPacket::Write_0xEF_Seed` | UO_NET | UOPacket::Write_0xEF_Seed | `call:sendSeed:4` |
| 0063c720 | `UOLoginCipher_C2S_Encrypt::Init` | UO_NET | UOLoginCipher_C2S_Encrypt::Init | `call:sendSeed:8` |
| 005feeb0 | `UONet::Connection_AddSendTransform` (probable) | UO_NET | UONet::Connection_AddSendTransform | `call:sendSeed:9` |
| 00613790 | `LoginManager::SendSelectedShard` | UO_LOGIN | LoginManager::SendSelectedShard | `str:LoginManager::SendSelectedShard:` |
| 0060e3e0 | `UOPacket::Build_0xA0_SelectServer` | UO_LOGIN | UOPacket::Build_0xA0_SelectServer | `call:selectShard:1` |
| 00611050 | `LoginManager::SelectCharacter` | UO_LOGIN | LoginManager::SelectCharacter | `call:luaSelectCharacter:9` |
| 00610f70 | `LoginManager::DeleteCharacter` | UO_LOGIN | LoginManager::DeleteCharacter | `call:luaDeleteCharacter:10` |
| 006111d0 | `LoginManager::CreateCharacter` | UO_LOGIN | LoginManager::CreateCharacter | `str:LoginManager::CreateCharacter` |
| 00611f10 | `UOPacket::Handle_0xA8_BritanniaList` | UO_LOGIN | UOPacket::Handle_0xA8_BritanniaList | `handler:0xA8` |
| 00612590 | `UOPacket::Handle_0x82_LoginFailed` | UO_LOGIN | UOPacket::Handle_0x82_LoginFailed | `handler:0x82` |
| 00612670 | `UOPacket::Handle_0x8C_UserServer` | UO_LOGIN | UOPacket::Handle_0x8C_UserServer | `handler:0x8C` |
| 00612990 | `UOPacket::Handle_0xB9_Features` (probable) | UO_LOGIN | UOPacket::Handle_0xB9_Features | `handler:0xB9` |
| 00612a80 | `UOPacket::Handle_0xA9_CitiesAndChars` | UO_LOGIN | UOPacket::Handle_0xA9_CitiesAndChars | `handler:0xA9` |
| 006130b0 | `UOPacket::Handle_0x86_AllCharacters` | UO_LOGIN | UOPacket::Handle_0x86_AllCharacters | `handler:0x86` |
| 00611950 | `UOPacket::Handle_0x1B_LoginConfirm` | UO_LOGIN | UOPacket::Handle_0x1B_LoginConfirm | `handler:0x1B` |
| 00612a10 | `UOPacket::Handle_0xC8_ViewRange` (probable) | UO_LOGIN | UOPacket::Handle_0xC8_ViewRange | `handler:0xC8` |
| 00613430 | `UOPacket::Handle_0x53_ErrorCode` (probable) | UO_LOGIN | UOPacket::Handle_0x53_ErrorCode | `handler:0x53` |
| 00615650 | `UOPacket::Handle_0x85_ChangeCharResult` | UO_LOGIN | UOPacket::Handle_0x85_ChangeCharResult | `handler:0x85` |
| 006133b0 | `UOPacket::Handle_0xEE_Unknown` (probable) | UO_LOGIN | UOPacket::Handle_0xEE_Unknown | `handler:0xEE` |
| 006145c0 | `UOPacket::Handle_0xE3_EncryptionRequest` (probable) | UO_LOGIN | UOPacket::Handle_0xE3_EncryptionRequest | `handler:0xE3` |
| 0060ee10 | `LoginManager::RegisterPacketHandlers` | UO_LOGIN | LoginManager::RegisterPacketHandlers | `regsite:0xA8` |
| 00644290 | `UONet::PacketHandlerMap_Register` | UO_NET | UONet::PacketHandlerMap_Register | `register:0xA8` |
| 0060c7a0 | `UOPacket::Read_0xA8_BritanniaList` | UO_LOGIN | UOPacket::Read_0xA8_BritanniaList | `call:h_a8:0` |
| 0060c3b0 | `UOPacket::Read_0x82_LoginFailed` | UO_LOGIN | UOPacket::Read_0x82_LoginFailed | `call:h_82:0` |
| 0060c6d0 | `UOPacket::Read_0x8C_UserServer` | UO_LOGIN | UOPacket::Read_0x8C_UserServer | `call:h_8c:0` |
| 0060ca50 | `UOPacket::Read_0xB9_Features` (probable) | UO_LOGIN | UOPacket::Read_0xB9_Features | `call:h_b9:0` |
| 0060c890 | `UOPacket::Read_0xA9_CitiesAndChars` | UO_LOGIN | UOPacket::Read_0xA9_CitiesAndChars | `call:h_a9:0` |
| 0060c5b0 | `UOPacket::Read_0x86_AllCharacters` | UO_LOGIN | UOPacket::Read_0x86_AllCharacters | `call:h_86:0` |
| 0060bee0 | `UOPacket::Read_0x1B_LoginConfirm` | UO_LOGIN | UOPacket::Read_0x1B_LoginConfirm | `call:h_1b:1` |
| 0060cac0 | `UOPacket::Read_0xC8_ViewRange` (probable) | UO_LOGIN | UOPacket::Read_0xC8_ViewRange | `call:h_c8:0` |
| 0060c050 | `UOPacket::Read_0x53_ErrorCode` (probable) | UO_LOGIN | UOPacket::Read_0x53_ErrorCode | `call:h_53:0` |
| 0060c4b0 | `UOPacket::Read_0x85_ChangeCharResult` | UO_LOGIN | UOPacket::Read_0x85_ChangeCharResult | `call:h_85:0` |
| 0060ccc0 | `UOPacket::Read_0xEE_Unknown` (probable) | UO_LOGIN | UOPacket::Read_0xEE_Unknown | `call:h_ee:0` |
| 0060cbc0 | `UOPacket::Read_0xE3_EncryptionRequest` (probable) | UO_LOGIN | UOPacket::Read_0xE3_EncryptionRequest | `call:h_e3:0` |
| 0060eb40 | `UOPacket::Build_0xE4_EncryptionResponse` | UO_LOGIN | UOPacket::Build_0xE4_EncryptionResponse | `call:h_e3:42` |
| 006105d0 | `LoginManager::Disconnect` | UO_LOGIN | LoginManager::Disconnect | `call:h_8c:2` |
| 00613e20 | `LoginManager::ConnectToGameServer` | UO_LOGIN | LoginManager::ConnectToGameServer | `call:h_8c:5` |
| 00612790 | `LoginManager::SendGameLogin` (probable) | UO_LOGIN | LoginManager::SendGameLogin | `call:connectGame:22` |
| 005fef40 | `UONet::Connection_AddReceiveTransform` (probable) | UO_NET | UONet::Connection_AddReceiveTransform | `call:connectGame:16` |
| 00639ea0 | `UOGameCipher_C2S_Encrypt::Init` | UO_NET | UOGameCipher_C2S_Encrypt::Init | `call:connectGame:10` |
| 0063be10 | `UOGameCipher_S2C_Decrypt::Init` | UO_NET | UOGameCipher_S2C_Decrypt::Init | `call:connectGame:15` |
| 0063bf30 | `UOHuffmanCoding_S2C_Decompress::UOHuffmanCoding_S2C_Decompress` | UO_NET | UOHuffmanCoding_S2C_Decompress::UOHuffmanCoding_S2C_Decompress | `call:connectGame:18` |
| 0063bd10 | `UOGameCipher_C2S_Encrypt::Transform` | UO_NET | UOGameCipher_C2S_Encrypt::Transform | `vtable:UOGameCipher_C2S_Encrypt:0` |
| 00639f90 | `UOGameCipher_C2S_Encrypt::EncryptByte` | UO_NET | UOGameCipher_C2S_Encrypt::EncryptByte | `call:gameC2STransform:1` |
| 0063be90 | `UOGameCipher_S2C_Decrypt::Transform` | UO_NET | UOGameCipher_S2C_Decrypt::Transform | `vtable:UOGameCipher_S2C_Decrypt:0` |
| 0063c410 | `UOHuffmanCoding_S2C_Decompress::Transform` | UO_NET | UOHuffmanCoding_S2C_Decompress::Transform | `vtable:UOHuffmanCoding_S2C_Decompress:0` |
| 0063c7b0 | `UOLoginCipher_C2S_Encrypt::Transform` | UO_NET | UOLoginCipher_C2S_Encrypt::Transform | `vtable:UOLoginCipher_C2S_Encrypt:0` |
| 0063a810 | `UOGameCipher2_C2S_Encrypt::Init` (probable) | UO_NET | UOGameCipher2_C2S_Encrypt::Init | `call:h_e3:55` |
| 0063b900 | `UOGameCipher2_S2C_Decrypt::Init` (probable) | UO_NET | UOGameCipher2_S2C_Decrypt::Init | `call:h_e3:60` |
| 006390b0 | `CryptoPPAdapter::Transform` | UO_NET | CryptoPPAdapter::Transform | `vtable:UOGameCipher2_C2S_Encrypt:0` |
| 005fd3c0 | `UONet::PacketRegistry_Find` | UO_NET | UONet::PacketRegistry_Find | `call:netMessageSend:1` |
| 005fd050 | `UONet::PacketRegistry_FindOrInsert` | UO_NET | UONet::PacketRegistry_FindOrInsert | `registry:registryFind` |
| 005f8630 | `UONet::PacketRegistry_RegisterAll` | UO_NET | UONet::PacketRegistry_RegisterAll | `busiest:registryInsert` |
| 005ff6d0 | `UONet::ExtractPacket` | UO_NET | UONet::ExtractPacket | `import:registryFind:ntohs` |
| 005ff5b0 | `BaseConnection::ProcessReceivedPackets` (probable) | UO_NET | BaseConnection::ProcessReceivedPackets | `caller:extractPacket` |
| 005ff1b0 | `BaseConnection::DispatchPacket` (probable) | UO_NET | BaseConnection::DispatchPacket | `call:processPackets:1` |
| 006442f0 | `UONet::PacketHandlerMap_Dispatch` (probable) | UO_NET | UONet::PacketHandlerMap_Dispatch | `call:dispatchPacket:0` |
| 005ff2d0 | `BaseConnection::OnReceive` (probable) | UO_NET | BaseConnection::OnReceive | `vtable:BaseConnection:5` |
| 00600720 | `GameServerConnection::FilterPacket` (probable) | UO_NET | GameServerConnection::FilterPacket | `vtable:GameServerConnection:11` |
| 00600800 | `GameServerConnection::OnDisconnected` (probable) | UO_NET | GameServerConnection::OnDisconnected | `vtable:GameServerConnection:7` |
| 00600ca0 | `LoginServerConnection::OnDisconnected` (probable) | UO_NET | LoginServerConnection::OnDisconnected | `vtable:LoginServerConnection:7` |
| 006052a0 | `LoginConfig::LoadLoginServers` (probable) | UO_LOGIN | LoginConfig::LoadLoginServers | `str:LoginServersDefault.xml` |
| 006054b0 | `LoginConfig::SetInfoForLogin` | UO_LOGIN | LoginConfig::SetInfoForLogin | `str:LoginConfig::SetInfoForLogin: Setting Login info for normal login` |
| 00e37304 | `g_LoginManager` | UO_LOGIN | g_LoginManager | `read:luaSelectShard:1` |
| 00e37338 | `g_LoginConfig` | UO_LOGIN | g_LoginConfig | `read:loginAccount:1` |
| 00e39954 | `g_LoginServerConnection` | UO_LOGIN | g_LoginServerConnection | `conn:0xA8` |
| 00e39958 | `g_LoginServerConnection_ctrl` | UO_LOGIN | g_LoginServerConnection_ctrl | `offset:g_LoginServerConnection:4` |
| 00e3994c | `g_GameServerConnection` | UO_LOGIN | g_GameServerConnection | `conn:0xA9` |
| 00e39950 | `g_GameServerConnection_ctrl` | UO_LOGIN | g_GameServerConnection_ctrl | `offset:g_GameServerConnection:4` |
| 00da987c | `g_ServerFeatureFlags` (probable) | UO_LOGIN | g_ServerFeatureFlags | `write:h_b9:0` |
| 00e33554 | `g_SelectedCharacterSlot` (probable) | UO_LOGIN | g_SelectedCharacterSlot | `write:luaSelectCharacter:0` |
| 00cb78cc | `g_DefaultLoginHost1` | UO_LOGIN | g_DefaultLoginHost1 | `eclo:ip1` |
| 00cb78dc | `g_DefaultLoginHost2` | UO_LOGIN | g_DefaultLoginHost2 | `eclo:ip2` |

### LoginManager packet handler registrations

| Site | Registering function | Packet | Connection global | Handler |
|---|---|---|---|---|
| 0060ee32 | LoginManager::RegisterPacketHandlers | 0xA8 | 00e39954 | 00611f10 UOPacket::Handle_0xA8_BritanniaList |
| 0060eee6 | LoginManager::RegisterPacketHandlers | 0x82 | 00e39954 | 00612590 UOPacket::Handle_0x82_LoginFailed |
| 0060ef9a | LoginManager::RegisterPacketHandlers | 0x8C | 00e39954 | 00612670 UOPacket::Handle_0x8C_UserServer |
| 0060f0de | LoginManager::RegisterPacketHandlers | 0x82 | 00e3994c | 00612590 UOPacket::Handle_0x82_LoginFailed |
| 0060f192 | LoginManager::RegisterPacketHandlers | 0xB9 | 00e3994c | 00612990 UOPacket::Handle_0xB9_Features |
| 0060f244 | LoginManager::RegisterPacketHandlers | 0xA9 | 00e3994c | 00612a80 UOPacket::Handle_0xA9_CitiesAndChars |
| 0060f2f6 | LoginManager::RegisterPacketHandlers | 0x86 | 00e3994c | 006130b0 UOPacket::Handle_0x86_AllCharacters |
| 0060f3a8 | LoginManager::RegisterPacketHandlers | 0x1B | 00e3994c | 00611950 UOPacket::Handle_0x1B_LoginConfirm |
| 0060f457 | LoginManager::RegisterPacketHandlers | 0xC8 | 00e3994c | 00612a10 UOPacket::Handle_0xC8_ViewRange |
| 0060f63d | LoginManager::RegisterPacketHandlers | 0x53 | 00e3994c | 00613430 UOPacket::Handle_0x53_ErrorCode |
| 0060f6ec | LoginManager::RegisterPacketHandlers | 0x85 | 00e3994c | 00615650 UOPacket::Handle_0x85_ChangeCharResult |
| 0060f79e | LoginManager::RegisterPacketHandlers | 0xEE | 00e3994c | 006133b0 UOPacket::Handle_0xEE_Unknown |
| 006144b2 | FUN_00614380 | 0xE3 | 00e3994c | 006145c0 UOPacket::Handle_0xE3_EncryptionRequest |

### Packet length table

251 ids registered ("variable": 2-byte big-endian length after the id). Registered twice (last wins): 0x3A, 0x3B, 0x66, 0x6F, 0x71, 0x98, 0xB8, 0xBD, 0xBE, 0xBF, 0xD6.

    0x00=106 0x01=5 0x02=7 0x03=variable 0x04=2 0x05=5 0x06=5 0x07=7 0x08=15 0x09=5 0x0A=11 0x0B=7 0x0C=variable 0x0D=3 0x0E=variable 0x0F=61
    0x10=215 0x11=variable 0x12=variable 0x13=10 0x14=6 0x15=9 0x16=variable 0x17=variable 0x18=variable 0x19=variable 0x1A=variable 0x1B=37 0x1C=variable 0x1D=5 0x1E=4 0x1F=8
    0x20=19 0x21=8 0x22=3 0x23=26 0x24=9 0x25=21 0x26=5 0x27=2 0x28=5 0x29=1 0x2A=5 0x2B=2 0x2C=2 0x2D=17 0x2E=15 0x2F=10
    0x30=5 0x31=variable 0x32=2 0x33=2 0x34=10 0x35=653 0x36=variable 0x37=8 0x38=7 0x39=9 0x3A=variable 0x3B=variable 0x3C=variable 0x3D=2 0x3E=37 0x3F=variable
    0x40=201 0x41=variable 0x42=variable 0x43=553 0x44=713 0x45=5 0x46=variable 0x47=11 0x48=73 0x49=93 0x4A=5 0x4B=9 0x4C=variable 0x4D=variable 0x4E=6 0x4F=2
    0x50=variable 0x51=variable 0x52=variable 0x53=2 0x54=12 0x55=1 0x56=11 0x57=110 0x58=106 0x59=variable 0x5A=variable 0x5B=4 0x5C=2 0x5D=73 0x5E=variable 0x5F=49
    0x60=5 0x61=9 0x62=15 0x63=13 0x64=1 0x65=4 0x66=variable 0x67=21 0x68=variable 0x69=variable 0x6A=3 0x6B=9 0x6C=19 0x6D=3 0x6E=14 0x6F=variable
    0x70=28 0x71=variable 0x72=5 0x73=2 0x74=variable 0x75=35 0x76=16 0x77=17 0x78=variable 0x79=9 0x7A=variable 0x7B=2 0x7C=variable 0x7D=13 0x7E=2 0x7F=variable
    0x80=62 0x81=variable 0x82=2 0x83=39 0x84=69 0x85=2 0x86=variable 0x87=variable 0x88=66 0x89=variable 0x8A=variable 0x8B=variable 0x8C=11 0x8D=variable 0x8E=variable 0x8F=variable
    0x90=19 0x91=65 0x92=variable 0x93=99 0x94=variable 0x95=9 0x96=variable 0x97=2 0x98=variable 0x99=30 0x9A=variable 0x9B=258 0x9C=309 0x9D=51 0x9E=variable 0x9F=variable
    0xA0=3 0xA1=9 0xA2=9 0xA3=9 0xA4=149 0xA5=variable 0xA6=variable 0xA7=4 0xA8=variable 0xA9=variable 0xAA=5 0xAB=variable 0xAC=variable 0xAD=variable 0xAE=variable 0xAF=13
    0xB0=variable 0xB1=variable 0xB2=variable 0xB3=variable 0xB4=variable 0xB5=64 0xB6=9 0xB7=variable 0xB8=variable 0xB9=5 0xBA=10 0xBB=9 0xBC=3 0xBD=variable 0xBE=variable 0xBF=variable
    0xC0=36 0xC1=variable 0xC2=variable 0xC3=variable 0xC4=6 0xC5=203 0xC6=1 0xC7=49 0xC8=2 0xC9=6 0xCA=6 0xCB=7 0xCC=variable 0xCD=1 0xCE=variable 0xCF=78
    0xD0=variable 0xD1=2 0xD2=25 0xD3=variable 0xD4=variable 0xD5=variable 0xD6=variable 0xD7=variable 0xD8=variable 0xD9=268 0xDA=variable 0xDB=variable 0xDC=9 0xDD=variable 0xDE=variable 0xDF=variable
    0xE0=variable 0xE1=variable 0xE2=10 0xE3=variable 0xE4=variable 0xE5=variable 0xE6=5 0xE7=12 0xE8=13 0xE9=75 0xEA=3 0xEB=variable 0xEC=variable 0xED=variable 0xEE=10 0xEF=21
    0xF0=variable 0xF1=9 0xF2=25 0xF3=26 0xF4=variable 0xF5=21 0xF6=variable 0xF7=variable 0xF9=variable 0xFA=1 0xFB=2

### ECLO patch sites

| Patch | Address | Function | Matches | Note |
|---|---|---|---|---|
| PORT1 | 00613612 | LoginManager::LoginAccount | 1 | ECLO port patch 1 rewrites the imm16 at 00613617 (default 7775): the port of the internal test address 172.16.17.41 (previous instruction, not patched by ECLO), used only when app +0x1c9 is set. |
| PORT2 | 006136bd | LoginManager::LoginAccount | 9 | ECLO port patch 2 (first port2 match after port1) rewrites the imm16 at 006136c2 (default 7775): the hardcoded-host port when rand() is odd. |
| PORT3 | 006136c6 | LoginManager::LoginAccount | 1 | ECLO port patch 3 (TOL clients) rewrites the imm16 at 006136cb (default 7776): the hardcoded-host port when rand() is even. |
| IP1 | 00cb78cc | - | 1 | ECLO IP patch 1 rewrites this 16-byte host slot (default "107.23.85.115"; host 1 is used when rand() is odd). Bypassed when LoginServers*.xml provides a LoginConfig. |
| IP2 | 00cb78dc | - | 1 | ECLO IP patch 2 rewrites this 16-byte host slot (default "107.23.176.74"; host 2 is used when rand() is even). Bypassed when LoginServers*.xml provides a LoginConfig. |
| ENC1 | 00639ff9 | UOGameCipher_C2S_Encrypt::EncryptByte | 1 | ECLO disable-encryption ENC1 (commented out in ECLO): NOP this XOR (game client-to-server Twofish pad XOR). |
| ENC2 | 0063becb | UOGameCipher_S2C_Decrypt::Transform | 1 | ECLO disable-encryption ENC2 (commented out in ECLO): NOP this XOR (game server-to-client digest XOR). |
| ENC3 | 0063c7da | UOLoginCipher_C2S_Encrypt::Transform | 1 | ECLO disable-encryption ENC3 (commented out in ECLO): NOP this XOR (login client-to-server key XOR). |

### Kept (not DEFAULT, not changed)


### Anomalies

