//! SDMCODED measurements: compress, then error-code, then mask, then store in a memory; read back end to end.
//! Run: `cargo run --release --example sdmcoded_measure <part> [args]`, part one of
//!   ratio | passages <dir> | fragility | bsc | grid <hop|sdm> | controls
//! Predictions were sealed in the SETTLE campaign ledger before this instrument was run. Everything is
//! seeded; the only timings printed are wall-clock totals, stamped by the caller with the machine load.

#![allow(clippy::needless_range_loop)] // index loops mirror the equations they measure

use settle::coded::{ac_encode, bytes_to_bits, crc16, CodeKind, Codec, Comp, English, Pipeline, TEST, TRAIN};
use settle::memory::{code, seed_of, shake, store_pattern};
use settle::model::{Model, State};
use settle::rng::Rng;
use settle::sdm::{radius_for, View};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};

const N: usize = 512;
const LENGTHS: [usize; 10] = [16, 24, 32, 40, 48, 61, 80, 100, 120, 140];
const DAMAGES: [f64; 3] = [0.1, 0.2, 0.3];
const TRIALS: usize = 20;
const HOP_LOADS: [usize; 4] = [10, 30, 50, 65];
const SDM_LOADS: [usize; 4] = [5, 10, 20, 40];
const SDM_LOCATIONS: usize = 2000;

fn combos() -> Vec<(Comp, CodeKind)> {
    let mut v = Vec::new();
    for c in [Comp::None, Comp::Ac] {
        for k in [CodeKind::None, CodeKind::Ldpc(750), CodeKind::Hamming74, CodeKind::Ldpc(500), CodeKind::Rep3] {
            v.push((c, k));
        }
    }
    v
}

/// Passage `t` of length `len`: starts at the first word boundary at or after t * 500 in the test text.
fn passage(t: usize, len: usize) -> Vec<u8> {
    let b = TEST.as_bytes();
    let mut s = t * 500;
    while s > 0 && b[s - 1] != b' ' {
        s += 1;
    }
    b[s..s + len].to_vec()
}

fn ov(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(x, y)| x * y).sum::<f64>() / a.len() as f64
}

// ------------------------------------------------------------------------------------------------ ratio

fn part_ratio() {
    let m = English::trained();
    println!("# compression on held-out Doyle passages (20 per length); model trained on Austen only");
    println!("len\tnone_bpc\tac_bpc\tac_ideal_bpc\tlz_bpc\tac_ratio\tlz_ratio");
    for len in [16usize, 32, 61, 100, 140, 200] {
        let (mut ac, mut ideal, mut lz) = (0.0, 0.0, 0.0);
        for t in 0..TRIALS {
            let p = passage(t, len);
            ac += Comp::Ac.compress(&p, m).len() as f64;
            ideal += m.ideal_bits(&p);
            lz += Comp::Lz.compress(&p, m).len() as f64;
        }
        let d = (TRIALS * len) as f64;
        println!("{}\t8.000\t{:.3}\t{:.3}\t{:.3}\t{:.3}\t{:.3}", len, ac / d, ideal / d, lz / d, ac / d / 8.0, lz / d / 8.0);
    }
    // in-sample reference: the same model on its own training text (not a test number)
    let tr = &TRAIN.as_bytes()[..20000];
    println!("# in-sample reference, first 20,000 training characters: ac {:.3} bits per char", ac_encode(tr, m).len() as f64 / tr.len() as f64);
}

fn part_passages(dir: &str) {
    std::fs::create_dir_all(dir).unwrap();
    for len in [16usize, 32, 61, 100, 140, 200] {
        for t in 0..TRIALS {
            std::fs::write(format!("{}/p{:03}_{:02}.txt", dir, len, t), passage(t, len)).unwrap();
        }
    }
}

// ------------------------------------------------------------------------------------------------ fragility

