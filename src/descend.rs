//! DESCEND: gradient descent is Settling continuous things on a loss. The loss is the energy, temperature is the
//! noise, temperature 0 is plain gradient descent, and temperature above 0 is Langevin dynamics whose cloud of
//! positions is the Bayesian posterior over the parameters.
//!
//! ```text
//! model :line do
//!   data "line.csv"                              # one example per row, the target in the last column
//!   loss :least_squares, noise: 0.5, prior: 10   # parameters :w1 .. :wp and :bias
//! end
//! run :line do
//!   descend 40_000, step: 0.002, temperature: 1, seed: 1    # Langevin: the cloud is the posterior
//!   ask                                                    # means and spreads, beside the exact posterior
//!   score                                                  # predictions of the cloud on the test rows
//! end
//! ```
//!
//! <claudes_code_comments>
//! ** Function List **
//! Data::parse_csv(text) / Data::mnist(dir, split, from, rows) - examples as rows of features and one target
//! Problem::nparams / names / outputs  - the parameter layout of each loss piece, and its names
//! Problem::grad(th, data, rows, scale, g, work) - the gradient of U = scale * sum_rows loss + prior, returns the loss
//! Problem::energy(th, data)           - U over every row (the full loss plus the prior)
//! Problem::predict(th, x, out)        - class probabilities, or the predicted value
//! Problem::gaussian()                 - (A, c) when U is exactly quadratic (springs, least squares)
//! descend(problem, data, opts, cb)    - the Settling loop: Langevin steps (or Adam at T = 0), minibatches,
//!                                       walkers, temperature and step schedules, kept snapshots
//! predictive(problem, samples, data)  - averaged predictions of a set of parameter vectors, with the mutual
//!                                       information between prediction and parameters (the cloud's own uncertainty)
//! metrics(problem, pred, data)        - accuracy, NLL, ECE (15 bins), or RMSE and 95% coverage for values
//! Descend (Ext)                       - the statements: data, test, loss (model); descend, ask, score (run)
//!
//! ** Technical Review **
//! - Energy. U(th) = sum_i loss_i(th) + |th|^2 / (2 prior^2). The loss pieces: least squares (a linear model
//!   with Gaussian noise of sd `noise`), logistic (a linear model with a sigmoid for two classes, a softmax for
//!   more), a net (one tanh hidden layer, then the same outputs), and the numbers family's springs
//!   (U = th.A.th/2 - b.th, no prior). exp(-U) is the Bayesian posterior with a Gaussian prior.
//! - The step. th <- th - h * grad_hat U + sqrt(2 * T * h) * xi, the Euler-Maruyama step of overdamped Langevin
//!   dynamics. With a minibatch of B of N rows, grad_hat U = (N/B) sum_batch grad loss + th/prior^2 (stochastic
//!   gradient Langevin dynamics, Welling and Teh 2011). T = 0 is gradient descent (full batch) or SGD.
//!   T falls along a straight line from `temperature` to `cool_to`; h falls geometrically to `step_to`.
//! - Springs parity. On `loss :springs` with one walker, every = 1 and no schedule, the arithmetic, random
//!   stream and accumulators are those of numbers.rs `walk`, so means, standard errors and covariance are equal
//!   bit for bit to `drift` (tested).
//! - Statistics. After `burn` steps every `every`-th position is kept: running sums give the mean, the
//!   variance (and the covariance when there are at most 64 parameters), and 20 batch means give standard
//!   errors. `keep` snapshots per walker are stored for predictions. For a quadratic U the stationary covariance
//!   of the discrete step is T A^-1 (I - h A/2)^-1; `ask` removes the step's inflation as `spread` does.
//! - Methods. `method: :adam` runs Adam (beta 0.9, 0.999, eps 1e-8) with rate `step`; it has no temperature and
//!   refuses one. Walkers start at 0, or at init * N(0, 1) for a net (0 is a saddle of a net).
//! - State lives in Model.notes: "descend:data", "descend:test", "descend:loss", "descend:last".
//! </claudes_code_comments>

use crate::ext::{Claim, Ctx, Ext};
use crate::lex::{err, kw, kwargs, num, only, text, SettleError, Tok};
use crate::model::{Model, State};
use crate::numbers::{gauss_inverse, gauss_solve, inverse_from_spread, rel_frob, stiffest, Springs, BLOWUP};
use crate::rng::Rng;
use std::time::Instant;

const DATA: &str = "descend:data";
const TEST: &str = "descend:test";
const LOSS: &str = "descend:loss";
const LAST: &str = "descend:last";
/// Parameters above this count keep only variances, not the covariance.
pub const COV_MAX: usize = 64;

// ---------------------------------------------------------------------------------------------------------
// Data: one example per row
// ---------------------------------------------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
pub struct Data {
    pub n: usize,
    pub p: usize,
    /// n x p, row-major
    pub x: Vec<f64>,
    pub y: Vec<f64>,
}

impl Data {
    pub fn row(&self, r: usize) -> &[f64] {
        &self.x[r * self.p..(r + 1) * self.p]
    }

    /// Rows of numbers separated by spaces or commas; the last column is the target. Empty lines and lines
    /// starting with `#` are skipped; a first line that is not all numbers is a header and is skipped.
    pub fn parse_csv(text: &str) -> Result<Data, String> {
        let mut d = Data::default();
        let mut width = 0;
        for (k, line) in text.lines().enumerate() {
            let t = line.trim();
            if t.is_empty() || t.starts_with('#') {
                continue;
            }
            let cells: Vec<&str> = t.split(|c: char| c.is_whitespace() || c == ',').filter(|s| !s.is_empty()).collect();
            let vals: Result<Vec<f64>, _> = cells.iter().map(|c| c.parse::<f64>()).collect();
            let vals = match vals {
                Ok(v) => v,
                Err(_) if d.n == 0 && width == 0 => continue,
                Err(_) => return Err(format!("row {} has a cell that is not a number", k + 1)),
            };
            if vals.len() < 2 {
                return Err(format!("row {} needs at least one feature and a target", k + 1));
            }
            if width == 0 {
                width = vals.len();
            } else if vals.len() != width {
                return Err(format!("row {} has {} columns, the first row has {}", k + 1, vals.len(), width));
            }
            d.x.extend_from_slice(&vals[..width - 1]);
            d.y.push(vals[width - 1]);
            d.n += 1;
        }
        if d.n == 0 {
            return Err("no rows of numbers".into());
        }
        d.p = width - 1;
        Ok(d)
    }

    /// MNIST pictures as 784 features in [0, 1] (grey / 255) and the digit as the target.
    pub fn mnist(dir: &str, split: &str, from: usize, rows: usize) -> Result<Data, String> {
        let dg = crate::mnist::load(dir, split)?;
        if from >= dg.n {
            return Err(format!("from: {} is past the {} pictures of the {} split", from, dg.n, split));
        }
        let to = (from + rows).min(dg.n);
        let p = crate::mnist::PIX;
        let mut d = Data { n: to - from, p, x: Vec::with_capacity((to - from) * p), y: Vec::with_capacity(to - from) };
        for r in from..to {
            d.x.extend(dg.images[r * p..(r + 1) * p].iter().map(|&v| v as f64 / 255.0));
            d.y.push(dg.labels[r] as f64);
        }
        Ok(d)
    }

    fn save(&self, m: &mut Model, key: &str) {
        let mut v = Vec::with_capacity(2 + self.x.len() + self.y.len());
        v.push(self.n as f64);
        v.push(self.p as f64);
        v.extend_from_slice(&self.x);
        v.extend_from_slice(&self.y);
        m.notes.insert(key.to_string(), (v, Vec::new()));
    }

    fn load(m: &Model, key: &str) -> Option<Data> {
        let (v, _) = m.notes.get(key)?;
        let (n, p) = (v[0] as usize, v[1] as usize);
        Some(Data { n, p, x: v[2..2 + n * p].to_vec(), y: v[2 + n * p..2 + n * p + n].to_vec() })
    }
}

// ---------------------------------------------------------------------------------------------------------
// The loss pieces
// ---------------------------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Springs,
    LeastSquares,
    Logistic,
    Net,
}

#[derive(Clone, Debug)]
pub struct Problem {
    pub kind: Kind,
    /// 0: the output is a value (Gaussian noise of sd `noise`); 2 or more: the output is a class.
    pub classes: usize,
    pub hidden: usize,
    pub noise: f64,
    /// Standard deviation of the Gaussian prior on every parameter; 0 means no prior.
    pub prior: f64,
    /// Walkers start at init * N(0, 1) (0: they start at 0).
    pub init: f64,
    /// Springs: the stiffness matrix and pushes, and the numbers' names.
    pub a: Vec<f64>,
    pub b: Vec<f64>,
    pub springs: Vec<String>,
}

/// Scratch space for one gradient (the hidden layer of a net).
#[derive(Default)]
pub struct Work {
    z: Vec<f64>,
    dz: Vec<f64>,
    h: Vec<f64>,
    dh: Vec<f64>,
}

fn softplus(z: f64) -> f64 {
    z.max(0.0) + (-z.abs()).exp().ln_1p()
}

fn sigmoid(z: f64) -> f64 {
    if z >= 0.0 {
        1.0 / (1.0 + (-z).exp())
    } else {
        let e = z.exp();
        e / (1.0 + e)
    }
}

