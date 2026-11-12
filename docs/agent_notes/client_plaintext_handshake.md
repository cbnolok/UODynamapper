# KR plaintext game login

Removing KR's cipher transforms alone does not complete game login. The stock client sends `0xffffffff`,
waits exclusively for 0xE3, then installs both ciphers and Huffman and sends 0x91. A plaintext bypass must
install Huffman and advance the connection state as well as send 0x91. Use the relay auth key as the seed
to avoid selecting servers' fixed-seed KR exchange path.

`client::kr_game_login` is deliberately specific to the audited KR executable. It validates the complete
game-connect function and original VA before replacing its setup. Do not wildcard its relative calls or
absolute operands without also deriving their replacements. The constructor takes its object in ESI;
the normal 0x91 sender takes LoginManager in EAX. The typed shared-pointer helper and original cleanup
preserve ownership. The assembly and full evidence are in
[`ghidra-login-names/results/uokr_login.md`](../../scripts/dev-helpers/ec-research/ghidra-login-names/results/uokr_login.md).

The historical UoKRLoader, UO-SALoader and UOCH sources remove cipher operations or patch addresses;
none provides the missing KR handshake bypass. UoKRLoader also writes beyond its game-cipher signature.

## Independent controls

`ExeEdit::NoLoginEncryption`, `NoGameEncryption` and `BypassKrHandshake` map to CLI flags
`--no-login-encryption`, `--no-game-encryption` and `--bypass-kr-handshake`. The first two only change cipher
transforms. Game cipher removal keeps KR's E3 validation, E4 response and delayed Huffman/0x91 setup.
Handshake bypass instead installs Huffman and sends plaintext 0x91 immediately without creating a game cipher;
stock adapter bytes do not imply encryption is active on that path. It does not disable login encryption.
The existing `NoEncryption` / `--no-encryption` shortcut still selects all applicable changes. Scoped selections
must validate only their own sites. Preserve legacy `no_encryption` manifest edits and the compatibility check for
older KR copies whose two cipher sites are patched but handshake is stock.

UoKRLoader's combo is `Yes` / `Remove Login` / `Remove Game` / `Remove Both`, defaulting to `Yes` (no removal).
Its game choice is cipher-only; do not equate it with the handshake bypass. The active UO-SALoader patches only
addresses, with its cipher routines commented out.

## Login seed, master keys and KR game cipher

The seed is not the version-derived value. KR `0x5e80c0` gets the local hostname, resolves its first IPv4 address
and applies `ntohl`. `0x5a4520` sends that login seed in 0xEF with protocol version **66.55.52.0**, distinct from
the PE version **2.58.0.6**. `0x5de0e0` mixes the seed with the same constants as EC. Embedded master keys
`0x5b6b4d6c` / `0xd3d51816` exactly match the standard version formula for 66.55.52; the fourth component is
ignored. EC 67.0.102 matches `0x1b92eb9d` / `0xb91e227f`. This is a formula match, not evidence that clients
recompute keys at runtime. The formula is in the local `vCrypter/vCrypter.cs:59-74`.

KR's post-DH/SHA-256 Cipher2 filters are AES/CFB in both directions: `0x5dc400 -> 0x5dc620` has
`Rijndael::Enc` and `CFB_EncryptionTemplate` RTTI; `0x5dd2e0 -> 0x5dd370` has `CFB_DecryptionTemplate` RTTI.
The compiled but unused older game classes are the Twofish/MD5 pair. See the linked executable research for details.
