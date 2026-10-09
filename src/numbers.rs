//! NUMBERS: real-valued things joined by springs, settled by noisy drift. The mean position solves a linear
//! system and the spread of the positions gives the inverse matrix (thermodynamic linear algebra).
//!
//! ```text
//! model :bowl do
//!   number :x, :y, :z
//!   x.springs :y, by: 0.5        # a spring pulling x and y together (energy 0.5/2 * (x - y)^2)
//!   y.opposes :z, by: 0.25       # a spring pulling y towards -z (energy 0.25/2 * (y + z)^2)
//!   x.leans_to 2.0, by: 1        # a spring tying x to the value 2.0 (energy 1/2 * (x - 2)^2)
//! end
//! run :bowl do
//!   drift 200_000, step: 0.01, temperature: 1, seed: 1
//!   means                         # settled averages, with standard errors, against an exact solve
//!   spread                        # the covariance; divided by the temperature it is the inverse matrix
//!   solve :a, :b, matrix: "2 1; 1 3", target: "1 2"   # springs for A x = b, settled and checked
//! end
//! ```
//!
//! The springs make an energy U(x) = x.A.x / 2 - b.x. `drift` runs the overdamped Langevin (Ornstein-Uhlenbeck)
//! update x <- x - step * (A x - b) + sqrt(2 * temperature * step) * noise, from x = 0. Its long-run average is
//! A^-1 b exactly, and its covariance is temperature * A^-1 * (I - step*A/2)^-1, which `spread` corrects for.
//!
//! State: the springs live in `Model.notes["numbers"]` (A row-major then b, names as the words) and the last
//! drift in `Model.notes["numbers:drift"]`. The +-1 `State` of a run is never touched: numbers and things are
//! separate worlds that share a program. `means` and `spread` report the most recent drift or solve of the model.
//!
//! Refusals: a non-symmetric matrix is not a set of springs and `solve` refuses it. A symmetric matrix that is not
//! positive definite has no valley; the drift is still run (it blows up or wanders) and the result is reported as
//! "did not settle", never as solved. A Cholesky factorisation certifies positive definiteness.

use crate::ext::{Claim, Ctx, Ext};
use crate::lex::{err, kw, kwargs, num, only, text, SettleError, Tok, whole};
use crate::model::{Model, State};
use crate::rng::Rng;
use std::time::Instant;

pub struct Numbers;

const NET: &str = "numbers";
const LAST: &str = "numbers:drift";
/// A coordinate beyond this size (or not finite) means the springs have blown up.
pub const BLOWUP: f64 = 1e12;

// ---------------------------------------------------------------------------------------------------------
// The springs of a model
// ---------------------------------------------------------------------------------------------------------

/// d real-valued numbers, a symmetric stiffness matrix `a` (row-major d x d) and pushes `b`.
#[derive(Clone, Debug, Default)]
pub struct Springs {
    pub names: Vec<String>,
    pub a: Vec<f64>,
    pub b: Vec<f64>,
}

impl Springs {
    pub fn len(&self) -> usize {
        self.names.len()
    }
    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }
    pub fn load(m: &Model) -> Springs {
        match m.notes.get(NET) {
            None => Springs::default(),
            Some((nums, words)) => {
                let d = words.len();
                Springs { names: words.clone(), a: nums[..d * d].to_vec(), b: nums[d * d..d * d + d].to_vec() }
            }
        }
    }
    pub fn save(&self, m: &mut Model) {
        let mut nums = self.a.clone();
        nums.extend_from_slice(&self.b);
        m.notes.insert(NET.to_string(), (nums, self.names.clone()));
    }
    pub fn find(&self, name: &str) -> Option<usize> {
        self.names.iter().position(|n| n == name)
    }
    pub fn need(&self, name: &str, ln: usize) -> Result<usize, SettleError> {
        self.find(name).ok_or_else(|| SettleError(format!("line {}: unknown number :{} (declare it with: number :{})", ln, name, name)))
    }
    /// Declare a number (or return the existing one); the matrix grows by a zero row and column.
    pub fn add(&mut self, name: &str) -> usize {
        if let Some(i) = self.find(name) {
            return i;
        }
        let d = self.len();
        let mut a = vec![0.0; (d + 1) * (d + 1)];
        for i in 0..d {
            for k in 0..d {
                a[i * (d + 1) + k] = self.a[i * d + k];
            }
        }
        self.a = a;
        self.b.push(0.0);
        self.names.push(name.to_string());
        d
    }
}

// ---------------------------------------------------------------------------------------------------------
// Exact linear algebra (the comparison every drift is checked against)
// ---------------------------------------------------------------------------------------------------------

/// Solve A x = b by Gaussian elimination with partial pivoting. `None` if A is singular.
pub fn gauss_solve(a: &[f64], b: &[f64], d: usize) -> Option<Vec<f64>> {
    let mut m = a.to_vec();
    let mut x = b.to_vec();
    let scale = a.iter().fold(0.0f64, |s, v| s.max(v.abs())).max(1e-300);
    for c in 0..d {
        let p = (c..d).max_by(|&i, &k| m[i * d + c].abs().total_cmp(&m[k * d + c].abs()))?;
        if m[p * d + c].abs() <= 1e-14 * scale {
            return None;
        }
        if p != c {
            for k in 0..d {
                m.swap(p * d + k, c * d + k);
            }
            x.swap(p, c);
        }
        let piv = m[c * d + c];
        for i in c + 1..d {
            let f = m[i * d + c] / piv;
            if f != 0.0 {
                for k in c..d {
                    m[i * d + k] -= f * m[c * d + k];
                }
                x[i] -= f * x[c];
            }
        }
    }
    for c in (0..d).rev() {
        let mut s = x[c];
        for k in c + 1..d {
            s -= m[c * d + k] * x[k];
        }
        x[c] = s / m[c * d + c];
    }
    Some(x)
}

