//! The core statements: thing, pulls, pushes (in a model); hold, settle, anneal, show, best, ask (in a run).
//!
//! <claudes_code_comments>
//! ** Function List **
//! ModelStmt / RunStmt      - one core statement as data: what the file face reads and the builder face builds
//! parse_model / parse_run  - the file face: tokens of one line to a statement (names checked in source order)
//! apply_model / apply_run  - the one executor both faces share: a statement against the engine, lines out
//! show_lines / best_line / settled_line / annealed_line / ask_line - the printed forms
//! Core (Ext)               - the family as the registry sees it: statements() for help, the two hooks
//!
//! ** Technical Review **
//! - THE TWO FACES, ONE EXECUTOR. A program line is parsed into a `ModelStmt` or `RunStmt`; the Rust builder
//!   (`words::builder`) constructs the same values. Both go through `apply_model` / `apply_run`, which call the
//!   engine (`engine::model`, `engine::answers`) and format the lines. So the two faces cannot print differently:
//!   `tests/two_faces.rs` holds them equal on the documented examples.
//! - Parsing checks names in the order the line names them (`Model::need`), then keywords, exactly as before the
//!   split, so every error message and its order is unchanged.
//! - Every keyword is a constant of `words::vocab`, the one word list the builder's method names are held to.
//!
//! </claudes_code_comments>

use crate::engine::answers::{count_yes, Join, Question};
use crate::engine::model::{Model, State, Update};
use crate::engine::rng::Rng;
use crate::words::lex::{err, kw, kwargs, num, only, yes_no, SettleError, Tok};
use crate::words::names::find;
use crate::words::registry::{Claim, Ctx, Ext};
use crate::words::vocab as w;

pub struct Core;

/// A core statement inside `model :name do ... end`.
#[derive(Clone, Debug, PartialEq)]
pub enum ModelStmt {
    /// `thing :a, :b, leans: :yes, by: 1`: declare the names; add `lean` (+1 yes, -1 no) times `by` to each.
    Thing { names: Vec<String>, lean: Option<(f64, f64)> },
    /// `a.pulls :b, by: 2` (positive `by`) or `a.pushes :b, by: 2` (stored as a pull of `-by`).
    Couple { a: String, b: String, pull: bool, by: f64 },
}

/// A core statement inside `run :name do ... end`.
#[derive(Clone, Debug, PartialEq)]
pub enum RunStmt {
    /// `hold :a, :yes`: fix a thing at yes (+1) or no (-1).
    Hold { thing: String, value: f64 },
    /// `settle 10_000, temperature: 1, seed: 1, update: :metro`.
    Settle { sweeps: usize, temperature: Option<f64>, seed: Option<u64>, update: Option<Update> },
    /// `anneal 4_000, temperature: 1, seed: 1, update: :metro`.
    Anneal { sweeps: usize, temperature: Option<f64>, seed: Option<u64>, update: Option<Update> },
    /// `show`: each thing's yes-rate over the last settle.
    Show,
    /// `best`: the calmest arrangement the last anneal found.
    Best,
    /// `ask :a, and: :b, or_not: :c`.
    Ask { first: String, terms: Vec<(Join, String)> },
}

/// The keyword of a join, as written in a program and as a builder method.
pub fn join_word(j: Join) -> &'static str {
    match j {
        Join::And => w::AND,
        Join::Or => w::OR,
        Join::AndNot => w::AND_NOT,
        Join::OrNot => w::OR_NOT,
    }
}

fn join_of(word: &str) -> Option<Join> {
    [Join::And, Join::Or, Join::AndNot, Join::OrNot].into_iter().find(|&j| join_word(j) == word)
}

// ---------------------------------------------------------------- the file face

