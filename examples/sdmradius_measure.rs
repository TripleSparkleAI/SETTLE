//! SDMRADIUS measurements: a activation radius chosen per address-noise level, a density-scaled pulls read, the trade-off
//! frontier across radii, and Hopfield at equal memory (`src/sdmradius.rs` on `src/sdmscale.rs`'s store).
//!
//! Run: `cargo run --release --example sdmradius_measure <part> [args]`, part one of
//!   theory                          S-map and Poisson-conditioned predictions and the chosen radii; no memory built
//!   capacity <target> <M,M,...>     address-read capacity at 10/20/30/40% with the activation radius chosen for <target>:
//!                                   snr (SDMSCALE's activation radius), 0.2, 0.3, 0.4 (critical-distance-optimal for that
//!                                   address-noise), p0.3 (Poisson-optimal for 30%), or r106 (a fixed activation radius)
//!   pulls <target> <M,M,...>        pulls read: fixed 0.4 n, density-scaled (eps 0.1), top-k (k = p M)
//!   frontier <M> <r,r,...>          address P90 at 10/30/40% for each activation radius (the trade-off curve)
//!   controls <target> <M,M,...>     never-stored read-addresses and the entry shuffle, address and density-pulls reads
//!   hopfield <M,M,...>              zero-temperature Hopfield at the counter budget M x n
//!   race                            trace 40%-address-noise failures at small loads (rival capture, mixtures)
//!   racetable                       the RACE predictor's P90/P50 for the measured radii
//! Env: SDMRADIUS_Q (queries per address-noise per seed), SDMRADIUS_SEEDS (count), SDMSCALE_THREADS (threads).
//! Everything is seeded; every part stamps UTC, load and power mode. Raw rows go to stdout as `ROW,...`.

use settle::rng::Rng;
use settle::sdm::radius_for;
use settle::sdmradius::*;
use settle::sdmscale::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Instant;

const DMG: [f64; 4] = [0.1, 0.2, 0.3, 0.4];
const OK: f64 = 0.95;
const N: usize = 256;

fn stamp(tag: &str) {
    let up = std::process::Command::new("uptime").output().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
    let pm = std::process::Command::new("sh")
        .args(["-c", "pmset -g | grep -i powermode | awk '{print $2}'"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    let utc = std::process::Command::new("date").args(["-u", "+%Y-%m-%dT%H:%M:%SZ"]).output().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
    println!("STAMP {} utc={} load=[{}] powermode={} threads={}", tag, utc, up, pm, threads());
}

fn env_usize(k: &str, d: usize) -> usize {
    std::env::var(k).ok().and_then(|s| s.parse().ok()).unwrap_or(d)
}

fn snr_radius(n: usize, m: usize) -> usize {
    radius_for(n, (m as f64 * m as f64 / 10.0).powf(-1.0 / 3.0))
}

/// The activation radius a target names, and a label for it.
fn target_radius(target: &str, n: usize, m: usize) -> (usize, String) {
    let (lo, hi) = search_window(n, m);
    if target == "snr" {
        return (snr_radius(n, m), "snr".into());
    }
    if let Some(r) = target.strip_prefix('r') {
        return (r.parse().unwrap(), target.into());
    }
    if let Some(d) = target.strip_prefix('p') {
        let d: f64 = d.parse().unwrap();
        return (radius_for_address_noise_poisson(n, m, d, lo, hi, 0.9).0, target.into());
    }
    let d: f64 = target.parse().unwrap();
    (radius_for_address_noise(n, m, d, lo, hi).0, format!("cd{}", target))
}

fn pats(seed: u64, n: usize, count: usize) -> Vec<Vec<i8>> {
    let mut r = Rng::new(seed.wrapping_mul(0x9E37_79B9).wrapping_add(n as u64 + 0xAD1));
    (0..count).map(|_| random_pattern(n, &mut r)).collect()
}

fn par<T: Send, F: Fn(usize) -> T + Sync>(k: usize, f: F) -> Vec<T> {
    let out: Mutex<Vec<Option<T>>> = Mutex::new((0..k).map(|_| None).collect());
    let next = AtomicUsize::new(0);
    std::thread::scope(|s| {
        for _ in 0..threads().min(k.max(1)) {
            s.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::SeqCst);
                if i >= k {
                    break;
                }
                let v = f(i);
                out.lock().unwrap()[i] = Some(v);
            });
        }
    });
    out.into_inner().unwrap().into_iter().map(|x| x.unwrap()).collect()
}

