//! FILMSHARP: the exact answers for a grid with no pulls, per budget, on a folder of PGM frames.
//!
//! Run: cargo run --release --example filmsharp_laws -- <frames dir>
//!
//! With no pulls each pixel is a lone thing with yes-probability p = its grey. Two update rules, two laws:
//!   Gibbs (independent coins):      MSE_i = p q / K                      (the coin-noise law)
//!   Metropolised Gibbs (two-state): the count distribution propagated exactly from a coin-flip start (lone_mse)
//!   ghost cluster step (two-state, positively correlated): the same propagation (lone_mse_cluster)
//! Median over frames of 10 log10(1 / mean_i MSE_i). Greys are 8-bit, so the work is done once per grey level.

use settle::filmsharp::{lone_mse, lone_mse_cluster};
use settle::grid::read_pgm;
use std::path::PathBuf;

fn main() {
    let dir = PathBuf::from(std::env::args().nth(1).expect("usage: filmsharp_laws <frames dir>"));
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().map(|x| x == "pgm").unwrap_or(false)).collect();
    files.sort();
    let frames: Vec<Vec<f64>> = files.iter().map(|f| read_pgm(f).unwrap().px).collect();
    let budgets = [5usize, 10, 20, 40, 80, 1000];
    println!("{} frames from {}", frames.len(), dir.display());
    println!("{:>6} {:>12} {:>14} {:>16} {:>15}", "K", "coin law dB", "metro law dB", "metro stationary", "cluster law dB");
    for &k in &budgets {
        let mut table = [[0.0f64; 4]; 256];
        for (v, row) in table.iter_mut().enumerate() {
            let p = (v as f64 / 255.0).clamp(0.001, 0.999);
            row[0] = p * (1.0 - p) / k as f64;
            row[1] = lone_mse(p, k, 0.5, true);
            row[2] = p * (1.0 - p) * (2.0 * p - 1.0).abs() / k as f64;
            row[3] = lone_mse_cluster(p, k, 0.5);
        }
        let mut per = [Vec::new(), Vec::new(), Vec::new(), Vec::new()];
        for f in &frames {
            for c in 0..4 {
                let mse = f.iter().map(|g| table[(g * 255.0).round() as usize][c]).sum::<f64>() / f.len() as f64;
                per[c].push(10.0 * (1.0 / mse).log10());
            }
        }
        let med = |v: &Vec<f64>| {
            let mut s = v.clone();
            s.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let n = s.len();
            if n % 2 == 1 { s[n / 2] } else { (s[n / 2 - 1] + s[n / 2]) / 2.0 }
        };
        println!("{:>6} {:>12.2} {:>14.2} {:>16.2} {:>15.2}", k, med(&per[0]), med(&per[1]), med(&per[2]), med(&per[3]));
    }
}
