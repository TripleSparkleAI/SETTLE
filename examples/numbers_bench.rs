//! SMOOTHNUMBERS measurement bench: error of the settled solution against settling time, the inverse from the
//! spread, wall clock against Gaussian elimination, and the refusal controls.
//!
//!     cargo run --release --example numbers_bench > ../runs/smoothnumbers/bench_output.txt
//!
//! Every matrix is A = Q diag(lambda) Q^T with Q a random orthogonal matrix and lambda log-spaced from 1 down to
//! 1/kappa, so the stiffest spring is 1 and the condition number is exactly kappa. Step 0.1, temperature 1.
//! Error at time t is the average of the positions over the window (t/10, t], against Gaussian elimination.

#![allow(clippy::needless_range_loop)] // index loops mirror the equations they measure

use settle::interp::Interp;
use settle::numbers::*;
use settle::rng::Rng;
use std::time::Instant;

const DT: f64 = 0.1;
const TEMP: f64 = 1.0;
const TIMES: [f64; 9] = [10.0, 30.0, 100.0, 300.0, 1_000.0, 3_000.0, 10_000.0, 30_000.0, 100_000.0];
const SEEDS: [u64; 3] = [11, 12, 13];

fn orthogonal(d: usize, rng: &mut Rng) -> Vec<f64> {
    // columns by Gram-Schmidt on Gaussian vectors; q[i*d + k] is row i of column k
    let mut cols: Vec<Vec<f64>> = Vec::new();
    while cols.len() < d {
        let mut v: Vec<f64> = (0..d).map(|_| rng.normal()).collect();
        for _ in 0..2 {
            for c in &cols {
                let p: f64 = v.iter().zip(c).map(|(x, y)| x * y).sum();
                v.iter_mut().zip(c).for_each(|(x, y)| *x -= p * y);
            }
        }
        let n = norm(&v);
        if n > 1e-8 {
            cols.push(v.iter().map(|x| x / n).collect());
        }
    }
    let mut q = vec![0.0; d * d];
    for k in 0..d {
        for i in 0..d {
            q[i * d + k] = cols[k][i];
        }
    }
    q
}

fn from_spectrum(q: &[f64], lam: &[f64], d: usize) -> Vec<f64> {
    let mut a = vec![0.0; d * d];
    for i in 0..d {
        for j in 0..d {
            a[i * d + j] = (0..d).map(|k| q[i * d + k] * lam[k] * q[j * d + k]).sum();
        }
    }
    // exact symmetry despite rounding
    for i in 0..d {
        for j in i + 1..d {
            let m = 0.5 * (a[i * d + j] + a[j * d + i]);
            a[i * d + j] = m;
            a[j * d + i] = m;
        }
    }
    a
}

fn spectrum(d: usize, kappa: f64) -> Vec<f64> {
    (0..d).map(|k| kappa.powf(-(k as f64) / (d - 1) as f64)).collect()
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(|a, b| a.total_cmp(b));
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2]
    } else {
        0.5 * (v[n / 2 - 1] + v[n / 2])
    }
}

fn steps_of(t: f64) -> usize {
    (t / DT).round() as usize
}

/// Window-average relative error at every checkpoint, from the running sums.
fn curve(w: &Walked, x: &[f64]) -> Vec<f64> {
    let sum_at = |s: usize| w.sums.iter().find(|(k, _)| *k == s).map(|(_, v)| v.clone()).unwrap();
    TIMES
        .iter()
        .map(|&t| {
            let (s1, s0) = (steps_of(t), steps_of(t / 10.0));
            let (a, b) = (sum_at(s1), sum_at(s0));
            let n = (s1 - s0) as f64;
            let diff: Vec<f64> = a.iter().zip(&b).zip(x).map(|((p, q), xi)| (p - q) / n - xi).collect();
            norm(&diff) / norm(x)
        })
        .collect()
}

fn marks() -> Vec<usize> {
    let mut m = Vec::new();
    for &t in &TIMES {
        m.push(steps_of(t));
        m.push(steps_of(t / 10.0));
        m.push(steps_of(t) - 1);
    }
    m.sort_unstable();
    m.dedup();
    m
}

fn slope(ys: &[f64], i0: usize, i1: usize) -> f64 {
    ((ys[i1]).ln() - (ys[i0]).ln()) / ((TIMES[i1]).ln() - (TIMES[i0]).ln())
}

fn fmt_row(v: &[f64]) -> String {
    v.iter().map(|x| format!("{:>10.3e}", x)).collect::<Vec<_>>().join("")
}

