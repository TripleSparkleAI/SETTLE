//! DENOISE: a chain of small restricted machines that turns coin noise into examples one small step at a
//! time, in the style of a denoising thermodynamic model (Jelinčič et al., arXiv 2510.23972).
//!
//! ```text
//! model :digits do
//!   examples :train, "train.txt"                  # rows of yes/no pixels (the learn family's examples)
//!   denoiser :d, steps: 8, hidden: 64, over: :train, width: 8
//! end
//! run :digits do
//!   d.generate 16, out: "untrained.pgm", seed: 1  # before training: random pulls, the negative control
//!   d.train :train, rounds: 200, rate: 0.05, sweeps: 1, batch: 50, seed: 1   # leans: :zero starts leans at 0
//!   d.generate 16, out: "samples.pgm", rows: "samples.txt", chain: "chain.pgm", sweeps: 100, seed: 2
//!   sample :train, 16, sweeps: 800, out: "direct.pgm"   # sample the model's own learned machine directly
//!   coins :train, 16, out: "coins.pgm"                  # every pixel an independent coin at its data rate
//! end
//! ```
//!
//! The forward process flips bits. Level t keeps a correlation rho_t = 1 - t/T with the clean example, so
//! level 0 is the data and level T is pure coin noise. Machine t (t = 1..T) is a restricted machine over the
//! less noisy picture v = x_{t-1} and `hidden:` hidden things, with the noisier picture x = x_t HELD. Its energy
//!
//!   E_t(v, z; x) = -a.v - b.z - v.W.z - x.C.z - gamma_t v.x,   gamma_t = atanh(rho_t / rho_{t-1})
//!
//! has a fixed diagonal pull gamma_t from each held pixel to its own pixel one level down. That pull is the
//! forward process's own likelihood, so by Bayes' rule the exact reverse step is (a model of level t-1)
//! times this pull; a, b, W learn the model and C lets the hidden things also read the held picture.
//! gamma_T = 0: the last machine sees pure noise and has to make a picture from nothing, which is what
//! a single machine sampled directly does. With T = 1 the stack is exactly that single machine (plus C).
//!
//! Learning is contrastive divergence conditioned on the held picture, with fresh noise every round; each
//! machine trains on its own thread. Generation starts from coins, and each machine settles `sweeps:` times
//! with the previous output held and hands its last arrangement down. Updates use the p-bit rule of `sweep`
//! in model.rs (a thing is yes with chance (1 + tanh(input)) / 2), in blocks: hidden things all at once, then
//! visible things, which is exact Gibbs sampling because each layer only pulls on the other.
//!
//! `sample` settles the model's own things (for example the restricted machine the learn family fits with
//! `hidden 64`) from a random start and reads the named examples' things; `coins` draws each pixel alone at
//! its rate in the examples. All three write a rows file in the examples format (first line the names, then
//! 1/0 rows), so their output can be read back as examples, and report how close each sample comes to the
//! nearest training example: a sample identical to a training row is counted, because it is a copy.

use crate::ext::{Claim, Ctx, Ext};
use crate::grid::{write_pgm, Pgm};
use crate::learn::{load_examples, Examples};
use crate::lex::{err, kw, kwargs, num, only, text, SettleError, Tok};
use crate::model::{Model, State};
use crate::rng::Rng;
use std::time::Instant;

pub struct Denoise;

/// One denoising step: a restricted machine over `nv` visible and `nh` hidden things with `nv` held inputs.
#[derive(Clone, Debug, PartialEq)]
pub struct Machine {
    pub a: Vec<f64>,
    pub b: Vec<f64>,
    /// nv x nh, visible-major: w[i * nh + k].
    pub w: Vec<f64>,
    /// nv x nh, held input i to hidden k.
    pub c: Vec<f64>,
}

/// The whole chain: `machines[t - 1]` maps level t to level t - 1.
#[derive(Clone, Debug, PartialEq)]
pub struct Stack {
    pub steps: usize,
    pub nv: usize,
    pub nh: usize,
    pub trained: bool,
    pub machines: Vec<Machine>,
}

/// Correlation of level t with the clean example: 1 at t = 0, 0 at t = T.
pub fn rho(t: usize, steps: usize) -> f64 {
    1.0 - t as f64 / steps as f64
}

/// Chance that one forward step t flips a bit: correlations multiply, so 1 - 2q = rho_t / rho_{t-1}.
pub fn flip_chance(t: usize, steps: usize) -> f64 {
    0.5 * (1.0 - rho(t, steps) / rho(t - 1, steps))
}

/// The fixed pull from a held pixel at level t to the same pixel at level t - 1 (the forward likelihood).
pub fn gamma(t: usize, steps: usize) -> f64 {
    (rho(t, steps) / rho(t - 1, steps)).atanh()
}

fn flip(v: &[f64], p: f64, rng: &mut Rng) -> Vec<f64> {
    v.iter().map(|&x| if rng.unit() < p { -x } else { x }).collect()
}

fn pbit(input: f64, rng: &mut Rng) -> f64 {
    if input.tanh() > rng.signed() {
        1.0
    } else {
        -1.0
    }
}

impl Machine {
    pub fn random(nv: usize, nh: usize, rng: &mut Rng) -> Machine {
        Machine {
            a: vec![0.0; nv],
            b: vec![0.0; nh],
            w: (0..nv * nh).map(|_| 0.1 * rng.normal()).collect(),
            c: (0..nv * nh).map(|_| 0.1 * rng.normal()).collect(),
        }
    }

