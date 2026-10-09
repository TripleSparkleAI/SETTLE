//! FILMSHARP scratch probe: the fit's residual trajectory on a synthetic picture (development, not a sealed arm).
use settle::filmsharp::{fit_leans, newton_leans, precond_leans, Sweeper, Update};
use settle::grid::{leans_for, psnr, Invert, Spec};
use settle::model::Model;
use settle::rng::Rng;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let j: f64 = a[1].parse().unwrap();
    let iters: usize = a[2].parse().unwrap();
    let sweeps: usize = a[3].parse().unwrap();
    let upd = match a[4].as_str() { "cluster" => Update::Cluster, "metro" => Update::Metro, "checker" => Update::Checker, _ => Update::Gibbs };
    let (w, h) = (40, 30);
    let px: Vec<f64> = (0..w * h).map(|k| {
        let (x, y) = ((k % w) as f64 / w as f64, (k / w) as f64 / h as f64);
        let disc = ((x - 0.35f64).powi(2) + (y - 0.5f64).powi(2)).sqrt() < 0.22;
        let bar = (x - 0.75f64).abs() < 0.06;
        let v: f64 = if disc { 0.12 } else if bar { 0.9 } else { 0.3 + 0.4 * x };
        (v * 255.0).round() / 255.0
    }).collect();
    let mut m = Model::default();
    for k in 0..w * h { m.add(&format!("p{}", k)); }
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
    let g = Spec { start: 0, w, h };
    let target: Vec<f64> = px.iter().map(|v| 2.0 * v.clamp(0.001, 0.999) - 1.0).collect();
    let tap = leans_for(&m, &g, &target, 1.0, Invert::Tap);
    let score = |m: &Model, seed: u64| {
        let mut rng = Rng::new(seed);
        let mut s: Vec<f64> = (0..w * h).map(|_| if rng.unit() < 0.5 { -1.0 } else { 1.0 }).collect();
        let mut free: Vec<usize> = (0..w * h).collect();
        let mut sw = Sweeper::new(Update::Cluster, &g, &free);
        let mut acc = vec![0.0; w * h];
        for k in 0..4000 { let rb = if k >= 500 { Some(&mut acc[..]) } else { None }; sw.sweep(m, &g, &mut s, &mut free, &mut rng, 1.0, rb); }
        let out: Vec<f64> = acc.iter().map(|v| (1.0 + v / 3500.0) / 2.0).collect();
        psnr(&out, &px)
    };
    m.h = tap.clone();
    println!("TAP long-run rb PSNR {:.2}", score(&m, 1));
    let method = a.get(5).cloned().unwrap_or("secant".into());
    let lambda: f64 = a.get(6).map(|x| x.parse().unwrap()).unwrap_or(0.1);
    let (hh, res) = if method == "precond" { precond_leans(&mut m, &g, &target, tap, upd, iters, sweeps, 1.0, 7, lambda) } else if method == "newton" { newton_leans(&mut m, &g, &target, tap, upd, iters, sweeps, 1.0, 7, lambda) } else { fit_leans(&mut m, &g, &target, tap, upd, iters, sweeps, 1.0, 7) };
    m.h = hh;
    println!("fit residuals {:?}", res.iter().map(|r| format!("{:.4}", r)).collect::<Vec<_>>());
    println!("fit long-run rb PSNR {:.2}", score(&m, 1));
}