/// Follow-up to P2: the kappa-1000 slopes with 16 seeds, root-mean-square over seeds.
fn slopes() {
    let total = steps_of(*TIMES.last().unwrap());
    println!("SLOPES follow-up · kappa 1000 · random b · 16 seeds · RMS and median over seeds");
    for &d in &[4usize, 16, 64] {
        let kappa = 1000.0;
        let mut rng = Rng::new(1000 + d as u64 * 7 + kappa as u64);
        let q = orthogonal(d, &mut rng);
        let a = from_spectrum(&q, &spectrum(d, kappa), d);
        let b: Vec<f64> = (0..d).map(|_| rng.normal()).collect();
        let x = gauss_solve(&a, &b, d).unwrap();
        let curves: Vec<Vec<f64>> = (0..16u64)
            .map(|s| {
                let w = Walk { steps: total, dt: DT, temp: TEMP, seed: 100 + s, burn: total / 10, want_cov: false, marks: marks() };
                curve(&walk(&a, &b, d, &w), &x)
            })
            .collect();
        let rms: Vec<f64> = (0..TIMES.len()).map(|k| (curves.iter().map(|c| c[k] * c[k]).sum::<f64>() / 16.0).sqrt()).collect();
        let med: Vec<f64> = (0..TIMES.len()).map(|k| median(curves.iter().map(|c| c[k]).collect())).collect();
        println!("d {:>2} rms: {}", d, fmt_row(&rms));
        println!("     slope t=10k..100k rms {:+.3} · median {:+.3} · t=30k..100k rms {:+.3}", slope(&rms, 6, 8), slope(&med, 6, 8), slope(&rms, 7, 8));
    }
}

/// Follow-up to P11: split the kappa-1000 error into burn-in bias (the temperature-zero window average, exact
/// by linearity) and noise (sqrt(RMS^2 - bias^2) over 16 seeds).
fn bias() {
    let total = steps_of(*TIMES.last().unwrap());
    println!("BIAS follow-up · kappa 1000 · random b · bias = T=0 window average · noise = sqrt(rms^2 - bias^2), 16 seeds");
    for &d in &[4usize, 16, 64] {
        let kappa = 1000.0;
        let mut rng = Rng::new(1000 + d as u64 * 7 + kappa as u64);
        let q = orthogonal(d, &mut rng);
        let a = from_spectrum(&q, &spectrum(d, kappa), d);
        let b: Vec<f64> = (0..d).map(|_| rng.normal()).collect();
        let x = gauss_solve(&a, &b, d).unwrap();
        let w0 = Walk { steps: total, dt: DT, temp: 0.0, seed: 1, burn: total / 10, want_cov: false, marks: marks() };
        let bias = curve(&walk(&a, &b, d, &w0), &x);
        let curves: Vec<Vec<f64>> = (0..16u64)
            .map(|s| {
                let w = Walk { steps: total, dt: DT, temp: TEMP, seed: 100 + s, burn: total / 10, want_cov: false, marks: marks() };
                curve(&walk(&a, &b, d, &w), &x)
            })
            .collect();
        let rms: Vec<f64> = (0..TIMES.len()).map(|k| (curves.iter().map(|c| c[k] * c[k]).sum::<f64>() / 16.0).sqrt()).collect();
        let noise: Vec<f64> = rms.iter().zip(&bias).map(|(r, b)| (r * r - b * b).max(1e-300).sqrt()).collect();
        println!("d {:>2} rms:   {}", d, fmt_row(&rms));
        println!("     bias:  {}", fmt_row(&bias));
        println!("     noise: {}", fmt_row(&noise));
        println!(
            "     bias share of rms at t=10000 {:.0}% · noise slope t=10k..100k {:+.3} · rms slope {:+.3}",
            100.0 * bias[6] / rms[6],
            slope(&noise, 6, 8),
            slope(&rms, 6, 8)
        );
    }
}

