//! LEARN: fit leans and pulls from examples, then classify by holding the inputs and settling.
//!
//! ```text
//! model :glyphs do
//!   examples :train, "train.txt"          # first line names the things, then one row of 1/0 per example
//!   examples :xor, over: "a b c", rows: "000 011 101 110"   # or inline rows
//!   hidden 16                             # optional: 16 hidden things, making a restricted machine
//! end
//! run :glyphs do
//!   learn :train, rounds: 200, rate: 0.05, method: :contrastive, sweeps: 1, batch: 50, decay: 0.0001, seed: 1
//!   classify :test, labels: "d*", sweeps: 100, seed: 2    # hold every input, settle, read the label things
//!   shuffle :train, labels: "d*", seed: 3                 # scramble which label goes with which row
//! end
//! ```
//!
//! A thing named in an examples set but not yet declared is declared. Things are yes/no (+1/-1); a lean is a
//! field and a pull a coupling, so the machine's chance of an arrangement is exp(-energy / temperature).
//! Learning raises a pull when the examples agree on that pair more often than the settled machine does,
//! and lowers it when less often (the Boltzmann machine rule). Four ways to estimate "the settled machine":
//!
//! - `:exact` enumerates every arrangement of the visible things (at most 20 of them; hidden things are summed
//!   out in closed form), so the gradient has no sampling noise.
//! - `:contrastive` starts a short settle (`sweeps:` of them, default 1) at each example (contrastive divergence).
//! - `:persistent` keeps `batch:` settles running across the whole fit instead of restarting at the examples.
//! - `:pseudo` maximises the chance of each thing given all the others (pseudo-likelihood); no hidden things.
//!
//! With `hidden N` the learnable pulls are visible-to-hidden only (a restricted Boltzmann machine); without
//! it they are every pair of visible things (a fully visible Boltzmann machine). The rate falls linearly to a
//! tenth of `rate:` over the rounds. Things outside the examples set and the hidden things are left alone and
//! ignored while learning. A machine with hidden things whose pulls are all zero starts from small random pulls,
//! because at zero every hidden thing looks alike and the gradient cannot separate them.

use crate::ext::{Claim, Ctx, Ext};
use crate::lex::{err, kw, kwargs, num, only, text, SettleError, Tok};
use crate::model::{Model, State};
use crate::rng::Rng;
use std::collections::HashMap;
use std::time::Instant;

pub struct Learn;

/// A named set of examples: the things it covers, and rows of +1/-1 in that order.
#[derive(Clone, Debug, PartialEq)]
pub struct Examples {
    pub names: Vec<String>,
    pub rows: Vec<Vec<f64>>,
}

fn ex_key(name: &str) -> String {
    format!("examples:{}", name)
}

pub fn load_examples(m: &Model, name: &str, ln: usize) -> Result<Examples, SettleError> {
    let (vals, names) = match m.notes.get(&ex_key(name)) {
        Some(x) => x,
        None => return err(ln, format!("no examples :{} (declare them with: examples :{}, \"file.txt\")", name, name)),
    };
    let rows = if names.is_empty() { Vec::new() } else { vals.chunks(names.len()).map(|c| c.to_vec()).collect() };
    Ok(Examples { names: names.clone(), rows })
}

fn keep_examples(m: &mut Model, name: &str, ex: &Examples) {
    m.notes.insert(ex_key(name), (ex.rows.concat(), ex.names.clone()));
}

fn cell(c: char, ln: usize) -> Result<f64, SettleError> {
    match c {
        '1' | '+' | 'y' | 'Y' => Ok(1.0),
        '0' | '-' | 'n' | 'N' => Ok(-1.0),
        _ => err(ln, format!("'{}' is not a yes/no cell (use 1/0, +/-, y/n)", c)),
    }
}

fn parse_row(s: &str, n: usize, ln: usize) -> Result<Vec<f64>, SettleError> {
    let row: Vec<f64> = s.chars().filter(|c| !c.is_whitespace() && *c != ',').map(|c| cell(c, ln)).collect::<Result<_, _>>()?;
    if row.len() != n {
        return err(ln, format!("a row has {} cells but the examples cover {} things", row.len(), n));
    }
    Ok(row)
}

fn words(s: &str) -> Vec<String> {
    s.split(|c: char| c.is_whitespace() || c == ',').filter(|w| !w.is_empty()).map(|w| w.to_string()).collect()
}

/// A file: `#` comments and blank lines skipped, the first line names the things, every later line is one row.
pub fn parse_file(src: &str, ln: usize) -> Result<Examples, SettleError> {
    let mut lines = src.lines().map(|l| l.split('#').next().unwrap_or("").trim()).filter(|l| !l.is_empty());
    let names = match lines.next() {
        Some(l) => words(l),
        None => return err(ln, "the examples file is empty"),
    };
    let rows = lines.map(|l| parse_row(l, names.len(), ln)).collect::<Result<_, _>>()?;
    Ok(Examples { names, rows })
}

fn declare_things(m: &mut Model, names: &[String], ln: usize) -> Result<(), SettleError> {
    for n in names {
        if n == "yes" || n == "no" {
            return err(ln, format!(":{} cannot be a thing name", n));
        }
        m.add(n);
    }
    Ok(())
}

