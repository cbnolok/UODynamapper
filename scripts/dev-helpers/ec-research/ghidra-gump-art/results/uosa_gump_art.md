# UOSA.exe: single Interface.uop gumps outside the paperdoll

Program `UOSA.exe` (SHA-256 `5a5e9dc536c836d1bf10b6f86bb88f61a6ae078f519d4060d3bc47aecbebad50`).

- **confirmed** = read in the decompiler and checked in the assembly.
- **decompiler** = read in the decompiler only.
- **inferred** = deduced from the code together with the Gamebryo SDK, Direct3D 9 behaviour or the shipped data.

SDK references are to Gamebryo 2.6 (`/home/claudio/Scaricati/gamebryo-v2.6-master/CoreLibs/`). The 3.2 sources were
not needed.

## Functions

Named by `NameGumpArtCode.java` (tag `UO_GUMP_ART`) unless marked "earlier".

| Address | Name | Notes |
|---|---|---|
| 0x0051af10 | `UOGumpArt::RequestGumpArtTexture` | the single entry point for gump art as a texture (item 1) |
| 0x00519700 | `MakeGumpTextureLocation` | earlier: `Data/Interface/Default/Textures/GumpArt/` + `%08d.tga` |
| 0x0051cfd0 | `UOLua::RequestGumpArt` | earlier (ghidra-lua-names) |
| 0x00900c72 | `MythUi::RegisterUiTexture` (probable) | (name, NiTexture) into the UI texture list |
| 0x00911701 | `MythUi::Texture::Texture` | wrapper of the NiTexture, no pixel change |
| 0x004dada0 | `GenericGump::BuildLayout` (probable) | the generic gump layout parser |
| 0x004dded0 | `GenericGump::AddGumpPic` (probable) | `gumppic` |
| 0x004de1c0 | `GenericGump::AddResizePic` (probable) | `resizepic` |
| 0x004e2960 | `GenericGump::CreateImage` (probable) | DynamicImage from `GenericGumpImageTemplate` |
| 0x004e2b80 | `GenericGump::CreateImageSlice` (probable) | `picinpic` |
| 0x004e2cb0 | `GenericGump::CreateButton` (probable) | `button`, `checkbox`, `radio`, `buttontileart` |
| 0x004e6c50 | `GenericGump::SetAlphaOfItemsInRect` (probable) | `checkertrans` |
| 0x004e3600 | `GGumpItemGumpPic::GGumpItemGumpPic` | |
| 0x004e3b50 | `GGumpItemGumpPicInPic::GGumpItemGumpPicInPic` | |
| 0x004e3db0 | `GGumpItemResizePic::GGumpItemResizePic` | resizepic ids 0x7778, 0x805, 0x806 |
| 0x004e4050 | `GGumpItemGumpPicResize::GGumpItemGumpPicResize` | every other resizepic id |
| 0x004e42d0 | `GGumpItemGumpPicResize::AddPiece` (probable) | |
| 0x004e71e0 | `GGumpItemGumpPicTiled::GGumpItemGumpPicTiled` | |
| 0x004e7400 | `GGumpItemVirtue::GGumpItemVirtue` | |
| 0x004e6b60 | `GGumpItemCheckerTrans::GGumpItemCheckerTrans` | |
| 0x004e4a30, 0x004e4b50, 0x004e4da0, 0x004e6e40 | `GGumpItemButton`, `GGumpItemCheckBox`, `GGumpItemRadioButton`, `GGumpItemButtonTileArt` constructors | |
| 0x008f47d9 | `MythUi::DynamicImage::SetTexture` (probable) | |
| 0x008f4826 | `MythUi::DynamicImage::RebuildScreenTexture` (probable) | |
| 0x008f4bd3 | `MythUi::DynamicImage::SetCustomShader` (probable) | |
| 0x008e58e6 | `MythUi::Window::SetTintColor` (probable) | |
| 0x008e563b | `MythUi::Window::SetAlpha` (probable) | |
| 0x00943724 | `MythUi::ScreenTexture::ScreenTexture` | the constructor DynamicImage uses (0x009435a1 is a second one, not named) |
| 0x00943cb6 | `MythUi::ScreenTexture::BuildGeometry` (probable) | |
| 0x00944687 | `MythUi::ScreenTexture::AttachProperties` (probable) | shader and render properties of every UI quad (item 6) |
| 0x009d3300 | `SimpleFFP::SimpleFFP` | the default UI shader |
| 0x00419f70, 0x0041a470, 0x0041a2b0, 0x00416bf0 | `NiAlphaProperty`, `NiZBufferProperty`, `NiStencilProperty`, `NiVertexColorProperty` constructors | |
| 0x00778160 | `NiDX9TextureData::FindMatchingPixelFormat` | |
| 0x004ccc80, 0x0068a8b0, 0x00770b90 | `UOTextureData::UOTextureData`, `NiSourceTexture::Create`, `NiDX9SourceTextureData::Create` | earlier (ghidra-image-decoders) |

