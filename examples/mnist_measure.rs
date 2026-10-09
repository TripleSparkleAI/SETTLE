//! MNIST measurements for lane MNIST (`src/mnist.rs`).
//!
//! Run: `cargo run --release --example mnist_measure -- <part> [args]` from settle-rs, part one of
//!   pilot                         joint machine (PILOT_H hidden, PILOT_EPOCHS, PILOT_RATE, PILOT_PCD), default 100 hidden, 3 epochs on train rows 0..50,000, scored on train
//!                                 rows 50,000..60,000 only (the test split is not read)
//!   baselines                     nearest centroid, logistic regression on the pixels (seeds 1 2 3), and a
//!                                 logistic regression on shuffled labels
//!   joint <H> <seed> [pcd] [shuffle]   joint machine (784 pixels + 10 label things, H hidden), 60,000 train,
//!                                 scored on the 10,000 test digits: exact readout, settled readout at 200
//!                                 sweeps, and settled accuracy against sweeps
//!   features <H> <seed>           pixel-only machine, then logistic regression on the hidden yes-rates
//!   export <weights.bin> <out.json> <n>   the first n test digits with the Rust readouts, for the browser port
//! Environment: MNIST_DIR (default ../runs/mnist/data), MNIST_OUT (default ../runs/mnist), THREADS (default 6),
//! EPOCHS (default 20), CDK (contrastive-divergence steps, default 1); a non-default EPOCHS or CDK is added to
//! the arm's name (for example joint_h500_s1_e60).
//! Every part prints `ROW,...` CSV lines beside the human lines and stamps the load and power mode.

use settle::mnist::*;
use settle::rng::Rng;
use std::fs;
use std::io::Write;
use std::time::Instant;

