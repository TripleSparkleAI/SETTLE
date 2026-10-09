//! ZOOHARD: harder instances for the zoo, and the plain-code solvers that decide them exactly.
//! No statements of its own (the language additions, `factor ..., encoding: :columns` and `anneal_each`, live
//! in `zoo.rs`); this file is the instrument: generators, exact deciders and helpers the measurement example
//! `examples/zoohard_measure.rs` uses. Report: `runs/zoohard/REPORT_ZOOHARD.md`.
//!
//! - 9x9 sudoku: a backtracking solver (candidate bitmasks, naked and hidden singles, fewest-candidates branching)
//!   that counts solutions up to a limit and the branch nodes it needed; a generator (a random full grid, then
//!   cells removed in random order while the solution stays unique); a rating: SINGLES when naked and hidden
//!   singles alone finish the grid, GUESS when they stall and the search must branch.
//! - graph 3-colouring: G(n, m) random graphs and an exact DSATUR backtracking decider with a node limit
//!   (answers colourable, not colourable, or undecided).
//! - factoring: a primality test and balanced semiprimes.

use crate::rng::Rng;

// ---------------------------------------------------------------------------------------------------------
// SUDOKU 9x9
// ---------------------------------------------------------------------------------------------------------

/// The 27 units (rows, columns, boxes) as cell lists.
pub fn units() -> Vec<[usize; 9]> {
    let mut u = Vec::new();
    for r in 0..9 {
        u.push(std::array::from_fn(|c| r * 9 + c));
    }
    for c in 0..9 {
        u.push(std::array::from_fn(|r| r * 9 + c));
    }
    for b in 0..9 {
        u.push(std::array::from_fn(|k| (b / 3 * 3 + k / 3) * 9 + b % 3 * 3 + k % 3));
    }
    u
}

fn peers() -> Vec<Vec<usize>> {
    (0..81)
        .map(|i| {
            (0..81)
                .filter(|&j| j != i && (j / 9 == i / 9 || j % 9 == i % 9 || (j / 27 == i / 27 && j % 9 / 3 == i % 9 / 3)))
                .collect()
        })
        .collect()
}

pub struct Sudoku {
    units: Vec<[usize; 9]>,
    peers: Vec<Vec<usize>>,
}

impl Default for Sudoku {
    fn default() -> Self {
        Sudoku { units: units(), peers: peers() }
    }
}

/// Result of a search: solutions found (up to the limit), branch nodes tried, the first solution.
pub struct Search {
    pub count: usize,
    pub nodes: u64,
    pub first: Option<[u8; 81]>,
}

impl Sudoku {
    /// Digits (bits 1..=9) a cell may still take.
    fn cands(&self, g: &[u8; 81], i: usize) -> u16 {
        let mut m: u16 = 0b11_1111_1110;
        for &p in &self.peers[i] {
            m &= !(1 << g[p]);
        }
        m
    }

    /// Place naked and hidden singles until none remain. False on a contradiction.
    pub fn propagate(&self, g: &mut [u8; 81]) -> bool {
        loop {
            let mut changed = false;
            for i in 0..81 {
                if g[i] == 0 {
                    let m = self.cands(g, i);
                    if m == 0 {
                        return false;
                    }
                    if m.count_ones() == 1 {
                        g[i] = m.trailing_zeros() as u8;
                        changed = true;
                    }
                }
            }
            for u in &self.units {
                for d in 1..=9u8 {
                    if u.iter().any(|&i| g[i] == d) {
                        if u.iter().filter(|&&i| g[i] == d).count() > 1 {
                            return false;
                        }
                        continue;
                    }
                    let spots: Vec<usize> = u.iter().copied().filter(|&i| g[i] == 0 && self.cands(g, i) & (1 << d) != 0).collect();
                    match spots.len() {
                        0 => return false,
                        1 => {
                            g[spots[0]] = d;
                            changed = true;
                        }
                        _ => {}
                    }
                }
            }
            if !changed {
                return true;
            }
        }
    }

    fn search(&self, g: &[u8; 81], limit: usize, out: &mut Search, order: Option<&mut Rng>) {
        let mut g = *g;
        if !self.propagate(&mut g) {
            return;
        }
        let mut pick = None;
        let mut fewest = 10;
        for i in 0..81 {
            if g[i] == 0 {
                let c = self.cands(&g, i).count_ones();
                if c < fewest {
                    fewest = c;
                    pick = Some(i);
                }
            }
        }
        let i = match pick {
            None => {
                out.count += 1;
                if out.first.is_none() {
                    out.first = Some(g);
                }
                return;
            }
            Some(i) => i,
        };
        let m = self.cands(&g, i);
        let mut ds: Vec<u8> = (1..=9u8).filter(|&d| m & (1 << d) != 0).collect();
        let mut order = order;
        if let Some(r) = order.as_deref_mut() {
            for k in (1..ds.len()).rev() {
                let j = r.below(k + 1);
                ds.swap(k, j);
            }
        }
        for d in ds {
            out.nodes += 1;
            let mut h = g;
            h[i] = d;
            self.search(&h, limit, out, order.as_deref_mut());
            if out.count >= limit {
                return;
            }
        }
    }

