//! CORE UPDATE MEASURE: Gibbs against Metropolised Gibbs on core models, against the exact answer.
//!
//!     cargo run --release --example core_update_measure
//!
//! SEALED PREDICTION (written and committed before this program was first run, 2026-10-06, lane SETTLEPERFECT):
//! - P1. On every model and budget below, the mean squared error of the yes-rates against exact enumeration is
//!   no larger under `update: :metro` than under Gibbs (Peskun ordering: Metropolised Gibbs changes a thing at
//!   least as often as Gibbs, with the same stationary distribution).
//! - P2. The ratio metro/gibbs of that error lies between 0.3 and 1.0 on every row.
//! - P3. The time per update differs by less than 30% between the two rules.
//!
//! A row that breaks P1 is reported as a miss, not re-run.
//!
//! The models (small enough to enumerate every arrangement): `weather` (3 things, wet_grass held yes), `ring`
//! (5 things pushing round a frustrated ring), `chain` (12 things, each pulling the next by 0.8, the first leaning
//! yes by 0.5), `dense` (10 things, random leans in [-0.2, 0.2), random pulls in [-0.3, 0.3)). Each budget is run
//! from `SEEDS` seeds; the error is the mean over seeds of the summed squared error over things.

use settle::engine::model::{exact_rates, Model, State, Update};
use settle::engine::rng::Rng;
use std::time::Instant;

const SEEDS: u64 = 300;

fn model(name: &str) -> (Model, Vec<(usize, f64)>) {
    let mut m = Model::default();
    let mut held = Vec::new();
    match name {
        "weather" => {
            for n in ["rain", "sprinkler", "wet_grass"] {
                m.add(n);
            }
            m.h[0] = -1.0;
            m.h[1] = -0.5;
            m.couple(0, 1, -0.5);
            m.couple(0, 2, 1.5);
            m.couple(1, 2, 1.0);
            held.push((2, 1.0));
        }
        "ring" => {
            for n in ["a", "b", "c", "d", "e"] {
                m.add(n);
            }
            m.h[1] = 0.2;
            for k in 0..5 {
                m.couple(k, (k + 1) % 5, -1.0);
            }
            m.couple(0, 2, 0.3);
        }
        "chain" => {
            for i in 0..12 {
                m.add(&format!("x{}", i));
                if i > 0 {
                    m.couple(i - 1, i, 0.8);
                }
            }
            m.h[0] = 0.5;
        }
        _ => {
            let mut r = Rng::new(7);
            for i in 0..10 {
                m.add(&format!("d{}", i));
                m.h[i] = 0.2 * r.signed();
            }
            for i in 0..10 {
                for k in i + 1..10 {
                    m.couple(i, k, 0.3 * r.signed());
                }
            }
        }
    }
    (m, held)
}

fn main() {
    println!(
        "stamp: load {} · power mode {}",
        String::from_utf8_lossy(&std::process::Command::new("sysctl").args(["-n", "vm.loadavg"]).output().map(|o| o.stdout).unwrap_or_default()).trim(),
        String::from_utf8_lossy(&std::process::Command::new("pmset").arg("-g").output().map(|o| o.stdout).unwrap_or_default())
            .lines()
            .find(|l| l.contains("powermode"))
            .and_then(|l| l.split_whitespace().last().map(String::from))
            .unwrap_or_else(|| "n/a".into())
    );
    println!("{:<8} {:>7}  {:>12} {:>12} {:>7}  {:>9} {:>9}", "model", "sweeps", "gibbs mse", "metro mse", "ratio", "gibbs ns", "metro ns");
    let mut misses = 0;
    for name in ["weather", "ring", "chain", "dense"] {
        let (m, held) = model(name);
        let mut st0 = State::new(0);
        for &(i, v) in &held {
            st0.held.insert(i, v);
        }
        let exact = exact_rates(&m, &st0);
        for sweeps in [100usize, 1000] {
            let mut mse = [0.0f64; 2];
            let mut time = [f64::INFINITY; 2];
            for seed in 0..SEEDS {
                for (k, rule) in [Update::Gibbs, Update::Metro].into_iter().enumerate() {
                    let mut st = State::new(1000 + seed);
                    st.held = st0.held.clone();
                    st.update = rule;
                    let t = Instant::now();
                    st.settle(&m, sweeps);
                    time[k] = time[k].min(t.elapsed().as_secs_f64());
                    mse[k] += st.rates().iter().zip(&exact).map(|(a, b)| (a - b).powi(2)).sum::<f64>() / SEEDS as f64;
                }
            }
            let free = (m.len() - held.len()) as f64;
            let per = |k: usize| time[k] / ((sweeps + (sweeps / 10).max(1)) as f64 * free) * 1e9;
            let ratio = mse[1] / mse[0];
            if ratio > 1.0 {
                misses += 1;
            }
            println!("{:<8} {:>7}  {:>12.3e} {:>12.3e} {:>7.3}  {:>9.1} {:>9.1}", name, sweeps, mse[0], mse[1], ratio, per(0), per(1));
        }
    }
    println!("P1 misses (metro worse than gibbs): {}", misses);
}
