# KR executable collection coverage

The supplied `/home/claudio/Scaricati/` collection contains 18 KR executables with versioned filenames. The original
Old KR `UOKR.exe` (2.58.0.6) is a nineteenth audited executable outside that folder. The inventory below
uses PE resource versions and decoded instructions, not filename-based version guesses. Originals are read only.

The complete encryption patch (including the independent E3 bypass) supports all supplied 2.48–2.57 builds
and the original 2.58.0.6. Their exact function addresses and executable hashes are recorded in
[the sibling-build table](uokr_login.md#audited-sibling-builds). Earlier builds remain partially supported;
`--no-encryption` refuses them rather than writing an unverified game-login replacement.

| Collection filename | PE resource version | Login address located | Login cipher located | Full encryption patch |
|---|---|---|---|---|
| `UOKR.2-45-3-13.exe` | 2.45.3.13 | no | no | no |
| `UOKR.2-45-4-4.exe` | 2.45.4.4 | no | no | no |
| `UOKR.2-45-5-6.exe` | 2.45.5.6 | no | no | no |
| `UOKR.2-45-6-3.exe` | 2.45.6.3 | no | no | no |
| `UOKR.2-46-0-3.exe` | 2.46.0.3 | no | yes | no |
| `UOKR.2-46-1-6.exe` | 2.46.1.6 | no | yes | no |
| `UOKR.2-46-1-9.exe` | 2.46.1.9 | no | yes | no |
| `UOKR.2-47-0-8.exe` | 2.47.0.6 | no | yes | no |
| `UOKR.2-47-1-10.exe` | 2.47.1.10 | yes | yes | no |
| `UOKR.2-47-1-6.exe` | 2.47.1.6 | yes | yes | no |
| `UOKR.2-48-0-3.exe` | 2.48.0.3 | yes | yes | yes |
| `UOKR.2-48-0-7.exe` | 2.48.0.7 | yes | yes | yes |
| `UOKR.2-50-0-3.exe` | 2.50.0.3 | yes | yes | yes |
| `UOKR.2-51-0-3.exe` | 2.51.0.3 | yes | yes | yes |
| `UOKR.2-52-0-4.exe` | 2.52.0.4 | yes | yes | yes |
| `UOKR.2-53-0-2.exe` | 2.53.0.2 | yes | yes | yes |
| `UOKR.2-54-0-7.exe` | 2.54.0.7 | yes | yes | yes |
| `UOKR.2-57-0-17.exe` | 2.57.0.17 | yes | yes | yes |

`UOKR.2-47-0-8.exe` contains resource version **2.47.0.6**; support and key reports must retain this distinction.
The located built-in `159.153.196.57:7775` address in 2.47.1.6 and 2.47.1.10 differs from the patcher's canonical
`.41` stock address and is therefore currently displayed as changed. That label alone does not establish that
the downloaded file was modified.

## Login keys and advertised versions

For the modern files, `uocf-cli exe show <exe> --crypt-keys` extracts the following values directly from
instructions. Every listed wire-version formula independently matches both actual keys.

| PE resource version | 0xEF wire version | Embedded key 1 | Embedded key 2 |
|---|---|---|---|
| 2.48.0.3 | 66.55.44.0 | `0x59b14cec` | `0xd2921816` |
| 2.48.0.7 | 66.55.44.0 | `0x59b14cec` | `0xd2921816` |
| 2.50.0.3 | 66.55.45.0 | `0x59c9befc` | `0xd2aa1416` |
| 2.51.0.3 | 66.55.45.0 | `0x59c9befc` | `0xd2aa1416` |
| 2.52.0.4 | 66.55.46.0 | `0x5a1ee4cc` | `0xd373a816` |
| 2.53.0.2 | 66.55.47.0 | `0x5a472edc` | `0xd35b3416` |
| 2.54.0.7 | 66.55.48.0 | `0x5a8c6d2c` | `0xd3375816` |
| 2.57.0.17 | 66.55.51.0 | `0x5b223f1c` | `0xd3f6d416` |
| 2.58.0.6 | 66.55.52.0 | `0x5b6b4d6c` | `0xd3d51816` |

All six supplied 2.46–2.47 executables embed the same login keys `0xd09c9589` and `0x1bd26d6b`;
the current CLI extracts these but reports zero supported version-writer matches. Direct tracing confirms
that all ten supplied pre-2.48 builds send a raw four-byte login seed, without a 0xEF version record on that
route. The CLI does not substitute a PE version or infer a wire version by reversing the key formula. Read-only disassembly also finds the same pair in
all four 2.45 login transforms, whose earlier instruction form the CLI currently refuses.

## Earlier-build boundaries

The six 2.46–2.47 game-connect functions share the 655-byte opcode layout but also differ at two relative-call
operands outside the current 21-field profile: `+0x5b` and `+0x1a9`, targeting seed/callback shared-pointer
helpers. Those calls still need a complete helper and replacement audit before adding safe profiles.

The four 2.45 builds use the older compiler/SEH login-transform layout: key 1 is loaded with `MOV EBX,imm32`
and key 2 enters `XOR ECX,imm32`. Their game-connect functions are 397 bytes with an aligned EBP frame and `FS:0` exception handling; modern
stack offsets and cleanup jumps cannot be reused. In 2.45.4.4, the traced login-seed setup at `0x4f90ff` creates a raw message,
appends only a four-byte seed through `0x442457` (`htonl`, then append four bytes), sends it through vtable
`+0x40`, and installs the login transform. This route sends no 0xEF version record. The corresponding seed routes were traced independently in all ten
early executables, rather than inferring packet behavior from a missing modern signature.

| PE version | Earlier login transform VA | Login byte XOR VA |
|---|---|---|
| 2.45.3.13 | `0x515c27` | `0x515c6b` |
| 2.45.4.4 | `0x51b7ce` | `0x51b812` |
| 2.45.5.6 | `0x52147c` | `0x5214c0` |
| 2.45.6.3 | `0x52ac65` | `0x52aca9` |

## Earlier raw-seed route evidence

Each setup below appends exactly one network-order seed DWORD, sends it raw through vtable `+0x40`, then
initializes and installs the login cipher. The identical compiled key pair does not establish how these older
builds originally selected their keys; no runtime formula or fallback selection was found in the traced loops.

| PE version | Seed setup VA | Raw-send call VA | Login-cipher init VA | Executable SHA-256 |
|---|---|---|---|---|
| 2.45.3.13 | `0x4f3844` | `0x4f38b7` | `0x515ba9` | `e94190c52febc13585a159f44f976686835ee4140197d0363a4d00ce6ab6e25b` |
| 2.45.4.4 | `0x4f90ff` | `0x4f9172` | `0x51b750` | `d2eb90b428d29d83a235fc7d553a816128d95c780e4707b0a19a2b4617e9363b` |
| 2.45.5.6 | `0x4fe645` | `0x4fe6b8` | `0x5213fe` | `9d7e8257d2e5e7309ec7a8e3fc86863db087ce18228e6ebb8b7589695555e0e5` |
| 2.45.6.3 | `0x507a8e` | `0x507b01` | `0x52abe7` | `aebb584d6bcf73ec8b7821606fbe11832d223b3554bfe94a57507a226a48833e` |
| 2.46.0.3 | `0x5983f0` | `0x5984a3` | `0x5cf6b0` | `619c458eb70175c7ca389e2c31415d262155d188d7388608eb86efd89140667c` |
| 2.46.1.6 | `0x59e980` | `0x59ea33` | `0x5d70b0` | `cc664cc8550bf40877b597889e893409924e3a42cf78fb177ba4f68fb19a76f7` |
| 2.46.1.9 | `0x59e980` | `0x59ea33` | `0x5d70b0` | `fb5c60cd6ea81969b30a54b1978ce47c8c7a1d200a736f321602c61517c051ec` |
| 2.47.0.6 | `0x59f120` | `0x59f1d3` | `0x5d7850` | `36ea9fadbbb2d4c8e1180b7845aaba74d3bf5e7bd654735d5f023de69ade3059` |
| 2.47.1.6 | `0x5a0ee0` | `0x5a0f93` | `0x5d99e0` | `3fde36e0ffa1920e5ac12c58e62d15647cb015b9b66ce1e31fe1d50678c2bd5b` |
| 2.47.1.10 | `0x5a0f50` | `0x5a1003` | `0x5d9a50` | `fc5eea0a9aa99eb3984cf7d24406799b19e82567861f44dd0385cabae44e7a4e` |

These are retained research results, not additional patch signatures. No source executable or Sphere
configuration was changed by this audit.
