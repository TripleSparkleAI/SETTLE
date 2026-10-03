//! FILMWARM: a fitted-lean player that warm-starts each frame's fit from the previous frame. No statements of its
//! own; `grid`'s `play` uses it through four options:
//!
//! ```text
//! play :img, frames: "f/", sweeps: 1000, read: :soft, correct: :tap, fit: 10, fit_sweeps: 400, fit_update: :cluster,
//!            warm_fit: 1, warm_fit_sweeps: 400, warm_from: :leans | :correction, cut: 0.25
//! ```
//!
//! The first frame is fitted cold exactly as FILMSHARP does (`fit:` iterations of `fit_sweeps:` sweeps from the TAP
//! leans, a fresh chain from coin flips). Every later frame is fitted WARM: `warm_fit:` iterations of
//! `warm_fit_sweeps:` sweeps, starting from
//!   `:leans`       the previous frame's fitted leans,   h0 = h_prev
//!   `:correction`  the previous frame's correction carried onto this frame's TAP leans,
//!                  h0 = h_tap(now) + (h_prev - h_tap(prev))
//! and continuing the previous frame's fit chain (its bits), not a fresh one. The step is FILMSHARP's
//! h <- h + eta P (m* - m_hat) with the positive-definite TAP preconditioner; eta starts at 1 each frame.
//!
//! `warm_step: e` (1 when absent) sets the first step size of each warm frame's fit; 0 keeps the start leans (the
//! fit then only measures). Added after the sealed runs, see the report.
//!
//! `cut: X` (off when absent) is a free cut detector: when the RMS grey change between this frame's target and the
//! previous one exceeds X, the frame is fitted cold instead (full budget, TAP start, fresh chain).
//!
//! Cost is counted in sweeps: a frame's fit spends iterations x sweeps, reported per frame.

use crate::filmsharp::{precond_fit_from, Sweeper, Update};
use crate::grid::Spec;
use crate::lex::{err, kw, num, SettleError, Tok};
use crate::model::Model;
use crate::rng::Rng;

/// Where a warm fit starts its leans.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WarmFrom {
    Leans,
    Correction,
}

/// The warm fit's settings and what it carries from frame to frame.
pub struct WarmFit {
    pub iters: usize,
    pub sweeps: usize,
    pub from: WarmFrom,
    /// RMS grey change above which a frame is fitted cold (0 = never).
    pub cut: f64,
    /// The first step size of a warm frame's fit (`warm_step:`, 1 when absent, as a cold fit's). Added after the
    /// sealed runs showed a full first step on one short measurement injects more noise than it removes.
    pub step: f64,
    leans: Vec<f64>,
    tap: Vec<f64>,
    grey: Vec<f64>,
    chain: Vec<f64>,
}

/// What one frame's fit did.
pub struct FitRecord {
    pub res: Vec<f64>,
    pub warm: bool,
    pub sweeps: usize,
    /// RMS grey change from the previous target (0 on the first frame).
    pub change: f64,
}

impl WarmFit {
    pub fn new(iters: usize, sweeps: usize, from: WarmFrom, cut: f64) -> Self {
        WarmFit { iters, sweeps, from, cut, step: 1.0, leans: Vec::new(), tap: Vec::new(), grey: Vec::new(), chain: Vec::new() }
    }

    /// Fit this frame's leans, warm when there is a previous frame and no cut, cold otherwise. `h_tap` are this
    /// frame's closed-form (TAP) leans, `grey` its target greys, `cold` the (iterations, sweeps) of a cold fit.
    /// Writes the fitted leans into the model and remembers them for the next frame.
    #[allow(clippy::too_many_arguments)]
    pub fn fit_frame(&mut self, m: &mut Model, g: &Spec, target: &[f64], grey: &[f64], h_tap: Vec<f64>, cold: (usize, usize), rule: Update, beta: f64, seed: u64, floor: f64) -> FitRecord {
        let n = g.w * g.h;
        let change = if self.grey.len() == n { rms_change(&self.grey, grey) } else { 0.0 };
        let warm = self.leans.len() == n && !(self.cut > 0.0 && change > self.cut);
        let (h0, iters, sweeps, eta0) = if warm {
            let h0 = match self.from {
                WarmFrom::Leans => self.leans.clone(),
                WarmFrom::Correction => (0..n).map(|k| h_tap[k] + self.leans[k] - self.tap[k]).collect(),
            };
            (h0, self.iters, self.sweeps, self.step)
        } else {
            self.chain.clear();
            (h_tap.clone(), cold.0, cold.1, 1.0)
        };
        let (h, res) = precond_leans_chain_from(m, g, target, h0, rule, iters, sweeps, beta, seed, floor, eta0, &mut self.chain);
        self.leans = h;
        self.tap = h_tap;
        self.grey = grey.to_vec();
        FitRecord { res, warm, sweeps: iters * sweeps.max(4), change }
    }
}

