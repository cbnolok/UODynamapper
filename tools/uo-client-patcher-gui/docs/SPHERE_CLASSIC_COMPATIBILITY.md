# Sphere compatibility with early Classic clients

This is a source-backed handoff for the early `client_1.*` executables examined on
2026-11-15. The inspected Sphere checkout is `/mnt/dati/_proj_local/_Source-X`,
with the rotation changes committed as **`94534efe`**, `Fix early Classic rotation
encryption compatibility`. This document describes that implementation and the
remaining work separately. No further server or client support is implemented by
this documentation phase.

Read [the cipher evidence](research/classic-rotation-cipher.md) for executable
identities, keys, dynamic patch acceptance and executed cipher checks. Read
[the native login protocol](research/early-classic-login.md) for the old clients'
packet writers, receive handlers and structures. The latter matters because the
supplied `1.23` sends a **two-byte** game continuation, whereas the supplied
original `1.25` sends a 65-byte continuation despite sharing its cipher keys.
Both bare clients also have an active server-to-client left-rotation cipher and
an uncompressed receive path. Sphere's current early-client output and the
current client send-only encryption patch do not address those requirements.
Successful outgoing-cipher tests do not establish a complete Sphere login for
either bare build.

## What is already committed

Sphere already had `ENC_LOGIN`, which applies its login rotation cipher to
client-to-server game traffic as well as account login. Commit `94534efe` makes
four narrowly scoped corrections:

| Change | Reason | Evidence boundary |
| --- | --- | --- |
| Mask both 1.25.36 variable shift counts with `0x1f` | The actual x86 `SHR r32,CL` uses five count bits; replacing a shift count of 32 or more with a zero result changes the stream | Original cipher functions executed under `qemu-i386`; Sphere API tests fail against the previous implementation |
| Add the original fixed key pair as `1230000 ... ENC_LOGIN` | The supplied bare `1.23` and `1.25` use keys absent from the previous table | Both executable cipher bodies contain the pair; the alias identifies the cipher, not a unique wire protocol |
| Permit `inLen == outLen` in plaintext login copying | A buffer exactly large enough is valid | Public-API regression with a 62-byte input and output |
| Suppress the login-stage 0xBD request for a recognized version through 1.26.4 | These supplied old receive dispatchers do not handle 0xBD; an existing game-stage gate did not protect the earlier send | Client disassembly and server control flow; no live packet-order validation |

The patch is preserved as [sphere-classic-rotation.patch](research/sphere-classic-rotation.patch).
It changes `CCryptoLogin.cpp`, `CClientLog.cpp`, the source `sphereCrypt.ini` and
the test registration/fixtures. It does not add old login packet writers, a
protocol hint, support for a two-byte 0x91, or a new authentication mechanism.
It was not installed into the running server's binary or configuration.

The earlier work is visible in Source-X commit `337188576e20` from 2018. That
commit introduced rotation game detection, special handling for old feature
flags and a game-stage version-request gate. Its message reports testing
1.25.35 until after character selection and explicitly notes unfinished packet
sequencing. This is useful provenance, not a current interactive-login result.

## Keys, seed and version identity are different inputs

`CCryptoKeysHolder::LoadKeyTable` loads rows from `[SPHERECRYPT]` in the form:

```text
<legacy client number> <master high> <master low> <encryption type>
1230000 0F1A372D5 03A1FD527 ENC_LOGIN
```

The version field is parsed as an unsigned 32-bit number. Key literals use
Sphere's leading-zero hexadecimal notation, and `ENC_LOGIN` resolves to type 4.
The row's number is the identity Sphere assigns when that candidate decrypts a
plausible first packet. It also affects algorithm selection in `DecryptLogin`:
the single rotation is used below 1.25.36, the multikey step at exactly 1.25.36,
and the later step at 1.25.37 or above. A row therefore cannot be assigned an
arbitrary version number solely to make protocol gates convenient.

