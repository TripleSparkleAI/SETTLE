//! ZOOTEMP measurements (`src/zootemp.rs`). Report: `runs/zootemp/REPORT_ZOOTEMP.md`.
//!
//! Run: `cargo run --release --example zootemp_measure <part>`, part one of
//!   factor      ZOOHARD's Z3 ladder (columns, Rosenberg, primes) re-run: best-so-far AND final state per row
//!   dwave       ZOOHARD's Z5 SETTLE side (100 reads, seeds 5,000+): energy hits of best and final
//!   grid        column factoring: cold end x schedule length (penalty 2), a hot-end arm, primes at every cold end
//!   long        column factoring at one million sweeps across cold ends (20 seeds)
//!   lambda      column factoring: penalty x cold end at 100,000 sweeps, temperature held at the penalty-2 value
//!   restarts    equal total sweeps: one long anneal against R short ones (ZOOTEMP_TOTAL, ZOOTEMP_N)
//!   sudoku      ZOOHARD's Z1 re-run with the final state; optional given-lean and cold-end arm on minimal puzzles
//!   colouring   ZOOHARD's Z2 re-run with the final state
//!   small       SETTLEZOO's rows (sudoku, colouring, max-cut, Rosenberg factoring, controls) with the final state
//!   shared      ZOOHARD's Z4 with the final state
//!   zlong       ZOOHARD's Z3b and Z3c (one million sweeps, 20 seeds) with the final state
//! Everything is seeded; success counts do not depend on the machine. Wall times are scale only.
//!
//! "best" = the calmest arrangement the walk visited, judged by the plain-code checker (ZOOHARD's measure).
//! "final" = the arrangement the walk ended in, judged by the same checker.

#![allow(clippy::needless_range_loop)] // index loops mirror the equations they measure

use settle::interp::Interp;
use settle::model::{Model, State};
use settle::rng::Rng;
use settle::zoo::*;
use settle::zoohard::*;
use settle::zootemp::*;
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
    std::env::var("ZOOTEMP_THREADS").ok().and_then(|v| v.parse().ok()).unwrap_or(8)
}

fn envu(k: &str, d: u64) -> u64 {
    std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d)
}

