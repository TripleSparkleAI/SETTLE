//! xorshift64* random numbers: uniform in [0, 1) and in [-1, 1), a bounded integer and a normal draw.
//!
//! SETTLE owns its generator (`standalone::Rng`), so the language builds and runs with no KANERVA at all. With the
//! `sdm` feature on (the default), `Rng` is KANERVA's (`kanerva::rng::Rng`) instead: one type, so a run's
//! `State::rng` can be handed straight to a KANERVA read or write. The two are the same algorithm, the same
//! constants and the same stream; `tests::settle_and_kanerva_draw_the_same_stream` holds them equal draw for draw.

/// SETTLE's own copy of the generator: the stream every seed has drawn since SETTLE began.
pub mod standalone {
    /// The xorshift64* generator. A clone continues the same stream from the same point.
    #[derive(Clone, Debug)]
    pub struct Rng(u64);

    impl Rng {
        /// A generator from a seed; the seed is mixed by a multiply and made odd, so seed 0 is valid.
        pub fn new(seed: u64) -> Self {
            Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
        }
        /// The next raw 64-bit output.
        pub fn next_u64(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x >> 12;
            x ^= x << 25;
            x ^= x >> 27;
            self.0 = x;
            x.wrapping_mul(0x2545_F491_4F6C_DD1D)
        }
        /// A uniform draw in [0, 1), from the top 53 bits.
        pub fn unit(&mut self) -> f64 {
            (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
        }
        /// A uniform draw in [-1, 1).
        pub fn signed(&mut self) -> f64 {
            2.0 * self.unit() - 1.0
        }
        /// An integer in 0..n by modulo (`n` 0 is treated as 1, so it returns 0).
        pub fn below(&mut self, n: usize) -> usize {
            (self.next_u64() % n.max(1) as u64) as usize
        }
        /// A standard normal draw (Box-Muller).
        pub fn normal(&mut self) -> f64 {
            let u1 = self.unit().max(1e-300);
            let u2 = self.unit();
            (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
        }
    }
}

#[cfg(feature = "sdm")]
pub use kanerva::rng::Rng;
#[cfg(not(feature = "sdm"))]
pub use standalone::Rng;

#[cfg(test)]
mod tests {
    #[test]
    fn the_stream_is_the_one_settle_has_always_drawn() {
        // pinned outputs: every recorded example output depends on this exact stream
        let mut r = super::standalone::Rng::new(1);
        assert_eq!(r.next_u64(), 0x0d83_b3e2_9a21_487a);
        assert_eq!(r.next_u64(), 0x54c4_4c79_f1fe_9d67);
        let mut q = super::standalone::Rng::new(0x5eed);
        assert_eq!(q.unit(), 0.6442583146599637);
        assert_eq!(q.below(1000), 546);
        assert_eq!(q.normal(), -0.43737970267191895);
    }

    #[cfg(feature = "sdm")]
    #[test]
    fn settle_and_kanerva_draw_the_same_stream() {
        for seed in [0u64, 1, 5, 0x5eed, u64::MAX] {
            let (mut k, mut s) = (kanerva::rng::Rng::new(seed), super::standalone::Rng::new(seed));
            for i in 0..2000 {
                match i % 5 {
                    0 => assert_eq!(k.next_u64(), s.next_u64()),
                    1 => assert_eq!(k.unit().to_bits(), s.unit().to_bits()),
                    2 => assert_eq!(k.signed().to_bits(), s.signed().to_bits()),
                    3 => assert_eq!(k.below(i + 3), s.below(i + 3)),
                    _ => assert_eq!(k.normal().to_bits(), s.normal().to_bits()),
                }
            }
        }
    }
}
