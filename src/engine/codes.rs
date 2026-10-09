//! Named ±1 patterns: a stable seed from a name and the random pattern belonging to a name. The `valleys`
//! survey and the `coded` codecs use them with no memory in sight, so SETTLE owns them; the same two functions
//! live in KANERVA (`kanerva::codes`), and `tests::settle_and_kanerva_give_the_same_codes` holds the copies equal.

use crate::engine::rng::standalone::Rng;

/// A stable 64-bit seed from a name (FNV-1a).
///
/// ```
/// use settle::engine::codes::seed_of;
/// assert_eq!(seed_of(""), 0xcbf2_9ce4_8422_2325); // the FNV-1a offset basis
/// assert_eq!(seed_of("a"), 0xaf63_dc4c_8601_ec8c); // the FNV-1a reference value for "a"
/// ```
pub fn seed_of(name: &str) -> u64 {
    name.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100_0000_01b3))
}

/// The random ±1 pattern belonging to a name: `size` fair coin flips from a generator seeded by `seed_of(name)`.
///
/// ```
/// use settle::engine::codes::code;
/// let cat = code("cat", 64);
/// assert_eq!(cat, code("cat", 64));
/// assert!(cat.iter().all(|&v| v == 1.0 || v == -1.0));
/// assert_ne!(cat, code("owl", 64));
/// ```
pub fn code(name: &str, size: usize) -> Vec<f64> {
    let mut r = Rng::new(seed_of(name));
    (0..size).map(|_| if r.unit() < 0.5 { -1.0 } else { 1.0 }).collect()
}

#[cfg(all(test, feature = "sdm"))]
mod tests {
    #[test]
    fn settle_and_kanerva_give_the_same_codes() {
        for name in ["", "a", "cat", "coded:note", "ldpc:512:256:1", "a longer name with spaces"] {
            assert_eq!(super::seed_of(name), kanerva::codes::seed_of(name));
            for size in [0, 1, 7, 64, 513] {
                assert_eq!(super::code(name, size), kanerva::codes::code(name, size));
            }
        }
    }
}