/// The inverse of A, column by column. `None` if singular.
pub fn gauss_inverse(a: &[f64], d: usize) -> Option<Vec<f64>> {
    let mut inv = vec![0.0; d * d];
    for c in 0..d {
        let mut e = vec![0.0; d];
        e[c] = 1.0;
        let col = gauss_solve(a, &e, d)?;
        for i in 0..d {
            inv[i * d + c] = col[i];
        }
    }
    Some(inv)
}

/// Positive-definiteness certificate by Cholesky factorisation. `Err(row)` names the first row whose pivot is
/// not clearly positive (a zero or negative stiffness direction: the springs have no valley).
pub fn cholesky_certificate(a: &[f64], d: usize) -> Result<(), usize> {
    let scale = (0..d).map(|i| a[i * d + i].abs()).fold(0.0f64, f64::max).max(1e-300);
    let mut l = vec![0.0; d * d];
    for i in 0..d {
        for k in 0..=i {
            let mut s = a[i * d + k];
            for j in 0..k {
                s -= l[i * d + j] * l[k * d + j];
            }
            if i == k {
                if s <= 1e-10 * scale {
                    return Err(i);
                }
                l[i * d + i] = s.sqrt();
            } else {
                l[i * d + k] = s / l[k * d + k];
            }
        }
    }
    Ok(())
}

/// Is the matrix symmetric? `Err((i, k))` names the first entry pair that differs.
pub fn symmetry(a: &[f64], d: usize) -> Result<(), (usize, usize)> {
    for i in 0..d {
        for k in i + 1..d {
            let (p, q) = (a[i * d + k], a[k * d + i]);
            if (p - q).abs() > 1e-12 * (1.0 + p.abs().max(q.abs())) {
                return Err((i, k));
            }
        }
    }
    Ok(())
}

/// Largest stiffness (largest eigenvalue magnitude) by power iteration, for the step-size diagnostic.
pub fn stiffest(a: &[f64], d: usize) -> f64 {
    let mut v: Vec<f64> = (0..d).map(|i| 1.0 + 0.1 * i as f64).collect();
    let mut lam = 0.0;
    for _ in 0..200 {
        let w: Vec<f64> = (0..d).map(|i| (0..d).map(|k| a[i * d + k] * v[k]).sum()).collect();
        let n = w.iter().map(|x| x * x).sum::<f64>().sqrt();
        if n == 0.0 {
            return 0.0;
        }
        lam = n / v.iter().map(|x| x * x).sum::<f64>().sqrt();
        v = w.iter().map(|x| x / n).collect();
    }
    lam
}

pub fn matmul(p: &[f64], q: &[f64], d: usize) -> Vec<f64> {
    let mut r = vec![0.0; d * d];
    for i in 0..d {
        for j in 0..d {
            let pij = p[i * d + j];
            if pij != 0.0 {
                for k in 0..d {
                    r[i * d + k] += pij * q[j * d + k];
                }
            }
        }
    }
    r
}

/// Relative Frobenius distance |p - q| / |q|.
pub fn rel_frob(p: &[f64], q: &[f64]) -> f64 {
    let num: f64 = p.iter().zip(q).map(|(x, y)| (x - y) * (x - y)).sum();
    let den: f64 = q.iter().map(|y| y * y).sum();
    (num / den.max(1e-300)).sqrt()
}

pub fn norm(v: &[f64]) -> f64 {
    v.iter().map(|x| x * x).sum::<f64>().sqrt()
}

/// The inverse estimated from a spread: covariance / temperature is A^-1 (I - step*A/2)^-1, so multiply back by
/// (I - step*A/2) to remove the step's bias.
pub fn inverse_from_spread(cov: &[f64], a: &[f64], d: usize, dt: f64, temp: f64) -> Vec<f64> {
    let raw: Vec<f64> = cov.iter().map(|c| c / temp).collect();
    let mut fix = vec![0.0; d * d];
    for i in 0..d {
        for k in 0..d {
            fix[i * d + k] = if i == k { 1.0 } else { 0.0 } - 0.5 * dt * a[i * d + k];
        }
    }
    matmul(&raw, &fix, d)
}

// ---------------------------------------------------------------------------------------------------------
// The drift: overdamped Langevin (Ornstein-Uhlenbeck) settling by Euler-Maruyama
// ---------------------------------------------------------------------------------------------------------

pub struct Walk {
    pub steps: usize,
    pub dt: f64,
    pub temp: f64,
    pub seed: u64,
    /// Steps discarded before averaging.
    pub burn: usize,
    /// Accumulate the covariance (costs d^2 per kept step).
    pub want_cov: bool,
    /// Steps at which to record the running sum of positions from step 1 (for error-versus-time curves).
    pub marks: Vec<usize>,
}

pub struct Walked {
    pub mean: Vec<f64>,
    /// Batch-means standard error of each mean (20 batches).
    pub se: Vec<f64>,
    pub cov: Vec<f64>,
    pub kept: usize,
    /// The step at which a coordinate left the finite range, if it did.
    pub blew: Option<usize>,
    /// (step, sum of positions over steps 1..=step).
    pub sums: Vec<(usize, Vec<f64>)>,
    /// Position at the last step taken.
    pub last: Vec<f64>,
    pub secs: f64,
}

