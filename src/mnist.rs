//! MNIST: the learn family at full size. Load the 60,000 + 10,000 handwritten digits, fit restricted settling
//! machines by contrastive divergence, and classify by settling the ten label things with the pixels held.
//!
//! <claudes_code_comments>
//! ** Function List **
//! parse_idx_images(bytes) / parse_idx_labels(bytes) - read the IDX files LeCun, Cortes and Burges published
//! load(dir, split)              - the train or test split from a folder of unzipped IDX files
//! spins(img)                    - a picture's pixels as things: +1 when the grey value is >= THRESHOLD, else -1
//! Digits::rows(with_labels)     - every picture as a flat row of +1/-1 (pixels, then ten one-hot label things)
//! Rbm::new / init_leans         - a restricted machine: visible leans a, hidden leans b, pulls W (hidden x visible)
//! Rbm::hidden_inputs / visible_inputs / neg_free - the two conditionals and -F(v) with the hidden things summed out
//! train(rbm, rows, opts, log)   - contrastive divergence (CD-k) or persistent chains, momentum and weight decay,
//!                                 minibatches split over threads with a per-example random stream
//! Coin, Xs128                   - a random source; Xs128 is the browser engine's xorshift128, for the port check
//! exact_label(rbm, px)          - the one-hot label pattern with the lowest free energy (no sampling)
//! settle_label(rbm, px, S, c)   - hold the pixels, settle hidden and label things S sweeps, pick the label most often yes
//! hidden_rates(rbm, px, out)    - the hidden things' yes-rates given a picture, for the linear readout
//! Softmax::fit / predict        - multinomial logistic regression by Adam (the readout and the baseline)
//! nearest_centroid(...)         - the plain baseline: the class whose mean picture is closest
//! save / load_rbm               - the weights file the browser reads (little-endian f32)
//!
//! ** Technical Review **
//! - Things are +1/-1 as everywhere in SETTLE, temperature 1. A restricted machine's energy is
//!   E(v, h) = -a.v - b.h - h.W.v; given the visible things each hidden thing is yes with (1 + tanh x_k)/2,
//!   x_k = b_k + W_k.v, and the reverse for visible things. Updating all hidden things, then all visible things,
//!   is exact Gibbs sampling, the same p-bit rule as `sweep` (s = +1 when tanh(x) > u, u uniform in [-1, 1)).
//! - The gradient is learn.rs's: data averages of v_i tanh(x_k) minus the machine's, where the machine's are
//!   taken after k sweeps started at the example (CD-k) or from standing chains (persistent). The generic
//!   learn.rs Net stores an n x n matrix and costs n^2 per sweep; 794 + 500 things need the restricted layout
//!   here, and a test checks that both give the same free energy and conditionals on one machine.
//! - Determinism: every example draws from Rng::new(mix(seed, epoch, index)), so a run is the same for any
//!   thread count. Gradients sum per thread, then add on the main thread.
//! - Classification with a joint machine (784 pixels + 10 label things): the settled readout starts the labels at
//!   random, burns in S/10 sweeps and counts yes over S sweeps (learn.rs `classify`); ties go to the lower digit.
//!   The exact readout is argmax over c of -F(pixels, e_c). The pixel part of every hidden input is computed once.
//! - Numbers: training is f32; the classify path converts to f64 so the browser port (JS numbers are f64) can
//!   repeat it operation for operation. THRESHOLD = 128 (grey values 0 to 255).
//! </claudes_code_comments>

use crate::rng::Rng;
use std::fs;
use std::io::Write;

pub const SIDE: usize = 28;
pub const PIX: usize = SIDE * SIDE;
pub const CLASSES: usize = 10;
/// A pixel is a yes thing when its grey value (0 to 255) is at least this.
pub const THRESHOLD: u8 = 128;

fn be_u32(b: &[u8], at: usize) -> Result<u32, String> {
    if b.len() < at + 4 {
        return Err("IDX file is shorter than its header".into());
    }
    Ok(u32::from_be_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]]))
}

/// (count, rows, cols, pixels) from an idx3-ubyte file (magic 2051).
pub fn parse_idx_images(b: &[u8]) -> Result<(usize, usize, usize, Vec<u8>), String> {
    if be_u32(b, 0)? != 2051 {
        return Err("not an IDX image file (magic is not 2051)".into());
    }
    let (n, r, c) = (be_u32(b, 4)? as usize, be_u32(b, 8)? as usize, be_u32(b, 12)? as usize);
    if b.len() != 16 + n * r * c {
        return Err(format!("IDX image file holds {} bytes, header promises {}", b.len(), 16 + n * r * c));
    }
    Ok((n, r, c, b[16..].to_vec()))
}

/// Labels from an idx1-ubyte file (magic 2049).
pub fn parse_idx_labels(b: &[u8]) -> Result<Vec<u8>, String> {
    if be_u32(b, 0)? != 2049 {
        return Err("not an IDX label file (magic is not 2049)".into());
    }
    let n = be_u32(b, 4)? as usize;
    if b.len() != 8 + n {
        return Err(format!("IDX label file holds {} bytes, header promises {}", b.len(), 8 + n));
    }
    Ok(b[8..].to_vec())
}

pub struct Digits {
    pub n: usize,
    pub images: Vec<u8>,
    pub labels: Vec<u8>,
}