/// RMS difference of two grey pictures.
pub fn rms_change(a: &[f64], b: &[f64]) -> f64 {
    (a.iter().zip(b).map(|(x, y)| (x - y) * (x - y)).sum::<f64>() / a.len().max(1) as f64).sqrt()
}

/// FILMSHARP's `precond_leans` with the chain handed in and out. An empty `chain` starts from coin flips drawn
/// from `seed` exactly as `precond_leans` does (so a cold fit here is bit-identical to it); a full one continues.
#[allow(clippy::too_many_arguments)]
pub fn precond_leans_chain(m: &mut Model, g: &Spec, target: &[f64], h0: Vec<f64>, rule: Update, iters: usize, sweeps: usize, beta: f64, seed: u64, floor: f64, chain: &mut Vec<f64>) -> (Vec<f64>, Vec<f64>) {
    precond_leans_chain_from(m, g, target, h0, rule, iters, sweeps, beta, seed, floor, 1.0, chain)
}

/// The same with the first step size `eta0`.
#[allow(clippy::too_many_arguments)]
pub fn precond_leans_chain_from(m: &mut Model, g: &Spec, target: &[f64], h0: Vec<f64>, rule: Update, iters: usize, sweeps: usize, beta: f64, seed: u64, floor: f64, eta0: f64, chain: &mut Vec<f64>) -> (Vec<f64>, Vec<f64>) {
    let n = g.w * g.h;
    let mut rng = Rng::new(seed);
    if chain.len() != m.len() {
        *chain = (0..m.len()).map(|_| if rng.unit() < 0.5 { -1.0 } else { 1.0 }).collect();
    }
    let mut free: Vec<usize> = (g.start..g.start + n).collect();
    let mut sw = Sweeper::new(rule, g, &free);
    let sweeps = sweeps.max(4);
    let burn = sweeps / 4;
    let s = chain;
    precond_fit_from(m, g, target, h0, iters, floor, true, eta0, |mm: &Model| {
        let mut acc = vec![0.0; n];
        for k in 0..sweeps {
            let rb = if k >= burn { Some(&mut acc[..]) } else { None };
            sw.sweep(mm, g, s, &mut free, &mut rng, beta, rb);
        }
        acc.iter().map(|a| a / (sweeps - burn) as f64).collect()
    })
}

/// Read `warm_fit:`, `warm_fit_sweeps:` (the cold `fit_sweeps` when absent), `warm_from:` (:leans when absent) and
/// `cut:` (off when absent). `None` when `warm_fit:` is absent or 0.
pub fn warm_opts(kv: &[(String, Tok)], fit: usize, fit_sweeps: usize, ln: usize) -> Result<Option<WarmFit>, SettleError> {
    let iters = kw(kv, "warm_fit").map(|v| num(v, ln)).transpose()?.unwrap_or(0.0);
    if iters < 0.0 || iters > 1000.0 || iters.fract() != 0.0 {
        return err(ln, "warm_fit must be a whole number from 0 to 1000");
    }
    let sweeps = kw(kv, "warm_fit_sweeps").map(|v| num(v, ln)).transpose()?.unwrap_or(fit_sweeps as f64);
    if sweeps < 4.0 || sweeps.fract() != 0.0 {
        return err(ln, "warm_fit_sweeps must be a whole number of at least 4");
    }
    let from = match kw(kv, "warm_from") {
        None => WarmFrom::Leans,
        Some(Tok::Sym(s)) if s == "leans" => WarmFrom::Leans,
        Some(Tok::Sym(s)) if s == "correction" => WarmFrom::Correction,
        Some(_) => return err(ln, "warm_from: takes :leans or :correction"),
    };
    let cut = kw(kv, "cut").map(|v| num(v, ln)).transpose()?.unwrap_or(0.0);
    let step = kw(kv, "warm_step").map(|v| num(v, ln)).transpose()?.unwrap_or(1.0);
    if !(0.0..=1.0).contains(&step) {
        return err(ln, "warm_step must be from 0 (keep the previous leans) to 1");
    }
    if !(0.0..=1.0).contains(&cut) {
        return err(ln, "cut must be an RMS grey change from 0 (off) to 1");
    }
    if iters == 0.0 {
        if kw(kv, "warm_fit_sweeps").is_some() || kw(kv, "warm_from").is_some() || kw(kv, "cut").is_some() || kw(kv, "warm_step").is_some() {
            return err(ln, "warm_fit_sweeps:, warm_from:, warm_step: and cut: need warm_fit: 1 or more");
        }
        return Ok(None);
    }
    if fit == 0 {
        return err(ln, "warm_fit: needs fit: (the first frame's cold fit)");
    }
    let mut w = WarmFit::new(iters as usize, sweeps as usize, from, cut);
    w.step = step;
    Ok(Some(w))
}