/// Run the drift from x = 0: x <- x - dt (A x - b) + sqrt(2 T dt) * normal, one normal per number per step.
pub fn walk(a: &[f64], b: &[f64], d: usize, w: &Walk) -> Walked {
    let t0 = Instant::now();
    let mut rng = Rng::new(w.seed);
    let sig = (2.0 * w.temp * w.dt).sqrt();
    let mut x = vec![0.0; d];
    let mut f = vec![0.0; d];
    let mut sum = vec![0.0; d];
    let mut ksum = vec![0.0; d];
    let mut ksq = if w.want_cov { vec![0.0; d * d] } else { Vec::new() };
    let kept_total = w.steps.saturating_sub(w.burn);
    let nb = 20usize;
    let bsize = (kept_total / nb).max(1);
    let mut bsum = vec![0.0; d];
    let mut bmeans: Vec<Vec<f64>> = Vec::new();
    let mut inb = 0usize;
    let mut kept = 0usize;
    let mut marks = w.marks.clone();
    marks.sort_unstable();
    let mut mi = 0usize;
    let mut sums = Vec::new();
    let mut blew = None;
    for s in 1..=w.steps {
        for i in 0..d {
            let row = &a[i * d..(i + 1) * d];
            let mut g = -b[i];
            for k in 0..d {
                g += row[k] * x[k];
            }
            f[i] = g;
        }
        let mut bad = false;
        for i in 0..d {
            x[i] -= w.dt * f[i];
            if sig > 0.0 {
                x[i] += sig * rng.normal();
            }
            if !x[i].is_finite() || x[i].abs() > BLOWUP {
                bad = true;
            }
            sum[i] += x[i];
        }
        if bad {
            blew = Some(s);
            break;
        }
        while mi < marks.len() && marks[mi] == s {
            sums.push((s, sum.clone()));
            mi += 1;
        }
        if s > w.burn {
            kept += 1;
            for i in 0..d {
                ksum[i] += x[i];
                bsum[i] += x[i];
            }
            if w.want_cov {
                for i in 0..d {
                    let xi = x[i];
                    let row = &mut ksq[i * d..(i + 1) * d];
                    for k in i..d {
                        row[k] += xi * x[k];
                    }
                }
            }
            inb += 1;
            if inb == bsize && bmeans.len() < nb {
                bmeans.push(bsum.iter().map(|v| v / bsize as f64).collect());
                bsum.iter_mut().for_each(|v| *v = 0.0);
                inb = 0;
            }
        }
    }
    let n = kept.max(1) as f64;
    let mean: Vec<f64> = ksum.iter().map(|v| v / n).collect();
    let mut cov = vec![0.0; if w.want_cov { d * d } else { 0 }];
    if w.want_cov {
        for i in 0..d {
            for k in i..d {
                let c = ksq[i * d + k] / n - mean[i] * mean[k];
                cov[i * d + k] = c;
                cov[k * d + i] = c;
            }
        }
    }
    let se: Vec<f64> = (0..d)
        .map(|i| {
            let m = bmeans.len();
            if m < 2 {
                return f64::NAN;
            }
            let bm: f64 = bmeans.iter().map(|v| v[i]).sum::<f64>() / m as f64;
            let var: f64 = bmeans.iter().map(|v| (v[i] - bm) * (v[i] - bm)).sum::<f64>() / (m - 1) as f64;
            (var / m as f64).sqrt()
        })
        .collect();
    Walked { mean, se, cov, kept, blew, sums, last: x, secs: t0.elapsed().as_secs_f64() }
}

// ---------------------------------------------------------------------------------------------------------
// The last drift of a model, kept in the notes
// ---------------------------------------------------------------------------------------------------------

struct Last {
    names: Vec<String>,
    steps: usize,
    dt: f64,
    temp: f64,
    settled: bool,
    a: Vec<f64>,
    b: Vec<f64>,
    mean: Vec<f64>,
    se: Vec<f64>,
    cov: Vec<f64>,
}

impl Last {
    fn save(&self, m: &mut Model) {
        let mut v = vec![self.steps as f64, self.dt, self.temp, if self.settled { 1.0 } else { 0.0 }];
        for part in [&self.a, &self.b, &self.mean, &self.se, &self.cov] {
            v.extend_from_slice(part);
        }
        m.notes.insert(LAST.to_string(), (v, self.names.clone()));
    }
    fn load(m: &Model, what: &str, ln: usize) -> Result<Last, SettleError> {
        let (v, names) = match m.notes.get(LAST) {
            Some(x) => x,
            None => return err(ln, format!("{} needs a drift or a solve first", what)),
        };
        let d = names.len();
        let mut at = 4;
        let mut take = |n: usize| {
            let s = v[at..at + n].to_vec();
            at += n;
            s
        };
        let (a, b, mean, se, cov) = (take(d * d), take(d), take(d), take(d), take(d * d));
        let last = Last { names: names.clone(), steps: v[0] as usize, dt: v[1], temp: v[2], settled: v[3] > 0.5, a, b, mean, se, cov };
        if !last.settled {
            return err(ln, format!("{}: the last drift did not settle, so there is nothing to report", what));
        }
        Ok(last)
    }
}

// ---------------------------------------------------------------------------------------------------------
// Statements
// ---------------------------------------------------------------------------------------------------------