fn envlist(k: &str, d: &[u64]) -> Vec<u64> {
    std::env::var(k).ok().map(|v| v.split(',').map(|x| x.trim().parse().unwrap()).collect()).unwrap_or(d.to_vec())
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

fn pct(k: usize, n: usize) -> String {
    if n == 0 {
        "n/a".into()
    } else {
        format!("{:.0}% ({}/{})", 100.0 * k as f64 / n as f64, k, n)
    }
}

fn cnt(k: usize, n: usize) -> String {
    if n == 0 {
        "n/a".into()
    } else {
        format!("{}/{}", k, n)
    }
}

/// One anneal judged both ways: (best ok, final ok, walk).
fn walk_ok(m: &Model, name: &str, seed: u64, sched: &Sched, sweeps: usize, target: Option<f64>) -> (bool, bool, Walk) {
    let mut st = State::new(seed);
    let w = anneal_walk(m, &mut st, sched, sweeps, target);
    let b = judge(m, name, &w.best).unwrap().0;
    let f = judge(m, name, &w.last).unwrap().0;
    (b, f, w)
}

fn both(r: &[(bool, bool)]) -> (usize, usize) {
    (r.iter().filter(|x| x.0).count(), r.iter().filter(|x| x.1).count())
}

// ---------------------------------------------------------------------------------------------------------
// factoring helpers
// ---------------------------------------------------------------------------------------------------------

fn next_prime(x: u64) -> u64 {
    (x..).find(|&p| is_prime(p)).unwrap()
}

/// ZOOHARD's balanced semiprimes p x q with q the next prime after p.
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

fn pq_of(n: u64) -> (u64, u64) {
    ladder().into_iter().find(|x| x.0 == n).map(|x| (x.1, x.2)).expect("a ladder semiprime")
}

fn col_model(n: u64, lambda: f64) -> Model {
    build(&format!("model :p do\n  factor :f, number: {}, encoding: :columns, penalty: {}\nend", n, lambda))
}

/// The model energy of the true factorisation with its carries: the ground energy.
fn col_ground(m: &Model, n: u64) -> f64 {
    let lay = column_layout(n);
    let (p, q) = pq_of(n);
    let y = columns_assign(n, &lay, p, q).or_else(|| columns_assign(n, &lay, q, p)).unwrap();
    let start = m.notes["zoo:f"].0[0] as usize;
    let mut s = vec![-1.0; m.len()];
    for (k, &b) in y.iter().enumerate() {
        s[start + k] = if b { 1.0 } else { -1.0 };
    }
    let e = m.energy(&s);
    assert!(judge(m, "f", &s).unwrap().0);
    e
}

/// Row summary of a factoring cell: best, final, walks that visited the answer, median final excess over the
/// ground energy, and the centre of the visits along the schedule (0 = hot start, 1 = cold end).
fn factor_cell(m: &Model, ground: Option<f64>, seeds: &[u64], sched: &Sched, sweeps: usize) -> String {
    let r = par(seeds, |&s| walk_ok(m, "f", s, sched, sweeps, ground));
    let b = r.iter().filter(|x| x.0).count();
    let f = r.iter().filter(|x| x.1).count();
    let vis = r.iter().filter(|x| x.2.walks_hit > 0).count();
    let mut ex: Vec<f64> = r.iter().map(|x| x.2.last_e - ground.unwrap_or(0.0)).collect();
    ex.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let med = ex[ex.len() / 2];
    let near1 = ex.iter().filter(|&&e| e <= 1.0 + 1e-9).count();
    let mut bins = [0u64; BINS];
    for x in &r {
        for k in 0..BINS {
            bins[k] += x.2.hit_bins[k];
        }
    }
    let tot: u64 = bins.iter().sum();
    // when each walk first reached the answer, as a fraction of its schedule, and the temperature there
    let mut firsts: Vec<f64> = r.iter().filter_map(|x| x.2.first_hit.map(|f| f as f64 / sweeps.max(1) as f64)).collect();
    firsts.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let first = if firsts.is_empty() {
        "-".to_string()
    } else {
        let f = firsts[firsts.len() / 2];
        format!("{:.2} (T {:.3})", f, sched.temp((f * sweeps as f64) as usize, sweeps))
    };
    let tail = if tot == 0 { "-".to_string() } else { format!("{:.2}", bins[BINS - 4..].iter().sum::<u64>() as f64 / tot as f64) };
    let n = seeds.len();
    format!("{} | {} | {} | {} | {} | {} | {}", cnt(b, n), cnt(f, n), cnt(vis, n), med, near1, first, tail)
}

const CELL_HEAD: &str = "best | final | visited | median final excess | final excess <= 1 | median first visit, fraction of schedule (T there) | share of at-answer sweeps in last 20%";

// ---------------------------------------------------------------------------------------------------------
// factor (ZOOHARD Z3 with the final state)
// ---------------------------------------------------------------------------------------------------------

fn part_factor() {
    stamp("factor start");
    println!("\n| N | encoding | things | T | sweeps | best of 50 | final of 50 | seconds |\n|---|---|---|---|---|---|---|---|");
    let seeds: Vec<u64> = (0..50).map(|s| 1_000 + s).collect();
    for (n, _, _) in ladder() {
        for enc in ["columns", "rosenberg"] {
            if enc == "rosenberg" && n > 10_403 {
                continue;
            }
            if n > 2_572_807 {
                continue;
            }
            let m = build(&format!("model :p do\n  factor :f, number: {}, encoding: :{}\nend", n, enc));
            let t = largest(&m) / 10.0;
            for sw in [1_000usize, 10_000, 100_000] {
                let t0 = Instant::now();
                let r: Vec<(bool, bool)> = par(&seeds, |&s| {
                    let x = walk_ok(&m, "f", s, &Sched::zoo(t), sw, None);
                    (x.0, x.1)
                });
                let (b, f) = both(&r);
                println!("| {} | {} | {} | {:.1} | {} | {} | {} | {:.1} |", n, enc, m.len(), t, sw, pct(b, 50), pct(f, 50), t0.elapsed().as_secs_f64());
            }
        }
    }
    for (n, _, _) in ladder().into_iter().take(8) {
        let pr = prime_below(n);
        let m = build(&format!("model :p do\n  factor :f, number: {}, encoding: :columns\nend", pr));
        let t = largest(&m) / 10.0;
        let r: Vec<(bool, bool)> = par(&seeds, |&s| {
            let x = walk_ok(&m, "f", s, &Sched::zoo(t), 100_000, None);
            (x.0, x.1)
        });
        let (b, f) = both(&r);
        println!("| NEG prime {} | columns | {} | {:.1} | 100000 | {} | {} | |", pr, m.len(), t, pct(b, 50), pct(f, 50));
    }
    stamp("factor end");
}

fn part_zlong() {
    stamp("zlong start");
    println!("\n| N | things | T | sweeps | best of 20 | final of 20 | seconds |\n|---|---|---|---|---|---|---|");
    let seeds: Vec<u64> = (0..20).map(|s| 1_000 + s).collect();
    for n in envlist("ZOOTEMP_LONG", &[644_773, 2_572_807, 10_278_427]) {
        let m = col_model(n, 2.0);
        let t = largest(&m) / 10.0;
        let t0 = Instant::now();
        let r: Vec<(bool, bool)> = par(&seeds, |&s| {
            let x = walk_ok(&m, "f", s, &Sched::zoo(t), 1_000_000, None);
            (x.0, x.1)
        });
        let (b, f) = both(&r);
        println!("| {} | {} | {:.1} | 1000000 | {} | {} | {:.0} |", n, m.len(), t, pct(b, 20), pct(f, 20), t0.elapsed().as_secs_f64());
    }
    stamp("zlong end");
}

// ---------------------------------------------------------------------------------------------------------
// dwave (ZOOHARD Z5 SETTLE side)
// ---------------------------------------------------------------------------------------------------------

fn part_dwave() {
    stamp("dwave start");
    let base = format!("{}/../runs/backends/out", env!("CARGO_MANIFEST_DIR"));
    let mut ms: Vec<(String, Model, f64, f64)> = Vec::new();
    for (tag, target) in [("mc16", -23.0), ("mc64", -196.0)] {
        let d = settle::export::parse_ising(&std::fs::read_to_string(format!("{}/{}.json", base, tag)).unwrap()).unwrap();
        ms.push((tag.to_string(), settle::export::into_model(&d), 1.0, target));
    }
    for n in [899u64, 3599] {
        let m = col_model(n, 2.0);
        let g = col_ground(&m, n);
        let t = largest(&m) / 10.0;
        ms.push((format!("factor{}", n), m, t, g));
    }
    println!("\n| model | sweeps | best-so-far hits of 100 | final hits of 100 | seconds |\n|---|---|---|---|---|");
    for (tag, m, temp, target) in &ms {
        for sw in [10usize, 100, 1_000, 10_000, 50_000] {
            let t0 = Instant::now();
            let seeds: Vec<u64> = (0..100).map(|s| 5_000 + s).collect();
            let r = par(&seeds, |&s| {
                let mut st = State::new(s);
                let w = anneal_walk(m, &mut st, &Sched::zoo(*temp), sw, None);
                (w.best_e, w.last_e)
            });
            let hit = |e: f64| (e - target).abs() < 1e-6 * target.abs().max(1.0);
            println!("| {} | {} | {} | {} | {:.1} |", tag, sw, r.iter().filter(|x| hit(x.0)).count(), r.iter().filter(|x| hit(x.1)).count(), t0.elapsed().as_secs_f64());
        }
    }
    stamp("dwave end");
}

// ---------------------------------------------------------------------------------------------------------
// grid, long, lambda: the column encoding's dials
// ---------------------------------------------------------------------------------------------------------

const COLDS: [f64; 4] = [0.2, 0.05, 0.01, 0.002];
const GRID_N: [u64; 7] = [143, 323, 899, 3_599, 10_403, 39_203, 159_197];

fn part_grid() {
    stamp("grid start");
    let seeds: Vec<u64> = (0..50).map(|s| 1_000 + s).collect();
    println!("\nCold end x schedule length, penalty 2, hot end 10 T, T = largest / 10 (ZOOHARD's rule), 50 seeds.");
    println!("Cold end is a multiple of T (0.05 = ZOOHARD's T/20).\n");
    println!("| N | things | T | cold x T | sweeps | {} | seconds |\n|---|---|---|---|---|---|---|---|---|---|---|---|", CELL_HEAD);
    for &n in &GRID_N {
        let m = col_model(n, 2.0);
        let t = largest(&m) / 10.0;
        let g = col_ground(&m, n);
        for &c in &COLDS {
            for sw in [1_000usize, 10_000, 100_000] {
                let t0 = Instant::now();
                let row = factor_cell(&m, Some(g), &seeds, &Sched::span(t, 10.0, c), sw);
                println!("| {} | {} | {:.2} | {} | {} | {} | {:.1} |", n, m.len(), t, c, sw, row, t0.elapsed().as_secs_f64());
            }
        }
    }
    println!("\nHot-end arm: cold 0.05 T, 100,000 sweeps, hot end x T varied.\n");
    println!("| N | hot x T | {} |\n|---|---|---|---|---|---|---|---|---|", CELL_HEAD);
    for n in [899u64, 3_599, 10_403, 39_203] {
        let m = col_model(n, 2.0);
        let t = largest(&m) / 10.0;
        let g = col_ground(&m, n);
        for hot in [2.0, 10.0, 50.0] {
            println!("| {} | {} | {} |", n, hot, factor_cell(&m, Some(g), &seeds, &Sched::span(t, hot, 0.05), 100_000));
        }
    }
    println!("\nNegative control: the largest prime below each N, 100,000 sweeps, 20 seeds, every cold end.\n");
    println!("| prime | cold x T | best of 20 | final of 20 |\n|---|---|---|---|");
    let s20: Vec<u64> = (0..20).map(|s| 1_000 + s).collect();
    for &n in &GRID_N {
        let pr = prime_below(n);
        let m = build(&format!("model :p do\n  factor :f, number: {}, encoding: :columns\nend", pr));
        let t = largest(&m) / 10.0;
        for &c in &COLDS {
            let r: Vec<(bool, bool)> = par(&s20, |&s| {
                let x = walk_ok(&m, "f", s, &Sched::span(t, 10.0, c), 100_000, None);
                (x.0, x.1)
            });
            let (b, f) = both(&r);
            println!("| {} | {} | {} | {} |", pr, c, cnt(b, 20), cnt(f, 20));
        }
    }
    stamp("grid end");
}

fn part_long() {
    stamp("long start");
    let seeds: Vec<u64> = (0..envu("ZOOTEMP_SEEDS", 20)).map(|s| 1_000 + s).collect();
    let sw = envu("ZOOTEMP_SWEEPS", 1_000_000) as usize;
    println!("\nOne long schedule, penalty 2, hot 10 T, {} seeds, {} sweeps.\n", seeds.len(), sw);
    println!("| N | things | T | cold x T | {} | seconds |\n|---|---|---|---|---|---|---|---|---|---|---|---|", CELL_HEAD);
    for n in envlist("ZOOTEMP_N", &[3_599, 10_403, 39_203, 159_197]) {
        let m = col_model(n, 2.0);
        let t = largest(&m) / 10.0;
        let g = col_ground(&m, n);
        for c in [0.05, 0.01, 0.002] {
            let t0 = Instant::now();
            println!("| {} | {} | {:.2} | {} | {} | {:.0} |", n, m.len(), t, c, factor_cell(&m, Some(g), &seeds, &Sched::span(t, 10.0, c), sw), t0.elapsed().as_secs_f64());
        }
    }
    stamp("long end");
}

fn part_lambda() {
    stamp("lambda start");
    let seeds: Vec<u64> = (0..50).map(|s| 1_000 + s).collect();
    println!("\nPenalty x cold end, 100,000 sweeps, 50 seeds. T held at the penalty-2 value of largest / 10.\n");
    println!("| N | penalty | largest at this penalty | T | cold x T | {} | seconds |\n|---|---|---|---|---|---|---|---|---|---|---|---|---|", CELL_HEAD);
    for n in [899u64, 3_599, 10_403, 39_203] {
        let t = largest(&col_model(n, 2.0)) / 10.0;
        for lambda in [0.5, 1.0, 2.0, 4.0, 8.0] {
            let m = col_model(n, lambda);
            let g = col_ground(&m, n);
            for c in [0.05, 0.01] {
                let t0 = Instant::now();
                println!("| {} | {} | {} | {:.2} | {} | {} | {:.1} |", n, lambda, largest(&m), t, c, factor_cell(&m, Some(g), &seeds, &Sched::span(t, 10.0, c), 100_000), t0.elapsed().as_secs_f64());
            }
        }
    }
    stamp("lambda end");
}

// ---------------------------------------------------------------------------------------------------------
// restarts
// ---------------------------------------------------------------------------------------------------------

fn part_restarts() {
    stamp("restarts start");
    let total = envu("ZOOTEMP_TOTAL", 1_000_000) as usize;
    let seeds: Vec<u64> = (0..envu("ZOOTEMP_SEEDS", 20)).map(|s| 1_000 + s).collect();
    let rs = envlist("ZOOTEMP_R", &[1, 10, 100, 1_000]);
    let cold = std::env::var("ZOOTEMP_COLD").ok().and_then(|v| v.parse().ok()).unwrap_or(0.05);
    let hot: f64 = std::env::var("ZOOTEMP_HOT").ok().and_then(|v| v.parse().ok()).unwrap_or(10.0);
    // ZOOTEMP_BASE: an absolute base temperature for every N instead of ZOOHARD's largest / 10
    let base: Option<f64> = std::env::var("ZOOTEMP_BASE").ok().and_then(|v| v.parse().ok());
    println!(
        "\nEqual total sweeps {}: R walks of {} / R sweeps each, from {} x T to {} x T within every walk, T = {}.",
        total,
        total,
        hot,
        cold,
        base.map(|b| format!("{} for every N", b)).unwrap_or("largest / 10 (ZOOHARD's rule)".into())
    );
    println!("best = calmest arrangement any walk visited; final = calmest of the R end states; walks hit = walks that visited the answer.\n");
    println!("| N | things | R | sweeps per walk | best of {n} | final of {n} | walks that visited, all seeds | walks that ended at the answer, all seeds | seconds |\n|---|---|---|---|---|---|---|---|---|", n = seeds.len());
    for n in envlist("ZOOTEMP_N", &[3_599, 10_403, 39_203, 159_197, 644_773]) {
        let m = col_model(n, 2.0);
        let t = base.unwrap_or(largest(&m) / 10.0);
        let g = col_ground(&m, n);
        for &r in &rs {
            let r = r as usize;
            let t0 = Instant::now();
            let res = par(&seeds, |&s| {
                let mut st = State::new(s);
                let w = restarts(&m, &mut st, &Sched::span(t, hot, cold), total, r, Some(g));
                let b = judge(&m, "f", &w.best).unwrap().0;
                let f = judge(&m, "f", &w.last).unwrap().0;
                let ends_at = w.ends.iter().filter(|&&e| (e - g).abs() < 1e-6).count();
                (b, f, w.walks_hit, ends_at)
            });
            let b = res.iter().filter(|x| x.0).count();
            let f = res.iter().filter(|x| x.1).count();
            let wh: usize = res.iter().map(|x| x.2).sum();
            let we: usize = res.iter().map(|x| x.3).sum();
            let nw = r * seeds.len();
            println!("| {} | {} | {} | {} | {} | {} | {} | {} | {:.0} |", n, m.len(), r, total / r, cnt(b, seeds.len()), cnt(f, seeds.len()), cnt(wh, nw), cnt(we, nw), t0.elapsed().as_secs_f64());
        }
    }
    stamp("restarts end");
}

// ---------------------------------------------------------------------------------------------------------
// sudoku and colouring (ZOOHARD Z1, Z2 with the final state); instance generation copied from zoohard_measure
// ---------------------------------------------------------------------------------------------------------

struct SudokuInst {
    band: &'static str,
    grid: [u8; 81],
    solution: [u8; 81],
}

const PER_BAND: usize = 5;

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
        out.push(SudokuInst { band, grid: g, solution: full });
    }
    out
}

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

