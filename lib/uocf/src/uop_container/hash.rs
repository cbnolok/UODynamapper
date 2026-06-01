use std::collections::BTreeMap;

use bytemuck::cast;
use wide::{u32x4, u32x8};

pub fn hash_data_block(data: &[u8]) -> std::io::Result<u32> {
    adler32::adler32(data)
}

trait HashWord:
    Copy
    + std::ops::BitOr<Output = Self>
    + std::ops::BitXor<Output = Self>
    + std::ops::Shl<u32, Output = Self>
    + std::ops::Shr<u32, Output = Self>
{
    fn wrapping_add(self, rhs: Self) -> Self;
    fn wrapping_sub(self, rhs: Self) -> Self;
}

impl HashWord for u32 {
    #[inline(always)]
    fn wrapping_add(self, rhs: Self) -> Self {
        self.wrapping_add(rhs)
    }

    #[inline(always)]
    fn wrapping_sub(self, rhs: Self) -> Self {
        self.wrapping_sub(rhs)
    }
}

impl HashWord for u32x4 {
    #[inline(always)]
    fn wrapping_add(self, rhs: Self) -> Self {
        self + rhs
    }

    #[inline(always)]
    fn wrapping_sub(self, rhs: Self) -> Self {
        self - rhs
    }
}

impl HashWord for u32x8 {
    #[inline(always)]
    fn wrapping_add(self, rhs: Self) -> Self {
        self + rhs
    }

    #[inline(always)]
    fn wrapping_sub(self, rhs: Self) -> Self {
        self - rhs
    }
}

#[inline(always)]
fn shift_xor<T>(value: T, right: u32, left: u32) -> T
where
    T: HashWord,
{
    (value >> right) ^ (value << left)
}

#[inline(always)]
fn mix_block<T>(mut ebx: T, mut esi: T, mut edi: T, word0: T, word1: T, word2: T) -> (T, T, T)
where
    T: HashWord,
{
    edi = edi.wrapping_add(word1);
    esi = esi.wrapping_add(word2);
    let mut edx = word0.wrapping_sub(esi);

    edx = edx.wrapping_add(ebx) ^ shift_xor(esi, 28, 4);
    esi = esi.wrapping_add(edi);
    edi = edi.wrapping_sub(edx) ^ shift_xor(edx, 26, 6);
    edx = edx.wrapping_add(esi);
    esi = esi.wrapping_sub(edi) ^ shift_xor(edi, 24, 8);
    edi = edi.wrapping_add(edx);
    ebx = edx.wrapping_sub(esi) ^ shift_xor(esi, 16, 16);
    esi = esi.wrapping_add(edi);
    edi = edi.wrapping_sub(ebx) ^ shift_xor(ebx, 13, 19);
    ebx = ebx.wrapping_add(esi);
    esi = esi.wrapping_sub(edi) ^ shift_xor(edi, 28, 4);
    edi = edi.wrapping_add(ebx);
    (ebx, esi, edi)
}

#[inline(always)]
fn final_mix<T>(ebx: T, mut esi: T, mut edi: T) -> (T, T)
where
    T: HashWord,
{
    esi = (esi ^ edi).wrapping_sub(shift_xor(edi, 18, 14));
    let ecx = (esi ^ ebx).wrapping_sub(shift_xor(esi, 21, 11));
    edi = (edi ^ ecx).wrapping_sub(shift_xor(ecx, 7, 25));
    esi = (esi ^ edi).wrapping_sub(shift_xor(edi, 16, 16));
    let edx = (esi ^ ecx).wrapping_sub(shift_xor(esi, 28, 4));
    edi = (edi ^ edx).wrapping_sub(shift_xor(edx, 18, 14));
    let eax = (esi ^ edi).wrapping_sub(shift_xor(edi, 8, 24));
    (eax, edi)
}

#[inline(always)]
fn load_word0(bytes: &[u8], offset: usize) -> u32 {
    ((bytes[offset + 3] as u32) << 24)
        | ((bytes[offset + 2] as u32) << 16)
        | ((bytes[offset + 1] as u32) << 8)
        | bytes[offset] as u32
}

