//! SOFTSDM: Kanerva's sparse distributed memory built inside the settle machine, with a SOFT cut-off.
//!
//! ```text
//! model :mind do
//!   softsdm :s, word-size: 256, hard-locations: 2000, activation-probability: 0.05, softness: 0.3
//!   s.write :cat                                  # a random pattern named :cat
//!   s.write :note, "meet at the harbour at nine"  # text, masked by the name so it looks random
//! end
//! run :mind do
//!   s.read read-address: :cat, address-noise: 0.3, rounds: 3      # hold a scrambled cat, settle, feed the answer back
//!   s.read                                        # start from pure noise
//!   s.attend read-address: :cat, address-noise: 0.3               # the attention limit, computed outside the sampler
//! end
//! ```
//!
//! Three kinds of thing. ADDRESS things `s_addr_j` hold the read-address during a read. HIDDEN LOCATION things
//! `s_loc_m` are p-bits, each with a fixed random address `x_m`; its pulls from the address things are
//! `x_mj / (4w)` and its lean is `(2t - n) / (4w)`, so its input is `(t - d) / (2w)` where `d` is the
//! Hamming distance from the read-address to `x_m`. A hard location is activated with probability `1 / (1 + exp(-(t - d) / w))`.
//! DATA things `s_data_j` are free on a read; they are pulled by the hard locations that fire.
//!
//! Softness `w` is in units of the spread of a random read-address's distance, `sqrt(n) / 2` bits. At softness 0
//! a hard location is activated exactly when `d <= r` (classic hard SDM); as softness rises the edge of the Hamming
//! ball blurs. The threshold `t` is re-fitted for each softness so the expected number of activated hard locations
//! for a random read-address stays the number the hard activation radius gives: the dial changes the SHAPE of the cut-off,
//! never how many hard locations fire on average.
//!
//! Write (Hebbian): hold the address at the pattern, let the hard locations fire `write_samples` times, and add
//! `f_m * p` to hard location `m`'s counter row, where `f_m` is the fraction of those settles it was activated in.
//! The bit-counters live in the pulls between hard location and data things (pull `g J / 4`, data lean
//! `g * sum_m J / 4`, with `g = gain / (expected activated count)`), so the data field is `g * sum over activated
//! m of J_m`.
//!
//! Read, `mode: :pass` (the default, faithful to Kanerva): hard locations fire from the address alone, data
//! things settle to the vote of the activated hard locations, repeated `samples` times; each data bit is the
//! majority over the samples. Read, `mode: :settle`: the whole machine is Gibbs-sampled with the address
//! held, so the data things ALSO pull back on the hard locations through the same symmetric pulls. That
//! feedback is the honest difference between SDM and a settle machine: classic SDM decodes with the
//! address matrix and reads with the counter matrix, and a read is a one-way pass. `rounds` feeds each
//! read-out back as the next read-address.
//!
//! `attend` computes, outside the sampler, the mean-field read of this machine (infinitely many samples),
//! the kernel read for infinitely many hard locations, and the softmax-attention read with the inverse
//! temperature fitted to that kernel.

//!
//! The machine, the activated step, the calibration, the kernel and the attention reads are KANERVA's
//! (`kanerva::soft`), with the codes from `kanerva::codes` and the binomial tables from
//! `kanerva::theory`, re-exported here under their old names. This file keeps the things, the pulls and
//! the statements.

use crate::ext::{Claim, Ctx, Ext};
use crate::lex::{err, kw, kwargs, num, only, text, SettleError, Tok};
use crate::memory::code;
use crate::model::{Model, State};
use crate::rng::Rng;
use kanerva::codes::bits_text;

pub use kanerva::codes::{with_address_noise, overlap, pattern};
pub use kanerva::soft::{attention_read, calibrate, phi, sign_of, Attn, Machine};
pub use kanerva::theory::{binom_log_pmf, hard_radius};

pub struct SoftSdm;

// ---------------------------------------------------------------------------------------------------------
// The model side: things, pulls, and the note that carries the machine between statements.

struct Stored {
    start: usize,
    mach: Machine,
    stored: Vec<(String, Option<String>)>,
}

fn key(name: &str) -> String {
    format!("softsdm:{}", name)
}

fn load(m: &Model, name: &str) -> Option<Stored> {
    let (nums, words) = m.notes.get(&key(name))?;
    let (start, n, lm, fire, soft, gain, seed, ws) =
        (nums[0] as usize, nums[1] as usize, nums[2] as usize, nums[3], nums[4], nums[5], nums[6] as u64, nums[7] as usize);
    let mut mach = Machine::new(n, lm, fire, soft, gain, seed, ws);
    mach.j.copy_from_slice(&nums[8..8 + lm * n]);
    let stored = words.chunks(2).map(|w| (w[0].clone(), if w[1].is_empty() { None } else { Some(w[1][1..].to_string()) })).collect();
    Some(Stored { start, mach, stored })
}

fn keep(m: &mut Model, name: &str, s: &Stored) {
    let mc = &s.mach;
    let mut nums = vec![s.start as f64, mc.n as f64, mc.m as f64, mc.fire, mc.softness, mc.gain, mc.seed as f64, mc.write_samples as f64];
    nums.extend_from_slice(&mc.j);
    let words = s.stored.iter().flat_map(|(n, t)| [n.clone(), t.as_ref().map(|t| format!("={}", t)).unwrap_or_default()]).collect();
    m.notes.insert(key(name), (nums, words));
}

/// Index layout: address things start..start+n, hard locations next m, data next n.
fn idx(s: &Stored) -> (usize, usize, usize) {
    (s.start, s.start + s.mach.n, s.start + s.mach.n + s.mach.m)
}

fn declare(m: &mut Model, rest: &[Tok], name: &str, ln: usize) -> Result<(), SettleError> {
    if m.notes.contains_key(&key(name)) {
        return err(ln, format!("softsdm :{} is already declared", name));
    }
    let kv = kwargs(rest, ln)?;
    only(&kv, &["word-size", "hard-locations", "activation-probability", "softness", "gain", "seed", "write_samples"], "softsdm", ln)?;
    let get = |k: &str, d: f64| kw(&kv, k).map(|v| num(v, ln)).transpose().map(|x| x.unwrap_or(d));
    let n = get("word-size", 256.0)? as usize;
    let lm = get("hard-locations", 2000.0)? as usize;
    let fire = get("activation-probability", 0.05)?;
    let soft = get("softness", 0.3)?;
    let gain = get("gain", 64.0)?;
    let seed = get("seed", 1.0)? as u64;
    let ws = get("write_samples", 16.0)? as usize;
    if !(8..=4096).contains(&n) {
        return err(ln, "softsdm size must be between 8 and 4096");
    }
    if lm < 1 || lm * n > 4_000_000 {
        return err(ln, "softsdm needs at least 1 hard location and at most 4,000,000 location-bits (hard-locations x word-size)");
    }
    if !(fire > 0.0 && fire < 1.0) {
        return err(ln, "activation-probability is a fraction of hard locations, above 0 and below 1");
    }
    if soft < 0.0 || gain <= 0.0 {
        return err(ln, "softness must be at least 0 and gain above 0");
    }
    for nm in [format!("{}_addr_0", name), format!("{}_loc_0", name), format!("{}_data_0", name)] {
        if m.idx.contains_key(&nm) {
            return err(ln, format!("a thing :{} already exists; pick another softsdm name", nm));
        }
    }
    let mach = Machine::new(n, lm, fire, soft, gain, seed, ws);
    let start = m.len();
    for i in 0..n {
        m.add(&format!("{}_addr_{}", name, i));
    }
    for l in 0..lm {
        m.add(&format!("{}_loc_{}", name, l));
    }
    for i in 0..n {
        m.add(&format!("{}_data_{}", name, i));
    }
    let s = Stored { start, mach, stored: Vec::new() };
    let (a0, l0, d0) = idx(&s);
    let w = s.mach.w_pull();
    // fixed layout: each hard location's neighbours are its n address things, then its n data things
    for l in 0..lm {
        m.h[l0 + l] = (2.0 * s.mach.t - n as f64) / (4.0 * w);
        for j in 0..n {
            let x = s.mach.addr[l * n + j] / (4.0 * w);
            m.adj[l0 + l].push((a0 + j, x));
            m.adj[a0 + j].push((l0 + l, x));
        }
    }
    for l in 0..lm {
        for k in 0..n {
            m.adj[l0 + l].push((d0 + k, 0.0));
            m.adj[d0 + k].push((l0 + l, 0.0));
        }
    }
    keep(m, name, &s);
    Ok(())
}