fn parse_list(s: Option<&String>, d: &[usize]) -> Vec<usize> {
    match s {
        Some(s) => s.split(',').map(|x| x.parse::<f64>().unwrap() as usize).collect(),
        None => d.to_vec(),
    }
}

/// Geometric checkpoints 1, 2, 3, 5, 7 per decade up to M/4.
fn checkpoints(m: usize) -> Vec<usize> {
    let mut v = Vec::new();
    let mut dec = 1usize;
    while dec <= m {
        for f in [1usize, 2, 3, 5, 7] {
            let t = f * dec;
            if t <= m / 4 + 1 {
                v.push(t);
            }
        }
        dec *= 10;
    }
    v
}

fn cap_at(rows: &[(usize, f64)], thr: f64) -> usize {
    let mut best = 0;
    for &(t, s) in rows {
        if s >= thr {
            best = t;
        } else {
            break;
        }
    }
    best
}

#[derive(Clone, Copy, Debug)]
enum Reader {
    Addr,
    PullsFixed,
    PullsDensity(f64),
    PullsBlocks,
    PullsTopk,
}

impl Reader {
    fn name(&self) -> String {
        match self {
            Reader::Addr => "address".into(),
            Reader::PullsFixed => "pulls-0.4n".into(),
            Reader::PullsDensity(e) => format!("pulls-density-eps{}", e),
            Reader::PullsBlocks => "pulls-density-blocks".into(),
            Reader::PullsTopk => "pulls-topk-pM".into(),
        }
    }
}

struct Ckpt {
    t: usize,
    /// rates[arm][address-noise]
    rates: Vec<Vec<f64>>,
    theta: f64,
    kappa: f64,
    load: f64,
}

/// Write patterns up the checkpoints; at each, read q read-addresses per address-noise with every arm (the same read-address for
/// every arm). Stops after two checkpoints in a row where every arm and address-noise is under 50%.
#[allow(clippy::too_many_arguments)]
fn run_caps(n: usize, m: usize, r: usize, seed: u64, dmgs: &[f64], q: usize, arms: &[Reader], eps: f64) -> Vec<Ckpt> {
    let cps = checkpoints(m);
    let all = pats(seed, n, *cps.last().unwrap());
    let mut st = Store::new(n, m, r, seed);
    let topk = (ball(n, r) * m as f64).round().max(1.0) as usize;
    let mut out = Vec::new();
    let (mut written, mut fails) = (0, 0);
    for &t in &cps {
        st.write_many(&all[written..t]);
        written = t;
        let (theta, kappa, load) = density_threshold(&st, eps);
        let (theta_b, _, _) = density_threshold_blocks(&st, eps, 0.01);
        let res = par(dmgs.len() * q, |k| {
            let di = k / q;
            let mut rr = Rng::new(seed * 1_000_003 + (t as u64) * 7919 + k as u64);
            let pi = rr.below(t);
            let cue = add_address_noise(&all[pi], dmgs[di], &mut rr);
            let oks: Vec<bool> = arms
                .iter()
                .map(|a| {
                    let o = match a {
                        Reader::Addr => st.read_addresses(&cue, 20),
                        Reader::PullsFixed => st.read_pulls(&cue, 20),
                        Reader::PullsDensity(_) => st.read_pulls_at(&cue, 20, theta),
                        Reader::PullsBlocks => st.read_pulls_at(&cue, 20, theta_b),
                        Reader::PullsTopk => st.read_pulls_topk(&cue, 20, topk),
                    };
                    overlap(&o.z, &all[pi]) >= OK
                })
                .collect();
            (di, oks)
        });
        let mut rates = vec![vec![0.0; dmgs.len()]; arms.len()];
        for (di, oks) in &res {
            for (a, &ok) in oks.iter().enumerate() {
                rates[a][*di] += ok as u8 as f64 / q as f64;
            }
        }
        if st.overflow > 0 {
            println!("WARNING overflow {} at T {}", st.overflow, t);
        }
        let dead = rates.iter().flatten().all(|&x| x < 0.5);
        out.push(Ckpt { t, rates, theta, kappa, load });
        fails = if dead { fails + 1 } else { 0 };
        if fails >= 2 {
            break;
        }
    }
    out
}