#[inline(always)]
fn load_word1(bytes: &[u8], offset: usize) -> u32 {
    ((bytes[offset + 7] as u32) << 24)
        | ((bytes[offset + 6] as u32) << 16)
        | ((bytes[offset + 5] as u32) << 8)
        | bytes[offset + 4] as u32
}

#[inline(always)]
fn load_word2(bytes: &[u8], offset: usize) -> u32 {
    ((bytes[offset + 11] as u32) << 24)
        | ((bytes[offset + 10] as u32) << 16)
        | ((bytes[offset + 9] as u32) << 8)
        | bytes[offset + 8] as u32
}

#[inline(always)]
fn tail_additions(bytes: &[u8], offset: usize) -> (u32, u32, u32) {
    let remaining = bytes.len().saturating_sub(offset);
    let mut ebx_add = 0u32;
    let mut edi_add = 0u32;
    let mut esi_add = 0u32;

    if remaining >= 1 {
        ebx_add = ebx_add.wrapping_add(bytes[offset] as u32);
    }
    if remaining >= 2 {
        ebx_add = ebx_add.wrapping_add((bytes[offset + 1] as u32) << 8);
    }
    if remaining >= 3 {
        ebx_add = ebx_add.wrapping_add((bytes[offset + 2] as u32) << 16);
    }
    if remaining >= 4 {
        ebx_add = ebx_add.wrapping_add((bytes[offset + 3] as u32) << 24);
    }
    if remaining >= 5 {
        edi_add = edi_add.wrapping_add(bytes[offset + 4] as u32);
    }
    if remaining >= 6 {
        edi_add = edi_add.wrapping_add((bytes[offset + 5] as u32) << 8);
    }
    if remaining >= 7 {
        edi_add = edi_add.wrapping_add((bytes[offset + 6] as u32) << 16);
    }
    if remaining >= 8 {
        edi_add = edi_add.wrapping_add((bytes[offset + 7] as u32) << 24);
    }
    if remaining >= 9 {
        esi_add = esi_add.wrapping_add(bytes[offset + 8] as u32);
    }
    if remaining >= 10 {
        esi_add = esi_add.wrapping_add((bytes[offset + 9] as u32) << 8);
    }
    if remaining >= 11 {
        esi_add = esi_add.wrapping_add((bytes[offset + 10] as u32) << 16);
    }
    if remaining >= 12 {
        esi_add = esi_add.wrapping_add((bytes[offset + 11] as u32) << 24);
    }

    (ebx_add, edi_add, esi_add)
}

#[inline(always)]
fn finalize_scalar(mut ebx: u32, mut esi: u32, mut edi: u32, bytes: &[u8], offset: usize) -> u64 {
    if offset < bytes.len() {
        let (ebx_add, edi_add, esi_add) = tail_additions(bytes, offset);
        ebx = ebx.wrapping_add(ebx_add);
        edi = edi.wrapping_add(edi_add);
        esi = esi.wrapping_add(esi_add);
        let (eax, edi) = final_mix(ebx, esi, edi);
        ((edi as u64) << 32) | eax as u64
    } else {
        (esi as u64) << 32
    }
}

#[inline(always)]
fn hash_file_name_bytes(bytes: &[u8]) -> u64 {
    let mut ebx = (bytes.len() as u32).wrapping_add(0xDEADBEEF);
    let mut esi = ebx;
    let mut edi = ebx;

    let mut offset = 0usize;
    while offset + 12 < bytes.len() {
        let word0 = load_word0(bytes, offset);
        let word1 = load_word1(bytes, offset);
        let word2 = load_word2(bytes, offset);
        (ebx, esi, edi) = mix_block(ebx, esi, edi, word0, word1, word2);
        offset += 12;
    }

    finalize_scalar(ebx, esi, edi, bytes, offset)
}

pub fn hash_file_name_single(s: &str) -> u64 {
    hash_file_name_bytes(s.as_bytes())
}

