//! Random numbers: the client's MT19937 and its `rand01`.
//!
//! UOSA.exe has one global MT19937 (`MersenneTwister::NextU32` 0x0099aa85, `Twist` 0x0099ac5a, `Seed`
//! 0x0099ac2a, `SeedByArray` 0x0099ab3e), seeded from the clock at startup and shared by every game system, so
//! the client's sequence is not reproducible. A simulator may use any seed; [`Mt19937`] gives the same stream
//! for the same seed.

/// A source of uniformly distributed 32-bit words.
pub trait Rng {
    fn next_u32(&mut self) -> u32;

    /// The client's `rand01` (Chaos samplers): `(float)((double)u * 2^-32)`. The rounding to f32 returns exactly
    /// 1.0 when `u >= 2^32 - 128`, as in the client.
    fn rand01(&mut self) -> f32 {
        (f64::from(self.next_u32()) * RAND01_SCALE) as f32
    }
}

/// 2^-32 (UOSA.exe `DAT_00d05190`).
const RAND01_SCALE: f64 = 1.0 / 4_294_967_296.0;

/// MT19937 state size in words.
const MT_N: usize = 624;
/// MT19937 shift offset.
const MT_M: usize = 397;
const MT_MATRIX_A: u32 = 0x9908_b0df;
const MT_UPPER_MASK: u32 = 0x8000_0000;
const MT_LOWER_MASK: u32 = 0x7fff_ffff;
/// `init_genrand` multiplier (the client's `Seed` uses 0x6c078965).
const MT_INIT_MULTIPLIER: u32 = 1_812_433_253;
/// `init_by_array` first seed.
const MT_ARRAY_INIT_SEED: u32 = 19_650_218;
/// `init_by_array` multipliers (the client's `SeedByArray` uses 0x19660d and 0x5d588b65).
const MT_ARRAY_MULTIPLIER_1: u32 = 1_664_525;
const MT_ARRAY_MULTIPLIER_2: u32 = 1_566_083_941;
/// The reference implementation's default seed.
pub const MT_DEFAULT_SEED: u32 = 5489;
const MT_TEMPER_SHIFT_U: u32 = 11;
const MT_TEMPER_SHIFT_S: u32 = 7;
const MT_TEMPER_MASK_B: u32 = 0x9d2c_5680;
const MT_TEMPER_SHIFT_T: u32 = 15;
const MT_TEMPER_MASK_C: u32 = 0xefc6_0000;
const MT_TEMPER_SHIFT_L: u32 = 18;
/// Right shift in the seeding recurrences.
const MT_SEED_SHIFT: u32 = 30;

/// The Mersenne Twister MT19937 (Matsumoto and Nishimura's `mt19937ar.c`), the generator of the client.
#[derive(Clone)]
pub struct Mt19937 {
    state: Box<[u32; MT_N]>,
    index: usize,
}

impl std::fmt::Debug for Mt19937 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Mt19937").field("index", &self.index).finish_non_exhaustive()
    }
}

impl Default for Mt19937 {
    fn default() -> Self {
        Self::new(MT_DEFAULT_SEED)
    }
}

impl Mt19937 {
    /// `init_genrand(seed)`.
    pub fn new(seed: u32) -> Self {
        let mut state = Box::new([0u32; MT_N]);
        state[0] = seed;
        for i in 1..MT_N {
            let previous = state[i - 1];
            state[i] = MT_INIT_MULTIPLIER.wrapping_mul(previous ^ (previous >> MT_SEED_SHIFT)).wrapping_add(i as u32);
        }
        Self { state, index: MT_N }
    }