fn declare(m: &mut Model, rest: &[Tok], ln: usize) -> Result<(), SettleError> {
    let mut sp = Springs::load(m);
    let mut any = false;
    for t in rest {
        match t {
            Tok::Sym(s) => {
                sp.add(s);
                any = true;
            }
            Tok::Comma => {}
            other => return err(ln, format!("unexpected `{}` in number (write: number :x, :y)", other)),
        }
    }
    if !any {
        return err(ln, "number needs at least one name, like: number :x");
    }
    sp.save(m);
    Ok(())
}

fn by_arg(rest: &[Tok], verb: &str, ln: usize) -> Result<f64, SettleError> {
    let kv = kwargs(rest, ln)?;
    only(&kv, &["by"], verb, ln)?;
    match kw(&kv, "by") {
        Some(v) => num(v, ln),
        None => err(ln, format!("{} needs `by:`", verb)),
    }
}

/// `x.springs :y, by: k` (energy k/2 (x-y)^2) and `x.opposes :y, by: k` (energy k/2 (x+y)^2).
fn spring(m: &mut Model, x: &str, verb: &str, y: &str, rest: &[Tok], ln: usize) -> Result<(), SettleError> {
    let mut sp = Springs::load(m);
    let (i, k) = (sp.need(x, ln)?, sp.need(y, ln)?);
    if i == k {
        return err(ln, "a number cannot spring to itself; use leans_to for a spring to a fixed value");
    }
    let by = by_arg(rest, verb, ln)?;
    let d = sp.len();
    let off = if verb == "springs" { -by } else { by };
    sp.a[i * d + i] += by;
    sp.a[k * d + k] += by;
    sp.a[i * d + k] += off;
    sp.a[k * d + i] += off;
    sp.save(m);
    Ok(())
}

/// `x.leans_to v, by: k`: a spring of strength k tying x to the value v (energy k/2 (x-v)^2).
fn lean(m: &mut Model, x: &str, v: f64, rest: &[Tok], ln: usize) -> Result<(), SettleError> {
    let mut sp = Springs::load(m);
    let i = sp.need(x, ln)?;
    let by = by_arg(rest, "leans_to", ln)?;
    let d = sp.len();
    sp.a[i * d + i] += by;
    sp.b[i] += by * v;
    sp.save(m);
    Ok(())
}

struct DriftOpts {
    steps: usize,
    dt: f64,
    temp: f64,
    seed: u64,
    burn: Option<usize>,
}

fn drift_opts(kv: &[(String, Tok)], steps: usize, ln: usize) -> Result<DriftOpts, SettleError> {
    let mut o = DriftOpts { steps, dt: 0.01, temp: 1.0, seed: 1, burn: None };
    if let Some(v) = kw(kv, "step") {
        o.dt = num(v, ln)?;
        if o.dt <= 0.0 {
            return err(ln, "step must be above zero");
        }
    }
    if let Some(v) = kw(kv, "temperature") {
        o.temp = num(v, ln)?;
        if o.temp < 0.0 {
            return err(ln, "temperature cannot be below zero (zero means no shaking: plain relaxation)");
        }
    }
    if let Some(v) = kw(kv, "seed") {
        o.seed = num(v, ln)? as u64;
    }
    if let Some(v) = kw(kv, "burn") {
        o.burn = Some(whole(num(v, ln)?, 0.0, f64::INFINITY, "burn:", ln)?);
    }
    if let Some(v) = kw(kv, "steps") {
        o.steps = whole(num(v, ln)?, 0.0, f64::INFINITY, "steps:", ln)?;
    }
    if o.steps < 20 {
        return err(ln, "drift needs at least 20 steps");
    }
    Ok(o)
}

/// Drift the springs, report, and keep the result. Returns whether it settled.
fn settle_springs(m: &mut Model, sp: &Springs, o: &DriftOpts, ctx: &mut Ctx) -> bool {
    let d = sp.len();
    let burn = o.burn.unwrap_or(o.steps / 10).min(o.steps - 20);
    let w = Walk { steps: o.steps, dt: o.dt, temp: o.temp, seed: o.seed, burn, want_cov: true, marks: vec![] };
    let r = walk(&sp.a, &sp.b, d, &w);
    let cert = cholesky_certificate(&sp.a, d);
    let what_drift_did = match r.blew {
        Some(s) => format!("the drift blew up at step {}", s),
        None => format!("the drift ended with its largest number at {:.3e}", r.last.iter().fold(0.0f64, |s, v| s.max(v.abs()))),
    };
    let settled = match (&cert, r.blew) {
        (Err(row), _) => {
            ctx.say(format!(
                "did not settle: the springs have no valley (not positive definite; the stiffness fails at :{}); {}",
                sp.names[*row], what_drift_did
            ));
            false
        }
        (Ok(()), Some(s)) => {
            let lam = stiffest(&sp.a, d);
            ctx.say(format!(
                "did not settle: step {} is too large for the stiffest spring (stiffness {:.4}); the drift blew up at step {}; keep step below {:.4}",
                o.dt,
                lam,
                s,
                2.0 / lam
            ));
            false
        }
        (Ok(()), None) => {
            ctx.say(format!(
                "drifted: {} steps of {} numbers, step {}, time {}, temperature {}; averaged the last {} ({:.1} ms)",
                o.steps,
                d,
                o.dt,
                o.steps as f64 * o.dt,
                o.temp,
                r.kept,
                1e3 * r.secs
            ));
            true
        }
    };
    Last { names: sp.names.clone(), steps: o.steps, dt: o.dt, temp: o.temp, settled, a: sp.a.clone(), b: sp.b.clone(), mean: r.mean, se: r.se, cov: r.cov }
        .save(m);
    settled
}