fn sudoku_model(g: &[u8; 81], extra: &str) -> Model {
    build(&format!("model :p do\n  sudoku :s, size: 9, given: \"{}\"{}\nend", grid_text(g), extra))
}

fn part_sudoku() {
    stamp("sudoku start");
    let si = sudoku_instances();
    let models: Vec<Model> = si.iter().map(|x| sudoku_model(&x.grid, "")).collect();
    println!("\n| band | sweeps | best, valid of 50 | final, valid of 50 | best per puzzle | final per puzzle | valid finals equal to the solver's solution | seconds |\n|---|---|---|---|---|---|---|---|");
    for band in ["G30", "SINGLES", "GUESS"] {
        for sw in [10_000usize, 50_000, 200_000] {
            let t0 = Instant::now();
            let jobs: Vec<(usize, u64)> = (0..si.len()).filter(|&i| si[i].band == band).flat_map(|i| (0..10u64).map(move |s| (i, s))).collect();
            let res = par(&jobs, |&(i, seed)| {
                let m = &models[i];
                let (b, f, w) = walk_ok(m, "s", 1_000 + seed, &Sched::zoo(1.0), sw, None);
                let (a, l) = span(m, "s").unwrap();
                let grid = sudoku_decode(9, &w.last[a..a + l].iter().map(|&v| v > 0.0).collect::<Vec<_>>());
                (i, b, f, f && grid.as_slice() == si[i].solution.as_slice())
            });
            let b = res.iter().filter(|r| r.1).count();
            let f = res.iter().filter(|r| r.2).count();
            let same = res.iter().filter(|r| r.3).count();
            let per = |k: usize| -> String { (0..si.len()).filter(|&i| si[i].band == band).map(|i| res.iter().filter(|r| r.0 == i && if k == 1 { r.1 } else { r.2 }).count().to_string()).collect::<Vec<_>>().join("/") };
            println!("| {} | {} | {} | {} | {} | {} | {} of {} | {:.1} |", band, sw, pct(b, 50), pct(f, 50), per(1), per(2), same, f, t0.elapsed().as_secs_f64());
        }
    }
    let bad = unsolvable(&si.iter().find(|x| x.band == "GUESS").unwrap().grid);
    assert_eq!(Sudoku::default().solve(&bad, 1).count, 0);
    let mb = sudoku_model(&bad, "");
    let seeds: Vec<u64> = (0..50).map(|s| 1_000 + s).collect();
    let neg: Vec<(bool, bool)> = par(&seeds, |&s| {
        let x = walk_ok(&mb, "s", s, &Sched::zoo(1.0), 200_000, None);
        (x.0, x.1)
    });
    let (b, f) = both(&neg);
    println!("| NEG unsolvable (no clashing givens) | 200000 | {} | {} | | | | |", pct(b, 50), pct(f, 50));
    stamp("sudoku end");
}