    /// C^T x: what the held picture says to each hidden thing (fixed while x is held).
    fn from_input(&self, x: &[f64], nh: usize) -> Vec<f64> {
        let mut cx = vec![0.0; nh];
        for (i, &xi) in x.iter().enumerate() {
            let row = &self.c[i * nh..(i + 1) * nh];
            cx.iter_mut().zip(row).for_each(|(o, &c)| *o += c * xi);
        }
        cx
    }

    fn hidden_inputs(&self, v: &[f64], cx: &[f64], nh: usize) -> Vec<f64> {
        let mut u: Vec<f64> = self.b.iter().zip(cx).map(|(b, c)| b + c).collect();
        for (i, &vi) in v.iter().enumerate() {
            let row = &self.w[i * nh..(i + 1) * nh];
            u.iter_mut().zip(row).for_each(|(o, &w)| *o += w * vi);
        }
        u
    }

    fn visible_input(&self, i: usize, z: &[f64], x: &[f64], g: f64, nh: usize) -> f64 {
        self.a[i] + self.w[i * nh..(i + 1) * nh].iter().zip(z).map(|(w, z)| w * z).sum::<f64>() + g * x[i]
    }

    /// `sweeps` rounds of hidden-then-visible updates with x held, starting from v; returns the last v.
    pub fn settle(&self, v: &mut [f64], x: &[f64], g: f64, sweeps: usize, rng: &mut Rng) {
        let nh = self.b.len();
        let cx = self.from_input(x, nh);
        for _ in 0..sweeps {
            let z: Vec<f64> = self.hidden_inputs(v, &cx, nh).into_iter().map(|u| pbit(u, rng)).collect();
            for i in 0..v.len() {
                v[i] = pbit(self.visible_input(i, &z, x, g, nh), rng);
            }
        }
    }

    /// Exact P(v | x) by enumerating visible and hidden arrangements (tests only; tiny machines).
    pub fn exact_conditional(&self, x: &[f64], g: f64) -> Vec<f64> {
        let (nv, nh) = (self.a.len(), self.b.len());
        let cx = self.from_input(x, nh);
        let lw: Vec<f64> = (0u64..(1 << nv))
            .map(|bits| {
                let v: Vec<f64> = (0..nv).map(|i| if (bits >> i) & 1 == 1 { 1.0 } else { -1.0 }).collect();
                let lin: f64 = (0..nv).map(|i| (self.a[i] + g * x[i]) * v[i]).sum();
                // hidden things sum out: each contributes ln 2cosh(input)
                lin + self.hidden_inputs(&v, &cx, nh).iter().map(|u| u.abs() + (-2.0 * u.abs()).exp().ln_1p()).sum::<f64>()
            })
            .collect();
        let mx = lw.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        let z: f64 = lw.iter().map(|l| (l - mx).exp()).sum();
        lw.iter().map(|l| (l - mx).exp() / z).collect()
    }
}

/// Settings for `train`.
#[derive(Clone, Debug)]
pub struct TrainOpts {
    pub rounds: usize,
    pub rate: f64,
    pub sweeps: usize,
    pub batch: usize,
    pub decay: f64,
    pub seed: u64,
    /// Start each visible lean at the data's rate for its noise level (`leans: :data`, the default) or at 0.
    pub data_leans: bool,
}

/// Fit machine t (1-based) of a `steps`-long chain to the rows by conditional contrastive divergence.
pub fn train_machine(m: &mut Machine, rows: &[Vec<f64>], t: usize, steps: usize, fresh: bool, o: &TrainOpts) {
    let nv = m.a.len();
    let nh = m.b.len();
    let mut rng = Rng::new(o.seed.wrapping_mul(1_000_003).wrapping_add(t as u64 * 7_919));
    let p_prev = 0.5 * (1.0 - rho(t - 1, steps));
    let q = flip_chance(t, steps);
    let g = gamma(t, steps);
    if fresh && o.data_leans {
        // start each visible lean at the rate the data has at this noise level, clipped away from certainty
        for i in 0..nv {
            let mean = rows.iter().map(|r| r[i]).sum::<f64>() / rows.len() as f64;
            m.a[i] = (rho(t - 1, steps) * mean).clamp(-0.9, 0.9).atanh();
        }
    }
    let batch = o.batch.clamp(1, rows.len());
    let mut order: Vec<usize> = (0..rows.len()).collect();
    let (mut ga, mut gb, mut gw, mut gc) = (vec![0.0; nv], vec![0.0; nh], vec![0.0; nv * nh], vec![0.0; nv * nh]);
    for round in 0..o.rounds {
        let rate = o.rate * (1.0 - 0.9 * round as f64 / o.rounds.max(1) as f64);
        for j in (1..order.len()).rev() {
            let r = rng.below(j + 1);
            order.swap(j, r);
        }
        for chunk in order.chunks(batch) {
            ga.iter_mut().for_each(|x| *x = 0.0);
            gb.iter_mut().for_each(|x| *x = 0.0);
            gw.iter_mut().for_each(|x| *x = 0.0);
            gc.iter_mut().for_each(|x| *x = 0.0);
            for &r in chunk {
                let vpos = flip(&rows[r], p_prev, &mut rng);
                let x = flip(&vpos, q, &mut rng);
                let cx = m.from_input(&x, nh);
                let zpos: Vec<f64> = m.hidden_inputs(&vpos, &cx, nh).iter().map(|u| u.tanh()).collect();
                let mut v = vpos.clone();
                for _ in 0..o.sweeps {
                    let z: Vec<f64> = m.hidden_inputs(&v, &cx, nh).into_iter().map(|u| pbit(u, &mut rng)).collect();
                    for i in 0..nv {
                        v[i] = pbit(m.visible_input(i, &z, &x, g, nh), &mut rng);
                    }
                }
                let zneg: Vec<f64> = m.hidden_inputs(&v, &cx, nh).iter().map(|u| u.tanh()).collect();
                let dz: Vec<f64> = zpos.iter().zip(&zneg).map(|(p, n)| p - n).collect();
                for i in 0..nv {
                    ga[i] += vpos[i] - v[i];
                    let (row_w, row_c) = (&mut gw[i * nh..(i + 1) * nh], &mut gc[i * nh..(i + 1) * nh]);
                    for k in 0..nh {
                        row_w[k] += vpos[i] * zpos[k] - v[i] * zneg[k];
                        row_c[k] += x[i] * dz[k];
                    }
                }
                gb.iter_mut().zip(&dz).for_each(|(o, d)| *o += d);
            }
            let step = rate / chunk.len() as f64;
            m.a.iter_mut().zip(&ga).for_each(|(p, d)| *p += step * d);
            m.b.iter_mut().zip(&gb).for_each(|(p, d)| *p += step * d);
            for (p, d) in m.w.iter_mut().zip(&gw) {
                *p += step * d - rate * o.decay * *p;
            }
            for (p, d) in m.c.iter_mut().zip(&gc) {
                *p += step * d - rate * o.decay * *p;
            }
        }
    }
}