fn drift_stmt(m: &mut Model, steps: usize, rest: &[Tok], ln: usize, ctx: &mut Ctx) -> Result<(), SettleError> {
    let kv = kwargs(rest, ln)?;
    only(&kv, &["step", "temperature", "seed", "burn"], "drift", ln)?;
    let o = drift_opts(&kv, steps, ln)?;
    let sp = Springs::load(m);
    if sp.is_empty() {
        return err(ln, "drift needs numbers; declare them in the model with: number :x");
    }
    if let Err((i, k)) = symmetry(&sp.a, sp.len()) {
        return err(ln, format!("the springs between :{} and :{} are not symmetric", sp.names[i], sp.names[k]));
    }
    settle_springs(m, &sp, &o, ctx);
    Ok(())
}

/// Print each settled mean against the exact solve; returns (largest error, relative error, largest |z|).
fn report_means(l: &Last, ctx: &mut Ctx, word: &str) -> (f64, f64, f64) {
    let d = l.names.len();
    let exact = gauss_solve(&l.a, &l.b, d).unwrap_or_else(|| vec![f64::NAN; d]);
    let (mut worst, mut zmax) = (0.0f64, 0.0f64);
    for i in 0..d {
        let off = l.mean[i] - exact[i];
        worst = worst.max(off.abs());
        let z = off.abs() / l.se[i];
        zmax = zmax.max(z);
        if d <= 16 {
            ctx.say(format!(
                "  {:<10} {} {:>10.4} ± {:.4}   exact {:>10.4}   off {:>+.4}",
                l.names[i], word, l.mean[i], l.se[i], exact[i], off
            ));
        }
    }
    let diff: Vec<f64> = l.mean.iter().zip(&exact).map(|(p, q)| p - q).collect();
    (worst, norm(&diff) / norm(&exact).max(1e-300), zmax)
}

fn means_stmt(m: &Model, ln: usize, ctx: &mut Ctx) -> Result<(), SettleError> {
    let l = Last::load(m, "means", ln)?;
    let (worst, rel, zmax) = report_means(&l, ctx, "mean");
    ctx.say(format!(
        "means: largest error {:.4}, relative error {:.3}% (exact by Gaussian elimination), largest error {:.1} standard errors",
        worst,
        100.0 * rel,
        zmax
    ));
    Ok(())
}

fn spread_stmt(m: &Model, ln: usize, ctx: &mut Ctx) -> Result<(), SettleError> {
    let l = Last::load(m, "spread", ln)?;
    if l.temp == 0.0 {
        return err(ln, "spread needs a drift above temperature zero; at zero the numbers do not shake");
    }
    let d = l.names.len();
    let inv = inverse_from_spread(&l.cov, &l.a, d, l.dt, l.temp);
    let raw: Vec<f64> = l.cov.iter().map(|c| c / l.temp).collect();
    let exact = gauss_inverse(&l.a, d).unwrap_or_else(|| vec![f64::NAN; d * d]);
    if d <= 6 {
        ctx.say("  covariance of the settled numbers:");
        for i in 0..d {
            let row: Vec<String> = (0..d).map(|k| format!("{:>9.4}", l.cov[i * d + k])).collect();
            ctx.say(format!("  {:<10}{}", l.names[i], row.join("")));
        }
        ctx.say("  inverse from the spread (covariance / temperature, step-corrected)  |  exact inverse:");
        for i in 0..d {
            let p: Vec<String> = (0..d).map(|k| format!("{:>9.4}", inv[i * d + k])).collect();
            let q: Vec<String> = (0..d).map(|k| format!("{:>9.4}", exact[i * d + k])).collect();
            ctx.say(format!("  {:<10}{}  | {}", l.names[i], p.join(""), q.join("")));
        }
    }
    ctx.say(format!(
        "spread: inverse from the spread against the exact inverse, relative error {:.2}% step-corrected, {:.2}% raw",
        100.0 * rel_frob(&inv, &exact),
        100.0 * rel_frob(&raw, &exact)
    ));
    Ok(())
}

/// Parse "4 1 0; 1 3 1; 0 1 2" (rows by `;` or newline, entries by spaces or commas).
fn parse_rows(s: &str, ln: usize) -> Result<Vec<Vec<f64>>, SettleError> {
    let mut rows = Vec::new();
    for r in s.split([';', '\n']) {
        let cells: Vec<&str> = r.split(|c: char| c.is_whitespace() || c == ',').filter(|t| !t.is_empty()).collect();
        if cells.is_empty() {
            continue;
        }
        let mut row = Vec::new();
        for c in cells {
            match c.replace('_', "").parse::<f64>() {
                Ok(v) => row.push(v),
                Err(_) => return err(ln, format!("'{}' is not a number", c)),
            }
        }
        rows.push(row);
    }
    Ok(rows)
}

