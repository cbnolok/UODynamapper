# Compression and Decompression Algorithms Explained

We are dealing with three types of compression relevant to the UOP format: standard Zlib, a custom "Mythic" compression, and a custom "ZlibBwt3" compression.

## 1. Zlib Compression (Standard)

Zlib is a widely adopted, lossless data compression library. It's built upon the DEFLATE algorithm, which combines two core techniques: LZ77 and Huffman coding.

* **How it works (Simplified):**
  * **LZ77 (Lempel-Ziv 1977):** This part focuses on finding and replacing repeating sequences of bytes (patterns) within the data. Instead of storing the repeated sequence directly, it stores a "pointer" to a previous occurrence. This pointer consists of two pieces of information: a "distance" (how far back in the data the sequence was found) and a "length" (how long the repeating sequence is). This effectively reduces redundancy.
  * **Huffman Coding:** After LZ77 has reduced the data by identifying and replacing repetitions, Huffman coding is applied. This is a method of assigning variable-length codes to symbols (which can be individual bytes or the LZ77 "pointers"). More frequently occurring symbols are given shorter binary codes, while less frequent ones receive longer codes. This minimizes the average code length, leading to further data compression.

* **Compression Process:** The Zlib encoder takes the raw input data, applies the LZ77 algorithm to identify and replace repeating patterns, and then uses Huffman coding to assign efficient, variable-length codes to the resulting data stream.
* **Decompression Process:** The Zlib decoder reverses these steps. It first decodes the Huffman codes to reconstruct the LZ77 pointers and literal bytes. Then, it uses these LZ77 pointers to reconstruct the original repeating sequences, effectively restoring the data to its uncompressed form.

* **Mathematical Concept:** Zlib leverages principles from information theory and statistical coding. It exploits the inherent redundancy in data to represent it using fewer bits.

## 2. Mythic Compression (Custom)

This is a proprietary, multi-stage compression/obfuscation algorithm. It is not a standard, publicly documented compression method.

* **Compression (`mythic_decompress::transform`):
    1. **Internal Compression (`internal_compress`):
        * **Frequency Analysis:** The first step involves counting the occurrences of each possible byte value (0-255) within the input data.
        * **Symbol Mapping & Shifting:** It constructs a complex internal representation using several arrays (`symbol_table`, `frequency_table`, and `partial_input`). These arrays are used to dynamically map original byte values to a compressed representation based on their frequency and position.
        * **Reverse Pass:** The algorithm processes the input data from end to beginning. For each byte, it determines its current position within a dynamically shifting `symbol_table` (which involves "shifting" elements to the right). It records this position and updates the `symbol_table`.
        * **Output Structure:** The output of this stage begins with a 1024-byte header (which stores the frequency information of the original bytes) followed by the compressed data generated during the reverse pass.
    2. **Move-to-Front (MTF) Encoding (`move_to_front_coding::encode`):
        * The output from the `internal_compress` stage is then further processed by a Move-to-Front (MTF) encoder.
        * **How MTF works:** MTF maintains an "alphabet" (initially a sorted list of all possible byte values, 0-255). For each incoming byte, it finds that byte's current position (index) in the alphabet. This index is output, and the byte itself is then moved to the very front of the alphabet. This strategy makes frequently occurring symbols (or recently seen symbols) have smaller indices, which can improve the efficiency of subsequent compression stages (though here, it's the final step of the compression).

* **Decompression (`mythic_decompress::decompress_with_header`):
    1. **Header Processing:** The decompression begins by reading a 4-byte header from the input. This header contains the original uncompressed data length, which was XORed with a magic number (`0x8E2C9A3D`) during compression. This length is used to verify the integrity of the decompressed data.
    2. **Move-to-Front (MTF) Decoding (`move_to_front_coding::decode`):
        * The data following the header is passed through an MTF decoder. This process reverses the MTF encoding, restoring the output of the `internal_compress` stage.
    3. **Internal Decompression (`internal_decompress`):
        * This function is the inverse of `internal_compress`. It first reads the frequency information from the initial 1024 bytes of its input.
        * It then reconstructs the `symbol_table` and `partial_input` arrays based on this frequency data.
        * Finally, it iterates through the MTF-decoded data, using the reconstructed symbol tables and "shifting" operations (left shifts this time) to reconstruct the original bytes in their correct order.

* **Mathematical Concept:** Mythic compression combines frequency analysis, a custom dynamic symbol mapping and shifting scheme, and the Move-to-Front transform. It's a bespoke algorithm likely optimized for specific data patterns found in Ultima Online game assets, possibly also serving an obfuscation purpose.

