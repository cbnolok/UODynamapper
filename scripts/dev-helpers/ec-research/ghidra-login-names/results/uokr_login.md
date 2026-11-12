# UOKR.exe login and game encryption

`UOKR.exe` (Old KR), SHA-256 `1b802e5c00f3bf89a771778b9c6c57406be0703808eeb4cd983e2f5baa348a43`. The build keeps
its RTTI, so the class names below are the vftable names; function names in `FUN_` form are Ghidra's, with a role
given here. Traced with read-only Ghidra probes, enough for the no-encryption patch
(`uocf::client::encryption`, `ExeEdit::NoEncryption`, `uocf-cli exe patch --no-encryption`). File offsets are in
`.text` (VA `0x401000`, raw `0x400`): file offset = VA - `0x400c00`.

## Transforms

The client sends and receives through per-connection transform chains (`boost::shared_ptr` transforms;
`FUN_005a2950` adds a send transform, `FUN_005a29e0` a receive transform). A transform's vtable has `Transform` in
slot 0 and the destructor in slot 1.

| Class | vftable | Transform | Installed by |
|---|---|---|---|
| `UOLoginCipher_C2S_Encrypt` | `0xb66104` | `0x5de170` | the login seed (`FUN_005a4520`) |
| `UOGameCipher_C2S_Encrypt` | `0xb660e0` | `0x5dd740` | nothing (dead code, see below) |
| `UOGameCipher_S2C_Decrypt` | `0xb660ec` | `0x5dd7f0` | nothing (dead code) |
| `UOHuffmanCoding_S2C_Decompress` | `0xb660f8` | `0x5dddc0` | the 0xE3 handler |
| `UOGameCipher2_C2S_Encrypt` | `0xb65e98` | `0x5dad60` (`CryptoPPAdapter::Transform`) | the 0xE3 handler |
| `UOGameCipher2_S2C_Decrypt` | `0xb66028` | `0x5dad60` (`CryptoPPAdapter::Transform`) | the 0xE3 handler |

