//! LDPCSETTLE measurements: decode SDMCODED's LDPC codes by settling, against its belief propagation.
//! Run: `cargo run --release --example ldpcsettle_measure <part> [args]`, part one of
//!   census | pilot | bsc [blocks] | kappa [blocks] | controls | soft <sum|chain> | diag [blocks]
//! Predictions were sealed in the SETTLE campaign ledger before the measuring parts were run. `pilot` prints
//! only wall-clock time per decode (to pick the sweep budget) and never an error count. Everything is seeded.

#![allow(clippy::needless_range_loop)] // index loops mirror the equations they measure

use settle::coded::{unframe, CodeKind, Comp, English, Ldpc, Pipeline, TEST};
use settle::ldpcsettle::{coded_ldpc, hard_lean, llr_of_lean, soft_lean_laplace, soft_lean_scaled, Gadget, SettleCode};
use settle::memory::{code, seed_of, store_pattern};
use settle::model::{Model, State};
use settle::rng::Rng;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Instant;

const N: usize = 512;
const RATES: [f64; 2] = [0.5, 0.75];
const PS: [f64; 4] = [0.01, 0.03, 0.05, 0.1];
/// Sweep budget of the settle decoder (fixed by the pilot before any error count was seen).
const SWEEPS: usize = 400;
/// Penalty strength as a multiple of one bit's log-likelihood ratio ln((1-p)/p).
const KAPPA: f64 = 1.0;
const THREADS: usize = 8;
const ASSUMED_P: f64 = 0.02;

fn llr_unit(p: f64) -> f64 {
    ((1.0 - p) / p).ln()
}

/// Run `f(i)` for i in 0..n on THREADS threads; results in order.
fn par<T: Send, F: Fn(usize) -> T + Sync>(n: usize, f: F) -> Vec<T> {
    let next = AtomicUsize::new(0);
    let out: Mutex<Vec<Option<T>>> = Mutex::new((0..n).map(|_| None).collect());
    std::thread::scope(|s| {
        for _ in 0..THREADS {
            s.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                if i >= n {
                    break;
                }
                let v = f(i);
                out.lock().unwrap()[i] = Some(v);
            });
        }
    });
    out.into_inner().unwrap().into_iter().map(|x| x.unwrap()).collect()
}

fn info_of(l: &Ldpc, c: &[u8]) -> Vec<u8> {
    l.info.iter().map(|&p| c[p]).collect()
}

// ------------------------------------------------------------------------------------------------ census

fn part_census() {
    println!("# gadget census, n = {}, the SDMCODED matrices (codebook seed 1), pulls and leans at lambda = 1", N);
    println!("rate\tchecks\trow_weight_min\trow_weight_mean\trow_weight_max\tfixed_zero\tgadget\tthings\thelpers\tpulls\tmax_abs_pull\tmax_abs_penalty_lean");
    for &r in &RATES {
        let l = coded_ldpc(N, r, 1);
        let w: Vec<usize> = l.rows.iter().map(|x| x.len()).collect();
        for g in [Gadget::Sum, Gadget::Chain] {
            let sc = SettleCode::from_ldpc(&l, g, 1.0);
            let (j, h) = sc.max_pull_and_lean();
            println!(
                "{}\t{}\t{}\t{:.2}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                r,
                l.rows.len(),
                w.iter().min().unwrap(),
                w.iter().sum::<usize>() as f64 / w.len() as f64,
                w.iter().max().unwrap(),
                l.fixed_zero.len(),
                g.name(),
                sc.things,
                sc.helpers(),
                sc.pulls(),
                j,
                h
            );
        }
    }
}

// ------------------------------------------------------------------------------------------------ pilot

fn part_pilot() {
    println!("# pilot: wall time per settle decode only (no error counts), rate 0.5 and 0.75, p 0.03, 8 blocks");
    for &r in &RATES {
        let l = coded_ldpc(N, r, 1);
        for g in [Gadget::Sum, Gadget::Chain] {
            let sc = SettleCode::from_ldpc(&l, g, KAPPA * llr_unit(0.03));
            for sweeps in [200usize, 400, 1000] {
                let t0 = Instant::now();
                for b in 0..8u64 {
                    let mut c = sc.clone();
                    let mut rr = Rng::new(b + 1);
                    let info: Vec<u8> = (0..l.info.len()).map(|_| (rr.unit() < 0.5) as u8).collect();
                    let y: Vec<u8> = l.encode(&info).iter().map(|&x| x ^ (rr.unit() < 0.03) as u8).collect();
                    c.set_leans(&y.iter().map(|&x| hard_lean(x, 0.03)).collect::<Vec<_>>());
                    let _ = c.decode(Some(&y), sweeps, 1.0, 0.05, b);
                }
                println!("rate {} {} sweeps {}: {:.1} ms per decode", r, g.name(), sweeps, t0.elapsed().as_secs_f64() * 1000.0 / 8.0);
            }
        }
    }
}