| Supplied client labels | Master high | Master low | Current table identity |
| --- | --- | --- | --- |
| `1.23`, `1.25` | `f1a372d5` | `3a1fd527` | `1230000`, shared cipher alias |
| `1.25.34` | `38ecedac` | `025720c6` | `1253400` |
| `1.25.35`, `1.25.35g/h/i` | `383477bc` | `02345cc6` | `1253500`, shared key row |
| `1.25.36` | `387fc5cc` | `021510c6` | `1253600`, multikey algorithm |

The existing formula-derived `1250000` row remains in the table. It does not
describe the supplied executable labelled `1.25`; retaining it avoids removing
an existing candidate without evidence about every other build. Generating keys
from the later version formula is not a substitute for extracting these older
executables' actual operands. The extra 1.25.36 update constants are implemented
by the algorithm, not six independent fields in `sphereCrypt.ini`.

The connection seed is a separate, cleartext 32-bit value. The clients initialize
their masks from its native numeric value and transmit `htonl(seed)` as four raw
bytes. Sphere's network `readInt32` recovers that same numeric value. On a normal
new game connection, the relay auth ID becomes the new seed. It is not a master
key and does not have to be derived from the executable version. Sphere's network
layer rejects a zero seed even though the isolated cipher has valid test vectors
for seed zero.

The initial outgoing masks are computed with unsigned 32-bit wrapping arithmetic:

```text
lo = (((~seed) ^ 0x00001357) << 16) | ((seed ^ 0xffffaaaa) & 0x0000ffff)
hi = ((seed ^ 0x43210000) >> 16) | (((~seed) ^ 0xabcdffff) & 0xffff0000)
```

This confirms the existing seed formula and byte order. No seed replacement or
version-based seed guess was part of `94534efe`.

## Current setup detection and framing limits

The network code consumes the raw seed before `CClient::xProcessClientSetup`
passes a setup buffer to `CCrypto::Init`. For a non-KR connection, that function
classifies the **entire supplied buffer length**, not its decrypted packet ID:

| Remaining setup length | Current path | Candidate checks |
| --- | --- | --- |
| Exactly 62 bytes | `LoginCryptStart` | Decrypted byte 0 is 0x80; bytes 30 and 60 are zero; login/password tail padding at 21–30 and 51–60 is zero; account-name sanitation agrees with its length |
| Exactly 65 bytes | `GameCryptStart` | Decrypted byte 0 is 0x91 and bytes 34 and 64 are zero |
| Other lengths | No Classic setup classification | The separate KR branch is irrelevant to these clients |

`LoginCryptStart` tries table rows in order, including the automatically inserted
no-crypt row at index zero. Each trial starts with the same initial masks. On a
match, it restores those masks so `xProcessClientSetup` can decrypt the real
packet once. If no row matches, it falls back to no crypt and lets later setup
processing reject or interpret the bytes. `CCrypto::Init` currently does not
propagate the boolean result of the individual detector calls; its successful
length classification is not proof that credentials or the protocol are valid.

`GameCryptStart` first tries no crypt, Blowfish, Blowfish plus Twofish, and
Twofish. If those fail, it tries table rows explicitly marked `ENC_LOGIN`, again
resetting the rotation masks before every trial. On a successful old-cipher
trial, it restores the initial masks before the actual packet decryption. The
three-byte game sanity check is weaker than the login padding/name checks.
Neither path proves the authenticity or exact release of an executable.

These exact-length conditions expose two distinct future tasks:

1. Initial Classic setup needs bounded framing across TCP fragmentation and
   coalescing. A partial first packet or a first packet followed immediately by
   another packet must not be confused with a different protocol length.
2. Native `1.23` needs a different game continuation path. Its actual writer
   constructs `91 00`, not a 65-byte credential packet. Both the seeded unknown
   network path and `processOtherClientData` have a less-than-five-byte
   ping/other-client branch; even if those were bypassed, `CCrypto::Init` and
   `PacketCharListReq` still assume the 65-byte form.