## 3. ZlibBwt3 Compression (Burrows-Wheeler Transform based)

This is another custom compression scheme, which appears to be based on the Burrows-Wheeler Transform (BWT).

* **Compression Process (Inferred):** A typical BWT-based compression pipeline involves:
    1. **Burrows-Wheeler Transform (BWT):** This transform rearranges the input data. It works by taking all cyclic shifts of the input string, sorting them alphabetically, and then extracting the last character of each sorted shift. The result is a block of data where identical characters are grouped together, making it much easier for subsequent compression algorithms to find and exploit patterns.
    2. **Post-BWT Processing:** The BWT output is often followed by a Move-to-Front (MTF) transform or Run-Length Encoding (RLE) to further enhance compressibility.
    3. **Entropy Coding:** Finally, an entropy encoder (like Huffman coding or arithmetic coding) is applied to the processed data to achieve the final compression.

* **Decompression (`zlib_bwt_codec::decompress`):
    1. **Header Processing:** The decompression starts by reading a 4-byte header (whose exact purpose isn't fully clear from the provided C# `BwtDecompress` code, but is part of the UOP entry structure) and a `first_char` byte.
    2. **`build_table`:** A 256x256 lookup table (`table`) is constructed based on the `first_char`. This table is then sorted. This table is critical for the inverse BWT process, allowing the algorithm to reconstruct the original data.
    3. **Inverse BWT-like Reconstruction:** The algorithm iterates through the input buffer (after the header), using the `first_char` and the dynamically updated `table` to reconstruct an intermediate `list` of bytes. This step effectively reverses the Burrows-Wheeler Transform.
    4. **Internal Decompression (`internal_decompress`):
        * The `list` generated from the inverse BWT step is then passed to `internal_decompress`.
        * This function is structurally similar to the `internal_decompress` found in the Mythic codec, also involving frequency counting and the use of `symbol_table`, `frequency_table`, and `partial_input` arrays. It reads frequency data from the first 1024 bytes of its input.
        * It reconstructs the original data by iterating through its input, using the symbol tables and shifting operations.

* **Mathematical Concept:** The Burrows-Wheeler Transform is a powerful reversible block sort that reorders data to improve its compressibility. The subsequent internal decompression step likely uses frequency-based coding to further unpack the data.

---

## Differences Between Mythic and ZlibBwt3

While both are custom compression algorithms used within the Ultima Online ecosystem, they employ different fundamental transforms.

1. **Core Transform:**
    * **Mythic:** Relies on a custom "internal compression" algorithm combined with a standard Move-to-Front (MTF) encoding. It's a frequency-based, dynamic symbol mapping scheme.
    * **ZlibBwt3:** Is based on the Burrows-Wheeler Transform (BWT). This is a distinct and more complex data transformation that reorders data to group similar characters.

2. **Algorithm Stages:**
    * **Mythic:** The compression involves `internal_compress` (custom) followed by `MoveToFrontCoding`. Decompression reverses these steps.
    * **ZlibBwt3:** The decompression involves `BuildTable` and an inverse BWT-like reconstruction, followed by a custom `internal_decompress`. The compression side (not provided in the code) would involve the BWT itself and subsequent encoding steps.

3. **Header Information:**
    * **Mythic:** Uses a 4-byte header that is the original data length XORed with a specific magic number (`0x8E2C9A3D`).
    * **ZlibBwt3:** Uses a 4-byte header (whose full meaning isn't explicitly detailed in the provided C# code) and a `first_char` byte, which is crucial for the inverse BWT process.

4. **Purpose and Evolution:**
    * **Mythic:** Appears to be an older, custom obfuscation or compression method, primarily seen with `GumpartLegacyMul` files. Its original C# implementation's lack of explicit decompression suggests it might have been more about obfuscation or a very specific client-side handling.
    * **ZlibBwt3:** The name `ZlibBwt` suggests a more modern approach, possibly aiming for better compression ratios than simple Zlib by leveraging the BWT. It's likely used in later versions of the game or its assets.

5. **Implementation Details:**
    * Both algorithms share some common helper functions (`frequency`, `shift_left`, `shift_right`) and the general concept of using `partial_input` arrays for managing data. However, the specific logic within their main compression/decompression loops and how they manipulate these arrays are distinct.
    * The `internal_compress` and `internal_decompress` functions, while structurally similar in their use of `partial_input` and symbol tables, have different core logic and operations within their main loops.
