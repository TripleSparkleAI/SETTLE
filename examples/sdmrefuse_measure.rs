//! SDMREFUSE measurements (`src/sdmrefuse.rs` on `src/sdmscale.rs`'s store).
//!
//! Run: `cargo run --release --example sdmrefuse_measure <part> [args]`, part one of
//!   bench <M> <T>                   time one top-k read (no claim; sizes the grid)
//!   refuse <read> <M> <T,T,T>       the refusal trade-off: stored read-addresses at 10/20/30/40% and never-stored read-addresses,
//!                                   every signal swept; read = topk (SNR activation radius) | addr0.3 | addr0.4 (the
//!                                   critical-distance activation radius for that address-noise)
//!   capacity <M> <spec>             top-k and block-threshold capacity; spec lists address-noise:checkpoints groups,
//!                                   e.g. 0.2:10000,20000;0.3:3000,5000 (checkpoints ascending)
//!   selfcal <read> <M> <T,T,T> <seed> the follow-up rule S (self-calibrated on the memory's own random probes),
//!                                   on fresh seeds, beside rule R
//!   track <M,M,...>                 predictor TRACK (FRESH and PERSIST) P90/P50 on the checkpoint grid for the
//!                                   radii SDMRADIUS measured, beside FULL and RACE
//!   fail <M> <r> <dmg> <T,T,...>    measured address-read failure rate (Q read-addresses) beside FULL, RACE, TRACK
//! Env: SDMREFUSE_QS (stored read-addresses per address-noise), SDMREFUSE_QN (never-stored read-addresses), SDMREFUSE_Q (capacity read-addresses
//! per address-noise per seed), SDMREFUSE_SEEDS, SDMREFUSE_SAMPLES (TRACK/RACE samples), SDMSCALE_THREADS.
//! Everything is seeded; every part stamps UTC, load and power mode. Raw rows go to stdout as `ROW,...`.

use settle::rng::Rng;
use settle::sdm::radius_for;
use settle::sdmradius::{density_threshold_blocks, goal, radius_for_address_noise, search_window, Lazy};
use settle::sdmrefuse::*;
use settle::sdmscale::*;
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
    let mut r = Rng::new(seed.wrapping_mul(0x9E37_79B9).wrapping_add(0x5EF0));
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

fn bench(m: usize, t: usize) {
    stamp("bench-start");
    let r = snr_radius(N, m);
    let ps = pats(1, t);
    let mut st = Store::new(N, m, r, 1);
    let t0 = Instant::now();
    st.write_many(&ps);
    let tw = t0.elapsed().as_secs_f64();
    let f = Fast::new(&st);
    let k = (ball(N, r) * m as f64).round() as usize;
    let mut rr = Rng::new(3);
    for d in [0.1, 0.4] {
        let cue = add_address_noise(&ps[0], d, &mut rr);
        let t1 = Instant::now();
        let a = f.topk(&cue, 20, k);
        let ta = t1.elapsed().as_secs_f64();
        let t2 = Instant::now();
        let b = st.read_pulls_topk(&cue, 20, k);
        let tb = t2.elapsed().as_secs_f64();
        println!("bench M {} T {} r {} k {} filled {} write {:.1}s | dmg {}: fast {:.3}s ({} rounds) store {:.3}s same {}", m, t, r, k, f.rows.len(), tw, d, ta, a.rounds, tb, a.z == b.z);
    }
    stamp("bench-end");
}

/// One read under test on a store.
#[derive(Clone, Copy)]
enum Read {
    Topk(usize),
    Addr,
}

/// Per-read-address record: (success, signals). Signals are oriented so that SMALLER = more confident (accept iff
/// value <= threshold): travel, -cos1, -cosf, -dot1, -dotf, rounds, rand-travel (control), -land.
const SIGS: [&str; 8] = ["travel", "cos1", "cosf", "dot1", "dotf", "rounds", "control-randtravel", "oracle-land"];

fn signals(cue: &[i8], d: &Diag, stored: &[Vec<u64>], rand: &[i8]) -> [f64; 8] {
    let zc = pack(&d.z);
    let travel = hd(&pack(cue), &zc) as f64;
    let (_, nd) = nearest(stored, &zc);
    let land = 1.0 - 2.0 * nd as f64 / N as f64;
    let rt = hd(&pack(rand), &zc) as f64;
    [travel, -d.cos1, -d.cosf, -d.dot1, -d.dotf, d.rounds as f64, rt, -land]
}

