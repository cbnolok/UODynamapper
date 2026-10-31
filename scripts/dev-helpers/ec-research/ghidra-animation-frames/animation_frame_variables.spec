# Variable spec for the functions tagged UO_ANIMATION_FRAME by NameAnimationFrameCode.java; applied by
# ../ApplyVariableSpecs.java to UOSA.exe and UOKR.exe. The types repeat, field for field, the ones of
# ../ghidra-tileart/tileart_variables.spec, ../ghidra-effects/NameEffectCode.java (MsvcVector) and
# ../ghidra-animation/animation_variables.spec, so that the spec also works on UOKR.exe, where those scripts
# were never run; on UOSA.exe they are unchanged. The prototypes are the UOSA.exe ones, checked at the
# instruction level on UOKR.exe (same registers, stack slots and RET purges). The UOMobFrameSet constructor is
# left out: UOSA.exe passes the object on the stack, UOKR.exe in ESI (see results/uokr_animation_frames.md).
tag UO_ANIMATION_FRAME

# ---- shared types ----

type /UO/SharedPtr 0x8 : boost::shared_ptr passed by value: object pointer and reference count block
  0x0 void* px
  0x4 void* pn
end

type /UO/AssetBlob 0xc : asset data read by UOAssets::StreamRead (only the fields it reads; the object may be larger)
  0x4 byte* data
  0x8 uint size
end

type /UO/AssetBlobSharedPtr 0x8 : shared_ptr<AssetBlob> passed by value to the asset factories and parsers
  0x0 /UO/AssetBlob* px
  0x4 void* pn
end

type /UOEffects/MsvcVector 0x10 : MSVC 2008 std::vector (here of 8-byte shared_ptr effect definitions unless noted)
  0x0 void* allocator
  0x4 void* first
  0x8 void* last
  0xc void* end
end

type /UO/AnimationFrameFile 0x28 : header of an AnimationFrame*.uop entry (the client reads 0x18..0x27 only)
  0x0 uint signature : 'AMOU'
  0x4 uint version : 1 in every file
  0x8 uint total_size : the entry size
  0xc uint animation_id : body of the entry name, never read
  0x18 uint colours_count
  0x1c uint colours_offset : from the header start; 4-byte entries [b0, b1, b2, keep colour flag]
  0x20 uint frames_count
  0x24 uint frames_offset : from the header start; sorted by (action, number)
end

type /UO/AnimationFrameEntry 0x10 : frame entry of an AnimationFrame*.uop file
  0x0 ushort action
  0x2 ushort number
  0x4 short x0 : box relative to the anchor
  0x6 short y0
  0x8 short x1
  0xa short y1
  0xc uint offset : RLE data at the entry address + offset
end

# ---- functions ----

function UOAnimationFrameSetFactory::Create
  proto __thiscall /UO/SharedPtr* custom
  param out_asset /UO/SharedPtr* @stack:0x4
  param asset_id uint @stack:0x8
  param asset_type uint @stack:0xc
  param blob /UO/AssetBlobSharedPtr @stack:0x10
end

function UOAnimations::FindFrameEntry
  proto __stdcall /UO/AnimationFrameEntry* custom
  param frame_file /UO/AnimationFrameFile* @EAX
  param action uint @stack:0x4
  param number uint @stack:0x8
end

function UOHues::GetHueTable
  proto __cdecl uint* custom
  param hue uint @ECX
end

function UOAnimations::BuildFramePalette
  proto __cdecl /UO/SharedPtr* custom
  param frame_file /UO/AnimationFrameFile* @EAX
  param hue uint @ECX
  param out_palette /UO/SharedPtr* @stack:0x4
end

function UOAnimations::DecodeFrameRle
  proto __cdecl void custom
  param entry /UO/AnimationFrameEntry* @EAX
  param palette uint* @stack:0x4
  param canvas uint* @stack:0x8
  param pitch int @stack:0xc
end

function UOAnimations::DecodeFrameRleMirrored
  proto __cdecl void custom
  param entry /UO/AnimationFrameEntry* @EAX
  param palette uint* @stack:0x4
  param canvas uint* @stack:0x8
  param pitch int @stack:0xc
end

function UOAnimations::DecodeFrameMask
  proto __cdecl void custom
  param entry /UO/AnimationFrameEntry* @EDX
  param mask byte* @stack:0x4
  param pitch int @stack:0x8
  param mark byte @stack:0xc
end

function UOAnimations::GetLayerBounds
  proto __cdecl void custom
  param layers /UOEffects/MsvcVector* @EAX
  param out_min_x int* @stack:0x4
  param out_min_y int* @stack:0x8
  param out_max_x int* @stack:0xc
  param out_max_y int* @stack:0x10
end

function UOAnimations::BuildFrameAtlas
  proto __stdcall void*
  param - void*
  param out_atlas void*
  param layers /UOEffects/MsvcVector*
end
