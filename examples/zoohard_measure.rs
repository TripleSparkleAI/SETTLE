//! ZOOHARD measurements (`src/zoohard.rs`, `src/zoo.rs`). Report: `runs/zoohard/REPORT_ZOOHARD.md`.
//!
//! Run: `cargo run --release --example zoohard_measure <part>`, part one of
//!   instances   generate the sudoku and colouring instances and decide them exactly (no annealing)
//!   sizes       things, pulls and coefficient spread of both factoring encodings (no annealing)
//!   sudoku      success vs sweeps per rating band, random baseline, the unsolvable control
//!   colouring   success vs sweeps vs average degree for n 40, 80, 160; uncolourable instances as controls
//!   factor      the column encoding up the semiprime ladder, the Rosenberg encoding beside it, primes as controls
//!   shared      puzzles sharing one model: anneal against anneal_each
//!   export <dir>        write the dwave comparison models as settle-ising JSON plus a targets file
//!   settleside <dir>    SETTLE's anneals on those models at equal sweeps (final and best-so-far energies)
//! Everything is seeded; the success rates do not depend on the machine. Wall times are scale only.

use settle::export::{ising_json, parse_ising, to_ising};
use settle::interp::Interp;
use settle::model::{Model, State};
use settle::rng::Rng;
use settle::zoo::*;
use settle::zoohard::*;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Instant;

fn sh(cmd: &str) -> String {
    std::process::Command::new("sh").args(["-c", cmd]).output().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default()
}

fn stamp(tag: &str) {
    println!(
        "STAMP {} utc={} load=[{}] powermode={} threads={}",
        tag,
        sh("date -u +%Y-%m-%dT%H:%M:%SZ"),
        sh("uptime"),
        sh("pmset -g | grep -i powermode | awk '{print $2}'"),
        threads()
    );
}

fn threads() -> usize {
    std::env::var("ZOOHARD_THREADS").ok().and_then(|v| v.parse().ok()).unwrap_or(10)
}

/// Run `f` over `jobs` on a fixed pool of worker threads; results in job order.
fn par<J: Sync, R: Send>(jobs: &[J], f: impl Fn(&J) -> R + Sync) -> Vec<R> {
    let next = AtomicUsize::new(0);
    let out: Mutex<Vec<Option<R>>> = Mutex::new((0..jobs.len()).map(|_| None).collect());
    std::thread::scope(|sc| {
        for _ in 0..threads().min(jobs.len().max(1)) {
            sc.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                if i >= jobs.len() {
                    break;
                }
                let r = f(&jobs[i]);
                out.lock().unwrap()[i] = Some(r);
            });
        }
    });
    out.into_inner().unwrap().into_iter().map(|r| r.unwrap()).collect()
}

fn build(src: &str) -> Model {
    let mut it = Interp::default();
    it.exec(src).unwrap_or_else(|e| panic!("{}", e));
    it.models.values().next().unwrap().clone()
}

fn largest(m: &Model) -> f64 {
    (0..m.len()).map(|i| m.h[i].abs().max(m.adj[i].iter().map(|e| e.1.abs()).fold(0.0, f64::max))).fold(0.0, f64::max)
}

fn anneal_ok(m: &Model, name: &str, seed: u64, temp: f64, sweeps: usize) -> bool {
    let mut st = State::new(seed);
    st.temp = temp;
    st.anneal(m, sweeps);
    judge(m, name, &st.best.as_ref().unwrap().0).unwrap().0
}

/// Keep the calmest of `samples` uniform random arrangements and check it.
fn random_ok(m: &Model, name: &str, seed: u64, samples: usize) -> bool {
    let mut r = Rng::new(9_000 + seed);
    let mut best = (Vec::new(), f64::INFINITY);
    for _ in 0..samples {
        let s: Vec<f64> = (0..m.len()).map(|_| if r.unit() < 0.5 { -1.0 } else { 1.0 }).collect();
        let e = m.energy(&s);
        if e < best.1 {
            best = (s, e);
        }
    }
    judge(m, name, &best.0).unwrap().0
}

