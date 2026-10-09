//! DOORS, the third floor: the ways in and out of the language. `cli` is the `settle` command (the binary in
//! `src/main.rs` is a shim over it), and `json` is JSON in and out: the command's `--json` answer, and the small
//! reader and writer the `export` statements use for their files.

pub mod cli;
pub mod json;