fn solve_stmt(m: &mut Model, rest: &[Tok], ln: usize, ctx: &mut Ctx) -> Result<(), SettleError> {
    let mut i = 0;
    let mut names = Vec::new();
    while i < rest.len() {
        match &rest[i] {
            Tok::Sym(s) => names.push(s.clone()),
            Tok::Comma => {}
            Tok::Label(_) => break,
            other => return err(ln, format!("unexpected `{}` in solve", other)),
        }
        i += 1;
    }
    let kv = kwargs(&rest[i..], ln)?;
    only(&kv, &["matrix", "target", "steps", "step", "temperature", "seed", "burn"], "solve", ln)?;
    let d = names.len();
    if d == 0 {
        return err(ln, "solve needs the numbers to solve for, like: solve :x, :y, matrix: \"2 1; 1 3\", target: \"1 2\"");
    }
    let mat = match kw(&kv, "matrix") {
        Some(v) => parse_rows(&text(v, ln)?, ln)?,
        None => return err(ln, "solve needs `matrix:`"),
    };
    let tgt: Vec<f64> = match kw(&kv, "target") {
        Some(v) => parse_rows(&text(v, ln)?, ln)?.concat(),
        None => return err(ln, "solve needs `target:`"),
    };
    if mat.len() != d || mat.iter().any(|r| r.len() != d) {
        return err(ln, format!("matrix must be {} by {} for {} numbers", d, d, d));
    }
    if tgt.len() != d {
        return err(ln, format!("target must have {} entries", d));
    }
    let a: Vec<f64> = mat.concat();
    if let Err((p, q)) = symmetry(&a, d) {
        return err(
            ln,
            format!(
                "refused: the matrix is not symmetric (row {} column {} is {}, row {} column {} is {}). Springs pull both ways \
                 equally, so only a symmetric matrix is a set of springs; for a general A, solve A^T A x = A^T b instead",
                p + 1,
                q + 1,
                a[p * d + q],
                q + 1,
                p + 1,
                a[q * d + p]
            ),
        );
    }
    let o = drift_opts(&kv, 100_000, ln)?;
    // The named numbers get exactly these springs; ties to any other number are cut.
    let mut sp = Springs::load(m);
    let idx: Vec<usize> = names.iter().map(|n| sp.add(n)).collect();
    let full = sp.len();
    let mut cut = 0;
    for &p in &idx {
        for k in 0..full {
            if !idx.contains(&k) && (sp.a[p * full + k] != 0.0 || sp.a[k * full + p] != 0.0) {
                sp.a[p * full + k] = 0.0;
                sp.a[k * full + p] = 0.0;
                cut += 1;
            }
        }
    }
    for (r, &p) in idx.iter().enumerate() {
        for (c, &q) in idx.iter().enumerate() {
            sp.a[p * full + q] = a[r * d + c];
        }
        sp.b[p] = tgt[r];
    }
    sp.save(m);
    if cut > 0 {
        ctx.say(format!("solve: cut {} springs between the solved numbers and the others", cut));
    }
    let sub = Springs { names: names.clone(), a: a.clone(), b: tgt.clone() };
    ctx.say(format!(
        "solve: {} numbers by drifting {} steps (time {}) at temperature {}",
        d,
        o.steps,
        o.steps as f64 * o.dt,
        o.temp
    ));
    if !settle_springs(m, &sub, &o, ctx) {
        return Ok(());
    }
    let l = Last::load(m, "solve", ln)?;
    let (worst, rel, zmax) = report_means(&l, ctx, "settled");
    let t0 = Instant::now();
    let reps = 200;
    for _ in 0..reps {
        std::hint::black_box(gauss_solve(std::hint::black_box(&a), &tgt, d));
    }
    let gauss_ms = 1e3 * t0.elapsed().as_secs_f64() / reps as f64;
    let drift_ms = {
        let w = Walk { steps: o.steps, dt: o.dt, temp: o.temp, seed: o.seed, burn: o.steps / 10, want_cov: false, marks: vec![] };
        1e3 * walk(&a, &tgt, d, &w).secs
    };
    ctx.say(format!(
        "solved: largest error {:.4}, relative error {:.3}%, largest error {:.1} standard errors; drift {:.2} ms against exact elimination {:.4} ms",
        worst,
        100.0 * rel,
        zmax,
        drift_ms,
        gauss_ms
    ));
    Ok(())
}

