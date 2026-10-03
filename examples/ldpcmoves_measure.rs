//! LDPCMOVES measurements: moves that change several things at once, decoding at the Nishimori temperature, and
//! the sealed 10,000-sweep rerun of LDPCSETTLE's settle decoder, on SDMCODED's LDPC codes (n 512, codebook seed 1).
//! Run: `cargo run --release --example ldpcmoves_measure <part> [args]`, part one of
//!   pilot | grid <blocks> <sweeps> | long <blocks> | controls <blocks> | exact <blocks>
//! Predictions were sealed in the SETTLE campaign ledger before any measuring part ran. `pilot` prints only
//! wall-clock time per decode, never an error count. Every result is seeded; timings are never claimed.
//! Threads: LDPCMOVES_THREADS (default 6).

use settle::coded::Ldpc;
use settle::ldpcmoves::{hard_leans, Collapsed, Mover};
use settle::ldpcsettle::{coded_ldpc, hard_lean, llr_of_lean, Gadget, SettleCode};
use settle::memory::seed_of;
use settle::rng::Rng;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Instant;

const N: usize = 512;

fn threads() -> usize {
    std::env::var("LDPCMOVES_THREADS").ok().and_then(|s| s.parse().ok()).unwrap_or(6)
}

fn llr_unit(p: f64) -> f64 {
    ((1.0 - p) / p).ln()
}

