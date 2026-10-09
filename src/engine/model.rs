//! ENGINE: the model (things, leans, pulls) and one run's state (the sampler and the anneal schedule).
//! Couplings are stored sparse, so a model can hold a 100x100 grid of pixels as easily as five named things.
//! No parsing, no printing and no keyword names live here: the words floor (`crate::words`) reads programs and
//! formats answers, and finds a thing by name with `Model::need` (in `words::names`).

use crate::engine::rng::Rng;
use std::collections::HashMap;

#[derive(Clone, Debug, Default)]
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

    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
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

/// The seed a run block starts from before any `seed:` is given (`State::new(RUN_SEED)`).
pub const RUN_SEED: u64 = 0x5eed;

/// Above this many stored values a run keeps only per-thing counts, and `ask` refuses.
pub const SAMPLE_BUDGET: usize = 20_000_000;

/// How a sweep updates one free thing. Both rules leave the same distribution stationary (the Boltzmann
/// distribution of the model at the run's temperature); they differ in how fast a chain forgets where it was.
/// Metropolised Gibbs is the default since 2026-10-06 (lane NEWDEFAULTS, the navigator's ruling on
/// DISCOVERIES.md section 3); `update: :gibbs` asks for the old default.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Update {
    /// Gibbs sampling, the p-bit rule: draw the thing afresh, yes with probability (1 + tanh(I / T)) / 2.
    Gibbs,
    /// Metropolised Gibbs: propose the other value and accept it with probability min(1, exp(-2 s I / T)). It
    /// changes a thing at least as often as Gibbs does, which Peskun-orders it ahead of Gibbs for every average
    /// (runs/filmsharp measured it on the film grid; `examples/core_update_measure.rs` on core models: its
    /// yes-rate error is 0.15 to 0.47 times Gibbs's). The default.
    #[default]
    Metro,
}

/// One run of a model.
pub struct State {
    pub held: HashMap<usize, f64>,
    pub temp: f64,
    /// The update rule of every sweep in this run (Metropolised Gibbs unless a statement chose another).
    pub update: Update,
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
        State {
            held: HashMap::new(),
            temp: 1.0,
            update: Update::default(),
            rng: Rng::new(seed),
            samples: Vec::new(),
            yes: Vec::new(),
            n: 0,
            last: Vec::new(),
            best: None,
        }
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

    /// Update every free thing once, in a fresh random order, by the run's update rule (Gibbs, the p-bit rule, by
    /// default; see `Update`).
    pub fn sweep(&mut self, m: &Model, s: &mut [f64], free: &mut [usize], beta: f64) {
        for k in (1..free.len()).rev() {
            let r = self.rng.below(k + 1);
            free.swap(k, r);
        }
        match self.update {
            Update::Gibbs => {
                for &i in free.iter() {
                    s[i] = if (beta * m.input(i, s)).tanh() > self.rng.signed() { 1.0 } else { -1.0 };
                }
            }
            Update::Metro => {
                for &i in free.iter() {
                    let x = beta * m.input(i, s);
                    let p = (-2.0 * s[i] * x).exp();
                    if p >= 1.0 || self.rng.unit() < p {
                        s[i] = -s[i];
                    }
                }
            }
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

    /// Cool from 10x the temperature to 1/20 of it and keep the calmest arrangement visited. The schedule is
    /// `anneal_temperature`.
    pub fn anneal(&mut self, m: &Model, sweeps: usize) -> f64 {
        let (mut s, mut free) = self.start(m);
        let mut best = (s.clone(), m.energy(&s));
        for step in 0..sweeps {
            let temp = anneal_temperature(self.temp, step, sweeps);
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

/// The anneal schedule: the temperature of sweep `step` of `sweeps`, a geometric cooling from 10 x `temp` at the
/// first sweep to 10 x 0.005 = 1/20 x `temp` at the last (one sweep counts as two, so a one-sweep anneal is hot).
///
/// ```
/// use settle::engine::model::anneal_temperature;
/// assert_eq!(anneal_temperature(1.0, 0, 100), 10.0);
/// assert!((anneal_temperature(1.0, 99, 100) - 0.05).abs() < 1e-12);
/// assert!((anneal_temperature(2.0, 99, 100) - 0.1).abs() < 1e-12);
/// ```
pub fn anneal_temperature(temp: f64, step: usize, sweeps: usize) -> f64 {
    temp * 10.0 * 0.005f64.powf(step as f64 / (sweeps.max(2) - 1) as f64)
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