fn pct(k: usize, n: usize) -> String {
    if n == 0 {
        "n/a".into()
    } else {
        format!("{:.0}% ({}/{})", 100.0 * k as f64 / n as f64, k, n)
    }
}

// ---------------------------------------------------------------------------------------------------------
// instances
// ---------------------------------------------------------------------------------------------------------

struct SudokuInst {
    band: &'static str,
    grid: [u8; 81],
    solution: [u8; 81],
    givens: usize,
    nodes: u64,
}

const PER_BAND: usize = 5;

/// Five puzzles per band, from generator seeds 1, 2, 3, ... in order: G30 (carving stopped at 30 givens),
/// SINGLES (minimal, naked and hidden singles finish it) and GUESS (minimal, singles stall).
fn sudoku_instances() -> Vec<SudokuInst> {
    let s = Sudoku::default();
    let mut out: Vec<SudokuInst> = Vec::new();
    let mut seed = 1u64;
    let count = |out: &Vec<SudokuInst>, b: &str| out.iter().filter(|x| x.band == b).count();
    while count(&out, "G30") < PER_BAND || count(&out, "SINGLES") < PER_BAND || count(&out, "GUESS") < PER_BAND {
        let mut r = Rng::new(seed);
        let full = s.full_grid(&mut r);
        let (band, g) = if count(&out, "G30") < PER_BAND {
            ("G30", s.carve(&full, &mut r, Some(30)))
        } else {
            let g = s.carve(&full, &mut r, None);
            (if s.singles_only(&g) { "SINGLES" } else { "GUESS" }, g)
        };
        seed += 1;
        if count(&out, band) >= PER_BAND {
            continue;
        }
        let res = s.solve(&g, 2);
        assert_eq!(res.count, 1, "carved puzzles are unique");
        assert_eq!(res.first.unwrap(), full);
        out.push(SudokuInst { band, grid: g, solution: full, givens: g.iter().filter(|&&d| d > 0).count(), nodes: res.nodes });
    }
    out
}

/// A copy of a GUESS puzzle with one given changed so that no two givens clash but no solution exists.
fn unsolvable(p: &[u8; 81]) -> [u8; 81] {
    let s = Sudoku::default();
    let us = units();
    for i in 0..81 {
        if p[i] == 0 {
            continue;
        }
        for d in 1..=9u8 {
            if d == p[i] {
                continue;
            }
            let mut q = *p;
            q[i] = d;
            let clash = us.iter().any(|u| u.contains(&i) && u.iter().any(|&j| j != i && q[j] == d));
            if !clash && s.solve(&q, 1).count == 0 {
                return q;
            }
        }
    }
    panic!("no single-given change makes this puzzle unsolvable");
}

struct ColInst {
    n: usize,
    c: f64,
    k: usize,
    edges: Vec<(usize, usize)>,
    exact: Option<bool>,
}

const DEGREES: [f64; 6] = [3.0, 4.0, 4.2, 4.4, 4.6, 5.0];
const PER_CELL: usize = 10;

fn colouring_instances() -> Vec<ColInst> {
    let mut jobs = Vec::new();
    for n in [40usize, 80, 160] {
        for &c in &DEGREES {
            for k in 0..PER_CELL {
                jobs.push((n, c, k));
            }
        }
    }
    par(&jobs, |&(n, c, k)| {
        let m = (c * n as f64 / 2.0).round() as usize;
        let mut r = Rng::new(100_000 * n as u64 + (c * 10.0) as u64 * 100 + k as u64);
        let edges = gnm(n, m, &mut r);
        let exact = colourable(n, &edges, 3, 50_000_000);
        ColInst { n, c, k, edges, exact }
    })
}