macro_rules! impl_lane_kernels {
    ($same_name:ident, $mixed_name:ident, $vec_ty:ty, $lanes:expr) => {
        fn $same_name(batch: [&[u8]; $lanes], full_chunks: usize, tail: usize) -> [u64; $lanes] {
            let base: [u32; $lanes] = std::array::from_fn(|lane| {
                (batch[lane].len() as u32).wrapping_add(0xDEADBEEF)
            });
            let mut ebx: $vec_ty = cast(base);
            let mut esi = ebx;
            let mut edi = ebx;

            for chunk in 0..full_chunks {
                let offset = chunk * 12;
                let word0: $vec_ty = cast(std::array::from_fn::<_, $lanes, _>(|lane| load_word0(batch[lane], offset)));
                let word1: $vec_ty = cast(std::array::from_fn::<_, $lanes, _>(|lane| load_word1(batch[lane], offset)));
                let word2: $vec_ty = cast(std::array::from_fn::<_, $lanes, _>(|lane| load_word2(batch[lane], offset)));
                (ebx, esi, edi) = mix_block(ebx, esi, edi, word0, word1, word2);
            }

            if tail > 0 {
                let offset = full_chunks * 12;
                let ebx_add: $vec_ty = cast(std::array::from_fn::<_, $lanes, _>(|lane| tail_additions(batch[lane], offset).0));
                let edi_add: $vec_ty = cast(std::array::from_fn::<_, $lanes, _>(|lane| tail_additions(batch[lane], offset).1));
                let esi_add: $vec_ty = cast(std::array::from_fn::<_, $lanes, _>(|lane| tail_additions(batch[lane], offset).2));
                let (eax, edi) = final_mix(
                    ebx.wrapping_add(ebx_add),
                    esi.wrapping_add(esi_add),
                    edi.wrapping_add(edi_add),
                );
                let eax_arr = eax.as_array();
                let edi_arr = edi.as_array();
                std::array::from_fn(|lane| ((edi_arr[lane] as u64) << 32) | eax_arr[lane] as u64)
            } else {
                let esi_arr = esi.as_array();
                std::array::from_fn(|lane| (esi_arr[lane] as u64) << 32)
            }
        }

        fn $mixed_name(batch: [&[u8]; $lanes], full_chunks: usize) -> [u64; $lanes] {
            let base: [u32; $lanes] = std::array::from_fn(|lane| {
                (batch[lane].len() as u32).wrapping_add(0xDEADBEEF)
            });
            let mut ebx: $vec_ty = cast(base);
            let mut esi = ebx;
            let mut edi = ebx;

            for chunk in 0..full_chunks {
                let offset = chunk * 12;
                let word0: $vec_ty = cast(std::array::from_fn::<_, $lanes, _>(|lane| load_word0(batch[lane], offset)));
                let word1: $vec_ty = cast(std::array::from_fn::<_, $lanes, _>(|lane| load_word1(batch[lane], offset)));
                let word2: $vec_ty = cast(std::array::from_fn::<_, $lanes, _>(|lane| load_word2(batch[lane], offset)));
                (ebx, esi, edi) = mix_block(ebx, esi, edi, word0, word1, word2);
            }

            let ebx_arr = *ebx.as_array();
            let esi_arr = *esi.as_array();
            let edi_arr = *edi.as_array();
            let offset = full_chunks * 12;
            std::array::from_fn(|lane| finalize_scalar(ebx_arr[lane], esi_arr[lane], edi_arr[lane], batch[lane], offset))
        }
    };
}

impl_lane_kernels!(hash_same_width_x4, hash_mixed_tail_x4, u32x4, 4);
impl_lane_kernels!(hash_same_width_x8, hash_mixed_tail_x8, u32x8, 8);

fn take_suffix<const LANES: usize>(indices: &mut Vec<usize>) -> Option<[usize; LANES]> {
    if indices.len() < LANES {
        return None;
    }
    let start = indices.len() - LANES;
    let chunk: [usize; LANES] = indices[start..].try_into().unwrap();
    indices.truncate(start);
    Some(chunk)
}

