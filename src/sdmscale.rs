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
//!
//! The statements are parsed by KANERVA (`kanerva::lang`), mounted in the registry by `crate::plug`; the printed
//! lines come from `kanerva::lang::say`, and `exec` runs the typed statement on the model.

use crate::ext::Ctx;
use crate::lex::{err, SettleError};
use crate::memory::{code, seed_of};
use kanerva::lang::{say, Stmt, Via, Wake};
use crate::model::{Model, State};
use crate::rng::Rng;

pub use kanerva::bits::{add_address_noise, flip_exactly, overlap, pack, random_pattern};
pub use kanerva::hopfield::{hop_units_for, Hop};
pub use kanerva::smap::SMap;
pub use kanerva::store::{threads, ReadOut, Store};
pub use kanerva::theory::{ball, intersection, phi, phi_inv};

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

#[allow(clippy::too_many_arguments)]
fn read(
    m: &Model,
    st: &mut State,
    name: &str,
    seed: Option<u64>,
    iters: usize,
    via: Via,
    cue_name: &str,
    damage: f64,
    wake: Wake,
    ln: usize,
    ctx: &mut Ctx,
) -> Result<(), SettleError> {
    if let Some(x) = seed {
        st.rng = Rng::new(x);
    }
    let (store, words) = rebuild(m, name, ln)?;
    let p: Vec<i8> = code(cue_name, store.n).into_iter().map(|v| if v > 0.0 { 1 } else { -1 }).collect();
    let cue = add_address_noise(&p, damage, &mut st.rng);
    let out = match (via, wake) {
        (Via::Addresses, _) => store.read_addresses(&cue, iters),
        (Via::Pulls, Wake::Density) => store.read_pulls_at(&cue, iters, crate::sdmradius::density_threshold(&store, 0.1).0),
        (Via::Pulls, Wake::Top) => store.read_pulls_topk(&cue, iters, (ball(store.n, store.radius) * store.m as f64).round().max(1.0) as usize),
        (Via::Pulls, Wake::Fixed) => store.read_pulls(&cue, iters),
    };
    let o = overlap(&out.z, &p);
    let written = words.iter().any(|w| w == cue_name);
    ctx.say(say::scale_read(name, cue_name, damage, via, out.rounds, out.awake, store.m, out.nonempty, o, written));
    Ok(())
}

crate::plug::mount!(SdmScale, kanerva::lang::Family::SdmScale);

/// Run one parsed sdmscale statement on this model: declare the store's note, put or fill, or read (the store is
/// rebuilt from the note for each read). `st` is None inside a model block.
pub fn exec(m: &mut Model, st: Option<&mut State>, s: Stmt, ln: usize, ctx: &mut Ctx) -> Result<(), SettleError> {
    match (s, st) {
        (Stmt::ScaleDeclare { name, word_size, hard_locations, activation_radius, seed }, _) => {
            m.notes.insert(note(&name), (vec![word_size as f64, hard_locations as f64, activation_radius as f64, seed], Vec::new()));
            Ok(())
        }
        (Stmt::ScalePut { name, what }, _) => put(m, &name, what, ln),
        (Stmt::ScaleFill { name, count }, _) => put(m, &name, format!("#{}", count), ln),
        (Stmt::ScaleRead { name, seed, iterated_reads, via, read_address, address_noise, wake }, Some(st)) => {
            read(m, st, &name, seed, iterated_reads, via, &read_address, address_noise, wake, ln, ctx)
        }
        _ => err(ln, "not an sdmscale statement in this place"),
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