fn part_instances() {
    stamp("instances start");
    let t0 = Instant::now();
    let si = sudoku_instances();
    println!("\n| band | # | givens | branch nodes (unique proof) | puzzle |\n|---|---|---|---|---|");
    for (i, x) in si.iter().enumerate() {
        println!("| {} | {} | {} | {} | `{}` |", x.band, i % PER_BAND + 1, x.givens, x.nodes, grid_text(&x.grid));
    }
    let bad = unsolvable(&si.iter().find(|x| x.band == "GUESS").unwrap().grid);
    println!("\nunsolvable control (GUESS #1 with one given changed, no clash): `{}`", grid_text(&bad));
    println!("sudoku instances: {:.1}s", t0.elapsed().as_secs_f64());
    let t1 = Instant::now();
    let ci = colouring_instances();
    println!("\n| n | avg degree | colourable | not | undecided |\n|---|---|---|---|---|");
    for n in [40usize, 80, 160] {
        for &c in &DEGREES {
            let cell: Vec<&ColInst> = ci.iter().filter(|x| x.n == n && x.c == c).collect();
            let yes = cell.iter().filter(|x| x.exact == Some(true)).count();
            let no = cell.iter().filter(|x| x.exact == Some(false)).count();
            println!("| {} | {} | {} | {} | {} |", n, c, yes, no, cell.len() - yes - no);
        }
    }
    println!("colouring instances: {:.1}s", t1.elapsed().as_secs_f64());
    stamp("instances end");
}

// ---------------------------------------------------------------------------------------------------------
// sizes
// ---------------------------------------------------------------------------------------------------------

fn next_prime(x: u64) -> u64 {
    (x..).find(|&p| is_prime(p)).unwrap()
}

/// Balanced semiprimes p x q with q the next prime after p.
fn ladder() -> Vec<(u64, u64, u64)> {
    [11u64, 17, 29, 59, 101, 197, 397, 797, 1601, 3203, 6421]
        .iter()
        .map(|&s| {
            let p = next_prime(s);
            let q = next_prime(p + 1);
            (p * q, p, q)
        })
        .collect()
}

fn part_sizes() {
    println!("\n| N | p x q | encoding | things | pulls | largest | smallest | spread | T = largest/10 |\n|---|---|---|---|---|---|---|---|---|");
    for (n, p, q) in ladder() {
        for enc in ["columns", "rosenberg"] {
            if enc == "rosenberg" && n > 1_000_000 {
                continue;
            }
            let m = build(&format!("model :p do\n  factor :f, number: {}, encoding: :{}\nend", n, enc));
            let mut v: Vec<f64> = m.h.iter().map(|x| x.abs()).collect();
            let mut pulls = 0;
            for i in 0..m.len() {
                for &(k, w) in &m.adj[i] {
                    if k > i {
                        pulls += 1;
                        v.push(w.abs());
                    }
                }
            }
            v.retain(|&x| x > 1e-12);
            let big = v.iter().cloned().fold(0.0, f64::max);
            let small = v.iter().cloned().fold(f64::INFINITY, f64::min);
            println!("| {} | {} x {} | {} | {} | {} | {} | {} | {:.0} | {:.1} |", n, p, q, enc, m.len(), pulls, big, small, big / small, largest(&m) / 10.0);
        }
    }
}

// ---------------------------------------------------------------------------------------------------------
// sudoku
// ---------------------------------------------------------------------------------------------------------

const SUDOKU_SEEDS: u64 = 10;

