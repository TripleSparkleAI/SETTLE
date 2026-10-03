//! GRIDPLAYER-2 small-grid control: how close do mean-field and TAP leans land to the target greys, measured
//! against the EXACT marginals of a 4x4 grid (every one of the 65,536 arrangements enumerated)?
//!
//! Run: cargo run --release --example gridplayer2_tap_exact
//!
//! For each pull J and 20 random targets (greys uniform in [0.1, 0.9]) it prints the RMS grey error of
//! no correction, mean-field, TAP, and an iterated exact inversion (h <- h + d (atanh m* - atanh m_exact)),
//! the last showing the error is an inversion error and not a sampler error.

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

fn main() {
    let (w, h, seeds) = (4, 4, 20);
    let st = State::new(0);
    println!("4x4 open grid, 20 targets, RMS grey error of the exact marginals (lower is better)");
    println!("{:>5} {:>10} {:>10} {:>10} {:>12} {:>8} {:>10}", "J", "none", "mean-fld", "TAP", "iter-exact", "MF/TAP", "TAP wins");
    for &j in &[0.05, 0.1, 0.15, 0.2, 0.25, 0.3, 0.35, 0.4, 0.5] {
        let (mut m, g) = grid(w, h, j);
        let mut r = Rng::new(42);
        let mut tot = [0.0f64; 4];
        let mut tap_wins = 0;
        for _ in 0..seeds {
            let grey: Vec<f64> = (0..w * h).map(|_| 0.1 + 0.8 * r.unit()).collect();
            let target: Vec<f64> = grey.iter().map(|v| 2.0 * v - 1.0).collect();
            let mut errs = [0.0f64; 4];
            for (k, inv) in [Invert::None, Invert::Mean, Invert::Tap].iter().enumerate() {
                m.h = leans_for(&m, &g, &target, 1.0, *inv);
                errs[k] = rms(&exact_rates(&m, &st), &grey);
            }
            // iterated exact inversion, starting from TAP
            m.h = leans_for(&m, &g, &target, 1.0, Invert::Tap);
            for _ in 0..60 {
                let got = exact_rates(&m, &st);
                for i in 0..w * h {
                    let mi = (2.0 * got[i] - 1.0).clamp(-0.999999, 0.999999);
                    m.h[i] += 0.8 * (target[i].atanh() - mi.atanh());
                }
            }
            errs[3] = rms(&exact_rates(&m, &st), &grey);
            if errs[2] < errs[1] {
                tap_wins += 1;
            }
            for k in 0..4 {
                tot[k] += errs[k];
            }
        }
        let a: Vec<f64> = tot.iter().map(|t| t / seeds as f64).collect();
        println!("{:>5.2} {:>10.5} {:>10.5} {:>10.5} {:>12.2e} {:>8.2} {:>7}/{}", j, a[0], a[1], a[2], a[3], a[1] / a[2], tap_wins, seeds);
    }
}
