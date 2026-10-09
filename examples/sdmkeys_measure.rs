//! SDMKEYS measurements: Hopfield (memory.rs) against Kanerva SDM (sdm.rs), fake valleys, fade, controls,
//! and what a key protects. Run: `cargo run --release --example sdmkeys_measure [part]`, part one of
//! capacity | noise | fade | controls | keys | all (default all). Predictions P1..P11 were sealed in the
//! campaign ledger before this file existed. Everything is seeded; no timing is measured.

#![allow(clippy::needless_range_loop)] // index loops mirror the equations they measure

use settle::memory::{code, key_turn, keyed_read_address, keyed_pattern, keyed_read, shake, store_pattern};
use settle::model::{Model, State};
use settle::rng::Rng;
use settle::sdm::{radius_for, View};
use std::sync::Mutex;

const N: usize = 256;
const SEEDS: [u64; 3] = [1, 2, 3];
const DAMAGES: [f64; 4] = [0.1, 0.2, 0.3, 0.4];
const CUES: usize = 20;
const OK: f64 = 0.95;

fn ov(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(x, y)| x * y).sum::<f64>() / a.len() as f64
}

fn add_address_noise(p: &[f64], d: f64, r: &mut Rng) -> Vec<f64> {
    p.iter().map(|&b| if r.unit() < d { -b } else { b }).collect()
}

fn pat(seed: u64, k: usize, n: usize) -> Vec<f64> {
    code(&format!("s{}-p{}", seed, k), n)
}

/// One memory system under test, grown one pattern at a time.
enum Sys {
    Hop { m: Model, n: usize, fade: f64 },
    Sdm { m: Model, v: View, pulls: bool },
}

impl Sys {
    fn hop(n: usize, fade: f64) -> Sys {
        let mut m = Model::default();
        for i in 0..n {
            m.add(&format!("h_{}", i));
        }
        Sys::Hop { m, n, fade }
    }
    fn sdm(n: usize, locs: usize, p_wake: f64, pulls: bool, fade: f64, seed: u64) -> Sys {
        let mut m = Model::default();
        let v = View::declare(&mut m, "s", n, locs, radius_for(n, p_wake), seed, fade);
        Sys::Sdm { m, v, pulls }
    }
    fn store(&mut self, p: &[f64]) {
        match self {
            Sys::Hop { m, n, fade } => store_pattern(m, 0, *n, *fade, p),
            Sys::Sdm { m, v, .. } => {
                v.write(m, p);
            }
        }
    }
    fn recall(&self, cue: &[f64], st: &mut State) -> Vec<f64> {
        self.recall2(cue, st).0
    }
    /// The final state and, for SDM, how many hard locations were awake at the end (0 = the read was silent).
    fn recall2(&self, cue: &[f64], st: &mut State) -> (Vec<f64>, Option<usize>) {
        match self {
            Sys::Hop { m, n, .. } => (shake(m, st, 0, &cue[..*n], 30, 0.1), None),
            Sys::Sdm { m, v, pulls } => {
                let (z, _, a) = if *pulls { v.read_pulls(m, st, cue, 10) } else { v.read_addresses(m, cue, 10) };
                (z, Some(a))
            }
        }
    }
}

