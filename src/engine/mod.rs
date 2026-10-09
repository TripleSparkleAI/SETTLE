//! ENGINE, the first floor: data in, data out. Things with leans, pulls between them, the sampler (the p-bit
//! rule, exact Gibbs sampling), the anneal schedule, energy, and answers by counting. Nothing here parses a
//! program, prints a line or knows a keyword: the words floor (`crate::words`) does that, over this engine.
//!
//! - `model`: `Model` (things, leans, sparse pulls, energy) and `State` (one run: held things, temperature,
//!   the sampler `settle`, the schedule `anneal`, the kept samples and yes-counts)
//! - `answers`: questions joined by and / or / and_not / or_not, answered by counting kept samples
//! - `rng`: the xorshift64* generator (SETTLE's own; KANERVA's when the `sdm` feature is on, the same stream)
//! - `codes`: named ±1 patterns (a stable seed from a name, the pattern belonging to a name)
//!
//! The statement families in `src/<family>.rs` still hold their own engines beside their words; the ledger of
//! which are split is in `settle-rs/DISCOVERIES.md` (THE FLOORS).

pub mod answers;
pub mod codes;
pub mod model;
pub mod rng;