fn part_sudoku() {
    stamp("sudoku start");
    let si = sudoku_instances();
    let models: Vec<Model> = si.iter().map(|x| build(&format!("model :p do\n  sudoku :s, size: 9, given: \"{}\"\nend", grid_text(&x.grid)))).collect();
    println!("\n| band | sweeps | valid of 50 (5 puzzles x 10 seeds) | per puzzle | equals the solver's solution | seconds |\n|---|---|---|---|---|---|");
    for band in ["G30", "SINGLES", "GUESS"] {
        for sw in [10_000usize, 50_000, 200_000] {
            let t0 = Instant::now();
            let jobs: Vec<(usize, u64)> = (0..si.len()).filter(|&i| si[i].band == band).flat_map(|i| (0..SUDOKU_SEEDS).map(move |s| (i, s))).collect();
            let res = par(&jobs, |&(i, seed)| {
                let m = &models[i];
                let mut st = State::new(1_000 + seed);
                st.anneal(m, sw);
                let b = &st.best.as_ref().unwrap().0;
                let ok = judge(m, "s", b).unwrap().0;
                let (a, l) = span(m, "s").unwrap();
                let grid = sudoku_decode(9, &b[a..a + l].iter().map(|&v| v > 0.0).collect::<Vec<_>>());
                (i, ok, ok && grid.as_slice() == si[i].solution.as_slice())
            });
            let ok = res.iter().filter(|r| r.1).count();
            let same = res.iter().filter(|r| r.2).count();
            let per: Vec<String> = (0..si.len()).filter(|&i| si[i].band == band).map(|i| res.iter().filter(|r| r.0 == i && r.1).count().to_string()).collect();
            println!("| {} | {} | {} | {} | {} of {} | {:.1} |", band, sw, pct(ok, res.len()), per.join("/"), same, ok, t0.elapsed().as_secs_f64());
        }
    }
    // random baseline and the unsolvable control
    let jobs: Vec<(usize, u64)> = (0..si.len()).filter(|&i| si[i].band == "GUESS").flat_map(|i| (0..SUDOKU_SEEDS).map(move |s| (i, s))).collect();
    let rnd = par(&jobs, |&(i, seed)| random_ok(&models[i], "s", seed, 50_000));
    println!("| GUESS random, keep calmest | 50000 | {} | | | |", pct(rnd.iter().filter(|&&x| x).count(), rnd.len()));
    let bad = unsolvable(&si.iter().find(|x| x.band == "GUESS").unwrap().grid);
    assert_eq!(Sudoku::default().solve(&bad, 1).count, 0);
    let mb = build(&format!("model :p do\n  sudoku :s, size: 9, given: \"{}\"\nend", grid_text(&bad)));
    let seeds: Vec<u64> = (0..50).collect();
    let neg = par(&seeds, |&s| anneal_ok(&mb, "s", 1_000 + s, 1.0, 200_000));
    println!("| NEG unsolvable (no clashing givens) | 200000 | {} | | | |", pct(neg.iter().filter(|&&x| x).count(), neg.len()));
    stamp("sudoku end");
}

// ---------------------------------------------------------------------------------------------------------
// colouring
// ---------------------------------------------------------------------------------------------------------

const COL_SEEDS: u64 = 5;

fn part_colouring() {
    stamp("colouring start");
    let ci = colouring_instances();
    let models: Vec<Model> = ci.iter().map(|x| build(&format!("model :p do\n  colouring :g, colours: 3, edges: \"{}\"\nend", edges_text(&x.edges)))).collect();
    println!("\n| n | avg degree | colourable / not / undecided | sweeps | proper, colourable instances | proper, NOT colourable (control) | seconds |\n|---|---|---|---|---|---|---|");
    for n in [40usize, 80, 160] {
        for &c in &DEGREES {
            let idx: Vec<usize> = (0..ci.len()).filter(|&i| ci[i].n == n && ci[i].c == c).collect();
            let yes = idx.iter().filter(|&&i| ci[i].exact == Some(true)).count();
            let no = idx.iter().filter(|&&i| ci[i].exact == Some(false)).count();
            for sw in [2_000usize, 20_000, 200_000] {
                let t0 = Instant::now();
                let jobs: Vec<(usize, u64)> = idx.iter().flat_map(|&i| (0..COL_SEEDS).map(move |s| (i, s))).collect();
                let res = par(&jobs, |&(i, s)| (i, anneal_ok(&models[i], "g", 1_000 + s, 1.0, sw)));
                let on = |want: Option<bool>| {
                    let r: Vec<&(usize, bool)> = res.iter().filter(|r| ci[r.0].exact == want).collect();
                    pct(r.iter().filter(|r| r.1).count(), r.len())
                };
                println!("| {} | {} | {} / {} / {} | {} | {} | {} | {:.1} |", n, c, yes, no, idx.len() - yes - no, sw, on(Some(true)), on(Some(false)), t0.elapsed().as_secs_f64());
            }
        }
        let _ = ci[0].k;
    }
    // random baseline on the easiest cell of each size
    for n in [40usize, 80, 160] {
        let idx: Vec<usize> = (0..ci.len()).filter(|&i| ci[i].n == n && ci[i].c == 3.0 && ci[i].exact == Some(true)).collect();
        let jobs: Vec<(usize, u64)> = idx.iter().flat_map(|&i| (0..COL_SEEDS).map(move |s| (i, s))).collect();
        let res = par(&jobs, |&(i, s)| random_ok(&models[i], "g", s, 20_000));
        println!("| {} | 3.0 random, keep calmest | | 20000 | {} | | |", n, pct(res.iter().filter(|&&x| x).count(), res.len()));
    }
    stamp("colouring end");
}

