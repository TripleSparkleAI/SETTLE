//! FILMWARM: the time correlation of each update rule with the SAME fitted leans near the critical pull, and the
//! 80-sweep read it predicts, bias included.
//!
//! Run: cargo run --release --example filmwarm_tau -- <frames dir> <J> <updates, comma> [T] [L] [frames] [fit] [fit_sweeps] [burn]
//!
//! For each of the first `frames` frames: TAP leans, then FILMSHARP's preconditioned fit (`fit` x `fit_sweeps`
//! sweeps, cluster chain, seed 77 + frame), so every update rule sees identical leans. For each rule: a chain from
//! coin flips (seed 5 + frame), `burn` sweeps, then T recorded sweeps. For a series x_t per pixel,
//!   rho(k) = sum_i [ C_i(k) - xbar_i^2 ] / sum_i var_i,   tau = 1/2 + sum_{k=1..M} rho(k), M the first lag >= 6 tau.
//! The K-sweep read then has per-pixel grey MSE  bias^2 + (var_i / 4) 2 tau / K.
//! bias^2 is estimated from the T-sweep mean of tanh(I) against the target grey, less its own variance
//! (var_i / 4) 2 tau / T. Printed per frame and rule: 2 tau (bits, tanh I), the variance ratio, the bias in dB,
//! and the PSNR predicted for an 80-sweep bits read and an 80-sweep tanh read.

#![allow(clippy::needless_range_loop)] // index loops mirror the equations they measure

use settle::filmsharp::{precond_leans, Sweeper, Update};
use settle::grid::{leans_for, magnetisations, read_pgm, Invert, Spec};
use settle::model::Model;
use settle::rng::Rng;
use std::path::PathBuf;