fn par<T: Send, F: Fn(usize) -> T + Sync>(n: usize, f: F) -> Vec<T> {
    let next = AtomicUsize::new(0);
    let out: Mutex<Vec<Option<T>>> = Mutex::new((0..n).map(|_| None).collect());
    std::thread::scope(|s| {
        for _ in 0..threads() {
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

/// The received word of LDPCSETTLE's BSC block b (same seed, so the same words).
fn block(l: &Ldpc, r: f64, p: f64, b: usize) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let mut rr = Rng::new(seed_of(&format!("ldpcsettle:bsc:{}:{}:{}", r, p, b)));
    let info: Vec<u8> = (0..l.info.len()).map(|_| (rr.unit() < 0.5) as u8).collect();
    let sent = l.encode(&info);
    let y: Vec<u8> = sent.iter().map(|&x| x ^ (rr.unit() < p) as u8).collect();
    (info, sent, y)
}

fn info_of(l: &Ldpc, c: &[u8]) -> Vec<u8> {
    l.info.iter().map(|&p| c[p]).collect()
}

/// Wilson 95% interval for k of n.
fn wilson(k: usize, n: usize) -> (f64, f64) {
    if n == 0 {
        return (0.0, 1.0);
    }
    let (k, n, z) = (k as f64, n as f64, 1.959964);
    let ph = k / n;
    let d = 1.0 + z * z / n;
    let c = ph + z * z / (2.0 * n);
    let h = z * (ph * (1.0 - ph) / n + z * z / (4.0 * n * n)).sqrt();
    (((c - h) / d).max(0.0), ((c + h) / d).min(1.0))
}

#[derive(Default, Clone, Copy)]
struct Tally {
    blocks: usize,
    block_err: usize,
    refused: usize,
    mis: usize,
    bit_err: usize,
    search_fail: usize,
    model_fail: usize,
    work: f64,
}

/// One decode's result as the tally needs it.
struct Res {
    bits: Vec<u8>,
    claimed: bool,
    /// Some(true) = the sent codeword is calmer than what was found (search failure), Some(false) = not.
    search: Option<bool>,
    work: f64,
}

impl Tally {
    fn add(&mut self, l: &Ldpc, info: &[u8], r: &Res) {
        self.blocks += 1;
        let got = info_of(l, &r.bits);
        let e = got.iter().zip(info).filter(|(a, b)| a != b).count();
        self.bit_err += e;
        self.work += r.work;
        let failed = if !r.claimed {
            self.refused += 1;
            true
        } else if e > 0 {
            self.mis += 1;
            true
        } else {
            false
        };
        if failed {
            self.block_err += 1;
            match r.search {
                Some(true) => self.search_fail += 1,
                Some(false) => self.model_fail += 1,
                None => {}
            }
        }
    }
    fn merge(&mut self, o: &Tally) {
        self.blocks += o.blocks;
        self.block_err += o.block_err;
        self.refused += o.refused;
        self.mis += o.mis;
        self.bit_err += o.bit_err;
        self.search_fail += o.search_fail;
        self.model_fail += o.model_fail;
        self.work += o.work;
    }
    fn row(&self, k: usize) -> String {
        let (lo, hi) = wilson(self.block_err, self.blocks);
        format!(
            "{}\t{}\t{:.4}\t{:.4}\t{:.4}\t{:.3e}\t{}\t{}\t{}\t{}\t{:.3e}",
            self.blocks,
            self.block_err,
            self.block_err as f64 / self.blocks as f64,
            lo,
            hi,
            self.bit_err as f64 / (self.blocks * k) as f64,
            self.refused,
            self.mis,
            self.search_fail,
            self.model_fail,
            self.work / self.blocks as f64
        )
    }
}

const HEADER: &str = "rate\tp\tsweeps\tarm\tblocks\tblock_errors\tblock_error\twilson95_lo\twilson95_hi\tinfo_bit_error\trefused\tmiscorrected\tsearch_fail\tmodel_fail\tmean_work";

fn stamp(what: &str) {
    let load = std::process::Command::new("sysctl").args(["-n", "vm.loadavg"]).output().ok().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
    let pm = std::process::Command::new("sh")
        .args(["-c", "pmset -g | awk '/powermode/ {print $2}'"])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    let date = std::process::Command::new("date").args(["-u", "+%Y-%m-%dT%H:%M:%SZ"]).output().ok().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
    println!("# STAMP {} utc {} load {} powermode {} threads {}", what, date, load, pm, threads());
}

// ------------------------------------------------------------------------------------------------ arms

/// Full-space decode (LDPCSETTLE's own decoder), classified like LDPCSETTLE's diag.
fn full_anneal(sc: &SettleCode, y: &[u8], sent: &[u8], sweeps: usize, seed: u64) -> Res {
    let d = sc.decode(Some(y), sweeps, 1.0, 0.05, seed);
    let es: Vec<f64> = sc.complete(sent).iter().map(|&v| if v { 1.0 } else { -1.0 }).collect();
    let e_sent = sc.energy(&es);
    let ok = d.codeword && d.bits == sent;
    Res { claimed: d.codeword, search: if ok { None } else { Some(e_sent < d.energy - 1e-9) }, bits: d.bits, work: (sweeps * sc.things) as f64 }
}

fn col_anneal(c: &Collapsed, y: &[u8], sent: &[u8], sweeps: usize, mover: Mover, seed: u64) -> Res {
    let o = c.anneal(Some(y), sweeps, 1.0, 0.05, mover, seed);
    let ok = o.codeword && o.bits == sent;
    Res { claimed: o.codeword, search: if ok { None } else { Some(c.e0(sent) < o.e0 - 1e-9) }, bits: o.bits, work: o.work as f64 }
}

fn col_nish(c: &Collapsed, y: &[u8], sweeps: usize, mover: Mover, seed: u64) -> Res {
    let (o, _) = c.average(Some(y), sweeps, sweeps / 2, 1.0, mover, seed);
    Res { claimed: o.codeword, search: None, bits: o.bits, work: o.work as f64 }
}

fn full_nish(sc: &SettleCode, l: &Ldpc, y: &[u8], sweeps: usize, seed: u64) -> Res {
    let avg = sc.gibbs_average(y, sweeps, sweeps / 2, 1.0, seed);
    let bits: Vec<u8> = (0..l.n).map(|i| if (avg[i] - 0.5).abs() < 1e-12 { y[i] } else { (avg[i] > 0.5) as u8 }).collect();
    let claimed = l.syndrome_ok(&bits) && l.fixed_zero.iter().all(|&j| bits[j] == 0);
    Res { claimed, search: None, bits, work: (sweeps * sc.things) as f64 }
}

fn bp(l: &Ldpc, y: &[u8], p: f64, rounds: usize) -> Res {
    let llr: Vec<f64> = y.iter().map(|&x| llr_of_lean(hard_lean(x, p))).collect();
    match l.decode(&llr, rounds) {
        Some(c) => Res { bits: c, claimed: true, search: None, work: rounds as f64 },
        None => Res { bits: y.to_vec(), claimed: false, search: None, work: rounds as f64 },
    }
}

// ------------------------------------------------------------------------------------------------ pilot

fn part_pilot() {
    stamp("pilot");
    println!("# pilot: wall time per decode only, no error counts; rate 0.5 and 0.75, p 0.03, 3 blocks, 100 sweeps");
    for &r in &[0.5, 0.75] {
        let l = coded_ldpc(N, r, 1);
        let lam = llr_unit(0.03);
        let sweeps = 100;
        let mut arms: Vec<(String, Box<dyn Fn(&[u8], &[u8], u64)>)> = Vec::new();
        for g in [Gadget::Chain, Gadget::Sum] {
            let sc = SettleCode::from_ldpc(&l, g, lam);
            arms.push((
                format!("full_{}", g.name()),
                Box::new(move |y: &[u8], _s: &[u8], seed: u64| {
                    let mut s = sc.clone();
                    s.set_leans(&hard_leans(y, 0.03));
                    let _ = s.decode(Some(y), sweeps, 1.0, 0.05, seed);
                }),
            ));
            for mover in [Mover::Single, Mover::Block(4)] {
                let c0 = Collapsed::from_ldpc(&l, g, lam);
                arms.push((
                    format!("col_{}_{}", g.name(), mover.name()),
                    Box::new(move |y: &[u8], _s: &[u8], seed: u64| {
                        let mut c = c0.clone();
                        c.set_leans(&hard_leans(y, 0.03));
                        let _ = c.anneal(Some(y), sweeps, 1.0, 0.05, mover, seed);
                    }),
                ));
            }
        }
        for (name, f) in &arms {
            let t0 = Instant::now();
            for b in 0..3 {
                let (_, sent, y) = block(&l, r, 0.03, b);
                f(&y, &sent, b as u64);
            }
            println!("rate {} {} {} sweeps: {:.1} ms per decode", r, name, sweeps, t0.elapsed().as_secs_f64() * 1000.0 / 3.0);
        }
    }
}

// ------------------------------------------------------------------------------------------------ grid

const GRID: [(f64, f64); 7] = [(0.5, 0.01), (0.5, 0.03), (0.5, 0.05), (0.5, 0.07), (0.75, 0.01), (0.75, 0.02), (0.75, 0.03)];

const GRID_ARMS: [&str; 12] = [
    "bp50",
    "full_chain_anneal",
    "full_sum_anneal",
    "col_chain_single_anneal",
    "col_chain_block4_anneal",
    "col_sum_single_anneal",
    "col_sum_block4_anneal",
    "nish_col_chain_single_k1",
    "nish_col_chain_block4_k1",
    "nish_col_chain_block4_k4",
    "nish_full_chain_k1",
    "bp_rounds_eq_sweeps",
];

fn part_grid(blocks: usize, sweeps: usize, only: Option<usize>) {
    stamp(&format!("grid start, sweeps {}", sweeps));
    println!("# grid: n {}, {} blocks per cell, LDPCSETTLE's received words, anneal = warm start cooling 1 -> 0.05 keep the calmest,", N, blocks);
    println!("# nish = stay at T 1 and read each bit's average over the second half; kappa 1 unless the arm says k4; block4 = check blocks of 4");
    println!("# work: collapsed arms = one-check ln Z evaluations; full arms = single-thing updates; bp = rounds allowed");
    println!("{}", HEADER);
    for (ci, &(r, p)) in GRID.iter().enumerate() {
        if only.map_or(false, |o| o != ci) {
            continue;
        }
        let t0 = Instant::now();
        let l = coded_ldpc(N, r, 1);
        let lam = llr_unit(p);
        let full = [SettleCode::from_ldpc(&l, Gadget::Chain, lam), SettleCode::from_ldpc(&l, Gadget::Sum, lam)];
        let col_chain = Collapsed::from_ldpc(&l, Gadget::Chain, lam);
        let col_sum = Collapsed::from_ldpc(&l, Gadget::Sum, lam);
        let col_chain4 = Collapsed::from_ldpc(&l, Gadget::Chain, 4.0 * lam);
        let rows: Vec<[Tally; 12]> = par(blocks, |b| {
            let (info, sent, y) = block(&l, r, p, b);
            let leans = hard_leans(&y, p);
            let mut t = [Tally::default(); 12];
            let tag = |a: &str| seed_of(&format!("ldpcmoves:{}:{}:{}:{}:{}", a, r, p, b, sweeps));
            t[0].add(&l, &info, &bp(&l, &y, p, 50));
            for (gi, base) in full.iter().enumerate() {
                let mut sc = base.clone();
                sc.set_leans(&leans);
                // at 400 sweeps the seed is LDPCSETTLE's own, so these two arms reproduce its bsc.txt exactly
                let s = if sweeps == 400 { seed_of(&format!("ldpcsettle:dec:{}:{}:{}:{}", r, p, b, gi)) } else { tag(&format!("full{}", gi)) };
                t[1 + gi].add(&l, &info, &full_anneal(&sc, &y, &sent, sweeps, s));
            }
            let mut cc = col_chain.clone();
            cc.set_leans(&leans);
            let mut cs = col_sum.clone();
            cs.set_leans(&leans);
            let mut c4 = col_chain4.clone();
            c4.set_leans(&leans);
            t[3].add(&l, &info, &col_anneal(&cc, &y, &sent, sweeps, Mover::Single, tag("cc1")));
            t[4].add(&l, &info, &col_anneal(&cc, &y, &sent, sweeps, Mover::Block(4), tag("cc4")));
            t[5].add(&l, &info, &col_anneal(&cs, &y, &sent, sweeps, Mover::Single, tag("cs1")));
            t[6].add(&l, &info, &col_anneal(&cs, &y, &sent, sweeps, Mover::Block(4), tag("cs4")));
            t[7].add(&l, &info, &col_nish(&cc, &y, sweeps, Mover::Single, tag("n1")));
            t[8].add(&l, &info, &col_nish(&cc, &y, sweeps, Mover::Block(4), tag("n4")));
            t[9].add(&l, &info, &col_nish(&c4, &y, sweeps, Mover::Block(4), tag("n4k4")));
            let mut fc = full[0].clone();
            fc.set_leans(&leans);
            t[10].add(&l, &info, &full_nish(&fc, &l, &y, sweeps, tag("nf")));
            t[11].add(&l, &info, &bp(&l, &y, p, sweeps));
            t
        });
        let mut tot = [Tally::default(); 12];
        for row in &rows {
            for a in 0..12 {
                tot[a].merge(&row[a]);
            }
        }
        for a in 0..12 {
            println!("{}\t{}\t{}\t{}\t{}", r, p, sweeps, GRID_ARMS[a], tot[a].row(l.info.len()));
        }
        eprintln!("# rate {} p {} sweeps {}: {:.0}s", r, p, sweeps, t0.elapsed().as_secs_f64());
        stamp(&format!("grid cell rate {} p {} done", r, p));
    }
}

// ------------------------------------------------------------------------------------------------ long

fn part_long(blocks: usize) {
    stamp("long start");
    println!("# sealed rerun of LDPCSETTLE's settle decoder at 10,000 sweeps: warm start cooling 1 -> 0.05, kappa 1, fresh decode seeds,");
    println!("# LDPCSETTLE's received words (its blocks 0..{}); search_fail = the sent codeword (helpers completed) is calmer than the calmest found", blocks);
    println!("{}", HEADER);
    let cells: [(f64, f64, Gadget); 7] = [
        (0.5, 0.01, Gadget::Chain),
        (0.5, 0.03, Gadget::Chain),
        (0.5, 0.05, Gadget::Chain),
        (0.75, 0.01, Gadget::Chain),
        (0.75, 0.03, Gadget::Chain),
        (0.5, 0.01, Gadget::Sum),
        (0.75, 0.01, Gadget::Sum),
    ];
    let sweeps = 10_000;
    for &(r, p, g) in &cells {
        let t0 = Instant::now();
        let l = coded_ldpc(N, r, 1);
        let base = SettleCode::from_ldpc(&l, g, llr_unit(p));
        let rows: Vec<Tally> = par(blocks, |b| {
            let (info, sent, y) = block(&l, r, p, b);
            let mut sc = base.clone();
            sc.set_leans(&hard_leans(&y, p));
            let mut t = Tally::default();
            t.add(&l, &info, &full_anneal(&sc, &y, &sent, sweeps, seed_of(&format!("ldpcmoves:long:{}:{}:{}:{}", r, p, b, g.name()))));
            t
        });
        let mut tot = Tally::default();
        for t in &rows {
            tot.merge(t);
        }
        println!("{}\t{}\t{}\tfull_{}_anneal\t{}", r, p, sweeps, g.name(), tot.row(l.info.len()));
        eprintln!("# long rate {} p {} {}: {:.0}s", r, p, g.name(), t0.elapsed().as_secs_f64());
        stamp(&format!("long cell rate {} p {} {} done", r, p, g.name()));
    }
}

// ------------------------------------------------------------------------------------------------ controls

fn part_controls(blocks: usize) {
    stamp("controls start");
    println!("# controls for the new decoders (collapsed chain gadget, kappa 1, 400 sweeps)");
    println!("control\trate\tp\tdecoder\tblocks\tsent_returned\tother_codeword\trefused");
    let sweeps = 400;
    for &r in &[0.5, 0.75] {
        let l = coded_ldpc(N, r, 1);
        // (1) zero noise: received = sent, leans at p 0.01
        let res: Vec<[u8; 12]> = par(blocks, |b| {
            let (_, sent, _) = block(&l, r, 0.0, b);
            let mut c = Collapsed::from_ldpc(&l, Gadget::Chain, llr_unit(0.01));
            c.set_leans(&hard_leans(&sent, 0.01));
            let s = seed_of(&format!("ldpcmoves:zero:{}:{}", r, b));
            let outs = [
                c.anneal(Some(&sent), sweeps, 1.0, 0.05, Mover::Block(4), s),
                c.anneal(None, sweeps, 10.0, 0.05, Mover::Block(4), s ^ 1),
                c.average(Some(&sent), sweeps, sweeps / 2, 1.0, Mover::Block(4), s ^ 2).0,
                c.average(None, sweeps, sweeps / 2, 1.0, Mover::Block(4), s ^ 3).0,
            ];
            let mut v = [0u8; 12];
            for (k, o) in outs.iter().enumerate() {
                let sent_back = o.codeword && o.bits == sent;
                v[3 * k] = sent_back as u8;
                v[3 * k + 1] = (o.codeword && !sent_back) as u8;
                v[3 * k + 2] = (!o.codeword) as u8;
            }
            v
        });
        for (k, name) in ["anneal_block4_warm", "anneal_block4_random", "nish_block4_from_received", "nish_block4_random"].iter().enumerate() {
            let s: [usize; 3] = [0, 1, 2].map(|j| res.iter().map(|v| v[3 * k + j] as usize).sum());
            println!("zero_noise\t{}\t0\t{}\t{}\t{}\t{}\t{}", r, name, blocks, s[0], s[1], s[2]);
        }
        // (2) wrong matrix: decoders built from codebook seed 2, seed-1 codewords at p 0 and 0.01
        let wrong = coded_ldpc(N, r, 2);
        for &p in &[0.0, 0.01] {
            let res: Vec<[u8; 9]> = par(blocks, |b| {
                let (_, sent, y) = block(&l, r, p, b);
                let pe = if p > 0.0 { p } else { 0.01 };
                let mut c = Collapsed::from_ldpc(&wrong, Gadget::Chain, llr_unit(pe));
                c.set_leans(&hard_leans(&y, pe));
                let s = seed_of(&format!("ldpcmoves:wrong:{}:{}:{}", r, p, b));
                let a = c.anneal(Some(&y), sweeps, 1.0, 0.05, Mover::Block(4), s);
                let n = c.average(Some(&y), sweeps, sweeps / 2, 1.0, Mover::Block(4), s ^ 1).0;
                let llr: Vec<f64> = y.iter().map(|&x| llr_of_lean(hard_lean(x, pe))).collect();
                let bpo = wrong.decode(&llr, 50);
                let mut v = [0u8; 9];
                for (k, (cw, bits)) in [(a.codeword, a.bits.clone()), (n.codeword, n.bits.clone()), (bpo.is_some(), bpo.clone().unwrap_or_default())].iter().enumerate() {
                    let back = *cw && *bits == sent;
                    v[3 * k] = back as u8;
                    v[3 * k + 1] = (*cw && !back) as u8;
                    v[3 * k + 2] = (!*cw) as u8;
                }
                v
            });
            for (k, name) in ["anneal_block4", "nish_block4", "bp50"].iter().enumerate() {
                let s: [usize; 3] = [0, 1, 2].map(|j| res.iter().map(|v| v[3 * k + j] as usize).sum());
                println!("wrong_matrix\t{}\t{}\t{}\t{}\t{}\t{}\t{}", r, p, name, blocks, s[0], s[1], s[2]);
            }
        }
        // (3) non-codeword target: leans at p 1e-4, lambda 0.2
        let res: Vec<(u8, u8, u8)> = par(blocks, |b| {
            let mut rr = Rng::new(seed_of(&format!("ldpcmoves:nonword:{}:{}", r, b)));
            let target: Vec<u8> = (0..N).map(|_| (rr.unit() < 0.5) as u8).collect();
            let mut c = Collapsed::from_ldpc(&l, Gadget::Chain, 0.2);
            c.set_leans(&hard_leans(&target, 1e-4));
            let a = c.anneal(Some(&target), sweeps, 1.0, 0.05, Mover::Block(4), b as u64);
            ((a.bits == target) as u8, a.codeword as u8, (l.syndrome_ok(&target)) as u8)
        });
        let calm: usize = res.iter().map(|x| x.0 as usize).sum();
        let claimed: usize = res.iter().map(|x| x.1 as usize).sum();
        let target_cw: usize = res.iter().map(|x| x.2 as usize).sum();
        println!("non_codeword\t{}\t1e-4\tanneal_block4 (calmest = target {} of {}; target is a codeword {})\t{}\t0\t{}\t{}", r, calm, blocks, target_cw, blocks, claimed, blocks - claimed);
    }
}

// ------------------------------------------------------------------------------------------------ exact

/// A small code, decoded exactly by listing every word: the exact bitwise decision at T 1 (kappa 1), the exact
/// calmest word, the maximum-likelihood codeword, and the exact bitwise decision over codewords only; against
/// the sampled Nishimori decoder, the annealed decoder and BP on the same received words.
fn part_exact(blocks: usize) {
    stamp("exact start");
    let (n, k, seed) = (20usize, 10usize, 7u64);
    let l = Ldpc::build(n, k, seed);
    let wts: Vec<usize> = l.rows.iter().map(|r| r.len()).collect();
    println!("# exact: Ldpc::build({}, {}, {}), {} checks of weight {:?}, {} information bits, fixed zero {:?}", n, k, seed, l.rows.len(), wts, l.info.len(), l.fixed_zero);
    let codewords: Vec<u32> = (0u32..(1 << n)).filter(|&w| l.rows.iter().all(|r| r.iter().filter(|&&j| (w >> j) & 1 == 1).count() % 2 == 0) && l.fixed_zero.iter().all(|&j| (w >> j) & 1 == 0)).collect();
    println!("# {} codewords", codewords.len());
    println!("p\tdecoder\tblocks\tblock_errors\tblock_error\twilson95_lo\twilson95_hi\tbit_errors_per_bit\tagree_with_exact_mpm");
    let names = ["exact_mpm_T1_k1", "exact_calmest_k1", "exact_ml_codeword", "exact_mpm_codewords", "nish_block4_2000", "anneal_block4_400", "bp50"];
    for &p in &[0.05, 0.1, 0.15] {
        let lam = llr_unit(p);
        let res: Vec<Vec<(bool, usize, bool)>> = par(blocks, |b| {
            let mut rr = Rng::new(seed_of(&format!("ldpcmoves:exact:{}:{}", p, b)));
            let info: Vec<u8> = (0..l.info.len()).map(|_| (rr.unit() < 0.5) as u8).collect();
            let sent = l.encode(&info);
            let y: Vec<u8> = sent.iter().map(|&x| x ^ (rr.unit() < p) as u8).collect();
            let mut c = Collapsed::from_ldpc(&l, Gadget::Chain, lam);
            c.set_leans(&hard_leans(&y, p));
            let t = c.temp(1.0);
            // every word
            let mut lw = vec![0.0f64; 1 << n];
            let mut best = (f64::INFINITY, 0u32);
            let mut w = vec![0u8; n];
            for x in 0u32..(1 << n) {
                for i in 0..n {
                    w[i] = ((x >> i) & 1) as u8;
                }
                lw[x as usize] = c.ln_weight(&w, &t);
                let e = c.e0(&w);
                if e < best.0 {
                    best = (e, x);
                }
            }
            let m = lw.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
            let mut z = 0.0;
            let mut one = vec![0.0; n];
            for x in 0..(1usize << n) {
                let q = (lw[x] - m).exp();
                z += q;
                for i in 0..n {
                    if (x >> i) & 1 == 1 {
                        one[i] += q;
                    }
                }
            }
            let mpm: Vec<u8> = (0..n).map(|i| (one[i] / z > 0.5) as u8).collect();
            let calm: Vec<u8> = (0..n).map(|i| ((best.1 >> i) & 1) as u8).collect();
            // codewords only: channel term
            let ch = |x: u32| -> f64 { (0..n).map(|i| if (x >> i) & 1 == 1 { c.lean[i] } else { -c.lean[i] }).sum::<f64>() };
            let mut mlw = (f64::NEG_INFINITY, 0u32);
            let mut zc = 0.0;
            let mut onec = vec![0.0; n];
            let mc = codewords.iter().map(|&x| ch(x)).fold(f64::NEG_INFINITY, f64::max);
            for &x in &codewords {
                let v = ch(x);
                if v > mlw.0 {
                    mlw = (v, x);
                }
                let q = (v - mc).exp();
                zc += q;
                for i in 0..n {
                    if (x >> i) & 1 == 1 {
                        onec[i] += q;
                    }
                }
            }
            let ml: Vec<u8> = (0..n).map(|i| ((mlw.1 >> i) & 1) as u8).collect();
            let mpmc: Vec<u8> = (0..n).map(|i| (onec[i] / zc > 0.5) as u8).collect();
            let s = seed_of(&format!("ldpcmoves:exactdec:{}:{}", p, b));
            let nish = c.average(Some(&y), 2000, 1000, 1.0, Mover::Block(4), s).0.bits;
            let ann = c.anneal(Some(&y), 400, 1.0, 0.05, Mover::Block(4), s ^ 1).bits;
            let bpo = bp(&l, &y, p, 50);
            let bpb = bpo.bits.clone();
            let outs: Vec<(Vec<u8>, bool)> = vec![
                (mpm.clone(), l.syndrome_ok(&mpm)),
                (calm.clone(), l.syndrome_ok(&calm)),
                (ml, true),
                (mpmc.clone(), l.syndrome_ok(&mpmc)),
                (nish.clone(), l.syndrome_ok(&nish)),
                (ann.clone(), l.syndrome_ok(&ann)),
                (bpb, bpo.claimed),
            ];
            outs.iter()
                .map(|(bits, cw)| {
                    let wrong = bits.iter().zip(&sent).filter(|(a, b)| a != b).count();
                    (!(*cw && wrong == 0), wrong, *bits == mpm)
                })
                .collect()
        });
        for (a, name) in names.iter().enumerate() {
            let errs: usize = res.iter().filter(|v| v[a].0).count();
            let bits: usize = res.iter().map(|v| v[a].1).sum();
            let agree: usize = res.iter().filter(|v| v[a].2).count();
            let (lo, hi) = wilson(errs, blocks);
            println!("{}\t{}\t{}\t{}\t{:.4}\t{:.4}\t{:.4}\t{:.4e}\t{}", p, name, blocks, errs, errs as f64 / blocks as f64, lo, hi, bits as f64 / (blocks * n) as f64, agree);
        }
    }
    stamp("exact done");
}

// ------------------------------------------------------------------------------------------------ follow-ups

/// Follow-up F-a: the penalty strength for the collapsed annealer (kappa 2 and 4), 400 sweeps.
fn part_kappa(blocks: usize) {
    stamp("kappa start");
    println!("# follow-up F-a: collapsed block4 anneal at kappa 2 and 4, 400 sweeps, LDPCSETTLE's received words");
    println!("{}", HEADER);
    let cells = [(0.5, 0.03), (0.5, 0.05), (0.5, 0.07), (0.75, 0.01), (0.75, 0.02)];
    let arms = ["col_chain_block4_anneal_k2", "col_chain_block4_anneal_k4", "col_sum_block4_anneal_k2", "col_sum_block4_anneal_k4"];
    for &(r, p) in &cells {
        let t0 = Instant::now();
        let l = coded_ldpc(N, r, 1);
        let lam = llr_unit(p);
        let codes = [
            Collapsed::from_ldpc(&l, Gadget::Chain, 2.0 * lam),
            Collapsed::from_ldpc(&l, Gadget::Chain, 4.0 * lam),
            Collapsed::from_ldpc(&l, Gadget::Sum, 2.0 * lam),
            Collapsed::from_ldpc(&l, Gadget::Sum, 4.0 * lam),
        ];
        let rows: Vec<[Tally; 4]> = par(blocks, |b| {
            let (info, sent, y) = block(&l, r, p, b);
            let leans = hard_leans(&y, p);
            let mut t = [Tally::default(); 4];
            for (a, base) in codes.iter().enumerate() {
                let mut c = base.clone();
                c.set_leans(&leans);
                let s = seed_of(&format!("ldpcmoves:f:{}:{}:{}:{}", arms[a], r, p, b));
                t[a].add(&l, &info, &col_anneal(&c, &y, &sent, 400, Mover::Block(4), s));
            }
            t
        });
        let mut tot = [Tally::default(); 4];
        for row in &rows {
            for a in 0..4 {
                tot[a].merge(&row[a]);
            }
        }
        for a in 0..4 {
            println!("{}\t{}\t400\t{}\t{}", r, p, arms[a], tot[a].row(l.info.len()));
        }
        eprintln!("# kappa rate {} p {}: {:.0}s", r, p, t0.elapsed().as_secs_f64());
    }
    stamp("kappa done");
}

/// Follow-up F-b: T 1 averaging with a stiffer penalty (kappa 4, 8, 16), against the annealer at the same sweeps.
fn part_nish(blocks: usize, sweeps: usize) {
    stamp("nish start");
    println!("# follow-up F-b: T 1 averaging (second half of {} sweeps) with stiff penalties, against the block4 annealer at the same sweeps", sweeps);
    println!("{}", HEADER);
    let cells = [(0.5, 0.03), (0.5, 0.05), (0.75, 0.01)];
    let arms = ["nish_sum_block4_k4", "nish_sum_block4_k8", "nish_sum_block4_k16", "nish_chain_block4_k8", "col_sum_block4_anneal_k4"];
    for &(r, p) in &cells {
        let t0 = Instant::now();
        let l = coded_ldpc(N, r, 1);
        let lam = llr_unit(p);
        let codes = [
            Collapsed::from_ldpc(&l, Gadget::Sum, 4.0 * lam),
            Collapsed::from_ldpc(&l, Gadget::Sum, 8.0 * lam),
            Collapsed::from_ldpc(&l, Gadget::Sum, 16.0 * lam),
            Collapsed::from_ldpc(&l, Gadget::Chain, 8.0 * lam),
        ];
        let rows: Vec<[Tally; 5]> = par(blocks, |b| {
            let (info, sent, y) = block(&l, r, p, b);
            let leans = hard_leans(&y, p);
            let mut t = [Tally::default(); 5];
            for (a, base) in codes.iter().enumerate() {
                let mut c = base.clone();
                c.set_leans(&leans);
                let s = seed_of(&format!("ldpcmoves:f:{}:{}:{}:{}:{}", arms[a], r, p, b, sweeps));
                t[a].add(&l, &info, &col_nish(&c, &y, sweeps, Mover::Block(4), s));
            }
            let mut c = codes[0].clone();
            c.set_leans(&leans);
            let s = seed_of(&format!("ldpcmoves:f:{}:{}:{}:{}:{}", arms[4], r, p, b, sweeps));
            t[4].add(&l, &info, &col_anneal(&c, &y, &sent, sweeps, Mover::Block(4), s));
            t
        });
        let mut tot = [Tally::default(); 5];
        for row in &rows {
            for a in 0..5 {
                tot[a].merge(&row[a]);
            }
        }
        for a in 0..5 {
            println!("{}\t{}\t{}\t{}\t{}", r, p, sweeps, arms[a], tot[a].row(l.info.len()));
        }
        eprintln!("# nish rate {} p {}: {:.0}s", r, p, t0.elapsed().as_secs_f64());
    }
    stamp("nish done");
}

/// The free-energy price of one odd check against an even one, per gadget and row weight: at temperature T the
/// summed-out check contributes -T ln Z, so an odd check costs lambda - T ln(m) where m counts its calmest
/// helper arrangements. A deterministic table, no sampling.
fn part_entropy() {
    println!("# price of an odd check: -T (ln Z_odd - ln Z_even) / lambda at T = 1 and T = 0.2, for lambda = ln(0.97/0.03)");
    println!("gadget\tweight\tprice_T1\tprice_T0.2\tcalmest_odd_arrangements");
    let lam = llr_unit(0.03);
    for g in [Gadget::Sum, Gadget::Chain] {
        for w in [4usize, 6, 7, 10, 12, 13] {
            let even: Vec<u8> = (0..w).map(|i| (i < 2) as u8).collect();
            let odd: Vec<u8> = (0..w).map(|i| (i < 1) as u8).collect();
            let price = |temp: f64| -> f64 {
                let bl = lam / temp;
                -temp * (settle::ldpcmoves::ln_z_check(g, &odd, bl) - settle::ldpcmoves::ln_z_check(g, &even, bl)) / lam
            };
            // multiplicity at very low temperature: Z_odd e^{bl} -> m
            let bl = 60.0;
            let m = (settle::ldpcmoves::ln_z_check(g, &odd, bl) + bl).exp();
            println!("{}\t{}\t{:.3}\t{:.3}\t{:.2}", g.name(), w, price(1.0), price(0.2), m);
        }
    }
}

/// Follow-up F-c: the penalty ramps from kappa 1 to kappa 4 while the temperature falls, 400 sweeps.
fn part_ramp(blocks: usize) {
    stamp("ramp start");
    println!("# follow-up F-c: collapsed block4 annealer, T 1 -> 0.05 while kappa ramps 1 -> 4 (geometric), 400 sweeps, calmest judged at kappa 4");
    println!("{}", HEADER);
    let arms = ["col_chain_block4_ramp_k1to4", "col_sum_block4_ramp_k1to4"];
    for &(r, p) in GRID.iter() {
        let t0 = Instant::now();
        let l = coded_ldpc(N, r, 1);
        let lam = llr_unit(p);
        let codes = [Collapsed::from_ldpc(&l, Gadget::Chain, lam), Collapsed::from_ldpc(&l, Gadget::Sum, lam)];
        let rows: Vec<[Tally; 2]> = par(blocks, |b| {
            let (info, sent, y) = block(&l, r, p, b);
            let leans = hard_leans(&y, p);
            let mut t = [Tally::default(); 2];
            for (a, base) in codes.iter().enumerate() {
                let mut c = base.clone();
                c.set_leans(&leans);
                let s = seed_of(&format!("ldpcmoves:f:{}:{}:{}:{}", arms[a], r, p, b));
                let o = c.anneal_ramp(Some(&y), 400, 1.0, 0.05, 1.0, 4.0, Mover::Block(4), s);
                let ok = o.codeword && o.bits == sent;
                let res = Res { claimed: o.codeword, search: if ok { None } else { Some(c.e0_with(&sent, 4.0 * lam) < o.e0 - 1e-9) }, bits: o.bits, work: o.work as f64 };
                t[a].add(&l, &info, &res);
            }
            t
        });
        let mut tot = [Tally::default(); 2];
        for row in &rows {
            for a in 0..2 {
                tot[a].merge(&row[a]);
            }
        }
        for a in 0..2 {
            println!("{}\t{}\t400\t{}\t{}", r, p, arms[a], tot[a].row(l.info.len()));
        }
        eprintln!("# ramp rate {} p {}: {:.0}s", r, p, t0.elapsed().as_secs_f64());
    }
    stamp("ramp done");
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let num = |i: usize, d: usize| args.get(i).and_then(|s| s.parse().ok()).unwrap_or(d);
    match args.get(1).map(|s| s.as_str()).unwrap_or("pilot") {
        "pilot" => part_pilot(),
        "grid" => part_grid(num(2, 200), num(3, 400), args.get(4).and_then(|s| s.parse().ok())),
        "long" => part_long(num(2, 200)),
        "controls" => part_controls(num(2, 50)),
        "exact" => part_exact(num(2, 400)),
        "kappa" => part_kappa(num(2, 200)),
        "nish" => part_nish(num(2, 100), num(3, 2000)),
        "entropy" => part_entropy(),
        "ramp" => part_ramp(num(2, 200)),
        p => eprintln!("unknown part {} (pilot | grid <blocks> <sweeps> [cell] | long <blocks> | controls <blocks> | exact <blocks>)", p),
    }
}