// ---------------------------------------------------------------------------------------------------------
// factor
// ---------------------------------------------------------------------------------------------------------

fn part_factor() {
    stamp("factor start");
    println!("\n| N | encoding | things | T = largest/10 | sweeps | valid of 50 | random keep-calmest, 10000 | seconds |\n|---|---|---|---|---|---|---|---|");
    let seeds: Vec<u64> = (0..50).collect();
    let mut dead = HashMap::new();
    for (n, _, _) in ladder() {
        for enc in ["columns", "rosenberg"] {
            if (enc == "rosenberg" && n > 1_000_000) || dead.get(enc).copied().unwrap_or(0) >= 2 {
                continue;
            }
            let m = build(&format!("model :p do\n  factor :f, number: {}, encoding: :{}\nend", n, enc));
            let t = largest(&m) / 10.0;
            let rnd = par(&seeds, |&s| random_ok(&m, "f", s, 10_000));
            let mut last = 0;
            for sw in [1_000usize, 10_000, 100_000] {
                let t0 = Instant::now();
                let r = par(&seeds, |&s| anneal_ok(&m, "f", 1_000 + s, t, sw));
                last = r.iter().filter(|&&x| x).count();
                println!("| {} | {} | {} | {:.1} | {} | {} | {} | {:.1} |", n, enc, m.len(), t, sw, pct(last, 50), pct(rnd.iter().filter(|&&x| x).count(), 50), t0.elapsed().as_secs_f64());
            }
            if last == 0 {
                *dead.entry(enc).or_insert(0) += 1;
            }
        }
    }
    // negative controls: the largest prime below each semiprime, column encoding, 100,000 sweeps
    for (n, _, _) in ladder().into_iter().take(8) {
        let pr = prime_below(n);
        let m = build(&format!("model :p do\n  factor :f, number: {}, encoding: :columns\nend", pr));
        let t = largest(&m) / 10.0;
        let r = par(&seeds, |&s| anneal_ok(&m, "f", 1_000 + s, t, 100_000));
        println!("| NEG prime {} | columns | {} | {:.1} | 100000 | {} | | |", pr, m.len(), t, pct(r.iter().filter(|&&x| x).count(), 50));
    }
    stamp("factor end");
}

/// Follow-up (sealed after the main ladder): one million sweeps, 20 seeds, where 100,000 found nothing.
fn part_factorlong() {
    stamp("factorlong start");
    println!("\n| N | encoding | things | T | sweeps | valid of 20 | seconds |\n|---|---|---|---|---|---|---|");
    let seeds: Vec<u64> = (0..20).collect();
    let which: Vec<u64> = std::env::var("ZOOHARD_LONG").ok().map(|v| v.split(",").map(|x| x.parse().unwrap()).collect()).unwrap_or(vec![644_773, 2_572_807]);
    for n in which {
        let m = build(&format!("model :p do\n  factor :f, number: {}, encoding: :columns\nend", n));
        let t = largest(&m) / 10.0;
        let t0 = Instant::now();
        let r = par(&seeds, |&s| anneal_ok(&m, "f", 1_000 + s, t, 1_000_000));
        println!("| {} | columns | {} | {:.1} | 1000000 | {} | {:.0} |", n, m.len(), t, pct(r.iter().filter(|&&x| x).count(), 20), t0.elapsed().as_secs_f64());
    }
    stamp("factorlong end");
}

