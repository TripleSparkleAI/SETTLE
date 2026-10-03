//! xorshift64* random numbers: uniform in [0, 1) and in [-1, 1), a bounded integer and a normal draw.
//! The generator lives in KANERVA (`kanerva::rng`) so the interpreter and the SDM toolbox draw from one
//! stream type; a run's `State::rng` can be handed straight to a KANERVA read or write.

pub use kanerva::rng::Rng;
