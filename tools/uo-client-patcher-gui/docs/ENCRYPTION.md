# Encryption and KR handshake

The login cipher, game ciphers and KR handshake have three independent controls in the GUI and CLI:

| CLI flag | Patch edit | Effect |
|---|---|---|
| `--no-login-encryption` | `ExeEdit::NoLoginEncryption` | Remove the login send cipher only; leave game setup alone |
| `--no-game-encryption` | `ExeEdit::NoGameEncryption` | Remove game cipher transforms only; retain KR's E3/E4 exchange |
| `--bypass-kr-handshake` | `ExeEdit::BypassKrHandshake` | KR only: immediate plaintext game login with Huffman, without E3/E4 |

Scoped flags may be combined. `ExeEdit::NoEncryption` (`exe patch --no-encryption`) retains its existing meaning:
remove login and game ciphers and bypass KR's handshake. It cannot be combined with the scoped flags.
All controls default off and only the selected sites must be located. Traced in
[the UOKR research](research/uokr_login.md),
which also lists every site of every build.

- **Login cipher** (both families): the login connection's client-to-server LFSR cipher, installed after the 0xEF
  seed. The patch writes NOPs over its `XOR [EAX],CL` (`30 08`); the keys still step. The login connection has no
  receive transform.
- **Game ciphers** (Enhanced Client only): the classic Twofish send cipher (`XOR [EBP+0],AL`, `30 45 00`) and MD5 pad
  receive cipher (`XOR [EAX],DL`, `30 10`), installed when the client connects to the game server; NOPs. Kingdom
  Reborn has the classes but never creates them.
- **0xE3 key exchange ciphers** (both): the Crypto++ pair the 0xE3 handler installs, behind one
  `CryptoPPAdapter::Transform`. The patch turns the `JZ` that enters its per-chunk loop into a `JMP` (`74` to `eb`),
  so messages pass through unchanged both ways. The audited KR and EC exchange filters use AES (Rijndael) in CFB mode,
  following Diffie-Hellman and SHA-256, rather than the older Twofish/MD5 pair. Removing their transforms alone
  leaves E3 validation, the plaintext E4 response and Huffman installation intact.
- **KR game handshake**: replace the fixed `0xffffffff` seed with the relay auth key, enter state 2, install
  Huffman and send 0x91 immediately. The client no longer waits for 0xE3 or sends 0xE4. This replacement is
  restricted to the four audited KR profiles below: each complete game-connect function must match at its
  original VA. Unsupported versions are refused rather than patched with guessed addresses.
