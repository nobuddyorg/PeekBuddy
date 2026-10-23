//! In-tree PRNG: `SplitMix64` seeding `xoshiro256**`. Kept here so operation
//! streams never change when a dependency updates.

/// One `SplitMix64` step; used to expand a seed into `xoshiro256**` state.
fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// `xoshiro256**` (Blackman and Vigna).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rng {
    s: [u64; 4],
}

impl Rng {
    /// A generator for `seed` and `stream` (one stream per thread).
    #[must_use]
    pub fn new(seed: u64, stream: u64) -> Self {
        let mut state = seed ^ stream.wrapping_mul(0xD1B5_4A32_D192_ED03);
        Self {
            s: [
                splitmix64(&mut state),
                splitmix64(&mut state),
                splitmix64(&mut state),
                splitmix64(&mut state),
            ],
        }
    }

    /// The next 64 random bits.
    pub fn next_u64(&mut self) -> u64 {
        let [s0, s1, s2, s3] = self.s;
        let result = s1.wrapping_mul(5).rotate_left(7).wrapping_mul(9);
        let t = s1 << 17;
        let s2 = s2 ^ s0;
        let s3 = s3 ^ s1;
        let s1 = s1 ^ s2;
        let s0 = s0 ^ s3;
        let s2 = s2 ^ t;
        let s3 = s3.rotate_left(45);
        self.s = [s0, s1, s2, s3];
        result
    }

    /// A value in `0..bound` (`0` when `bound` is 0), by Lemire's
    /// multiply-shift; the tiny bias is irrelevant for workload shaping.
    pub fn below(&mut self, bound: u64) -> u64 {
        let wide = u128::from(self.next_u64()) * u128::from(bound);
        u64::try_from(wide >> 64).unwrap_or(0)
    }

    /// An index into a collection of `len` items (`0` when `len` is 0).
    pub fn index(&mut self, len: usize) -> usize {
        let bound = u64::try_from(len).unwrap_or(u64::MAX);
        usize::try_from(self.below(bound)).unwrap_or(0)
    }
}

#[cfg(test)]
#[expect(
    clippy::indexing_slicing,
    reason = "tests index fixed-size arrays; a panic is a test failure"
)]
mod tests {
    use super::{Rng, splitmix64};

    #[test]
    fn splitmix64_matches_the_reference() {
        // Reference values for seed 1234567 (Vigna's `splitmix64.c`).
        let mut state = 1_234_567;
        assert_eq!(splitmix64(&mut state), 6_457_827_717_110_365_317);
        assert_eq!(splitmix64(&mut state), 3_203_168_211_198_807_973);
    }

    #[test]
    fn xoshiro_matches_the_reference() {
        // xoshiro256** from state [1, 2, 3, 4] (xoshiro256starstar.c).
        let mut rng = Rng { s: [1, 2, 3, 4] };
        let first: Vec<u64> = (0..4).map(|_| rng.next_u64()).collect();
        assert_eq!(first, [11_520, 0, 1_509_978_240, 1_215_971_899_390_074_240]);
    }

    #[test]
    fn same_seed_same_stream() {
        let a: Vec<u64> = {
            let mut rng = Rng::new(7, 0);
            (0..16).map(|_| rng.next_u64()).collect()
        };
        let b: Vec<u64> = {
            let mut rng = Rng::new(7, 0);
            (0..16).map(|_| rng.next_u64()).collect()
        };
        assert_eq!(a, b);
        let mut other = Rng::new(7, 1);
        assert_ne!(other.next_u64(), a[0]);
        let mut seeded = Rng::new(8, 0);
        assert_ne!(seeded.next_u64(), a[0]);
    }

    #[test]
    fn below_stays_in_range() {
        let mut rng = Rng::new(1, 0);
        for bound in [1u64, 2, 3, 10, 1000] {
            for _ in 0..1000 {
                assert!(rng.below(bound) < bound);
            }
        }
        assert_eq!(rng.below(0), 0);
        assert_eq!(rng.index(0), 0);
        let mut seen = [false; 3];
        for _ in 0..100 {
            seen[rng.index(3)] = true;
        }
        assert_eq!(seen, [true; 3]);
    }
}
