//! VALLEYS: measure the shape of a settle landscape itself. How many valleys, how wide, how regular.
//!
//! ```text
//! model :glass do
//!   landscape :random, size: 16, seed: 1   # 16 things, every pair pulled or pushed by a random amount
//! end
//! run :glass do
//!   valleys show: 8                        # exact: visit all 2^16 arrangements, count every valley
//!   survey starts: 2000, sweeps: 50, temperature: 0.05, seed: 1   # sampled: shake from many random starts
//! end
//! ```
//!
//! A VALLEY is an arrangement no single flip can make calmer, together with every arrangement reachable from
//! it by flips that leave the energy unchanged (a flat floor), provided no member of that flat floor has a
//! downhill flip. A flat floor with a way down is a SADDLE, not a valley.
//!
//! `valleys` is exact for up to 24 free things. It walks every arrangement in Gray-code order, keeping each
//! thing's input up to date with one column update per step, and records for each arrangement its steepest
//! downhill flip (ties go to the lowest index). Following those flips from every arrangement gives the exact
//! BASIN of each valley under steepest descent: the share of all 2^n arrangements that roll into it.
//!
//! `survey` is sampled and has no size limit. Each start is a random arrangement, shaken `sweeps` times at
//! `temperature`, then quenched: flip any thing whose flip lowers the energy, in random order, until none does.
//! The quenched arrangement is an exact local minimum; it is counted by identity. The Chao1 estimate
//! V_obs + f1^2 / (2 f2) (f1 valleys seen once, f2 seen twice) guesses how many valleys the starts missed.
//!
//! Landscapes to measure (model statements):
//!   landscape :random, size: 16, seed: 1, scale: 1, field: 0   pulls J_ij ~ scale * N(0,1)/sqrt(n)
//!   landscape :grid, width: 16, height: 16, wrap: :yes        nearest neighbours pull by 1
//!   landscape :ring, size: 12                                 each thing pulls the next by 1
//!   landscape :code, bits: 12, checks: 6, seed: 1, strength: 1   random 3-bit parity checks, each built from
//!       pulls with one helper thing; codewords are the calmest arrangements
//! A `memory` from the memory family is recognised too: survey and valleys label each valley stored, mirror
//! or fake.

use crate::ext::{Claim, Ctx, Ext};
use crate::lex::{err, kw, kwargs, num, only, yes_no, SettleError, Tok};
use crate::memory::code;
use crate::model::{Model, State};
use crate::rng::Rng;
use std::collections::HashMap;

pub struct Valleys;

/// Largest number of free things `valleys` will enumerate (2^24 arrangements, about 150 MB of tables).
pub const MAX_EXACT: usize = 24;

const UNSET: u32 = u32::MAX;
const SADDLE: u32 = u32::MAX - 1;
const VISITING: u32 = u32::MAX - 2;

// ---------------------------------------------------------------------------------------------------------
// Dense view of a model, restricted to its free things.

/// Fields and pulls of the free things as dense arrays, with held things folded into the fields.
pub struct Dense {
    pub n: usize,
    pub h: Vec<f64>,
    pub j: Vec<f64>,
    /// Model index of each free thing.
    pub free: Vec<usize>,
}

impl Dense {
    pub fn of(m: &Model, held: &HashMap<usize, f64>) -> Dense {
        let free: Vec<usize> = (0..m.len()).filter(|i| !held.contains_key(i)).collect();
        let pos: HashMap<usize, usize> = free.iter().enumerate().map(|(a, &i)| (i, a)).collect();
        let n = free.len();
        let mut h = vec![0.0; n];
        let mut j = vec![0.0; n * n];
        for (a, &i) in free.iter().enumerate() {
            h[a] = m.h[i];
            for &(k, w) in &m.adj[i] {
                match pos.get(&k) {
                    Some(&b) => j[a * n + b] = w,
                    None => h[a] += w * held[&k],
                }
            }
        }
        Dense { n, h, j, free }
    }

    fn spin(c: u32, i: usize) -> f64 {
        if (c >> i) & 1 == 1 {
            1.0
        } else {
            -1.0
        }
    }

    /// Energy of the arrangement whose bit i is thing i (1 = yes).
    pub fn energy(&self, c: u32) -> f64 {
        let n = self.n;
        let mut e = 0.0;
        for i in 0..n {
            let si = Self::spin(c, i);
            e -= self.h[i] * si;
            for k in (i + 1)..n {
                e -= self.j[i * n + k] * si * Self::spin(c, k);
            }
        }
        e
    }

    /// Energy change of flipping each thing, written into `d`.
    pub fn deltas(&self, c: u32, d: &mut [f64]) {
        let n = self.n;
        for i in 0..n {
            let mut f = self.h[i];
            for k in 0..n {
                f += self.j[i * n + k] * Self::spin(c, k);
            }
            d[i] = 2.0 * Self::spin(c, i) * f;
        }
    }

    /// A tolerance for "this flip changes nothing", scaled to the size of the pulls.
    pub fn eps(&self) -> f64 {
        let big = self.j.iter().chain(&self.h).fold(1.0f64, |a, &w| a.max(w.abs()));
        1e-9 * big
    }
}

// ---------------------------------------------------------------------------------------------------------
// Exact enumeration.

/// One valley found by enumeration.
#[derive(Clone, Debug)]
pub struct Valley {
    /// The lowest-numbered arrangement in the valley (bit i = free thing i at yes).
    pub rep: u32,
    pub energy: f64,
    /// How many arrangements share its flat floor (1 for a strict minimum).
    pub floor: u64,
    /// How many of all 2^n arrangements roll into it under steepest descent.
    pub basin: u64,
}