/// Pool seeds on the shared checkpoint prefix and print table, P90/P50 and the predictions beside them.
#[allow(clippy::too_many_arguments)]
fn report(tag: &str, n: usize, m: usize, r: usize, dmgs: &[f64], arms: &[Reader], runs: &[Vec<Ckpt>], secs: f64) {
    let len = runs.iter().map(|x| x.len()).min().unwrap();
    let lz = Lazy::new(n, m, r);
    let g = goal(n);
    println!("## {} n {} M {} r {} p {:.5} pM {:.1}", tag, n, m, r, ball(n, r), ball(n, r) * m as f64);
    let pred: Vec<String> = dmgs
        .iter()
        .map(|&d| {
            let d0 = (d * n as f64).round() as usize;
            let la = Lazy::averaged(n, m, r);
            format!("{:.0}%: shared {:.2}, S-map {}, Poisson P90 {}, FULL P90 {} P50 {}", 100.0 * d, lz.shared(d0), lz.capacity(d0, g), lz.p_capacity(d0, g, 0.9), la.f_capacity(d, g, 0.9), la.f_capacity(d, g, 0.5))
        })
        .collect();
    println!("predicted | {}", pred.join(" | "));
    for (ai, a) in arms.iter().enumerate() {
        let mut rows: Vec<Vec<(usize, f64)>> = vec![Vec::new(); dmgs.len()];
        println!("arm {} | T | rates {:?} | theta kappa L", a.name(), dmgs);
        for i in 0..len {
            let t = runs[0][i].t;
            let rs: Vec<f64> = (0..dmgs.len()).map(|d| runs.iter().map(|x| x[i].rates[ai][d]).sum::<f64>() / runs.len() as f64).collect();
            for d in 0..dmgs.len() {
                rows[d].push((t, rs[d]));
            }
            let (th, ka, lo) = (runs[0][i].theta, runs[0][i].kappa, runs[0][i].load);
            let rstr: Vec<String> = rs.iter().map(|x| format!("{:.3}", x)).collect();
            println!("  {} | {} | {:.1} {:.2} {:.2}", t, rstr.join(" "), th, ka, lo);
            println!("ROW,ckpt,{},{},{},{},{},{},{},{:.1},{:.3},{:.3}", tag, a.name(), n, m, r, t, rstr.join(","), th, ka, lo);
        }
        let c90: Vec<usize> = rows.iter().map(|x| cap_at(x, 0.9)).collect();
        let c50: Vec<usize> = rows.iter().map(|x| cap_at(x, 0.5)).collect();
        let sm: Vec<usize> = dmgs.iter().map(|&d| lz.capacity((d * n as f64).round() as usize, g)).collect();
        let pp: Vec<usize> = dmgs.iter().map(|&d| lz.p_capacity((d * n as f64).round() as usize, g, 0.9)).collect();
        let la = Lazy::averaged(n, m, r);
        let pa: Vec<usize> = dmgs.iter().map(|&d| la.f_capacity(d, g, 0.9)).collect();
        let lm = Lazy::mirrored(n, m, r);
        let pm: Vec<usize> = dmgs.iter().map(|&d| lm.f_capacity(d, g, 0.9)).collect();
        println!("CAP {} {} n {} M {} r {} damages {:?} P90 {:?} P50 {:?} | S-map {:?} Poisson-P90 {:?} FULL-P90 {:?} MIRROR-P90 {:?} | {:.1}s", tag, a.name(), n, m, r, dmgs, c90, c50, sm, pp, pa, pm, secs);
        let j = |v: &[usize]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(";");
        println!("ROW,cap,{},{},{},{},{},{},{},{},{},{},{},{:.1}", tag, a.name(), n, m, r, j(&c90), j(&c50), j(&sm), j(&pp), j(&pa), j(&pm), secs);
    }
}