The classic game ciphers are compiled in but never created: their constructors (`0x5dd6e0`, ...) have no callers
or references, and their `Transform` functions are referenced only from their vftables. The Enhanced Client's game cipher XOR
patterns (ECLO's ENC1 and ENC2) are not in this build.

## Login connection

1. `FUN_005b38e0` (connect to the login server) connects the login connection (connection vtable `+0x34`), sends
   the seed, then the 0x80 account login through the transforms (vtable `+0x38`).
2. `FUN_005a4520` (send the seed, install the login cipher) builds 0xEF (`FUN_005a4400` allocates it,
   `FUN_005a4360` writes the seed and four version dwords, each `htonl`), sends it raw (vtable `+0x40`, past the
   transforms), creates `UOLoginCipher_C2S_Encrypt`, initializes it (`FUN_005de0e0`) and adds it as the send
   transform.
3. `FUN_005de0e0` derives the initial two-word state from the seed with the classic constants (`0xffffaaaa`, `0x1357`,
   `0xabcdffff`, `0x43210000`), as EC does. This seed mixing is separate from the version-derived master keys.
4. `UOLoginCipher_C2S_Encrypt::Transform` (`0x5de170`) XORs each byte with the low byte of key 1, then steps the
   keys (the KR constants `0x5b6b4d6c` twice and `0xd3d51816`; the Enhanced Client has `0x1b92eb9c`/`0x1b92eb9d`
   and `0xb91e227f`):

   ```
   5de190  53 50 e8 rel32      PUSH EBX; PUSH EAX; CALL (the message byte)
   5de197  8a 4e 04            MOV CL,[ESI+4]
   5de19a  30 08               XOR [EAX],CL          <- patched to 90 90
   5de19c  8b 4e 04 8b 46 08   MOV ECX,[ESI+4]; MOV EAX,[ESI+8]
   5de1a2  8b d0 d1 e8 8b f9 c1 e7 1f 0b c7 35 imm32   the key step
   ```

The login connection has no receive transform: the server's login replies are plaintext and uncompressed.

### Seed versus version-derived keys

Read-only Ghidra tracing identifies `FUN_005e80c0` as the login-seed source: get the local hostname, resolve it
with `gethostbyname`, take the first IPv4 address and apply `ntohl`. `FUN_005a4520` passes that seed and the
four dwords **66, 55, 52, 0** to the 0xEF writer. Thus the wire protocol version is **66.55.52.0**; the PE resource
version **2.58.0.6** is a separate identifier. Changing the login destination does not make that hostname-derived
seed a version constant. The game connection's fixed `0xffffffff` seed is a third, separate value.

The master-key formula is preserved in the local source
`/mnt/dati/__cloud/mega/_proj/_UOTools/__nonmio/vCrypter-master (src)/vCrypter/vCrypter.cs:59-74`.
For unsigned 32-bit arithmetic, with protocol major `a`, minor `b` and third component `c`, it is:

```
k1 = ((a << 23) | (b << 14) | (c << 4))
     ^ ((c * c) << 9) ^ (b * b) ^ ((b * 11) << 24) ^ ((c * 7) << 19) ^ 0x2c13a5fd
k2 = ((a << 22) | (c << 13) | (b << 3))
     ^ ((c * c * 3) << 10) ^ (b * b) ^ ((b * 13) << 23) ^ ((c * 7) << 18) ^ 0xa31d527f
```

The fourth version component is read by that tool but does not enter these expressions. Recalculation gives:

| Protocol major/minor/third | Formula k1 | Formula k2 |
|---|---|---|
| KR 66.55.52 | `0x5b6b4d6c` | `0xd3d51816` |
| EC 67.0.102 | `0x1b92eb9d` | `0xb91e227f` |

These exactly match the embedded master keys. KR's step uses `0x5b6b4d6c` in both relevant operations; EC's
step contains both `0x1b92eb9c` and `0x1b92eb9d`. This proves the formula match for these builds, not runtime
recomputation: the traced transforms use immediates embedded in their machine code.

## Game connection and the 0xE3 / 0xE4 key exchange

1. `FUN_005b58b0` (handles 0x8C, the game server relay) disconnects the login connection, adds the port offset at
   `+0xac`, stores the auth key at `+0x58` (the local IPv4 when it is 0) and connects the game connection
   (vtable `+0x34`). When the byte at `+0x5c` is 0 it calls `FUN_005b6b40`.
2. `FUN_005b6b40` sends a raw 4-byte seed `htonl(0xffffffff)` (vtable `+0x40`; not the auth key, unlike the Enhanced
   Client), sets the game connection state (`+0x80`) to 1 and registers the 0xE3 handler `FUN_005b6e10`
   (`FUN_005e5aa0(0xe3, ...)`). No game cipher is installed yet.
3. `FUN_005a41c0`, the game connection's packet filter (`GameServerConnection` vtable `0xb636ac`, slot 11): in state
   1 only 0xE3 passes; in state 2 everything passes; anything else disconnects both connections
   (`FUN_005b3b90(1)`).
4. `FUN_005b6e10` (handles 0xE3): reads the server's Diffie-Hellman parameters and public value (Crypto++
   `DH_Domain`, SHA-256, `AutoSeededRandomPool`) and validates them; on failure it disconnects (`Disconnect(1)`)
   and reports an error. Otherwise it sends 0xE4 with the client's public value (vtable `+0x38`; no send transform
   yet, so 0xE4 is plaintext), sets the state to 2, then installs, in this order:
   - `UOGameCipher2_C2S_Encrypt` (`::Init` `FUN_005dc400`; added as the send transform at `0x5b7587`);
   - `UOGameCipher2_S2C_Decrypt` (`::Init` `FUN_005dd2e0`; receive transform);
   - `UOHuffmanCoding_S2C_Decompress` (constructor `FUN_005dd850`; receive transform after the cipher).
   Then `FUN_005b59d0` sends 0x91 (the auth key from `+0x58`, account and password) through the new send cipher.

The Cipher2 pair is **AES (Rijndael) in CFB mode**, following the DH/SHA-256 exchange. The send initializer
`0x5dc400` calls `0x5dc620`, whose RTTI identifies `Rijndael::Enc` and `CFB_EncryptionTemplate`. The receive
initializer `0x5dd2e0` calls `0x5dd370`, identified by `CFB_DecryptionTemplate`. These filters are distinct from
the compiled but unused older `UOGameCipher` Twofish/MD5 classes. CFB uses the underlying AES encryption
primitive in both feedback directions; the receive filter's decryption role does not imply AES block decryption.

So Kingdom Reborn sends 0x91 only after a valid 0xE3. Until then it waits in state 1, and any other packet
disconnects it. No timeout was traced. The Huffman decompression is installed by the 0xE3 handler too, so before
0xE3 the server's bytes are not decompressed.

