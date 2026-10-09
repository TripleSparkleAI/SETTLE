//! LDPCSETTLE: decode a low-density parity-check code by letting a SETTLE model settle.
//!
//! ```text
//! model :line do
//!   ldpc :c, bits: 64, rate: 0.5, gadget: :chain, strength: 2, codebook_seed: 1
//! end
//! run :line do
//!   c.transmit flip: 0.03, seed: 1        # random message, encoded, sent through a random-flip channel
//!   c.decode start: :received, sweeps: 400, seed: 2   # settle from the received word, keep the calmest
//!   c.decode start: :random, sweeps: 400, seed: 3     # anneal from noise instead
//!   c.decode_bp                          # belief propagation on the same code, for comparison
//! end
//! ```
//!
//! The engine has only leans and pairwise pulls, so every parity check is built from helper things and a
//! penalty that is zero exactly on an even check (the VALLEYMAP gadget `(a + b + c - 2x)^2`, and a
//! generalisation of it). Two gadgets:
//!
//! - `:sum` puts one integer helper z per check, written in binary with K = bits of floor(w/2) helper things,
//!   and charges `lambda (sum of the w bits - 2 z)^2`. Zero exactly when the sum is even and z is half of it.
//! - `:chain` splits a check of weight w into a chain of w - 2 three-bit checks through w - 3 partial-parity
//!   things (a1 = b0 xor b1, a2 = a1 xor b2, ...), each three-bit check charged `lambda (u + v + t - 2x)^2`
//!   with its own helper x. Every coefficient stays at most 4 lambda, where the sum gadget's grow with w.
//!
//! In both, the helper and partial-parity values of a codeword are unique, and every arrangement that
//! breaks a check costs at least lambda (proved by enumeration in the tests below). The channel adds a lean
//! on each code bit: `atanh(1 - 2p)` toward the received value for a hard bit, or a scaled spin average for
//! soft input. With lambda large the calmest arrangement is the most likely codeword, and at temperature 1
//! the settled distribution over code bits is the exact posterior (the Nishimori line of Sourlas's framing).
//!
//! A decode keeps the calmest arrangement visited and reports its code bits; it claims success only when
//! those bits satisfy every check of the code it was built with, and refuses otherwise. Equations with plain
//! readings: `runs/ldpcsettle/REPORT_LDPCSETTLE.md`.

use crate::coded::{CodeKind, Codec, Ldpc};
use crate::ext::{Claim, Ctx, Ext};
use crate::lex::{err, kw, kwargs, num, only, SettleError, Tok, whole};
use crate::model::{Model, State};
use crate::rng::Rng;
use crate::zoo::Qubo;

pub struct LdpcSettle;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Gadget {
    Sum,
    Chain,
}

impl Gadget {
    pub fn parse(s: &str) -> Option<Gadget> {
        match s {
            "sum" => Some(Gadget::Sum),
            "chain" => Some(Gadget::Chain),
            _ => None,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Gadget::Sum => "sum",
            Gadget::Chain => "chain",
        }
    }
}

/// How one check's helper things are laid out, so a codeword's helper values can be written down.
#[derive(Clone, Debug)]
enum Local {
    /// Binary helpers of z, lowest bit first.
    Sum { helpers: Vec<usize> },
    /// Partial parities a_1..a_{w-3}, and one helper per three-bit check (w - 2 of them, or none for w < 3).
    Chain { partial: Vec<usize>, helpers: Vec<usize> },
}

/// The penalty of a parity-check matrix as a QUBO over code bits 0..n and helper variables n..total, at
/// lambda = 1. Returns the QUBO and the per-check layout.
fn penalty_qubo(rows: &[Vec<usize>], n: usize, gadget: Gadget) -> (Qubo, Vec<Local>) {
    let mut next = n;
    let mut layout = Vec::new();
    // First pass: count the helper variables.
    for row in rows {
        let w = row.len();
        match gadget {
            Gadget::Sum => {
                let k = helper_bits(w);
                layout.push(Local::Sum { helpers: (next..next + k).collect() });
                next += k;
            }
            Gadget::Chain => {
                let (np, nh) = if w >= 3 { (w - 3, w - 2) } else { (0, 0) };
                let partial = (next..next + np).collect();
                next += np;
                let helpers = (next..next + nh).collect();
                next += nh;
                layout.push(Local::Chain { partial, helpers });
            }
        }
    }
    let mut q = Qubo::new(next);
    for (row, loc) in rows.iter().zip(&layout) {
        let w = row.len();
        match loc {
            Local::Sum { helpers } => {
                // (sum_i b_i - sum_k c_k x_k)^2 with c_k = 2^(k+1)
                let mut terms: Vec<(usize, f64)> = row.iter().map(|&b| (b, 1.0)).collect();
                for (k, &x) in helpers.iter().enumerate() {
                    terms.push((x, -(2.0f64.powi(k as i32 + 1))));
                }
                square(&mut q, &terms);
            }
            Local::Chain { partial, helpers } => match w {
                0 => {}
                1 => q.lin[row[0]] += 1.0,
                2 => square(&mut q, &[(row[0], 1.0), (row[1], -1.0)]),
                _ => {
                    // triples: (b0, b1, a1), (a1, b2, a2), ..., (a_{w-3}, b_{w-2}, b_{w-1})
                    let mut left = row[0];
                    for t in 0..(w - 2) {
                        let mid = row[t + 1];
                        let right = if t + 1 < w - 2 { partial[t] } else { row[w - 1] };
                        square(&mut q, &[(left, 1.0), (mid, 1.0), (right, 1.0), (helpers[t], -2.0)]);
                        left = right;
                    }
                }
            },
        }
    }
    (q, layout)
}