The second task also needs authentication design. A fresh two-byte continuation
does not carry the account name/password fields that
`xProcessClientSetup` currently reads to find the account and validate its relay
auth ID. Simply accepting length two, or padding it to 65, would not provide the
missing account/session association. How the native earlier exchange establishes
that association must be traced and specified before implementing it.

The existing `addRelay` path uses `7f000001` when `UseAuthID` is disabled. With
`UseAuthID` enabled it derives a CRC32 from server/account names and saves it as
the account's `customerid` tag. The current 65-byte setup first obtains the
account from its credential fields, then checks that account's expected auth ID
and consumes its saved tags. There is no corresponding fresh two-byte
account-lookup path here; the default seed is not even unique to an account.
A retained authenticated socket already has different available context from a
fresh socket. Future work must specify each case rather than assume a raw seed
alone supplies the missing authenticated identity.

## New game connections and retained sockets

For the supported 65-byte fresh game form, a new `CCrypto` instance detects the
game cipher using the new seed and begins at the initial masks. This matches
the supplied original `1.25` and later observed 65-byte writers at the cipher
lifecycle level; it does not make their server packet vocabularies identical.
It does not validate the native two-byte `1.23` continuation.

The retained-socket path behaves differently:

- `PacketServerRelay::onSent` calls `InitFast(customerId, CONNECT_GAME)` and
  changes the client connection type.
- `InitFast` stores the new seed, initializes the later game-cipher helpers and
  sets a relay flag, but does not reinitialize the rotation masks.
- `Decrypt` handles `ENC_LOGIN` before its relay/double-encryption branch. It
  continues the existing rotation stream and returns immediately.

Consequently, an already connected old client must not have its masks reset at
shard selection. The tests exercise this retained-stream behavior. The normal
relay/new-connection behavior, packet parsing and account binding remain separate
validation targets; these API tests are not a socket-level capture.

## Encryption direction and compression

The same evolving outgoing rotation stream protects **client-to-server**
login/game bytes in these executables. Their incoming handling divides into two
verified code families:

| Supplied clients | Native server-to-client receive path | Current Sphere compatibility |
| --- | --- | --- |
| Bare `1.23` and original `1.25` | An independent left-rotation XOR, followed by direct packet framing without Huffman decompression | Not implemented by Sphere's `ENC_LOGIN` outbound path; the current client send-only patch also leaves this incoming cipher active |
| `1.25.34`, `1.25.35/g/h/i`, `1.25.36` | No active incoming XOR on the traced path; the later game receive path supports Huffman | Consistent with Sphere's output model at this layer, subject to native framing, phase selection and packet-order verification |

For the bare clients, separate receive masks at connection offsets
`0x20018`/`0x2001c` start as `rxLo = seed ^ a8612ae5` and
`rxHi = seed ^ 044cc16d`. A received byte is XORed with the old low byte of
`rxLo`, then both next masks are calculated from the old pair:

```text
next_rxLo = ((rxLo << 1) | (rxHi >> 31)) ^ 0xaf572d13
next_rxHi = ((rxHi << 1) | (rxLo >> 31)) ^ 0xfa7d5231
```

This is a distinct directional state and key schedule, not the existing
client-to-server right-rotation stream run in reverse. These incoming findings
are supported by static instruction/data-flow analysis; they have not received
the executed vector coverage of the outgoing cipher. Exact addresses and seed
initialization evidence are in [the incoming-cipher research](research/classic-rotation-cipher.md).

The **current Sphere implementation** handles the directions as follows:

| Direction and phase | Current early-client handling |
| --- | --- |
| Client to login server | Rotation, or plaintext with `ENC_NONE` |
| Client to game server | Rotation with `ENC_LOGIN`, or plaintext with `ENC_NONE` |
| Login server to client | Uncompressed plaintext packet bytes |
| Game server to client | Huffman-compressed packet bytes, without rotation encryption |