impl Problem {
    pub fn springs(sp: &Springs) -> Problem {
        Problem { kind: Kind::Springs, classes: 0, hidden: 0, noise: 1.0, prior: 0.0, init: 0.0, a: sp.a.clone(), b: sp.b.clone(), springs: sp.names.clone() }
    }
    pub fn least_squares(noise: f64, prior: f64) -> Problem {
        Problem { kind: Kind::LeastSquares, classes: 0, hidden: 0, noise, prior, init: 0.0, a: vec![], b: vec![], springs: vec![] }
    }
    pub fn logistic(classes: usize, prior: f64) -> Problem {
        Problem { kind: Kind::Logistic, classes, hidden: 0, noise: 1.0, prior, init: 0.0, a: vec![], b: vec![], springs: vec![] }
    }
    pub fn net(hidden: usize, classes: usize, noise: f64, prior: f64, init: f64) -> Problem {
        Problem { kind: Kind::Net, classes, hidden, noise, prior, init, a: vec![], b: vec![], springs: vec![] }
    }

    /// Number of outputs: one for a value or for two classes (a sigmoid), else one per class (a softmax).
    pub fn outputs(&self) -> usize {
        if self.classes > 2 {
            self.classes
        } else {
            1
        }
    }

    pub fn nparams(&self, p: usize) -> usize {
        let m = self.outputs();
        match self.kind {
            Kind::Springs => self.springs.len(),
            Kind::LeastSquares | Kind::Logistic => m * (p + 1),
            Kind::Net => self.hidden * (p + 1) + m * (self.hidden + 1),
        }
    }

    /// Parameter names, in layout order. Features are numbered from 1; classes from 0; hidden things from 1.
    pub fn names(&self, p: usize) -> Vec<String> {
        let m = self.outputs();
        let mut out = Vec::new();
        match self.kind {
            Kind::Springs => return self.springs.clone(),
            Kind::LeastSquares | Kind::Logistic if m == 1 => {
                out.extend((1..=p).map(|i| format!("w{}", i)));
                out.push("bias".into());
            }
            Kind::LeastSquares | Kind::Logistic => {
                for c in 0..m {
                    out.extend((1..=p).map(|i| format!("w{}_{}", c, i)));
                    out.push(format!("bias{}", c));
                }
            }
            Kind::Net => {
                for j in 1..=self.hidden {
                    out.extend((1..=p).map(|i| format!("h{}_{}", j, i)));
                    out.push(format!("hbias{}", j));
                }
                for c in 0..m {
                    let tag = if m == 1 { String::new() } else { c.to_string() };
                    out.extend((1..=self.hidden).map(|j| format!("o{}_{}", tag, j)).map(|s| s.replace("o_", "o")));
                    out.push(format!("obias{}", tag));
                }
            }
        }
        out
    }

    /// The output layer z (length `outputs()`) of one example; a net also fills the hidden layer w.h.
    fn forward(&self, th: &[f64], x: &[f64], w: &mut Work) {
        let (p, m) = (x.len(), self.outputs());
        w.z.resize(m, 0.0);
        match self.kind {
            Kind::Springs => unreachable!(),
            Kind::LeastSquares | Kind::Logistic => {
                for c in 0..m {
                    let row = &th[c * (p + 1)..(c + 1) * (p + 1)];
                    let mut s = row[p];
                    for i in 0..p {
                        s += row[i] * x[i];
                    }
                    w.z[c] = s;
                }
            }
            Kind::Net => {
                let hd = self.hidden;
                w.h.resize(hd, 0.0);
                for j in 0..hd {
                    let row = &th[j * (p + 1)..(j + 1) * (p + 1)];
                    let mut s = row[p];
                    for i in 0..p {
                        s += row[i] * x[i];
                    }
                    w.h[j] = s.tanh();
                }
                let off = hd * (p + 1);
                for c in 0..m {
                    let row = &th[off + c * (hd + 1)..off + (c + 1) * (hd + 1)];
                    let mut s = row[hd];
                    for j in 0..hd {
                        s += row[j] * w.h[j];
                    }
                    w.z[c] = s;
                }
            }
        }
    }

    /// The loss of one example from its output layer, and d loss / d z into w.dz.
    fn out_loss(&self, y: f64, w: &mut Work) -> f64 {
        let m = self.outputs();
        w.dz.resize(m, 0.0);
        if self.classes == 0 {
            let s2 = self.noise * self.noise;
            let r = w.z[0] - y;
            w.dz[0] = r / s2;
            0.5 * r * r / s2
        } else if self.classes == 2 {
            let z = w.z[0];
            w.dz[0] = sigmoid(z) - y;
            softplus(z) - y * z
        } else {
            let mx = w.z.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
            let s: f64 = w.z.iter().map(|v| (v - mx).exp()).sum();
            let lse = mx + s.ln();
            let yc = y as usize;
            for c in 0..m {
                w.dz[c] = (w.z[c] - lse).exp() - if c == yc { 1.0 } else { 0.0 };
            }
            lse - w.z[yc]
        }
    }

    /// g = scale * sum over `rows` of grad loss + th / prior^2 (springs: g = A th - b). Returns the summed loss
    /// of the rows (unscaled, without the prior; springs: 0). `rows: None` means every row.
    pub fn grad(&self, th: &[f64], data: &Data, rows: Option<&[usize]>, scale: f64, g: &mut [f64], w: &mut Work) -> f64 {
        let d = th.len();
        if self.kind == Kind::Springs {
            // the same arithmetic, in the same order, as numbers.rs `walk`
            for i in 0..d {
                let row = &self.a[i * d..(i + 1) * d];
                let mut gi = -self.b[i];
                for k in 0..d {
                    gi += row[k] * th[k];
                }
                g[i] = gi;
            }
            return 0.0;
        }
        g.iter_mut().for_each(|v| *v = 0.0);
        let p = data.p;
        let m = self.outputs();
        let mut total = 0.0;
        let mut one = |r: usize, g: &mut [f64], w: &mut Work| {
            let x = data.row(r);
            self.forward(th, x, w);
            total += self.out_loss(data.y[r], w);
            match self.kind {
                Kind::Springs => unreachable!(),
                Kind::LeastSquares | Kind::Logistic => {
                    for c in 0..m {
                        let dz = scale * w.dz[c];
                        let gr = &mut g[c * (p + 1)..(c + 1) * (p + 1)];
                        for i in 0..p {
                            gr[i] += dz * x[i];
                        }
                        gr[p] += dz;
                    }
                }
                Kind::Net => {
                    let hd = self.hidden;
                    let off = hd * (p + 1);
                    w.dh.clear();
                    w.dh.resize(hd, 0.0);
                    for c in 0..m {
                        let dz = scale * w.dz[c];
                        let base = off + c * (hd + 1);
                        for j in 0..hd {
                            w.dh[j] += th[base + j] * dz;
                            g[base + j] += dz * w.h[j];
                        }
                        g[base + hd] += dz;
                    }
                    for j in 0..hd {
                        let dj = w.dh[j] * (1.0 - w.h[j] * w.h[j]);
                        let gr = &mut g[j * (p + 1)..(j + 1) * (p + 1)];
                        for i in 0..p {
                            gr[i] += dj * x[i];
                        }
                        gr[p] += dj;
                    }
                }
            }
        };
        match rows {
            Some(rs) => rs.iter().for_each(|&r| one(r, g, w)),
            None => (0..data.n).for_each(|r| one(r, g, w)),
        }
        if self.prior > 0.0 {
            let ip = 1.0 / (self.prior * self.prior);
            for i in 0..d {
                g[i] += ip * th[i];
            }
        }
        total
    }

    /// The summed loss of every row (springs: the springs' energy).
    pub fn data_loss(&self, th: &[f64], data: &Data) -> f64 {
        if self.kind == Kind::Springs {
            let d = th.len();
            let mut e = 0.0;
            for i in 0..d {
                e -= self.b[i] * th[i];
                for k in 0..d {
                    e += 0.5 * th[i] * self.a[i * d + k] * th[k];
                }
            }
            return e;
        }
        let mut w = Work::default();
        (0..data.n)
            .map(|r| {
                self.forward(th, data.row(r), &mut w);
                self.out_loss(data.y[r], &mut w)
            })
            .sum()
    }

    /// U = the summed loss plus the prior's |th|^2 / (2 prior^2).
    pub fn energy(&self, th: &[f64], data: &Data) -> f64 {
        let pr = if self.prior > 0.0 { th.iter().map(|v| v * v).sum::<f64>() / (2.0 * self.prior * self.prior) } else { 0.0 };
        self.data_loss(th, data) + pr
    }

    /// Class probabilities (length = classes), or the predicted value (length 1).
    pub fn predict(&self, th: &[f64], x: &[f64], out: &mut Vec<f64>, w: &mut Work) {
        self.forward(th, x, w);
        out.clear();
        if self.classes == 0 {
            out.push(w.z[0]);
        } else if self.classes == 2 {
            let q = sigmoid(w.z[0]);
            out.push(1.0 - q);
            out.push(q);
        } else {
            let mx = w.z.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
            let s: f64 = w.z.iter().map(|v| (v - mx).exp()).sum();
            out.extend(w.z.iter().map(|v| (v - mx).exp() / s));
        }
    }

    /// When U is exactly quadratic, U = th.A.th/2 - c.th + const: returns (A, c). Springs, and least squares
    /// (A = X~^T X~ / noise^2 + I / prior^2, c = X~^T y / noise^2, X~ = the features with a column of ones).
    pub fn gaussian(&self, data: &Data) -> Option<(Vec<f64>, Vec<f64>)> {
        match self.kind {
            Kind::Springs => Some((self.a.clone(), self.b.clone())),
            Kind::LeastSquares => {
                let (p, d) = (data.p, data.p + 1);
                let s2 = self.noise * self.noise;
                let mut a = vec![0.0; d * d];
                let mut c = vec![0.0; d];
                let mut xt = vec![1.0; d];
                for r in 0..data.n {
                    xt[..p].copy_from_slice(data.row(r));
                    for i in 0..d {
                        c[i] += xt[i] * data.y[r] / s2;
                        for k in 0..d {
                            a[i * d + k] += xt[i] * xt[k] / s2;
                        }
                    }
                }
                if self.prior > 0.0 {
                    for i in 0..d {
                        a[i * d + i] += 1.0 / (self.prior * self.prior);
                    }
                }
                Some((a, c))
            }
            _ => None,
        }
    }