impl Stack {
    pub fn new(steps: usize, nv: usize, nh: usize, seed: u64) -> Stack {
        let mut rng = Rng::new(seed);
        Stack { steps, nv, nh, trained: false, machines: (0..steps).map(|_| Machine::random(nv, nh, &mut rng)).collect() }
    }

    /// Train every machine, each on its own thread (they share nothing but the rows).
    pub fn train(&mut self, rows: &[Vec<f64>], o: &TrainOpts) {
        let (steps, fresh) = (self.steps, !self.trained);
        std::thread::scope(|sc| {
            for (k, m) in self.machines.iter_mut().enumerate() {
                sc.spawn(move || train_machine(m, rows, k + 1, steps, fresh, o));
            }
        });
        self.trained = true;
    }

    /// One sample: coins at level T, then machine T down to machine 1. Returns every level, x_T first.
    pub fn generate_chain(&self, sweeps: usize, rng: &mut Rng) -> Vec<Vec<f64>> {
        let mut x: Vec<f64> = (0..self.nv).map(|_| if rng.unit() < 0.5 { -1.0 } else { 1.0 }).collect();
        let mut levels = vec![x.clone()];
        for t in (1..=self.steps).rev() {
            let mut v = x.clone();
            self.machines[t - 1].settle(&mut v, &x, gamma(t, self.steps), sweeps, rng);
            x = v;
            levels.push(x.clone());
        }
        levels
    }

    fn to_note(&self) -> Vec<f64> {
        let mut out = vec![self.steps as f64, self.nv as f64, self.nh as f64, if self.trained { 1.0 } else { 0.0 }];
        for m in &self.machines {
            out.extend(&m.a);
            out.extend(&m.b);
            out.extend(&m.w);
            out.extend(&m.c);
        }
        out
    }

    fn from_note(v: &[f64]) -> Stack {
        let (steps, nv, nh) = (v[0] as usize, v[1] as usize, v[2] as usize);
        let mut at = 4;
        let mut take = |n: usize| {
            let s = v[at..at + n].to_vec();
            at += n;
            s
        };
        let machines = (0..steps).map(|_| Machine { a: take(nv), b: take(nh), w: take(nv * nh), c: take(nv * nh) }).collect();
        Stack { steps, nv, nh, trained: v[3] > 0.5, machines }
    }
}

/// Settings kept for a declared denoiser before its size is known.
fn cfg_key(name: &str) -> String {
    format!("denoise:{}", name)
}
fn stack_key(name: &str) -> String {
    format!("denoise:{}:stack", name)
}

/// Hamming distance from each sample to its nearest reference row; the count of exact copies.
pub fn nearest(samples: &[Vec<f64>], refs: &[Vec<f64>]) -> (Vec<usize>, usize) {
    let d: Vec<usize> = samples
        .iter()
        .map(|s| refs.iter().map(|r| s.iter().zip(r).filter(|(a, b)| (**a > 0.0) != (**b > 0.0)).count()).min().unwrap_or(s.len()))
        .collect();
    let copies = d.iter().filter(|&&x| x == 0).count();
    (d, copies)
}

fn median(v: &[usize]) -> f64 {
    let mut s = v.to_vec();
    s.sort_unstable();
    if s.is_empty() {
        return 0.0;
    }
    let n = s.len();
    if n % 2 == 1 {
        s[n / 2] as f64
    } else {
        0.5 * (s[n / 2 - 1] + s[n / 2]) as f64
    }
}

