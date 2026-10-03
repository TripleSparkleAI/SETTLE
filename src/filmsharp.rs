//! FILMSHARP: sharper grid play. Four additions that `grid` and `colour` use (no statements of their own):
//!
//! 1. THE BETHE INVERSION (`correct: :bethe`). Each pixel's lean from the pair equations of its edges. For one
//!    edge (i, j) with t = tanh J, the cavity magnetisations a = mu_{i\j}, b = mu_{j\i} solve
//!    m_i (1 + t a b) = a + t b and m_j (1 + t a b) = b + t a, and then h_i = atanh(m_i) - sum_j atanh(t mu_{j\i}).
//!    It is exact on a tree (tested on a chain) and the next order past TAP on the grid.
//! 2. THE FITTED LEANS (`fit: N, fit_sweeps: S, fit_update:`). A Newton-like step h <- h + eta P (m* - m_hat),
//!    with m_hat measured by settling the grid itself (the grid is its own oracle for the response TAP only
//!    guesses) and P TAP's inverse response at the target, made positive definite row by row (`tap_precond`).
//!    eta halves when the residual grows; the returned leans average the second half of the iterations.
//!    Two other fits are kept for the record and the exact checks: `secant_fit` steps by H_tap(m*) - H_tap(m_hat)
//!    (fails near the critical pull, where the TAP map stops being monotone and steps the wrong way), and
//!    `newton_fit` solves with the measured covariance (exact Newton on a small grid; on a big grid a few hundred
//!    samples give a rank-deficient covariance that amplifies noise).
//! 3. THE RAO-BLACKWELLISED READ (`read: :rb`). Instead of the 0/1 bit, average the probability of yes given the
//!    neighbours at the moment each pixel draws its coin: (1 + tanh I_i) / 2. The soft read is the same estimator
//!    evaluated at the end of each sweep; both need only the neighbours' bits and the known lean.
//! 4. FASTER-MIXING UPDATES (`update:`). `:gibbs` random order (the default, unchanged); `:checker` all even
//!    (x + y) pixels then all odd; `:metro` the Metropolised Gibbs rule (propose the other state, accept with
//!    min(1, exp(-2 s I))), the binary cousin of over-relaxation, which Peskun-dominates Gibbs; `:metro_checker`;
//!    and `:cluster` a Swendsen-Wang step with a ghost thing carrying the leans (non-local, not a p-bit rule).

use crate::grid::{leans_for, Invert, Spec};
use crate::lex::{err, kw, SettleError, Tok};
use crate::model::Model;
use crate::rng::Rng;

/// How one sweep updates the grid.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Update {
    Gibbs,
    Checker,
    Metro,
    MetroChecker,
    Cluster,
}

/// Read `update:` (:gibbs when absent).
pub fn update_opt(kv: &[(String, Tok)], ln: usize) -> Result<Update, SettleError> {
    match kw(kv, "update") {
        None => Ok(Update::Gibbs),
        Some(Tok::Sym(s)) => match s.as_str() {
            "gibbs" => Ok(Update::Gibbs),
            "checker" => Ok(Update::Checker),
            "metro" => Ok(Update::Metro),
            "metro_checker" => Ok(Update::MetroChecker),
            "cluster" => Ok(Update::Cluster),
            _ => err(ln, "update: takes :gibbs, :checker, :metro, :metro_checker or :cluster"),
        },
        Some(_) => err(ln, "update: takes :gibbs, :checker, :metro, :metro_checker or :cluster"),
    }
}