// ------------------------------------------------------------------------------------------------ bsc

#[derive(Default, Clone, Copy)]
struct Tally {
    blocks: usize,
    block_err: usize,
    refused: usize,
    mis: usize,
    bit_err: usize,
}

impl Tally {
    fn add(&mut self, sent_info: &[u8], got_info: &[u8], claimed: bool) {
        self.blocks += 1;
        let e = got_info.iter().zip(sent_info).filter(|(a, b)| a != b).count();
        self.bit_err += e;
        if !claimed {
            self.refused += 1;
            self.block_err += 1;
        } else if e > 0 {
            self.mis += 1;
            self.block_err += 1;
        }
    }
    fn row(&self, k: usize) -> String {
        format!(
            "{}\t{:.4}\t{:.3e}\t{:.4}\t{}",
            self.blocks,
            self.block_err as f64 / self.blocks as f64,
            self.bit_err as f64 / (self.blocks * k) as f64,
            self.refused as f64 / self.blocks as f64,
            self.mis
        )
    }
}

const ARMS: [&str; 6] = ["bp_true_p", "bp_p0.02", "settle_chain_warm", "settle_sum_warm", "settle_chain_random", "settle_sum_random"];

fn part_bsc(blocks: usize, kappa: f64, only_p: Option<f64>) {
    println!("# binary symmetric channel, n = {}, {} blocks per cell, same received words for every arm", N, blocks);
    println!("# settle: {} sweeps, warm = start at the received word cooling 1 -> 0.05, random = start at noise cooling 10 -> 0.05; lambda = {} x ln((1-p)/p)", SWEEPS, kappa);
    println!("rate\tp\tarm\tblocks\tblock_error\tinfo_bit_error\trefused\tmiscorrected");
    for &r in &RATES {
        let l = coded_ldpc(N, r, 1);
        for &p in &PS {
            if let Some(q) = only_p {
                if (q - p).abs() > 1e-12 {
                    continue;
                }
            }
            let t0 = Instant::now();
            let lam = kappa * llr_unit(p);
            let codes = [SettleCode::from_ldpc(&l, Gadget::Chain, lam), SettleCode::from_ldpc(&l, Gadget::Sum, lam)];
            let rows: Vec<[Tally; 6]> = par(blocks, |b| {
                let mut rr = Rng::new(seed_of(&format!("ldpcsettle:bsc:{}:{}:{}", r, p, b)));
                let info: Vec<u8> = (0..l.info.len()).map(|_| (rr.unit() < 0.5) as u8).collect();
                let sent = l.encode(&info);
                let y: Vec<u8> = sent.iter().map(|&x| x ^ (rr.unit() < p) as u8).collect();
                let mut t = [Tally::default(); 6];
                // BP, matched and as SDMCODED ran it
                for (a, pp) in [(0, p), (1, ASSUMED_P)] {
                    let llr: Vec<f64> = y.iter().map(|&x| llr_of_lean(hard_lean(x, pp))).collect();
                    match l.decode(&llr, 50) {
                        Some(c) => t[a].add(&info, &info_of(&l, &c), true),
                        None => t[a].add(&info, &info_of(&l, &y), false),
                    }
                }
                let lean: Vec<f64> = y.iter().map(|&x| hard_lean(x, p)).collect();
                for (gi, base) in codes.iter().enumerate() {
                    let mut sc = base.clone();
                    sc.set_leans(&lean);
                    let s = seed_of(&format!("ldpcsettle:dec:{}:{}:{}:{}", r, p, b, gi));
                    let w = sc.decode(Some(&y), SWEEPS, 1.0, 0.05, s);
                    t[2 + gi].add(&info, &info_of(&l, &w.bits), w.codeword);
                    let z = sc.decode(None, SWEEPS, 10.0, 0.05, s ^ 0x5bd1);
                    t[4 + gi].add(&info, &info_of(&l, &z.bits), z.codeword);
                }
                t
            });
            let mut tot = [Tally::default(); 6];
            for row in &rows {
                for a in 0..6 {
                    tot[a].blocks += row[a].blocks;
                    tot[a].block_err += row[a].block_err;
                    tot[a].refused += row[a].refused;
                    tot[a].mis += row[a].mis;
                    tot[a].bit_err += row[a].bit_err;
                }
            }
            for a in 0..6 {
                println!("{}\t{}\t{}\t{}", r, p, ARMS[a], tot[a].row(l.info.len()));
            }
            eprintln!("# rate {} p {}: {:.0}s", r, p, t0.elapsed().as_secs_f64());
        }
    }
}