/// Instrument check: the kappa-1000 matrices' true softest stiffness, the soft-mode share of x*, and the
/// predicted noise of the window average, sqrt(sum_k 2T / (lambda_k^2 * 0.9 t)) / |x*|, against the 16-seed RMS.
fn diag() {
    let total = steps_of(*TIMES.last().unwrap());
    for &d in &[4usize, 16, 64] {
        let kappa = 1000.0;
        let mut rng = Rng::new(1000 + d as u64 * 7 + kappa as u64);
        let q = orthogonal(d, &mut rng);
        let lam = spectrum(d, kappa);
        let a = from_spectrum(&q, &lam, d);
        let b: Vec<f64> = (0..d).map(|_| rng.normal()).collect();
        let x = gauss_solve(&a, &b, d).unwrap();
        let qtq = matmul(&(0..d * d).map(|k| q[(k % d) * d + k / d]).collect::<Vec<_>>(), &q, d);
        let off = (0..d * d).map(|k| (qtq[k] - if k % (d + 1) == 0 { 1.0 } else { 0.0 }).abs()).fold(0.0, f64::max);
        let lmin = 1.0 / stiffest(&gauss_inverse(&a, d).unwrap(), d);
        let soft: f64 = (0..d).map(|i| q[i * d + d - 1] * x[i]).sum();
        let pred = |t: f64| (lam.iter().map(|l| 2.0 * TEMP / (l * l * 0.9 * t)).sum::<f64>()).sqrt() / norm(&x);
        let curves: Vec<Vec<f64>> = (0..16u64)
            .map(|s| {
                let w = Walk { steps: total, dt: DT, temp: TEMP, seed: 100 + s, burn: total / 10, want_cov: false, marks: marks() };
                curve(&walk(&a, &b, d, &w), &x)
            })
            .collect();
        let rms = |k: usize| (curves.iter().map(|c| c[k] * c[k]).sum::<f64>() / 16.0).sqrt();
        println!(
            "d {:>2}: max|QtQ - I| {:.1e} · softest stiffness {:.6} · |x*| {:.1} of which soft mode {:.1} · predicted noise t=30k {:.4} t=100k {:.4} · measured rms {:.4} {:.4}",
            d, off, lmin, norm(&x), soft.abs(), pred(30_000.0), pred(100_000.0), rms(7), rms(8)
        );
        let per: Vec<String> = curves.iter().map(|c| format!("{:.4}", c[8])).collect();
        println!("      per-seed t=100k: {}", per.join(" "));
    }
}

/// Instrument check at size 4, kappa 1000: RMS of the t=100,000 error over 200 seeds, two seed schemes.
fn diag4() {
    let total = steps_of(*TIMES.last().unwrap());
    let (d, kappa) = (4usize, 1000.0);
    let mut rng = Rng::new(1000 + d as u64 * 7 + kappa as u64);
    let q = orthogonal(d, &mut rng);
    let lam = spectrum(d, kappa);
    let a = from_spectrum(&q, &lam, d);
    let b: Vec<f64> = (0..d).map(|_| rng.normal()).collect();
    let x = gauss_solve(&a, &b, d).unwrap();
    let pred = (lam.iter().map(|l| 2.0 * TEMP / (l * l * 0.9 * 100_000.0)).sum::<f64>()).sqrt() / norm(&x);
    for (label, base, mul) in [("consecutive seeds 1000..1199", 1000u64, 1u64), ("spread seeds 7919*k+17", 17, 7919)] {
        let v: Vec<f64> = (0..200u64)
            .map(|k| {
                let w = Walk { steps: total, dt: DT, temp: TEMP, seed: base + mul * k, burn: total / 10, want_cov: false, marks: marks() };
                curve(&walk(&a, &b, d, &w), &x)[8]
            })
            .collect();
        let rms = (v.iter().map(|e| e * e).sum::<f64>() / v.len() as f64).sqrt();
        println!("d 4 kappa 1000 · {} · 200 seeds · RMS rel error at t=100000 {:.4} · predicted {:.4} · ratio {:.2}", label, rms, pred, rms / pred);
    }
}

