# Miles Sound System 7.2c: what the EC (and KR) decoder accepts

Programs `mss32.dll` (SHA-256 `b588b4791ff6803247962c661b6d3ff9a8625d0f60363a6fa53f61d401f0e74e`) and
`miles/mssmp3.asi` (`0fb784a9cc9826a9ec2412b7d71453b917af6c74442647f38061512e2c3a05bb`), both from the EC
folder. UOSA.exe sets the redist directory to `miles` (`SoundSystem_Startup`); KR ships the same `miles`
folder. **confirmed** = read in the code; **probable** = inferred.

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

1. If the data starts with a valid ID3v2 header, Miles skips its size + 10 bytes and treats the file as
   `.mp3`, whatever the name. A valid header is `ID3`, version bytes < 0xFF and syncsafe size bytes < 0x80.
   The ID3v2 footer flag is not handled at this level.
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
  - **PCM** (tag 1): 8 or 16 bits per sample, any other bit depth fails ("Unsupported wave file format.").
    1 or 2 channels; more use the multichannel path.
  - **IMA ADPCM** (tag 0x11, 4 bits) is native. It uses `fact` when present.
  - **WAVE_FORMAT_EXTENSIBLE** (0xFFFE): `cbSize` ≥ 22, SubFormat must be `KSDATAFORMAT_SUBTYPE_PCM`, and
    block align must equal channels × 2 (16-bit).
  - **Other tags** need an ASI codec with a matching "Input wave tag". The EC ships none for WAV.
- **Rate.** Any rate works: `AIL_set_sample_playback_rate(file rate)`, and the mixer resamples. A
  22,048 Hz WAV plays at 22,048 Hz with no correction.
- **Other chunks** (LIST, bext, iXML, _PMX, ...) are skipped by the walk.
- **Errors:** "Not a WAV file.", "Invalid or missing data block.", "Not a WAVEFORMATPCMEX file.",
  "Not KSDATAFORMAT_SUBTYPE_PCM.", "Unsupported multichannel format.".

**probable**: the 110 shipped legacy WAVs with an unpadded odd chunk (see `ec_audio_formats.md`) would
mis-step the walk if that chunk comes before `fmt ` or `data`. This does not matter for the EC, which never
plays legacy WAVs.

### MP3 (mssmp3.asi)

**confirmed**, from `MilesMp3::SeekFrameHeader`:

- **Frame search.** The decoder scans byte by byte, up to 0x2000 (8 KiB), for an 11-bit frame sync. MPEG-1,
  MPEG-2 and MPEG-2.5 headers are all accepted. Leading junk shorter than 8 KiB is fine; this covers the
  110 shipped files with leading zero bytes.
- **Layers.** Layer III only. Layers I and II and the reserved layer are rejected.
- **Header fields.** Bitrate index 15 and sample-rate index 3 are rejected.
- **Later frames** must match the first frame's version, MPEG-2.5 flag, layer and sample rate. The bitrate
  is not compared, so VBR works.
- **Xing/Info headers** are not parsed. There is no `Xing` string, so the Info frame decodes as an ordinary
  (silent) frame.
- **Tags.** An inline ID3v1 `TAG` (128 bytes) and an inline ID3v2 tag (+10 bytes for the footer flag) are
  skipped while seeking. A leading ID3v2 tag is already skipped by `SetNamedSampleFile_Impl`.
- **CRC.** The protection CRC (2 bytes) is skipped, not checked.

**probable**: the decoder itself (not read here) handles all the rates its header table holds (MPEG-1
32/44.1/48 kHz, MPEG-2 16/22.05/24 kHz, MPEG-2.5 8/11.025/12 kHz).

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