pub fn from_word(f: WarmFrom) -> &'static str {
    match f {
        WarmFrom::Leans => "leans",
        WarmFrom::Correction => "correction",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filmsharp::{precond_fit, precond_leans};
    use crate::grid::{declare, leans_for, load, Invert};
    use crate::model::{exact_rates, State};

    fn grid(w: usize, h: usize, j: f64) -> (Model, Spec) {
        let mut m = Model::default();
        declare(&mut m, "g", &[Tok::Label("width".into()), Tok::Num(w as f64), Tok::Comma, Tok::Label("height".into()), Tok::Num(h as f64), Tok::Comma, Tok::Label("smooth".into()), Tok::Num(j)], 1).unwrap();
        let g = load(&m, "g", 1).unwrap();
        (m, g)
    }

    fn target(seed: u64, n: usize) -> Vec<f64> {
        let mut r = Rng::new(seed);
        (0..n).map(|_| 1.4 * r.unit() - 0.7).collect()
    }

    #[test]
    fn a_cold_fit_with_an_empty_chain_is_bit_identical_to_filmsharps() {
        let (mut m, g) = grid(12, 8, 0.4);
        let t = target(3, 96);
        let h0 = leans_for(&m, &g, &t, 1.0, Invert::Tap);
        let (a, ra) = precond_leans(&mut m, &g, &t, h0.clone(), Update::Cluster, 4, 40, 1.0, 17, 0.05);
        let mut chain = Vec::new();
        let (b, rb) = precond_leans_chain(&mut m, &g, &t, h0, Update::Cluster, 4, 40, 1.0, 17, 0.05, &mut chain);
        assert_eq!(a, b);
        assert_eq!(ra, rb);
        assert_eq!(chain.len(), m.len());
    }

    #[test]
    fn a_warm_fit_starts_from_the_previous_leans_or_the_carried_correction() {
        // exact identities: with zero warm iterations the warm fit returns its start, so the start can be read.
        // :leans starts at the previous fitted leans; :correction at this frame's TAP leans plus the previous
        // frame's (fitted - TAP). Vacuity control: the two starts differ when the target moved.
        let (w, h) = (12, 8);
        let (mut m, g) = grid(w, h, 0.4);
        let t1 = target(21, w * h);
        let t2: Vec<f64> = t1.iter().enumerate().map(|(k, v)| if k % 3 == 0 { -v } else { *v }).collect();
        let grey = |t: &[f64]| t.iter().map(|v| (1.0 + v) / 2.0).collect::<Vec<f64>>();
        let (tap1, tap2) = (leans_for(&m, &g, &t1, 1.0, Invert::Tap), leans_for(&m, &g, &t2, 1.0, Invert::Tap));
        let mut starts = Vec::new();
        for from in [WarmFrom::Leans, WarmFrom::Correction] {
            let mut wf = WarmFit::new(0, 40, from, 0.0);
            let r1 = wf.fit_frame(&mut m, &g, &t1, &grey(&t1), tap1.clone(), (4, 40), Update::Gibbs, 1.0, 1, 0.05);
            assert!(!r1.warm && r1.sweeps == 160);
            let h1 = wf.leans.clone();
            assert!(h1.iter().zip(&tap1).any(|(a, b)| (a - b).abs() > 1e-3), "the cold fit moved the leans");
            let r2 = wf.fit_frame(&mut m, &g, &t2, &grey(&t2), tap2.clone(), (4, 40), Update::Gibbs, 1.0, 2, 0.05);
            assert!(r2.warm && r2.sweeps == 0 && r2.change > 0.1);
            let want: Vec<f64> = match from {
                WarmFrom::Leans => h1.clone(),
                WarmFrom::Correction => (0..w * h).map(|k| tap2[k] + h1[k] - tap1[k]).collect(),
            };
            assert_eq!(wf.leans, want);
            assert_eq!(&m.h[..w * h], &want[..]);
            starts.push(want);
        }
        let d = starts[0].iter().zip(&starts[1]).map(|(a, b)| (a - b).abs()).fold(0.0, f64::max);
        assert!(d > 0.1, "{}", d);
    }

    #[test]
    fn the_cut_detector_fits_cold_only_across_a_large_change() {
        let (mut m, g) = grid(10, 6, 0.3);
        let a = target(7, 60);
        let a2: Vec<f64> = a.iter().map(|v| (v + 0.02).min(0.99)).collect();
        let b: Vec<f64> = a.iter().map(|v| -v).collect();
        let grey = |t: &[f64]| t.iter().map(|v| (1.0 + v) / 2.0).collect::<Vec<f64>>();
        let mut wf = WarmFit::new(1, 40, WarmFrom::Correction, 0.2);
        let mut warm = Vec::new();
        for (k, t) in [&a, &a2, &b, &b].iter().enumerate() {
            let tap = leans_for(&m, &g, t, 1.0, Invert::Tap);
            let r = wf.fit_frame(&mut m, &g, t, &grey(t), tap, (3, 40), Update::Gibbs, 1.0, 10 + k as u64, 0.05);
            warm.push(r.warm);
        }
        assert_eq!(warm, vec![false, true, false, true]);
        // vacuity control: with the detector off the same large change is fitted warm
        let mut wf = WarmFit::new(1, 40, WarmFrom::Correction, 0.0);
        let mut warm = Vec::new();
        for (k, t) in [&a, &b].iter().enumerate() {
            let tap = leans_for(&m, &g, t, 1.0, Invert::Tap);
            warm.push(wf.fit_frame(&mut m, &g, t, &grey(t), tap, (3, 40), Update::Gibbs, 1.0, 10 + k as u64, 0.05).warm);
        }
        assert_eq!(warm, vec![false, true]);
    }

    #[test]
    fn correction_carry_equals_leans_carry_when_the_target_does_not_move() {
        // exact identity: with the same target the TAP leans cancel, so both starts are the same leans
        let (mut m, g) = grid(8, 6, 0.35);
        let t = target(9, 48);
        let grey: Vec<f64> = t.iter().map(|v| (1.0 + v) / 2.0).collect();
        let tap = leans_for(&m, &g, &t, 1.0, Invert::Tap);
        let mut out = Vec::new();
        for from in [WarmFrom::Leans, WarmFrom::Correction] {
            let mut wf = WarmFit::new(2, 40, from, 0.0);
            wf.fit_frame(&mut m, &g, &t, &grey, tap.clone(), (4, 40), Update::Checker, 1.0, 1, 0.05);
            wf.fit_frame(&mut m, &g, &t, &grey, tap.clone(), (4, 40), Update::Checker, 1.0, 2, 0.05);
            out.push(wf.leans.clone());
        }
        let d = out[0].iter().zip(&out[1]).map(|(a, b)| (a - b).abs()).fold(0.0, f64::max);
        assert!(d < 1e-12, "{}", d);
    }

    #[test]
    fn warm_fits_with_exact_moments_track_a_slowly_moving_target() {
        // exact answer on a 4x3 grid: with the exact magnetisations as the oracle, one warm step per frame keeps the
        // fit within a small residual of a target drifting by 0.02 a frame, while TAP alone stays off
        let (mut m, g) = grid(4, 3, 0.4);
        let base = target(13, 12);
        let mut h = leans_for(&m, &g, &base, 1.0, Invert::Tap);
        let mut worst_fit: f64 = 0.0;
        let mut tap_err: f64 = 0.0;
        for f in 0..12 {
            let t: Vec<f64> = base.iter().enumerate().map(|(k, v)| (v + 0.02 * f as f64 * if k % 2 == 0 { 1.0 } else { -1.0 }).clamp(-0.9, 0.9)).collect();
            let tap = leans_for(&m, &g, &t, 1.0, Invert::Tap);
            m.h.copy_from_slice(&tap);
            let e = exact_rates(&m, &State::new(0));
            tap_err = tap_err.max((0..12).map(|k| (2.0 * e[k] - 1.0 - t[k]).abs()).fold(0.0, f64::max));
            let iters = if f == 0 { 30 } else { 2 };
            let (hn, _) = precond_fit(&mut m, &g, &t, h.clone(), iters, 0.05, false, |mm: &Model| exact_rates(mm, &State::new(0)).iter().map(|v| 2.0 * v - 1.0).collect());
            h = hn;
            m.h.copy_from_slice(&h);
            let e = exact_rates(&m, &State::new(0));
            let wf = (0..12).map(|k| (2.0 * e[k] - 1.0 - t[k]).abs()).fold(0.0, f64::max);
            if f > 0 {
                worst_fit = worst_fit.max(wf);
            }
        }
        assert!(worst_fit < 0.01 && tap_err > 0.02, "warm fit worst {} TAP worst {}", worst_fit, tap_err);
    }

    #[test]
    fn play_with_warm_fit_runs_end_to_end_and_its_first_frame_repeats_the_cold_fit() {
        use crate::grid::{write_pgm, Pgm};
        use crate::interp::Interp;
        let d = std::env::temp_dir().join(format!("settle-filmwarm-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("in")).unwrap();
        for k in 0..3 {
            let px: Vec<f64> = (0..20 * 12).map(|i| if (i % 20) as i32 - 6 - k < 6 && (i % 20) as i32 - 6 - k >= 0 { 0.2 } else { 0.8 }).collect();
            write_pgm(&d.join("in").join(format!("f{}.pgm", k)), &Pgm { w: 20, h: 12, px }).unwrap();
        }
        let prog = |extra: &str| format!("model :m do\n  grid :g, width: 20, height: 12, smooth: 0.4\nend\nrun :m do\n  play :g, frames: \"in/\", sweeps: 50, read: :soft, correct: :tap, fit: 4, fit_sweeps: 40, fit_update: :cluster, seed: 3{}\nend", extra);
        let run = |src: String| {
            let mut it = Interp::default();
            it.base_dir = d.clone();
            it.exec(&src).unwrap_or_else(|e| panic!("{}", e))
        };
        let cold = run(prog(""));
        let warm = run(prog(", warm_fit: 1, warm_fit_sweeps: 40, warm_from: :correction"));
        let first = |out: &[String]| out[0].split("PSNR ").nth(1).unwrap().split(' ').next().unwrap().to_string();
        assert_eq!(first(&cold), first(&warm), "{:?} {:?}", cold, warm);
        assert!(warm[0].contains("fit cold 160 sweeps") && warm[1].contains("fit warm 40 sweeps"), "{:?}", warm);
        let last = warm.last().unwrap();
        assert!(last.contains("2 warm and 1 cold frames, fit sweeps 240 in all, 40.0 per frame after the first"), "{}", last);
        // a bad option is refused by line
        let mut it = Interp::default();
        it.base_dir = d.clone();
        assert!(it.exec(&prog(", warm_from: :leans")).is_err());
    }

    #[test]
    fn warm_step_zero_keeps_the_start_and_a_small_step_moves_less() {
        let (w, h) = (12, 8);
        let (mut m, g) = grid(w, h, 0.4);
        let t1 = target(31, w * h);
        let t2: Vec<f64> = t1.iter().map(|v| (v * 0.9).clamp(-0.9, 0.9)).collect();
        let grey = |t: &[f64]| t.iter().map(|v| (1.0 + v) / 2.0).collect::<Vec<f64>>();
        let (tap1, tap2) = (leans_for(&m, &g, &t1, 1.0, Invert::Tap), leans_for(&m, &g, &t2, 1.0, Invert::Tap));
        let mut moved = Vec::new();
        for step in [0.0, 0.25, 1.0] {
            let mut wf = WarmFit::new(1, 40, WarmFrom::Leans, 0.0);
            wf.step = step;
            wf.fit_frame(&mut m, &g, &t1, &grey(&t1), tap1.clone(), (4, 40), Update::Gibbs, 1.0, 1, 0.05);
            let h1 = wf.leans.clone();
            wf.fit_frame(&mut m, &g, &t2, &grey(&t2), tap2.clone(), (4, 40), Update::Gibbs, 1.0, 2, 0.05);
            moved.push(h1.iter().zip(&wf.leans).map(|(a, b)| (a - b).abs()).sum::<f64>());
        }
        assert!(moved[0] == 0.0 && moved[1] > 0.0 && moved[2] > 2.0 * moved[1], "{:?}", moved);
    }
}
