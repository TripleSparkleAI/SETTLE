//! THE PLUG: KANERVA's sdm family, mounted in SETTLE's statement registry.
//!
//! SETTLE does not parse `sdm`, `softsdm`, `sdmscale`, `refusal` or `contenttrack` lines itself. Each family is
//! one registry entry, a [`Mount`] of a `kanerva::lang::Family`: the line goes to KANERVA's parser (the same one
//! the `kanerva` command uses), and the typed statement it returns goes to SETTLE's engine bridge for that family
//! (`sdm::exec`, `softsdm::exec`, `sdmscale::exec`, `sdmrefuse::exec`, `sdmtrack::exec`), where the bit-counters
//! live in SETTLE's pulls. So a `.settle` line and a `.kanerva` line can never be read two ways.
//!
//! <claudes_code_comments>
//! ** Function List **
//! Mount(Family)           - one KANERVA family as a registry entry (implements Ext)
//! Declared(&Model)        - KANERVA's Names question answered from the model's notes
//! to_kanerva(toks)        - SETTLE's tokens as KANERVA's (the same seven kinds)
//! exec(m, st, stmt, ...)  - a parsed statement handed to its family's engine bridge
//! mount!(Struct, Family)  - declares a unit struct that registers as a Mount (keeps the registry's old names)
//!
//! ** Technical Review **
//! - The registry keeps today's five entries in today's five places (`sdm::Sdm`, `softsdm::SoftSdm`,
//!   `sdmscale::SdmScale`, `sdmrefuse::SdmRefuse`, `sdmtrack::SdmTrack`), each now a mount, so no line changes
//!   which family claims it.
//! - A declared memory is a note on the model, `sdm:<name>`, `softsdm:<name>` or `sdmscale:<name>`; that is how
//!   KANERVA's parser asks whether `s.read` belongs to an sdm.
//! - KANERVA's errors carry SETTLE's words (`line N: ...`), so they pass through unchanged.
//! - The help lines and the bracketed family names come from `kanerva::words::HELP`.
//!
//! </claudes_code_comments>

use crate::ext::{Claim, Ctx, Ext};
use crate::lex::{SettleError, Tok};
use crate::model::{Model, State};
use kanerva::lang::{self, Family, Names, Stmt};
use kanerva::words::Place;

/// One KANERVA statement family, mounted as a SETTLE registry entry.
pub struct Mount(pub Family);

/// KANERVA's parser asks whether a name is a declared memory; the model's notes answer.
pub struct Declared<'a>(pub &'a Model);

impl Names for Declared<'_> {
    fn declared(&self, family: Family, name: &str) -> bool {
        match family {
            Family::Sdm | Family::SoftSdm | Family::SdmScale => self.0.notes.contains_key(&format!("{}:{}", family.word(), name)),
            Family::Refusal | Family::ContentTrack => false,
        }
    }
}

/// SETTLE's tokens as KANERVA's: the two lexers read the same syntax into the same seven kinds.
pub fn to_kanerva(t: &[Tok]) -> Vec<lang::Tok> {
    t.iter()
        .map(|x| match x {
            Tok::Sym(s) => lang::Tok::Sym(s.clone()),
            Tok::Label(s) => lang::Tok::Label(s.clone()),
            Tok::Ident(s) => lang::Tok::Ident(s.clone()),
            Tok::Num(v) => lang::Tok::Num(*v),
            Tok::Str(s) => lang::Tok::Str(s.clone()),
            Tok::Comma => lang::Tok::Comma,
            Tok::Dot => lang::Tok::Dot,
        })
        .collect()
}

/// Hand a parsed statement to its family's engine bridge. `st` is the run's state, None inside a model.
pub fn exec(m: &mut Model, st: Option<&mut State>, s: Stmt, ln: usize, ctx: &mut Ctx) -> Result<(), SettleError> {
    match s {
        Stmt::SdmDeclare { .. } | Stmt::SdmWrite { .. } | Stmt::SdmRead { .. } => crate::sdm::exec(m, st, s, ln, ctx),
        Stmt::SoftDeclare { .. } | Stmt::SoftWrite { .. } | Stmt::SoftRead { .. } | Stmt::SoftAttend { .. } => crate::softsdm::exec(m, st, s, ln, ctx),
        Stmt::ScaleDeclare { .. } | Stmt::ScalePut { .. } | Stmt::ScaleFill { .. } | Stmt::ScaleRead { .. } => crate::sdmscale::exec(m, st, s, ln, ctx),
        Stmt::Refusal { word_size, load, level } => {
            ctx.say(lang::say::refusal(word_size, load, level));
            Ok(())
        }
        Stmt::ContentTrack { word_size, hard_locations, load, address_noise, block, samples } => {
            ctx.say(lang::say::contenttrack(word_size, hard_locations, load, address_noise, block, samples));
            Ok(())
        }
    }
}