## Drawing paths

1. **One entry point.** `RequestGumpArtTexture(id & 0xffff, out name, out width, out height)` is the only code that
   turns a gump id into a texture. It keeps a cache keyed by the id with a reference count. On a miss it builds the
   location with `MakeGumpTextureLocation` and asks `TextureManager::RequestTexture` for it, reads the width and
   height, and registers the texture with the UI texture list (`RegisterUiTexture`) under the name
   `%08d_GumpArt`. It never touches the pixels. **confirmed** (decompiler; the calls and the string checked by the
   naming script).
2. **Callers.** It has 11 call sites, all direct calls, in three families. **confirmed** (cross-references):
   - **(a) The paperdoll:** `UOPaperdoll::BuildImage` 0x0050c790 and `DrawGumpLayer` 0x00509b30. This is a CPU
     composite with the first texel as colour key; see ghidra-paperdoll items 15 and 16.
   - **(b) Lua:** `UOLua::RequestGumpArt` 0x0051cfd0 returns (texture name, width, height) to the scripts (item 9).
   - **(c) Generic server gumps:** the `GGumpItem*` constructors in the table above. `GenericGump::BuildLayout`
     parses the layout commands. It is called by 0x004da970 (new gump), 0x004daaa0 (rebuild of an existing
     `GenericGump%d` window) and 0x004da8a0 (all gumps, on UI reload). That these are the 0xB0 and 0xDD
     handlers, through 0x0062b850, is **inferred**: the packet dispatch was not traced.
3. **Generic gump commands that draw gump art.** **decompiler** for each:
   - `gumppic` → `GGumpItemGumpPic`, or `GGumpItemVirtue` when the class argument is `VirtueGumpItem`.
   - `resizepic`:
     - ids 0x7778, 0x805 and 0x806 → `GGumpItemResizePic`: one image, its size clamped to the texture.
     - every other id → `GGumpItemGumpPicResize`: nine images, ids id..id+8, laid out as corners, stretched
       edges and a stretched centre.
   - `picinpic` → `GGumpItemGumpPicInPic` (a texture slice).
   - `gumppictiled` → `GGumpItemGumpPicTiled`.
   - `button`, `checkbox`, `radio`, `buttontileart` → `CreateButton`: both state ids through the same request, as
     the textures of a `GenericGumpButtonTemplate` button.

   Each image is a DynamicImage made by `CreateImage` (`CreateImageSlice` for picinpic) from the
   `GenericGumpImageTemplate` window. The template is in `Source/GenericGump.xml` and sets no alpha, layer effect
   or texture flag.
4. **`checkertrans`** (`GGumpItemCheckerTrans`) has no texture. It multiplies by 0.5 the window alpha of every
   gump item that lies inside its rectangle (`SetAlphaOfItemsInRect` → `Window::SetAlpha`; the 0.5 is the float
   at 0x00c95604). **confirmed** (assembly 0x004e6b89 `FLD [0x00c95604]`, then the call at 0x004e6bb8).

## Texture creation

