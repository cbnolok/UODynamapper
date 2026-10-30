# UOP data block headers in UOSA.exe

Program: `UOSA.exe`, SHA-256 `5a5e9dc536c836d1bf10b6f86bb88f61a6ae078f519d4060d3bc47aecbebad50`.
Data: the client packages in `_Ultima Online Enhanced` (EC) and `_Ultima Online Classic` (CC), plus the
copies in `_Ultima Online Enhanced fp`, `_Ultima Online Classic fp` and `Ultima Online Classic_7_0_20_0`.

Confidence: **confirmed** (read in code and/or checked on every entry), **high** (read in code, no
counter-evidence, but absence claims rest on reference searches), **probable** (inferred from partial
code reading), **unknown**.

## Summary

- A data block header is a sequence of records `{u16 type, u16 length, data}` (little-endian). Client
  packages hold exactly one record per entry: a **timestamp** (type 3, 8-byte FILETIME, 12-byte header) in
  every version-4 package, a **signature** (type 4, 134–137-byte header) in every version-5 package.
  **Confirmed.**
- `data_block_hash` is a hash of the whole header: **Adler-32** for timestamp headers, Bob Jenkins'
  **lookup3 `hashlittle` with initial value 0** for signature headers (the same lookup3 the client uses for
  file names). 282002 of 282002 entries match in the EC + CC reference clients; 347373 of 347373 more in
  the three other client copies. **Confirmed.** It is unkeyed and covers only the header, never the payload.
- The signature is a 1025/1026-bit big-endian integer under key id 20 (EC) or 21 (CC). It covers the
  stored payload (identical payloads have identical signatures under the same key, whatever their name).
  It looks like an RSA-family signature, but **the scheme and the key are not in `UOSA.exe`**: there is no
  public-key verifier in the binary, and the modulus could not be recovered. We cannot verify these
  signatures, and we cannot produce them (that would need the private key).
- **The client never checks either.** Every payload read skips `data_block_length` bytes; no code reads a
  signature record; `data_block_hash` is never computed, only read back as a cache stamp for the string
  table cache and the audio configuration cache. A header of length 0 works. **High.**

## Format

Package and entry table, as read by `UOP::MypArchive_ctor` 0x0098d044 (**confirmed**):

| Offset | Field | Notes |
|---|---|---|
| 0x00 | `u32` magic `MYP\0` | not checked by the client (its error strings at 0x00cf8b88/0x00cf8bc0 are unreferenced) |
| 0x04 | `u32` version | must be 4 or 5, else the archive has no entries |
| 0x08 | `u32` misc 0xFD23EC43 | if it differs, the client byte-swaps it and, on a match, reads the package as big-endian |
| 0x0c | `u64` first block | |
| 0x14 | `u32` block size | the client reads at most 100 entries (0xd54 bytes) per block read |
| 0x18 | `u32` file count | reading stops after this many non-empty entries |

Entries (0x22 bytes): `u64 offset`, `u32 data_block_length`, `u32 compressed`, `u32 decompressed`,
`u64 name hash`, `u32 data_block_hash`, `u16 compression`. Entries with offset 0 are skipped. A name hash
already in the index is not inserted again (the first entry wins; `UOP::MypArchive_EntryHashTable_Insert`
0x0098db81).

Data block header (`data_block_length` bytes at `offset`; the payload follows):

| Record | Data | Where |
|---|---|---|
| type 1, string | bytes | read by `UOP::MypArchive_GetHeaderString` 0x0098d8d1; in no client package |
| type 3, timestamp | `u64` FILETIME (LE) | every entry of every version-4 package; header `03 00 08 00` + FILETIME |
| type 4, signature | `u16 key_id` (BE), `u16 len` (BE, = record length − 4), `len` bytes: big-endian unsigned integer, no leading zeros | every entry of every version-5 package |

Signature records seen (**confirmed** on all entries):

| Client | Key id | Value lengths | Header lengths |
|---|---|---|---|
| EC (28 version-5 packages) | 20 | 126–129 | 134 (1, facet3), 135, 136, 137 |
| CC (12 version-5 packages: `artLegacyMUL`, `gumpartLegacyMUL`, `map*LegacyMUL`) | 21 | 127–129 | 135, 136, 137 |

No client entry has an empty header or more than one record.

Package survey (reference clients; `verify_uop_headers.py` prints per-package counts and lengths):

- EC version 4 (timestamp): AnimationDefinition, AnimationFrame4, AnimationSequence,
  EffectDefinitionCollection, Hues, LegacyTexture, LocalizedStrings, MultiCollection, TerrainDefinition,
  facets, string_dictionary, tileart.
- EC version 5 (signature, key 20): AnimationFrame1/2/3/5/6, Audio, EffectTexture, GameData, Interface,
  LegacyTerrain, MainMisc, Paperdoll, Shaders, SystemTextures, TerrainTexture, Texture, facet0/0x/1/1x/2/2x/
  3/4/5/5x, gumpartLegacyMUL, waypoint.