fn parse_thing(rest: &[Tok], ln: usize) -> Result<ModelStmt, SettleError> {
    let mut i = 0;
    let mut names = Vec::new();
    while i < rest.len() {
        match &rest[i] {
            Tok::Sym(s) => {
                if [w::YES, w::NO].contains(&s.as_str()) {
                    return err(ln, format!(":{} cannot be a thing name", s));
                }
                names.push(s.clone());
                i += 1;
            }
            Tok::Comma => i += 1,
            Tok::Label(_) => break,
            other => return err(ln, format!("unexpected `{}` in thing; things are named by symbols, like `thing :rain, :sprinkler`", other)),
        }
    }
    let kv = kwargs(&rest[i..], ln)?;
    only(&kv, &[w::LEANS, w::BY], w::THING, ln)?;
    let lean = kw(&kv, w::LEANS).map(|v| yes_no(v, ln)).transpose()?;
    let by = kw(&kv, w::BY).map(|v| num(v, ln)).transpose()?;
    match (lean, by) {
        (Some(l), Some(b)) => Ok(ModelStmt::Thing { names, lean: Some((l, b)) }),
        (None, None) => Ok(ModelStmt::Thing { names, lean: None }),
        _ => err(ln, "use `leans:` and `by:` together"),
    }
}

fn parse_couple(m: &Model, a: &str, verb: &str, b: &str, rest: &[Tok], ln: usize) -> Result<ModelStmt, SettleError> {
    let (i, k) = (m.need(a, ln)?, m.need(b, ln)?);
    if i == k {
        return err(ln, "a thing cannot pull itself");
    }
    let kv = kwargs(rest, ln)?;
    only(&kv, &[w::BY], verb, ln)?;
    let by = match kw(&kv, w::BY) {
        Some(v) => num(v, ln)?,
        None => return err(ln, format!("{} needs `by:`", verb)),
    };
    Ok(ModelStmt::Couple { a: a.to_string(), b: b.to_string(), pull: verb == w::PULLS, by })
}

/// A model line as a core statement; `None` when the line is not a core statement.
pub fn parse_model(m: &Model, t: &[Tok], ln: usize) -> Option<Result<ModelStmt, SettleError>> {
    match t {
        [Tok::Ident(k), rest @ ..] if k == w::THING => Some(parse_thing(rest, ln)),
        [Tok::Ident(a), Tok::Dot, Tok::Ident(verb), Tok::Sym(b), rest @ ..] if verb == w::PULLS || verb == w::PUSHES => {
            Some(parse_couple(m, a, verb, b, rest, ln))
        }
        _ => None,
    }
}

/// The count of a `settle` or `anneal`: a whole number of sweeps, one or more.
fn sweeps_of(v: f64, what: &str, ln: usize) -> Result<usize, SettleError> {
    if v >= 1.0 && v.fract() == 0.0 && v <= usize::MAX as f64 {
        Ok(v as usize)
    } else {
        err(ln, format!("{} takes a whole number of sweeps, 1 or more, like `{} 10_000`; got {}", what, what, v))
    }
}

/// The word of an update rule, as written in a program and passed to the builder.
pub fn update_word(u: Update) -> &'static str {
    match u {
        Update::Gibbs => w::GIBBS,
        Update::Metro => w::METRO,
    }
}

/// An update rule by its word.
pub fn update_of(word: &str) -> Option<Update> {
    [Update::Gibbs, Update::Metro].into_iter().find(|&u| update_word(u) == word)
}

/// `temperature:`, `seed:` and `update:`, shared by settle and anneal.
type RunOpts = (Option<f64>, Option<u64>, Option<Update>);

fn run_opts(rest: &[Tok], what: &str, ln: usize) -> Result<RunOpts, SettleError> {
    let kv = kwargs(rest, ln)?;
    only(&kv, &[w::TEMPERATURE, w::SEED, w::UPDATE], what, ln)?;
    let temperature = match kw(&kv, w::TEMPERATURE) {
        Some(v) => {
            let t = num(v, ln)?;
            if t <= 0.0 {
                return err(ln, "temperature must be above zero");
            }
            Some(t)
        }
        None => None,
    };
    let seed = kw(&kv, w::SEED).map(|v| num(v, ln).map(|x| x as u64)).transpose()?;
    let update = match kw(&kv, w::UPDATE) {
        None => None,
        Some(Tok::Sym(s)) if update_of(s).is_some() => update_of(s),
        Some(_) => return err(ln, "`update:` takes :gibbs or :metro"),
    };
    Ok((temperature, seed, update))
}

