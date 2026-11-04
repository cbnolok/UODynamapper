# Miles playback reference

What a Rust playback path must do to sound like the clients. It describes how the Miles Sound System behaves, then
what the Classic, Kingdom Reborn and Enhanced clients actually ask of it. For what the decoder accepts, see
`scripts/dev-helpers/ec-research/ghidra-audio/results/miles_audio.md` and `docs/agent_notes/ec_audio_formats.md`.

Everything here is in our own words. The SDK is third-party and proprietary, so none of its code or tables belong in
this repository, and no reimplementation should be copied from it.

## How far to trust each line

Each finding carries one of these labels:

| Label | Meaning |
|---|---|
| **[7.2c]** | Read in the shipped Enhanced Client binary, or in code shown to be the same as the 7.2a code it was read from |
| **[7.0x]** | Read in the shipped Kingdom Reborn Miles binary, in Ghidra |
| **[7.2a]** | Read in the 7.2a SDK source only. Likely true for 7.2c, but not checked |
| **[client]** | Read in the client, in Ghidra. The addresses not given here are in `ghidra-audio/results/`: `uosa_audio.md`, `uokr_audio.md`, `classic_audio.md` |
| **not determined** | The Ghidra pass of 2026-11-03 did not settle it. The list is at the end |

All of it is a static reading. Nothing was run or measured, so a loudness figure here is arithmetic from the code, not a
comparison with a recording of a client.

The versions compared:

| Binary | Version | Build date | Code comparison |
|---|---|---|---|
| Enhanced `mss32.dll` (`b588b479...`) | 7.2c, rebuilt from source on a developer branch | 2008-07-27 | With 7.2a: 758 of 808 code blocks have the same instructions, ignoring constants and addresses |
| Enhanced `miles/mssmp3.asi` (`0fb784a9...`) | ships with 7.2c | 2008-04-01 | With 7.2a: all 46 code blocks match. Only 4 instructions differ, all field offsets near the start of the file (from 0x26f01260), consistent with a struct packing change |
| Kingdom Reborn `mss32.dll` (`272f65a0...`) | 7.0x | 2007-06-04 | With 7.2c: 573 of 808 blocks match |
| Kingdom Reborn `miles/mssmp3.asi` (`f6bbeccf...`) | ships with 7.0x | 2007-06-04 | With 7.2c: 38 of 46 blocks match, including the frame seek |

So the **decoder findings from 7.2a apply to 7.2c as they stand**. The `MSSMIXER` code section, which holds the
merge, resample and clip loops, is byte-identical between 7.2a and 7.2c (93.6% at the same offsets against 7.0x).
Blocks that differ in `mss32.dll` were looked at only where noted below. These comparisons used `objdump` on the
binaries, not a Ghidra pass, and they count as a static check.

Kingdom Reborn findings are weaker: where a line says [7.2c] it does not cover 7.0x unless it says so.

The Ghidra pass also carried the SDK's function names to the shipped binaries by comparing instructions, which placed
1,044 of 1,078 functions in Enhanced `mss32.dll`, 32 of 36 in its `mssmp3.asi`, 976 of 1,050 in Kingdom Reborn's
`mss32.dll` and 29 of 36 in its `mssmp3.asi`. Those names stay in the Ghidra project (`ghidra-audio/README.md`).

## Volume and pan

**`AIL_set_sample_volume_pan(volume, pan)`** **[7.2c]**, read in `mss32.dll` at 0x2112ce50:

- Gain is `volume ^ (5/3)`, so 0.5 is about -10 dB and 0.25 about -20 dB.
- Left is `gain * (1 - pan) ^ 0.3`. Right is `gain * pan ^ 0.3`. Pan 0 is left, 0.5 centre, 1 right.
- Centre pan is special-cased to the same factor on both sides, `0.5 ^ 0.3`, about 0.8123 or -1.8 dB. A sound panned
  fully to one side is therefore 1.8 dB louder on that side than the same sound at centre.
- On drivers with 4 or more logical channels, both sides take one more factor of 0.8123.

**Defaults [7.2a]:** `init_sample` sets volume 1.0 and pan 0.5, so an untouched sample plays at 0.8123 on both sides.

