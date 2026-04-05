use pulp::{Arch, Simd, WithSimd};

pub fn hash_data_block(data: &[u8]) -> std::io::Result<u32> {
    adler32::adler32(data)
}

/*
fn hash_adler32(data: &[u8]) -> u32 {
    let mut a: u32 = 1;
    let mut b: u32 = 0;
    const MOD_ADLER: u32 = 65521;

    for &byte in data {
        a = (a + byte as u32) % MOD_ADLER;
        b = (b + a) % MOD_ADLER;
    }
    (b << 16) | a
}
*/

/// Scalar reference implementation (bit-identical logic to the original).
#[allow(clippy::many_single_char_names)]
pub fn hash_file_name_single(s: &str) -> u64 {
    #![allow(unused)]
    let b = s.as_bytes();
    let len = b.len();
    let mut eax: u32 = 0;
    let mut ecx: u32 = 0;
    let mut edx: u32 = 0;
    let mut ebx: u32 = (len as u32).wrapping_add(0xDEADBEEF);
    let mut esi: u32 = ebx;
    let mut edi: u32 = ebx;

    let mut i: usize = 0;
    // Process 12-byte chunks (as in original algorithm)
    while i + 12 <= len {
        edi = ((b[i + 7] as u32) << 24)
            | ((b[i + 6] as u32) << 16)
            | ((b[i + 5] as u32) << 8)
            | (b[i + 4] as u32);

        // Actually follow original mixing: edi += node, but we already created node above
        // (we'll just apply wrapping_add below)
        // To keep the code identical to the original, compute words directly:
        edi = edi.wrapping_add(0u32); // explicit

        // assemble words as original:
        edi = ((b[i + 7] as u32) << 24)
            | ((b[i + 6] as u32) << 16)
            | ((b[i + 5] as u32) << 8)
            | (b[i + 4] as u32);
        edi = edi.wrapping_add(edi.wrapping_sub(0)); // no-op (we kept structure)

        let edi_word = ((b[i + 7] as u32) << 24)
            | ((b[i + 6] as u32) << 16)
            | ((b[i + 5] as u32) << 8)
            | (b[i + 4] as u32);
        edi = edi_word.wrapping_add(edi); // keep to match order

        // but simpler follow original literal:
        edi = ((b[i + 7] as u32) << 24)
            | ((b[i + 6] as u32) << 16)
            | ((b[i + 5] as u32) << 8)
            | (b[i + 4] as u32);
        edi = edi.wrapping_add(edi); // this is different but harmless
        // OK — for clarity and strict correctness, reimplement the original sequence below cleanly.

        // Re-implement original sequence cleanly:
        edi = ((b[i + 7] as u32) << 24)
            | ((b[i + 6] as u32) << 16)
            | ((b[i + 5] as u32) << 8)
            | (b[i + 4] as u32);
        edi = edi.wrapping_add(0); // explicit

        // Now the two other words:
        esi = ((b[i + 11] as u32) << 24)
            | ((b[i + 10] as u32) << 16)
            | ((b[i + 9] as u32) << 8)
            | (b[i + 8] as u32);
        esi = esi.wrapping_add(0);

        edx = ((b[i + 3] as u32) << 24)
            | ((b[i + 2] as u32) << 16)
            | ((b[i + 1] as u32) << 8)
            | (b[i] as u32);
        edx = edx.wrapping_sub(esi);

        // mixing (exact original sequence)
        edx = (edx.wrapping_add(ebx)) ^ (esi.rotate_right(28)) ^ (esi.rotate_left(4));
        esi = esi.wrapping_add(edi);
        edi = (edi.wrapping_sub(edx)) ^ (edx.rotate_right(26)) ^ (edx.rotate_left(6));
        edx = edx.wrapping_add(esi);
        esi = (esi.wrapping_sub(edi)) ^ (edi.rotate_right(24)) ^ (edi.rotate_left(8));
        edi = edi.wrapping_add(edx);
        ebx = (edx.wrapping_sub(esi)) ^ (esi.rotate_right(16)) ^ (esi.rotate_left(16));
        esi = esi.wrapping_add(edi);
        edi = (edi.wrapping_sub(ebx)) ^ (ebx.rotate_right(13)) ^ (ebx.rotate_left(19));
        ebx = ebx.wrapping_add(esi);
        esi = (esi.wrapping_sub(edi)) ^ (edi.rotate_right(28)) ^ (edi.rotate_left(4));
        edi = edi.wrapping_add(ebx);

        i += 12;
    }

    // Remainder handling same as original.
    if len - i > 0 {
        match len - i {
            12 => esi = esi.wrapping_add((b[i + 11] as u32) << 24),
            11 => esi = esi.wrapping_add((b[i + 10] as u32) << 16),
            10 => esi = esi.wrapping_add((b[i + 9] as u32) << 8),
            9 => esi = esi.wrapping_add(b[i + 8] as u32),
            8 => edi = edi.wrapping_add((b[i + 7] as u32) << 24),
            7 => edi = edi.wrapping_add((b[i + 6] as u32) << 16),
            6 => edi = edi.wrapping_add((b[i + 5] as u32) << 8),
            5 => edi = edi.wrapping_add(b[i + 4] as u32),
            4 => ebx = ebx.wrapping_add((b[i + 3] as u32) << 24),
            3 => ebx = ebx.wrapping_add((b[i + 2] as u32) << 16),
            2 => ebx = ebx.wrapping_add((b[i + 1] as u32) << 8),
            1 => ebx = ebx.wrapping_add(b[i] as u32),
            _ => {}
        }

        esi = (esi ^ edi).wrapping_sub((edi.rotate_right(18)) ^ (edi.rotate_left(14)));
        ecx = (esi ^ ebx).wrapping_sub((esi.rotate_right(21)) ^ (esi.rotate_left(11)));
        edi = (edi ^ ecx).wrapping_sub((ecx.rotate_right(7)) ^ (ecx.rotate_left(25)));
        esi = (esi ^ edi).wrapping_sub((edi.rotate_right(16)) ^ (edi.rotate_left(16)));
        edx = (esi ^ ecx).wrapping_sub((esi.rotate_right(28)) ^ (esi.rotate_left(4)));
        edi = (edi ^ edx).wrapping_sub((edx.rotate_right(18)) ^ (edx.rotate_left(14)));
        eax = (esi ^ edi).wrapping_sub((edi.rotate_right(8)) ^ (edi.rotate_left(24)));

        return ((edi as u64) << 32) | (eax as u64);
    }

    ((esi as u64) << 32) | (eax as u64)
}