fn stamp(tag: &str) -> String {
    let up = std::process::Command::new("uptime").output().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
    let pm = std::process::Command::new("sh")
        .args(["-c", "pmset -g | grep -i powermode | awk '{print $2}'"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    let s = format!("STAMP,{},powermode {},{}", tag, pm, up);
    println!("{}", s);
    s
}

fn env_or(k: &str, d: &str) -> String {
    std::env::var(k).unwrap_or_else(|_| d.to_string())
}

fn opts(seed: u64, persistent: bool) -> TrainOpts {
    TrainOpts {
        epochs: env_or("EPOCHS", "20").parse().unwrap(),
        rate: 0.005,
        batch: 100,
        momentum: 0.9,
        momentum_from: 5,
        decay: 0.0002,
        cd_k: env_or("CDK", "1").parse().unwrap(),
        persistent,
        seed,
        threads: env_or("THREADS", "6").parse().unwrap(),
    }
}

const SM: SoftmaxOpts = SoftmaxOpts { epochs: 30, rate: 0.001, batch: 100, l2: 1e-4, seed: 1 };

fn pixels01(d: &Digits) -> Vec<f32> {
    d.images.iter().map(|&p| if p >= THRESHOLD { 1.0 } else { 0.0 }).collect()
}

/// Exact and settled readouts over rows of pixels; settled at the given sweep counts, threads over digits.
fn score_joint(m: &Rbm, px: &[f32], ys: &[u8], sweeps: &[usize], seed: u64, threads: usize) -> (Vec<usize>, Vec<Vec<usize>>) {
    let n = ys.len();
    let per = n.div_ceil(threads);
    let mut exact = vec![0usize; n];
    let mut settled = vec![vec![0usize; n]; sweeps.len()];
    std::thread::scope(|sc| {
        let mut hs = Vec::new();
        for t in 0..threads {
            let lo = t * per;
            let hi = ((t + 1) * per).min(n);
            if lo >= hi {
                continue;
            }
            hs.push(sc.spawn(move || {
                let mut ex = Vec::new();
                let mut st = vec![Vec::new(); sweeps.len()];
                for i in lo..hi {
                    let xp = pixel_inputs(m, &px[i * PIX..(i + 1) * PIX]);
                    ex.push(exact_from(m, &xp).0);
                    for (j, &s) in sweeps.iter().enumerate() {
                        let mut coin = Rng::new(mix(seed, i as u64, 1000 + s as u64));
                        st[j].push(settle_from(m, &xp, s, &mut coin).0);
                    }
                }
                (lo, ex, st)
            }));
        }
        for h in hs {
            let (lo, ex, st) = h.join().unwrap();
            for (k, v) in ex.into_iter().enumerate() {
                exact[lo + k] = v;
            }
            for (j, col) in st.into_iter().enumerate() {
                for (k, v) in col.into_iter().enumerate() {
                    settled[j][lo + k] = v;
                }
            }
        }
    });
    (exact, settled)
}

fn acc(pred: &[usize], ys: &[u8]) -> f64 {
    100.0 * pred.iter().zip(ys).filter(|(p, y)| **p == **y as usize).count() as f64 / ys.len() as f64
}

fn confusion(pred: &[usize], ys: &[u8]) -> Vec<Vec<usize>> {
    let mut c = vec![vec![0usize; CLASSES]; CLASSES];
    for (p, y) in pred.iter().zip(ys) {
        c[*y as usize][*p] += 1;
    }
    c
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let dir = env_or("MNIST_DIR", "../runs/mnist/data");
    let out = env_or("MNIST_OUT", "../runs/mnist");
    let threads: usize = env_or("THREADS", "6").parse().unwrap();
    let part = args.first().map(|s| s.as_str()).unwrap_or("");
    let t0 = Instant::now();
    stamp(&format!("start {}", args.join(" ")));
    match part {
        "pilot" => {
            let tr = load(&dir, "train").unwrap();
            let rows = tr.rows(true, &tr.labels);
            let nv = PIX + CLASSES;
            let (fit, val) = rows.split_at(50_000 * nv);
            let val_px: Vec<f32> = val.chunks_exact(nv).flat_map(|r| r[..PIX].to_vec()).collect();
            let val_y = &tr.labels[50_000..];
            let ph: usize = env_or("PILOT_H", "100").parse().unwrap();
            let mut m = Rbm::new(nv, ph, 0.01, &mut Rng::new(1));
            m.init_leans(fit);
            let mut o = opts(1, env_or("PILOT_PCD", "0") == "1");
            o.epochs = env_or("PILOT_EPOCHS", "3").parse().unwrap();
            o.rate = env_or("PILOT_RATE", "0.005").parse().unwrap();
            o.momentum_from = env_or("PILOT_MOM_FROM", "5").parse().unwrap();
            train(&mut m, fit, &o, &mut |e, m| {
                let (ex, st) = score_joint(m, &val_px[..2000 * PIX], &val_y[..2000], &[100], 5, threads);
                println!(
                    "ROW,pilot,epoch,{},recon,{:.5},secs,{:.1},val2000_exact,{:.2},val2000_settled100,{:.2}",
                    e.epoch,
                    e.recon,
                    e.secs,
                    acc(&ex, &val_y[..2000]),
                    acc(&st[0], &val_y[..2000])
                );
            });
            let fit_px: Vec<f32> = fit.chunks_exact(nv).flat_map(|r| r[..PIX].iter().map(|&x| (x + 1.0) / 2.0).collect::<Vec<_>>()).collect();
            let val01: Vec<f32> = val_px.iter().map(|&x| (x + 1.0) / 2.0).collect();
            let sm = Softmax::fit(&fit_px, PIX, &tr.labels[..50_000], &SM);
            let pred: Vec<usize> = val01.chunks_exact(PIX).map(|r| sm.predict(r)).collect();
            println!("ROW,pilot,logreg_pixels_val10000,{:.2}", acc(&pred, val_y));
        }
        "baselines" => {
            let (tr, te) = (load(&dir, "train").unwrap(), load(&dir, "test").unwrap());
            let (xtr, xte) = (pixels01(&tr), pixels01(&te));
            let nc = nearest_centroid(&xtr, &tr.labels, &xte, PIX);
            println!("ROW,baseline,nearest_centroid,test_acc,{:.2}", acc(&nc, &te.labels));
            for seed in 1..=3u64 {
                let sm = Softmax::fit(&xtr, PIX, &tr.labels, &SoftmaxOpts { seed, ..SM });
                let pred: Vec<usize> = xte.chunks_exact(PIX).map(|r| sm.predict(r)).collect();
                let trp: Vec<usize> = xtr.chunks_exact(PIX).map(|r| sm.predict(r)).collect();
                println!("ROW,baseline,logreg_pixels,seed,{},test_acc,{:.2},train_acc,{:.2}", seed, acc(&pred, &te.labels), acc(&trp, &tr.labels));
                if seed == 1 {
                    let cm = confusion(&pred, &te.labels);
                    fs::write(format!("{}/confusion_logreg_pixels_s1.json", out), serde_like(&cm)).unwrap();
                }
            }
            let sy = shuffled_labels(&tr.labels, 1);
            let sm = Softmax::fit(&xtr, PIX, &sy, &SM);
            let pred: Vec<usize> = xte.chunks_exact(PIX).map(|r| sm.predict(r)).collect();
            println!("ROW,control,logreg_pixels_shuffled_labels,seed,1,test_acc,{:.2}", acc(&pred, &te.labels));
        }
        "joint" => {
            let h: usize = args[1].parse().unwrap();
            let seed: u64 = args[2].parse().unwrap();
            let pcd = args.iter().any(|a| a == "pcd");
            let shuf = args.iter().any(|a| a == "shuffle");
            let (tr, te) = (load(&dir, "train").unwrap(), load(&dir, "test").unwrap());
            let labels = if shuf { shuffled_labels(&tr.labels, seed) } else { tr.labels.clone() };
            let rows = tr.rows(true, &labels);
            let nv = PIX + CLASSES;
            let name = format!("joint_h{}_s{}{}{}{}", h, seed, if pcd { "_pcd" } else { "" }, if shuf { "_shuffled" } else { "" }, variant());
            let mut m = Rbm::new(nv, h, 0.01, &mut Rng::new(mix(seed, 0x1417, 2)));
            m.init_leans(&rows);
            let tr_px: Vec<f32> = rows.chunks_exact(nv).take(2000).flat_map(|r| r[..PIX].to_vec()).collect();
            let mut curve = String::new();
            let mut o = opts(seed, pcd);
            if pcd {
                // the sealed persistent settings: a gentler rate and momentum held at 0.5 (0.9 diverged in the pilot)
                o.rate = 0.002;
                o.momentum_from = usize::MAX;
            }
            train(&mut m, &rows, &o, &mut |e, m| {
                let (ex, _) = score_joint(m, &tr_px, &labels[..2000], &[], 5, threads);
                let line = format!("ROW,{},epoch,{},recon,{:.5},secs,{:.1},train2000_exact,{:.2}", name, e.epoch, e.recon, e.secs, acc(&ex, &labels[..2000]));
                println!("{}", line);
                curve.push_str(&line);
                curve.push('\n');
            });
            fs::create_dir_all(format!("{}/weights", out)).unwrap();
            m.save(&format!("{}/weights/{}.bin", out, name)).unwrap();
            let te_rows = te.rows(false, &te.labels);
            let sweeps = [1usize, 3, 10, 30, 100, 200];
            let ts = Instant::now();
            let (ex, st) = score_joint(&m, &te_rows, &te.labels, &sweeps, seed * 7919, threads);
            println!("ROW,{},test_exact,{:.2}", name, acc(&ex, &te.labels));
            for (j, s) in sweeps.iter().enumerate() {
                println!("ROW,{},test_settled,sweeps,{},acc,{:.2}", name, s, acc(&st[j], &te.labels));
            }
            println!("ROW,{},classify_secs_all_sweeps,{:.1}", name, ts.elapsed().as_secs_f64());
            let cm = confusion(&st[5], &te.labels);
            fs::write(format!("{}/confusion_{}_settled200.json", out, name), serde_like(&cm)).unwrap();
            fs::write(format!("{}/curve_{}.csv", out, name), curve).unwrap();
        }
        "features" => {
            let h: usize = args[1].parse().unwrap();
            let seed: u64 = args[2].parse().unwrap();
            let (tr, te) = (load(&dir, "train").unwrap(), load(&dir, "test").unwrap());
            let rows = tr.rows(false, &tr.labels);
            let name = format!("pixels_h{}_s{}{}", h, seed, variant());
            let mut m = Rbm::new(PIX, h, 0.01, &mut Rng::new(mix(seed, 0xFEA7, 2)));
            m.init_leans(&rows);
            let o = opts(seed, false);
            train(&mut m, &rows, &o, &mut |e, _| {
                println!("ROW,{},epoch,{},recon,{:.5},secs,{:.1}", name, e.epoch, e.recon, e.secs);
            });
            fs::create_dir_all(format!("{}/weights", out)).unwrap();
            m.save(&format!("{}/weights/{}.bin", out, name)).unwrap();
            let feats = |rows: &[f32]| {
                let mut f = vec![0f32; rows.len() / PIX * h];
                for (r, o) in rows.chunks_exact(PIX).zip(f.chunks_exact_mut(h)) {
                    hidden_rates(&m, r, o);
                }
                f
            };
            let te_rows = te.rows(false, &te.labels);
            let (ftr, fte) = (feats(&rows), feats(&te_rows));
            let sm = Softmax::fit(&ftr, h, &tr.labels, &SoftmaxOpts { seed, ..SM });
            let pred: Vec<usize> = fte.chunks_exact(h).map(|r| sm.predict(r)).collect();
            let trp: Vec<usize> = ftr.chunks_exact(h).map(|r| sm.predict(r)).collect();
            println!("ROW,{},readout,test_acc,{:.2},train_acc,{:.2}", name, acc(&pred, &te.labels), acc(&trp, &tr.labels));
            fs::write(format!("{}/confusion_{}_readout.json", out, name), serde_like(&confusion(&pred, &te.labels))).unwrap();
            if seed == 1 {
                let sy = shuffled_labels(&tr.labels, seed);
                let sm = Softmax::fit(&ftr, h, &sy, &SoftmaxOpts { seed, ..SM });
                let pred: Vec<usize> = fte.chunks_exact(h).map(|r| sm.predict(r)).collect();
                println!("ROW,control,{}_readout_shuffled_labels,test_acc,{:.2}", name, acc(&pred, &te.labels));
            }
        }
        "export" => {
            let m = load_rbm(&args[1]).unwrap();
            let n: usize = args[3].parse().unwrap();
            let te = load(&dir, "test").unwrap();
            let sweeps = 200;
            let mut f = fs::File::create(&args[2]).unwrap();
            writeln!(f, "{{\"weights\": \"{}\", \"sweeps\": {}, \"coin\": \"xs128 seed 1000+i\", \"digits\": [", args[1], sweeps).unwrap();
            for i in 0..n {
                let px = spins(te.image(i));
                let (ex, scores) = exact_label(&m, &px);
                let mut coin = Xs128::new(1000 + i as u32);
                let (st, cnt) = settle_label(&m, &px, sweeps, &mut coin);
                let bits: String = px.iter().map(|&x| if x > 0.0 { '1' } else { '0' }).collect();
                writeln!(
                    f,
                    "  {{\"i\": {}, \"label\": {}, \"bits\": \"{}\", \"exact\": {}, \"scores\": [{}], \"settled\": {}, \"counts\": [{}]}}{}",
                    i,
                    te.labels[i],
                    bits,
                    ex,
                    scores.iter().map(|s| format!("{:.6}", s)).collect::<Vec<_>>().join(","),
                    st,
                    cnt.iter().map(|c| c.to_string()).collect::<Vec<_>>().join(","),
                    if i + 1 < n { "," } else { "" }
                )
                .unwrap();
            }
            writeln!(f, "]}}").unwrap();
        }
        _ => {
            eprintln!("parts: pilot | baselines | joint <H> <seed> [pcd] [shuffle] | features <H> <seed> | export <w.bin> <out.json> <n>");
            std::process::exit(2);
        }
    }
    println!("ROW,wall_secs,{:.1}", t0.elapsed().as_secs_f64());
    stamp("end");
}

/// A suffix naming any setting that differs from the sealed defaults (EPOCHS=20, CDK=1), so weights never collide.
fn variant() -> String {
    let e = env_or("EPOCHS", "20");
    let k = env_or("CDK", "1");
    format!("{}{}", if e != "20" { format!("_e{}", e) } else { String::new() }, if k != "1" { format!("_cd{}", k) } else { String::new() })
}

fn serde_like(m: &[Vec<usize>]) -> String {
    format!("[{}]", m.iter().map(|r| format!("[{}]", r.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(","))).collect::<Vec<_>>().join(","))
}