/// Add a write's counter change to the model's pulls and data leans, at the fixed layout positions.
fn apply(m: &mut Model, s: &Stored, touched: &[(usize, f64)], p: &[f64]) {
    let (_, l0, d0) = idx(s);
    let n = s.mach.n;
    let q = s.mach.g() / 4.0;
    for &(l, f) in touched {
        for k in 0..n {
            let dj = q * f * p[k];
            let e = &mut m.adj[l0 + l][n + k];
            debug_assert_eq!(e.0, d0 + k);
            e.1 += dj;
            let e2 = &mut m.adj[d0 + k][l];
            debug_assert_eq!(e2.0, l0 + l);
            e2.1 += dj;
            m.h[d0 + k] += dj;
        }
    }
}

fn write(m: &mut Model, rng: &mut Rng, name: &str, what: &str, saved: Option<String>, ln: usize) -> Result<(), SettleError> {
    let mut s = load(m, name).unwrap();
    let n = s.mach.n;
    if let Some(t) = &saved {
        if t.len() * 8 > n {
            return err(ln, format!("{} bytes of text need {} bits; softsdm :{} has size {}", t.len(), t.len() * 8, name, n));
        }
    }
    if s.stored.iter().any(|(x, _)| x == what) {
        return err(ln, format!(":{} is already written in :{}", what, name));
    }
    let (_, l0, d0) = idx(&s);
    if m.adj[l0].len() != 2 * n || m.adj[d0].len() != s.mach.m {
        return err(ln, format!("the pulls of softsdm :{} were changed by hand; its layout is fixed", name));
    }
    let p = pattern(what, saved.as_deref(), n);
    let touched = s.mach.write(&p, rng);
    apply(m, &s, &touched, &p);
    s.stored.push((what.to_string(), saved));
    keep(m, name, &s);
    Ok(())
}

fn scores(got: &[f64], s: &Stored) -> Vec<(String, f64)> {
    let mut v: Vec<(String, f64)> =
        s.stored.iter().map(|(nm, t)| (nm.clone(), overlap(got, &pattern(nm, t.as_deref(), s.mach.n)))).collect();
    v.sort_by(|x, y| y.1.partial_cmp(&x.1).unwrap());
    v
}

fn verdict(sc: &[(String, f64)]) -> String {
    match sc.first() {
        Some((n, o)) if *o >= 0.9 => format!("-> :{}", n),
        Some((n, o)) => format!("-> nothing clear (closest :{} at {:+.2})", n, o),
        None => "-> nothing is written".to_string(),
    }
}

/// Build the read-address named by `read-address:` (noisy) or pure noise; returns the read-address and a description.
fn make_cue(s: &Stored, kv: &[(String, Tok)], rng: &mut Rng, ln: usize) -> Result<(Vec<f64>, String), SettleError> {
    let n = s.mach.n;
    let damage = kw(kv, "address-noise").map(|v| num(v, ln)).transpose()?.unwrap_or(0.3);
    if !(0.0..=1.0).contains(&damage) {
        return err(ln, "address-noise is a fraction between 0 and 1");
    }
    match kw(kv, "read-address") {
        Some(Tok::Sym(c)) => {
            let saved = s.stored.iter().find(|(x, _)| x == c).and_then(|(_, t)| t.clone());
            let p = pattern(c, saved.as_deref(), n);
            Ok((with_address_noise(&p, damage, rng), format!("read-address :{} with {:.0}% address-noise", c, 100.0 * damage)))
        }
        Some(_) => err(ln, "read-address: takes a symbol, like read-address: :cat"),
        None => Ok(((0..n).map(|_| if rng.unit() < 0.5 { -1.0 } else { 1.0 }).collect(), "pure noise".to_string())),
    }
}

fn text_line(got: &[f64], s: &Stored, sc: &[(String, f64)], ctx: &mut Ctx) {
    if let Some((nm, o)) = sc.first() {
        if *o >= 0.9 {
            if let Some((_, Some(t))) = s.stored.iter().find(|(x, _)| x == nm) {
                let mask = code(nm, s.mach.n);
                let bits: Vec<f64> = got.iter().zip(&mask).map(|(v, k)| v * k).collect();
                ctx.say(format!("  text: \"{}\"", bits_text(&bits, t.len())));
            }
        }
    }
}

fn read(m: &Model, st: &mut State, name: &str, rest: &[Tok], ln: usize, ctx: &mut Ctx) -> Result<(), SettleError> {
    let s = load(m, name).unwrap();
    let kv = kwargs(rest, ln)?;
    only(&kv, &["read-address", "address-noise", "rounds", "samples", "mode", "burn", "seed"], "read", ln)?;
    if let Some(v) = kw(&kv, "seed") {
        st.rng = Rng::new(num(v, ln)? as u64);
    }
    let rounds = kw(&kv, "rounds").map(|v| num(v, ln)).transpose()?.unwrap_or(3.0) as usize;
    let samples = kw(&kv, "samples").map(|v| num(v, ln)).transpose()?.unwrap_or(16.0) as usize;
    let burn = kw(&kv, "burn").map(|v| num(v, ln)).transpose()?.unwrap_or(10.0) as usize;
    let settle = match kw(&kv, "mode") {
        None => false,
        Some(Tok::Sym(x)) if x == "pass" => false,
        Some(Tok::Sym(x)) if x == "settle" => true,
        Some(_) => return err(ln, "mode: is :pass or :settle"),
    };
    let (mut cue, from) = make_cue(&s, &kv, &mut st.rng, ln)?;
    let mut trail = Vec::new();
    for _ in 0..rounds.max(1) {
        let (out, _) = if settle { s.mach.read_settle(&cue, burn, samples, &mut st.rng) } else { s.mach.read_pass(&cue, samples, &mut st.rng) };
        let sc = scores(&out, &s);
        trail.push(sc.first().map(|(_, o)| format!("{:+.2}", o)).unwrap_or_default());
        cue = out;
    }
    let sc = scores(&cue, &s);
    let top: Vec<String> = sc.iter().take(3).map(|(nm, o)| format!(":{} {:+.2}", nm, o)).collect();
    ctx.say(format!(
        "read :{} from {} ({}, softness {}, {} rounds, best overlap by round {}): {}  {}",
        name,
        from,
        if settle { "settle" } else { "pass" },
        s.mach.softness,
        rounds.max(1),
        trail.join(" "),
        top.join("  "),
        verdict(&sc)
    ));
    text_line(&cue, &s, &sc, ctx);
    let (a0, _, d0) = idx(&s);
    let mut last = vec![0.0; m.len()];
    last[a0..a0 + s.mach.n].copy_from_slice(&cue);
    last[d0..d0 + s.mach.n].copy_from_slice(&cue);
    st.last = last;
    Ok(())
}