fn parse_ask(m: &Model, st: &State, first: &str, rest: &[Tok], ln: usize) -> Result<RunStmt, SettleError> {
    if st.n == 0 {
        return err(ln, "ask needs a settle first");
    }
    if st.samples.is_empty() {
        return err(ln, "this settle was too large to keep every sample; ask needs a smaller one");
    }
    m.need(first, ln)?;
    let mut terms = Vec::new();
    for (k, v) in kwargs(rest, ln)? {
        let other = match &v {
            Tok::Sym(s) => {
                m.need(s, ln)?;
                s.clone()
            }
            _ => return err(ln, "ask terms are symbols, like `and: :sprinkler`"),
        };
        match join_of(&k) {
            Some(j) => terms.push((j, other)),
            None => return err(ln, format!("ask takes and: / or: / and_not: / or_not:, not `{}:`", k)),
        }
    }
    Ok(RunStmt::Ask { first: first.to_string(), terms })
}

/// A run line as a core statement; `None` when the line is not a core statement.
pub fn parse_run(m: &Model, st: &State, t: &[Tok], ln: usize) -> Option<Result<RunStmt, SettleError>> {
    Some(match t {
        [Tok::Ident(k), Tok::Sym(a), Tok::Comma, v] if k == w::HOLD => {
            m.need(a, ln).and_then(|_| Ok(RunStmt::Hold { thing: a.clone(), value: yes_no(v, ln)? }))
        }
        [Tok::Ident(k), Tok::Num(nv), rest @ ..] if k == w::SETTLE => run_opts(rest, w::SETTLE, ln)
            .and_then(|(temperature, seed, update)| Ok(RunStmt::Settle { sweeps: sweeps_of(*nv, w::SETTLE, ln)?, temperature, seed, update })),
        [Tok::Ident(k), Tok::Num(nv), rest @ ..] if k == w::ANNEAL => run_opts(rest, w::ANNEAL, ln)
            .and_then(|(temperature, seed, update)| Ok(RunStmt::Anneal { sweeps: sweeps_of(*nv, w::ANNEAL, ln)?, temperature, seed, update })),
        [Tok::Ident(k)] if k == w::SHOW => Ok(RunStmt::Show),
        [Tok::Ident(k)] if k == w::BEST => Ok(RunStmt::Best),
        // a model with a `loss` (the descend family) asks about its parameters when :first is not a thing
        [Tok::Ident(k), Tok::Sym(first), rest @ ..]
            if k == w::ASK && (m.idx.contains_key(first) || !m.notes.contains_key("descend:loss")) =>
        {
            parse_ask(m, st, first, rest, ln)
        }
        _ => return None,
    })
}

// ---------------------------------------------------------------- the one executor

/// Apply a model statement. The error is the message without a line; the file face adds its line.
pub fn apply_model(m: &mut Model, s: &ModelStmt) -> Result<(), String> {
    match s {
        ModelStmt::Thing { names, lean } => {
            if let Some(bad) = names.iter().find(|n| *n == w::YES || *n == w::NO) {
                return Err(format!(":{} cannot be a thing name", bad));
            }
            let declared: Vec<usize> = names.iter().map(|n| m.add(n)).collect();
            if let Some((l, b)) = lean {
                for &d in &declared {
                    m.h[d] += l * b;
                }
            }
            Ok(())
        }
        ModelStmt::Couple { a, b, pull, by } => {
            let (i, k) = (find(m, a)?, find(m, b)?);
            if i == k {
                return Err("a thing cannot pull itself".into());
            }
            m.couple(i, k, if *pull { *by } else { -by });
            Ok(())
        }
    }
}

/// The line `settle` prints.
pub fn settled_line(sweeps: usize, things: usize, temp: f64) -> String {
    format!("settled: {} samples of {} things at temperature {}", sweeps, things, temp)
}

/// The line `anneal` prints.
pub fn annealed_line(sweeps: usize, energy: f64) -> String {
    format!("annealed: {} sweeps, calmest energy found {:.3}", sweeps, energy)
}