impl Ext for Numbers {
    fn name(&self) -> &'static str {
        "numbers"
    }

    fn statements(&self) -> &'static [&'static str] {
        &[
            "model: number :x, :y, :z",
            "model: x.springs :y, by: 0.5   /   x.opposes :y, by: 0.5   /   x.leans_to 2.0, by: 1",
            "run: drift 20_000, step: 0.01, temperature: 1, seed: 1, burn: 2_000",
            "run: means   /   spread",
            "run: solve :x, :y, matrix: \"2 1; 1 3\", target: \"1 2\", steps: 100_000, step: 0.01",
        ]
    }

    fn model_stmt(&self, m: &mut Model, t: &[Tok], ln: usize, _ctx: &mut Ctx) -> Claim {
        match t {
            [Tok::Ident(k), rest @ ..] if k == "number" => Some(declare(m, rest, ln)),
            [Tok::Ident(x), Tok::Dot, Tok::Ident(verb), Tok::Sym(y), rest @ ..] if verb == "springs" || verb == "opposes" => {
                Some(spring(m, x, verb, y, rest, ln))
            }
            [Tok::Ident(x), Tok::Dot, Tok::Ident(verb), Tok::Num(v), rest @ ..] if verb == "leans_to" => Some(lean(m, x, *v, rest, ln)),
            _ => None,
        }
    }

    fn run_stmt(&self, m: &mut Model, _st: &mut State, t: &[Tok], ln: usize, ctx: &mut Ctx) -> Claim {
        Some(match t {
            [Tok::Ident(k), Tok::Num(n), rest @ ..] if k == "drift" => whole(*n, 0.0, f64::INFINITY, "drift", ln).and_then(|steps| drift_stmt(m, steps, rest, ln, ctx)),
            [Tok::Ident(k)] if k == "means" => means_stmt(m, ln, ctx),
            [Tok::Ident(k)] if k == "spread" => spread_stmt(m, ln, ctx),
            [Tok::Ident(k), rest @ ..] if k == "solve" => solve_stmt(m, rest, ln, ctx),
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interp::Interp;

    const BOWL: &str = "model :bowl do
  number :x, :y, :z
  x.springs :y, by: 0.5
  y.opposes :z, by: 0.25
  x.leans_to 2.0, by: 1
  y.leans_to -1.0, by: 0.5
  z.leans_to 0.5, by: 2
end";

    fn run(src: &str) -> Vec<String> {
        Interp::default().exec(src).unwrap_or_else(|e| panic!("{}", e))
    }

    fn lines_with<'a>(out: &'a [String], p: &str) -> Vec<&'a String> {
        out.iter().filter(|l| l.contains(p)).collect()
    }

    #[test]
    fn gaussian_elimination_is_exact_and_cholesky_certifies() {
        let a = [4.0, 1.0, 0.0, 1.0, 3.0, 1.0, 0.0, 1.0, 2.0];
        let x = gauss_solve(&a, &[1.0, 2.0, 3.0], 3).unwrap();
        let back: Vec<f64> = (0..3).map(|i| (0..3).map(|k| a[i * 3 + k] * x[k]).sum()).collect();
        assert!(back.iter().zip([1.0, 2.0, 3.0]).all(|(p, q)| (p - q).abs() < 1e-12));
        assert!(cholesky_certificate(&a, 3).is_ok());
        assert_eq!(cholesky_certificate(&[1.0, 2.0, 2.0, 1.0], 2), Err(1));
        assert!(gauss_solve(&[1.0, 2.0, 2.0, 4.0], &[1.0, 1.0], 2).is_none());
        let inv = gauss_inverse(&a, 3).unwrap();
        let id = matmul(&a, &inv, 3);
        for i in 0..3 {
            for k in 0..3 {
                assert!((id[i * 3 + k] - if i == k { 1.0 } else { 0.0 }).abs() < 1e-12);
            }
        }
    }

    #[test]
    fn the_springs_build_the_right_matrix() {
        let mut it = Interp::default();
        it.exec(BOWL).unwrap();
        let sp = Springs::load(&it.models["bowl"]);
        assert_eq!(sp.names, ["x", "y", "z"]);
        let want_a = [1.5, -0.5, 0.0, -0.5, 1.25, 0.25, 0.0, 0.25, 2.25];
        let want_b = [2.0, -0.5, 1.0];
        assert!(sp.a.iter().zip(want_a).all(|(p, q)| (p - q).abs() < 1e-12), "{:?}", sp.a);
        assert!(sp.b.iter().zip(want_b).all(|(p, q)| (p - q).abs() < 1e-12), "{:?}", sp.b);
    }

    #[test]
    fn the_drift_mean_is_the_solution_and_the_spread_is_the_step_biased_inverse() {
        let a = [1.5, -0.5, 0.0, -0.5, 1.25, 0.25, 0.0, 0.25, 2.25];
        let b = [2.0, -0.5, 1.0];
        let (dt, temp) = (0.05, 0.7);
        let w = Walk { steps: 2_000_000, dt, temp, seed: 7, burn: 1000, want_cov: true, marks: vec![] };
        let r = walk(&a, &b, 3, &w);
        let x = gauss_solve(&a, &b, 3).unwrap();
        for i in 0..3 {
            assert!((r.mean[i] - x[i]).abs() < 5.0 * r.se[i] + 1e-9, "mean {} {} se {}", r.mean[i], x[i], r.se[i]);
        }
        // the discrete stationary covariance is T A^-1 (I - dt A/2)^-1; the corrected estimate is A^-1
        let est = inverse_from_spread(&r.cov, &a, 3, dt, temp);
        let inv = gauss_inverse(&a, 3).unwrap();
        assert!(rel_frob(&est, &inv) < 0.02, "corrected {}", rel_frob(&est, &inv));
        let raw: Vec<f64> = r.cov.iter().map(|c| c / temp).collect();
        assert!(rel_frob(&raw, &inv) > rel_frob(&est, &inv), "the step correction should help at dt = 0.05");
    }

    #[test]
    fn zero_temperature_is_plain_relaxation_and_converges_exactly() {
        let a = [1.5, -0.5, 0.0, -0.5, 1.25, 0.25, 0.0, 0.25, 2.25];
        let b = [2.0, -0.5, 1.0];
        let w = Walk { steps: 5_000, dt: 0.1, temp: 0.0, seed: 1, burn: 4_999, want_cov: false, marks: vec![] };
        let r = walk(&a, &b, 3, &w);
        let x = gauss_solve(&a, &b, 3).unwrap();
        assert!(r.last.iter().zip(&x).all(|(p, q)| (p - q).abs() < 1e-10));
    }

    #[test]
    fn a_program_drifts_and_reports_means_and_spread() {
        let out = run(&format!("{}\nrun :bowl do\n  drift 400_000, step: 0.02, temperature: 1, seed: 3\n  means\n  spread\nend", BOWL));
        assert!(out[0].starts_with("drifted: 400000 steps of 3 numbers"), "{:?}", out);
        let m = lines_with(&out, "means: largest error");
        assert_eq!(m.len(), 1);
        let s = lines_with(&out, "spread: inverse from the spread");
        assert_eq!(s.len(), 1);
        let rel: f64 = s[0].split("relative error ").nth(1).unwrap().split('%').next().unwrap().parse().unwrap();
        assert!(rel < 5.0, "{}", s[0]);
    }

    #[test]
    fn solve_matches_the_exact_answer() {
        let src = "model :sys do\nend\nrun :sys do\n  solve :x, :y, :z, matrix: \"4 1 0; 1 3 1; 0 1 2\", target: \"1 2 3\", steps: 400_000, step: 0.02, seed: 2\nend";
        let out = run(src);
        let s = lines_with(&out, "solved:");
        assert_eq!(s.len(), 1, "{:?}", out);
        let worst: f64 = s[0].split("largest error ").nth(1).unwrap().split(',').next().unwrap().parse().unwrap();
        assert!(worst < 0.02, "{}", s[0]);
    }

    #[test]
    fn control_non_symmetric_matrix_is_refused() {
        let src = "model :sys do\nend\nrun :sys do\n  solve :x, :y, matrix: \"2 1; 0 3\", target: \"1 2\"\nend";
        let e = Interp::default().exec(src).err().unwrap().0;
        assert!(e.starts_with("line 4: refused: the matrix is not symmetric"), "{}", e);
    }

    #[test]
    fn control_non_positive_definite_is_never_solved() {
        // eigenvalues 3 and -1 (blows up), and a flat direction (eigenvalue 0: wanders, never blows up)
        for mat in ["1 2; 2 1", "1 1; 1 1", "2 0 0; 0 -0.001 0; 0 0 1"] {
            let d = mat.split(';').count();
            let tgt = vec!["1"; d].join(" ");
            let names: Vec<String> = (0..d).map(|i| format!(":n{}", i)).collect();
            let src = format!("model :s do\nend\nrun :s do\n  solve {}, matrix: \"{}\", target: \"{}\", steps: 50_000\nend", names.join(", "), mat, tgt);
            let out = run(&src);
            assert!(out.iter().all(|l| !l.starts_with("solved")), "{} gave {:?}", mat, out);
            assert!(out.iter().any(|l| l.starts_with("did not settle: the springs have no valley")), "{} gave {:?}", mat, out);
            // and means refuses afterwards
            let with_means = format!("{}  means\nend", src.strip_suffix("end").unwrap());
            let e = Interp::default().exec(&with_means).err().unwrap().0;
            assert!(e.contains("did not settle"), "{}", e);
        }
    }

    #[test]
    fn control_a_step_too_large_is_diagnosed_not_solved() {
        let src = "model :s do\nend\nrun :s do\n  solve :x, :y, matrix: \"40 1; 1 30\", target: \"1 1\", step: 0.1, steps: 10_000\nend";
        let out = run(src);
        assert!(out.iter().any(|l| l.starts_with("did not settle: step 0.1 is too large")), "{:?}", out);
        assert!(out.iter().all(|l| !l.starts_with("solved")));
    }

    #[test]
    fn negative_control_wrong_springs_disagree_with_the_exact_answer() {
        // the same instrument on springs with the off-diagonals flipped reports means far from the true solution
        let a = [1.5, -0.5, 0.0, -0.5, 1.25, 0.25, 0.0, 0.25, 2.25];
        let bad = [1.5, 0.5, 0.0, 0.5, 1.25, -0.25, 0.0, -0.25, 2.25];
        let b = [2.0, -0.5, 1.0];
        let w = Walk { steps: 400_000, dt: 0.05, temp: 1.0, seed: 4, burn: 1000, want_cov: false, marks: vec![] };
        let r = walk(&bad, &b, 3, &w);
        let x = gauss_solve(&a, &b, 3).unwrap();
        assert!((0..3).any(|i| (r.mean[i] - x[i]).abs() > 10.0 * r.se[i]));
    }

    #[test]
    fn numbers_leave_the_plus_minus_one_state_alone() {
        let src = format!("{}\nmodel :bowl do\n  thing :rain\nend\nrun :bowl do\n  drift 2_000\n  means\nend", BOWL);
        let mut it = Interp::default();
        it.exec(&src).unwrap();
        let m = &it.models["bowl"];
        assert_eq!(m.names, ["rain"]);
        assert!(m.h.iter().all(|&h| h == 0.0) && m.adj.iter().all(|r| r.is_empty()));
        let mut st = State::new(1);
        let mut out = Vec::new();
        let mut ctx = Ctx { base_dir: ".".into(), out: &mut out };
        let mut mm = m.clone();
        Numbers.run_stmt(&mut mm, &mut st, &[Tok::Ident("drift".into()), Tok::Num(100.0)], 1, &mut ctx).unwrap().unwrap();
        assert!(st.n == 0 && st.samples.is_empty() && st.held.is_empty());
    }

    #[test]
    fn errors_name_their_line() {
        let cases = [
            ("model :m do\n  number :x\n  x.springs :q, by: 1\nend", "line 3: unknown number :q"),
            ("model :m do\n  number :x\nend\nrun :m do\n  means\nend", "line 5: means needs a drift or a solve first"),
            ("model :m do\n  number :x\n  x.leans_to 1\nend", "line 3: leans_to needs `by:`"),
            ("model :m do\nend\nrun :m do\n  drift 100\nend", "line 4: drift needs numbers"),
            ("model :m do\nend\nrun :m do\n  solve :a, :b, matrix: \"1 0; 0 1\", target: \"1\"\nend", "line 4: target must have 2"),
            ("model :m do\n  number :x\n  x.leans_to 1, by: 1\nend\nrun :m do\n  drift 1_000, temperature: 0\n  spread\nend", "line 7: spread needs"),
        ];
        for (src, want) in cases {
            let e = Interp::default().exec(src).err().map(|e| e.0).unwrap_or_default();
            assert!(e.starts_with(want), "{:?} gave {:?}", src, e);
        }
    }
}