/// Optional arm: minimal puzzles (SINGLES and GUESS) with a stronger given lean and a colder, slower end.
fn part_sudokuarm() {
    stamp("sudokuarm start");
    let si = sudoku_instances();
    println!("\nMinimal 9x9, 5 puzzles x 10 seeds per band (seeds 1,000+, as ZOOHARD), 200,000 sweeps.\n");
    println!("| band | given lean (x A) | cold x T | best of 50 | final of 50 | seconds |\n|---|---|---|---|---|---|");
    // one arm beside ZOOHARD's (lean 4A, cold 0.05 T): lean 16A (cut from six cells to fit the budget)
    for band in ["SINGLES", "GUESS"] {
        {
            let (g, c) = (16.0, 0.05);
            let models: Vec<Model> = si.iter().map(|x| sudoku_model(&x.grid, &format!(", given_by: {}", g))).collect();
            {
                let t0 = Instant::now();
                let jobs: Vec<(usize, u64)> = (0..si.len()).filter(|&i| si[i].band == band).flat_map(|i| (0..10u64).map(move |s| (i, s))).collect();
                let res: Vec<(bool, bool)> = par(&jobs, |&(i, seed)| {
                    let x = walk_ok(&models[i], "s", 1_000 + seed, &Sched::span(1.0, 10.0, c), 200_000, None);
                    (x.0, x.1)
                });
                let (b, f) = both(&res);
                println!("| {} | {} | {} | {} | {} | {:.0} |", band, g, c, pct(b, res.len()), pct(f, res.len()), t0.elapsed().as_secs_f64());
            }
        }
    }
    stamp("sudokuarm end");
}

