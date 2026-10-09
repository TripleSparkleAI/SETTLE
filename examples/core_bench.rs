//! CORE BENCH: how fast the core sampler settles, on four model shapes, with the stamps the measurement laws ask for.
//!
//!     cargo run --release --example core_bench
//!
//! Each workload is settled `REPS` times, the arms interleaved, and the MINIMUM wall time is reported (one-sided
//! noise: a busy machine only ever makes a run slower). A checksum of the yes-counts is printed beside each, so a
//! faster version that changed the samples would show a different number. The machine's load and power mode are
//! printed first; a run taken on a busy machine measures the machine, not the code.
//!
//! Workloads (single thread, the default `State::settle`):
//! - `weather`: 3 things, 2,000,000 sweeps (per-sweep overhead: the shuffle, the bookkeeping)
//! - `chain`: 20,000 things in a line, 200 sweeps, samples not kept (the sample budget)
//! - `grid`: a wrapped 100 x 100 lattice, 4 neighbours each, 200 sweeps, samples not kept
//! - `dense`: 64 things, every pair pulled or pushed, 20,000 sweeps, every sample kept

use settle::engine::model::{Model, State};
use settle::engine::rng::Rng;
use std::process::Command;
use std::time::Instant;

const REPS: usize = 7;

fn weather() -> Model {
    let mut m = Model::default();
    for n in ["rain", "sprinkler", "wet_grass"] {
        m.add(n);
    }
    m.h[0] = -1.0;
    m.h[1] = -0.5;
    m.couple(0, 1, -0.5);
    m.couple(0, 2, 1.5);
    m.couple(1, 2, 1.0);
    m
}

fn chain() -> Model {
    let mut m = Model::default();
    for i in 0..20_000 {
        m.add(&format!("x{}", i));
        if i > 0 {
            m.couple(i - 1, i, 0.8);
        }
    }
    m.h[0] = 2.0;
    m
}

fn grid() -> Model {
    let (w, h) = (100, 100);
    let mut m = Model::default();
    for i in 0..w * h {
        m.add(&format!("p{}", i));
    }
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            m.couple(i, y * w + (x + 1) % w, 0.4);
            m.couple(i, ((y + 1) % h) * w + x, 0.4);
        }
    }
    m
}

fn dense() -> Model {
    let mut m = Model::default();
    let mut r = Rng::new(3);
    for i in 0..64 {
        m.add(&format!("d{}", i));
        m.h[i] = r.signed() * 0.2;
    }
    for i in 0..64 {
        for k in i + 1..64 {
            m.couple(i, k, r.signed() * 0.15);
        }
    }
    m
}

fn sh(cmd: &str, args: &[&str]) -> String {
    Command::new(cmd).args(args).output().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_else(|_| "unknown".into())
}

fn main() {
    let power = sh("pmset", &["-g"]).lines().find(|l| l.contains("powermode")).map(|l| l.split_whitespace().last().unwrap_or("?").to_string());
    println!(
        "stamp: load {} on {} cpus · power mode {} (0 normal, 1 low, 2 high) · os {} · rustc {}",
        sh("sysctl", &["-n", "vm.loadavg"]),
        std::thread::available_parallelism().map(|n| n.get()).unwrap_or(0),
        power.unwrap_or_else(|| "n/a".into()),
        sh("sw_vers", &["-buildVersion"]),
        sh("rustc", &["--version"]),
    );
    let work: Vec<(&str, Model, usize)> = vec![("weather", weather(), 2_000_000), ("chain", chain(), 200), ("grid", grid(), 200), ("dense", dense(), 20_000)];
    let mut best = vec![f64::INFINITY; work.len()];
    let mut sums = vec![0u64; work.len()];
    for _ in 0..REPS {
        for (k, (_, m, sweeps)) in work.iter().enumerate() {
            let mut st = State::new(1);
            let t = Instant::now();
            st.settle(m, *sweeps);
            best[k] = best[k].min(t.elapsed().as_secs_f64());
            sums[k] = st.yes.iter().enumerate().fold(0u64, |h, (i, &c)| h.wrapping_mul(1_000_003).wrapping_add(c ^ i as u64));
        }
    }
    for (k, (name, m, sweeps)) in work.iter().enumerate() {
        let updates = (*sweeps * m.len()) as f64;
        println!(
            "{:<8} {:>6} things x {:>9} sweeps: min {:>8.2} ms of {}, {:>7.1} M updates/s, yes-count checksum {:016x}",
            name,
            m.len(),
            sweeps,
            best[k] * 1e3,
            REPS,
            updates / best[k] / 1e6,
            sums[k]
        );
    }
}
