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
| **[7.2a]** | Read in the 7.2a SDK source only. Likely true for 7.2c, but not checked |
| **[client]** | Taken from our notes on the client, which were read in Ghidra |
| **needs a Ghidra pass** | Neither the notes nor a static look show it |

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

## Volume and pan

**`AIL_set_sample_volume_pan(volume, pan)`** **[7.2c]**, read in `mss32.dll` at 0x2112ce50:

- Gain is `volume ^ (5/3)`, so 0.5 is about -10 dB and 0.25 about -20 dB.
- Left is `gain * (1 - pan) ^ 0.3`. Right is `gain * pan ^ 0.3`. Pan 0 is left, 0.5 centre, 1 right.
- Centre pan is special-cased to the same factor on both sides, `0.5 ^ 0.3`, about 0.8123 or -1.8 dB. A sound panned
  fully to one side is therefore 1.8 dB louder on that side than the same sound at centre.
- On drivers with 4 or more logical channels, both sides take one more factor of 0.8123.

**Defaults [7.2a]:** `init_sample` sets volume 1.0 and pan 0.5, so an untouched sample plays at 0.8123 on both sides.

**`AIL_set_sample_volume_levels(left, right)` [7.2a]** stores the two values as linear scalars. It applies neither
curve and no centre factor. A client that uses this call gets a different loudness law from one using volume and pan.

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
**The mixer rate of each client's `AIL_open_digital_driver` call needs a Ghidra pass**, and it decides how much of
this applies: at 44,100 Hz, a 22,050 Hz sound is upsampled by linear interpolation.

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
  by the block align (0x21129fea), so a block align of 0 faults.

**WAV types [7.2c]** (0x2112b4c0, 0x2112a030). Tag 1 takes any bit depth: 4 is treated as ADPCM, 16 as 16-bit and every
other depth as 8-bit, so 24 or 32-bit plays as noise. The block align is used only for ADPCM. Any other tag goes to a
provider whose "Input wave tag" matches, with raw output: `mssmp3.asi` declares 0x55 (0x26f07850), so an MP3-in-RIFF
`.wav` reaches it.

**Partial sample frames [7.2a]** (`wavefile.cpp`). The source runs out when its remaining length is 0 or below, and the
mixer may read a whole sample frame past the end of the data. Not checked in 7.2c, so the validator leaves such a file
unverified.

## Streams and one-shot samples

**[7.2a]**. The stream code is among the 7.2c blocks that changed (at 0x211103b0; `AIL_set_file_callbacks` also gained
a parameter), so check these against 7.2c before relying on the details.

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
- Polygroups and priority, where they exist, are the client's own logic (see below).

## 3D distance and the software path

**[7.2a]**. The block holding the speaker falloff (0x21132f60) changed in 7.2c and was not reviewed. The 200.0 default
below is in a block that did not change.

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
  (0x2112e120).

## The DS3D provider

**[7.2a]**. `mssds3d.flt`, with `msseax.flt` as its EAX variant, moves voices out of the software mixer:

- Each sample maps to a secondary hardware buffer, up to 96 or the device's limit less one.
- Volume becomes DirectSound decibels. Minimum, maximum and rolloff become native DirectSound 3D values.
- It applies only when the program opens it as a voice filter. A failed open reports that no 3D voices are available.
- `mssdolby.flt` and `msssrs.flt` are post-mix matrix encoders.

Under this provider, the falloff and pan above come from DirectSound rather than from Miles, so whether a client
opens it changes the sound. **Needs a Ghidra pass for both clients.**

## DSP filters

**[7.2a]**, brief because nothing shows UO relies on them. `mssdsp.flt` offers band-pass, all-pass, capture, chorus,
compressor, flange, high-pass, low-pass, delay, parametric EQ, phaser, resonator, ring modulator, shelving EQ, volume
ramp and gain. A sample has a chain of up to 8 stages, set through `AIL_sample_stage_property`.

Separately from these, the master reverb is built in (`AIL_set_digital_master_reverb`, with time, predelay and
damping). `AIL_set_room_type` picks a preset, and each sample has its own send level.

## What the clients use

Imports show only that a call is linked, not that it runs or with what values. **Classic does not link Miles at all.**

