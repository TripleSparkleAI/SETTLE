//! The model (things, leans, pulls) and one run's state. Couplings are stored sparse, so a model can hold a
//! 100x100 grid of pixels as easily as five named things.

use crate::lex::{err, SettleError};
use crate::rng::Rng;
use std::collections::HashMap;

#[derive(Clone, Default)]
pub struct Model {
    pub names: Vec<String>,
    pub idx: HashMap<String, usize>,
    /// Lean (field) of each thing.
    pub h: Vec<f64>,
    /// Neighbours of each thing with the coupling strength; symmetric, no self entries.
    pub adj: Vec<Vec<(usize, f64)>>,
    /// Scratch space a statement family may keep on a model, by key: numbers and words.
    pub notes: HashMap<String, (Vec<f64>, Vec<String>)>,
}

impl Model {
    pub fn len(&self) -> usize {
        self.names.len()
    }

    pub fn need(&self, name: &str, ln: usize) -> Result<usize, SettleError> {
        match self.idx.get(name) {
            Some(&i) => Ok(i),
            None => err(ln, format!("unknown thing :{} (declare it with: thing :{})", name, name)),
        }
    }

    /// Declare a thing (or return the existing one).
    pub fn add(&mut self, name: &str) -> usize {
        if let Some(&i) = self.idx.get(name) {
            return i;
        }
        let i = self.names.len();
        self.names.push(name.to_string());
        self.idx.insert(name.to_string(), i);
        self.h.push(0.0);
        self.adj.push(Vec::new());
        i
    }

    /// Add `w` to the coupling between two different things (positive pulls, negative pushes).
    pub fn couple(&mut self, i: usize, k: usize, w: f64) {
        assert_ne!(i, k, "a thing cannot pull itself");
        for (a, b) in [(i, k), (k, i)] {
            match self.adj[a].iter_mut().find(|(n, _)| *n == b) {
                Some(e) => e.1 += w,
                None => self.adj[a].push((b, w)),
            }
        }
    }

    pub fn coupling(&self, i: usize, k: usize) -> f64 {
        self.adj[i].iter().find(|(n, _)| *n == k).map(|e| e.1).unwrap_or(0.0)
    }

    /// Field plus the pull of every neighbour on thing `i` in arrangement `s`.
    pub fn input(&self, i: usize, s: &[f64]) -> f64 {
        self.h[i] + self.adj[i].iter().map(|&(k, w)| w * s[k]).sum::<f64>()
    }

    /// Energy of an arrangement: lower means calmer, and calmer arrangements are sampled more often.
    pub fn energy(&self, s: &[f64]) -> f64 {
        let mut e = 0.0;
        for i in 0..s.len() {
            e -= self.h[i] * s[i];
            for &(k, w) in &self.adj[i] {
                if k > i {
                    e -= w * s[i] * s[k];
                }
            }
        }
        e
    }
}

/// Above this many stored values a run keeps only per-thing counts, and `ask` refuses.
pub const SAMPLE_BUDGET: usize = 20_000_000;

/// One run of a model.
pub struct State {
    pub held: HashMap<usize, f64>,
    pub temp: f64,
    pub rng: Rng,
    /// Every kept arrangement, when they fit the budget.
    pub samples: Vec<Vec<f64>>,
    /// How many sampled arrangements had each thing at yes.
    pub yes: Vec<u64>,
    pub n: u64,
    /// The last arrangement visited.
    pub last: Vec<f64>,
    pub best: Option<(Vec<f64>, f64)>,
}

impl State {
    pub fn new(seed: u64) -> Self {
        State { held: HashMap::new(), temp: 1.0, rng: Rng::new(seed), samples: Vec::new(), yes: Vec::new(), n: 0, last: Vec::new(), best: None }
    }

    /// A random starting arrangement with held things in place, and the list of free things.
    pub fn start(&mut self, m: &Model) -> (Vec<f64>, Vec<usize>) {
        let n = m.len();
        let mut s: Vec<f64> = (0..n).map(|_| if self.rng.unit() < 0.5 { -1.0 } else { 1.0 }).collect();
        for (&i, &v) in &self.held {
            s[i] = v;
        }
        let free: Vec<usize> = (0..n).filter(|i| !self.held.contains_key(i)).collect();
        (s, free)
    }

    /// Update every free thing once, in a fresh random order (the p-bit rule, exact Gibbs sampling).
    pub fn sweep(&mut self, m: &Model, s: &mut [f64], free: &mut [usize], beta: f64) {
        for k in (1..free.len()).rev() {
            let r = self.rng.below(k + 1);
            free.swap(k, r);
        }
        for &i in free.iter() {
            s[i] = if (beta * m.input(i, s)).tanh() > self.rng.signed() { 1.0 } else { -1.0 };
        }
    }

    /// Burn in a tenth of `sweeps`, then record one arrangement per sweep.
    pub fn settle(&mut self, m: &Model, sweeps: usize) {
        let (mut s, mut free) = self.start(m);
        let beta = 1.0 / self.temp;
        for _ in 0..(sweeps / 10).max(1) {
            self.sweep(m, &mut s, &mut free, beta);
        }
        let keep = m.len() * sweeps <= SAMPLE_BUDGET;
        self.samples.clear();
        self.yes = vec![0; m.len()];
        self.n = 0;
        for _ in 0..sweeps {
            self.sweep(m, &mut s, &mut free, beta);
            self.record(&s, keep);
        }
        self.last = s;
    }

    pub fn record(&mut self, s: &[f64], keep: bool) {
        for (c, &v) in self.yes.iter_mut().zip(s) {
            if v > 0.0 {
                *c += 1;
            }
        }
        self.n += 1;
        if keep {
            self.samples.push(s.to_vec());
        }
    }

    /// Cool from 10x the temperature to 1/20 of it and keep the calmest arrangement visited.
    pub fn anneal(&mut self, m: &Model, sweeps: usize) -> f64 {
        let (mut s, mut free) = self.start(m);
        let mut best = (s.clone(), m.energy(&s));
        for step in 0..sweeps {
            let temp = self.temp * 10.0 * 0.005f64.powf(step as f64 / (sweeps.max(2) - 1) as f64);
            self.sweep(m, &mut s, &mut free, 1.0 / temp);
            let e = m.energy(&s);
            if e < best.1 {
                best = (s.clone(), e);
            }
        }
        self.last = s;
        let e = best.1;
        self.best = Some(best);
        e
    }

    /// Yes-rate of each thing over the recorded samples.
    pub fn rates(&self) -> Vec<f64> {
        self.yes.iter().map(|&c| c as f64 / self.n.max(1) as f64).collect()
    }
}

/// True yes-rates by enumerating every arrangement of the free things. Tests only, few things.
pub fn exact_rates(m: &Model, st: &State) -> Vec<f64> {
    let n = m.len();
    let free: Vec<usize> = (0..n).filter(|i| !st.held.contains_key(i)).collect();
    assert!(free.len() <= 24, "exact_rates is for small models");
    let mut z = 0.0;
    let mut acc = vec![0.0; n];
    for bits in 0u64..(1u64 << free.len()) {
        let mut s = vec![0.0; n];
        for (&i, &v) in &st.held {
            s[i] = v;
        }
        for (k, &i) in free.iter().enumerate() {
            s[i] = if (bits >> k) & 1 == 1 { 1.0 } else { -1.0 };
        }
        let w = (-m.energy(&s) / st.temp).exp();
        z += w;
        for i in 0..n {
            if s[i] > 0.0 {
                acc[i] += w;
            }
        }
    }
    acc.iter().map(|a| a / z).collect()
}