/// The lines `show` prints: a bar and a percentage per thing, `(held)` beside held things.
pub fn show_lines(m: &Model, st: &State) -> Vec<String> {
    m.names
        .iter()
        .zip(st.rates())
        .enumerate()
        .map(|(i, (nm, p))| {
            let tag = if st.held.contains_key(&i) { "  (held)" } else { "" };
            format!("  {:<14} {} {:.1}%{}", nm, "#".repeat((p * 30.0).round() as usize), 100.0 * p, tag)
        })
        .collect()
}

/// The line `best` prints.
pub fn best_line(m: &Model, b: &[f64], e: f64) -> String {
    let parts: Vec<String> = m.names.iter().enumerate().map(|(i, nm)| format!("{} {}", nm, if b[i] > 0.0 { w::YES } else { w::NO })).collect();
    format!("best (energy {:.3}): {}", e, parts.join(", "))
}

/// The line `ask` prints.
pub fn ask_line(first: &str, terms: &[(Join, String)], yes: usize, of: usize) -> String {
    let mut desc = format!(":{}", first);
    for (j, o) in terms {
        desc.push_str(&format!(", {}: :{}", join_word(*j), o));
    }
    format!("ask {}: yes {:.1}% of {} samples", desc, 100.0 * (yes as f64 / of as f64), of)
}

fn set_opts(st: &mut State, temperature: Option<f64>, seed: Option<u64>, update: Option<Update>) -> Result<(), String> {
    if let Some(t) = temperature {
        if t <= 0.0 {
            return Err("temperature must be above zero".into());
        }
        st.temp = t;
    }
    if let Some(s) = seed {
        st.rng = Rng::new(s);
    }
    if let Some(u) = update {
        st.update = u;
    }
    Ok(())
}

/// Apply a run statement, appending what it prints to `out`. The error is the message without a line.
pub fn apply_run(m: &Model, st: &mut State, s: &RunStmt, out: &mut Vec<String>) -> Result<(), String> {
    match s {
        RunStmt::Hold { thing, value } => {
            let i = find(m, thing)?;
            st.held.insert(i, *value);
        }
        RunStmt::Settle { sweeps, temperature, seed, update } => {
            if *sweeps == 0 {
                return Err("settle takes a whole number of sweeps, 1 or more".into());
            }
            set_opts(st, *temperature, *seed, *update)?;
            st.settle(m, *sweeps);
            out.push(settled_line(*sweeps, m.len(), st.temp));
        }
        RunStmt::Anneal { sweeps, temperature, seed, update } => {
            if *sweeps == 0 {
                return Err("anneal takes a whole number of sweeps, 1 or more".into());
            }
            set_opts(st, *temperature, *seed, *update)?;
            let e = st.anneal(m, *sweeps);
            out.push(annealed_line(*sweeps, e));
        }
        RunStmt::Show => {
            if st.n == 0 {
                return Err("show needs a settle first".into());
            }
            out.extend(show_lines(m, st));
        }
        RunStmt::Best => match &st.best {
            None => return Err("best needs an anneal first".into()),
            Some((b, e)) => out.push(best_line(m, b, *e)),
        },
        RunStmt::Ask { first, terms } => {
            if st.n == 0 {
                return Err("ask needs a settle first".into());
            }
            if st.samples.is_empty() {
                return Err("this settle was too large to keep every sample; ask needs a smaller one".into());
            }
            let q = Question { first: find(m, first)?, terms: terms.iter().map(|(j, o)| find(m, o).map(|k| (*j, k))).collect::<Result<_, _>>()? };
            let (yes, of) = count_yes(&st.samples, &q);
            out.push(ask_line(first, terms, yes, of));
        }
    }
    Ok(())
}

