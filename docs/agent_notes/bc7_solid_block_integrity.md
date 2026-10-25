# BC7 solid-block endpoint integrity

The analytical encoder's solid-color fast path uses Mode 5 with color selector
1 for all texels. Its RGB endpoint table must therefore use the fixed BC7
weights 43/64 and 21/64 after expanding 7-bit endpoints by bit replication.
Pairs optimal for another selector cannot be reused: the selector is shared
by all three RGB channels and is already fixed by the block tail.

The former endpoint table failed this reconstruction for 246 of 256 channel
values. Constant source blocks became dark patches even though alpha remained
255. An independently wrapped DDS decoded by Pillow agreed with our decoder,
establishing that the stored blocks, rather than the decoder, were wrong.
The defect was especially visible in original EC reflection image 1000017.

The table is now generated at compile time from the specified interpolation
equation. `solid_mode5_roundtrips_every_channel_value_exactly` covers every value
in each channel, including alpha, through the public encoder/decoder API.

Existing packages retain their corrupted bytes until rebuilt from source.
Do not compensate with shader tinting or reinterpret the bad endpoint pairs
in the decoder: distinct original colors can share a former table pair.

Specification: [Microsoft BC7 format](https://learn.microsoft.com/en-us/windows/win32/direct3d11/bc7-format).