5. **Pixel format.** Every non-paperdoll path gets the `NiSourceTexture` of item 1.
   - **Preferences.** `UOTextureData::UOTextureData` creates it with FormatPrefs {`PIX_DEFAULT` (6),
     `ALPHA_DEFAULT` (3), mipmaps `NO` (0)}, and the TGA is decoded by `NiTGAReader`, which keeps an alpha channel
     only at 32 bits. **confirmed** (the preferences; see ghidra-image-decoders for the reader).
   - **Format choice.** `FindMatchingPixelFormat`, for an RGB (or palette) source whose layout is neither
     `COMPRESSED` nor `HIGH_COLOR_16`, tries the renderer's slot 2 first (`TEX_RGB888`), then 5 (`TEX_RGBA8888`),
     then the 16-bit slots. **confirmed** (switch table 0x007787ec: formats 0 and 2 go to 0x00778238; slot 2 is
     `[EBP+0x8]` at 0x0077832f, slot 5 is `[EBP+0x14]`).
   - **RGBA source.** An RGBA source takes slot 5 first. **decompiler**.
   - **D3D formats.** In the 2.6 SDK the slots are `TEX_RGB888` → `BGRX8888` = `D3DFMT_X8R8G8B8` and
     `TEX_RGBA8888` → `BGRA8888` = `D3DFMT_A8R8G8B8` (`NiDX9Renderer.cpp` lines 4193 and 4265;
     `NiDX9TextureData.cpp` line 322; `NiTexture.h` FormatPrefs). **inferred** for the binary's table.
   - **Result.** A 24-bit gump becomes a texture without alpha, and Direct3D 9 samples alpha 1.0 from it. If the
     device fell back to slot 5, the converter fills alpha with 255, so the result is the same. A 32-bit gump
     keeps its alpha byte unchanged. No DXT: the preferences do not ask for compression, and the location only
     names `%08d.tga`. **inferred** (D3D9 format rules plus the SDK converter).
   - **No fix-up.** Nothing between the file and the D3D texture keys a colour or rewrites alpha. The only colour
     key in the client's gump code is the paperdoll's CPU composite. **confirmed** for the functions read: the
     request, `UOTextureData`, `RegisterUiTexture`, `MythUi::Texture`, `CreateImage` and `DynamicImage::SetTexture`.

## Draw state

6. **The UI quad.**
   - **Construction.** `DynamicImage::SetTexture` → `RebuildScreenTexture` builds a `MythUi::ScreenTexture`
     (constructor 0x00943724 → `BuildGeometry` → `AttachProperties`). **confirmed** (calls).
   - **Properties.** `AttachProperties` gives every UI quad these properties. **confirmed** (decompiler and
     assembly 0x009447e8..0x00944810):
     - `NiAlphaProperty`: the constructor default 0xec, then `OR 0x1`, `AND 0xffed | 0xc`, `AND 0xfeff | 0xe0`.
       That gives blend **on**, source `SRCALPHA`, destination `INVSRCALPHA`, and the alpha **test bit never set**.
       The masks are from 2.6 `NiAlphaProperty.h`: `ALPHA_BLEND_MASK` 0x1, `SRC_BLEND_MASK` 0x1e,
       `DEST_BLEND_MASK` 0x1e0, `TEST_ENABLE_MASK` 0x200. It is straight (non-premultiplied) alpha.
     - `NiZBufferProperty` with test and write off (`& 0xfffc`).
     - `NiStencilProperty` for clipping (set later through 0x00944537..0x0094460a; only stencil bits).
     - `NiTexturingProperty` with the texture as base map and apply mode `MODULATE` (`& 0xfff5 | 4`).
     - `NiVertexColorProperty`, and an `NiMaterialProperty` whose diffuse and alpha 0x00944b08 later sets to the
       window tint and alpha.
   - **Shader.** It is the custom shader named by the quad when one is set, otherwise `SimpleFFP` (string
     0x00ce6508). `SimpleFFP` (vtable 0x00cfbdfc) is a one-pass fixed-function shader. **decompiler**.
   - **Inferred:**
     - The output colour is texture × tint.
     - The output alpha is texture alpha × window alpha.
     - It is blended over the frame with `SrcAlpha, InvSrcAlpha`.
     - The 2,118 ordinary gumps, whose transparency is alpha 0, depend on this.
7. **Hued generic gumps.** `CreateImage` with a non-zero hue sets the custom shader `UOSpriteUIShader` with
   parameters {hue} or {hue, second value}. It also sets the window tint (r, g, b, 255) from 0x004279e0, the
   function behind `UOLua::HueRGBAValue`. The quad keeps the same alpha property. What `UOSpriteUI.psh` (in
   `Shaders.uop`) does with alpha was not read. **decompiler** for the C++ part.