**`AIL_set_sample_volume_levels(left, right)` [7.2c]** (0x2112cd80) stores the two values as linear scalars. It applies
neither curve and no centre factor. A client that uses this call gets a different loudness law from one using volume
and pan.

**[7.0x]:** `volume_pan` (0x2112afb0) has the same `5/3` exponent and the same centre factor. Its pan exponent was not
re-read. `volume_levels` is at 0x2112aed0.

**Which law each client gets [client]:**

- **Enhanced** calls `volume_pan` with a constant volume of 0.81 and its pan, reads the levels back, multiplies both by
  its own volume and sets them (`SampleSound_ApplyVolume` 0x00898993). So `v ^ (5/3)` applies only to the constant.
  **A sound's volume is linear**, on a fixed factor of `0.81 ^ (5/3) * 0.8123`, about 0.572 or -4.9 dB, at centre.
- **Kingdom Reborn** sends a 2D sound's volume straight to `volume_pan` (`Sound_StartSample` 0x0072428d, call at
  0x00724458), so **the curve and the pan law apply**. Its 3D sounds set channel levels and its music sets
  `volume_levels`, both linear in the volume.
- **Classic** has no Miles, and works in DirectSound hundredths of a dB.

**How the levels combine [7.2a]**, for each output channel:

```text
dry   = master_volume * master_dry * sample_dry * (1 - obstruction)
level = channel_volume * dry * speaker_response * user_channel_level * auto_3D_channel_level
scale = truncate(2048 * level + 0.5), clamped to at most 2048
```

- Master volume is linear, with a default of 1.0. It has no curve.
- Scale tops out at unity, so no combination boosts a sound above its source level.
- `scale` is 11-bit, about -66 dB per step at the bottom.
- In 7.2c, the block that uses the 2048 and 65536 constants (0x21107220) differs from 7.2a by one field access and
  padding, so this arithmetic is very likely unchanged. A second user of 2048 (0x2112f5f6) changed and was not
  reviewed.
- The reverb send is `master_volume * master_wet * sample_volume * wet_level * (1 - exclusion)`, also multiplied by
  the 3D attenuation when automatic attenuation is on. Wet defaults to 0 and dry to 1, so reverb is silent until
  something raises it.

**Level changes [7.2c]:** a new level is ramped linearly over the first 32 samples of a buffer (preference 48, read
from the 7.2c defaults), which avoids zipper noise.

## Mixing and clipping

**[7.2c]** (`MSSMIXER` is byte-identical to the 7.2a code):

- Samples are signed 16-bit. Each voice adds `sample * scale` into a 32-bit accumulator per output channel.
- The final stage shifts right by 11 (arithmetic) and saturates to 16 bits. This is a **hard clip**.
- There is no limiter, compressor or per-voice-count normalisation. Loud overlapping sounds simply clip.
- A mono source is copied to both channels with its left and right scales. A stereo source's left and right scales
  act as a balance.
- A voice at zero volume keeps running and advancing. It is not dropped, so a sound can fade back in at the right
  place.

A floating-point mix that hard-clips at ±1.0 is close enough. Reproducing the 11-bit quantisation gains nothing
audible.

## Resampling

**[7.2c]** (preference defaults read at 0x211010e0, loops in `MSSMIXER`):

- The step is `playback_rate * doppler * 65536 / mixer_rate`, in 16.16 fixed point.
- **Within ±131/65536 of unity (about ±0.2%) no resampling happens**: the step is forced to exactly 1. A 22,048 Hz
  voice on a 22,050 Hz mixer therefore plays at the mixer's rate.
- The resampling filter is on by default (preference 31). Upsampling is linear interpolation between two samples.
  Downsampling is a box average weighted by each source sample's overlap.
- Without the filter, resampling takes the nearest sample. UOSA and UOKR do not import `AIL_set_preference`, so the
  default stands and the filter is on.

The MP3 decoder never resamples. It outputs at the file's own rate and always 16-bit, and the mixer handles the rate.

