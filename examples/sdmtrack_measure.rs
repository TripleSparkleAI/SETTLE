//! SDMTRACK measurements (`src/sdmtrack.rs` on `src/sdmscale.rs`'s store, SDMREFUSE's diagnostic reads).
//!
//! Run: `cargo run --release --example sdmtrack_measure <part> [args]`, part one of
//!   bench <M> <T>                    time one store top-k read and one TRACK-C / TRACK-R sample (no claim)
//!   fail <M> <spec>                  store top-k and block failure rates (Q read-addresses per cell) beside TRACK-C
//!                                    FRESH and PERSIST, TRACK-R and the address-read TRACK (baseline); spec lists
//!                                    address-noise:checkpoints groups, e.g. 0.3:3000,5000;0.4:2,10
//!   capgrid <M> <spec>               TRACK-C PERSIST on a capacity checkpoint grid (P90 / P50), no store
//!   geo <M> <spec>                   POST-HOC: TRACK-G (addresses placed relative to the state) on a failure grid
//!   census <M> <T,T,T>               never-stored top-k read-addresses: which patterns the woken rows hold, round by round,
//!                                    the self-vote, the woken-set overlap between rounds; beside TRACK-C and TRACK-R
//!   selfcal2 <read> <M> <T,T,T> <seed> rule S (union at alpha/4) beside the combined scores (product and
//!                                    calibrated min) and travel alone, on fresh seeds; read = topk | addr0.3 | addr0.4
//! Env: SDMTRACK_Q (store read-addresses per cell), SDMTRACK_QR (TRACK-R read-addresses), SDMTRACK_SAMPLES (TRACK-C samples),
//! SDMTRACK_ADDR_SAMPLES (address-read TRACK baseline samples, default SDMTRACK_SAMPLES), SDMTRACK_GEO_SAMPLES
//! (post-hoc TRACK-G samples in the fail part, default 0 = off),
//! SDMTRACK_QN (never-stored read-addresses), SDMTRACK_QS (stored read-addresses per address-noise), SDMTRACK_PROBES (probes per set),
//! SDMSCALE_THREADS. Everything is seeded; every part stamps UTC, load and power mode; raw rows go to stdout as
//! `ROW,...`.

use settle::rng::Rng;
use settle::sdm::radius_for;
use settle::sdmradius::{density_threshold_blocks, radius_for_address_noise, search_window};
use settle::sdmrefuse::{hd, nearest, pack_all, travel_threshold, Diag, Fast, Track};
use settle::sdmscale::*;
use settle::sdmtrack::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Instant;

const N: usize = 256;
const OK: f64 = 0.95;
const DMG: [f64; 4] = [0.1, 0.2, 0.3, 0.4];

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

fn cd_radius(n: usize, m: usize, d: f64) -> usize {
    let (lo, hi) = search_window(n, m);
    radius_for_address_noise(n, m, d, lo, hi).0
}