// ---------------------------------------------------------------------------------------------------------
// shared
// ---------------------------------------------------------------------------------------------------------

fn each_ok(m: &Model, name: &str, seed: u64, temp: f64, sweeps: usize) -> bool {
    let mut m2 = m.clone();
    let mut st = State::new(seed);
    st.temp = temp;
    anneal_each(&mut m2, &mut st, sweeps);
    let (s, own) = judged_arrangement(&m2, &st, name).unwrap();
    assert!(own);
    judge(&m2, name, &s).unwrap().0
}

fn part_shared() {
    stamp("shared start");
    println!("\n| case | T | sweeps | alone, anneal | shared, anneal | shared, anneal_each |\n|---|---|---|---|---|---|");
    let seeds: Vec<u64> = (0..50).collect();
    let cases = [
        ("factor 143 (rosenberg) beside prime 127", "factor :f, number: 143", "factor :g, number: 127", vec![1158.4, 115.8], 10_000usize),
        ("factor 3599 (columns) beside prime 3593", "factor :f, number: 3599, encoding: :columns", "factor :g, number: 3593, encoding: :columns", vec![0.0], 10_000),
    ];
    for (label, a, b, temps, sw) in cases {
        let alone = build(&format!("model :p do\n  {}\nend", a));
        let pair = build(&format!("model :p do\n  {}\n  {}\nend", a, b));
        for t in temps {
            let t = if t == 0.0 { largest(&alone) / 10.0 } else { t };
            let r1 = par(&seeds, |&s| anneal_ok(&alone, "f", 1_000 + s, t, sw));
            let r2 = par(&seeds, |&s| anneal_ok(&pair, "f", 1_000 + s, t, sw));
            let r3 = par(&seeds, |&s| each_ok(&pair, "f", 1_000 + s, t, sw));
            let c = |r: &Vec<bool>| pct(r.iter().filter(|&&x| x).count(), 50);
            println!("| {} | {:.1} | {} | {} | {} | {} |", label, t, sw, c(&r1), c(&r2), c(&r3));
            // control: the prime is never factored, by either walk
            let g1 = par(&seeds, |&s| anneal_ok(&pair, "g", 1_000 + s, t, sw));
            let g2 = par(&seeds, |&s| each_ok(&pair, "g", 1_000 + s, t, sw));
            println!("| NEG the prime in the same model | {:.1} | {} | | {} | {} |", t, sw, c(&g1), c(&g2));
        }
    }
    stamp("shared end");
}

// ---------------------------------------------------------------------------------------------------------
// export and settleside: the equal-sweeps comparison with dwave-samplers
// ---------------------------------------------------------------------------------------------------------

/// (tag, model, temperature, target energy, what the target is)
fn dwave_models() -> Vec<(String, Model, f64, f64, String)> {
    let mut out = Vec::new();
    let base = format!("{}/../runs/backends/out", env!("CARGO_MANIFEST_DIR"));
    for (tag, target, what) in [("mc16", -23.0, "exact ground (SETTLEBACKENDS)"), ("mc64", -196.0, "best known (SETTLEBACKENDS, tabu and SETTLE)")] {
        let d = parse_ising(&std::fs::read_to_string(format!("{}/{}.json", base, tag)).unwrap()).unwrap();
        out.push((tag.to_string(), settle::export::into_model(&d), 1.0, target, what.to_string()));
    }
    for n in [899u64, 3599] {
        let m = build(&format!("model :p do\n  factor :f, number: {}, encoding: :columns\nend", n));
        let lay = column_layout(n);
        let (p, q) = [(29u64, 31u64), (59, 61)][(n == 3599) as usize];
        let y = columns_assign(n, &lay, p, q).or_else(|| columns_assign(n, &lay, q, p)).unwrap();
        let s: Vec<f64> = y.iter().map(|&b| if b { 1.0 } else { -1.0 }).collect();
        out.push((format!("factor{}", n), m.clone(), largest(&m) / 10.0, m.energy(&s), "exact ground (energy of the factorisation)".into()));
    }
    let si = sudoku_instances();
    let g30 = &si.iter().find(|x| x.band == "G30").unwrap();
    let m = build(&format!("model :p do\n  sudoku :s, size: 9, given: \"{}\"\nend", grid_text(&g30.grid)));
    let s: Vec<f64> = (0..729).map(|v| if g30.solution[v / 9] as usize == v % 9 + 1 { 1.0 } else { -1.0 }).collect();
    out.push(("sudokuG30".into(), m.clone(), 1.0, m.energy(&s), "exact ground (energy of the unique solution)".into()));
    out
}