type System = (&'static str, Box<dyn Fn(u64) -> Sys + Sync>, usize);

fn systems() -> Vec<System> {
    vec![
        ("H", Box::new(|_s| Sys::hop(N, 1.0)), 200),
        ("S2000a", Box::new(|s| Sys::sdm(N, 2000, 0.02, false, 1.0, s)), 1000),
        ("S2000p", Box::new(|s| Sys::sdm(N, 2000, 0.02, true, 1.0, s)), 1000),
        ("S128a", Box::new(|s| Sys::sdm(N, 128, 0.10, false, 1.0, s)), 200),
        ("S128p", Box::new(|s| Sys::sdm(N, 128, 0.10, true, 1.0, s)), 200),
    ]
}

const CHECKS: [usize; 18] = [5, 10, 15, 20, 25, 30, 40, 50, 60, 80, 100, 150, 200, 300, 400, 600, 800, 1000];

fn capacity(out: &Mutex<Vec<String>>) {
    let rows = Mutex::new(Vec::new());
    let sys_list = systems();
    std::thread::scope(|sc| {
        for (name, make, pmax) in sys_list.iter() {
            for &seed in &SEEDS {
                let rows = &rows;
                sc.spawn(move || {
                    let mut sys = make(seed);
                    let mut stored: Vec<Vec<f64>> = Vec::new();
                    let mut r = Rng::new(seed * 1000 + 7);
                    let mut st = State::new(seed);
                    for &p in CHECKS.iter().filter(|&&p| p <= *pmax) {
                        while stored.len() < p {
                            let q = pat(seed, stored.len(), N);
                            sys.store(&q);
                            stored.push(q);
                        }
                        for &d in &DAMAGES {
                            let mut hit = 0;
                            let cues = CUES.min(p);
                            for c in 0..cues {
                                let t = if p <= CUES { c } else { r.below(p) };
                                let cue = add_address_noise(&stored[t], d, &mut r);
                                let z = sys.recall(&cue, &mut st);
                                if ov(&z, &stored[t]) >= OK {
                                    hit += 1;
                                }
                            }
                            rows.lock().unwrap().push((name.to_string(), seed, p, d, hit, cues));
                        }
                    }
                });
            }
        }
    });
    let rows = rows.into_inner().unwrap();
    let mut o = out.lock().unwrap();
    o.push("## capacity: success rate (final overlap >= 0.95) by stored count and damage, 3 seeds pooled".into());
    for (name, _, pmax) in systems().iter() {
        o.push(format!("\n{} (checkpoints to {})", name, pmax));
        o.push(format!("{:>6} {}", "stored", DAMAGES.iter().map(|d| format!("  d={:.1}", d)).collect::<String>()));
        let mut p90 = [None::<usize>; 4];
        let mut dead = [false; 4];
        for &p in CHECKS.iter().filter(|&&p| p <= *pmax) {
            let mut line = format!("{:>6} ", p);
            for (k, &d) in DAMAGES.iter().enumerate() {
                let (h, t) = rows.iter().filter(|x| x.0 == *name && x.2 == p && x.3 == d).fold((0, 0), |a, x| (a.0 + x.4, a.1 + x.5));
                let rate = h as f64 / t as f64;
                line.push_str(&format!("  {:>5.2}", rate));
                if !dead[k] {
                    if rate >= 0.9 {
                        p90[k] = Some(p);
                    } else {
                        dead[k] = true;
                    }
                }
            }
            o.push(line);
        }
        o.push(format!(
            "P90 {}: {}",
            name,
            DAMAGES.iter().zip(p90).map(|(d, p)| format!("d={:.1} -> {}", d, p.map(|x| x.to_string()).unwrap_or("0 (fails at 5)".into()))).collect::<Vec<_>>().join(" · ")
        ));
    }
}

/// Classify a final state against the stored set: stored hit, mirror, three-way mixture, or other.
fn classify(z: &[f64], stored: &[Vec<f64>]) -> &'static str {
    let mut s: Vec<(usize, f64)> = stored.iter().enumerate().map(|(i, p)| (i, ov(z, p))).collect();
    s.sort_by(|a, b| b.1.abs().partial_cmp(&a.1.abs()).unwrap());
    if s[0].1 >= OK {
        return "stored";
    }
    if s[0].1 <= -OK {
        return "mirror";
    }
    if s.len() >= 3 {
        let mix: Vec<f64> = (0..z.len())
            .map(|j| {
                let v: f64 = s[..3].iter().map(|&(i, o)| o.signum() * stored[i][j]).sum();
                v.signum()
            })
            .collect();
        if ov(z, &mix).abs() >= 0.9 {
            return "mixture3";
        }
    }
    "other"
}