impl Ext for Core {
    fn name(&self) -> &'static str {
        "core"
    }

    fn statements(&self) -> &'static [&'static str] {
        &[
            "model: thing :a, :b, leans: :no, by: 1",
            "model: a.pulls :b, by: 2   /   a.pushes :b, by: 2",
            "run: hold :a, :yes",
            "run: settle 10_000, temperature: 1, seed: 1, update: :metro   (or :gibbs)",
            "run: anneal 4_000, seed: 1, update: :metro   (or :gibbs)",
            "run: show   /   best",
            "run: ask :a, and: :b, or_not: :c",
        ]
    }

    fn model_stmt(&self, m: &mut Model, t: &[Tok], ln: usize, _ctx: &mut Ctx) -> Claim {
        let parsed = parse_model(m, t, ln)?;
        Some(parsed.and_then(|s| apply_model(m, &s).or_else(|e| err(ln, e))))
    }

    fn run_stmt(&self, m: &mut Model, st: &mut State, t: &[Tok], ln: usize, ctx: &mut Ctx) -> Claim {
        let parsed = parse_run(m, st, t, ln)?;
        Some(parsed.and_then(|s| apply_run(m, st, &s, ctx.out).or_else(|e| err(ln, e))))
    }
}

#[cfg(test)]
mod tests {
    use crate::engine::model::{exact_rates, Model, State};
    use crate::words::interp::Interp;

    const WEATHER: &str = "model :weather do
  thing :rain,      leans: :no, by: 1
  thing :sprinkler, leans: :no, by: 0.5
  thing :wet_grass
  rain.pushes :sprinkler, by: 0.5
  rain.pulls  :wet_grass, by: 1.5
  sprinkler.pulls :wet_grass, by: 1
end";

    fn settled(n: usize) -> (Model, State) {
        let mut it = Interp::default();
        it.exec(WEATHER).unwrap();
        let m = it.models["weather"].clone();
        let mut st = State::new(1);
        st.held.insert(m.idx["wet_grass"], 1.0);
        st.settle(&m, n);
        (m, st)
    }

    #[test]
    fn sampled_rates_match_exact_enumeration() {
        let (m, st) = settled(80_000);
        let ex = exact_rates(&m, &st);
        let got = st.rates();
        for i in 0..ex.len() {
            assert!((ex[i] - got[i]).abs() < 0.01, "{} sampled {} exact {}", m.names[i], got[i], ex[i]);
        }
    }

    #[test]
    fn metropolised_gibbs_matches_exact_enumeration_too() {
        let (m, st) = settled(10);
        let ex = exact_rates(&m, &st);
        let mut mt = State::new(1);
        mt.held = st.held.clone();
        mt.update = crate::engine::model::Update::Metro;
        mt.settle(&m, 80_000);
        for (i, (a, b)) in ex.iter().zip(mt.rates()).enumerate() {
            assert!((a - b).abs() < 0.01, "{} metro {} exact {}", m.names[i], b, a);
        }
        // and Gibbs, the default until 2026-10-06, matches too and is a different chain on the same seed, not a
        // renamed copy
        let mut gibbs = State::new(1);
        gibbs.held = st.held.clone();
        gibbs.update = crate::engine::model::Update::Gibbs;
        gibbs.settle(&m, 80_000);
        for (i, (a, b)) in ex.iter().zip(gibbs.rates()).enumerate() {
            assert!((a - b).abs() < 0.01, "{} gibbs {} exact {}", m.names[i], b, a);
        }
        assert_ne!(gibbs.yes, mt.yes);
        // the default is Metropolised Gibbs: settled() names no rule and walks the Metro chain draw for draw
        let (_, default) = settled(80_000);
        assert_eq!(default.yes, mt.yes);
    }

    #[test]
    fn swapped_couplings_are_visible_against_the_exact_answer() {
        // negative control: the same sampler on the model with pulls and pushes swapped disagrees with the true rates
        let (m, st) = settled(40_000);
        let ex = exact_rates(&m, &st);
        let mut bad = m.clone();
        for row in bad.adj.iter_mut() {
            for e in row.iter_mut() {
                e.1 = -e.1;
            }
        }
        let mut st2 = State::new(1);
        st2.held = st.held.clone();
        st2.settle(&bad, 40_000);
        assert!(ex.iter().zip(st2.rates().iter()).any(|(a, b)| (a - b).abs() > 0.1));
    }

