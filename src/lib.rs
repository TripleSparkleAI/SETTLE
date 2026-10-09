//! SETTLE: a toy language where a program is springs and running it means letting it settle.
//! Syntax in the style of Rails (blocks, symbols, keyword arguments); engine in Rust.
//!
//! THE THREE FLOORS (the same three KANERVA has):
//! - `engine`: data in, data out. Things, leans, pulls, the sampler, the anneal schedule, energy, answers, the
//!   generator and the word codes. No parsing, no printing, no keyword names.
//! - `words`: the language. Lines, blocks, THE REGISTRY every statement family plugs into, the core family with
//!   its two faces (program lines, and the Rust builder `Model::build()`) over one word list.
//! - `doors`: the ways in and out. The `settle` command (`doors::cli`) and JSON (`doors::json`).
//!
//! The statement families, one file each, plug into the registry: `memory`, `grid`, `zoo`, `learn`, `valleys`,
//! `numbers`, `sdm`, `softsdm`, `export`, `colour`, `sdmscale`, `coded`, `denoise`, `ldpcsettle`, `ldpcmoves`,
//! `sdmrefuse`, `zootemp`, `sdmtrack`, `descend` (and `core`, in `words`). Modules with no statements of their
//! own: `filmsharp` and `filmwarm` (grid and colour options), `zoohard` (hard puzzle generators), `sdmradius`
//! (radius per damage), `mnist` (the MNIST reader and restricted machines).
//!
//! KANERVA, the sparse distributed memory crate, is SETTLE's one dependency and it is OPTIONAL: the `sdm` feature
//! (on by default) brings it in with the sdm-family statements (`memory`, `sdm`, `softsdm`, `sdmscale`,
//! `sdmrefuse`, `sdmtrack`, and `coded`, which stores into those memories). Built with `--no-default-features`,
//! SETTLE runs every other statement with no KANERVA at all, and an sdm-family line is refused by name
//! (`sdmoff`). The documentation is in `docs/` (start at `docs/README.md`).

// Numerical kernels here index arrays by position the way their equations do, and several are ported line for line
// to the site's JavaScript (mnist, the film player); iterator rewrites would hide that correspondence.
#![allow(clippy::needless_range_loop)]

pub mod doors;
pub mod engine;
pub mod words;

// The paths every family and every outside caller used before the floors: kept, so nothing had to move with them.
pub use engine::{model, rng};
pub use words::registry as ext;
pub use words::{core, interp, lex};

pub mod coded;
pub mod colour;
pub mod denoise;
pub mod descend;
pub mod export;
pub mod filmsharp;
pub mod filmwarm;
pub mod grid;
pub mod ldpcmoves;
pub mod ldpcsettle;
pub mod learn;
#[cfg(feature = "sdm")]
pub mod memory;
pub mod mnist;
pub mod numbers;
#[cfg(feature = "sdm")]
pub mod plug;
#[cfg(feature = "sdm")]
pub mod sdm;
#[cfg(not(feature = "sdm"))]
pub mod sdmoff;
#[cfg(feature = "sdm")]
pub mod sdmradius;
#[cfg(feature = "sdm")]
pub mod sdmrefuse;
#[cfg(feature = "sdm")]
pub mod sdmscale;
#[cfg(feature = "sdm")]
pub mod sdmtrack;
#[cfg(feature = "sdm")]
pub mod softsdm;
pub mod valleys;
pub mod zoo;
pub mod zoohard;
pub mod zootemp;