/// Every valley of a landscape, with exact basins.
pub struct Census {
    pub n: usize,
    /// Sorted by energy, calmest first.
    pub valleys: Vec<Valley>,
    /// Arrangements whose steepest descent stops on a flat saddle instead of in a valley.
    pub saddle: u64,
}

impl Census {
    pub fn total(&self) -> u64 {
        1u64 << self.n
    }
    /// Index of the valley whose floor holds `c`, if any.
    pub fn find(&self, reps: &HashMap<u32, usize>, c: u32) -> Option<usize> {
        reps.get(&c).copied()
    }
}

/// Steepest-descent pointers and flat-flip flags for every arrangement of a pairwise landscape.
fn flow_dense(d: &Dense, eps: f64) -> (Vec<u32>, Vec<bool>) {
    let n = d.n;
    let total = 1usize << n;
    let mut next = vec![0u32; total];
    let mut flat = vec![false; total];
    let mut s = vec![-1.0f64; n];
    let refresh = |s: &[f64], f: &mut Vec<f64>| {
        for i in 0..n {
            f[i] = d.h[i] + (0..n).map(|k| d.j[i * n + k] * s[k]).sum::<f64>();
        }
    };
    let mut f = vec![0.0; n];
    refresh(&s, &mut f);
    let mut c: u32 = 0;
    for step in 0..total {
        if step > 0 {
            let k = step.trailing_zeros() as usize;
            c ^= 1 << k;
            s[k] = -s[k];
            if step % 65_536 == 0 {
                refresh(&s, &mut f);
            } else {
                let col = 2.0 * s[k];
                for (i, fi) in f.iter_mut().enumerate() {
                    *fi += d.j[i * n + k] * col;
                }
            }
        }
        let (mut best, mut bi, mut z) = (-eps, usize::MAX, false);
        for i in 0..n {
            let delta = 2.0 * s[i] * f[i];
            // a flip counts as steeper only if it beats the best by more than the tolerance, so equal slopes
            // (up to rounding) always go to the lowest index and mirror arrangements flow as mirrors
            if delta < best - if bi == usize::MAX { 0.0 } else { eps } {
                best = delta;
                bi = i;
            }
            if delta.abs() <= eps {
                z = true;
            }
        }
        next[c as usize] = if bi == usize::MAX { c } else { c ^ (1 << bi) };
        flat[c as usize] = z;
    }
    (next, flat)
}

/// Steepest-descent pointers and flat-flip flags for an arbitrary energy table over n bits.
fn flow_table(n: usize, e: &[f64], eps: f64) -> (Vec<u32>, Vec<bool>) {
    let total = 1usize << n;
    let mut next = vec![0u32; total];
    let mut flat = vec![false; total];
    for c in 0..total {
        let (mut best, mut bi, mut z) = (-eps, usize::MAX, false);
        for i in 0..n {
            let delta = e[c ^ (1 << i)] - e[c];
            // a flip counts as steeper only if it beats the best by more than the tolerance, so equal slopes
            // (up to rounding) always go to the lowest index and mirror arrangements flow as mirrors
            if delta < best - if bi == usize::MAX { 0.0 } else { eps } {
                best = delta;
                bi = i;
            }
            if delta.abs() <= eps {
                z = true;
            }
        }
        next[c] = if bi == usize::MAX { c as u32 } else { (c ^ (1 << bi)) as u32 };
        flat[c] = z;
    }
    (next, flat)
}

/// Turn descent pointers into valleys and basins.
fn census_from(
    n: usize,
    next: &[u32],
    flat: &[bool],
    eps: f64,
    deltas: &dyn Fn(u32, &mut [f64]),
    energy: &dyn Fn(u32) -> f64,
) -> Census {
    let total = 1usize << n;
    let mut label = vec![UNSET; total];
    let mut valleys: Vec<Valley> = Vec::new();
    let mut d = vec![0.0; n];
    for c in 0..total {
        if next[c] as usize != c || label[c] != UNSET {
            continue;
        }
        if !flat[c] {
            label[c] = valleys.len() as u32;
            valleys.push(Valley { rep: c as u32, energy: energy(c as u32), floor: 1, basin: 0 });
            continue;
        }
        // a flat floor: gather every floor arrangement joined to c by energy-neutral flips
        let mut stack = vec![c as u32];
        let mut members = Vec::new();
        let mut escape = false;
        label[c] = VISITING;
        while let Some(x) = stack.pop() {
            members.push(x);
            if next[x as usize] != x {
                escape = true;
                continue;
            }
            deltas(x, &mut d);
            for (i, &di) in d.iter().enumerate() {
                if di.abs() <= eps {
                    let y = x ^ (1 << i);
                    if label[y as usize] == UNSET {
                        label[y as usize] = VISITING;
                        stack.push(y);
                    }
                }
            }
        }
        let id = valleys.len() as u32;
        let floor_count = members.iter().filter(|&&x| next[x as usize] == x).count() as u64;
        for &x in &members {
            label[x as usize] = if next[x as usize] != x {
                UNSET
            } else if escape {
                SADDLE
            } else {
                id
            };
        }
        if !escape {
            let rep = *members.iter().min().unwrap();
            valleys.push(Valley { rep, energy: energy(rep), floor: floor_count, basin: 0 });
        }
    }
    let mut path = Vec::new();
    for c in 0..total {
        if label[c] != UNSET {
            continue;
        }
        let mut x = c;
        while label[x] == UNSET {
            path.push(x);
            x = next[x] as usize;
        }
        let l = label[x];
        for &p in &path {
            label[p] = l;
        }
        path.clear();
    }
    let mut saddle = 0u64;
    for &l in &label {
        if l == SADDLE {
            saddle += 1;
        } else {
            valleys[l as usize].basin += 1;
        }
    }
    valleys.sort_by(|a, b| a.energy.partial_cmp(&b.energy).unwrap().then(a.rep.cmp(&b.rep)));
    Census { n, valleys, saddle }
}