/// `split` is "train" (60,000) or "test" (10,000); `dir` holds the four unzipped files.
pub fn load(dir: &str, split: &str) -> Result<Digits, String> {
    let stem = match split {
        "train" => "train",
        "test" => "t10k",
        _ => return Err(format!("split is train or test, not {}", split)),
    };
    let read = |f: String| fs::read(&f).map_err(|e| format!("{}: {}", f, e));
    let (n, r, c, images) = parse_idx_images(&read(format!("{}/{}-images-idx3-ubyte", dir, stem))?)?;
    let labels = parse_idx_labels(&read(format!("{}/{}-labels-idx1-ubyte", dir, stem))?)?;
    if r != SIDE || c != SIDE || labels.len() != n {
        return Err(format!("{} split: {} pictures of {}x{}, {} labels", split, n, r, c, labels.len()));
    }
    Ok(Digits { n, images, labels })
}

pub fn spins(img: &[u8]) -> Vec<f32> {
    img.iter().map(|&p| if p >= THRESHOLD { 1.0 } else { -1.0 }).collect()
}

impl Digits {
    pub fn image(&self, i: usize) -> &[u8] {
        &self.images[i * PIX..(i + 1) * PIX]
    }
    /// Flat rows of +1/-1: 784 pixels, then (with labels) ten label things, +1 at the true digit.
    pub fn rows(&self, with_labels: bool, labels: &[u8]) -> Vec<f32> {
        let nv = PIX + if with_labels { CLASSES } else { 0 };
        let mut out = Vec::with_capacity(self.n * nv);
        for i in 0..self.n {
            out.extend(spins(self.image(i)));
            if with_labels {
                out.extend((0..CLASSES).map(|c| if c == labels[i] as usize { 1.0 } else { -1.0 }));
            }
        }
        out
    }
}

/// ln(2 cosh y), without overflow.
pub fn ln2cosh(y: f64) -> f64 {
    y.abs() + (-2.0 * y.abs()).exp().ln_1p()
}

#[derive(Clone, Debug)]
pub struct Rbm {
    pub nv: usize,
    pub nh: usize,
    pub a: Vec<f32>,
    pub b: Vec<f32>,
    /// nh x nv: row k is hidden thing k's pulls onto every visible thing.
    pub w: Vec<f32>,
}

#[inline]
fn dot(x: &[f32], y: &[f32]) -> f32 {
    // eight lanes so the compiler vectorises the sum
    let mut acc = [0f32; 8];
    let n8 = x.len() / 8 * 8;
    for (cx, cy) in x[..n8].chunks_exact(8).zip(y[..n8].chunks_exact(8)) {
        for j in 0..8 {
            acc[j] += cx[j] * cy[j];
        }
    }
    let mut s = acc.iter().sum::<f32>();
    for j in n8..x.len() {
        s += x[j] * y[j];
    }
    s
}

#[inline]
fn axpy(alpha: f32, x: &[f32], y: &mut [f32]) {
    for (yy, xx) in y.iter_mut().zip(x) {
        *yy += alpha * xx;
    }
}

impl Rbm {
    /// Pulls start at `scale` times a normal draw, leans at zero.
    pub fn new(nv: usize, nh: usize, scale: f64, rng: &mut Rng) -> Rbm {
        Rbm { nv, nh, a: vec![0.0; nv], b: vec![0.0; nh], w: (0..nv * nh).map(|_| (scale * rng.normal()) as f32).collect() }
    }

    /// Visible leans at the data's own rates: a_i = atanh(mean_i), the mean clipped to [-0.99, 0.99].
    pub fn init_leans(&mut self, rows: &[f32]) {
        let n = rows.len() / self.nv;
        let mut m = vec![0f64; self.nv];
        for r in rows.chunks_exact(self.nv) {
            for (mm, x) in m.iter_mut().zip(r) {
                *mm += *x as f64;
            }
        }
        for (a, mm) in self.a.iter_mut().zip(&m) {
            *a = (mm / n as f64).clamp(-0.99, 0.99).atanh() as f32;
        }
    }

    pub fn hidden_inputs(&self, v: &[f32], out: &mut [f32]) {
        for k in 0..self.nh {
            out[k] = self.b[k] + dot(&self.w[k * self.nv..(k + 1) * self.nv], v);
        }
    }

    pub fn visible_inputs(&self, h: &[f32], out: &mut [f32]) {
        out.copy_from_slice(&self.a);
        for k in 0..self.nh {
            axpy(h[k], &self.w[k * self.nv..(k + 1) * self.nv], out);
        }
    }

    /// -F(v): the log of v's unnormalised chance with the hidden things summed out.
    pub fn neg_free(&self, v: &[f32]) -> f64 {
        let mut x = vec![0f32; self.nh];
        self.hidden_inputs(v, &mut x);
        self.a.iter().zip(v).map(|(a, v)| (*a * *v) as f64).sum::<f64>() + x.iter().map(|&y| ln2cosh(y as f64)).sum::<f64>()
    }

    pub fn save(&self, path: &str) -> std::io::Result<()> {
        let mut f = fs::File::create(path)?;
        f.write_all(b"SETTLERBM1")?;
        f.write_all(&(self.nv as u32).to_le_bytes())?;
        f.write_all(&(self.nh as u32).to_le_bytes())?;
        for x in self.a.iter().chain(&self.b).chain(&self.w) {
            f.write_all(&x.to_le_bytes())?;
        }
        Ok(())
    }
}

