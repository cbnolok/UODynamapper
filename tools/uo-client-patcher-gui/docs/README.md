# Client patcher documentation

The GUI and `uocf-cli` share the patch API in `lib/uocf/src/client/`. These documents cover login destinations,
directional ciphers, native login sequences and the emulator behavior needed by each protocol group.

- [Login server](LOGIN_SERVER.md): Classic configuration files, EC/KR built-in addresses, supported edits and survey.
- [Client encryption and login handshakes](ENCRYPTION.md): current controls, coverage boundaries, cross-family
  behavior and the source-only history of later Classic cipher modes.
- [Early Classic rotation ciphers](research/classic-rotation-cipher.md): where each directional cipher runs,
  seed/key/state arithmetic, shared outgoing patches, implementation rationale and executed validation.
- [Early Classic login sequences](research/early-classic-login.md): the eight supplied builds, packet layouts,
  original 1.23's two-byte continuation, old 0x81 lists, relay branches and receive/compression differences.
- [Sphere/Classic compatibility](SPHERE_CLASSIC_COMPATIBILITY.md): current server behavior, committed cipher
  fixes, incomplete incoming-cipher removal in the two earliest builds, protocol gaps and future validation.
- [KR executable coverage](research/kr-build-coverage.md): the expanded collection, exact version distinctions and earlier-build limits.
- [Enhanced Client research](research/uosa_login.md): login and network flow, packet registry, cipher code,
  ECLO patch sites and the complete recovered function/name tables.
- [Kingdom Reborn research](research/uokr_login.md): RTTI classes, cipher/handshake evidence, byte patches and validation.
- [KR plaintext callback assembly](research/kr-plaintext-game-login.s): the reviewed source for the bypass callback.
- [KR 2.45 callback assembly](research/kr-legacy-plaintext-game-login.s): the separate callback preserving the earlier SEH frame and cleanup.
- [Sphere/KR packet compatibility](SPHERE_KR_COMPATIBILITY.md): post-login packet formats, the applied server patch and the reported successful login.

For early Classic work, read the cipher reference, native login sequences and Sphere compatibility report
together. Outgoing XOR removal has been executed and tested for all eight files; complete encryption removal
and native Sphere login are still incomplete for original 1.23/1.25. The separate incoming rotation and original
1.23's two-byte `0x91` were established in the subsequent protocol audit. The documents distinguish those code
findings from the earlier constructed 65-byte cipher probes and from proposed compatibility changes.

The [Ghidra naming workflow](../../../docs/client_research/ghidra-login-names/README.md) and
`NameLoginCode.java` remain with the reusable inspection helpers. Research reports live here so future helper
cleanup cannot discard their recovered class names, addresses and evidence.
