//! SDMRADIUS: choose the SDM activation radius for the address-noise a read-address will carry, and scale the pulls read's wake
//! threshold with the store's density. Theory and reads only; the store is `sdmscale::Store`.
//!
//! SDMSCALE found that the SNR-optimal activation radius (sized for an undamaged read-address) leaves a 30%- or 40%-noisy
//! read-address sharing about one hard location with its pattern at every M, so capacity at those address-noise levels does not grow
//! with M. Here:
//!
//! * `Lazy` is the Bricken-Pehlevan S-map for any activation radius, computing the ball intersections only for the
//!   distances it visits (a fresh activation radius costs a few hundred intersections, not n + 1).
//! * `radius_for_address_noise` picks the activation radius whose S-map converges from a read-address D bits away at the largest
//!   stored count (the critical-distance-optimal activation radius for target address-noise D); `cd_optimal_radius` is
//!   Bricken-Pehlevan's d*_CD itself (the activation radius with the largest critical distance at a given load).
//! * `Lazy::p_converge` conditions the first read on the realised number of shared hard locations
//!   k ~ Poisson(M I(D)) (the S-map uses only the mean, which says nothing about the chance that k = 0).
//! * `density_threshold` and `Store::read_pulls_at` / `read_pulls_topk` give the pulls read a wake
//!   threshold scaled with the store's measured load per row.
//!
//! Equations with one-line readings are in `runs/sdmradius/REPORT_SDMRADIUS.md`.
//!
//! Every function here is KANERVA's: the S-map refinements and the activation radius choice live in `kanerva::smap`,
//! the density-scaled wake thresholds in `kanerva::store`. They are re-exported under their old names so
//! `sdmscale.rs`, the examples and the tests below keep their paths.

pub use kanerva::smap::{cd_optimal_radius, goal, radius_for_address_noise, radius_for_address_noise_poisson, search_window, Lazy};
pub use kanerva::store::{density_threshold, density_threshold_blocks, mean_row_load};