    /// `init_by_array(key)`.
    pub fn from_key(key: &[u32]) -> Self {
        let mut generator = Self::new(MT_ARRAY_INIT_SEED);
        let state = &mut generator.state;
        let mut i = 1;
        let mut j = 0;
        for _ in 0..MT_N.max(key.len()) {
            let previous = state[i - 1];
            state[i] = (state[i] ^ (previous ^ (previous >> MT_SEED_SHIFT)).wrapping_mul(MT_ARRAY_MULTIPLIER_1))
                .wrapping_add(key.get(j).copied().unwrap_or(0))
                .wrapping_add(j as u32);
            i += 1;
            j += 1;
            if i >= MT_N {
                state[0] = state[MT_N - 1];
                i = 1;
            }
            if j >= key.len() {
                j = 0;
            }
        }
        for _ in 0..MT_N - 1 {
            let previous = state[i - 1];
            state[i] = (state[i] ^ (previous ^ (previous >> MT_SEED_SHIFT)).wrapping_mul(MT_ARRAY_MULTIPLIER_2))
                .wrapping_sub(i as u32);
            i += 1;
            if i >= MT_N {
                state[0] = state[MT_N - 1];
                i = 1;
            }
        }
        state[0] = MT_UPPER_MASK;
        generator
    }

    fn twist(&mut self) {
        let state = &mut self.state;
        for i in 0..MT_N {
            let y = (state[i] & MT_UPPER_MASK) | (state[(i + 1) % MT_N] & MT_LOWER_MASK);
            let magic = if y & 1 == 0 { 0 } else { MT_MATRIX_A };
            state[i] = state[(i + MT_M) % MT_N] ^ (y >> 1) ^ magic;
        }
        self.index = 0;
    }
}

impl Rng for Mt19937 {
    fn next_u32(&mut self) -> u32 {
        if self.index >= MT_N {
            self.twist();
        }
        let mut y = self.state[self.index];
        self.index += 1;
        y ^= y >> MT_TEMPER_SHIFT_U;
        y ^= (y << MT_TEMPER_SHIFT_S) & MT_TEMPER_MASK_B;
        y ^= (y << MT_TEMPER_SHIFT_T) & MT_TEMPER_MASK_C;
        y ^= y >> MT_TEMPER_SHIFT_L;
        y
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `init_by_array({0x123, 0x234, 0x345, 0x456})`: the first outputs of `mt19937ar.out`.
    const REFERENCE_KEY: [u32; 4] = [0x123, 0x234, 0x345, 0x456];
    const REFERENCE_KEY_OUTPUTS: [u32; 5] = [1_067_595_299, 955_945_823, 477_289_528, 4_107_218_783, 4_228_976_476];
    /// `init_genrand(5489)`: the first output, and the 10000th (the C++ `std::mt19937` conformance value).
    const DEFAULT_SEED_FIRST: u32 = 3_499_211_612;
    const DEFAULT_SEED_10000TH: u32 = 4_123_659_995;
    const CONFORMANCE_INDEX: usize = 10_000;

    #[test]
    fn matches_the_reference_key_stream() {
        let mut generator = Mt19937::from_key(&REFERENCE_KEY);
        let outputs: Vec<u32> = (0..REFERENCE_KEY_OUTPUTS.len()).map(|_| generator.next_u32()).collect();
        assert_eq!(outputs, REFERENCE_KEY_OUTPUTS);
    }

    #[test]
    fn matches_the_default_seed_stream() {
        let mut generator = Mt19937::default();
        assert_eq!(generator.next_u32(), DEFAULT_SEED_FIRST);
        let mut generator = Mt19937::new(MT_DEFAULT_SEED);
        let last = (0..CONFORMANCE_INDEX).map(|_| generator.next_u32()).last();
        assert_eq!(last, Some(DEFAULT_SEED_10000TH));
    }

    struct Fixed(u32);

    impl Rng for Fixed {
        fn next_u32(&mut self) -> u32 {
            self.0
        }
    }

    #[test]
    fn rand01_rounds_like_the_client() {
        /// Below this word, rand01 is less than 1.0.
        const LAST_BELOW_ONE: u32 = u32::MAX - 128;
        assert_eq!(Fixed(0).rand01(), 0.0);
        assert_eq!(Fixed(u32::MAX).rand01(), 1.0);
        assert_eq!(Fixed(LAST_BELOW_ONE + 1).rand01(), 1.0);
        assert!(Fixed(LAST_BELOW_ONE).rand01() < 1.0);
    }
}