pub fn load_rbm(path: &str) -> Result<Rbm, String> {
    let b = fs::read(path).map_err(|e| format!("{}: {}", path, e))?;
    if b.len() < 18 || &b[..10] != b"SETTLERBM1" {
        return Err(format!("{} is not a SETTLERBM1 weights file", path));
    }
    let nv = u32::from_le_bytes([b[10], b[11], b[12], b[13]]) as usize;
    let nh = u32::from_le_bytes([b[14], b[15], b[16], b[17]]) as usize;
    let want = 18 + 4 * (nv + nh + nv * nh);
    if b.len() != want {
        return Err(format!("{} holds {} bytes, expected {}", path, b.len(), want));
    }
    let f: Vec<f32> = b[18..].chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
    Ok(Rbm { nv, nh, a: f[..nv].to_vec(), b: f[nv..nv + nh].to_vec(), w: f[nv + nh..].to_vec() })
}

#[derive(Clone, Debug)]
pub struct TrainOpts {
    pub epochs: usize,
    pub rate: f64,
    pub batch: usize,
    pub momentum: f64,
    /// momentum starts at 0.5 and switches to `momentum` at this epoch (Hinton's practical guide)
    pub momentum_from: usize,
    pub decay: f64,
    pub cd_k: usize,
    pub persistent: bool,
    pub seed: u64,
    pub threads: usize,
}

#[derive(Clone, Debug)]
pub struct EpochLog {
    pub epoch: usize,
    /// fraction of visible bits that differ between an example and its one-step sampled reconstruction
    pub recon: f64,
    pub secs: f64,
}