/// Cavity magnetisations (mu_{i\j}, mu_{j\i}) of one edge with t = tanh J, given the two magnetisations.
pub fn bethe_pair(mi: f64, mj: f64, t: f64) -> (f64, f64) {
    if t == 0.0 {
        return (mi, mj);
    }
    let a_of = |b: f64| (mi - t * b) / (1.0 - t * mi * b);
    // f(-1) > 0 and f(1) < 0 for |t|, |m| < 1, so bisection finds the root
    let f = |b: f64| {
        let a = a_of(b);
        mj * (1.0 + t * a * b) - b - t * a
    };
    let (mut lo, mut hi) = (-1.0 + 1e-15, 1.0 - 1e-15);
    for _ in 0..100 {
        let mid = 0.5 * (lo + hi);
        if f(mid) > 0.0 {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let b = 0.5 * (lo + hi);
    (a_of(b), b)
}

/// Bethe leans for magnetisations `target` on one grid.
pub fn bethe_leans(m: &Model, g: &Spec, target: &[f64]) -> Vec<f64> {
    let (a, b) = (g.start, g.start + g.w * g.h);
    (0..g.w * g.h)
        .map(|k| {
            let i = a + k;
            let mi = target[k];
            let mut lean = mi.atanh();
            for &(j, w) in &m.adj[i] {
                if j >= a && j < b {
                    let t = w.tanh();
                    let (_, mu_j) = bethe_pair(mi, target[j - a], t);
                    lean -= (t * mu_j).atanh();
                }
            }
            lean
        })
        .collect()
}

/// The closed-form leans for `inv` (Bethe here, the rest in grid.rs).
pub fn closed_form_leans(m: &Model, g: &Spec, target: &[f64], by: f64, inv: Invert) -> Vec<f64> {
    if inv == Invert::Bethe {
        let mut l = bethe_leans(m, g, target);
        if by != 1.0 {
            for (k, v) in l.iter_mut().enumerate() {
                *v += (by - 1.0) * target[k].atanh();
            }
        }
        l
    } else {
        leans_for(m, g, target, by, inv)
    }
}

/// The secant Newton fit: start at `h0`, measure magnetisations with `oracle`, step toward the target.
/// Returns the leans and the RMS grey residual (|m* - m_hat| / 2) measured at each iteration.
pub fn secant_fit(
    m: &mut Model,
    g: &Spec,
    target: &[f64],
    h0: Vec<f64>,
    iters: usize,
    average: bool,
    mut oracle: impl FnMut(&Model) -> Vec<f64>,
) -> (Vec<f64>, Vec<f64>) {
    let n = g.w * g.h;
    let h_target = leans_for(m, g, target, 1.0, Invert::Tap);
    let (mut h, mut eta, mut prev) = (h0, 1.0f64, f64::INFINITY);
    let (mut avg, mut navg, mut res) = (vec![0.0; n], 0usize, Vec::new());
    for t in 0..iters {
        m.h[g.start..g.start + n].copy_from_slice(&h);
        let mhat: Vec<f64> = oracle(m).iter().map(|v| v.clamp(-0.9999, 0.9999)).collect();
        let r = (target.iter().zip(&mhat).map(|(a, b)| (a - b) * (a - b) / 4.0).sum::<f64>() / n as f64).sqrt();
        res.push(r);
        if r > prev {
            eta = (eta * 0.5).max(1.0 / 16.0);
        }
        prev = r;
        let h_hat = leans_for(m, g, &mhat, 1.0, Invert::Tap);
        for k in 0..n {
            h[k] += (eta * (h_target[k] - h_hat[k])).clamp(-1.0, 1.0);
        }
        if average && t >= iters / 2 {
            for k in 0..n {
                avg[k] += h[k];
            }
            navg += 1;
        }
    }
    if navg > 0 {
        h = avg.iter().map(|v| v / navg as f64).collect();
    }
    m.h[g.start..g.start + n].copy_from_slice(&h);
    (h, res)
}

/// TAP's inverse response at magnetisations `mm`, made positive definite row by row (Gershgorin): each row's
/// diagonal is raised until it exceeds the sum of its off-diagonal sizes by `floor`. Applied as a sparse product.
/// diag_i = 1/(1 - m_i^2) + sum_j J^2 (1 - m_j^2),  off_ij = -J - 2 J^2 m_i m_j.
pub fn tap_precond(m: &Model, g: &Spec, mm: &[f64], floor: f64) -> (Vec<f64>, Vec<Vec<(usize, f64)>>) {
    let (a, b, n) = (g.start, g.start + g.w * g.h, g.w * g.h);
    let mut diag = vec![0.0; n];
    let mut off = vec![Vec::new(); n];
    for k in 0..n {
        let i = a + k;
        let mi = mm[k];
        let mut d = 1.0 / (1.0 - mi * mi).max(1e-6);
        let mut abs_off = 0.0;
        for &(j, w) in &m.adj[i] {
            if j >= a && j < b {
                let mj = mm[j - a];
                d += w * w * (1.0 - mj * mj);
                let o = -w - 2.0 * w * w * mi * mj;
                abs_off += o.abs();
                off[k].push((j - a, o));
            }
        }
        diag[k] = d.max(abs_off + floor);
    }
    (diag, off)
}

/// Fit leans with the TAP preconditioner held at the target: h <- h + eta P (m* - m_hat), P PD (see tap_precond).
pub fn precond_fit(
    m: &mut Model,
    g: &Spec,
    target: &[f64],
    h0: Vec<f64>,
    iters: usize,
    floor: f64,
    average: bool,
    oracle: impl FnMut(&Model) -> Vec<f64>,
) -> (Vec<f64>, Vec<f64>) {
    precond_fit_from(m, g, target, h0, iters, floor, average, 1.0, oracle)
}

/// `precond_fit` with the starting step size `eta0` (1 in FILMSHARP; FILMWARM's `warm_step:` sets it for warm frames).
#[allow(clippy::too_many_arguments)]
pub fn precond_fit_from(
    m: &mut Model,
    g: &Spec,
    target: &[f64],
    h0: Vec<f64>,
    iters: usize,
    floor: f64,
    average: bool,
    eta0: f64,
    mut oracle: impl FnMut(&Model) -> Vec<f64>,
) -> (Vec<f64>, Vec<f64>) {
    let n = g.w * g.h;
    let (diag, off) = tap_precond(m, g, target, floor);
    let (mut h, mut eta, mut prev) = (h0, eta0, f64::INFINITY);
    let (mut avg, mut navg, mut res) = (vec![0.0; n], 0usize, Vec::new());
    for t in 0..iters {
        m.h[g.start..g.start + n].copy_from_slice(&h);
        let mhat = oracle(m);
        let r: Vec<f64> = target.iter().zip(&mhat).map(|(a, b)| a - b).collect();
        let rms = (r.iter().map(|x| x * x / 4.0).sum::<f64>() / n as f64).sqrt();
        res.push(rms);
        if rms > prev {
            eta = (eta * 0.5).max(1.0 / 64.0);
        }
        prev = rms;
        for k in 0..n {
            let pr = diag[k] * r[k] + off[k].iter().map(|&(j, o)| o * r[j]).sum::<f64>();
            h[k] += (eta * pr).clamp(-2.0, 2.0);
        }
        if average && t >= iters / 2 {
            for k in 0..n {
                avg[k] += h[k];
            }
            navg += 1;
        }
    }
    if navg > 0 {
        h = avg.iter().map(|v| v / navg as f64).collect();
    }
    m.h[g.start..g.start + n].copy_from_slice(&h);
    (h, res)
}

/// The same with magnetisations measured by settling the grid (RB read, a quarter burn-in per iteration).
pub fn precond_leans(m: &mut Model, g: &Spec, target: &[f64], h0: Vec<f64>, rule: Update, iters: usize, sweeps: usize, beta: f64, seed: u64, floor: f64) -> (Vec<f64>, Vec<f64>) {
    let n = g.w * g.h;
    let mut rng = Rng::new(seed);
    let mut s: Vec<f64> = (0..m.len()).map(|_| if rng.unit() < 0.5 { -1.0 } else { 1.0 }).collect();
    let mut free: Vec<usize> = (g.start..g.start + n).collect();
    let mut sw = Sweeper::new(rule, g, &free);
    let sweeps = sweeps.max(4);
    let burn = sweeps / 4;
    precond_fit(m, g, target, h0, iters, floor, true, |mm: &Model| {
        let mut acc = vec![0.0; n];
        for k in 0..sweeps {
            let rb = if k >= burn { Some(&mut acc[..]) } else { None };
            sw.sweep(mm, g, &mut s, &mut free, &mut rng, beta, rb);
        }
        acc.iter().map(|a| a / (sweeps - burn) as f64).collect()
    })
}

/// Fit leans by settling the grid itself: `iters` rounds of `sweeps` sweeps (a quarter burn-in), the
/// magnetisation read Rao-Blackwellised, a fresh chain from coin flips drawn from `seed`.
pub fn fit_leans(m: &mut Model, g: &Spec, target: &[f64], h0: Vec<f64>, rule: Update, iters: usize, sweeps: usize, beta: f64, seed: u64) -> (Vec<f64>, Vec<f64>) {
    let n = g.w * g.h;
    let mut rng = Rng::new(seed);
    let mut s: Vec<f64> = (0..m.len()).map(|_| if rng.unit() < 0.5 { -1.0 } else { 1.0 }).collect();
    let mut free: Vec<usize> = (g.start..g.start + n).collect();
    let mut sw = Sweeper::new(rule, g, &free);
    let sweeps = sweeps.max(4);
    let burn = sweeps / 4;
    secant_fit(m, g, target, h0, iters, true, |mm: &Model| {
        let mut acc = vec![0.0; n];
        for k in 0..sweeps {
            let rb = if k >= burn { Some(&mut acc[..]) } else { None };
            sw.sweep(mm, g, &mut s, &mut free, &mut rng, beta, rb);
        }
        acc.iter().map(|a| a / (sweeps - burn) as f64).collect()
    })
}

/// The grid's response to its leans, dm/dh = C (the covariance), either measured from samples or given exactly.
pub enum Response {
    /// K stored arrangements (row-major K x n, each +-1) and their mean.
    Samples { s: Vec<f32>, k: usize, mean: Vec<f64> },
    /// The whole covariance (small grids, by enumeration).
    Dense(Vec<Vec<f64>>),
}

impl Response {
    /// C v.
    pub fn apply(&self, v: &[f64]) -> Vec<f64> {
        match self {
            Response::Dense(c) => c.iter().map(|row| row.iter().zip(v).map(|(a, b)| a * b).sum()).collect(),
            Response::Samples { s, k, mean } => {
                let n = v.len();
                let mut out = vec![0.0; n];
                let mv: f64 = mean.iter().zip(v).map(|(a, b)| a * b).sum();
                for r in 0..*k {
                    let row = &s[r * n..(r + 1) * n];
                    let d: f64 = row.iter().zip(v).map(|(a, b)| *a as f64 * b).sum::<f64>() - mv;
                    for i in 0..n {
                        out[i] += (row[i] as f64 - mean[i]) * d;
                    }
                }
                out.iter().map(|x| x / *k as f64).collect()
            }
        }
    }
    pub fn diag(&self, n: usize) -> Vec<f64> {
        match self {
            Response::Dense(c) => (0..n).map(|i| c[i][i]).collect(),
            Response::Samples { s, k, mean } => (0..n).map(|i| (0..*k).map(|r| { let d = s[r * n + i] as f64 - mean[i]; d * d }).sum::<f64>() / *k as f64).collect(),
        }
    }
}

/// Solve (C + lambda D) x = r by conjugate gradient, D = diag(1 - m^2), Jacobi-preconditioned, at most `iters` steps.
pub fn solve_response(resp: &Response, mhat: &[f64], r: &[f64], lambda: f64, iters: usize) -> Vec<f64> {
    let n = r.len();
    let d: Vec<f64> = mhat.iter().map(|m| (1.0 - m * m).max(1e-3)).collect();
    let cd = resp.diag(n);
    let pre: Vec<f64> = (0..n).map(|i| 1.0 / (cd[i] + lambda * d[i]).max(1e-6)).collect();
    let a = |v: &[f64]| -> Vec<f64> {
        let cv = resp.apply(v);
        (0..n).map(|i| cv[i] + lambda * d[i] * v[i]).collect()
    };
    let dot = |x: &[f64], y: &[f64]| x.iter().zip(y).map(|(p, q)| p * q).sum::<f64>();
    let mut x = vec![0.0; n];
    let mut res = r.to_vec();
    let mut z: Vec<f64> = (0..n).map(|i| pre[i] * res[i]).collect();
    let mut p = z.clone();
    let mut rz = dot(&res, &z);
    let r0 = dot(r, r).sqrt();
    for _ in 0..iters {
        let ap = a(&p);
        let alpha = rz / dot(&p, &ap).max(1e-300);
        for i in 0..n {
            x[i] += alpha * p[i];
            res[i] -= alpha * ap[i];
        }
        if dot(&res, &res).sqrt() < 1e-8 * r0.max(1e-300) {
            break;
        }
        z = (0..n).map(|i| pre[i] * res[i]).collect();
        let rz2 = dot(&res, &z);
        let beta = rz2 / rz.max(1e-300);
        rz = rz2;
        for i in 0..n {
            p[i] = z[i] + beta * p[i];
        }
    }
    x
}

/// Newton on the leans with a measured response: h <- h + eta (C + lambda D)^-1 (m* - m_hat). The oracle returns
/// the magnetisations and the response at the current leans. eta halves when the residual grows; with `average`
/// the returned leans average the second half of the iterations. Returns the leans and the RMS grey residuals.
pub fn newton_fit(
    m: &mut Model,
    g: &Spec,
    target: &[f64],
    h0: Vec<f64>,
    iters: usize,
    lambda: f64,
    average: bool,
    mut oracle: impl FnMut(&Model) -> (Vec<f64>, Response),
) -> (Vec<f64>, Vec<f64>) {
    let n = g.w * g.h;
    let (mut h, mut eta, mut prev) = (h0, 1.0f64, f64::INFINITY);
    let (mut avg, mut navg, mut res) = (vec![0.0; n], 0usize, Vec::new());
    for t in 0..iters {
        m.h[g.start..g.start + n].copy_from_slice(&h);
        let (mhat, resp) = oracle(m);
        let r: Vec<f64> = target.iter().zip(&mhat).map(|(a, b)| a - b).collect();
        let rms = (r.iter().map(|x| x * x / 4.0).sum::<f64>() / n as f64).sqrt();
        res.push(rms);
        if rms > prev {
            eta = (eta * 0.5).max(1.0 / 16.0);
        }
        prev = rms;
        let dh = solve_response(&resp, &mhat, &r, lambda, 60);
        for k in 0..n {
            h[k] += (eta * dh[k]).clamp(-2.0, 2.0);
        }
        if average && t >= iters / 2 {
            for k in 0..n {
                avg[k] += h[k];
            }
            navg += 1;
        }
    }
    if navg > 0 {
        h = avg.iter().map(|v| v / navg as f64).collect();
    }
    m.h[g.start..g.start + n].copy_from_slice(&h);
    (h, res)
}

/// Newton with the response measured by settling the grid: each iteration `sweeps` sweeps (a quarter burn-in),
/// m_hat Rao-Blackwellised, the covariance from the kept arrangements (at most 400, evenly spaced).
pub fn newton_leans(m: &mut Model, g: &Spec, target: &[f64], h0: Vec<f64>, rule: Update, iters: usize, sweeps: usize, beta: f64, seed: u64, lambda: f64) -> (Vec<f64>, Vec<f64>) {
    let n = g.w * g.h;
    let mut rng = Rng::new(seed);
    let mut s: Vec<f64> = (0..m.len()).map(|_| if rng.unit() < 0.5 { -1.0 } else { 1.0 }).collect();
    let mut free: Vec<usize> = (g.start..g.start + n).collect();
    let mut sw = Sweeper::new(rule, g, &free);
    let sweeps = sweeps.max(4);
    let burn = sweeps / 4;
    let every = ((sweeps - burn) / 400).max(1);
    newton_fit(m, g, target, h0, iters, lambda, true, |mm: &Model| {
        let mut acc = vec![0.0; n];
        let (mut store, mut kk, mut mean) = (Vec::new(), 0usize, vec![0.0; n]);
        for t in 0..sweeps {
            let rb = if t >= burn { Some(&mut acc[..]) } else { None };
            sw.sweep(mm, g, &mut s, &mut free, &mut rng, beta, rb);
            if t >= burn && (t - burn) % every == 0 {
                for i in 0..n {
                    store.push(s[g.start + i] as f32);
                    mean[i] += s[g.start + i];
                }
                kk += 1;
            }
        }
        let mean: Vec<f64> = mean.iter().map(|v| v / kk as f64).collect();
        let mhat = acc.iter().map(|a| a / (sweeps - burn) as f64).collect();
        (mhat, Response::Samples { s: store, k: kk, mean })
    })
}

/// One grid's sweep machinery for the update rules above.
pub struct Sweeper {
    pub rule: Update,
    checker: Vec<usize>,
    in_free: Vec<bool>,
    parent: Vec<usize>,
    flip: Vec<bool>,
}

fn find(p: &mut [usize], mut x: usize) -> usize {
    while p[x] != x {
        p[x] = p[p[x]];
        x = p[x];
    }
    x
}

fn union(p: &mut [usize], a: usize, b: usize) {
    let (ra, rb) = (find(p, a), find(p, b));
    if ra != rb {
        // the ghost (largest index) stays a root so its cluster is easy to spot
        if ra > rb {
            p[rb] = ra;
        } else {
            p[ra] = rb;
        }
    }
}

impl Sweeper {
    pub fn new(rule: Update, g: &Spec, free: &[usize]) -> Self {
        let n = g.w * g.h;
        let mut in_free = vec![false; n];
        for &i in free {
            if i >= g.start && i < g.start + n {
                in_free[i - g.start] = true;
            }
        }
        let parity = |i: usize| ((i - g.start) % g.w + (i - g.start) / g.w) % 2;
        let mut checker: Vec<usize> = free.iter().cloned().filter(|&i| parity(i) == 0).collect();
        checker.extend(free.iter().cloned().filter(|&i| parity(i) == 1));
        Sweeper { rule, checker, in_free, parent: vec![0; n + 1], flip: vec![false; n + 1] }
    }

    /// One sweep. `free` is shuffled in place for the random-order rules exactly as `State::sweep` does, so
    /// `:gibbs` consumes the same random numbers and visits the same states as the plain sweep. With `rb`, each
    /// pixel adds tanh(beta I) to rb[k] (the single-site rules at the moment it draws; `:cluster` after the step).
    #[allow(clippy::too_many_arguments)]
    pub fn sweep(&mut self, m: &Model, g: &Spec, s: &mut [f64], free: &mut [usize], rng: &mut Rng, beta: f64, mut rb: Option<&mut [f64]>) {
        let metro = matches!(self.rule, Update::Metro | Update::MetroChecker);
        match self.rule {
            Update::Cluster => {
                self.cluster(m, g, s, rng, beta);
                if let Some(acc) = rb.as_deref_mut() {
                    for k in 0..g.w * g.h {
                        acc[k] += (beta * m.input(g.start + k, s)).tanh();
                    }
                }
            }
            Update::Gibbs | Update::Metro => {
                for k in (1..free.len()).rev() {
                    let r = rng.below(k + 1);
                    free.swap(k, r);
                }
                for &i in free.iter() {
                    single(m, g, s, i, rng, beta, metro, rb.as_deref_mut());
                }
            }
            Update::Checker | Update::MetroChecker => {
                for idx in 0..self.checker.len() {
                    let i = self.checker[idx];
                    single(m, g, s, i, rng, beta, metro, rb.as_deref_mut());
                }
            }
        }
    }

    /// Swendsen-Wang with a ghost: a satisfied edge (w s_i s_j > 0) bonds with probability 1 - exp(-2 beta |w|);
    /// a pixel agreeing with its lean bonds to the ghost with probability 1 - exp(-2 beta |h|); every cluster
    /// without the ghost flips with probability 1/2. Held pixels are tied to the ghost. Exact for any leans.
    fn cluster(&mut self, m: &Model, g: &Spec, s: &mut [f64], rng: &mut Rng, beta: f64) {
        let (n, a, b) = (g.w * g.h, g.start, g.start + g.w * g.h);
        for k in 0..=n {
            self.parent[k] = k;
        }
        for k in 0..n {
            let i = a + k;
            if !self.in_free[k] {
                union(&mut self.parent, k, n);
                continue;
            }
            let mut field = m.h[i];
            for &(j, w) in &m.adj[i] {
                if j >= a && j < b {
                    if j > i && w * s[i] * s[j] > 0.0 && rng.unit() < 1.0 - (-2.0 * beta * w.abs()).exp() {
                        union(&mut self.parent, k, j - a);
                    }
                } else {
                    field += w * s[j];
                }
            }
            if field * s[i] > 0.0 && rng.unit() < 1.0 - (-2.0 * beta * field.abs()).exp() {
                union(&mut self.parent, k, n);
            }
        }
        let ghost = find(&mut self.parent, n);
        for k in 0..=n {
            self.flip[k] = false;
        }
        let mut decided = vec![false; n + 1];
        for k in 0..n {
            let r = find(&mut self.parent, k);
            if r == ghost {
                continue;
            }
            if !decided[r] {
                decided[r] = true;
                self.flip[r] = rng.unit() < 0.5;
            }
            if self.flip[r] {
                s[a + k] = -s[a + k];
            }
        }
    }
}

/// One single-site update of thing `i`: heat bath (Gibbs) or Metropolised Gibbs.
#[inline]
fn single(m: &Model, g: &Spec, s: &mut [f64], i: usize, rng: &mut Rng, beta: f64, metro: bool, rb: Option<&mut [f64]>) {
    let x = beta * m.input(i, s);
    let t = x.tanh();
    if let Some(acc) = rb {
        acc[i - g.start] += t;
    }
    if metro {
        let p = (-2.0 * s[i] * x).exp();
        if p >= 1.0 || rng.unit() < p {
            s[i] = -s[i];
        }
    } else {
        s[i] = if t > rng.signed() { 1.0 } else { -1.0 };
    }
}

/// Exact expected squared error of the yes-count estimate of one lone thing with yes-probability `p`, K samples,
/// started with yes-probability `p0`, under Gibbs (independent coins) or Metropolised Gibbs (a two-state chain).
/// The no-pull exact answer for both rules; computed by propagating the count distribution.
pub fn lone_mse(p: f64, k: usize, p0: f64, metro: bool) -> f64 {
    let q = 1.0 - p;
    // transition probabilities: up = P(no -> yes), down = P(yes -> no)
    let (up, down) = if metro {
        (if q <= 0.0 { 1.0 } else { (p / q).min(1.0) }, if p <= 0.0 { 1.0 } else { (q / p).min(1.0) })
    } else {
        (p, q)
    };
    chain_mse(p, k, p0, up, down)
}

/// The same for one lone thing under the ghost cluster step: a thing agreeing with its lean bonds to the ghost with
/// probability 1 - exp(-2|h|) = 1 - q'/p' (p' the larger of p, q) and is kept; otherwise it flips a fair coin.
pub fn lone_mse_cluster(p: f64, k: usize, p0: f64) -> f64 {
    let (hi, lo) = if p >= 0.5 { (p, 1.0 - p) } else { (1.0 - p, p) };
    let leave = 0.5 * lo / hi; // P(agreeing -> disagreeing)
    let back = 0.5; // P(disagreeing -> agreeing)
    if p >= 0.5 {
        chain_mse(p, k, p0, back, leave)
    } else {
        chain_mse(p, k, p0, leave, back)
    }
}

/// Exact expected squared error of the yes-share of K steps of a two-state chain (up = P(no -> yes),
/// down = P(yes -> no)) started yes with probability p0, against the target p.
pub fn chain_mse(p: f64, k: usize, p0: f64, up: f64, down: f64) -> f64 {
    // dist[state][count]
    let mut no = vec![0.0; k + 1];
    let mut yes = vec![0.0; k + 1];
    no[0] = 1.0 - p0;
    yes[0] = p0;
    for _ in 0..k {
        let mut nno = vec![0.0; k + 1];
        let mut nyes = vec![0.0; k + 1];
        for c in 0..k {
            // from no: to yes with up (count + 1), stay no
            nyes[c + 1] += no[c] * up;
            nno[c] += no[c] * (1.0 - up);
            // from yes: to no with down, stay yes (count + 1)
            nno[c] += yes[c] * down;
            nyes[c + 1] += yes[c] * (1.0 - down);
        }
        no = nno;
        yes = nyes;
    }
    (0..=k).map(|c| (no[c] + yes[c]) * (c as f64 / k as f64 - p).powi(2)).sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grid::{declare, load};
    use crate::model::{exact_rates, State};

    fn chain(n: usize, j: f64) -> (Model, Spec) {
        let mut m = Model::default();
        declare(&mut m, "c", &[Tok::Label("width".into()), Tok::Num(n as f64), Tok::Comma, Tok::Label("height".into()), Tok::Num(1.0), Tok::Comma, Tok::Label("smooth".into()), Tok::Num(j)], 1).unwrap();
        let g = load(&m, "c", 1).unwrap();
        (m, g)
    }

    #[test]
    fn bethe_leans_are_exact_on_a_chain() {
        // a chain is a tree, where the Bethe approximation is exact: the exact marginals must hit the target
        for &j in &[0.2, 0.5, 0.9] {
            let (mut m, g) = chain(10, j);
            let mut r = Rng::new(9);
            let target: Vec<f64> = (0..10).map(|_| 1.6 * r.unit() - 0.8).collect();
            let h = bethe_leans(&m, &g, &target);
            m.h.copy_from_slice(&h);
            let got = exact_rates(&m, &State::new(0));
            for k in 0..10 {
                assert!((2.0 * got[k] - 1.0 - target[k]).abs() < 1e-9, "J {} pixel {}: {} vs {}", j, k, 2.0 * got[k] - 1.0, target[k]);
            }
            // vacuity control: TAP on the same chain is not exact at these pulls
            let tap = leans_for(&m, &g, &target, 1.0, Invert::Tap);
            m.h.copy_from_slice(&tap);
            let got = exact_rates(&m, &State::new(0));
            let worst = (0..10).map(|k| (2.0 * got[k] - 1.0 - target[k]).abs()).fold(0.0, f64::max);
            assert!(worst > 1e-4, "J {}: TAP worst {}", j, worst);
        }
    }

    #[test]
    fn the_secant_fit_with_exact_moments_reaches_the_target_on_a_small_grid() {
        let mut m = Model::default();
        declare(&mut m, "g", &[Tok::Label("width".into()), Tok::Num(4.0), Tok::Comma, Tok::Label("height".into()), Tok::Num(3.0), Tok::Comma, Tok::Label("smooth".into()), Tok::Num(0.4)], 1).unwrap();
        let g = load(&m, "g", 1).unwrap();
        let mut r = Rng::new(4);
        let target: Vec<f64> = (0..12).map(|_| 1.6 * r.unit() - 0.8).collect();
        let h0 = leans_for(&m, &g, &target, 1.0, Invert::Tap);
        let (_, res) = secant_fit(&mut m, &g, &target, h0, 40, false, |mm: &Model| exact_rates(mm, &State::new(0)).iter().map(|v| 2.0 * v - 1.0).collect());
        assert!(res[0] > 1e-3 && *res.last().unwrap() < 1e-8, "{:?}", res);
    }

    #[test]
    fn metropolised_gibbs_keeps_the_right_marginals_and_beats_the_coin_law() {
        // exact answer for one lone thing with p = 0.7: the Metropolised chain's squared error is below the
        // independent-coin error, and propagating the count distribution for Gibbs gives p q / K exactly
        let (p, k) = (0.7, 40);
        let gibbs = lone_mse(p, k, p, false);
        assert!((gibbs - p * (1.0 - p) / k as f64).abs() < 1e-12, "{}", gibbs);
        let metro = lone_mse(p, k, p, true);
        // stationary limit: p q |2p - 1| / K plus an order 1/K^2 term
        assert!(metro < 0.5 * gibbs && metro > 0.3 * gibbs, "metro {} gibbs {}", metro, gibbs);
    }

    fn sampled_rates(rule: Update, j: f64, sweeps: usize, seed: u64) -> (Vec<f64>, Vec<f64>) {
        let mut m = Model::default();
        declare(&mut m, "g", &[Tok::Label("width".into()), Tok::Num(4.0), Tok::Comma, Tok::Label("height".into()), Tok::Num(3.0), Tok::Comma, Tok::Label("smooth".into()), Tok::Num(j)], 1).unwrap();
        let g = load(&m, "g", 1).unwrap();
        let mut r = Rng::new(11);
        for k in 0..12 {
            m.h[k] = 1.2 * r.unit() - 0.6;
        }
        let exact = exact_rates(&m, &State::new(0));
        let mut rng = Rng::new(seed);
        let mut s: Vec<f64> = (0..12).map(|_| if rng.unit() < 0.5 { -1.0 } else { 1.0 }).collect();
        let mut free: Vec<usize> = (0..12).collect();
        let mut sw = Sweeper::new(rule, &g, &free);
        let mut yes = vec![0.0; 12];
        for _ in 0..100 {
            sw.sweep(&m, &g, &mut s, &mut free, &mut rng, 1.0, None);
        }
        for _ in 0..sweeps {
            sw.sweep(&m, &g, &mut s, &mut free, &mut rng, 1.0, None);
            for k in 0..12 {
                if s[k] > 0.0 {
                    yes[k] += 1.0;
                }
            }
        }
        (yes.iter().map(|y| y / sweeps as f64).collect(), exact)
    }

    #[test]
    fn every_update_rule_samples_the_exact_marginals() {
        // exact answer by enumeration of a 4x3 grid with random leans at pull 0.4
        for rule in [Update::Gibbs, Update::Checker, Update::Metro, Update::MetroChecker, Update::Cluster] {
            let (got, exact) = sampled_rates(rule, 0.4, 200_000, 3);
            let worst = got.iter().zip(&exact).map(|(a, b)| (a - b).abs()).fold(0.0, f64::max);
            assert!(worst < 0.01, "{:?}: worst {}", rule, worst);
        }
    }

    #[test]
    fn the_marginal_check_can_fail() {
        // negative control for the test above: a chain sampling the model with half the pull (what a cluster rule
        // with half the bond exponent would sample) misses the exact marginals by far more than the 0.01 bound
        let (half, _) = sampled_rates(Update::Cluster, 0.2, 200_000, 3);
        let (_, exact) = sampled_rates(Update::Gibbs, 0.4, 10, 3);
        let off = half.iter().zip(&exact).map(|(a, b)| (a - b).abs()).fold(0.0, f64::max);
        assert!(off > 0.03, "half-pull chain against the full-pull exact answer: {}", off);
    }
}