- CC version 4: AnimationFrame1–4, AnimationSequence, MainMisc, MultiCollection, soundLegacyMUL,
  string_dictionary, tileart. CC version 5 (key 21): artLegacyMUL, gumpartLegacyMUL, map0–5(x)LegacyMUL.
- `Ultima Online Classic_7_0_20_0` has only version-4 timestamp packages.
- `EffectTexture.uop` (EC): 829 entries, header lengths 135: 1, 136: 429, 137: 399; 829 of 829 hashes match.

## data_block_hash

| Header | Algorithm | Matches |
|---|---|---|
| timestamp (type 3) | `adler32(header)` | all |
| signature (type 4) | lookup3 `hashlittle(header, length, 0)` (= the `c` output of `hashlittle2` with `pc = pb = 0`; the file name hash is `b << 32 \| c` of the same function) | all |

| Client copy | Entries | Match |
|---|---|---|
| `_Ultima Online Enhanced` | 187348 | 187348 |
| `_Ultima Online Classic` | 94654 | 94654 |
| `_Ultima Online Enhanced fp` | 203529 | 203529 |
| `_Ultima Online Classic fp` | 103612 | 103612 |
| `Ultima Online Classic_7_0_20_0` | 40232 | 40232 |

Checked by `verify_uop_headers.py` (Python, stdlib) and by
`lib/uocf/tests/uop_data_block_header.rs::client_package_headers_match_their_data_block_hash` (Rust,
reference clients). Adler-32 is ruled out for signature headers: their hash's low half reaches 65535, above
Adler's 65520 limit.

Open: the record type and the package version are perfectly correlated, so the data cannot tell which of the
two selects the algorithm. The uocf implementation selects by the first record type. Nothing in `UOSA.exe`
computes either hash (the packer is not part of the client).

## The signature

What the data shows (**confirmed** unless noted):

- Values are uniformly distributed below a bound just over 2^1024: key 20 at most 1025 bits, top bits
  0xf44f (N ≈ 1.91·2^1024; 47% of values 129 bytes long); key 21 at most 1026 bits, top bits 0x97ac
  (N ≈ 2.37·2^1024; 58% 129 bytes). This fits `s < N` for a ~1025/1026-bit modulus (**probable**).
- Deterministic: 20548 pairs of entries with the same stored payload (SHA-1 over the compressed bytes) have
  the same signature and the same `data_block_hash` under the same key; the 43 pairs that differ are all
  key 20 vs key 21. Identical payloads under different names share a signature, so the name is not signed.
- No randomness or timestamp is in the header, so a signature is a function of (key, payload) only
  (**probable**: the signed message may also include fixed data such as the key id).

What was tried and failed (all **confirmed** negative): modulus recovery by gcd of `s^e − m` over many
entries, for e ∈ {3, 5, 17, 257} and m = PKCS#1 v1.5 (SHA-1, SHA-256, MD5) or the raw digest, over the
compressed and the decompressed payload, for k ∈ {128, 129, 130}. e = 65537 was not tried (too slow without
gmpy2). Even a recovered public key would only allow verification; making signatures for edited entries
needs the private key, which only the publisher has.

What `UOSA.exe` contains (**high**):

- Crypto++ 5.4 is linked, used for SHA-1, SHA-256, DH2 and AES-CFB by login and other code.
- No RSA, DSA, Rabin-Williams or ESIGN verifier class is present.
  `SignatureVerificationFilter`'s constructor 0x0086ab9f and the `PK_Signer` functions 0x00873d5b,
  0x00873ea3, 0x00873fb1, 0x00874044 have no references, and a scan for pointers to them found none.
- The patcher binaries (`uopatch.exe`, `patchui.dll`) show no crypto strings, so they are probably packed.
  Whether the patcher checks signatures is **unknown**.

## Client behaviour