    pub fn label(&self) -> String {
        match self.kind {
            Kind::Springs => "springs".into(),
            Kind::LeastSquares => "least squares".into(),
            Kind::Logistic => format!("logistic, {} classes", self.classes),
            Kind::Net => {
                let out = if self.classes == 0 { "a value".to_string() } else { format!("{} classes", self.classes) };
                format!("net, {} hidden, {}", self.hidden, out)
            }
        }
    }

    fn save(&self, m: &mut Model) {
        let kind = match self.kind {
            Kind::Springs => 0.0,
            Kind::LeastSquares => 1.0,
            Kind::Logistic => 2.0,
            Kind::Net => 3.0,
        };
        let mut v = vec![kind, self.classes as f64, self.hidden as f64, self.noise, self.prior, self.init];
        v.extend_from_slice(&self.a);
        v.extend_from_slice(&self.b);
        m.notes.insert(LOSS.to_string(), (v, self.springs.clone()));
    }

    fn load(m: &Model) -> Option<Problem> {
        let (v, words) = m.notes.get(LOSS)?;
        let kind = [Kind::Springs, Kind::LeastSquares, Kind::Logistic, Kind::Net][v[0] as usize];
        let d = words.len();
        Some(Problem {
            kind,
            classes: v[1] as usize,
            hidden: v[2] as usize,
            noise: v[3],
            prior: v[4],
            init: v[5],
            a: v[6..6 + d * d].to_vec(),
            b: v[6 + d * d..6 + d * d + d].to_vec(),
            springs: words.clone(),
        })
    }
}

// ---------------------------------------------------------------------------------------------------------
// The Settling loop
// ---------------------------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Method {
    Langevin,
    Adam,
}

#[derive(Clone, Debug)]
pub struct Opts {
    pub steps: usize,
    pub step: f64,
    /// The step falls geometrically from `step` to this over the run.
    pub step_to: Option<f64>,
    pub temp: f64,
    /// The temperature falls along a straight line from `temp` to this over the run.
    pub cool_to: Option<f64>,
    /// Rows per gradient; None or >= n means every row (full batch).
    pub batch: Option<usize>,
    pub walkers: usize,
    pub seed: u64,
    pub burn: usize,
    pub every: usize,
    /// Snapshots kept per walker for predictions.
    pub keep: usize,
    pub method: Method,
}

impl Opts {
    pub fn new(steps: usize, step: f64, temp: f64, seed: u64) -> Opts {
        Opts { steps, step, step_to: None, temp, cool_to: None, batch: None, walkers: 1, seed, burn: steps / 10, every: 1, keep: 0, method: Method::Langevin }
    }
    pub fn temp_at(&self, s: usize) -> f64 {
        match self.cool_to {
            None => self.temp,
            Some(t1) => self.temp + (t1 - self.temp) * (s - 1) as f64 / (self.steps.max(2) - 1) as f64,
        }
    }
    pub fn step_at(&self, s: usize) -> f64 {
        match self.step_to {
            None => self.step,
            Some(h1) => self.step * (h1 / self.step).powf((s - 1) as f64 / (self.steps.max(2) - 1) as f64),
        }
    }
}

pub struct Descended {
    pub d: usize,
    pub mean: Vec<f64>,
    pub var: Vec<f64>,
    /// d x d when d <= COV_MAX, else empty.
    pub cov: Vec<f64>,
    /// Batch-means standard error of each mean (20 batches).
    pub se: Vec<f64>,
    pub kept: usize,
    /// (walker, step) at which a walker left the finite range.
    pub blew: Option<(usize, usize)>,
    /// Last position of each walker.
    pub last: Vec<Vec<f64>>,
    pub samples: Vec<Vec<f64>>,
    pub secs: f64,
    /// Gradient evaluations, in rows (the budget).
    pub rows_seen: u64,
}

/// What the callback sees every `report` steps: the step, the walker, its position and the batch loss.
pub struct Tick<'a> {
    pub step: usize,
    pub walker: usize,
    pub th: &'a [f64],
    /// Mean loss per row of the last batch.
    pub batch_loss: f64,
    pub temp: f64,
}

/// Run every walker. `report` > 0 calls `cb` every `report` steps (and at the last step).
pub fn descend(pb: &Problem, data: &Data, o: &Opts, report: usize, cb: &mut dyn FnMut(&Tick)) -> Descended {
    let t0 = Instant::now();
    let d = pb.nparams(data.p);
    let n = data.n;
    let full = pb.kind == Kind::Springs || o.batch.map_or(true, |b| b >= n);
    let bsz = if full { n } else { o.batch.unwrap().max(1) };
    let scale = if full { 1.0 } else { n as f64 / bsz as f64 };
    let want_cov = d <= COV_MAX;
    let per_walker = o.steps.saturating_sub(o.burn) / o.every.max(1);
    let kept_total = o.walkers * per_walker;
    let nb = 20usize;
    let bsize = (kept_total / nb).max(1);
    let snap_every = if o.keep == 0 { usize::MAX } else { (per_walker / o.keep).max(1) };
    let mut ksum = vec![0.0; d];
    let mut ksq = if want_cov { vec![0.0; d * d] } else { vec![0.0; d] };
    let mut bsum = vec![0.0; d];
    let mut bmeans: Vec<Vec<f64>> = Vec::new();
    let mut inb = 0usize;
    let mut kept = 0usize;
    let mut blew = None;
    let mut lasts = Vec::new();
    let mut samples = Vec::new();
    let mut rows_seen = 0u64;
    let mut g = vec![0.0; d];
    let mut wk = Work::default();
    for wi in 0..o.walkers {
        let mut rng = if wi == 0 { Rng::new(o.seed) } else { Rng::new(crate::mnist::mix(o.seed, 0xD35C, wi as u64)) };
        let mut brng = Rng::new(crate::mnist::mix(o.seed, 0xBA7C, wi as u64));
        let mut th = vec![0.0; d];
        if pb.init > 0.0 {
            let mut irng = Rng::new(crate::mnist::mix(o.seed, 0x1417, wi as u64));
            th.iter_mut().for_each(|v| *v = pb.init * irng.normal());
        }
        let (mut m1, mut m2) = if o.method == Method::Adam { (vec![0.0; d], vec![0.0; d]) } else { (vec![], vec![]) };
        let mut order: Vec<usize> = (0..n).collect();
        let mut cursor = n;
        let mut rows: Vec<usize> = Vec::with_capacity(bsz);
        let mut wkept = 0usize;
        for s in 1..=o.steps {
            let h = o.step_at(s);
            let temp = o.temp_at(s).max(0.0);
            let batch_loss = if full {
                rows_seen += n as u64;
                pb.grad(&th, data, None, 1.0, &mut g, &mut wk) / n.max(1) as f64
            } else {
                rows.clear();
                while rows.len() < bsz {
                    if cursor >= n {
                        for j in (1..n).rev() {
                            let r = brng.below(j + 1);
                            order.swap(j, r);
                        }
                        cursor = 0;
                    }
                    rows.push(order[cursor]);
                    cursor += 1;
                }
                rows_seen += bsz as u64;
                pb.grad(&th, data, Some(&rows), scale, &mut g, &mut wk) / bsz as f64
            };
            let mut bad = false;
            match o.method {
                Method::Langevin => {
                    let sig = (2.0 * temp * h).sqrt();
                    for i in 0..d {
                        th[i] -= h * g[i];
                        if sig > 0.0 {
                            th[i] += sig * rng.normal();
                        }
                        if !th[i].is_finite() || th[i].abs() > BLOWUP {
                            bad = true;
                        }
                    }
                }
                Method::Adam => {
                    let (b1, b2, eps) = (0.9f64, 0.999f64, 1e-8f64);
                    let (c1, c2) = (1.0 - b1.powi(s as i32), 1.0 - b2.powi(s as i32));
                    for i in 0..d {
                        m1[i] = b1 * m1[i] + (1.0 - b1) * g[i];
                        m2[i] = b2 * m2[i] + (1.0 - b2) * g[i] * g[i];
                        th[i] -= h * (m1[i] / c1) / ((m2[i] / c2).sqrt() + eps);
                        if !th[i].is_finite() || th[i].abs() > BLOWUP {
                            bad = true;
                        }
                    }
                }
            }
            if bad {
                blew = Some((wi, s));
                break;
            }
            if report > 0 && (s % report == 0 || s == o.steps) {
                cb(&Tick { step: s, walker: wi, th: &th, batch_loss, temp });
            }
            if s > o.burn && (s - o.burn) % o.every.max(1) == 0 {
                kept += 1;
                wkept += 1;
                for i in 0..d {
                    ksum[i] += th[i];
                    bsum[i] += th[i];
                }
                if want_cov {
                    for i in 0..d {
                        let xi = th[i];
                        let row = &mut ksq[i * d..(i + 1) * d];
                        for k in i..d {
                            row[k] += xi * th[k];
                        }
                    }
                } else {
                    for i in 0..d {
                        ksq[i] += th[i] * th[i];
                    }
                }
                inb += 1;
                if inb == bsize && bmeans.len() < nb {
                    bmeans.push(bsum.iter().map(|v| v / bsize as f64).collect());
                    bsum.iter_mut().for_each(|v| *v = 0.0);
                    inb = 0;
                }
                if wkept % snap_every == 0 && samples.len() < (wi + 1) * o.keep {
                    samples.push(th.clone());
                }
            }
        }
        lasts.push(th);
        if blew.is_some() {
            break;
        }
    }
    let nk = kept.max(1) as f64;
    let mean: Vec<f64> = ksum.iter().map(|v| v / nk).collect();
    let mut cov = Vec::new();
    let var: Vec<f64>;
    if want_cov {
        cov = vec![0.0; d * d];
        for i in 0..d {
            for k in i..d {
                let c = ksq[i * d + k] / nk - mean[i] * mean[k];
                cov[i * d + k] = c;
                cov[k * d + i] = c;
            }
        }
        var = (0..d).map(|i| cov[i * d + i]).collect();
    } else {
        var = (0..d).map(|i| ksq[i] / nk - mean[i] * mean[i]).collect();
    }
    let se: Vec<f64> = (0..d)
        .map(|i| {
            let m = bmeans.len();
            if m < 2 {
                return f64::NAN;
            }
            let bm: f64 = bmeans.iter().map(|v| v[i]).sum::<f64>() / m as f64;
            let vv: f64 = bmeans.iter().map(|v| (v[i] - bm) * (v[i] - bm)).sum::<f64>() / (m - 1) as f64;
            (vv / m as f64).sqrt()
        })
        .collect();
    Descended { d, mean, var, cov, se, kept, blew, last: lasts, samples, secs: t0.elapsed().as_secs_f64(), rows_seen }
}