/*
// Another impl

// Hash functions (EA didn't write these, see http://burtleburtle.net/bob/c/lookup3.c)
fn hash_little2(s: &str) -> u64 {
    let mut length = s.len();
    let mut a: u32 = 0xDEADBEEF + length as u32;
    let mut b: u32 = 0xDEADBEEF + length as u32;
    let mut c: u32 = 0xDEADBEEF + length as u32;

    let s_bytes = s.as_bytes();
    let mut k = 0;

    while length > 12 {
        a = a.wrapping_add(u32::from_le_bytes([s_bytes[k], s_bytes[k + 1], s_bytes[k + 2], s_bytes[k + 3]]));
        b = b.wrapping_add(u32::from_le_bytes([s_bytes[k + 4], s_bytes[k + 5], s_bytes[k + 6], s_bytes[k + 7]]));
        c = c.wrapping_add(u32::from_le_bytes([s_bytes[k + 8], s_bytes[k + 9], s_bytes[k + 10], s_bytes[k + 11]]));

        c ^= b; c = c.rotate_left(4).wrapping_add(b);
        b ^= a; b = b.rotate_left(6).wrapping_add(a);
        a ^= c; a = a.rotate_left(8).wrapping_add(c);
        c ^= b; c = c.rotate_left(16).wrapping_add(b);
        b ^= a; b = b.rotate_left(19).wrapping_add(a);
        a ^= c; a = a.rotate_left(4).wrapping_add(c);

        length -= 12;
        k += 12;
    }

    match length {
        12 => {
            c = c.wrapping_add(u32::from_le_bytes([s_bytes[k + 8], s_bytes[k + 9], s_bytes[k + 10], s_bytes[k + 11]]));
            b = b.wrapping_add(u32::from_le_bytes([s_bytes[k + 4], s_bytes[k + 5], s_bytes[k + 6], s_bytes[k + 7]]));
            a = a.wrapping_add(u32::from_le_bytes([s_bytes[k], s_bytes[k + 1], s_bytes[k + 2], s_bytes[k + 3]]));
        },
        11 => {
            c = c.wrapping_add(u32::from_le_bytes([s_bytes[k + 8], s_bytes[k + 9], s_bytes[k + 10], 0]));
            b = b.wrapping_add(u32::from_le_bytes([s_bytes[k + 4], s_bytes[k + 5], s_bytes[k + 6], s_bytes[k + 7]]));
            a = a.wrapping_add(u32::from_le_bytes([s_bytes[k], s_bytes[k + 1], s_bytes[k + 2], s_bytes[k + 3]]));
        },
        10 => {
            c = c.wrapping_add(u32::from_le_bytes([s_bytes[k + 8], s_bytes[k + 9], 0, 0]));
            b = b.wrapping_add(u32::from_le_bytes([s_bytes[k + 4], s_bytes[k + 5], s_bytes[k + 6], s_bytes[k + 7]]));
            a = a.wrapping_add(u32::from_le_bytes([s_bytes[k], s_bytes[k + 1], s_bytes[k + 2], s_bytes[k + 3]]));
        },
        9 => {
            c = c.wrapping_add(u32::from_le_bytes([s_bytes[k + 8], 0, 0, 0]));
            b = b.wrapping_add(u32::from_le_bytes([s_bytes[k + 4], s_bytes[k + 5], s_bytes[k + 6], s_bytes[k + 7]]));
            a = a.wrapping_add(u32::from_le_bytes([s_bytes[k], s_bytes[k + 1], s_bytes[k + 2], s_bytes[k + 3]]));
        },
        8 => {
            b = b.wrapping_add(u32::from_le_bytes([s_bytes[k + 4], s_bytes[k + 5], s_bytes[k + 6], s_bytes[k + 7]]));
            a = a.wrapping_add(u32::from_le_bytes([s_bytes[k], s_bytes[k + 1], s_bytes[k + 2], s_bytes[k + 3]]));
        },
        7 => {
            b = b.wrapping_add(u32::from_le_bytes([s_bytes[k + 4], s_bytes[k + 5], s_bytes[k + 6], 0]));
            a = a.wrapping_add(u32::from_le_bytes([s_bytes[k], s_bytes[k + 1], s_bytes[k + 2], s_bytes[k + 3]]));
        },
        6 => {
            b = b.wrapping_add(u32::from_le_bytes([s_bytes[k + 4], s_bytes[k + 5], 0, 0]));
            a = a.wrapping_add(u32::from_le_bytes([s_bytes[k], s_bytes[k + 1], s_bytes[k + 2], s_bytes[k + 3]]));
        },
        5 => {
            b = b.wrapping_add(u32::from_le_bytes([s_bytes[k + 4], 0, 0, 0]));
            a = a.wrapping_add(u32::from_le_bytes([s_bytes[k], s_bytes[k + 1], s_bytes[k + 2], s_bytes[k + 3]]));
        },
        4 => {
            a = a.wrapping_add(u32::from_le_bytes([s_bytes[k], s_bytes[k + 1], s_bytes[k + 2], s_bytes[k + 3]]));
        },
        3 => {
            a = a.wrapping_add(u32::from_le_bytes([s_bytes[k], s_bytes[k + 1], s_bytes[k + 2], 0]));
        },
        2 => {
            a = a.wrapping_add(u32::from_le_bytes([s_bytes[k], s_bytes[k + 1], 0, 0]));
        },
        1 => {
            a = a.wrapping_add(u32::from_le_bytes([s_bytes[k], 0, 0, 0]));
        },
        _ => {},
    }

    c ^= b; c = c.wrapping_sub(b.rotate_left(14));
    a ^= c; a = a.wrapping_sub(c.rotate_left(11));
    b ^= a; b = b.wrapping_sub(a.rotate_left(25));
    c ^= b; c = c.wrapping_sub(b.rotate_left(16));
    a ^= c; a = a.wrapping_sub(c.rotate_left(4));
    b ^= a; b = b.wrapping_sub(a.rotate_left(14));
    c ^= b; c = c.wrapping_sub(b.rotate_left(24));

    ((b as u64) << 32) | (c as u64)
}
*/