    #[test]
    fn a_held_thing_never_moves() {
        let (m, st) = settled(5_000);
        assert!(st.samples.iter().all(|s| s[m.idx["wet_grass"]] > 0.0));
    }

    #[test]
    fn anneal_reaches_the_true_minimum_of_a_frustrated_ring() {
        let src = "model :ring do
  thing :a, :b, :c, :d, :e
  thing :b, leans: :yes, by: 0.2
  a.pushes :b, by: 1
  b.pushes :c, by: 1
  c.pushes :d, by: 1
  d.pushes :e, by: 1
  e.pushes :a, by: 1
  a.pulls :c, by: 0.3
end
run :ring do
  anneal 4_000, seed: 5
  best
end";
        let mut it = Interp::default();
        let out = it.exec(src).unwrap();
        let m = it.models["ring"].clone();
        let n = m.len();
        let true_min = (0u64..(1 << n))
            .map(|b| m.energy(&(0..n).map(|k| if (b >> k) & 1 == 1 { 1.0 } else { -1.0 }).collect::<Vec<_>>()))
            .fold(f64::INFINITY, f64::min);
        assert!(out[0].contains(&format!("{:.3}", true_min)), "{:?} vs {}", out, true_min);
    }

    #[test]
    fn the_rails_style_program_runs_end_to_end() {
        let src = format!(
            "{}\nrun :weather do\n  hold :wet_grass, :yes\n  settle 20_000, temperature: 1, seed: 1\n  show\n  ask :rain, and: :sprinkler\n  ask :rain, or_not: :sprinkler\nend",
            WEATHER
        );
        let out = Interp::default().exec(&src).unwrap();
        assert!(out[0].starts_with("settled: 20000 samples of 3 things"));
        assert!(out.iter().any(|l| l.starts_with("ask :rain, and: :sprinkler: yes")));
        assert!(out.iter().any(|l| l.contains("(held)")));
    }

    #[test]
    fn errors_name_their_line() {
        let cases = [
            ("model :m do\n  thing :a\n  a.pulls :zz, by: 1\nend", "line 3: unknown thing :zz"),
            ("model :m do\n  thing :a\nend\nrun :m do\n  ask :a\nend", "line 5: ask needs a settle first"),
            ("model :m do\n  thing :a\n", "line 1: block :m is never closed"),
            ("run :nope do\nend", "line 1: no model :nope"),
            ("model :m do\n  thing :a, leans: :maybe, by: 1\nend", "line 2: expected :yes or :no"),
            ("model :m do\n  thing :a\nend\nrun :m do\n  dance :a\nend", "line 5: no statement family knows"),
            ("model :m do\n  thing :a, 3\nend", "line 2: unexpected `3` in thing"),
            ("model :m do\n  thing :a\nend\nrun :m do\n  settle 0\nend", "line 5: settle takes a whole number of sweeps, 1 or more"),
            ("model :m do\n  thing :a\nend\nrun :m do\n  settle 2.5\nend", "line 5: settle takes a whole number of sweeps"),
            ("model :m do\n  thing :a\nend\nrun :m do\n  anneal -4\nend", "line 5: anneal takes a whole number of sweeps"),
        ];
        for (src, want) in cases {
            let e = Interp::default().exec(src).err().map(|e| e.0).unwrap_or_default();
            assert!(e.starts_with(want), "{:?} gave {:?}", src, e);
        }
    }

    #[test]
    fn a_large_sparse_chain_settles_without_keeping_every_sample() {
        // 20,000 things in a chain: sparse storage makes this cheap, and the sample budget keeps only counts
        let mut m = Model::default();
        for i in 0..20_000 {
            m.add(&format!("x{}", i));
            if i > 0 {
                m.couple(i - 1, i, 1.0);
            }
        }
        m.h[0] = 5.0;
        let mut st = State::new(3);
        st.settle(&m, 2_000);
        assert!(st.samples.is_empty() && st.n == 2_000);
        assert!(st.rates()[1] > 0.7, "the first link should follow its strongly-leaning neighbour");
    }
}