fn examples_stmt(m: &mut Model, name: &str, rest: &[Tok], ln: usize, ctx: &mut Ctx) -> Result<(), SettleError> {
    let rest = if rest.first() == Some(&Tok::Comma) { &rest[1..] } else { rest };
    let (ex, what) = if let [Tok::Str(path), more @ ..] = rest {
        if !more.is_empty() {
            return err(ln, "examples from a file take nothing after the file name");
        }
        if m.notes.contains_key(&ex_key(name)) {
            return err(ln, format!("examples :{} already exist", name));
        }
        let p = ctx.path(path);
        let src = std::fs::read_to_string(&p).or_else(|e| err(ln, format!("cannot read {}: {}", p.display(), e)))?;
        (parse_file(&src, ln)?, format!("from {}", path))
    } else {
        let kv = kwargs(rest, ln)?;
        only(&kv, &["over", "rows"], "examples", ln)?;
        let rows_src = kw(&kv, "rows").map(|v| text(v, ln)).transpose()?.unwrap_or_default();
        let mut ex = match (kw(&kv, "over"), m.notes.contains_key(&ex_key(name))) {
            (Some(v), false) => Examples { names: words(&text(v, ln)?), rows: Vec::new() },
            (Some(_), true) => return err(ln, format!("examples :{} already exist; add rows with `rows:` alone", name)),
            (None, true) => load_examples(m, name, ln)?,
            (None, false) => return err(ln, format!("new examples :{} need `over:` naming their things", name)),
        };
        if ex.names.is_empty() {
            return err(ln, "`over:` names no things");
        }
        let before = ex.rows.len();
        for r in words(&rows_src) {
            ex.rows.push(parse_row(&r, ex.names.len(), ln)?);
        }
        (ex.clone(), format!("inline: {} rows added", ex.rows.len() - before))
    };
    let mut seen = std::collections::HashSet::new();
    if let Some(d) = ex.names.iter().find(|n| !seen.insert(n.as_str())) {
        return err(ln, format!(":{} is named twice in examples :{}", d, name));
    }
    declare_things(m, &ex.names, ln)?;
    ctx.say(format!("examples :{} {}, {} in all, over {} things", name, what, ex.rows.len(), ex.names.len()));
    keep_examples(m, name, &ex);
    Ok(())
}

fn hidden_of(m: &Model) -> Vec<usize> {
    match m.notes.get("learn:hidden") {
        Some((v, _)) => (v[0] as usize..(v[0] + v[1]) as usize).collect(),
        None => Vec::new(),
    }
}

fn hidden_stmt(m: &mut Model, count: f64, ln: usize) -> Result<(), SettleError> {
    if m.notes.contains_key("learn:hidden") {
        return err(ln, "hidden things are already declared in this model");
    }
    if count < 1.0 || count > 4096.0 || count.fract() != 0.0 {
        return err(ln, "hidden takes a whole number from 1 to 4096");
    }
    let start = m.len();
    for i in 0..count as usize {
        m.add(&format!("hidden_{}", i));
    }
    m.notes.insert("learn:hidden".into(), (vec![start as f64, count], Vec::new()));
    Ok(())
}

/// ln(2 cosh y), without overflow.
fn ln2cosh(y: f64) -> f64 {
    y.abs() + (-2.0 * y.abs()).exp().ln_1p()
}

fn log_sum_exp(xs: &[f64]) -> f64 {
    let mx = xs.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    mx + xs.iter().map(|x| (x - mx).exp()).sum::<f64>().ln()
}

/// The part of a model being learned, as dense arrays: visible things first, then hidden things.
#[derive(Clone, Debug)]
pub struct Net {
    pub units: Vec<usize>,
    pub nv: usize,
    pub h: Vec<f64>,
    /// n x n, symmetric, zero diagonal.
    pub w: Vec<f64>,
    /// The learnable pairs (a < b).
    pub pairs: Vec<(usize, usize)>,
}

impl Net {
    pub fn n(&self) -> usize {
        self.units.len()
    }

    pub fn from_model(m: &Model, vis: &[usize], hid: &[usize], ln: usize) -> Result<Net, SettleError> {
        let units: Vec<usize> = vis.iter().chain(hid).copied().collect();
        let n = units.len();
        let pos: HashMap<usize, usize> = units.iter().enumerate().map(|(a, &u)| (u, a)).collect();
        let mut w = vec![0.0; n * n];
        for (a, &u) in units.iter().enumerate() {
            for &(k, x) in &m.adj[u] {
                if let Some(&b) = pos.get(&k) {
                    w[a * n + b] = x;
                }
            }
        }
        let nv = vis.len();
        for a in nv..n {
            for b in nv..n {
                if w[a * n + b] != 0.0 {
                    return err(ln, "hidden things may not pull each other (this is a restricted machine)");
                }
            }
        }
        let pairs = if hid.is_empty() {
            (0..nv).flat_map(|a| ((a + 1)..nv).map(move |b| (a, b))).collect()
        } else {
            (0..nv).flat_map(|a| (nv..n).map(move |b| (a, b))).collect()
        };
        Ok(Net { units: units.clone(), nv, h: units.iter().map(|&u| m.h[u]).collect(), w, pairs })
    }

    /// Put the learned leans and pulls back into the model.
    pub fn write_back(&self, m: &mut Model) {
        let n = self.n();
        for (a, &u) in self.units.iter().enumerate() {
            m.h[u] = self.h[a];
        }
        for &(a, b) in &self.pairs {
            let (ua, ub) = (self.units[a], self.units[b]);
            let d = self.w[a * n + b] - m.coupling(ua, ub);
            if d != 0.0 {
                m.couple(ua, ub, d);
            }
        }
    }

    fn input(&self, a: usize, s: &[f64]) -> f64 {
        let n = self.n();
        self.h[a] + self.w[a * n..a * n + n].iter().zip(s).map(|(w, x)| w * x).sum::<f64>()
    }

    fn input_from_visible(&self, a: usize, v: &[f64]) -> f64 {
        let n = self.n();
        self.h[a] + self.w[a * n..a * n + self.nv].iter().zip(v).map(|(w, x)| w * x).sum::<f64>()
    }