fn theory() {
    stamp("theory-start");
    let n = N;
    let g = goal(n);
    println!("\n# THEORY n {} (no memory built). S-map = Bricken-Pehlevan Eq. 25 iterated (goal overlap 0.95);", n);
    println!("# Poisson = first read conditioned on k ~ Poisson(M I(D)) shared locations. cd<D> = radius maximising the S-map capacity from D.");
    for &m in &[10_000usize, 30_000, 100_000, 300_000, 1_000_000] {
        let (lo, hi) = search_window(n, m);
        let mut radii: Vec<(String, usize)> = vec![("snr".into(), snr_radius(n, m))];
        for &d in &[0.1, 0.2, 0.3, 0.4] {
            radii.push((format!("cd{}", d), radius_for_address_noise(n, m, d, lo, hi).0));
        }
        for &d in &[0.3, 0.4] {
            radii.push((format!("p{}", d), radius_for_address_noise_poisson(n, m, d, lo, hi, 0.9).0));
        }
        let (rcd, dcd) = cd_optimal_radius(n, m, m / 100, lo, hi);
        println!("## M {} window [{}, {}] | BP d*_CD at T = M/100: r {} (critical distance {})", m, lo, hi, rcd, dcd);
        println!("policy | r | p | pM | shared at 10/20/30/40% | S-map cap 10/20/30/40% | Poisson P90 10/20/30/40% | Poisson P50 | FULL (Poisson, averaged noise, flip-damaged cue) P90 | P50 | MIRROR (FULL + the mirror field) P90 | P50");
        for (name, r) in &radii {
            let lz = Lazy::new(n, m, *r);
            let d0s: Vec<usize> = DMG.iter().map(|&d| (d * n as f64).round() as usize).collect();
            let sh: Vec<String> = d0s.iter().map(|&d| format!("{:.2}", lz.shared(d))).collect();
            let sm: Vec<usize> = d0s.iter().map(|&d| lz.capacity(d, g)).collect();
            let p9: Vec<usize> = d0s.iter().map(|&d| lz.p_capacity(d, g, 0.9)).collect();
            let p5: Vec<usize> = d0s.iter().map(|&d| lz.p_capacity(d, g, 0.5)).collect();
            let la = Lazy::averaged(n, m, *r);
            let a9: Vec<usize> = DMG.iter().map(|&d| la.f_capacity(d, g, 0.9)).collect();
            let a5: Vec<usize> = DMG.iter().map(|&d| la.f_capacity(d, g, 0.5)).collect();
            let lm = Lazy::mirrored(n, m, *r);
            let m9: Vec<usize> = DMG.iter().map(|&d| lm.f_capacity(d, g, 0.9)).collect();
            let m5: Vec<usize> = DMG.iter().map(|&d| lm.f_capacity(d, g, 0.5)).collect();
            println!("{} | {} | {:.5} | {:.1} | {} | {:?} | {:?} | {:?} | {:?} | {:?} | {:?} | {:?}", name, r, ball(n, *r), ball(n, *r) * m as f64, sh.join(" "), sm, p9, p5, a9, a5, m9, m5);
            let j = |v: &[usize]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(";");
            println!("ROW,theory,{},{},{},{},{:.6},{},{},{},{},{},{},{},{}", name, n, m, r, ball(n, *r), sh.join(";"), j(&sm), j(&p9), j(&p5), j(&a9), j(&a5), j(&m9), j(&m5));
        }
        println!("frontier by radius: S-map | Poisson P90 | FULL P90 | MIRROR P90, each at 10% 30% 40%:");
        for r in lo..=hi {
            let lz = Lazy::new(n, m, r);
            let la = Lazy::averaged(n, m, r);
            let s3: Vec<usize> = [26usize, 77, 102].iter().map(|&d| lz.capacity(d, g)).collect();
            let p3: Vec<usize> = [26usize, 77, 102].iter().map(|&d| lz.p_capacity(d, g, 0.9)).collect();
            let a3: Vec<usize> = [0.1, 0.3, 0.4].iter().map(|&d| la.f_capacity(d, g, 0.9)).collect();
            let lm = Lazy::mirrored(n, m, r);
            let m3: Vec<usize> = [0.1, 0.3, 0.4].iter().map(|&d| lm.f_capacity(d, g, 0.9)).collect();
            println!("  r {} | {:?} | {:?} | {:?} | {:?}", r, s3, p3, a3, m3);
            println!("ROW,tfrontier,{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}", n, m, r, s3[0], s3[1], s3[2], p3[0], p3[1], p3[2], a3[0], a3[1], a3[2], m3[0], m3[1], m3[2]);
        }
    }
    let (r, dc) = cd_optimal_radius(1000, 1_000_000, 10_000, 440, 456);
    println!("Bricken-Pehlevan canonical point n 1000 M 1e6 T 1e4: our d*_CD {} (critical distance {}); BP report 448 (188)", r, dc);
    stamp("theory-end");
}

