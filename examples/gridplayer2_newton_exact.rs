//! GRIDPLAYER-2 post-hoc (not sealed): the exact inversion by Newton's method on a 4x4 grid.
//!
//! Run: cargo run --release --example gridplayer2_newton_exact
//!
//! The sealed damped iteration h <- h + 0.8 (atanh m* - atanh m) stopped converging from J = 0.3 (its step
//! overshoots once the grid's response to a lean exceeds about 2.5). Newton uses the exact response itself:
//! dm_i/dh_j = <s_i s_j> - m_i m_j (the covariance, by enumeration), and steps dh = C^-1 (m* - m).
//! This shows the exact leans exist at every pull tried and how far TAP's leans sit from them.

use settle::grid::{leans_for, Invert, Spec};
use settle::model::Model;
use settle::rng::Rng;

fn grid(w: usize, h: usize, j: f64) -> Model {
    let mut m = Model::default();
    for k in 0..w * h {
        m.add(&format!("p{}", k));
    }
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
    m
}

/// Exact magnetisations and covariance of all things by enumeration (temperature 1).
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
            for j in i..n {
                ss[i][j] += w * s[i] * s[j];
            }
        }
    }
    let mag: Vec<f64> = mm.iter().map(|v| v / z).collect();
    let mut c = vec![vec![0.0; n]; n];
    for i in 0..n {
        for j in i..n {
            c[i][j] = ss[i][j] / z - mag[i] * mag[j];
            c[j][i] = c[i][j];
        }
    }
    (mag, c)
}

/// Solve A x = b by Gaussian elimination with partial pivoting.
fn solve(mut a: Vec<Vec<f64>>, mut b: Vec<f64>) -> Vec<f64> {
    let n = b.len();
    for col in 0..n {
        let p = (col..n).max_by(|&x, &y| a[x][col].abs().partial_cmp(&a[y][col].abs()).unwrap()).unwrap();
        a.swap(col, p);
        b.swap(col, p);
        for r in col + 1..n {
            let f = a[r][col] / a[col][col];
            for k in col..n {
                a[r][k] -= f * a[col][k];
            }
            b[r] -= f * b[col];
        }
    }
    let mut x = vec![0.0; n];
    for r in (0..n).rev() {
        x[r] = (b[r] - (r + 1..n).map(|k| a[r][k] * x[k]).sum::<f64>()) / a[r][r];
    }
    x
}

fn main() {
    let (w, h, seeds) = (4, 4, 20);
    let g = Spec { start: 0, w, h };
    println!("4x4 open grid, 20 targets (same seeds as gridplayer2_tap_exact), Newton toward the exact leans");
    println!("{:>5} {:>12} {:>9} {:>14} {:>16}", "J", "newton-rms", "steps", "|h_tap-h*|rms", "|h_mf-h*|rms");
    for &j in &[0.05, 0.1, 0.15, 0.2, 0.25, 0.3, 0.35, 0.4, 0.5] {
        let mut m = grid(w, h, j);
        let mut r = Rng::new(42);
        let (mut err, mut steps, mut dtap, mut dmf) = (0.0, 0.0, 0.0, 0.0);
        for _ in 0..seeds {
            let grey: Vec<f64> = (0..w * h).map(|_| 0.1 + 0.8 * r.unit()).collect();
            let target: Vec<f64> = grey.iter().map(|v| 2.0 * v - 1.0).collect();
            let tap = leans_for(&m, &g, &target, 1.0, Invert::Tap);
            let mf = leans_for(&m, &g, &target, 1.0, Invert::Mean);
            m.h = tap.clone();
            let mut k = 0;
            let mut e = 1.0;
            while k < 30 {
                let (mag, c) = moments(&m);
                let d: Vec<f64> = target.iter().zip(&mag).map(|(t, x)| t - x).collect();
                e = (d.iter().map(|x| x * x / 4.0).sum::<f64>() / d.len() as f64).sqrt();
                if e < 1e-13 {
                    break;
                }
                let dh = solve(c, d);
                for i in 0..w * h {
                    m.h[i] += dh[i];
                }
                k += 1;
            }
            let rmsd = |a: &[f64]| (a.iter().zip(&m.h).map(|(x, y)| (x - y) * (x - y)).sum::<f64>() / a.len() as f64).sqrt();
            err += e;
            steps += k as f64;
            dtap += rmsd(&tap);
            dmf += rmsd(&mf);
        }
        let s = seeds as f64;
        println!("{:>5.2} {:>12.2e} {:>9.1} {:>14.5} {:>16.5}", j, err / s, steps / s, dtap / s, dmf / s);
    }
}