**Mixer rate [client].** Enhanced opens 48,000 Hz, 16 bits (`SoundSystem_OpenDriver` 0x008859f1). Kingdom Reborn opens
44,100 Hz, 16 bits (`AudioDevice_OpenDriver` 0x007241f3). Both are constants in `.data`, not options. Both pass spec
0x10, for which Miles takes the layout from the Windows speaker setting (0x21113f90 **[7.2c]**), stereo by default. A
failed open falls back: Enhanced to 44,100, 22,050 and 11,025, Kingdom Reborn by halving down to 11,025, then both to 8
bits.

So Enhanced resamples nearly everything, its 44,100 Hz music included, by linear interpolation. Kingdom Reborn leaves a
44,100 Hz file alone at multiplier 1 and doubles a 22,050 Hz one.

## MP3 start, end and looping

**[7.2c]** unless noted. The decoder is instruction-identical to the 7.2a source that this was read from.

**Headers are ignored.** Miles parses neither Xing nor LAME headers. None of the three `mssmp3.asi` builds contains
either string. Consequences:

- An Info frame decodes as a normal frame of silence, one frame long.
- The encoder delay (LAME usually records 576 samples) is **not** trimmed.
- The end padding is **not** trimmed.
- The decoder's own delay (about 529 samples, from the overlap and synthesis filters) is not compensated.
- A sample's length is frames × samples per frame, Info frame included. Our `MpegAnalysis::samples()` already counts
  this way, before any trimming.

**Start.** The first audible sample of a tagged file plays after the Info frame, then the encoder delay, then the
decoder delay, then the mixer's own latency. The DirectSound defaults are 8 ms fragments with 8 mixed ahead, so about
64 ms (read from the 7.2c preference defaults).

**End.** When the data runs out, the decoder pads the rest of its output buffer with zeros and the sample ends. A
trailing ID3v1 tag is skipped by the frame seek.

**Damaged data:**

- A frame that points back to reservoir data that isn't buffered outputs one silent frame rather than noise.
- Up to 8 KiB of junk between frames is scanned over silently. Each frame gets its own 8 KiB window.
- More junk than that, or the end of the file, ends the stream normally with no error.
- A last frame cut short still decodes. The zero-fill for a short read is written at the wrong place: at the buffer
  start plus the bytes read, instead of the write cursor plus the bytes read. So the frame decodes partly from stale
  bytes, and a brief glitch is possible. This code is the same in 7.0x and 7.2c. It answers open question 2 of
  `ec_audio_formats.md`, and it covers Kingdom Reborn's 13 cut files.

**Looping** with `AIL_set_sample_loop_count` (0 = forever, 1 = once, the default) **[7.2a for the sample-side code,
7.2c for the decoder]**:

1. At the end of the data, the position returns to the loop start, which is offset 0 of the image. A leading ID3v2
   tag was already stripped when the file was set, so it isn't read again.
2. The decoder is told to reset (`ASI_stream_seek` with -2). This clears the input buffer and bit reservoir only. The
   overlap and synthesis filter state carry on, so **there is no click from decoder state**.
3. Decoding restarts at the first frame, **Info frame included**.

**So a looped MP3 has a gap.** It is the file's end padding, then the Info frame's silence, then the encoder delay.
The decoder delay adds nothing, because its state is continuous. With LAME's usual 576-sample delay and before any end
padding:

| Rate | Frame | Info frame + 576 delay | Gap at least |
|---|---|---|---|
| 44.1 kHz (MPEG-1) | 1,152 | 1,728 samples | 39 ms |
| 32 kHz (MPEG-1) | 1,152 | 1,728 samples | 54 ms |
| 22.05 kHz (MPEG-2) | 576 | 1,152 samples | 52 ms |
| 16 kHz (MPEG-2) | 576 | 1,152 samples | 72 ms |

The 326 Enhanced MP3s with no Info frame lose the frame's share of this. The actual delay and padding of each file
are in its LAME tag, which our census did not collect. A gapless player would remove a gap the client has, so **a
matching player must not honour the LAME tag**.

## What the tag skip and the walks accept

