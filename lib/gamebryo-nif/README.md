# gamebryo-nif

Reads and writes Gamebryo NIF files. The block, struct and enum types are generated from our own
schema ([`codegen/schema/`](codegen/schema)), written from the stream loading code
(`LoadBinary`) of the Gamebryo 2.6 SDK and verified against real files. Field names are the SDK
member names in snake_case (`m_kPropertyList` -> `property_list`). The crate is generic Gamebryo,
not specific to Ultima Online, and has no dependencies.

Reading a file and writing it back without changes gives the same bytes. This was checked on all
466 NIFs in the Enhanced Client's `EffectTexture.uop`.

## Supported

- **Version**: 20.0.0.4, user version 0, little-endian. This version has no block size table and
  no string table, and stores strings inline. Other versions are rejected with
  `ErrorKind::UnsupportedVersion` or `ErrorKind::UnsupportedUserVersion`.
- **Block types**: the 73 types in [`codegen/allowlist.txt`](codegen/allowlist.txt). These are the
  ones the EC effect NIFs use: nodes, geometry, properties, interpolators and controllers, plus
  the `NiPSys*` particle systems, emitters, modifiers and controllers. The other light types
  (`NiAmbientLight`, `NiPointLight`, `NiSpotLight`) and the light controllers
  (`NiLightColorController`, `NiLightDimmerController`) are included too, although the shipped
  files use only `NiDirectionalLight`. Their ancestors (28 more objects), 25 structs and 50
  enums/bitflags/bitfields are generated with them.
  A block of any other type fails with `ErrorKind::UnknownBlockType { name }`.

## API

```rust
use gamebryo_nif::{Block, KeyType, NiFloatData, NiNode, NiSourceTexture, NifFile};

let mut nif = NifFile::read(&bytes)?;           // header, blocks: Vec<Block>, roots
for (index, node) in nif.blocks_of::<NiNode>() { // NiNode and subtypes (NiBillboardNode, ...)
    // Objects embed their parent as `base` and Deref to it: `node.name` is NiObjectNET's field.
    for child in &node.children {                // Vec<Ref<NiAVObject>>
        let _child = nif.get(*child);            // Option<&NiAVObject>, type-checked
    }
}
if let Some(texture) = nif.blocks[3].get_mut::<NiSourceTexture>() {
    texture.filename.set("textures/fx.dds");
}
let mut data = NiFloatData::default();           // defaults are the SDK constructors' values
data.key_type = KeyType::BEZKEY;                 // keys: Vec<NiKey<f32>>, num_keys: u32
nif.blocks.push(Block::from(data));
let bytes = nif.write()?;                        // Result: array lengths are checked
```

- `Block` is an enum with one variant per block type. `BlockType` lists the type names and their
  ancestry. Each object type implements `NiObjectType`, so links resolve to the right typed view.
- `Ref<T>` / `Ptr<T>` are typed block indices. `-1` is none. `NifFile::link_issues()` lists links
  that point at a missing block or at a block of the wrong type.
- Enums, bitflags and bitfields are newtypes over their storage integer, with named constants.
  Bitfields also get getters and setters. Values the file stores but the SDK does not name are
  kept as they are. Field names that are Rust keywords are raw identifiers (`r#static`, `r#type`,
  `r#ref`); their reflected names have no `r#`.
- Strings are `NifString` (raw bytes). `NiBool` is the SDK's one-byte boolean, kept raw because
  files store values other than 0 and 1.
- Errors (`NifError`) carry the kind, the byte offset, the block index and type, and the field
  path. For example:
  `block 12 (NiNode), field rotate.m01, offset 0x7c: unexpected end of data (need 4 bytes, 1 left)`.

### Reflection (for editors)

`Block::fields()` / `fields_mut()` (and the `Reflect` trait on every block and struct) list
every field in file order. Each field comes with:

- `info`: the schema name, type, the type that declares it, the length expression and the
  condition text.