fn part_fragility() {
    let m = English::trained();
    println!("# one flipped bit inside the compressed payload, decoded with the true length (20 passages x 50 flips)");
    println!("comp\tlen\tpayload_bits\tmean_chars_wrong\tmean_share_wrong\tshare_after_first_wrong");
    for c in [Comp::None, Comp::Ac, Comp::Lz] {
        for len in [61usize, 120] {
            let (mut wrong, mut share, mut after, mut bits, mut cnt) = (0.0, 0.0, 0.0, 0.0, 0.0);
            for t in 0..TRIALS {
                let p = passage(t, len);
                let enc = c.compress(&p, m);
                let mut r = Rng::new(seed_of(&format!("frag:{}:{}:{}", c.name(), len, t)));
                for _ in 0..50 {
                    let mut e = enc.clone();
                    let k = r.below(e.len());
                    e[k] ^= 1;
                    let out = c.decompress(&e, len, m).unwrap_or_default();
                    let w: Vec<bool> = (0..len).map(|i| out.get(i) != Some(&p[i])).collect();
                    let nw = w.iter().filter(|&&x| x).count() as f64;
                    wrong += nw;
                    share += nw / len as f64;
                    if let Some(f) = w.iter().position(|&x| x) {
                        after += w[f..].iter().filter(|&&x| x).count() as f64 / (len - f) as f64;
                    } else {
                        after += 0.0;
                    }
                    bits += enc.len() as f64;
                    cnt += 1.0;
                }
            }
            println!("{}\t{}\t{:.1}\t{:.2}\t{:.4}\t{:.4}", c.name(), len, bits / cnt, wrong / cnt, share / cnt, after / cnt);
        }
    }
}

// ------------------------------------------------------------------------------------------------ bsc

/// Exact post-decoding information-bit error of Hamming(7,4) on a binary symmetric channel, by enumeration.
fn hamming_exact(p: f64) -> (f64, f64) {
    let c = Codec::new(CodeKind::Hamming74, 7, 1);
    let cw = c.encode(&[0, 0, 0, 0]);
    let (mut bit, mut block) = (0.0, 0.0);
    for e in 0u32..128 {
        let w = e.count_ones() as i32;
        let pr = p.powi(w) * (1.0 - p).powi(7 - w);
        let y: Vec<u8> = (0..7).map(|i| cw[i] ^ ((e >> i) & 1) as u8).collect();
        let d = c.decode(&y).unwrap();
        let errs = d.iter().filter(|&&b| b == 1).count();
        bit += pr * errs as f64 / 4.0;
        if errs > 0 {
            block += pr;
        }
    }
    (bit, block)
}

fn part_bsc() {
    println!("# binary symmetric channel, n = 512 per block, 2000 blocks per cell (1000 for ldpc)");
    println!("code\trate\tp\tinfo_bit_error\ttextbook\tblock_error\tdecoder_says_failed\tmiscorrected_blocks");
    for kind in [CodeKind::None, CodeKind::Ldpc(750), CodeKind::Hamming74, CodeKind::Ldpc(500), CodeKind::Rep3] {
        let c = Codec::new(kind, N, 1);
        for p in [0.005, 0.01, 0.02, 0.03, 0.05, 0.08, 0.1] {
            let blocks = if matches!(kind, CodeKind::Ldpc(_)) { 1000 } else { 2000 };
            let mut r = Rng::new(seed_of(&format!("bsc:{}:{}", kind.name(), p)));
            let (mut be, mut blk, mut fail, mut mis) = (0usize, 0usize, 0usize, 0usize);
            for _ in 0..blocks {
                let info: Vec<u8> = (0..c.k).map(|_| (r.unit() < 0.5) as u8).collect();
                let y: Vec<u8> = c.encode(&info).iter().map(|&b| b ^ (r.unit() < p) as u8).collect();
                let mut y = y;
                y.resize(N, 0);
                let got = match c.decode(&y) {
                    Some(g) => g,
                    None => {
                        fail += 1;
                        c.decode_or_raw(&y)
                    }
                };
                let e = got.iter().zip(&info).filter(|(a, b)| a != b).count();
                be += e;
                if e > 0 {
                    blk += 1;
                    if c.decode(&y).is_some() {
                        mis += 1;
                    }
                }
            }
            let textbook = match kind {
                CodeKind::None => format!("{:.3e}", p),
                CodeKind::Rep3 => format!("{:.3e}", 3.0 * p * p - 2.0 * p * p * p),
                CodeKind::Hamming74 => format!("{:.3e}", hamming_exact(p).0),
                CodeKind::Ldpc(_) => "-".into(),
            };
            println!(
                "{}\t{:.3}\t{}\t{:.3e}\t{}\t{:.4}\t{:.4}\t{}",
                kind.name(),
                c.rate(),
                p,
                be as f64 / (blocks * c.k) as f64,
                textbook,
                blk as f64 / blocks as f64,
                fail as f64 / blocks as f64,
                mis
            );
        }
    }
}

// ------------------------------------------------------------------------------------------------ memories

enum Mem {
    Hop(Model),
    Sdm(Model, View),
}