`CNetworkOutput::sendPacketData` selects compression from `CONNECT_GAME`, not
from whether client encryption is enabled. Its extra outbound crypto call is
only for `ENC_TFISH`; that later algorithm is outside this early-client evidence.
In game mode `CCrypto::Encrypt` leaves `ENC_LOGIN` bytes unchanged, and a
regression checks that doing so does not advance the client-to-server masks. That test establishes
current server behavior; it does **not** validate the native bare clients'
incoming requirements. The existing `ENC_LOGIN` outbound path has no encoder for
their left-rotation stream, and the network layer would add Huffman in game mode
although their traced readers frame packets directly.

The current client `NoEncryption` patch disables the outgoing XOR in the bare
builds but leaves their incoming XOR and enable flag active. It therefore does
not yet make their transport fully plaintext. Future server support must either
supply the native incoming stream, or a separately designed and verified client
patch must bypass that incoming cipher too. Cipher direction and compression
must be selected independently; `ENC_LOGIN` alone is insufficient to describe
these two eras.

The current patcher's `removed (patched: plaintext)` state is calculated from
its located sites, which for these early builds currently cover the outgoing
routine. For the two bare builds, that report is therefore not proof that the
newly identified incoming cipher has been removed. This documentation records
the gap without changing the patcher or its manifest format.

## Current packet writers versus early native expectations

These are the inspected Sphere writers, not an implemented compatibility plan:

| Sphere output | Current layout/selection | Early-client gap |
| --- | --- | --- |
| `PacketServerList`, 0xA8 | ID, big-endian length, `ff`, big-endian count; 40-byte entries: index(2), name(32), fullness(1), time zone(1), address(4) | Bare `1.23` and `1.25` receive dispatch does not reach 0xA8. Their native earlier exchange must be used or explicitly replaced by a separately designed flow |
| `PacketEnableFeatures`, 0xB9 | `Setup_ListReq` always queues it; recognized crypt version below 1.26 gets flags `0003`. Width is normally 2 bytes, extended for later clients with the KR exception | Bare `1.23` and `1.25` cannot handle 0xB9 at all; changing the flag value or width cannot solve that |
| `PacketCharacterList`, 0xA9 | Count and 60-byte character records, start-location count, old or extended start records, and conditional later suffixes | Bare `1.23` and `1.25` expect the earlier 0x81/0x86 family, with different framing and additional data in the initial response |
| `PacketClientVersionReq`, 0xBD | Login probe suppressed only for recognized old versions; unknown versions still receive it | A plaintext patched old client is still unknown and cannot answer it |

The exact native 0x81/0x86 field maps and address/credential handling belong in
[early-classic-login.md](research/early-classic-login.md). Packet IDs cannot
simply be substituted: the old header offsets, record start, counts, trailing
indexed lists and phase-dependent compression must agree with the
client's actual reader.

For comparison, the two bare clients' readers consume these layouts. Offsets
include the packet ID, `nA`/`nB` are the two trailing list counts, and lengths are
big-endian on the wire:

| Native response | Required structure | Total bytes |
| --- | --- | --- |
| 0x81 | ID(1), length(2), character count(1), flag/status(1), **five** `{name[30], password[30]}` records, `nA`(1), `nA × {index(1), ASCII[16]}`, `nB`(1), `nB × {index(1), ASCII[31], ASCII[31]}` | `307 + 17*nA + 63*nB` |
| 0x86 | ID(1), length(2), character count(1), **five** `{name[30], password[30]}` records | `304` |

The first 0x81 list count is at offset 305; the second is at
`306 + 17*nA`. Both bare handlers use signed-byte counts, so a future writer must
enforce compatible count/index bounds, not merely fit an unsigned packet field.
The first list is an indexed set of 16-byte strings used by UI controls; its
meaning is not proven to be a modern 0xA8 server/IP/port list. The second has the
same field widths as an index plus two creation-location strings, but its exact
semantics/order should remain tied to the native readers. A zero-length list
still needs its count byte. Character count does not make the reader's fixed
five-record region optional.

The `password[30]` label denotes the legacy per-character field. Existing Sphere
fills it with an empty string; future reuse should preserve that behavior unless
the native protocol evidence requires another value.

Sphere does already have two other writers with the old IDs. They are not its
initial login implementation:

- `PacketChangeCharacter` (0x81) writes length, character count, a zero byte and
  60-byte character records. It is used by the `CHANGECHAR` path for Classic
  clients. Even with exactly five records, it lacks both indexed lists/counts
  required by the native earlier 0x81 reader.
- `PacketCharacterListUpdate` (0x86) writes length, count and 60-byte character
  records after character deletion. With exactly five records its structure
  resembles the earlier 0x86, which is a reuse candidate requiring byte checks.
  Its existence does not prove the rest of the old creation/deletion/start
  sequence is implemented correctly.

Several existing gates also need to be considered together before changing a
writer:

- `PacketServerList` requests reversed address bytes through
  `CNetState::isClientLessVersionNumber(4.0.0)`. Version zero does not satisfy
  that test. More subtly, the detected version in `m_Crypt` is not immediately
  copied into `CNetState::m_clientVersionNumber`: the current assignment is in
  the successful game auth path, after `Setup_ListReq`. Thus the presence of
  the reverse-IP branch is not proof that it is selected for an initial old
  encrypted login.
- `Setup_FillCharList` initializes a minimum of five character records for all
  clients. Its later-client/unknown-crypt condition can raise the minimum to
  `max(actual quantity, 5)`. The common 30-byte name plus 30-byte empty password
  records and padding must be checked against each native reader; a declared
  count must match the records actually sent. A minimum of five is insufficient
  for the audited readers that consume exactly five slots: the actual record
  count and slot mapping need a compatible upper limit too.
- The old 0xA9 start records are index(1), area(31), name(31). Extended records
  use longer strings and coordinates; the checked source selects those using
  account version tags with a KR exception. The trailing flags branch uses
  the **reported** version tag above 1.26, or KR, rather than just the detected
  crypt version. These are current conditions, not verified universal era
  boundaries.
- `Setup_ListReq` queues 0xB9 before 0xA9, before `xProcessClientSetup` finishes
  restoring and validating the account's saved relay/version tags. Packet
  priorities and the actual send/compression transition must be tested; source
  construction order alone is not a full wire-order guarantee.
- `Login_ServerList` constructs 0xA8 before `xProcessClientSetup` saves the
  detected version into the account tags. Moving only a later version assignment
  cannot retroactively change bytes already constructed.

The supplied 1.25.34–1.25.36 clients have a later vocabulary than the bare builds.
Their shared or related cipher does not justify applying one packet-version
range to every supplied executable.

## Plaintext version zero and protocol identity

The no-crypt row sets the detected version to zero. After the shared XOR patch,
the server sees plaintext credentials but cannot recover a release from a key
match. The legacy four-byte seed carries no client version, and these clients
do not implement the later 0xBD response. Version zero means unknown here, not a
modern client and not an automatically selected early-client profile.

Even with encryption enabled, the `1230000` alias cannot distinguish the supplied
`1.23` from original `1.25`. They have the same master keys but differ in their
native game continuation length. Treating the alias as an exact protocol version
would select the wrong path for at least one of them. Letter builds likewise
share keys, so cipher detection should not be advertised as exact executable
identification.

The existing server `ClientVersion` configuration is not an implemented protocol
hint. `CClient` initially copies it into `m_Crypt`, but setup detection overwrites
the selected crypt row. `xCanEncLogin` uses it as an admission constraint for
encrypted clients; plaintext admission/version checks are deferred. Forcing the
old version into `m_Crypt` merely to affect packet layouts would also make the
login decryptor apply rotation to plaintext. It is therefore not a safe existing
workaround for the missing hint.

## Proposed boundaries for the next implementation

The following is a proposal for review, **not code in `94534efe`**:

1. Keep transport cipher state separate from effective protocol capabilities.
   Preserve the detected key-row identity, master keys, masks and stream offset.
   Resolve packet vocabulary, initial-response structure, continuation shape,
   incoming-cipher support, compression and version-probe support separately;
   avoid deriving every choice from one version-number comparison. If native
   server-to-client rotation is implemented, its masks must be independent of
   the existing client-to-server masks.
