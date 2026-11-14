# Login and network names for Ghidra

`NameLoginCode.java` names the code in `UOSA.exe` that logs in to a shard, from the UO Lua
API down to the sockets. That covers `LoginManager`, the login packet handlers, the
packet readers and builders, the login and game ciphers, packet framing and the packet
length registry. It also labels the globals involved and comments the sites patched by
ECLO (the Enhanced Client login patcher).

| Path | Content |
|---|---|
| `NameLoginCode.java` | Ghidra script: report, and with `--apply` annotations |
| [Enhanced Client research](../../../../tools/uo-client-patcher-gui/docs/research/uosa_login.md) | Findings for the tested `UOSA.exe` (flow, packets, encryption, ECLO sites, every name) |
| [Kingdom Reborn research](../../../../tools/uo-client-patcher-gui/docs/research/uokr_login.md) | KR flow, RTTI classes, cipher and handshake evidence |
| [KR callback assembly](../../../../tools/uo-client-patcher-gui/docs/research/kr-plaintext-game-login.s) | Source for the reviewed plaintext game-login replacement |

## What it recovers

Each name comes from a locator, not from a fixed address. Locators are resolved in order,
and a locator may start from a function found by an earlier one.

- **Strings**: the one function that uses the one string starting with a given prefix.
  These are the client's own log lines, such as `LoginManager::LoginAccount: UI Login
  started` and `LuaTryLogin: logging in account`.
  - The UO Lua API functions are used only as anchors (another script names them).
  - Most `LoginManager` names copy the class and method names from these log lines.
- **Calls**: the target of the N-th direct call, in address order, inside a function
  found earlier. For example, `ConnectToLoginServer` is call 14 of `LoginAccount`.
- **Packet handler registrations**: the script scans every store of
  `UOClassCallback<shared_ptr<UONetMessageReceive>,LoginManager>::vftable`. A store
  builds a callback object `{vftable, this, handler}` and is followed by `PUSH id;
  LEA ECX,[conn+0x24]; CALL Register`. From each site the script reads:
  - the packet id;
  - the handler (stored at +8);
  - the connection global (the last `MOV reg,[abs]` of the `LEA` base register);
  - the register function.

  Handlers are named `UOPacket::Handle_0xNN_<Name>`, and their first call is the packet
  reader `UOPacket::Read_0xNN_<Name>`.
- **RTTI vtables**: a slot of `Class::vftable`. This gives the cipher, Huffman and
  connection methods.
- **Data accesses**: the N-th read or write of an absolute non-code, non-string address
  in a function. This is used for the globals.
- **Callers**: the only caller of a function. It can be narrowed to "other than X" or to
  "also calls import Y".
- **Packet registry**: the caller of `PacketRegistry_Find` that is itself called at least
  200 times from one function. That function is the table builder.
  - The script emulates the straight-line builder (stack slots and register constants) to
    print the length of every packet id.
- **ECLO byte patterns**: these come from ECLO's `patcher_address.cpp` (ports and IPs)
  and the commented-out patterns in `patcher_encryption.cpp`. Each site gets an EOL
  comment with the default value read from the program.

On the tested build (SHA-256 `5a5e9dc5...`), every located address is checked against the
address recorded in the script:

- A mismatch is reported and the name is skipped.
- A locator that fails falls back to the recorded address and is reported.

On other builds, only the locators are used.

## Running

In the GUI, add this folder to the Script Manager's script directories and run
`NameLoginCode.java` (category `ECResearch`) with arguments `OUTPUT_FILE [--apply]`.
Headless:

```sh
analyzeHeadless <project_dir> <project> -process UOSA.exe -noanalysis \
  -scriptPath scripts/dev-helpers/ec-research/ghidra-login-names \
  -postScript NameLoginCode.java /tmp/uosa_login_report.md --apply
```

The output file is created new, so the run fails without changing anything if the file
already exists. Without `--apply`, the run only writes the report.

## What `--apply` changes

- It renames functions whose name is still `DEFAULT` (`FUN_...`), with
  `SourceType.ANALYSIS`, into namespaces such as `LoginManager`, `UOPacket`, `UONet`,
  `UOLogin` and the RTTI classes.
  - Any other existing name is kept and listed under "Kept".
  - If the name already exists, an `_<address>` suffix is added.
- Tags: `UO_LOGIN` (login flow, handlers, globals) or `UO_NET` (connections, ciphers,
  framing, socket helpers).
- Plate comments are set when the plate is empty, or refreshed when the plate is an
  earlier `NameLoginCode:` comment. Each gives the evidence, says "Probable." for
  uncertain names, and names the locator used.
- Labels are created for globals whose primary symbol is `DEFAULT` (`DAT_...`, `s_...`),
  each with a plate comment if it has none.
- EOL comments are set at the ECLO patch sites when empty, or refreshed when they are an
  earlier `NameLoginCode:` comment. A different existing comment is kept and reported.

A second `--apply` changes nothing.

## Limitations

- Call ordinals depend on Ghidra's disassembly of the function. A build with different
  inlining needs new ordinals; the report shows which locator failed.
- The packet table emulation only follows the instruction forms used by this builder.
  Undecoded entries are counted under "Anomalies".
- Other managers (GumpManager, UOObjectManager, ...) register their packet handlers
  through per-class wrappers that take the packet id as an argument. They are out of scope.
- Twofish and MD5 primitives (`FUN_00a6990e`, `FUN_00a6a37e`, `FUN_009a876a`,
  `FUN_009a87fa`) are left unnamed (library code).

## Tested result

On `UOSA.exe` 4.0.102.3 (SHA-256
`5a5e9dc536c836d1bf10b6f86bb88f61a6ae078f519d4060d3bc47aecbebad50`), the first
`--apply` made these changes:

- 72 functions renamed;
- 72 tags and 72 plate comments;
- 5 namespaces created;
- 10 labels, each with a comment;
- 8 ECLO comments.

The second `--apply` changed nothing. There were no anomalies and nothing was kept. See
[the Enhanced Client report](../../../../tools/uo-client-patcher-gui/docs/research/uosa_login.md).