- `present`: whether the field's condition holds for the current values.
- `value`: a typed view, either `Value` or `ValueMut`. The views are integers, `F32`, `Bool`,
  `String`, `Enum` (as `&dyn NifEnum`, with its options or bitfield members), `Link`, `Struct`
  (nested fields; `type_name()` tells colors and vectors apart) and `List` (with insert,
  duplicate and remove).

With these, a property grid can show and edit every block type, including key arrays such as
`NiKey<NiColorA>`, with no code for individual types.

Conditional fields always exist in the Rust struct. When the condition is false, the field is not
read or written and keeps its default value. Turning on the controlling flag (for example
`has_texture_transform`) makes the field present again. Lists that the SDK stores with their own
count in front (`ReadMultipleLinkIDs`, or a count followed by the elements) have type `T[u32]`
and no `info.length`: they are written with their current length. Other lists take their length
from other fields, such as `vertices` or `num_keys`. After resizing one, update the fields named
in `info.length`, or `write()` reports `LengthMismatch` with the field path.

## Layout

```
codegen/
  generate.py        generator (Python 3.9+ standard library only); the schema format is
                     described in its docstring
  allowlist.txt      block types to generate (ancestors, structs and enums follow)
  schema/            the stream layout of each class, written from the Gamebryo 2.6 SDK
    basic.schema       math types (NiPoint3, NiMatrix3, NiColorA, NiQuatTransform, ...)
    nimain.schema      scene graph, properties, extra data, effects, textures, geometry
    nianimation.schema interpolator controllers, interpolators, keys, key data
    niparticle.schema  particle systems, modifiers, emitters, controllers
src/
  lib.rs             crate docs, re-exports, the enum trait macro
  file.rs            NifFile, header and footer (hand-written for 20.0.0.4), link checks
  io.rs              Reader/Writer, the NifIo trait, field and array helpers
  types.rs           NifString, NiBool, Ref/Ptr, NiObjectType
  reflect.rs         Reflect, Field/FieldInfo, Value/ValueMut, NifEnum
  error.rs           NifError with offset, block and field path
  generated/         @generated: enums.rs, compounds.rs, objects.rs, blocks.rs, mod.rs
  tests.rs           unit tests on synthetic files
tests/real_files.rs  ignored test: round trip of real files
```

## The schema

Each class is described the way its `LoadBinary` reads it, with the SDK source cited:

```
# m_kEffectList is a NiTPointerList<NiDynamicEffect*>: the node does not own its effects.
object NiNode : NiAVObject from "CoreLibs/NiMain/NiNode.cpp"
    children: Ref<NiAVObject>[u32]
    effect_list: Ptr<NiDynamicEffect>[u32]
```

- Field names are the SDK member names without their prefix, in snake_case. Values read into
  locals take the local's name (`NiBool bHasMap` -> `has_map`). Defaults are the values the SDK
  constructors set.
- `Ref<T>` is a link the object owns (a smart pointer, or a ref-counted list), `Ptr<T>` one it
  does not (a raw pointer, or a `NiTPointerList<T*>`). `T[u32]` is a list stored with its count in
  front, as `ReadMultipleLinkIDs` does.
- Version checks (`NiStream::GetVersion(...)`) become `since`/`before` gates. Gates outside the
  range the schema covers are left out.
- Where the SDK reads a stream in a loop or through a table of key classes, the schema spells the
  result out:
  - `NiTexturingProperty` reads `map_count` map slots, the bump map slot as a `BumpMap`. The
    schema has one field per slot up to the bump map, then the remaining slots (the decal maps,
    before 20.2.0.5) as a list.
  - Key arrays are `NiKey<T>` (`time`, `value`, the Bezier tangents and the TCB parameters, each
    present for its key type) and `NiRotKey`; the data block passes its key type as `arg`.
    Euler rotation keys (`NiEulerRotKey`) hold three float key lists each, as in the SDK.
  - `NiGeometryData` stores three normal arrays in `normal` when an NBT method is set in
    `data_flags`, and the texture coordinate sets as rows of `vertices` entries.
