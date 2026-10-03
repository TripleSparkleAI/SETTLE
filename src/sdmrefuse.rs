//! SDMREFUSE: when should a sparse distributed memory say "I never stored that"?
//!
//! SDMRADIUS found that the top-k pulls read (wake the p M rows whose bit-counters best match the state) beats
//! every address read at every address-noise, but never refuses: a never-stored read-address settles onto some stored
//! pattern. Its final state is then a stored pattern, exactly as a correct recall's is, so no signal read
//! off the final state alone can tell the two apart. This family gives the reads the extra signals a real
//! memory can see, the exact nearest-neighbour ceiling it is judged against, and a predictor that follows
//! the state's overlap with every stored pattern through every read:
//!
//! * `Fast` wraps `sdmscale::Store` with reads that return diagnostics (`Diag`): the answer, the rounds,
//!   the first-round and last-round votes' agreement with the answer, and the woken rows' mean dot. The
//!   content-woken reads update the rows' dots incrementally (only the flipped bits), and unit tests prove
//!   the answers identical to `Store::read_pulls_topk`, `read_pulls_at` and `read_addresses`.
//! * `travel_threshold(n, T, level)`: the largest travel h (Hamming distance from read-address to answer) such that a
//!   random read-address has a stored pattern within h with probability at most `level`. A memory knows how many
//!   patterns it holds, so this rule is visible. Accept iff travel <= h.
//! * `oracle_point`: the exact recall and refusal of the nearest-neighbour oracle (it sees every stored
//!   pattern, answers the nearest, refuses beyond h). No read can beat it at the same refusal.
//! * `track_converge`: predictor TRACK. It keeps the target and every other stored pattern explicitly, and
//!   at each read draws the number of hard locations the state shares with each one from its current distance;
//!   the vote is then exactly sum_mu k_mu x_mu. FRESH redraws the counts at every read; PERSIST keeps a
//!   fraction of the previous count (the fraction of the old wake set still inside the new one) and draws
//!   only the arrivals.
//!
//! Statement (a calculator, no memory built): `refusal word-size: 256, load: 3000, level: 0.01` prints h and
//! the oracle's recall at 10/20/30/40% address-noise with that h. Equations with one-line readings are in
//! `runs/sdmrefuse/REPORT_SDMREFUSE.md`.
//!
//! The refusal rules, the oracle, the diagnostic reads and TRACK are KANERVA's (`kanerva::refuse`), with
//! `binom_pmf` from `kanerva::theory` and `hd`, `nearest`, `pack_all` from `kanerva::bits`, re-exported
//! here under their old names. This file keeps the `refusal` statement.

use crate::ext::{Claim, Ctx, Ext};
use crate::lex::{err, kw, kwargs, num, only, SettleError, Tok};
use crate::model::{Model, State};

pub use kanerva::bits::{hd, nearest, pack_all};
pub use kanerva::refuse::{half_cdf, oracle_point, poisson, refusal_prob, travel_threshold, Diag, Fast, Track};
pub use kanerva::theory::binom_pmf;

#[cfg(test)]
use crate::rng::Rng;
#[cfg(test)]
use kanerva::bits::pack;
#[cfg(test)]
use kanerva::store::Store;
#[cfg(test)]
use kanerva::theory::ball;

pub struct SdmRefuse;

fn refusal_stmt(t: &[Tok], ln: usize, ctx: &mut Ctx) -> Result<(), SettleError> {
    let kv = kwargs(t, ln)?;
    only(&kv, &["word-size", "load", "level"], "refusal", ln)?;
    let get = |k: &str, d: f64| kw(&kv, k).map(|v| num(v, ln)).transpose().map(|x| x.unwrap_or(d));
    let n = get("word-size", 256.0)? as usize;
    let load = get("load", 1000.0)? as usize;
    let level = get("level", 0.01)?;
    if !(16..=4096).contains(&n) {
        return err(ln, "refusal size must be between 16 and 4096");
    }
    if load < 1 || !(level > 0.0 && level < 1.0) {
        return err(ln, "refusal needs load: at least 1 and level: between 0 and 1");
    }
    let h = travel_threshold(n, load, level);
    let recs: Vec<String> = [0.1, 0.2, 0.3, 0.4]
        .iter()
        .map(|&d| {
            let (rc, all, _) = oracle_point(n, d, load, h);
            format!("{:.0}%: {:.3} of {:.3}", 100.0 * d, rc, all)
        })
        .collect();
    ctx.say(format!(
        "refusal for {} patterns of {} bits: accept an answer only if it lies within {} bits of the cue (a never-stored cue is refused with probability {:.4}); the nearest-neighbour ceiling then recalls {}",
        load,
        n,
        h,
        refusal_prob(n, load, h),
        recs.join(", ")
    ));
    Ok(())
}

