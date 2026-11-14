# Sphere and KR login packet formats

Source examined: local Source-X commit `7e46c5ae`, with the server/configuration in
`build/buildlinux_clang_nightly/bin-x86_64`. The configuration enables both `UseCrypt` and `UseNoCrypt`.
The running server binary has not been matched to that source revision, and no live KR packet capture has
yet confirmed which packet stops a particular connection.

The game relay and plaintext 0x91 can succeed while the client remains at shard selection. There are two
independent packet-format disagreements in the inspected source:

| Packet | Audited KR 2.58 parser | Source-X version-selected output |
|---|---|---|
| 0xB9 features | Fixed length 3; 16-bit flags (`0x59f7f9`, reader `0x5b0770`) | Length 5; 32-bit flags when saved version is at least 6.0.14.2 |
| 0xA9 character list | Legacy 63-byte city entries (`0x5b05c0`) | Extended entries when saved version is at least 7.0.13.0 |

KR advertises protocol major 66 in 0xEF. Treating this as a Classic version selects both newer formats.
The handshake bypass also uses the auth-key seed rather than `0xffffffff`, so the game socket is initially
typed as 2D. KR reports its type with 0xE1 only after parsing 0xA9's request flag `0x400` (`0x5b5ca0` calls
`0x5b5b10`). Checking only the game connection's current KR type is therefore insufficient for that route.
KR consumes the optional trailing 0xA9 word only when present; it need not be added to the legacy packet.

The [Source-X patch](research/sphere-kr-login.patch) recognizes protocol major 66 from the account's
saved reported version, with the connection's known type/version as fallback. It emits 16-bit 0xB9 flags and
legacy 0xA9 cities, retaining 32-bit character-list flags and the initial type-request bit. EC and modern Classic
retain their respective packet layouts. This fixes server packet selection, not client encryption.

The approved change is committed in Source-X as `6bcbc951`. The actual patched constructors compiled in a
standalone framework fixture with `-Wall -Wextra -Werror`; seven format cases passed, including KR before/after
0xE1, EC and modern Classic. The full Clang Nightly server build also passed. Its separate candidate is
`build/kr-login-compat/SphereSvrX64_nightly`; the existing running server binary and configuration were preserved.
The candidate was not launched by the patcher work. The user subsequently reported a successful login with
the unencrypted KR client and identified incompatible `sphereCrypt.ini` entries. No packet capture or exact
client/server binary pair accompanied that report, so it confirms the working local route without identifying
which earlier change resolved each symptom. See [Sphere crypt-table IDs](ENCRYPTION.md#sphere-crypt-table-version-ids)
for the independently verified configuration errors.
The archived patch uses zero context; apply it to the recorded base with `git apply --unidiff-zero`.

Reproduce the isolated build from the patched Source-X checkout:

```
cmake -S . -B /tmp/sphere-kr-login-build -G Ninja \
  -DCMAKE_TOOLCHAIN_FILE=cmake/toolchains/Linux-Clang-x86_64.cmake \
  -DCMAKE_BUILD_TYPE=Nightly -DUNIT_TESTING=OFF
cmake --build /tmp/sphere-kr-login-build --target spheresvr_nightly --parallel 8
```

See [encryption controls](ENCRYPTION.md), [KR evidence](research/uokr_login.md) and
[EC evidence](research/uosa_login.md) for the separate client-side behavior.