// ---------------------------------------------------------------------------------------------------------
// Predictions of a cloud, and how good they are
// ---------------------------------------------------------------------------------------------------------

pub struct Pred {
    /// n x k: averaged class probabilities, or (mean, sd) of the predicted value (k = 2).
    pub out: Vec<f64>,
    pub k: usize,
    /// Mutual information between the prediction and the parameters (nats), per row: H[mean p] - mean H[p].
    /// Zero for one parameter vector: a point has no doubt about itself.
    pub mi: Vec<f64>,
}

fn threads() -> usize {
    std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(16)
}

/// Average the predictions of `samples` (each a parameter vector) over every row of `data`.
pub fn predictive(pb: &Problem, samples: &[&[f64]], data: &Data) -> Pred {
    let k = if pb.classes == 0 { 2 } else { pb.classes };
    let mut out = vec![0.0; data.n * k];
    let mut mi = vec![0.0; data.n];
    let chunk = data.n.div_ceil(threads()).max(1);
    std::thread::scope(|sc| {
        for (ci, (oc, mc)) in out.chunks_mut(chunk * k).zip(mi.chunks_mut(chunk)).enumerate() {
            sc.spawn(move || {
                let mut w = Work::default();
                let mut q = Vec::new();
                let s_n = samples.len() as f64;
                for (j, (orow, mrow)) in oc.chunks_mut(k).zip(mc.iter_mut()).enumerate() {
                    let r = ci * chunk + j;
                    let x = data.row(r);
                    let mut h_avg = 0.0;
                    let (mut f1, mut f2) = (0.0, 0.0);
                    for th in samples {
                        pb.predict(th, x, &mut q, &mut w);
                        if pb.classes == 0 {
                            f1 += q[0];
                            f2 += q[0] * q[0];
                        } else {
                            for c in 0..k {
                                orow[c] += q[c] / s_n;
                                if q[c] > 0.0 {
                                    h_avg -= q[c] * q[c].ln() / s_n;
                                }
                            }
                        }
                    }
                    if pb.classes == 0 {
                        let m = f1 / s_n;
                        let v = (f2 / s_n - m * m).max(0.0);
                        orow[0] = m;
                        orow[1] = (v + pb.noise * pb.noise).sqrt();
                        *mrow = 0.5 * (1.0 + v / (pb.noise * pb.noise)).ln();
                    } else {
                        let h_bar: f64 = orow.iter().filter(|&&v| v > 0.0).map(|&v| -v * v.ln()).sum();
                        *mrow = (h_bar - h_avg).max(0.0);
                    }
                }
            });
        }
    });
    Pred { out, k, mi }
}

#[derive(Clone, Debug, Default)]
pub struct Metrics {
    pub n: usize,
    /// Classes: fraction right; values: root mean squared error.
    pub accuracy: f64,
    pub rmse: f64,
    /// Mean negative log-likelihood per row (nats).
    pub nll: f64,
    /// Expected calibration error, 15 equal-width confidence bins.
    pub ece: f64,
    pub conf: f64,
    /// Values: fraction of targets inside the predictive 95% interval.
    pub coverage: f64,
    pub mi: f64,
}

pub fn metrics(pb: &Problem, pr: &Pred, data: &Data) -> Metrics {
    let n = data.n;
    let mut m = Metrics { n, mi: pr.mi.iter().sum::<f64>() / n.max(1) as f64, ..Default::default() };
    if pb.classes == 0 {
        let (mut se, mut nll, mut inside) = (0.0, 0.0, 0usize);
        for r in 0..n {
            let (mu, sd) = (pr.out[2 * r], pr.out[2 * r + 1]);
            let e = data.y[r] - mu;
            se += e * e;
            nll += 0.5 * (e / sd).powi(2) + sd.ln() + 0.5 * (2.0 * std::f64::consts::PI).ln();
            if e.abs() <= 1.959964 * sd {
                inside += 1;
            }
        }
        m.rmse = (se / n as f64).sqrt();
        m.nll = nll / n as f64;
        m.coverage = inside as f64 / n as f64;
        return m;
    }
    let k = pr.k;
    let nbins = 15;
    let (mut bc, mut ba, mut bn) = (vec![0.0; nbins], vec![0.0; nbins], vec![0usize; nbins]);
    let (mut right, mut nll, mut csum) = (0usize, 0.0, 0.0);
    for r in 0..n {
        let row = &pr.out[r * k..(r + 1) * k];
        let best = (0..k).fold(0, |b, c| if row[c] > row[b] { c } else { b });
        let yc = data.y[r] as usize;
        let ok = best == yc;
        if ok {
            right += 1;
        }
        nll -= row[yc].max(1e-300).ln();
        let conf = row[best];
        csum += conf;
        let bi = ((conf * nbins as f64) as usize).min(nbins - 1);
        bc[bi] += conf;
        ba[bi] += if ok { 1.0 } else { 0.0 };
        bn[bi] += 1;
    }
    m.accuracy = right as f64 / n as f64;
    m.nll = nll / n as f64;
    m.conf = csum / n as f64;
    m.ece = (0..nbins).map(|b| if bn[b] > 0 { (ba[b] - bc[b]).abs() / n as f64 } else { 0.0 }).sum();
    m
}

// ---------------------------------------------------------------------------------------------------------
// The last descent of a model, kept in the notes
// ---------------------------------------------------------------------------------------------------------

struct Last {
    steps: usize,
    h: f64,
    temp: f64,
    temp_end: f64,
    batch: bool,
    method: Method,
    settled: bool,
    walkers: usize,
    d: usize,
    mean: Vec<f64>,
    var: Vec<f64>,
    se: Vec<f64>,
    cov: Vec<f64>,
    last0: Vec<f64>,
    samples: Vec<Vec<f64>>,
}

impl Last {
    fn save(&self, m: &mut Model) {
        let mut v = vec![
            self.steps as f64,
            self.h,
            self.temp,
            self.temp_end,
            if self.batch { 1.0 } else { 0.0 },
            if self.method == Method::Adam { 1.0 } else { 0.0 },
            if self.settled { 1.0 } else { 0.0 },
            self.walkers as f64,
            self.d as f64,
            self.cov.len() as f64,
            self.samples.len() as f64,
        ];
        for part in [&self.mean, &self.var, &self.se, &self.cov, &self.last0] {
            v.extend_from_slice(part);
        }
        for s in &self.samples {
            v.extend_from_slice(s);
        }
        m.notes.insert(LAST.to_string(), (v, Vec::new()));
    }
    fn load(m: &Model, what: &str, ln: usize) -> Result<Last, SettleError> {
        let v = match m.notes.get(LAST) {
            Some((v, _)) => v,
            None => return err(ln, format!("{} needs a descend first", what)),
        };
        let (d, nc, ns) = (v[8] as usize, v[9] as usize, v[10] as usize);
        let mut at = 11;
        let mut take = |k: usize| {
            let s = v[at..at + k].to_vec();
            at += k;
            s
        };
        let (mean, var, se, cov, last0) = (take(d), take(d), take(d), take(nc), take(d));
        let samples = (0..ns).map(|_| take(d)).collect();
        let l = Last {
            steps: v[0] as usize,
            h: v[1],
            temp: v[2],
            temp_end: v[3],
            batch: v[4] > 0.5,
            method: if v[5] > 0.5 { Method::Adam } else { Method::Langevin },
            settled: v[6] > 0.5,
            walkers: v[7] as usize,
            d,
            mean,
            var,
            se,
            cov,
            last0,
            samples,
        };
        if !l.settled {
            return err(ln, format!("{}: the last descend did not settle, so there is nothing to report", what));
        }
        Ok(l)
    }
}

// ---------------------------------------------------------------------------------------------------------
// Statements
// ---------------------------------------------------------------------------------------------------------