/// Helper things needed to write floor(w/2) in binary.
fn helper_bits(w: usize) -> usize {
    let mut top = w / 2;
    let mut k = 0;
    while top > 0 {
        k += 1;
        top >>= 1;
    }
    k
}

/// Add (sum_i c_i y_i)^2 to a QUBO, using y^2 = y for 0/1 variables.
fn square(q: &mut Qubo, terms: &[(usize, f64)]) {
    for (a, &(i, ci)) in terms.iter().enumerate() {
        q.lin[i] += ci * ci;
        for &(j, cj) in &terms[a + 1..] {
            q.add_quad(i, j, 2.0 * ci * cj);
        }
    }
}

/// A code built as springs, ready to settle. Things 0..n are the code bits (bit 1 = thing at +1).
#[derive(Clone)]
pub struct SettleCode {
    pub n: usize,
    pub rows: Vec<Vec<usize>>,
    pub fixed_zero: Vec<usize>,
    pub gadget: Gadget,
    pub strength: f64,
    pub things: usize,
    layout: Vec<Local>,
    qubo: Qubo,
    /// Leans from the penalty alone; channel leans are added on top of these.
    base_h: Vec<f64>,
    pub h: Vec<f64>,
    off: Vec<usize>,
    nbr: Vec<usize>,
    wt: Vec<f64>,
}

/// The outcome of one settle decode.
#[derive(Clone, Debug)]
pub struct Decoded {
    /// Code bits of the calmest arrangement visited.
    pub bits: Vec<u8>,
    /// True when those bits satisfy every check (and the fixed-zero bits are 0).
    pub codeword: bool,
    /// Model energy of the calmest arrangement (penalty plus channel leans, SETTLE convention).
    pub energy: f64,
    /// Checks the calmest arrangement's code bits break.
    pub broken: usize,
}

impl SettleCode {
    pub fn new(rows: &[Vec<usize>], n: usize, fixed_zero: &[usize], gadget: Gadget, strength: f64) -> SettleCode {
        let (mut q, layout) = penalty_qubo(rows, n, gadget);
        for a in q.lin.iter_mut() {
            *a *= strength;
        }
        for b in q.quad.values_mut() {
            *b *= strength;
        }
        q.c *= strength;
        let mut m = Model::default();
        for i in 0..q.lin.len() {
            m.add(&format!("t{}", i));
        }
        q.apply(&mut m, 0);
        let things = m.len();
        let mut off = vec![0];
        let (mut nbr, mut wt) = (Vec::new(), Vec::new());
        for row in &m.adj {
            for &(k, w) in row {
                nbr.push(k);
                wt.push(w);
            }
            off.push(nbr.len());
        }
        SettleCode {
            n,
            rows: rows.to_vec(),
            fixed_zero: fixed_zero.to_vec(),
            gadget,
            strength,
            things,
            layout,
            qubo: q,
            base_h: m.h.clone(),
            h: m.h,
            off,
            nbr,
            wt,
        }
    }

    /// Build from one of `coded.rs`'s LDPC codes.
    pub fn from_ldpc(l: &Ldpc, gadget: Gadget, strength: f64) -> SettleCode {
        SettleCode::new(&l.rows, l.n, &l.fixed_zero, gadget, strength)
    }

    /// Helper things in total (the gadget count).
    pub fn helpers(&self) -> usize {
        self.things - self.n
    }

    /// Pairwise pulls in the model (each counted once).
    pub fn pulls(&self) -> usize {
        self.nbr.len() / 2
    }

    /// The strongest pull or push, and the strongest penalty lean, in SETTLE units.
    pub fn max_pull_and_lean(&self) -> (f64, f64) {
        let j = self.wt.iter().fold(0.0f64, |a, &w| a.max(w.abs()));
        let h = self.base_h.iter().fold(0.0f64, |a, &w| a.max(w.abs()));
        (j, h)
    }

