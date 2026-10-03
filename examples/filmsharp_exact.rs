//! FILMSHARP: every new inversion against the exact answer on a 4x4 open grid (enumeration of all 2^16 states).
//!
//! Run: cargo run --release --example filmsharp_exact
//!
//! Same 20 targets as gridplayer2_tap_exact (Rng 42, greys uniform in [0.1, 0.9]). Columns: RMS grey error of the
//! exact marginals for the leans each inversion gives:
//!   TAP, Bethe                      closed forms
//!   secant20                        h <- h + eta (H_tap(m*) - H_tap(m)), exact m, 20 iterations
//!   precond20                       h <- h + eta P (m* - m), P = PD-shaped TAP inverse response, exact m, 20 its
//!   newton5                         h <- h + C^-1 (m* - m) with the exact covariance, 5 iterations
//!   precond sampled                 as precond, m from a Gibbs chain, 10 iterations of 2,000 sweeps
//!   newton sampled                  as newton, C from 400 samples of that chain (lambda 0.1), 5 x 2,000 sweeps

use settle::filmsharp::{bethe_leans, newton_fit, newton_leans, precond_fit, precond_leans, secant_fit, Response, Update};
use settle::grid::{leans_for, Invert, Spec};
use settle::model::{exact_rates, Model, State};
use settle::rng::Rng;

fn grid(w: usize, h: usize, j: f64) -> (Model, Spec) {
    let mut m = Model::default();
    for y in 0..h {
        for x in 0..w {
            m.add(&format!("p_{}_{}", x, y));
        }
    }
    if j != 0.0 {
        for y in 0..h {
            for x in 0..w {
                let i = y * w + x;
                if x + 1 < w {
                    m.couple(i, i + 1, j);
                }
                if y + 1 < h {
                    m.couple(i, i + w, j);
                }
            }
        }
    }
    (m, Spec { start: 0, w, h })
}

fn rms(a: &[f64], b: &[f64]) -> f64 {
    (a.iter().zip(b).map(|(x, y)| (x - y) * (x - y)).sum::<f64>() / a.len() as f64).sqrt()
}

/// Exact magnetisations and covariance by enumeration (temperature 1).
fn moments(m: &Model) -> (Vec<f64>, Vec<Vec<f64>>) {
    let n = m.len();
    let (mut z, mut mm, mut ss) = (0.0, vec![0.0; n], vec![vec![0.0; n]; n]);
    let mut s = vec![0.0; n];
    for bits in 0u64..(1 << n) {
        for i in 0..n {
            s[i] = if (bits >> i) & 1 == 1 { 1.0 } else { -1.0 };
        }
        let w = (-m.energy(&s)).exp();
        z += w;
        for i in 0..n {
            mm[i] += w * s[i];
            for j in 0..n {
                ss[i][j] += w * s[i] * s[j];
            }
        }
    }
    let mag: Vec<f64> = mm.iter().map(|v| v / z).collect();
    let c = (0..n).map(|i| (0..n).map(|j| ss[i][j] / z - mag[i] * mag[j]).collect()).collect();
    (mag, c)
}

fn main() {
    let (w, h, seeds) = (4, 4, 20);
    let st = State::new(0);
    println!("4x4 open grid, 20 targets, RMS grey error of the exact marginals (lower is better)");
    let names = ["TAP", "Bethe", "secant20", "precond20", "newton5", "precond smp", "newton smp"];
    print!("{:>5}", "J");
    for nm in names {
        print!(" {:>11}", nm);
    }
    println!(" {:>10}", "Bethe<TAP");
    for &j in &[0.05, 0.1, 0.2, 0.25, 0.3, 0.35, 0.4, 0.44, 0.5, 0.6] {
        let (mut m, g) = grid(w, h, j);
        let mut r = Rng::new(42);
        let mut tot = [0.0f64; 7];
        let mut bethe_wins = 0;
        for seed in 0..seeds {
            let grey: Vec<f64> = (0..w * h).map(|_| 0.1 + 0.8 * r.unit()).collect();
            let target: Vec<f64> = grey.iter().map(|v| 2.0 * v - 1.0).collect();
            let exact_m = |mm: &Model| exact_rates(mm, &State::new(0)).iter().map(|v| 2.0 * v - 1.0).collect::<Vec<f64>>();
            let mut e = [0.0f64; 7];
            m.h = leans_for(&m, &g, &target, 1.0, Invert::Tap);
            let tap = m.h.clone();
            e[0] = rms(&exact_rates(&m, &st), &grey);
            m.h = bethe_leans(&m, &g, &target);
            e[1] = rms(&exact_rates(&m, &st), &grey);
            m.h = secant_fit(&mut m, &g, &target, tap.clone(), 20, false, exact_m).0;
            e[2] = rms(&exact_rates(&m, &st), &grey);
            m.h = precond_fit(&mut m, &g, &target, tap.clone(), 20, 0.05, false, exact_m).0;
            e[3] = rms(&exact_rates(&m, &st), &grey);
            m.h = newton_fit(&mut m, &g, &target, tap.clone(), 5, 1e-9, false, |mm: &Model| {
                let (mag, c) = moments(mm);
                (mag, Response::Dense(c))
            })
            .0;
            e[4] = rms(&exact_rates(&m, &st), &grey);
            m.h = precond_leans(&mut m, &g, &target, tap.clone(), Update::Gibbs, 10, 2000, 1.0, 1000 + seed, 0.05).0;
            e[5] = rms(&exact_rates(&m, &st), &grey);
            m.h = newton_leans(&mut m, &g, &target, tap.clone(), Update::Gibbs, 5, 2000, 1.0, 2000 + seed, 0.1).0;
            e[6] = rms(&exact_rates(&m, &st), &grey);
            if e[1] < e[0] {
                bethe_wins += 1;
            }
            for k in 0..7 {
                tot[k] += e[k];
            }
        }
        print!("{:>5.2}", j);
        for t in tot {
            print!(" {:>11.2e}", t / seeds as f64);
        }
        println!(" {:>7}/{}", bethe_wins, seeds);
    }
}