fn capacity(target: &str, ms: &[usize], pulls: bool) {
    capacity_arms(target, ms, pulls, if pulls { vec![Reader::PullsFixed, Reader::PullsDensity(0.1), Reader::PullsTopk] } else { vec![Reader::Addr] })
}

fn capacity_arms(target: &str, ms: &[usize], pulls: bool, arms: Vec<Reader>) {
    stamp(&format!("{}-start-{}", if pulls { "pulls" } else { "capacity" }, target));
    let q = env_usize("SDMRADIUS_Q", if pulls { 30 } else { 40 });
    let ns = env_usize("SDMRADIUS_SEEDS", if pulls { 2 } else { 3 }) as u64;
    println!("\n# {} target {} (success = overlap >= 0.95 after <= 20 reads; {} queries per damage per seed x {} seeds)", if pulls { "PULLS" } else { "CAPACITY" }, target, q, ns);
    for &m in ms {
        let (r, tag) = target_radius(target, N, m);
        let t0 = Instant::now();
        let runs: Vec<Vec<Ckpt>> = (1..=ns).map(|s| run_caps(N, m, r, s, &DMG, q, &arms, 0.1)).collect();
        report(&tag, N, m, r, &DMG, &arms, &runs, t0.elapsed().as_secs_f64());
        stamp(&format!("M{}", m));
    }
}

fn frontier(m: usize, rs: &[usize]) {
    stamp("frontier-start");
    let q = env_usize("SDMRADIUS_Q", 30);
    let ns = env_usize("SDMRADIUS_SEEDS", 2) as u64;
    let dm = [0.1, 0.3, 0.4];
    println!("\n# FRONTIER n {} M {} ({} queries x {} seeds per cell), address read", N, m, q, ns);
    for &r in rs {
        let t0 = Instant::now();
        let runs: Vec<Vec<Ckpt>> = (1..=ns).map(|s| run_caps(N, m, r, 100 + s, &dm, q, &[Reader::Addr], 0.1)).collect();
        report(&format!("r{}", r), N, m, r, &dm, &[Reader::Addr], &runs, t0.elapsed().as_secs_f64());
        stamp(&format!("frontier-M{}-r{}", m, r));
    }
}