8. **Filtering.** Mipmaps are off (item 5). The texture filter of the UI quads was not traced. At 1:1 it does not
   matter. When a gump is scaled (`DynamicImageSetTextureScale`, or the stretched resizepic pieces), a bilinear
   filter on straight alpha mixes the RGB of alpha-0 texels into the edge texels.

## Lua side

9. **Scripts** (`UserInterface/Default/Source`, read only).
   - **Callers of `RequestGumpArt`:**
     - `ContainerWindow.lua` lines 125 and 607: the legacy container art. Lines 138–142 and 1423–1427 call
       `WindowSetDimensions`, `DynamicImageSetTexture(..LegacyContainerArt, texture, 0, 0)` and
       `DynamicImageSetTextureScale(.., scale)`.
     - `spellbook.lua` line 448: the legacy book gumps.
     - `CharacterAbilities.lua` line 45: gump 11010.
   - **The windows:**
     - The DynamicImage windows are plain (`ContainerWindow.xml` line 207: `layer="default"`, no alpha).
     - No script sets alpha, a tint or a shader on gump art.
     - `UOSpriteUIShader` is set from Lua only on item icons (`EquipmentData.lua` line 164, `HotbarSystem.lua`
       line 672 and `WindowUtils.lua` line 1876, each with {hue, objectType}).
   - **`GenericGump.lua`** only forwards events (`GenericGumpOnClicked` and the like). The images are built in
     C++ (items 2–3).
   - **The texture name.** The Lua-visible name is `%08d_GumpArt`. It exists only after `RequestGumpArt` (or a
     generic gump) has registered it. `UOLua::UOGetTextureSize` 0x00528370 looks names up in the same list.
   - **The bindings:** `UOLua::DynamicImageSetTexture` 0x008f4dc3, `DynamicImageSetCustomShader` 0x008f54d8
     (→ `DynamicImage::SetCustomShader`), `WindowSetAlpha` 0x008e9d2e (→ `Window::SetAlpha`) and
     `WindowSetTintColor` 0x008e9ea5 (→ `Window::SetTintColor`). **confirmed** (calls).

## The example gumps

The texel counts below were taken from the extracted `Interface.uop` in scratch. The data is not committed.

10. **1059** (190x140, 24-bit RLE TGA, 17,347 of 26,600 texels near black). The texture has no alpha, so every
    texel draws at full opacity. The client shows an **opaque black box** around the art. **confirmed** for the
    code path (no key, no alpha source, blend without test). **inferred** for the D3D format detail.
11. **5595** (383x383, 32-bit).
    - 64,758 texels are exactly (0, 0, 0) at alpha 255. Classic draws colour 0 of a gump as transparent.
    - The one-texel border (1,528 texels) has alpha 246, apart from a few texels at 238 or 248.
    - The client draws the black **fully opaque**, the rim at 93–96 % over what lies behind it. The result is
      an **opaque black backdrop**, not a translucent one. **inferred** from items 5–6 and the data.
12. **1621** (346x300, 32-bit).
    - All 35,673 exact-black texels have alpha 2 or 3, so they draw at about 1 % black. They are **effectively
      transparent**, as in Classic.
    - Its 16,886 other near-black texels have alpha 255. They are dark art, opaque in both clients.
    - **inferred** from items 5–6 and the data.

## For Dynamapper

To match the Enhanced Client for single gumps outside the paperdoll:

- Decode the TGA as stored: 24 bits gives alpha 255 everywhere, and 32 bits keeps the alpha byte.
- Draw with straight-alpha "over" blending (`src * a + dst * (1 - a)`). Use no alpha test and no threshold, and
  multiply by the window alpha and tint where the UI applies them.
- Apply no colour key: black stays black. 1059 is a black box and 5595 a black backdrop, which is what the
  official client shows.
- The first-texel RGB key belongs only to the paperdoll composite.
- `checkertrans` is a 0.5 multiplier on the alpha of the items inside its rectangle.

## Open questions

- What `UOSpriteUI.psh` and `UiShader.psh` (in `Shaders.uop`) do with alpha for hued gumps.
- The texture filter of UI quads (point or bilinear).
- The packet handlers above `GenericGump::BuildLayout` (0x0062b850 → 0x004da970 / 0x004daaa0), assumed to be 0xB0
  and 0xDD.
- Whether the shipped 24-bit and alpha-free black gumps are meant to look like that, or are conversion leftovers
  that the client simply shows as they are.
