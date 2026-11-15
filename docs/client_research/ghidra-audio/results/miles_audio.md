# Miles Sound System 7.2c: what the EC (and KR) decoder accepts

Programs `mss32.dll` (SHA-256 `b588b4791ff6803247962c661b6d3ff9a8625d0f60363a6fa53f61d401f0e74e`) and
`miles/mssmp3.asi` (`0fb784a9cc9826a9ec2412b7d71453b917af6c74442647f38061512e2c3a05bb`), both from the EC
folder. UOSA.exe sets the redist directory to `miles` (`SoundSystem_Startup`). KR ships its own Miles, 7.0x, in a
`miles` folder of the same layout (see `docs/agent_notes/ec_audio_formats.md`). **confirmed** = read in the code;
**probable** = inferred. How Miles plays what it accepts is in `docs/agent_notes/miles_playback_reference.md`.

## Functions

| Address | Name | Notes |
|---|---|---|
| 0x2112b670 | `Miles::SetNamedSampleFile_Impl` | body of `AIL_set_named_sample_file` |
| 0x2112b4c0 | `Miles::SetSampleFile_Impl` | body of `AIL_set_sample_file` |
| 0x21129cc0 | `Miles::WavInfo_Impl` | body of `AIL_WAV_info` |
| 0x2112b3c0 | `Miles::InitSampleFromWavInfo` | body of `AIL_set_sample_info` |
| 0x21109d00 | `Miles::StrnicmpN` | case-insensitive compare |
| 0x26f07080 | `MilesMp3::SeekFrameHeader` | mssmp3.asi frame header search |

## Q3. Decoder and formats

### The client checks nothing

**confirmed**. `SampleSound_Start` (UOSA.exe) hands the whole entry to
`AIL_set_named_sample_file(handle, name, data, size, 0)`, with `name` = the UOP path. It checks no header,
rate, channel count or tag. The playback rate is the file's own; the client changes it only for pitch.
Everything below is Miles.

### Which decoder: by name, not by content

**confirmed**. `SetNamedSampleFile_Impl` decides as follows:

1. If the data starts with a valid ID3v2 header, Miles skips it and treats the file as `.mp3`, whatever the
   name. A valid header is `ID3`, version bytes < 0xFF and syncsafe size bytes < 0x80. The ID3v2 footer flag is
   not handled at this level.

   The skip is not quite size + 10. Miles adds 10 to the lowest size byte, then ORs in the other size bytes
   shifted (0x2112b6ca to 0x2112b6e6; the same in the stream code and in KR's 7.0x). A carry out of bit 6 is lost
   when the next size byte is odd, so the skip is 128 bytes short and the scan starts inside the tag. This happens
   when the lowest size byte is 118 or more and the next is odd: 157 of Enhanced's 181 tagged MP3s and 10 of
   KR's 47. Every one of those has only zero bytes in those 128, so the first frame found is the same
   (checked on the shipped files, 2026-11-02).
2. Otherwise Miles takes the ASI provider whose "Input file types" match the name's extension. `.mp3` goes
   to mssmp3.asi.
3. With no provider, `.wav` and `.voc` (case-insensitive) go to the built-in parser
   (`SetSampleFile_Impl`). Any other extension fails with "Couldn't find provider for this file type".

So the content must match the extension. MP3 data in a `.wav` entry fails, unless it starts with ID3v2.
RIFF data in a `.mp3` entry fails in the MP3 decoder.

### WAV (built-in parser)

**confirmed**:

- **RIFF walk** (`WavInfo_Impl`):
  - `WAVE` must be at +8. Chunk ids are compared case-insensitively.
  - Chunks are walked from +12 with step `size + (size & 1) + 8`, so the RIFF pad byte after an odd-sized
    chunk is required. The walk is bounded by the RIFF size at +4.
  - `fmt ` is searched first. Miles reads only its first 16 bytes: tag, channels, rate, block align, bits.
    So a `fmt ` size of 16 or 18 both work.
  - `data` is searched again from the first chunk. Its length is the chunk size, and Miles does not clamp
    it to the file.
- **Formats** (`SetSampleFile_Impl`, `InitSampleFromWavInfo`):
  - **PCM** (tag 1): 8 or 16 bits per sample. Other depths are not usable, but how they fail depends on the path.
    `InitSampleFromWavInfo` (the body of `AIL_set_sample_info`) refuses them with "Unsupported wave file format.".
    The named-file path has no such check: a 24 or 32-bit file plays as noise, and a 4-bit one is flagged as ADPCM
    (7.2a source, and the same in 7.2c by `objdump`, see `miles_playback_reference.md`).
    1 or 2 channels; more use the multichannel path.
  - **IMA ADPCM** (tag 0x11, 4 bits) is native. It uses `fact` when present.
  - **WAVE_FORMAT_EXTENSIBLE** (0xFFFE): `cbSize` ≥ 22, SubFormat must be `KSDATAFORMAT_SUBTYPE_PCM`, and
    block align must equal channels × 2 (16-bit).
  - **Other tags** need an ASI codec with a matching "Input wave tag". `mssmp3.asi` declares 0x55, so an MP3 in a
    RIFF `.wav` would reach it (by `objdump`); no shipped entry does this, and we refuse it.
- **Rate.** Any rate works: `AIL_set_sample_playback_rate(file rate)`, and the mixer resamples. A
  22,048 Hz WAV plays at 22,048 Hz with no correction.
- **Other chunks** (LIST, bext, iXML, _PMX, ...) are skipped by the walk.
- **Errors:** "Not a WAV file.", "Invalid or missing data block.", "Not a WAVEFORMATPCMEX file.",
  "Not KSDATAFORMAT_SUBTYPE_PCM.", "Unsupported multichannel format.".

**confirmed**: the 110 shipped legacy WAVs with an unpadded odd chunk (see `ec_audio_formats.md`) do not
mis-step the walk. All are `fmt  data LIST DISP bext cart`, the odd chunk is `bext`, and each search stops at its
match and restarts from the first chunk. `fact` is searched only for IMA ADPCM, so a PCM file's walk never goes
past `data`. An unpadded odd chunk before `fmt ` or `data` would land the walk one byte late and most likely fail
as "Not a WAV file.". The EC never plays legacy WAVs in any case.

### MP3 (mssmp3.asi)

**confirmed**, from `MilesMp3::SeekFrameHeader`:

- **Frame search.** The decoder scans byte by byte, up to 0x2000 (8 KiB), for an 11-bit frame sync. MPEG-1,
  MPEG-2 and MPEG-2.5 headers are all accepted. Leading junk shorter than 8 KiB is fine; this covers the
  110 shipped files with leading zero bytes.
- **Layers.** Layer III only. Other layers are skipped and the scan goes on in the same window; the open fails
  only when the window runs out with no Layer III header, and the error names the last layer it saw.
- **Header fields.** Bitrate index 15 and sample-rate index 3 are skipped the same way.
- **Later frames** are compared with the previous header seen, not the first frame's: version, MPEG-2.5 flag,
  layer and sample rate. A header that differs is skipped, **and it replaces the values compared against**
  (0x26f075cd to 0x26f0765b). So a real change of rate loses one frame, and decoding then goes on at the new
  values while the sample's playback rate stays as set at open. The bitrate is not compared, so VBR works.
- **Between frames** each frame gets its own 8 KiB window. Junk shorter than that is skipped silently; beyond it,
  or at the end of the data, the stream ends without an error.
- **Xing/Info headers** are not parsed. There is no `Xing` string, so the Info frame decodes as an ordinary
  (silent) frame.
- **Tags.** An inline ID3v1 `TAG` (128 bytes) and an inline ID3v2 tag (+10 bytes for the footer flag) are
  skipped while seeking. A leading ID3v2 tag is already skipped by `SetNamedSampleFile_Impl`.
- **CRC.** The protection CRC (2 bytes) is skipped, not checked.

**confirmed, with a qualifier**: the decoder plays all nine rates its header table holds, at the right rate and
without failing. MPEG-1 and MPEG-2 are correct. **MPEG-2.5 is not**: the band tables have six rows only, so the
three MPEG-2.5 rates reuse the MPEG-2 rows of the same index (11,025 uses 22,050's, 12,000 uses 24,000's, 8,000
uses 16,000's). The 8 kHz long-block row is in none of the three `mssmp3.asi` builds (byte search). Frame sizes and
the output rate are right, so these files play with wrong band edges and audible artefacts are to be expected,
though that was not measured. No shipped file uses MPEG-2.5.

The 7.2a source of `mp3api.cpp` and `mp3dec.cpp` applies to this `mssmp3.asi`: all 46 code blocks match it, and
only 4 instructions differ, none in the decoding.

## Edge cases of the mixer and the WAV path (2026-11-03)

**confirmed** by a static reading in the shipped binaries, not measured. The 7.2a source was read only to find the
places, and `MSSMIXER` is byte-identical between the two. The first 0x300 bytes of the ADPCM decoder (0x2114f2c7) were
also compared directly: they are the same in 7.2c, and 7.0x differs after 112 bytes.

- **PCM data that is not whole sample frames.** The stream-to-buffer routine (0x211305d0) ends a sample only when 0 bytes
  or fewer remain, so a short tail reaches the merge (0x2112f3f0), which still asks for a sample frame. The loops
  read (no resampling, nearest and upsampling) load a whole sample frame first and compare with the end only after
  stepping. So the last read **starts inside the tail and goes at most `frame - 1` bytes past the data**: 1 byte for
  mono 16-bit or stereo 8-bit, 3 for stereo 16-bit, not a whole frame. The extra bytes are what follows in the image: a
  pad byte, the next chunk, or memory past the image. With a zero pad byte, a mono 16-bit tail gives one source frame
  of at most 255/32768, about -42 dB, which should be inaudible. With other bytes it can be one source frame at up to
  full scale, a possible click at the very end. The downsampling loops and the size of the image's allocation were not
  read.
- **IMA ADPCM with block align 0.** With a `fact` chunk nothing divides by it, and block size 0 is stored
  (0x2112a098), replacing the default of 256. The decoder sets the bytes left in the block to the block size less the
  header, 4 for mono and 8 for stereo. That wraps to nearly 2^32 and only falls by 4 or 8 per dword, so **the whole
  data chunk is one block**: one header, then nibbles to the end of the data. A genuinely single-block file plays
  correctly. A file with real blocks has its later headers decoded as audio, which is noise, not a fault. Without
  `fact`, `WavInfo_Impl` divides by 0 (0x21129fea). The position calls (0x2112e9a4, 0x2112eb0c) do not fault, and report
  position 0. A loop resets the block state (0x211305d0).
- **Channels.** In both set-up paths (0x2112a030, 0x2112b3c0): more than 2 channels or a channel mask gives the
  multichannel path with that count; 2 is stereo; **anything else, 0 included, is mono** (the sample type setter,
  0x211298f0). The depth flags are separate. A mask comes only from EXTENSIBLE (0x21129cc0), so mono EXTENSIBLE
  takes the multichannel path with one channel. EXTENSIBLE needs block align = channels × 2. A plain tag 1 file has no
  such test, so a block align or an 8-bit depth that does not fit reaches the merge, which was not read for that case.
  `FUN_211290f0` gives each source channel its state and maps the mask's first 9 bits to source channels, and the merge
  de-interleaves. The gains from there to the speakers (the table at 0x21151db8) were not read.
- **`mssvoice.asi`** (SHA-256 `c0163aaf5c6ac0b252261dfbf718c32de02a34efc97761d6612bda3d529b772e`). Only `_RIB_Main@8` is
  exported. It registers Voxware MetaVoice codecs under "ASI codec" and "ASI stream", each with an "Input wave tag":

  | Codec | Files | Input wave tag | Property function |
  |---|---|---|---|
  | RT24 | `.V24` | 0x74 | 0x26402990 |
  | RT29 | `.V29` | 0x75 | 0x26401000 |
  | VR12 | `.V12` | 0x77 | 0x26404360 |

  A second provider for each declares tag 1 (raw PCM) as input and the codec's files as output: the encoder. So a WAV
  with one of those three tags would reach this codec. No shipped sound is known to use them. Kingdom Reborn ships a
  different build (SHA-256 `8a99feb190a814cf06e8e4d838938e91905b3ed4bc270245161c830a3c262ae3`), which was not read.

**7.0x** (`/libs-kr`): the WAV walk (`FUN_21128100`) has the same step, RIFF bound, `fmt ` and `data` searches, and the
same `fact` search and divide. It has **no EXTENSIBLE branch**, so tag 0xFFFE goes to the provider search as any other
tag. `SeekFrameHeader` (0x26f07050) differs from the 7.2c one (0x26f07080) only in field offsets, so its `TAG` and `ID3`
handling is the same. KR ships no WAV, so neither path matters for its shipped sounds.

## Replace path whitelist (recommendation)

These are the tightest rules that match both what Miles accepts and what ships:

| Entry | Accept | Reject |
|---|---|---|
| `.mp3` | MPEG audio Layer III; constant sample rate and version across frames; CBR or VBR; optional leading ID3v2 (2.2/2.3/2.4) and trailing ID3v1; Xing/Info frame allowed; first sync within 8 KiB | Layer I/II, free-format or reserved bitrate, reserved rate, rate or version change mid-file, RIFF/WAV data |
| `.wav` | RIFF/WAVE; PCM 8 or 16-bit (or EXTENSIBLE with PCM subformat, 16-bit); mono or stereo; any rate; `fmt ` before `data`; every odd chunk padded; `data` size within the file | other tags, other bit depths, more than 2 channels, a missing or truncated `data`, MP3 data |

Also:

- **Write the shipped conventions, warn on the rest.** Accept any rate Miles accepts, but warn when it
  differs from the shipped ones: EC MP3 at the shipped rates, and legacy WAVs at 22,048 Hz mono 16-bit,
  which the Classic tooling expects.
- **Padding.** Pad odd chunks when writing, and refuse (or repair) unpadded input.
- **Name and content must agree.** Never store MP3 data under a `.wav` name or the reverse.
