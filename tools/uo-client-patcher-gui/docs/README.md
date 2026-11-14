# Client patcher documentation

The GUI and `uocf-cli` share the patch API in `lib/uocf/src/client/`. These documents separate login destinations
from cipher removal and KR game-login setup.

- [Login server](LOGIN_SERVER.md): Classic configuration files, EC/KR built-in addresses, supported edits and survey.
- [Encryption and KR handshake](ENCRYPTION.md): independent controls, server requirements, seeds and master keys.
- [KR executable coverage](research/kr-build-coverage.md): the expanded collection, exact version distinctions and earlier-build limits.
- [Enhanced Client research](research/uosa_login.md): login and network flow, packet registry, cipher code,
  ECLO patch sites and the complete recovered function/name tables.
- [Kingdom Reborn research](research/uokr_login.md): RTTI classes, cipher/handshake evidence, byte patches and validation.
- [KR plaintext callback assembly](research/kr-plaintext-game-login.s): the reviewed source for the bypass callback.
- [KR 2.45 callback assembly](research/kr-legacy-plaintext-game-login.s): the separate callback preserving the earlier SEH frame and cleanup.
- [Sphere/KR packet compatibility](SPHERE_KR_COMPATIBILITY.md): post-login packet formats, the applied server patch and the reported successful login.

The [Ghidra naming workflow](../../../scripts/dev-helpers/ec-research/ghidra-login-names/README.md) and
`NameLoginCode.java` remain with the reusable inspection helpers. Research reports live here so future helper
cleanup cannot discard their recovered class names, addresses and evidence.
