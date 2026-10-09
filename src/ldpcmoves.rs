//! LDPCMOVES: decode an LDPC code by settling, with moves that change several things at once, and at the
//! Nishimori temperature by averaging bits.
//!
//! ```text
//! model :line do
//!   ldpc :c, bits: 64, rate: 0.5, gadget: :chain, strength: 4.6, codebook_seed: 1
//! end
//! run :line do
//!   c.transmit flip: 0.03, seed: 1
//!   c.decode_moves mover: :block, block: 4, sweeps: 400, seed: 2   # anneal 1 -> 0.05, keep the calmest
//!   c.decode_nishimori mover: :block, sweeps: 2000, seed: 3        # stay at T = 1, average each bit
//! end
//! ```
//!
//! LDPCSETTLE built every parity check from helper things and updated one thing at a time. Its failures were
//! search failures: a broken check had to walk along its helpers one flip at a time. This family moves a code
//! bit TOGETHER with the helpers of every check it sits in. The helpers of one check see only that check's code
//! bits, so given the code bits they can be summed out exactly:
//!
//! - `Z_r(bits) = sum over the helper arrangements of check r of exp(-beta lambda P_r)`, computed by listing
//!   the helper integer (`:sum`) or by a two-state forward pass along the xor chain (`:chain`);
//! - the code bits then follow `pi(bits) proportional to exp(beta sum_i h_i s_i) prod_r Z_r(bits)`, which is
//!   exactly the SETTLE model's distribution with the helpers summed out (proved by enumeration below).
//!
//! A move that changes code bits and redraws the touched checks' helpers from their exact conditional is the
//! same as a heat-bath move on `pi`, so every mover here keeps the SETTLE model's own distribution:
//!
//! - `:single` (one code bit and its checks' helpers, heat bath),
//! - `:block` (pick a check at random, pick `block` of its bits at random, heat bath over all their joint
//!   values, so pair flips that keep the check even are one move).
//!
//! Detailed balance is proved on small codes by building each mover's exact transition matrix.
//! Equations with plain readings: `runs/ldpcmoves/REPORT_LDPCMOVES.md`.

use crate::ext::{Claim, Ctx, Ext};
use crate::ldpcsettle::{coded_ldpc, hard_lean, Gadget};
use crate::lex::{err, kw, kwargs, num, only, SettleError, Tok, whole};
use crate::model::{Model, State};
use crate::rng::Rng;

pub struct LdpcMoves;

const NEG: f64 = f64::NEG_INFINITY;

fn lse(a: f64, b: f64) -> f64 {
    if a == NEG {
        return b;
    }
    if b == NEG {
        return a;
    }
    let m = a.max(b);
    m + ((a - m).exp() + (b - m).exp()).ln()
}

/// Helper things needed to write floor(w/2) in binary (the `:sum` gadget's layout).
fn helper_bits(w: usize) -> usize {
    let mut top = w / 2;
    let mut k = 0;
    while top > 0 {
        k += 1;
        top >>= 1;
    }
    k
}

/// ln Z of one check's helpers given its code bits, at `bl` = beta times lambda. Works for any weight.
pub fn ln_z_check(g: Gadget, bits: &[u8], bl: f64) -> f64 {
    let w = bits.len();
    match g {
        Gadget::Sum => {
            let c = bits.iter().map(|&b| b as i64).sum::<i64>();
            let mut z = NEG;
            for v in 0..(1i64 << helper_bits(w)) {
                let d = (c - 2 * v) as f64;
                z = lse(z, -bl * d * d);
            }
            z
        }
        Gadget::Chain => match w {
            0 => 0.0,
            1 => -bl * bits[0] as f64,
            2 => {
                let d = bits[0] as f64 - bits[1] as f64;
                -bl * d * d
            }
            _ => {
                let lg = chain_lg(bl);
                chain_forward(bits, &lg)
            }
        },
    }
}

/// ln of the sum over one three-bit check's helper x of exp(-bl (n - 2x)^2), for n = 0..3 ones.
fn chain_lg(bl: f64) -> [f64; 4] {
    let mut g = [0.0; 4];
    for (n, gn) in g.iter_mut().enumerate() {
        let n = n as f64;
        *gn = lse(-bl * n * n, -bl * (n - 2.0) * (n - 2.0));
    }
    g
}

/// The chain's forward pass over its free partial parities (w >= 3).
fn chain_forward(bits: &[u8], lg: &[f64; 4]) -> f64 {
    let w = bits.len();
    let mut f = [NEG, NEG];
    f[bits[0] as usize] = 0.0;
    for t in 0..(w - 2) {
        let mid = bits[t + 1] as usize;
        if t + 1 < w - 2 {
            let mut nf = [NEG, NEG];
            for (r, slot) in nf.iter_mut().enumerate() {
                for left in 0..2 {
                    if f[left] != NEG {
                        *slot = lse(*slot, f[left] + lg[left + mid + r]);
                    }
                }
            }
            f = nf;
        } else {
            let r = bits[w - 1] as usize;
            let mut z = NEG;
            for left in 0..2 {
                if f[left] != NEG {
                    z = lse(z, f[left] + lg[left + mid + r]);
                }
            }
            return z;
        }
    }
    unreachable!()
}

/// The chain's forward pass in plain numbers (fast; used while exp(-9 bl) stays representable).
fn chain_forward_lin(bits: &[u8], g: &[f64; 4]) -> f64 {
    let w = bits.len();
    let mut f = [0.0, 0.0];
    f[bits[0] as usize] = 1.0;
    for t in 0..(w - 2) {
        let mid = bits[t + 1] as usize;
        if t + 1 < w - 2 {
            f = [f[0] * g[mid] + f[1] * g[1 + mid], f[0] * g[mid + 1] + f[1] * g[mid + 2]];
            // keep the numbers in range on long chains
            let m = f[0].max(f[1]);
            if m > 1e200 {
                f = [f[0] * 1e-200, f[1] * 1e-200];
                return chain_forward_lin_scaled(bits, g, t + 1, f, 200.0 * std::f64::consts::LN_10);
            }
        } else {
            let r = bits[w - 1] as usize;
            return (f[0] * g[mid + r] + f[1] * g[1 + mid + r]).ln();
        }
    }
    unreachable!()
}