impl Ext for SdmRefuse {
    fn name(&self) -> &'static str {
        "sdmrefuse"
    }

    fn statements(&self) -> &'static [&'static str] {
        &["run: refusal word-size: 256, load: 3000, level: 0.01   (the travel rule's threshold and the nearest-neighbour ceiling; no memory built)"]
    }

    fn run_stmt(&self, _m: &mut Model, _st: &mut State, t: &[Tok], ln: usize, ctx: &mut Ctx) -> Claim {
        match t {
            [Tok::Ident(k), rest @ ..] if k == "refusal" => Some(refusal_stmt(rest, ln, ctx)),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sdmscale::{add_address_noise, overlap, random_pattern};

    fn store(m: usize, r: usize, t: usize, seed: u64) -> (Store, Vec<Vec<i8>>) {
        let mut rr = Rng::new(seed);
        let ps: Vec<Vec<i8>> = (0..t).map(|_| random_pattern(256, &mut rr)).collect();
        let mut st = Store::new(256, m, r, seed);
        st.write_many(&ps);
        (st, ps)
    }

    #[test]
    fn diagnostic_reads_give_the_same_answers_as_the_store() {
        let (st, ps) = store(20_000, 105, 300, 3);
        let f = Fast::new(&st);
        let k = (ball(256, 105) * 20_000.0).round() as usize;
        let mut rr = Rng::new(9);
        for q in 0..12 {
            let cue = if q % 4 == 3 { random_pattern(256, &mut rr) } else { add_address_noise(&ps[q], 0.1 * (q % 4 + 1) as f64, &mut rr) };
            assert_eq!(f.topk(&cue, 20, k).z, st.read_pulls_topk(&cue, 20, k).z, "topk q {}", q);
            assert_eq!(f.thresh(&cue, 20, 60.0).z, st.read_pulls_at(&cue, 20, 60.0).z, "thresh q {}", q);
            let a = f.address(&cue, 20);
            let b = st.read_addresses(&cue, 20);
            assert_eq!(a.z, b.z, "address q {}", q);
            assert_eq!(a.rounds, b.rounds);
        }
    }

    #[test]
    fn a_clean_recall_has_a_high_first_vote_cosine_and_short_travel() {
        let (st, ps) = store(20_000, 105, 30, 5);
        let f = Fast::new(&st);
        let k = (ball(256, 105) * 20_000.0).round() as usize;
        let mut rr = Rng::new(1);
        let cue = add_address_noise(&ps[0], 0.2, &mut rr);
        let d = f.topk(&cue, 20, k);
        assert!(overlap(&d.z, &ps[0]) > 0.99);
        assert!(d.cos1 > 0.8 && d.cosf > 0.9, "{} {}", d.cos1, d.cosf);
        let travel = hd(&pack(&cue), &pack(&d.z));
        assert!((30..=80).contains(&travel), "{}", travel);
    }

    #[test]
    fn travel_threshold_and_oracle_match_the_hand_numbers() {
        // n 256: T 10 -> h 102 (refusal 0.9931); T 3000 -> h 91 (0.9935), computed by hand with exact binomials
        assert_eq!(travel_threshold(256, 10, 0.01), 102);
        assert_eq!(travel_threshold(256, 3000, 0.01), 91);
        assert!((refusal_prob(256, 10, 102) - 0.9931).abs() < 2e-4);
        let (rc, all, rf) = oracle_point(256, 0.4, 10, 102);
        assert!((rc - 0.506).abs() < 2e-3 && (all - 0.931).abs() < 2e-3, "{} {}", rc, all);
        assert!(rf > 0.99);
        // one stored pattern: the oracle always names it
        let (_, all1, _) = oracle_point(256, 0.4, 1, 256);
        assert!((all1 - 1.0).abs() < 1e-9);
    }

    #[test]
    fn simulated_oracle_matches_the_closed_form() {
        let (n, t, dmg) = (256usize, 300usize, 0.4);
        let mut rr = Rng::new(11);
        let (mut win, q) = (0usize, 3000);
        for _ in 0..q {
            let ps: Vec<Vec<i8>> = (0..t).map(|_| random_pattern(n, &mut rr)).collect();
            let pk = pack_all(&ps);
            let tgt = rr.below(t);
            let cue = add_address_noise(&ps[tgt], dmg, &mut rr);
            let (i, _) = nearest(&pk, &pack(&cue));
            win += (i == tgt) as usize;
        }
        let (_, all, _) = oracle_point(n, dmg, t, n);
        let got = win as f64 / q as f64;
        // lowest-index tie breaking is uniform over a random target position; 3 standard errors
        assert!((got - all).abs() < 3.0 * (all * (1.0 - all) / q as f64).sqrt() + 0.01, "{} vs {}", got, all);
    }

    #[test]
    fn track_recalls_a_lone_pattern_and_fails_when_no_location_is_shared() {
        let tr = Track::new(256, 100_000, 106);
        assert!(tr.p_converge(0.1, 1, 200, false, 1) > 0.99);
        // a activation radius so small that a 40% read-address shares nothing: every read fails
        let tiny = Track::new(256, 1000, 90);
        assert!(tiny.p_converge(0.4, 1, 200, false, 1) < 0.01);
        // more stored patterns never help (common random numbers, loose monotonicity)
        let a = tr.p_converge(0.3, 30, 300, true, 2);
        let b = tr.p_converge(0.3, 3000, 300, true, 2);
        assert!(b <= a + 0.05, "{} {}", a, b);
    }

    #[test]
    fn the_refusal_statement_prints_the_threshold() {
        let out = crate::interp::Interp::default()
            .exec(
                "model :m do
end
run :m do
  refusal word-size: 256, load: 3000, level: 0.01
end",
            )
            .unwrap();
        let all = out.join("\n");
        assert!(all.contains("within 91 bits"), "{}", all);
        let bad = crate::interp::Interp::default().exec("model :m do\nend\nrun :m do\n  refusal load: 0\nend");
        assert!(bad.is_err());
    }

    #[test]
    fn persist_holds_a_fixed_point() {
        // a state that does not move keeps every shared hard location: keep[0] = 1 and no arrivals
        let tr = Track::new(256, 100_000, 106);
        assert!((tr.keep[0] - 1.0).abs() < 1e-12);
        assert!(tr.keep[40] < tr.keep[5]);
    }
}