impl Mem {
    fn new(sdm: bool, trial: usize) -> Mem {
        let mut m = Model::default();
        if sdm {
            let v = View::declare(&mut m, "s", N, SDM_LOCATIONS, radius_for(N, 0.02), 100 + trial as u64, 1.0);
            Mem::Sdm(m, v)
        } else {
            for i in 0..N {
                m.add(&format!("m_{}", i));
            }
            Mem::Hop(m)
        }
    }
    fn store(&mut self, p: &[f64]) {
        match self {
            Mem::Hop(m) => store_pattern(m, 0, N, 1.0, p),
            Mem::Sdm(m, v) => {
                v.write(m, p);
            }
        }
    }
    /// Exactly undo `store(p)` (every increment is a multiple of a power of two, so the sums are exact).
    fn unstore(&mut self, p: &[f64]) {
        match self {
            Mem::Hop(m) => {
                let w = 1.0 / N as f64;
                for i in 0..N {
                    for e in m.adj[i].iter_mut() {
                        e.1 -= w * p[i] * p[e.0];
                    }
                }
            }
            Mem::Sdm(m, v) => {
                let half = v.gain() / 2.0;
                for i in v.awake(p) {
                    for j in 0..N {
                        let d = half * p[j];
                        m.adj[v.loc + i][j].1 -= d;
                        m.adj[v.data + j][i].1 -= d;
                        m.h[v.data + j] -= d;
                    }
                }
            }
        }
    }
    fn recall(&self, cue: &[f64], seed: u64) -> Vec<f64> {
        match self {
            Mem::Hop(m) => {
                let mut st = State::new(seed);
                shake(m, &mut st, 0, cue, 30, 0.1)
            }
            Mem::Sdm(m, v) => v.read_addresses(m, cue, 10).0,
        }
    }
    fn snapshot(&self) -> Vec<f64> {
        match self {
            Mem::Hop(m) | Mem::Sdm(m, _) => {
                let mut s: Vec<f64> = m.h.clone();
                for row in &m.adj {
                    s.extend(row.iter().map(|e| e.1));
                }
                s
            }
        }
    }
}

#[derive(Default, Clone)]
struct Cell {
    n: usize,
    fits: bool,
    exact: usize,
    refused: usize,
    silent: usize,
    found: usize,
    residual: usize,
    char_right: f64,
}

type Key = (usize, u8, u8, String, String, usize); // load, damage%, mode, comp, code, len

fn run_trial(sdm: bool, t: usize, loads: &[usize], out: &mut BTreeMap<Key, Cell>) {
    let model = English::trained();
    let mut mem = Mem::new(sdm, t);
    let mut stored_others = 0;
    let pipes: Vec<(Comp, CodeKind, Pipeline)> =
        combos().into_iter().map(|(c, k)| (c, k, Pipeline::new(&format!("note{}", t), N, c, k, 1))).collect();
    for &load in loads {
        while stored_others < load {
            mem.store(&code(&format!("other-{}-{}", t, stored_others), N));
            stored_others += 1;
        }
        let check = if t == 0 { Some(mem.snapshot()) } else { None };
        for (c, k, pl) in &pipes {
            for &len in &LENGTHS {
                let text = passage(t, len);
                let pat = pl.pattern(&text, model);
                for &d in &DAMAGES {
                    for mode in [0u8, 1u8] {
                        let key = (load, (d * 100.0).round() as u8, mode, c.name().to_string(), k.name(), len);
                        let cell = out.entry(key).or_default();
                        cell.n += 1;
                        let Ok(p) = &pat else { continue };
                        cell.fits = true;
                        mem.store(p);
                        let s = seed_of(&format!("cue:{}:{}:{}:{}:{}:{}:{}:{}", sdm, t, load, c.name(), k.name(), len, d, mode));
                        let mut r = Rng::new(s);
                        let base = if mode == 0 { pl.name_read_address() } else { p.clone() };
                        let cue: Vec<f64> = base.iter().map(|&b| if r.unit() < d { -b } else { b }).collect();
                        let got = mem.recall(&cue, s ^ 0x9e37);
                        mem.unstore(p);
                        let o = ov(&got, p);
                        if o.abs() >= 0.9 {
                            cell.found += 1;
                            cell.residual += got.iter().zip(p).filter(|(a, b)| (**a * o.signum()) != **b).count();
                        }
                        match pl.decode(&got, model) {
                            Ok((txt, _)) if txt == text => cell.exact += 1,
                            Ok(_) => cell.silent += 1,
                            Err(_) => cell.refused += 1,
                        }
                        let sign = if o >= 0.0 { 1.0 } else { -1.0 };
                        let raw = pl.raw_read(&got, sign, len, model);
                        cell.char_right += (0..len).filter(|&i| raw.get(i) == Some(&text[i])).count() as f64 / len as f64;
                    }
                }
            }
        }
        if let Some(before) = check {
            assert_eq!(before, mem.snapshot(), "store then unstore did not restore the memory exactly");
        }
    }
}

