# Login server

Where each client family reads its login server address from, and how `uocf` reads and changes it
(`client_config::login_servers`, `client_exe::login`, `ExeEdit::LoginServer`). Address and port only; the login
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
has no callers, and its global config is only ever cleared. The schema the dead loader parses, for writing files
anyway:

```xml
<LoginConfig>                                            (the root, or a child of the root)
  <LoginServer title="My Shard" name="host" port="2593" standalone="..."/>
  <LoginCredential title="..." server="..." account="..." password="..."/>
  <Login title="..." credential="..." character="..."/>
</LoginConfig>
```

Every attribute is optional (null-checked); `title` keys a map; `port` is `strtol` base 10; `LoginServersUser.xml`
loads after `LoginServersDefault.xml`; KR also looks in `client\`. `LoginServersXml` edits values in place and appends
new entries before `</LoginConfig>`, keeping all other text.

What the client connects to is in its code (`LoginAccount`):

- **Fixed store** `C7 44 24 d imm32` (the IPv4 address as a host-order u32, so 172.16.17.41 is stored `29 11 10 ac`)
  `66 C7 44 24 d+4 imm16` (port), then `JMP` to `LEA r32,[ESP+d]`. EC: an internal test address (172.16.17.41:7775, TC
  10.18.37.29:7775) used only behind an internal flag; not patched. KR: the only address (159.153.196.41:7775); KR is
  refused by the patcher.
- **EC public hosts**: pushed between the fixed store and the port store. TOL and later: `74 07 68 h1 EB 05 68 h2`, an
  odd `rand()` pushes host 1, an even one host 2. Older builds push one host.
- **EC public ports**: `89 modrm 24 d` `66 C7 44 24 d+4 imm16` plus, with two servers, `75 07 66 C7 44 24 d+4 imm16`.
  The immediate is a little-endian u16 of the port number (7775 = `5f 1e`); the client applies `htons` itself.
  A second `rand()` picks the port: 7775 when odd, 7776 when even, independently of the host draw. The patch writes
  the same host to both slots and the same port to both immediates, so every draw gives the one address.
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

## Tools

- `uocf-cli exe show <exe>`: the built-in address, slots, ports and whether they are stock.
- `uocf-cli exe patch <exe> --output <copy> --login-server HOST:PORT [--dry-run]`: EC only, all public slots and ports.
- `uocf-cli login show <client_dir>` / `login set <client_dir|file> --output <path> --server [NAME=]HOST:PORT ...`.