### `CryptoPPAdapter::Transform` (`0x5dad60`)

Both Cipher2 classes have this function in slot 0. The `CryptoPPAdapter` vftable (`0xb65c04`) is used only by
these two classes. The function reads the message's chunk range, then loops over the chunks and runs the Crypto++
filter on each in place (filter vtable `+0x1c`, `ProcessData(out = data, in = data, length)`). It returns the same
message. The loop is entered by a `JZ` that skips it for an empty range:

```
5dad60  55 8b ec 83 e4 f8 83 ec 44 8b 45 0c 53 56 57 83 c0 04 50 8d 7c 24 28 89 4c 24 10 e8 rel32
5dad80  8b 7c 24 2c 3b 7c 24 3c            MOV EDI,[ESP+2C]; CMP EDI,[ESP+3C]
5dad88  8b 4c 24 34 8b 54 24 38 89 4c 24 1c 89 54 24 20
5dad98  74 5d                              JZ 5dadf7 (done)   <- patched to EB 5D (JMP)
5dad9a  8b 37 8b 44 24 0c 8b 40 04 8b 18 8b 16 89 44 24 10 8b 42 04 8b ce 83 c3 1c ff d0 ...
```

As a `JMP`, every message passes through unchanged in both directions: the key exchange still runs (0xE3 is
validated, 0xE4 is sent, the state becomes 2), but nothing is enciphered or deciphered afterwards.

## Compression

`UOHuffmanCoding_S2C_Decompress` (Transform `0x5dddc0`) decompresses server-to-client game traffic with the
classic Huffman tree. The patch leaves it alone: the server still compresses its game packets.

## The plaintext patch

The controls are independent:

| Flag / edit | Patched sites | KR behavior |
|---|---|---|
| `--no-login-encryption` / `NoLoginEncryption` | Login XOR | Unencrypted account login; game setup unchanged |
| `--no-game-encryption` / `NoGameEncryption` | Game cipher transforms | Still waits for valid E3, sends E4, then plaintext 0x91 with Huffman |
| `--bypass-kr-handshake` / `BypassKrHandshake` | Game-connect replacement | Auth-key seed, state 2, Huffman, immediate plaintext 0x91; no game cipher created |
| `--no-encryption` / `NoEncryption` | All applicable sites | Plaintext login and immediate plaintext game login |

The scoped flags may combine; the all-in-one flag conflicts with them. Defaults leave every site unchanged.
The bypass alone leaves login encryption enabled. It also leaves the Crypto++ adapter's bytes unchanged, but
that game path never installs its ciphers, so its game traffic is still plaintext. Conversely, removing the adapter
alone preserves the handshake and still requires an E3-capable server. Inspection's aggregate state describes
patched sites, not whether a cipher is instantiated on a particular runtime path. The existing serialized
`no_encryption` edit and older KR manifests remain readable.

| File offset | VA | Stock | Patched | Cipher |
|---|---|---|---|---|
| `0x1dd59a` | `0x5de19a` | `30 08` | `90 90` | login, client to server |
| `0x1da198` | `0x5dad98` | `74` | `eb` | 0xE3 key exchange, both directions |

These cipher sites alone leave the client waiting for 0xE3. The complete plaintext patch also replaces the
game connection setup in `FUN_005b6b40`:

| File range | VA range | Change |
|---|---|---|
| `0x1b5f9f..0x1b5fb1` | `0x5b6b9f..0x5b6bb1` | Replace the no-op boost hook and `htonl(-1)` with `htonl(LoginManager.auth_key)` |
| `0x1b605d` | `0x5b6c5d` | State immediate 1 becomes 2 |
| `0x1b608d..0x1b6169` | `0x5b6c8d..0x5b6d69` | Replace E3 callback construction/registration with Huffman construction, receive-transform installation and immediate 0x91 |

The patcher records this as one 458-byte site starting at `0x1b5f9f`. Unchanged instructions within that range
are preserved. The **entire 655-byte function** (through `RET 4`, ending exclusively at VA `0x5b6dcf`) must
match its stock or patched form at the original VA; calls and absolute operands are specific to this build.
Unknown changes, mixed handshake replacements and a relocated function are refused. Cipher patterns still
accept their stock or patched form and require exactly one match.