2. Define an explicit protocol hint for clients that cannot identify themselves,
   including plaintext old clients and the ambiguous fixed-key pair. Its name,
   configuration scope, validation and precedence require a design decision.
   Do not silently repurpose the existing admission-only `ClientVersion`, or
   pretend that changing the cipher alias distinguishes 1.23 from original 1.25.
3. Resolve the protocol before constructing the first response. Propagate it
   through the authenticated relay/session association independently of the
   cipher. A two-byte continuation requires a validated account/session binding
   through the relay seed/token/context and the existing authentication machinery.
   Specify replay and expiry handling where appropriate. It must not borrow
   credentials or identity from an unrelated connection, bypass authentication,
   or call `Setup_ListReq` with username/password fields the packet never supplied.
4. Add dedicated earlier packet serialization and continuation handling from the
   native field maps. Reuse existing character/server data where appropriate,
   but do not rename a modern 0xA8/0xA9 packet to 0x81. Specify 0x86 updates and
   character creation/deletion behavior as separate messages.
5. Specify bounded first-packet framing and the compression switch for both a
   fresh game socket and a retained socket. Compression must be a protocol
   capability rather than an automatic consequence of `ENC_LOGIN` or
   `CONNECT_GAME`. Cipher resets, directional mask state, version hints, packet
   lengths and authentication checks must remain explicit at each step.
6. Treat any direct-relay shortcut as an alternate design requiring explicit
   specification and validation. It is not the observed native early-client
   sequence and must not erase the two-byte-continuation/authentication question.

## Validation already available and work still required

The committed server regressions use **crafted Sphere-format 62/65-byte probes**
encrypted by executing the original client cipher-append routines. They are not
captures of every client's native packet writer. In particular, the 65-byte
probe through the 1.23 cipher proves that cipher can process those bytes; it does
not prove the native 1.23 client sends them.

The focused Sphere runner passed four cases and 192 assertions: outgoing-key
detection, chunking, exact sizes, in-place processing, later packets, retained
client-to-server masks and the current plaintext outbound identity. It does not
test the newly identified incoming cipher or native uncompressed framing. Replacing only the cipher source with the original
implementation makes three cases fail. A clean isolated test build and a normal
review binary were built; neither replaced the running server. Full CTest still
has the pre-existing version-parser failures and the temporary focused runner
avoids the unrelated variadic debug-logger shutdown bug. Exact commands and
workarounds are recorded in [the cipher research](research/classic-rotation-cipher.md#sphere-regression-and-build-evidence).

Before claiming interactive support, future validation must cover:

- Every distinct native protocol shape, with both stock encrypted and patched
  plaintext clients, using an explicit identity where detection is ambiguous.
- Login-only, fresh game connection and retained-socket traces, including the
  initial seed, selected keys, stream offsets, relay auth binding and exact
  compression transition. Trace packet headers/lengths without retaining account
  passwords in durable logs.
- Fragmented and coalesced first packets, unsupported probes, wrong continuation
  lengths and failed/expired relay authentication. Verify refusal without state
  reuse or accidental mask advancement.
- Independent server-to-client cipher vectors and seed/state lifecycle for both
  bare clients; raw uncompressed framing there versus the later clients' Huffman
  path. For a future complete-plaintext client patch, verify that both directional
  XORs are actually bypassed.
- Byte-exact initial 0x81/modern list output, address byte order, declared counts,
  fixed fields, suffixes, compression selection and delivery order. Follow with 0x86
  refresh and creation/deletion flows before extending the supported claim.
- Reaching a character list and selecting a character, then verifying the first
  accepted world packets in the actual client. Broader gameplay compatibility
  requires its own evidence and must not be inferred from reaching this point.
- Regression traces for modern Classic, KR and Enhanced paths before changing
  shared setup framing, packet selection or version state.

An isolated test instance or a separately approved deployment is needed for those
live checks. The current evidence establishes cipher correctness and identifies
the remaining protocol work; it does not establish a complete old-client session.