    /// -beta times the free energy of a visible arrangement, hidden things summed out: log of its unnormalised chance.
    pub fn neg_beta_free(&self, v: &[f64], beta: f64) -> f64 {
        let n = self.n();
        let mut e = 0.0;
        for a in 0..self.nv {
            e += self.h[a] * v[a];
            for b in (a + 1)..self.nv {
                e += self.w[a * n + b] * v[a] * v[b];
            }
        }
        beta * e + (self.nv..n).map(|k| ln2cosh(beta * self.input_from_visible(k, v))).sum::<f64>()
    }

    /// Visible values followed by the hidden things' expected values given them.
    fn with_hidden_means(&self, v: &[f64], beta: f64) -> Vec<f64> {
        let mut s = v.to_vec();
        for k in self.nv..self.n() {
            s.push((beta * self.input_from_visible(k, v)).tanh());
        }
        s
    }

    fn flip(&self, a: usize, s: &mut [f64], beta: f64, rng: &mut Rng) {
        s[a] = if (beta * self.input(a, s)).tanh() > rng.signed() { 1.0 } else { -1.0 };
    }

    /// `k` rounds of: every visible thing in a fresh random order, then every hidden thing.
    fn gibbs(&self, s: &mut [f64], k: usize, beta: f64, rng: &mut Rng, order: &mut [usize]) {
        for _ in 0..k {
            for j in (1..order.len()).rev() {
                let r = rng.below(j + 1);
                order.swap(j, r);
            }
            for &a in order.iter() {
                self.flip(a, s, beta, rng);
            }
            for a in self.nv..self.n() {
                self.flip(a, s, beta, rng);
            }
        }
    }