struct ColInst {
    n: usize,
    c: f64,
    edges: Vec<(usize, usize)>,
    exact: Option<bool>,
}

const DEGREES: [f64; 6] = [3.0, 4.0, 4.2, 4.4, 4.6, 5.0];

fn colouring_instances() -> Vec<ColInst> {
    let mut jobs = Vec::new();
    for n in [40usize, 80, 160] {
        for &c in &DEGREES {
            for k in 0..10usize {
                jobs.push((n, c, k));
            }
        }
    }
    par(&jobs, |&(n, c, k)| {
        let m = (c * n as f64 / 2.0).round() as usize;
        let mut r = Rng::new(100_000 * n as u64 + (c * 10.0) as u64 * 100 + k as u64);
        let edges = gnm(n, m, &mut r);
        let exact = colourable(n, &edges, 3, 50_000_000);
        ColInst { n, c, edges, exact }
    })
}

fn part_colouring() {
    stamp("colouring start");
    let ci = colouring_instances();
    let models: Vec<Model> = ci.iter().map(|x| build(&format!("model :p do\n  colouring :g, colours: 3, edges: \"{}\"\nend", edges_text(&x.edges)))).collect();
    println!("\n| n | avg degree | colourable / not | sweeps | best, colourable | final, colourable | best, NOT colourable (control) | final, NOT colourable | seconds |\n|---|---|---|---|---|---|---|---|---|");
    for n in [40usize, 80, 160] {
        for &c in &DEGREES {
            let idx: Vec<usize> = (0..ci.len()).filter(|&i| ci[i].n == n && ci[i].c == c).collect();
            let yes = idx.iter().filter(|&&i| ci[i].exact == Some(true)).count();
            let no = idx.iter().filter(|&&i| ci[i].exact == Some(false)).count();
            for sw in [2_000usize, 20_000, 200_000] {
                let t0 = Instant::now();
                let jobs: Vec<(usize, u64)> = idx.iter().flat_map(|&i| (0..5u64).map(move |s| (i, s))).collect();
                let res = par(&jobs, |&(i, s)| {
                    let x = walk_ok(&models[i], "g", 1_000 + s, &Sched::zoo(1.0), sw, None);
                    (i, x.0, x.1)
                });
                let on = |want: Option<bool>, k: usize| {
                    let r: Vec<&(usize, bool, bool)> = res.iter().filter(|r| ci[r.0].exact == want).collect();
                    pct(r.iter().filter(|r| if k == 1 { r.1 } else { r.2 }).count(), r.len())
                };
                println!("| {} | {} | {} / {} | {} | {} | {} | {} | {} | {:.1} |", n, c, yes, no, sw, on(Some(true), 1), on(Some(true), 2), on(Some(false), 1), on(Some(false), 2), t0.elapsed().as_secs_f64());
            }
        }
    }
    stamp("colouring end");
}