fn controls(target: &str, ms: &[usize]) {
    stamp(&format!("controls-start-{}", target));
    let q = 60;
    println!("\n# CONTROLS target {} ({} cues per cell). Loads: T_a = half the Poisson P90 at 30% (floor 2), T_b = M/100.", target, q);
    println!("M | r | load | T | arm | stored@10% | stored@30% | never@10% | never@30% | never@0% | entry-shuffle@10% | entry-shuffle@30%");
    for &m in ms {
        let (r, tag) = target_radius(target, N, m);
        let lz = Lazy::new(N, m, r);
        let ta = (lz.p_capacity(77, goal(N), 0.9) / 2).max(2);
        for (lname, t) in [("half-P90@30%", ta), ("M/100", (m / 100).max(2))] {
            let stored = pats(31337 + m as u64, N, t);
            let mut st = Store::new(N, m, r, 555 + m as u64);
            st.write_many(&stored);
            let never = pats(999_999 + m as u64, N, q);
            let mut e = st.clone();
            e.shuffle_entries(&mut Rng::new(3));
            let topk = (ball(N, r) * m as f64).round().max(1.0) as usize;
            // per store: (density threshold, block threshold)
            let ths = |s: &Store| (density_threshold(s, 0.1).0, density_threshold_blocks(s, 0.1, 0.01).0);
            let (th_s, th_e) = (ths(&st), ths(&e));
            for arm in [Reader::Addr, Reader::PullsDensity(0.1), Reader::PullsBlocks, Reader::PullsTopk] {
                let read = |s: &Store, th: (f64, f64), cue: &[i8], iters: usize| -> ReadOut {
                    match arm {
                        Reader::Addr => s.read_addresses(cue, iters),
                        Reader::PullsDensity(_) => s.read_pulls_at(cue, iters, th.0),
                        Reader::PullsBlocks => s.read_pulls_at(cue, iters, th.1),
                        _ => s.read_pulls_topk(cue, iters, topk),
                    }
                };
                let rec = |s: &Store, th: (f64, f64), dmg: f64, salt: u64| -> usize {
                    par(q, |k| {
                        let mut rr = Rng::new(salt * 1000 + k as u64);
                        let pi = rr.below(stored.len());
                        let cue = add_address_noise(&stored[pi], dmg, &mut rr);
                        overlap(&read(s, th, &cue, 20).z, &stored[pi]) >= OK
                    })
                    .into_iter()
                    .filter(|&x| x)
                    .count()
                };
                // (recalled, recalled on a silent first round, landed on some stored pattern instead)
                let nv = |dmg: f64| -> (usize, usize, usize) {
                    let res = par(q, |k| {
                        let mut rr = Rng::new(5000 + k as u64);
                        let cue = add_address_noise(&never[k], dmg, &mut rr);
                        let first = read(&st, th_s, &cue, 1);
                        let silent = match arm {
                            Reader::Addr => first.nonempty == 0,
                            _ => first.awake == 0,
                        };
                        let z = read(&st, th_s, &cue, 20).z;
                        let lands = stored.iter().any(|p| overlap(&z, p) >= OK);
                        (overlap(&z, &never[k]) >= OK, silent, lands)
                    });
                    (res.iter().filter(|x| x.0).count(), res.iter().filter(|x| x.0 && x.1).count(), res.iter().filter(|x| x.2).count())
                };
                let (n10, _, _) = nv(0.1);
                let (n30, _, l30) = nv(0.3);
                let (n0, s0, _) = nv(0.0);
                let row = [rec(&st, th_s, 0.1, 1), rec(&st, th_s, 0.3, 2), n10, n30, n0, rec(&e, th_e, 0.1, 3), rec(&e, th_e, 0.3, 4)];
                println!("{} | {} | {} | {} | {} | {} (never@0% silent {}; never@30% landed on a stored pattern {})", m, r, lname, t, arm.name(), row.iter().map(|x| format!("{}/{}", x, q)).collect::<Vec<_>>().join(" | "), s0, l30);
                println!("ROW,controls,{},{},{},{},{},{},{},{},{},{}", tag, m, r, lname, t, arm.name(), q, row.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(","), s0, l30);
            }
        }
        stamp(&format!("controls-M{}", m));
    }
}

