//! The stand-in for the sdm-family statements when SETTLE is built without the `sdm` feature (no KANERVA). It
//! claims the lines that open those families and refuses each one by name, so a program written for the full
//! language gets a clear answer instead of `no statement family knows this line`.

use crate::ext::{Claim, Ctx, Ext};
use crate::lex::{err, Tok};
use crate::model::{Model, State};
use crate::words::vocab::{SDM_MODEL_HEADS as MODEL_HEADS, SDM_RUN_HEADS as RUN_HEADS};

pub struct SdmOff;


fn refuse(word: &str, ln: usize) -> Claim {
    Some(err(
        ln,
        format!(
            "`{}` is a KANERVA statement, and this SETTLE was built without the `sdm` feature; build it with the feature on (the default): cargo build --release --features sdm",
            word
        ),
    ))
}

impl Ext for SdmOff {
    fn name(&self) -> &'static str {
        "sdm (off)"
    }

    fn statements(&self) -> &'static [&'static str] {
        &[
            "model: memory :m   /   sdm :s   /   softsdm :s   /   sdmscale :k   (need the sdm feature)",
            "run: refusal ...   /   contenttrack ...   (need the sdm feature)",
        ]
    }

    fn model_stmt(&self, _m: &mut Model, t: &[Tok], ln: usize, _ctx: &mut Ctx) -> Claim {
        match t {
            [Tok::Ident(k), Tok::Sym(_), ..] if MODEL_HEADS.contains(&k.as_str()) => refuse(k, ln),
            _ => None,
        }
    }

    fn run_stmt(&self, _m: &mut Model, _st: &mut State, t: &[Tok], ln: usize, _ctx: &mut Ctx) -> Claim {
        match t {
            [Tok::Ident(k), ..] if RUN_HEADS.contains(&k.as_str()) => refuse(k, ln),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::interp::Interp;

    #[test]
    fn an_sdm_line_is_refused_by_name_and_the_rest_still_runs() {
        let e = Interp::default().exec("model :m do\n  sdm :s, word-size: 64\nend").unwrap_err().0;
        assert!(e.starts_with("line 2: `sdm` is a KANERVA statement"), "{}", e);
        assert!(e.contains("--features sdm"), "{}", e);
        let e = Interp::default().exec("model :m do\n  thing :a\nend\nrun :m do\n  refusal word-size: 256\nend").unwrap_err().0;
        assert!(e.starts_with("line 5: `refusal` is a KANERVA statement"), "{}", e);
        // the control: a core program needs no KANERVA and runs
        let out = Interp::default().exec("model :m do\n  thing :a\nend\nrun :m do\n  settle 10, seed: 1\nend").unwrap();
        assert_eq!(out[0], "settled: 10 samples of 1 things at temperature 1");
    }
}
