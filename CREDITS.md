# Credits and references

### UO Files

- Good ol' [Heptazane wiki](https://uo.stratics.com/heptazane/fileformats.shtml#3.8)
- Lots of C++ tools/libs i wrote in the years: uocf and uopp (mainly wrote to be used by Leviathan), UO ETE (Enhanced Tileart Editor), UO EMC (Enhanced Map Converter), huesmul2uop.
- ClassicUO: "zlib_bwt" codec.  
- KR facet format from ManawydanMapConverter.
- Lots of EC file formats from [Kons' wiki](https://code.google.com/archive/p/kprojects/wikis).
- Stefanomerotta's proof of concept [UO Client](https://github.com/stefanomerotta/UOClient/) for guidance on animated land textures shaders.
- UO-EC-Super-Viewer: xml source data for AnimationsCollection, AudioCollection, MultiCollection.
- UOFiddler: audio, fonts.

### Algorithms

Used as references or sources for Rust translation, which has been incrementally optimized further.

- BC7 codec: [basis-universal implementation](https://github.com/BinomialLLC/basis_universal/tree/master/transcoder)
- RDO implementation: [richgel999/bc7enc_rdo](https://github.com/richgel999/bc7enc_rdo) and [richgel999/bc7enc_rdo_devel](https://github.com/richgel999/bc7enc_rdo_devel/blob/master/bc7enc.cpp#L3711)
- Upscalers:
  - NEDI: [Python implementation](https://github.com/Kirstihly/Edge-Directed_Interpolation)
  - Kopf-Lischinski Depixelizer: [Python implementation](https://github.com/vvanirudh/Pixel-Art/tree/master)
  - AMD FidelityFX 1.1 (EASU and RCAS passes): [Official C implementation](https://github.com/GPUOpen-Effects/FidelityFX-FSR).
  - xBRZ: [Rust crate](https://docs.rs/xbrz-rs/latest/xbrz/)
  - Super-xBR: [hansonw/super-xbr](https://github.com/hansonw/super-xbr)
  - ScaleFX: [Themaister/slang-shaders ScaleFX](https://github.com/Themaister/slang-shaders/tree/master/scalefx)
  - Deblur: [guest(r) `deblur.glsl`](https://github.com/libretro/glsl-shaders/blob/master/deblur/shaders/deblur.glsl)
  - OmniScale: [Themaister/slang-shaders OmniScale](https://github.com/Themaister/slang-shaders/tree/master/omniscale)
  - Jinc2: local Hyllian [GLSL passes](https://github.com/mrgriscom/emu/blob/master/configs/retroarch/shaders/shaders_glsl/windowed/shaders/) and [RiskyJumps revised Jinc2 shaders](https://forums.libretro.com/t/jinc-2-fixes/21146)
  - Cheap Upscaling Triangulation (CUT): [Swordfish90/cheap-upscaling-triangulation](https://github.com/Swordfish90/cheap-upscaling-triangulation)
  - lqx: [PixelScalerWin repo](https://github.com/daelsepara/PixelScalerWin) and [Libretro implementation](https://github.com/libretro/RetroArch/blob/master/gfx/video_filters/lq2x.c)
  - hqx: [C++ implementation](https://github.com/brunexgeek/hqx)
  - epx: [PixelScalerWin repo](https://github.com/daelsepara/PixelScalerWin) and [Libretro implementation](https://github.com/libretro/RetroArch/blob/master/gfx/video_filters/epx.c)
  - 2xSaI, Super2xSaI, and SuperEagle (SAI variants): [PixelScalerWin repo](https://github.com/daelsepara/PixelScalerWin) and [Libretro implementations folder](https://github.com/libretro/RetroArch/blob/master/gfx/video_filters/)
  - MMPX-style: [Rust crate](https://crates.io/crates/mmpx).