| Feature | Classic | Kingdom Reborn (7.0x) | Enhanced (7.2c) |
|---|---|---|---|
| Audio API | DirectSound directly **[client]** | Miles, through its MythAudio system **[client]** | Miles **[client]** |
| Voices | 8 **[client]** | Polygroups and a priority per sound **[client]**; handle count needs a pass | Handle pool needs a pass |
| Sound start | Secondary buffer **[client]** | Imports `AIL_set_named_sample_file` and `AIL_open_stream`; each sound has a type and a streamed flag **[client]** | `AIL_set_named_sample_file` in `SampleSound_Start` **[client]** |
| Format and rate | Mono 16-bit, 22,048 Hz, ± random variation when on **[client]** | Rate multiplier from a float range, mostly 1.0 to 1.1 **[client]** | File rate **[client]**; any pitch call needs a pass |
| Volume API | Needs a pass | Imports both `volume_pan` and `volume_levels`; needs a pass | Imports both; needs a pass |
| Position | Needs a pass | Own levels: imports `AIL_calculate_3D_channel_levels` and `AIL_set_sample_channel_levels`, and no `AIL_set_sample_3D_position` | Imports the 3D calls. Mode 1 of `PlaySoundById` computes a gain `1 - d * (1/24) * 0.9`, clamped to 0..1 **[client]**; how it reaches Miles needs a pass |
| Range and rolloff | Not applicable | Per-sound min and max, and a `ROLLOFF FACTOR` section **[client]**; values need a pass | Imports `AIL_set_3D_rolloff_factor`; `ROLLOFF FACTOR` string; use needs a pass |
| Music | `MUSIC\DIGITAL\CONFIG.TXT` with looping flags **[client]**; decoder needs a pass | `audio_music.csv` with loop and fades **[client]**; path needs a pass | `audio_music.csv` with `Loop` 0 or 1, through `PlayMusicById` **[client]**; stream or sample needs a pass |
| Loop calls | Needs a pass | Imports `set_sample_loop_count` and `set_stream_loop_count` | Imports the same two |
| Master volume | Needs a pass | Imports `AIL_set_digital_master_volume_level`; needs a pass | Same import; `MASTER VOLUME` string, and a `Master volume hits target volume` debug string suggests a ramp; needs a pass |
| Low-pass, occlusion | None | Imports occlusion; `Low pass cutoff` debug string; needs a pass | Imports occlusion, and has `LowPassFilter` and `OcclusionFilter` classes; needs a pass |
| Reverb | None | Imports `set_digital_master_reverb` and sample reverb levels; `enable_reverb`, `reverb_time`, `reverb_pre_delay`, `reverb_damping` strings; needs a pass | Imports master reverb, room type and sample levels; `RoomReverbFilter`, `ReverbFilter`, `EAXReverbFilter` classes; needs a pass |
| Providers | None | Both name DirectSound3D, EAX 2 to 4, Dolby and SRS, and both import `AIL_set_speaker_configuration` and `AIL_set_sample_processor`, but neither imports a digital driver processor call; which one opens needs a pass | As Kingdom Reborn |
| Preferences | Not applicable | `AIL_set_preference` not imported: the defaults apply | Same |
| Mixer rate and format | Not applicable | Needs a pass | Needs a pass |
| Other limits | 8 voices, ids up to 1,667 **[client]** | Needs a pass | `MP3 LIMIT` string, possibly a cap on concurrent MP3s; needs a pass |

`AudioConfig_Init` has no caller in UOSA, so the MythAudio config tables are dead code there **[client]**.
Its filter classes may still run, which is part of the same pass.

## What our playback must do

**Must match:**

1. Decode every MP3 frame and output it all. **Do not trim** on Xing or LAME data, and keep any gapless option off.
2. Loop by restarting at the first frame, Info frame included, without clearing the decoder's overlap and synthesis
   state, but clearing the bit reservoir. This keeps the client's gap without adding a click.
3. Output silence for a frame whose reservoir isn't available.
4. Skip junk under 8 KiB between frames, and end the stream without error past that.
5. Mix as 16-bit samples with linear gains, summed and **hard-clipped** with no limiter.
6. Apply the volume curve `v ^ (5/3)` and the pan law `^ 0.3` with its -1.8 dB centre, wherever a client uses volume
   and pan. Multiply master volume in linearly. Never boost past 1.0.
7. Ramp level changes over about 32 samples.
8. Resample with linear interpolation up and overlap-weighted box average down, skipping resampling within ±0.2%.
9. Enforce each client's own limits: 8 voices for Classic, and client-side polygroups for Kingdom Reborn. Drop rather
   than steal when past 64 handles.
10. Keep Classic separate: mono 16-bit at 22,048 Hz, through its own path.

**Can ignore:**

- The 11-bit scale quantisation and the 16.16 stepping. Float is fine.
- The zero-fill bug for a cut last frame, unless a bit-exact comparison is wanted. Decoding the frame with zeros
  is close, and only Kingdom Reborn's 13 files reach it.
- MPEG-2.5 band table behaviour. No shipped file uses those rates, and the validator refuses them unless asked to
  allow them for a test client.
- ADPCM and multichannel paths. No client depends on them for shipped sounds.
- Stream buffer sizing, and starvation gaps.
- Mixer latency, beyond being reasonably low.
- Doppler and cones, on current evidence.
- The DSP filter set, except a low-pass, if the pending pass shows the clients use it.
- The Dolby and SRS matrix encoders.

**Blocked on the pending pass:** which volume call carries the Enhanced distance gain (it decides whether the
`v ^ (5/3)` curve applies to it), the mixer rate, stream or sample for music, and whether occlusion, reverb or a
DS3D provider run at all.
