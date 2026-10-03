//! The core statements: thing, pulls, pushes (in a model); hold, settle, anneal, show, best, ask (in a run).

use crate::ext::{Claim, Ctx, Ext};
use crate::lex::{err, kw, kwargs, num, only, yes_no, SettleError, Tok};
use crate::model::{Model, State};
use crate::rng::Rng;

pub struct Core;

fn thing(m: &mut Model, rest: &[Tok], ln: usize) -> Result<(), SettleError> {
    let mut i = 0;
    let mut declared = Vec::new();
    while i < rest.len() {
        match &rest[i] {
            Tok::Sym(s) => {
                if ["yes", "no"].contains(&s.as_str()) {
                    return err(ln, format!(":{} cannot be a thing name", s));
                }
                declared.push(m.add(s));
                i += 1;
            }
            Tok::Comma => i += 1,
            Tok::Label(_) => break,
            other => return err(ln, format!("unexpected {:?} in thing", other)),
        }
    }
    let kv = kwargs(&rest[i..], ln)?;
    only(&kv, &["leans", "by"], "thing", ln)?;
    let lean = kw(&kv, "leans").map(|v| yes_no(v, ln)).transpose()?;
    let by = kw(&kv, "by").map(|v| num(v, ln)).transpose()?;
    match (lean, by) {
        (Some(l), Some(b)) => {
            for &d in &declared {
                m.h[d] += l * b;
            }
            Ok(())
        }
        (None, None) => Ok(()),
        _ => err(ln, "use `leans:` and `by:` together"),
    }
}

fn pull(m: &mut Model, a: &str, verb: &str, b: &str, rest: &[Tok], ln: usize) -> Result<(), SettleError> {
    let (i, k) = (m.need(a, ln)?, m.need(b, ln)?);
    if i == k {
        return err(ln, "a thing cannot pull itself");
    }
    let kv = kwargs(rest, ln)?;
    only(&kv, &["by"], verb, ln)?;
    let by = match kw(&kv, "by") {
        Some(v) => num(v, ln)?,
        None => return err(ln, format!("{} needs `by:`", verb)),
    };
    m.couple(i, k, if verb == "pulls" { by } else { -by });
    Ok(())
}

/// `temperature:` and `seed:` shared by settle and anneal.
fn run_opts(st: &mut State, rest: &[Tok], what: &str, ln: usize) -> Result<(), SettleError> {
    let kv = kwargs(rest, ln)?;
    only(&kv, &["temperature", "seed"], what, ln)?;
    if let Some(v) = kw(&kv, "temperature") {
        st.temp = num(v, ln)?;
        if st.temp <= 0.0 {
            return err(ln, "temperature must be above zero");
        }
    }
    if let Some(v) = kw(&kv, "seed") {
        st.rng = Rng::new(num(v, ln)? as u64);
    }
    Ok(())
}

fn ask(m: &Model, st: &State, first: &str, rest: &[Tok], ln: usize, ctx: &mut Ctx) -> Result<(), SettleError> {
    if st.n == 0 {
        return err(ln, "ask needs a settle first");
    }
    if st.samples.is_empty() {
        return err(ln, "this settle was too large to keep every sample; ask needs a smaller one");
    }
    let i0 = m.need(first, ln)?;
    let mut truth: Vec<bool> = st.samples.iter().map(|s| s[i0] > 0.0).collect();
    let mut desc = format!(":{}", first);
    for (k, v) in kwargs(rest, ln)? {
        let (other, oname) = match &v {
            Tok::Sym(s) => (m.need(s, ln)?, s.clone()),
            _ => return err(ln, "ask terms are symbols, like `and: :sprinkler`"),
        };
        for (x, s) in truth.iter_mut().zip(st.samples.iter()) {
            let y = s[other] > 0.0;
            *x = match k.as_str() {
                "and" => *x && y,
                "or" => *x || y,
                "and_not" => *x && !y,
                "or_not" => *x || !y,
                _ => return err(ln, format!("ask takes and: / or: / and_not: / or_not:, not `{}:`", k)),
            };
        }
        desc.push_str(&format!(", {}: :{}", k, oname));
    }
    let p = truth.iter().filter(|&&x| x).count() as f64 / truth.len() as f64;
    ctx.say(format!("ask {}: yes {:.1}% of {} samples", desc, 100.0 * p, truth.len()));
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
            "run: settle 10_000, temperature: 1, seed: 1",
            "run: anneal 4_000, seed: 1",
            "run: show   /   best",
            "run: ask :a, and: :b, or_not: :c",
        ]
    }

    fn model_stmt(&self, m: &mut Model, t: &[Tok], ln: usize, _ctx: &mut Ctx) -> Claim {
        match t {
            [Tok::Ident(k), rest @ ..] if k == "thing" => Some(thing(m, rest, ln)),
            [Tok::Ident(a), Tok::Dot, Tok::Ident(verb), Tok::Sym(b), rest @ ..] if verb == "pulls" || verb == "pushes" => {
                Some(pull(m, a, verb, b, rest, ln))
            }
            _ => None,
        }
    }

    fn run_stmt(&self, m: &mut Model, st: &mut State, t: &[Tok], ln: usize, ctx: &mut Ctx) -> Claim {
        let m = &*m;
        Some(match t {
            [Tok::Ident(k), Tok::Sym(a), Tok::Comma, v] if k == "hold" => m.need(a, ln).and_then(|i| {
                st.held.insert(i, yes_no(v, ln)?);
                Ok(())
            }),
            [Tok::Ident(k), Tok::Num(nv), rest @ ..] if k == "settle" => run_opts(st, rest, "settle", ln).map(|_| {
                st.settle(m, *nv as usize);
                ctx.say(format!("settled: {} samples of {} things at temperature {}", *nv as usize, m.len(), st.temp));
            }),
            [Tok::Ident(k), Tok::Num(nv), rest @ ..] if k == "anneal" => run_opts(st, rest, "anneal", ln).map(|_| {
                let e = st.anneal(m, *nv as usize);
                ctx.say(format!("annealed: {} sweeps, calmest energy found {:.3}", *nv as usize, e));
            }),
            [Tok::Ident(k)] if k == "show" => {
                if st.n == 0 {
                    err(ln, "show needs a settle first")
                } else {
                    for (i, (nm, p)) in m.names.iter().zip(st.rates()).enumerate() {
                        let tag = if st.held.contains_key(&i) { "  (held)" } else { "" };
                        ctx.say(format!("  {:<14} {} {:.1}%{}", nm, "#".repeat((p * 30.0).round() as usize), 100.0 * p, tag));
                    }
                    Ok(())
                }
            }
            [Tok::Ident(k)] if k == "best" => match &st.best {
                None => err(ln, "best needs an anneal first"),
                Some((b, e)) => {
                    let parts: Vec<String> =
                        m.names.iter().enumerate().map(|(i, nm)| format!("{} {}", nm, if b[i] > 0.0 { "yes" } else { "no" })).collect();
                    ctx.say(format!("best (energy {:.3}): {}", e, parts.join(", ")));
                    Ok(())
                }
            },
            // a model with a `loss` (the descend family) asks about its parameters when :first is not a thing
            [Tok::Ident(k), Tok::Sym(first), rest @ ..] if k == "ask" && !(m.notes.contains_key("descend:loss") && !m.idx.contains_key(first)) => {
                ask(m, st, first, rest, ln, ctx)
            }
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::interp::Interp;
    use crate::model::{exact_rates, Model, State};

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