    fn visible_arrangements(&self) -> impl Iterator<Item = Vec<f64>> + '_ {
        (0u64..(1u64 << self.nv)).map(move |bits| (0..self.nv).map(|k| if (bits >> k) & 1 == 1 { 1.0 } else { -1.0 }).collect())
    }

    /// Exact expected leans and pair products under the machine, by enumerating the visible things.
    pub fn exact_moments(&self, beta: f64) -> (Vec<f64>, Vec<f64>) {
        assert!(self.nv <= 20, "exact moments are for at most 20 visible things");
        let all: Vec<Vec<f64>> = self.visible_arrangements().collect();
        let lw: Vec<f64> = all.iter().map(|v| self.neg_beta_free(v, beta)).collect();
        let lz = log_sum_exp(&lw);
        let (mut mh, mut mw) = (vec![0.0; self.n()], vec![0.0; self.pairs.len()]);
        for (v, l) in all.iter().zip(&lw) {
            let p = (l - lz).exp();
            let s = self.with_hidden_means(v, beta);
            self.add_stats(&s, p, &mut mh, &mut mw);
        }
        (mh, mw)
    }

    /// Exact average log-chance of the rows (natural log), at most 20 visible things.
    pub fn exact_log_likelihood(&self, rows: &[Vec<f64>], beta: f64) -> f64 {
        let lw: Vec<f64> = self.visible_arrangements().map(|v| self.neg_beta_free(&v, beta)).collect();
        let lz = log_sum_exp(&lw);
        rows.iter().map(|v| self.neg_beta_free(v, beta) - lz).sum::<f64>() / rows.len().max(1) as f64
    }

    /// Average over rows of the sum over things of log P(thing | all the others). No hidden things.
    pub fn pseudo_log_likelihood(&self, rows: &[Vec<f64>], beta: f64) -> f64 {
        let mut t = 0.0;
        for v in rows {
            for a in 0..self.nv {
                let x = beta * self.input(a, v);
                t += v[a] * x - ln2cosh(x);
            }
        }
        t / rows.len().max(1) as f64
    }

    fn add_stats(&self, s: &[f64], wgt: f64, gh: &mut [f64], gw: &mut [f64]) {
        for (g, x) in gh.iter_mut().zip(s) {
            *g += wgt * x;
        }
        for (g, &(a, b)) in gw.iter_mut().zip(&self.pairs) {
            *g += wgt * s[a] * s[b];
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Method {
    Exact,
    Contrastive,
    Persistent,
    Pseudo,
}

#[derive(Clone, Debug)]
pub struct Opts {
    pub method: Method,
    pub rounds: usize,
    pub rate: f64,
    pub batch: usize,
    pub sweeps: usize,
    pub decay: f64,
    pub beta: f64,
}

/// Fit the net's leans and learnable pulls to the rows (visible values in net order).
pub fn train(net: &mut Net, rows: &[Vec<f64>], o: &Opts, rng: &mut Rng) {
    let n = net.n();
    let nv = net.nv;
    let batch = if o.method == Method::Exact { rows.len() } else { o.batch.clamp(1, rows.len().max(1)) };
    let mut order: Vec<usize> = (0..rows.len()).collect();
    let mut vis_order: Vec<usize> = (0..nv).collect();
    let mut chains: Vec<Vec<f64>> =
        (0..batch).map(|_| (0..n).map(|_| if rng.unit() < 0.5 { -1.0 } else { 1.0 }).collect()).collect();
    let (mut gh, mut gw) = (vec![0.0; n], vec![0.0; net.pairs.len()]);
    if n > nv && net.pairs.iter().all(|&(a, b)| net.w[a * n + b] == 0.0) {
        // with every hidden pull at zero the hidden things all look alike and the gradient cannot tell them apart
        for &(a, b) in &net.pairs {
            let x = 0.1 * rng.normal();
            net.w[a * n + b] = x;
            net.w[b * n + a] = x;
        }
    }
    for round in 0..o.rounds {
        let rate = o.rate * (1.0 - 0.9 * round as f64 / o.rounds.max(1) as f64);
        for j in (1..order.len()).rev() {
            let r = rng.below(j + 1);
            order.swap(j, r);
        }
        for chunk in order.chunks(batch) {
            gh.iter_mut().for_each(|g| *g = 0.0);
            gw.iter_mut().for_each(|g| *g = 0.0);
            match o.method {
                Method::Pseudo => {
                    for &r in chunk {
                        let v = &rows[r];
                        let res: Vec<f64> = (0..nv).map(|a| v[a] - (o.beta * net.input(a, v)).tanh()).collect();
                        for (g, x) in gh.iter_mut().zip(&res) {
                            *g += x;
                        }
                        for (g, &(a, b)) in gw.iter_mut().zip(&net.pairs) {
                            *g += res[a] * v[b] + res[b] * v[a];
                        }
                    }
                }
                _ => {
                    for &r in chunk {
                        let s = net.with_hidden_means(&rows[r], o.beta);
                        net.add_stats(&s, 1.0, &mut gh, &mut gw);
                    }
                    match o.method {
                        Method::Exact => {
                            let (mh, mw) = net.exact_moments(o.beta);
                            let c = chunk.len() as f64;
                            gh.iter_mut().zip(&mh).for_each(|(g, x)| *g -= c * x);
                            gw.iter_mut().zip(&mw).for_each(|(g, x)| *g -= c * x);
                        }
                        Method::Contrastive => {
                            for &r in chunk {
                                let mut s = rows[r].clone();
                                s.resize(n, 0.0);
                                for a in nv..n {
                                    net.flip(a, &mut s, o.beta, rng);
                                }
                                net.gibbs(&mut s, o.sweeps, o.beta, rng, &mut vis_order);
                                let s = net.with_hidden_means(&s[..nv], o.beta);
                                net.add_stats(&s, -1.0, &mut gh, &mut gw);
                            }
                        }
                        Method::Persistent => {
                            let scale = chunk.len() as f64 / batch as f64;
                            for c in chains.iter_mut() {
                                net.gibbs(c, o.sweeps, o.beta, rng, &mut vis_order);
                                let s = net.with_hidden_means(&c[..nv], o.beta);
                                net.add_stats(&s, -scale, &mut gh, &mut gw);
                            }
                        }
                        Method::Pseudo => unreachable!(),
                    }
                }
            }
            let step = rate / chunk.len() as f64;
            for (a, g) in gh.iter().enumerate() {
                net.h[a] += step * g;
            }
            for (&(a, b), g) in net.pairs.iter().zip(&gw) {
                let x = net.w[a * n + b] + step * g - rate * o.decay * net.w[a * n + b];
                net.w[a * n + b] = x;
                net.w[b * n + a] = x;
            }
        }
    }
}

fn learn_stmt(m: &mut Model, st: &mut State, name: &str, rest: &[Tok], ln: usize, ctx: &mut Ctx) -> Result<(), SettleError> {
    let ex = load_examples(m, name, ln)?;
    if ex.rows.is_empty() {
        return err(ln, format!("examples :{} have no rows", name));
    }
    let rest = if rest.first() == Some(&Tok::Comma) { &rest[1..] } else { rest };
    let kv = kwargs(rest, ln)?;
    only(&kv, &["rounds", "rate", "method", "sweeps", "batch", "decay", "seed"], "learn", ln)?;
    let get = |k: &str, d: f64| kw(&kv, k).map(|v| num(v, ln)).transpose().map(|x| x.unwrap_or(d));
    let method = match kw(&kv, "method") {
        None => Method::Contrastive,
        Some(Tok::Sym(s)) => match s.as_str() {
            "exact" => Method::Exact,
            "contrastive" => Method::Contrastive,
            "persistent" => Method::Persistent,
            "pseudo" => Method::Pseudo,
            _ => return err(ln, "method: is :exact, :contrastive, :persistent or :pseudo"),
        },
        Some(_) => return err(ln, "method: takes a symbol, like method: :contrastive"),
    };
    let o = Opts {
        method,
        rounds: get("rounds", 100.0)? as usize,
        rate: get("rate", 0.05)?,
        batch: get("batch", 50.0)? as usize,
        sweeps: get("sweeps", 1.0)? as usize,
        decay: get("decay", 0.0)?,
        beta: 1.0 / st.temp,
    };
    if o.rate <= 0.0 || o.sweeps == 0 || o.decay < 0.0 {
        return err(ln, "rate must be above zero, sweeps at least 1, decay not negative");
    }
    if let Some(v) = kw(&kv, "seed") {
        st.rng = Rng::new(num(v, ln)? as u64);
    }
    let vis: Vec<usize> = ex.names.iter().map(|n| m.need(n, ln)).collect::<Result<_, _>>()?;
    let hid = hidden_of(m);
    if method == Method::Pseudo && !hid.is_empty() {
        return err(ln, "method: :pseudo needs every thing visible; this model has hidden things");
    }
    if method == Method::Exact && vis.len() > 20 {
        return err(ln, format!("method: :exact enumerates the visible things and allows at most 20, not {}", vis.len()));
    }
    let mut net = Net::from_model(m, &vis, &hid, ln)?;
    let t0 = Instant::now();
    train(&mut net, &ex.rows, &o, &mut st.rng);
    let secs = t0.elapsed().as_secs_f64();
    net.write_back(m);
    let fit = if vis.len() <= 20 {
        format!("exact log-likelihood per example {:.4}", net.exact_log_likelihood(&ex.rows, o.beta))
    } else if hid.is_empty() {
        format!("pseudo-log-likelihood per example {:.3}", net.pseudo_log_likelihood(&ex.rows, o.beta))
    } else {
        "no cheap fit score for a large machine with hidden things".to_string()
    };
    let how = match method {
        Method::Exact => "exact gradients",
        Method::Contrastive => "contrastive divergence",
        Method::Persistent => "persistent contrastive divergence",
        Method::Pseudo => "pseudo-likelihood",
    };
    ctx.say(format!(
        "learned :{} by {} over {} rounds of {} rows: {} visible, {} hidden, {} pulls, {:.2}s; {}",
        name,
        how,
        o.rounds,
        ex.rows.len(),
        vis.len(),
        hid.len(),
        net.pairs.len(),
        secs,
        fit
    ));
    Ok(())
}

/// Positions in `ex.names` of the label things: space-separated names, `d*` meaning every name starting with d.
fn label_positions(ex: &Examples, spec: &str, ln: usize) -> Result<Vec<usize>, SettleError> {
    let mut out = Vec::new();
    for w in words(spec) {
        let hits: Vec<usize> = match w.strip_suffix('*') {
            Some(pre) => (0..ex.names.len()).filter(|&i| ex.names[i].starts_with(pre)).collect(),
            None => (0..ex.names.len()).filter(|&i| ex.names[i] == w).collect(),
        };
        if hits.is_empty() {
            return err(ln, format!("no thing in these examples matches label `{}`", w));
        }
        out.extend(hits.into_iter().filter(|i| !out.contains(i)).collect::<Vec<_>>());
    }
    if out.len() < 2 || out.len() == ex.names.len() {
        return err(ln, "labels: needs at least two label things and at least one input thing");
    }
    Ok(out)
}

/// Accuracy of reading the labels by settling with the inputs held, and by the exact one-hot readout.
pub struct Scores {
    pub rows: usize,
    pub settled: usize,
    pub exact: usize,
    pub skipped: usize,
}

pub fn classify_rows(m: &Model, st: &mut State, ex: &Examples, labels: &[usize], sweeps: usize, ln: usize) -> Result<Scores, SettleError> {
    let idx: Vec<usize> = ex.names.iter().map(|n| m.need(n, ln)).collect::<Result<_, _>>()?;
    let net = Net::from_model(m, &idx, &hidden_of(m), ln)?;
    let beta = 1.0 / st.temp;
    let saved = st.held.clone();
    let mut sc = Scores { rows: 0, settled: 0, exact: 0, skipped: 0 };
    for row in &ex.rows {
        let on: Vec<usize> = (0..labels.len()).filter(|&j| row[labels[j]] > 0.0).collect();
        if on.len() != 1 {
            sc.skipped += 1;
            continue;
        }
        let truth = on[0];
        sc.rows += 1;
        st.held = saved.clone();
        for (p, &u) in idx.iter().enumerate() {
            if !labels.contains(&p) {
                st.held.insert(u, row[p]);
            }
        }
        let (mut s, mut free) = st.start(m);
        for _ in 0..(sweeps / 10).max(1) {
            st.sweep(m, &mut s, &mut free, beta);
        }
        let mut cnt = vec![0usize; labels.len()];
        for _ in 0..sweeps {
            st.sweep(m, &mut s, &mut free, beta);
            for (j, &p) in labels.iter().enumerate() {
                if s[idx[p]] > 0.0 {
                    cnt[j] += 1;
                }
            }
        }
        let pick = (0..cnt.len()).fold(0, |b, j| if cnt[j] > cnt[b] { j } else { b });
        if pick == truth {
            sc.settled += 1;
        }
        let mut v = row.clone();
        let score = |c: usize, v: &mut Vec<f64>| {
            for (j, &p) in labels.iter().enumerate() {
                v[p] = if j == c { 1.0 } else { -1.0 };
            }
            net.neg_beta_free(v, beta)
        };
        let (mut best, mut bs) = (0, f64::NEG_INFINITY);
        for c in 0..labels.len() {
            let x = score(c, &mut v);
            if x > bs {
                best = c;
                bs = x;
            }
        }
        if best == truth {
            sc.exact += 1;
        }
    }
    st.held = saved;
    Ok(sc)
}

fn classify_stmt(m: &Model, st: &mut State, name: &str, rest: &[Tok], ln: usize, ctx: &mut Ctx) -> Result<(), SettleError> {
    let ex = load_examples(m, name, ln)?;
    let rest = if rest.first() == Some(&Tok::Comma) { &rest[1..] } else { rest };
    let kv = kwargs(rest, ln)?;
    only(&kv, &["labels", "sweeps", "seed", "temperature"], "classify", ln)?;
    let spec = match kw(&kv, "labels") {
        Some(v) => text(v, ln)?,
        None => return err(ln, "classify needs `labels:`, like labels: \"d0 d1 d2\" or labels: \"d*\""),
    };
    let labels = label_positions(&ex, &spec, ln)?;
    let sweeps = kw(&kv, "sweeps").map(|v| num(v, ln)).transpose()?.unwrap_or(100.0) as usize;
    if let Some(v) = kw(&kv, "seed") {
        st.rng = Rng::new(num(v, ln)? as u64);
    }
    if let Some(v) = kw(&kv, "temperature") {
        st.temp = num(v, ln)?;
        if st.temp <= 0.0 {
            return err(ln, "temperature must be above zero");
        }
    }
    let sc = classify_rows(m, st, &ex, &labels, sweeps.max(1), ln)?;
    let pct = |k: usize| 100.0 * k as f64 / sc.rows.max(1) as f64;
    ctx.say(format!(
        "classify :{} by settling {} sweeps with {} inputs held: {} of {} right ({:.1}%); exact one-hot readout {:.1}%; chance {:.1}%{}",
        name,
        sweeps,
        ex.names.len() - labels.len(),
        sc.settled,
        sc.rows,
        pct(sc.settled),
        pct(sc.exact),
        100.0 / labels.len() as f64,
        if sc.skipped > 0 { format!("; {} rows skipped (not exactly one label on)", sc.skipped) } else { String::new() }
    ));
    Ok(())
}

fn shuffle_stmt(m: &mut Model, st: &mut State, name: &str, rest: &[Tok], ln: usize, ctx: &mut Ctx) -> Result<(), SettleError> {
    let mut ex = load_examples(m, name, ln)?;
    let rest = if rest.first() == Some(&Tok::Comma) { &rest[1..] } else { rest };
    let kv = kwargs(rest, ln)?;
    only(&kv, &["labels", "seed"], "shuffle", ln)?;
    let spec = match kw(&kv, "labels") {
        Some(v) => text(v, ln)?,
        None => return err(ln, "shuffle needs `labels:` naming the columns to scramble"),
    };
    let labels = label_positions(&ex, &spec, ln)?;
    let mut rng = match kw(&kv, "seed") {
        Some(v) => Rng::new(num(v, ln)? as u64),
        None => Rng::new(st.rng.next_u64()),
    };
    let blocks: Vec<Vec<f64>> = ex.rows.iter().map(|r| labels.iter().map(|&p| r[p]).collect()).collect();
    let mut perm: Vec<usize> = (0..blocks.len()).collect();
    for j in (1..perm.len()).rev() {
        let r = rng.below(j + 1);
        perm.swap(j, r);
    }
    for (row, &from) in ex.rows.iter_mut().zip(&perm) {
        for (j, &p) in labels.iter().enumerate() {
            row[p] = blocks[from][j];
        }
    }
    keep_examples(m, name, &ex);
    ctx.say(format!("shuffled the {} label columns of :{} across {} rows", labels.len(), name, ex.rows.len()));
    Ok(())
}

impl Ext for Learn {
    fn name(&self) -> &'static str {
        "learn"
    }

    fn statements(&self) -> &'static [&'static str] {
        &[
            "model: examples :data, \"file.txt\"   /   examples :data, over: \"a b c\", rows: \"011 101\"",
            "model: hidden 16",
            "model: shuffle :data, labels: \"d*\", seed: 3",
            "run: examples :data, \"file.txt\"   /   examples :data, rows: \"110\"",
            "run: learn :data, rounds: 200, rate: 0.05, method: :contrastive, sweeps: 1, batch: 50, decay: 0, seed: 1",
            "run: classify :test, labels: \"d*\", sweeps: 100, seed: 2",
            "run: shuffle :data, labels: \"d*\", seed: 3",
        ]
    }

    fn model_stmt(&self, m: &mut Model, t: &[Tok], ln: usize, ctx: &mut Ctx) -> Claim {
        match t {
            [Tok::Ident(k), Tok::Sym(name), rest @ ..] if k == "examples" => Some(examples_stmt(m, name, rest, ln, ctx)),
            [Tok::Ident(k), Tok::Num(c)] if k == "hidden" => Some(hidden_stmt(m, *c, ln)),
            [Tok::Ident(k), Tok::Sym(name), rest @ ..] if k == "shuffle" => {
                let mut st = State::new(0x5eed);
                Some(shuffle_stmt(m, &mut st, name, rest, ln, ctx))
            }
            _ => None,
        }
    }

    fn run_stmt(&self, m: &mut Model, st: &mut State, t: &[Tok], ln: usize, ctx: &mut Ctx) -> Claim {
        match t {
            [Tok::Ident(k), Tok::Sym(name), rest @ ..] if k == "examples" => Some(examples_stmt(m, name, rest, ln, ctx)),
            [Tok::Ident(k), Tok::Sym(name), rest @ ..] if k == "learn" => Some(learn_stmt(m, st, name, rest, ln, ctx)),
            [Tok::Ident(k), Tok::Sym(name), rest @ ..] if k == "classify" => Some(classify_stmt(m, st, name, rest, ln, ctx)),
            [Tok::Ident(k), Tok::Sym(name), rest @ ..] if k == "shuffle" => Some(shuffle_stmt(m, st, name, rest, ln, ctx)),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interp::Interp;

    fn run(src: &str) -> (Vec<String>, Interp) {
        let mut it = Interp::default();
        let out = it.exec(src).unwrap_or_else(|e| panic!("{}", e));
        (out, it)
    }

    /// A fully visible machine of `n` things with random leans and pulls.
    fn random_net(n: usize, seed: u64) -> Net {
        let mut r = Rng::new(seed);
        let mut net = Net { units: (0..n).collect(), nv: n, h: vec![0.0; n], w: vec![0.0; n * n], pairs: Vec::new() };
        for a in 0..n {
            net.h[a] = 0.6 * r.signed();
            for b in (a + 1)..n {
                let x = 0.7 * r.signed();
                net.w[a * n + b] = x;
                net.w[b * n + a] = x;
                net.pairs.push((a, b));
            }
        }
        net
    }

    fn blank_like(net: &Net) -> Net {
        let n = net.n();
        Net { h: vec![0.0; n], w: vec![0.0; n * n], ..net.clone() }
    }

    /// Exact samples from a small fully visible machine, by enumeration.
    fn exact_samples(net: &Net, count: usize, seed: u64) -> Vec<Vec<f64>> {
        let all: Vec<Vec<f64>> = net.visible_arrangements().collect();
        let lw: Vec<f64> = all.iter().map(|v| net.neg_beta_free(v, 1.0)).collect();
        let lz = log_sum_exp(&lw);
        let mut cum = Vec::new();
        let mut c = 0.0;
        for l in &lw {
            c += (l - lz).exp();
            cum.push(c);
        }
        let mut r = Rng::new(seed);
        (0..count)
            .map(|_| {
                let u = r.unit() * c;
                all[cum.iter().position(|&x| x >= u).unwrap_or(all.len() - 1)].clone()
            })
            .collect()
    }

    fn fit(net: &Net, rows: &[Vec<f64>], method: Method, rounds: usize, rate: f64, seed: u64) -> Net {
        let mut f = blank_like(net);
        let o = Opts { method, rounds, rate, batch: 50, sweeps: 1, decay: 0.0, beta: 1.0 };
        train(&mut f, rows, &o, &mut Rng::new(seed));
        f
    }

    fn max_param_gap(a: &Net, b: &Net) -> f64 {
        let n = a.n();
        let hg = a.h.iter().zip(&b.h).map(|(x, y)| (x - y).abs()).fold(0.0, f64::max);
        let wg = a.pairs.iter().map(|&(i, k)| (a.w[i * n + k] - b.w[i * n + k]).abs()).fold(0.0, f64::max);
        hg.max(wg)
    }

    fn data_moments(net: &Net, rows: &[Vec<f64>]) -> (Vec<f64>, Vec<f64>) {
        let (mut h, mut w) = (vec![0.0; net.n()], vec![0.0; net.pairs.len()]);
        for v in rows {
            net.add_stats(v, 1.0 / rows.len() as f64, &mut h, &mut w);
        }
        (h, w)
    }

    #[test]
    fn exact_fit_reproduces_the_data_averages() {
        // the maximum-likelihood condition: at the fit, the machine's own averages equal the examples' averages
        let truth = random_net(5, 11);
        let rows = exact_samples(&truth, 4000, 12);
        let f = fit(&truth, &rows, Method::Exact, 1500, 0.3, 1);
        let (dh, dw) = data_moments(&f, &rows);
        let (mh, mw) = f.exact_moments(1.0);
        for (a, b) in dh.iter().chain(&dw).zip(mh.iter().chain(&mw)) {
            assert!((a - b).abs() < 2e-3, "data {} machine {}", a, b);
        }
    }

    #[test]
    fn exact_fit_recovers_the_machine_that_made_the_data() {
        let truth = random_net(5, 21);
        let rows = exact_samples(&truth, 8000, 22);
        let f = fit(&truth, &rows, Method::Exact, 1500, 0.3, 1);
        assert!(max_param_gap(&f, &truth) < 0.12, "gap {}", max_param_gap(&f, &truth));
    }

    #[test]
    fn sampled_fits_agree_with_the_exact_fit() {
        // the control asked for: gradients estimated by settling must land where the exact gradients land
        let truth = random_net(5, 31);
        let rows = exact_samples(&truth, 4000, 32);
        let exact = fit(&truth, &rows, Method::Exact, 1500, 0.3, 1);
        let pcd = fit(&truth, &rows, Method::Persistent, 300, 0.05, 2);
        let cd = fit(&truth, &rows, Method::Contrastive, 300, 0.05, 3);
        let pl = fit(&truth, &rows, Method::Pseudo, 300, 0.05, 4);
        for (name, f, tol) in [("persistent", &pcd, 0.05), ("contrastive", &cd, 0.05), ("pseudo", &pl, 0.05)] {
            let g = max_param_gap(f, &exact);
            println!("{} vs exact: largest parameter gap {:.4} (exact vs truth {:.4})", name, g, max_param_gap(&exact, &truth));
            assert!(g < tol, "{} differs from the exact fit by {}", name, g);
        }
    }

    #[test]
    fn a_fit_to_other_data_does_not_agree() {
        // negative control for the agreement test: the same comparison against a different machine's data fails
        let (a, b) = (random_net(5, 31), random_net(5, 41));
        let exact_a = fit(&a, &exact_samples(&a, 4000, 32), Method::Exact, 1500, 0.3, 1);
        let pcd_b = fit(&b, &exact_samples(&b, 4000, 42), Method::Persistent, 300, 0.05, 2);
        println!("persistent fit to other data vs exact fit: largest parameter gap {:.4}", max_param_gap(&pcd_b, &exact_a));
        assert!(max_param_gap(&pcd_b, &exact_a) > 0.3, "gap {}", max_param_gap(&pcd_b, &exact_a));
    }

    const PARITY: &str = "examples :par, over: \"a b c\", rows: \"000 011 101 110 000 011 101 110 000 011 101 110 000 011 101 110 000 011 101 110 000 011 101 110 000 011 101 110 000 011 101 110\"";

    fn parity_ll(hidden: usize, method: &str, rate: f64, seed: u64) -> f64 {
        let hid = if hidden > 0 { format!("  hidden {}\n", hidden) } else { String::new() };
        let src = format!(
            "model :p do\n  {}\n{}end\nrun :p do\n  learn :par, rounds: 3000, rate: {}, method: :{}, batch: 32, sweeps: 5, seed: {}\nend",
            PARITY, hid, rate, method, seed
        );
        let (out, _) = run(&src);
        let line = out.iter().find(|l| l.starts_with("learned")).unwrap();
        line.rsplit(' ').next().unwrap().parse().unwrap()
    }

    #[test]
    fn hidden_things_learn_parity_which_pulls_between_visible_things_cannot() {
        // even parity over three bits has no pairwise signal: the best fully visible machine is uniform, ln(1/8)
        let fv = parity_ll(0, "exact", 0.5, 1);
        assert!((fv - (1.0f64 / 8.0).ln()).abs() < 0.01, "fully visible {}", fv);
        // four hidden things can put all the chance on the four even patterns, ln(1/4) = -1.386
        for seed in 1..=2 {
            let rbm = parity_ll(4, "exact", 0.5, seed);
            assert!(rbm > -1.40, "restricted, exact, seed {}: {}", seed, rbm);
            let rbm_cd = parity_ll(4, "contrastive", 0.1, seed);
            assert!(rbm_cd > -1.45, "restricted, contrastive, seed {}: {}", seed, rbm_cd);
        }
    }

    #[test]
    fn a_restricted_machine_can_stall_at_the_uniform_saddle() {
        // recorded, not hidden: at seed 3 the small random start does not escape the flat point where
        // every hidden thing sees zero net signal, and the sampled fit ends at the fully visible answer
        let ll = parity_ll(4, "contrastive", 0.1, 3);
        assert!((ll - (1.0f64 / 8.0).ln()).abs() < 0.01, "{}", ll);
    }

    /// Three 3x3 glyphs with pixel noise; columns p0..p8 then labels l0 l1 l2.
    fn glyph_rows(count: usize, noise: f64, seed: u64) -> String {
        let glyphs = ["010111010", "101010101", "111101111"];
        let mut r = Rng::new(seed);
        let mut rows = Vec::new();
        for i in 0..count {
            let c = i % 3;
            let px: String = glyphs[c].chars().map(|ch| if r.unit() < noise { if ch == '1' { '0' } else { '1' } } else { ch }).collect();
            let lab: String = (0..3).map(|j| if j == c { '1' } else { '0' }).collect();
            rows.push(format!("{}{}", px, lab));
        }
        rows.join(" ")
    }

    fn glyph_program(method: &str, shuffle: bool) -> String {
        format!(
            "model :g do\n  examples :train, over: \"p0 p1 p2 p3 p4 p5 p6 p7 p8 l0 l1 l2\", rows: \"{}\"\n  examples :test, over: \"p0 p1 p2 p3 p4 p5 p6 p7 p8 l0 l1 l2\", rows: \"{}\"\n{}end\nrun :g do\n  learn :train, rounds: 150, rate: 0.05, method: :{}, seed: 1\n  classify :test, labels: \"l*\", sweeps: 200, seed: 2\nend",
            glyph_rows(300, 0.15, 5),
            glyph_rows(150, 0.15, 6),
            if shuffle { "  shuffle :train, labels: \"l*\", seed: 7\n" } else { "" },
            method
        )
    }

    fn scores(out: &[String]) -> (f64, f64) {
        let line = out.iter().find(|l| l.starts_with("classify")).unwrap();
        let settled: f64 = line.split('(').nth(1).unwrap().split('%').next().unwrap().parse().unwrap();
        let exact: f64 = line.split("readout ").nth(1).unwrap().split('%').next().unwrap().parse().unwrap();
        (settled, exact)
    }

    #[test]
    fn a_settled_classifier_reads_noisy_glyphs() {
        for method in ["pseudo", "contrastive"] {
            let (out, _) = run(&glyph_program(method, false));
            let (settled, exact) = scores(&out);
            assert!(settled > 85.0 && exact > 85.0, "{}: {:?}", method, out);
        }
    }

    #[test]
    fn shuffled_labels_give_chance_accuracy() {
        // negative control: the same pipeline trained on scrambled labels must not beat chance (33.3%) by much
        let (out, _) = run(&glyph_program("pseudo", true));
        let (settled, exact) = scores(&out);
        assert!(settled < 50.0 && exact < 50.0, "{:?}", out);
    }

    #[test]
    fn learning_persists_in_the_model_and_settling_then_follows_it() {
        let src = "model :m do
  examples :d, over: \"a b\", rows: \"11 11 11 00 00 00 11 00\"
end
run :m do
  learn :d, rounds: 400, rate: 0.2, method: :exact
  hold :a, :yes
  settle 20_000, seed: 1
  show
end";
        let (out, it) = run(src);
        let m = &it.models["m"];
        assert!(m.coupling(m.idx["a"], m.idx["b"]) > 1.0, "the pair always agrees, so its pull must grow large");
        let b_line = out.iter().find(|l| l.trim_start().starts_with("b ")).unwrap();
        let pct: f64 = b_line.split_whitespace().nth(2).unwrap().trim_end_matches('%').parse().unwrap();
        assert!(pct > 90.0, "{}", b_line);
    }

    #[test]
    fn files_parse_with_comments_and_spacing() {
        let ex = parse_file("# a comment\na b c\n1 0 1\n\n+-+  # trailing\ny,n,y\n", 1).unwrap();
        assert_eq!(ex.names, ["a", "b", "c"]);
        assert_eq!(ex.rows, vec![vec![1.0, -1.0, 1.0]; 3]);
    }

    #[test]
    fn errors_name_their_line() {
        let cases = [
            ("model :m do\n  examples :d, over: \"a b\", rows: \"101\"\nend", "line 2: a row has 3 cells"),
            ("model :m do\n  examples :d, over: \"a b\", rows: \"1x\"\nend", "line 2: 'x' is not a yes/no cell"),
            ("model :m do\n  thing :a\nend\nrun :m do\n  learn :nope\nend", "line 5: no examples :nope"),
            ("model :m do\n  examples :d, over: \"a b\", rows: \"11\"\n  hidden 2\nend\nrun :m do\n  learn :d, method: :pseudo\nend", "line 6: method: :pseudo needs"),
            ("model :m do\n  examples :d, over: \"a b\", rows: \"11\"\nend\nrun :m do\n  classify :d, labels: \"a b\"\nend", "line 5: labels: needs at least two label things and at least one input"),
            ("model :m do\n  examples :d, \"no/such/file.txt\"\nend", "line 2: cannot read"),
        ];
        for (src, want) in cases {
            let e = Interp::default().exec(src).err().map(|e| e.0).unwrap_or_default();
            assert!(e.starts_with(want), "{:?} gave {:?}", src, e);
        }
    }
}
