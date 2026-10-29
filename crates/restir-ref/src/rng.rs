//! PCG-XSH-RR (64-bit state, 32-bit output) with a 16-bit unit float.
//!
//! `Rng::new(seed)` sets the stream increment to `(seed << 1) | 1`, adds
//! `seed` after one multiplier step, then discards one output. `next_f32`
//! takes the low 16 bits of `next_u32 >> 8` and divides by 65536, so the
//! value is in `[0, 1)`. `uniform_usize(n)` rejects draws at or above
//! `u32::MAX - (u32::MAX % n)` so each index has probability `1/n`.

/// Deterministic stream. The same seed yields the same draws.
#[derive(Clone, Debug)]
pub struct Rng {
    state: u64,
    increment: u64,
}

impl Rng {
    /// Stream for `seed`. See the module note for the warm-up.
    #[must_use]
    pub fn new(seed: u64) -> Self {
        let increment = seed.wrapping_shl(1) | 1;
        let mut rng = Self {
            state: 0,
            increment,
        };
        rng.state = rng.step().wrapping_add(seed);
        let _ = rng.next_u32();
        rng
    }

    fn step(&mut self) -> u64 {
        let old = self.state;
        self.state = old
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(self.increment);
        old
    }

    /// Next 32 bits of the PCG-XSH-RR output.
    pub fn next_u32(&mut self) -> u32 {
        let old = self.step();
        let mixed = ((old >> 18) ^ old) >> 27;
        let bits = u32::try_from(mixed & 0xffff_ffff).unwrap_or(0);
        let rotate = u32::try_from(old >> 59).unwrap_or(0);
        bits.rotate_right(rotate)
    }

    /// Uniform in `[0, 1)`. Sixteen bits of fraction.
    pub fn next_f32(&mut self) -> f32 {
        let bits = self.next_u32() >> 8;
        let low = u16::try_from(bits & 0xffff).unwrap_or(0);
        f32::from(low) * (1.0 / 65536.0)
    }

    /// Uniform index in `0..n`. `n <= 1` returns 0 and draws nothing.
    pub fn uniform_usize(&mut self, n: usize) -> usize {
        if n <= 1 {
            return 0;
        }
        let width = u32::try_from(n).unwrap_or(u32::MAX);
        let limit = u32::MAX - (u32::MAX % width);
        loop {
            let draw = self.next_u32();
            if draw < limit {
                return usize::try_from(draw % width).unwrap_or(0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Rng;

    #[test]
    fn the_same_seed_repeats() {
        let mut a = Rng::new(7);
        let mut b = Rng::new(7);
        for _ in 0..8 {
            assert_eq!(a.next_u32(), b.next_u32());
        }
    }

    #[test]
    fn a_different_seed_diverges() {
        let mut a = Rng::new(1);
        let mut b = Rng::new(2);
        assert_ne!(a.next_u32(), b.next_u32());
    }

    #[test]
    fn unit_float_stays_below_one() {
        let mut rng = Rng::new(3);
        for _ in 0..64 {
            let value = rng.next_f32();
            assert!((0.0..1.0).contains(&value), "{value}");
        }
    }
}