// ------------------------------------------------------------------------------------------------ controls

fn part_controls() {
    println!("# controls");
    // (1) zero noise: the received word is the sent codeword; leans at p = 0.01.
    for &r in &RATES {
        let l = coded_ldpc(N, r, 1);
        for g in [Gadget::Chain, Gadget::Sum] {
            let base = SettleCode::from_ldpc(&l, g, KAPPA * llr_unit(0.01));
            let res = par(50, |b| {
                let mut rr = Rng::new(seed_of(&format!("ldpcsettle:zero:{}:{}", r, b)));
                let info: Vec<u8> = (0..l.info.len()).map(|_| (rr.unit() < 0.5) as u8).collect();
                let c = l.encode(&info);
                let mut sc = base.clone();
                sc.set_leans(&c.iter().map(|&x| hard_lean(x, 0.01)).collect::<Vec<_>>());
                let w = sc.decode(Some(&c), SWEEPS, 1.0, 0.05, b as u64);
                let z = sc.decode(None, SWEEPS, 10.0, 0.05, b as u64 + 999);
                ((w.codeword && w.bits == c) as usize, (z.codeword && z.bits == c) as usize, (z.codeword && z.bits != c) as usize)
            });
            let (a, b, m) = res.iter().fold((0, 0, 0), |x, y| (x.0 + y.0, x.1 + y.1, x.2 + y.2));
            println!("zero noise, rate {} {}: warm returns the sent codeword {} of 50; random start {} of 50 (a wrong codeword {})", r, g.name(), a, b, m);
        }
    }
    // (2) a wrong parity matrix: gadgets from codebook seed 2, codewords of seed 1 at p = 0.01 and at p = 0.
    for &r in &RATES {
        let l = coded_ldpc(N, r, 1);
        let wrong = coded_ldpc(N, r, 2);
        for &p in &[0.0, 0.01] {
            let pe = if p == 0.0 { 0.01 } else { p };
            let base = SettleCode::from_ldpc(&wrong, Gadget::Chain, KAPPA * llr_unit(pe));
            let res = par(50, |b| {
                let mut rr = Rng::new(seed_of(&format!("ldpcsettle:wrong:{}:{}:{}", r, p, b)));
                let info: Vec<u8> = (0..l.info.len()).map(|_| (rr.unit() < 0.5) as u8).collect();
                let c = l.encode(&info);
                let y: Vec<u8> = c.iter().map(|&x| x ^ (rr.unit() < p) as u8).collect();
                let mut sc = base.clone();
                sc.set_leans(&y.iter().map(|&x| hard_lean(x, pe)).collect::<Vec<_>>());
                let w = sc.decode(Some(&y), SWEEPS, 1.0, 0.05, b as u64);
                let llr: Vec<f64> = y.iter().map(|&x| llr_of_lean(hard_lean(x, pe))).collect();
                let bp = wrong.decode(&llr, 50);
                (
                    (w.codeword && w.bits == c) as usize,
                    w.codeword as usize,
                    bp.as_ref().map_or(0, |x| (x == &c) as usize),
                    bp.is_some() as usize,
                )
            });
            let t = res.iter().fold((0, 0, 0, 0), |x, y| (x.0 + y.0, x.1 + y.1, x.2 + y.2, x.3 + y.3));
            println!(
                "wrong matrix, rate {} p {}: settle returns the sent word {} of 50 (claims a codeword of the wrong code {}); BP returns the sent word {} of 50 (claims {})",
                r, p, t.0, t.1, t.2, t.3
            );
        }
    }
    // (3) a non-codeword target: leans far stronger than the penalty (lambda 0.2, p 1e-4), start at the target.
    let l = coded_ldpc(N, 0.5, 1);
    let base = SettleCode::from_ldpc(&l, Gadget::Chain, 0.2);
    let res = par(50, |b| {
        let mut rr = Rng::new(seed_of(&format!("ldpcsettle:nonword:{}", b)));
        let target: Vec<u8> = (0..N).map(|j| if l.fixed_zero.contains(&j) { 0 } else { (rr.unit() < 0.5) as u8 }).collect();
        let mut sc = base.clone();
        sc.set_leans(&target.iter().map(|&x| hard_lean(x, 1e-4)).collect::<Vec<_>>());
        let w = sc.decode(Some(&target), SWEEPS, 1.0, 0.05, b as u64);
        ((w.bits == target) as usize, w.codeword as usize)
    });
    let t = res.iter().fold((0, 0), |x, y| (x.0 + y.0, x.1 + y.1));
    println!("non-codeword target, rate 0.5: the calmest arrangement is the target {} of 50; the decoder claims a codeword {} of 50", t.0, t.1);
}