#[cfg(test)]
use crate::rng::Rng;
#[cfg(test)]
use kanerva::store::Store;
#[cfg(test)]
use kanerva::theory::ball;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sdmscale::{add_address_noise, overlap, random_pattern, SMap};

    #[test]
    fn lazy_agrees_with_the_full_smap() {
        let (n, m, r) = (256usize, 30_000usize, 105usize);
        let (full, lazy) = (SMap::new(n, m, r), Lazy::new(n, m, r));
        for &t in &[1usize, 10, 300] {
            for d in [0usize, 10, 26, 51, 77, 102, 128] {
                assert!((full.snr(d, t) - lazy.snr(d, t)).abs() < 1e-12);
            }
            assert_eq!(full.critical(t), lazy.critical(t));
        }
        for d in [26usize, 51, 77, 102] {
            assert_eq!(full.capacity(d, 6.4), lazy.capacity(d, 6.4));
        }
    }

    #[test]
    fn bricken_pehlevan_cd_optimal_radius_is_448() {
        // BP 2021 Appendix B.5 (wiki WIKI_SDR paper digest): n 1000, M 10^6, T 10^4 -> d*_CD = 448
        let (r, dc) = cd_optimal_radius(1000, 1_000_000, 10_000, 444, 452);
        assert_eq!(r, 448, "critical distance {}", dc);
    }

    #[test]
    fn poisson_prediction_is_at_most_the_mean_field_one_and_fails_with_no_shared_locations() {
        let l = Lazy::new(256, 100_000, 103);
        // at 40% address-noise the read-address shares ~0.7 hard locations: P(k = 0) = e^-0.7 ~ 0.5 caps the recall rate
        let d0 = 102;
        let lam = l.shared(d0);
        let p1 = l.p_converge(d0, 1, goal(256));
        assert!(p1 <= 1.0 - (-lam).exp() + 1e-9, "{} vs {}", p1, 1.0 - (-lam).exp());
        assert!(p1 > 0.3, "{}", p1);
        // the Poisson capacity never exceeds the mean-field capacity by more than one doubling
        for d in [26usize, 51, 77] {
            assert!(l.p_capacity(d, goal(256), 0.9) <= 2 * l.capacity(d, goal(256)) + 1);
        }
    }

    #[test]
    fn averaged_noise_first_moment_is_p_squared_and_second_exceeds_p_fourth() {
        for r in [100usize, 106, 112] {
            let l = Lazy::averaged(256, 100_000, r);
            let p = ball(256, r);
            let (e1, e2, em) = l.moments();
            assert!(em > 0.0, "r {}", r);
            assert!((e1 / (p * p) - 1.0).abs() < 1e-6, "r {} E[I] {} p^2 {}", r, e1, p * p);
            assert!(e2 > p.powi(4), "r {}", r);
            assert!(l.capacity(26, goal(256)) <= Lazy::new(256, 100_000, r).capacity(26, goal(256)));
        }
    }

    #[test]
    fn the_mirror_makes_a_crowded_wide_store_return_any_cue() {
        // M 10^6, r 103, T 10^4: mu / sigma ~ 2.8, so a read leaves about 1 bit of 256 changed: any read-address,
        // stored or not, is (nearly) a fixed point
        let l = Lazy::mirrored(256, 1_000_000, 103);
        let (mu, sd) = (l.mirror_field(10_000), l.noise_var(10_000).sqrt());
        assert!(mu / sd > 2.0 && mu / sd < 3.5, "{}", mu / sd);
        // distance from the read-address itself after one read of a never-stored read-address (no vote of its own): n Phi(-mu/sigma)
        assert!(l.step(0, 0.0, l.noise_var(10_000), 10_000) < 2.0);
        let a = Lazy::averaged(256, 1_000_000, 103);
        // with a fresh draw of the noise at every read the mirror holds right bits right, so this map predicts
        // MORE capacity than without it (2,002 against 461 at 30%): the mirror alone does not explain why
        // measured high-address-noise capacity falls short of these maps
        assert!(l.f_capacity(0.3, goal(256), 0.9) > a.f_capacity(0.3, goal(256), 0.9));
    }

    #[test]
    fn the_race_predicts_the_traced_forty_percent_failure_rates() {
        // traced (examples/debug run, 600 read-addresses each): M 10^5 r 107 fails 37/600 at T 2 and 95/600 at T 5;
        // M 10^6 r 104 fails 66/600 at T 5
        for &(m, r, t, meas) in &[(100_000usize, 107usize, 2usize, 37.0 / 600.0), (100_000, 107, 5, 95.0 / 600.0), (1_000_000, 104, 5, 66.0 / 600.0)] {
            let f = 1.0 - Lazy::new(256, m, r).p_converge_race(0.4, t, goal(256), 3000, 5);
            assert!((f - meas).abs() < 0.06, "M {} r {} T {}: race {} measured {}", m, r, t, f, meas);
        }
        // the averaged S-map says these reads almost never fail
        assert!(Lazy::averaged(256, 100_000, 107).p_converge_flips(0.4, 5, goal(256)) > 0.99);
    }

    #[test]
    fn flip_damage_spreads_the_cue_distance_and_lowers_the_high_bar() {
        let l = Lazy::averaged(256, 100_000, 106);
        // a fixed-distance read-address and a flip-noisy read-address agree at the median, not at the 90% bar
        let (a, b) = (l.p_capacity(77, goal(256), 0.9), l.f_capacity(0.3, goal(256), 0.9));
        assert!(b < a, "{} {}", a, b);
        assert!((l.p_converge_flips(0.0, 5, goal(256)) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn a_damage_radius_is_wider_and_tolerates_more() {
        let (lo, hi) = search_window(256, 100_000);
        let (r10, _) = radius_for_address_noise(256, 100_000, 0.1, lo, hi);
        let (r30, c30) = radius_for_address_noise(256, 100_000, 0.3, lo, hi);
        assert!(r30 > r10, "{} {}", r30, r10);
        assert!(c30 > Lazy::new(256, 100_000, 103).capacity(77, goal(256)));
        // and a store at that activation radius recalls a 30%-noisy read-address that the SNR activation radius cannot
        let mut st = Store::new(256, 100_000, r30, 3);
        let mut rr = Rng::new(4);
        let ps: Vec<Vec<i8>> = (0..10).map(|_| random_pattern(256, &mut rr)).collect();
        st.write_many(&ps);
        let ok = ps.iter().filter(|p| overlap(&st.read_addresses(&add_address_noise(p, 0.3, &mut rr), 20).z, p) >= 0.95).count();
        assert!(ok >= 8, "{}", ok);
    }

    #[test]
    fn the_block_threshold_stops_a_whole_other_pattern_waking() {
        // two stored patterns: every row holds exactly one, so the row-level threshold lets the other
        // pattern's whole block wake about 5% of the time; the block threshold about 1%
        let mut st = Store::new(256, 30_000, 105, 1);
        let mut rr = Rng::new(7);
        let ps: Vec<Vec<i8>> = (0..2).map(|_| random_pattern(256, &mut rr)).collect();
        st.write_many(&ps);
        let (t1, _, _) = density_threshold(&st, 0.1);
        let (t2, _, _) = density_threshold_blocks(&st, 0.1, 0.01);
        assert!(t2 > t1);
        let fails = |th: f64, rr: &mut Rng| (0..200).filter(|k| overlap(&st.read_pulls_at(&add_address_noise(&ps[k % 2], 0.2, rr), 20, th).z, &ps[k % 2]) < 0.95).count();
        let (f1, f2) = (fails(t1, &mut rr), fails(t2, &mut rr));
        // eps_pat 0.01 with one other pattern: the block wakes about 1% of the time (2 of 200 here)
        assert!(f1 >= 3 * f2.max(1) && f2 <= 4, "row-level {} block-level {}", f1, f2);
    }

    #[test]
    fn density_threshold_scales_with_load() {
        let mut st = Store::new(256, 20_000, 106, 2);
        let mut rr = Rng::new(5);
        let few: Vec<Vec<i8>> = (0..5).map(|_| random_pattern(256, &mut rr)).collect();
        st.write_many(&few);
        let (t1, _, l1) = density_threshold(&st, 0.1);
        let many: Vec<Vec<i8>> = (0..400).map(|_| random_pattern(256, &mut rr)).collect();
        st.write_many(&many);
        let (t2, _, l2) = density_threshold(&st, 0.1);
        assert!(l2 > l1 && t2 > t1, "{} {} {} {}", l1, l2, t1, t2);
        // the rows' load is measured, not assumed: 405 writes at p ~ 0.0047 put ~2 patterns in a row
        assert!((1.0..4.0).contains(&l2), "{}", l2);
    }
}