    /// Count solutions up to `limit`, with the branch nodes the search needed.
    pub fn solve(&self, g: &[u8; 81], limit: usize) -> Search {
        let mut out = Search { count: 0, nodes: 0, first: None };
        // givens that already clash have no solution
        for u in &self.units {
            for d in 1..=9u8 {
                if u.iter().filter(|&&i| g[i] == d).count() > 1 {
                    return out;
                }
            }
        }
        self.search(g, limit, &mut out, None);
        out
    }

    /// True when naked and hidden singles alone fill the whole grid.
    pub fn singles_only(&self, g: &[u8; 81]) -> bool {
        let mut h = *g;
        self.propagate(&mut h) && h.iter().all(|&d| d > 0)
    }

    /// A random complete grid.
    pub fn full_grid(&self, r: &mut Rng) -> [u8; 81] {
        let mut out = Search { count: 0, nodes: 0, first: None };
        self.search(&[0; 81], 1, &mut out, Some(r));
        out.first.expect("an empty grid has solutions")
    }

    /// Remove cells of `full` in random order while the puzzle keeps exactly one solution. Stops once only
    /// `stop_at` givens remain (None: go on until no cell can be removed, a minimal puzzle).
    pub fn carve(&self, full: &[u8; 81], r: &mut Rng, stop_at: Option<usize>) -> [u8; 81] {
        let mut g = *full;
        let mut order: Vec<usize> = (0..81).collect();
        for k in (1..81).rev() {
            let j = r.below(k + 1);
            order.swap(k, j);
        }
        for i in order {
            if let Some(s) = stop_at {
                if g.iter().filter(|&&d| d > 0).count() <= s {
                    break;
                }
            }
            let keep = g[i];
            g[i] = 0;
            if self.solve(&g, 2).count != 1 {
                g[i] = keep;
            }
        }
        g
    }
}