fn attend(m: &Model, st: &mut State, name: &str, rest: &[Tok], ln: usize, ctx: &mut Ctx) -> Result<(), SettleError> {
    let s = load(m, name).unwrap();
    let kv = kwargs(rest, ln)?;
    only(&kv, &["read-address", "address-noise", "rounds", "seed"], "attend", ln)?;
    if let Some(v) = kw(&kv, "seed") {
        st.rng = Rng::new(num(v, ln)? as u64);
    }
    let rounds = kw(&kv, "rounds").map(|v| num(v, ln)).transpose()?.unwrap_or(3.0) as usize;
    let (cue0, from) = make_cue(&s, &kv, &mut st.rng, ln)?;
    let pats: Vec<Vec<f64>> = s.stored.iter().map(|(nm, t)| pattern(nm, t.as_deref(), s.mach.n)).collect();
    let kern = s.mach.kernel_inf();
    let beta = Machine::fit_beta(&kern);
    ctx.say(format!("attend :{} from {} (softness {}, fitted softmax inverse temperature {:.1} on cosine):", name, from, s.mach.softness, beta));
    for label in ["mean-field read of this machine", "kernel read, infinitely many locations", "softmax attention"] {
        let mut cue = cue0.clone();
        for _ in 0..rounds.max(1) {
            cue = match label {
                "mean-field read of this machine" => sign_of(&s.mach.mean_field(&cue), &cue),
                "kernel read, infinitely many locations" => attention_read(&cue, &pats, &Attn::Kernel(&kern)),
                _ => attention_read(&cue, &pats, &Attn::Softmax(beta)),
            };
        }
        let sc = scores(&cue, &s);
        let top: Vec<String> = sc.iter().take(2).map(|(nm, o)| format!(":{} {:+.2}", nm, o)).collect();
        ctx.say(format!("  {:<40} {}  {}", label, top.join("  "), verdict(&sc)));
    }
    Ok(())
}

impl Ext for SoftSdm {
    fn name(&self) -> &'static str {
        "softsdm"
    }

    fn statements(&self) -> &'static [&'static str] {
        &[
            "model: softsdm :s, word-size: 256, hard-locations: 2000, activation-probability: 0.05, softness: 0.3, gain: 64, seed: 1, write_samples: 16",
            "model: s.write :cat   /   s.write :note, \"some text\"",
            "run: s.write :cat   /   s.write :note, \"some text\"",
            "run: s.read read-address: :cat, address-noise: 0.3, rounds: 3, samples: 16, mode: :pass, seed: 1   (mode :settle adds burn: 10)",
            "run: s.attend read-address: :cat, address-noise: 0.3, rounds: 3, seed: 1",
        ]
    }

    fn model_stmt(&self, m: &mut Model, t: &[Tok], ln: usize, _ctx: &mut Ctx) -> Claim {
        match t {
            [Tok::Ident(k), Tok::Sym(name), rest @ ..] if k == "softsdm" => {
                let rest = if rest.first() == Some(&Tok::Comma) { &rest[1..] } else { rest };
                Some(declare(m, rest, name, ln))
            }
            _ => {
                let mut rng = Rng::new(0x5eed_5d31);
                self.write_stmt(m, &mut rng, t, ln)
            }
        }
    }

    fn run_stmt(&self, m: &mut Model, st: &mut State, t: &[Tok], ln: usize, ctx: &mut Ctx) -> Claim {
        match t {
            [Tok::Ident(name), Tok::Dot, Tok::Ident(v), rest @ ..] if v == "read" && m.notes.contains_key(&key(name)) => {
                Some(read(m, st, name, rest, ln, ctx))
            }
            [Tok::Ident(name), Tok::Dot, Tok::Ident(v), rest @ ..] if v == "attend" && m.notes.contains_key(&key(name)) => {
                Some(attend(m, st, name, rest, ln, ctx))
            }
            _ => self.write_stmt(m, &mut st.rng, t, ln),
        }
    }
}