- **EC normal route (4.0.102.3)**: the relay handler directly selects the auth-key seed, ready state 2,
  Twofish/MD5 and Huffman, then sends 0x91. Its compiled-in fixed-seed/E3 initializer has no callers or references;
  E3 is not registered by the normal route. The exchange handler clears and replaces both transform chains before
  AES/CFB; it does not add AES over Twofish/MD5. See [EC control-flow evidence](research/uosa_login.md#normal-game-login-versus-compiled-in-key-exchange).
- Huffman compression of server-to-client game traffic is left alone.

Each selected site is located by its instruction pattern and must match exactly once (stock or patched); otherwise the edit is
refused, naming the cipher and its match count. Classic is refused. `exe show` prints the state (on, removed or
partly removed) and each site; the manifest records it (`values.encryption`: `encrypted`, `removed`, `partial`).

With all controls selected, a server must read 0xEF / 0x80 and the game 0x91 unencrypted, and send game packets
Huffman-compressed, not encrypted. Patched Kingdom Reborn sends the relay auth key as the game seed and 0x91
immediately, with Huffman already installed; it needs no 0xE3 / 0xE4 support from the server. This also avoids
advertising KR's fixed `0xffffffff` seed to servers that use it to select the key exchange. Stock KR still requires
0xE3; stock ServUO does not send it. The Enhanced Client already sends the auth key and 0x91 immediately.

In KR, selecting only game cipher removal still requires a server that sends valid E3 and accepts E4. Selecting only
the KR handshake bypass installs no game ciphers: game traffic is plaintext even when the adapter's code remains
stock. Login encryption remains enabled unless its separate control is selected. The reported patch-site state
describes executable bytes, so a stock cipher site does not prove that this login path installs that cipher.

The login seed and the version-derived master keys are separate. In the audited KR build, `0x5e80c0` resolves
the local hostname with `gethostbyname`, takes its first IPv4 address and applies `ntohl`; `0x5a4520` sends that
seed in 0xEF with protocol version dwords **66, 55, 52, 0**. These are not the PE resource version **2.58.0.6**.
`0x5de0e0` mixes the seed into initial cipher state; the byte transform uses embedded master keys
`0x5b6b4d6c` and `0xd3d51816`. They exactly match the standard formula for protocol major/minor/third
`66.55.52`; the fourth component is unused. EC protocol `67.0.102` similarly yields `0x1b92eb9d` and
`0xb91e227f`. The match identifies the constants' derivation; the traced clients use constants embedded in code,
not a runtime calculation from their PE version. Formula source and instruction evidence are in the linked research.

| exe | login | game send | game receive | key exchange | KR handshake |
|---|---|---|---|---|---|
| Old KR/UOKR.exe | 0x1dd59a | - | - | 0x1da198 | 0x1b5f9f (458 bytes) |
| Enhanced/UOSA.exe 4.0.102.3, UOSA_eclo.exe | 0x23c7da | 0x239ff9 | 0x23becb | 0x2390e8 | - |
| Enhanced/UOSA_TC.exe 4.0.29.97 | 0x22ab6a | 0x228389 | 0x22a25b | 0x227478 | - |
| Enhanced fp/UOSA.exe 4.0.115.0 | 0x240daa | 0x23e5c9 | 0x24049b | 0x23d6b8 | - |

Earlier KR copies with only the two cipher patches inspect as partly removed. Reapplying `--no-encryption`
adds the handshake bypass; their hash-matching manifests remain usable when the original cipher sites are removed
and only the newly introduced handshake site is stock.

## Tools

- `uocf-cli exe show <exe>` lists every located patch site and its stock or patched bytes.
- `uocf-cli exe patch <exe> --output <copy> [--dry-run]` accepts the independent flags above, or the combined
  `--no-encryption` shortcut. The input is read only; the copy and its manifest record the selected changes.
- Login destinations are configured separately; see [Login server](LOGIN_SERVER.md).

## KR build coverage and upgrades

| PE version | Game-connect VA | Handshake patch file offset |
|---|---|---|
| 2.48.0.3 | `0x5baeb0` | `0x1ba30f` |
| 2.54.0.7 | `0x5babe0` | `0x1ba03f` |
| 2.57.0.17 | `0x5b6b60` | `0x1b5fbf` |
| 2.58.0.6 | `0x5b6b40` | `0x1b5f9f` |

Every profile checks all 655 bytes and its original VA, with exact per-build address operands and relative calls.
The replacement changes a 458-byte span and uses each build's own helpers. KR 2.45.4.4 uses an earlier compiler/SEH
layout and is not supported by these handshake profiles or by the current crypt-key extractor.

The older 81-byte callback omitted the EDI LoginManager argument on Huffman-allocation failure. The corrected
85-byte callback supplies it. Earlier fully patched 2.58 copies are recognized for an explicit handshake upgrade,
including their matching manifests; the other patches stay intact. This failure-path fix does not explain normal
login stalls.

## Display embedded login keys

```
uocf-cli exe show /path/to/UOKR.exe --crypt-keys
```

This reads the actual login transform immediates and 0xEF version instructions, supports stock and NOP-patched
login XORs, and compares the vCrypter version formula. It never substitutes the PE resource version or silently
guesses keys for unknown code. Missing/ambiguous version instructions are reported separately from real keys.
Login master keys are separate from the transmitted seed and the DH-derived game keys. Keeping login encryption
on does not enable Sphere to implement KR's AES game cipher; configure and test the game path separately.

For the audited KR files, the extracted wire versions and login keys can be written in Sphere's existing
`[SPHERECRYPT]` format as follows (leading zero denotes a hexadecimal key):

```
66554400 059B14CEC 0D2921816 ENC_LOGIN // KR resource 2.48.0.3; wire 66.55.44.0
66554800 05A8C6D2C 0D3375816 ENC_LOGIN // KR resource 2.54.0.7; wire 66.55.48.0
66555100 05B223F1C 0D3F6D416 ENC_LOGIN // KR resource 2.57.0.17; wire 66.55.51.0
66555200 05B6B4D6C 0D3D51816 ENC_LOGIN // KR resource 2.58.0.6; wire 66.55.52.0
```

These describe the login rotation cipher, not KR AES game encryption. The patcher does not edit the server's
crypt configuration. The source for the entries is the executable's instructions, independently cross-checked
with the version formula.

## Sphere packet-format compatibility

The inspected local Source-X source chooses extended B9 feature flags and A9 city entries by comparing saved
client versions against Classic thresholds. KR's reported protocol major 66 exceeds both thresholds. The audited
KR parser expects B9 length 3 and legacy 63-byte A9 city entries, whereas those branches send B9 length 5 and
extended entries. This is a concrete format mismatch after 0x91, separate from encryption or the handshake wait.
A live packet trace is still needed to tie it to a particular failed login; server source and running-binary
correspondence must also be checked before claiming a server fix is validated.