fn noise(out: &Mutex<Vec<String>>) {
    let mut lines = vec!["\n## fake valleys: 100 starts from pure noise per seed, 3 seeds".to_string()];
    lines.push(format!("{:<8} {:>6} {:>8} {:>8} {:>9} {:>7} {:>6} {:>8}", "system", "stored", "stored%", "mirror%", "mixture3%", "other%", "fake%", "silent%"));
    for (name, make, _) in systems().iter() {
        for &p in &[5usize, 10, 20, 50] {
            let mut c = std::collections::HashMap::new();
            let mut tot = 0;
            for &seed in &SEEDS {
                let mut sys = make(seed);
                let stored: Vec<Vec<f64>> = (0..p).map(|k| pat(seed, k, N)).collect();
                stored.iter().for_each(|q| sys.store(q));
                let mut r = Rng::new(seed * 31 + p as u64);
                let mut st = State::new(seed + 99);
                for _ in 0..100 {
                    let x: Vec<f64> = (0..N).map(|_| if r.unit() < 0.5 { -1.0 } else { 1.0 }).collect();
                    let (z, a) = sys.recall2(&x, &mut st);
                    *c.entry(classify(&z, &stored)).or_insert(0) += 1;
                    if a == Some(0) {
                        *c.entry("silent").or_insert(0) += 1;
                    }
                    tot += 1;
                }
            }
            let f = |k: &str| 100.0 * *c.get(k).unwrap_or(&0) as f64 / tot as f64;
            lines.push(format!(
                "{:<8} {:>6} {:>8.1} {:>8.1} {:>9.1} {:>7.1} {:>6.1} {:>8.1}",
                name,
                p,
                f("stored"),
                f("mirror"),
                f("mixture3"),
                f("other"),
                100.0 - f("stored") - f("mirror"),
                f("silent")
            ));
        }
    }
    out.lock().unwrap().extend(lines);
}

fn fade(out: &Mutex<Vec<String>>) {
    let mut lines = vec![
        "\n## fade: 300 stored in order, recall the most recent 60 at d = 0.2 (3 seeds x 5 draws per age)".to_string(),
        "K = number of ages (0 = newest) recalled at >= 90%; first = first age below 90%".to_string(),
    ];
    let specs: Vec<(String, f64, bool)> = [1.0, 0.99, 0.98, 0.97, 0.95, 0.9, 0.8, 0.6]
        .iter()
        .map(|&f| (format!("H fade {}", f), f, true))
        .chain([1.0, 0.99, 0.98, 0.95, 0.9].iter().map(|&f| (format!("S2000a fade {} (unsealed)", f), f, false)))
        .collect();
    let res = Mutex::new(Vec::new());
    std::thread::scope(|sc| {
        for (label, f, hop) in specs.iter() {
            let res = &res;
            sc.spawn(move || {
                let mut ok = vec![0usize; 60];
                for &seed in &SEEDS {
                    let mut sys = if *hop { Sys::hop(N, *f) } else { Sys::sdm(N, 2000, 0.02, false, *f, seed) };
                    let stored: Vec<Vec<f64>> = (0..300).map(|k| pat(seed, k, N)).collect();
                    stored.iter().for_each(|q| sys.store(q));
                    let mut r = Rng::new(seed * 7 + 3);
                    let mut st = State::new(seed);
                    for age in 0..60 {
                        for _ in 0..5 {
                            let t = &stored[299 - age];
                            let z = sys.recall(&add_address_noise(t, 0.2, &mut r), &mut st);
                            if ov(&z, t) >= OK {
                                ok[age] += 1;
                            }
                        }
                    }
                }
                let k = ok.iter().filter(|&&x| x as f64 >= 0.9 * 15.0).count();
                let first = ok.iter().position(|&x| (x as f64) < 0.9 * 15.0).map(|a| a.to_string()).unwrap_or(">59".into());
                let curve: String = ok.iter().step_by(5).map(|x| format!("{:>3}", x)).collect();
                res.lock().unwrap().push((label.clone(), format!("{:<28} K = {:>2}  first = {:>3}  hits/15 at ages 0,5,..,55:{}", label, k, first, curve)));
            });
        }
    });
    let mut r = res.into_inner().unwrap();
    r.sort_by_key(|x| specs.iter().position(|s| s.0 == x.0).unwrap());
    lines.extend(r.into_iter().map(|x| x.1));
    out.lock().unwrap().extend(lines);
}