pub fn mix(seed: u64, a: u64, b: u64) -> u64 {
    let mut z = seed ^ a.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ b.wrapping_mul(0xD1B5_4A32_D192_ED03);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

#[inline]
fn coin_f32(x: f32, rng: &mut Rng) -> f32 {
    if (x as f64).tanh() > rng.signed() {
        1.0
    } else {
        -1.0
    }
}

struct Buf {
    gw: Vec<f32>,
    ga: Vec<f32>,
    gb: Vec<f32>,
    x: Vec<f32>,
    hp: Vec<f32>,
    h: Vec<f32>,
    y: Vec<f32>,
    v: Vec<f32>,
    wrong: f64,
}

impl Buf {
    fn new(nv: usize, nh: usize) -> Buf {
        Buf {
            gw: vec![0.0; nv * nh],
            ga: vec![0.0; nv],
            gb: vec![0.0; nh],
            x: vec![0.0; nh],
            hp: vec![0.0; nh],
            h: vec![0.0; nh],
            y: vec![0.0; nv],
            v: vec![0.0; nv],
            wrong: 0.0,
        }
    }
    fn clear(&mut self) {
        self.gw.iter_mut().for_each(|g| *g = 0.0);
        self.ga.iter_mut().for_each(|g| *g = 0.0);
        self.gb.iter_mut().for_each(|g| *g = 0.0);
        self.wrong = 0.0;
    }
}

/// One example's contribution: positive statistics from the example, negative from k sweeps started at it
/// (or from a standing chain when `chain` is given).
fn example_grad(rbm: &Rbm, v0: &[f32], k: usize, chain: Option<&mut [f32]>, rng: &mut Rng, buf: &mut Buf) {
    let (nv, nh) = (rbm.nv, rbm.nh);
    rbm.hidden_inputs(v0, &mut buf.x);
    for j in 0..nh {
        buf.hp[j] = buf.x[j].tanh();
        buf.h[j] = coin_f32(buf.x[j], rng);
    }
    // the reconstruction error is read from one down step from the example, in both modes
    rbm.visible_inputs(&buf.h, &mut buf.y);
    for i in 0..nv {
        buf.v[i] = coin_f32(buf.y[i], rng);
        if buf.v[i] != v0[i] {
            buf.wrong += 1.0;
        }
    }
    if let Some(c) = chain {
        // standing chain: k sweeps from where it stood
        for _ in 0..k {
            rbm.hidden_inputs(c, &mut buf.x);
            for j in 0..nh {
                buf.h[j] = coin_f32(buf.x[j], rng);
            }
            rbm.visible_inputs(&buf.h, &mut buf.y);
            for i in 0..nv {
                c[i] = coin_f32(buf.y[i], rng);
            }
        }
        buf.v.copy_from_slice(c);
    } else {
        for _ in 1..k {
            rbm.hidden_inputs(&buf.v, &mut buf.x);
            for j in 0..nh {
                buf.h[j] = coin_f32(buf.x[j], rng);
            }
            rbm.visible_inputs(&buf.h, &mut buf.y);
            for i in 0..nv {
                buf.v[i] = coin_f32(buf.y[i], rng);
            }
        }
    }
    rbm.hidden_inputs(&buf.v, &mut buf.x);
    for j in 0..nh {
        let hn = buf.x[j].tanh();
        let row = &mut buf.gw[j * nv..(j + 1) * nv];
        let (p, q) = (buf.hp[j], hn);
        for i in 0..nv {
            row[i] += p * v0[i] - q * buf.v[i];
        }
        buf.gb[j] += p - q;
    }
    for i in 0..nv {
        buf.ga[i] += v0[i] - buf.v[i];
    }
}

/// Fit the machine to `rows` (flat, nv per row). Calls `log` after every epoch.
pub fn train(rbm: &mut Rbm, rows: &[f32], o: &TrainOpts, log: &mut dyn FnMut(&EpochLog, &Rbm)) {
    let (nv, nh) = (rbm.nv, rbm.nh);
    let n = rows.len() / nv;
    let threads = o.threads.max(1);
    let batch = o.batch.clamp(1, n);
    let mut bufs: Vec<Buf> = (0..threads).map(|_| Buf::new(nv, nh)).collect();
    let (mut vw, mut va, mut vb) = (vec![0f32; nv * nh], vec![0f32; nv], vec![0f32; nh]);
    let mut order: Vec<usize> = (0..n).collect();
    let mut shuf = Rng::new(mix(o.seed, 0xABCD, 1));
    let mut chains: Vec<Vec<f32>> = if o.persistent {
        (0..batch).map(|j| rows[(j % n) * nv..(j % n + 1) * nv].to_vec()).collect()
    } else {
        Vec::new()
    };
    for epoch in 0..o.epochs {
        let t0 = std::time::Instant::now();
        for j in (1..n).rev() {
            let r = shuf.below(j + 1);
            order.swap(j, r);
        }
        let mom = if epoch < o.momentum_from { 0.5 } else { o.momentum } as f32;
        // the rate falls linearly to a tenth over the epochs, as in learn.rs
        let rate = (o.rate * (1.0 - 0.9 * epoch as f64 / o.epochs.max(1) as f64)) as f32;
        let mut wrong = 0.0;
        for (bi, chunk) in order.chunks(batch).enumerate() {
            let per = (chunk.len() + threads - 1) / threads;
            let rbm_ref: &Rbm = rbm;
            let mut chain_parts: Vec<&mut [Vec<f32>]> = if o.persistent {
                chains[..chunk.len()].chunks_mut(per).collect()
            } else {
                Vec::new()
            };
            std::thread::scope(|sc| {
                for (t, (buf, part)) in bufs.iter_mut().zip(chunk.chunks(per)).enumerate() {
                    let mut cp = if o.persistent { Some(std::mem::take(&mut chain_parts[t])) } else { None };
                    sc.spawn(move || {
                        buf.clear();
                        for (q, &r) in part.iter().enumerate() {
                            let mut rng = Rng::new(mix(o.seed, (epoch * n + bi * batch + t * per + q) as u64, 7));
                            let chain = cp.as_mut().map(|c| &mut c[q][..]);
                            example_grad(rbm_ref, &rows[r * nv..(r + 1) * nv], o.cd_k.max(1), chain, &mut rng, buf);
                        }
                    });
                }
            });
            let used = (chunk.len() + per - 1) / per;
            for t in 1..used {
                let (head, tail) = bufs.split_at_mut(t);
                let (b0, bt) = (&mut head[0], &tail[0]);
                axpy(1.0, &bt.gw, &mut b0.gw);
                axpy(1.0, &bt.ga, &mut b0.ga);
                axpy(1.0, &bt.gb, &mut b0.gb);
                b0.wrong += bt.wrong;
            }
            let g = &bufs[0];
            wrong += g.wrong;
            let s = 1.0 / chunk.len() as f32;
            let dec = o.decay as f32;
            for i in 0..nv * nh {
                vw[i] = mom * vw[i] + rate * (g.gw[i] * s - dec * rbm.w[i]);
                rbm.w[i] += vw[i];
            }
            for i in 0..nv {
                va[i] = mom * va[i] + rate * g.ga[i] * s;
                rbm.a[i] += va[i];
            }
            for j in 0..nh {
                vb[j] = mom * vb[j] + rate * g.gb[j] * s;
                rbm.b[j] += vb[j];
            }
        }
        log(&EpochLog { epoch: epoch + 1, recon: wrong / (n * nv) as f64, secs: t0.elapsed().as_secs_f64() }, rbm);
    }
}

/// A random source for the classify path: `signed()` uniform in [-1, 1), `unit()` in [0, 1).
pub trait Coin {
    fn signed(&mut self) -> f64;
    fn unit(&mut self) -> f64;
}

impl Coin for Rng {
    fn signed(&mut self) -> f64 {
        Rng::signed(self)
    }
    fn unit(&mut self) -> f64 {
        Rng::unit(self)
    }
}

/// The browser engine's xorshift128 (sites/settle-site/src/engine/ising.js), bit for bit, so a JS port can
/// repeat a Rust settle exactly.
pub struct Xs128 {
    a: u32,
    b: u32,
    c: u32,
    d: u32,
}

impl Xs128 {
    pub fn new(seed: u32) -> Xs128 {
        let s = seed ^ 0x9e37_79b9;
        let mut r = Xs128 { a: if s == 0 { 1 } else { s }, b: 362436069, c: 521288629, d: 88675123 };
        for _ in 0..16 {
            r.u32();
        }
        r
    }
    pub fn u32(&mut self) -> u32 {
        let t = self.a ^ (self.a << 11);
        self.a = self.b;
        self.b = self.c;
        self.c = self.d;
        self.d = self.d ^ (self.d >> 19) ^ (t ^ (t >> 8));
        self.d
    }
}

impl Coin for Xs128 {
    fn unit(&mut self) -> f64 {
        self.u32() as f64 / 4294967296.0
    }
    fn signed(&mut self) -> f64 {
        2.0 * self.unit() - 1.0
    }
}

/// The pixel part of every hidden input, in f64: b_k + sum_i W_ki p_i over the 784 pixels.
pub fn pixel_inputs(rbm: &Rbm, px: &[f32]) -> Vec<f64> {
    (0..rbm.nh)
        .map(|k| {
            let row = &rbm.w[k * rbm.nv..k * rbm.nv + PIX];
            let mut s = rbm.b[k] as f64;
            for i in 0..PIX {
                s += row[i] as f64 * px[i] as f64;
            }
            s
        })
        .collect()
}

/// Exact readout of a joint machine: argmax over c of -F(pixels, e_c), and the ten scores (pixel term dropped).
pub fn exact_label(rbm: &Rbm, px: &[f32]) -> (usize, Vec<f64>) {
    let xp = pixel_inputs(rbm, px);
    exact_from(rbm, &xp)
}

pub fn exact_from(rbm: &Rbm, xp: &[f64]) -> (usize, Vec<f64>) {
    let nv = rbm.nv;
    let mut scores = vec![0f64; CLASSES];
    for c in 0..CLASSES {
        let mut t = 0f64;
        for l in 0..CLASSES {
            let s = if l == c { 1.0 } else { -1.0 };
            t += rbm.a[PIX + l] as f64 * s;
        }
        for k in 0..rbm.nh {
            let mut x = xp[k];
            for l in 0..CLASSES {
                let s = if l == c { 1.0 } else { -1.0 };
                x += rbm.w[k * nv + PIX + l] as f64 * s;
            }
            t += ln2cosh(x);
        }
        scores[c] = t;
    }
    let best = (0..CLASSES).fold(0, |b, c| if scores[c] > scores[b] { c } else { b });
    (best, scores)
}

/// Hold the pixels, start the label things at random, settle: each sweep samples every hidden thing given the
/// pixels and labels, then every label thing given the hidden things. Burn in sweeps/10, then count yes over
/// `sweeps`. Returns the label most often yes (ties to the lower digit) and the counts.
pub fn settle_label<C: Coin>(rbm: &Rbm, px: &[f32], sweeps: usize, coin: &mut C) -> (usize, Vec<u32>) {
    let xp = pixel_inputs(rbm, px);
    settle_from(rbm, &xp, sweeps, coin)
}

pub fn settle_from<C: Coin>(rbm: &Rbm, xp: &[f64], sweeps: usize, coin: &mut C) -> (usize, Vec<u32>) {
    let (nv, nh) = (rbm.nv, rbm.nh);
    let mut lab: Vec<f64> = (0..CLASSES).map(|_| if coin.unit() < 0.5 { 1.0 } else { -1.0 }).collect();
    let mut h = vec![0f64; nh];
    let mut cnt = vec![0u32; CLASSES];
    let burn = (sweeps / 10).max(1);
    for t in 0..burn + sweeps {
        for k in 0..nh {
            let mut x = xp[k];
            for l in 0..CLASSES {
                x += rbm.w[k * nv + PIX + l] as f64 * lab[l];
            }
            h[k] = if x.tanh() > coin.signed() { 1.0 } else { -1.0 };
        }
        for l in 0..CLASSES {
            let mut y = rbm.a[PIX + l] as f64;
            for k in 0..nh {
                y += rbm.w[k * nv + PIX + l] as f64 * h[k];
            }
            lab[l] = if y.tanh() > coin.signed() { 1.0 } else { -1.0 };
        }
        if t >= burn {
            for l in 0..CLASSES {
                if lab[l] > 0.0 {
                    cnt[l] += 1;
                }
            }
        }
    }
    let best = (0..CLASSES).fold(0, |b, c| if cnt[c] > cnt[b] { c } else { b });
    (best, cnt)
}

/// Hidden things' yes-rates (1 + tanh x_k)/2 given the visible row (a pixel-only machine).
pub fn hidden_rates(rbm: &Rbm, v: &[f32], out: &mut [f32]) {
    rbm.hidden_inputs(v, out);
    for x in out.iter_mut() {
        *x = 0.5 * (1.0 + x.tanh());
    }
}

/// Multinomial logistic regression: p(c | x) = softmax(W x + b).
#[derive(Clone, Debug)]
pub struct Softmax {
    pub d: usize,
    /// CLASSES x (d + 1), the last column is the bias
    pub w: Vec<f32>,
}

#[derive(Clone, Debug)]
pub struct SoftmaxOpts {
    pub epochs: usize,
    pub rate: f64,
    pub batch: usize,
    pub l2: f64,
    pub seed: u64,
}

impl Softmax {
    pub fn logits(&self, x: &[f32]) -> [f32; CLASSES] {
        let mut z = [0f32; CLASSES];
        for c in 0..CLASSES {
            let row = &self.w[c * (self.d + 1)..(c + 1) * (self.d + 1)];
            z[c] = dot(&row[..self.d], x) + row[self.d];
        }
        z
    }
    pub fn predict(&self, x: &[f32]) -> usize {
        let z = self.logits(x);
        (0..CLASSES).fold(0, |b, c| if z[c] > z[b] { c } else { b })
    }
    /// Adam on the mean cross-entropy plus l2/2 |W|^2 (bias not decayed).
    pub fn fit(x: &[f32], d: usize, y: &[u8], o: &SoftmaxOpts) -> Softmax {
        let n = x.len() / d;
        let p = CLASSES * (d + 1);
        let mut m = Softmax { d, w: vec![0.0; p] };
        let (mut m1, mut m2) = (vec![0f64; p], vec![0f64; p]);
        let (b1, b2, eps) = (0.9f64, 0.999f64, 1e-8f64);
        let mut order: Vec<usize> = (0..n).collect();
        let mut rng = Rng::new(mix(o.seed, 0x50F7, 3));
        let mut g = vec![0f64; p];
        let mut step = 0i32;
        for _ in 0..o.epochs {
            for j in (1..n).rev() {
                let r = rng.below(j + 1);
                order.swap(j, r);
            }
            for chunk in order.chunks(o.batch.max(1)) {
                g.iter_mut().for_each(|v| *v = 0.0);
                for &r in chunk {
                    let xr = &x[r * d..(r + 1) * d];
                    let z = m.logits(xr);
                    let mx = z.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
                    let e: Vec<f64> = z.iter().map(|v| ((v - mx) as f64).exp()).collect();
                    let s: f64 = e.iter().sum();
                    for c in 0..CLASSES {
                        let q = e[c] / s - if c == y[r] as usize { 1.0 } else { 0.0 };
                        let gr = &mut g[c * (d + 1)..(c + 1) * (d + 1)];
                        for i in 0..d {
                            gr[i] += q * xr[i] as f64;
                        }
                        gr[d] += q;
                    }
                }
                step += 1;
                let inv = 1.0 / chunk.len() as f64;
                let (c1, c2) = (1.0 - b1.powi(step), 1.0 - b2.powi(step));
                for i in 0..p {
                    let wd = if (i % (d + 1)) == d { 0.0 } else { o.l2 * m.w[i] as f64 };
                    let gi = g[i] * inv + wd;
                    m1[i] = b1 * m1[i] + (1.0 - b1) * gi;
                    m2[i] = b2 * m2[i] + (1.0 - b2) * gi * gi;
                    m.w[i] -= (o.rate * (m1[i] / c1) / ((m2[i] / c2).sqrt() + eps)) as f32;
                }
            }
        }
        m
    }
}

/// Class means of the training rows; returns the predicted class of each test row (squared Euclidean distance).
pub fn nearest_centroid(train: &[f32], ytr: &[u8], test: &[f32], d: usize) -> Vec<usize> {
    let mut mu = vec![0f64; CLASSES * d];
    let mut cnt = vec![0f64; CLASSES];
    for (r, &y) in train.chunks_exact(d).zip(ytr) {
        cnt[y as usize] += 1.0;
        for i in 0..d {
            mu[y as usize * d + i] += r[i] as f64;
        }
    }
    for c in 0..CLASSES {
        for i in 0..d {
            mu[c * d + i] /= cnt[c].max(1.0);
        }
    }
    test.chunks_exact(d)
        .map(|r| {
            let dist = |c: usize| (0..d).map(|i| (r[i] as f64 - mu[c * d + i]).powi(2)).sum::<f64>();
            (0..CLASSES).fold(0, |b, c| if dist(c) < dist(b) { c } else { b })
        })
        .collect()
}

/// Scramble which label goes with which picture (the negative control).
pub fn shuffled_labels(y: &[u8], seed: u64) -> Vec<u8> {
    let mut out = y.to_vec();
    let mut rng = Rng::new(mix(seed, 0x5AFF, 11));
    for j in (1..out.len()).rev() {
        let r = rng.below(j + 1);
        out.swap(j, r);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::learn::Net;

    fn data_dir() -> Option<String> {
        let d = concat!(env!("CARGO_MANIFEST_DIR"), "/../runs/mnist/data");
        if std::path::Path::new(&format!("{}/train-labels-idx1-ubyte", d)).exists() {
            Some(d.to_string())
        } else {
            None
        }
    }

    #[test]
    fn idx_parsers_read_headers_and_refuse_bad_magic() {
        let mut b = vec![0, 0, 8, 3, 0, 0, 0, 2, 0, 0, 0, 2, 0, 0, 0, 2];
        b.extend([0, 255, 127, 128, 1, 2, 3, 4]);
        let (n, r, c, px) = parse_idx_images(&b).unwrap();
        assert_eq!((n, r, c, px.len()), (2, 2, 2, 8));
        assert_eq!(spins(&px[..4]), vec![-1.0, 1.0, -1.0, 1.0], "128 is the threshold: 127 no, 128 yes");
        let l = vec![0, 0, 8, 1, 0, 0, 0, 3, 5, 0, 4];
        assert_eq!(parse_idx_labels(&l).unwrap(), vec![5, 0, 4]);
        assert!(parse_idx_labels(&b).is_err(), "an image file is not a label file");
        assert!(parse_idx_images(&l).is_err());
        let mut short = l.clone();
        short.pop();
        assert!(parse_idx_labels(&short).is_err(), "a truncated file is refused");
    }

    #[test]
    fn mnist_files_hold_the_published_counts_and_first_labels() {
        let Some(d) = data_dir() else {
            eprintln!("runs/mnist/data is absent (gitignored); fetch it per runs/mnist/PROVENANCE.md to run this test");
            return;
        };
        let tr = load(&d, "train").unwrap();
        let te = load(&d, "test").unwrap();
        assert_eq!((tr.n, te.n), (60_000, 10_000));
        // the first training digit is a 5 and the first test digit a 7 (LeCun, Cortes and Burges' ordering)
        assert_eq!(&tr.labels[..5], &[5, 0, 4, 1, 9]);
        assert_eq!(&te.labels[..5], &[7, 2, 1, 0, 4]);
        let mut per = [0usize; 10];
        tr.labels.iter().for_each(|&y| per[y as usize] += 1);
        assert_eq!(per, [5923, 6742, 5958, 6131, 5842, 5421, 5918, 6265, 5851, 5949]);
        assert!(load(&d, "valid").is_err());
    }

    fn tiny() -> (Rbm, Rng) {
        let mut rng = Rng::new(3);
        let mut m = Rbm::new(5, 3, 0.7, &mut rng);
        m.a = vec![0.2, -0.3, 0.1, 0.0, 0.5];
        m.b = vec![-0.1, 0.4, 0.0];
        (m, rng)
    }

    #[test]
    fn restricted_layout_agrees_with_learn_net() {
        // the same machine in learn.rs's dense Net: free energies must match
        let (m, _) = tiny();
        let n = m.nv + m.nh;
        let mut w = vec![0.0f64; n * n];
        for k in 0..m.nh {
            for i in 0..m.nv {
                w[i * n + m.nv + k] = m.w[k * m.nv + i] as f64;
                w[(m.nv + k) * n + i] = m.w[k * m.nv + i] as f64;
            }
        }
        let net = Net {
            units: (0..n).collect(),
            nv: m.nv,
            h: m.a.iter().chain(&m.b).map(|&x| x as f64).collect(),
            w,
            pairs: (0..m.nv).flat_map(|a| (m.nv..n).map(move |b| (a, b))).collect(),
        };
        for bits in 0..32u32 {
            let v: Vec<f32> = (0..5).map(|i| if bits >> i & 1 == 1 { 1.0 } else { -1.0 }).collect();
            let vd: Vec<f64> = v.iter().map(|&x| x as f64).collect();
            assert!((m.neg_free(&v) - net.neg_beta_free(&vd, 1.0)).abs() < 1e-5, "free energy differs at {:05b}", bits);
        }
    }

    #[test]
    fn block_gibbs_matches_exact_marginals_and_a_reversed_pull_does_not() {
        let (m, mut rng) = tiny();
        let exact = |m: &Rbm| {
            let lw: Vec<(Vec<f32>, f64)> = (0..32u32)
                .map(|bits| {
                    let v: Vec<f32> = (0..5).map(|i| if bits >> i & 1 == 1 { 1.0 } else { -1.0 }).collect();
                    let l = m.neg_free(&v);
                    (v, l)
                })
                .collect();
            let mx = lw.iter().map(|x| x.1).fold(f64::NEG_INFINITY, f64::max);
            let z: f64 = lw.iter().map(|x| (x.1 - mx).exp()).sum();
            (0..5).map(|i| lw.iter().map(|(v, l)| (l - mx).exp() / z * v[i] as f64).sum::<f64>()).collect::<Vec<f64>>()
        };
        let want = exact(&m);
        let mut v = vec![1.0f32; 5];
        let (mut x, mut h, mut y) = (vec![0f32; 3], vec![0f32; 3], vec![0f32; 5]);
        let mut acc = vec![0f64; 5];
        let sweeps = 1_000_000;
        for t in 0..sweeps + 1000 {
            m.hidden_inputs(&v, &mut x);
            for k in 0..3 {
                h[k] = coin_f32(x[k], &mut rng);
            }
            m.visible_inputs(&h, &mut y);
            for i in 0..5 {
                v[i] = coin_f32(y[i], &mut rng);
            }
            if t >= 1000 {
                for i in 0..5 {
                    acc[i] += v[i] as f64;
                }
            }
        }
        let worst = (0..5).map(|i| (acc[i] / sweeps as f64 - want[i]).abs()).fold(0f64, f64::max);
        eprintln!("block Gibbs worst gap to exact over {} sweeps: {:.4}", sweeps, worst);
        for i in 0..5 {
            assert!((acc[i] / sweeps as f64 - want[i]).abs() < 0.01, "thing {}: {} vs exact {}", i, acc[i] / sweeps as f64, want[i]);
        }
        let mut r = m.clone();
        r.w.iter_mut().for_each(|w| *w = -*w);
        r.a.iter_mut().for_each(|a| *a = -*a);
        let rev = exact(&r);
        assert!((0..5).any(|i| (rev[i] - want[i]).abs() > 0.05), "the negative control must differ");
    }

    #[test]
    fn contrastive_divergence_learns_ten_templates_and_shuffled_labels_do_not() {
        // ten random sparse 784-pixel templates, 10% of pixels flipped per example, labels 0..9
        let mut rng = Rng::new(9);
        let tpl: Vec<Vec<f32>> = (0..CLASSES).map(|_| (0..PIX).map(|_| if rng.unit() < 0.2 { 1.0 } else { -1.0 }).collect()).collect();
        let n = 500;
        let (mut rows, mut ys) = (Vec::new(), Vec::new());
        for r in 0..n {
            let c = r % CLASSES;
            for &p in &tpl[c] {
                rows.push(if rng.unit() < 0.1 { -p } else { p });
            }
            for l in 0..CLASSES {
                rows.push(if l == c { 1.0 } else { -1.0 });
            }
            ys.push(c as u8);
        }
        let o = TrainOpts { epochs: 8, rate: 0.005, batch: 20, momentum: 0.9, momentum_from: 2, decay: 0.0002, cd_k: 1, persistent: false, seed: 1, threads: 2 };
        let run = |rows: &[f32]| {
            let mut m = Rbm::new(PIX + CLASSES, 24, 0.01, &mut Rng::new(4));
            m.init_leans(rows);
            let mut logs = Vec::new();
            train(&mut m, rows, &o, &mut |e, _| logs.push(e.recon));
            (m, logs)
        };
        let score = |m: &Rbm, coin: &mut Rng| {
            let mut right = (0, 0);
            for c in 0..CLASSES {
                if exact_label(m, &tpl[c]).0 == c {
                    right.0 += 1;
                }
                if settle_label(m, &tpl[c], 100, coin).0 == c {
                    right.1 += 1;
                }
            }
            right
        };
        let (m, logs) = run(&rows);
        assert!(logs.last().unwrap() < &logs[0], "reconstruction error falls: {:?}", logs);
        let right = score(&m, &mut Rng::new(5));
        assert!(right.0 == 10 && right.1 >= 9, "templates classified (exact, settled): {:?}", right);
        // negative control: labels scrambled across rows; chance is 1 in 10
        let sy = shuffled_labels(&ys, 1);
        let mut srows = rows.clone();
        for r in 0..n {
            for l in 0..CLASSES {
                srows[r * (PIX + CLASSES) + PIX + l] = if l == sy[r] as usize { 1.0 } else { -1.0 };
            }
        }
        let (ms, _) = run(&srows);
        let rs = score(&ms, &mut Rng::new(5));
        assert!(rs.0 <= 4 && rs.1 <= 4, "shuffled labels must not classify the templates: {:?}", rs);
    }

    #[test]
    fn training_is_the_same_for_any_thread_count() {
        let mut rng = Rng::new(2);
        let rows: Vec<f32> = (0..60 * 20).map(|_| if rng.unit() < 0.4 { 1.0 } else { -1.0 }).collect();
        let mut out = Vec::new();
        for threads in [1, 3] {
            for persistent in [false, true] {
                let mut m = Rbm::new(20, 7, 0.1, &mut Rng::new(8));
                let o = TrainOpts { epochs: 2, rate: 0.02, batch: 10, momentum: 0.9, momentum_from: 1, decay: 0.0001, cd_k: 2, persistent, seed: 3, threads };
                train(&mut m, &rows, &o, &mut |_, _| {});
                out.push(m.w.clone());
            }
        }
        // sums over threads add in a different grouping, so compare to float tolerance
        for (a, b) in [(0, 2), (1, 3)] {
            let d = out[a].iter().zip(&out[b]).map(|(x, y)| (x - y).abs()).fold(0f32, f32::max);
            assert!(d < 1e-4, "thread count changed the fit by {}", d);
        }
    }

    #[test]
    fn softmax_and_centroid_fit_separable_data_and_fail_on_shuffled_labels() {
        let mut rng = Rng::new(6);
        let d = 12;
        let (mut x, mut y) = (Vec::new(), Vec::new());
        for r in 0..600 {
            let c = r % CLASSES;
            for i in 0..d {
                x.push(if i == c { 1.0 } else { 0.0 } + 0.2 * rng.normal() as f32);
            }
            y.push(c as u8);
        }
        let o = SoftmaxOpts { epochs: 30, rate: 0.01, batch: 20, l2: 1e-4, seed: 1 };
        let m = Softmax::fit(&x, d, &y, &o);
        let acc = |pred: &dyn Fn(usize) -> usize| (0..600).filter(|&r| pred(r) == y[r] as usize).count() as f64 / 600.0;
        let a1 = acc(&|r| m.predict(&x[r * d..(r + 1) * d]));
        let nc = nearest_centroid(&x, &y, &x, d);
        let a2 = acc(&|r| nc[r]);
        assert!(a1 > 0.95 && a2 > 0.95, "softmax {} centroid {}", a1, a2);
        let sy = shuffled_labels(&y, 2);
        let ms = Softmax::fit(&x[..300 * d], d, &sy[..300], &o);
        let held = (300..600).filter(|&r| ms.predict(&x[r * d..(r + 1) * d]) == y[r] as usize).count() as f64 / 300.0;
        assert!(held < 0.3, "a readout trained on shuffled labels stays near chance on held rows: {}", held);
    }

    #[test]
    fn weights_file_round_trips() {
        let (m, _) = tiny();
        let p = std::env::temp_dir().join(format!("settle_rbm_{}.bin", std::process::id()));
        let ps = p.to_str().unwrap();
        m.save(ps).unwrap();
        let back = load_rbm(ps).unwrap();
        assert_eq!((back.nv, back.nh, back.a.clone(), back.b.clone(), back.w.clone()), (m.nv, m.nh, m.a, m.b, m.w));
        let _ = fs::remove_file(ps);
    }

    #[test]
    fn xs128_matches_the_browser_engine_first_draws() {
        // the first four u32 draws of ising.js `new Rng(1)`, computed there (node) and pinned here
        let mut r = Xs128::new(1);
        let got: Vec<u32> = (0..4).map(|_| r.u32()).collect();
        assert_eq!(got, XS128_SEED1_FIRST4.to_vec());
    }
}

/// Pinned by `xs128_matches_the_browser_engine_first_draws`; produced by the browser engine under node.
#[cfg(test)]
const XS128_SEED1_FIRST4: [u32; 4] = [3449496019, 3747456639, 1409788197, 648723358];