What `lib/uocf/src/audio/validate.rs` simulates. **[7.2c]**: read in the 7.2a source and checked with `objdump` on
the shipped binaries, at the addresses given, not in Ghidra. **[7.2a]**: the source only.

**Leading ID3v2 skip [7.2c]** (`mss32.dll` 0x2112b68d to 0x2112b6ee; the sum is also in 7.0x, `miles_audio.md`). It
looks at the start of the data only. It wants `ID3`, both version bytes below 0xFF and all four size bytes below
0x80. It reads no flag, so unsynchronisation, an extended header and a footer change nothing. The skip is the three
upper size bytes shifted into place, ORed with the lowest byte plus 10. So a sum that reaches bit 7 is lost when the
next size byte is odd, and the scan starts 128 bytes inside the tag. A footer is not skipped: its 10 bytes are
scanned and count in the window, though they cannot hold a sync. The remaining size is not checked against the file,
so a tag longer than the file leaves the image.

**Frame scan and walk [7.2c]** (`mssmp3.asi` from 0x26f07080; the frame seek also matches 7.0x):

- One byte at a time, while the read position is at most 0x2000 past the window start.
- The newest three bytes read as `TAG`: 125 more are consumed, even past the end of the data, and the scan and window
  restart.
- The newest three as `ID3`: 7 more are read. A valid header (the same tests) skips its size, plus 10 when the footer
  flag is set here, and the scan and window restart. A short read ends the stream. An invalid header is passed over
  in the same window, and its 7 bytes are not looked at for a sync.
- The older two of the last four as a sync: bitrate index 15, rate index 3 or a layer other than III are passed over,
  in the same window.
- The first Layer III header sets the values later ones are compared with: version and sample rate. A later header that
  differs is lost and replaces them. Nothing looks for the next frame.
- Side information that does not fit in the data ends the stream ("Invalid MPEG audio stream"). So does running out of
  window or data, without an error, except at open, which fails.
- The next scan starts at the header plus the frame length, or at the end of the side information when that is further.
- Free-format (index 0) and reserved version bits are taken; a free-format frame has data size 0.

**WAV chunk walk [7.2c]** (`mss32.dll` 0x21129cc0 to 0x2112a010), named-file path with a `.wav` name. It never sees
the file's length.

- The bound is the start of the data plus the RIFF size, not plus 8. So in a well-formed file it falls 8 bytes before
  the end of the file, and a chunk starting at it or after is out.
- The first chunk's id is compared before any bound. After that, the step `size + (size & 1) + 8` wraps in 32 bits
  and the walk stops when the signed 32-bit difference between the next position and the bound is not negative. So a
  size can step back or in place, a RIFF size past the file lets it read outside, and a RIFF size of more than 2 GiB
  past the step stops it.
- `fmt ` and `data` are separate searches from the first chunk. The `fmt ` fields are read without looking at the
  chunk's size. The byte rate is never read.
- The `data` length is the chunk's own size, clamped to neither the RIFF size nor the file.
- IMA ADPCM (tag 0x11, 4 bits) also searches `fact`, over the whole RIFF size. Without one, the sample count divides
  by the block align (0x21129fea), so a block align of 0 faults. With one, nothing divides by it: block size 0 is stored
  (0x2112a098), the decoder's count of bytes left in the block wraps, and the whole data chunk is one block. A
  single-block file plays correctly. A file with real blocks has its later headers decoded as noise, not a fault. The
  position calls report 0.

**WAV types [7.2c]** (0x2112b4c0, 0x2112a030). Tag 1 takes any bit depth: 4 is treated as ADPCM, 16 as 16-bit and every
other depth as 8-bit, so 24 or 32-bit plays as noise. The block align is used only for ADPCM. Any other tag goes to a
provider whose "Input wave tag" matches, with raw output: `mssmp3.asi` declares 0x55 (0x26f07850), so an MP3-in-RIFF
`.wav` reaches it. `mssvoice.asi`, as Enhanced ships it, declares 0x74 (RT24), 0x75 (RT29) and 0x77 (VR12), the Voxware
MetaVoice codecs, so a WAV with one of those tags reaches them. Kingdom Reborn ships a different `mssvoice.asi`, which
was not read.

