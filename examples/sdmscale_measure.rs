//! SDMSCALE measurements: Kanerva SDM from 2,000 to 1,000,000 hard locations (`src/sdmscale.rs`).
//!
//! Run: `cargo run --release --example sdmscale_measure <part> [args]`, part one of
//!   theory                      analytic predictions only (S-map, Kanerva 0.1 M, budgets); no memory is built
//!   capacity <n> <M,M,...>      recall vs stored count, address and pulls reads, address-noise 10/20/30/40%
//!   critical <n> <M,M,...>      iterated-read convergence vs exact read-address distance, loads M/100 and M/30
//!   anchor                      Kanerva's own point: n 1000, M 10^6, r 451, T 10^4 (wiki 85: d_crit 188)
//!   controls <n> <M,M,...>      never-stored read-addresses, entry shuffle, row shuffle with its mechanism counted
//!   anomaly                     SOFTSDM's row-shuffle recall (fire 0.01, softness 0) re-run with its cause counted
//!   hopfield <n> <budgetM,...>  zero-temperature Hopfield at the SDM's counter budget M x n (systematic expansion)
//! Everything is seeded. Wall times are printed with the load at the start and end of each part.
//! Raw rows go to stdout as `ROW,...` lines (CSV) beside the human tables.

use settle::rng::Rng;
use settle::sdm::radius_for;
use settle::sdmscale::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Instant;

const DMG: [f64; 4] = [0.1, 0.2, 0.3, 0.4];
const OK: f64 = 0.95;