fn chain_forward_lin_scaled(bits: &[u8], g: &[f64; 4], from: usize, mut f: [f64; 2], mut shift: f64) -> f64 {
    let w = bits.len();
    for t in from..(w - 2) {
        let mid = bits[t + 1] as usize;
        if t + 1 < w - 2 {
            f = [f[0] * g[mid] + f[1] * g[1 + mid], f[0] * g[mid + 1] + f[1] * g[mid + 2]];
            let m = f[0].max(f[1]);
            if m > 1e200 {
                f = [f[0] * 1e-200, f[1] * 1e-200];
                shift += 200.0 * std::f64::consts::LN_10;
            }
        } else {
            let r = bits[w - 1] as usize;
            return (f[0] * g[mid + r] + f[1] * g[1 + mid + r]).ln() + shift;
        }
    }
    unreachable!()
}

/// Tables for one temperature: the `:sum` gadget's ln Z by (weight, count), the `:chain` gadget's triple table.
#[derive(Clone)]
pub struct Temp {
    pub beta: f64,
    bl: f64,
    lg: [f64; 4],
    /// exp of lg, and whether it is safe to use (bl small enough that exp(-bl) stays well above underflow).
    glin: [f64; 4],
    lin: bool,
    sum: Vec<Vec<f64>>,
}

/// The mover that proposes each update.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Mover {
    /// One code bit with its checks' helpers, heat bath.
    Single,
    /// A random check, `b` of its bits at random, heat bath over their 2^b joint values.
    Block(usize),
}

impl Mover {
    pub fn name(self) -> String {
        match self {
            Mover::Single => "single".into(),
            Mover::Block(b) => format!("block{}", b),
        }
    }
}

/// A code with its helpers summed out: the code bits only, channel leans, and exact per-check partition sums.
#[derive(Clone)]
pub struct Collapsed {
    pub n: usize,
    pub rows: Vec<Vec<usize>>,
    pub var_checks: Vec<Vec<usize>>,
    pub gadget: Gadget,
    pub lambda: f64,
    pub fixed_zero: Vec<usize>,
    /// Bits that may move (not fixed at zero).
    pub free: Vec<usize>,
    /// Checks that hold at least one free bit (the block mover picks among these).
    block_rows: Vec<usize>,
    max_w: usize,
    /// Channel lean on each code bit (positive toward bit 1), the same lean LDPCSETTLE puts on the model.
    pub lean: Vec<f64>,
    is_free: Vec<bool>,
}

/// Reusable buffers for one mover.
#[derive(Default)]
struct Scratch {
    checks: Vec<usize>,
    saved: Vec<u8>,
    lw: Vec<f64>,
    per: Vec<f64>,
    of_bit: Vec<Vec<usize>>,
}

/// The outcome of one decode.
#[derive(Clone, Debug)]
pub struct Outcome {
    pub bits: Vec<u8>,
    /// True when the bits satisfy every check and the fixed-zero bits are 0.
    pub codeword: bool,
    /// Zero-temperature energy of the returned bits: -sum h s + lambda (checks broken).
    pub e0: f64,
    pub broken: usize,
    /// ln Z evaluations of one check, the unit of work.
    pub work: u64,
}

impl Collapsed {
    pub fn new(rows: &[Vec<usize>], n: usize, fixed_zero: &[usize], gadget: Gadget, lambda: f64) -> Collapsed {
        let mut var_checks = vec![Vec::new(); n];
        for (r, row) in rows.iter().enumerate() {
            for &j in row {
                var_checks[j].push(r);
            }
        }
        let mut is_free = vec![true; n];
        for &j in fixed_zero {
            is_free[j] = false;
        }
        let free: Vec<usize> = (0..n).filter(|&j| is_free[j]).collect();
        let block_rows = (0..rows.len()).filter(|&r| rows[r].iter().any(|&j| is_free[j])).collect();
        let max_w = rows.iter().map(|r| r.len()).max().unwrap_or(0);
        assert!(max_w <= 64, "a check of weight above 64");
        Collapsed {
            n,
            rows: rows.to_vec(),
            var_checks,
            gadget,
            lambda,
            fixed_zero: fixed_zero.to_vec(),
            free,
            block_rows,
            max_w,
            lean: vec![0.0; n],
            is_free,
        }
    }

    pub fn from_ldpc(l: &crate::coded::Ldpc, gadget: Gadget, lambda: f64) -> Collapsed {
        Collapsed::new(&l.rows, l.n, &l.fixed_zero, gadget, lambda)
    }

    pub fn set_leans(&mut self, lean: &[f64]) {
        assert_eq!(lean.len(), self.n);
        self.lean = lean.to_vec();
    }

    pub fn temp(&self, beta: f64) -> Temp {
        self.temp_with(beta, self.lambda)
    }

    /// Tables for inverse temperature beta with the penalty strength `lambda` in place of the code's own.
    pub fn temp_with(&self, beta: f64, lambda: f64) -> Temp {
        let bl = beta * lambda;
        let sum = (0..=self.max_w)
            .map(|w| {
                (0..=w)
                    .map(|c| {
                        let bits: Vec<u8> = (0..w).map(|i| (i < c) as u8).collect();
                        ln_z_check(Gadget::Sum, &bits, bl)
                    })
                    .collect()
            })
            .collect();
        let lg = chain_lg(bl);
        Temp { beta, bl, lg, glin: [lg[0].exp(), lg[1].exp(), lg[2].exp(), lg[3].exp()], lin: bl < 250.0, sum }
    }

