//! An example statement family, `chain`, used by docs/07-extending.md and compiled by tests/docs_examples.rs.
//!
//! model:  chain :x, length: 5, by: 1    declares x1 .. x5, each pulling the next by 1
//! run:    x.ends                         prints how often the two ends of the chain agreed in the last settle

use settle::ext::{Claim, Ctx, Ext};
use settle::lex::{err, kw, kwargs, num, only, SettleError, Tok};
use settle::model::{Model, State};

pub struct Chain;

/// Declare the links and their pulls, and remember the chain's length in the model's notes.
fn declare(m: &mut Model, name: &str, rest: &[Tok], ln: usize) -> Result<(), SettleError> {
    let kv = kwargs(rest, ln)?;
    only(&kv, &["length", "by"], "chain", ln)?;
    let length = kw(&kv, "length").map(|v| num(v, ln)).transpose()?.unwrap_or(5.0) as usize;
    let by = kw(&kv, "by").map(|v| num(v, ln)).transpose()?.unwrap_or(1.0);
    if length < 2 {
        return err(ln, "a chain needs at least 2 links");
    }
    let ids: Vec<usize> = (1..=length).map(|k| m.add(&format!("{}{}", name, k))).collect();
    for w in ids.windows(2) {
        m.couple(w[0], w[1], by);
    }
    m.notes.insert(format!("chain:{}", name), (vec![length as f64], Vec::new()));
    Ok(())
}

/// The fraction of recorded samples in which the first and last links agree.
fn ends(m: &Model, st: &State, name: &str, ln: usize, ctx: &mut Ctx) -> Result<(), SettleError> {
    let length = match m.notes.get(&format!("chain:{}", name)) {
        Some((v, _)) => v[0] as usize,
        None => return err(ln, format!("no chain :{} (declare it with: chain :{})", name, name)),
    };
    if st.samples.is_empty() {
        return err(ln, "ends needs a settle first");
    }
    let (a, b) = (m.need(&format!("{}1", name), ln)?, m.need(&format!("{}{}", name, length), ln)?);
    let agree = st.samples.iter().filter(|s| s[a] == s[b]).count();
    ctx.say(format!("chain :{}: ends agree in {:.1}% of {} samples", name, 100.0 * agree as f64 / st.samples.len() as f64, st.samples.len()));
    Ok(())
}

impl Ext for Chain {
    fn name(&self) -> &'static str {
        "chain"
    }

    fn statements(&self) -> &'static [&'static str] {
        &["model: chain :x, length: 5, by: 1", "run: x.ends"]
    }

    fn model_stmt(&self, m: &mut Model, t: &[Tok], ln: usize, _ctx: &mut Ctx) -> Claim {
        match t {
            [Tok::Ident(k), Tok::Sym(name), rest @ ..] if k == "chain" => {
                let rest = rest.strip_prefix(&[Tok::Comma]).unwrap_or(rest);
                Some(declare(m, name, rest, ln))
            }
            _ => None,
        }
    }

    fn run_stmt(&self, m: &mut Model, st: &mut State, t: &[Tok], ln: usize, ctx: &mut Ctx) -> Claim {
        match t {
            [Tok::Ident(name), Tok::Dot, Tok::Ident(verb)] if verb == "ends" => Some(ends(m, st, name, ln, ctx)),
            _ => None,
        }
    }
}
