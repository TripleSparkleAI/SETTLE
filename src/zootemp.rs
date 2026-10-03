//! ZOOTEMP: the anneal's schedule as a dial, restarts, and the final state judged beside the best-so-far.
//!
//! ```text
//! model :p do
//!   factor :f, number: 10_403, encoding: :columns, penalty: 2
//! end
//! run :p do
//!   anneal_schedule 1_000_000, temperature: 2.9, hot: 10, cold: 0.05, restarts: 100, seed: 1
//!   f.solution     # judged on the calmest arrangement visited by any of the 100 walks
//!   f.final        # judged on the calmest of the 100 end states: what the walks LANDED on
//! end
//! ```
//!
//! `anneal_schedule S, hot: H, cold: C` cools geometrically from H·T to C·T over S sweeps. With H = 10 and
//! C = 0.05 (the defaults) it is the core `anneal` exactly: the same random draws, the same temperatures to the
//! last bit, the same best and last arrangements (tested). `restarts: R` splits S into R independent walks of
//! S/R sweeps each, one after another on the same random stream; the run then keeps the calmest arrangement
//! any walk visited as its best, and the calmest of the R end states as its last.
//!
//! `x.final` judges the last arrangement with the zoo's plain-code checker (the energy is never consulted), so
//! every zoo row can state which it measured: a record of what the walk passed through (`x.solution`), or
//! where it came to rest (`x.final`).
//!
//! The walk keeps its energy up to date flip by flip instead of recomputing it every sweep. A candidate for the
//! best is always re-measured exactly before it is compared, so the chosen best is the one `anneal` chooses.
//! Equations and their plain readings: `runs/zootemp/REPORT_ZOOTEMP.md`.

use crate::ext::{Claim, Ctx, Ext};
use crate::lex::{err, kw, kwargs, num, only, SettleError, Tok};
use crate::model::{Model, State};
use crate::zoo::judge;

pub struct ZooTemp;

/// T(step) = t · hot · ratio^(step / (S - 1)), with ratio = cold / hot. hot 10, ratio 0.005 is `anneal`.
#[derive(Clone, Copy, Debug)]
pub struct Sched {
    pub t: f64,
    pub hot: f64,
    pub ratio: f64,
}

impl Sched {
    /// The core anneal's schedule at base temperature t: from 10t to t/20.
    pub fn zoo(t: f64) -> Self {
        Sched { t, hot: 10.0, ratio: 0.005 }
    }
    /// From hot·t to cold·t.
    pub fn span(t: f64, hot: f64, cold: f64) -> Self {
        Sched { t, hot, ratio: cold / hot }
    }
    pub fn temp(&self, step: usize, sweeps: usize) -> f64 {
        self.t * self.hot * self.ratio.powf(step as f64 / (sweeps.max(2) - 1) as f64)
    }
    pub fn cold(&self) -> f64 {
        self.t * self.hot * self.ratio
    }
}

/// Number of bins the schedule is cut into for the record of when the target was visited.
pub const BINS: usize = 20;

/// One walk (or a set of restarts) and what it passed through.
#[derive(Clone, Debug)]
pub struct Walk {
    pub best: Vec<f64>,
    pub best_e: f64,
    /// The end state (for restarts: the calmest end state of all the walks).
    pub last: Vec<f64>,
    pub last_e: f64,
    /// Energies of every walk's end state, in order.
    pub ends: Vec<f64>,
    /// Sweeps (over all walks) whose arrangement sat at the target energy.
    pub at_target: u64,
    /// First sweep (counted over all walks) at the target.
    pub first_hit: Option<u64>,
    /// Sweeps at the target, by twentieth of the walk's own schedule (summed over walks).
    pub hit_bins: [u64; BINS],
    /// Number of walks that visited the target at least once.
    pub walks_hit: usize,
}

fn near(e: f64, target: Option<f64>) -> bool {
    match target {
        Some(t) => (e - t).abs() <= 1e-6 * t.abs().max(1.0),
        None => false,
    }
}

