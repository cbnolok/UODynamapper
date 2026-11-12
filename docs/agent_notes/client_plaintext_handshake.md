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
