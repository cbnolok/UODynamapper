# Direct3D 9 types for Ghidra

Ghidra's bundled `windows_vs12_32.gdt` has no DirectX headers, so `UOSA.exe`'s D3D9 and
D3DX9 imports and its COM calls are untyped. This folder builds a 32-bit data type
archive with the Direct3D 9 / D3DX9 function signatures, structs, enums and COM
interfaces in C layout (`IDirect3DDevice9 { const IDirect3DDevice9Vtbl *lpVtbl; }`).

| Path | Content |
|---|---|
| `CreateD3D9Archive.java` | Headless Ghidra script that parses the headers into a `.gdt` |
| `build_d3d9_gdt.sh` | Runs the script; patches the one header Ghidra cannot preprocess |
| `headers/` | Macro prelude and empty stubs for `windows.h`, `objbase.h` and the CRT |
| `ImportSignatureCoverage.java` | Reports imports without a signature in the archives |
| `fix_wine_exports.py` | Restores real names in `.exports` files Ghidra built from Wine DLLs |
| `results/d3d9_x86.gdt` | Built archive |
| `results/uosa_import_signature_coverage.md` | Coverage report for the tested `UOSA.exe` |

## Building the archive

The headers are mingw-w64's (Wine-derived) `d3d9*.h` and `d3dx9*.h`; zig bundles them
under `lib/libc/include/any-windows-any`. The output is created exclusively.

```sh
scripts/dev-helpers/ec-research/ghidra-d3d9-types/build_d3d9_gdt.sh \
  /path/to/ghidra /usr/lib/zig/libc/include/any-windows-any /tmp/d3d9_x86.gdt
```

Ghidra's C parser cannot parse mingw's `windows.h` and CRT headers, so `headers/` replaces
them with empty stubs and `d3d9_prelude.h` supplies the WinAPI/COM macros they would
define. Base types (`UINT`, `HRESULT`, `GUID`, `RECT`, ...) resolve from
`windows_vs12_32.gdt`, and parsed types equivalent to its types are replaced by them,
so the archive shares base types with programs that already use that archive. The
`d3dx9math.inl` stub drops inline helper bodies. The build script also drops the
multi-line string `*_TEMPLATES` macros from a temporary copy of `d3dx9mesh.h`: they
hold `.x` file templates, not types, and Ghidra's preprocessor rejects them.

The result was built with Ghidra 12.1.4 and zig 0.13.0 headers. It contains 1,648
function signatures and no placeholder structs. `IDirect3DDevice9Vtbl` has 119 entries
(476 bytes) and `IDirect3DTexture9Vtbl` 22, matching the interfaces. Ghidra drops
`const`, so `srcdata` in `D3DXCreateTextureFromFileInMemoryEx` appears as `void *`.

## Applying it

In the Data Type Manager use **Open File Archive...**, then right-click the archive and
choose **Apply Function Data Types** to type the D3D9 and D3DX9 imports. For COM calls,
retype the variables holding the `Direct3DCreate9` result, the `CreateDevice` output and
textures as `IDirect3D9 *`, `IDirect3DDevice9 *` and `IDirect3DTexture9 *`. Vtable
offsets to look for before retyping: `IDirect3DDevice9::CreateTexture` is `0x5c`,
`IDirect3DTexture9::LockRect` is `0x4c`.

## Import coverage

```sh
/path/to/ghidra/support/analyzeHeadless /tmp/ec-ghidra coverage \
  -import /path/to/UOSA.exe -noanalysis -deleteProject \
  -scriptPath /absolute/repo/scripts/dev-helpers/ec-research/ghidra-d3d9-types \
  -postScript ImportSignatureCoverage.java /tmp/uosa-coverage.md \
  /absolute/repo/scripts/dev-helpers/ec-research/ghidra-d3d9-types/results/d3d9_x86.gdt
```

The report matches import names against function definitions in `windows_vs12_32.gdt`
and the extra archives. For the tested executable every D3D9 and D3DX9 import is
covered. The uncovered imports are unrelated to texture loading: Miles Sound System
(`MSS32.DLL`, no public headers; its stdcall-decorated names give the argument byte
count), the `DBGHELP.DLL` crash-dump calls, `DirectDrawCreateEx`, `DirectInput8Create`,
`SHGetFolderPathA` and three Winsock 2 calls. mingw-w64 has headers for all of these
except Miles if they are ever needed.

## Wine DLLs as Ghidra libraries

Wine's 32-bit PE DLLs (`<prefix>/drive_c/windows/syswow64` in a win64 prefix) work as
Ghidra library search paths, and they contain DWARF, so analysing a loaded library
gives Wine's internal names and types. Libraries are saved to the project only with
**Load Libraries From Disk** enabled; otherwise the ordinal lookup still loads them
temporarily and logs `Library not saved to project.`

Ghidra names imports by ordinal from cached `.exports` files under
`~/.config/ghidra/<version>/symbols/win32/`. Built from a Wine DLL, those files take the
DWARF symbol at each export, such as `_WSAStartup@8` or
`___wine_stub_WSCUpdateProvider`, so ordinal imports no longer match the signature
archives. In the tested `UOSA.exe` only `WS2_32.DLL` is imported by ordinal. Restore the
export-table names before importing; the original is kept with an `.orig` suffix:

```sh
python3 -I scripts/dev-helpers/ec-research/ghidra-d3d9-types/fix_wine_exports.py \
  ~/.wine/drive_c/windows/syswow64/ws2_32.dll \
  ~/.config/ghidra/ghidra_12.1.4_DEV/symbols/win32/ws2_32.exports
```