/// One anneal along `sched`, mirroring `State::anneal` draw for draw. Sets `st.best` and `st.last` exactly as
/// `State::anneal` would. `target` (an energy) only feeds the visit record; it never steers the walk.
pub fn anneal_walk(m: &Model, st: &mut State, sched: &Sched, sweeps: usize, target: Option<f64>) -> Walk {
    let (mut s, mut free) = st.start(m);
    let mut e = m.energy(&s);
    let mut best = (s.clone(), e);
    let mut w = Walk { best: Vec::new(), best_e: 0.0, last: Vec::new(), last_e: 0.0, ends: Vec::new(), at_target: 0, first_hit: None, hit_bins: [0; BINS], walks_hit: 0 };
    let mut hit = false;
    // incremental energy drifts by rounding; a candidate within `tol` of the best is re-measured exactly
    let bound: f64 = m.h.iter().map(|x| x.abs()).sum::<f64>() + m.adj.iter().flatten().map(|e| e.1.abs()).sum::<f64>();
    let tol = 1e-9 * (1.0 + bound);
    for step in 0..sweeps {
        let beta = 1.0 / sched.temp(step, sweeps);
        // the same shuffle and the same updates as State::sweep, with the energy kept flip by flip
        for k in (1..free.len()).rev() {
            let r = st.rng.below(k + 1);
            free.swap(k, r);
        }
        for &i in free.iter() {
            let x = m.input(i, &s);
            let v = if (beta * x).tanh() > st.rng.signed() { 1.0 } else { -1.0 };
            if v != s[i] {
                e -= (v - s[i]) * x;
                s[i] = v;
            }
        }
        if step % 256 == 255 {
            e = m.energy(&s);
        }
        if e < best.1 + tol {
            let exact = m.energy(&s);
            e = exact;
            if exact < best.1 {
                best = (s.clone(), exact);
            }
        }
        if near(e, target) {
            w.at_target += 1;
            if w.first_hit.is_none() {
                w.first_hit = Some(step as u64);
            }
            w.hit_bins[(step * BINS / sweeps.max(1)).min(BINS - 1)] += 1;
            hit = true;
        }
    }
    let last_e = m.energy(&s);
    st.last = s.clone();
    st.best = Some(best.clone());
    w.best = best.0;
    w.best_e = best.1;
    w.last = s;
    w.last_e = last_e;
    w.ends.push(last_e);
    w.walks_hit = hit as usize;
    w
}

/// `r` walks of `total / r` sweeps each on one random stream. Best = calmest arrangement any walk visited;
/// last = calmest end state (the first such walk on ties). `st.best` and `st.last` are set to those.
pub fn restarts(m: &Model, st: &mut State, sched: &Sched, total: usize, r: usize, target: Option<f64>) -> Walk {
    let r = r.max(1);
    let each = (total / r).max(1);
    let mut acc: Option<Walk> = None;
    for k in 0..r {
        let w = anneal_walk(m, st, sched, each, target);
        acc = Some(match acc {
            None => w,
            Some(mut a) => {
                let off = (k * each) as u64;
                if w.best_e < a.best_e {
                    a.best = w.best;
                    a.best_e = w.best_e;
                }
                if w.last_e < a.last_e {
                    a.last = w.last;
                    a.last_e = w.last_e;
                }
                a.ends.push(w.ends[0]);
                a.at_target += w.at_target;
                if a.first_hit.is_none() {
                    a.first_hit = w.first_hit.map(|f| f + off);
                }
                for b in 0..BINS {
                    a.hit_bins[b] += w.hit_bins[b];
                }
                a.walks_hit += w.walks_hit;
                a
            }
        });
    }
    let a = acc.unwrap();
    st.best = Some((a.best.clone(), a.best_e));
    st.last = a.last.clone();
    a
}

/// Largest lean or pull in size: the zoo's temperature rule is T = this / 10.
pub fn largest(m: &Model) -> f64 {
    (0..m.len()).map(|i| m.h[i].abs().max(m.adj[i].iter().map(|e| e.1.abs()).fold(0.0, f64::max))).fold(0.0, f64::max)
}

fn schedule_stmt(m: &mut Model, st: &mut State, sweeps: usize, rest: &[Tok], ln: usize, ctx: &mut Ctx) -> Result<(), SettleError> {
    let rest = if rest.first() == Some(&Tok::Comma) { &rest[1..] } else { rest };
    let kv = kwargs(rest, ln)?;
    only(&kv, &["temperature", "seed", "hot", "cold", "restarts"], "anneal_schedule", ln)?;
    if let Some(v) = kw(&kv, "temperature") {
        st.temp = num(v, ln)?;
        if st.temp <= 0.0 {
            return err(ln, "temperature must be above zero");
        }
    }
    if let Some(v) = kw(&kv, "seed") {
        st.rng = crate::rng::Rng::new(num(v, ln)? as u64);
    }
    let get = |k: &str, d: f64| -> Result<f64, SettleError> { kw(&kv, k).map(|v| num(v, ln)).unwrap_or(Ok(d)) };
    let hot = get("hot", 10.0)?;
    let cold = get("cold", 0.05)?;
    let r = get("restarts", 1.0)?;
    if hot <= 0.0 || cold <= 0.0 {
        return err(ln, "hot and cold must be above zero");
    }
    if r < 1.0 || r.fract() != 0.0 || r as usize > sweeps.max(1) {
        return err(ln, "restarts is a whole number from 1 to the number of sweeps");
    }
    let sched = if hot == 10.0 && cold == 0.05 { Sched::zoo(st.temp) } else { Sched::span(st.temp, hot, cold) };
    let w = restarts(m, st, &sched, sweeps, r as usize, None);
    ctx.say(format!(
        "annealed on a schedule: {} sweeps in {} walk(s), temperature {} to {}; calmest visited {:.3}, calmest end state {:.3}",
        sweeps,
        r as usize,
        sched.temp(0, 2),
        sched.cold(),
        w.best_e,
        w.last_e
    ));
    Ok(())
}