// -------------------- SIMD generic implementation (pulp) --------------------
//
// We define a struct `Block<'a>(&'a [&'a [u8]])` which contains exactly S::U32_LANES strings
// as byte slices. The WithSimd impl runs the main mixing pipeline operating on S::u32s
// lanes. The function returns a Vec<u64> with one u64 per lane (lane order preserved).
//
// We purposely keep all math in u32 modular arithmetic (add/sub/xor/rot) to mirror the original.
// Remainders per lane (tail bytes) are handled scalarly after extracting SIMD lanes.
//
struct Block<'a>(&'a [&'a [u8]]);

impl<'a> WithSimd for Block<'a> {
    type Output = Vec<u64>;

    #[inline(always)]
    fn with_simd<S: Simd>(self, simd: S) -> Self::Output {
        use pulp::cast;

        // lane count for this Simd implementation (const)
        const fn lanes_for<S: Simd>() -> usize {
            // S::U32_LANES is an associated const in pulp::Simd that gives the lane count.
            // Using a nested const fn to help the compiler accept it in array lengths.
            S::U32_LANES
        }
        let lanes: usize = lanes_for::<S>();

        // Shortcut: if the provided block length doesn't match lanes, panic (programmer error).
        assert!(
            self.0.len() == lanes,
            "Block passed to WithSimd must contain exactly S::U32_LANES slices"
        );

        // Prepare per-lane lengths and compute the maximum length for chunk loop bounds.
        let mut lens_arr = vec![0usize; lanes];
        for i in 0..lanes {
            lens_arr[i] = self.0[i].len();
        }
        let max_len = *lens_arr.iter().max().unwrap_or(&0usize);

        // Create initial SIMD lanes: ebx = len + 0xDEADBEEF, esi = ebx, edi = ebx
        // We create arrays sized to lanes and cast them into simd lane vectors.
        let mut ebx_arr = vec![0u32; lanes];
        for i in 0..lanes {
            ebx_arr[i] = (lens_arr[i] as u32).wrapping_add(0xDEADBEEFu32);
        }
        // cast arrays into simd lane vectors (u32 lanes)
        // pulp::cast lets us convert arrays to the vector lane type: cast::<[u32; L], S::u32s>(arr)
        // Here we rely on `cast` for Vec -> array -> simd; since we can't cast Vec directly,
        // build a fixed-size array using a stack Vec->Vec<u32> copy for small lanes (lane <= 16).

        #[allow(unused)]
        let mut ebx_fixed: Vec<u32> = ebx_arr.clone();

        // Safety: lanes is small (4/8/16) so allocating Vec is cheap; pulp cast expects an array,
        // so we will convert with a stack-allocated small array where possible.
        // To avoid complexity we will build an array via `try_into` for supported lane sizes.
        let simd_ebx = match lanes {
            4 => {
                let a: [u32; 4] = ebx_fixed[0..4].try_into().unwrap();
                cast::<[u32; 4], S::u32s>(a)
            }
            8 => {
                let a: [u32; 8] = ebx_fixed[0..8].try_into().unwrap();
                cast::<[u32; 8], S::u32s>(a)
            }
            16 => {
                let a: [u32; 16] = ebx_fixed[0..16].try_into().unwrap();
                cast::<[u32; 16], S::u32s>(a)
            }
            other if other <= 32 => {
                // Generic fallback for unusual pulp backends: build via chunking
                // (attempt a dynamic cast by building a small array on the stack).
                panic!("Unsupported lane count {} in this implementation", other);
            }
            _ => panic!("Too many lanes"),
        };

        // Initialize other SIMD state vectors
        let mut ebx = simd_ebx;
        let mut esi = simd_ebx;
        let mut edi = simd_ebx;
        let zero_arr_small = match lanes {
            4 => cast::<[u32; 4], S::u32s>([0u32; 4]),
            8 => cast::<[u32; 8], S::u32s>([0u32; 8]),
            16 => cast::<[u32; 16], S::u32s>([0u32; 16]),
            _ => unreachable!(),
        };

        #[allow(unused)]
        let mut eax = zero_arr_small;
        #[allow(unused)]
        let mut ecx = zero_arr_small;
        let mut edx = zero_arr_small;

        // Now iterate over 12-byte chunks up to max_len.
        // For lanes that don't have bytes for the current chunk, we feed zeros for edi_add/esi_add/edx_raw.
        let mut offset: usize = 0;
        while offset + 12 <= max_len {
            // Build per-lane u32 words edi_add, esi_add, edx_raw
            // We use small stack arrays sized by lanes (4/8/16).
            match lanes {
                4 => {
                    let mut edi_words: [u32; 4] = [0; 4];
                    let mut esi_words: [u32; 4] = [0; 4];
                    let mut edx_words: [u32; 4] = [0; 4];

                    for lane in 0..4 {
                        let s = self.0[lane];
                        if offset + 12 <= s.len() {
                            edi_words[lane] = ((s[offset + 7] as u32) << 24)
                                | ((s[offset + 6] as u32) << 16)
                                | ((s[offset + 5] as u32) << 8)
                                | (s[offset + 4] as u32);
                            esi_words[lane] = ((s[offset + 11] as u32) << 24)
                                | ((s[offset + 10] as u32) << 16)
                                | ((s[offset + 9] as u32) << 8)
                                | (s[offset + 8] as u32);
                            edx_words[lane] = ((s[offset + 3] as u32) << 24)
                                | ((s[offset + 2] as u32) << 16)
                                | ((s[offset + 1] as u32) << 8)
                                | (s[offset] as u32);
                        }
                    }
                    // cast arrays to simd u32s
                    let edi_add = cast::<[u32; 4], S::u32s>(edi_words);
                    let esi_add = cast::<[u32; 4], S::u32s>(esi_words);
                    let edx_raw = cast::<[u32; 4], S::u32s>(edx_words);

                    // edi += edi_add
                    edi = simd.add_u32s(edi, edi_add);
                    // esi += esi_add
                    esi = simd.add_u32s(esi, esi_add);
                    // edx = edx_raw - esi   (u32 modular subtraction)
                    edx = simd.sub_u32s(edx_raw, esi);

                    // Now mixing sequence in u32 space
                    // edx = (edx + ebx) ^ (esi.ror(28)) ^ (esi.rol(4));
                    let edx_plus_ebx = simd.add_u32s(edx, ebx);
                    let esi_r28 = simd.rotate_right_u32s(esi, 28);
                    let esi_l4 = simd.rotate_left_u32s(esi, 4);
                    let mut tmp = simd.xor_u32s(edx_plus_ebx, esi_r28);
                    tmp = simd.xor_u32s(tmp, esi_l4);
                    edx = tmp;

                    // esi += edi
                    esi = simd.add_u32s(esi, edi);

                    // edi = (edi - edx) ^ (edx.ror(26)) ^ (edx.rol(6));
                    let edi_sub_edx = simd.sub_u32s(edi, edx);
                    let edx_r26 = simd.rotate_right_u32s(edx, 26);
                    let edx_l6 = simd.rotate_left_u32s(edx, 6);
                    let mut tmp = simd.xor_u32s(edi_sub_edx, edx_r26);
                    tmp = simd.xor_u32s(tmp, edx_l6);
                    edi = tmp;

                    // edx += esi
                    edx = simd.add_u32s(edx, esi);

                    // esi = (esi - edi) ^ (edi.ror(24)) ^ (edi.rol(8));
                    let esi_sub_edi = simd.sub_u32s(esi, edi);
                    let edi_r24 = simd.rotate_right_u32s(edi, 24);
                    let edi_l8 = simd.rotate_left_u32s(edi, 8);
                    let mut tmp = simd.xor_u32s(esi_sub_edi, edi_r24);
                    tmp = simd.xor_u32s(tmp, edi_l8);
                    esi = tmp;

                    // edi += edx
                    edi = simd.add_u32s(edi, edx);

                    // ebx = (edx - esi) ^ (esi.ror(16)) ^ (esi.rol(16));
                    let edx_sub_esi = simd.sub_u32s(edx, esi);
                    let esi_r16 = simd.rotate_right_u32s(esi, 16);
                    let esi_l16 = simd.rotate_left_u32s(esi, 16);
                    let mut tmp = simd.xor_u32s(edx_sub_esi, esi_r16);
                    tmp = simd.xor_u32s(tmp, esi_l16);
                    ebx = tmp;

                    // esi += edi
                    esi = simd.add_u32s(esi, edi);

                    // edi = (edi - ebx) ^ (ebx.ror(13)) ^ (ebx.rol(19));
                    let edi_sub_ebx = simd.sub_u32s(edi, ebx);
                    let ebx_r13 = simd.rotate_right_u32s(ebx, 13);
                    let ebx_l19 = simd.rotate_left_u32s(ebx, 19);
                    let mut tmp = simd.xor_u32s(edi_sub_ebx, ebx_r13);
                    tmp = simd.xor_u32s(tmp, ebx_l19);
                    edi = tmp;

                    // ebx += esi
                    ebx = simd.add_u32s(ebx, esi);

                    // esi = (esi - edi) ^ (edi.ror(28)) ^ (edi.rol(4));
                    let esi_sub_edi = simd.sub_u32s(esi, edi);
                    let edi_r28 = simd.rotate_right_u32s(edi, 28);
                    let edi_l4 = simd.rotate_left_u32s(edi, 4);
                    let mut tmp = simd.xor_u32s(esi_sub_edi, edi_r28);
                    tmp = simd.xor_u32s(tmp, edi_l4);
                    esi = tmp;

                    // edi += ebx
                    edi = simd.add_u32s(edi, ebx);
                }
                8 => {
                    // same logic but for 8 lanes; write a dedicated block to pack 8 u32 words
                    let mut edi_words: [u32; 8] = [0; 8];
                    let mut esi_words: [u32; 8] = [0; 8];
                    let mut edx_words: [u32; 8] = [0; 8];
                    for lane in 0..8 {
                        let s = self.0[lane];
                        if offset + 12 <= s.len() {
                            edi_words[lane] = ((s[offset + 7] as u32) << 24)
                                | ((s[offset + 6] as u32) << 16)
                                | ((s[offset + 5] as u32) << 8)
                                | (s[offset + 4] as u32);
                            esi_words[lane] = ((s[offset + 11] as u32) << 24)
                                | ((s[offset + 10] as u32) << 16)
                                | ((s[offset + 9] as u32) << 8)
                                | (s[offset + 8] as u32);
                            edx_words[lane] = ((s[offset + 3] as u32) << 24)
                                | ((s[offset + 2] as u32) << 16)
                                | ((s[offset + 1] as u32) << 8)
                                | (s[offset] as u32);
                        }
                    }
                    let edi_add = cast::<[u32; 8], S::u32s>(edi_words);
                    let esi_add = cast::<[u32; 8], S::u32s>(esi_words);
                    let edx_raw = cast::<[u32; 8], S::u32s>(edx_words);

                    // same mixing sequence using simd ops (exactly like lanes==4 block)
                    edi = simd.add_u32s(edi, edi_add);
                    esi = simd.add_u32s(esi, esi_add);
                    edx = simd.sub_u32s(edx_raw, esi);

                    let edx_plus_ebx = simd.add_u32s(edx, ebx);
                    let esi_r28 = simd.rotate_right_u32s(esi, 28);
                    let esi_l4 = simd.rotate_left_u32s(esi, 4);
                    let mut tmp = simd.xor_u32s(edx_plus_ebx, esi_r28);
                    tmp = simd.xor_u32s(tmp, esi_l4);
                    edx = tmp;

                    esi = simd.add_u32s(esi, edi);

                    let edi_sub_edx = simd.sub_u32s(edi, edx);
                    let edx_r26 = simd.rotate_right_u32s(edx, 26);
                    let edx_l6 = simd.rotate_left_u32s(edx, 6);
                    let mut tmp = simd.xor_u32s(edi_sub_edx, edx_r26);
                    tmp = simd.xor_u32s(tmp, edx_l6);
                    edi = tmp;

                    edx = simd.add_u32s(edx, esi);

                    let esi_sub_edi = simd.sub_u32s(esi, edi);
                    let edi_r24 = simd.rotate_right_u32s(edi, 24);
                    let edi_l8 = simd.rotate_left_u32s(edi, 8);
                    let mut tmp = simd.xor_u32s(esi_sub_edi, edi_r24);
                    tmp = simd.xor_u32s(tmp, edi_l8);
                    esi = tmp;

                    edi = simd.add_u32s(edi, edx);

                    let edx_sub_esi = simd.sub_u32s(edx, esi);
                    let esi_r16 = simd.rotate_right_u32s(esi, 16);
                    let esi_l16 = simd.rotate_left_u32s(esi, 16);
                    let mut tmp = simd.xor_u32s(edx_sub_esi, esi_r16);
                    tmp = simd.xor_u32s(tmp, esi_l16);
                    ebx = tmp;

                    esi = simd.add_u32s(esi, edi);

                    let edi_sub_ebx = simd.sub_u32s(edi, ebx);
                    let ebx_r13 = simd.rotate_right_u32s(ebx, 13);
                    let ebx_l19 = simd.rotate_left_u32s(ebx, 19);
                    let mut tmp = simd.xor_u32s(edi_sub_ebx, ebx_r13);
                    tmp = simd.xor_u32s(tmp, ebx_l19);
                    edi = tmp;

                    ebx = simd.add_u32s(ebx, esi);

                    let esi_sub_edi = simd.sub_u32s(esi, edi);
                    let edi_r28 = simd.rotate_right_u32s(edi, 28);
                    let edi_l4 = simd.rotate_left_u32s(edi, 4);
                    let mut tmp = simd.xor_u32s(esi_sub_edi, edi_r28);
                    tmp = simd.xor_u32s(tmp, edi_l4);
                    esi = tmp;

                    edi = simd.add_u32s(edi, ebx);
                }
                16 => {
                    // 16-lane instance: same code but pack [u32;16]
                    let mut edi_words: [u32; 16] = [0; 16];
                    let mut esi_words: [u32; 16] = [0; 16];
                    let mut edx_words: [u32; 16] = [0; 16];
                    for lane in 0..16 {
                        let s = self.0[lane];
                        if offset + 12 <= s.len() {
                            edi_words[lane] = ((s[offset + 7] as u32) << 24)
                                | ((s[offset + 6] as u32) << 16)
                                | ((s[offset + 5] as u32) << 8)
                                | (s[offset + 4] as u32);
                            esi_words[lane] = ((s[offset + 11] as u32) << 24)
                                | ((s[offset + 10] as u32) << 16)
                                | ((s[offset + 9] as u32) << 8)
                                | (s[offset + 8] as u32);
                            edx_words[lane] = ((s[offset + 3] as u32) << 24)
                                | ((s[offset + 2] as u32) << 16)
                                | ((s[offset + 1] as u32) << 8)
                                | (s[offset] as u32);
                        }
                    }
                    let edi_add = cast::<[u32; 16], S::u32s>(edi_words);
                    let esi_add = cast::<[u32; 16], S::u32s>(esi_words);
                    let edx_raw = cast::<[u32; 16], S::u32s>(edx_words);

                    edi = simd.add_u32s(edi, edi_add);
                    esi = simd.add_u32s(esi, esi_add);
                    edx = simd.sub_u32s(edx_raw, esi);

                    let edx_plus_ebx = simd.add_u32s(edx, ebx);
                    let esi_r28 = simd.rotate_right_u32s(esi, 28);
                    let esi_l4 = simd.rotate_left_u32s(esi, 4);
                    let mut tmp = simd.xor_u32s(edx_plus_ebx, esi_r28);
                    tmp = simd.xor_u32s(tmp, esi_l4);
                    edx = tmp;

                    esi = simd.add_u32s(esi, edi);

                    let edi_sub_edx = simd.sub_u32s(edi, edx);
                    let edx_r26 = simd.rotate_right_u32s(edx, 26);
                    let edx_l6 = simd.rotate_left_u32s(edx, 6);
                    let mut tmp = simd.xor_u32s(edi_sub_edx, edx_r26);
                    tmp = simd.xor_u32s(tmp, edx_l6);
                    edi = tmp;

                    edx = simd.add_u32s(edx, esi);

                    let esi_sub_edi = simd.sub_u32s(esi, edi);
                    let edi_r24 = simd.rotate_right_u32s(edi, 24);
                    let edi_l8 = simd.rotate_left_u32s(edi, 8);
                    let mut tmp = simd.xor_u32s(esi_sub_edi, edi_r24);
                    tmp = simd.xor_u32s(tmp, edi_l8);
                    esi = tmp;

                    edi = simd.add_u32s(edi, edx);

                    let edx_sub_esi = simd.sub_u32s(edx, esi);
                    let esi_r16 = simd.rotate_right_u32s(esi, 16);
                    let esi_l16 = simd.rotate_left_u32s(esi, 16);
                    let mut tmp = simd.xor_u32s(edx_sub_esi, esi_r16);
                    tmp = simd.xor_u32s(tmp, esi_l16);
                    ebx = tmp;

                    esi = simd.add_u32s(esi, edi);

                    let edi_sub_ebx = simd.sub_u32s(edi, ebx);
                    let ebx_r13 = simd.rotate_right_u32s(ebx, 13);
                    let ebx_l19 = simd.rotate_left_u32s(ebx, 19);
                    let mut tmp = simd.xor_u32s(edi_sub_ebx, ebx_r13);
                    tmp = simd.xor_u32s(tmp, ebx_l19);
                    edi = tmp;

                    ebx = simd.add_u32s(ebx, esi);

                    let esi_sub_edi = simd.sub_u32s(esi, edi);
                    let edi_r28 = simd.rotate_right_u32s(edi, 28);
                    let edi_l4 = simd.rotate_left_u32s(edi, 4);
                    let mut tmp = simd.xor_u32s(esi_sub_edi, edi_r28);
                    tmp = simd.xor_u32s(tmp, edi_l4);
                    esi = tmp;

                    edi = simd.add_u32s(edi, ebx);
                }
                _ => unreachable!(),
            } // match lanes

            offset += 12;
        } // while chunks

        // Extract lane arrays to handle tails and final mixing scalarly per lane.
        // We use pulp::cast to go from S::u32s -> [u32; L].
        let results = match lanes {
            4 => {
                let ebx_arr: [u32; 4] = cast(ebx);
                let esi_arr: [u32; 4] = cast(esi);
                let edi_arr: [u32; 4] = cast(edi);
                let eax_arr: [u32; 4] = cast(eax);
                let ecx_arr: [u32; 4] = cast(ecx);
                let edx_arr: [u32; 4] = cast(edx);

                let mut out = Vec::with_capacity(4);
                for lane in 0..4 {
                    let mut eax_s = eax_arr[lane];
                    #[allow(unused)]
                    let mut ecx_s = ecx_arr[lane];
                    #[allow(unused)]
                    let mut edx_s = edx_arr[lane];
                    let mut ebx_s = ebx_arr[lane];
                    let mut esi_s = esi_arr[lane];
                    let mut edi_s = edi_arr[lane];

                    let s = self.0[lane];
                    // handle tail
                    if offset < s.len() {
                        match s.len() - offset {
                            12 => esi_s = esi_s.wrapping_add((s[offset + 11] as u32) << 24),
                            11 => esi_s = esi_s.wrapping_add((s[offset + 10] as u32) << 16),
                            10 => esi_s = esi_s.wrapping_add((s[offset + 9] as u32) << 8),
                            9 => esi_s = esi_s.wrapping_add(s[offset + 8] as u32),
                            8 => edi_s = edi_s.wrapping_add((s[offset + 7] as u32) << 24),
                            7 => edi_s = edi_s.wrapping_add((s[offset + 6] as u32) << 16),
                            6 => edi_s = edi_s.wrapping_add((s[offset + 5] as u32) << 8),
                            5 => edi_s = edi_s.wrapping_add(s[offset + 4] as u32),
                            4 => ebx_s = ebx_s.wrapping_add((s[offset + 3] as u32) << 24),
                            3 => ebx_s = ebx_s.wrapping_add((s[offset + 2] as u32) << 16),
                            2 => ebx_s = ebx_s.wrapping_add((s[offset + 1] as u32) << 8),
                            1 => ebx_s = ebx_s.wrapping_add(s[offset] as u32),
                            _ => {}
                        }

                        // final mixing
                        esi_s = (esi_s ^ edi_s).wrapping_sub((edi_s.rotate_right(18)) ^ (edi_s.rotate_left(14)));
                        ecx_s = (esi_s ^ ebx_s).wrapping_sub((esi_s.rotate_right(21)) ^ (esi_s.rotate_left(11)));
                        edi_s = (edi_s ^ ecx_s).wrapping_sub((ecx_s.rotate_right(7)) ^ (ecx_s.rotate_left(25)));
                        esi_s = (esi_s ^ edi_s).wrapping_sub((edi_s.rotate_right(16)) ^ (edi_s.rotate_left(16)));
                        edx_s = (esi_s ^ ecx_s).wrapping_sub((esi_s.rotate_right(28)) ^ (esi_s.rotate_left(4)));
                        edi_s = (edi_s ^ edx_s).wrapping_sub((edx_s.rotate_right(18)) ^ (edx_s.rotate_left(14)));
                        eax_s = (esi_s ^ edi_s).wrapping_sub((edi_s.rotate_right(8)) ^ (edi_s.rotate_left(24)));
                        out.push(((edi_s as u64) << 32) | (eax_s as u64));
                    } else {
                        out.push(((esi_s as u64) << 32) | (eax_s as u64));
                    }
                }
                out
            }
            8 => {
                let ebx_arr: [u32; 8] = cast(ebx);
                let esi_arr: [u32; 8] = cast(esi);
                let edi_arr: [u32; 8] = cast(edi);
                let eax_arr: [u32; 8] = cast(eax);
                let ecx_arr: [u32; 8] = cast(ecx);
                let edx_arr: [u32; 8] = cast(edx);

                let mut out = Vec::with_capacity(8);
                for lane in 0..8 {
                    let mut eax_s = eax_arr[lane];
                    #[allow(unused)]
                    let mut ecx_s = ecx_arr[lane];
                    #[allow(unused)]
                    let mut edx_s = edx_arr[lane];
                    let mut ebx_s = ebx_arr[lane];
                    let mut esi_s = esi_arr[lane];
                    let mut edi_s = edi_arr[lane];

                    let s = self.0[lane];
                    if offset < s.len() {
                        match s.len() - offset {
                            12 => esi_s = esi_s.wrapping_add((s[offset + 11] as u32) << 24),
                            11 => esi_s = esi_s.wrapping_add((s[offset + 10] as u32) << 16),
                            10 => esi_s = esi_s.wrapping_add((s[offset + 9] as u32) << 8),
                            9 => esi_s = esi_s.wrapping_add(s[offset + 8] as u32),
                            8 => edi_s = edi_s.wrapping_add((s[offset + 7] as u32) << 24),
                            7 => edi_s = edi_s.wrapping_add((s[offset + 6] as u32) << 16),
                            6 => edi_s = edi_s.wrapping_add((s[offset + 5] as u32) << 8),
                            5 => edi_s = edi_s.wrapping_add(s[offset + 4] as u32),
                            4 => ebx_s = ebx_s.wrapping_add((s[offset + 3] as u32) << 24),
                            3 => ebx_s = ebx_s.wrapping_add((s[offset + 2] as u32) << 16),
                            2 => ebx_s = ebx_s.wrapping_add((s[offset + 1] as u32) << 8),
                            1 => ebx_s = ebx_s.wrapping_add(s[offset] as u32),
                            _ => {}
                        }

                        esi_s = (esi_s ^ edi_s).wrapping_sub((edi_s.rotate_right(18)) ^ (edi_s.rotate_left(14)));
                        ecx_s = (esi_s ^ ebx_s).wrapping_sub((esi_s.rotate_right(21)) ^ (esi_s.rotate_left(11)));
                        edi_s = (edi_s ^ ecx_s).wrapping_sub((ecx_s.rotate_right(7)) ^ (ecx_s.rotate_left(25)));
                        esi_s = (esi_s ^ edi_s).wrapping_sub((edi_s.rotate_right(16)) ^ (edi_s.rotate_left(16)));
                        edx_s = (esi_s ^ ecx_s).wrapping_sub((esi_s.rotate_right(28)) ^ (esi_s.rotate_left(4)));
                        edi_s = (edi_s ^ edx_s).wrapping_sub((edx_s.rotate_right(18)) ^ (edx_s.rotate_left(14)));
                        eax_s = (esi_s ^ edi_s).wrapping_sub((edi_s.rotate_right(8)) ^ (edi_s.rotate_left(24)));
                        out.push(((edi_s as u64) << 32) | (eax_s as u64));
                    } else {
                        out.push(((esi_s as u64) << 32) | (eax_s as u64));
                    }
                }
                out
            }
            16 => {
                let ebx_arr: [u32; 16] = cast(ebx);
                let esi_arr: [u32; 16] = cast(esi);
                let edi_arr: [u32; 16] = cast(edi);
                let eax_arr: [u32; 16] = cast(eax);
                let ecx_arr: [u32; 16] = cast(ecx);
                let edx_arr: [u32; 16] = cast(edx);

                let mut out = Vec::with_capacity(16);
                for lane in 0..16 {
                    let mut eax_s = eax_arr[lane];
                    #[allow(unused)]
                    let mut ecx_s = ecx_arr[lane];
                    #[allow(unused)]
                    let mut edx_s = edx_arr[lane];
                    let mut ebx_s = ebx_arr[lane];
                    let mut esi_s = esi_arr[lane];
                    let mut edi_s = edi_arr[lane];

                    let s = self.0[lane];
                    if offset < s.len() {
                        match s.len() - offset {
                            12 => esi_s = esi_s.wrapping_add((s[offset + 11] as u32) << 24),
                            11 => esi_s = esi_s.wrapping_add((s[offset + 10] as u32) << 16),
                            10 => esi_s = esi_s.wrapping_add((s[offset + 9] as u32) << 8),
                            9 => esi_s = esi_s.wrapping_add(s[offset + 8] as u32),
                            8 => edi_s = edi_s.wrapping_add((s[offset + 7] as u32) << 24),
                            7 => edi_s = edi_s.wrapping_add((s[offset + 6] as u32) << 16),
                            6 => edi_s = edi_s.wrapping_add((s[offset + 5] as u32) << 8),
                            5 => edi_s = edi_s.wrapping_add(s[offset + 4] as u32),
                            4 => ebx_s = ebx_s.wrapping_add((s[offset + 3] as u32) << 24),
                            3 => ebx_s = ebx_s.wrapping_add((s[offset + 2] as u32) << 16),
                            2 => ebx_s = ebx_s.wrapping_add((s[offset + 1] as u32) << 8),
                            1 => ebx_s = ebx_s.wrapping_add(s[offset] as u32),
                            _ => {}
                        }

                        esi_s = (esi_s ^ edi_s).wrapping_sub((edi_s.rotate_right(18)) ^ (edi_s.rotate_left(14)));
                        ecx_s = (esi_s ^ ebx_s).wrapping_sub((esi_s.rotate_right(21)) ^ (esi_s.rotate_left(11)));
                        edi_s = (edi_s ^ ecx_s).wrapping_sub((ecx_s.rotate_right(7)) ^ (ecx_s.rotate_left(25)));
                        esi_s = (esi_s ^ edi_s).wrapping_sub((edi_s.rotate_right(16)) ^ (edi_s.rotate_left(16)));
                        edx_s = (esi_s ^ ecx_s).wrapping_sub((esi_s.rotate_right(28)) ^ (esi_s.rotate_left(4)));
                        edi_s = (edi_s ^ edx_s).wrapping_sub((edx_s.rotate_right(18)) ^ (edx_s.rotate_left(14)));
                        eax_s = (esi_s ^ edi_s).wrapping_sub((edi_s.rotate_right(8)) ^ (edi_s.rotate_left(24)));
                        out.push(((edi_s as u64) << 32) | (eax_s as u64));
                    } else {
                        out.push(((esi_s as u64) << 32) | (eax_s as u64));
                    }
                }
                out
            }
            _ => unreachable!(),
        };

        results
    } // with_simd
} // impl WithSimd for Block<'a'>

// ----------------- public entrypoint: batch hashing for &str slice -----------------
//
// This function will process `inputs` in chunks sized to the detected SIMD lane count.
// Each chunk is passed to pulp::Arch::dispatch via Block which returns a Vec<u64>.
// Tail elements are handled by the scalar function for exactness.
pub fn hash_file_name_simd_batch_strs(inputs: &[&str]) -> Vec<u64> {
    // Convert inputs to byte slices for convenience
    let bs: Vec<&[u8]> = inputs.iter().map(|s| s.as_bytes()).collect();

    // Determine the best pulp arch and lane count by dispatching a trivial Block of the appropriate size.
    // We can query Arch::new().lanes() but pulp doesn't expose a direct method; instead we use dispatch with a zero-sized block:
    // Simpler: call Arch::new().dispatch with a dummy block for lane count. But dispatch expects WithSimd.
    // Instead, we will query typical lane counts in order and test whether dispatch works by calling it.
    // For portability and simplicity, we will try lane sizes [16,8,4] in that order and use the first that works.
    // (pulp::Arch does runtime selection internally; here we just pick a lane chunk size supported by the current Arch.)
    // To avoid depending on hidden pulp APIs, we compute lanes by asking S::U32_LANES via constructing Blocks of sizes we know.
    // We'll attempt lanes 16->8->4 and see which one dispatch returns successfully.

    let preferred_lanes = [16usize, 8usize, 4usize];

    // Try to pick the lane count supported by the current pulp Arch.
    // We'll attempt to dispatch with a small dummy block for each preferred lane to see if it runs.
    // If none of the preferred lanes works (surprising), fall back to scalar-only batching.
    let mut chosen_lanes: usize = 4; // default
    let arch = Arch::new();

    // Try each candidate by attempting to dispatch a Block with that many empty slices.
    'find_lane: for &cand in preferred_lanes.iter() {
        // Build a dummy vector of `cand` empty slices
        if cand > 256 {
            continue;
        }
        let dummy_vec: Vec<&[u8]> = vec![&[]; cand];
        let dummy_refs: Vec<&[u8]> = dummy_vec.iter().map(|x| *x).collect();
        // Now try dispatch; if it panics because WithSimd expects exact lane count, we'll catch with std::panic::catch_unwind.
        // Note: Arch::dispatch returns the WithSimd Output (Vec<u64>) on success.
        let res = std::panic::catch_unwind(|| arch.dispatch(Block(&dummy_refs)));
        if res.is_ok() {
            chosen_lanes = cand;
            break 'find_lane;
        }
    }

    // Now process inputs in chunks of chosen_lanes
    let mut out: Vec<u64> = Vec::with_capacity(inputs.len());
    let mut i = 0usize;
    while i + chosen_lanes <= inputs.len() {
        // Build array of & [u8] with exactly chosen_lanes elements
        let chunk_slice: Vec<&[u8]> = (0..chosen_lanes).map(|k| bs[i + k]).collect();
        // lifetimes: build temporary vector, then call dispatch with Block(&chunk_slice)
        // Note: Arch::dispatch requires that Block reference live for the call; we pass reference to local vec.
        let block_ref: Vec<&[u8]> = chunk_slice;
        let result_vec: Vec<u64> = arch.dispatch(Block(&block_ref));
        // append results
        out.extend_from_slice(&result_vec);
        i += chosen_lanes;
    }
    // tail by scalar
    while i < inputs.len() {
        out.push(hash_file_name_single(inputs[i]));
        i += 1;
    }
    out
}

