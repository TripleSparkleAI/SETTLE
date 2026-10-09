//! THE REGISTRY (the socket): the plug-in slot of the words floor. Every family of statements (the core, grids, memory, learning, numbers) is one `Ext`
//! in its own file. The interpreter offers each statement to every Ext in turn; the first to claim it runs it.
//!
//! To add a family: write `src/<family>.rs` with a unit struct implementing `Ext`, add `pub mod <family>;` in
//! lib.rs, and add one line to `registry()` below. A statement no Ext claims is an error naming the line.

use crate::engine::model::{Model, State};
use crate::words::lex::{SettleError, Tok};
use std::path::PathBuf;

/// What a statement can see besides its model and run.
pub struct Ctx<'a> {
    /// Directory of the program file; relative paths in statements resolve against it.
    pub base_dir: PathBuf,
    /// Lines to print.
    pub out: &'a mut Vec<String>,
}

impl Ctx<'_> {
    pub fn path(&self, p: &str) -> PathBuf {
        let q = PathBuf::from(p);
        if q.is_absolute() {
            q
        } else {
            self.base_dir.join(q)
        }
    }
    pub fn say(&mut self, line: impl Into<String>) {
        self.out.push(line.into());
    }
}

/// `None` means "not mine"; `Some(result)` means the statement was claimed.
pub type Claim = Option<Result<(), SettleError>>;

pub trait Ext {
    /// Short family name, used in the `help` listing.
    fn name(&self) -> &'static str;
    /// One line per statement this family adds, for error messages and help.
    fn statements(&self) -> &'static [&'static str];
    /// A statement inside `model :name do ... end`.
    fn model_stmt(&self, _m: &mut Model, _t: &[Tok], _ln: usize, _ctx: &mut Ctx) -> Claim {
        None
    }
    /// A statement inside `run :name do ... end`. The model is mutable so learning can change it.
    fn run_stmt(&self, _m: &mut Model, _st: &mut State, _t: &[Tok], _ln: usize, _ctx: &mut Ctx) -> Claim {
        None
    }
}

/// Every statement family, core first, in the order they are offered each line. The sdm-family entries come with
/// the `sdm` feature (KANERVA); without it one stand-in (`sdmoff`) claims their lines with a clear refusal.
pub fn registry() -> Vec<Box<dyn Ext>> {
    let mut v: Vec<Box<dyn Ext>> = vec![Box::new(crate::core::Core)];
    #[cfg(feature = "sdm")]
    v.push(Box::new(crate::memory::Memory));
    v.push(Box::new(crate::grid::Grid));
    v.push(Box::new(crate::zoo::Zoo));
    v.push(Box::new(crate::learn::Learn));
    v.push(Box::new(crate::valleys::Valleys));
    v.push(Box::new(crate::numbers::Numbers));
    #[cfg(feature = "sdm")]
    {
        v.push(Box::new(crate::sdm::Sdm));
        v.push(Box::new(crate::softsdm::SoftSdm));
    }
    v.push(Box::new(crate::export::Export));
    v.push(Box::new(crate::colour::Colour));
    #[cfg(feature = "sdm")]
    {
        v.push(Box::new(crate::sdmscale::SdmScale));
        v.push(Box::new(crate::coded::Coded));
    }
    v.push(Box::new(crate::denoise::Denoise));
    v.push(Box::new(crate::ldpcsettle::LdpcSettle));
    v.push(Box::new(crate::ldpcmoves::LdpcMoves));
    #[cfg(feature = "sdm")]
    v.push(Box::new(crate::sdmrefuse::SdmRefuse));
    v.push(Box::new(crate::zootemp::ZooTemp));
    #[cfg(feature = "sdm")]
    v.push(Box::new(crate::sdmtrack::SdmTrack));
    v.push(Box::new(crate::descend::Descend));
    #[cfg(not(feature = "sdm"))]
    v.push(Box::new(crate::sdmoff::SdmOff));
    v
}
