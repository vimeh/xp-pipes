//! The executable's MSVC RNG, not a general-purpose or security RNG.
//! Evidence: `_rand` 01010786, `iRand` 0100e18e, `iRand2` 0100e1b4, `RandInit` 0100e224.
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug)]
pub struct Random {
    state: u32,
}

impl Random {
    pub fn new(seed: Option<u32>) -> Self {
        let mut random = Self {
            state: seed.unwrap_or_else(|| {
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .subsec_millis()
            }),
        };
        for _ in 0..10 {
            random.next();
        }
        random
    }

    #[cfg(test)]
    pub fn checkpoint(&self) -> u32 {
        self.state
    }

    fn next(&mut self) -> u32 {
        self.state = self.state.wrapping_mul(214_013).wrapping_add(2_531_011);
        (self.state >> 16) & 0x7fff
    }

    /// iRand: multiply by 1/32768 and truncate. Even n=1 consumes a draw.
    pub fn below(&mut self, n: u32) -> u32 {
        assert!(n > 0);
        u32::try_from((u64::from(self.next()) * u64::from(n)) >> 15)
            .expect("scaled random value is less than its u32 bound")
    }

    /// iRand2: inclusive, reversed bounds accepted, equal bounds consume nothing.
    pub fn between(&mut self, a: u32, b: u32) -> u32 {
        if a == b {
            return a;
        }
        let (low, high) = (a.min(b), a.max(b));
        let width = high.checked_sub(low).unwrap().checked_add(1).unwrap();
        low + self.below(width)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_state_and_output_after_startup_warmup() {
        for group in crate::reference::rng_rows().as_chunks::<12>().0 {
            let mut random = Random::new(Some(group[0].0));
            for &(_, _, state, value) in group {
                assert_eq!(random.next(), value);
                assert_eq!(random.state, state);
            }
        }
    }

    #[test]
    fn sampling_scales_instead_of_modulo_and_preserves_draw_counts() {
        let mut random = Random::new(Some(42));
        let mut raw = random.clone();
        for bound in [1, 3, 6, 16, 100, 100_000] {
            let value = u32::try_from(u64::from(raw.next()) * u64::from(bound) / 32768).unwrap();
            assert_eq!(random.below(bound), value);
            assert_eq!(random.state, raw.state);
        }
        let state = random.state;
        assert_eq!(random.between(17, 17), 17);
        assert_eq!(random.state, state);
        for _ in 0..1000 {
            assert_eq!(random.between(100, 25), raw.between(25, 100));
        }
        // Every 15-bit output is reachable; the inclusive sampler reaches both ends.
        let mut ends = [false; 2];
        for _ in 0..1000 {
            ends[usize::try_from(random.between(5, 6) - 5).unwrap()] = true;
        }
        assert_eq!(ends, [true; 2]);
    }
}
