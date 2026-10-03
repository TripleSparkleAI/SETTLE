//! SDMSCALE: Kanerva's sparse distributed memory at Kanerva's own scale (10^5 to 10^6 hard locations).
//!
//! ```text
//! model :big do
//!   sdmscale :k, word-size: 256, hard-locations: 100000, activation-probability: 0.001, seed: 1   # fire = fraction of hard locations a write wakes
//!   k.put :cat                                                         # a named random pattern
//!   k.fill 500                                                         # 500 more random patterns (load)
//! end
//! run :big do
//!   k.read read-address: :cat, address-noise: 0.3                  # Kanerva's address read, iterated to a fixed point
//!   k.read read-address: :cat, address-noise: 0.2, via: :pulls     # the content-woken read of the sdm family
//! end
//! ```
//!
//! This family does NOT live in SETTLE's pulls. `sdm.rs` keeps every counter as a pull, which caps it at
//! 20 million pulls (2000 hard locations x 256 is 512,000); a million hard locations needs 256 million bit-counters. Here
//! the bit-counters are one byte each in a flat array, the addresses are bit-packed, and a read is a popcount
//! scan. The arithmetic is the same as `sdm.rs` (a test proves bit-exact agreement with `View`), so the
//! numbers carry over; the store simply sits outside the springs. The model keeps only the list of what
//! was written, and the store is rebuilt from that list when a run reads it.
//!
//! Also here, for the SDMSCALE instrument: the entry and row shuffles (negative controls), the exact
//! Hamming-ball intersection and the Bricken-Pehlevan signal-to-noise map (the analytic prediction), and a
//! zero-temperature Hopfield memory, native or with a random sign expansion, sized to a given pull budget.
//!
//! The store, the shuffles, the ball and intersection counting, the S-map and the Hopfield baseline are
//! KANERVA's (`kanerva::store`, `kanerva::theory`, `kanerva::smap`, `kanerva::hopfield`), re-exported
//! here under their old names. This file keeps the SETTLE statements.

use crate::ext::{Claim, Ctx, Ext};
use crate::lex::{err, kw, kwargs, num, only, SettleError, Tok};
use crate::memory::{code, seed_of};
use crate::model::{Model, State};
use crate::rng::Rng;

pub use kanerva::bits::{add_address_noise, flip_exactly, overlap, pack, random_pattern};
pub use kanerva::hopfield::{hop_units_for, Hop};
pub use kanerva::smap::SMap;
pub use kanerva::store::{threads, ReadOut, Store};
pub use kanerva::theory::{ball, intersection, phi, phi_inv};

pub struct SdmScale;

// ---------------------------------------------------------------- the SETTLE statements

fn note(name: &str) -> String {
    format!("sdmscale:{}", name)
}

/// Rebuild a declared store from the model's note: (n, m, activation radius, seed) and the written list.
fn rebuild(m: &Model, name: &str, ln: usize) -> Result<(Store, Vec<String>), SettleError> {
    let (nums, words) = match m.notes.get(&note(name)) {
        Some(x) => x,
        None => return err(ln, format!("no sdmscale :{} (declare it with: sdmscale :{}, word-size: 256, hard-locations: 100000)", name, name)),
    };
    let (n, mm, radius, seed) = (nums[0] as usize, nums[1] as usize, nums[2] as usize, nums[3] as u64);
    let mut st = Store::new(n, mm, radius, seed);
    let mut ps = Vec::new();
    for w in words {
        if let Some(k) = w.strip_prefix('#') {
            let k: usize = k.parse().unwrap_or(0);
            let mut r = Rng::new(seed_of(&format!("sdmscale-fill:{}:{}", name, seed)));
            for _ in 0..k {
                ps.push(random_pattern(n, &mut r));
            }
        } else {
            ps.push(code(w, n).into_iter().map(|v| if v > 0.0 { 1 } else { -1 }).collect());
        }
    }
    st.write_many(&ps);
    Ok((st, words.clone()))
}