// ---------------------------------------------------------------------------------------------------------
// small: SETTLEZOO's rows (generators copied from zoo.rs's measure module)
// ---------------------------------------------------------------------------------------------------------

fn planted(n: usize, edges: usize, seed: u64) -> String {
    let mut r = Rng::new(seed);
    let col: Vec<usize> = (0..n).map(|_| r.below(3)).collect();
    let mut set = std::collections::BTreeSet::new();
    while set.len() < edges {
        let (a, b) = (r.below(n), r.below(n));
        if a != b && col[a] != col[b] {
            set.insert((a.min(b), a.max(b)));
        }
    }
    set.iter().map(|(a, b)| format!("v{}-v{}", a, b)).collect::<Vec<_>>().join(" ")
}

fn gnp(n: usize, seed: u64) -> String {
    let mut r = Rng::new(seed);
    let mut out = Vec::new();
    for a in 0..n {
        for b in a + 1..n {
            if r.unit() < 0.5 {
                out.push(format!("v{}-v{}", a, b));
            }
        }
    }
    out.join(" ")
}

fn sparse(n: usize) -> String {
    let mut r = Rng::new(700 + n as u64);
    let mut e = Vec::new();
    for a in 0..n {
        for b in a + 1..n {
            if r.unit() < 3.0 / (n - 1) as f64 {
                e.push(format!("v{}-v{}", a, b));
            }
        }
    }
    e.join(" ")
}