    /// Set the channel lean on each code bit (positive leans toward bit 1). Replaces earlier channel leans.
    pub fn set_leans(&mut self, lean: &[f64]) {
        assert_eq!(lean.len(), self.n);
        self.h = self.base_h.clone();
        for i in 0..self.n {
            self.h[i] += lean[i];
        }
    }

    /// The penalty (QUBO units, lambda included) of an arrangement given as 0/1 over all things.
    pub fn penalty(&self, y: &[bool]) -> f64 {
        self.qubo.energy(y)
    }

    /// Model energy of a ±1 arrangement over all things (the SETTLE convention: -sum h s - sum J s s).
    pub fn energy(&self, s: &[f64]) -> f64 {
        let mut e = 0.0;
        for i in 0..self.things {
            e -= self.h[i] * s[i];
            for p in self.off[i]..self.off[i + 1] {
                let k = self.nbr[p];
                if k > i {
                    e -= self.wt[p] * s[i] * s[k];
                }
            }
        }
        e
    }

    /// Checks broken by a bit vector.
    pub fn broken(&self, bits: &[u8]) -> usize {
        self.rows.iter().filter(|row| row.iter().fold(0u8, |a, &j| a ^ bits[j]) != 0).count()
    }

    /// The full 0/1 arrangement for code bits `bits`: helpers set to their best values (unique on an even
    /// check; on an odd check the lower half for `:sum`, the floor for each `:chain` helper).
    pub fn complete(&self, bits: &[u8]) -> Vec<bool> {
        let mut y = vec![false; self.things];
        for i in 0..self.n {
            y[i] = bits[i] == 1;
        }
        for (row, loc) in self.rows.iter().zip(&self.layout) {
            match loc {
                Local::Sum { helpers } => {
                    let z = row.iter().map(|&j| bits[j] as usize).sum::<usize>() / 2;
                    for (k, &x) in helpers.iter().enumerate() {
                        y[x] = (z >> k) & 1 == 1;
                    }
                }
                Local::Chain { partial, helpers } => {
                    let w = row.len();
                    if w < 3 {
                        continue;
                    }
                    let mut left = bits[row[0]];
                    for t in 0..(w - 2) {
                        let mid = bits[row[t + 1]];
                        let right = if t + 1 < w - 2 {
                            let a = left ^ mid;
                            y[partial[t]] = a == 1;
                            a
                        } else {
                            bits[row[w - 1]]
                        };
                        y[helpers[t]] = (left + mid + right) / 2 == 1;
                        left = right;
                    }
                }
            }
        }
        y
    }

    /// Settle-decode: start from `init` code bits (helpers completed) or from noise, cool geometrically from
    /// `t_hi` to `t_lo` over `sweeps` Gibbs sweeps, keep the calmest arrangement visited.
    pub fn decode(&self, init: Option<&[u8]>, sweeps: usize, t_hi: f64, t_lo: f64, seed: u64) -> Decoded {
        let mut rng = Rng::new(seed);
        let mut s: Vec<f64> = match init {
            Some(b) => self.complete(b).iter().map(|&v| if v { 1.0 } else { -1.0 }).collect(),
            None => (0..self.things).map(|_| if rng.unit() < 0.5 { -1.0 } else { 1.0 }).collect(),
        };
        let mut held = vec![false; self.things];
        for &j in &self.fixed_zero {
            held[j] = true;
            s[j] = -1.0;
        }
        let mut free: Vec<usize> = (0..self.things).filter(|&i| !held[i]).collect();
        let mut e = self.energy(&s);
        let mut best = (s.clone(), e);
        for step in 0..sweeps {
            let temp = t_hi * (t_lo / t_hi).powf(step as f64 / (sweeps.max(2) - 1) as f64);
            let beta = 1.0 / temp;
            for k in (1..free.len()).rev() {
                let r = rng.below(k + 1);
                free.swap(k, r);
            }
            for &i in free.iter() {
                let mut f = self.h[i];
                for p in self.off[i]..self.off[i + 1] {
                    f += self.wt[p] * s[self.nbr[p]];
                }
                let v = if (beta * f).tanh() > rng.signed() { 1.0 } else { -1.0 };
                if v != s[i] {
                    e -= (v - s[i]) * f;
                    s[i] = v;
                }
            }
            if e < best.1 - 1e-9 {
                best = (s.clone(), e);
            }
        }
        let bits: Vec<u8> = best.0[..self.n].iter().map(|&v| (v > 0.0) as u8).collect();
        let broken = self.broken(&bits);
        let codeword = broken == 0 && self.fixed_zero.iter().all(|&j| bits[j] == 0);
        Decoded { energy: self.energy(&best.0), bits, codeword, broken }
    }