fn process_same_width<const LANES: usize>(
    indices: &mut Vec<usize>,
    bytes: &[&[u8]],
    out: &mut [u64],
    full_chunks: usize,
    tail: usize,
    kernel: fn([&[u8]; LANES], usize, usize) -> [u64; LANES],
) {
    while let Some(chunk) = take_suffix::<LANES>(indices) {
        let batch = std::array::from_fn(|lane| bytes[chunk[lane]]);
        let hashes = kernel(batch, full_chunks, tail);
        for lane in 0..LANES {
            out[chunk[lane]] = hashes[lane];
        }
    }
}

fn process_mixed_tail<const LANES: usize>(
    indices: &mut Vec<usize>,
    bytes: &[&[u8]],
    out: &mut [u64],
    full_chunks: usize,
    kernel: fn([&[u8]; LANES], usize) -> [u64; LANES],
) {
    while let Some(chunk) = take_suffix::<LANES>(indices) {
        let batch = std::array::from_fn(|lane| bytes[chunk[lane]]);
        let hashes = kernel(batch, full_chunks);
        for lane in 0..LANES {
            out[chunk[lane]] = hashes[lane];
        }
    }
}

#[cfg(target_arch = "aarch64")]
const PREFERRED_LANES: &[usize] = &[4, 8];

#[cfg(not(target_arch = "aarch64"))]
const PREFERRED_LANES: &[usize] = &[8, 4];

pub fn hash_file_name_simd_batch_bytes_into(inputs: &[&[u8]], out: &mut [u64]) {
    assert_eq!(
        inputs.len(),
        out.len(),
        "UOP batch hash input and output lengths must match"
    );

    let mut chunk_buckets: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (index, bytes) in inputs.iter().enumerate() {
        let full_chunks = bytes.len().saturating_sub(1) / 12;
        chunk_buckets.entry(full_chunks).or_default().push(index);
    }

    for (full_chunks, chunk_group) in chunk_buckets {
        let mut tail_buckets = vec![Vec::new(); 13];
        for index in chunk_group {
            let tail = inputs[index].len() - full_chunks * 12;
            tail_buckets[tail].push(index);
        }

        for &lanes in PREFERRED_LANES {
            for (tail, indices) in tail_buckets.iter_mut().enumerate() {
                match lanes {
                    8 => process_same_width::<8>(
                        indices,
                        inputs,
                        out,
                        full_chunks,
                        tail,
                        hash_same_width_x8,
                    ),
                    4 => process_same_width::<4>(
                        indices,
                        inputs,
                        out,
                        full_chunks,
                        tail,
                        hash_same_width_x4,
                    ),
                    _ => unreachable!(),
                }
            }
        }

        let mut leftovers = Vec::new();
        for indices in &mut tail_buckets {
            leftovers.append(indices);
        }

        for &lanes in PREFERRED_LANES {
            match lanes {
                8 => process_mixed_tail::<8>(
                    &mut leftovers,
                    inputs,
                    out,
                    full_chunks,
                    hash_mixed_tail_x8,
                ),
                4 => process_mixed_tail::<4>(
                    &mut leftovers,
                    inputs,
                    out,
                    full_chunks,
                    hash_mixed_tail_x4,
                ),
                _ => unreachable!(),
            }
        }

        for index in leftovers {
            out[index] = hash_file_name_bytes(inputs[index]);
        }
    }
}

pub fn hash_file_name_simd_batch_strs_into(inputs: &[&str], out: &mut [u64]) {
    let bytes: Vec<&[u8]> = inputs.iter().map(|input| input.as_bytes()).collect();
    hash_file_name_simd_batch_bytes_into(&bytes, out);
}

pub fn hash_file_name_simd_batch_strs(inputs: &[&str]) -> Vec<u64> {
    let mut out = vec![0u64; inputs.len()];
    hash_file_name_simd_batch_strs_into(inputs, &mut out);

    out
}