fn hopfield(budgets_m: &[usize]) {
    stamp("hopfield-start");
    let n = N;
    let q = env_usize("SDMRADIUS_Q", 60);
    println!("\n# HOPFIELD at the counter budget M x n (nh(nh-1)/2 >= M n), zero temperature, async sweeps <= 30, expansion [p ; sign(R p)]; {} queries per damage", q);
    for &m in budgets_m {
        let nh = hop_units_for(m * n);
        let t0 = Instant::now();
        let mut h = Hop::new(n, nh, nh > n, 17 + m as u64);
        let maxt = (0.2 * nh as f64) as usize + 2;
        let all = pats(271828 + m as u64, n, maxt);
        let mut cps: Vec<usize> = checkpoints(maxt * 4).into_iter().filter(|&t| t <= maxt).collect();
        cps.dedup();
        let mut rows: [Vec<(usize, f64)>; 4] = Default::default();
        let (mut written, mut fails) = (0, 0);
        println!("## Hopfield units {} at SDM M {}", h.nh, m);
        for &t in &cps {
            while written < t {
                h.store(&all[written]);
                written += 1;
            }
            let res = par(4 * q, |k| {
                let di = k / q;
                let mut rr = Rng::new(m as u64 * 1_000_003 + t as u64 * 7919 + k as u64);
                let pi = rr.below(t);
                let cue = add_address_noise(&all[pi], DMG[di], &mut rr);
                (di, overlap(&h.recall(&cue, 30, &mut rr).0, &all[pi]) >= OK)
            });
            let mut a = [0.0; 4];
            for &(di, ok) in &res {
                a[di] += ok as u8 as f64 / q as f64;
            }
            for d in 0..4 {
                rows[d].push((t, a[d]));
            }
            println!("  {} | {:.3} {:.3} {:.3} {:.3}", t, a[0], a[1], a[2], a[3]);
            println!("ROW,hopfield,{},{},{},{},{:.3},{:.3},{:.3},{:.3}", n, m, h.nh, t, a[0], a[1], a[2], a[3]);
            fails = if a.iter().all(|&x| x < 0.5) { fails + 1 } else { 0 };
            if fails >= 2 {
                break;
            }
        }
        let c90: Vec<usize> = (0..4).map(|d| cap_at(&rows[d], 0.9)).collect();
        let c50: Vec<usize> = (0..4).map(|d| cap_at(&rows[d], 0.5)).collect();
        println!("HCAP n {} M {} units {} P90 {:?} P50 {:?} | {:.1}s", n, m, h.nh, c90, c50, t0.elapsed().as_secs_f64());
        println!("ROW,hcap,{},{},{},{:?},{:?}", n, m, h.nh, c90, c50);
        stamp(&format!("hopfield-M{}", m));
    }
}

/// Trace 40%-address-noise failures on small loads: classify each failure (captured by a rival, a mixture of
/// patterns, other) and tabulate read-addresses and failures by the target's shared-location count minus the best
/// rival's; print the RACE prediction beside.
fn race_trace() {
    stamp("race-start");
    println!("\n# RACE TRACE: 40% damage, 600 cues per case, address read");
    for &(m, r, t) in &[(100_000usize, 107usize, 2usize), (100_000, 107, 5), (1_000_000, 104, 5), (1_000_000, 104, 20), (100_000, 103, 5)] {
        let mut st = Store::new(N, m, r, 1);
        let mut rr = Rng::new(7);
        let ps: Vec<Vec<i8>> = (0..t).map(|_| random_pattern(N, &mut rr)).collect();
        st.write_many(&ps);
        let awake_sets: Vec<std::collections::HashSet<u32>> = ps.iter().map(|p| st.awake(p).into_iter().collect()).collect();
        let (mut fails, mut rival, mut mix, mut other) = (0, 0, 0, 0);
        let mut by: std::collections::BTreeMap<i64, (usize, usize)> = std::collections::BTreeMap::new();
        let q = 600;
        for k in 0..q {
            let pi = k % t;
            let cue = add_address_noise(&ps[pi], 0.4, &mut rr);
            let a = st.awake(&cue);
            let shared: Vec<i64> = awake_sets.iter().map(|s| a.iter().filter(|i| s.contains(i)).count() as i64).collect();
            let best = shared.iter().enumerate().filter(|(i, _)| *i != pi).map(|(_, x)| *x).max().unwrap_or(0);
            let margin = (shared[pi] - best).clamp(-4, 6);
            let o = st.read_addresses(&cue, 20);
            let e = by.entry(margin).or_insert((0, 0));
            e.0 += 1;
            if overlap(&o.z, &ps[pi]) < OK {
                fails += 1;
                e.1 += 1;
                let ovs: Vec<f64> = ps.iter().map(|p| overlap(&o.z, p)).collect();
                if ovs.iter().enumerate().any(|(i, &x)| i != pi && x >= OK) {
                    rival += 1;
                } else if ovs.iter().filter(|&&x| x > 0.3 && x < 0.7).count() >= 2 {
                    mix += 1;
                } else {
                    other += 1;
                }
            }
        }
        let race = 1.0 - Lazy::new(N, m, r).p_converge_race(0.4, t, goal(N), 4000, 5);
        let full = 1.0 - Lazy::averaged(N, m, r).p_converge_flips(0.4, t, goal(N));
        let tab: Vec<String> = by.iter().map(|(mg, (c, f))| format!("{}{}: {}/{}", if *mg == -4 { "<=" } else if *mg == 6 { ">=" } else { "" }, mg, f, c)).collect();
        println!("M {} r {} T {}: fail {}/{} = {:.3} (RACE {:.3}, FULL {:.3}); captured by a rival {}, mixture {}, other {}", m, r, t, fails, q, fails as f64 / q as f64, race, full, rival, mix, other);
        println!("  fails/cues by target-minus-best-rival shared locations: {}", tab.join(", "));
        println!("ROW,race,{},{},{},{},{},{:.4},{:.4},{},{},{}", m, r, t, fails, q, race, full, rival, mix, other);
    }
    stamp("race-end");
}