/// A contact sheet: each sample a `width`-wide tile of black ink on white, `scale` cells per pixel, grey gaps.
pub fn sheet(rows: &[Vec<Vec<f64>>], width: usize, scale: usize) -> Pgm {
    let hgt = rows.first().and_then(|r| r.first()).map(|s| s.len().div_ceil(width)).unwrap_or(1);
    let cols = rows.iter().map(|r| r.len()).max().unwrap_or(1);
    let (tw, th) = (width * scale + 1, hgt * scale + 1);
    let (w, h) = (cols * tw + 1, rows.len() * th + 1);
    let mut px = vec![0.6; w * h];
    for (ri, r) in rows.iter().enumerate() {
        for (ci, s) in r.iter().enumerate() {
            for (p, &val) in s.iter().enumerate() {
                let (py, pxx) = (p / width, p % width);
                for dy in 0..scale {
                    for dx in 0..scale {
                        let y = ri * th + 1 + py * scale + dy;
                        let x = ci * tw + 1 + pxx * scale + dx;
                        px[y * w + x] = if val > 0.0 { 0.0 } else { 1.0 };
                    }
                }
            }
        }
    }
    Pgm { w, h, px }
}

fn cells_to_rows(samples: &[Vec<f64>], cols: usize) -> Vec<Vec<Vec<f64>>> {
    samples.chunks(cols.max(1)).map(|c| c.to_vec()).collect()
}

/// Write the rows file (examples format) and/or the contact sheet, and say how close the samples come to the data.
#[allow(clippy::too_many_arguments)]
fn report(
    what: String,
    samples: &[Vec<f64>],
    names: &[String],
    data: Option<(&str, &Examples)>,
    kv: &[(String, Tok)],
    width: usize,
    ln: usize,
    ctx: &mut Ctx,
) -> Result<(), SettleError> {
    let rate = samples.iter().flatten().filter(|x| **x > 0.0).count() as f64 / samples.iter().map(|s| s.len()).sum::<usize>().max(1) as f64;
    let mut line = format!("{}: {} samples, yes-rate {:.3}", what, samples.len(), rate);
    if let Some((set, ex)) = data {
        let drate = ex.rows.iter().flatten().filter(|x| **x > 0.0).count() as f64 / ex.rows.iter().map(|s| s.len()).sum::<usize>().max(1) as f64;
        let (d, copies) = nearest(samples, &ex.rows);
        line.push_str(&format!(
            " (:{} {:.3}); nearest :{} row: median {} of {} pixels differ, {} exact copies",
            set,
            drate,
            set,
            median(&d),
            names.len(),
            copies
        ));
    }
    if let Some(p) = kw(kv, "rows") {
        let path = ctx.path(&text(p, ln)?);
        let mut s = names.join(" ");
        s.push('\n');
        for r in samples {
            s.extend(r.iter().map(|&x| if x > 0.0 { '1' } else { '0' }));
            s.push('\n');
        }
        std::fs::write(&path, s).or_else(|e| err(ln, format!("cannot write {}: {}", path.display(), e)))?;
        line.push_str(&format!("; wrote {}", path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default()));
    }
    if let Some(p) = kw(kv, "out") {
        let path = ctx.path(&text(p, ln)?);
        let cols = match kw(kv, "cols") {
            Some(v) => num(v, ln)? as usize,
            None => (samples.len() as f64).sqrt().ceil() as usize,
        };
        let scale = kw(kv, "scale").map(|v| num(v, ln)).transpose()?.unwrap_or(4.0) as usize;
        write_pgm(&path, &sheet(&cells_to_rows(samples, cols), width, scale.max(1))).or_else(|e| err(ln, e))?;
        line.push_str(&format!("; wrote {}", path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default()));
    }
    ctx.say(line);
    Ok(())
}

fn whole(v: f64, lo: f64, hi: f64, what: &str, ln: usize) -> Result<usize, SettleError> {
    if v < lo || v > hi || v.fract() != 0.0 {
        return err(ln, format!("{} takes a whole number from {} to {}", what, lo, hi));
    }
    Ok(v as usize)
}

fn width_for(nv: usize, kv: &[(String, Tok)], cfg_width: f64, ln: usize) -> Result<usize, SettleError> {
    let w = match kw(kv, "width") {
        Some(v) => num(v, ln)?,
        None if cfg_width > 0.0 => cfg_width,
        None => {
            let r = (nv as f64).sqrt().round();
            if (r * r) as usize == nv {
                r
            } else {
                nv as f64
            }
        }
    };
    whole(w, 1.0, nv as f64, "width:", ln)
}

/// `denoiser :d, steps: 8, hidden: 64, over: :train, width: 8, seed: 7`
fn declare(m: &mut Model, name: &str, rest: &[Tok], ln: usize, ctx: &mut Ctx) -> Result<(), SettleError> {
    if m.notes.contains_key(&cfg_key(name)) {
        return err(ln, format!("denoiser :{} is already declared", name));
    }
    if m.idx.contains_key(name) {
        return err(ln, format!(":{} is already a thing in this model; name the denoiser something else", name));
    }
    let rest = if rest.first() == Some(&Tok::Comma) { &rest[1..] } else { rest };
    let kv = kwargs(rest, ln)?;
    only(&kv, &["steps", "hidden", "over", "width", "seed"], "denoiser", ln)?;
    let steps = whole(kw(&kv, "steps").map(|v| num(v, ln)).transpose()?.unwrap_or(4.0), 1.0, 64.0, "steps:", ln)?;
    let nh = whole(kw(&kv, "hidden").map(|v| num(v, ln)).transpose()?.unwrap_or(32.0), 1.0, 4096.0, "hidden:", ln)?;
    let seed = kw(&kv, "seed").map(|v| num(v, ln)).transpose()?.unwrap_or(7.0);
    let width = kw(&kv, "width").map(|v| num(v, ln)).transpose()?.unwrap_or(0.0);
    let mut names = Vec::new();
    match kw(&kv, "over") {
        Some(Tok::Sym(set)) => {
            let ex = load_examples(m, set, ln)?;
            names = ex.names.clone();
            let st = Stack::new(steps, ex.names.len(), nh, seed as u64);
            m.notes.insert(stack_key(name), (st.to_note(), Vec::new()));
        }
        Some(_) => return err(ln, "over: takes an examples set, like over: :train"),
        None => {}
    }
    m.notes.insert(cfg_key(name), (vec![steps as f64, nh as f64, seed, width], names.clone()));
    ctx.say(format!(
        "denoiser :{}: {} steps, {} hidden things per step{}",
        name,
        steps,
        nh,
        if names.is_empty() { String::new() } else { format!(", over {} pixels", names.len()) }
    ));
    Ok(())
}