fn c90(rows: &BTreeMap<Key, Cell>, load: usize, dmg: u8, mode: u8, comp: &str, code: &str) -> usize {
    let mut best = 0;
    for &len in &LENGTHS {
        let c = &rows[&(load, dmg, mode, comp.to_string(), code.to_string(), len)];
        if !c.fits || (c.exact as f64) < 0.9 * c.n as f64 {
            break;
        }
        best = len;
    }
    best
}

fn part_grid(which: &str) {
    let sdm = which == "sdm";
    let loads: Vec<usize> = if sdm { SDM_LOADS.to_vec() } else { HOP_LOADS.to_vec() };
    let next = AtomicUsize::new(0);
    let threads = std::thread::available_parallelism().map(|x| x.get()).unwrap_or(4).min(TRIALS);
    let start = std::time::Instant::now();
    let parts: Vec<BTreeMap<Key, Cell>> = std::thread::scope(|sc| {
        let hs: Vec<_> = (0..threads)
            .map(|_| {
                sc.spawn(|| {
                    let mut out = BTreeMap::new();
                    loop {
                        let t = next.fetch_add(1, Ordering::SeqCst);
                        if t >= TRIALS {
                            break;
                        }
                        run_trial(sdm, t, &loads, &mut out);
                        eprintln!("trial {} done at {:.0}s", t, start.elapsed().as_secs_f64());
                    }
                    out
                })
            })
            .collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let mut rows: BTreeMap<Key, Cell> = BTreeMap::new();
    for p in parts {
        for (k, c) in p {
            let e = rows.entry(k).or_default();
            e.n += c.n;
            e.fits |= c.fits;
            e.exact += c.exact;
            e.refused += c.refused;
            e.silent += c.silent;
            e.found += c.found;
            e.residual += c.residual;
            e.char_right += c.char_right;
        }
    }
    let sys = if sdm { format!("SDM {} things, {} locations, radius {}, address read 10 rounds", N, SDM_LOCATIONS, radius_for(N, 0.02)) } else { format!("Hopfield {} things, 30 sweeps at temperature 0.1", N) };
    println!("# grid: {}; {} trials per cell; wall {:.0}s", sys, TRIALS, start.elapsed().as_secs_f64());
    println!("load\tdamage\tcue\tcomp\tcode\tlen\tfits\texact\trefused\tsilent_wrong\tfound\tresidual_bits_per_found\tchar_right");
    let mut silent = 0;
    let mut total = 0;
    for (k, c) in &rows {
        total += c.n;
        silent += c.silent;
        let per = if c.found > 0 { c.residual as f64 / c.found as f64 } else { f64::NAN };
        let cr = if c.fits { c.char_right / c.n as f64 } else { f64::NAN };
        println!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{:.2}\t{:.3}",
            k.0,
            k.1,
            if k.2 == 0 { "name" } else { "all" },
            k.3,
            k.4,
            k.5,
            c.fits as u8,
            c.exact,
            c.refused,
            c.silent,
            c.found,
            per,
            cr
        );
    }
    println!("# silent wrong texts (check passed, text wrong): {} of {} recalls", silent, total);
    println!("# C90 = largest length (bytes) before the first length with exact recovery under 18 of 20; bits per thing = 8 C90 / {}", N);
    for mode in [0u8, 1u8] {
        println!("## cue knows {}", if mode == 0 { "only the name" } else { "the whole pattern" });
        let head: Vec<String> = combos().iter().map(|(c, k)| format!("{}+{}", c.name(), k.name())).collect();
        println!("load\tdamage\t{}", head.join("\t"));
        for &load in &loads {
            for &d in &DAMAGES {
                let dm = (d * 100.0).round() as u8;
                let vals: Vec<String> = combos().iter().map(|(c, k)| c90(&rows, load, dm, mode, c.name(), &k.name()).to_string()).collect();
                println!("{}\t{}\t{}", load, dm, vals.join("\t"));
            }
        }
    }
}

// ------------------------------------------------------------------------------------------------ controls

fn part_controls() {
    let model = English::trained();
    let bad_dict = English::swapped(TRAIN.as_bytes(), b'e', b't');
    for sdm in [false, true] {
        let load = if sdm { 10 } else { 30 };
        let (mut ghost_ok, mut ghost_n, mut noise_ok, mut noise_n) = (0, 0, 0, 0);
        let mut cb: BTreeMap<&str, (usize, usize, usize)> = BTreeMap::new(); // exact, refused, silent
        for t in 0..TRIALS {
            let mut mem = Mem::new(sdm, t);
            for k in 0..load {
                mem.store(&code(&format!("other-{}-{}", t, k), N));
            }
            let text = passage(t, 40);
            let pl = Pipeline::new(&format!("note{}", t), N, Comp::Ac, CodeKind::Ldpc(500), 1);
            let p = pl.pattern(&text, model).unwrap();
            mem.store(&p);
            // never-saved read-addresses, every compressor and code
            for (i, (c, k)) in combos().into_iter().enumerate() {
                let g = Pipeline::new(&format!("ghost{}-{}", t, i), N, c, k, 1);
                let got = mem.recall(&g.name_read_address(), seed_of(&format!("ghost:{}:{}:{}", sdm, t, i)));
                ghost_n += 1;
                if g.decode(&got, model).is_ok() {
                    ghost_ok += 1;
                }
                let mut r = Rng::new(seed_of(&format!("noise:{}:{}:{}", sdm, t, i)));
                let noise: Vec<f64> = (0..N).map(|_| if r.unit() < 0.5 { -1.0 } else { 1.0 }).collect();
                let got = mem.recall(&noise, seed_of(&format!("noise2:{}:{}:{}", sdm, t, i)));
                noise_n += 1;
                if g.decode(&got, model).is_ok() {
                    noise_ok += 1;
                }
            }
            // corrupted codebooks, recalling the real note from a 10%-noisy full read-address
            let mut r = Rng::new(seed_of(&format!("cb:{}:{}", sdm, t)));
            let cue: Vec<f64> = p.iter().map(|&b| if r.unit() < 0.1 { -b } else { b }).collect();
            let got = mem.recall(&cue, seed_of(&format!("cb2:{}:{}", sdm, t)));
            let wrong_ldpc = Pipeline::new(&format!("note{}", t), N, Comp::Ac, CodeKind::Ldpc(500), 2);
            let wrong_mask = Pipeline::new(&format!("other-note{}", t), N, Comp::Ac, CodeKind::Ldpc(500), 1);
            let tally = |cb: &mut BTreeMap<&str, (usize, usize, usize)>, k: &'static str, res: Result<(Vec<u8>, f64), String>| {
                let e = cb.entry(k).or_default();
                match res {
                    Ok((x, _)) if x == text => e.0 += 1,
                    Ok(_) => e.2 += 1,
                    Err(_) => e.1 += 1,
                }
            };
            tally(&mut cb, "right codebook (vacuity control)", pl.decode(&got, model));
            tally(&mut cb, "LDPC matrix from another seed", wrong_ldpc.decode(&got, model));
            tally(&mut cb, "mask of another name", wrong_mask.decode(&got, model));
            tally(&mut cb, "dictionary with e and t swapped", pl.decode(&got, &bad_dict));
        }
        let sys = if sdm { "SDM (load 10)" } else { "Hopfield (load 30)" };
        println!("# {}: never-saved cue accepted {} of {}; noise start decoded under a never-saved name accepted {} of {}", sys, ghost_ok, ghost_n, noise_ok, noise_n);
        for (k, (e, rf, s)) in cb {
            println!("# {}: {}: exact {} refused {} silent wrong {} (of {})", sys, k, e, rf, s, TRIALS);
        }
    }
    // the check itself: one flipped bit anywhere in a 40-byte text changes the CRC (always, for 16-bit CRCs)
    let t = passage(0, 40);
    let bits = bytes_to_bits(&t);
    let base = crc16(&t);
    let mut changed = 0;
    for i in 0..bits.len() {
        let mut u = t.clone();
        u[i / 8] ^= 0x80 >> (i % 8);
        changed += (crc16(&u) != base) as usize;
    }
    println!("# crc16 detects {} of {} single-bit flips in a 40-byte text", changed, bits.len());
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(|s| s.as_str()).unwrap_or("ratio") {
        "ratio" => part_ratio(),
        "passages" => part_passages(args.get(2).map(|s| s.as_str()).unwrap_or("passages")),
        "fragility" => part_fragility(),
        "bsc" => part_bsc(),
        "grid" => part_grid(args.get(2).map(|s| s.as_str()).unwrap_or("hop")),
        "controls" => part_controls(),
        p => eprintln!("unknown part {} (ratio | passages <dir> | fragility | bsc | grid <hop|sdm> | controls)", p),
    }
}