impl SoftSdm {
    /// `s.write ...` is claimed only when `s` is a declared softsdm, so another family may use `write` too.
    fn write_stmt(&self, m: &mut Model, rng: &mut Rng, t: &[Tok], ln: usize) -> Claim {
        match t {
            [Tok::Ident(name), Tok::Dot, Tok::Ident(v), rest @ ..] if v == "write" && m.notes.contains_key(&key(name)) => match rest {
                [Tok::Sym(what)] => Some(write(m, rng, name, what, None, ln)),
                [Tok::Sym(what), Tok::Comma, s] => Some(text(s, ln).and_then(|txt| write(m, rng, name, what, Some(txt), ln))),
                _ => Some(err(ln, "write takes a symbol, and optionally text: s.write :note, \"some text\"")),
            },
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interp::Interp;

    fn run(src: &str) -> Vec<String> {
        Interp::default().exec(src).unwrap_or_else(|e| panic!("{}", e))
    }

    /// An independent hard SDM: integer Hamming distance, activation radius test, plain bit-counters, majority read.
    struct HardSdm {
        n: usize,
        addr: Vec<Vec<f64>>,
        r: usize,
        c: Vec<Vec<f64>>,
    }
    impl HardSdm {
        fn active(&self, a: &[f64]) -> Vec<usize> {
            (0..self.addr.len())
                .filter(|&l| self.addr[l].iter().zip(a).filter(|(x, y)| x != y).count() <= self.r)
                .collect()
        }
        fn write(&mut self, p: &[f64]) {
            for l in self.active(p) {
                for k in 0..self.n {
                    self.c[l][k] += p[k];
                }
            }
        }
        fn read(&self, a: &[f64]) -> Vec<f64> {
            let act = self.active(a);
            (0..self.n)
                .map(|k| {
                    let s: f64 = act.iter().map(|&l| self.c[l][k]).sum();
                    s.signum() * (s != 0.0) as u8 as f64
                })
                .collect()
        }
    }

    /// Bits where the hard read is decided (non-zero vote) must match; ties are a coin flip in the machine.
    fn same_where_decided(got: &[f64], hard: &[f64]) -> bool {
        got.iter().zip(hard).all(|(g, h)| *h == 0.0 || g == h)
    }

    fn hard_twin(mc: &Machine) -> HardSdm {
        HardSdm {
            n: mc.n,
            addr: (0..mc.m).map(|l| mc.addr[l * mc.n..(l + 1) * mc.n].to_vec()).collect(),
            r: mc.radius,
            c: vec![vec![0.0; mc.n]; mc.m],
        }
    }

    #[test]
    fn softness_zero_is_exactly_hard_sdm() {
        // the machine at softness 0, read with a huge data gain, must agree bit for bit with the independent
        // hard SDM on every read, including noisy and never-written read-addresses
        let mut mc = Machine::new(128, 600, 0.05, 0.0, 1e6, 3, 4);
        let mut hard = hard_twin(&mc);
        let mut rng = Rng::new(9);
        let pats: Vec<Vec<f64>> = (0..12).map(|i| code(&format!("p{}", i), 128)).collect();
        for p in &pats {
            mc.write(p, &mut rng);
            hard.write(p);
        }
        for (x, y) in mc.j.chunks(128).zip(&hard.c) {
            assert_eq!(x, &y[..], "counters differ");
        }
        for (i, p) in pats.iter().enumerate() {
            let cue = with_address_noise(p, 0.1 * (i % 4) as f64, &mut rng);
            let (got, _) = mc.read_pass(&cue, 4, &mut rng);
            let h = hard.read(&cue);
            assert!(same_where_decided(&got, &h), "pattern {}", i);
            assert!(h.iter().filter(|v| **v == 0.0).count() < 8, "too many ties to be a test");
        }
        let zebra = code("zebra", 128);
        assert!(same_where_decided(&mc.read_pass(&zebra, 4, &mut rng).0, &hard.read(&zebra)));
    }

    #[test]
    fn the_calibration_keeps_the_firing_count_fixed_across_softness() {
        for soft in [0.25, 0.5, 1.0, 2.0] {
            let mc = Machine::new(256, 4000, 0.05, soft, 8.0, 1, 0);
            let mut rng = Rng::new(2);
            let mut tot = 0.0;
            for _ in 0..20 {
                let cue: Vec<f64> = (0..256).map(|_| if rng.unit() < 0.5 { -1.0 } else { 1.0 }).collect();
                tot += mc.fire_probs(&cue).iter().sum::<f64>();
            }
            let mean = tot / 20.0 / 4000.0;
            assert!((mean - mc.frac).abs() < 0.006, "softness {} fires {} want {}", soft, mean, mc.frac);
        }
    }

    #[test]
    fn the_model_pulls_give_the_same_inputs_as_the_dense_sampler() {
        // the dense sampler must sample the Boltzmann distribution of the model's own things and pulls
        let src = "model :mind do\n  softsdm :s, word-size: 32, hard-locations: 50, activation-probability: 0.1, softness: 0.5, seed: 4\n  s.write :cat\n  s.write :dog\nend";
        let mut it = Interp::default();
        it.exec(src).unwrap();
        let m = &it.models["mind"];
        let s = load(m, "s").unwrap();
        let (a0, l0, d0) = idx(&s);
        let mut rng = Rng::new(5);
        let arr: Vec<f64> = (0..m.len()).map(|_| if rng.unit() < 0.5 { -1.0 } else { 1.0 }).collect();
        let cue = &arr[a0..a0 + 32];
        let z = &arr[d0..d0 + 32];
        let y = &arr[l0..l0 + 50];
        let u = s.mach.loc_input(cue);
        let q = s.mach.g() / 4.0;
        for l in 0..50 {
            let fb: f64 = (0..32).map(|k| s.mach.j[l * 32 + k] * z[k]).sum();
            let want = u[l] + q * fb;
            assert!((m.input(l0 + l, &arr) - want).abs() < 1e-9, "location {}", l);
        }
        for k in 0..32 {
            let want: f64 = q * (0..50).map(|l| s.mach.j[l * 32 + k] * (y[l] + 1.0)).sum::<f64>();
            assert!((m.input(d0 + k, &arr) - want).abs() < 1e-9, "data {}", k);
        }
    }

    #[test]
    fn damaged_cues_come_back_at_moderate_softness() {
        let out = run("model :mind do
  softsdm :s, word-size: 256, hard-locations: 2000, activation-probability: 0.05, softness: 0.25
  s.write :cat
  s.write :dog
  s.write :owl
end
run :mind do
  s.read read-address: :cat, address-noise: 0.2, seed: 1
  s.read read-address: :dog, address-noise: 0.2, seed: 2
  s.read read-address: :owl, address-noise: 0.2, seed: 3
end");
        for (line, want) in out.iter().zip(["-> :cat", "-> :dog", "-> :owl"]) {
            assert!(line.ends_with(want), "{}", line);
        }
    }

    #[test]
    fn settle_mode_at_softness_zero_reads_like_the_pass() {
        // at softness 0 the address pulls dwarf the data feedback, so the joint settle is the one-way pass
        let out = run("model :mind do
  softsdm :s, word-size: 256, hard-locations: 2000, softness: 0
  s.write :cat
  s.write :dog
  s.write :owl
end
run :mind do
  s.read read-address: :owl, address-noise: 0.2, seed: 3, mode: :settle
end");
        assert!(out[0].ends_with("-> :owl"), "{}", out[0]);
    }

    #[test]
    fn a_never_written_cue_is_not_recalled() {
        let out = run("model :mind do
  softsdm :s, word-size: 256, hard-locations: 2000, softness: 0.25
  s.write :cat
  s.write :dog
end
run :mind do
  s.read read-address: :zebra, address-noise: 0.0, seed: 4
end");
        assert!(!out[0].contains("-> :zebra") && !out[0].contains(":zebra +0.9"), "{}", out[0]);
    }

    #[test]
    fn shuffled_counters_recall_nothing() {
        // negative control: the same bit-counters dealt to the wrong hard locations carry no memory
        let mut mc = Machine::new(256, 2000, 0.05, 0.25, 64.0, 1, 16);
        let mut rng = Rng::new(3);
        let pats: Vec<Vec<f64>> = (0..5).map(|i| code(&format!("q{}", i), 256)).collect();
        for p in &pats {
            mc.write(p, &mut rng);
        }
        let mut good = 0;
        for p in &pats {
            if overlap(&mc.recall(&with_address_noise(p, 0.15, &mut rng), 3, 16, false, 0, &mut rng), p) > 0.95 {
                good += 1;
            }
        }
        assert_eq!(good, 5, "the unshuffled machine must recall all five (vacuity control)");
        let mut rows: Vec<Vec<f64>> = mc.j.chunks(256).map(|c| c.to_vec()).collect();
        for i in (1..rows.len()).rev() {
            let r = rng.below(i + 1);
            rows.swap(i, r);
        }
        mc.j = rows.concat();
        for p in &pats {
            let o = overlap(&mc.recall(&with_address_noise(p, 0.15, &mut rng), 3, 16, false, 0, &mut rng), p);
            assert!(o < 0.9, "shuffled machine recalled a pattern at {}", o);
        }
    }

    #[test]
    fn saved_text_comes_back_letter_for_letter() {
        let out = run("model :mind do
  softsdm :s, word-size: 256, hard-locations: 2000, softness: 0.3
  s.write :cat
  s.write :note, \"meet at the harbour\"
end
run :mind do
  s.read read-address: :note, address-noise: 0.25, seed: 7
end");
        assert_eq!(out[1], "  text: \"meet at the harbour\"", "{:?}", out);
    }

    #[test]
    fn write_and_read_only_claim_names_that_are_softsdms() {
        // another family may own `x.write`; for a name that is not a softsdm this family must not claim it
        let e = Interp::default().exec("model :mind do\n  thing :a\n  a.write :cat\nend").unwrap_err();
        // with the sdm family registered, that family now names the line; either way softsdm stays out of it
        assert!(!e.0.contains("softsdm"), "{}", e.0);
        assert!(e.0.contains("no statement family knows this line") || e.0.contains("no sdm :a"), "{}", e.0);
    }

    #[test]
    fn the_kernel_falls_with_distance_and_softness_flattens_it() {
        let hard = Machine::new(64, 10, 0.05, 0.0, 8.0, 1, 0).kernel_inf();
        let soft = Machine::new(64, 10, 0.05, 2.0, 8.0, 1, 0).kernel_inf();
        for d in 1..20 {
            assert!(hard[d] <= hard[d - 1] + 1e-15 && soft[d] <= soft[d - 1] + 1e-15);
        }
        let (bh, bs) = (Machine::fit_beta(&hard), Machine::fit_beta(&soft));
        assert!(bs < bh, "softness should lower the fitted inverse temperature: hard {} soft {}", bh, bs);
    }

    #[test]
    fn the_attend_statement_prints_three_reads() {
        let out = run("model :mind do
  softsdm :s, word-size: 128, hard-locations: 500, softness: 1
  s.write :cat
  s.write :dog
end
run :mind do
  s.attend read-address: :cat, address-noise: 0.2, seed: 1
end");
        assert_eq!(out.len(), 4, "{:?}", out);
        assert!(out[1..].iter().all(|l| l.ends_with("-> :cat")), "{:?}", out);
    }
}

/// The Hopfield baseline, built fast but identical to `memory.rs` (same things, same pull values in the same
/// order, same recall steps), so it can run at 2,512 things. A test proves the equivalence against memory.rs.
pub mod hopfield {
    use crate::model::{Model, State};
    use crate::rng::Rng;

    /// Upper-triangle Hebbian sums, accumulated pattern by pattern exactly as `memory.rs` adds them.
    pub struct Weights {
        pub n: usize,
        pub w: Vec<f64>,
        pub count: usize,
    }

    impl Weights {
        pub fn new(n: usize) -> Self {
            Weights { n, w: vec![0.0; n * n], count: 0 }
        }
        pub fn add(&mut self, p: &[f64]) {
            let w = 1.0 / self.n as f64;
            for i in 0..self.n {
                for k in (i + 1)..self.n {
                    let v = w * p[i] * p[k];
                    if self.count == 0 {
                        self.w[i * self.n + k] = v;
                    } else {
                        self.w[i * self.n + k] += v;
                    }
                }
            }
            self.count += 1;
        }
        /// The model memory.rs would have built: things m_0.., neighbours in ascending order.
        pub fn model(&self, name: &str) -> Model {
            let mut m = Model::default();
            for i in 0..self.n {
                m.add(&format!("{}_{}", name, i));
            }
            for i in 0..self.n {
                let row: Vec<(usize, f64)> = (0..self.n)
                    .filter(|&k| k != i)
                    .map(|k| (k, if i < k { self.w[i * self.n + k] } else { self.w[k * self.n + i] }))
                    .collect();
                m.adj[i] = row;
            }
            m
        }
    }

    /// memory.rs's recall: random start, read-address with each bit flipped with probability `damage`, then sweeps.
    /// `read-address: None` starts from pure noise. Returns the final arrangement.
    pub fn recall(m: &Model, cue: Option<&[f64]>, damage: f64, sweeps: usize, temp: f64, seed: u64) -> Vec<f64> {
        let mut st = State::new(0x5eed);
        st.rng = Rng::new(seed);
        let (mut s, mut free) = st.start(m);
        if let Some(p) = cue {
            for i in 0..p.len() {
                s[i] = if st.rng.unit() < damage { -p[i] } else { p[i] };
            }
        }
        for _ in 0..sweeps {
            st.sweep(m, &mut s, &mut free, 1.0 / temp);
        }
        s
    }

    /// Continue settling from an arrangement (for the stability check of a fake valley).
    pub fn more(m: &Model, s: &[f64], sweeps: usize, temp: f64, seed: u64) -> Vec<f64> {
        let mut st = State::new(seed);
        let mut s = s.to_vec();
        let mut free: Vec<usize> = (0..m.len()).collect();
        for _ in 0..sweeps {
            st.sweep(m, &mut s, &mut free, 1.0 / temp);
        }
        s
    }
}

#[cfg(test)]
mod hopfield_tests {
    use super::hopfield::*;
    use super::overlap;
    use crate::interp::Interp;
    use crate::memory::code;

    #[test]
    fn the_fast_hopfield_is_exactly_memory_rs() {
        // the same recalls through memory.rs (via the interpreter) and through the fast builder print identical
        // overlaps, for noisy read-addresses and for pure noise
        let names = ["cat", "dog", "owl", "emu", "yak", "gnu", "elk"];
        let mut src = String::from("model :mind do\n  memory :m, size: 40\n");
        for n in names {
            src.push_str(&format!("  m.remember :{}\n", n));
        }
        src.push_str("end\nrun :mind do\n  m.recall read-address: :cat, address-noise: 0.4, seed: 11\n  m.recall read-address: :dog, address-noise: 0.5, seed: 12\n  m.recall seed: 13\n  m.recall seed: 14\nend");
        let out = Interp::default().exec(&src).unwrap();
        let mut w = Weights::new(40);
        let pats: Vec<Vec<f64>> = names.iter().map(|n| code(n, 40)).collect();
        for p in &pats {
            w.add(p);
        }
        let m = w.model("m");
        let cases: [(Option<&[f64]>, f64, u64); 4] = [(Some(&pats[0]), 0.4, 11), (Some(&pats[1]), 0.5, 12), (None, 0.0, 13), (None, 0.0, 14)];
        for (line, (cue, dmg, seed)) in out.iter().zip(cases) {
            let s = recall(&m, cue, dmg, 30, 0.1, seed);
            let mut sc: Vec<(&str, f64)> = names.iter().zip(&pats).map(|(n, p)| (*n, overlap(&s, p))).collect();
            sc.sort_by(|x, y| y.1.abs().partial_cmp(&x.1.abs()).unwrap());
            let top: Vec<String> = sc.iter().take(3).map(|(n, o)| format!(":{} {:+.2}", n, o)).collect();
            assert!(line.contains(&top.join("  ")), "memory.rs: {}\nfast: {}", line, top.join("  "));
        }
    }
}

/// The measurement sweep behind runs/softsdm/REPORT_SOFTSDM.md. Run with:
/// `cargo test --release measure_softsdm -- --ignored --nocapture`
#[cfg(test)]
mod measure {
    use super::hopfield;
    use super::*;
    use std::collections::BTreeMap;
    use std::fmt::Write as _;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    const N: usize = 256;
    const M: usize = 2000;
    const TS: [usize; 6] = [5, 10, 20, 40, 80, 160];
    const DMG: [f64; 4] = [0.1, 0.2, 0.3, 0.4];
    const SOFT: [f64; 6] = [0.0, 0.1, 0.25, 0.5, 1.0, 2.0];
    const FIRES: [f64; 2] = [0.05, 0.01];
    const SEEDS: [u64; 3] = [1, 2, 3];
    const Q: usize = 10;
    const ROUNDS: usize = 3;
    const SAMPLES: usize = 16;
    const NOISE_CUES: usize = 20;
    const NOISE_ROUNDS: usize = 10;

    #[derive(Clone)]
    struct Rec {
        reader: String,
        fire: f64,
        soft: f64,
        t: usize,
        dmg: f64,
        seed: u64,
        n: usize,
        ok: usize,
        ov: f64,
    }
    #[derive(Clone)]
    struct Fake {
        reader: String,
        fire: f64,
        soft: f64,
        t: usize,
        seed: u64,
        memory: usize,
        fake: usize,
        wander: usize,
    }
    #[derive(Clone)]
    struct Agree {
        fire: f64,
        soft: f64,
        seed: u64,
        beta: f64,
        pairs: [f64; 4], // pass~meanfield, pass~softmax, pass~kernel, meanfield~softmax (fraction of bits)
    }

    fn pats(seed: u64, n: usize, count: usize) -> Vec<Vec<f64>> {
        let mut r = Rng::new(seed.wrapping_mul(7919).wrapping_add(17));
        (0..count).map(|_| (0..n).map(|_| if r.unit() < 0.5 { -1.0 } else { 1.0 }).collect()).collect()
    }

    fn cue_rng(seed: u64, t: usize, d: f64, q: usize) -> Rng {
        Rng::new(seed.wrapping_mul(1_000_003).wrapping_add((t * 101 + (d * 100.0) as usize * 7 + q * 13) as u64))
    }

    fn agree(a: &[f64], b: &[f64]) -> f64 {
        a.iter().zip(b).filter(|(x, y)| x == y).count() as f64 / a.len() as f64
    }

    fn classify(s: &[f64], s2: &[f64], stored: &[Vec<f64>], abs: bool) -> usize {
        let best = stored.iter().map(|p| if abs { overlap(s, p).abs() } else { overlap(s, p) }).fold(f64::NEG_INFINITY, f64::max);
        let stab = if abs { overlap(s, s2).abs() } else { overlap(s, s2) };
        if best >= 0.9 {
            0
        } else if stab >= 0.95 {
            1
        } else {
            2
        }
    }

    type Out = (Vec<Rec>, Vec<Fake>, Vec<Agree>);

    fn sdm_job(fire: f64, soft: f64, seed: u64) -> Out {
        let (mut recs, mut fakes, mut agrees) = (Vec::new(), Vec::new(), Vec::new());
        let mut mc = Machine::new(N, M, fire, soft, 64.0, seed, 16);
        let ps = pats(seed, N, 161); // pattern 160 is never written: the never-stored control
        let never = ps[160].clone();
        let kern = mc.kernel_inf();
        let beta = Machine::fit_beta(&kern);
        let mut rng = Rng::new(seed ^ 0xBEEF);
        let mut written = 0;
        let do_settle = [0.0, 0.25, 1.0].contains(&soft);
        for &t in &TS {
            while written < t {
                mc.write(&ps[written], &mut rng);
                written += 1;
            }
            let stored = &ps[..t];
            let iter_attn = |cue: &[f64], how: &Attn, rounds: usize| {
                let mut c = cue.to_vec();
                for _ in 0..rounds {
                    c = attention_read(&c, stored, how);
                }
                c
            };
            for &d in &DMG {
                let mut acc: BTreeMap<&str, (usize, usize, f64)> = BTreeMap::new();
                for q in 0..t.min(Q) {
                    let cue = with_address_noise(&ps[q], d, &mut cue_rng(seed, t, d, q));
                    let mut outs: Vec<(&str, Vec<f64>)> = vec![
                        ("pass", mc.recall(&cue, ROUNDS, SAMPLES, false, 0, &mut rng)),
                        ("meanfield", mc.recall_mean_field(&cue, ROUNDS)),
                        ("kernel", iter_attn(&cue, &Attn::Kernel(&kern), ROUNDS)),
                        ("softmax", iter_attn(&cue, &Attn::Softmax(beta), ROUNDS)),
                    ];
                    if do_settle && t <= 40 {
                        outs.push(("settle", mc.recall(&cue, ROUNDS, SAMPLES, true, 10, &mut rng)));
                    }
                    for (nm, o) in outs {
                        let ov = overlap(&o, &ps[q]);
                        let e = acc.entry(nm).or_insert((0, 0, 0.0));
                        e.0 += 1;
                        e.1 += (ov >= 0.95) as usize;
                        e.2 += ov;
                    }
                }
                // controls: the never-written pattern as an undamaged read-address, and shuffled bit-counters
                if d == 0.1 {
                    let o = mc.recall(&never, ROUNDS, SAMPLES, false, 0, &mut rng);
                    let e = acc.entry("ctl_never_written").or_insert((0, 0, 0.0));
                    e.0 += 1;
                    e.1 += (overlap(&o, &never) >= 0.9) as usize;
                    e.2 += overlap(&o, &never);
                    let mut sh = mc.clone();
                    let mut rows: Vec<Vec<f64>> = sh.j.chunks(N).map(|c| c.to_vec()).collect();
                    let mut r2 = Rng::new(seed ^ 0x5A5A);
                    for i in (1..rows.len()).rev() {
                        let r = r2.below(i + 1);
                        rows.swap(i, r);
                    }
                    sh.j = rows.concat();
                    for q in 0..t.min(Q) {
                        let cue = with_address_noise(&ps[q], d, &mut cue_rng(seed, t, d, q));
                        let o = sh.recall(&cue, ROUNDS, SAMPLES, false, 0, &mut rng);
                        let e = acc.entry("ctl_shuffled").or_insert((0, 0, 0.0));
                        e.0 += 1;
                        e.1 += (overlap(&o, &ps[q]) >= 0.95) as usize;
                        e.2 += overlap(&o, &ps[q]);
                    }
                }
                for (nm, (n, ok, ov)) in acc {
                    recs.push(Rec { reader: nm.to_string(), fire, soft, t, dmg: d, seed, n, ok, ov: ov / n as f64 });
                }
            }
            if t == 10 || t == 40 {
                let mut nr = Rng::new(seed ^ 0x0015E + t as u64);
                let mut tally: BTreeMap<&str, [usize; 3]> = BTreeMap::new();
                for _ in 0..NOISE_CUES {
                    let cue: Vec<f64> = (0..N).map(|_| if nr.unit() < 0.5 { -1.0 } else { 1.0 }).collect();
                    let s = mc.recall(&cue, NOISE_ROUNDS, SAMPLES, false, 0, &mut rng);
                    let s2 = mc.recall(&s, 1, SAMPLES, false, 0, &mut rng);
                    tally.entry("pass").or_insert([0; 3])[classify(&s, &s2, stored, false)] += 1;
                    let s = mc.recall_mean_field(&cue, NOISE_ROUNDS);
                    let s2 = mc.recall_mean_field(&s, 1);
                    tally.entry("meanfield").or_insert([0; 3])[classify(&s, &s2, stored, false)] += 1;
                    let s = iter_attn(&cue, &Attn::Softmax(beta), NOISE_ROUNDS);
                    let s2 = iter_attn(&s, &Attn::Softmax(beta), 1);
                    tally.entry("softmax").or_insert([0; 3])[classify(&s, &s2, stored, false)] += 1;
                }
                for (nm, c) in tally {
                    fakes.push(Fake { reader: nm.to_string(), fire, soft, t, seed, memory: c[0], fake: c[1], wander: c[2] });
                }
            }
            if t == 20 {
                let mut pairs = [0.0; 4];
                for q in 0..Q {
                    let cue = with_address_noise(&ps[q], 0.2, &mut cue_rng(seed, t, 0.2, q));
                    let a = mc.read_pass(&cue, SAMPLES, &mut rng).0;
                    let b = sign_of(&mc.mean_field(&cue), &cue);
                    let c = attention_read(&cue, stored, &Attn::Softmax(beta));
                    let k = attention_read(&cue, stored, &Attn::Kernel(&kern));
                    pairs[0] += agree(&a, &b);
                    pairs[1] += agree(&a, &c);
                    pairs[2] += agree(&a, &k);
                    pairs[3] += agree(&b, &c);
                }
                agrees.push(Agree { fire, soft, seed, beta, pairs: pairs.map(|x| x / Q as f64) });
            }
        }
        (recs, fakes, agrees)
    }

    fn hopfield_job(n: usize, seed: u64) -> Out {
        let (mut recs, mut fakes) = (Vec::new(), Vec::new());
        let ps = pats(seed + 100, n, 160);
        let mut w = hopfield::Weights::new(n);
        let reader = format!("hopfield{}", n);
        for &t in &TS {
            while w.count < t {
                w.add(&ps[w.count]);
            }
            let m = w.model("m");
            for &d in &DMG {
                let (mut ok, mut ov) = (0, 0.0);
                let nq = t.min(Q);
                for q in 0..nq {
                    let s = hopfield::recall(&m, Some(&ps[q]), d, 30, 0.1, seed * 1000 + (t * 10 + q) as u64 + (d * 1e4) as u64);
                    let o = overlap(&s, &ps[q]).abs();
                    ok += (o >= 0.95) as usize;
                    ov += o;
                }
                recs.push(Rec { reader: reader.clone(), fire: 0.0, soft: 0.0, t, dmg: d, seed, n: nq, ok, ov: ov / nq as f64 });
            }
            if t == 10 || t == 40 {
                let mut c = [0usize; 3];
                for k in 0..NOISE_CUES {
                    let s = hopfield::recall(&m, None, 0.0, 30, 0.1, seed * 7777 + k as u64);
                    let s2 = hopfield::more(&m, &s, 30, 0.1, seed * 31 + k as u64);
                    c[classify(&s, &s2, &ps[..t], true)] += 1;
                }
                fakes.push(Fake { reader: reader.clone(), fire: 0.0, soft: 0.0, t, seed, memory: c[0], fake: c[1], wander: c[2] });
            }
        }
        (recs, fakes, Vec::new())
    }

    fn stamp() -> String {
        let run = |c: &str, a: &[&str]| std::process::Command::new(c).args(a).output().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
        let pm = run("pmset", &["-g"]).lines().find(|l| l.contains("powermode")).map(|l| l.trim().to_string()).unwrap_or("powermode unknown".into());
        format!("host {} · load {} · {} · macOS build {} · git {}", run("hostname", &["-s"]), run("sysctl", &["-n", "vm.loadavg"]), pm, run("sw_vers", &["-buildVersion"]), run("git", &["rev-parse", "--short", "HEAD"]))
    }

    fn rate(recs: &[Rec], f: impl Fn(&Rec) -> bool) -> Option<(f64, f64, usize)> {
        let sel: Vec<&Rec> = recs.iter().filter(|r| f(r)).collect();
        let n: usize = sel.iter().map(|r| r.n).sum();
        if n == 0 {
            return None;
        }
        let ok: usize = sel.iter().map(|r| r.ok).sum();
        let ov: f64 = sel.iter().map(|r| r.ov * r.n as f64).sum();
        Some((ok as f64 / n as f64, ov / n as f64, n))
    }

    fn cap(recs: &[Rec], f: &dyn Fn(&Rec) -> bool, d: f64) -> String {
        let mut best = None;
        for &t in &TS {
            if let Some((r, _, _)) = rate(recs, |x| f(x) && x.t == t && x.dmg == d) {
                if r >= 0.5 {
                    best = Some(t);
                }
            }
        }
        best.map(|t| t.to_string()).unwrap_or("<5".into())
    }

    #[test]
    #[ignore]
    fn measure_softsdm() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../runs/softsdm");
        std::fs::create_dir_all(&dir).unwrap();
        let stamp_before = stamp();
        let t0 = std::time::Instant::now();
        let mut jobs: Vec<Box<dyn Fn() -> Out + Send + Sync>> = Vec::new();
        for &n in &[256usize, 2512] {
            for &s in &SEEDS {
                jobs.push(Box::new(move || hopfield_job(n, s)));
            }
        }
        for &f in &FIRES {
            for &so in &SOFT {
                for &s in &SEEDS {
                    jobs.push(Box::new(move || sdm_job(f, so, s)));
                }
            }
        }
        let next = AtomicUsize::new(0);
        let all: Mutex<Vec<Out>> = Mutex::new(Vec::new());
        let workers = std::thread::available_parallelism().map(|x| x.get()).unwrap_or(8).min(16);
        std::thread::scope(|sc| {
            for _ in 0..workers {
                sc.spawn(|| loop {
                    let i = next.fetch_add(1, Ordering::SeqCst);
                    if i >= jobs.len() {
                        break;
                    }
                    let o = jobs[i]();
                    eprintln!("job {} of {} done at {:.0}s", i + 1, jobs.len(), t0.elapsed().as_secs_f64());
                    all.lock().unwrap().push(o);
                });
            }
        });
        let (mut recs, mut fakes, mut agrees) = (Vec::new(), Vec::new(), Vec::new());
        for (a, b, c) in all.into_inner().unwrap() {
            recs.extend(a);
            fakes.extend(b);
            agrees.extend(c);
        }
        // raw data
        let mut csv = String::from("reader,fire,softness,stored,damage,seed,queries,successes,mean_overlap\n");
        for r in &recs {
            writeln!(csv, "{},{},{},{},{},{},{},{},{:.4}", r.reader, r.fire, r.soft, r.t, r.dmg, r.seed, r.n, r.ok, r.ov).unwrap();
        }
        std::fs::write(dir.join("raw_recall.csv"), csv).unwrap();
        let mut csv = String::from("reader,fire,softness,stored,seed,memory,fake_valley,wandering\n");
        for f in &fakes {
            writeln!(csv, "{},{},{},{},{},{},{},{}", f.reader, f.fire, f.soft, f.t, f.seed, f.memory, f.fake, f.wander).unwrap();
        }
        std::fs::write(dir.join("raw_fake_valleys.csv"), csv).unwrap();
        let mut csv = String::from("fire,softness,seed,beta,pass_meanfield,pass_softmax,pass_kernel,meanfield_softmax\n");
        for a in &agrees {
            writeln!(csv, "{},{},{},{:.3},{:.4},{:.4},{:.4},{:.4}", a.fire, a.soft, a.seed, a.beta, a.pairs[0], a.pairs[1], a.pairs[2], a.pairs[3]).unwrap();
        }
        std::fs::write(dir.join("raw_agreement.csv"), csv).unwrap();

        // summary tables
        let mut md = String::new();
        writeln!(md, "# SOFTSDM sweep summary (generated)\n\nstamp before: {}\nstamp after: {}\nwall time {:.0} s, {} worker threads\n", stamp_before, stamp(), t0.elapsed().as_secs_f64(), workers).unwrap();
        for &fire in &FIRES {
            for &d in &DMG {
                writeln!(md, "\n## success rate, pass read, fire {}, damage {:.0}% (rows softness, columns stored)\n", fire, d * 100.0).unwrap();
                writeln!(md, "| softness | {} |", TS.map(|t| t.to_string()).join(" | ")).unwrap();
                writeln!(md, "|---|{}|", TS.map(|_| "---").join("|")).unwrap();
                for &so in &SOFT {
                    let cells: Vec<String> = TS.iter().map(|&t| rate(&recs, |r| r.reader == "pass" && r.fire == fire && r.soft == so && r.t == t && r.dmg == d).map(|(x, o, _)| format!("{:.2} ({:+.2})", x, o)).unwrap_or("-".into())).collect();
                    writeln!(md, "| {} | {} |", so, cells.join(" | ")).unwrap();
                }
            }
        }
        for h in ["hopfield256", "hopfield2512"] {
            writeln!(md, "\n## success rate, {} (rows damage, columns stored)\n", h).unwrap();
            writeln!(md, "| damage | {} |", TS.map(|t| t.to_string()).join(" | ")).unwrap();
            writeln!(md, "|---|{}|", TS.map(|_| "---").join("|")).unwrap();
            for &d in &DMG {
                let cells: Vec<String> = TS.iter().map(|&t| rate(&recs, |r| r.reader == h && r.t == t && r.dmg == d).map(|(x, o, _)| format!("{:.2} ({:+.2})", x, o)).unwrap_or("-".into())).collect();
                writeln!(md, "| {:.0}% | {} |", d * 100.0, cells.join(" | ")).unwrap();
            }
        }
        writeln!(md, "\n## capacity: largest stored count with success rate >= 0.5\n").unwrap();
        writeln!(md, "| machine | reader | 10% | 20% | 30% | 40% |\n|---|---|---|---|---|---|").unwrap();
        for &fire in &FIRES {
            for &so in &SOFT {
                for rd in ["pass", "meanfield", "kernel", "softmax", "settle"] {
                    if rate(&recs, |r| r.reader == rd && r.fire == fire && r.soft == so).is_none() {
                        continue;
                    }
                    let f = move |r: &Rec| r.reader == rd && r.fire == fire && r.soft == so;
                    let cs: Vec<String> = DMG.iter().map(|&d| cap(&recs, &f, d)).collect();
                    writeln!(md, "| sdm fire {} softness {} | {} | {} |", fire, so, rd, cs.join(" | ")).unwrap();
                }
            }
        }
        for h in ["hopfield256", "hopfield2512"] {
            let f = move |r: &Rec| r.reader == h;
            let cs: Vec<String> = DMG.iter().map(|&d| cap(&recs, &f, d)).collect();
            writeln!(md, "| {} | settle | {} |", h, cs.join(" | ")).unwrap();
        }
        writeln!(md, "\n## mean success rate over every (stored, damage) cell, per reader\n\n| machine | pass | meanfield | kernel | softmax | settle (stored <= 40) | pass (stored <= 40) |\n|---|---|---|---|---|---|---|").unwrap();
        for &fire in &FIRES {
            for &so in &SOFT {
                let g = |rd: &str, lim: usize| rate(&recs, |r| r.reader == rd && r.fire == fire && r.soft == so && r.t <= lim).map(|x| format!("{:.3}", x.0)).unwrap_or("-".into());
                writeln!(md, "| fire {} softness {} | {} | {} | {} | {} | {} | {} |", fire, so, g("pass", 999), g("meanfield", 999), g("kernel", 999), g("softmax", 999), g("settle", 40), g("pass", 40)).unwrap();
            }
        }
        writeln!(md, "\n## one-round bit agreement at 20 stored, 20% add_address_noise (mean of 3 seeds x 10 cues)\n\n| machine | fitted beta | pass~meanfield | pass~softmax | pass~kernel | meanfield~softmax |\n|---|---|---|---|---|---|").unwrap();
        for &fire in &FIRES {
            for &so in &SOFT {
                let sel: Vec<&Agree> = agrees.iter().filter(|a| a.fire == fire && a.soft == so).collect();
                let m = |i: usize| sel.iter().map(|a| a.pairs[i]).sum::<f64>() / sel.len() as f64;
                writeln!(md, "| fire {} softness {} | {:.2} | {:.3} | {:.3} | {:.3} | {:.3} |", fire, so, sel[0].beta, m(0), m(1), m(2), m(3)).unwrap();
            }
        }
        writeln!(md, "\n## reads from pure noise, {} cues x 3 seeds, {} rounds: memory / fake valley / wandering\n\n| machine | reader | stored 10 | stored 40 |\n|---|---|---|---|", NOISE_CUES, NOISE_ROUNDS).unwrap();
        let mut keys: Vec<(String, String)> = fakes.iter().map(|f| (if f.reader.starts_with("hop") { f.reader.clone() } else { format!("sdm fire {} softness {}", f.fire, f.soft) }, f.reader.clone())).collect();
        keys.sort();
        keys.dedup();
        for (mach, rd) in keys {
            let cell = |t: usize| {
                let sel: Vec<&Fake> = fakes.iter().filter(|f| f.reader == rd && f.t == t && (rd.starts_with("hop") || format!("sdm fire {} softness {}", f.fire, f.soft) == mach)).collect();
                let tot: usize = sel.iter().map(|f| f.memory + f.fake + f.wander).sum();
                let s = |k: fn(&Fake) -> usize| sel.iter().map(|f| k(f)).sum::<usize>() as f64 / tot.max(1) as f64;
                format!("{:.2} / {:.2} / {:.2}", s(|f| f.memory), s(|f| f.fake), s(|f| f.wander))
            };
            writeln!(md, "| {} | {} | {} | {} |", mach, rd, cell(10), cell(40)).unwrap();
        }
        writeln!(md, "\n## controls\n\n| machine | never-written cue recalled (rate) | shuffled counters, 10% damage, success rate |\n|---|---|---|").unwrap();
        for &fire in &FIRES {
            for &so in &SOFT {
                let a = rate(&recs, |r| r.reader == "ctl_never_written" && r.fire == fire && r.soft == so).unwrap();
                let b = rate(&recs, |r| r.reader == "ctl_shuffled" && r.fire == fire && r.soft == so).unwrap();
                let base = rate(&recs, |r| r.reader == "pass" && r.fire == fire && r.soft == so && r.dmg == 0.1).unwrap();
                writeln!(md, "| fire {} softness {} | {:.2} (mean overlap {:+.2}) | {:.2} (unshuffled {:.2}) |", fire, so, a.0, a.1, b.0, base.0).unwrap();
            }
        }
        std::fs::write(dir.join("SUMMARY_SOFTSDM_SWEEP.md"), &md).unwrap();
        println!("{}", md);
    }
}

/// P10: softness 0 at fire 0.02 (activation radius 112) against SDMKEYS' S2000a table. Run with:
/// `cargo test --release measure_vs_sdmkeys -- --ignored --nocapture`
#[cfg(test)]
mod measure_p10 {
    use super::*;

    #[test]
    #[ignore]
    fn measure_vs_sdmkeys() {
        // SDMKEYS S2000a success rates (runs/sdmkeys/measure_all.txt on branch settle-sdmkeys), stored x address-noise
        let theirs: [(usize, [f64; 4]); 5] = [
            (5, [1.00, 1.00, 0.93, 0.60]),
            (10, [1.00, 0.97, 0.73, 0.37]),
            (20, [1.00, 0.93, 0.50, 0.22]),
            (40, [0.95, 0.63, 0.28, 0.03]),
            (80, [0.38, 0.13, 0.02, 0.00]),
        ];
        let dmg = [0.1, 0.2, 0.3, 0.4];
        assert_eq!(hard_radius(256, 0.02).0, 112);
        let mut ok = vec![[0usize; 4]; 5];
        let mut nq = vec![[0usize; 4]; 5];
        for seed in 1..=3u64 {
            let mut mc = Machine::new(256, 2000, 0.02, 0.0, 64.0, seed + 40, 1);
            let mut r = Rng::new(seed * 991);
            let ps: Vec<Vec<f64>> = (0..80).map(|_| (0..256).map(|_| if r.unit() < 0.5 { -1.0 } else { 1.0 }).collect()).collect();
            let mut rng = Rng::new(seed ^ 0xC0FFEE);
            let mut written = 0;
            for (i, (t, _)) in theirs.iter().enumerate() {
                while written < *t {
                    mc.write(&ps[written], &mut rng);
                    written += 1;
                }
                for (j, &d) in dmg.iter().enumerate() {
                    for q in 0..(*t).min(10) {
                        let cue = with_address_noise(&ps[q], d, &mut rng);
                        let o = mc.recall(&cue, 10, 16, false, 0, &mut rng);
                        nq[i][j] += 1;
                        ok[i][j] += (overlap(&o, &ps[q]) >= 0.95) as usize;
                    }
                }
            }
        }
        let mut worst: f64 = 0.0;
        println!("stored | ours d=.1 .2 .3 .4 | SDMKEYS S2000a | largest gap");
        for (i, (t, th)) in theirs.iter().enumerate() {
            let ours: Vec<f64> = (0..4).map(|j| ok[i][j] as f64 / nq[i][j] as f64).collect();
            let gap = ours.iter().zip(th).map(|(a, b)| (a - b).abs()).fold(0.0, f64::max);
            worst = worst.max(gap);
            println!("{:>6} | {} | {} | {:.3}", t, ours.iter().map(|x| format!("{:.2}", x)).collect::<Vec<_>>().join(" "), th.map(|x| format!("{:.2}", x)).join(" "), gap);
        }
        println!("largest gap over all cells: {:.3} (P10 sealed: within 0.15)", worst);
    }
}
