# ghidra-texture-roles

Does `UOSA.exe` (Enhanced Client) ever infer a texture's role from its file name or path? Or does it bind
each tileart texture item to a shader input by data alone? The question comes from the converter rule
`stable_role_from_tileart_texture_path` in `lib/udd-conv/src/metadata/tilemeta.rs`, which assigns roles from
substrings of the file stem ("normal"/"_n", "alpha", "mask", "noise", "detail", "light"/"glow").

Findings: [results/uosa_texture_roles.md](results/uosa_texture_roles.md) has the binding chain with
addresses (shader definition, material builder, shader maps, per-shader stages and samplers), the string
sweep, a verdict for each name rule, and a census of the current tileart data by shader and item position.
[results/uosa_texture_roles_names.md](results/uosa_texture_roles_names.md) is the script report of the first
`--apply` (9 functions renamed, tagged and commented); a second `--apply` reports `changes: none`.

In short: the client never tests a texture file name to choose its role. The shader name in the block picks
the shader class. The item position is the shader map index, and each shader class reads its maps by fixed
index. The only path test is the folder test that picks the package and asset type for loading.

Earlier work used here: `ghidra-tileart` (record format, texture items, `UOShaderDefinition`, material
builder), `ghidra-image-decoders` (texture request chain) and `docs/agent_notes/ec_original_water_code.md`
(water and static terrain stage operations).

## NameTextureRoleCode.java

GhidraScript, category `ECResearch`. Built on `ghidra-image-decoders/NameImageDecoderCode.java`.

```
NameTextureRoleCode.java OUTPUT_FILE [--apply]
```

- Names fixed addresses, so it refuses any program whose SHA-256 is not the tested `UOSA.exe`
  (`5a5e9dc5...ebad50`).
- Before naming, each address is checked: a function exists there, and its stated evidence holds (vtable
  slot, callers, callees, constants, referenced strings). A failed check is reported as a mismatch and
  nothing is named. The shader vftables have no labels, so their slots are checked with `vtslot:ADDR:N`
  (addresses resolved from the MSVC RTTI type descriptors).
- `OUTPUT_FILE` is created with CREATE_NEW, so an existing file stops the run before any change. Pass
  spaces in the path as `%20`.
- Without `--apply` the script only reports. With `--apply` it:
  - renames DEFAULT (`FUN_...`) functions with source ANALYSIS, creating namespaces as needed;
  - adds the tag `UO_TEXROLE`;
  - sets the plate comment `NameTextureRoleCode: UO_TEXROLE: <evidence>` when the comment is empty, and
    refreshes its own comment from an earlier run when the evidence has changed.
- Names with another source type are kept and reported. A name already used in the namespace is reported as
  a conflict.

Run it through ghidra-mcp, then save the program:

```
run_ghidra_script(program="UOSA.exe", script_name="<abs path>/NameTextureRoleCode.java",
                  args="<scratch>/texture_role_names.md --apply")
save_program(program="UOSA.exe")
```

The probe scripts used for the sweep are kept outside the repo in `~/ghidra_scripts/ProbeTexRoles01.java`
to `ProbeTexRoles04.java`. They cover role-token literals and their users, the call sites of the C runtime
string helpers and of the path finder `FUN_0058d9d0`, per-function literal scans, and the RTTI vtable
resolution.