**Channels [7.2c]** (0x2112a030, 0x2112b3c0, 0x211298f0). More than 2 channels or a channel mask takes the multichannel
path, 2 is stereo, and anything else, **0 included, is mono**. A mask comes only from `WAVE_FORMAT_EXTENSIBLE`, which
needs block align = channels × 2, so a mono EXTENSIBLE file takes the multichannel path with one channel. A plain tag 1
file has no such test. 7.0x has no EXTENSIBLE branch **[7.0x]**: tag 0xFFFE goes to the provider search.

**Partial sample frames [7.2c]** (the stream-to-buffer routine 0x211305d0, the merge 0x2112f3f0). A sample ends only
when 0 bytes or fewer remain, so a short tail still gets one read of a sample frame, and the end is compared only
after the step. **The read goes at most `frame - 1` bytes past the data**, not a whole frame: 1 byte for mono 16-bit or
stereo 8-bit, 3 for stereo 16-bit. Those bytes are what follows in the image: a pad byte, the next chunk, or memory past
the image.

- With a zero pad byte, a mono 16-bit tail is one source frame of at most 255/32768, about -42 dB, which should be
  inaudible.
- With other bytes, it can be one source frame at up to full scale, a possible click at the very end.

This covers the no-resampling, nearest and upsampling loops. The downsampling loops and the size of the image's
allocation were not read. The validator still leaves such a file unverified.

## Streams and one-shot samples

**[7.2a]**. The stream code is among the 7.2c blocks that changed (at 0x211103b0; `AIL_set_file_callbacks` also gained
a parameter), so check these against 7.2c before relying on the details. The 2026-11-03 pass did not re-read this code,
in 7.2c or in 7.0x. It matters less than it seemed: only Kingdom Reborn's music streams, on 7.0x, and Enhanced's
`StreamSound` has no live path **[client]**.

| | `AIL_set_named_sample_file` | `AIL_open_stream` |
|---|---|---|
| Source | The whole file image in memory | Read through the Miles file callbacks |
| Buffers | None beyond the decoder's | Three, each about a third of a second of data, unless the file is short enough to preload |
| Leading ID3v2 | Always skipped first | Skipped only if opening fails, so a tag under 8 KiB is scanned over instead |
| Loop | As above | Pads with zeros to the block size, then resets the decoder in the same way |
| Starvation | Cannot happen | Heard as a silent gap |

A stream takes one sample handle, then runs through the same mixer path. Streaming is therefore a memory choice, not
a sound choice, apart from any block padding at a loop.

## Voice limits

**[7.2c]** for the limit (preference 1 defaults to 64), **[7.2a]** for the rest:

- There are 64 sample handles per driver. Streams use them too.
- Past the limit, `AIL_allocate_sample_handle` returns null with "Out of sample handles.". **Miles never steals a
  voice.** The caller decides what to drop, and the new sound doesn't play if it does nothing.
- Reusing a handle through `init_sample` resets everything to the defaults above.
- Polygroups and priority, where they exist, are the client's own logic.

**Each client [client]:**

- **Enhanced** allocates 64 handles and releases the last, so 63 voices (`SoundSystem_Startup` 0x008840e1). None free
  drops the new sound (`SoundSystem_FindFreeHandle` 0x008859be), with no stealing and no priority.
- **Kingdom Reborn** allocates handles 1 to 30 (0x00722658) and its start loop uses 1 to 29, so 29 voices. None free
  drops the new sound. Its 25 polygroups (`FUN_0072a90f`) refuse at the group's maximum in mode 0. In mode 1 they start
  the new sound and stop the group's oldest.
- **Classic** has 8 voices (`PlaySound` 0x005fa840). An id already in a voice returns that voice with no restart, unless
  the duplicate flag is set. Otherwise it takes a free voice, else steals the one with the lowest volume, never the
  ambient one.

## 3D distance and the software path

**[7.2a]**. The block holding the speaker falloff (0x21132f60) changed in 7.2c and was not reviewed. The 200.0 default
below is in a block that did not change.