fn load_data(rest: &[Tok], ln: usize, ctx: &Ctx, verb: &str) -> Result<Data, SettleError> {
    match rest {
        [Tok::Str(path), more @ ..] => {
            let kv = kwargs(more, ln)?;
            only(&kv, &[], verb, ln)?;
            let p = ctx.path(path);
            let txt = std::fs::read_to_string(&p).map_err(|e| SettleError(format!("line {}: cannot read {}: {}", ln, path, e)))?;
            Data::parse_csv(&txt).map_err(|e| SettleError(format!("line {}: {}: {}", ln, path, e)))
        }
        [Tok::Sym(s), more @ ..] if s == "mnist" => {
            let kv = kwargs(more, ln)?;
            only(&kv, &["dir", "split", "from", "rows"], verb, ln)?;
            let dir = match kw(&kv, "dir") {
                Some(v) => text(v, ln)?,
                None => return err(ln, format!("{} :mnist needs `dir:`, the folder of the four unzipped IDX files", verb)),
            };
            let split = match kw(&kv, "split") {
                Some(v) => text(v, ln)?,
                None => (if verb == "test" { "test" } else { "train" }).to_string(),
            };
            let from = match kw(&kv, "from") {
                Some(v) => num(v, ln)? as usize,
                None => 0,
            };
            let rows = match kw(&kv, "rows") {
                Some(v) => num(v, ln)? as usize,
                None => usize::MAX,
            };
            let dir = ctx.path(&dir).to_string_lossy().to_string();
            Data::mnist(&dir, &split, from, rows).map_err(|e| SettleError(format!("line {}: {}", ln, e)))
        }
        _ => err(ln, format!("{} needs a \"file.csv\" or :mnist, dir: \"...\"", verb)),
    }
}

fn data_stmt(m: &mut Model, rest: &[Tok], ln: usize, ctx: &mut Ctx, verb: &str) -> Result<(), SettleError> {
    let d = load_data(rest, ln, ctx, verb)?;
    if let Some(other) = Data::load(m, if verb == "data" { TEST } else { DATA }) {
        if other.p != d.p {
            return err(ln, format!("{} has {} features but the {} rows have {}", verb, d.p, if verb == "data" { "test" } else { "data" }, other.p));
        }
    }
    let what = if verb == "data" { "examples" } else { "test examples" };
    ctx.say(format!("{}: {} {} of {} feature{}", verb, d.n, what, d.p, if d.p == 1 { "" } else { "s" }));
    d.save(m, if verb == "data" { DATA } else { TEST });
    Ok(())
}

fn loss_stmt(m: &mut Model, rest: &[Tok], ln: usize) -> Result<(), SettleError> {
    let (kind, more) = match rest {
        [Tok::Sym(s), more @ ..] => (s.as_str(), more),
        _ => return err(ln, "loss needs a piece: :springs, :least_squares, :logistic or :net"),
    };
    let kv = kwargs(more, ln)?;
    let get = |k: &str, dflt: f64| -> Result<f64, SettleError> {
        match kw(&kv, k) {
            Some(v) => num(v, ln),
            None => Ok(dflt),
        }
    };
    let prior = get("prior", 10.0)?;
    if prior < 0.0 {
        return err(ln, "prior is the standard deviation of the parameters' prior; it cannot be below zero (0 means none)");
    }
    let pb = match kind {
        "springs" => {
            only(&kv, &[], "loss :springs", ln)?;
            let sp = Springs::load(m);
            if sp.is_empty() {
                return err(ln, "loss :springs needs numbers and springs; declare them with: number :x");
            }
            Problem::springs(&sp)
        }
        "least_squares" => {
            only(&kv, &["noise", "prior"], "loss :least_squares", ln)?;
            let noise = get("noise", 1.0)?;
            if noise <= 0.0 {
                return err(ln, "noise must be above zero");
            }
            Problem::least_squares(noise, prior)
        }
        "logistic" => {
            only(&kv, &["classes", "prior"], "loss :logistic", ln)?;
            let k = get("classes", 0.0)? as usize;
            if kw(&kv, "classes").is_some() && k < 2 {
                return err(ln, "classes must be at least 2");
            }
            Problem::logistic(k, prior)
        }
        "net" => {
            only(&kv, &["hidden", "classes", "noise", "prior", "init"], "loss :net", ln)?;
            let hidden = get("hidden", 0.0)? as usize;
            if hidden == 0 {
                return err(ln, "loss :net needs `hidden:`, the number of hidden things");
            }
            let k = get("classes", 0.0)? as usize;
            if kw(&kv, "classes").is_some() && k < 2 {
                return err(ln, "classes must be at least 2");
            }
            Problem::net(hidden, k, get("noise", 1.0)?, prior, get("init", 0.5)?)
        }
        other => return err(ln, format!("no loss piece :{} (the pieces: :springs, :least_squares, :logistic, :net)", other)),
    };
    pb.save(m);
    Ok(())
}

/// Classes from the targets when the loss did not say: integers from 0, at least 2.
fn settle_classes(pb: &mut Problem, data: &Data, ln: usize) -> Result<(), SettleError> {
    let wants_classes = pb.kind == Kind::Logistic || (pb.kind == Kind::Net && pb.classes > 0);
    if !wants_classes {
        return Ok(());
    }
    let mut top = 0usize;
    for &y in &data.y {
        if y < 0.0 || y.fract() != 0.0 {
            return err(ln, format!("a target of {} is not a class; classes are whole numbers from 0", y));
        }
        top = top.max(y as usize);
    }
    if pb.classes == 0 {
        pb.classes = (top + 1).max(2);
    } else if top >= pb.classes {
        return err(ln, format!("a target of {} is not one of the {} classes 0 to {}", top, pb.classes, pb.classes - 1));
    }
    Ok(())
}

fn descend_stmt(m: &mut Model, steps: usize, rest: &[Tok], ln: usize, ctx: &mut Ctx) -> Result<(), SettleError> {
    let kv = kwargs(rest, ln)?;
    only(&kv, &["step", "step_to", "temperature", "cool_to", "batch", "walkers", "seed", "burn", "every", "keep", "method"], "descend", ln)?;
    let mut pb = match Problem::load(m) {
        Some(p) => p,
        None => return err(ln, "descend needs a loss; declare one in the model, like: loss :least_squares"),
    };
    let data = if pb.kind == Kind::Springs {
        Data::default()
    } else {
        match Data::load(m, DATA) {
            Some(d) => d,
            None => return err(ln, "descend needs data for this loss; declare it in the model, like: data \"points.csv\""),
        }
    };
    settle_classes(&mut pb, &data, ln)?;
    let num_or = |k: &str, d: f64| -> Result<f64, SettleError> {
        match kw(&kv, k) {
            Some(v) => num(v, ln),
            None => Ok(d),
        }
    };
    let mut o = Opts::new(steps, num_or("step", 0.01)?, num_or("temperature", 0.0)?, num_or("seed", 1.0)? as u64);
    if o.step <= 0.0 {
        return err(ln, "step must be above zero");
    }
    if o.temp < 0.0 {
        return err(ln, "temperature cannot be below zero (zero means no shaking: plain gradient descent)");
    }
    if steps < 20 {
        return err(ln, "descend needs at least 20 steps");
    }
    if let Some(v) = kw(&kv, "cool_to") {
        let t1 = num(v, ln)?;
        if t1 < 0.0 {
            return err(ln, "cool_to cannot be below zero");
        }
        o.cool_to = Some(t1);
    }
    if let Some(v) = kw(&kv, "step_to") {
        let h1 = num(v, ln)?;
        if h1 <= 0.0 {
            return err(ln, "step_to must be above zero");
        }
        o.step_to = Some(h1);
    }
    if let Some(v) = kw(&kv, "batch") {
        let b = num(v, ln)? as usize;
        if b == 0 {
            return err(ln, "batch must be at least 1");
        }
        o.batch = Some(b);
    }
    o.walkers = num_or("walkers", 1.0)?.max(1.0) as usize;
    o.every = num_or("every", 1.0)?.max(1.0) as usize;
    o.burn = (num_or("burn", (steps / 10) as f64)? as usize).min(steps - 20);
    o.keep = num_or("keep", if pb.kind == Kind::Springs || pb.kind == Kind::LeastSquares { 0.0 } else { 50.0 })? as usize;
    if let Some(v) = kw(&kv, "method") {
        o.method = match v {
            Tok::Sym(s) if s == "langevin" => Method::Langevin,
            Tok::Sym(s) if s == "adam" => Method::Adam,
            _ => return err(ln, "method is :langevin (Settling, the default) or :adam"),
        };
    }
    if o.method == Method::Adam && (o.temp > 0.0 || o.cool_to.map_or(false, |t| t > 0.0)) {
        return err(ln, "refused: Adam has no temperature. Settling with noise is method :langevin; Adam is for temperature 0");
    }
    let d = pb.nparams(data.p);
    let r = descend(&pb, &data, &o, 0, &mut |_| {});
    let temp_end = o.cool_to.unwrap_or(o.temp);
    let settled = match r.blew {
        Some((w, s)) => {
            let hint = match pb.gaussian(&data) {
                Some((a, _)) => {
                    let lam = stiffest(&a, d);
                    format!("; the stiffest direction has stiffness {:.4}, keep step below {:.4}", lam, 2.0 / lam)
                }
                None => "; try a smaller step".to_string(),
            };
            ctx.say(format!("did not settle: walker {} blew up at step {} (step {} is too large){}", w + 1, s, o.step, hint));
            false
        }
        None => {
            let full = pb.kind == Kind::Springs || o.batch.map_or(true, |b| b >= data.n);
            let how = if o.method == Method::Adam {
                "Adam".to_string()
            } else if o.temp == 0.0 && temp_end == 0.0 {
                if full { "gradient descent".into() } else { "stochastic gradient descent".into() }
            } else {
                "Langevin".to_string()
            };
            let temp = if o.cool_to.is_some() { format!("{} cooling to {}", o.temp, temp_end) } else { format!("{}", o.temp) };
            let batch = if full { "every row".to_string() } else { format!("batches of {}", o.batch.unwrap()) };
            let step = match o.step_to {
                Some(h1) => format!("{} falling to {}", o.step, h1),
                None => format!("{}", o.step),
            };
            ctx.say(format!(
                "descended: {} steps of {} parameters ({}), {}, step {}, temperature {}, {}{}; kept {} ({:.1} ms)",
                steps,
                d,
                pb.label(),
                how,
                step,
                temp,
                batch,
                if o.walkers > 1 { format!(", {} walkers", o.walkers) } else { String::new() },
                r.kept,
                1e3 * r.secs
            ));
            let u = pb.energy(&r.last[0], &data);
            if pb.kind == Kind::Springs {
                ctx.say(format!("  energy at the end {:.4}", u));
            } else {
                ctx.say(format!("  loss at the end {:.4} per example, energy {:.4}", pb.data_loss(&r.last[0], &data) / data.n as f64, u));
            }
            true
        }
    };
    Last {
        steps,
        h: o.step,
        temp: o.temp,
        temp_end,
        batch: pb.kind != Kind::Springs && o.batch.map_or(false, |b| b < data.n),
        method: o.method,
        settled,
        walkers: o.walkers,
        d,
        mean: r.mean,
        var: r.var,
        se: r.se,
        cov: r.cov,
        last0: r.last[0].clone(),
        samples: r.samples,
    }
    .save(m);
    Ok(())
}