fn declare(m: &mut Model, rest: &[Tok], name: &str, ln: usize) -> Result<(), SettleError> {
    if m.notes.contains_key(&note(name)) {
        return err(ln, format!("sdmscale :{} is already declared", name));
    }
    let kv = kwargs(rest, ln)?;
    only(&kv, &["word-size", "hard-locations", "activation-probability", "activation-radius", "tolerate-noise", "seed"], "sdmscale", ln)?;
    let get = |k: &str, d: f64| kw(&kv, k).map(|v| num(v, ln)).transpose().map(|x| x.unwrap_or(d));
    let n = get("word-size", 256.0)? as usize;
    let mm = get("hard-locations", 100_000.0)? as usize;
    let seed = get("seed", 1.0)?;
    if !(16..=4096).contains(&n) {
        return err(ln, "sdmscale word-size must be between 16 and 4096");
    }
    if !(1..=2_000_000).contains(&mm) || n * mm > 1_100_000_000 {
        return err(ln, "sdmscale hard-locations must be 1 to 2,000,000, and word-size x hard-locations at most 1.1 billion bytes");
    }
    let fire = get("activation-probability", 0.001)?;
    if !(fire > 0.0 && fire < 1.0) {
        return err(ln, "activation-probability must be between 0 and 1");
    }
    let tolerate = get("tolerate-noise", -1.0)?;
    let by_fire = if tolerate >= 0.0 {
        if tolerate >= 0.5 {
            return err(ln, "tolerate-noise is an address-noise fraction below 0.5");
        }
        // SDMRADIUS: the activation radius whose S-map capacity from a read-address at this address-noise is largest
        let (lo, hi) = crate::sdmradius::search_window(n, mm);
        crate::sdmradius::radius_for_address_noise(n, mm, tolerate, lo, hi).0
    } else {
        crate::sdm::radius_for(n, fire)
    };
    let radius = get("activation-radius", by_fire as f64)? as usize;
    if radius > n {
        return err(ln, "activation-radius cannot be larger than word-size");
    }
    m.notes.insert(note(name), (vec![n as f64, mm as f64, radius as f64, seed], Vec::new()));
    Ok(())
}

fn put(m: &mut Model, name: &str, what: String, ln: usize) -> Result<(), SettleError> {
    match m.notes.get_mut(&note(name)) {
        Some((_, words)) => {
            if !what.starts_with('#') && words.contains(&what) {
                return err(ln, format!(":{} is already in :{}", what, name));
            }
            if what.starts_with('#') && words.iter().any(|w| w.starts_with('#')) {
                return err(ln, format!(":{} is already filled; one fill per store", name));
            }
            words.push(what);
            Ok(())
        }
        None => err(ln, format!("no sdmscale :{}", name)),
    }
}

fn read(m: &Model, st: &mut State, name: &str, rest: &[Tok], ln: usize, ctx: &mut Ctx) -> Result<(), SettleError> {
    let kv = kwargs(rest, ln)?;
    only(&kv, &["read-address", "address-noise", "iterated-reads", "via", "wake", "seed"], "read", ln)?;
    if let Some(x) = kw(&kv, "seed") {
        st.rng = Rng::new(num(x, ln)? as u64);
    }
    let iters = kw(&kv, "iterated-reads").map(|x| num(x, ln)).transpose()?.unwrap_or(20.0) as usize;
    let pulls = match kw(&kv, "via") {
        None => false,
        Some(Tok::Sym(s)) if s == "addresses" => false,
        Some(Tok::Sym(s)) if s == "pulls" => true,
        Some(_) => return err(ln, "via: takes :addresses or :pulls"),
    };
    let cue_name = match kw(&kv, "read-address") {
        Some(Tok::Sym(c)) => c.clone(),
        _ => return err(ln, "read needs read-address: :name"),
    };
    let damage_f = kw(&kv, "address-noise").map(|x| num(x, ln)).transpose()?.unwrap_or(0.2);
    let (store, words) = rebuild(m, name, ln)?;
    let p: Vec<i8> = code(&cue_name, store.n).into_iter().map(|v| if v > 0.0 { 1 } else { -1 }).collect();
    let cue = add_address_noise(&p, damage_f, &mut st.rng);
    let wake = match kw(&kv, "wake") {
        None => "fixed".to_string(),
        Some(Tok::Sym(s)) if s == "fixed" || s == "density" || s == "top" => s.clone(),
        Some(_) => return err(ln, "wake: takes :fixed (0.4 n), :density (scaled with the rows' load) or :top (the p M best rows)"),
    };
    if !pulls && wake != "fixed" {
        return err(ln, "wake: applies to via: :pulls");
    }
    let out = if !pulls {
        store.read_addresses(&cue, iters)
    } else if wake == "density" {
        store.read_pulls_at(&cue, iters, crate::sdmradius::density_threshold(&store, 0.1).0)
    } else if wake == "top" {
        store.read_pulls_topk(&cue, iters, (ball(store.n, store.radius) * store.m as f64).round().max(1.0) as usize)
    } else {
        store.read_pulls(&cue, iters)
    };
    let o = overlap(&out.z, &p);
    let written = words.contains(&cue_name);
    let verdict = if o >= 0.95 && written {
        format!("-> :{}", cue_name)
    } else if o >= 0.95 {
        format!("-> back to :{} though it was never written (the read did not move it)", cue_name)
    } else {
        format!("-> nothing clear (overlap with :{} {:+.2})", cue_name, o)
    };
    ctx.say(format!(
        "read :{} from read-address :{} with {:.0}% address-noise via {} ({} iterated reads, {} of {} hard locations activated, {} holding bit-counters): {}",
        name,
        cue_name,
        100.0 * damage_f,
        if pulls { "pulls" } else { "addresses" },
        out.rounds,
        out.awake,
        store.m,
        out.nonempty,
        verdict
    ));
    Ok(())
}

