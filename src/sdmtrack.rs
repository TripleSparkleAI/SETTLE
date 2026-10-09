//! SDMTRACK: predicting the content-woken reads of Kanerva's memory, and why never-stored read-addresses stop moving.
//!
//! SDMREFUSE's predictor TRACK follows every stored pattern through every read of the ADDRESS read, where the
//! number of woken hard locations holding pattern mu is Poisson in the state's distance to mu. The content reads
//! (top-k: wake the k filled rows whose bit-counters best match the state; block: wake every row whose match
//! exceeds a density threshold) choose rows by what they hold, so that count depends on the whole stored
//! population. This family gives:
//!
//! * `compound_pmf`: the distribution of a row's match when the row holds a Poisson number of stored patterns
//!   drawn from the current population of overlaps (an FFT of the characteristic function).
//! * `Content` (predictor TRACK-C): keeps the target and every rival pattern explicitly; at each read it
//!   computes every pattern's overlap u_mu with the state, the row-match distribution, the top-k cut (or the
//!   block threshold), and each pattern's wake probability q_mu = P[u_mu + rest > cut]; the number of woken
//!   rows holding mu is then Poisson(p M q_mu) (FRESH) or coupled to the previous read's count through a
//!   Poisson process of per-row rest quantiles (PERSIST); the vote is sum_mu k_mu x_mu.
//! * `Member` (reference TRACK-R): an explicit random membership graph (each pattern written to Poisson(p M)
//!   uniformly chosen rows, no addresses), read exactly as the store reads. It drops only the address
//!   geometry, so it separates the count approximation's error from the geometry's.
//! * `trace_topk`: the store's own top-k read with the woken rows of every round, and `census`: which stored
//!   patterns those rows hold (distinct count, effective number, the top pattern's share), and the vote's
//!   component along the state against its spread (the self-vote).
//! * `TailCal`: the probe tail fractions and the combined-score refusal rules (product and calibrated min).
//!
//! Statement (a calculator, no memory built):
//! `contenttrack word-size: 256, hard-locations: 100000, load: 3000, address-noise: 0.3, block: 0, samples: 400` prints
//! TRACK-C's success probability (FRESH and PERSIST). Equations with one-line readings are in
//! `runs/sdmtrack/REPORT_SDMTRACK.md`.
//!
//! Every predictor here is KANERVA's (`kanerva::track`), re-exported under its old name. This file keeps
//! the `contenttrack` statement.
//!
//! The statements are parsed by KANERVA (`kanerva::lang`), mounted in the registry by `crate::plug`; the printed
//! line is computed by `kanerva::lang::say`, so this file holds no statement code of its own.


pub use kanerva::track::{
    block_theta, block_theta_geo, calibrate, census, compound_pmf, compound_pmf_len, fft, flips_estimate, packed, trace_topk, Census,
    Content, Geo, Member, Round, Sample, TailCal, Wake, G, GG, H,
};

#[cfg(test)]
use crate::rng::Rng;
#[cfg(test)]
use crate::sdmrefuse::hd;
#[cfg(test)]
use kanerva::bits::pack;
#[cfg(test)]
use kanerva::store::Store;
#[cfg(test)]
use kanerva::theory::ball;