/// Every valley of a pairwise landscape with at most `MAX_EXACT` free things.
pub fn enumerate(d: &Dense) -> Census {
    assert!(d.n <= MAX_EXACT, "exact enumeration is for at most {} free things", MAX_EXACT);
    let eps = d.eps();
    let (next, flat) = flow_dense(d, eps);
    census_from(d.n, &next, &flat, eps, &|c, out| d.deltas(c, out), &|c| d.energy(c))
}

/// Every valley of an arbitrary energy table over n bits (used for the shuffled-energy control).
pub fn enumerate_table(n: usize, e: &[f64]) -> Census {
    assert_eq!(e.len(), 1 << n);
    let eps = 1e-12;
    let (next, flat) = flow_table(n, e, eps);
    let dfn = |c: u32, out: &mut [f64]| {
        for (i, o) in out.iter_mut().enumerate() {
            *o = e[(c ^ (1 << i)) as usize] - e[c as usize];
        }
    };
    census_from(n, &next, &flat, eps, &dfn, &|c| e[c as usize])
}

/// The full energy table of a small pairwise landscape.
pub fn energy_table(d: &Dense) -> Vec<f64> {
    (0..(1u32 << d.n)).map(|c| d.energy(c)).collect()
}

// ---------------------------------------------------------------------------------------------------------
// Survey: many random starts, shake, quench.

/// One valley found by the survey.
#[derive(Clone, Debug)]
pub struct Found {
    pub state: Vec<f64>,
    pub energy: f64,
    pub count: u64,
    /// The quench stopped on an arrangement with an energy-neutral flip (a flat floor or a saddle).
    pub flat: bool,
}

pub fn key(s: &[f64]) -> Vec<u64> {
    let mut k = vec![0u64; s.len().div_ceil(64)];
    for (i, &v) in s.iter().enumerate() {
        if v > 0.0 {
            k[i / 64] |= 1 << (i % 64);
        }
    }
    k
}

fn model_eps(m: &Model) -> f64 {
    let big = m.h.iter().chain(m.adj.iter().flatten().map(|e| &e.1)).fold(1.0f64, |a, &w| a.max(w.abs()));
    1e-9 * big
}