    /// ln Z of check r at the current bits.
    pub fn lnz_row(&self, r: usize, y: &[u8], t: &Temp) -> f64 {
        let row = &self.rows[r];
        match self.gadget {
            Gadget::Sum => {
                let c = row.iter().filter(|&&j| y[j] == 1).count();
                t.sum[row.len()][c]
            }
            Gadget::Chain => {
                let w = row.len();
                if w < 3 {
                    let b: Vec<u8> = row.iter().map(|&j| y[j]).collect();
                    return ln_z_check(Gadget::Chain, &b, t.bl);
                }
                let mut buf = [0u8; 64];
                for (k, &j) in row.iter().enumerate() {
                    buf[k] = y[j];
                }
                if t.lin {
                    chain_forward_lin(&buf[..w], &t.glin)
                } else {
                    chain_forward(&buf[..w], &t.lg)
                }
            }
        }
    }

    /// ln of the (unnormalised) probability of code bits y at temperature t: beta sum h s + sum_r ln Z_r.
    pub fn ln_weight(&self, y: &[u8], t: &Temp) -> f64 {
        let mut l = 0.0;
        for i in 0..self.n {
            l += t.beta * self.lean[i] * if y[i] == 1 { 1.0 } else { -1.0 };
        }
        for r in 0..self.rows.len() {
            l += self.lnz_row(r, y, t);
        }
        l
    }

    pub fn broken(&self, y: &[u8]) -> usize {
        self.rows.iter().filter(|row| row.iter().fold(0u8, |a, &j| a ^ y[j]) != 0).count()
    }

    /// Zero-temperature energy: -sum h s + lambda times the checks broken (every broken check's calmest
    /// helper arrangement costs exactly lambda, for both gadgets).
    pub fn e0(&self, y: &[u8]) -> f64 {
        let mut e = 0.0;
        for i in 0..self.n {
            e -= self.lean[i] * if y[i] == 1 { 1.0 } else { -1.0 };
        }
        e + self.lambda * self.broken(y) as f64
    }

    /// The checks a set of bits touches, each once.
    fn touched(&self, bits: &[usize], out: &mut Vec<usize>) {
        out.clear();
        for &i in bits {
            for &r in &self.var_checks[i] {
                if !out.contains(&r) {
                    out.push(r);
                }
            }
        }
    }

    /// ln weights of the 2^b joint values of `bits` (value v sets bit k to (v >> k) & 1), everything else
    /// held. Only the terms that change are included. Restores y. Adds the ln Z evaluations to `work`.
    pub fn config_logw(&self, y: &mut [u8], bits: &[usize], t: &Temp, work: &mut u64) -> Vec<f64> {
        let mut sc = Scratch::default();
        self.config_logw_into(y, bits, t, work, &mut sc);
        sc.lw
    }

    fn config_logw_into(&self, y: &mut [u8], bits: &[usize], t: &Temp, work: &mut u64, sc: &mut Scratch) {
        // Walk the 2^b values in Gray-code order, so each step flips one bit and only that bit's checks are
        // recomputed. lw[v] is the ln weight of value v.
        self.touched(bits, &mut sc.checks);
        let b = bits.len();
        sc.saved.clear();
        sc.saved.extend(bits.iter().map(|&i| y[i]));
        sc.of_bit.clear();
        for &i in bits {
            let idx: Vec<usize> = self.var_checks[i].iter().map(|r| sc.checks.iter().position(|q| q == r).unwrap()).collect();
            sc.of_bit.push(idx);
        }
        for &i in bits {
            y[i] = 0;
        }
        sc.per.clear();
        let mut total = 0.0;
        for &r in &sc.checks {
            let z = self.lnz_row(r, y, t);
            sc.per.push(z);
            total += z;
        }
        *work += sc.checks.len() as u64;
        let mut lean_sum: f64 = bits.iter().map(|&i| -t.beta * self.lean[i]).sum();
        sc.lw.clear();
        sc.lw.resize(1 << b, 0.0);
        sc.lw[0] = lean_sum + total;
        let mut g = 0usize;
        for step in 1..(1usize << b) {
            let k = step.trailing_zeros() as usize;
            g ^= 1 << k;
            let i = bits[k];
            y[i] ^= 1;
            lean_sum += 2.0 * t.beta * self.lean[i] * if y[i] == 1 { 1.0 } else { -1.0 };
            for &ci in &sc.of_bit[k] {
                let z = self.lnz_row(sc.checks[ci], y, t);
                total += z - sc.per[ci];
                sc.per[ci] = z;
            }
            *work += sc.of_bit[k].len() as u64;
            sc.lw[g] = lean_sum + total;
        }
        for (k, &i) in bits.iter().enumerate() {
            y[i] = sc.saved[k];
        }
    }

    /// ln P(bit i = 1) - ln P(bit i = 0) given every other bit (the single mover's fast path). Restores y.
    pub fn one_bit_log_odds(&self, y: &mut [u8], i: usize, t: &Temp, work: &mut u64) -> f64 {
        let old = y[i];
        let mut d = 2.0 * t.beta * self.lean[i];
        for &r in &self.var_checks[i] {
            y[i] = 1;
            d += self.lnz_row(r, y, t);
            y[i] = 0;
            d -= self.lnz_row(r, y, t);
        }
        *work += 2 * self.var_checks[i].len() as u64;
        y[i] = old;
        d
    }

    /// Draw one joint value of `bits` from its heat-bath conditional and set it.
    fn heat_bath(&self, y: &mut [u8], bits: &[usize], t: &Temp, rng: &mut Rng, work: &mut u64, sc: &mut Scratch) {
        if bits.len() == 1 {
            let d = self.one_bit_log_odds(y, bits[0], t, work);
            let p1 = 1.0 / (1.0 + (-d).exp());
            y[bits[0]] = (rng.unit() < p1) as u8;
            return;
        }
        self.config_logw_into(y, bits, t, work, sc);
        let m = sc.lw.iter().cloned().fold(NEG, f64::max);
        let mut tot = 0.0;
        for l in sc.lw.iter_mut() {
            *l = (*l - m).exp();
            tot += *l;
        }
        let mut u = rng.unit() * tot;
        let mut pick = sc.lw.len() - 1;
        for (v, &w) in sc.lw.iter().enumerate() {
            if u < w {
                pick = v;
                break;
            }
            u -= w;
        }
        for (k, &i) in bits.iter().enumerate() {
            y[i] = ((pick >> k) & 1) as u8;
        }
    }