impl Ext for ZooTemp {
    fn name(&self) -> &'static str {
        "zootemp"
    }

    fn statements(&self) -> &'static [&'static str] {
        &[
            "run: anneal_schedule 100_000, temperature: 2.9, hot: 10, cold: 0.05, restarts: 10, seed: 1",
            "run: f.final   (judge the end state the walk came to rest in, beside f.solution's best-so-far)",
        ]
    }

    fn run_stmt(&self, m: &mut Model, st: &mut State, t: &[Tok], ln: usize, ctx: &mut Ctx) -> Claim {
        match t {
            [Tok::Ident(k), Tok::Num(nv), rest @ ..] if k == "anneal_schedule" => Some(schedule_stmt(m, st, *nv as usize, rest, ln, ctx)),
            [Tok::Ident(name), Tok::Dot, Tok::Ident(v)] if v == "final" && m.notes.contains_key(&format!("zoo:{}", name)) => Some(if st.last.len() != m.len() {
                err(ln, "final needs an anneal first")
            } else {
                let (_, lines) = judge(m, name, &st.last).expect("a declared puzzle always judges");
                ctx.say(format!("  (:{} judged on the end state, not on the best visited)", name));
                for l in lines {
                    ctx.say(l);
                }
                Ok(())
            }),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interp::Interp;
    use crate::zoo::{column_layout, columns_assign};

    fn model_of(src: &str) -> Model {
        let mut it = Interp::default();
        it.exec(src).unwrap_or_else(|e| panic!("{}", e));
        it.models.values().next().unwrap().clone()
    }

    fn run(src: &str) -> Result<String, String> {
        let mut it = Interp::default();
        it.exec(src).map(|o| o.join("\n")).map_err(|e| e.0)
    }

    #[test]
    fn default_schedule_is_the_core_anneal_draw_for_draw() {
        for (src, t, sweeps) in [
            ("model :p do\n  factor :f, number: 899, encoding: :columns\nend", 0.65, 3_000usize),
            ("model :p do\n  factor :f, number: 143\nend", 1158.4, 2_000),
            ("model :p do\n  colouring :g, colours: 3, edges: \"a-b b-c c-a c-d d-e e-a b-e\"\nend", 1.0, 500),
            ("model :p do\n  sudoku :s, size: 4, given: \"1... .4.. ..4. ...1\"\nend", 1.0, 700),
        ] {
            let m = model_of(src);
            for seed in [1u64, 7, 1_003] {
                let mut a = State::new(seed);
                a.temp = t;
                let ea = a.anneal(&m, sweeps);
                let mut b = State::new(seed);
                let w = anneal_walk(&m, &mut b, &Sched::zoo(t), sweeps, None);
                assert_eq!(a.best.as_ref().unwrap().0, w.best, "same best arrangement");
                assert_eq!(ea, w.best_e, "same best energy to the bit");
                assert_eq!(a.last, w.last, "same last arrangement");
                assert_eq!(b.best, a.best);
                assert_eq!(b.last, a.last);
                assert!((w.last_e - m.energy(&a.last)).abs() < 1e-9);
            }
        }
    }

    #[test]
    fn one_restart_is_one_walk_and_restarts_keep_the_calmest() {
        let m = model_of("model :p do\n  factor :f, number: 323, encoding: :columns\nend");
        let s = Sched::zoo(0.7);
        let mut a = State::new(5);
        let w1 = anneal_walk(&m, &mut a, &s, 4_000, None);
        let mut b = State::new(5);
        let w2 = restarts(&m, &mut b, &s, 4_000, 1, None);
        assert_eq!(w1.best, w2.best);
        assert_eq!(w1.last, w2.last);
        let mut c = State::new(5);
        let w = restarts(&m, &mut c, &s, 4_000, 8, None);
        assert_eq!(w.ends.len(), 8);
        let calm = w.ends.iter().cloned().fold(f64::INFINITY, f64::min);
        assert_eq!(w.last_e, calm);
        assert!(w.best_e <= w.last_e + 1e-9);
        assert!((m.energy(&c.last) - calm).abs() < 1e-9);
        // the first of the eight walks is the same walk a single 500-sweep anneal makes
        let mut d = State::new(5);
        let w0 = anneal_walk(&m, &mut d, &s, 500, None);
        assert_eq!(w0.ends[0], w.ends[0]);
    }

    #[test]
    fn the_target_record_counts_visits_to_the_answer_and_nothing_else() {
        let n = 899u64;
        let m = model_of(&format!("model :p do\n  factor :f, number: {}, encoding: :columns\nend", n));
        let lay = column_layout(n);
        let y = columns_assign(n, &lay, 29, 31).or_else(|| columns_assign(n, &lay, 31, 29)).unwrap();
        let start = m.notes["zoo:f"].0[0] as usize;
        let mut sol = vec![-1.0; m.len()];
        for (k, &b) in y.iter().enumerate() {
            sol[start + k] = if b { 1.0 } else { -1.0 };
        }
        let ground = m.energy(&sol);
        let mut st = State::new(1_001);
        let w = anneal_walk(&m, &mut st, &Sched::zoo(0.65), 20_000, Some(ground));
        assert_eq!(w.hit_bins.iter().sum::<u64>(), w.at_target);
        if w.first_hit.is_some() {
            assert!(w.best_e <= ground + 1e-6, "a visited ground is the best");
            assert!(judge(&m, "f", &w.best).unwrap().0);
        }
        let final_ok = judge(&m, "f", &w.last).unwrap().0;
        assert_eq!(final_ok, (w.last_e - ground).abs() < 1e-6, "a valid end state sits exactly at the ground energy");
        // a prime never reaches zero: no visit is ever recorded at a made-up target of the ground energy's value
        let p = model_of("model :p do\n  factor :f, number: 887, encoding: :columns\nend");
        let mut sp = State::new(1_001);
        let wp = anneal_walk(&p, &mut sp, &Sched::zoo(0.4), 20_000, Some(ground));
        assert!(!judge(&p, "f", &wp.best).unwrap().0 && !judge(&p, "f", &wp.last).unwrap().0);
    }

    #[test]
    fn a_colder_end_is_colder_and_the_span_reaches_both_ends() {
        let s = Sched::span(2.0, 10.0, 0.01);
        assert!((s.temp(0, 100) - 20.0).abs() < 1e-12);
        assert!((s.temp(99, 100) - 0.02).abs() < 1e-12);
        assert!((s.cold() - 0.02).abs() < 1e-12);
        let z = Sched::zoo(2.0);
        assert!((z.cold() - 0.1).abs() < 1e-12);
    }

    #[test]
    fn the_language_runs_schedules_restarts_and_final() {
        let out = run("model :p do\n  factor :f, number: 143, encoding: :columns\nend\nrun :p do\n  anneal_schedule 20_000, temperature: 0.55, restarts: 4, seed: 3\n  f.solution\n  f.final\nend").unwrap();
        assert!(out.contains("4 walk(s)"), "{}", out);
        assert!(out.contains("judged on the end state"), "{}", out);
        assert!(out.matches("VALID").count() >= 2, "{}", out);
        // the default statement equals the core anneal
        let a = run("model :p do\n  factor :f, number: 323, encoding: :columns\nend\nrun :p do\n  anneal 3_000, temperature: 0.7, seed: 9\n  f.solution\nend").unwrap();
        let b = run("model :p do\n  factor :f, number: 323, encoding: :columns\nend\nrun :p do\n  anneal_schedule 3_000, temperature: 0.7, seed: 9\n  f.solution\nend").unwrap();
        assert_eq!(a.lines().last(), b.lines().last());
        let cases = [
            ("model :p do\n  factor :f, number: 15\nend\nrun :p do\n  f.final\nend", "line 5: final needs an anneal first"),
            ("model :p do\n  factor :f, number: 15\nend\nrun :p do\n  anneal_schedule 100, restarts: 0\nend", "line 5: restarts is a whole number"),
            ("model :p do\n  factor :f, number: 15\nend\nrun :p do\n  anneal_schedule 100, cold: 0\nend", "line 5: hot and cold must be above zero"),
            ("model :p do\n  factor :f, number: 15\nend\nrun :p do\n  anneal_schedule 100, warm: 1\nend", "line 5:"),
        ];
        for (src, want) in cases {
            let e = run(src).err().unwrap_or_default();
            assert!(e.starts_with(want), "{:?} gave {:?}", src, e);
        }
    }
}