- `NiPixelFormat` is a struct inside `NiPixelData` (it is a member there, not a streamed object).
- Objects that are only link targets (`NiCollisionObject`, `NiSkinInstance`,
  `NiAdditionalGeometryData`, `NiPalette`) are declared `extern`; they become uninhabited marker
  types, and a block of their type cannot be read.

The layouts were checked by the real-file test below: every file reads and writes back byte for
byte, and every link points at a block of the declared type.

## Regenerating

```sh
python3 -I lib/gamebryo-nif/codegen/generate.py
```

The defaults reproduce the checked-in `src/generated/` byte for byte. The output is
deterministic. Options are `--version`, `--user-version`, `--schema`, `--allowlist` and `--out`.
The generator refuses versions outside the range the schema covers, 10.1.0.114 (the oldest the
SDK loads) up to but not including 20.1.0.1 (where strings move to a string table).

How the generator works:

- It resolves every `since`/`before` gate for the requested version. The structs then hold
  exactly the fields that version stores.
- `if` conditions, array lengths and `arg` values become Rust expressions over the struct's
  fields. A field that is not stored evaluates to its default.
- `arg` is passed down at read and write time, for example a key list's key type to each key.
- A field declared more than once under one name (alternative positions or conditions) is one
  Rust field.
- Generic structs (`NiKey<T>`, `TexturingMapSlot<T>`) become Rust generics; structs with a parent
  (`TexturingBumpMap : TexturingMap`) embed it as `base` and deref to it, like objects.
- Unsupported constructs stop the generator with an error rather than producing wrong code:
  jagged arrays, nested generic arguments and unknown names in expressions.

**Adding block types**: describe them in the schema from the SDK's `LoadBinary`, add them to
`allowlist.txt`, regenerate, and extend the tests.

**Adding a version**: generate it into its own module (`--out`). The header in `file.rs`, and
the choice of which module `lib.rs` re-exports, would then need to be per version.

### License

The crate is Apache-2.0, like the repository. The schema is our own description of the stream
format, derived from the Gamebryo SDK's stream loading code; it contains no third-party format
description. The generated code contains only what the schema states.

## Tests

```sh
cargo test -p gamebryo-nif
# Real files, from the client package (or GAMEBRYO_NIF_DIR=<dir of extracted files>):
UO_EC_CLIENT_DIR=/path/to/enhanced/client \
  cargo test -p gamebryo-nif --test real_files -- --ignored --nocapture
```

The unit tests use synthetic files. They cover a hand-written file read and written, conditional
fields and key arguments, error locations, length checks, link checks, reflection reads and
edits, and the SDK constructor defaults.

The real-file test checks every NIF for a byte-exact round trip and for valid links. When it
reads `EffectTexture.uop`, it also checks the file count, the block count and the per-type block
counts against the survey of that package.

## Limitations

- Only one version (20.0.0.4 / user 0) is generated. The header and footer are written by hand
  for it. Versions with block size tables or string tables would need that support added.
- The schema covers versions 10.1.0.114 to 20.1.0.0; the crate is generated for 20.0.0.4.
- Only allowlisted types can be blocks. These ancestors, which the SDK can also stream, are
  generated as structs but are not readable as blocks: `NiExtraData`, `NiParticles` and
  `NiParticlesData`. Add them to the allowlist if a file uses them.
- Counts that other fields hold (`vertices`, `num_keys`, `map_count`, ...) are not updated
  automatically: `write()` checks them and reports any mismatch.
- Values the SDK fixes up after reading (for example deprecated apply modes, or the texture
  transform controller's map index before 20.2.0.5) are kept as stored.
- Not supported: big-endian files, string tables, jagged arrays.
- Build cost in the dev profile is about 3.5 s for a clean `cargo build -p gamebryo-nif`,
  0.9 s for `cargo check`, and about 15k generated lines.