// ------------------------------------------------------------------------------------------------ soft input

const LENGTHS: [usize; 10] = [16, 24, 32, 40, 48, 61, 80, 100, 120, 140];
const LOAD: usize = 65;
const TRIALS: usize = 20;
const LAST: usize = 10;

/// Passage `t` of length `len` (SDMCODED's rule).
fn passage(t: usize, len: usize) -> Vec<u8> {
    let b = TEST.as_bytes();
    let mut s = t * 500;
    while s > 0 && b[s - 1] != b' ' {
        s += 1;
    }
    b[s..s + len].to_vec()
}

fn unstore(m: &mut Model, p: &[f64]) {
    let w = 1.0 / N as f64;
    for i in 0..N {
        for e in m.adj[i].iter_mut() {
            e.1 -= w * p[i] * p[e.0];
        }
    }
}

/// SDMCODED's recall (30 sweeps at temperature 0.1), also averaging the last LAST sweeps.
fn recall(m: &Model, cue: &[f64], seed: u64) -> (Vec<f64>, Vec<f64>) {
    let mut st = State::new(seed);
    let (mut s, mut free) = st.start(m);
    s[..N].copy_from_slice(cue);
    let mut avg = vec![0.0; N];
    for k in 0..30 {
        st.sweep(m, &mut s, &mut free, 1.0 / 0.1);
        if k >= 30 - LAST {
            for i in 0..N {
                avg[i] += s[i] / LAST as f64;
            }
        }
    }
    (s[..N].to_vec(), avg)
}

const SOFT_ARMS: [&str; 6] = ["bp_hard", "bp_soft_scaled", "bp_soft_laplace", "settle_hard", "settle_soft_scaled", "settle_soft_laplace"];

/// Decode a recall with one of the arms; Ok(text) or Err.
fn soft_decode(pl: &Pipeline, sc: &SettleCode, fin: &[f64], avg: &[f64], arm: usize, model: &English, seed: u64) -> Result<Vec<u8>, ()> {
    let l = pl.codec.ldpc.as_ref().unwrap();
    let l0 = llr_unit(ASSUMED_P);
    for sign in [1.0, -1.0] {
        // unmasked, sign-corrected: +1 means bit 1
        let hard: Vec<u8> = fin.iter().zip(&pl.mask).map(|(&v, &k)| (sign * v * k > 0.0) as u8).collect();
        let m: Vec<f64> = avg.iter().zip(&pl.mask).map(|(&v, &k)| sign * v * k).collect();
        let lean: Vec<f64> = match arm % 3 {
            0 => hard.iter().map(|&b| hard_lean(b, ASSUMED_P)).collect(),
            1 => m.iter().map(|&x| soft_lean_scaled(x, ASSUMED_P)).collect(),
            _ => m.iter().map(|&x| soft_lean_laplace(x, LAST)).collect(),
        };
        let cw = if arm < 3 {
            let llr: Vec<f64> = lean.iter().map(|&x| llr_of_lean(x)).collect();
            l.decode(&llr, 50)
        } else {
            let mut s = sc.clone();
            s.set_leans(&lean);
            let d = s.decode(Some(&hard), SWEEPS, 1.0, 0.05, seed ^ (sign > 0.0) as u64);
            if d.codeword {
                Some(d.bits)
            } else {
                None
            }
        };
        let _ = l0;
        if let Some(c) = cw {
            let info = info_of(l, &c);
            if let Ok(t) = unframe(&info, pl.comp, model) {
                return Ok(t);
            }
        }
    }
    Err(())
}

