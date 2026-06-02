# uocf-asset-cli

`uocf-asset-cli` contains asset-level tooling built on top of `uocf`.

## Commands

- `export-anim-patch`: export animation payloads as single-entry `.vd` files or
  Michelangelo/UOAnimTool `.uop` patch streams.

Supported animation sources:

- `classic-mul`: reads raw blocks from `anim*.idx` / `anim*.mul`.
- `cc-animation-frame`: reads Classic Client `AnimationFrame*.uop` entries.
- `ec-animation-frame`: reads Enhanced Client `AnimationFrame*.uop` entries.

Example:

```sh
uocf-asset-cli export-anim-patch \
  --source classic-mul \
  --idx anim.idx \
  --mul anim.mul \
  --block 7 \
  --format vd \
  --output body_7.vd
```

Use `--format michelangelo-uop` for the old Michelangelo/UOAnimTool patch
stream. This is not a modern Mythic UOP package.