/// max recall over thresholds with refusal >= level; recall = success and value <= h.
fn best_recall(stored: &[(bool, f64)], never: &[f64], level: f64) -> (f64, f64) {
    let mut hs: Vec<f64> = stored.iter().map(|x| x.1).chain(never.iter().copied()).collect();
    hs.push(f64::NEG_INFINITY);
    hs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    hs.dedup();
    let (mut best, mut bh) = (0.0, f64::NEG_INFINITY);
    for &h in &hs {
        let refuse = never.iter().filter(|&&v| v > h).count() as f64 / never.len().max(1) as f64;
        if refuse + 1e-12 < level {
            continue;
        }
        let rec = stored.iter().filter(|x| x.0 && x.1 <= h).count() as f64 / stored.len().max(1) as f64;
        if rec > best {
            best = rec;
            bh = h;
        }
    }
    (best, bh)
}

fn refuse(read: &str, m: usize, ts: &[usize]) {
    stamp(&format!("refuse-start-{}-M{}", read, m));
    let qs = env_usize("SDMREFUSE_QS", 100);
    let qn = env_usize("SDMREFUSE_QN", 200);
    let r = match read {
        "topk" => snr_radius(N, m),
        "addr0.3" => cd_radius(N, m, 0.3),
        "addr0.4" => cd_radius(N, m, 0.4),
        _ => panic!("read is topk | addr0.3 | addr0.4"),
    };
    let rd = if read == "topk" { Read::Topk((ball(N, r) * m as f64).round().max(1.0) as usize) } else { Read::Addr };
    println!("\n# REFUSE read {} n {} M {} r {} p {:.5} pM {:.1}; {} stored cues per damage, {} never-stored cues per load", read, N, m, r, ball(N, r), ball(N, r) * m as f64, qs, qn);
    let tmax = *ts.iter().max().unwrap();
    let all = pats(7000 + m as u64, tmax);
    let never = pats(9_000_000 + m as u64, qn);
    let rands = pats(8_000_000 + m as u64, qn.max(4 * qs));
    let mut st = Store::new(N, m, r, 4242 + m as u64);
    let mut written = 0;
    let mut tsorted = ts.to_vec();
    tsorted.sort();
    for &t in &tsorted {
        let t0 = Instant::now();
        st.write_many(&all[written..t]);
        written = t;
        let packed = pack_all(&all[..t]);
        let f = Fast::new(&st);
        let run = |cue: &[i8]| -> Diag {
            match rd {
                Read::Topk(k) => f.topk(cue, 20, k),
                Read::Addr => f.address(cue, 20),
            }
        };
        // stored read-addresses: (address-noise index, target, success, signals, oracle nn index, oracle nn distance)
        let srec = par(4 * qs, |k| {
            let di = k / qs;
            let mut rr = Rng::new(m as u64 * 7_919 + t as u64 * 104_729 + k as u64);
            let pi = rr.below(t);
            let cue = add_address_noise(&all[pi], DMG[di], &mut rr);
            let d = run(&cue);
            let ok = overlap(&d.z, &all[pi]) >= OK;
            let sg = signals(&cue, &d, &packed, &rands[k % rands.len()]);
            let (ni, nd) = nearest(&packed, &pack(&cue));
            (di, ok, sg, ni == pi, nd as f64)
        });
        let nrec = par(qn, |k| {
            let d = run(&never[k]);
            let sg = signals(&never[k], &d, &packed, &rands[k]);
            let (_, nd) = nearest(&packed, &pack(&never[k]));
            let landed = -sg[7] >= OK;
            (sg, nd as f64, landed)
        });
        let secs = t0.elapsed().as_secs_f64();
        // raw rows
        for (k, x) in srec.iter().enumerate() {
            let s: Vec<String> = x.2.iter().map(|v| format!("{:.4}", v)).collect();
            println!("ROW,cue,{},{},{},{},stored,{},{},{},{},{},{}", read, m, r, t, DMG[x.0], k, x.1 as u8, s.join(","), x.3 as u8, x.4);
        }
        for (k, x) in nrec.iter().enumerate() {
            let s: Vec<String> = x.0.iter().map(|v| format!("{:.4}", v)).collect();
            println!("ROW,cue,{},{},{},{},never,0,{},0,{},0,{}", read, m, r, t, k, s.join(","), x.1);
        }
        let landed = nrec.iter().filter(|x| x.2).count();
        let h = travel_threshold(N, t, 0.01);
        println!("## T {} ({:.1}s): never-stored cues landing on a stored pattern {}/{}; travel rule h(T) = {} (sealed rule R)", t, secs, landed, qn, h);
        let norec: Vec<String> = (0..4).map(|di| format!("{:.3}", srec.iter().filter(|x| x.0 == di && x.1).count() as f64 / qs as f64)).collect();
        println!("  read recall with no refusal at 10/20/30/40%: {}", norec.join(" "));
        println!("ROW,norefuse,{},{},{},{},{}", read, m, r, t, norec.join(","));
        // rule R point
        let rref = nrec.iter().filter(|x| x.0[0] > h as f64).count() as f64 / qn as f64;
        let rrec: Vec<String> = (0..4)
            .map(|di| {
                let v = srec.iter().filter(|x| x.0 == di && x.1 && x.2[0] <= h as f64).count() as f64 / qs as f64;
                let (orc, _, _) = oracle_point(N, DMG[di], t, h);
                format!("{:.3} (oracle {:.3})", v, orc)
            })
            .collect();
        println!("  rule R (accept iff travel <= {}): refusal {:.3} (exact {:.4}) | recall {}", h, rref, refusal_prob(N, t, h), rrec.join(" | "));
        println!("ROW,ruleR,{},{},{},{},{},{:.4},{:.4},{}", read, m, r, t, h, rref, refusal_prob(N, t, h), rrec.join(";"));
        // sweep every signal, plus the oracle nearest-neighbour arm
        println!("  signal | R50 / R90 / R99 at 10% | 20% | 30% | 40%   (R_x = best recall with never-stored refusal >= x)");
        let mut arms: Vec<(String, Vec<Vec<(bool, f64)>>, Vec<f64>)> = Vec::new();
        for (si, name) in SIGS.iter().enumerate() {
            let st_: Vec<Vec<(bool, f64)>> = (0..4).map(|di| srec.iter().filter(|x| x.0 == di).map(|x| (x.1, x.2[si])).collect()).collect();
            let nv: Vec<f64> = nrec.iter().map(|x| x.0[si]).collect();
            arms.push((name.to_string(), st_, nv));
        }
        let ost: Vec<Vec<(bool, f64)>> = (0..4).map(|di| srec.iter().filter(|x| x.0 == di).map(|x| (x.3, x.4)).collect()).collect();
        let onv: Vec<f64> = nrec.iter().map(|x| x.1).collect();
        arms.push(("oracle-nn".into(), ost, onv));
        for (name, st_, nv) in &arms {
            let cells: Vec<String> = (0..4)
                .map(|di| {
                    let v: Vec<String> = [0.5, 0.9, 0.99].iter().map(|&l| format!("{:.2}", best_recall(&st_[di], nv, l).0)).collect();
                    v.join("/")
                })
                .collect();
            println!("  {:<20} | {}", name, cells.join(" | "));
            let flat: Vec<String> = (0..4).flat_map(|di| [0.5, 0.9, 0.99].iter().map(move |&l| (di, l))).map(|(di, l)| format!("{:.4}", best_recall(&st_[di], nv, l).0)).collect();
            println!("ROW,sweep,{},{},{},{},{},{}", read, m, r, t, name, flat.join(","));
        }
        let ex: Vec<String> = (0..4)
            .map(|di| {
                let v: Vec<String> = [0.5, 0.9, 0.99]
                    .iter()
                    .map(|&l| {
                        let h = (0..=N).rev().find(|&h| refusal_prob(N, t, h) >= l).unwrap_or(0);
                        format!("{:.2}", oracle_point(N, DMG[di], t, h).0)
                    })
                    .collect();
                v.join("/")
            })
            .collect();
        println!("  {:<20} | {}", "oracle-nn exact", ex.join(" | "));
        stamp(&format!("refuse-{}-M{}-T{}", read, m, t));
    }
}