/// SDM only: permute every counter entry across the whole matrix, keep the pulls symmetric, and recompute
/// each data thing's lean as the sum of its pulls. A real negative control for both reads.
fn shuffle_entries(sys: &mut Sys, seed: u64) {
    let mut r = Rng::new(seed ^ 0xdef);
    if let Sys::Sdm { m, v, .. } = sys {
        let mut vals: Vec<f64> = (0..v.m_loc).flat_map(|i| m.adj[v.loc + i][..v.n].iter().map(|e| e.1).collect::<Vec<_>>()).collect();
        for k in (1..vals.len()).rev() {
            let j = r.below(k + 1);
            vals.swap(k, j);
        }
        for i in 0..v.m_loc {
            for j in 0..v.n {
                let w = vals[i * v.n + j];
                m.adj[v.loc + i][j].1 = w;
                m.adj[v.data + j][i].1 = w;
            }
        }
        for j in 0..v.n {
            m.h[v.data + j] = m.adj[v.data + j][..v.m_loc].iter().map(|e| e.1).sum();
        }
    }
}

/// Shuffle the stored pulls: Hopfield pair values permuted among pairs; SDM counter rows dealt to other hard locations.
fn shuffle(sys: &mut Sys, seed: u64) {
    let mut r = Rng::new(seed ^ 0xabc);
    match sys {
        Sys::Hop { m, n, .. } => {
            let mut vals = Vec::new();
            for i in 0..*n {
                for k in (i + 1)..*n {
                    vals.push(m.coupling(i, k));
                }
            }
            for k in (1..vals.len()).rev() {
                let j = r.below(k + 1);
                vals.swap(k, j);
            }
            let mut q = 0;
            for i in 0..*n {
                for k in (i + 1)..*n {
                    let w = vals[q];
                    q += 1;
                    for (a, b) in [(i, k), (k, i)] {
                        m.adj[a].iter_mut().find(|e| e.0 == b).unwrap().1 = w;
                    }
                }
            }
        }
        Sys::Sdm { m, v, .. } => {
            let rows: Vec<Vec<f64>> = (0..v.m_loc).map(|i| m.adj[v.loc + i][..v.n].iter().map(|e| e.1).collect()).collect();
            let mut perm: Vec<usize> = (0..v.m_loc).collect();
            for k in (1..perm.len()).rev() {
                let j = r.below(k + 1);
                perm.swap(k, j);
            }
            for i in 0..v.m_loc {
                for j in 0..v.n {
                    let w = rows[perm[i]][j];
                    m.adj[v.loc + i][j].1 = w;
                    m.adj[v.data + j][i].1 = w;
                }
            }
        }
    }
}