Only Kingdom Reborn reaches this on a live path. It calls `AIL_calculate_3D_channel_levels` itself, with cones at 360
degrees and the sound's own minimum and maximum, then sets channel levels (`Sound_Update3D` 0x00724823) **[client]**. It
never calls `AIL_set_sample_3D_position`. The attenuation routine matches the SDK's with only addresses masked, in both
**[7.0x]** (0x21128790) and **[7.2c]** (0x2112a460), so the formula below holds for it. The routine that turns the
angles into channel levels matches in 7.2c (0x2112a670), but in 7.0x (0x211289a0) only by call position, so the panning
below is weaker for Kingdom Reborn.

- `AIL_set_sample_3D_position` turns on 3D for a sample until its next `init_sample`.
- Each sample defaults to minimum distance 1.0 and maximum 200.0, with automatic attenuation on and no cone.
- Attenuation is 1 inside the minimum, then `min / (min + rolloff * (d - min))`. The driver's rolloff defaults to 1.
- Past the maximum, the voice is muted but keeps running. The driver sets this mute behaviour itself when it opens, so
  the `DIG_3D_MUTE_AT_MAX` preference, whose default says off, is not what decides it.
- The listener starts at the origin, facing +Z with +Y up.
- Speaker panning is constant-power, from the angle of each speaker to the source, with a falloff power of 3.0 by
  default. The result multiplies into the channel levels, along with volume and pan and the dry scale.
- Stereo sources are mixed down to mono at -6 dB once they are 3D.
- Doppler needs velocities. UOSA and UOKR import no velocity call, so doppler is 1 for both.
- `AIL_set_sample_occlusion` runs a low-pass over the voice with a cut-off fraction of `(1 - occlusion) + 0.01`. It is
  a two-pole filter in the merge path. The setup code changed a little in 7.2c, where it runs one filter per channel
  (0x2112e120). **[7.0x]** has the same cut-off (0x2111d080). The setter (0x2112bfd0) turns the filter off outside 0 to
  0.999, scales the fraction by the rates and zeroes the coefficients below 1e-4.
- Kingdom Reborn sets occlusion from distance **[client]**: with `t = (d - 2000) / 16000` clamped to 0..1, it is
  `0.5 * log(400 * t) / log(20)`, at least 0. That is 0 up to `d` = 2,040, about 0.88 at 10,000 and 1.0 from 18,000. It
  is set only when it changed (0x00724a6d). Enhanced has no live occlusion.

## The DS3D provider

**[7.2a]**. `mssds3d.flt`, with `msseax.flt` as its EAX variant, moves voices out of the software mixer:

- Each sample maps to a secondary hardware buffer, up to 96 or the device's limit less one.
- Volume becomes DirectSound decibels. Minimum, maximum and rolloff become native DirectSound 3D values.
- It applies only when the program opens it as a voice filter. A failed open reports that no 3D voices are available.
- `mssdolby.flt` and `msssrs.flt` are post-mix matrix encoders.

Under this provider, the falloff and pan above come from DirectSound rather than from Miles, so whether a client
opens it changes the sound. **Neither client opens one [client]:** both open the driver with spec 0x10. Enhanced opens
spec 0xE0 (EAX 4) once as a probe (`FUN_008843fe`), closes it and restores the spec.

## DSP filters

**[7.2a]**, brief because little of it is used. `mssdsp.flt` offers band-pass, all-pass, capture, chorus,
compressor, flange, high-pass, low-pass, delay, parametric EQ, phaser, resonator, ring modulator, shelving EQ, volume
ramp and gain. A sample has a chain of up to 8 stages, set through `AIL_sample_stage_property`.

Separately from these, the master reverb is built in (`AIL_set_digital_master_reverb`, with time, predelay and
damping). `AIL_set_room_type` picks a preset, and each sample has its own send level.

**What runs [client]:**

- **Kingdom Reborn's `Delay Filter`.** It goes on a sample as stage 2 when the sound's maximum distance reaches 8,000,
  with `Max Delay Time` 600 (0x00724409). `Sound_SetDistanceDelay` (0x00724b19) sets `Mix` to 0 below 8,000, and from
  0.05 to 0.4 between 8,000 and 20,000, with `Delay TimeL` and `R` from 300 to 600 ms. That the tick passes the distance
  is probable, not proven.