/// Follow-up rule S (sealed after the refusal grid): the memory reads P random probes of its own and, for each of
/// travel, cos1, dot1 and rounds, sets tau_s = the probes' value at rank floor(alpha / 4 * P) (sorted ascending, the
/// orientation lower = more confident). An answer is ACCEPTED iff some signal is strictly below its tau_s. Every
/// probe is activated a given signal with probability at most alpha / 4, so a never-stored read-address is accepted with
/// probability at most about alpha. Run on fresh seeds (stores, patterns, read-addresses and probes all new).
fn selfcal(read: &str, m: usize, ts: &[usize], off: u64) {
    stamp(&format!("selfcal-start-{}-M{}-seed{}", read, m, off));
    let qs = env_usize("SDMREFUSE_QS", 200);
    let qn = env_usize("SDMREFUSE_QN", 500);
    let np = env_usize("SDMREFUSE_PROBES", 1500);
    let alpha = 0.01;
    let used = [0usize, 1, 3, 5];
    let r = match read {
        "topk" => snr_radius(N, m),
        "addr0.3" => cd_radius(N, m, 0.3),
        "addr0.4" => cd_radius(N, m, 0.4),
        _ => panic!("read is topk | addr0.3 | addr0.4"),
    };
    let rd = if read == "topk" { Read::Topk((ball(N, r) * m as f64).round().max(1.0) as usize) } else { Read::Addr };
    println!("\n# SELFCAL read {} n {} M {} r {} seed offset {}; {} stored cues per damage, {} never-stored test cues, {} calibration probes, alpha {}", read, N, m, r, off, qs, qn, np, alpha);
    let tmax = *ts.iter().max().unwrap();
    let all = pats(7000 + m as u64 + 1_000_003 * off, tmax);
    let never = pats(9_000_000 + m as u64 + 1_000_003 * off, qn);
    let probes = pats(6_000_000 + m as u64 + 1_000_003 * off, np);
    let rands = pats(8_000_000 + m as u64 + 1_000_003 * off, qn.max(4 * qs).max(np));
    let mut st = Store::new(N, m, r, 4242 + m as u64 + 7 * off);
    let mut written = 0;
    let mut tsorted = ts.to_vec();
    tsorted.sort();
    for &t in &tsorted {
        let t0 = Instant::now();
        st.write_many(&all[written..t]);
        written = t;
        let packed = pack_all(&all[..t]);
        let f = Fast::new(&st);
        let run = |cue: &[i8]| -> Diag {
            match rd {
                Read::Topk(k) => f.topk(cue, 20, k),
                Read::Addr => f.address(cue, 20),
            }
        };
        let prec = par(np, |k| signals(&probes[k], &run(&probes[k]), &packed, &rands[k]));
        let rank = ((alpha / used.len() as f64) * np as f64).floor() as usize;
        let taus: Vec<f64> = used
            .iter()
            .map(|&si| {
                let mut v: Vec<f64> = prec.iter().map(|x| x[si]).collect();
                v.sort_by(|a, b| a.partial_cmp(b).unwrap());
                v[rank.min(v.len() - 1)]
            })
            .collect();
        let accept = |sg: &[f64; 8]| used.iter().zip(&taus).any(|(&si, &tau)| sg[si] < tau);
        let fires = |sg: &[f64; 8]| used.iter().zip(&taus).map(|(&si, &tau)| (sg[si] < tau) as u8).collect::<Vec<_>>();
        let srec = par(4 * qs, |k| {
            let di = k / qs;
            let mut rr = Rng::new(m as u64 * 7_919 + t as u64 * 104_729 + k as u64 + 99_991 * off);
            let pi = rr.below(t);
            let cue = add_address_noise(&all[pi], DMG[di], &mut rr);
            let d = run(&cue);
            (di, overlap(&d.z, &all[pi]) >= OK, signals(&cue, &d, &packed, &rands[k]))
        });
        let nrec = par(qn, |k| signals(&never[k], &run(&never[k]), &packed, &rands[k]));
        let h = travel_threshold(N, t, 0.01);
        let pf = prec.iter().filter(|x| accept(x)).count() as f64 / np as f64;
        let s_ref = nrec.iter().filter(|x| !accept(x)).count() as f64 / qn as f64;
        let r_ref = nrec.iter().filter(|x| x[0] > h as f64).count() as f64 / qn as f64;
        let per_sig: Vec<String> = (0..used.len()).map(|i| format!("{}", nrec.iter().filter(|x| fires(x)[i] == 1).count())).collect();
        println!(
            "## T {} ({:.1}s): taus travel {} cos1 {:.3} dot1 {:.3} rounds {} (rank {} of {}); probes accepted {:.4}; test never-stored refused: rule S {:.3} (fires per signal {}), rule R {:.3}",
            t,
            t0.elapsed().as_secs_f64(),
            taus[0],
            -taus[1],
            -taus[2],
            taus[3],
            rank,
            np,
            pf,
            s_ref,
            per_sig.join("/"),
            r_ref
        );
        let mut cells = Vec::new();
        for di in 0..4 {
            let xs: Vec<&(usize, bool, [f64; 8])> = srec.iter().filter(|x| x.0 == di).collect();
            let none = xs.iter().filter(|x| x.1).count() as f64 / qs as f64;
            let rs = xs.iter().filter(|x| x.1 && accept(&x.2)).count() as f64 / qs as f64;
            let rr_ = xs.iter().filter(|x| x.1 && x.2[0] <= h as f64).count() as f64 / qs as f64;
            println!("  {:.0}%: no refusal {:.3} | rule S {:.3} | rule R {:.3}", 100.0 * DMG[di], none, rs, rr_);
            cells.push(format!("{:.4};{:.4};{:.4}", none, rs, rr_));
        }
        println!("ROW,selfcal,{},{},{},{},{},{:.4},{:.4},{:.4},{}", read, m, r, t, off, pf, s_ref, r_ref, cells.join(","));
        stamp(&format!("selfcal-{}-M{}-T{}", read, m, t));
    }
}

