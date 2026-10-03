//! FILMSHARP: how correlated in time is the chain? Pooled autocorrelation of the bits and of the Rao-Blackwellised
//! value tanh(I) on real frames, and the variance-inflation factor 2 tau each carries into a K-sweep average.
//!
//! Run: cargo run --release --example filmsharp_tau -- <frames dir> <J> <update> <tap|fit> [T sweeps] [L lags] [frames]
//!   update: gibbs | checker | metro | metro_checker | cluster
//!
//! For each of the first `frames` frames: leans (TAP, or TAP then 8 x 400-sweep secant fit), a chain from coin flips,
//! 300 burn-in sweeps, then T recorded sweeps. For a series x_t per pixel, the pooled autocorrelation is
//!   rho(k) = sum_i [ C_i(k) - xbar_i^2 ] / sum_i var_i,   C_i(k) = mean_t x_i(t) x_i(t + k)
//! and tau = 1/2 + sum_{k=1..M} rho(k), with M the first lag at which M >= 6 tau (Sokal's window).
//! The K-sweep average then has MSE about sum_i var_i 2 tau / K per pixel, which is what the PSNR pays.
//! Printed per frame and pooled: 2 tau for bits and for tanh(I), the per-sample variance ratio var(tanh I) / var(s),
//! and the PSNR each predicts at K = 80 against the frame's own grey (bias not included).

use settle::filmsharp::{fit_leans, Sweeper, Update};
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

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let dir = PathBuf::from(&a[1]);
    let j: f64 = a[2].parse().unwrap();
    let upd = match a[3].as_str() {
        "gibbs" => Update::Gibbs,
        "checker" => Update::Checker,
        "metro" => Update::Metro,
        "metro_checker" => Update::MetroChecker,
        "cluster" => Update::Cluster,
        u => panic!("unknown update {}", u),
    };
    let fit = a[4] == "fit";
    let t_rec: usize = a.get(5).map(|s| s.parse().unwrap()).unwrap_or(2000);
    let lags: usize = a.get(6).map(|s| s.parse().unwrap()).unwrap_or(100);
    let nfr: usize = a.get(7).map(|s| s.parse().unwrap()).unwrap_or(3);
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().map(|x| x == "pgm").unwrap_or(false)).collect();
    files.sort();
    println!("J {} update {:?} leans {} T {} lags {} frames {}", j, upd, a[4], t_rec, lags, nfr);
    println!("{:>12} {:>9} {:>9} {:>10} {:>10} {:>9} {:>9}", "frame", "2tau bits", "2tau rb", "var ratio", "window ok", "dB bits80", "dB rb80");
    let (mut vb_all, mut vr_all, mut tb_all, mut tr_all) = (0.0, 0.0, 0.0, 0.0);
    for (fi, f) in files.iter().take(nfr).enumerate() {
        let pic = read_pgm(f).unwrap();
        let (mut m, g) = grid(pic.w, pic.h, j);
        let n = pic.w * pic.h;
        let target = magnetisations(&pic);
        m.h = leans_for(&m, &g, &target, 1.0, Invert::Tap);
        if fit {
            let h0 = m.h.clone();
            m.h = fit_leans(&mut m, &g, &target, h0, upd, 8, 400, 1.0, 77 + fi as u64).0;
        }
        let mut rng = Rng::new(5 + fi as u64);
        let mut s: Vec<f64> = (0..n).map(|_| if rng.unit() < 0.5 { -1.0 } else { 1.0 }).collect();
        let mut free: Vec<usize> = (0..n).collect();
        let mut sw = Sweeper::new(upd, &g, &free);
        for _ in 0..300 {
            sw.sweep(&m, &g, &mut s, &mut free, &mut rng, 1.0, None);
        }
        let (mut ab, mut ar) = (Acf::new(n, lags), Acf::new(n, lags));
        let mut xr = vec![0.0; n];
        for _ in 0..t_rec {
            sw.sweep(&m, &g, &mut s, &mut free, &mut rng, 1.0, None);
            for i in 0..n {
                xr[i] = m.input(i, &s).tanh();
            }
            ab.push(&s);
            ar.push(&xr);
        }
        let ((vb, rb), (vr, rr)) = (ab.rho(), ar.rho());
        let ((tb, okb), (tr, okr)) = (tau(&rb), tau(&rr));
        // variances are of s in {-1, 1}; the grey is (1 + s) / 2, so its variance is a quarter
        let db = |v: f64, t: f64| 10.0 * (1.0 / (v / 4.0 / n as f64 * 2.0 * t / 80.0)).log10();
        println!(
            "{:>12} {:>9.3} {:>9.3} {:>10.4} {:>10} {:>9.2} {:>9.2}",
            f.file_name().unwrap().to_string_lossy(),
            2.0 * tb,
            2.0 * tr,
            vr / vb,
            format!("{}/{}", okb, okr),
            db(vb, tb),
            db(vr, tr)
        );
        vb_all += vb;
        vr_all += vr;
        tb_all += vb * tb;
        tr_all += vr * tr;
    }
    println!(
        "pooled: 2tau bits {:.3} (variance-weighted), 2tau rb {:.3}, var ratio {:.4}",
        2.0 * tb_all / vb_all,
        2.0 * tr_all / vr_all,
        vr_all / vb_all
    );
}