fn main() {
    if std::env::args().nth(1).as_deref() == Some("diag4") {
        return diag4();
    }
    if std::env::args().nth(1).as_deref() == Some("diag") {
        return diag();
    }
    if std::env::args().nth(1).as_deref() == Some("slopes") {
        return slopes();
    }
    if std::env::args().nth(1).as_deref() == Some("bias") {
        return bias();
    }
    let total = steps_of(*TIMES.last().unwrap());
    println!("SMOOTHNUMBERS bench · step {} · temperature {} · seeds {:?} · windows (t/10, t]", DT, TEMP, SEEDS);
    println!("times: {}", TIMES.iter().map(|t| format!("{:>10}", t)).collect::<Vec<_>>().join(""));
    let mut zs: Vec<f64> = Vec::new();
    for &d in &[4usize, 16, 64] {
        for &kappa in &[10.0f64, 1000.0] {
            let mut rng = Rng::new(1000 + d as u64 * 7 + kappa as u64);
            let q = orthogonal(d, &mut rng);
            let lam = spectrum(d, kappa);
            let a = from_spectrum(&q, &lam, d);
            let b_rand: Vec<f64> = (0..d).map(|_| rng.normal()).collect();
            let b_worst: Vec<f64> = (0..d).map(|i| q[i * d]).collect(); // stiffest eigenvector
            let x_rand = gauss_solve(&a, &b_rand, d).unwrap();
            let x_worst = gauss_solve(&a, &b_worst, d).unwrap();
            let inv = gauss_inverse(&a, d).unwrap();
            println!("\n=== d {} · kappa {} · stiffest {:.4} (power iteration) · |x*| random {:.3}", d, kappa, stiffest(&a, d), norm(&x_rand));

            // random b, with covariance
            let mut curves = Vec::new();
            let (mut inv_c, mut inv_r) = (Vec::new(), Vec::new());
            let mut per_step_cov = 0.0;
            for &s in &SEEDS {
                let w = Walk { steps: total, dt: DT, temp: TEMP, seed: s, burn: total / 10, want_cov: true, marks: marks() };
                let r = walk(&a, &b_rand, d, &w);
                assert!(r.blew.is_none());
                per_step_cov = r.secs / total as f64;
                curves.push(curve(&r, &x_rand));
                let est = inverse_from_spread(&r.cov, &a, d, DT, TEMP);
                inv_c.push(rel_frob(&est, &inv));
                let raw: Vec<f64> = r.cov.iter().map(|c| c / TEMP).collect();
                inv_r.push(rel_frob(&raw, &inv));
                for i in 0..d {
                    zs.push((r.mean[i] - x_rand[i]) / r.se[i]);
                }
            }
            let med: Vec<f64> = (0..TIMES.len()).map(|k| median(curves.iter().map(|c| c[k]).collect())).collect();
            println!("random b   rel err (median of 3): {}", fmt_row(&med));
            println!("           slope t=10k..100k {:+.3}   slope t=1k..10k {:+.3}", slope(&med, 6, 8), slope(&med, 4, 6));
            println!(
                "           inverse from spread at t=100000: step-corrected median {:.4} (seeds {:?}) · raw median {:.4}",
                median(inv_c.clone()),
                inv_c.iter().map(|x| format!("{:.4}", x)).collect::<Vec<_>>(),
                median(inv_r.clone())
            );

            // worst-case b (the stiffest eigenvector), no covariance, and the wall clock per step
            let mut wcurves = Vec::new();
            let mut per_step = 0.0;
            for &s in &SEEDS {
                let w = Walk { steps: total, dt: DT, temp: TEMP, seed: s, burn: total / 10, want_cov: false, marks: marks() };
                let r = walk(&a, &b_worst, d, &w);
                per_step = r.secs / total as f64;
                wcurves.push(curve(&r, &x_worst));
            }
            let wmed: Vec<f64> = (0..TIMES.len()).map(|k| median(wcurves.iter().map(|c| c[k]).collect())).collect();
            println!("worst b    rel err (median of 3): {}", fmt_row(&wmed));

            // temperature zero: plain relaxation, error of the final iterate
            let w0 = Walk { steps: total, dt: DT, temp: 0.0, seed: 1, burn: total - 20, want_cov: false, marks: marks() };
            let r0 = walk(&a, &b_rand, d, &w0);
            let it: Vec<f64> = TIMES
                .iter()
                .map(|&t| {
                    let s = steps_of(t);
                    let p = &r0.sums.iter().find(|(k, _)| *k == s).unwrap().1;
                    let m = &r0.sums.iter().find(|(k, _)| *k == s - 1).unwrap().1;
                    let diff: Vec<f64> = p.iter().zip(m).zip(&x_rand).map(|((p, m), xi)| p - m - xi).collect();
                    norm(&diff) / norm(&x_rand)
                })
                .collect();
            println!("T=0 final  rel err (iterate):     {}", fmt_row(&it));

            // wall clock
            let reps = if d <= 16 { 20_000 } else { 2_000 };
            let t0 = Instant::now();
            for _ in 0..reps {
                std::hint::black_box(gauss_solve(std::hint::black_box(&a), &b_rand, d));
            }
            let gauss_s = t0.elapsed().as_secs_f64() / reps as f64;
            let hit = TIMES.iter().zip(&med).find(|(_, e)| **e < 0.01).map(|(t, _)| *t);
            match hit {
                Some(t) => println!(
                    "wall clock: exact elimination {:.4} ms · drift {:.3} us/step ({:.3} with covariance) · first checkpoint under 1%: t={} = {:.1} ms · ratio {:.0}x",
                    1e3 * gauss_s,
                    1e6 * per_step,
                    1e6 * per_step_cov,
                    t,
                    1e3 * per_step * steps_of(t) as f64,
                    per_step * steps_of(t) as f64 / gauss_s
                ),
                None => println!(
                    "wall clock: exact elimination {:.4} ms · drift {:.3} us/step · never under 1% by t=100000",
                    1e3 * gauss_s,
                    1e6 * per_step
                ),
            }
        }
    }
    let within = zs.iter().filter(|z| z.abs() < 2.0).count();
    println!("\nstandard-error calibration at t=100000: {} of {} coordinates within 2 standard errors ({:.1}%)", within, zs.len(), 100.0 * within as f64 / zs.len() as f64);

    // ---------------- controls ----------------
    println!("\n=== controls");
    let mut rng = Rng::new(77);
    // 1. lambda_min sweep: does the drift alone notice, and does the certificate
    let d = 16;
    let q = orthogonal(d, &mut rng);
    let b: Vec<f64> = (0..d).map(|_| rng.normal()).collect();
    for &lmin in &[-1.0, -0.1, -0.03, -0.01, -1e-3, -1e-4, 0.0] {
        let mut lam: Vec<f64> = (0..d - 1).map(|k| 10f64.powf(-(k as f64) / (d - 2) as f64)).collect();
        lam.push(lmin);
        let a = from_spectrum(&q, &lam, d);
        let w = Walk { steps: 10_000, dt: DT, temp: TEMP, seed: 5, burn: 1_000, want_cov: false, marks: vec![] };
        let r = walk(&a, &b, d, &w);
        let big = r.last.iter().fold(0.0f64, |s, v| s.max(v.abs()));
        let cert = cholesky_certificate(&a, d);
        // and through the language: never "solved"
        let mat: Vec<String> = (0..d).map(|i| (0..d).map(|k| format!("{:e}", a[i * d + k])).collect::<Vec<_>>().join(" ")).collect();
        let names: Vec<String> = (0..d).map(|i| format!(":n{}", i)).collect();
        let tgt: Vec<String> = b.iter().map(|v| format!("{:e}", v)).collect();
        let src = format!(
            "model :c do\nend\nrun :c do\n  solve {}, matrix: \"{}\", target: \"{}\", steps: 10_000, step: 0.1\nend",
            names.join(", "),
            mat.join("; "),
            tgt.join(" ")
        );
        let out = Interp::default().exec(&src).unwrap();
        let solved = out.iter().any(|l| l.starts_with("solved"));
        println!(
            "lambda_min {:>8}: drift blew up {:<14} largest |x| {:>9.2e} · certificate {} · language says {}",
            lmin,
            match r.blew {
                Some(s) => format!("at step {}", s),
                None => "no".to_string(),
            },
            big,
            if cert.is_ok() { "PASSES" } else { "refuses" },
            if solved { "SOLVED (defect)" } else { "did not settle" }
        );
    }
    // 2. twenty random non-symmetric matrices, twenty random symmetric indefinite ones
    let (mut refused, mut nonpd_solved, mut nonpd_total) = (0, 0, 0);
    for trial in 0..20 {
        let d = 8;
        let mut g: Vec<f64> = (0..d * d).map(|_| rng.normal()).collect();
        let rows = |m: &[f64]| (0..d).map(|i| (0..d).map(|k| format!("{:e}", m[i * d + k])).collect::<Vec<_>>().join(" ")).collect::<Vec<_>>().join("; ");
        let names: Vec<String> = (0..d).map(|i| format!(":m{}", i)).collect();
        let src = format!("model :c do\nend\nrun :c do\n  solve {}, matrix: \"{}\", target: \"1 1 1 1 1 1 1 1\"\nend", names.join(", "), rows(&g));
        if let Err(e) = Interp::default().exec(&src) {
            if e.0.contains("refused: the matrix is not symmetric") {
                refused += 1;
            }
        }
        for i in 0..d {
            for k in i + 1..d {
                let m = g[i * d + k];
                g[k * d + i] = m;
            }
        }
        if cholesky_certificate(&g, d).is_err() {
            nonpd_total += 1;
            let src = format!(
                "model :c do\nend\nrun :c do\n  solve {}, matrix: \"{}\", target: \"1 1 1 1 1 1 1 1\", steps: 20_000, seed: {}\nend",
                names.join(", "),
                rows(&g),
                trial + 1
            );
            let out = Interp::default().exec(&src).unwrap();
            if out.iter().any(|l| l.starts_with("solved")) {
                nonpd_solved += 1;
            }
        }
    }
    println!("non-symmetric: {} of 20 refused · symmetric indefinite: {} of {} reported solved", refused, nonpd_solved, nonpd_total);
    println!("\nNEXT -> fill the report: SETTLE/runs/smoothnumbers/REPORT_SMOOTHNUMBERS.md");
}