/// RACE P90 and P50 beside the measured capacities: radii snr, cd0.2, cd0.3, cd0.4 at every M and address-noise.
fn race_table() {
    stamp("racetable-start");
    println!("\n# RACE capacity (2,000 sampled reads per T) for the radii the capacity part measured");
    for &m in &[10_000usize, 30_000, 100_000, 300_000, 1_000_000] {
        for tg in ["snr", "0.2", "0.3", "0.4"] {
            let (r, tag) = target_radius(tg, N, m);
            let l = Lazy::new(N, m, r);
            let c9: Vec<usize> = DMG.iter().map(|&d| l.r_capacity(d, goal(N), 0.9, 2000)).collect();
            let c5: Vec<usize> = DMG.iter().map(|&d| l.r_capacity(d, goal(N), 0.5, 2000)).collect();
            println!("M {} {} r {}: RACE P90 {:?} P50 {:?}", m, tag, r, c9, c5);
            let j = |v: &[usize]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(";");
            println!("ROW,racecap,{},{},{},{},{}", m, tag, r, j(&c9), j(&c5));
        }
    }
    stamp("racetable-end");
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let part = a.get(1).map(|s| s.as_str()).unwrap_or("theory");
    let def = [10_000usize, 30_000, 100_000, 300_000, 1_000_000];
    match part {
        "theory" => theory(),
        "capacity" => capacity(a.get(2).map(|s| s.as_str()).unwrap_or("0.3"), &parse_list(a.get(3), &def), false),
        "pulls" => capacity(a.get(2).map(|s| s.as_str()).unwrap_or("snr"), &parse_list(a.get(3), &def[..4]), true),
        "pulls2" => capacity_arms(a.get(2).map(|s| s.as_str()).unwrap_or("snr"), &parse_list(a.get(3), &def[..4]), true, vec![Reader::PullsBlocks, Reader::PullsTopk]),
        "topk" => capacity_arms(a.get(2).map(|s| s.as_str()).unwrap_or("snr"), &parse_list(a.get(3), &def[4..]), true, vec![Reader::PullsTopk]),
        "frontier" => {
            let m = a.get(2).and_then(|s| s.parse::<f64>().ok()).unwrap_or(100_000.0) as usize;
            let (lo, hi) = search_window(N, m);
            let d: Vec<usize> = (lo..=hi).collect();
            frontier(m, &parse_list(a.get(3), &d))
        }
        "controls" => controls(a.get(2).map(|s| s.as_str()).unwrap_or("0.3"), &parse_list(a.get(3), &def)),
        "hopfield" => hopfield(&parse_list(a.get(2), &def)),
        "race" => race_trace(),
        "racetable" => race_table(),
        _ => eprintln!("unknown part {} (theory | capacity | pulls | frontier | controls | hopfield)", part),
    }
}