crate::plug::mount!(SdmTrack, kanerva::lang::Family::ContentTrack);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sdmrefuse::Fast;
    use crate::sdmscale::{add_address_noise, overlap, random_pattern};

    #[test]
    fn compound_pmf_matches_direct_convolution() {
        let sev = vec![(-3i64, 0.2), (1, 0.5), (4, 0.3)];
        let lam = 1.7;
        let got = compound_pmf(&sev, lam);
        // direct: sum_j Poisson(j) sev^{*j}, on a dense array offset 200
        let mut acc = vec![0.0f64; 401];
        let mut cur = vec![0.0f64; 401];
        cur[200] = 1.0;
        let mut pj = (-lam).exp();
        for j in 0..40 {
            for i in 0..401 {
                acc[i] += pj * cur[i];
            }
            let mut nx = vec![0.0f64; 401];
            for i in 0..401 {
                if cur[i] == 0.0 {
                    continue;
                }
                for &(u, w) in &sev {
                    let k = i as i64 + u;
                    if (0..401).contains(&k) {
                        nx[k as usize] += cur[i] * w;
                    }
                }
            }
            cur = nx;
            pj *= lam / (j + 1) as f64;
        }
        for u in -150i64..150 {
            assert!((got[(u + H) as usize] - acc[(u + 200) as usize]).abs() < 1e-12, "u {}", u);
        }
        assert!((got.iter().sum::<f64>() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn the_short_grid_matches_the_long_grid() {
        let sev = vec![(-20i64, 0.3), (5, 0.4), (31, 0.3)];
        let a = compound_pmf(&sev, 6.5);
        let b = compound_pmf_len(&sev, 6.5, GG);
        for u in -600i64..600 {
            assert!((a[(u + H) as usize] - b[(u + (GG / 2) as i64) as usize]).abs() < 1e-12, "u {}", u);
        }
    }

    #[test]
    fn block_read_on_one_pattern_matches_the_exact_answer() {
        // one stored pattern, 40% address-noise: the block read recalls iff the target's match s = 256 - 2 Bin(256, 0.4)
        // exceeds theta, and a woken row always recalls (every woken row holds only the target)
        let (n, m) = (256usize, 100_000usize);
        let r = crate::sdm::radius_for(n, (m as f64 * m as f64 / 10.0).powf(-1.0 / 3.0));
        let th = block_theta(n, m, r, 1, 0.1, 0.01);
        let exact: f64 = crate::sdmrefuse::binom_pmf(n, 0.4).iter().enumerate().filter(|(d, _)| (n as f64 - 2.0 * *d as f64) > th).map(|x| *x.1).sum();
        let c = Content::new(n, m, r);
        let got = c.p_converge(Wake::Block(th), 0.4, 1, 3000, false, 3);
        assert!((got - exact).abs() < 0.03, "{} vs exact {}", got, exact);
        assert!((0.7..0.9).contains(&exact), "{}", exact);
        // and the top-k read always wakes the target's rows
        assert!(c.p_converge(Wake::Topk(c.k()), 0.4, 1, 300, false, 3) > 0.99);
    }

    #[test]
    fn geometry_membership_averages_to_the_write_probability() {
        // over all shells, a pattern of any overlap lies within r of a random address with probability p
        let g = Geo::new(256, 1_000_000_000_000, 104, 1e-3);
        let p = ball(256, 104);
        for u in [0i64, 40, -30, 100] {
            let s: f64 = g.shells.iter().enumerate().map(|(k, &(_, pm))| pm * g.h[(u + 128) as usize][k]).sum();
            assert!((s - p).abs() < 1e-6 * p.max(1e-12) + 1e-12, "u {}: {} vs {}", u, s, p);
        }
        assert!(g.kappa > 0.15 && g.kappa < 0.25, "{}", g.kappa);
    }

    #[test]
    fn geometry_predicts_the_store_block_threshold() {
        // the store's row load includes co-member overlaps; TRACK-G's kappa^2 term reproduces it, TRACK-C's does not
        let (m, r, t) = (20_000usize, 104usize, 2000usize);
        let mut rr = Rng::new(13);
        let ps: Vec<Vec<i8>> = (0..t).map(|_| random_pattern(256, &mut rr)).collect();
        let mut st = Store::new(256, m, r, 13);
        st.write_many(&ps);
        let (th, _, _) = crate::sdmradius::density_threshold_blocks(&st, 0.1, 0.01);
        let g = Geo::new(256, m, r, 0.01);
        let tg = block_theta_geo(256, m, r, t, 0.1, 0.01, g.kappa);
        let tc = block_theta(256, m, r, t, 0.1, 0.01);
        println!("block threshold: store {:.1} TRACK-G {:.1} TRACK-C {:.1}", th, tg, tc);
        assert!((tg / th - 1.0).abs() < 0.02, "geo {} store {}", tg, th);
        assert!((tc / th - 1.0).abs() > 0.03, "plain {} store {}", tc, th);
    }

    #[test]
    fn geometry_predicts_the_targets_woken_rows_in_a_real_store() {
        // fixed state (30% read-address), one top-k read: rows holding the target that wake, summed over targets, against
        // p M q_target from TRACK-G and TRACK-C
        let (m, r, t) = (30_000usize, 104usize, 3000usize);
        let mut rr = Rng::new(17);
        let ps: Vec<Vec<i8>> = (0..t).map(|_| random_pattern(256, &mut rr)).collect();
        let mut st = Store::new(256, m, r, 17);
        st.write_many(&ps);
        let pk: Vec<Vec<u64>> = ps.iter().map(|p| pack(p)).collect();
        let cg = Content::with_geometry(256, m, r);
        let cc = Content::new(256, m, r);
        let k = cc.k();
        let rows: Vec<usize> = (0..m).filter(|&i| st.filled[i]).collect();
        let (mut meas, mut pg, mut pc) = (0.0, 0.0, 0.0);
        for tg in 0..30 {
            let cue = add_address_noise(&ps[tg], 0.3, &mut rr);
            let mut dots: Vec<(usize, i32)> = rows.iter().map(|&i| (i, st.row(i).iter().zip(&cue).map(|(&c, &v)| c as i32 * v as i32).sum())).collect();
            dots.select_nth_unstable_by(k - 1, |a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
            meas += dots[..k].iter().filter(|x| st.dist(x.0, &pk[tg]) <= r).count() as f64;
            let cp = pack(&cue);
            let u: Vec<i64> = pk.iter().map(|p| 128 - hd(p, &cp) as i64).collect();
            pg += cg.pm * cg.wake_probs(&u, Wake::Topk(k))[tg];
            pc += cc.pm * cc.wake_probs(&u, Wake::Topk(k))[tg];
        }
        println!("target rows woken over 30 reads: measured {} TRACK-G {:.1} TRACK-C {:.1}", meas, pg, pc);
        assert!((pg - meas).abs() < (pc - meas).abs(), "geo {} plain {} measured {}", pg, pc, meas);
        assert!((pg / meas - 1.0).abs() < 0.3, "geo {} measured {}", pg, meas);
    }

    #[test]
    fn small_load_wake_probabilities_exclude_the_pattern_itself() {
        // T <= 64: each pattern's rest is the compound Poisson over the OTHER patterns; check one pattern by hand
        let c = Content::new(256, 20_000, 104);
        let u: Vec<i64> = vec![30, 30, -4, 12, 7, 30, -20, 1];
        let th = 50.0;
        let q = c.wake_probs(&u, Wake::Block(th));
        for mu in [0usize, 2, 6] {
            let sev: Vec<(i64, f64)> = u.iter().enumerate().filter(|x| x.0 != mu).map(|x| (*x.1, 1.0 / 7.0)).collect();
            let rp = compound_pmf(&sev, c.p * 7.0);
            let cut = (th / 2.0).floor() as i64;
            let want: f64 = (-H..H).filter(|&x| x > cut - u[mu]).map(|x| rp[(x + H) as usize]).sum();
            assert!((q[mu] - want).abs() < 1e-12, "mu {}: {} vs {}", mu, q[mu], want);
        }
        assert_eq!(q[0], q[5]);
    }

    #[test]
    fn wake_probabilities_match_a_random_membership_graph() {
        // at a fixed state, the fraction of a pattern's rows that the top-k read wakes, binned by the pattern's
        // overlap, against TRACK-C's q
        let (n, m, r, t) = (256usize, 20_000usize, 104usize, 3000usize);
        let mut rr = Rng::new(5);
        let w = 4;
        let pats: Vec<u64> = (0..t * w).map(|_| rr.next_u64()).collect();
        let g = Member::random(n, m, r, pats.clone(), &mut rr);
        let z: Vec<u64> = (0..w).map(|_| rr.next_u64()).collect();
        let u: Vec<i64> = (0..t).map(|mu| 128 - hd(&pats[mu * w..(mu + 1) * w], &z) as i64).collect();
        let c = Content::new(n, m, r);
        let k = c.k();
        let q = c.wake_probs(&u, Wake::Topk(k));
        let mut dots: Vec<(u32, i64)> = (0..g.rows()).map(|i| (i as u32, g.row(i).iter().map(|&mu| u[mu as usize]).sum())).collect();
        dots.select_nth_unstable_by(k - 1, |a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        let woken: std::collections::HashSet<u32> = dots[..k].iter().map(|x| x.0).collect();
        // per pattern: rows held and rows woken
        let mut held = vec![0usize; t];
        let mut woke = vec![0usize; t];
        for i in 0..g.rows() {
            for &mu in g.row(i) {
                held[mu as usize] += 1;
                if woken.contains(&(i as u32)) {
                    woke[mu as usize] += 1;
                }
            }
        }
        // compare totals of woken (pattern, row) pairs over patterns with u >= 16 and u < 16
        for (lo, hi) in [(16i64, 200i64), (-200, 16)] {
            let sel: Vec<usize> = (0..t).filter(|&mu| u[mu] >= lo && u[mu] < hi).collect();
            let meas: f64 = sel.iter().map(|&mu| woke[mu] as f64).sum();
            let pred: f64 = sel.iter().map(|&mu| held[mu] as f64 * q[mu]).sum();
            assert!((meas - pred).abs() < 4.0 * pred.sqrt() + 0.15 * pred + 3.0, "band {}..{}: {} vs {}", lo, hi, meas, pred);
        }
    }

    #[test]
    fn the_store_read_is_the_membership_read_of_its_own_rows() {
        // the summed awake rows are exactly the sum over stored patterns of (awake rows holding it) x pattern
        let (m, r, t) = (20_000usize, 105usize, 200usize);
        let mut rr = Rng::new(21);
        let ps: Vec<Vec<i8>> = (0..t).map(|_| random_pattern(256, &mut rr)).collect();
        let mut st = Store::new(256, m, r, 21);
        st.write_many(&ps);
        let pk: Vec<Vec<u64>> = ps.iter().map(|p| pack(p)).collect();
        let mut edges = Vec::new();
        for i in 0..m {
            for (mu, p) in pk.iter().enumerate() {
                if st.dist(i, p) <= r {
                    edges.push((i as u32, mu as u32));
                }
            }
        }
        let g = Member::from_edges(256, pk.concat(), edges);
        let k = (ball(256, r) * m as f64).round() as usize;
        let f = Fast::new(&st);
        for q in 0..8 {
            let cue = add_address_noise(&ps[q], 0.1 * (q % 4 + 1) as f64, &mut rr);
            let a = st.read_pulls_topk(&cue, 20, k).z;
            let (b, _, _, _) = g.read(&pack(&cue), Wake::Topk(k), 20);
            // the graph's rows are the store's nonempty rows in index order, so ties break the same way
            assert_eq!(pack(&a), b, "cue {}", q);
            let th = 60.0;
            assert_eq!(pack(&f.thresh(&cue, 20, th).z), g.read(&pack(&cue), Wake::Block(th), 20).0, "block cue {}", q);
        }
    }

    #[test]
    fn trace_gives_the_store_answer_and_a_clean_census() {
        let (m, r, t) = (20_000usize, 105usize, 30usize);
        let mut rr = Rng::new(8);
        let ps: Vec<Vec<i8>> = (0..t).map(|_| random_pattern(256, &mut rr)).collect();
        let mut st = Store::new(256, m, r, 8);
        st.write_many(&ps);
        let rows: Vec<u32> = (0..m).filter(|&i| st.filled[i]).map(|i| i as u32).collect();
        let k = (ball(256, r) * m as f64).round() as usize;
        let pk: Vec<Vec<u64>> = ps.iter().map(|p| pack(p)).collect();
        for q in 0..4 {
            let cue = add_address_noise(&ps[q], 0.2, &mut rr);
            let (z, rounds) = trace_topk(&st, &rows, &cue, 20, k);
            assert_eq!(z, st.read_pulls_topk(&cue, 20, k).z);
            assert!(overlap(&z, &ps[q]) > 0.99);
            // light load: the last round's woken rows are dominated by the target
            let c = census(&st, &pk, &rounds.last().unwrap().woken, &pack(&z));
            assert_eq!(c.top, q);
            assert!(c.share > 0.5 && c.top_dist <= 2, "{:?}", c);
        }
    }

    #[test]
    fn tail_fractions_and_calibration() {
        let probes: Vec<Vec<f64>> = (0..99).map(|i| vec![i as f64, (98 - i) as f64]).collect();
        let tc = TailCal::new(&probes);
        let f = tc.fractions(&[-1.0, 200.0]);
        assert!((f[0] - 0.01).abs() < 1e-12 && (f[1] - 1.0).abs() < 1e-12);
        assert!((tc.min(&[-1.0, 200.0]) - 0.01).abs() < 1e-12);
        let s: Vec<f64> = (0..1000).map(|i| i as f64).collect();
        assert_eq!(calibrate(&s, 0.01), 10.0);
        assert_eq!(s.iter().filter(|&&x| x < calibrate(&s, 0.01)).count(), 10);
    }

    #[test]
    fn the_contenttrack_statement_prints() {
        let out = crate::interp::Interp::default()
            .exec("model :m do\nend\nrun :m do\n  contenttrack hard-locations: 20000, load: 10, address-noise: 0.1, samples: 20\nend")
            .unwrap();
        assert!(out.join("\n").contains("TRACK-C predicts recall"), "{:?}", out);
        assert!(crate::interp::Interp::default().exec("model :m do\nend\nrun :m do\n  contenttrack address-noise: 0.7\nend").is_err());
    }
}