fn small_row(label: &str, m: &Model, name: &str, sw: usize, t: f64) {
    let seeds: Vec<u64> = (0..50).map(|s| 1_000 + s).collect();
    let r: Vec<(bool, bool)> = par(&seeds, |&s| {
        let x = walk_ok(m, name, s, &Sched::zoo(t), sw, None);
        (x.0, x.1)
    });
    let (b, f) = both(&r);
    println!("| {} | {} | {} | {} | {} |", label, sw, if t == 1.0 { "1".to_string() } else { format!("{:.1}", t) }, pct(b, 50), pct(f, 50));
}

const EASY9: &str = "53..7.... 6..195... .98....6. 8...6...3 4..8.3..1 7...2...6 .6....28. ...419..5 ....8..79";
const BAD9: &str = "55..7.... 6..195... .98....6. 8...6...3 4..8.3..1 7...2...6 .6....28. ...419..5 ....8..79";

fn part_small() {
    stamp("small start");
    println!("\n| SETTLEZOO row | sweeps | T | best of 50 | final of 50 |\n|---|---|---|---|---|");
    let sd = |n: usize, g: &str, extra: &str| build(&format!("model :p do\n  sudoku :s, size: {}, given: \"{}\"{}\nend", n, g, extra));
    let four = sd(4, "1... .4.. ..4. ...1", "");
    for sw in [100, 500, 2_000] {
        small_row("sudoku 4x4, 4 givens, G 4A", &four, "s", sw, 1.0);
    }
    small_row("sudoku 4x4, 4 givens, G A", &sd(4, "1... .4.. ..4. ...1", ", given_by: 1"), "s", 2_000, 1.0);
    let nine = sd(9, EASY9, "");
    for sw in [2_000, 10_000, 50_000] {
        small_row("sudoku 9x9 easy, 30 givens", &nine, "s", sw, 1.0);
    }
    small_row("NEG sudoku 4x4, two 1s in row 1", &sd(4, "1.1. .... .... ....", ""), "s", 2_000, 1.0);
    small_row("NEG sudoku 9x9, two 5s in row 1", &sd(9, BAD9, ""), "s", 10_000, 1.0);
    for n in [10usize, 20, 40, 80] {
        let m = build(&format!("model :p do\n  colouring :g, colours: 3, edges: \"{}\"\nend", planted(n, 2 * n, 77 + n as u64)));
        for sw in [500, 2_000, 10_000] {
            small_row(&format!("colouring planted n={}", n), &m, "g", sw, 1.0);
        }
    }
    for (label, e, k) in [("NEG triangle, 2 colours", "a-b b-c c-a", 2), ("NEG K4, 3 colours", "a-b a-c a-d b-c b-d c-d", 3)] {
        small_row(label, &build(&format!("model :p do\n  colouring :g, colours: {}, edges: \"{}\"\nend", k, e)), "g", 2_000, 1.0);
    }
    for n in [8usize, 12, 16, 20] {
        let m = build(&format!("model :p do\n  maxcut :g, edges: \"{}\"\nend", gnp(n, 500 + n as u64)));
        for sw in [200, 1_000, 5_000] {
            small_row(&format!("max-cut G(n,1/2) n={}", n), &m, "g", sw, 1.0);
        }
    }
    for n in [16usize, 20, 22] {
        let m = build(&format!("model :p do\n  maxcut :g, edges: \"{}\"\nend", sparse(n)));
        for sw in [50, 200, 1_000] {
            small_row(&format!("max-cut sparse degree 3 n={}", n), &m, "g", sw, 1.0);
        }
    }
    small_row("NEG max-cut triangle, target 3", &build("model :p do\n  maxcut :t, edges: \"a-b b-c c-a\", target: 3\nend"), "t", 1_000, 1.0);
    for n in [15u64, 21, 35, 77, 143, 31, 97, 127] {
        let m = build(&format!("model :p do\n  factor :f, number: {}\nend", n));
        let auto = largest(&m) / 10.0;
        let tag = if [31, 97, 127].contains(&n) { "NEG prime " } else { "factor Rosenberg " };
        for (t, sw) in [(auto, 1_000), (auto, 10_000), (5.0, 10_000)] {
            small_row(&format!("{}{}", tag, n), &m, "f", sw, t);
        }
    }
    stamp("small end");
}