fn cfg(m: &Model, name: &str) -> Option<(Vec<f64>, Vec<String>)> {
    m.notes.get(&cfg_key(name)).cloned()
}

/// `d.train :train, rounds: 200, rate: 0.05, sweeps: 1, batch: 50, decay: 0, seed: 1`
fn train_stmt(m: &mut Model, name: &str, set: &str, rest: &[Tok], ln: usize, ctx: &mut Ctx) -> Result<(), SettleError> {
    let (c, mut names) = cfg(m, name).unwrap();
    let ex = load_examples(m, set, ln)?;
    if ex.rows.is_empty() {
        return err(ln, format!("examples :{} have no rows", set));
    }
    if !names.is_empty() && names != ex.names {
        return err(ln, format!("denoiser :{} covers other things than examples :{}", name, set));
    }
    let rest = if rest.first() == Some(&Tok::Comma) { &rest[1..] } else { rest };
    let kv = kwargs(rest, ln)?;
    only(&kv, &["rounds", "rate", "sweeps", "batch", "decay", "seed", "leans"], "train", ln)?;
    let get = |k: &str, d: f64| kw(&kv, k).map(|v| num(v, ln)).transpose().map(|x| x.unwrap_or(d));
    let o = TrainOpts {
        rounds: whole(get("rounds", 100.0)?, 1.0, 1e6, "rounds:", ln)?,
        rate: get("rate", 0.05)?,
        sweeps: whole(get("sweeps", 1.0)?, 1.0, 1e4, "sweeps:", ln)?,
        batch: whole(get("batch", 50.0)?, 1.0, 1e6, "batch:", ln)?,
        decay: get("decay", 0.0)?,
        seed: get("seed", 1.0)? as u64,
        data_leans: match kw(&kv, "leans") {
            None => true,
            Some(Tok::Sym(x)) if x == "data" => true,
            Some(Tok::Sym(x)) if x == "zero" => false,
            Some(_) => return err(ln, "leans: is :data (start at the data's rates) or :zero"),
        },
    };
    if o.rate <= 0.0 || o.decay < 0.0 {
        return err(ln, "rate must be above zero and decay not negative");
    }
    let mut st = match m.notes.get(&stack_key(name)) {
        Some((v, _)) => Stack::from_note(v),
        None => Stack::new(c[0] as usize, ex.names.len(), c[1] as usize, c[2] as u64),
    };
    let t0 = Instant::now();
    st.train(&ex.rows, &o);
    let secs = t0.elapsed().as_secs_f64();
    names = ex.names.clone();
    m.notes.insert(cfg_key(name), (c, names));
    m.notes.insert(stack_key(name), (st.to_note(), vec![set.to_string()]));
    ctx.say(format!(
        "trained :{} on :{} ({} rows, {} pixels): {} machines of {} hidden, {} rounds, noise per step {}, {:.2}s",
        name,
        set,
        ex.rows.len(),
        ex.names.len(),
        st.steps,
        st.nh,
        o.rounds,
        (1..=st.steps).map(|t| format!("{:.3}", flip_chance(t, st.steps))).collect::<Vec<_>>().join("/"),
        secs
    ));
    Ok(())
}

/// `d.generate 16, out: "s.pgm", rows: "s.txt", chain: "c.pgm", sweeps: 100, seed: 2, cols: 4, scale: 4`
fn generate_stmt(m: &Model, name: &str, count: f64, rest: &[Tok], ln: usize, ctx: &mut Ctx) -> Result<(), SettleError> {
    let (c, names) = cfg(m, name).unwrap();
    let (stv, data) = match m.notes.get(&stack_key(name)) {
        Some(x) => x.clone(),
        None => return err(ln, format!("denoiser :{} does not know its pixels yet: train it, or declare it with over: :examples", name)),
    };
    let st = Stack::from_note(&stv);
    let count = whole(count, 1.0, 1e6, "generate", ln)?;
    let rest = if rest.first() == Some(&Tok::Comma) { &rest[1..] } else { rest };
    let kv = kwargs(rest, ln)?;
    only(&kv, &["out", "rows", "chain", "sweeps", "seed", "cols", "scale", "width"], "generate", ln)?;
    let sweeps = whole(kw(&kv, "sweeps").map(|v| num(v, ln)).transpose()?.unwrap_or(100.0), 1.0, 1e6, "sweeps:", ln)?;
    let mut rng = Rng::new(kw(&kv, "seed").map(|v| num(v, ln)).transpose()?.unwrap_or(2.0) as u64);
    let width = width_for(st.nv, &kv, c[3], ln)?;
    let chains: Vec<Vec<Vec<f64>>> = (0..count).map(|_| st.generate_chain(sweeps, &mut rng)).collect();
    let samples: Vec<Vec<f64>> = chains.iter().map(|ch| ch.last().unwrap().clone()).collect();
    if let Some(p) = kw(&kv, "chain") {
        let path = ctx.path(&text(p, ln)?);
        let scale = kw(&kv, "scale").map(|v| num(v, ln)).transpose()?.unwrap_or(4.0) as usize;
        write_pgm(&path, &sheet(&chains[..chains.len().min(8)], width, scale.max(1))).or_else(|e| err(ln, e))?;
    }
    let ex = match data.first() {
        Some(set) => Some((set.as_str(), load_examples(m, set, ln)?)),
        None => None,
    };
    let what = format!(
        "generated from :{} ({}, {} steps x {} sweeps)",
        name,
        if st.trained { "trained" } else { "untrained: random pulls" },
        st.steps,
        sweeps
    );
    report(what, &samples, &names, ex.as_ref().map(|(s, e)| (*s, e)), &kv, width, ln, ctx)
}