/// address-noise:checkpoints groups, e.g. "0.2:10000,20000;0.3:3000,5000".
fn parse_spec(s: &str) -> Vec<(f64, Vec<usize>)> {
    s.split(';')
        .filter(|x| !x.is_empty())
        .map(|g| {
            let (d, c) = g.split_once(':').unwrap();
            (d.parse().unwrap(), c.split(',').map(|x| x.parse::<f64>().unwrap() as usize).collect())
        })
        .collect()
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

fn capacity(m: usize, spec: &str) {
    stamp(&format!("capacity-start-M{}", m));
    let q = env_usize("SDMREFUSE_Q", 150);
    let ns = env_usize("SDMREFUSE_SEEDS", 2) as u64;
    let groups = parse_spec(spec);
    let r = snr_radius(N, m);
    let k = (ball(N, r) * m as f64).round().max(1.0) as usize;
    let mut cps: Vec<usize> = groups.iter().flat_map(|g| g.1.clone()).collect();
    cps.sort();
    cps.dedup();
    println!("\n# CAPACITY top-k and block threshold, n {} M {} r {} k {}; {} cues per damage per checkpoint per seed x {} seeds; windows {}", N, m, r, k, q, ns, spec);
    // rates[group][checkpoint] = (topk successes, block successes, read-addresses)
    let mut tally: Vec<Vec<(usize, usize, usize)>> = groups.iter().map(|g| vec![(0, 0, 0); g.1.len()]).collect();
    for seed in 1..=ns {
        let all = pats(seed * 31 + m as u64, *cps.last().unwrap());
        let mut st = Store::new(N, m, r, 1000 + seed);
        let mut written = 0;
        for &t in &cps {
            let t0 = Instant::now();
            st.write_many(&all[written..t]);
            written = t;
            let f = Fast::new(&st);
            let (theta, kap, _) = density_threshold_blocks(&st, 0.1, 0.01);
            for (gi, (dmg, list)) in groups.iter().enumerate() {
                let Some(ci) = list.iter().position(|&x| x == t) else { continue };
                let res = par(q, |kq| {
                    let mut rr = Rng::new(seed * 1_000_003 + t as u64 * 7919 + (gi * 100_000 + kq) as u64);
                    let pi = rr.below(t);
                    let cue = add_address_noise(&all[pi], *dmg, &mut rr);
                    let a = overlap(&f.topk(&cue, 20, k).z, &all[pi]) >= OK;
                    let b = overlap(&f.thresh(&cue, 20, theta).z, &all[pi]) >= OK;
                    (a, b)
                });
                let (a, b) = (res.iter().filter(|x| x.0).count(), res.iter().filter(|x| x.1).count());
                tally[gi][ci].0 += a;
                tally[gi][ci].1 += b;
                tally[gi][ci].2 += q;
                println!("ROW,capcell,{},{},{},{},{},{},{},{},{:.1},{:.2}", m, r, seed, dmg, t, q, a, b, theta, kap);
            }
            println!("  seed {} T {} done ({:.1}s)", seed, t, t0.elapsed().as_secs_f64());
        }
        stamp(&format!("capacity-M{}-seed{}", m, seed));
    }
    for (gi, (dmg, list)) in groups.iter().enumerate() {
        let tk: Vec<(usize, f64)> = list.iter().zip(&tally[gi]).map(|(&t, x)| (t, x.0 as f64 / x.2.max(1) as f64)).collect();
        let bl: Vec<(usize, f64)> = list.iter().zip(&tally[gi]).map(|(&t, x)| (t, x.1 as f64 / x.2.max(1) as f64)).collect();
        let fmt = |v: &[(usize, f64)]| v.iter().map(|(t, s)| format!("{}:{:.3}", t, s)).collect::<Vec<_>>().join(" ");
        println!("CAP M {} damage {} top-k P90 {} P50 {} | {}", m, dmg, cap_from(&tk, 0.9), cap_from(&tk, 0.5), fmt(&tk));
        println!("CAP M {} damage {} block P90 {} P50 {} | {}", m, dmg, cap_from(&bl, 0.9), cap_from(&bl, 0.5), fmt(&bl));
        println!("ROW,cap,{},{},{},topk,{},{}", m, r, dmg, cap_from(&tk, 0.9), cap_from(&tk, 0.5));
        println!("ROW,cap,{},{},{},block,{},{}", m, r, dmg, cap_from(&bl, 0.9), cap_from(&bl, 0.5));
    }
    stamp(&format!("capacity-end-M{}", m));
}

/// Geometric checkpoints 1, 2, 3, 5, 7 per decade up to M/4 (SDMRADIUS's grid).
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

/// P90 / P50 on the checkpoint grid from a success-probability function (stops two checkpoints past 50%).
fn grid_caps<F: Fn(usize) -> f64>(m: usize, p: F) -> (usize, usize, Vec<(usize, f64)>) {
    let mut rows = Vec::new();
    let (mut c90, mut c50, mut f90, mut f50) = (0, 0, false, false);
    for t in checkpoints(m) {
        let s = p(t);
        rows.push((t, s));
        if !f90 {
            if s >= 0.9 {
                c90 = t;
            } else {
                f90 = true;
            }
        }
        if !f50 {
            if s >= 0.5 {
                c50 = t;
            } else {
                f50 = true;
            }
        }
        if f50 && s < 0.2 {
            break;
        }
    }
    (c90, c50, rows)
}

fn track(ms: &[usize]) {
    stamp("track-start");
    let smp = env_usize("SDMREFUSE_SAMPLES", 400);
    println!("\n# TRACK on SDMRADIUS's grid (checkpoints 1,2,3,5,7 per decade), {} sampled reads per T; FRESH and PERSIST beside FULL and RACE", smp);
    for &m in ms {
        for (tag, dmg) in [("snr", 0.1), ("0.2", 0.2), ("0.3", 0.3), ("0.4", 0.4)] {
            let r = if tag == "snr" { snr_radius(N, m) } else { cd_radius(N, m, dmg) };
            let t0 = Instant::now();
            let tr = Track::new(N, m, r);
            let fresh = grid_caps(m, |t| {
                let per = par(8, |c| tr.p_converge(dmg, t, smp / 8, false, 17 + c as u64));
                per.iter().sum::<f64>() / 8.0
            });
            let pers = grid_caps(m, |t| {
                let per = par(8, |c| tr.p_converge(dmg, t, smp / 8, true, 17 + c as u64));
                per.iter().sum::<f64>() / 8.0
            });
            let la = Lazy::averaged(N, m, r);
            let full = grid_caps(m, |t| la.p_converge_flips(dmg, t, goal(N)));
            let lz = Lazy::new(N, m, r);
            let race = grid_caps(m, |t| lz.p_converge_race(dmg, t, goal(N), 2000, 77));
            println!(
                "TRACK M {} {} r {} damage {}: P90 fresh {} persist {} full {} race {} | P50 fresh {} persist {} full {} race {} ({:.1}s)",
                m, tag, r, dmg, fresh.0, pers.0, full.0, race.0, fresh.1, pers.1, full.1, race.1, t0.elapsed().as_secs_f64()
            );
            let j = |v: &[(usize, f64)]| v.iter().map(|(t, s)| format!("{}:{:.3}", t, s)).collect::<Vec<_>>().join(" ");
            println!("  fresh   {}", j(&fresh.2));
            println!("  persist {}", j(&pers.2));
            println!("ROW,track,{},{},{},{},{},{},{},{},{},{},{},{}", m, tag, r, dmg, fresh.0, pers.0, full.0, race.0, fresh.1, pers.1, full.1, race.1);
        }
        stamp(&format!("track-M{}", m));
    }
}

fn fail(m: usize, r: usize, dmg: f64, ts: &[usize]) {
    stamp(&format!("fail-start-M{}-r{}", m, r));
    let q = env_usize("SDMREFUSE_Q", 400);
    let smp = env_usize("SDMREFUSE_SAMPLES", 2000);
    println!("\n# FAIL address read n {} M {} r {} damage {}: {} cues per T (store rebuilt per T, patterns 1..T)", N, m, r, dmg, q);
    let tmax = *ts.iter().max().unwrap();
    let all = pats(55_555 + m as u64 + r as u64, tmax);
    let mut st = Store::new(N, m, r, 77 + m as u64);
    let mut written = 0;
    let mut tsorted = ts.to_vec();
    tsorted.sort();
    let tr = Track::new(N, m, r);
    for &t in &tsorted {
        st.write_many(&all[written..t]);
        written = t;
        let f = Fast::new(&st);
        let ok = par(q, |k| {
            let mut rr = Rng::new(m as u64 * 31 + t as u64 * 7_919 + k as u64);
            let pi = rr.below(t);
            let cue = add_address_noise(&all[pi], dmg, &mut rr);
            overlap(&f.address(&cue, 20).z, &all[pi]) >= OK
        })
        .into_iter()
        .filter(|&x| x)
        .count();
        let meas = 1.0 - ok as f64 / q as f64;
        let se = (meas * (1.0 - meas) / q as f64).sqrt();
        let full = 1.0 - Lazy::averaged(N, m, r).p_converge_flips(dmg, t, goal(N));
        let race = 1.0 - Lazy::new(N, m, r).p_converge_race(dmg, t, goal(N), smp, 5);
        let fr = 1.0 - par(8, |c| tr.p_converge(dmg, t, smp / 8, false, 91 + c as u64)).iter().sum::<f64>() / 8.0;
        let pe = 1.0 - par(8, |c| tr.p_converge(dmg, t, smp / 8, true, 91 + c as u64)).iter().sum::<f64>() / 8.0;
        println!("T {}: measured failure {:.3} +- {:.3} | FULL {:.3} RACE {:.3} TRACK-fresh {:.3} TRACK-persist {:.3}", t, meas, se, full, race, fr, pe);
        println!("ROW,fail,{},{},{},{},{},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4}", m, r, dmg, t, q, meas, se, full, race, fr, pe);
    }
    stamp("fail-end");
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let part = a.get(1).map(|s| s.as_str()).unwrap_or("bench");
    let num = |i: usize, d: f64| a.get(i).and_then(|s| s.parse::<f64>().ok()).unwrap_or(d);
    match part {
        "bench" => bench(num(2, 1e6) as usize, num(3, 10_000.0) as usize),
        "refuse" => refuse(a.get(2).map(|s| s.as_str()).unwrap_or("topk"), num(3, 1e5) as usize, &parse_list(a.get(4))),
        "capacity" => capacity(num(2, 1e5) as usize, a.get(3).map(|s| s.as_str()).unwrap_or("0.3:1000,2000,3000")),
        "selfcal" => selfcal(a.get(2).map(|s| s.as_str()).unwrap_or("topk"), num(3, 1e5) as usize, &parse_list(a.get(4)), num(5, 1.0) as u64),
        "track" => track(&parse_list(a.get(2))),
        "fail" => fail(num(2, 1e5) as usize, num(3, 106.0) as usize, num(4, 0.3), &parse_list(a.get(5))),
        _ => eprintln!("unknown part {} (bench | refuse | capacity | track | fail)", part),
    }
}