pub fn grid_text(g: &[u8; 81]) -> String {
    (0..9)
        .map(|r| (0..9).map(|c| if g[r * 9 + c] == 0 { '.' } else { (b'0' + g[r * 9 + c]) as char }).collect::<String>())
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn grid_of(s: &str) -> [u8; 81] {
    let v: Vec<u8> = s.chars().filter(|c| !c.is_whitespace()).map(|c| if c == '.' { 0 } else { c as u8 - b'0' }).collect();
    v.try_into().expect("81 cells")
}

// ---------------------------------------------------------------------------------------------------------
// GRAPH COLOURING
// ---------------------------------------------------------------------------------------------------------

/// A G(n, m) random graph: m distinct edges drawn uniformly (average degree 2m/n). Isolated nodes are allowed.
pub fn gnm(n: usize, m: usize, r: &mut Rng) -> Vec<(usize, usize)> {
    let mut set = std::collections::BTreeSet::new();
    while set.len() < m {
        let (a, b) = (r.below(n), r.below(n));
        if a != b {
            set.insert((a.min(b), a.max(b)));
        }
    }
    set.into_iter().collect()
}

/// Exact k-colourability by DSATUR backtracking. Some(true) colourable, Some(false) not, None when the node
/// limit ran out first.
pub fn colourable(n: usize, edges: &[(usize, usize)], k: usize, node_limit: u64) -> Option<bool> {
    let mut adj = vec![Vec::new(); n];
    for &(a, b) in edges {
        adj[a].push(b);
        adj[b].push(a);
    }
    let mut col = vec![usize::MAX; n];
    let mut nodes = 0u64;
    fn go(adj: &[Vec<usize>], col: &mut Vec<usize>, k: usize, left: usize, used: usize, nodes: &mut u64, limit: u64) -> Option<bool> {
        if left == 0 {
            return Some(true);
        }
        *nodes += 1;
        if *nodes > limit {
            return None;
        }
        // the uncoloured node with the most distinct neighbour colours, ties by uncoloured degree
        let mut best = (usize::MAX, 0usize, 0usize);
        for u in 0..col.len() {
            if col[u] != usize::MAX {
                continue;
            }
            let mut seen = 0u32;
            let mut free_deg = 0;
            for &v in &adj[u] {
                if col[v] == usize::MAX {
                    free_deg += 1;
                } else {
                    seen |= 1 << col[v];
                }
            }
            let key = (seen.count_ones() as usize, free_deg);
            if best.0 == usize::MAX || key > (best.1, best.2) {
                best = (u, key.0, key.1);
            }
        }
        let u = best.0;
        let mut seen = 0u32;
        for &v in &adj[u] {
            if col[v] != usize::MAX {
                seen |= 1 << col[v];
            }
        }
        // a colour never used before is tried only once (colour symmetry)
        for c in 0..k.min(used + 1) {
            if seen & (1 << c) == 0 {
                col[u] = c;
                match go(adj, col, k, left - 1, used.max(c + 1), nodes, limit) {
                    Some(true) => return Some(true),
                    None => return None,
                    Some(false) => {}
                }
                col[u] = usize::MAX;
            }
        }
        Some(false)
    }
    go(&adj, &mut col, k, n, 0, &mut nodes, node_limit)
}

pub fn edges_text(edges: &[(usize, usize)]) -> String {
    edges.iter().map(|(a, b)| format!("v{}-v{}", a, b)).collect::<Vec<_>>().join(" ")
}

// ---------------------------------------------------------------------------------------------------------
// FACTORING
// ---------------------------------------------------------------------------------------------------------

pub fn is_prime(n: u64) -> bool {
    if n < 2 {
        return false;
    }
    let mut d = 2;
    while d * d <= n {
        if n.is_multiple_of(d) {
            return false;
        }
        d += 1;
    }
    true
}

/// The largest prime below x.
pub fn prime_below(x: u64) -> u64 {
    (2..x).rev().find(|&p| is_prime(p)).expect("a prime below x")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::zoo::{column_layout, columns_assign, factor_check, factor_columns_qubo, factor_decode, factor_layout, factor_qubo, factor_widths, sudoku_check};

    // A well-known published 9x9 (Wikipedia's example grid) used only as a solver test.
    const EASY: &str = "53..7.... 6..195... .98....6. 8...6...3 4..8.3..1 7...2...6 .6....28. ...419..5 ....8..79";
    const EASY_SOLVED: &str = "534678912 672195348 198342567 859761423 426853791 713924856 961537284 287419635 345286179";

    #[test]
    fn the_backtracking_solver_finds_the_known_solution_and_counts_uniqueness() {
        let s = Sudoku::default();
        let r = s.solve(&grid_of(EASY), 2);
        assert_eq!(r.count, 1);
        assert_eq!(grid_text(&r.first.unwrap()), EASY_SOLVED);
        assert!(s.singles_only(&grid_of(EASY)));
        // negative control: an empty grid has many solutions, and two clashing givens have none
        assert_eq!(s.solve(&[0; 81], 2).count, 2);
        let mut bad = grid_of(EASY);
        bad[1] = 5;
        assert_eq!(s.solve(&bad, 2).count, 0);
    }

    #[test]
    fn carved_puzzles_are_unique_minimal_and_their_solutions_pass_the_zoo_checker() {
        let s = Sudoku::default();
        let mut r = Rng::new(4);
        let full = s.full_grid(&mut r);
        let full_u: Vec<u8> = full.to_vec();
        assert!(sudoku_check(9, &full_u, &full_u).is_ok());
        let p = s.carve(&full, &mut r, None);
        assert_eq!(s.solve(&p, 2).count, 1);
        for i in 0..81 {
            if p[i] > 0 {
                let mut q = p;
                q[i] = 0;
                assert!(s.solve(&q, 2).count > 1, "minimal: removing any given breaks uniqueness");
            }
        }
        let p30 = s.carve(&full, &mut Rng::new(5), Some(30));
        assert_eq!(p30.iter().filter(|&&d| d > 0).count(), 30);
    }

    #[test]
    fn the_colouring_decider_agrees_with_small_known_cases() {
        let tri = [(0, 1), (1, 2), (0, 2)];
        assert_eq!(colourable(3, &tri, 2, 1_000), Some(false));
        assert_eq!(colourable(3, &tri, 3, 1_000), Some(true));
        let k4: Vec<(usize, usize)> = (0..4).flat_map(|a| (a + 1..4).map(move |b| (a, b))).collect();
        assert_eq!(colourable(4, &k4, 3, 1_000), Some(false));
        assert_eq!(colourable(4, &k4, 4, 1_000), Some(true));
        // the odd wheel W5 (a 5-cycle plus a hub) needs 4 colours
        let w5 = [(0, 1), (1, 2), (2, 3), (3, 4), (4, 0), (5, 0), (5, 1), (5, 2), (5, 3), (5, 4)];
        assert_eq!(colourable(6, &w5, 3, 1_000), Some(false));
        // agrees with the zoo's exhaustive search on random small graphs, up to past the threshold
        let mut r = Rng::new(3);
        for t in 0..60 {
            let (n, m) = if t < 30 { (9, 12 + t % 8) } else { (18, 30 + t % 20) };
            let e = gnm(n, m, &mut r);
            let g = crate::zoo::parse_edges(&edges_text(&e)).unwrap();
            // the zoo's search only sees nodes that appear in an edge; isolated nodes never matter
            assert_eq!(colourable(n, &e, 3, 1_000_000), Some(crate::zoo::colouring_exists(&g, 3)), "n {} m {}", n, m);
        }
    }

    /// Slow cross-check at n = 40 against the zoo's plain index-order search, near the threshold.
    #[test]
    #[ignore]
    fn the_decider_agrees_at_forty_nodes() {
        let mut both = [0usize; 2];
        for t in 0..20u64 {
            let mut r = Rng::new(40_000 + t);
            let e = gnm(40, 84 + (t as usize % 12), &mut r);
            let g = crate::zoo::parse_edges(&edges_text(&e)).unwrap();
            let want = crate::zoo::colouring_exists(&g, 3);
            assert_eq!(colourable(40, &e, 3, 100_000_000), Some(want));
            both[want as usize] += 1;
        }
        println!("colourable {} not {}", both[1], both[0]);
        assert!(both[0] > 0 && both[1] > 0, "both verdicts exercised");
    }

    #[test]
    fn column_energy_is_zero_at_every_factorisation_and_positive_elsewhere() {
        for (n, p, q) in [(15u64, 3u64, 5u64), (143, 11, 13), (323, 17, 19), (3599, 59, 61), (10403, 101, 103), (644773, 797, 809)] {
            let lay = column_layout(n);
            let qq = factor_columns_qubo(n, &lay, 2.0);
            for (a, b) in [(p, q), (q, p)] {
                if let Some(y) = columns_assign(n, &lay, a, b) {
                    assert_eq!(qq.energy(&y), 0.0, "N={} p={} q={}", n, a, b);
                    let (dp, dq) = factor_decode(&lay.base, &y);
                    assert!(factor_check(n, dp, dq).is_ok());
                }
            }
            assert!(columns_assign(n, &lay, p, q).is_some() || columns_assign(n, &lay, q, p).is_some());
            // random arrangements never go below zero, and none that decodes wrongly reaches zero
            let mut r = Rng::new(n);
            for _ in 0..2_000 {
                let y: Vec<bool> = (0..lay.nvars).map(|_| r.unit() < 0.5).collect();
                let e = qq.energy(&y);
                assert!(e >= 0.0);
                let (dp, dq) = factor_decode(&lay.base, &y);
                if e == 0.0 {
                    assert!(factor_check(n, dp, dq).is_ok());
                }
            }
        }
    }

    #[test]
    fn brute_force_confirms_the_column_ground_state_for_small_numbers() {
        for (n, want) in [(15u64, true), (21, true), (35, true), (13, false), (31, false)] {
            let lay = column_layout(n);
            assert!(lay.nvars <= 22, "N={} has {} things", n, lay.nvars);
            let qq = factor_columns_qubo(n, &lay, 2.0);
            let mut best = (f64::INFINITY, false);
            for bits in 0u64..(1 << lay.nvars) {
                let y: Vec<bool> = (0..lay.nvars).map(|k| (bits >> k) & 1 == 1).collect();
                let e = qq.energy(&y);
                if e < best.0 {
                    let (p, q) = factor_decode(&lay.base, &y);
                    best = (e, factor_check(n, p, q).is_ok());
                }
            }
            if want {
                assert_eq!(best, (0.0, true), "N={}", n);
            } else {
                assert!(best.0 > 0.0, "prime {} must stay above zero", n);
            }
        }
    }

    #[test]
    fn column_pulls_stay_close_in_size_where_rosenberg_pulls_blow_up() {
        // the spread of coefficient sizes (largest / smallest non-zero) for both encodings
        let spread = |q: &crate::zoo::Qubo| {
            let v: Vec<f64> = q.lin.iter().chain(q.quad.values()).map(|x| x.abs()).filter(|&x| x > 0.0).collect();
            v.iter().cloned().fold(0.0, f64::max) / v.iter().cloned().fold(f64::INFINITY, f64::min)
        };
        let n = 10403u64;
        let (pb, qb) = factor_widths(n);
        let ros = factor_qubo(n, &factor_layout(pb, qb), (1u64 << (pb + qb - 2)) as f64);
        let col = factor_columns_qubo(n, &column_layout(n), 2.0);
        assert!(spread(&col) < 1_000.0 && spread(&ros) > 1e6, "columns {} rosenberg {}", spread(&col), spread(&ros));
    }

    #[test]
    fn primes_and_semiprimes() {
        assert!(is_prime(10007) && !is_prime(10403) && is_prime(101) && is_prime(103));
        assert_eq!(prime_below(10403), 10399);
    }
}