    /// (LDPCMOVES) Single-thing Gibbs at a fixed temperature from `init` (helpers completed): the fraction of
    /// the sweeps after `burn` in which each code bit sat at 1. The same update rule as `decode`.
    pub fn gibbs_average(&self, init: &[u8], sweeps: usize, burn: usize, temp: f64, seed: u64) -> Vec<f64> {
        let mut rng = Rng::new(seed);
        let mut s: Vec<f64> = self.complete(init).iter().map(|&v| if v { 1.0 } else { -1.0 }).collect();
        let mut held = vec![false; self.things];
        for &j in &self.fixed_zero {
            held[j] = true;
            s[j] = -1.0;
        }
        let mut free: Vec<usize> = (0..self.things).filter(|&i| !held[i]).collect();
        let beta = 1.0 / temp;
        let mut acc = vec![0.0; self.n];
        let mut kept = 0usize;
        for step in 0..sweeps {
            for k in (1..free.len()).rev() {
                let r = rng.below(k + 1);
                free.swap(k, r);
            }
            for &i in free.iter() {
                let mut f = self.h[i];
                for p in self.off[i]..self.off[i + 1] {
                    f += self.wt[p] * s[self.nbr[p]];
                }
                s[i] = if (beta * f).tanh() > rng.signed() { 1.0 } else { -1.0 };
            }
            if step >= burn {
                for i in 0..self.n {
                    acc[i] += (s[i] > 0.0) as u8 as f64;
                }
                kept += 1;
            }
        }
        acc.iter().map(|a| a / kept.max(1) as f64).collect()
    }
}

/// Channel lean for a hard received bit through a flip probability p: atanh(1 - 2p) toward the bit.
pub fn hard_lean(bit: u8, p: f64) -> f64 {
    let l = (1.0 - 2.0 * p).atanh();
    if bit == 1 {
        l
    } else {
        -l
    }
}

/// Log-likelihood ratio for belief propagation (positive means 0) matching a lean: llr = -2 lean.
pub fn llr_of_lean(lean: f64) -> f64 {
    -2.0 * lean
}

/// A code bit's spin average m in [-1, 1] (bit 1 at +1) as a lean, scaled so a bit that never moved gets
/// the hard lean at p0: lean = atanh(1 - 2 p0) * m.
pub fn soft_lean_scaled(m: f64, p0: f64) -> f64 {
    (1.0 - 2.0 * p0).atanh() * m
}

/// The same spin average as a lean by Laplace's rule over `k` samples: P(+1) = (count + 1) / (k + 2).
pub fn soft_lean_laplace(m: f64, k: usize) -> f64 {
    let plus = (m + 1.0) / 2.0 * k as f64;
    let p1 = (plus + 1.0) / (k as f64 + 2.0);
    0.5 * (p1 / (1.0 - p1)).ln()
}

/// The LDPC code `coded.rs` builds for n bits at `rate` with codebook seed `cb` (the same matrix SDMCODED used).
pub fn coded_ldpc(n: usize, rate: f64, cb: u64) -> Ldpc {
    let kind = CodeKind::parse("ldpc", rate).expect("rate between 0.05 and 0.99");
    Codec::new(kind, n, cb).ldpc.expect("an LDPC codec")
}

// ---------------------------------------------------------------------------------------------------------
// Statements.

fn note(name: &str) -> String {
    format!("ldpcsettle:{}", name)
}

struct Spec {
    n: usize,
    rate: f64,
    gadget: Gadget,
    strength: f64,
    cb: u64,
}

fn spec(m: &Model, name: &str) -> Option<Spec> {
    let (nums, words) = m.notes.get(&note(name))?;
    Some(Spec { n: nums[0] as usize, rate: nums[1], strength: nums[2], cb: nums[3] as u64, gadget: Gadget::parse(&words[0])? })
}

fn sym(t: &Tok, what: &str, ln: usize) -> Result<String, SettleError> {
    match t {
        Tok::Sym(s) => Ok(s.clone()),
        _ => err(ln, format!("{} takes a symbol like :sum", what)),
    }
}