impl Mount {
    fn claim(&self, m: &mut Model, st: Option<&mut State>, place: Place, t: &[Tok], ln: usize, ctx: &mut Ctx) -> Claim {
        let parsed = self.0.parse(place, &to_kanerva(t), ln, &Declared(m))?;
        Some(parsed.map_err(|e| SettleError(e.0)).and_then(|s| exec(m, st, s, ln, ctx)))
    }
}

impl Ext for Mount {
    fn name(&self) -> &'static str {
        self.0.ext_name()
    }

    fn statements(&self) -> &'static [&'static str] {
        self.0.help()
    }

    fn model_stmt(&self, m: &mut Model, t: &[Tok], ln: usize, ctx: &mut Ctx) -> Claim {
        self.claim(m, None, Place::Model, t, ln, ctx)
    }

    fn run_stmt(&self, m: &mut Model, st: &mut State, t: &[Tok], ln: usize, ctx: &mut Ctx) -> Claim {
        self.claim(m, Some(st), Place::Run, t, ln, ctx)
    }
}

/// `mount!(Sdm, Family::Sdm)`: a unit struct the registry names, registering as `Mount(Family::Sdm)`.
macro_rules! mount {
    ($name:ident, $family:expr) => {
        /// This family as SETTLE registers it: KANERVA's parser mounted (see `crate::plug`).
        pub struct $name;

        impl $crate::ext::Ext for $name {
            fn name(&self) -> &'static str {
                $crate::plug::Mount($family).name()
            }
            fn statements(&self) -> &'static [&'static str] {
                $crate::plug::Mount($family).statements()
            }
            fn model_stmt(&self, m: &mut $crate::model::Model, t: &[$crate::lex::Tok], ln: usize, ctx: &mut $crate::ext::Ctx) -> $crate::ext::Claim {
                $crate::plug::Mount($family).model_stmt(m, t, ln, ctx)
            }
            fn run_stmt(
                &self,
                m: &mut $crate::model::Model,
                st: &mut $crate::model::State,
                t: &[$crate::lex::Tok],
                ln: usize,
                ctx: &mut $crate::ext::Ctx,
            ) -> $crate::ext::Claim {
                $crate::plug::Mount($family).run_stmt(m, st, t, ln, ctx)
            }
        }
    };
}
pub(crate) use mount;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interp::Interp;

    #[test]
    fn the_registry_holds_the_five_mounts_in_their_places() {
        let names: Vec<&str> = crate::ext::registry().iter().map(|e| e.name()).collect();
        for f in Family::ALL {
            assert!(names.contains(&f.ext_name()), "{} is not registered", f.ext_name());
        }
        // the order SETTLE has always offered lines in: sdm before softsdm before sdmscale before sdmrefuse
        // before sdmtrack, with the memory family ahead of them all
        let at = |n: &str| names.iter().position(|x| *x == n).unwrap();
        assert!(at("memory") < at("sdm") && at("sdm") < at("softsdm") && at("softsdm") < at("sdmscale"));
        assert!(at("sdmscale") < at("sdmrefuse") && at("sdmrefuse") < at("sdmtrack"));
    }

    #[test]
    fn a_line_reaches_kanervas_parser_and_its_error_comes_back_word_for_word() {
        let e = Interp::default().exec("model :m do\n  sdm :s, word-size: 64, sed: 1\nend").err().unwrap().0;
        assert_eq!(e, "line 2: sdm does not take `sed:`; did you mean `seed:`?");
        // the alias: write_samples: is write-samples: in both spellings
        let a = Interp::default().exec("model :m do\n  softsdm :f, word-size: 64, hard-locations: 50, write_samples: 4\n  f.write :c\nend");
        let b = Interp::default().exec("model :m do\n  softsdm :f, word-size: 64, hard-locations: 50, write-samples: 4\n  f.write :c\nend");
        assert!(a.is_ok() && b.is_ok());
        // the negative control: a keyword no statement takes is still refused
        assert!(Interp::default().exec("model :m do\n  softsdm :f, write-sample: 4\nend").is_err());
    }

    #[test]
    fn declared_reads_the_notes_and_nothing_else() {
        let mut it = Interp::default();
        it.exec("model :m do\n  sdm :s, word-size: 64, hard-locations: 10\n  thing :k\nend").unwrap();
        let m = &it.models["m"];
        assert!(Declared(m).declared(Family::Sdm, "s"));
        assert!(!Declared(m).declared(Family::SoftSdm, "s"));
        assert!(!Declared(m).declared(Family::Sdm, "k"));
    }
}