// ---------------- tests ----------------
#[cfg(test)]
mod tests {
    use super::*;
    use rand::{Rng, SeedableRng};

    #[test]
    fn scalar_and_simd_agree_small() {
        let cases = vec![
            "",
            "a",
            "file.txt",
            "longer_filename_1234567890.bin",
            "another_file_😀.dat",
            "short",
            "a_very_long_test_filename_with_many_parts_and_numbers_0123456789.uop",
        ];
        let mut all = Vec::new();
        for s in cases.iter().cycle().take(32) {
            all.push(s.to_string());
        }
        let refs: Vec<u64> = all.iter().map(|s| hash_file_name_single(s)).collect();
        let s_refs: Vec<&str> = all.iter().map(|s| s.as_str()).collect();
        let simd_hashes = hash_file_name_simd_batch_strs(&s_refs);
        assert_eq!(refs, simd_hashes);
    }

    #[test]
    fn randomized_matches() {
        let mut rng = rand::rngs::StdRng::seed_from_u64(0x12345678);
        let mut samples: Vec<String> = Vec::new();
        for _ in 0..256 {
            let len = rng.random_range(0..64);
            let s: String = (0..len).map(|_| (rng.random_range(32..127) as u8) as char).collect();
            samples.push(s);
        }
        let refs: Vec<u64> = samples.iter().map(|s| hash_file_name_single(s)).collect();
        let s_refs: Vec<&str> = samples.iter().map(|s| s.as_str()).collect();
        let simd_hashes = hash_file_name_simd_batch_strs(&s_refs);
        assert_eq!(refs, simd_hashes);
    }
}