    /// Choose one block: a check holding a free bit, uniformly; then min(b, its free bits) of them, uniformly.
    pub fn choose_block(&self, b: usize, rng: &mut Rng, out: &mut Vec<usize>) {
        let r = self.block_rows[rng.below(self.block_rows.len())];
        out.clear();
        out.extend(self.rows[r].iter().copied().filter(|&j| self.is_free[j]));
        let take = b.min(out.len());
        for k in 0..take {
            let s = k + rng.below(out.len() - k);
            out.swap(k, s);
        }
        out.truncate(take);
    }

    /// One sweep: `:single` visits every free bit once in a random order; `:block(b)` makes
    /// ceil(free / b) block moves, so each free bit is visited about once.
    pub fn sweep(&self, y: &mut [u8], t: &Temp, mover: Mover, rng: &mut Rng, order: &mut Vec<usize>, work: &mut u64) {
        match mover {
            Mover::Single => {
                order.clear();
                order.extend_from_slice(&self.free);
                for k in (1..order.len()).rev() {
                    let r = rng.below(k + 1);
                    order.swap(k, r);
                }
                let mut sc = Scratch::default();
                for idx in 0..order.len() {
                    let i = order[idx];
                    self.heat_bath(y, &[i], t, rng, work, &mut sc);
                }
            }
            Mover::Block(b) => {
                let moves = (self.free.len() + b - 1) / b.max(1);
                let mut blk = Vec::new();
                let mut sc = Scratch::default();
                for _ in 0..moves {
                    self.choose_block(b, rng, &mut blk);
                    self.heat_bath(y, &blk, t, rng, work, &mut sc);
                }
            }
        }
    }

    fn start(&self, init: Option<&[u8]>, rng: &mut Rng) -> Vec<u8> {
        let mut y: Vec<u8> = match init {
            Some(b) => b.to_vec(),
            None => (0..self.n).map(|_| (rng.unit() < 0.5) as u8).collect(),
        };
        for &j in &self.fixed_zero {
            y[j] = 0;
        }
        y
    }

    fn outcome(&self, bits: Vec<u8>, work: u64) -> Outcome {
        let broken = self.broken(&bits);
        let codeword = broken == 0 && self.fixed_zero.iter().all(|&j| bits[j] == 0);
        Outcome { e0: self.e0(&bits), bits, codeword, broken, work }
    }

    /// Anneal from `init` (or noise), cooling geometrically from t_hi to t_lo over `sweeps`, and return the
    /// calmest bits visited (by the zero-temperature energy), the same rule as LDPCSETTLE's decoder.
    pub fn anneal(&self, init: Option<&[u8]>, sweeps: usize, t_hi: f64, t_lo: f64, mover: Mover, seed: u64) -> Outcome {
        self.anneal_ramp(init, sweeps, t_hi, t_lo, 1.0, 1.0, mover, seed)
    }

    /// Zero-temperature energy with penalty strength `lambda` in place of the code's own.
    pub fn e0_with(&self, y: &[u8], lambda: f64) -> f64 {
        let mut e = 0.0;
        for i in 0..self.n {
            e -= self.lean[i] * if y[i] == 1 { 1.0 } else { -1.0 };
        }
        e + lambda * self.broken(y) as f64
    }

    /// Anneal while the penalty also ramps geometrically from `k_lo` to `k_hi` times the code's lambda (each
    /// sweep keeps the distribution at its own temperature and penalty). The calmest bits are judged by the
    /// zero-temperature energy at the final penalty. With k_lo = k_hi = 1 this is `anneal`.
    #[allow(clippy::too_many_arguments)]
    pub fn anneal_ramp(&self, init: Option<&[u8]>, sweeps: usize, t_hi: f64, t_lo: f64, k_lo: f64, k_hi: f64, mover: Mover, seed: u64) -> Outcome {
        let mut rng = Rng::new(seed);
        let mut y = self.start(init, &mut rng);
        let lam_end = self.lambda * k_hi;
        let mut best = (y.clone(), self.e0_with(&y, lam_end));
        let mut order = Vec::new();
        let mut work = 0u64;
        for step in 0..sweeps {
            let f = step as f64 / (sweeps.max(2) - 1) as f64;
            let temp = t_hi * (t_lo / t_hi).powf(f);
            let k = k_lo * (k_hi / k_lo).powf(f);
            let t = self.temp_with(1.0 / temp, self.lambda * k);
            self.sweep(&mut y, &t, mover, &mut rng, &mut order, &mut work);
            let e = self.e0_with(&y, lam_end);
            if e < best.1 - 1e-9 {
                best = (y.clone(), e);
            }
        }
        let mut o = self.outcome(best.0, work);
        o.e0 = self.e0_with(&o.bits, lam_end);
        o
    }

    /// Stay at inverse temperature beta for `sweeps`, average each bit over the sweeps after `burn`, and return
    /// each bit's more likely value (the bitwise posterior decision at beta = 1). Also returns the averages.
    pub fn average(&self, init: Option<&[u8]>, sweeps: usize, burn: usize, beta: f64, mover: Mover, seed: u64) -> (Outcome, Vec<f64>) {
        let mut rng = Rng::new(seed);
        let mut y = self.start(init, &mut rng);
        let t = self.temp(beta);
        let mut order = Vec::new();
        let mut work = 0u64;
        let mut acc = vec![0.0; self.n];
        let mut kept = 0usize;
        for step in 0..sweeps {
            self.sweep(&mut y, &t, mover, &mut rng, &mut order, &mut work);
            if step >= burn {
                for i in 0..self.n {
                    acc[i] += y[i] as f64;
                }
                kept += 1;
            }
        }
        let avg: Vec<f64> = acc.iter().map(|a| a / kept.max(1) as f64).collect();
        let init_bits = init.map(|b| b.to_vec()).unwrap_or_else(|| vec![0; self.n]);
        let bits: Vec<u8> = (0..self.n)
            .map(|i| {
                if self.fixed_zero.contains(&i) {
                    0
                } else if (avg[i] - 0.5).abs() < 1e-12 {
                    init_bits[i]
                } else {
                    (avg[i] > 0.5) as u8
                }
            })
            .collect();
        (self.outcome(bits, work), avg)
    }
}