// ---------------------------------------------------------------------------------------------------------
// shared (ZOOHARD Z4 with the final state)
// ---------------------------------------------------------------------------------------------------------

fn part_shared() {
    stamp("shared start");
    println!("\n| case | T | sweeps | alone best | alone final | shared anneal best | shared final (same walk for anneal_each) | anneal_each own best |\n|---|---|---|---|---|---|---|---|");
    let seeds: Vec<u64> = (0..50).map(|s| 1_000 + s).collect();
    let cases = [
        ("factor 143 (rosenberg) beside prime 127", "factor :f, number: 143", "factor :g, number: 127", vec![1158.4, 115.8]),
        ("factor 3599 (columns) beside prime 3593", "factor :f, number: 3599, encoding: :columns", "factor :g, number: 3593, encoding: :columns", vec![0.0]),
    ];
    for (label, a, b, temps) in cases {
        let alone = build(&format!("model :p do\n  {}\nend", a));
        let pair = build(&format!("model :p do\n  {}\n  {}\nend", a, b));
        for t in temps {
            let t = if t == 0.0 { largest(&alone) / 10.0 } else { t };
            let r1: Vec<(bool, bool)> = par(&seeds, |&s| {
                let x = walk_ok(&alone, "f", s, &Sched::zoo(t), 10_000, None);
                (x.0, x.1)
            });
            let r2: Vec<(bool, bool)> = par(&seeds, |&s| {
                let x = walk_ok(&pair, "f", s, &Sched::zoo(t), 10_000, None);
                (x.0, x.1)
            });
            let r3: Vec<bool> = par(&seeds, |&s| {
                let mut m2 = pair.clone();
                let mut st = State::new(s);
                st.temp = t;
                anneal_each(&mut m2, &mut st, 10_000);
                let (sb, own) = judged_arrangement(&m2, &st, "f").unwrap();
                assert!(own);
                judge(&m2, "f", &sb).unwrap().0
            });
            let (b1, f1) = both(&r1);
            let (b2, f2) = both(&r2);
            let b3 = r3.iter().filter(|&&x| x).count();
            println!("| {} | {:.1} | 10000 | {} | {} | {} | {} | {} |", label, t, pct(b1, 50), pct(f1, 50), pct(b2, 50), pct(f2, 50), pct(b3, 50));
        }
    }
    stamp("shared end");
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    match a.get(1).map(|s| s.as_str()) {
        Some("factor") => part_factor(),
        Some("zlong") => part_zlong(),
        Some("dwave") => part_dwave(),
        Some("grid") => part_grid(),
        Some("long") => part_long(),
        Some("lambda") => part_lambda(),
        Some("restarts") => part_restarts(),
        Some("sudoku") => part_sudoku(),
        Some("sudokuarm") => part_sudokuarm(),
        Some("colouring") => part_colouring(),
        Some("small") => part_small(),
        Some("shared") => part_shared(),
        Some("speed") => {
            // scale only: sweeps per second on one thread for a few models
            for n in [10_403u64, 159_197, 644_773] {
                let m = col_model(n, 2.0);
                let t = largest(&m) / 10.0;
                let t0 = Instant::now();
                let mut st = State::new(1);
                anneal_walk(&m, &mut st, &Sched::zoo(t), 200_000, None);
                println!("{} things {} : {:.1} us per sweep", n, m.len(), t0.elapsed().as_secs_f64() / 200_000.0 * 1e6);
            }
            stamp("speed");
        }
        _ => eprintln!("usage: zootemp_measure factor|zlong|dwave|grid|long|lambda|restarts|sudoku|sudokuarm|colouring|small|shared|speed"),
    }
}