fn part_soft(gadget: Gadget) {
    let model = English::trained();
    let combos: Vec<(Comp, CodeKind)> = vec![
        (Comp::Ac, CodeKind::Ldpc(750)),
        (Comp::Ac, CodeKind::Ldpc(500)),
        (Comp::None, CodeKind::Ldpc(750)),
        (Comp::None, CodeKind::Ldpc(500)),
    ];
    let damages = [0.1, 0.2];
    // key: (combo, damage index, len) -> [exact per arm], found, fits
    type Row = ([usize; 6], [usize; 6], usize, usize);
    let t0 = Instant::now();
    type Cell = ((usize, usize, usize), Row);
    let per_trial: Vec<Vec<Cell>> = par(TRIALS, |t| {
        let mut m = Model::default();
        for i in 0..N {
            m.add(&format!("m_{}", i));
        }
        for o in 0..LOAD {
            store_pattern(&mut m, 0, N, 1.0, &code(&format!("other-{}-{}", t, o), N));
        }
        let mut out = Vec::new();
        for (ci, (c, k)) in combos.iter().enumerate() {
            let pl = Pipeline::new(&format!("note{}", t), N, *c, *k, 1);
            let sc = SettleCode::from_ldpc(pl.codec.ldpc.as_ref().unwrap(), gadget, KAPPA * llr_unit(ASSUMED_P));
            for &len in &LENGTHS {
                let text = passage(t, len);
                let pat = pl.pattern(&text, model);
                for (di, &d) in damages.iter().enumerate() {
                    let mut row: Row = ([0; 6], [0; 6], 0, 0);
                    if let Ok(p) = &pat {
                        row.3 = 1;
                        store_pattern(&mut m, 0, N, 1.0, p);
                        let s = seed_of(&format!("cue:{}:{}:{}:{}:{}:{}:{}:{}", false, t, LOAD, c.name(), k.name(), len, d, 1u8));
                        let mut r = Rng::new(s);
                        let cue: Vec<f64> = p.iter().map(|&b| if r.unit() < d { -b } else { b }).collect();
                        let (fin, avg) = recall(&m, &cue, s ^ 0x9e37);
                        unstore(&mut m, p);
                        let o = fin.iter().zip(p).map(|(a, b)| a * b).sum::<f64>() / N as f64;
                        row.2 = (o.abs() >= 0.9) as usize;
                        for a in 0..6 {
                            match soft_decode(&pl, &sc, &fin, &avg, a, model, s) {
                                Ok(x) if x == text => row.0[a] = 1,
                                Ok(_) => row.1[a] = 1,
                                Err(()) => {}
                            }
                        }
                    }
                    out.push(((ci, di, len), row));
                }
            }
        }
        out
    });
    eprintln!("# soft cell: {:.0}s", t0.elapsed().as_secs_f64());
    println!("# crowded Hopfield: 512 things, {} other memories, whole-pattern cue, recall 30 sweeps at T 0.1, soft = mean of the last {} sweeps", LOAD, LAST);
    println!("# settle arms: {} gadget, lambda = ln(0.98/0.02), {} sweeps from the recalled hard bits, cooling 1 -> 0.05", gadget.name(), SWEEPS);
    println!("comp\tcode\tdamage\tlen\tfits\tfound\t{}\tsilent_wrong_any", SOFT_ARMS.join("\t"));
    let mut totals = vec![[0usize; 6]; combos.len() * damages.len()];
    let mut found_tot = vec![0usize; combos.len() * damages.len()];
    let mut silent = 0;
    for (ci, (c, k)) in combos.iter().enumerate() {
        for di in 0..damages.len() {
            for &len in &LENGTHS {
                let mut ex = [0usize; 6];
                let (mut fits, mut found, mut sw) = (0, 0, 0);
                for trial in &per_trial {
                    for ((a, b, ln), row) in trial {
                        if *a == ci && *b == di && *ln == len {
                            for x in 0..6 {
                                ex[x] += row.0[x];
                                sw += row.1[x];
                            }
                            found += row.2;
                            fits += row.3;
                        }
                    }
                }
                silent += sw;
                for x in 0..6 {
                    totals[ci * damages.len() + di][x] += ex[x];
                }
                found_tot[ci * damages.len() + di] += found;
                println!(
                    "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                    c.name(),
                    k.name(),
                    damages[di],
                    len,
                    fits,
                    found,
                    ex.iter().map(|v| v.to_string()).collect::<Vec<_>>().join("\t"),
                    sw
                );
            }
        }
    }
    println!("# totals over lengths (exact notes of 200 attempts per row; found = recalls within overlap 0.9)");
    println!("comp\tcode\tdamage\tfound\t{}", SOFT_ARMS.join("\t"));
    for (ci, (c, k)) in combos.iter().enumerate() {
        for di in 0..damages.len() {
            let t = &totals[ci * damages.len() + di];
            println!(
                "{}\t{}\t{}\t{}\t{}",
                c.name(),
                k.name(),
                damages[di],
                found_tot[ci * damages.len() + di],
                t.iter().map(|v| v.to_string()).collect::<Vec<_>>().join("\t")
            );
        }
    }
    println!("# silent wrong texts (check passed, text wrong) across every arm and cell: {}", silent);
}