The seed replacement reads the original stack argument `[ESP+0x24]`, then its auth key at `+0x58`. Its outgoing
four bytes still pass through the original `htonl` and raw send. Using that auth key, as EC does, avoids the
fixed `0xffffffff` seed that servers can use to select the KR key exchange.

The callback replacement is reproduced by [`../kr-plaintext-game-login.s`](../kr-plaintext-game-login.s).
It allocates the same 32-byte Huffman object and calls the original constructor (`0x5dd850`, object in ESI),
creates its correctly typed shared pointer (`0x5bca20`), and calls `AddReceiveTransform` (`0x5a29e0`). The
by-value shared pointer consumes one reference; EDI retains the local reference for the original cleanup at
`0x5b6d69`. EBX retains the seed message reference, released by the original cleanup at `0x5b6d97`. No
Crypto++ cipher or E3 callback is installed. `0x5b59d0` receives the original LoginManager in EAX and sends 0x91.
A failed Huffman allocation calls `Disconnect(1)` and releases the seed message without sending 0x91.

Assemble the callback for review with GNU binutils:

```sh
as --32 kr-plaintext-game-login.s -o /tmp/kr-callback.o
ld -m elf_i386 -Ttext=0x5b6c8d -e _start /tmp/kr-callback.o -o /tmp/kr-callback.elf
objdump -d -Mintel /tmp/kr-callback.elf
```

With the complete patch, a server reads plaintext 0xEF / 0x80, the relay auth key as the game seed, and
plaintext 0x91 immediately. It sends Huffman-compressed, unencrypted game packets. No E3/E4 support is required.
This removes the stock KR handshake obstacle on ServUO; it does not establish compatibility with every other
KR protocol feature. Sphere's fixed E3 exchange is no longer required for this login path.

### Historical loader comparison

The supplied `uokrloader/UoKRLoader/Utility.cs` (lines 188–272) NOPs the login XOR and zeroes the game cipher's
output. It does not bypass E3, install Huffman early or send 0x91 early. Its game signature covers 50 bytes but
the replacement writes 89; the replacement must not be copied without validating its full extent.
`UO-SALoader/Form1.cs` actively patches login addresses only; its encryption patches in `StaticData.cs` are
commented out. `UOCH_2.2-master/Main.c` patches Classic login, Blowfish/Twofish and MD5 ciphers; it has no KR
handshake implementation. None supplies an E3/E4 bypass.

### Validation

The actual patched 655-byte function was executed under `qemu-i386` with stubs for the client APIs. Normal
login, a null connection control block, and failed Huffman allocation passed seed byte-order, state, call-order,
stack, nonvolatile-register and shared-pointer lifetime checks. The CLI-written executable contains exactly the
executed function (SHA-256 `9caf512831decaafb0ab67803693abf5c302105567d1fc6aade3af19ca49895e`).
This checks the x86 flow and calling conventions; a live Sphere login has not been tested.

The encryption pattern tests, patch/legacy-manifest tests, GUI toggle test and real KR/EC encryption regression
passed, as did `cargo check --workspace`. The real-client regression only patches temporary copies.

## Enhanced Client (UOSA.exe) for comparison

`LoginManager::ConnectToGameServer` (UOSA 4.0.102.3 `0x613e20`) sends the auth key as the game seed, sets the state
to 2 at once and installs the classic `UOGameCipher_C2S_Encrypt` (Twofish), `UOGameCipher_S2C_Decrypt` (MD5 pad)
and Huffman, then sends 0x91. 0xE3 is optional there; when a server sends it, the Cipher2 pair is added on top. The
no-encryption patch therefore writes four sites in the Enhanced Client: the login XOR (ENC3), the game send XOR
(ENC1, `30 45 00`), the game receive XOR (ENC2, `30 10`) and the same adapter `JZ`. Patched bytes, by file offset:

| exe | login | game send | game receive | key exchange |
|---|---|---|---|---|
| Old KR/UOKR.exe | `0x1dd59a` | - | - | `0x1da198` |
| Enhanced/UOSA.exe 4.0.102.3 (and UOSA_eclo.exe) | `0x23c7da` | `0x239ff9` | `0x23becb` | `0x2390e8` |
| Enhanced/UOSA_TC.exe 4.0.29.97 | `0x22ab6a` | `0x228389` | `0x22a25b` | `0x227478` |
| Enhanced fp/UOSA.exe 4.0.115.0 | `0x240daa` | `0x23e5c9` | `0x24049b` | `0x23d6b8` |

Each pattern matches exactly once in each build.