fn part_export(dir: &str) {
    std::fs::create_dir_all(dir).unwrap();
    let mut t = String::from("{\n");
    let ms = dwave_models();
    for (k, (tag, m, temp, target, what)) in ms.iter().enumerate() {
        let d = to_ising(m, &HashMap::new(), *temp);
        std::fs::write(format!("{}/{}.json", dir, tag), ising_json(&d).unwrap()).unwrap();
        t.push_str(&format!("  \"{}\": {{\"T\": {:?}, \"target\": {:?}, \"what\": \"{}\", \"things\": {}}}{}\n", tag, temp, target, what, m.len(), if k + 1 < ms.len() { "," } else { "" }));
    }
    t.push_str("}\n");
    std::fs::write(format!("{}/targets.json", dir), t).unwrap();
    println!("wrote {} models to {}", ms.len(), dir);
}

pub const DW_SWEEPS: [usize; 5] = [10, 100, 1_000, 10_000, 50_000];
pub const DW_READS: u64 = 100;

fn part_settleside(dir: &str) {
    stamp("settleside start");
    let mut rows = Vec::new();
    for (tag, m, temp, target, _) in dwave_models() {
        for &sw in &DW_SWEEPS {
            if tag.starts_with("sudoku") && sw < 1_000 {
                continue;
            }
            let t0 = Instant::now();
            let seeds: Vec<u64> = (0..DW_READS).collect();
            let r = par(&seeds, |&s| {
                let mut st = State::new(5_000 + s);
                st.temp = temp;
                st.anneal(&m, sw);
                (st.best.as_ref().unwrap().1, m.energy(&st.last))
            });
            let hit = |e: f64| (e - target).abs() < 1e-6 * target.abs().max(1.0);
            let hb = r.iter().filter(|x| hit(x.0)).count();
            let hf = r.iter().filter(|x| hit(x.1)).count();
            let bmin = r.iter().map(|x| x.0).fold(f64::INFINITY, f64::min);
            println!("| {} | {} | {} | {} | {} | {:.1} |", tag, sw, hb, hf, bmin, t0.elapsed().as_secs_f64());
            rows.push(format!("{{\"tag\": \"{}\", \"sweeps\": {}, \"reads\": {}, \"best_so_far_hits\": {}, \"final_hits\": {}, \"best\": {:?}}}", tag, sw, DW_READS, hb, hf, bmin));
        }
    }
    std::fs::write(format!("{}/settle_side.json", dir), format!("[\n{}\n]\n", rows.join(",\n"))).unwrap();
    stamp("settleside end");
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    match a.get(1).map(|s| s.as_str()) {
        Some("instances") => part_instances(),
        Some("sizes") => part_sizes(),
        Some("sudoku") => part_sudoku(),
        Some("colouring") => part_colouring(),
        Some("factor") => part_factor(),
        Some("factorlong") => part_factorlong(),
        Some("shared") => part_shared(),
        Some("export") => part_export(a.get(2).map(|s| s.as_str()).unwrap_or("work")),
        Some("settleside") => part_settleside(a.get(2).map(|s| s.as_str()).unwrap_or("work")),
        _ => eprintln!("usage: zoohard_measure instances|sizes|sudoku|colouring|factor|shared|export <dir>|settleside <dir>"),
    }
}