// ------------------------------------------------------------------------------------------------ diag (post-hoc)

/// Post-hoc, declared after the sealed parts ran: is a settle failure a SEARCH failure (the sent codeword is
/// calmer than what was found) or a MODEL failure (something calmer than the sent codeword exists)? And does
/// a longer settle help?
fn part_diag(blocks: usize) {
    println!("# post-hoc diagnosis: warm start, kappa 1; search failure = the sent codeword (helpers completed) is calmer");
    println!("# than the calmest arrangement found; model failure = the found arrangement is calmer than the sent codeword");
    println!("rate\tp\tgadget\tsweeps\tblocks\tsuccess\tsearch_failure\tmodel_failure\tmean_broken_checks_when_failed");
    for &(r, p) in &[(0.5, 0.01), (0.5, 0.03), (0.75, 0.01)] {
        let l = coded_ldpc(N, r, 1);
        for g in [Gadget::Chain, Gadget::Sum] {
            let base = SettleCode::from_ldpc(&l, g, KAPPA * llr_unit(p));
            for sweeps in [400usize, 2000, 10000] {
                let res = par(blocks, |b| {
                    let mut rr = Rng::new(seed_of(&format!("ldpcsettle:bsc:{}:{}:{}", r, p, b)));
                    let info: Vec<u8> = (0..l.info.len()).map(|_| (rr.unit() < 0.5) as u8).collect();
                    let sent = l.encode(&info);
                    let y: Vec<u8> = sent.iter().map(|&x| x ^ (rr.unit() < p) as u8).collect();
                    let mut sc = base.clone();
                    sc.set_leans(&y.iter().map(|&x| hard_lean(x, p)).collect::<Vec<_>>());
                    let d = sc.decode(Some(&y), sweeps, 1.0, 0.05, seed_of(&format!("ldpcsettle:diag:{}:{}:{}:{}", r, p, b, sweeps)));
                    let es: Vec<f64> = sc.complete(&sent).iter().map(|&v| if v { 1.0 } else { -1.0 }).collect();
                    let e_sent = sc.energy(&es);
                    let ok = d.codeword && d.bits == sent;
                    (ok as usize, (!ok && e_sent < d.energy - 1e-9) as usize, (!ok && e_sent >= d.energy - 1e-9) as usize, if ok { 0 } else { d.broken })
                });
                let t = res.iter().fold((0, 0, 0, 0), |a, x| (a.0 + x.0, a.1 + x.1, a.2 + x.2, a.3 + x.3));
                let fails = blocks - t.0;
                println!(
                    "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{:.1}",
                    r, p, g.name(), sweeps, blocks, t.0, t.1, t.2,
                    if fails > 0 { t.3 as f64 / fails as f64 } else { 0.0 }
                );
            }
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let num = |i: usize, d: usize| args.get(i).and_then(|s| s.parse().ok()).unwrap_or(d);
    match args.get(1).map(|s| s.as_str()).unwrap_or("census") {
        "census" => part_census(),
        "pilot" => part_pilot(),
        "bsc" => part_bsc(num(2, 200), KAPPA, None),
        "kappa" => {
            for kap in [0.5, 1.0, 2.0, 4.0] {
                println!("# kappa {}", kap);
                part_bsc(num(2, 100), kap, Some(0.03));
            }
        }
        "controls" => part_controls(),
        "diag" => part_diag(num(2, 50)),
        "soft" => part_soft(Gadget::parse(args.get(2).map(|s| s.as_str()).unwrap_or("chain")).expect("soft <sum|chain>")),
        p => eprintln!("unknown part {} (census | pilot | bsc [blocks] | kappa [blocks] | controls | soft <sum|chain> | diag [blocks])", p),
    }
}