fn declare(m: &mut Model, name: &str, rest: &[Tok], ln: usize) -> Result<(), SettleError> {
    if m.notes.contains_key(&note(name)) {
        return err(ln, format!("code :{} is already declared", name));
    }
    let kv = kwargs(rest, ln)?;
    only(&kv, &["bits", "rate", "gadget", "strength", "codebook_seed"], "ldpc", ln)?;
    let get = |k: &str, d: f64| -> Result<f64, SettleError> { kw(&kv, k).map(|v| num(v, ln)).transpose().map(|x| x.unwrap_or(d)) };
    let n = get("bits", 64.0)? as usize;
    if !(8..=4096).contains(&n) {
        return err(ln, "bits must be between 8 and 4096");
    }
    let rate = get("rate", 0.5)?;
    if !(0.1..=0.9).contains(&rate) {
        return err(ln, "rate must be between 0.1 and 0.9");
    }
    let strength = get("strength", 2.0)?;
    if strength <= 0.0 {
        return err(ln, "strength must be above zero");
    }
    let gadget = match kw(&kv, "gadget") {
        Some(t) => {
            let g = sym(t, "gadget:", ln)?;
            Gadget::parse(&g).ok_or_else(|| SettleError(format!("line {}: gadget is :sum or :chain, not :{}", ln, g)))?
        }
        None => Gadget::Chain,
    };
    let cb = get("codebook_seed", 1.0)? as u64;
    let l = coded_ldpc(n, rate, cb);
    let sc = SettleCode::from_ldpc(&l, gadget, strength);
    // The springs also go on the model itself, so the core `anneal` and `settle` can run them too.
    let start = m.len();
    for i in 0..sc.things {
        if i < n {
            m.add(&format!("{}_b{}", name, i));
        } else {
            m.add(&format!("{}_x{}", name, i - n));
        }
    }
    for i in 0..sc.things {
        m.h[start + i] += sc.base_h[i];
        for p in sc.off[i]..sc.off[i + 1] {
            let k = sc.nbr[p];
            if k > i {
                m.couple(start + i, start + k, sc.wt[p]);
            }
        }
    }
    m.notes.insert(note(name), (vec![n as f64, rate, strength, cb as f64, start as f64], vec![gadget.name().into()]));
    Ok(())
}

fn transmit(m: &mut Model, st: &mut State, name: &str, rest: &[Tok], ln: usize, ctx: &mut Ctx) -> Result<(), SettleError> {
    let sp = spec(m, name).unwrap();
    let kv = kwargs(rest, ln)?;
    only(&kv, &["flip", "seed"], "transmit", ln)?;
    if let Some(v) = kw(&kv, "seed") {
        st.rng = Rng::new(num(v, ln)? as u64);
    }
    let p = kw(&kv, "flip").map(|v| num(v, ln)).transpose()?.unwrap_or(0.03);
    if !(0.0..0.5).contains(&p) {
        return err(ln, "flip must be at least 0 and below 0.5");
    }
    let l = coded_ldpc(sp.n, sp.rate, sp.cb);
    let info: Vec<u8> = (0..l.info.len()).map(|_| (st.rng.unit() < 0.5) as u8).collect();
    let sent = l.encode(&info);
    let recv: Vec<u8> = sent.iter().map(|&b| b ^ (st.rng.unit() < p) as u8).collect();
    let flips = sent.iter().zip(&recv).filter(|(a, b)| a != b).count();
    let pe = p.max(1e-6);
    let start = m.notes[&note(name)].0[4] as usize;
    let key = note(name);
    // channel leans also go on the model's own code-bit things (replacing earlier channel leans)
    let old: Vec<f64> = m.notes.get(&format!("{}:lean", key)).map(|x| x.0.clone()).unwrap_or_else(|| vec![0.0; sp.n]);
    let lean: Vec<f64> = recv.iter().map(|&b| hard_lean(b, pe)).collect();
    for i in 0..sp.n {
        m.h[start + i] += lean[i] - old[i];
    }
    m.notes.insert(format!("{}:lean", key), (lean, Vec::new()));
    m.notes.insert(format!("{}:sent", key), (sent.iter().map(|&b| b as f64).collect(), Vec::new()));
    m.notes.insert(format!("{}:recv", key), (recv.iter().map(|&b| b as f64).collect(), vec![p.to_string()]));
    ctx.say(format!("{}: sent {} bits ({} message bits), the channel flipped {}", name, sp.n, l.info.len(), flips));
    Ok(())
}

fn report(ctx: &mut Ctx, name: &str, how: &str, got: Option<&[u8]>, sent: &[u8], broken: usize) {
    match got {
        Some(b) => {
            let wrong = b.iter().zip(sent).filter(|(a, c)| a != c).count();
            let verdict = if wrong == 0 { "the sent codeword" } else { "a DIFFERENT codeword (miscorrection)" };
            ctx.say(format!("{} {}: CODEWORD, {}; {} bits differ from what was sent", name, how, verdict, wrong));
        }
        None => ctx.say(format!("{} {}: REFUSED, {} checks still broken", name, how, broken)),
    }
}