/// `sample :train, 16, sweeps: 800, seed: 3, out:, rows:` settles the model's own things from a random start.
fn sample_stmt(m: &Model, stt: &mut State, set: &str, count: f64, rest: &[Tok], ln: usize, ctx: &mut Ctx) -> Result<(), SettleError> {
    let ex = load_examples(m, set, ln)?;
    let count = whole(count, 1.0, 1e6, "sample", ln)?;
    let rest = if rest.first() == Some(&Tok::Comma) { &rest[1..] } else { rest };
    let kv = kwargs(rest, ln)?;
    only(&kv, &["out", "rows", "sweeps", "seed", "cols", "scale", "width"], "sample", ln)?;
    let sweeps = whole(kw(&kv, "sweeps").map(|v| num(v, ln)).transpose()?.unwrap_or(800.0), 1.0, 1e7, "sweeps:", ln)?;
    if let Some(v) = kw(&kv, "seed") {
        stt.rng = Rng::new(num(v, ln)? as u64);
    }
    let idx: Vec<usize> = ex.names.iter().map(|n| m.need(n, ln)).collect::<Result<_, _>>()?;
    let beta = 1.0 / stt.temp;
    let mut samples = Vec::with_capacity(count);
    for _ in 0..count {
        let (mut s, mut free) = stt.start(m);
        for _ in 0..sweeps {
            stt.sweep(m, &mut s, &mut free, beta);
        }
        samples.push(idx.iter().map(|&i| s[i]).collect::<Vec<f64>>());
    }
    let width = width_for(idx.len(), &kv, 0.0, ln)?;
    let what = format!("sampled the model directly over :{} ({} sweeps from a random start, {} things settling)", set, sweeps, m.len());
    report(what, &samples, &ex.names, Some((set, &ex)), &kv, width, ln, ctx)
}

/// `coins :train, 16, seed: 4, out:, rows:` draws every pixel alone at its rate in the examples.
fn coins_stmt(m: &Model, set: &str, count: f64, rest: &[Tok], ln: usize, ctx: &mut Ctx) -> Result<(), SettleError> {
    let ex = load_examples(m, set, ln)?;
    if ex.rows.is_empty() {
        return err(ln, format!("examples :{} have no rows", set));
    }
    let count = whole(count, 1.0, 1e6, "coins", ln)?;
    let rest = if rest.first() == Some(&Tok::Comma) { &rest[1..] } else { rest };
    let kv = kwargs(rest, ln)?;
    only(&kv, &["out", "rows", "seed", "cols", "scale", "width"], "coins", ln)?;
    let mut rng = Rng::new(kw(&kv, "seed").map(|v| num(v, ln)).transpose()?.unwrap_or(4.0) as u64);
    let nv = ex.names.len();
    let p: Vec<f64> = (0..nv).map(|i| ex.rows.iter().filter(|r| r[i] > 0.0).count() as f64 / ex.rows.len() as f64).collect();
    let samples: Vec<Vec<f64>> = (0..count).map(|_| p.iter().map(|&q| if rng.unit() < q { 1.0 } else { -1.0 }).collect()).collect();
    let width = width_for(nv, &kv, 0.0, ln)?;
    report(format!("coins at the pixel rates of :{}", set), &samples, &ex.names, Some((set, &ex)), &kv, width, ln, ctx)
}