fn grid(w: usize, h: usize, j: f64) -> (Model, Spec) {
    let mut m = Model::default();
    for y in 0..h {
        for x in 0..w {
            m.add(&format!("p_{}_{}", x, y));
        }
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
    (m, Spec { start: 0, w, h })
}

struct Acf {
    l: usize,
    n: usize,
    hist: Vec<Vec<f32>>,
    sum: Vec<f64>,
    lag: Vec<Vec<f64>>,
    cnt: Vec<f64>,
    t: usize,
}

impl Acf {
    fn new(n: usize, l: usize) -> Self {
        Acf { l, n, hist: vec![vec![0.0; n]; l + 1], sum: vec![0.0; n], lag: vec![vec![0.0; n]; l + 1], cnt: vec![0.0; l + 1], t: 0 }
    }
    fn push(&mut self, x: &[f64]) {
        let slot = self.t % (self.l + 1);
        for i in 0..self.n {
            self.hist[slot][i] = x[i] as f32;
            self.sum[i] += x[i];
        }
        for k in 0..=self.l.min(self.t) {
            let old = &self.hist[(self.t - k) % (self.l + 1)];
            let acc = &mut self.lag[k];
            for i in 0..self.n {
                acc[i] += x[i] * old[i] as f64;
            }
            self.cnt[k] += 1.0;
        }
        self.t += 1;
    }
    /// (sum of per-pixel variances, pooled rho(k) for k = 0..=l)
    fn rho(&self) -> (f64, Vec<f64>) {
        let tt = self.t as f64;
        let mean: Vec<f64> = self.sum.iter().map(|s| s / tt).collect();
        let var: f64 = (0..self.n).map(|i| self.lag[0][i] / self.cnt[0] - mean[i] * mean[i]).sum();
        let r = (0..=self.l).map(|k| (0..self.n).map(|i| self.lag[k][i] / self.cnt[k] - mean[i] * mean[i]).sum::<f64>() / var.max(1e-300)).collect();
        (var, r)
    }
}

fn tau(r: &[f64]) -> (f64, bool) {
    let mut t = 0.5;
    for m in 1..r.len() {
        t += r[m];
        if m as f64 >= 6.0 * t {
            return (t, true);
        }
    }
    (t, false)
}

fn rule(s: &str) -> Update {
    match s {
        "gibbs" => Update::Gibbs,
        "checker" => Update::Checker,
        "metro" => Update::Metro,
        "metro_checker" => Update::MetroChecker,
        "cluster" => Update::Cluster,
        u => panic!("unknown update {}", u),
    }
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let dir = PathBuf::from(&a[1]);
    let j: f64 = a[2].parse().unwrap();
    let rules: Vec<(String, Update)> = a[3].split(',').map(|s| (s.to_string(), rule(s))).collect();
    let t_rec: usize = a.get(4).map(|s| s.parse().unwrap()).unwrap_or(3000);
    let lags: usize = a.get(5).map(|s| s.parse().unwrap()).unwrap_or(300);
    let nfr: usize = a.get(6).map(|s| s.parse().unwrap()).unwrap_or(3);
    let fit: usize = a.get(7).map(|s| s.parse().unwrap()).unwrap_or(10);
    let fit_sweeps: usize = a.get(8).map(|s| s.parse().unwrap()).unwrap_or(400);
    let burn: usize = a.get(9).map(|s| s.parse().unwrap()).unwrap_or(300);
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().map(|x| x == "pgm").unwrap_or(false)).collect();
    files.sort();
    println!("J {} leans TAP + fit {} x {} cluster; T {} lags {} burn {} frames {}", j, fit, fit_sweeps, t_rec, lags, burn, nfr);
    println!("{:>14} {:>14} {:>9} {:>9} {:>9} {:>10} {:>9} {:>9} {:>9} {:>9}", "frame", "update", "2tau bits", "2tau tanh", "var ratio", "window ok", "dB long", "dB bias", "dB bits80", "dB tanh80");
    let mut pooled: Vec<(f64, f64, f64, f64, f64, f64, f64)> = vec![(0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0); rules.len()];
    for (fi, f) in files.iter().take(nfr).enumerate() {
        let pic = read_pgm(f).unwrap();
        let (mut m, g) = grid(pic.w, pic.h, j);
        let n = pic.w * pic.h;
        let target = magnetisations(&pic);
        let h0 = leans_for(&m, &g, &target, 1.0, Invert::Tap);
        m.h = precond_leans(&mut m, &g, &target, h0, Update::Cluster, fit, fit_sweeps, 1.0, 77 + fi as u64, 0.05).0;
        for (ri, (name, upd)) in rules.iter().enumerate() {
            let mut rng = Rng::new(5 + fi as u64);
            let mut s: Vec<f64> = (0..n).map(|_| if rng.unit() < 0.5 { -1.0 } else { 1.0 }).collect();
            let mut free: Vec<usize> = (0..n).collect();
            let mut sw = Sweeper::new(*upd, &g, &free);
            for _ in 0..burn {
                sw.sweep(&m, &g, &mut s, &mut free, &mut rng, 1.0, None);
            }
            let (mut ab, mut ar) = (Acf::new(n, lags), Acf::new(n, lags));
            let mut xr = vec![0.0; n];
            let mut mean_t = vec![0.0; n];
            for _ in 0..t_rec {
                sw.sweep(&m, &g, &mut s, &mut free, &mut rng, 1.0, None);
                for i in 0..n {
                    xr[i] = m.input(i, &s).tanh();
                    mean_t[i] += xr[i];
                }
                ab.push(&s);
                ar.push(&xr);
            }
            let ((vb, rb), (vr, rr)) = (ab.rho(), ar.rho());
            let ((tb, okb), (tr, okr)) = (tau(&rb), tau(&rr));
            // long-run grey error of the T-sweep tanh mean, and the bias left after removing its own variance
            let mse_long = (0..n).map(|i| { let gg = (1.0 + mean_t[i] / t_rec as f64) / 2.0; (gg - pic.px[i]).powi(2) }).sum::<f64>() / n as f64;
            let var_long = vr / 4.0 / n as f64 * 2.0 * tr / t_rec as f64;
            let bias2 = (mse_long - var_long).max(1e-9);
            let db = |x: f64| 10.0 * (1.0 / x).log10();
            let p_bits = bias2 + vb / 4.0 / n as f64 * 2.0 * tb / 80.0;
            let p_tanh = bias2 + vr / 4.0 / n as f64 * 2.0 * tr / 80.0;
            println!(
                "{:>14} {:>14} {:>9.3} {:>9.3} {:>9.4} {:>10} {:>9.2} {:>9.2} {:>9.2} {:>9.2}",
                f.file_name().unwrap().to_string_lossy(),
                name,
                2.0 * tb,
                2.0 * tr,
                vr / vb,
                format!("{}/{}", okb, okr),
                db(mse_long),
                db(bias2),
                db(p_bits),
                db(p_tanh)
            );
            let p = &mut pooled[ri];
            p.0 += vb;
            p.1 += vr;
            p.2 += vb * tb;
            p.3 += vr * tr;
            p.4 += bias2;
            p.5 += p_bits;
            p.6 += p_tanh;
        }
    }
    let k = nfr.min(files.len()) as f64;
    for (ri, (name, _)) in rules.iter().enumerate() {
        let p = pooled[ri];
        println!(
            "pooled {:>14}: 2tau bits {:.3}, 2tau tanh {:.3}, var ratio {:.4}, mean dB bias {:.2}, mean predicted dB bits80 {:.2} tanh80 {:.2}",
            name,
            2.0 * p.2 / p.0,
            2.0 * p.3 / p.1,
            p.1 / p.0,
            10.0 * (k / p.4).log10(),
            10.0 * (k / p.5).log10(),
            10.0 * (k / p.6).log10()
        );
    }
}