fn ask_stmt(m: &Model, rest: &[Tok], ln: usize, ctx: &mut Ctx) -> Result<(), SettleError> {
    let l = Last::load(m, "ask", ln)?;
    let pb = Problem::load(m).unwrap();
    let data = if pb.kind == Kind::Springs { Data::default() } else { Data::load(m, DATA).unwrap_or_default() };
    let mut pb = pb;
    settle_classes(&mut pb, &data, ln)?;
    let names = pb.names(data.p);
    let mut want = Vec::new();
    for t in rest {
        match t {
            Tok::Sym(s) => match names.iter().position(|n| n == s) {
                Some(i) => want.push(i),
                None => {
                    let some: Vec<&str> = names.iter().take(4).map(|s| s.as_str()).collect();
                    return err(ln, format!("no parameter :{} (this loss has {} parameters: {}, ...)", s, names.len(), some.join(", ")));
                }
            },
            Tok::Comma => {}
            other => return err(ln, format!("unexpected {:?} in ask (write: ask :w1, :bias)", other)),
        }
    }
    let shown: Vec<usize> = if want.is_empty() { (0..l.d.min(16)).collect() } else { want };
    let gauss = pb.gaussian(&data);
    let exact = gauss.as_ref().and_then(|(a, c)| Some((gauss_solve(a, c, l.d)?, gauss_inverse(a, l.d)?)));
    let hot = l.temp > 0.0 || l.temp_end > 0.0;
    // when U is quadratic and the run had one fixed temperature and every row, remove the step's inflation of the
    // spread (as numbers' `spread` does), so the spread column is comparable with the posterior's
    let exact_cov = hot && l.temp_end == l.temp && !l.batch && l.method == Method::Langevin && !l.cov.is_empty() && gauss.is_some();
    let corrected: Option<Vec<f64>> = if exact_cov {
        let a = &gauss.as_ref().unwrap().0;
        Some(inverse_from_spread(&l.cov, a, l.d, l.h, l.temp).iter().map(|v| v * l.temp).collect())
    } else {
        None
    };
    for &i in &shown {
        let sd = match &corrected {
            Some(c) => c[i * l.d + i].max(0.0).sqrt(),
            None => l.var[i].max(0.0).sqrt(),
        };
        let mut line = format!("  {:<10} mean {:>10.4} ± {:.4}   spread {:.4}", names[i], l.mean[i], l.se[i], sd);
        if let Some((mu, inv)) = &exact {
            line.push_str(&format!("   posterior mean {:>10.4}  spread {:.4}", mu[i], inv[i * l.d + i].sqrt()));
        }
        ctx.say(line);
    }
    if l.d > shown.len() {
        ctx.say(format!("  ... and {} more parameters", l.d - shown.len()));
    }
    let mean_sd = l.var.iter().map(|v| v.max(0.0).sqrt()).sum::<f64>() / l.d as f64;
    match &exact {
        Some((mu, inv)) => {
            let (mut worst, mut zmax) = (0.0f64, 0.0f64);
            for i in 0..l.d {
                let off = (l.mean[i] - mu[i]).abs();
                worst = worst.max(off);
                zmax = zmax.max(off / l.se[i]);
            }
            let mut msg = if hot {
                format!("ask: {} parameters; largest mean error {:.4} ({:.1} standard errors) against the exact posterior mean", l.d, worst, zmax)
            } else {
                format!("ask: {} parameters; largest error {:.2e} against the exact answer", l.d, worst)
            };
            if let Some(c) = &corrected {
                msg.push_str(&format!(
                    "; spreads are step-corrected, and the cloud's covariance is {:.2}% off the posterior's (temperature {})",
                    100.0 * rel_frob(c, inv),
                    l.temp
                ));
            } else if !hot {
                msg.push_str("; temperature 0, so the cloud has no spread");
            } else if l.batch {
                msg.push_str("; minibatch noise adds to the spread, so the spread is not compared (use every row)");
            }
            ctx.say(msg);
        }
        None => {
            let note = if hot { "the spread of the cloud" } else { "temperature 0: the spread is the path's wander, not a posterior" };
            ctx.say(format!("ask: {} parameters; mean spread {:.4} ({})", l.d, mean_sd, note));
        }
    }
    Ok(())
}

fn score_stmt(m: &Model, ln: usize, ctx: &mut Ctx) -> Result<(), SettleError> {
    let l = Last::load(m, "score", ln)?;
    let mut pb = Problem::load(m).unwrap();
    if pb.kind == Kind::Springs {
        return err(ln, "score needs a loss with data; springs have no examples to predict");
    }
    let train = Data::load(m, DATA).unwrap();
    settle_classes(&mut pb, &train, ln)?;
    let (data, which) = match Data::load(m, TEST) {
        Some(t) => (t, "test"),
        None => (train, "training"),
    };
    if pb.classes > 0 {
        settle_classes(&mut pb.clone(), &data, ln)?;
    }
    let mut rows: Vec<(&str, Metrics)> = vec![
        ("last", metrics(&pb, &predictive(&pb, &[&l.last0], &data), &data)),
        ("mean", metrics(&pb, &predictive(&pb, &[&l.mean], &data), &data)),
    ];
    if !l.samples.is_empty() {
        let refs: Vec<&[f64]> = l.samples.iter().map(|s| s.as_slice()).collect();
        rows.push(("cloud", metrics(&pb, &predictive(&pb, &refs, &data), &data)));
    }
    ctx.say(format!("score on {} {} rows:", data.n, which));
    for (name, mt) in &rows {
        if pb.classes == 0 {
            ctx.say(format!("  {:<6} rmse {:.4}   nll {:.4}   inside the 95% interval {:.1}%", name, mt.rmse, mt.nll, 100.0 * mt.coverage));
        } else {
            ctx.say(format!(
                "  {:<6} accuracy {:.2}%   nll {:.4}   calibration error {:.4}   doubt {:.4}",
                name,
                100.0 * mt.accuracy,
                mt.nll,
                mt.ece,
                mt.mi
            ));
        }
    }
    let note = if l.samples.is_empty() {
        "no snapshots were kept (keep: 0), so there is no cloud row".to_string()
    } else if l.temp == 0.0 && l.temp_end == 0.0 {
        format!("the cloud is {} snapshots of the path at temperature 0, not a posterior", l.samples.len())
    } else {
        format!("the cloud averages {} snapshots' predictions; doubt is what the snapshots disagree on (nats)", l.samples.len())
    };
    ctx.say(format!("score: last = the final position, mean = the averaged parameters, {}", note));
    if l.walkers > 1 && pb.kind == Kind::Net {
        ctx.say(format!(
            "  the {} walkers of a net settle in different valleys (its hidden things can swap places), so averaging their \
             parameters mixes valleys; average predictions, as the cloud row does",
            l.walkers
        ));
    }
    Ok(())
}

pub struct Descend;

