//! THE WORD LIST of the core family: every word a program writes and every name the Rust builder uses, once.
//! The file face (`words::core`) parses with these constants and the builder face (`words::builder`) names its
//! methods after them; `tests/two_faces.rs` holds the two faces to this one list in both directions.

pub const THING: &str = "thing";
pub const LEANS: &str = "leans";
pub const BY: &str = "by";
pub const PULLS: &str = "pulls";
pub const PUSHES: &str = "pushes";
pub const HOLD: &str = "hold";
pub const SETTLE: &str = "settle";
pub const ANNEAL: &str = "anneal";
pub const TEMPERATURE: &str = "temperature";
pub const SEED: &str = "seed";
pub const UPDATE: &str = "update";
pub const GIBBS: &str = "gibbs";
pub const METRO: &str = "metro";
pub const SHOW: &str = "show";
pub const BEST: &str = "best";
pub const ASK: &str = "ask";
pub const AND: &str = "and";
pub const OR: &str = "or";
pub const AND_NOT: &str = "and_not";
pub const OR_NOT: &str = "or_not";
pub const YES: &str = "yes";
pub const NO: &str = "no";

/// Every word above, in the order a program meets them.
pub const CORE: &[&str] =
    &[THING, LEANS, BY, PULLS, PUSHES, HOLD, SETTLE, ANNEAL, TEMPERATURE, SEED, UPDATE, GIBBS, METRO, SHOW, BEST, ASK, AND, OR, AND_NOT, OR_NOT, YES, NO];

/// The statements that open an sdm-family memory in a model block, and the sdm-family statements that stand alone
/// in a run block. They need the `sdm` feature (KANERVA); without it `sdmoff` refuses them by these names. All but
/// `memory` are KANERVA's plug; `memory` (a Hopfield memory) is SETTLE's own but uses KANERVA's keys.
pub const SDM_MODEL_HEADS: &[&str] = &["memory", "sdm", "softsdm", "sdmscale"];
pub const SDM_RUN_HEADS: &[&str] = &["refusal", "contenttrack"];
