//! WORDS, the second floor: the language. A program is read line by line (`lex`), blocks are opened and closed
//! (`interp`), and each statement line is offered to the statement families through THE REGISTRY (`registry`, the
//! socket every family plugs into). The core family (`core`) has two faces over one word list (`vocab`): the file
//! face, which parses program lines, and the builder face (`builder`), which writes the same statements in Rust.
//! Both run through one executor, so they cannot disagree.
//!
//! The other statement families live in `src/<family>.rs` and plug into the registry the same way.

pub mod builder;
pub mod core;
pub mod interp;
pub mod lex;
pub mod names;
pub mod registry;
pub mod vocab;