fn controls(out: &Mutex<Vec<String>>) {
    let mut lines = vec!["\n## controls (3 seeds pooled; recalled = final overlap >= 0.95 with the target)".to_string()];
    lines.push("never: 50 never-stored patterns per seed as cues at damage 0. moved = recalled AND at least one".into());
    lines.push("location awake at the end (a silent SDM read returns the cue untouched). rows / entries: cued recall".into());
    lines.push("at d = 0.1 after dealing counter rows to other locations / permuting every counter entry (Hopfield: pair values).".into());
    lines.push(format!("{:<8} {:>6} {:>10} {:>9} {:>8} {:>10} {:>12} {:>10}", "system", "stored", "never rec", "never mv", "silent", "rows rec", "entries rec", "plain rec"));
    for (name, make, _) in systems().iter() {
        for &p in &[20usize, 100] {
            let (mut never, mut never_mv, mut silent, mut rows, mut ents, mut plain, mut tn, mut ts) = (0, 0, 0, 0, 0, 0, 0, 0);
            for &seed in &SEEDS {
                let mut sys = make(seed);
                let stored: Vec<Vec<f64>> = (0..p).map(|k| pat(seed, k, N)).collect();
                stored.iter().for_each(|q| sys.store(q));
                let mut st = State::new(seed);
                for k in 0..50 {
                    let x = pat(seed + 1000, k, N);
                    let (z, a) = sys.recall2(&x, &mut st);
                    let rec = ov(&z, &x) >= OK;
                    never += rec as usize;
                    never_mv += (rec && a != Some(0)) as usize;
                    silent += (a == Some(0)) as usize;
                    tn += 1;
                }
                let mut r = Rng::new(seed + 5);
                let cues: Vec<(usize, Vec<f64>)> = (0..20).map(|c| (c % p, add_address_noise(&stored[c % p], 0.1, &mut r))).collect();
                let hits = |sys: &Sys, st: &mut State| cues.iter().filter(|(t, cue)| ov(&sys.recall(cue, st), &stored[*t]) >= OK).count();
                plain += hits(&sys, &mut st);
                ts += cues.len();
                let mut sys_e = make(seed);
                stored.iter().for_each(|q| sys_e.store(q));
                shuffle(&mut sys, seed);
                rows += hits(&sys, &mut st);
                if matches!(sys_e, Sys::Sdm { .. }) {
                    shuffle_entries(&mut sys_e, seed);
                    ents += hits(&sys_e, &mut st);
                } else {
                    ents = rows;
                }
            }
            lines.push(format!(
                "{:<8} {:>6} {:>6}/{:<3} {:>5}/{:<3} {:>4}/{:<3} {:>6}/{:<3} {:>8}/{:<3} {:>6}/{:<3}",
                name, p, never, tn, never_mv, tn, silent, tn, rows, ts, ents, ts, plain, ts
            ));
        }
    }
    out.lock().unwrap().extend(lines);
}

const TEXT: &str = "the quick brown fox jumps over the lazy dog while the harbour sleeps at nine";

fn text_bits(t: &str) -> Vec<f64> {
    t.bytes().flat_map(|b| (0..8).rev().map(move |k| if (b >> k) & 1 == 1 { 1.0 } else { -1.0 })).collect()
}

/// Build a memory holding `publics` public codes and one keyed note. Returns (system, keyed pattern, publics).
fn keyed_system(hop_n: Option<usize>, publics: usize, key: &str, note: &str, seed: u64) -> (Sys, Vec<f64>, Vec<Vec<f64>>) {
    let n = hop_n.unwrap_or(N);
    let mut sys = match hop_n {
        Some(n) => Sys::hop(n, 1.0),
        None => Sys::sdm(N, 2000, 0.02, false, 1.0, seed),
    };
    let pubs: Vec<Vec<f64>> = (0..publics).map(|k| pat(seed, k, n)).collect();
    let kp = keyed_pattern(key, note, n);
    for (k, q) in pubs.iter().enumerate() {
        sys.store(q);
        if k == publics / 2 {
            sys.store(&kp);
        }
    }
    if publics == 0 {
        sys.store(&kp);
    }
    (sys, kp, pubs)
}