/// Flip any free thing whose flip lowers the energy, in random order, until none does. Returns whether the
/// final arrangement has an energy-neutral flip.
pub fn quench(m: &Model, st: &mut State, s: &mut [f64], free: &mut [usize]) -> bool {
    let eps = model_eps(m);
    for _pass in 0..100_000 {
        for k in (1..free.len()).rev() {
            let r = st.rng.below(k + 1);
            free.swap(k, r);
        }
        let mut changed = false;
        for &i in free.iter() {
            if 2.0 * s[i] * m.input(i, s) < -eps {
                s[i] = -s[i];
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    free.iter().any(|&i| (2.0 * s[i] * m.input(i, s)).abs() <= eps)
}

/// Shake from `starts` random arrangements, quench each, and count the valleys by identity.
pub fn survey(m: &Model, st: &mut State, starts: usize, sweeps: usize, temp: f64) -> Vec<Found> {
    let mut seen: HashMap<Vec<u64>, Found> = HashMap::new();
    for _ in 0..starts {
        let (mut s, mut free) = st.start(m);
        for _ in 0..sweeps {
            st.sweep(m, &mut s, &mut free, 1.0 / temp);
        }
        let flat = quench(m, st, &mut s, &mut free);
        let e = seen.entry(key(&s)).or_insert_with(|| Found { state: s.clone(), energy: m.energy(&s), count: 0, flat });
        e.count += 1;
    }
    let mut v: Vec<Found> = seen.into_values().collect();
    v.sort_by(|a, b| b.count.cmp(&a.count).then(a.energy.partial_cmp(&b.energy).unwrap()));
    v
}

/// Chao1 estimate of the total number of valleys from the counts of a survey.
pub fn chao1(found: &[Found]) -> f64 {
    let f1 = found.iter().filter(|f| f.count == 1).count() as f64;
    let f2 = found.iter().filter(|f| f.count == 2).count() as f64;
    let v = found.len() as f64;
    if f2 > 0.0 {
        v + f1 * f1 / (2.0 * f2)
    } else {
        v + f1 * (f1 - 1.0) / 2.0
    }
}

/// Overlap between two arrangements: +1 identical, -1 mirror images, near 0 unrelated.
pub fn overlap(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(x, y)| x * y).sum::<f64>() / a.len().max(1) as f64
}

// ---------------------------------------------------------------------------------------------------------
// Recognising designed valleys: memories and codes.

/// The patterns stored in one memory of the memory family.
pub struct Stored {
    pub mem: String,
    pub start: usize,
    pub size: usize,
    pub patterns: Vec<(String, Vec<f64>)>,
}

/// Rebuild every memory's stored patterns from the model's notes (the same rule memory.rs uses).
pub fn stored_patterns(m: &Model) -> Vec<Stored> {
    let mut out = Vec::new();
    let mut keys: Vec<&String> = m.notes.keys().filter(|k| k.starts_with("memory:")).collect();
    keys.sort();
    for k in keys {
        let (nums, words) = &m.notes[k];
        let (start, size) = (nums[0] as usize, nums[1] as usize);
        let patterns = words
            .chunks(2)
            .map(|w| {
                let mask = code(&w[0], size);
                let mut p = mask.clone();
                if let Some(t) = w[1].strip_prefix('=') {
                    let bits = t.bytes().flat_map(|b| (0..8).rev().map(move |q| if (b >> q) & 1 == 1 { 1.0 } else { -1.0 }));
                    for (i, b) in bits.enumerate() {
                        p[i] = b * mask[i];
                    }
                }
                (w[0].clone(), p)
            })
            .collect();
        out.push(Stored { mem: k["memory:".len()..].to_string(), start, size, patterns });
    }
    out
}

/// What a valley is, relative to the designed things in the model.
#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    Stored(String),
    Mirror(String),
    Fake(f64),
    Codeword,
    NotCodeword(usize),
    Plain,
}

/// Classify an arrangement (full model length) against the model's first memory, or its code.
pub fn classify(m: &Model, s: &[f64]) -> Kind {
    if let Some(st) = stored_patterns(m).into_iter().next() {
        let part = &s[st.start..st.start + st.size];
        let best = st
            .patterns
            .iter()
            .map(|(n, p)| (n.clone(), overlap(part, p)))
            .max_by(|a, b| a.1.abs().partial_cmp(&b.1.abs()).unwrap());
        return match best {
            Some((n, o)) if o >= 0.9 => Kind::Stored(n),
            Some((n, o)) if o <= -0.9 => Kind::Mirror(n),
            Some((_, o)) => Kind::Fake(o),
            None => Kind::Fake(0.0),
        };
    }
    if let Some((nums, _)) = m.notes.get("valleys:code") {
        let bad = code_violations(nums, s);
        return if bad == 0 { Kind::Codeword } else { Kind::NotCodeword(bad) };
    }
    Kind::Plain
}

fn code_violations(nums: &[f64], s: &[f64]) -> usize {
    let start = nums[0] as usize;
    nums[2..]
        .chunks(3)
        .filter(|t| t.iter().filter(|&&b| s[start + b as usize] > 0.0).count() % 2 == 1)
        .count()
}

fn kind_text(k: &Kind) -> String {
    match k {
        Kind::Stored(n) => format!("stored :{}", n),
        Kind::Mirror(n) => format!("mirror of :{}", n),
        Kind::Fake(o) => format!("fake (best overlap {:+.2})", o),
        Kind::Codeword => "codeword".into(),
        Kind::NotCodeword(b) => format!("not a codeword ({} checks broken)", b),
        Kind::Plain => String::new(),
    }
}

// ---------------------------------------------------------------------------------------------------------
// Landscape builders.

fn add_things(m: &mut Model, prefix: &str, n: usize, ln: usize) -> Result<usize, SettleError> {
    let start = m.len();
    for i in 0..n {
        let name = format!("{}{}", prefix, i);
        if m.idx.contains_key(&name) {
            return err(ln, format!("thing :{} already exists; one landscape of this kind per model", name));
        }
        m.add(&name);
    }
    Ok(start)
}

/// A random landscape: every pair pulled or pushed by scale * N(0,1) / sqrt(n), each lean scale * field * N(0,1).
pub fn random_landscape(m: &mut Model, n: usize, seed: u64, scale: f64, field: f64) -> usize {
    let start = add_things(m, "v", n, 0).expect("fresh model");
    let mut r = Rng::new(seed);
    let w = scale / (n as f64).sqrt();
    for i in 0..n {
        for k in (i + 1)..n {
            m.couple(start + i, start + k, w * r.normal());
        }
    }
    if field != 0.0 {
        for i in 0..n {
            m.h[start + i] += scale * field * r.normal();
        }
    }
    start
}

/// A width x height grid where every thing pulls its four neighbours by 1 (wrapping into a torus if asked).
pub fn grid(m: &mut Model, w: usize, h: usize, wrap: bool) -> usize {
    let start = add_things(m, "g", w * h, 0).expect("fresh model");
    let at = |r: usize, c: usize| start + r * w + c;
    for r in 0..h {
        for c in 0..w {
            if c + 1 < w || wrap {
                m.couple(at(r, c), at(r, (c + 1) % w), 1.0);
            }
            if r + 1 < h || wrap {
                m.couple(at(r, c), at((r + 1) % h, c), 1.0);
            }
        }
    }
    start
}

/// A ring where each thing pulls the next by 1.
pub fn ring(m: &mut Model, n: usize) -> usize {
    let start = add_things(m, "r", n, 0).expect("fresh model");
    for i in 0..n {
        m.couple(start + i, start + (i + 1) % n, 1.0);
    }
    start
}

/// Add strength * b_i * b_j (binary b = (1 + s) / 2) to the energy, as pulls and leans.
fn qubo_pair(m: &mut Model, i: usize, k: usize, q: f64) {
    m.couple(i, k, -q / 4.0);
    m.h[i] -= q / 4.0;
    m.h[k] -= q / 4.0;
}
fn qubo_one(m: &mut Model, i: usize, q: f64) {
    m.h[i] -= q / 2.0;
}

/// Rank over GF(2) of rows given as bit masks.
pub fn gf2_rank(rows: &[u64]) -> usize {
    let mut rows = rows.to_vec();
    let mut rank = 0;
    for bit in 0..64 {
        if let Some(p) = (rank..rows.len()).find(|&r| (rows[r] >> bit) & 1 == 1) {
            rows.swap(rank, p);
            for r in 0..rows.len() {
                if r != rank && (rows[r] >> bit) & 1 == 1 {
                    rows[r] ^= rows[rank];
                }
            }
            rank += 1;
        }
    }
    rank
}

/// A code landscape: `bits` data things and `checks` random 3-bit parity checks covering every bit. Each check a+b+c even is the
/// penalty strength * (a + b + c - 2x)^2 with one helper thing x, which is 0 on an even check (x = sum/2) and
/// at least 1 on an odd one. Returns (data start, number of codewords).
pub fn code_landscape(m: &mut Model, bits: usize, checks: usize, seed: u64, strength: f64, ln: usize) -> Result<(usize, u64), SettleError> {
    if !(3..=64).contains(&bits) {
        return err(ln, "a code needs between 3 and 64 bits");
    }
    if checks * 3 < bits {
        return err(ln, format!("{} checks of 3 bits cannot cover {} bits; an unchecked bit makes a flat valley", checks, bits));
    }
    let start = add_things(m, "d", bits, ln)?;
    let aux = add_things(m, "x", checks, ln)?;
    let mut r = Rng::new(seed);
    let mut nums = vec![start as f64, bits as f64];
    let mut rows = Vec::new();
    // the first checks walk a random order of the bits, so every bit is in at least one check
    let mut order: Vec<usize> = (0..bits).collect();
    for k in (1..bits).rev() {
        let j = r.below(k + 1);
        order.swap(k, j);
    }
    for c in 0..checks {
        let mut t: Vec<usize> = order.iter().skip(3 * c).take(3).copied().collect();
        while t.len() < 3 {
            let b = r.below(bits);
            if !t.contains(&b) {
                t.push(b);
            }
        }
        rows.push(t.iter().fold(0u64, |a, &b| a | (1 << b)));
        let x = aux + c;
        let q = strength;
        for &a in &t {
            qubo_one(m, start + a, q);
            qubo_pair(m, start + a, x, -4.0 * q);
        }
        qubo_one(m, x, 4.0 * q);
        for p in 0..3 {
            for s in (p + 1)..3 {
                qubo_pair(m, start + t[p], start + t[s], 2.0 * q);
            }
        }
        nums.extend(t.iter().map(|&b| b as f64));
    }
    m.notes.insert("valleys:code".into(), (nums, Vec::new()));
    Ok((start, 1u64 << (bits - gf2_rank(&rows))))
}

// ---------------------------------------------------------------------------------------------------------
// Statements.

fn landscape(m: &mut Model, kind: &str, rest: &[Tok], ln: usize) -> Result<(), SettleError> {
    let kv = kwargs(rest, ln)?;
    let get = |k: &str, d: f64| -> Result<f64, SettleError> { kw(&kv, k).map(|v| num(v, ln)).transpose().map(|x| x.unwrap_or(d)) };
    let fresh = |m: &Model, p: &str| -> Result<(), SettleError> {
        if m.idx.contains_key(&format!("{}0", p)) {
            err(ln, format!("this model already has a :{} landscape", kind))
        } else {
            Ok(())
        }
    };
    match kind {
        "random" => {
            only(&kv, &["size", "seed", "scale", "field"], "landscape :random", ln)?;
            fresh(m, "v")?;
            let n = get("size", 16.0)? as usize;
            if !(2..=100_000).contains(&n) {
                return err(ln, "size must be between 2 and 100000");
            }
            random_landscape(m, n, get("seed", 1.0)? as u64, get("scale", 1.0)?, get("field", 0.0)?);
        }
        "grid" => {
            only(&kv, &["width", "height", "wrap"], "landscape :grid", ln)?;
            fresh(m, "g")?;
            let (w, h) = (get("width", 8.0)? as usize, get("height", 8.0)? as usize);
            let wrap = kw(&kv, "wrap").map(|v| yes_no(v, ln)).transpose()?.unwrap_or(1.0) > 0.0;
            if wrap && (w < 3 || h < 3) {
                return err(ln, "a wrapped grid needs width and height of at least 3");
            }
            grid(m, w, h, wrap);
        }
        "ring" => {
            only(&kv, &["size"], "landscape :ring", ln)?;
            fresh(m, "r")?;
            let n = get("size", 12.0)? as usize;
            if n < 3 {
                return err(ln, "a ring needs at least 3 things");
            }
            ring(m, n);
        }
        "code" => {
            only(&kv, &["bits", "checks", "seed", "strength"], "landscape :code", ln)?;
            fresh(m, "d")?;
            code_landscape(m, get("bits", 12.0)? as usize, get("checks", 6.0)? as usize, get("seed", 1.0)? as u64, get("strength", 1.0)?, ln)?;
        }
        other => return err(ln, format!("no landscape :{} (try :random, :grid, :ring or :code)", other)),
    }
    Ok(())
}

fn bar(p: f64) -> String {
    "#".repeat((p * 30.0).round() as usize)
}

fn bits_of(m: &Model, s: &[f64]) -> String {
    if m.len() > 64 {
        return String::new();
    }
    let t: String = s.iter().map(|&v| if v > 0.0 { '1' } else { '0' }).collect();
    format!(" {}", t)
}

fn valleys_stmt(m: &Model, st: &State, rest: &[Tok], ln: usize, ctx: &mut Ctx) -> Result<(), SettleError> {
    let kv = kwargs(rest, ln)?;
    only(&kv, &["show"], "valleys", ln)?;
    let show = kw(&kv, "show").map(|v| num(v, ln)).transpose()?.unwrap_or(10.0) as usize;
    let d = Dense::of(m, &st.held);
    if d.n > MAX_EXACT {
        return err(ln, format!("valleys is exact and visits every arrangement: {} free things is above {}; use survey", d.n, MAX_EXACT));
    }
    let c = enumerate(&d);
    let total = c.total() as f64;
    let reps: HashMap<u32, usize> = c.valleys.iter().enumerate().map(|(i, v)| (v.rep, i)).collect();
    let mask: u32 = ((1u64 << d.n) - 1) as u32;
    let mirrors = c.valleys.iter().filter(|v| v.floor == 1 && reps.contains_key(&(!v.rep & mask))).count();
    ctx.say(format!(
        "valleys (exact, all {} arrangements of {} free things): {} valleys, {} of them in mirror pairs, calmest energy {:.3}",
        c.total(),
        d.n,
        c.valleys.len(),
        mirrors,
        c.valleys.first().map(|v| v.energy).unwrap_or(0.0)
    ));
    let mut order: Vec<usize> = (0..c.valleys.len()).collect();
    order.sort_by(|&a, &b| c.valleys[b].basin.cmp(&c.valleys[a].basin));
    for &i in order.iter().take(show) {
        let v = &c.valleys[i];
        let mut s = vec![0.0; m.len()];
        for (&k, &val) in &st.held {
            s[k] = val;
        }
        for (a, &k) in d.free.iter().enumerate() {
            s[k] = if (v.rep >> a) & 1 == 1 { 1.0 } else { -1.0 };
        }
        let flat = if v.floor > 1 { format!("  flat floor of {}", v.floor) } else { String::new() };
        let kind = kind_text(&classify(m, &s));
        ctx.say(format!(
            "  energy {:>9.3}  basin {:>6.2}% {:<30}{}{}{}",
            v.energy,
            100.0 * v.basin as f64 / total,
            bar(v.basin as f64 / total),
            bits_of(m, &s),
            flat,
            if kind.is_empty() { String::new() } else { format!("  {}", kind) }
        ));
    }
    if c.valleys.len() > show {
        ctx.say(format!("  ... {} more valleys", c.valleys.len() - show));
    }
    if c.saddle > 0 {
        ctx.say(format!("  steepest descent stops on a flat saddle from {:.2}% of arrangements", 100.0 * c.saddle as f64 / total));
    }
    Ok(())
}

fn survey_stmt(m: &Model, st: &mut State, rest: &[Tok], ln: usize, ctx: &mut Ctx) -> Result<(), SettleError> {
    let kv = kwargs(rest, ln)?;
    only(&kv, &["starts", "sweeps", "temperature", "seed", "show"], "survey", ln)?;
    let get = |k: &str, d: f64| -> Result<f64, SettleError> { kw(&kv, k).map(|v| num(v, ln)).transpose().map(|x| x.unwrap_or(d)) };
    let starts = get("starts", 1000.0)? as usize;
    let sweeps = get("sweeps", 50.0)? as usize;
    let temp = get("temperature", 0.05)?;
    let show = get("show", 8.0)? as usize;
    if temp <= 0.0 {
        return err(ln, "temperature must be above zero");
    }
    if starts == 0 {
        return err(ln, "survey needs at least one start");
    }
    if let Some(v) = kw(&kv, "seed") {
        st.rng = Rng::new(num(v, ln)? as u64);
    }
    let found = survey(m, st, starts, sweeps, temp);
    let total = starts as f64;
    let once = found.iter().filter(|f| f.count == 1).count();
    let calm = found.iter().map(|f| f.energy).fold(f64::INFINITY, f64::min);
    let calm_share = found.iter().filter(|f| (f.energy - calm).abs() < 1e-9).map(|f| f.count).sum::<u64>() as f64 / total;
    ctx.say(format!(
        "survey: {} starts, {} sweeps at temperature {} then a quench: {} valleys found ({} seen once; Chao1 estimate {:.0})",
        starts,
        sweeps,
        temp,
        found.len(),
        once,
        chao1(&found)
    ));
    ctx.say(format!("  calmest found: energy {:.3}, reached from {:.1}% of starts", calm, 100.0 * calm_share));
    let keys: HashMap<Vec<u64>, usize> = found.iter().enumerate().map(|(i, f)| (key(&f.state), i)).collect();
    for (i, f) in found.iter().enumerate().take(show) {
        let mirror: Vec<f64> = f.state.iter().map(|v| -v).collect();
        let mtag = match keys.get(&key(&mirror)) {
            Some(&j) => format!("  mirror of #{}", j + 1),
            None => String::new(),
        };
        let kind = kind_text(&classify(m, &f.state));
        ctx.say(format!(
            "  #{:<3} energy {:>9.3}  basin {:>6.2}% {:<30}{}{}{}{}",
            i + 1,
            f.energy,
            100.0 * f.count as f64 / total,
            bar(f.count as f64 / total),
            bits_of(m, &f.state),
            if f.flat { "  (flat)" } else { "" },
            mtag,
            if kind.is_empty() { String::new() } else { format!("  {}", kind) }
        ));
    }
    let top: Vec<&Found> = found.iter().take(show.min(5)).collect();
    if top.len() > 1 {
        ctx.say("  overlaps among the top valleys (+1 same, -1 mirror, 0 unrelated):");
        for a in &top {
            let row: Vec<String> = top.iter().map(|b| format!("{:+.2}", overlap(&a.state, &b.state))).collect();
            ctx.say(format!("    {}", row.join(" ")));
        }
    }
    let flat = found.iter().filter(|f| f.flat).map(|f| f.count).sum::<u64>();
    if flat > 0 {
        ctx.say(format!("  {:.1}% of starts stopped on a flat floor or saddle", 100.0 * flat as f64 / total));
    }
    if !stored_patterns(m).is_empty() || m.notes.contains_key("valleys:code") {
        let mut tally: HashMap<&str, (u64, usize)> = HashMap::new();
        for f in &found {
            let k = match classify(m, &f.state) {
                Kind::Stored(_) => "stored",
                Kind::Mirror(_) => "mirror",
                Kind::Fake(_) => "fake",
                Kind::Codeword => "codeword",
                Kind::NotCodeword(_) => "not a codeword",
                Kind::Plain => "plain",
            };
            let e = tally.entry(k).or_default();
            e.0 += f.count;
            e.1 += 1;
        }
        let mut parts: Vec<String> =
            tally.iter().map(|(k, (c, v))| format!("{} {:.1}% of starts ({} valleys)", k, 100.0 * *c as f64 / total, v)).collect();
        parts.sort();
        ctx.say(format!("  kinds: {}", parts.join(" · ")));
    }
    Ok(())
}

impl Ext for Valleys {
    fn name(&self) -> &'static str {
        "valleys"
    }

    fn statements(&self) -> &'static [&'static str] {
        &[
            "model: landscape :random, size: 16, seed: 1, scale: 1, field: 0",
            "model: landscape :grid, width: 8, height: 8, wrap: :yes   /   landscape :ring, size: 12",
            "model: landscape :code, bits: 12, checks: 6, seed: 1, strength: 1",
            "run: valleys show: 10   (exact, up to 24 free things)",
            "run: survey starts: 1000, sweeps: 50, temperature: 0.05, seed: 1, show: 8",
        ]
    }

    fn model_stmt(&self, m: &mut Model, t: &[Tok], ln: usize, _ctx: &mut Ctx) -> Claim {
        match t {
            [Tok::Ident(k), Tok::Sym(kind), rest @ ..] if k == "landscape" => {
                let rest = if rest.first() == Some(&Tok::Comma) { &rest[1..] } else { rest };
                Some(landscape(m, kind, rest, ln))
            }
            _ => None,
        }
    }

    fn run_stmt(&self, m: &mut Model, st: &mut State, t: &[Tok], ln: usize, ctx: &mut Ctx) -> Claim {
        match t {
            [Tok::Ident(k), rest @ ..] if k == "valleys" => Some(valleys_stmt(m, st, rest, ln, ctx)),
            [Tok::Ident(k), rest @ ..] if k == "survey" => Some(survey_stmt(m, st, rest, ln, ctx)),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interp::Interp;

    fn brute_minima(d: &Dense) -> Vec<u32> {
        let mut out = Vec::new();
        for c in 0..(1u32 << d.n) {
            let e = d.energy(c);
            if (0..d.n).all(|i| d.energy(c ^ (1 << i)) > e + 1e-9) {
                out.push(c);
            }
        }
        out
    }

    fn sk(n: usize, seed: u64) -> (Model, Dense) {
        let mut m = Model::default();
        random_landscape(&mut m, n, seed, 1.0, 0.0);
        let d = Dense::of(&m, &HashMap::new());
        (m, d)
    }

    #[test]
    fn enumeration_matches_brute_force_on_a_random_landscape() {
        for seed in 1..6 {
            let (_, d) = sk(11, seed);
            let c = enumerate(&d);
            let mut got: Vec<u32> = c.valleys.iter().map(|v| v.rep).collect();
            got.sort();
            assert_eq!(got, brute_minima(&d), "seed {}", seed);
            assert_eq!(c.valleys.iter().map(|v| v.basin).sum::<u64>() + c.saddle, c.total());
            assert!(c.valleys.iter().all(|v| v.basin >= 1));
        }
    }

    #[test]
    fn no_pulls_with_leans_has_exactly_one_valley_at_the_lean_signs() {
        let mut m = Model::default();
        let lean = [0.5, -1.0, 2.0, -0.25, 1.0, -3.0, 0.7, 0.1];
        for (i, &l) in lean.iter().enumerate() {
            m.add(&format!("t{}", i));
            m.h[i] = l;
        }
        let c = enumerate(&Dense::of(&m, &HashMap::new()));
        assert_eq!(c.valleys.len(), 1);
        let want = lean.iter().enumerate().fold(0u32, |a, (i, &l)| if l > 0.0 { a | (1 << i) } else { a });
        assert_eq!(c.valleys[0].rep, want);
        assert_eq!(c.valleys[0].basin, c.total());
    }

    #[test]
    fn no_pulls_and_no_leans_is_one_flat_valley_holding_everything() {
        let mut m = Model::default();
        for i in 0..9 {
            m.add(&format!("t{}", i));
        }
        let c = enumerate(&Dense::of(&m, &HashMap::new()));
        assert_eq!(c.valleys.len(), 1);
        assert_eq!(c.valleys[0].floor, 512);
        assert_eq!(c.valleys[0].basin, 512);
    }

    #[test]
    fn a_ring_has_exactly_two_valleys_with_equal_basins() {
        let mut m = Model::default();
        ring(&mut m, 12);
        let c = enumerate(&Dense::of(&m, &HashMap::new()));
        assert_eq!(c.valleys.len(), 2);
        let reps: Vec<u32> = c.valleys.iter().map(|v| v.rep).collect();
        assert!(reps.contains(&0) && reps.contains(&0xfff));
        assert_eq!(c.valleys[0].basin, c.valleys[1].basin);
        assert!(c.saddle > 0, "the flat domain-wall moves of a ring leave some arrangements on saddles");
    }

    #[test]
    fn shuffled_energies_have_about_2n_over_n_plus_1_valleys() {
        // control: a structureless energy table has, in expectation, 2^n/(n+1) strict minima
        let n = 12;
        let mut tot = 0.0;
        let seeds = 20;
        for seed in 0..seeds {
            let (_, d) = sk(n, 100 + seed);
            let mut e = energy_table(&d);
            let mut r = Rng::new(seed);
            for k in (1..e.len()).rev() {
                let j = r.below(k + 1);
                e.swap(k, j);
            }
            tot += enumerate_table(n, &e).valleys.len() as f64;
        }
        let mean = tot / seeds as f64;
        let want = 4096.0 / 13.0;
        assert!((mean - want).abs() < 0.05 * want, "{} vs {}", mean, want);
        let (_, d) = sk(n, 100);
        assert!((enumerate(&d).valleys.len() as f64) < want / 5.0, "the unshuffled landscape has far fewer valleys");
    }

    #[test]
    fn random_landscape_valleys_come_in_mirror_pairs_with_equal_basins() {
        let (_, d) = sk(12, 9);
        let c = enumerate(&d);
        let by: HashMap<u32, &Valley> = c.valleys.iter().map(|v| (v.rep, v)).collect();
        for v in &c.valleys {
            let w = by[&(!v.rep & 0xfff)];
            assert_eq!(v.basin, w.basin);
            assert!((v.energy - w.energy).abs() < 1e-9);
        }
    }

    #[test]
    fn survey_finds_only_true_valleys_and_all_of_a_small_landscape() {
        let (m, d) = sk(12, 4);
        let c = enumerate(&d);
        let exact: std::collections::HashSet<u32> = c.valleys.iter().map(|v| v.rep).collect();
        let mut st = State::new(2);
        let found = survey(&m, &mut st, 3000, 5, 0.05);
        for f in &found {
            let bits = f.state.iter().enumerate().fold(0u32, |a, (i, &v)| if v > 0.0 { a | (1 << i) } else { a });
            assert!(exact.contains(&bits), "survey returned a non-valley");
        }
        assert_eq!(found.len(), exact.len());
    }

    #[test]
    fn a_hopfield_memory_of_one_pattern_has_two_valleys_its_pattern_and_mirror() {
        let mut it = Interp::default();
        it.exec("model :mind do\n  memory :m, size: 16\n  m.remember :cat\nend").unwrap();
        let m = &it.models["mind"];
        let c = enumerate(&Dense::of(m, &HashMap::new()));
        assert_eq!(c.valleys.len(), 2);
        assert_eq!(c.valleys.iter().map(|v| v.basin).sum::<u64>(), 1 << 16);
        let mut s = vec![0.0; 16];
        for (i, v) in s.iter_mut().enumerate() {
            *v = if (c.valleys[0].rep >> i) & 1 == 1 { 1.0 } else { -1.0 };
        }
        assert!(matches!(classify(m, &s), Kind::Stored(_) | Kind::Mirror(_)));
    }

    #[test]
    fn mirror_basins_stay_equal_when_many_slopes_tie() {
        // Hopfield pulls are multiples of 1/size, so slopes tie often; rounding must not break the mirror symmetry
        let mut it = Interp::default();
        it.exec("model :mind do\n  memory :m, size: 16\n  m.remember :cat\n  m.remember :dog\n  m.remember :owl\nend").unwrap();
        let c = enumerate(&Dense::of(&it.models["mind"], &HashMap::new()));
        let by: HashMap<u32, u64> = c.valleys.iter().map(|v| (v.rep, v.basin)).collect();
        for v in c.valleys.iter().filter(|v| v.floor == 1) {
            assert_eq!(Some(&v.basin), by.get(&(!v.rep & 0xffff)));
        }
    }

    #[test]
    fn every_codeword_is_a_calmest_valley() {
        let mut m = Model::default();
        let (_, words) = code_landscape(&mut m, 10, 5, 3, 1.0, 0).unwrap();
        let d = Dense::of(&m, &HashMap::new());
        let c = enumerate(&d);
        let calm = c.valleys[0].energy;
        let bottom: Vec<&Valley> = c.valleys.iter().filter(|v| (v.energy - calm).abs() < 1e-9).collect();
        assert_eq!(bottom.len() as u64, words);
        for v in bottom {
            let s: Vec<f64> = (0..d.n).map(|i| if (v.rep >> i) & 1 == 1 { 1.0 } else { -1.0 }).collect();
            assert_eq!(classify(&m, &s), Kind::Codeword);
        }
    }

    #[test]
    fn the_program_runs_end_to_end() {
        let out = Interp::default()
            .exec(
                "model :g do\n  landscape :random, size: 10, seed: 2\nend\nrun :g do\n  valleys show: 3\n  survey starts: 300, sweeps: 10, seed: 1, show: 3\nend",
            )
            .unwrap();
        assert!(out[0].starts_with("valleys (exact, all 1024 arrangements of 10 free things)"));
        assert!(out.iter().any(|l| l.starts_with("survey: 300 starts")));
        let e = Interp::default().exec("model :g do\n  landscape :volcano\nend").err().unwrap().0;
        assert!(e.starts_with("line 2: no landscape :volcano"), "{}", e);
    }
}