fn pats(seed: u64, count: usize) -> Vec<Vec<i8>> {
    let mut r = Rng::new(seed.wrapping_mul(0x9E37_79B9).wrapping_add(0x7AC0));
    (0..count).map(|_| random_pattern(N, &mut r)).collect()
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

fn parse_list(s: Option<&String>) -> Vec<usize> {
    s.map(|s| s.split(',').map(|x| x.parse::<f64>().unwrap() as usize).collect()).unwrap_or_default()
}

fn parse_spec(s: &str) -> Vec<(f64, Vec<usize>)> {
    s.split(';')
        .filter(|x| !x.is_empty())
        .map(|g| {
            let (d, c) = g.split_once(':').unwrap();
            (d.parse().unwrap(), c.split(',').map(|x| x.parse::<f64>().unwrap() as usize).collect())
        })
        .collect()
}

/// Random packed patterns for TRACK-R.
fn packed_random(t: usize, r: &mut Rng) -> Vec<u64> {
    (0..t * 4).map(|_| r.next_u64()).collect()
}

/// The block threshold on a TRACK-R graph, by the store's own formula (filled rows and mean row load measured).
fn member_theta(g: &Member, m: usize, r: usize, t: usize) -> f64 {
    let pm = ball(N, r) * m as f64;
    let filled = g.rows().max(1) as f64;
    let l = g.mean_load(20_000).max(1.0);
    let tail = (0.1 * pm / filled).min(0.5);
    let kr = if tail <= 0.0 { 1.0 } else { phi_inv(1.0 - tail).max(1.0) };
    let kp = phi_inv(1.0 - (0.01 / t.saturating_sub(1).max(1) as f64).min(0.5)).max(1.0);
    kr.max(kp) * (N as f64 * l).sqrt()
}

/// TRACK-R: fraction of `q` noisy read-addresses (targets drawn at random) recalled on a random membership graph.
fn trackr_rate(g: &Member, wake: Wake, dmg: f64, q: usize, seed: u64) -> f64 {
    let t = g.t;
    let ok = par(q, |k| {
        let mut rr = Rng::new(seed * 1_000_003 + k as u64);
        let pi = rr.below(t);
        let mut z = g.pats[pi * 4..(pi + 1) * 4].to_vec();
        for j in 0..N {
            if rr.unit() < dmg {
                z[j / 64] ^= 1 << (j % 64);
            }
        }
        let (f, _, _, _) = g.read(&z, wake, 20);
        let d = hd(&f, &g.pats[pi * 4..(pi + 1) * 4]);
        1.0 - 2.0 * d as f64 / N as f64 >= OK
    });
    ok.iter().filter(|&&x| x).count() as f64 / q.max(1) as f64
}

fn trackc_rate(c: &Content, wake: Wake, dmg: f64, t: usize, smp: usize, persist: bool, seed: u64) -> f64 {
    let per = par(8, |i| c.p_converge(wake, dmg, t, smp / 8, persist, seed + i as u64));
    per.iter().sum::<f64>() / 8.0
}

fn bench(m: usize, t: usize) {
    stamp("bench-start");
    let r = snr_radius(N, m);
    let c = Content::new(N, m, r);
    println!("bench M {} T {} r {} p {:.3e} pM {:.1} k {} lam pT {:.3} block theta {:.1}", m, t, r, c.p, c.pm, c.k(), c.p * t as f64, block_theta(N, m, r, t, 0.1, 0.01));
    let ps = pats(1, t);
    let mut st = Store::new(N, m, r, 1);
    let t0 = Instant::now();
    st.write_many(&ps);
    let tw = t0.elapsed().as_secs_f64();
    let f = Fast::new(&st);
    let mut rr = Rng::new(3);
    let cue = add_address_noise(&ps[0], 0.3, &mut rr);
    let t1 = Instant::now();
    let d = f.topk(&cue, 20, c.k());
    println!("  store write {:.1}s, top-k read {:.3}s ({} rounds), filled {}", tw, t1.elapsed().as_secs_f64(), d.rounds, f.rows.len());
    let t2 = Instant::now();
    let s = c.sample(Wake::Topk(c.k()), t, 0.3, false, 20, true, &mut rr);
    println!("  TRACK-C sample {:.3}s ({} rounds, overlap {:.3})", t2.elapsed().as_secs_f64(), s.rounds, s.overlap);
    let t3 = Instant::now();
    let g = Member::random(N, m, r, packed_random(t, &mut rr), &mut rr);
    let tb = t3.elapsed().as_secs_f64();
    let t4 = Instant::now();
    let (_, rounds, _, _) = g.read(&g.pats[..4], Wake::Topk(c.k()), 20);
    println!("  TRACK-R build {:.2}s rows {}, read {:.3}s ({} rounds)", tb, g.rows(), t4.elapsed().as_secs_f64(), rounds);
    stamp("bench-end");
}

fn fail(m: usize, spec: &str) {
    stamp(&format!("fail-start-M{}", m));
    let q = env_usize("SDMTRACK_Q", 400);
    let qr = env_usize("SDMTRACK_QR", 400);
    let smp = env_usize("SDMTRACK_SAMPLES", 2000);
    let groups = parse_spec(spec);
    let r = snr_radius(N, m);
    let c = Content::new(N, m, r);
    let k = c.k();
    let addr = Track::new(N, m, r);
    let gsmp = env_usize("SDMTRACK_GEO_SAMPLES", 0);
    let cg = if gsmp > 0 { Some(Content::with_geometry(N, m, r)) } else { None };
    let mut cps: Vec<usize> = groups.iter().flat_map(|g| g.1.clone()).collect();
    cps.sort();
    cps.dedup();
    println!("\n# FAIL top-k and block, n {} M {} r {} k {}; store {} cues per cell, TRACK-R {} cues, TRACK-C {} samples; spec {}", N, m, r, k, q, qr, smp, spec);
    let all = pats(424_242 + m as u64, *cps.last().unwrap());
    let mut st = Store::new(N, m, r, 31_337 + m as u64);
    let mut written = 0;
    for &t in &cps {
        let t0 = Instant::now();
        st.write_many(&all[written..t]);
        written = t;
        let f = Fast::new(&st);
        let (theta, _, _) = density_threshold_blocks(&st, 0.1, 0.01);
        let theta_c = block_theta(N, m, r, t, 0.1, 0.01);
        let mut rg = Rng::new(777 + t as u64 + m as u64);
        let g = Member::random(N, m, r, packed_random(t, &mut rg), &mut rg);
        let theta_r = member_theta(&g, m, r, t);
        println!("## T {}: store theta {:.1} (TRACK-C {:.1}, TRACK-R {:.1}), store filled {} (TRACK-R {})", t, theta, theta_c, theta_r, f.rows.len(), g.rows());
        for (gi, (dmg, list)) in groups.iter().enumerate() {
            if !list.contains(&t) {
                continue;
            }
            let res = par(q, |kq| {
                let mut rr = Rng::new(m as u64 * 104_729 + t as u64 * 7_919 + (gi * 1_000_000 + kq) as u64);
                let pi = rr.below(t);
                let cue = add_address_noise(&all[pi], *dmg, &mut rr);
                let a = overlap(&f.topk(&cue, 20, k).z, &all[pi]) >= OK;
                let b = overlap(&f.thresh(&cue, 20, theta).z, &all[pi]) >= OK;
                (a, b)
            });
            let ma = 1.0 - res.iter().filter(|x| x.0).count() as f64 / q as f64;
            let mb = 1.0 - res.iter().filter(|x| x.1).count() as f64 / q as f64;
            let se = |p: f64| (p * (1.0 - p) / q as f64).sqrt();
            let seed = 5_000 + gi as u64 * 100 + t as u64;
            let cf_a = 1.0 - trackc_rate(&c, Wake::Topk(k), *dmg, t, smp, false, seed);
            let cp_a = 1.0 - trackc_rate(&c, Wake::Topk(k), *dmg, t, smp, true, seed);
            let cf_b = 1.0 - trackc_rate(&c, Wake::Block(theta_c), *dmg, t, smp, false, seed);
            let cp_b = 1.0 - trackc_rate(&c, Wake::Block(theta_c), *dmg, t, smp, true, seed);
            let tr_a = 1.0 - trackr_rate(&g, Wake::Topk(k), *dmg, qr, seed);
            let tr_b = 1.0 - trackr_rate(&g, Wake::Block(theta_r), *dmg, qr, seed + 1);
            let sa = env_usize("SDMTRACK_ADDR_SAMPLES", smp);
            let ad = 1.0 - par(8, |i| addr.p_converge(*dmg, t, sa / 8, true, seed + i as u64)).iter().sum::<f64>() / 8.0;
            println!(
                "  {:.0}% T {}: top-k measured {:.3} +- {:.3} | TRACK-C fresh {:.3} persist {:.3} | TRACK-R {:.3} | address TRACK {:.3}",
                100.0 * dmg, t, ma, se(ma), cf_a, cp_a, tr_a, ad
            );
            println!("  {:.0}% T {}: block measured {:.3} +- {:.3} | TRACK-C fresh {:.3} persist {:.3} | TRACK-R {:.3}", 100.0 * dmg, t, mb, se(mb), cf_b, cp_b, tr_b);
            let (ga, gb) = match &cg {
                Some(cg) => {
                    let kappa = cg.geo.as_ref().unwrap().kappa;
                    let a = 1.0 - trackc_rate(cg, Wake::Topk(k), *dmg, t, gsmp, true, seed + 7);
                    let b = 1.0 - trackc_rate(cg, Wake::Block(block_theta_geo(N, m, r, t, 0.1, 0.01, kappa)), *dmg, t, gsmp, true, seed + 8);
                    println!("  {:.0}% T {}: TRACK-G (post-hoc design) top-k {:.3} block {:.3}", 100.0 * dmg, t, a, b);
                    (format!("{:.4}", a), format!("{:.4}", b))
                }
                None => ("NA".to_string(), "NA".to_string()),
            };
            println!("ROW,fail,{},{},{},topk,{},{},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{},{},{},{},{}", m, r, dmg, t, q, ma, se(ma), cf_a, cp_a, tr_a, ad, smp, qr, sa, ga, gsmp);
            println!("ROW,fail,{},{},{},block,{},{},{:.4},{:.4},{:.4},{:.4},{:.4},NA,{},{},{},{},{}", m, r, dmg, t, q, mb, se(mb), cf_b, cp_b, tr_b, smp, qr, sa, gb, gsmp);
        }
        println!("  T {} done ({:.1}s)", t, t0.elapsed().as_secs_f64());
        stamp(&format!("fail-M{}-T{}", m, t));
    }
}

/// POST-HOC (built after part A and B showed TRACK-C and TRACK-R over-predicting failure at heavy load):
/// TRACK-G, the content predictor with the rows' addresses placed relative to the state, on a failure grid.
fn geo(m: usize, spec: &str) {
    stamp(&format!("geo-start-M{}", m));
    let smp = env_usize("SDMTRACK_SAMPLES", 800);
    let r = snr_radius(N, m);
    let c = Content::with_geometry(N, m, r);
    let k = c.k();
    let kappa = c.geo.as_ref().unwrap().kappa;
    println!("\n# GEO (post-hoc) TRACK-G persist, n {} M {} r {} k {} kappa {:.4} shells {}; {} samples per cell; spec {}", N, m, r, k, kappa, c.geo.as_ref().unwrap().shells.len(), smp, spec);
    for (gi, (dmg, list)) in parse_spec(spec).iter().enumerate() {
        for &t in list {
            let t0 = Instant::now();
            let seed = 70_000 + gi as u64 * 100 + t as u64;
            let th = block_theta_geo(N, m, r, t, 0.1, 0.01, kappa);
            let a = 1.0 - trackc_rate(&c, Wake::Topk(k), *dmg, t, smp, true, seed);
            let b = 1.0 - trackc_rate(&c, Wake::Block(th), *dmg, t, smp, true, seed + 1);
            println!("  {:.0}% T {}: TRACK-G failure top-k {:.3} block {:.3} (theta {:.1}) ({:.1}s)", 100.0 * dmg, t, a, b, th, t0.elapsed().as_secs_f64());
            println!("ROW,geo,{},{},{},topk,{},{:.4},{}", m, r, dmg, t, a, smp);
            println!("ROW,geo,{},{},{},block,{},{:.4},{}", m, r, dmg, t, b, smp);
        }
    }
    stamp(&format!("geo-end-M{}", m));
}

fn cap_from(rows: &[(usize, f64)], thr: f64) -> String {
    let mut best: Option<usize> = None;
    for &(t, s) in rows {
        if s >= thr {
            best = Some(t);
        } else {
            return match best {
                Some(b) => b.to_string(),
                None => format!("<{}", t),
            };
        }
    }
    match best {
        Some(b) => format!("{}+", b),
        None => "none".into(),
    }
}

fn capgrid(m: usize, spec: &str) {
    stamp(&format!("capgrid-start-M{}", m));
    let smp = env_usize("SDMTRACK_SAMPLES", 2000);
    let r = snr_radius(N, m);
    let c = Content::new(N, m, r);
    println!("\n# CAPGRID TRACK-C (persist) top-k and block, n {} M {} r {} k {}; {} samples per point; spec {}", N, m, r, c.k(), smp, spec);
    for (gi, (dmg, list)) in parse_spec(spec).iter().enumerate() {
        let mut tk = Vec::new();
        let mut bl = Vec::new();
        for &t in list {
            let seed = 9_000 + gi as u64 * 100 + t as u64;
            tk.push((t, trackc_rate(&c, Wake::Topk(c.k()), *dmg, t, smp, true, seed)));
            bl.push((t, trackc_rate(&c, Wake::Block(block_theta(N, m, r, t, 0.1, 0.01)), *dmg, t, smp, true, seed)));
        }
        let fmt = |v: &[(usize, f64)]| v.iter().map(|(t, s)| format!("{}:{:.3}", t, s)).collect::<Vec<_>>().join(" ");
        println!("CAPC M {} damage {} top-k P90 {} P50 {} | {}", m, dmg, cap_from(&tk, 0.9), cap_from(&tk, 0.5), fmt(&tk));
        println!("CAPC M {} damage {} block P90 {} P50 {} | {}", m, dmg, cap_from(&bl, 0.9), cap_from(&bl, 0.5), fmt(&bl));
        println!("ROW,capc,{},{},{},topk,{},{}", m, r, dmg, cap_from(&tk, 0.9), cap_from(&tk, 0.5));
        println!("ROW,capc,{},{},{},block,{},{}", m, r, dmg, cap_from(&bl, 0.9), cap_from(&bl, 0.5));
    }
    stamp(&format!("capgrid-end-M{}", m));
}

fn median(v: &mut [f64]) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

fn jaccard(a: &[u32], b: &[u32]) -> f64 {
    let sa: std::collections::HashSet<u32> = a.iter().copied().collect();
    let inter = b.iter().filter(|x| sa.contains(x)).count() as f64;
    let uni = (a.len() + b.len()) as f64 - inter;
    if uni == 0.0 {
        1.0
    } else {
        inter / uni
    }
}

fn census_part(m: usize, ts: &[usize]) {
    stamp(&format!("census-start-M{}", m));
    let qn = env_usize("SDMTRACK_QN", 200);
    let smp = env_usize("SDMTRACK_SAMPLES", 400);
    let r = snr_radius(N, m);
    let c = Content::new(N, m, r);
    let k = c.k();
    println!("\n# CENSUS never-stored top-k cues, n {} M {} r {} k {}; {} cues per load; TRACK-C and TRACK-R {} each", N, m, r, k, qn, smp);
    let tmax = *ts.iter().max().unwrap();
    let all = pats(606_060 + m as u64, tmax);
    let never = pats(707_070 + m as u64, qn);
    let mut st = Store::new(N, m, r, 80_808 + m as u64);
    let mut written = 0;
    let mut tsorted = ts.to_vec();
    tsorted.sort();
    for &t in &tsorted {
        let t0 = Instant::now();
        st.write_many(&all[written..t]);
        written = t;
        let pk = pack_all(&all[..t]);
        let rows: Vec<u32> = (0..m).filter(|&i| st.filled[i]).map(|i| i as u32).collect();
        // per read-address: (travel, rounds, nearest distance, per-round records)
        let recs = par(qn, |kq| {
            let cue = &never[kq];
            let (z, rounds) = trace_topk(&st, &rows, cue, 20, k);
            let zp = pack(&z);
            let travel = hd(&pack(cue), &zp);
            let (_, nd) = nearest(&pk, &zp);
            let mut per = Vec::new();
            let mut prev: Option<&Vec<u32>> = None;
            for rd in &rounds {
                let cs = census(&st, &pk, &rd.woken, &rd.state);
                let jac = prev.map(|p| jaccard(p, &rd.woken)).unwrap_or(f64::NAN);
                per.push((cs, rd.flips, rd.a, rd.sigma, flips_estimate(N, rd.a, rd.sigma), jac));
                prev = Some(&rd.woken);
            }
            let jfl = if rounds.len() >= 2 { jaccard(&rounds[0].woken, &rounds.last().unwrap().woken) } else { 1.0 };
            (travel, rounds.len(), nd, per, jfl)
        });
        let landed = recs.iter().filter(|x| 1.0 - 2.0 * x.2 as f64 / N as f64 >= OK).count();
        let mut tr: Vec<f64> = recs.iter().map(|x| x.0 as f64).collect();
        let mut rd: Vec<f64> = recs.iter().map(|x| x.1 as f64).collect();
        type Point = (Census, usize, f64, f64, f64, f64);
        let pick = |f: &dyn Fn(&Point) -> f64, last: bool| -> f64 {
            let mut v: Vec<f64> = recs.iter().map(|x| f(if last { x.3.last().unwrap() } else { &x.3[0] })).collect();
            median(&mut v)
        };
        // woken-set overlap between the first and the last round, over read-addresses that read more than once
        let jac_last: Vec<f64> = recs.iter().filter(|x| x.3.len() >= 2).map(|x| x.4).collect();
        println!(
            "## T {} ({:.1}s): never-stored cues landing {}/{}; median travel {} rounds {}",
            t,
            t0.elapsed().as_secs_f64(),
            landed,
            qn,
            median(&mut tr),
            median(&mut rd)
        );
        println!(
            "  round 1 (median): distinct patterns {} of K {} pairs, effective number {:.1}, top share {:.3}, top pattern {} bits from the cue, empty rows {}",
            pick(&|x| x.0.distinct as f64, false),
            pick(&|x| x.0.total as f64, false),
            pick(&|x| x.0.neff, false),
            pick(&|x| x.0.share, false),
            pick(&|x| x.0.top_dist as f64, false),
            pick(&|x| x.0.empty as f64, false)
        );
        println!(
            "  last round (median): distinct {} of K {}, effective number {:.1}, top share {:.3}; woken-set overlap first vs last round {:.3} ({} cues read more than once)",
            pick(&|x| x.0.distinct as f64, true),
            pick(&|x| x.0.total as f64, true),
            pick(&|x| x.0.neff, true),
            pick(&|x| x.0.share, true),
            median(&mut jac_last.clone()),
            jac_last.len()
        );
        println!(
            "  round 1 self-vote (median): A {:.1} sigma {:.1} A/sigma {:.2}; flips measured {} Gaussian estimate {:.1}",
            pick(&|x| x.2, false),
            pick(&|x| x.3, false),
            pick(&|x| x.2 / x.3.max(1e-9), false),
            pick(&|x| x.1 as f64, false),
            pick(&|x| x.4, false)
        );
        for (kq, x) in recs.iter().enumerate() {
            for (ri, p) in x.3.iter().enumerate() {
                println!(
                    "ROW,census,{},{},{},{},{},{},{},{},{},{},{},{},{:.2},{:.4},{},{:.2},{:.2},{:.2},{:.4}",
                    m, r, t, kq, x.0, x.1, x.2, ri + 1, p.0.rows, p.0.empty, p.0.distinct, p.0.total, p.0.neff, p.0.share, p.0.top_dist, p.2, p.3, p.4, p.5
                );
                let _ = p.1;
            }
        }
        // the predictors on never-stored read-addresses
        let cs = |persist: bool| {
            par(smp, |i| {
                let mut rr = Rng::new(12_345 + i as u64 * 7 + t as u64 * 1_000_003 + persist as u64);
                c.sample(Wake::Topk(k), t, 0.0, true, 20, persist, &mut rr)
            })
        };
        for (name, v) in [("TRACK-C fresh", cs(false)), ("TRACK-C persist", cs(true))] {
            let mut tv: Vec<f64> = v.iter().map(|s| s.travel as f64).collect();
            let mut nf: Vec<f64> = v.iter().map(|s| s.neff1).collect();
            let mut sh: Vec<f64> = v.iter().map(|s| s.share1).collect();
            let mut di: Vec<f64> = v.iter().map(|s| s.distinct1 as f64).collect();
            let land = v.iter().filter(|s| 1.0 - 2.0 * s.nearest as f64 / N as f64 >= OK).count();
            println!(
                "  {}: landing {}/{}; median travel {}; round 1 distinct {} effective number {:.1} top share {:.3}",
                name,
                land,
                v.len(),
                median(&mut tv),
                median(&mut di),
                median(&mut nf),
                median(&mut sh)
            );
            println!("ROW,censuspred,{},{},{},{},{},{:.1},{:.1},{:.2},{:.4}", m, r, t, name, land, median(&mut tv), median(&mut di), median(&mut nf), median(&mut sh));
        }
        let mut rg = Rng::new(4_444 + t as u64);
        let g = Member::random(N, m, r, packed_random(t, &mut rg), &mut rg);
        let gr = par(smp.min(qn), |i| {
            let mut rr = Rng::new(98_765 + i as u64);
            let cue: Vec<u64> = (0..4).map(|_| rr.next_u64()).collect();
            let (z, _, neff, share) = g.read(&cue, Wake::Topk(k), 20);
            let nd = (0..g.t).map(|mu| hd(&g.pats[mu * 4..(mu + 1) * 4], &z)).min().unwrap();
            (hd(&cue, &z) as f64, neff, share, nd)
        });
        let mut tv: Vec<f64> = gr.iter().map(|x| x.0).collect();
        let mut nf: Vec<f64> = gr.iter().map(|x| x.1).collect();
        let mut sh: Vec<f64> = gr.iter().map(|x| x.2).collect();
        let land = gr.iter().filter(|x| 1.0 - 2.0 * x.3 as f64 / N as f64 >= OK).count();
        println!("  TRACK-R: landing {}/{}; median travel {}; round 1 effective number {:.1} top share {:.3}", land, gr.len(), median(&mut tv), median(&mut nf), median(&mut sh));
        println!("ROW,censuspred,{},{},{},TRACK-R,{},{:.1},NA,{:.2},{:.4}", m, r, t, land, median(&mut tv), median(&mut nf), median(&mut sh));
        stamp(&format!("census-M{}-T{}", m, t));
    }
}

/// Signals oriented SMALLER = more confident: travel, -cos1, -dot1, rounds.
fn sig4(cue: &[i8], d: &Diag) -> Vec<f64> {
    vec![hd(&pack(cue), &pack(&d.z)) as f64, -d.cos1, -d.dot1, d.rounds as f64]
}

fn selfcal2(read: &str, m: usize, ts: &[usize], off: u64) {
    stamp(&format!("selfcal2-start-{}-M{}-seed{}", read, m, off));
    let qs = env_usize("SDMTRACK_QS", 200);
    let qn = env_usize("SDMTRACK_QN", 500);
    let np = env_usize("SDMTRACK_PROBES", 1500);
    let alpha = 0.01;
    let r = match read {
        "topk" => snr_radius(N, m),
        "addr0.3" => cd_radius(N, m, 0.3),
        "addr0.4" => cd_radius(N, m, 0.4),
        _ => panic!("read is topk | addr0.3 | addr0.4"),
    };
    let kk = (ball(N, r) * m as f64).round().max(1.0) as usize;
    println!(
        "\n# SELFCAL2 read {} n {} M {} r {} seed offset {}; {} stored cues per damage, {} never-stored test cues, probe sets P1 {} (union rule S and tail fractions) and P2 {} (combined-score cut), alpha {}",
        read, N, m, r, off, qs, qn, np, np, alpha
    );
    let base = 20_000_000 + 1_000_003 * off + m as u64;
    let tmax = *ts.iter().max().unwrap();
    let all = pats(base + 1, tmax);
    let never = pats(base + 2, qn);
    let p1 = pats(base + 3, np);
    let p2 = pats(base + 4, np);
    let mut st = Store::new(N, m, r, 5_151 + m as u64 + 13 * off);
    let mut written = 0;
    let mut tsorted = ts.to_vec();
    tsorted.sort();
    for &t in &tsorted {
        let t0 = Instant::now();
        st.write_many(&all[written..t]);
        written = t;
        let f = Fast::new(&st);
        let run = |cue: &[i8]| -> Vec<f64> {
            let d = if read == "topk" { f.topk(cue, 20, kk) } else { f.address(cue, 20) };
            sig4(cue, &d)
        };
        let s1 = par(np, |i| run(&p1[i]));
        let s2 = par(np, |i| run(&p2[i]));
        // union rule S (SDMREFUSE): per signal the P1 value at rank floor(alpha/4 P); accept iff some signal below
        let rank = ((alpha / 4.0) * np as f64).floor() as usize;
        let taus: Vec<f64> = (0..4)
            .map(|si| {
                let mut v: Vec<f64> = s1.iter().map(|x| x[si]).collect();
                v.sort_by(|a, b| a.partial_cmp(b).unwrap());
                v[rank.min(v.len() - 1)]
            })
            .collect();
        let union = |x: &[f64]| x.iter().zip(&taus).any(|(a, b)| a < b);
        // travel alone at alpha on P1
        let mut tv: Vec<f64> = s1.iter().map(|x| x[0]).collect();
        tv.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let tau_t = tv[((alpha * np as f64).floor() as usize).min(np - 1)];
        // combined scores: tail fractions from P1, cut calibrated on P2
        let tc = TailCal::new(&s1);
        let cut_p = calibrate(&s2.iter().map(|x| tc.product(x)).collect::<Vec<_>>(), alpha);
        let cut_m = calibrate(&s2.iter().map(|x| tc.min(x)).collect::<Vec<_>>(), alpha);
        let product = |x: &[f64]| tc.product(x) < cut_p;
        let cmin = |x: &[f64]| tc.min(x) < cut_m;
        let h = travel_threshold(N, t, 0.01);
        let ruler = |x: &[f64]| x[0] <= h as f64;
        type Rule<'a> = (&'a str, &'a dyn Fn(&[f64]) -> bool);
        let rules: [Rule; 5] = [("unionS", &union), ("product", &product), ("calmin", &cmin), ("travel", &|x: &[f64]| x[0] < tau_t), ("ruleR", &ruler)];
        let srec = par(4 * qs, |k| {
            let di = k / qs;
            let mut rr = Rng::new(base * 3 + t as u64 * 104_729 + k as u64);
            let pi = rr.below(t);
            let cue = add_address_noise(&all[pi], DMG[di], &mut rr);
            let d = if read == "topk" { f.topk(&cue, 20, kk) } else { f.address(&cue, 20) };
            (di, overlap(&d.z, &all[pi]) >= OK, sig4(&cue, &d))
        });
        let nrec = par(qn, |i| run(&never[i]));
        let p2acc: Vec<String> = rules.iter().map(|(nm, rl)| format!("{} {:.4}", nm, s2.iter().filter(|x| rl(x)).count() as f64 / np as f64)).collect();
        println!(
            "## T {} ({:.1}s): union taus {:?}, travel alone tau {}, product cut {:.3} (log), min cut {:.5}, rule R h {}; P2 probes accepted: {}",
            t,
            t0.elapsed().as_secs_f64(),
            taus,
            tau_t,
            cut_p,
            cut_m,
            h,
            p2acc.join(", ")
        );
        let mut cells = Vec::new();
        for (nm, rl) in rules.iter() {
            let refuse = nrec.iter().filter(|x| !rl(x)).count() as f64 / qn as f64;
            let recs: Vec<f64> = (0..4).map(|di| srec.iter().filter(|x| x.0 == di && x.1 && rl(&x.2)).count() as f64 / qs as f64).collect();
            println!("  {:<8} refusal {:.3} | recall 10/20/30/40% {:.3} {:.3} {:.3} {:.3}", nm, refuse, recs[0], recs[1], recs[2], recs[3]);
            cells.push(format!("{}:{:.4}:{:.4};{:.4};{:.4};{:.4}", nm, refuse, recs[0], recs[1], recs[2], recs[3]));
        }
        let none: Vec<f64> = (0..4).map(|di| srec.iter().filter(|x| x.0 == di && x.1).count() as f64 / qs as f64).collect();
        println!("  no refusal recall {:.3} {:.3} {:.3} {:.3}", none[0], none[1], none[2], none[3]);
        println!("ROW,selfcal2,{},{},{},{},{},{},{:.4};{:.4};{:.4};{:.4}", read, m, r, t, off, cells.join(","), none[0], none[1], none[2], none[3]);
        stamp(&format!("selfcal2-{}-M{}-T{}", read, m, t));
    }
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let part = a.get(1).map(|s| s.as_str()).unwrap_or("bench");
    let num = |i: usize, d: f64| a.get(i).and_then(|s| s.parse::<f64>().ok()).unwrap_or(d);
    match part {
        "bench" => bench(num(2, 1e6) as usize, num(3, 10_000.0) as usize),
        "fail" => fail(num(2, 1e5) as usize, a.get(3).map(|s| s.as_str()).unwrap_or("0.3:3000")),
        "geo" => geo(num(2, 1e5) as usize, a.get(3).map(|s| s.as_str()).unwrap_or("0.3:3000")),
        "capgrid" => capgrid(num(2, 1e5) as usize, a.get(3).map(|s| s.as_str()).unwrap_or("0.3:3000")),
        "census" => census_part(num(2, 1e6) as usize, &parse_list(a.get(3))),
        "selfcal2" => selfcal2(a.get(2).map(|s| s.as_str()).unwrap_or("topk"), num(3, 1e5) as usize, &parse_list(a.get(4)), num(5, 3.0) as u64),
        _ => eprintln!("unknown part {} (bench | fail | capgrid | census | selfcal2)", part),
    }
}