- **Kingdom Reborn's reverb is probably silent.** It sets sample reverb levels (0x00728065, 0x0072d924), but the function
  that sets the master reverb (`FUN_00724517`) has no caller. Whether 7.0x has a non-zero master default was not checked.
- **Enhanced runs none of this**, as far as static references go: see the table below.

## What the clients use

All **[client]**, read in Ghidra. **Classic does not link Miles at all.** "None live" means no static reference reaches
the code from a live root.

| Feature | Classic | Kingdom Reborn (7.0x) | Enhanced (7.2c) |
|---|---|---|---|
| Audio API | DirectSound directly | Miles, through its MythAudio system | Miles |
| Mixer | Primary buffer stereo 16-bit 22,048 Hz | 44,100 Hz, 16-bit, spec 0x10 | 48,000 Hz, 16-bit, spec 0x10 |
| Voices | 8, steals the quietest | 29, then a drop; polygroups, stealing the oldest in mode 1 | 63, then a drop |
| Sound start | Secondary buffer, mono 16-bit | `Sound_StartSample`; `AIL_open_stream` in `Sound_StartStream` for the stream type | `AIL_set_named_sample_file` in `SampleSound_Start` |
| Pitch | 22,048 Hz, ± random variation when on | Rate multiplier from a float range, mostly 1.0 to 1.1 | `native_rate * multiplier`, through `AIL_set_sample_playback_rate` |
| Volume | DirectSound hundredths of a dB | 2D: `volume_pan`. 3D: channel levels. Music: `volume_levels` | `volume_pan` with a constant 0.81, then `volume_levels` times the volume |
| Distance | `scale * -2500 * min(1, d / R)`, linear in dB, so -25 dB at `R` and past it with no cut-off; `R` defaults to 18 (`PlayPositionalSound` 0x005fabe0) | Miles' attenuation, with each sound's minimum and maximum and the listener's rolloff; values and units not determined | `1 - d * (1/24) * 0.9`, clamped to 0..1, as volume and cap (`PlaySoundById` 0x004b5770) |
| Pan | ±2500 by screen X, so ±25 dB at the edges | 2D: the client's pan through `volume_pan`. 3D: from the levels call | None: always 0.5 |
| Music | Own MPEG decoder; **2 s crossfade**, linear in dB (`DigitalMusic_Pump` 0x005adf00) | A stream, one at a time; hard cut (`Sound_StartStream` 0x00723d1d) | A sample; hard cut; 3 s linear fade only on id 0xffff; no fade-in (`PlayMusicById` 0x004b5a30) |
| Music loop | Rewinds the decoder and keeps filling the buffer, so no Miles gap | `AIL_set_stream_loop_count(0)`: the stream column under "Streams and one-shot samples" | `AIL_set_sample_loop_count(0)`, so the sample loop gap |
| Master volume | None | `AIL_set_digital_master_volume_level`, linear | The same |
| Music and effects options | Effects: a gain on the dB value, where 0 is -50 dB (0x005f9f50). Music: 0 is -25 dB, 255 is -0.1 dB (0x005f7c70) | **Reach no audio call.** Only their enable flags act (`ApplyVolumeOptions` 0x00495720) | Linear factors: music category 0, effects categories 1 to 4 (`ApplyVolumeOptions` 0x004b5c00) |
| Occlusion | None | From distance, above | None live |
| Delay | None | From distance, above | None live |
| Reverb | None | Sample levels set, master reverb never set | None live |
| DS3D, EAX | None | None | An EAX 4 probe only |
| Preferences | Not applicable | `AIL_set_preference` not imported: the defaults apply | The same |
| Other limits | Ids up to 1,667 | 31 sound slots, 4,000 sample slots, 99 streams | 25,000 sample slots; the MP3 limit of 7 is in dead code |