fn stamp(tag: &str) {
    let up = std::process::Command::new("uptime").output().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
    let pm = std::process::Command::new("sh")
        .args(["-c", "pmset -g | grep -i powermode | awk '{print $2}'"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    println!("STAMP {} utc={} load=[{}] powermode={} threads={} policy={}", tag, utc(), up, pm, threads(), policy());
}

fn utc() -> String {
    std::process::Command::new("date").args(["-u", "+%Y-%m-%dT%H:%M:%SZ"]).output().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default()
}

/// The activation radius policy: activation probability at Bricken-Pehlevan's SNR optimum p* = (2 M T)^(-1/3) for a
/// design load T = M/20, i.e. p = (M^2/10)^(-1/3); activation radius = smallest r with P[Bin(n,1/2) <= r] >= p.
/// With SDMSCALE_POLICY=wide the activation radius is fixed at fire 0.02 (r = 112 at n = 256, SDMKEYS' S2000a) at every M.
/// SDMSCALE_RADIUS=<r> overrides both policies with one fixed activation radius (the activation radius scan).
fn policy_radius(n: usize, m: usize) -> usize {
    if let Some(r) = std::env::var("SDMSCALE_RADIUS").ok().and_then(|s| s.parse().ok()) {
        return r;
    }
    if policy() == "wide" {
        return radius_for(n, 0.02);
    }
    let p = (m as f64 * m as f64 / 10.0).powf(-1.0 / 3.0);
    radius_for(n, p)
}

fn policy() -> String {
    std::env::var("SDMSCALE_POLICY").unwrap_or_else(|_| "snr".to_string())
}

fn pats(seed: u64, n: usize, count: usize) -> Vec<Vec<i8>> {
    let mut r = Rng::new(seed.wrapping_mul(0x9E37_79B9).wrapping_add(n as u64));
    (0..count).map(|_| random_pattern(n, &mut r)).collect()
}

/// Run `f(i)` for i in 0..k across threads, collecting results in order.
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

fn checkpoints(m: usize) -> Vec<usize> {
    let mut v: Vec<usize> = vec![1, 2, 5, 10, 20, 50];
    for f in [0.0005, 0.001, 0.002, 0.005, 0.01, 0.02, 0.03, 0.05, 0.07, 0.1, 0.14, 0.2] {
        v.push((m as f64 * f).round() as usize);
    }
    v.retain(|&t| t >= 1 && t <= m / 4 + 1);
    v.sort();
    v.dedup();
    v
}

/// Largest checkpoint before the first one under `thr` (0 if the first is already under).
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

fn theory() {
    stamp("theory-start");
    println!("\n# THEORY (no memory built): S-map (Bricken-Pehlevan SNR, wiki WIKI_SDR 85 section 2) and Kanerva's rules");
    println!("# cap_smap(D) = largest T whose S-map from D*n reaches overlap 0.95 (distance <= 0.025 n)");
    println!("# n | M | r | p | kappa=pM | 0.1M | 0.089M | smap cap 10/20/30/40% | d_crit smap at T=M/100, M/30 | hop units at budget M*n | 0.138*units");
    for (pol, n) in [("snr", 256usize), ("snr", 1000), ("wide", 256)] {
        std::env::set_var("SDMSCALE_POLICY", pol);
        println!("## policy {}", pol);
        let ms: Vec<usize> = if n == 256 { vec![2000, 10000, 30000, 100000, 300000, 1000000] } else { vec![10000, 30000, 100000, 300000, 1000000] };
        for &m in &ms {
            let r = policy_radius(n, m);
            let p = ball(n, r);
            let s = SMap::new(n, m, r);
            let caps: Vec<usize> = DMG.iter().map(|&d| s.capacity((d * n as f64).round() as usize, 0.025 * n as f64)).collect();
            let hu = hop_units_for(m * n);
            println!(
                "{} | {} | {} | {:.5} | {:.1} | {} | {} | {:?} | {} {} | {} | {:.0}",
                n,
                m,
                r,
                p,
                p * m as f64,
                m / 10,
                (0.089 * m as f64) as usize,
                caps,
                s.critical(m / 100),
                s.critical(m / 30),
                hu,
                0.138 * hu as f64
            );
            let shared: Vec<String> = DMG.iter().map(|&d| format!("{:.2}", m as f64 * intersection(n, r, (d * n as f64).round() as usize))).collect();
            println!("    expected locations shared by a cue and its pattern, M I(d), at 0/10/20/30/40%: {:.1} {}", p * m as f64, shared.join(" "));
            println!("ROW,theory,{},{},{},{},{:.6},{},{},{},{},{},{},{}", pol, n, m, r, p, caps[0], caps[1], caps[2], caps[3], s.critical(m / 100), s.critical(m / 30), hu);
        }
    }
    std::env::remove_var("SDMSCALE_POLICY");
    let k = SMap::new(1000, 1_000_000, 451);
    println!("Kanerva anchor n 1000 M 1e6 r 451 T 1e4: smap d_crit {} (wiki 85: exact solver 159, Kanerva Fig. 7.3 188)", k.critical(10_000));
    stamp("theory-end");
}

struct CapRow {
    t: usize,
    addr: [f64; 4],
    pulls: [f64; 4],
    awake: f64,
    nonempty: f64,
}

fn capacity_one(n: usize, m: usize, r: usize, seed: u64, q: usize, do_pulls: bool) -> Vec<CapRow> {
    let cps = checkpoints(m);
    let all = pats(seed, n, *cps.last().unwrap());
    let mut st = Store::new(n, m, r, seed);
    let mut out = Vec::new();
    let mut written = 0;
    let mut fails = 0;
    for &t in &cps {
        st.write_many(&all[written..t]);
        written = t;
        // q queries per address-noise; the same read-address feeds both reads
        let res = par(DMG.len() * q, |k| {
            let (di, qi) = (k / q, k % q);
            let mut rr = Rng::new(seed * 1_000_003 + (t as u64) * 7919 + k as u64);
            let pi = rr.below(t);
            let cue = add_address_noise(&all[pi], DMG[di], &mut rr);
            let a = st.read_addresses(&cue, 20);
            let oa = overlap(&a.z, &all[pi]) >= OK;
            let op = if do_pulls { overlap(&st.read_pulls(&cue, 20).z, &all[pi]) >= OK } else { false };
            let _ = qi;
            (di, oa, op, a.awake, a.nonempty)
        });
        let mut addr = [0.0; 4];
        let mut pulls = [0.0; 4];
        let (mut aw, mut ne) = (0.0, 0.0);
        for &(di, oa, op, a, e) in &res {
            addr[di] += oa as u8 as f64 / q as f64;
            pulls[di] += op as u8 as f64 / q as f64;
            aw += a as f64;
            ne += e as f64;
        }
        let row = CapRow { t, addr, pulls, awake: aw / res.len() as f64, nonempty: ne / res.len() as f64 };
        let dead = row.addr[0] < 0.5 && (!do_pulls || row.pulls[0] < 0.5);
        out.push(row);
        if st.overflow > 0 {
            println!("WARNING overflow {} at T {}", st.overflow, t);
        }
        fails = if dead { fails + 1 } else { 0 };
        if fails >= 2 {
            break;
        }
    }
    out
}

fn capacity(n: usize, ms: &[usize], seeds: &[u64], q: usize, pulls_max_m: usize) {
    stamp("capacity-start");
    println!("\n# CAPACITY n={} (success = final overlap >= 0.95 after <= 20 reads; {} queries per damage per seed; seeds {:?})", n, q, seeds);
    for &m in ms {
        let r = policy_radius(n, m);
        let t0 = Instant::now();
        let do_pulls = m <= pulls_max_m;
        let runs: Vec<Vec<CapRow>> = seeds.iter().map(|&s| capacity_one(n, m, r, s, q, do_pulls)).collect();
        // pool seeds on the shared checkpoint prefix
        let len = runs.iter().map(|x| x.len()).min().unwrap();
        let mut addr_rows: [Vec<(usize, f64)>; 4] = Default::default();
        let mut pull_rows: [Vec<(usize, f64)>; 4] = Default::default();
        println!("## n {} M {} r {} p {:.5} (pulls read {})", n, m, r, ball(n, r), if do_pulls { "on" } else { "off: M is above the pulls_max_m argument" });
        println!("T | address 10/20/30/40% | pulls 10/20/30/40% | awake | awake holding counters");
        for i in 0..len {
            let t = runs[0][i].t;
            let mut a = [0.0; 4];
            let mut p = [0.0; 4];
            let (mut aw, mut ne) = (0.0, 0.0);
            for rr in &runs {
                for d in 0..4 {
                    a[d] += rr[i].addr[d] / runs.len() as f64;
                    p[d] += rr[i].pulls[d] / runs.len() as f64;
                }
                aw += rr[i].awake / runs.len() as f64;
                ne += rr[i].nonempty / runs.len() as f64;
            }
            for d in 0..4 {
                addr_rows[d].push((t, a[d]));
                pull_rows[d].push((t, p[d]));
            }
            println!("{} | {:.2} {:.2} {:.2} {:.2} | {:.2} {:.2} {:.2} {:.2} | {:.1} | {:.1}", t, a[0], a[1], a[2], a[3], p[0], p[1], p[2], p[3], aw, ne);
            println!("ROW,capacity,{},{},{},{},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.1},{:.1}", n, m, r, t, a[0], a[1], a[2], a[3], p[0], p[1], p[2], p[3], aw, ne);
        }
        let c90: Vec<usize> = (0..4).map(|d| cap_at(&addr_rows[d], 0.9)).collect();
        let c50: Vec<usize> = (0..4).map(|d| cap_at(&addr_rows[d], 0.5)).collect();
        let p90: Vec<usize> = (0..4).map(|d| cap_at(&pull_rows[d], 0.9)).collect();
        let p50: Vec<usize> = (0..4).map(|d| cap_at(&pull_rows[d], 0.5)).collect();
        println!("CAP n {} M {} address P90 {:?} P50 {:?} | pulls P90 {:?} P50 {:?} | {:.1}s", n, m, c90, c50, p90, p50, t0.elapsed().as_secs_f64());
        println!(
            "ROW,cap,{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{:.1}",
            n, m, r, c90[0], c90[1], c90[2], c90[3], c50[0], c50[1], c50[2], c50[3], p90[0], p90[1], p90[2], p90[3], p50[0], p50[1], p50[2], p50[3], t0.elapsed().as_secs_f64()
        );
        stamp(&format!("capacity-M{}", m));
    }
}

/// Convergence rate vs exact read-address distance d for one store; converged = final overlap >= 0.95.
fn conv_curve(st: &Store, stored: &[Vec<i8>], ds: &[usize], trials: usize, seed: u64) -> Vec<(usize, f64, f64)> {
    let res = par(ds.len() * trials, |k| {
        let (di, ti) = (k / trials, k % trials);
        let mut rr = Rng::new(seed * 7_777_777 + k as u64);
        let pi = rr.below(stored.len());
        let cue = flip_exactly(&stored[pi], ds[di], &mut rr);
        let o = st.read_addresses(&cue, 40);
        let _ = ti;
        (di, overlap(&o.z, &stored[pi]) >= OK, o.rounds as f64)
    });
    ds.iter()
        .enumerate()
        .map(|(di, &d)| {
            let hits: Vec<&(usize, bool, f64)> = res.iter().filter(|x| x.0 == di).collect();
            let ok = hits.iter().filter(|x| x.1).count() as f64 / hits.len() as f64;
            let rounds = hits.iter().map(|x| x.2).sum::<f64>() / hits.len() as f64;
            (d, ok, rounds)
        })
        .collect()
}

/// The distance at which the convergence rate falls through 0.5, interpolated between grid points.
fn crossing(curve: &[(usize, f64, f64)]) -> f64 {
    for w in curve.windows(2) {
        if w[0].1 >= 0.5 && w[1].1 < 0.5 {
            let (d0, r0, d1, r1) = (w[0].0 as f64, w[0].1, w[1].0 as f64, w[1].1);
            return d0 + (r0 - 0.5) / (r0 - r1) * (d1 - d0);
        }
    }
    if curve.first().map(|c| c.1 < 0.5).unwrap_or(true) {
        0.0
    } else {
        curve.last().unwrap().0 as f64
    }
}

fn critical(n: usize, ms: &[usize], trials: usize) {
    stamp("critical-start");
    println!("\n# CRITICAL DISTANCE n={} (cue exactly d bits from a stored pattern; converged = overlap >= 0.95 after <= 40 reads; {} trials per d)", n, trials);
    let step = if n == 256 { 8 } else { 25 };
    let ds: Vec<usize> = (0..=n / 2).step_by(step).collect();
    for &m in ms {
        let r = policy_radius(n, m);
        for &div in &[100usize, 30] {
            let t = (m / div).max(1);
            let t0 = Instant::now();
            let stored = pats(9000 + m as u64 + div as u64, n, t);
            let mut st = Store::new(n, m, r, 77 + div as u64);
            st.write_many(&stored);
            let curve = conv_curve(&st, &stored, &ds, trials, m as u64 + div as u64);
            let dc = crossing(&curve);
            let sm = SMap::new(n, m, r).critical(t);
            let line: Vec<String> = curve.iter().map(|(d, ok, _)| format!("{}:{:.2}", d, ok)).collect();
            println!("n {} M {} r {} T {} (M/{}): measured d_crit {:.1} ({:.3} n) | S-map {} ({:.3} n) | {:.1}s", n, m, r, t, div, dc, dc / n as f64, sm, sm as f64 / n as f64, t0.elapsed().as_secs_f64());
            println!("  curve {}", line.join(" "));
            println!("ROW,critical,{},{},{},{},{:.2},{}", n, m, r, t, dc, sm);
        }
        stamp(&format!("critical-M{}", m));
    }
}

fn anchor(trials: usize) {
    stamp("anchor-start");
    let (n, m, r, t) = (1000usize, 1_000_000usize, 451usize, 10_000usize);
    let t0 = Instant::now();
    let stored = pats(4242, n, t);
    let mut st = Store::new(n, m, r, 4242);
    st.write_many(&stored);
    println!("\n# KANERVA ANCHOR n {} M {} r {} T {}: built in {:.1}s, {} rows filled, overflow {}", n, m, r, t, t0.elapsed().as_secs_f64(), st.filled_rows(), st.overflow);
    let ds: Vec<usize> = (100..=300).step_by(10).collect();
    let curve = conv_curve(&st, &stored, &ds, trials, 4243);
    let dc = crossing(&curve);
    let line: Vec<String> = curve.iter().map(|(d, ok, rd)| format!("{}:{:.2}/{:.1}r", d, ok, rd)).collect();
    println!("measured d_crit {:.1} | S-map {} | wiki 85: exact solver 159, Kanerva Fig. 7.3 188 | {:.1}s", dc, SMap::new(n, m, r).critical(t), t0.elapsed().as_secs_f64());
    println!("  curve {}", line.join(" "));
    println!("ROW,anchor,{},{},{},{},{:.2},{}", n, m, r, t, dc, SMap::new(n, m, r).critical(t));
    stamp("anchor-end");
}

/// For each stored pattern q: is any hard location in A(q) now holding a row that q wrote (a row from inside A(q))?
fn self_landings(st: &Store, perm: &[usize], p: &[i8]) -> usize {
    let a: Vec<u32> = st.awake(p);
    let set: std::collections::HashSet<u32> = a.iter().copied().collect();
    a.iter().filter(|&&i| set.contains(&(perm[i as usize] as u32))).count()
}

fn controls(n: usize, ms: &[usize], q: usize) {
    stamp("controls-start");
    println!("\n# CONTROLS n={} ({} cues per cell; recall = overlap >= 0.95 with the cue's own pattern)", n, q);
    println!("M | load | T | unshuffled@10% | never@10% | never@0% (silent) | entries@10% | rows@10% | rows: cues with a self-landing q-row, successes among them | mean awake / holding counters (unshuffled, rows)");
    for &m in ms {
        let r = policy_radius(n, m);
        let p = ball(n, r);
        for (lname, t) in [("sparse pT=0.2", ((0.2 / p).round() as usize).max(2)), ("M/100", (m / 100).max(2))] {
            let stored = pats(31337 + m as u64, n, t);
            let mut st = Store::new(n, m, r, 555 + m as u64);
            st.write_many(&stored);
            let recall = |s: &Store, dmg: f64, salt: u64| -> Vec<(bool, usize, usize)> {
                par(q, |k| {
                    let mut rr = Rng::new(salt * 1000 + k as u64);
                    let pi = rr.below(stored.len());
                    let cue = add_address_noise(&stored[pi], dmg, &mut rr);
                    let first = s.read_addresses(&cue, 1);
                    let o = s.read_addresses(&cue, 20);
                    (overlap(&o.z, &stored[pi]) >= OK, first.awake, first.nonempty)
                })
            };
            let base = recall(&st, 0.1, 1);
            let never = pats(999_999 + m as u64, n, q);
            let nv = |dmg: f64| -> (usize, usize) {
                let res = par(q, |k| {
                    let mut rr = Rng::new(5000 + k as u64);
                    let cue = add_address_noise(&never[k], dmg, &mut rr);
                    let first = st.read_addresses(&cue, 1);
                    (overlap(&st.read_addresses(&cue, 20).z, &never[k]) >= OK, first.nonempty == 0)
                });
                (res.iter().filter(|x| x.0).count(), res.iter().filter(|x| x.1).count())
            };
            let (nv10, _) = nv(0.1);
            let (nv0, sil0) = nv(0.0);
            let mut e = st.clone();
            e.shuffle_entries(&mut Rng::new(3));
            let ent = recall(&e, 0.1, 2);
            // row shuffle, three seeds, with the mechanism counted per read-address
            let (mut rows_ok, mut rows_n, mut land_cues, mut land_ok, mut row_aw, mut row_ne) = (0, 0, 0, 0, 0.0, 0.0);
            for sh in 0..3u64 {
                let mut w = st.clone();
                let perm = w.shuffle_rows(&mut Rng::new(10 + sh));
                let res = par(q, |k| {
                    let mut rr = Rng::new(sh * 100_000 + 7000 + k as u64);
                    let pi = rr.below(stored.len());
                    let cue = add_address_noise(&stored[pi], 0.1, &mut rr);
                    let first = w.read_addresses(&cue, 1);
                    let ok = overlap(&w.read_addresses(&cue, 20).z, &stored[pi]) >= OK;
                    (ok, self_landings(&w, &perm, &stored[pi]) > 0, first.awake, first.nonempty)
                });
                for &(ok, land, aw, ne) in &res {
                    rows_n += 1;
                    rows_ok += ok as usize;
                    if land {
                        land_cues += 1;
                        land_ok += ok as usize;
                    }
                    row_aw += aw as f64;
                    row_ne += ne as f64;
                }
            }
            let cnt = |v: &[(bool, usize, usize)]| v.iter().filter(|x| x.0).count();
            let mean = |v: &[(bool, usize, usize)], f: fn(&(bool, usize, usize)) -> usize| v.iter().map(|x| f(x) as f64).sum::<f64>() / v.len() as f64;
            println!(
                "{} | {} | {} | {}/{} | {}/{} | {}/{} ({} silent) | {}/{} | {}/{} | {} cues, {} recalled | {:.1}/{:.1} , {:.1}/{:.1}",
                m, lname, t, cnt(&base), q, nv10, q, nv0, q, sil0, cnt(&ent), q, rows_ok, rows_n, land_cues, land_ok,
                mean(&base, |x| x.1), mean(&base, |x| x.2), row_aw / rows_n as f64, row_ne / rows_n as f64
            );
            println!("ROW,controls,{},{},{},{},{},{},{},{},{},{},{},{},{},{}", n, m, t, cnt(&base), nv10, nv0, sil0, cnt(&ent), rows_ok, rows_n, land_cues, land_ok, q, lname);
        }
        stamp(&format!("controls-M{}", m));
    }
}

/// SOFTSDM's machine and its own row-shuffle protocol, with the cause counted.
fn anomaly(seeds: u64) {
    use settle::softsdm::{with_address_noise, overlap as fov, Machine};
    stamp("anomaly-start");
    println!("\n# ANOMALY: softsdm Machine(256, 2000, fire 0.01, softness 0, gain 64), 10% damage, 3 rounds x 16 samples, rows permuted");
    println!("T | cues | recalled | recalled with a self-landing q-row in A(q) | cues with one | q is a fixed point of the shuffled mean-field read (of stored) | mean firing / firing rows holding counters | same, recalled cues");
    for &t in &[5usize, 10, 20, 40] {
        let res = par(seeds as usize, |si| {
            let seed = si as u64 + 1;
            let mut mc = Machine::new(256, 2000, 0.01, 0.0, 64.0, seed, 16);
            let mut rng = Rng::new(seed ^ 0xBEEF);
            let ps: Vec<Vec<f64>> = (0..t).map(|_| (0..256).map(|_| if rng.unit() < 0.5 { -1.0 } else { 1.0 }).collect()).collect();
            // which hard locations each pattern woke at write time (hard activation radius: is activated iff distance < t)
            let fired = |mc: &Machine, p: &[f64]| -> Vec<usize> { (0..mc.m).filter(|&l| mc.dist(l, p) < mc.t).collect() };
            let wrote: Vec<Vec<usize>> = ps.iter().map(|p| fired(&mc, p)).collect();
            for p in &ps {
                mc.write(p, &mut rng);
            }
            let mut rows: Vec<Vec<f64>> = mc.j.chunks(256).map(|c| c.to_vec()).collect();
            let mut perm: Vec<usize> = (0..rows.len()).collect();
            let mut r2 = Rng::new(seed ^ 0x5A5A);
            for i in (1..rows.len()).rev() {
                let r = r2.below(i + 1);
                rows.swap(i, r);
                perm.swap(i, r);
            }
            mc.j = rows.concat();
            let filled: Vec<bool> = mc.j.chunks(256).map(|c| c.iter().any(|&x| x != 0.0)).collect();
            let mut out = Vec::new();
            for (qi, p) in ps.iter().enumerate() {
                let aq: std::collections::HashSet<usize> = wrote[qi].iter().copied().collect();
                let land = wrote[qi].iter().any(|&i| aq.contains(&perm[i]));
                let fixed = fov(&mc.recall_mean_field(p, 1), p) >= 0.95;
                let cue = with_address_noise(p, 0.1, &mut rng);
                let fl = fired(&mc, &cue);
                let ne = fl.iter().filter(|&&l| filled[l]).count();
                // the same three pass reads mc.recall makes (same rng draws), traced round by round:
                // (was activated, was activated rows holding bit-counters, was activated rows that hold q, overlap with q after the round)
                let mut c = cue.clone();
                let mut trace = Vec::new();
                for _ in 0..3 {
                    let f = fired(&mc, &c);
                    let fne = f.iter().filter(|&&l| filled[l]).count();
                    let fq = f.iter().filter(|&&l| aq.contains(&perm[l])).count();
                    c = mc.read_pass(&c, 16, &mut rng).0;
                    trace.push((f.len(), fne, fq, fov(&c, p)));
                }
                let o = fov(&c, p);
                // stability: one more read from the final state; and the same three reads from pure noise
                let stable = o >= 0.95 && fov(&mc.read_pass(&c, 16, &mut rng).0, p) >= 0.95;
                let noise: Vec<f64> = (0..256).map(|_| if rng.unit() < 0.5 { -1.0 } else { 1.0 }).collect();
                let from_noise = fov(&mc.recall(&noise, 3, 16, false, 0, &mut rng), p) >= 0.95;
                let first_round_ok = trace[0].3 >= 0.95;
                if o >= 0.95 {
                    println!("  RECALL T {} seed {} q {}: self-landed {} | rounds (fired, holding, holding q, overlap after) {:?}", ps.len(), seed, qi, land, trace);
                }
                out.push((o >= 0.95, land, fixed, fl.len(), ne, stable, from_noise, first_round_ok && o >= 0.95));
            }
            out
        });
        type Trial = (bool, bool, bool, usize, usize, bool, bool, bool);
        let all: Vec<&Trial> = res.iter().flatten().collect();
        let stable = all.iter().filter(|x| x.5).count();
        let noise_hits = all.iter().filter(|x| x.6).count();
        let r1 = all.iter().filter(|x| x.7).count();
        let n = all.len();
        let ok = all.iter().filter(|x| x.0).count();
        let ok_land = all.iter().filter(|x| x.0 && x.1).count();
        let land = all.iter().filter(|x| x.1).count();
        let fixed = all.iter().filter(|x| x.2).count();
        let mfire = all.iter().map(|x| x.3 as f64).sum::<f64>() / n as f64;
        let mne = all.iter().map(|x| x.4 as f64).sum::<f64>() / n as f64;
        let okv: Vec<&&Trial> = all.iter().filter(|x| x.0).collect();
        let (of, on) = if okv.is_empty() { (0.0, 0.0) } else { (okv.iter().map(|x| x.3 as f64).sum::<f64>() / okv.len() as f64, okv.iter().map(|x| x.4 as f64).sum::<f64>() / okv.len() as f64) };
        println!("{} | {} | {} | {} | {} | {} | {:.1}/{:.2} | {:.1}/{:.2}", t, n, ok, ok_land, land, fixed, mfire, mne, of, on);
        println!("  of the {} recalls: {} already at q after round 1, {} still at q after a 4th read; the same 3 reads from PURE NOISE land on q {} times of {}", ok, r1, stable, noise_hits, n);
        println!("ROW,anomaly,{},{},{},{},{},{},{:.2},{:.3},{:.2},{:.3},{},{},{}", t, n, ok, ok_land, land, fixed, mfire, mne, of, on, r1, stable, noise_hits);
    }
    stamp("anomaly-end");
}

fn hopfield(n: usize, budgets_m: &[usize], q: usize) {
    stamp("hopfield-start");
    println!("\n# HOPFIELD at the SDM's counter budget M x n (pulls nh(nh-1)/2 >= M n), zero temperature, async sweeps <= 30, systematic expansion [p ; sign(R p)]");
    for &m in budgets_m {
        let budget = m * n;
        let nh = if m == 0 { n } else { hop_units_for(budget) };
        let native = nh <= n;
        let t0 = Instant::now();
        let mut h = Hop::new(n, nh, !native, 17 + m as u64);
        let maxt = (0.2 * nh as f64) as usize + 2;
        let all = pats(271828 + m as u64, n, maxt);
        let mut cps: Vec<usize> = vec![1, 2, 5, 10, 20];
        for f in [0.005, 0.01, 0.02, 0.03, 0.05, 0.07, 0.1, 0.14, 0.2] {
            cps.push((nh as f64 * f).round() as usize);
        }
        cps.retain(|&t| t >= 1 && t <= maxt);
        cps.sort();
        cps.dedup();
        let mut rows: [Vec<(usize, f64)>; 4] = Default::default();
        let mut written = 0;
        let mut fails = 0;
        println!("## Hopfield units {} (pulls {}) at SDM M {} (budget {})", h.nh, h.nh * (h.nh - 1) / 2, m, budget);
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
            println!("{} | {:.2} {:.2} {:.2} {:.2}", t, a[0], a[1], a[2], a[3]);
            println!("ROW,hopfield,{},{},{},{},{:.3},{:.3},{:.3},{:.3}", n, m, h.nh, t, a[0], a[1], a[2], a[3]);
            fails = if a[0] < 0.5 { fails + 1 } else { 0 };
            if fails >= 2 {
                break;
            }
        }
        let c90: Vec<usize> = (0..4).map(|d| cap_at(&rows[d], 0.9)).collect();
        let c50: Vec<usize> = (0..4).map(|d| cap_at(&rows[d], 0.5)).collect();
        println!("HCAP n {} M {} units {} P90 {:?} P50 {:?} | {:.1}s", n, m, h.nh, c90, c50, t0.elapsed().as_secs_f64());
        println!("ROW,hcap,{},{},{},{},{},{},{},{},{},{},{},{:.1}", n, m, h.nh, c90[0], c90[1], c90[2], c90[3], c50[0], c50[1], c50[2], c50[3], t0.elapsed().as_secs_f64());
        stamp(&format!("hopfield-M{}", m));
    }
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let part = a.get(1).map(|s| s.as_str()).unwrap_or("theory");
    let n = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(256usize);
    match part {
        "theory" => theory(),
        "capacity" => {
            let ms = parse_list(a.get(3), &[2000, 10000, 30000, 100000]);
            let pm = a.get(4).and_then(|s| s.parse().ok()).unwrap_or(100_000usize);
            capacity(n, &ms, &[1, 2], 30, pm)
        }
        "critical" => critical(n, &parse_list(a.get(3), &[2000, 10000, 30000, 100000]), 16),
        "anchor" => anchor(12),
        "controls" => controls(n, &parse_list(a.get(3), &[2000, 10000, 100000]), 60),
        "anomaly" => anomaly(a.get(2).and_then(|s| s.parse().ok()).unwrap_or(40)),
        "hopfield" => hopfield(n, &parse_list(a.get(3), &[0, 2000, 10000, 30000, 100000]), 30),
        _ => eprintln!("unknown part {}", part),
    }
}
