# Login server

Where each client family reads its login server address from, and how `uocf` reads and changes it
(`client::login_servers`, `client::login`, `ExeEdit::LoginServer`). Address and port only; the login
encryption is out of scope here.

## Classic (`client.exe`)

The client loads `uo.cfg`, then the file its `LoginServerFile` key names or, without the key, `login.cfg`; both go
through the same `key=value` loader. Real files (Gold to 7.0.x) are CRLF, comments start with `;`, the last line may
lack its newline and the names vary in case (`Login.cfg`, `Uo.cfg`). Entries seen:

```
;Loginservers for Ultima Online
LoginServer=127.0.0.1,2593
LoginServer=login.owo.com,7775        (UO Gold: a host name)
BackupLoginServer=host,port
```

`LoginCfg` keeps every line byte for byte; `set_servers` replaces the lines of one kind (the first old line's place
takes the new lines). Host names are accepted (Gold ships one). Edit with `uocf-cli login set`; the executable holds no
address to patch.

## Enhanced Client (`UOSA.exe`) and Kingdom Reborn (`UOKR.exe`)

**The `LoginServers*.xml` files are not read.** In every traced EC and KR build the XML loader (`LoadLoginServers`)
has no callers, and its global config is only ever cleared. XML parsing, editing and file enumeration are not supported.

In the examined UOSA.exe, `LoginConfig::LoadLoginServers` at `0x006052a0` has no incoming references. The only write
to `g_LoginConfig` (`0x00e37338`) is the cleanup function at `0x004aa830`, which clears it at `0x004aab6d`.
`LoginManager::LoginAccount` (`0x00613510`) therefore follows its built-in-address branch in that build.
The earlier inference that simply adding XML files enables this config was wrong; the dormant parser's presence
does not make it active. See the [login research](../../../../scripts/dev-helpers/ec-research/ghidra-login-names/results/uosa_login.md).

What the client connects to is in its code (`LoginAccount`):

- **Fixed store** `C7 44 24 d imm32` (the IPv4 address as a host-order u32, so 172.16.17.41 is stored `29 11 10 ac`)
  `66 C7 44 24 d+4 imm16` (port), then `JMP` to `LEA r32,[ESP+d]`. EC: an internal test address (172.16.17.41:7775, TC
  10.18.37.29:7775) used only behind an internal flag; not patched. KR: the only address (159.153.196.41:7775); the
  patch writes the imm32 and imm16 there. The imm32 is a number, not a string the client resolves, so KR takes a
  dotted IPv4 address only (a host name is refused).
- **EC public hosts**: pushed between the fixed store and the port store. TOL and later: `74 07 68 h1 EB 05 68 h2`, an
  odd `rand()` pushes host 1, an even one host 2. Older builds push one host.
- **EC public ports**: `89 modrm 24 d` `66 C7 44 24 d+4 imm16` plus, with two servers, `75 07 66 C7 44 24 d+4 imm16`.
  The immediate is a little-endian u16 of the port number (7775 = `5f 1e`); the client applies `htons` itself.
  A second `rand()` picks the port: 7775 when odd, 7776 when even, independently of the host draw.
- **The edit** (`LoginServerEdit`) holds one host per host site and one port per port site, in site order (`exe show`
  numbers them from 1); counts that differ from the build's sites are refused. `LoginServerEdit::uniform` (and
  `--login-server`) puts the same host on every slot and the same port on every immediate, so every draw gives the
  one address. Different hosts per slot each pass their own slot's capacity. Since the host and port draws are
  independent, different ports pair with either host. Manifests written before per-site edits recorded
  `{"host", "port"}`: they read as one host and one port.
- Hosts are strings resolved by `sscanf("%u.%u.%u.%u")`, else `gethostbyname`, so host names work. The string lives
  in a slot whose capacity is its length plus the zero padding after it, capped at 16 (`LOGIN_HOST_SLOT_SIZE`); each
  slot is referenced once in the image. The patch writes the host zero-filled to the whole slot, and refuses a host
  that does not fit with its terminator.

## Survey (file offsets)

| exe | fixed (internal / KR) | hosts: slot, value (capacity) | ports: imm16 |
|---|---|---|---|
| Enhanced/UOSA.exe 4.0.102.3 | 0x21360a 172.16.17.41:7775 | 0x8b78cc 107.23.85.115 (16), 0x8b78dc 107.23.176.74 (16) | 0x2136c2 7775, 0x2136cb 7776 |
| Enhanced/UOSA_TC.exe 4.0.29.97 | 0x2036aa 10.18.37.29:7775 | 0x8a1aa8 107.23.85.115 (16) | 0x203728 7775 |
| Enhanced/UOSA_eclo.exe | 0x21360a 172.16.17.41:2593 | 0x8b78cc 127.0.0.1 (16), 0x8b78dc 127.0.0.1 (16) | 0x2136c2 2593, 0x2136cb 2593 |
| Enhanced fp/UOSA.exe 4.0.115.0 | 0x216aea 172.16.17.41:7775 | 0x8bcaa4 13.248.219.129 (16), 0x8bcab4 75.2.70.184 (**12**) | 0x216ba2 7775, 0x216bab 7776 |
| Old KR/UOKR.exe | 0x1b594b 159.153.196.41:7775 | none | none |

Each pattern matches once per build. `UOSA_eclo.exe` differs from the stock 4.0.102.3 only in these 26 bytes (ECLO
also rewrote the internal port; it did not update the PE checksum); it inspects as "not stock" and can be patched
again (with a warning, since no manifest records its stock values). In 4.0.115.0 the second slot holds at most 11
characters: ECLO's fixed 16-byte write would run into the next string.

The survey's versions come from each exe's version resource (`VS_FIXEDFILEINFO`, `client::version`), which
`exe show`, the inspection and the manifest (`exe_version`) report; nothing is decided from them.

## Tools

- `uocf-cli exe show <exe>`: the version, the built-in address, slots, ports and whether they are stock.
- `uocf-cli exe patch <exe> --output <copy> [--dry-run]` with `--login-server HOST:PORT` (every site),
  `--login-host N=HOST` (one host slot; overrides `--login-server`'s host) and `--login-port [N=]PORT` (one port site,
  or every site without `N`). Without `--login-server` every site needs its own value. EC and KR (KR: IPv4 only).
- `uocf-cli login show <client_dir>` / `login set <client_dir|file> --output <path> --server HOST:PORT ...`.