impl Ext for Denoise {
    fn name(&self) -> &'static str {
        "denoise"
    }

    fn statements(&self) -> &'static [&'static str] {
        &[
            "model: denoiser :d, steps: 8, hidden: 64, over: :train, width: 8, seed: 7",
            "run: d.train :train, rounds: 200, rate: 0.05, sweeps: 1, batch: 50, decay: 0, seed: 1, leans: :data",
            "run: d.generate 16, out: \"samples.pgm\", rows: \"samples.txt\", chain: \"chain.pgm\", sweeps: 100, seed: 2",
            "run: sample :train, 16, sweeps: 800, seed: 3, out: \"direct.pgm\", rows: \"direct.txt\"",
            "run: coins :train, 16, seed: 4, out: \"coins.pgm\", rows: \"coins.txt\"",
        ]
    }

    fn model_stmt(&self, m: &mut Model, t: &[Tok], ln: usize, ctx: &mut Ctx) -> Claim {
        match t {
            [Tok::Ident(k), Tok::Sym(name), rest @ ..] if k == "denoiser" => Some(declare(m, name, rest, ln, ctx)),
            _ => None,
        }
    }

    fn run_stmt(&self, m: &mut Model, st: &mut State, t: &[Tok], ln: usize, ctx: &mut Ctx) -> Claim {
        match t {
            [Tok::Ident(d), Tok::Dot, Tok::Ident(v), Tok::Sym(set), rest @ ..] if v == "train" && m.notes.contains_key(&cfg_key(d)) => {
                Some(train_stmt(m, d, set, rest, ln, ctx))
            }
            [Tok::Ident(d), Tok::Dot, Tok::Ident(v), Tok::Num(n), rest @ ..] if v == "generate" && m.notes.contains_key(&cfg_key(d)) => {
                Some(generate_stmt(m, d, *n, rest, ln, ctx))
            }
            [Tok::Ident(k), Tok::Sym(set), Tok::Comma, Tok::Num(n), rest @ ..] if k == "sample" => Some(sample_stmt(m, st, set, *n, rest, ln, ctx)),
            [Tok::Ident(k), Tok::Sym(set), Tok::Comma, Tok::Num(n), rest @ ..] if k == "coins" => Some(coins_stmt(m, set, *n, rest, ln, ctx)),
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

    /// Three clean 3x3 glyphs: plus, cross, ring.
    const GLYPHS: [&str; 3] = ["010111010", "101010101", "111101111"];

    fn glyph_rows(copies: usize) -> String {
        (0..copies).flat_map(|_| GLYPHS.iter()).cloned().collect::<Vec<_>>().join(" ")
    }

    fn glyph_program(extra_run: &str, steps: usize) -> String {
        format!(
            "model :g do\n  examples :train, over: \"p0 p1 p2 p3 p4 p5 p6 p7 p8\", rows: \"{}\"\n  denoiser :d, steps: {}, hidden: 8, over: :train, width: 3\nend\nrun :g do\n{}\nend",
            glyph_rows(10),
            steps,
            extra_run
        )
    }

    #[test]
    fn the_schedule_runs_from_data_to_pure_coins() {
        for steps in [1, 2, 4, 8] {
            assert_eq!(rho(0, steps), 1.0);
            assert_eq!(rho(steps, steps), 0.0);
            assert_eq!(gamma(steps, steps), 0.0);
            assert!((flip_chance(steps, steps) - 0.5).abs() < 1e-12);
            for t in 1..steps {
                assert!(gamma(t, steps) > 0.0 && flip_chance(t, steps) < 0.5);
            }
        }
        // composing the forward flips gives agreement (1 + rho_t) / 2 with the clean bits
        let mut rng = Rng::new(3);
        let x0 = vec![1.0; 200_000];
        let mut x = x0.clone();
        for t in 1..=4 {
            x = flip(&x, flip_chance(t, 4), &mut rng);
            let agree = x.iter().filter(|v| **v > 0.0).count() as f64 / x.len() as f64;
            assert!((agree - 0.5 * (1.0 + rho(t, 4))).abs() < 0.005, "t {} agree {}", t, agree);
        }
    }

    #[test]
    fn the_block_sampler_matches_the_exact_conditional() {
        let mut rng = Rng::new(11);
        let mut m = Machine::random(3, 2, &mut rng);
        for p in m.w.iter_mut().chain(m.c.iter_mut()) {
            *p *= 8.0;
        }
        m.a = vec![0.3, -0.2, 0.1];
        m.b = vec![-0.4, 0.5];
        let x = [1.0, -1.0, 1.0];
        let g = 0.7;
        let exact = m.exact_conditional(&x, g);
        let count = |mm: &Machine, gg: f64| {
            let mut r = Rng::new(5);
            let mut v = vec![1.0, 1.0, 1.0];
            let mut hist = [0.0f64; 8];
            let n = 200_000;
            for _ in 0..n {
                mm.settle(&mut v, &x, gg, 1, &mut r);
                let bits = (0..3).fold(0, |b, i| b | if v[i] > 0.0 { 1 << i } else { 0 });
                hist[bits] += 1.0 / n as f64;
            }
            hist
        };
        let got = count(&m, g);
        let worst = exact.iter().zip(&got).map(|(a, b)| (a - b).abs()).fold(0.0, f64::max);
        assert!(worst < 0.01, "exact {:?} sampled {:?}", exact, got);
        // negative control: the sampler with the forward pull reversed disagrees with the exact answer
        let wrong = count(&m, -g);
        assert!(exact.iter().zip(&wrong).any(|(a, b)| (a - b).abs() > 0.05));
    }

    fn exact_glyph_share(samples: &[Vec<f64>]) -> f64 {
        let pats: Vec<Vec<f64>> = GLYPHS.iter().map(|g| g.chars().map(|c| if c == '1' { 1.0 } else { -1.0 }).collect()).collect();
        let (_, copies) = nearest(samples, &pats);
        copies as f64 / samples.len() as f64
    }

    #[test]
    fn a_trained_stack_makes_glyphs_and_an_untrained_one_makes_noise() {
        let (_, it) = run(&glyph_program("  d.train :train, rounds: 150, rate: 0.1, seed: 1", 4));
        let m = &it.models["g"];
        let trained = Stack::from_note(&m.notes[&stack_key("d")].0);
        let untrained = Stack::new(4, 9, 8, 7);
        let mut rng = Rng::new(9);
        let draw = |s: &Stack, rng: &mut Rng| (0..400).map(|_| s.generate_chain(50, rng).pop().unwrap()).collect::<Vec<_>>();
        let good = exact_glyph_share(&draw(&trained, &mut rng));
        let bad = exact_glyph_share(&draw(&untrained, &mut rng));
        assert!(good > 0.6, "trained stack drew an exact glyph {:.2} of the time", good);
        assert!(bad < 0.05, "untrained stack drew an exact glyph {:.2} of the time", bad);
        // each glyph appears: the stack does not collapse onto one of them
        let samples = draw(&trained, &mut rng);
        for g in GLYPHS {
            let pat: Vec<f64> = g.chars().map(|c| if c == '1' { 1.0 } else { -1.0 }).collect();
            assert!(samples.iter().filter(|s| **s == pat).count() > 40, "glyph {} is rare", g);
        }
    }

    #[test]
    fn the_notes_round_trip_the_whole_stack() {
        let s = Stack::new(3, 5, 4, 2);
        assert_eq!(Stack::from_note(&s.to_note()), s);
    }

    #[test]
    fn nearest_counts_copies_and_distances() {
        let refs = vec![vec![1.0, 1.0, -1.0], vec![-1.0, -1.0, -1.0]];
        let (d, copies) = nearest(&[vec![1.0, 1.0, -1.0], vec![1.0, -1.0, 1.0]], &refs);
        assert_eq!(d, vec![0, 2]);
        assert_eq!(copies, 1);
    }

    #[test]
    fn the_statements_write_sheets_and_rows_that_read_back() {
        let dir = std::env::temp_dir().join(format!("settle_denoise_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = glyph_program(
            "  d.generate 4, out: \"u.pgm\", seed: 1\n  d.train :train, rounds: 20, seed: 1\n  d.generate 6, out: \"s.pgm\", rows: \"s.txt\", chain: \"c.pgm\", sweeps: 20, cols: 3, scale: 2\n  coins :train, 5, rows: \"k.txt\"\n  sample :train, 3, sweeps: 10, rows: \"x.txt\"",
            2,
        );
        let mut it = Interp::default();
        it.base_dir = dir.clone();
        let out = it.exec(&src).unwrap();
        assert!(out.iter().any(|l| l.contains("untrained: random pulls")), "{:?}", out);
        assert!(out.iter().any(|l| l.starts_with("trained :d on :train (30 rows, 9 pixels): 2 machines")), "{:?}", out);
        assert!(out.iter().any(|l| l.contains("exact copies")));
        let s = crate::grid::read_pgm(&dir.join("s.pgm")).unwrap();
        assert_eq!((s.w, s.h), (3 * 7 + 1, 2 * 7 + 1)); // 3 columns, 2 rows of 3x3 tiles at scale 2 with gaps
        let c = crate::grid::read_pgm(&dir.join("c.pgm")).unwrap();
        assert_eq!((c.w, c.h), (3 * 7 + 1, 6 * 7 + 1)); // levels x_2, x_1, x_0 across, one sample per row
        for (f, n) in [("s.txt", 6), ("k.txt", 5), ("x.txt", 3)] {
            let ex = crate::learn::parse_file(&std::fs::read_to_string(dir.join(f)).unwrap(), 1).unwrap();
            assert_eq!((ex.rows.len(), ex.names.len()), (n, 9), "{}", f);
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn coins_follow_the_pixel_rates() {
        let (out, _) = run(&glyph_program("  coins :train, 3000, seed: 5", 1));
        // the glyphs' mean yes-rate is 18 / 27 = 0.667
        let line = out.iter().find(|l| l.starts_with("coins")).unwrap();
        let rate: f64 = line.split("yes-rate ").nth(1).unwrap()[..5].parse().unwrap();
        assert!((rate - 18.0 / 27.0).abs() < 0.02, "{}", line);
    }

    #[test]
    fn errors_name_their_line() {
        let base = "model :g do\n  examples :train, over: \"a b\", rows: \"10 01\"\n";
        let cases = [
            (format!("{}  denoiser :d, steps: 0\nend", base), "line 3: steps: takes a whole number"),
            (format!("{}  denoiser :d, colour: 3\nend", base), "line 3: denoiser does not take `colour:`"),
            (format!("{}  denoiser :d\n  denoiser :d\nend", base), "line 4: denoiser :d is already declared"),
            (format!("{}  denoiser :d\nend\nrun :g do\n  d.generate 3\nend", base), "line 6: denoiser :d does not know its pixels"),
            (format!("{}  denoiser :d\nend\nrun :g do\n  d.train :nope\nend", base), "line 6: no examples :nope"),
            (format!("{}  denoiser :d, over: :train\nend\nrun :g do\n  d.train :train, leans: :maybe\nend", base), "line 6: leans: is :data"),
            (format!("{}  denoiser :d, over: :train\n  examples :other, over: \"x y\", rows: \"10\"\nend\nrun :g do\n  d.train :other\nend", base), "line 7: denoiser :d covers other things"),
        ];
        for (src, want) in cases {
            let e = Interp::default().exec(&src).err().map(|e| e.0).unwrap_or_default();
            assert!(e.starts_with(want), "{:?} gave {:?}", src, e);
        }
    }

    #[test]
    fn a_single_step_stack_learns_what_a_direct_machine_learns() {
        // T = 1: the one machine sees pure noise (gamma 0), so it must make glyphs from nothing, like a single machine
        let (_, it) = run(&glyph_program("  d.train :train, rounds: 300, rate: 0.1, seed: 2", 1));
        let st = Stack::from_note(&it.models["g"].notes[&stack_key("d")].0);
        assert_eq!(gamma(1, 1), 0.0);
        let mut rng = Rng::new(4);
        let samples: Vec<Vec<f64>> = (0..400).map(|_| st.generate_chain(200, &mut rng).pop().unwrap()).collect();
        assert!(exact_glyph_share(&samples) > 0.5);
    }
}
