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
- The signature is an RSA signature (e = 65537) under key id 20 (EC, 1025-bit N) or 21 (CC, 1026-bit N):
  `s^e mod N = 0xFF × 32 ‖ SHA-256(decompressed payload) ‖ 0xFF × 32`, the libtomcrypt 0.9x `rsa_signpad`
  layout. The keys are in the **patchers** (`uopatch.exe`, Classic `UO.exe`), not in `UOSA.exe`. With them
  every signature verifies offline, except 918 stale EC ones. **Confirmed.** We still cannot produce
  signatures (that needs the private key).
- **The client never checks either.** Every payload read skips `data_block_length` bytes; no code reads a
  signature record; `data_block_hash` is never computed, only read back as a cache stamp by the string
  table cache and the audio configuration cache, and in the shipped client neither cache is ever used.
  A header of length 0 works. **High.**
- **The patcher does check them.** In its normal mode it compares each entry's header length and
  `data_block_hash` with the manifest, and in its full file check mode it verifies every signature. So an
  edited entry is detected and downloaded again on the next normal or full patcher run; it survives only
  quick runs (product up to date, options unchanged) that do not change the entry count. **High** (code;
  not tested by running the patcher).

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
`lib/uocf/tests/synthetic/uop_data_block_header.rs::client_package_headers_match_their_data_block_hash` (Rust,
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

Earlier attempts to recover the modulus from the data (gcd of `s^e − m` for e ∈ {3, 5, 17, 257}, PKCS#1
v1.5 with SHA-1/SHA-256/MD5 or the raw digest) failed, as they had to: the scheme is not PKCS#1 v1.5 and
e is 65537. The keys were then read from the patchers (see "The patcher"). Making signatures for edited
entries needs the private key, which only the publisher has.

What `UOSA.exe` contains (**high**):

- Crypto++ 5.4 is linked, used for SHA-1, SHA-256, DH2 and AES-CFB by login and other code.
- No RSA, DSA, Rabin-Williams or ESIGN verifier class is present.
  `SignatureVerificationFilter`'s constructor 0x0086ab9f and the `PK_Signer` functions 0x00873d5b,
  0x00873ea3, 0x00873fb1, 0x00874044 have no references, and a scan for pointers to them found none.
- The verifier is in the patcher (see "The patcher"). The earlier note that the patcher binaries look
  packed was wrong.

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
| `AudioConfig_CollectFileStamps` 0x0088e3ca writes the audio cache header (format in "Cache stamps"); `AudioConfig_LoadCache` 0x0088e519 reads the cache's header and `AudioConfig_CacheHeaderDiffers` 0x0088d2eb compares version, flag and all six stamps field by field; any difference rebuilds | decompiled | confirmed |
| Neither cache is used by the shipped client: no shipped Lua or XML calls `LoadStringTable`, and `AudioConfig_Init` 0x0088db6a has no code or data reference | reference scans, package and folder searches | high |
| Hence: the value of `data_block_hash` matters to the client only as a change detector for those caches, which never run. It matters to the patcher (see "The patcher") | the above | high |

## Cache stamps

The two users of `UOP::GetFileDataBlockHash`. Both come from the Mythic engine; UO never exercises them.

String tables (**confirmed** unless noted):

- Lua `LoadStringTable(name, dir, file, cacheDir, errTarget)` (`LoadStringTable` 0x008d9781, registered by
  `FUN_008d88e7` at 0x008d89bb) → `UIStringTables_LoadStringTable` 0x008d754e → `UIStringTables_Create`
  0x008d72a5: rejects a name already registered (list at object +0x44), formats the table path `"%s/%s"`
  (dir/file) and the cache path `"%s/%s.cache"` (cacheDir/name), constructs a `UIStringTable`
  (`UIStringTable_ctor` 0x00930727, which replaces `<LANG>` in the paths through
  `UIStringTable_ReplaceLangToken` 0x009308d4, language at +0xc74, fallback language at +0xcb4, and calls
  `StringTable_ctor` 0x009a2e69 with (table, fallback table, cache)), then `StringTable_Load` 0x009a2f50 →
  `StringTable_BuildCache` 0x009a310c. `UIStringTables_ReloadAll` 0x008d46eb rebuilds every registered table
  on an interface reload.
- The table is UTF-16 (BOM FEFF) tab-separated text, looked up as a UOP entry or loose file (`UOP::FileExists`).
- Cache file: `cacheDir/name.cache` (after `<LANG>` substitution), opened with `UOP::OpenFilePtr(path, 1)`, so
  a loose file relative to the client folder. Header 0x10 bytes: `u32 version` (10), `u32 data_block_hash`
  of the table, `u32 data_block_hash` of the fallback table, `u32` (probably the string count). Reused only
  if all three match.
- **No shipped Lua or XML calls `LoadStringTable`** (searched: the whole EC folder, `Interface.uop` and
  the other packages). The only trace is `ChatWindow.lua`'s unused `StringTables.Chat` reference, left over
  from Mythic's engine. So no string table, no package entry and no cache file exist. **High.**
  The user data folders on the test machine (`Documents/EA Games/Ultima Online*/User Data`) and the client
  folders hold no `.cache` file either.

Audio configuration (**confirmed** unless noted):

- `AudioConfig_Init` 0x0088db6a (dir, prefix, cache path) → `AudioConfig_CollectFileStamps` 0x0088e3ca →
  `AudioConfig_LoadCache` 0x0088e519 → `AudioConfig_CacheHeaderDiffers` 0x0088d2eb (per stamp
  `FileStampDiffers` 0x0088d2d6). On a mismatch it parses the CSVs (`FUN_0088fbd1`, `FUN_0088fd0d`) and
  writes the cache (`AudioConfig_SaveCache` 0x0088e87e).
- Cache header, 0x38 bytes: `u32 version` = 2, `u8 complete` (written 0 first and set to 1 once the rest is
  written), 3 bytes padding, then six stamps `{u32 data_block_hash, u8 exists, 3 bytes padding}` (from
  `UOP::GetFileStamp` 0x0088e39d; hash 0 for a missing file) of `audio/audio_filelist.csv` (0x00ce4278),
  `audio/audio_filelist_vo.csv` (0x00ce425c) and `"%s%s%s"` = dir (+ `/`) + prefix + `sounds.csv`,
  `groups.csv`, `queues.csv`, `loops.csv` (pointer table 0x00da6fac..0x00da6fb8). Then a 0x40-byte block of
  counts, then records of 0x4c, 0x28, 0x4c, 0x1c and 0x28 bytes and length-prefixed strings. The file is
  opened with `UOP::OpenFilePtr(path, 1)`.
- `AudioConfig_Init` has no code or data reference (full pointer and rel32 scan), and none of the audio
  CSV names is in any EC package (lookup3 check). UO's audio uses its own tables instead
  (`data/audio/Sounds/audio_sounds.csv`, `Music/audio_music.csv`; `FUN_004b5180`, `FUN_004b53a0`). **High.**

So no package the repo writes can stale a client cache, whatever its `data_block_hash`.

## The patcher

Binaries (EC folder): `uopatch.exe` (SHA-256
`7c4ec66d9c53a8e45e35b4eba8fcfd5a7ce96dbf4f3baaea4ae32823fcf44dad`; `uopatch.bin` is identical),
`patchui.dll`, `patch.cfg`, `mft.myp`, `patch.myp`. Classic: `UO.exe` is the patcher (SHA-256
`422c8fbbbe2105c4486bea88c9a6dea27a0e07d233ebd96f3f659143f3d83417`, the same libpatchclient); its
`UOPatch.exe` is a 73 KB stub. Both are the EAMythic patcher (libpatchclient).

**Not packed (confirmed):** standard MSVC sections, `.text` entropy 6.57, normal imports (WINHTTP, ADVAPI32
`CryptAcquireContextA`/`CryptGenRandom`), and the 5720-byte overlay is the Authenticode certificate
(security directory size 0x1658).

Crypto (`uopatch.exe`, **confirmed**; names applied by `NameUopHeaderCode.java`, namespace `UOPatch`):

| Address | Name | Role |
|---|---|---|
| 0x0043e3e0 | `RegisterKey20` | builds the 0x91-byte libtomcrypt public key packet on the stack, imports it, registers it as id 20 |
| 0x00458dc0 / 0x00459250 | `ImportPublicKey` / `rsa_import` | libtomcrypt 0.9x key import |
| 0x00459050 | `rsa_exptmod` | public operation |
| 0x00458e00 / 0x00458fe0 | `rsa_public_decrypt_chunks` / `rsa_verify_decrypt` | chunked exptmod, then mode-2 depad |
| 0x00459690 | `rsa_signdepad` | takes the middle third of the 3n-byte block (pads not checked) |
| 0x00458b40 | `sha256_memory` | one-shot SHA-256 |
| 0x0042aba0 | `VerifySignature` | SHA-256 of the data, key by id, depadded value compared with the digest |
| 0x0042acf0 | `VerifyFileSignature` | whole file; logs "Signature failed for file [%hs]" |
| 0x0043e770 / 0x00407930 | `GetKeyStore` / `KeyStore_Add` | key store |

Classic `UO.exe` 0x0043e240 `RegisterKey21` builds key id 21 the same way.

Keys (libtomcrypt packets; e = 65537):

- id 20 (EC), N (1025 bits) =
  `01e8a070ecc32e3672106a4b4d855373fff86fdb27064cd0f936969608adbdc6f262959bcfa91d4dff6c58b0ce3e2f85dbb88d91597a05e5fa6ada578d61c1e9f35445509f2b5fe00c442c45e2f987b653fb1d8a0843f72df6180bf8c9638c9e7a8fdbe3ae9ce5f7f85bb251083611b5b539b010a9a6c5403f0a1f7b3236d905`
- id 21 (CC), N (1026 bits) =
  `025eb6c9c8ee58e29d12ce13f13b5a842a065ad182fc0c7494415ddd313981d54bc9381a12a504da3c35659ea992385e9ce184feb1d4a9c8c2aac7351126f76f00a50f5bf28a144ce277d9d107c3b5b69dfd2028dafb572678cbb93a24fafa5f92ff7f9f51cfa579a4270f088e91b5ab1cf9dd24309044ec3e6b8aae89beb3a325`

Scheme: `s^65537 mod N` is 96 bytes `0xFF × 32 ‖ SHA-256(decompressed payload) ‖ 0xFF × 32` (libtomcrypt
0.9x `rsa_signpad`). PKCS#1 v1.5 (SHA-1, SHA-256, MD5) and raw digests do not match, nor does SHA-256 of the
compressed bytes. Offline check (Python, stdlib) of every type-4 signature:

| Client | Key | OK | Bad |
|---|---|---|---|
| EC | 20 | 91007 | 918 (well-formed pads around a different digest: stale publisher signatures; e.g. `EffectTexture.uop` 820 / 9, `facet5x.uop` 1273 / 8) |
| CC | 21 | 44395 | 0 |

The manifests (`mft.myp`, a headerless version-5 MYP of MythicMFT XML) also sign whole files: version-4
packages appear as `<f n=... ul ct cl t><sig id="20" v=...>`, signed over SHA-256 of the whole file; 113 of
113 verify (including `Hues.uop`, `tileart.uop`, `EffectDefinitionCollection.uop`, `MultiCollection.uop` and
`UOSA.exe`). Version-5 packages are listed as packs with per-entry `<f ph sh ul ct cl ft ml mc t/>`
(`sh`/`ph` = high/low half of the name hash, `ul`/`cl` sizes, `ct` compression, `ml` header length, `mc` =
`data_block_hash`); all 829 `EffectTexture.uop` entries match. No pack has `nodelete`.

Patch flow (**high**, decompiled):

- `PatchProduct` 0x00438d20 sets the mode: **2 full file check** (`client.setting.fullfilecheck=1`, a
  "Product file mismatch" in quick mode, or a size threshold), **1 quick** ("Product [%ls] is up to date.
  Using quick patch mode.", unless `client.setting.skipquickcheck` or "Options ... different than last
  run"), else **0 normal**.
- `DeterminePatchSet` 0x00437c00 always runs `ExamineUnpackedFiles` 0x0042e9e0 and `ExamineLocalPacks`
  0x00437380 → `ExaminePack` 0x00436350 per pack.
- `ExaminePack`: a missing pack is downloaded whole. Quick mode compares only the entry count with the
  manifest (a mismatch drops to normal). Otherwise manifest and local entries are merge-joined: missing →
  download; local only → "exists in pack but not in manifest. Deleting."; different length or compression
  → download; different `ml` or `mc` → state 2 (re-fetch); in full mode every entry's signature record and
  payload go through `VerifySignature`, and a failure ("File [%08x-%08x] is corrupt.") downloads it.
- `ExamineUnpackedFiles` (whole files such as the version-4 packages): skipped in quick mode unless the
  stage sets `verifyunpacked`. Size ≠ `ul` → download. Full mode, `verifyunpacked`, or `.exe`/`.dll` →
  signature check. Otherwise the mtime is compared with `t` (± 3600 s); if it differs the signature is
  checked, and a failure ("File [%hs] has new data.") downloads the file.

In practice (**high** from code; not tested by running the patcher):

- Quick run: an entry replaced in place survives; an added entry changes the count and triggers a normal
  examination.
- Normal run: an edited version-5 entry is found by `ml`/`mc` (the repo's option-4 writer always changes
  both) or by length, and downloaded again; added entries are deleted. An edited version-4 package is
  restored if its size changed, or if its mtime changed and its signature then fails (any edit).
- Full file check: every edit is restored.

So edited packages must be kept away from the patcher (a separate client copy, or no patcher run).

## Writer policy (implemented: option 4)

Policy before option 4 was implemented (`lib/uocf/src/enhanced/uop_entries.rs::put_entry`, used by the
EffectTexture and EffectDefinitionCollection writers):

- A replaced entry kept its header and its `data_block_hash`. In a version-5 package the kept signature no
  longer matched the new payload.
- An added entry got a fresh 12-byte timestamp header (current FILETIME) and its Adler-32 when the package
  already had 12-byte headers (version 4); otherwise (version 5, e.g. `EffectTexture.uop`) no header and
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

Chosen: **option 4.** It is the client's own version-4 convention, it replaces a signature we can
no longer make valid with a record the client knows how to read, and the new FILETIME changes the stamp on
every write, so the string and audio caches would rebuild when their files are edited (they are not used,
see "Cache stamps"). Mixed type-3/type-4 headers in one version-5 package are harmless to the client
(records are parsed generically). The patcher restores any edited entry in a normal or full run whatever
the policy (see "The patcher"); option 4 only makes the edit visible to it through `ml`/`mc` as well.

Implementation (`lib/uocf/src/uop_container/`):

- `UopPackage::replace_file` replaces an entry's payload; `put_entry` (and through it the EffectTexture and
  EffectDefinitionCollection writers), `HousingPieces::replace_in_package` (`MultiCollection.uop`) and
  `uoppackage-cli replace` use it. `put_entry` gives added entries the same header.
- The header is `UopPackage::edit_data_block_header`: `DataBlockHeader::from_timestamp` of the package's
  edit FILETIME, hashed by `UopFile::set_data_block_header_and_hash` with the algorithm of its first record
  (Adler-32). The FILETIME is the current system time (`filetime_from_system_time`), or a fixed value set
  with `UopPackage::set_edit_filetime` for tests and reproducible output.
- An entry replaced by the payload it already holds, with the same compression, is left untouched: re-saving
  unchanged content does not change its stamp. Untouched entries keep header and hash byte for byte;
  `UopPackage::recompress` (`uoppackage-cli rebuild`) keeps every header and hash too.
- Checked on a copy of the EC `EffectTexture.uop` with one entry replaced by `uoppackage-cli replace`: 829 of
  829 headers match their `data_block_hash` (`client_package_headers_match_their_data_block_hash`), the
  replaced entry has a 12-byte timestamp header and the other 828 entries are byte-identical.

## Names applied

`NameUopHeaderCode.java` named 36 `UOSA.exe` functions (tag `UO_UOP`, plate comments with the evidence
above), 18 `uopatch.exe` functions and 1 Classic `UO.exe` function (namespace `UOPatch`). The reports of a
run after `--apply` (all "already applied"): [uosa_uop_header_names.md](uosa_uop_header_names.md),
[uopatch_names.md](uopatch_names.md), [uo_classic_patcher_names.md](uo_classic_patcher_names.md). Earlier names from `ghidra-source-names` (`UOP::OpenFilePtr`, `MypArchive_ReadEntry*`,
`UOP::MountArchive`, `UOP::FindEntryIndex`, `HashLittle2`, ...) were kept.

## Open items

- Which of record type or package version selects the hash algorithm: indistinguishable in the data.
- The patcher behaviour is read from code only; a run against an edited client copy would confirm it.
- The 918 stale EC signatures: why the publisher left them is unknown. From the code, a full file check
  should download those entries every time (**probable**).