impl Ext for Descend {
    fn name(&self) -> &'static str {
        "descend"
    }

    fn statements(&self) -> &'static [&'static str] {
        &[
            "model: data \"points.csv\"   /   data :mnist, dir: \"data\", rows: 1_000",
            "model: test \"held_out.csv\"   /   test :mnist, dir: \"data\", rows: 1_000",
            "model: loss :least_squares, noise: 0.5, prior: 10   /   loss :logistic   /   loss :net, hidden: 8   /   loss :springs",
            "run: descend 20_000, step: 0.01, temperature: 1, cool_to: 0, batch: 100, walkers: 4, seed: 1, method: :adam",
            "run: ask   /   ask :w1, :bias   /   score",
        ]
    }

    fn model_stmt(&self, m: &mut Model, t: &[Tok], ln: usize, ctx: &mut Ctx) -> Claim {
        match t {
            [Tok::Ident(k), rest @ ..] if k == "data" || k == "test" => Some(data_stmt(m, rest, ln, ctx, k)),
            [Tok::Ident(k), rest @ ..] if k == "loss" => Some(loss_stmt(m, rest, ln)),
            _ => None,
        }
    }

    fn run_stmt(&self, m: &mut Model, _st: &mut State, t: &[Tok], ln: usize, ctx: &mut Ctx) -> Claim {
        Some(match t {
            [Tok::Ident(k), Tok::Num(n), rest @ ..] if k == "descend" => descend_stmt(m, *n as usize, rest, ln, ctx),
            [Tok::Ident(k), rest @ ..] if k == "ask" && m.notes.contains_key(LOSS) => ask_stmt(m, rest, ln, ctx),
            [Tok::Ident(k)] if k == "score" => score_stmt(m, ln, ctx),
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interp::Interp;
    use crate::numbers::{walk, Walk};

    fn run(src: &str) -> Vec<String> {
        Interp::default().exec(src).unwrap_or_else(|e| panic!("{}", e))
    }

    /// y = 1.5 x1 - 0.7 x2 + 0.3 + noise 0.5, 40 rows, deterministic.
    fn line_data(n: usize, seed: u64) -> Data {
        let mut rng = Rng::new(seed);
        let mut d = Data { n, p: 2, x: vec![], y: vec![] };
        for _ in 0..n {
            let (a, b) = (rng.normal(), rng.normal());
            d.x.extend([a, b]);
            d.y.push(1.5 * a - 0.7 * b + 0.3 + 0.5 * rng.normal());
        }
        d
    }

    fn fd_check(pb: &Problem, data: &Data, th: &[f64]) -> f64 {
        let d = th.len();
        let mut g = vec![0.0; d];
        let mut w = Work::default();
        pb.grad(th, data, None, 1.0, &mut g, &mut w);
        let mut worst = 0.0f64;
        for i in 0..d {
            let eps = 1e-6;
            let (mut up, mut dn) = (th.to_vec(), th.to_vec());
            up[i] += eps;
            dn[i] -= eps;
            let num = (pb.energy(&up, data) - pb.energy(&dn, data)) / (2.0 * eps);
            worst = worst.max((num - g[i]).abs() / (1.0 + g[i].abs()));
        }
        worst
    }

    #[test]
    fn every_piece_has_the_gradient_of_its_energy() {
        let mut rng = Rng::new(9);
        let reg = line_data(25, 3);
        let mut cls = Data { n: 30, p: 3, x: vec![], y: vec![] };
        for r in 0..30 {
            cls.x.extend([rng.normal(), rng.normal(), rng.normal()]);
            cls.y.push((r % 3) as f64);
        }
        let mut bin = cls.clone();
        bin.y.iter_mut().for_each(|y| *y = if *y > 0.5 { 1.0 } else { 0.0 });
        let cases: Vec<(Problem, &Data)> = vec![
            (Problem::least_squares(0.7, 2.0), &reg),
            (Problem::logistic(2, 3.0), &bin),
            (Problem::logistic(3, 3.0), &cls),
            (Problem::net(4, 3, 1.0, 3.0, 0.5), &cls),
            (Problem::net(4, 2, 1.0, 0.0, 0.5), &bin),
            (Problem::net(5, 0, 0.6, 2.0, 0.5), &reg),
        ];
        for (pb, data) in cases {
            let d = pb.nparams(data.p);
            let th: Vec<f64> = (0..d).map(|_| 0.4 * rng.normal()).collect();
            let worst = fd_check(&pb, data, &th);
            assert!(worst < 1e-6, "{}: gradient off by {}", pb.label(), worst);
            assert_eq!(pb.names(data.p).len(), d, "{}", pb.label());
        }
    }

    #[test]
    fn descend_on_springs_is_drift_bit_for_bit() {
        let a = vec![1.5, -0.5, 0.0, -0.5, 1.25, 0.25, 0.0, 0.25, 2.25];
        let b = vec![2.0, -0.5, 1.0];
        let sp = Springs { names: vec!["x".into(), "y".into(), "z".into()], a: a.clone(), b: b.clone() };
        let pb = Problem::springs(&sp);
        for (steps, dt, temp, seed, burn) in [(50_000usize, 0.05, 0.7, 7u64, 1000usize), (20_000, 0.02, 0.0, 3, 2000)] {
            let w = Walk { steps, dt, temp, seed, burn, want_cov: true, marks: vec![] };
            let r = walk(&a, &b, 3, &w);
            let mut o = Opts::new(steps, dt, temp, seed);
            o.burn = burn;
            let q = descend(&pb, &Data::default(), &o, 0, &mut |_| {});
            assert_eq!(r.mean, q.mean);
            assert_eq!(r.cov, q.cov);
            assert_eq!(r.last, q.last[0]);
            for (p, s) in r.se.iter().zip(&q.se) {
                assert!(p == s || (p.is_nan() && s.is_nan()));
            }
        }
    }

    #[test]
    fn a_program_on_springs_reports_the_same_means_as_drift() {
        let model = "model :bowl do\n  number :x, :y\n  x.springs :y, by: 0.5\n  x.leans_to 2.0, by: 1\n  y.leans_to -1.0, by: 0.5\n  loss :springs\nend";
        let a = run(&format!("{}\nrun :bowl do\n  drift 40_000, step: 0.02, temperature: 1, seed: 5\n  means\nend", model));
        let b = run(&format!("{}\nrun :bowl do\n  descend 40_000, step: 0.02, temperature: 1, seed: 5\n  ask\nend", model));
        let pick = |out: &[String], word: &str| -> Vec<String> {
            out.iter().filter(|l| l.trim_start().starts_with('x') || l.trim_start().starts_with('y')).map(|l| l.split(word).nth(1).unwrap().split("±").next().unwrap().trim().to_string()).collect()
        };
        assert_eq!(pick(&a, "mean "), pick(&b, "mean "), "{:?}\n{:?}", a, b);
    }

    #[test]
    fn temperature_zero_least_squares_is_the_exact_ridge_solution() {
        let data = line_data(40, 1);
        let pb = Problem::least_squares(0.5, 10.0);
        let (a, c) = pb.gaussian(&data).unwrap();
        let exact = gauss_solve(&a, &c, 3).unwrap();
        let lam = stiffest(&a, 3);
        let mut o = Opts::new(20_000, 1.0 / lam, 0.0, 1);
        o.burn = 19_000;
        let r = descend(&pb, &data, &o, 0, &mut |_| {});
        for i in 0..3 {
            assert!((r.last[0][i] - exact[i]).abs() < 1e-10, "{} {}", r.last[0][i], exact[i]);
        }
    }

    #[test]
    fn temperature_zero_logistic_matches_newtons_method() {
        // binary logistic regression with a prior, solved independently by Newton's method
        let mut rng = Rng::new(4);
        let mut data = Data { n: 60, p: 2, x: vec![], y: vec![] };
        for _ in 0..60 {
            let (u, v) = (rng.normal(), rng.normal());
            data.x.extend([u, v]);
            let z = 1.2 * u - 0.8 * v + 0.4;
            data.y.push(if rng.unit() < sigmoid(z) { 1.0 } else { 0.0 });
        }
        let pb = Problem::logistic(2, 3.0);
        let mut th = vec![0.0; 3];
        for _ in 0..50 {
            let mut g = vec![0.0; 3];
            let mut hess = vec![0.0; 9];
            for r in 0..data.n {
                let xt = [data.x[2 * r], data.x[2 * r + 1], 1.0];
                let z: f64 = (0..3).map(|i| th[i] * xt[i]).sum();
                let q = sigmoid(z);
                for i in 0..3 {
                    g[i] += (q - data.y[r]) * xt[i];
                    for k in 0..3 {
                        hess[i * 3 + k] += q * (1.0 - q) * xt[i] * xt[k];
                    }
                }
            }
            for i in 0..3 {
                g[i] += th[i] / 9.0;
                hess[i * 3 + i] += 1.0 / 9.0;
            }
            let dx = gauss_solve(&hess, &g, 3).unwrap();
            for i in 0..3 {
                th[i] -= dx[i];
            }
        }
        let mut o = Opts::new(40_000, 0.05, 0.0, 1);
        o.burn = 39_000;
        let r = descend(&pb, &data, &o, 0, &mut |_| {});
        for i in 0..3 {
            assert!((r.last[0][i] - th[i]).abs() < 1e-8, "{:?} {:?}", r.last[0], th);
        }
    }

    #[test]
    fn the_least_squares_cloud_is_the_bayesian_posterior_and_a_wrong_temperature_is_caught() {
        let data = line_data(40, 2);
        let pb = Problem::least_squares(0.5, 10.0);
        let (a, c) = pb.gaussian(&data).unwrap();
        let mu = gauss_solve(&a, &c, 3).unwrap();
        let post = gauss_inverse(&a, 3).unwrap();
        let lam = stiffest(&a, 3);
        let h = 0.2 / lam;
        let check = |temp: f64| -> (f64, f64) {
            let mut o = Opts::new(1_000_000, h, temp, 11);
            o.burn = 20_000;
            let r = descend(&pb, &data, &o, 0, &mut |_| {});
            let zmax = (0..3).map(|i| (r.mean[i] - mu[i]).abs() / r.se[i]).fold(0.0, f64::max);
            let corrected: Vec<f64> = inverse_from_spread(&r.cov, &a, 3, h, temp).iter().map(|v| v * temp).collect();
            (zmax, rel_frob(&corrected, &post))
        };
        let (z1, e1) = check(1.0);
        assert!(z1 < 5.0, "the mean is {} standard errors off", z1);
        assert!(e1 < 0.05, "the cloud's covariance is {} off the posterior's", e1);
        // negative control: twice the temperature gives twice the posterior covariance
        let (z2, e2) = check(2.0);
        assert!(z2 < 5.0);
        assert!((e2 - 1.0).abs() < 0.1, "at temperature 2 the covariance should be about 100% off, got {}", e2);
    }

    #[test]
    fn a_non_gaussian_posterior_matches_quadrature() {
        // one-feature logistic regression without a bias column is still two parameters (w, bias); the exact
        // posterior mean and spread come from a 400 x 400 grid
        let xs = [-2.0, -1.5, -1.0, -0.5, 0.0, 0.3, 0.6, 1.0, 1.4, 2.0, -0.2, 0.8];
        let ys = [0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 1.0, 1.0, 1.0];
        let data = Data { n: xs.len(), p: 1, x: xs.to_vec(), y: ys.to_vec() };
        let pb = Problem::logistic(2, 2.0);
        let (lo, hi, k) = (-6.0, 8.0, 400usize);
        let du = (hi - lo) / k as f64;
        let (mut z, mut m1, mut m2, mut s1, mut s2) = (0.0, 0.0, 0.0, 0.0, 0.0);
        let mut wmin = f64::INFINITY;
        let mut es = vec![0.0; k * k];
        for i in 0..k {
            for j in 0..k {
                let th = [lo + (i as f64 + 0.5) * du, lo + (j as f64 + 0.5) * du];
                es[i * k + j] = pb.energy(&th, &data);
                wmin = wmin.min(es[i * k + j]);
            }
        }
        for i in 0..k {
            for j in 0..k {
                let th = [lo + (i as f64 + 0.5) * du, lo + (j as f64 + 0.5) * du];
                let w = (-(es[i * k + j] - wmin)).exp();
                z += w;
                m1 += w * th[0];
                m2 += w * th[1];
                s1 += w * th[0] * th[0];
                s2 += w * th[1] * th[1];
            }
        }
        let (mw, mb) = (m1 / z, m2 / z);
        let (sw, sb) = ((s1 / z - mw * mw).sqrt(), (s2 / z - mb * mb).sqrt());
        let mut o = Opts::new(400_000, 0.005, 1.0, 21);
        o.burn = 10_000;
        o.walkers = 4;
        let r = descend(&pb, &data, &o, 0, &mut |_| {});
        assert!((r.mean[0] - mw).abs() < 5.0 * r.se[0] + 0.01 * sw, "w {} vs {} (se {})", r.mean[0], mw, r.se[0]);
        assert!((r.mean[1] - mb).abs() < 5.0 * r.se[1] + 0.01 * sb, "bias {} vs {}", r.mean[1], mb);
        // the step size biases the spread slightly; 6% covers it at h = 0.005
        assert!((r.var[0].sqrt() / sw - 1.0).abs() < 0.06, "spread of w {} vs {}", r.var[0].sqrt(), sw);
        assert!((r.var[1].sqrt() / sb - 1.0).abs() < 0.06, "spread of bias {} vs {}", r.var[1].sqrt(), sb);
    }

    #[test]
    fn adam_and_minibatches_reach_the_same_valley() {
        let data = line_data(200, 5);
        let pb = Problem::least_squares(0.5, 10.0);
        let (a, c) = pb.gaussian(&data).unwrap();
        let exact = gauss_solve(&a, &c, 3).unwrap();
        let mut o = Opts::new(6_000, 0.02, 0.0, 1);
        o.method = Method::Adam;
        o.batch = Some(20);
        o.step_to = Some(1e-4);
        let r = descend(&pb, &data, &o, 0, &mut |_| {});
        for i in 0..3 {
            assert!((r.last[0][i] - exact[i]).abs() < 0.01, "adam {:?} exact {:?}", r.last[0], exact);
        }
        assert_eq!(r.rows_seen, 6_000 * 20);
    }

    #[test]
    fn annealing_crosses_a_net_hill_and_walkers_differ_by_seed() {
        // XOR needs the hidden layer: a linear piece cannot fit it, a 4-hidden net can
        let xs = [0.0, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0, 1.0];
        let ys = [0.0, 1.0, 1.0, 0.0];
        let data = Data { n: 4, p: 2, x: xs.to_vec(), y: ys.to_vec() };
        let lin = Problem::logistic(2, 10.0);
        let mut o = Opts::new(20_000, 0.1, 0.0, 1);
        let r = descend(&lin, &data, &o, 0, &mut |_| {});
        let acc_lin = metrics(&lin, &predictive(&lin, &[&r.last[0]], &data), &data).accuracy;
        assert!(acc_lin <= 0.75, "a linear piece fits XOR? {}", acc_lin);
        let net = Problem::net(4, 2, 1.0, 10.0, 0.5);
        o.temp = 0.05;
        o.cool_to = Some(0.0);
        o.walkers = 3;
        let r = descend(&net, &data, &o, 0, &mut |_| {});
        assert_ne!(r.last[0], r.last[1]);
        let best = r.last.iter().map(|th| metrics(&net, &predictive(&net, &[th], &data), &data).accuracy).fold(0.0, f64::max);
        assert_eq!(best, 1.0);
    }

    #[test]
    fn the_cloud_doubts_more_far_from_the_data() {
        let data = line_data(30, 6);
        let pb = Problem::net(6, 0, 0.5, 2.0, 0.5);
        let mut o = Opts::new(60_000, 0.002, 1.0, 2);
        o.burn = 20_000;
        o.keep = 100;
        let r = descend(&pb, &data, &o, 0, &mut |_| {});
        let refs: Vec<&[f64]> = r.samples.iter().map(|s| s.as_slice()).collect();
        let near = Data { n: 1, p: 2, x: vec![0.1, -0.2], y: vec![0.0] };
        let far = Data { n: 1, p: 2, x: vec![6.0, -6.0], y: vec![0.0] };
        let (pn, pf) = (predictive(&pb, &refs, &near), predictive(&pb, &refs, &far));
        assert!(pf.out[1] > 1.5 * pn.out[1], "predictive sd near {} far {}", pn.out[1], pf.out[1]);
        // a single point has no doubt
        let one = predictive(&pb, &[&r.mean], &far);
        assert!(one.mi[0].abs() < 1e-12 && (one.out[1] - 0.5).abs() < 1e-12);
    }

    #[test]
    fn a_program_fits_a_line_and_scores_it() {
        let dir = std::env::temp_dir().join(format!("settle-descend-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let tr = line_data(40, 7);
        let te = line_data(200, 8);
        let csv = |d: &Data| -> String { (0..d.n).map(|r| format!("{} {} {}\n", d.x[2 * r], d.x[2 * r + 1], d.y[r])).collect() };
        std::fs::write(dir.join("tr.csv"), format!("x1 x2 y\n{}", csv(&tr))).unwrap();
        std::fs::write(dir.join("te.csv"), csv(&te)).unwrap();
        let src = "model :line do\n  data \"tr.csv\"\n  test \"te.csv\"\n  loss :least_squares, noise: 0.5, prior: 10\nend\nrun :line do\n  descend 200_000, step: 0.004, temperature: 1, seed: 1, keep: 200\n  ask\n  score\nend";
        let mut it = Interp::default();
        it.base_dir = dir.clone();
        let out = it.exec(src).unwrap();
        assert!(out[0].starts_with("data: 40 examples of 2 features"), "{:?}", out);
        let ask = out.iter().find(|l| l.starts_with("ask:")).unwrap();
        let rel: f64 = ask.split("covariance is ").nth(1).unwrap().split('%').next().unwrap().parse().unwrap();
        assert!(rel < 5.0, "{}", ask);
        let cloud = out.iter().find(|l| l.trim_start().starts_with("cloud")).unwrap();
        let cov: f64 = cloud.split("interval ").nth(1).unwrap().trim_end_matches('%').parse().unwrap();
        assert!((88.0..99.0).contains(&cov), "{}", cloud);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn errors_name_their_line() {
        let cases = [
            ("model :m do\nend\nrun :m do\n  descend 100\nend", "line 4: descend needs a loss"),
            ("model :m do\n  loss :logistic\nend\nrun :m do\n  descend 100\nend", "line 5: descend needs data"),
            ("model :m do\n  loss :wobble\nend", "line 2: no loss piece :wobble"),
            ("model :m do\n  loss :net\nend", "line 2: loss :net needs `hidden:`"),
            ("model :m do\n  number :x\n  x.leans_to 1, by: 1\n  loss :springs\nend\nrun :m do\n  descend 100, temperature: 1, method: :adam\nend", "line 7: refused: Adam has no temperature"),
            ("model :m do\n  number :x\n  x.leans_to 1, by: 1\n  loss :springs\nend\nrun :m do\n  score\nend", "line 7: score needs a descend first"),
            ("model :m do\n  number :x\n  x.leans_to 1, by: 1\n  loss :springs\nend\nrun :m do\n  descend 100\n  ask :q\nend", "line 8: no parameter :q"),
            ("model :m do\n  data :mnist\nend", "line 2: data :mnist needs `dir:`"),
        ];
        for (src, want) in cases {
            let e = Interp::default().exec(src).err().map(|e| e.0).unwrap_or_default();
            assert!(e.starts_with(want), "{:?} gave {:?}", src, e);
        }
    }

    #[test]
    fn a_step_too_large_is_diagnosed() {
        let src = "model :m do\n  number :x, :y\n  x.springs :y, by: 20\n  x.leans_to 1, by: 1\n  loss :springs\nend\nrun :m do\n  descend 1_000, step: 0.5\nend";
        let out = run(src);
        assert!(out[0].starts_with("did not settle: walker 1 blew up"), "{:?}", out);
        assert!(out[0].contains("keep step below"), "{:?}", out);
    }
}