| Claim | Evidence | Confidence |
|---|---|---|
| Archives are mounted by `UOP::MountArchive` 0x0098a341, which keeps one if `UOP::MypArchive_IsOpen` (slot 1, 0x0098d447: stream size ≠ 0) | decompiled | confirmed |
| Lookups walk the mounted archives from the last mounted to the first (list at 0x00e35238..0x00e3523c) | callers of `UOP::FindEntryIndex` | high |
| Payload reads seek `offset + data_block_length`: `MypArchive_ReadEntryRaw` 0x0098d61b (slot 4), `UOP::MypArchive_ReadEntryRange` 0x0098d670 (slot 5), `UOP::MypArchive_GetEntryPayloadOffset` 0x0098d995 (used by `UOP::OpenFileHandleAtPayload` 0x0098e39d) | decompiled | confirmed |
| A header of length 0 works: the payload is then at `offset`; nothing requires a header | the same functions; no length check | high |
| `MypArchive_ReadEntry` 0x0098d57c (slot 3) fails if the compressed or decompressed size is 0; compression 1 is zlib, 0 a copy | decompiled | confirmed |
| Header records are read only by `UOP::MypArchive_GetHeaderRecord` 0x0098d76e (slot 6): type 4 returns the whole raw header, any other type walks the records (byte-swapping prefixes for big-endian packages, `UOP::SwapHeaderRecordPrefix` 0x0098da27) and returns the first match | decompiled | confirmed |
| Slot 6 is called only by slot 2 `UOP::MypArchive_GetEntryTimestamp` 0x0098d53d (type 3) and slot 7 `UOP::MypArchive_GetHeaderString` 0x0098d8d1 (type 1); no call through slot 6 with type 4 exists | virtual-call scan for `[reg+0x18]` with pushed constants | high |
| The timestamp is unused: slot 2's callers `UOP::GetFileTimestamp` 0x0098a125 and `UOP::MypArchive_CollectEntryTimestamps` 0x0098d9c0 have no references; no slot-7 caller was found | references and virtual-call scan | high |
| No code verifies a signature or compares a header with its payload | the above plus the Crypto++ survey | high |
| `data_block_hash` is read only by `UOP::MypArchive_GetEntryDataBlockHash` 0x0098d978, through `UOP::GetFileDataBlockHash` 0x0098a1f5 | references | confirmed |
| `StringTable_BuildCache` 0x009a310c stores the hashes of the string table and its fallback table in its cache file header and reuses the cache only if the cache version is 10 and both hashes match; otherwise it rebuilds | decompiled | confirmed |
| `AudioConfig_CollectFileStamps` 0x0088e3ca collects the hashes (`UOP::GetFileStamp` 0x0088e39d, 0 for a missing file) of `audio/audio_filelist.csv`, `audio_filelist_vo.csv` and the sounds/groups/queues/loops CSVs; its caller 0x0088db6a passes them to the cache loader 0x0088e519, which reads a 0x38-byte cache header, compares it (0x0088d2eb) and uses the cache only on a match | decompiled | probable (the comparison at 0x0088d2eb was not read field by field) |
| Hence: the value of `data_block_hash` matters only as a change detector for those caches. If one of those files changes and its hash does not, the client may keep a stale cache | the two users above | probable |

## Writer policy (proposal; not implemented)

Current policy (`lib/uocf/src/enhanced/uop_entries.rs::put_entry`, used by the EffectTexture and
EffectDefinitionCollection writers):

- A replaced entry keeps its header and its `data_block_hash`. In a version-5 package the kept signature no
  longer matches the new payload.
- An added entry gets a fresh 12-byte timestamp header (current FILETIME) and its Adler-32 when the package
  already has 12-byte headers (version 4); otherwise (version 5, e.g. `EffectTexture.uop`) no header and
  `data_block_hash` = Adler-32 of the stored payload.

The client accepts all of these. Options:

1. **Keep as is.** Works for every package the effect writers touch (no cache is keyed on them). Leaves
   stale signatures, and a replaced cache-stamped file (string table, audio CSVs) would keep its old stamp,
   so the client could reuse a stale cache.
2. **Recompute the hash only.** Has no effect while the header is kept: the hash covers the header, not the
   payload, so it comes out the same.
3. **Drop the header** (`data_block_length` 0) and store any new `data_block_hash`. Accepted by the client
   (payload at `offset`), smallest output, but no client packer produces headerless entries and the hash
   then has no defined input.
4. **Write a fresh timestamp header** (type 3, current FILETIME) and set `data_block_hash` to its Adler-32,
   for replaced and added entries in both versions.

Recommendation: **option 4.** It is the client's own version-4 convention, it replaces a signature we can
no longer make valid with a record the client knows how to read, and the new FILETIME changes the stamp on
every write, so the string and audio caches rebuild when their files are edited. Mixed type-3/type-4
headers in one version-5 package are harmless to the client (records are parsed generically). Its one
unknown is the patcher: if it checks signatures it will flag the entry, but it would flag an edited payload
under the old signature just as well. `DataBlockHeader` / `DataBlockHashAlgorithm` in
`lib/uocf/src/uop_container/data_block_header.rs` already provide what this needs.

## Names applied

`NameUopHeaderCode.java` named 24 functions (tag `UO_UOP`, plate comments with the evidence above);
[uosa_uop_header_names.md](uosa_uop_header_names.md) is the report of a run after `--apply` (all
"already applied"). Earlier names from `ghidra-source-names` (`UOP::OpenFilePtr`, `MypArchive_ReadEntry*`,
`UOP::MountArchive`, `UOP::FindEntryIndex`, `HashLittle2`, ...) were kept.

## Open items

- Signature scheme and public key: not in the client; modulus recovery with e = 65537 not attempted.
- Whether the patcher (`uopatch.exe`) verifies signatures: unknown, its binaries appear packed.
- Which of record type or package version selects the hash algorithm: indistinguishable in the data.
- Which packages hold the string table and audio CSV files the caches stamp: their UOP names were not
  resolved, so it is not established whether any of them is in a package the repo writes.
- The audio cache header comparison (0x0088d2eb) is read only in outline.
