# Dynamic KR patch resolution

The handshake bypass no longer selects a profile by version, hash or original virtual address. It scans executable
sections for either verified game-connect instruction layout, resolves its operands from the image and generates
replacement code for those addresses. The nineteen known executables remain regression fixtures. An untested
version can work when its layout and dependencies satisfy the same checks; a new compiler layout needs further
analysis.

Login encryption, game encryption and the KR handshake bypass remain independent controls. This change concerns
the KR handshake locator; the existing cipher and login-destination locators still use instruction patterns.
See [encryption controls](ENCRYPTION.md) for CLI flags and KR/EC behavior.

## Acceptance checks

The resolver requires exactly one complete candidate across both layouts. Masking address fields is only the
first search step. Before accepting a candidate, it checks:

- The full stock or supported patched instruction layout, including stack and register conventions.
- Relative calls and jumps, absolute globals and section bounds. Writable globals may lie in a section's virtual
  BSS tail; instruction bytes and metadata must be present in the file.
- The `htonl` import identity and bounded MSVC RTTI structures for Huffman, typed shared-pointer control blocks
  and the game connection.
- The instructions and calling conventions of helpers used by the replacement, including allocation, Huffman
  construction, shared ownership, receive-transform installation, Send91 and Disconnect.
- The retained E3 handler's relationships to those helpers and globals. This independently corroborates pointers
  erased by an earlier handshake patch; the injected code is not its own sole evidence.
- For the older aligned EBP/FS:0 layout, the exception-state graph and cleanup actions needed by the replacement.

Both layouts preserve their original connection/seed cleanup and use their own verified callback ABI. The modern
layout replaces 458 bytes in a 655-byte function; the older layout replaces 270 bytes in a 397-byte function.
Current replacements can be inspected again, and the earlier 81-byte modern callback can be upgraded to the
corrected 85-byte form. Ambiguous candidates, mixed replacements and inconsistent dependencies are refused.

## PE base relocations

When an image retains a base-relocation directory, stale HIGHLOW entries inside the overwritten span would
corrupt generated instructions after loader rebasing. The patch planner removes those entries and records the
replacement's absolute DWORD operands instead. Unrelated entries remain intact. Images without a relocation
directory need no relocation edits; this does not add rebasing support to relocation-stripped executables.

The planner reuses suitable empty entries before appending blocks into verified zero padding. Growth beyond the
directory requires a dedicated discardable `.reloc` section. It rejects malformed or unsupported relocation
records, partial boundary overlaps, duplicate targets, insufficient padding and overlap with other relocation
targets or PE data directories. The checksum and manifest include these edits.

`exe show` and the GUI report current code with stale records as needing PE relocation repair. Reapplying
`--bypass-kr-handshake` (or `--no-encryption`) repairs the records without replacing already-current code.

## Validation

The corpus covers all nineteen supplied executables from 2.45.3.13 through 2.58.0.6, including independent scoped
controls and patched readback. Generated functions are compared byte for byte with the previously executed
x86 callback buffers. Synthetic fixtures move code, data, imports and image bases, and exercise malformed
helpers, RTTI, exception metadata and ambiguous candidates.

A real-file test uses 2.46.0.3, which retains base relocations. It verifies that patching then rebasing produces
exactly the same bytes as rebasing then patching, accepts a copy whose resource version is changed to the
unknown 2.99.0.0, and repairs a previously patched copy's stale relocation records. These are offline code and
PE checks; they do not establish live Windows login behavior for every untested client version.
