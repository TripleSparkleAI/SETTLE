//! SETTLE: a toy language where a program is springs and running it means letting it settle.
//! Syntax in the style of Rails (blocks, symbols, keyword arguments); engine in Rust with no dependencies.
//!
//! Layout: `lex` tokens and errors · `rng` randomness · `model` things, pulls and one run's state ·
//! `ext` the plug-in slot every statement family uses · `interp` blocks and dispatch · `core` the base
//! statements · `memory` store patterns and text in the pulls, recall by shaking · `grid` one thing per pixel, pictures in and out, play a folder of frames.
//! statements · `memory` store patterns and text in the pulls, recall by shaking · `learn` fit leans and pulls
//! from examples, and classify by settling.
//! statements · `memory` store patterns and text in the pulls, recall by shaking · `sdm` Kanerva hard locations.
//! statements · `memory` store patterns and text in the pulls, recall by shaking · `grid` one thing per pixel, pictures in and out, play a folder of frames · `colour` three grids per colour picture, PPM in and out.

pub mod coded;
pub mod colour;
pub mod core;
pub mod export;
pub mod filmsharp;
pub mod filmwarm;
pub mod denoise;
pub mod descend;
pub mod ext;
pub mod grid;
pub mod interp;
pub mod learn;
pub mod ldpcsettle;
pub mod ldpcmoves;
pub mod lex;
pub mod memory;
pub mod mnist;
pub mod model;
pub mod numbers;
pub mod rng;
pub mod zoo;
pub mod zoohard;
pub mod zootemp;
pub mod valleys;
pub mod sdm;
pub mod softsdm;
pub mod sdmscale;
pub mod sdmradius;
pub mod sdmrefuse;
pub mod sdmtrack;
