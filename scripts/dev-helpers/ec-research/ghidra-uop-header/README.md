# ghidra-uop-header

Names the code in `UOSA.exe` (Enhanced Client) that reads UOP (MYP) package headers, entry tables, data
block headers and `data_block_hash`, and the two users of `data_block_hash` (the string table and audio
caches); and, in the patchers (`uopatch.exe` of the EC, `UO.exe` of the Classic client), the signature
check and the patch flow that uses it. Checks every data block header of a set of packages against its
`data_block_hash`.

Findings: [results/uosa_uop_header.md](results/uosa_uop_header.md) covers the header format, the two
`data_block_hash` algorithms with their verification counts, what is known about the signature records,
the client's behaviour with addresses and confidence, the cache stamp users, the patcher (keys, signature
scheme, offline verification counts, patch modes) and the repo's writer policy (option 4, implemented).
[results/uosa_uop_header_names.md](results/uosa_uop_header_names.md),
[results/uopatch_names.md](results/uopatch_names.md) and
[results/uo_classic_patcher_names.md](results/uo_classic_patcher_names.md) are the script reports after
`--apply`.

## NameUopHeaderCode.java

GhidraScript, category `ECResearch`.

```
NameUopHeaderCode.java OUTPUT_FILE [--apply]
```

- Names fixed addresses, so it refuses any program whose SHA-256 is not one of the tested builds:
  `UOSA.exe` (`5a5e9dc5...ebad50`), `uopatch.exe` (`7c4ec66d...44dad`, namespace `UOPatch`) or Classic
  `UO.exe` (`422c8fbb...83417`, namespace `UOPatch`). Import the patchers into the Ghidra project to run it
  on them.
- Before naming, each address is checked: a function exists there, and its stated evidence holds (vtable
  slot of `MypArchive`, callers, callees, constants, referenced strings). A failed check is reported as a
  mismatch and nothing is named.
- `OUTPUT_FILE` is created with CREATE_NEW; an existing file stops the run before any change. Spaces in the
  path are passed as `%20`.
- Without `--apply` the script only reports. With `--apply` it:
  - renames DEFAULT (`FUN_...`) functions with source ANALYSIS, creating the `UOP` namespace if needed;
  - adds the tag `UO_UOP`;
  - sets a plate comment `NameUopHeaderCode: UO_UOP: <evidence>` when the comment is empty, and refreshes
    its own comment from an earlier run when the evidence changed.
- Names with another source type are kept and reported; a name already used in the namespace is reported as
  a conflict. A second `--apply` reports `changes: none`.

Run it through ghidra-mcp:

```
run_ghidra_script(program="UOSA.exe", script_name="<abs path>/NameUopHeaderCode.java",
                  args="<scratch>/uop_header_names.md --apply")
```

## verify_uop_headers.py

```
python3 -I verify_uop_headers.py PACKAGE_OR_DIR [...]
```

Stdlib only; a directory stands for every `*.uop` in it. Parses each entry's data block header into
records, computes the expected `data_block_hash` (Adler-32 for a timestamp header, lookup3 `hashlittle`
with initial value 0 for a signature header) and prints per package the version, matches, header kinds
(record type, key id) and header lengths, then the total. Exits 1 on any mismatch.

The same check runs in Rust: `lib/uocf/tests/synthetic/uop_data_block_header.rs`
(`UO_EC_CLIENT_DIR` / `UO_CC_CLIENT_DIR`, `cargo test -p uocf --test synthetic uop_data_block_header:: -- --ignored`).

### Notes

- `ghidra-source-names/NameUopTrace.java` named the read path earlier (`UOP::OpenFilePtr`,
  `MypArchive_ReadEntry*`, `UOP::Decompress`, ...); this script keeps those names and names the rest of the
  `MypArchive` vtable and the header and hash accessors.