impl Ext for SdmScale {
    fn name(&self) -> &'static str {
        "sdmscale"
    }

    fn statements(&self) -> &'static [&'static str] {
        &[
            "model: sdmscale :k, word-size: 256, hard-locations: 100000, activation-probability: 0.001, seed: 1   (activation-radius: overrides activation-probability; tolerate-noise: 0.3 picks the activation-radius for 30% address-noise)",
            "model: k.put :cat   /   k.fill 500   (random patterns, one fill per store)",
            "run: k.read read-address: :cat, address-noise: 0.2, iterated-reads: 20, via: :addresses, seed: 1   (via: :pulls too, with wake: :fixed | :density | :top)",
        ]
    }

    fn model_stmt(&self, m: &mut Model, t: &[Tok], ln: usize, _ctx: &mut Ctx) -> Claim {
        match t {
            [Tok::Ident(k), Tok::Sym(name), rest @ ..] if k == "sdmscale" => {
                let rest = if rest.first() == Some(&Tok::Comma) { &rest[1..] } else { rest };
                Some(declare(m, rest, name, ln))
            }
            [Tok::Ident(name), Tok::Dot, Tok::Ident(v), Tok::Sym(what)] if v == "put" && m.notes.contains_key(&note(name)) => {
                Some(put(m, name, what.clone(), ln))
            }
            [Tok::Ident(name), Tok::Dot, Tok::Ident(v), Tok::Num(k)] if v == "fill" && m.notes.contains_key(&note(name)) => {
                let name = name.clone();
                Some(put(m, &name, format!("#{}", *k as usize), ln))
            }
            _ => None,
        }
    }

    fn run_stmt(&self, m: &mut Model, st: &mut State, t: &[Tok], ln: usize, ctx: &mut Ctx) -> Claim {
        match t {
            [Tok::Ident(name), Tok::Dot, Tok::Ident(v), rest @ ..] if v == "read" && m.notes.contains_key(&note(name)) => {
                Some(read(m, st, name, rest, ln, ctx))
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interp::Interp;
    use crate::sdm::{radius_for, View};

    fn f(p: &[i8]) -> Vec<f64> {
        p.iter().map(|&v| v as f64).collect()
    }

    #[test]
    fn bit_exact_with_sdm_rs_view() {
        // same addresses, same writes: every counter and every address read agrees with sdm.rs
        let (n, mm) = (64usize, 400usize);
        let radius = radius_for(n, 0.05);
        let mut model = Model::default();
        let v = View::declare(&mut model, "s", n, mm, radius, 3, 1.0);
        let mut st = Store::like_view("s", n, mm, radius, 3);
        let mut r = Rng::new(11);
        let ps: Vec<Vec<i8>> = (0..12).map(|_| random_pattern(n, &mut r)).collect();
        for p in &ps {
            let a = v.write(&mut model, &f(p));
            let b = st.write(p);
            assert_eq!(a, b);
        }
        for i in 0..mm {
            for j in 0..n {
                assert_eq!(v.counter(&model, i, j).round() as i8, st.counter(i, j));
            }
        }
        for (k, p) in ps.iter().enumerate() {
            let cue = add_address_noise(p, 0.2, &mut Rng::new(100 + k as u64));
            let (z1, r1, a1) = v.read_addresses(&model, &f(&cue), 10);
            let o = st.read_addresses(&cue, 10);
            assert_eq!(z1, f(&o.z));
            assert_eq!((r1, a1), (o.rounds, o.awake));
            let mut s = State::new(1);
            let (z2, _, _) = v.read_pulls(&model, &mut s, &f(&cue), 10);
            assert_eq!(z2, f(&st.read_pulls(&cue, 10).z));
        }
    }

    #[test]
    fn write_many_equals_one_by_one() {
        let mut a = Store::new(256, 3000, radius_for(256, 0.01), 5);
        let mut b = a.clone();
        let mut r = Rng::new(2);
        let ps: Vec<Vec<i8>> = (0..40).map(|_| random_pattern(256, &mut r)).collect();
        for p in &ps {
            a.write(p);
        }
        b.write_many(&ps);
        assert!(a.ctr == b.ctr && a.filled == b.filled && a.overflow == 0);
    }

    #[test]
    fn a_larger_store_recalls_and_both_shuffles_do_not() {
        let mut st = Store::new(256, 20_000, radius_for(256, 0.005), 7);
        let mut r = Rng::new(9);
        let ps: Vec<Vec<i8>> = (0..100).map(|_| random_pattern(256, &mut r)).collect();
        st.write_many(&ps);
        let ok = |s: &Store, r: &mut Rng| ps.iter().take(20).filter(|p| overlap(&s.read_addresses(&add_address_noise(p, 0.1, r), 20).z, p) >= 0.95).count();
        assert_eq!(ok(&st, &mut r), 20, "vacuity control: the unshuffled store recalls");
        let mut e = st.clone();
        e.shuffle_entries(&mut Rng::new(4));
        assert_eq!(ok(&e, &mut r), 0);
        let mut w = st.clone();
        w.shuffle_rows(&mut Rng::new(4));
        assert_eq!(ok(&w, &mut r), 0);
        // a never-written pattern does not come back from 10% address-noise
        let never = random_pattern(256, &mut r);
        assert!(overlap(&st.read_addresses(&add_address_noise(&never, 0.1, &mut r), 20).z, &never) < 0.95);
    }

    #[test]
    fn intersection_matches_ball_at_zero_and_counts() {
        let (n, r) = (64usize, 26usize);
        assert!((intersection(n, r, 0) - ball(n, r)).abs() < 1e-12);
        // brute force over all 2^16 addresses at n = 16
        let (n, r, d) = (16usize, 6usize, 5usize);
        let mut c = 0;
        for a in 0u32..(1 << 16) {
            let dx = a.count_ones() as usize;
            let dy = (a ^ 0b11111).count_ones() as usize;
            if dx <= r && dy <= r {
                c += 1;
            }
        }
        assert!((intersection(n, r, d) - c as f64 / 65536.0).abs() < 1e-12);
    }

    #[test]
    fn smap_reproduces_the_wiki_critical_distance() {
        // wiki WIKI_SDR 85 section 2: the exact S-map gives d_crit 159 at n 1000, M 10^6, T 10^4, r 451
        let s = SMap::new(1000, 1_000_000, 451);
        let dc = s.critical(10_000);
        assert!((150..=170).contains(&dc), "{}", dc);
        assert!((phi(1.0) - 0.841344746).abs() < 1e-6 && (phi_inv(0.975) - 1.959964).abs() < 1e-4);
    }

    #[test]
    fn hopfield_native_and_expanded_recall_a_few() {
        let mut r = Rng::new(3);
        for &(nh, ex) in &[(256usize, false), (1024usize, true)] {
            let mut h = Hop::new(256, nh, ex, 1);
            let ps: Vec<Vec<i8>> = (0..10).map(|_| random_pattern(256, &mut r)).collect();
            for p in &ps {
                h.store(p);
            }
            let ok = ps.iter().filter(|p| overlap(&h.recall(&add_address_noise(p, 0.1, &mut r), 30, &mut r).0, p) >= 0.95).count();
            assert_eq!(ok, 10, "nh {} expand {}", nh, ex);
        }
        assert_eq!(hop_units_for(32_640), 256);
    }

    #[test]
    fn statements_put_fill_and_read() {
        let out = Interp::default()
            .exec(
                "model :big do
  sdmscale :k, word-size: 256, hard-locations: 20000, activation-probability: 0.005, seed: 1
  k.put :cat
  k.put :dog
  k.fill 50
end
run :big do
  k.read read-address: :cat, address-noise: 0.2, seed: 1
  k.read read-address: :dog, address-noise: 0.1, via: :pulls, seed: 2
  k.read read-address: :zebra, address-noise: 0.1, seed: 3
end",
            )
            .unwrap_or_else(|e| panic!("{}", e));
        assert!(out[0].ends_with("-> :cat"), "{}", out[0]);
        assert!(out[1].ends_with("-> :dog"), "{}", out[1]);
        assert!(out[2].contains("nothing clear"), "{}", out[2]);
        let out = Interp::default()
            .exec(
                "model :wide do
  sdmscale :k, word-size: 256, hard-locations: 30000, tolerate-noise: 0.3, seed: 1
  k.put :cat
  k.fill 20
end
run :wide do
  k.read read-address: :cat, address-noise: 0.3, seed: 4
  k.read read-address: :cat, address-noise: 0.3, via: :pulls, wake: :density, seed: 5
end",
            )
            .unwrap_or_else(|e| panic!("{}", e));
        assert!(out[0].ends_with("-> :cat") && out[1].ends_with("-> :cat"), "{:?}", out);
        let e = Interp::default().exec("model :m do\n  sdmscale :k, word-size: 8\nend").err().unwrap().0;
        assert!(e.starts_with("line 2: sdmscale word-size"), "{}", e);
    }
}