/// The hard channel leans for a received word at flip probability p (the same leans LDPCSETTLE uses).
pub fn hard_leans(recv: &[u8], p: f64) -> Vec<f64> {
    recv.iter().map(|&b| hard_lean(b, p.max(1e-6))).collect()
}

// ---------------------------------------------------------------------------------------------------------
// Statements. They run on a code declared by the ldpcsettle family (`ldpc :c, ...` and `c.transmit`).

fn key(name: &str) -> String {
    format!("ldpcsettle:{}", name)
}

fn sym(t: &Tok, what: &str, ln: usize) -> Result<String, SettleError> {
    match t {
        Tok::Sym(s) => Ok(s.clone()),
        _ => err(ln, format!("{} takes a symbol like :block", what)),
    }
}

fn decode(m: &Model, st: &mut State, name: &str, rest: &[Tok], ln: usize, ctx: &mut Ctx, nishimori: bool) -> Result<(), SettleError> {
    let (nums, words) = m.notes[&key(name)].clone();
    let (n, rate, strength, cb) = (nums[0] as usize, nums[1], nums[2], nums[3] as u64);
    let gadget = Gadget::parse(&words[0]).unwrap_or(Gadget::Chain);
    let Some((sent, _)) = m.notes.get(&format!("{}:sent", key(name))) else {
        return err(ln, format!("{}.{} needs {}.transmit first", name, if nishimori { "decode_nishimori" } else { "decode_moves" }, name));
    };
    let sent: Vec<u8> = sent.iter().map(|&v| v as u8).collect();
    let (recv, pw) = m.notes[&format!("{}:recv", key(name))].clone();
    let recv: Vec<u8> = recv.iter().map(|&v| v as u8).collect();
    let p: f64 = pw[0].parse().unwrap_or(0.03);
    let kv = kwargs(rest, ln)?;
    only(&kv, &["mover", "block", "sweeps", "seed"], if nishimori { "decode_nishimori" } else { "decode_moves" }, ln)?;
    if let Some(v) = kw(&kv, "seed") {
        st.rng = Rng::new(num(v, ln)? as u64);
    }
    let b = whole(kw(&kv, "block").map(|v| num(v, ln)).transpose()?.unwrap_or(4.0), 0.0, f64::INFINITY, "block:", ln)?;
    if !(1..=10).contains(&b) {
        return err(ln, "block must be between 1 and 10");
    }
    let mover = match kw(&kv, "mover") {
        Some(t) => match sym(t, "mover:", ln)?.as_str() {
            "single" => Mover::Single,
            "block" => Mover::Block(b),
            other => return err(ln, format!("mover is :single or :block, not :{}", other)),
        },
        None => Mover::Block(b),
    };
    let sweeps = whole(kw(&kv, "sweeps").map(|v| num(v, ln)).transpose()?.unwrap_or(if nishimori { 2000.0 } else { 400.0 }), 0.0, f64::INFINITY, "sweeps:", ln)?;
    if sweeps < 2 {
        return err(ln, "sweeps must be at least 2");
    }
    let l = coded_ldpc(n, rate, cb);
    let mut c = Collapsed::from_ldpc(&l, gadget, strength);
    c.set_leans(&hard_leans(&recv, p));
    let seed = st.rng.next_u64();
    let (o, how) = if nishimori {
        (c.average(Some(&recv), sweeps, sweeps / 2, 1.0, mover, seed).0, format!("averaged at T 1 ({})", mover.name()))
    } else {
        (c.anneal(Some(&recv), sweeps, 1.0, 0.05, mover, seed), format!("settled with {} moves", mover.name()))
    };
    if o.codeword {
        let wrong = o.bits.iter().zip(&sent).filter(|(a, c)| a != c).count();
        let verdict = if wrong == 0 { "the sent codeword" } else { "a DIFFERENT codeword (miscorrection)" };
        ctx.say(format!("{} {}: CODEWORD, {}; {} bits differ from what was sent", name, how, verdict, wrong));
    } else {
        ctx.say(format!("{} {}: REFUSED, {} checks still broken", name, how, o.broken));
    }
    Ok(())
}