fn keys(out: &Mutex<Vec<String>>) {
    let mut lines = vec!["\n## keys".to_string()];
    let note = "meet at the harbour at nine";
    // K1 + K2: right and wrong keys, Hopfield 512 things, 27-byte text.
    lines.push("K1/K2 Hopfield 512 things, 27-byte note, 20 trials per row".into());
    lines.push(format!("{:>7} {:>12} {:>12} {:>20} {:>22}", "public", "right exact", "wrong exact", "wrong: in keyed valley", "forced text bits right"));
    for &publics in &[2usize, 5, 10, 20, 40] {
        let (mut right, mut wrong, mut in_valley, mut bits_ok, mut bits_tot) = (0, 0, 0, 0usize, 0usize);
        for trial in 0..20u64 {
            let key = format!("key-{}", trial);
            let (sys, kp, _) = keyed_system(Some(512), publics, &key, note, trial + 1);
            let mut st = State::new(trial);
            let z = sys.recall(&keyed_read_address(&key, 512), &mut st);
            if keyed_read(&key, &z).as_deref() == Some(note) {
                right += 1;
            }
            let bad = format!("wrong-{}", trial);
            let zw = sys.recall(&keyed_read_address(&bad, 512), &mut st);
            if keyed_read(&bad, &zw).as_deref() == Some(note) {
                wrong += 1;
            }
            if ov(&zw, &kp).abs() >= OK {
                in_valley += 1;
            }
            // forced decode with the wrong key: bits 8.. of the undone state against the note's bits
            let b = key_turn(&bad, 512).undo(&zw);
            let tb = text_bits(note);
            bits_ok += tb.iter().enumerate().filter(|(i, &v)| b[8 + i] * v > 0.0).count();
            bits_tot += tb.len();
        }
        lines.push(format!("{:>7} {:>9}/20 {:>9}/20 {:>17}/20 {:>21.1}%", publics, right, wrong, in_valley, 100.0 * bits_ok as f64 / bits_tot as f64));
    }
    // K3: a stranger shaking the public landscape from noise.
    lines.push("\nK3 stranger, Hopfield 512, 5 public + 1 keyed, 10 trials x 100 noise starts".into());
    let (mut land, mut starts, mut raw_ok, mut raw_tot, mut chars_ok, mut cipher_ov) = (0, 0, 0usize, 0usize, 0usize, 0.0);
    let (mut d_chars, mut d_bits, mut d_notes) = (0usize, 0usize, 0usize);
    let (mut guard_true, mut guard_false, mut guard_tries, mut guard_trials) = (0, 0, 0, 0);
    for trial in 0..10u64 {
        let key = format!("key-{}", trial);
        let (sys, kp, _) = keyed_system(Some(512), 5, &key, note, trial + 1);
        let mut r = Rng::new(trial + 77);
        let mut st = State::new(trial + 5);
        let tb = text_bits(note);
        let mut landed_state = None;
        for _ in 0..100 {
            let x: Vec<f64> = (0..512).map(|_| if r.unit() < 0.5 { -1.0 } else { 1.0 }).collect();
            let z = sys.recall(&x, &mut st);
            starts += 1;
            let o = ov(&z, &kp);
            if o.abs() >= OK {
                land += 1;
                cipher_ov += o.abs();
                let sg = o.signum();
                // the stranger reads the raw state in plain order (no key): payload text sits at bits 8..
                raw_ok += tb.iter().enumerate().filter(|(i, &v)| sg * z[8 + i] * v > 0.0).count();
                raw_tot += tb.len();
                chars_ok += (0..note.len()).filter(|c| (0..8).all(|k| sg * z[8 + c * 8 + k] * tb[c * 8 + k] > 0.0)).count();
                landed_state = Some(z);
            }
        }
        // offline guessing: does the refusal guard single out the true key among 1000 wrong ones?
        if let Some(z) = landed_state {
            let sg = ov(&z, &kp).signum();
            d_notes += 1;
            d_bits += tb.iter().enumerate().filter(|(i, &v)| sg * z[8 + i] * v > 0.0).count();
            d_chars += (0..note.len()).filter(|c| (0..8).all(|k| sg * z[8 + c * 8 + k] * tb[c * 8 + k] > 0.0)).count();
            guard_trials += 1;
            if keyed_read(&key, &z).is_some() {
                guard_true += 1;
            }
            for g in 0..1000 {
                if keyed_read(&format!("guess-{}-{}", trial, g), &z).is_some() {
                    guard_false += 1;
                }
                guard_tries += 1;
            }
        }
    }
    lines.push(format!("noise starts landing in the keyed valley: {}/{} ({:.1}%)", land, starts, 100.0 * land as f64 / starts as f64));
    lines.push(format!("  mean |overlap| with the stored (turned) pattern when landed: {:.3}  (the ciphertext is public)", cipher_ov / land.max(1) as f64));
    lines.push(format!("  text bits right reading the raw state with no key: {:.1}% of {}", 100.0 * raw_ok as f64 / raw_tot.max(1) as f64, raw_tot));
    lines.push(format!("  whole characters right: {} of {} ({:.2}%, chance 1/256 = 0.39%)", chars_ok, land * note.len(), 100.0 * chars_ok as f64 / (land * note.len()).max(1) as f64));
    lines.push(format!(
        "  counted once per distinct keyed note that was landed in ({} notes): bits right {:.1}% of {}, whole characters {} of {} (chance {:.1})",
        d_notes,
        100.0 * d_bits as f64 / (d_notes * note.len() * 8).max(1) as f64,
        d_notes * note.len() * 8,
        d_chars,
        d_notes * note.len(),
        (d_notes * note.len()) as f64 / 256.0
    ));
    lines.push(format!("  key check on a landed state: true key passes {}/{}; wrong guesses pass {}/{}", guard_true, guard_trials, guard_false, guard_tries));
    // K4: length sweep, Hopfield 512, 5 public.
    lines.push("\nK4 key finds and reads its note vs note length, Hopfield 512, 5 public, 20 trials".into());
    for &len in &[8usize, 16, 27, 40, 50, 55, 63] {
        let t = &TEXT[..len];
        let (mut ok, mut found, mut cue_ov) = (0, 0, 0.0);
        for trial in 0..20u64 {
            let key = format!("key-{}", trial);
            let (sys, kp, _) = keyed_system(Some(512), 5, &key, t, trial + 1);
            let mut st = State::new(trial);
            let cue = keyed_read_address(&key, 512);
            cue_ov += ov(&cue, &kp);
            let z = sys.recall(&cue, &mut st);
            if ov(&z, &kp) >= OK {
                found += 1;
            }
            if keyed_read(&key, &z).as_deref() == Some(t) {
                ok += 1;
            }
        }
        let pay = keyed_read_address("x", 512).len();
        lines.push(format!("  {:>2} bytes ({:>3} of {} bits carry text): cue overlap {:+.2} · found {:>2}/20 · read {:>2}/20", len, 8 * (len + 1), pay, cue_ov / 20.0, found, ok));
    }
    // K5: keyed SDM, 2000 hard locations, 5 public.
    lines.push("\nK5 keyed SDM 2000 locations (read via addresses), 256 things, 5 public, 20 trials".into());
    for &len in &[4usize, 8, 12, 16, 20, 24, 31] {
        let t = &TEXT[..len];
        let (mut ok, mut found, mut cue_ov) = (0, 0, 0.0);
        for trial in 0..20u64 {
            let key = format!("key-{}", trial);
            let (sys, kp, _) = keyed_system(None, 5, &key, t, trial + 1);
            let mut st = State::new(trial);
            let cue = keyed_read_address(&key, N);
            cue_ov += ov(&cue, &kp);
            let z = sys.recall(&cue, &mut st);
            if ov(&z, &kp) >= OK {
                found += 1;
            }
            if keyed_read(&key, &z).as_deref() == Some(t) {
                ok += 1;
            }
        }
        lines.push(format!("  {:>2} bytes ({:>3} of 256 bits carry text): cue overlap {:+.2} · found {:>2}/20 · read {:>2}/20", len, 8 * (len + 1), cue_ov / 20.0, found, ok));
    }
    out.lock().unwrap().extend(lines);
}

fn main() {
    let part = std::env::args().nth(1).unwrap_or_else(|| "all".into());
    let out = Mutex::new(vec![format!("# SDMKEYS measurements, part {}", part)]);
    std::thread::scope(|sc| {
        if part == "capacity" || part == "all" {
            sc.spawn(|| capacity(&out));
        }
        if part == "noise" || part == "all" {
            sc.spawn(|| noise(&out));
        }
        if part == "fade" || part == "all" {
            sc.spawn(|| fade(&out));
        }
        if part == "controls" || part == "all" {
            sc.spawn(|| controls(&out));
        }
        if part == "keys" || part == "all" {
            sc.spawn(|| keys(&out));
        }
    });
    for l in out.into_inner().unwrap() {
        println!("{}", l);
    }
}