The Enhanced entries that say "none live" rest on the closure of the code. `AudioConfig_Init` has no caller, so the
MythAudio config tables are dead. `SoundSystem_PlaySample3D` (0x008851db) and `StreamSound` have only one route in, and
it ends in roots with no reference. The filter adders work on a list filled from zones set only in one of those roots.
A caller through a table that Ghidra did not see would change this, and two references from code outside any function
(0x00893adb, 0x00893b30) could not be attributed.

## What our playback must do

**Must match:**

1. Decode every MP3 frame and output it all. **Do not trim** on Xing or LAME data, and keep any gapless option off.
2. Loop by restarting at the first frame, Info frame included, without clearing the decoder's overlap and synthesis
   state, but clearing the bit reservoir. This keeps the client's gap without adding a click.
3. Output silence for a frame whose reservoir isn't available.
4. Skip junk under 8 KiB between frames, and end the stream without error past that.
5. Mix as 16-bit samples with linear gains, summed and **hard-clipped** with no limiter.
6. Apply each client's gain law. Multiply master volume in linearly. Never boost past 1.0.
   - Enhanced: linear volume on the fixed factor of about 0.572, pan always centre, and `1 - d * (1/24) * 0.9` for
     distance. **Its options and distance do not go through `v ^ (5/3)`.**
   - Kingdom Reborn: `v ^ (5/3)` and the pan law `^ 0.3` with its -1.8 dB centre for 2D sounds, Miles' attenuation and
     panning for 3D sounds, and linear levels for music.
7. Ramp level changes over about 32 samples.
8. Mix at each client's rate, 48,000 Hz for Enhanced and 44,100 Hz for Kingdom Reborn, and resample with linear
   interpolation up and overlap-weighted box average down, skipping resampling within ±0.2%.
9. Enforce each client's own voices: 63 and a drop for Enhanced, 29 and a drop with client-side polygroups for Kingdom
   Reborn.
10. Keep each client's music: a looping sample with a hard cut and the loop gap for Enhanced, a stream with a hard cut
    for Kingdom Reborn.
11. Kingdom Reborn only: the occlusion low-pass and the `Delay Filter`, both from distance. Its effects and music options
    do nothing in the client.
12. Keep Classic separate, through its own path: mono 16-bit at 22,048 Hz, 8 voices that steal the quietest, gains linear
    in dB, a 2 s music crossfade, and its own decoder with no Miles loop gap.

**Can ignore:**

- The 11-bit scale quantisation and the 16.16 stepping. Float is fine.
- The zero-fill bug for a cut last frame, unless a bit-exact comparison is wanted. Decoding the frame with zeros
  is close, and only Kingdom Reborn's 13 files reach it.
- MPEG-2.5 band table behaviour. No shipped file uses those rates, and the validator refuses them unless asked to
  allow them for a test client.
- ADPCM and multichannel paths. No client depends on them for shipped sounds.
- Stream buffer sizing, and starvation gaps.
- Mixer latency, beyond being reasonably low.
- Doppler and cones. No client sets a velocity, and Kingdom Reborn's cones are 360 degrees.
- The DSP filter set, except Kingdom Reborn's low-pass and `Delay Filter`.
- Reverb, unless 7.0x turns out to have a non-zero master default.
- A partial last sample frame, beyond not faulting.
- The Dolby and SRS matrix encoders.

**Not determined:**

- Enhanced: the two unattributed references above, which are the weak point of "3D and filters are dead", and the ramp
  that the `Master volume hits target volume` string suggests, which was not followed.
- Kingdom Reborn: the distance units (the packet position goes through `FUN_0045b6a0`), the `audio_music.csv` reader and
  its fade fields, that the shipped music rows are of the stream type, the value the tick passes to the delay, the
  rolloff value, and the master reverb default of 7.0x.
- Classic: what the device really runs at under a modern Windows, and the delay and padding of its own decoder.
- Miles: the stream code in 7.2c and 7.0x, the 7.2c speaker falloff (0x21132f60), the second user of 2048 (0x2112f5f6),
  the speaker gains of the multichannel path and its 8-bit case, the downsampling loops at a partial tail, the size of
  the image's allocation, and what a Windows speaker setting other than stereo does to either client.
- Everything here against a real client: nothing has been measured.
