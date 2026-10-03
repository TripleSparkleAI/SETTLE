//! The plug-in slot. Every family of statements (the core, grids, memory, learning, numbers) is one `Ext`
//! in its own file. The interpreter offers each statement to every Ext in turn; the first to claim it runs it.
//!
//! To add a family: write `src/<family>.rs` with a unit struct implementing `Ext`, add `pub mod <family>;` in
//! lib.rs, and add one line to `registry()` below. A statement no Ext claims is an error naming the line.

use crate::lex::{SettleError, Tok};
use crate::model::{Model, State};
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

/// Every statement family, core first. Lanes each add one line here.
pub fn registry() -> Vec<Box<dyn Ext>> {
    vec![
        Box::new(crate::core::Core),
        Box::new(crate::memory::Memory),
        Box::new(crate::grid::Grid),
        Box::new(crate::zoo::Zoo),
        Box::new(crate::learn::Learn),
        Box::new(crate::valleys::Valleys),
        Box::new(crate::numbers::Numbers),
        Box::new(crate::sdm::Sdm),
        Box::new(crate::softsdm::SoftSdm),
        Box::new(crate::export::Export),
        Box::new(crate::colour::Colour),
        Box::new(crate::sdmscale::SdmScale),
        Box::new(crate::coded::Coded),
        Box::new(crate::denoise::Denoise),
        Box::new(crate::ldpcsettle::LdpcSettle),
        Box::new(crate::ldpcmoves::LdpcMoves),
        Box::new(crate::sdmrefuse::SdmRefuse),
        Box::new(crate::zootemp::ZooTemp),
        Box::new(crate::sdmtrack::SdmTrack),
        Box::new(crate::descend::Descend),
    ]
}