fn decode_stmt(m: &Model, st: &mut State, name: &str, rest: &[Tok], ln: usize, ctx: &mut Ctx) -> Result<(), SettleError> {
    let sp = spec(m, name).unwrap();
    let key = note(name);
    let Some((sent, _)) = m.notes.get(&format!("{}:sent", key)) else {
        return err(ln, format!("{}.decode needs {}.transmit first", name, name));
    };
    let sent: Vec<u8> = sent.iter().map(|&v| v as u8).collect();
    let (recv, pw) = m.notes[&format!("{}:recv", key)].clone();
    let recv: Vec<u8> = recv.iter().map(|&v| v as u8).collect();
    let kv = kwargs(rest, ln)?;
    only(&kv, &["start", "sweeps", "seed", "hot", "cold"], "decode", ln)?;
    if let Some(v) = kw(&kv, "seed") {
        st.rng = Rng::new(num(v, ln)? as u64);
    }
    let from = match kw(&kv, "start") {
        Some(t) => sym(t, "start:", ln)?,
        None => "received".into(),
    };
    if from != "received" && from != "random" {
        return err(ln, "start is :received or :random");
    }
    let sweeps = whole(kw(&kv, "sweeps").map(|v| num(v, ln)).transpose()?.unwrap_or(400.0), 0.0, f64::INFINITY, "sweeps:", ln)?;
    let default_hot = if from == "random" { 10.0 } else { 1.0 };
    let hot = kw(&kv, "hot").map(|v| num(v, ln)).transpose()?.unwrap_or(default_hot);
    let cold = kw(&kv, "cold").map(|v| num(v, ln)).transpose()?.unwrap_or(0.05);
    if !(cold > 0.0 && hot >= cold) {
        return err(ln, "need hot >= cold > 0");
    }
    let l = coded_ldpc(sp.n, sp.rate, sp.cb);
    let mut sc = SettleCode::from_ldpc(&l, sp.gadget, sp.strength);
    let p: f64 = pw[0].parse().unwrap_or(0.03);
    let lean: Vec<f64> = recv.iter().map(|&b| hard_lean(b, p.max(1e-6))).collect();
    sc.set_leans(&lean);
    let seed = st.rng.next_u64();
    let init = if from == "received" { Some(&recv[..]) } else { None };
    let d = sc.decode(init, sweeps, hot, cold, seed);
    report(ctx, name, &format!("settled from {}", from), if d.codeword { Some(&d.bits) } else { None }, &sent, d.broken);
    Ok(())
}

fn bp_stmt(m: &Model, name: &str, ctx: &mut Ctx) -> Result<(), SettleError> {
    let sp = spec(m, name).unwrap();
    let key = note(name);
    let Some((sent, _)) = m.notes.get(&format!("{}:sent", key)) else {
        return err(0, format!("{}.decode_bp needs {}.transmit first", name, name));
    };
    let sent: Vec<u8> = sent.iter().map(|&v| v as u8).collect();
    let (recv, pw) = m.notes[&format!("{}:recv", key)].clone();
    let p: f64 = pw[0].parse().unwrap_or(0.03);
    let l = coded_ldpc(sp.n, sp.rate, sp.cb);
    let llr: Vec<f64> = recv.iter().map(|&v| llr_of_lean(hard_lean(v as u8, p.max(1e-6)))).collect();
    let got = l.decode(&llr, 50);
    let broken = l.rows.len();
    report(ctx, name, "by belief propagation", got.as_deref(), &sent, broken);
    Ok(())
}