impl Ext for LdpcMoves {
    fn name(&self) -> &'static str {
        "ldpcmoves"
    }

    fn statements(&self) -> &'static [&'static str] {
        &["run: c.decode_moves mover: :block, block: 4, sweeps: 400, seed: 2", "run: c.decode_nishimori mover: :block, sweeps: 2000, seed: 3"]
    }

    fn run_stmt(&self, m: &mut Model, st: &mut State, t: &[Tok], ln: usize, ctx: &mut Ctx) -> Claim {
        match t {
            [Tok::Ident(name), Tok::Dot, Tok::Ident(v), rest @ ..] if m.notes.contains_key(&key(name)) => match v.as_str() {
                "decode_moves" => Some(decode(m, st, name, rest, ln, ctx, false)),
                "decode_nishimori" => Some(decode(m, st, name, rest, ln, ctx, true)),
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
    use crate::ldpcsettle::SettleCode;

    /// ln Z of one check, by listing every helper arrangement of the ldpcsettle gadget itself.
    fn brute_lnz(g: Gadget, bits: &[u8], bl: f64) -> f64 {
        let w = bits.len();
        let row: Vec<usize> = (0..w).collect();
        let sc = SettleCode::new(&[row], w, &[], g, 1.0);
        let h = sc.helpers();
        let mut z = NEG;
        for hb in 0u64..(1 << h) {
            let y: Vec<bool> = bits.iter().map(|&b| b == 1).chain((0..h).map(|k| (hb >> k) & 1 == 1)).collect();
            z = lse(z, -bl * sc.penalty(&y));
        }
        z
    }

    #[test]
    fn summed_out_helpers_match_enumeration_of_the_gadget() {
        for &bl in &[0.3, 1.0, 2.5, 40.0] {
            for (g, top) in [(Gadget::Sum, 10usize), (Gadget::Chain, 9)] {
                for w in 1..=top {
                    for data in 0u64..(1 << w) {
                        let bits: Vec<u8> = (0..w).map(|i| ((data >> i) & 1) as u8).collect();
                        let a = ln_z_check(g, &bits, bl);
                        let b = brute_lnz(g, &bits, bl);
                        assert!((a - b).abs() < 1e-9 * (1.0 + b.abs()), "{:?} w {} {:b} bl {}: {} vs {}", g, w, data, bl, a, b);
                    }
                }
            }
        }
    }

    #[test]
    fn the_fast_chain_pass_matches_the_log_pass() {
        let mut r = Rng::new(4);
        for &bl in &[0.01, 0.5, 3.0, 40.0, 200.0, 249.0] {
            let lg = chain_lg(bl);
            let gl = [lg[0].exp(), lg[1].exp(), lg[2].exp(), lg[3].exp()];
            for w in 3..=40 {
                for _ in 0..20 {
                    let bits: Vec<u8> = (0..w).map(|_| (r.unit() < 0.5) as u8).collect();
                    let a = chain_forward(&bits, &lg);
                    let b = chain_forward_lin(&bits, &gl);
                    assert!((a - b).abs() < 1e-9 * (1.0 + a.abs()), "bl {} w {}: {} vs {}", bl, w, a, b);
                }
            }
        }
    }

    fn tiny_codes() -> Vec<(usize, Vec<Vec<usize>>)> {
        vec![
            (7, vec![vec![0, 1, 2, 4], vec![0, 1, 3, 5], vec![0, 2, 3, 6]]),
            (8, vec![vec![0, 1, 2], vec![2, 3, 4], vec![4, 5, 6, 7], vec![0, 7]]),
            (9, vec![vec![0, 1, 2, 3, 4], vec![3, 5, 6], vec![1, 6, 7, 8]]),
        ]
    }

    fn leans(n: usize, seed: u64) -> Vec<f64> {
        let mut r = Rng::new(seed);
        (0..n).map(|_| 1.2 * r.signed()).collect()
    }

    /// The code-bit distribution of the summed-out model equals the SETTLE model's full distribution over
    /// every thing (code bits and helpers), summed over the helpers.
    #[test]
    fn the_code_bit_marginal_is_the_settle_models_own() {
        for (n, rows) in tiny_codes() {
            for g in [Gadget::Sum, Gadget::Chain] {
                for &beta in &[1.0, 0.6] {
                    let lam = 1.3;
                    let lean = leans(n, n as u64 * 7 + beta as u64);
                    let mut sc = SettleCode::new(&rows, n, &[], g, lam);
                    sc.set_leans(&lean);
                    let mut full = vec![NEG; 1 << n];
                    for a in 0u64..(1 << sc.things) {
                        let s: Vec<f64> = (0..sc.things).map(|i| if (a >> i) & 1 == 1 { 1.0 } else { -1.0 }).collect();
                        let k = (a & ((1 << n) - 1)) as usize;
                        full[k] = lse(full[k], -beta * sc.energy(&s));
                    }
                    let mut c = Collapsed::new(&rows, n, &[], g, lam);
                    c.set_leans(&lean);
                    let t = c.temp(beta);
                    let mine: Vec<f64> = (0..(1usize << n))
                        .map(|k| c.ln_weight(&(0..n).map(|i| ((k >> i) & 1) as u8).collect::<Vec<_>>(), &t))
                        .collect();
                    let nf = full.iter().fold(NEG, |a, &b| lse(a, b));
                    let nm = mine.iter().fold(NEG, |a, &b| lse(a, b));
                    for k in 0..(1usize << n) {
                        assert!(((full[k] - nf).exp() - (mine[k] - nm).exp()).abs() < 1e-10, "{:?} n {} state {}", g, n, k);
                    }
                }
            }
        }
    }

    fn exact_pi(c: &Collapsed, t: &Temp) -> Vec<f64> {
        let n = c.n;
        let lw: Vec<f64> = (0..(1usize << n)).map(|k| c.ln_weight(&(0..n).map(|i| ((k >> i) & 1) as u8).collect::<Vec<_>>(), t)).collect();
        let z = lw.iter().fold(NEG, |a, &b| lse(a, b));
        lw.iter().map(|&l| (l - z).exp()).collect()
    }

    fn state_bits(k: usize, n: usize) -> Vec<u8> {
        (0..n).map(|i| ((k >> i) & 1) as u8).collect()
    }

    fn index(y: &[u8]) -> usize {
        y.iter().enumerate().map(|(i, &b)| (b as usize) << i).sum()
    }

    /// Every block a mover can choose, with its probability (the random-scan kernel of one move).
    fn blocks_of(c: &Collapsed, mover: Mover) -> Vec<(f64, Vec<usize>)> {
        match mover {
            Mover::Single => c.free.iter().map(|&i| (1.0 / c.free.len() as f64, vec![i])).collect(),
            Mover::Block(b) => {
                let mut out = Vec::new();
                for &r in &c.block_rows {
                    let fr: Vec<usize> = c.rows[r].iter().copied().filter(|&j| c.is_free[j]).collect();
                    let take = b.min(fr.len());
                    let subsets: Vec<Vec<usize>> = (0u64..(1 << fr.len()))
                        .filter(|m| m.count_ones() as usize == take)
                        .map(|m| (0..fr.len()).filter(|&k| (m >> k) & 1 == 1).map(|k| fr[k]).collect())
                        .collect();
                    let ns = subsets.len() as f64;
                    for s in subsets {
                        out.push((1.0 / (c.block_rows.len() as f64 * ns), s));
                    }
                }
                out
            }
        }
    }

    /// The exact one-move transition matrix; `local` is the mutant that drops every touched check but the first.
    fn kernel(c: &Collapsed, t: &Temp, mover: Mover, local: bool) -> Vec<Vec<f64>> {
        let n = c.n;
        let size = 1usize << n;
        let mut p = vec![vec![0.0; size]; size];
        let blocks = blocks_of(c, mover);
        for x in 0..size {
            let mut y = state_bits(x, n);
            for (q, blk) in &blocks {
                let lw = if local {
                    let r0 = c.var_checks[blk[0]][0];
                    let saved: Vec<u8> = blk.iter().map(|&i| y[i]).collect();
                    let v: Vec<f64> = (0..(1usize << blk.len()))
                        .map(|v| {
                            let mut l = 0.0;
                            for (k, &i) in blk.iter().enumerate() {
                                y[i] = ((v >> k) & 1) as u8;
                                l += t.beta * c.lean[i] * if y[i] == 1 { 1.0 } else { -1.0 };
                            }
                            l + c.lnz_row(r0, &y, t)
                        })
                        .collect();
                    for (k, &i) in blk.iter().enumerate() {
                        y[i] = saved[k];
                    }
                    v
                } else {
                    let mut w = 0;
                    c.config_logw(&mut y, blk, t, &mut w)
                };
                let z = lw.iter().fold(NEG, |a, &b| lse(a, b));
                for (v, &l) in lw.iter().enumerate() {
                    let mut to = y.clone();
                    for (k, &i) in blk.iter().enumerate() {
                        to[i] = ((v >> k) & 1) as u8;
                    }
                    p[x][index(&to)] += q * (l - z).exp();
                }
            }
        }
        p
    }

    fn worst_balance(pi: &[f64], p: &[Vec<f64>]) -> f64 {
        let mut worst = 0.0f64;
        for x in 0..pi.len() {
            let row: f64 = p[x].iter().sum();
            assert!((row - 1.0).abs() < 1e-9, "rows sum to one");
            for y in 0..pi.len() {
                worst = worst.max((pi[x] * p[x][y] - pi[y] * p[y][x]).abs());
            }
        }
        worst
    }

    /// Detailed balance by exact transition matrices: pi(x) P(x, y) = pi(y) P(y, x) for every pair, both
    /// gadgets, the single mover and blocks of 2 and 3, on three small codes. A mutant that leaves out all
    /// but one touched check breaks it, so the test can fail.
    #[test]
    fn every_mover_keeps_detailed_balance_exactly() {
        for (n, rows) in tiny_codes() {
            for g in [Gadget::Sum, Gadget::Chain] {
                let mut c = Collapsed::new(&rows, n, &[], g, 1.1);
                c.set_leans(&leans(n, 3 + n as u64));
                let t = c.temp(0.9);
                let pi = exact_pi(&c, &t);
                for mover in [Mover::Single, Mover::Block(2), Mover::Block(3)] {
                    let p = kernel(&c, &t, mover, false);
                    let wb = worst_balance(&pi, &p);
                    assert!(wb < 1e-14, "{:?} {:?} n {}: violation {}", g, mover, n, wb);
                }
                let bad = kernel(&c, &t, Mover::Block(2), true);
                assert!(worst_balance(&pi, &bad) > 1e-4, "the mutant must break balance ({:?} n {})", g, n);
            }
        }
        // a fixed-zero bit never moves and balance still holds on the rest
        let (n, rows) = tiny_codes()[0].clone();
        let mut c = Collapsed::new(&rows, n, &[3], Gadget::Chain, 1.1);
        c.set_leans(&leans(n, 11));
        let t = c.temp(1.0);
        for mover in [Mover::Single, Mover::Block(3)] {
            let p = kernel(&c, &t, mover, false);
            for x in 0..(1usize << n) {
                for y in 0..(1usize << n) {
                    if p[x][y] > 0.0 {
                        assert_eq!((x >> 3) & 1, (y >> 3) & 1);
                    }
                }
            }
        }
    }

    /// The single mover's fast path gives the same log odds as the general block weights.
    #[test]
    fn the_one_bit_fast_path_matches_the_block_weights() {
        let l = coded_ldpc(128, 0.75, 1);
        let mut r = Rng::new(21);
        for g in [Gadget::Sum, Gadget::Chain] {
            let mut c = Collapsed::from_ldpc(&l, g, 3.1);
            c.set_leans(&leans(128, 8));
            for &beta in &[0.05, 1.0, 20.0] {
                let t = c.temp(beta);
                let mut y: Vec<u8> = (0..128).map(|_| (r.unit() < 0.5) as u8).collect();
                for i in [0usize, 17, 63, 127] {
                    let mut w = 0;
                    let lw = c.config_logw(&mut y, &[i], &t, &mut w);
                    let d = c.one_bit_log_odds(&mut y, i, &t, &mut w);
                    assert!((d - (lw[1] - lw[0])).abs() < 1e-9 * (1.0 + d.abs()), "{:?} beta {} bit {}", g, beta, i);
                }
            }
        }
    }

    /// The sweep code (random order, block counts) samples the exact distribution: its total variation from
    /// the exact answer stays within 4 times that of the same number of independent exact draws.
    #[test]
    fn long_runs_sample_the_exact_distribution() {
        let (n, rows) = tiny_codes()[0].clone();
        for g in [Gadget::Sum, Gadget::Chain] {
            let mut c = Collapsed::new(&rows, n, &[], g, 0.9);
            c.set_leans(&leans(n, 5));
            let t = c.temp(1.0);
            let pi = exact_pi(&c, &t);
            for mover in [Mover::Single, Mover::Block(3)] {
                let mut rng = Rng::new(17);
                let mut y = vec![0u8; n];
                let mut hist = vec![0.0; 1 << n];
                let (mut order, mut w) = (Vec::new(), 0u64);
                let sweeps = 100_000;
                for _ in 0..sweeps {
                    c.sweep(&mut y, &t, mover, &mut rng, &mut order, &mut w);
                    hist[index(&y)] += 1.0 / sweeps as f64;
                }
                let tv: f64 = pi.iter().zip(&hist).map(|(a, b)| (a - b).abs()).sum::<f64>() / 2.0;
                // the noise floor: the same number of independent draws from pi
                let mut iid = vec![0.0; 1 << n];
                let mut r2 = Rng::new(99);
                for _ in 0..sweeps {
                    let mut u = r2.unit();
                    let mut k = pi.len() - 1;
                    for (s, &q) in pi.iter().enumerate() {
                        if u < q {
                            k = s;
                            break;
                        }
                        u -= q;
                    }
                    iid[k] += 1.0 / sweeps as f64;
                }
                let tv_iid: f64 = pi.iter().zip(&iid).map(|(a, b)| (a - b).abs()).sum::<f64>() / 2.0;
                assert!(tv < 4.0 * tv_iid && tv < 0.03, "{:?} {:?}: tv {} against independent draws {}", g, mover, tv, tv_iid);
                // negative control: the same histogram against a different distribution is far off
                let off: f64 = hist.iter().take(pi.len()).map(|b| (1.0 / pi.len() as f64 - b).abs()).sum::<f64>() / 2.0;
                assert!(off > 0.1 && off > 10.0 * tv, "the test can tell distributions apart: {}", off);
            }
        }
    }

    #[test]
    fn at_zero_noise_every_decoder_returns_the_sent_codeword() {
        let l = coded_ldpc(128, 0.5, 1);
        for g in [Gadget::Sum, Gadget::Chain] {
            let mut c = Collapsed::from_ldpc(&l, g, (0.99f64 / 0.01).ln());
            let mut r = Rng::new(9);
            for s in 0..3 {
                let info: Vec<u8> = (0..l.info.len()).map(|_| (r.unit() < 0.5) as u8).collect();
                let cw = l.encode(&info);
                c.set_leans(&hard_leans(&cw, 0.01));
                for mover in [Mover::Single, Mover::Block(4)] {
                    let a = c.anneal(Some(&cw), 40, 1.0, 0.05, mover, s);
                    assert!(a.codeword && a.bits == cw, "{:?} {:?} anneal", g, mover);
                    let (b, _) = c.average(Some(&cw), 60, 30, 1.0, mover, s);
                    assert!(b.codeword && b.bits == cw, "{:?} {:?} average", g, mover);
                }
            }
        }
    }

    /// The penalty ramp with a flat schedule is the plain annealer, bit for bit; a real ramp still corrects flips.
    #[test]
    fn a_flat_ramp_is_the_plain_annealer_and_a_real_ramp_decodes() {
        let l = coded_ldpc(128, 0.5, 1);
        let p: f64 = 0.02;
        let mut r = Rng::new(6);
        let info: Vec<u8> = (0..l.info.len()).map(|_| (r.unit() < 0.5) as u8).collect();
        let cw = l.encode(&info);
        let mut y = cw.clone();
        for j in [5, 60, 101] {
            y[j] ^= 1;
        }
        for g in [Gadget::Sum, Gadget::Chain] {
            let mut c = Collapsed::from_ldpc(&l, g, ((1.0 - p) / p).ln());
            c.set_leans(&hard_leans(&y, p));
            let a = c.anneal(Some(&y), 60, 1.0, 0.05, Mover::Block(4), 3);
            let b = c.anneal_ramp(Some(&y), 60, 1.0, 0.05, 1.0, 1.0, Mover::Block(4), 3);
            assert_eq!(a.bits, b.bits);
            assert_eq!(a.work, b.work);
            let d = c.anneal_ramp(Some(&y), 100, 1.0, 0.05, 1.0, 4.0, Mover::Block(4), 3);
            assert!(d.codeword && d.bits == cw, "{:?}", g);
        }
    }

    #[test]
    fn a_non_codeword_target_is_never_claimed() {
        let l = coded_ldpc(128, 0.5, 1);
        let mut c = Collapsed::from_ldpc(&l, Gadget::Chain, 0.2);
        let mut r = Rng::new(3);
        for s in 0..3 {
            let target: Vec<u8> = (0..128).map(|_| (r.unit() < 0.5) as u8).collect();
            assert!(c.broken(&target) > 0);
            c.set_leans(&hard_leans(&target, 1e-4));
            let a = c.anneal(Some(&target), 20, 0.5, 0.05, Mover::Block(4), s);
            assert_eq!(a.bits, target);
            assert!(!a.codeword);
        }
    }

    #[test]
    fn three_flips_are_corrected_by_both_movers() {
        let l = coded_ldpc(128, 0.5, 1);
        let p: f64 = 0.02;
        let mut r = Rng::new(5);
        let info: Vec<u8> = (0..l.info.len()).map(|_| (r.unit() < 0.5) as u8).collect();
        let cw = l.encode(&info);
        let mut y = cw.clone();
        for j in [3, 40, 90] {
            y[j] ^= 1;
        }
        for g in [Gadget::Sum, Gadget::Chain] {
            let mut c = Collapsed::from_ldpc(&l, g, ((1.0 - p) / p).ln());
            c.set_leans(&hard_leans(&y, p));
            for mover in [Mover::Single, Mover::Block(4)] {
                let a = c.anneal(Some(&y), 100, 1.0, 0.05, mover, 1);
                assert!(a.codeword && a.bits == cw, "{:?} {:?}", g, mover);
            }
        }
    }

    #[test]
    fn the_statements_run() {
        let src = "model :line do
  ldpc :c, bits: 64, rate: 0.5, gadget: :chain, strength: 4.6
end
run :line do
  c.transmit flip: 0.0, seed: 1
  c.decode_moves mover: :block, block: 4, sweeps: 30, seed: 2
  c.decode_moves mover: :single, sweeps: 30, seed: 2
  c.decode_nishimori mover: :block, sweeps: 40, seed: 3
end";
        let out = Interp::default().exec(src).unwrap_or_else(|e| panic!("{}", e));
        let joined = out.join("\n");
        assert!(joined.contains("settled with block4 moves: CODEWORD, the sent codeword"), "{}", joined);
        assert!(joined.contains("settled with single moves: CODEWORD, the sent codeword"), "{}", joined);
        assert!(joined.contains("averaged at T 1 (block4): CODEWORD, the sent codeword"), "{}", joined);
        let bad = "model :x do\n  ldpc :c, bits: 64\nend\nrun :x do\n  c.transmit flip: 0.0, seed: 1\n  c.decode_moves mover: :swirl\nend";
        assert!(Interp::default().exec(bad).is_err());
    }
}