impl Ext for LdpcSettle {
    fn name(&self) -> &'static str {
        "ldpcsettle"
    }

    fn statements(&self) -> &'static [&'static str] {
        &[
            "model: ldpc :c, bits: 64, rate: 0.5, gadget: :chain, strength: 2, codebook_seed: 1",
            "run: c.transmit flip: 0.03, seed: 1",
            "run: c.decode start: :received, sweeps: 400, hot: 1, cold: 0.05, seed: 2",
            "run: c.decode_bp",
        ]
    }

    fn model_stmt(&self, m: &mut Model, t: &[Tok], ln: usize, _ctx: &mut Ctx) -> Claim {
        match t {
            [Tok::Ident(k), Tok::Sym(name), rest @ ..] if k == "ldpc" => {
                let rest = if rest.first() == Some(&Tok::Comma) { &rest[1..] } else { rest };
                Some(declare(m, name, rest, ln))
            }
            _ => None,
        }
    }

    fn run_stmt(&self, m: &mut Model, st: &mut State, t: &[Tok], ln: usize, ctx: &mut Ctx) -> Claim {
        match t {
            [Tok::Ident(name), Tok::Dot, Tok::Ident(v), rest @ ..] if m.notes.contains_key(&note(name)) => match v.as_str() {
                "transmit" => Some(transmit(m, st, name, rest, ln, ctx)),
                "decode" => Some(decode_stmt(m, st, name, rest, ln, ctx)),
                "decode_bp" if rest.is_empty() => Some(bp_stmt(m, name, ctx)),
                _ => None,
            },
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interp::Interp;

    /// Every data pattern of one check of weight w, every helper assignment: zero penalty exactly on an even
    /// check with exactly one helper assignment, and at least 1 (lambda = 1) on every other arrangement.
    fn prove_one_check(w: usize, g: Gadget) {
        let row: Vec<usize> = (0..w).collect();
        let sc = SettleCode::new(&[row], w, &[], g, 1.0);
        let h = sc.helpers();
        assert!(w + h <= 26, "too many to enumerate");
        for data in 0u64..(1 << w) {
            let even = data.count_ones() % 2 == 0;
            let mut zeros = 0;
            let mut min_nonzero = f64::INFINITY;
            for hb in 0u64..(1 << h) {
                let y: Vec<bool> = (0..w).map(|i| (data >> i) & 1 == 1).chain((0..h).map(|k| (hb >> k) & 1 == 1)).collect();
                let e = sc.penalty(&y);
                assert!(e > -1e-9, "penalty below zero");
                if e.abs() < 1e-9 {
                    zeros += 1;
                } else {
                    min_nonzero = min_nonzero.min(e);
                }
            }
            assert_eq!(zeros, if even { 1 } else { 0 }, "w {} {:?} data {:b}", w, g, data);
            assert!(min_nonzero >= 1.0 - 1e-9, "w {} {:?}: a broken arrangement costs {}", w, g, min_nonzero);
            // the completed helpers of an even check are the unique zero
            let bits: Vec<u8> = (0..w).map(|i| ((data >> i) & 1) as u8).collect();
            let y = sc.complete(&bits);
            let e = sc.penalty(&y);
            if even {
                assert!(e.abs() < 1e-9);
            } else {
                assert!((e - 1.0).abs() < 1e-9, "an odd check completes at the lowest penalty, 1");
            }
        }
    }

    #[test]
    fn every_single_check_gadget_is_zero_exactly_on_even_parity() {
        for w in 1..=12 {
            prove_one_check(w, Gadget::Sum);
        }
        for w in 1..=9 {
            prove_one_check(w, Gadget::Chain);
        }
    }

    /// A whole small code, every arrangement of every thing: the zero-penalty arrangements are exactly the
    /// codewords (by their code bits), one each.
    #[test]
    fn a_small_code_has_exactly_its_codewords_at_zero_penalty() {
        let codes: Vec<(usize, Vec<Vec<usize>>)> = vec![
            (7, vec![vec![0, 1, 2, 4], vec![0, 1, 3, 5], vec![0, 2, 3, 6]]), // Hamming(7,4)
            (8, vec![vec![0, 1, 2], vec![2, 3, 4], vec![4, 5, 6, 7], vec![0, 7]]),
            (9, vec![vec![0, 1, 2, 3, 4], vec![3, 5, 6], vec![1, 6, 7, 8]]),
        ];
        for (n, rows) in codes {
            for g in [Gadget::Sum, Gadget::Chain] {
                let sc = SettleCode::new(&rows, n, &[], g, 1.0);
                let total = sc.things;
                assert!(total <= 22);
                let mut zero_words = std::collections::HashSet::new();
                let mut zero_count = 0;
                for a in 0u64..(1 << total) {
                    let y: Vec<bool> = (0..total).map(|i| (a >> i) & 1 == 1).collect();
                    let e = sc.penalty(&y);
                    if e.abs() < 1e-9 {
                        zero_count += 1;
                        zero_words.insert(a & ((1 << n) - 1));
                    } else {
                        assert!(e >= 1.0 - 1e-9);
                    }
                }
                let codewords: Vec<u64> = (0u64..(1 << n))
                    .filter(|&c| rows.iter().all(|r| r.iter().filter(|&&j| (c >> j) & 1 == 1).count() % 2 == 0))
                    .collect();
                assert_eq!(zero_count, codewords.len(), "{:?}: one zero arrangement per codeword", g);
                for c in codewords {
                    assert!(zero_words.contains(&c));
                }
            }
        }
    }

    /// The SETTLE model energy and the QUBO penalty differ by one constant over every arrangement.
    #[test]
    fn model_energy_matches_the_penalty_up_to_a_constant() {
        let rows = vec![vec![0, 1, 2, 3, 4], vec![2, 5, 6]];
        for g in [Gadget::Sum, Gadget::Chain] {
            let sc = SettleCode::new(&rows, 7, &[], g, 1.7);
            let mut c: Option<f64> = None;
            for a in 0u64..(1 << sc.things) {
                let y: Vec<bool> = (0..sc.things).map(|i| (a >> i) & 1 == 1).collect();
                let s: Vec<f64> = y.iter().map(|&v| if v { 1.0 } else { -1.0 }).collect();
                let d = sc.penalty(&y) - sc.energy(&s);
                match c {
                    None => c = Some(d),
                    Some(c0) => assert!((d - c0).abs() < 1e-9),
                }
            }
        }
    }

    #[test]
    fn at_zero_noise_the_settle_decoder_returns_the_sent_codeword() {
        let l = coded_ldpc(128, 0.5, 1);
        for g in [Gadget::Sum, Gadget::Chain] {
            let mut sc = SettleCode::from_ldpc(&l, g, 2.0 * (0.98f64 / 0.02).ln());
            let mut r = Rng::new(9);
            for t in 0..5 {
                let info: Vec<u8> = (0..l.info.len()).map(|_| (r.unit() < 0.5) as u8).collect();
                let c = l.encode(&info);
                sc.set_leans(&c.iter().map(|&b| hard_lean(b, 0.01)).collect::<Vec<_>>());
                let d = sc.decode(Some(&c), 60, 1.0, 0.05, t);
                assert!(d.codeword && d.bits == c, "{:?} warm", g);
            }
        }
    }

    #[test]
    fn a_non_codeword_target_is_never_claimed() {
        // Leans far stronger than the penalty pin a non-codeword as the calmest arrangement.
        let l = coded_ldpc(128, 0.5, 1);
        let mut sc = SettleCode::from_ldpc(&l, Gadget::Chain, 0.2);
        let mut r = Rng::new(3);
        for t in 0..5 {
            let target: Vec<u8> = (0..128).map(|j| if l.fixed_zero.contains(&j) { 0 } else { (r.unit() < 0.5) as u8 }).collect();
            assert!(sc.broken(&target) > 0);
            sc.set_leans(&target.iter().map(|&b| hard_lean(b, 1e-4)).collect::<Vec<_>>());
            let d = sc.decode(Some(&target), 30, 0.5, 0.05, t);
            assert_eq!(d.bits, target, "the leans win, so the calmest is the target");
            assert!(!d.codeword, "and the decoder refuses it");
        }
    }

    #[test]
    fn a_few_flips_are_corrected_from_the_received_word() {
        let l = coded_ldpc(128, 0.5, 1);
        let p: f64 = 0.02;
        let mut sc = SettleCode::from_ldpc(&l, Gadget::Chain, 2.0 * ((1.0 - p) / p).ln());
        let mut r = Rng::new(5);
        let info: Vec<u8> = (0..l.info.len()).map(|_| (r.unit() < 0.5) as u8).collect();
        let c = l.encode(&info);
        let mut y = c.clone();
        for j in [3, 40, 90] {
            if !l.fixed_zero.contains(&j) {
                y[j] ^= 1;
            }
        }
        sc.set_leans(&y.iter().map(|&b| hard_lean(b, p)).collect::<Vec<_>>());
        let d = sc.decode(Some(&y), 300, 1.0, 0.05, 1);
        assert!(d.codeword && d.bits == c);
    }

    #[test]
    fn soft_leans_have_the_sealed_shapes() {
        assert!((soft_lean_scaled(1.0, 0.02) - (0.96f64).atanh()).abs() < 1e-12);
        assert_eq!(soft_lean_scaled(0.0, 0.02), 0.0);
        assert!((soft_lean_laplace(1.0, 10) - 0.5 * 11f64.ln()).abs() < 1e-12);
        assert!(soft_lean_laplace(0.0, 10).abs() < 1e-12);
        assert!((llr_of_lean(hard_lean(0, 0.1)) - (0.9f64 / 0.1).ln()).abs() < 1e-9);
    }

    #[test]
    fn the_statements_run() {
        let src = "model :line do
  ldpc :c, bits: 64, rate: 0.5, gadget: :chain, strength: 8
end
run :line do
  c.transmit flip: 0.0, seed: 1
  c.decode start: :received, sweeps: 50, seed: 2
  c.decode_bp
end";
        let mut it = Interp::default();
        let out = it.exec(src).unwrap_or_else(|e| panic!("{}", e));
        let joined = out.join("\n");
        assert!(joined.contains("flipped 0"), "{}", joined);
        assert!(joined.contains("settled from received: CODEWORD, the sent codeword"), "{}", joined);
        assert!(joined.contains("by belief propagation: CODEWORD, the sent codeword"), "{}", joined);
        let bad = "model :x do\n  ldpc :c, gadget: :star\nend";
        assert!(Interp::default().exec(bad).is_err());
    }
}
