//! ZOO: hard puzzles written as settling. Each puzzle statement builds the pulls, pushes and leans whose
//! calmest arrangement is the answer; `anneal` looks for it; `x.solution` decodes the calmest arrangement
//! found and checks it with plain code, never with the energy.
//!
//! ```text
//! model :zoo do
//!   sudoku :s, size: 4, given: "1... .4.. ..4. ...1"
//!   colouring :g, colours: 3, edges: "a-b b-c c-a c-d"
//!   maxcut :m, edges: "a-b b-c c-d d-a a-c:2"      # an optional :weight after an edge
//!   factor :f, number: 15
//!   nonogram :n, rows: "1 1/5/5/3/1", cols: "2/4/4/4/2"   # a heart
//! end
//! run :zoo do
//!   anneal 5_000, seed: 1
//!   s.solution        # the grid, then VALID or NOT VALID with the first broken rule
//!   g.solution        # a colour per node, then PROPER or NOT PROPER
//!   m.solution        # the two sides, the cut, and the exact best cut by brute force when small
//!   f.solution        # p x q, then VALID or NOT VALID
//!   n.solution        # the picture, then VALID or NOT VALID by reading each line back
//! end
//! ```
//!
//! Every puzzle is written first as a QUBO (energy over 0/1 variables y: a linear part, a pair part and a
//! constant) and then turned into leans and pulls on ±1 things with y = (1 + s) / 2. Equations and their
//! plain readings: `runs/settlezoo/REPORT_SETTLEZOO.md`.
//!
//! - sudoku: one thing per (cell, digit). "Exactly one" penalties A(Σy - 1)² on every cell, and on every
//!   digit within every row, column and box. A given digit gets a lean of strength G toward yes (default
//!   G = 4A: at G = A a pilot anneal settled into a valid grid that simply dropped one given, since
//!   dropping a given costs only G while moving between valid grids crosses a high ridge).
//! - colouring: one thing per (node, colour). "Exactly one" per node, and a penalty B·y_uc·y_vc for every
//!   edge u-v and colour c (two ends painted the same).
//! - maxcut: one thing per node; every edge pushes its two ends apart by its weight. No QUBO needed.
//! - factor: bits of p and q (lowest bit fixed at 1, both odd), plus one helper thing z_ij per bit product
//!   p_i·q_j, held to the product by the Rosenberg penalty λ(3z + xy - 2xz - 2yz). The energy is
//!   (N - Σ 2^(i+j) z_ij)² plus those penalties, so a pairwise-only machine can multiply.
//!
//! - factor, `encoding: :columns` (ZOOHARD): the same bits and helpers plus carry bits; each column of the
//!   long multiplication is squared on its own, (Σ_{i+j=k} p_i q_j + carries in - N_k - Σ_m 2^m c_{k,m})², so
//!   the pulls stay within a factor of about 50 in size where the Rosenberg form's grow like N².
//!
//! - nonogram (PUZZLEFEATURE): one thing per cell, plus each block's start as a row of "past here" bits
//!   u_p = [start > p] that must read 1..1 0..0 (a single domain wall), so moving a block one cell is one flip.
//!   Costs: one wall per block, blocks in order with a gap, and each cell equal to the blocks covering it, for its
//!   row and for its column. Zero exactly at a picture whose every line reads back as its clue.
//!
//! All five keep their layout on the model under the note `zoo:<name>`, so several puzzles can share one model.
//! `anneal_each` walks the same path as `anneal` but lets every puzzle keep the block of its own things that was
//! calmest for its own energy, so puzzles sharing a model no longer judge each other's best.

use crate::ext::{Claim, Ctx, Ext};
use crate::lex::{err, kw, kwargs, num, only, text, SettleError, Tok, whole};
use crate::model::{Model, State};
use std::collections::HashMap;

pub struct Zoo;

// ---------------------------------------------------------------------------------------------------------
// QUBO: energy over 0/1 variables, and its translation into leans and pulls
// ---------------------------------------------------------------------------------------------------------

/// E(y) = Σ lin_i y_i + Σ_{i<j} quad_ij y_i y_j + c, over n variables y in {0, 1}.
#[derive(Clone, Debug, Default)]
pub struct Qubo {
    pub lin: Vec<f64>,
    pub quad: HashMap<(usize, usize), f64>,
    pub c: f64,
}

impl Qubo {
    pub fn new(n: usize) -> Self {
        Qubo { lin: vec![0.0; n], quad: HashMap::new(), c: 0.0 }
    }
    pub fn add_quad(&mut self, i: usize, j: usize, w: f64) {
        if i == j {
            self.lin[i] += w; // y² = y for a 0/1 variable
        } else {
            *self.quad.entry((i.min(j), i.max(j))).or_insert(0.0) += w;
        }
    }
    /// A(Σ y - 1)² = A(2 Σ_{i<j} y_i y_j - Σ y_i + 1): zero exactly when one of `vars` is on.
    pub fn exactly_one(&mut self, vars: &[usize], a: f64) {
        for (k, &i) in vars.iter().enumerate() {
            self.lin[i] -= a;
            for &j in &vars[k + 1..] {
                self.add_quad(i, j, 2.0 * a);
            }
        }
        self.c += a;
    }
    pub fn energy(&self, y: &[bool]) -> f64 {
        let mut e = self.c;
        for (i, &a) in self.lin.iter().enumerate() {
            if y[i] {
                e += a;
            }
        }
        for (&(i, j), &b) in &self.quad {
            if y[i] && y[j] {
                e += b;
            }
        }
        e
    }
    /// Write onto things `start..start+n` of the model. With y = (1+s)/2 and model energy -Σ h s - Σ J s s:
    /// h_i gains -(a_i/2 + Σ_j b_ij/4), J_ij gains -b_ij/4. Returns the constant that makes
    /// model energy + constant = QUBO energy.
    pub fn apply(&self, m: &mut Model, start: usize) -> f64 {
        let mut off = self.c;
        for (i, &a) in self.lin.iter().enumerate() {
            m.h[start + i] -= a / 2.0;
            off += a / 2.0;
        }
        let mut pairs: Vec<(&(usize, usize), &f64)> = self.quad.iter().collect();
        pairs.sort_by_key(|p| *p.0); // a fixed order keeps runs reproducible
        for (&(i, j), &b) in pairs {
            if b == 0.0 {
                continue;
            }
            m.h[start + i] -= b / 4.0;
            m.h[start + j] -= b / 4.0;
            m.couple(start + i, start + j, -b / 4.0);
            off += b / 4.0;
        }
        off
    }
}

// ---------------------------------------------------------------------------------------------------------
// Notes: where each puzzle's things live on the model
// ---------------------------------------------------------------------------------------------------------

fn note_key(name: &str) -> String {
    format!("zoo:{}", name)
}

fn fresh(m: &Model, name: &str, ln: usize) -> Result<(), SettleError> {
    if m.notes.contains_key(&note_key(name)) {
        return err(ln, format!("puzzle :{} is already declared", name));
    }
    Ok(())
}

fn add_block(m: &mut Model, names: &[String]) -> usize {
    let start = m.len();
    for n in names {
        m.add(n);
    }
    start
}

fn bits_of(s: &[f64]) -> Vec<bool> {
    s.iter().map(|&v| v > 0.0).collect()
}

// ---------------------------------------------------------------------------------------------------------
// SUDOKU
// ---------------------------------------------------------------------------------------------------------

/// Parse givens: n*n characters after dropping spaces; '.' or '0' is empty, '1'..'9' a digit.
pub fn parse_givens(g: &str, n: usize) -> Result<Vec<u8>, String> {
    let cells: Vec<char> = g.chars().filter(|c| !c.is_whitespace()).collect();
    if cells.len() != n * n {
        return Err(format!("a {0}x{0} sudoku needs {1} cells in given:, found {2}", n, n * n, cells.len()));
    }
    cells
        .iter()
        .map(|&c| match c {
            '.' | '0' => Ok(0),
            d if d.is_ascii_digit() && (d as u8 - b'0') as usize <= n => Ok(d as u8 - b'0'),
            d => Err(format!("'{}' is not a digit from 1 to {} or '.'", d, n)),
        })
        .collect()
}

fn box_side(n: usize) -> usize {
    (n as f64).sqrt().round() as usize
}

/// The sudoku QUBO: variable (r*n + c)*n + (d-1) is "cell (r,c) holds digit d".
pub fn sudoku_qubo(n: usize, givens: &[u8], a: f64, g_lean: f64) -> Qubo {
    let b = box_side(n);
    let v = |r: usize, c: usize, d: usize| (r * n + c) * n + d;
    let mut q = Qubo::new(n * n * n);
    for r in 0..n {
        for c in 0..n {
            q.exactly_one(&(0..n).map(|d| v(r, c, d)).collect::<Vec<_>>(), a); // one digit per cell
        }
    }
    for d in 0..n {
        for r in 0..n {
            q.exactly_one(&(0..n).map(|c| v(r, c, d)).collect::<Vec<_>>(), a); // each digit once per row
        }
        for c in 0..n {
            q.exactly_one(&(0..n).map(|r| v(r, c, d)).collect::<Vec<_>>(), a); // once per column
        }
        for br in 0..b {
            for bc in 0..b {
                let cells: Vec<usize> = (0..n).map(|k| v(br * b + k / b, bc * b + k % b, d)).collect();
                q.exactly_one(&cells, a); // once per box
            }
        }
    }
    for (i, &g) in givens.iter().enumerate() {
        if g > 0 {
            q.lin[i * n + (g as usize - 1)] -= g_lean; // a lean toward the given digit
        }
    }
    q
}

/// Decode a grid from yes/no per (cell, digit): the digit if exactly one is on, 0 if none, 255 if several.
pub fn sudoku_decode(n: usize, y: &[bool]) -> Vec<u8> {
    (0..n * n)
        .map(|cell| {
            let on: Vec<usize> = (0..n).filter(|&d| y[cell * n + d]).collect();
            match on.len() {
                0 => 0,
                1 => on[0] as u8 + 1,
                _ => 255,
            }
        })
        .collect()
}

/// Plain rule check: every cell one digit, no repeats in any row, column or box, every given kept.
pub fn sudoku_check(n: usize, givens: &[u8], grid: &[u8]) -> Result<(), String> {
    let b = box_side(n);
    for (i, &g) in grid.iter().enumerate() {
        match g {
            0 => return Err(format!("cell r{}c{} has no digit", i / n + 1, i % n + 1)),
            255 => return Err(format!("cell r{}c{} has several digits", i / n + 1, i % n + 1)),
            _ => {}
        }
        if givens[i] > 0 && givens[i] != g {
            return Err(format!("cell r{}c{} was given {} but holds {}", i / n + 1, i % n + 1, givens[i], g));
        }
    }
    let groups: Vec<(String, Vec<usize>)> = (0..n)
        .flat_map(|k| {
            let (br, bc) = (k / b, k % b);
            [
                (format!("row {}", k + 1), (0..n).map(|c| k * n + c).collect()),
                (format!("column {}", k + 1), (0..n).map(|r| r * n + k).collect()),
                (format!("box {}", k + 1), (0..n).map(|j| (br * b + j / b) * n + bc * b + j % b).collect()),
            ]
        })
        .collect();
    for (label, cells) in groups {
        let mut seen = vec![false; n + 1];
        for &i in &cells {
            if seen[grid[i] as usize] {
                return Err(format!("{} has two {}s", label, grid[i]));
            }
            seen[grid[i] as usize] = true;
        }
    }
    Ok(())
}

fn declare_sudoku(m: &mut Model, name: &str, rest: &[Tok], ln: usize) -> Result<(), SettleError> {
    fresh(m, name, ln)?;
    let kv = kwargs(rest, ln)?;
    only(&kv, &["size", "given", "by", "given_by"], "sudoku", ln)?;
    let n = whole(kw(&kv, "size").map(|v| num(v, ln)).transpose()?.unwrap_or(9.0), 0.0, f64::INFINITY, "size:", ln)?;
    if ![4, 9].contains(&n) {
        return err(ln, "sudoku size is 4 or 9");
    }
    let given = match kw(&kv, "given") {
        Some(t) => text(t, ln)?,
        None => ".".repeat(n * n),
    };
    let a = kw(&kv, "by").map(|v| num(v, ln)).transpose()?.unwrap_or(1.0);
    let gl = kw(&kv, "given_by").map(|v| num(v, ln)).transpose()?.unwrap_or(4.0 * a);
    if a <= 0.0 || gl <= 0.0 {
        return err(ln, "by: and given_by: must be above zero");
    }
    let givens = parse_givens(&given, n).or_else(|e| err(ln, e))?;
    let q = sudoku_qubo(n, &givens, a, gl);
    let names: Vec<String> =
        (0..n * n * n).map(|v| format!("{}_r{}c{}_{}", name, v / n / n + 1, v / n % n + 1, v % n + 1)).collect();
    let start = add_block(m, &names);
    q.apply(m, start);
    let gs: String = givens.iter().map(|&g| if g == 0 { '.' } else { (b'0' + g) as char }).collect();
    m.notes.insert(note_key(name), (vec![start as f64, n as f64], vec!["sudoku".into(), gs]));
    Ok(())
}

// ---------------------------------------------------------------------------------------------------------
// GRAPHS: colouring and max-cut share an edge list
// ---------------------------------------------------------------------------------------------------------

pub struct Graph {
    pub nodes: Vec<String>,
    pub edges: Vec<(usize, usize, f64)>,
}

/// "a-b b-c c-a:2": edges separated by spaces, an optional :weight (default 1).
pub fn parse_edges(s: &str) -> Result<Graph, String> {
    let mut g = Graph { nodes: Vec::new(), edges: Vec::new() };
    let mut idx: HashMap<String, usize> = HashMap::new();
    for item in s.split_whitespace() {
        let (pair, w) = match item.split_once(':') {
            Some((p, w)) => (p, w.parse::<f64>().map_err(|_| format!("'{}' has a weight that is not a number", item))?),
            None => (item, 1.0),
        };
        let (u, v) = pair.split_once('-').ok_or_else(|| format!("'{}' is not an edge like a-b", item))?;
        for x in [u, v] {
            if x.is_empty() || !x.chars().all(|c| c.is_alphanumeric() || c == '_') {
                return Err(format!("'{}' in '{}' is not a node name", x, item));
            }
        }
        if u == v {
            return Err(format!("'{}' joins a node to itself", item));
        }
        let mut id = |x: &str| {
            *idx.entry(x.to_string()).or_insert_with(|| {
                g.nodes.push(x.to_string());
                g.nodes.len() - 1
            })
        };
        let (a, b) = (id(u), id(v));
        g.edges.push((a, b, w));
    }
    if g.edges.is_empty() {
        return Err("edges: needs at least one edge like \"a-b\"".into());
    }
    Ok(g)
}

pub fn colouring_qubo(g: &Graph, k: usize, a: f64, b: f64) -> Qubo {
    let mut q = Qubo::new(g.nodes.len() * k);
    for u in 0..g.nodes.len() {
        q.exactly_one(&(0..k).map(|c| u * k + c).collect::<Vec<_>>(), a); // one colour per node
    }
    for &(u, v, _) in &g.edges {
        for c in 0..k {
            q.add_quad(u * k + c, v * k + c, b); // both ends the same colour costs b
        }
    }
    q
}

/// A colour per node (1-based), 0 for none, 255 for several.
pub fn colouring_decode(n: usize, k: usize, y: &[bool]) -> Vec<u8> {
    (0..n)
        .map(|u| {
            let on: Vec<usize> = (0..k).filter(|&c| y[u * k + c]).collect();
            match on.len() {
                0 => 0,
                1 => on[0] as u8 + 1,
                _ => 255,
            }
        })
        .collect()
}

pub fn colouring_check(g: &Graph, col: &[u8]) -> Result<(), String> {
    for (u, &c) in col.iter().enumerate() {
        match c {
            0 => return Err(format!("node {} has no colour", g.nodes[u])),
            255 => return Err(format!("node {} has several colours", g.nodes[u])),
            _ => {}
        }
    }
    for &(u, v, _) in &g.edges {
        if col[u] == col[v] {
            return Err(format!("edge {}-{} has colour {} at both ends", g.nodes[u], g.nodes[v], col[u]));
        }
    }
    Ok(())
}

/// Exhaustive search for a proper k-colouring (small graphs only; tests and baselines).
pub fn colouring_exists(g: &Graph, k: usize) -> bool {
    let n = g.nodes.len();
    let mut col = vec![0usize; n];
    fn go(u: usize, g: &Graph, k: usize, col: &mut Vec<usize>) -> bool {
        if u == col.len() {
            return true;
        }
        for c in 0..k {
            let ok = g.edges.iter().all(|&(a, b, _)| {
                let (x, y) = if a == u { (b, c) } else if b == u { (a, c) } else { return true };
                x >= u || col[x] != y
            });
            if ok {
                col[u] = c;
                if go(u + 1, g, k, col) {
                    return true;
                }
            }
        }
        false
    }
    go(0, g, k, &mut col)
}

pub fn cut_value(g: &Graph, side: &[bool]) -> f64 {
    g.edges.iter().filter(|&&(u, v, _)| side[u] != side[v]).map(|e| e.2).sum()
}

/// The best cut by trying every split (node 0 fixed on one side). Up to 24 nodes.
pub fn maxcut_exact(g: &Graph) -> f64 {
    let n = g.nodes.len();
    assert!(n <= 24, "maxcut_exact is for small graphs");
    let mut best = f64::NEG_INFINITY;
    let mut side = vec![false; n];
    for bits in 0u64..(1u64 << (n - 1)) {
        for (u, s) in side.iter_mut().enumerate().skip(1) {
            *s = (bits >> (u - 1)) & 1 == 1;
        }
        best = best.max(cut_value(g, &side));
    }
    best
}

fn graph_arg(kv: &[(String, Tok)], ln: usize) -> Result<(Graph, String), SettleError> {
    let s = match kw(kv, "edges") {
        Some(t) => text(t, ln)?,
        None => return err(ln, "needs edges: \"a-b b-c ...\""),
    };
    let g = parse_edges(&s).or_else(|e| err(ln, e))?;
    Ok((g, s))
}

fn declare_colouring(m: &mut Model, name: &str, rest: &[Tok], ln: usize) -> Result<(), SettleError> {
    fresh(m, name, ln)?;
    let kv = kwargs(rest, ln)?;
    only(&kv, &["colours", "edges", "by"], "colouring", ln)?;
    let k = whole(kw(&kv, "colours").map(|v| num(v, ln)).transpose()?.unwrap_or(3.0), 0.0, f64::INFINITY, "colours:", ln)?;
    if !(2..=9).contains(&k) {
        return err(ln, "colours must be from 2 to 9");
    }
    let a = kw(&kv, "by").map(|v| num(v, ln)).transpose()?.unwrap_or(1.0);
    let (g, s) = graph_arg(&kv, ln)?;
    let q = colouring_qubo(&g, k, a, a);
    let names: Vec<String> = (0..g.nodes.len() * k).map(|v| format!("{}_{}_{}", name, g.nodes[v / k], v % k + 1)).collect();
    let start = add_block(m, &names);
    q.apply(m, start);
    m.notes.insert(note_key(name), (vec![start as f64, k as f64], vec!["colouring".into(), s]));
    Ok(())
}

fn declare_maxcut(m: &mut Model, name: &str, rest: &[Tok], ln: usize) -> Result<(), SettleError> {
    fresh(m, name, ln)?;
    let kv = kwargs(rest, ln)?;
    only(&kv, &["edges", "target"], "maxcut", ln)?;
    let (g, s) = graph_arg(&kv, ln)?;
    let target = kw(&kv, "target").map(|v| num(v, ln)).transpose()?.unwrap_or(f64::NAN);
    let names: Vec<String> = g.nodes.iter().map(|u| format!("{}_{}", name, u)).collect();
    let start = add_block(m, &names);
    for &(u, v, w) in &g.edges {
        m.couple(start + u, start + v, -w); // each edge pushes its ends apart: energy Σ w s_u s_v
    }
    m.notes.insert(note_key(name), (vec![start as f64, target], vec!["maxcut".into(), s]));
    Ok(())
}

// ---------------------------------------------------------------------------------------------------------
// FACTOR
// ---------------------------------------------------------------------------------------------------------

fn bitlen(x: u64) -> usize {
    64 - x.leading_zeros() as usize
}

/// Bit widths for p (the smaller factor, at most √N) and q, so that 1 x N cannot fit.
pub fn factor_widths(n: u64) -> (usize, usize) {
    let r = (n as f64).sqrt().floor() as u64;
    let pb = bitlen(r).max(2);
    (pb, bitlen(n) - pb + 1)
}

/// Layout: variables 0..pb-1 are p_1..p_{pb-1}, then q_1..q_{qb-1}, then one helper per product of two
/// free bits. Bit 0 of each factor is fixed at 1.
pub struct FactorLayout {
    pub pb: usize,
    pub qb: usize,
    pub nvars: usize,
    pub helpers: Vec<(usize, usize, usize)>, // (p bit i, q bit j, variable of z_ij)
}

pub fn factor_layout(pb: usize, qb: usize) -> FactorLayout {
    let mut nvars = (pb - 1) + (qb - 1);
    let mut helpers = Vec::new();
    for i in 1..pb {
        for j in 1..qb {
            helpers.push((i, j, nvars));
            nvars += 1;
        }
    }
    FactorLayout { pb, qb, nvars, helpers }
}

/// (N - p·q)² with p·q written through helper bits, plus λ·Rosenberg for each helper.
pub fn factor_qubo(n: u64, lay: &FactorLayout, lambda: f64) -> Qubo {
    let pv = |i: usize| i - 1; // variable of p_i, i >= 1
    let qv = |j: usize| (lay.pb - 1) + j - 1;
    // p·q = Σ_{i,j} 2^(i+j) p_i q_j = c0 + Σ_k a_k y_k
    let c0 = 1.0; // p_0 q_0
    let mut terms: HashMap<usize, f64> = HashMap::new();
    for j in 1..lay.qb {
        *terms.entry(qv(j)).or_insert(0.0) += (1u64 << j) as f64; // p_0 q_j
    }
    for i in 1..lay.pb {
        *terms.entry(pv(i)).or_insert(0.0) += (1u64 << i) as f64; // p_i q_0
    }
    for &(i, j, z) in &lay.helpers {
        *terms.entry(z).or_insert(0.0) += (1u64 << (i + j)) as f64;
    }
    let mut t: Vec<(usize, f64)> = terms.into_iter().collect();
    t.sort_by_key(|x| x.0);
    let r = n as f64 - c0; // (r - Σ a y)² = r² - 2rΣ a y + Σ a² y + 2Σ_{k<l} a_k a_l y_k y_l
    let mut q = Qubo::new(lay.nvars);
    q.c += r * r;
    for (k, &(yk, ak)) in t.iter().enumerate() {
        q.lin[yk] += ak * ak - 2.0 * r * ak;
        for &(yl, al) in &t[k + 1..] {
            q.add_quad(yk, yl, 2.0 * ak * al);
        }
    }
    for &(i, j, z) in &lay.helpers {
        // λ(3z + xy - 2xz - 2yz): zero when z = x·y, at least λ otherwise
        q.lin[z] += 3.0 * lambda;
        q.add_quad(pv(i), qv(j), lambda);
        q.add_quad(pv(i), z, -2.0 * lambda);
        q.add_quad(qv(j), z, -2.0 * lambda);
    }
    q
}

pub fn factor_decode(lay: &FactorLayout, y: &[bool]) -> (u64, u64) {
    let mut p = 1u64;
    let mut q = 1u64;
    for i in 1..lay.pb {
        if y[i - 1] {
            p |= 1 << i;
        }
    }
    for j in 1..lay.qb {
        if y[lay.pb - 1 + j - 1] {
            q |= 1 << j;
        }
    }
    (p, q)
}

pub fn factor_check(n: u64, p: u64, q: u64) -> Result<(), String> {
    if p < 2 || q < 2 {
        return Err(format!("{} x {} uses a trivial factor", p, q));
    }
    if p.checked_mul(q) != Some(n) {
        return Err(format!("{} x {} = {}, not {}", p, q, p * q, n));
    }
    Ok(())
}

/// Column-with-carries layout (ZOOHARD). Variables: p_1..p_{pb-1}, q_1..q_{qb-1} and the helpers z_ij in the
/// same order as `factor_layout` (so `factor_decode` reads p and q unchanged), then the carry bits. A carry
/// c_{k,m} leaves column k with weight 2^m and lands in column k+m with weight 1.
pub struct ColumnLayout {
    pub base: FactorLayout,
    /// (column k, m, variable) for every carry bit.
    pub carries: Vec<(usize, usize, usize)>,
    /// Columns 1..=top are written; column 0 is p_0 q_0 = 1 = the low bit of N.
    pub top: usize,
    pub nvars: usize,
}

pub fn column_layout(n: u64) -> ColumnLayout {
    let (pb, qb) = factor_widths(n);
    let base = factor_layout(pb, qb);
    let top = pb + qb - 2; // = bitlen(N) - 1
    let mut nvars = base.nvars;
    let mut landing = vec![0usize; top + 2];
    let mut carries = Vec::new();
    for k in 1..=top {
        let terms = (0..pb).filter(|&i| k >= i && k - i < qb).count();
        let most = terms + landing[k]; // the largest the column's left side can be
        let nk = ((n >> k) & 1) as usize;
        let cmax = most.saturating_sub(nk) / 2; // the carry out C_k = (S_k - N_k) / 2
        let bits = 64 - (cmax as u64).leading_zeros() as usize;
        for m in 1..=bits {
            if k + m <= top {
                carries.push((k, m, nvars));
                landing[k + m] += 1;
                nvars += 1;
            }
        }
    }
    ColumnLayout { base, carries, top, nvars }
}

/// Σ_k (Σ_{i+j=k} p_i q_j + carries in - N_k - Σ_m 2^m c_{k,m})² with every free-by-free product replaced by its
/// helper, plus λ·Rosenberg per helper. Every term is at least zero, and the energy is zero exactly at a
/// factorisation with its carries, for any λ > 0.
pub fn factor_columns_qubo(n: u64, lay: &ColumnLayout, lambda: f64) -> Qubo {
    let (pb, qb) = (lay.base.pb, lay.base.qb);
    let pv = |i: usize| i - 1;
    let qv = |j: usize| (pb - 1) + j - 1;
    let zv: HashMap<(usize, usize), usize> = lay.base.helpers.iter().map(|&(i, j, z)| ((i, j), z)).collect();
    let mut q = Qubo::new(lay.nvars);
    for k in 1..=lay.top {
        let mut t: Vec<(usize, f64)> = Vec::new();
        for i in 0..pb {
            if k < i || k - i >= qb {
                continue;
            }
            let j = k - i;
            t.push(match (i, j) {
                (0, j) => (qv(j), 1.0),
                (i, 0) => (pv(i), 1.0),
                (i, j) => (zv[&(i, j)], 1.0),
            });
        }
        for &(kc, m, v) in &lay.carries {
            if kc + m == k {
                t.push((v, 1.0)); // carry in
            }
            if kc == k {
                t.push((v, -((1u64 << m) as f64))); // carry out
            }
        }
        let r = -(((n >> k) & 1) as f64); // (r + Σ a y)² = r² + Σ (a² + 2ra) y + 2 Σ_{u<v} a_u a_v y_u y_v
        q.c += r * r;
        for (a, &(ya, ca)) in t.iter().enumerate() {
            q.lin[ya] += ca * ca + 2.0 * r * ca;
            for &(yb, cb) in &t[a + 1..] {
                q.add_quad(ya, yb, 2.0 * ca * cb);
            }
        }
    }
    for &(i, j, z) in &lay.base.helpers {
        q.lin[z] += 3.0 * lambda;
        q.add_quad(pv(i), qv(j), lambda);
        q.add_quad(pv(i), z, -2.0 * lambda);
        q.add_quad(qv(j), z, -2.0 * lambda);
    }
    q
}

/// The 0/1 variables of the column encoding for given odd p and q (helpers and carries filled in), or None when
/// p and q do not fit the layout. For a true factorisation the column energy of this arrangement is zero.
pub fn columns_assign(n: u64, lay: &ColumnLayout, p: u64, q: u64) -> Option<Vec<bool>> {
    let (pb, qb) = (lay.base.pb, lay.base.qb);
    if p >> pb != 0 || q >> qb != 0 || p & 1 == 0 || q & 1 == 0 {
        return None;
    }
    let mut y = vec![false; lay.nvars];
    for i in 1..pb {
        y[i - 1] = (p >> i) & 1 == 1;
    }
    for j in 1..qb {
        y[pb - 1 + j - 1] = (q >> j) & 1 == 1;
    }
    for &(i, j, z) in &lay.base.helpers {
        y[z] = (p >> i) & (q >> j) & 1 == 1;
    }
    let mut landing = vec![0u64; lay.top + 2];
    for k in 1..=lay.top {
        let mut s = landing[k];
        for i in 0..pb {
            if k >= i && k - i < qb {
                s += (p >> i) & (q >> (k - i)) & 1;
            }
        }
        let nk = (n >> k) & 1;
        let c = s.checked_sub(nk)? / 2;
        let outs: Vec<&(usize, usize, usize)> = lay.carries.iter().filter(|x| x.0 == k).collect();
        if c >> outs.len() != 0 {
            return None;
        }
        for &&(_, m, v) in &outs {
            let bit = (c >> (m - 1)) & 1 == 1;
            y[v] = bit;
            if bit {
                landing[k + m] += 1;
            }
        }
    }
    Some(y)
}

fn declare_factor(m: &mut Model, name: &str, rest: &[Tok], ln: usize) -> Result<(), SettleError> {
    fresh(m, name, ln)?;
    let kv = kwargs(rest, ln)?;
    only(&kv, &["number", "penalty", "encoding"], "factor", ln)?;
    // The column encoding is the default since 2026-10-06 (lane NEWDEFAULTS; ZOOHARD: 899 at 100,000 sweeps
    // factors in 100% of 50 seeds against 14% for Rosenberg). `encoding: :rosenberg` is the old default.
    let columns = match kw(&kv, "encoding") {
        None => true,
        Some(Tok::Sym(e)) if e == "rosenberg" => false,
        Some(Tok::Sym(e)) if e == "columns" => true,
        Some(_) => return err(ln, "encoding is :columns (the default) or :rosenberg"),
    };
    let n = match kw(&kv, "number") {
        Some(v) => num(v, ln)?,
        None => return err(ln, "factor needs number:"),
    };
    let top = if columns { 1e12 } else { 1_000_000.0 };
    if n.fract() != 0.0 || !(9.0..=top).contains(&n) || (n as u64).is_multiple_of(2) {
        return err(ln, if columns { "factor takes an odd whole number from 9 to 10^12" } else { "factor with encoding: :rosenberg takes an odd whole number from 9 to 1,000,000" });
    }
    let n = n as u64;
    let (pb, qb) = factor_widths(n);
    let lay = factor_layout(pb, qb);
    let mut names = Vec::new();
    for i in 1..pb {
        names.push(format!("{}_p{}", name, i));
    }
    for j in 1..qb {
        names.push(format!("{}_q{}", name, j));
    }
    for &(i, j, _) in &lay.helpers {
        names.push(format!("{}_z{}_{}", name, i, j));
    }
    let (q, count) = if columns {
        let cl = column_layout(n);
        for &(k, mm, _) in &cl.carries {
            names.push(format!("{}_c{}_{}", name, k, mm));
        }
        let lambda = kw(&kv, "penalty").map(|v| num(v, ln)).transpose()?.unwrap_or(2.0);
        (factor_columns_qubo(n, &cl, lambda), cl.nvars)
    } else {
        let lambda = kw(&kv, "penalty").map(|v| num(v, ln)).transpose()?.unwrap_or((1u64 << (pb + qb - 2)) as f64);
        (factor_qubo(n, &lay, lambda), lay.nvars)
    };
    let start = add_block(m, &names);
    q.apply(m, start);
    let enc = if columns { "columns" } else { "rosenberg" };
    m.notes.insert(note_key(name), (vec![start as f64, n as f64, pb as f64, qb as f64, count as f64], vec!["factor".into(), enc.into()]));
    Ok(())
}

// ---------------------------------------------------------------------------------------------------------
// NONOGRAM (lane PUZZLEFEATURE): the domain-wall encoding, the same as SETTLE/settle-site/src/engine/nonogram.js
// ---------------------------------------------------------------------------------------------------------

/// Clues: lines separated by '/', run lengths by spaces; "0" or nothing is an empty line. "1 1/5/5/3/1".
pub fn parse_clues(s: &str) -> Result<Vec<Vec<usize>>, String> {
    s.split('/')
        .map(|line| {
            line.split_whitespace()
                .map(|t| t.parse::<usize>().map_err(|_| format!("'{}' is not a run length", t)))
                .filter(|r| r != &Ok(0))
                .collect()
        })
        .collect()
}

/// The first and last start of block k of `clue` in a line of n cells.
fn start_range(clue: &[usize], n: usize, k: usize) -> (usize, usize) {
    let lo: usize = clue[..k].iter().map(|l| l + 1).sum();
    let tail: usize = clue[k..].iter().sum::<usize>() + clue.len() - k - 1;
    (lo, n - tail)
}

/// One block: its length, start range and its "past here" bits u_p = [start > p] for p in lo..hi.
#[derive(Clone, Debug)]
pub struct NonoBlock {
    pub len: usize,
    pub lo: usize,
    pub hi: usize,
    pub bits: usize,
}

#[derive(Clone, Debug)]
pub struct NonoLayout {
    pub h: usize,
    pub w: usize,
    pub rows: Vec<Vec<NonoBlock>>,
    pub cols: Vec<Vec<NonoBlock>>,
    pub nvars: usize,
}

pub fn nonogram_layout(rows: &[Vec<usize>], cols: &[Vec<usize>]) -> Result<NonoLayout, String> {
    let (h, w) = (rows.len(), cols.len());
    let mut n = h * w;
    let mut lines = |clues: &[Vec<usize>], len: usize, what: &str| -> Result<Vec<Vec<NonoBlock>>, String> {
        clues
            .iter()
            .enumerate()
            .map(|(i, clue)| {
                if clue.iter().sum::<usize>() + clue.len().saturating_sub(1) > len {
                    return Err(format!("{} {} clue does not fit in {} cells", what, i + 1, len));
                }
                Ok((0..clue.len())
                    .map(|k| {
                        let (lo, hi) = start_range(clue, len, k);
                        let b = NonoBlock { len: clue[k], lo, hi, bits: n };
                        n += hi - lo;
                        b
                    })
                    .collect())
            })
            .collect()
    };
    let rb = lines(rows, w, "row")?;
    let cb = lines(cols, h, "column")?;
    Ok(NonoLayout { h, w, rows: rb, cols: cb, nvars: n })
}

/// An affine term: constant plus Σ coefficient · y.
type Affine = (f64, Vec<(usize, f64)>);

fn past(b: &NonoBlock, p: isize) -> Affine {
    if p < b.lo as isize {
        (1.0, vec![])
    } else if p >= b.hi as isize {
        (0.0, vec![])
    } else {
        (0.0, vec![(b.bits + p as usize - b.lo, 1.0)])
    }
}

fn add_affine(terms: &[(Affine, f64)]) -> Affine {
    let mut k = 0.0;
    let mut m: Vec<(usize, f64)> = Vec::new();
    for ((c, v), sign) in terms {
        k += sign * c;
        for &(y, a) in v {
            match m.iter_mut().find(|e| e.0 == y) {
                Some(e) => e.1 += sign * a,
                None => m.push((y, sign * a)),
            }
        }
    }
    m.retain(|e| e.1 != 0.0);
    (k, m)
}

fn sq_affine(q: &mut Qubo, t: &Affine, w: f64) {
    q.c += w * t.0 * t.0;
    for (a, &(ya, ca)) in t.1.iter().enumerate() {
        q.lin[ya] += w * (ca * ca + 2.0 * t.0 * ca);
        for &(yb, cb) in &t.1[a + 1..] {
            q.add_quad(ya, yb, 2.0 * w * ca * cb);
        }
    }
}

fn mul_affine(q: &mut Qubo, s: &Affine, t: &Affine, w: f64) {
    q.c += w * s.0 * t.0;
    for &(y, c) in &s.1 {
        q.lin[y] += w * c * t.0;
    }
    for &(y, c) in &t.1 {
        q.lin[y] += w * c * s.0;
    }
    for &(ya, ca) in &s.1 {
        for &(yb, cb) in &t.1 {
            q.add_quad(ya, yb, w * ca * cb);
        }
    }
}

fn cover(b: &NonoBlock, c: usize) -> Affine {
    add_affine(&[(past(b, c as isize - b.len as isize), 1.0), (past(b, c as isize), -1.0)])
}

/// The nonogram QUBO. Variables: cell (r, c) is r*w + c; then each row block's wall bits, then each column's.
/// Costs: one wall per block (A u_(p+1) (1 - u_p)), blocks in order with a gap (B G_k(p) (1 - G_(k+1)(p + L + 1))),
/// and each cell agreeing with its row and its column (C (x - Σ cover)^2). Zero exactly at a solution.
pub fn nonogram_qubo(rows: &[Vec<usize>], cols: &[Vec<usize>], a: f64, b: f64, c: f64) -> Result<(Qubo, NonoLayout), String> {
    let lay = nonogram_layout(rows, cols)?;
    let mut q = Qubo::new(lay.nvars);
    let one: Affine = (1.0, vec![]);
    for line in lay.rows.iter().chain(lay.cols.iter()) {
        for bl in line {
            for i in bl.bits..bl.bits + (bl.hi - bl.lo) {
                if i + 1 < bl.bits + (bl.hi - bl.lo) {
                    let next: Affine = (0.0, vec![(i + 1, 1.0)]);
                    let not_here = add_affine(&[(one.clone(), 1.0), ((0.0, vec![(i, 1.0)]), -1.0)]);
                    mul_affine(&mut q, &next, &not_here, a);
                }
            }
        }
        for k in 0..line.len().saturating_sub(1) {
            let (x, y) = (&line[k], &line[k + 1]);
            for p in x.lo..x.hi {
                let later = past(y, (p + x.len + 1) as isize);
                if later.1.is_empty() && later.0 == 1.0 {
                    continue;
                }
                let gate = add_affine(&[(one.clone(), 1.0), (later, -1.0)]);
                mul_affine(&mut q, &past(x, p as isize), &gate, b);
            }
        }
    }
    for r in 0..lay.h {
        for cc in 0..lay.w {
            let x: Affine = (0.0, vec![(r * lay.w + cc, 1.0)]);
            let from_row = add_affine(&lay.rows[r].iter().map(|bl| (cover(bl, cc), 1.0)).collect::<Vec<_>>());
            let from_col = add_affine(&lay.cols[cc].iter().map(|bl| (cover(bl, r), 1.0)).collect::<Vec<_>>());
            sq_affine(&mut q, &add_affine(&[(x.clone(), 1.0), (from_row, -1.0)]), c);
            sq_affine(&mut q, &add_affine(&[(x, 1.0), (from_col, -1.0)]), c);
        }
    }
    Ok((q, lay))
}

/// The run lengths of one line.
pub fn line_clue(cells: &[bool]) -> Vec<usize> {
    let mut out = Vec::new();
    let mut run = 0;
    for &v in cells {
        if v {
            run += 1;
        } else if run > 0 {
            out.push(run);
            run = 0;
        }
    }
    if run > 0 {
        out.push(run);
    }
    out
}

/// Plain check: every row and column of the grid read back and compared with its clue.
pub fn nonogram_check(rows: &[Vec<usize>], cols: &[Vec<usize>], grid: &[bool]) -> Result<(), String> {
    let w = cols.len();
    for (r, clue) in rows.iter().enumerate() {
        if &line_clue(&grid[r * w..(r + 1) * w]) != clue {
            return Err(format!("row {} reads {:?}, its clue is {:?}", r + 1, line_clue(&grid[r * w..(r + 1) * w]), clue));
        }
    }
    for (c, clue) in cols.iter().enumerate() {
        let line: Vec<bool> = (0..rows.len()).map(|r| grid[r * w + c]).collect();
        if &line_clue(&line) != clue {
            return Err(format!("column {} reads {:?}, its clue is {:?}", c + 1, line_clue(&line), clue));
        }
    }
    Ok(())
}

/// The 0/1 arrangement that writes `grid` (cells plus each block's wall at its run's start), if it fits.
pub fn nonogram_assign(lay: &NonoLayout, grid: &[bool]) -> Option<Vec<bool>> {
    let mut y = vec![false; lay.nvars];
    y[..lay.h * lay.w].copy_from_slice(grid);
    let place = |blocks: &[NonoBlock], cells: Vec<bool>, y: &mut Vec<bool>| -> bool {
        let mut starts = Vec::new();
        for (i, &v) in cells.iter().enumerate() {
            if v && (i == 0 || !cells[i - 1]) {
                starts.push(i);
            }
        }
        if starts.len() != blocks.len() {
            return false;
        }
        for (b, &s) in blocks.iter().zip(&starts) {
            if s < b.lo || s > b.hi {
                return false;
            }
            for p in b.lo..b.hi {
                y[b.bits + p - b.lo] = s > p;
            }
        }
        true
    };
    for r in 0..lay.h {
        if !place(&lay.rows[r], grid[r * lay.w..(r + 1) * lay.w].to_vec(), &mut y) {
            return None;
        }
    }
    for c in 0..lay.w {
        if !place(&lay.cols[c], (0..lay.h).map(|r| grid[r * lay.w + c]).collect(), &mut y) {
            return None;
        }
    }
    Some(y)
}

fn declare_nonogram(m: &mut Model, name: &str, rest: &[Tok], ln: usize) -> Result<(), SettleError> {
    fresh(m, name, ln)?;
    let kv = kwargs(rest, ln)?;
    only(&kv, &["rows", "cols", "by"], "nonogram", ln)?;
    let get = |k: &str| -> Result<String, SettleError> {
        match kw(&kv, k) {
            Some(t) => text(t, ln),
            None => err(ln, format!("nonogram needs {}: \"1 1/5/5/3/1\"", k)),
        }
    };
    let (rs, cs) = (get("rows")?, get("cols")?);
    let rows = parse_clues(&rs).or_else(|e| err(ln, e))?;
    let cols = parse_clues(&cs).or_else(|e| err(ln, e))?;
    let a = kw(&kv, "by").map(|v| num(v, ln)).transpose()?.unwrap_or(1.0);
    if a <= 0.0 {
        return err(ln, "by: must be above zero");
    }
    let (q, lay) = nonogram_qubo(&rows, &cols, a, a, a).or_else(|e| err(ln, e))?;
    let mut names: Vec<String> = (0..lay.h * lay.w).map(|v| format!("{}_r{}c{}", name, v / lay.w + 1, v % lay.w + 1)).collect();
    for (dir, lines) in [("row", &lay.rows), ("col", &lay.cols)] {
        for (i, line) in lines.iter().enumerate() {
            for (k, b) in line.iter().enumerate() {
                for p in b.lo..b.hi {
                    names.push(format!("{}_{}{}_b{}_past{}", name, dir, i + 1, k + 1, p + 1));
                }
            }
        }
    }
    let start = add_block(m, &names);
    q.apply(m, start);
    m.notes.insert(note_key(name), (vec![start as f64, lay.h as f64, lay.w as f64, lay.nvars as f64], vec!["nonogram".into(), rs, cs]));
    Ok(())
}

/// Where a declared puzzle's things sit: (first thing, how many).
pub fn span(m: &Model, name: &str) -> Option<(usize, usize)> {
    let (nums, words) = m.notes.get(&note_key(name))?;
    let start = nums[0] as usize;
    let len = match words[0].as_str() {
        "sudoku" => (nums[1] as usize).pow(3),
        "colouring" => parse_edges(&words[1]).ok()?.nodes.len() * nums[1] as usize,
        "maxcut" => parse_edges(&words[1]).ok()?.nodes.len(),
        "factor" => nums[4] as usize,
        "nonogram" => nums[3] as usize,
        _ => return None,
    };
    Some((start, len))
}

/// Energy of one block of things on its own: its leans and the pulls inside it.
pub fn part_energy(m: &Model, s: &[f64], start: usize, len: usize) -> f64 {
    let mut e = 0.0;
    for i in start..start + len {
        e -= m.h[i] * s[i];
        for &(k, w) in &m.adj[i] {
            if k > i && k < start + len {
                e -= w * s[i] * s[k];
            }
        }
    }
    e
}

/// A fingerprint of an arrangement and its energy, to tell whether a stored per-puzzle best belongs to the
/// anneal that produced the run's current best.
fn stamp(best: &(Vec<f64>, f64)) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &v in &best.0 {
        h = (h ^ (v > 0.0) as u64).wrapping_mul(0x100_0000_01b3);
    }
    format!("{:016x}/{}", h, best.1)
}

/// The same walk as the core `anneal` (same schedule, same random draws, so the same arrangements are visited
/// for the same seed), but every declared puzzle also keeps the arrangement of its OWN things that was calmest
/// for its OWN energy. Returns (name, that energy) per puzzle; the per-puzzle bests are kept on the model under
/// `zoo:each:<name>`, stamped with the run's whole-model best, and `x.solution` prefers them while the stamp
/// matches.
pub fn anneal_each(m: &mut Model, st: &mut State, sweeps: usize) -> Vec<(String, f64)> {
    let mut parts: Vec<(String, usize, usize)> = m
        .notes
        .keys()
        .filter_map(|k| k.strip_prefix("zoo:").filter(|r| !r.starts_with("each:")).map(|r| r.to_string()))
        .filter_map(|nm| span(m, &nm).map(|(a, l)| (nm, a, l)))
        .collect();
    parts.sort();
    let (mut s, mut free) = st.start(m);
    let mut best = (s.clone(), m.energy(&s));
    let mut pb: Vec<(Vec<f64>, f64)> = parts.iter().map(|(_, a, l)| (s[*a..a + l].to_vec(), part_energy(m, &s, *a, *l))).collect();
    for step in 0..sweeps {
        let temp = st.temp * 10.0 * 0.005f64.powf(step as f64 / (sweeps.max(2) - 1) as f64);
        st.sweep(m, &mut s, &mut free, 1.0 / temp);
        let e = m.energy(&s);
        if e < best.1 {
            best = (s.clone(), e);
        }
        for ((_, a, l), b) in parts.iter().zip(pb.iter_mut()) {
            let pe = part_energy(m, &s, *a, *l);
            if pe < b.1 {
                *b = (s[*a..a + l].to_vec(), pe);
            }
        }
    }
    st.last = s;
    let tag = stamp(&best);
    st.best = Some(best);
    let mut out = Vec::new();
    for ((nm, _, _), (v, e)) in parts.into_iter().zip(pb) {
        m.notes.insert(format!("zoo:each:{}", nm), (v, vec![tag.clone()]));
        out.push((nm, e));
    }
    out
}

/// The arrangement `x.solution` judges: the run's best, with this puzzle's block replaced by its own calmest
/// block when the last anneal was `anneal_each`.
pub fn judged_arrangement(m: &Model, st: &State, name: &str) -> Option<(Vec<f64>, bool)> {
    let best = st.best.as_ref()?;
    let mut s = best.0.clone();
    if let (Some((v, w)), Some((a, l))) = (m.notes.get(&format!("zoo:each:{}", name)), span(m, name)) {
        if w.first() == Some(&stamp(best)) && v.len() == l {
            s[a..a + l].copy_from_slice(v);
            return Some((s, true));
        }
    }
    Some((s, false))
}

// ---------------------------------------------------------------------------------------------------------
// x.solution: decode the calmest arrangement and check it with plain code
// ---------------------------------------------------------------------------------------------------------

/// The verdict for one puzzle on an arrangement, plus the lines to print. Public so measurements can use it.
pub fn judge(m: &Model, name: &str, s: &[f64]) -> Option<(bool, Vec<String>)> {
    let (nums, words) = m.notes.get(&note_key(name))?;
    let start = nums[0] as usize;
    let mut out = Vec::new();
    let ok = match words[0].as_str() {
        "sudoku" => {
            let n = nums[1] as usize;
            let givens = parse_givens(&words[1], n).ok()?;
            let grid = sudoku_decode(n, &bits_of(&s[start..start + n * n * n]));
            let b = box_side(n);
            for r in 0..n {
                let mut line = String::from("  ");
                for c in 0..n {
                    if c > 0 && c % b == 0 {
                        line.push_str("| ");
                    }
                    line.push(match grid[r * n + c] {
                        0 => '.',
                        255 => '?',
                        d => (b'0' + d) as char,
                    });
                    line.push(' ');
                }
                if r > 0 && r % b == 0 {
                    out.push(format!("  {}", "-".repeat(line.len() - 3)));
                }
                out.push(line.trim_end().to_string());
            }
            let v = sudoku_check(n, &givens, &grid);
            out.push(match &v {
                Ok(()) => format!("sudoku :{}: VALID (checked rule by rule, not by energy)", name),
                Err(e) => format!("sudoku :{}: NOT VALID: {}", name, e),
            });
            v.is_ok()
        }
        "colouring" => {
            let k = nums[1] as usize;
            let g = parse_edges(&words[1]).ok()?;
            let col = colouring_decode(g.nodes.len(), k, &bits_of(&s[start..start + g.nodes.len() * k]));
            let parts: Vec<String> = g
                .nodes
                .iter()
                .zip(&col)
                .map(|(u, &c)| format!("{} {}", u, match c { 0 => "-".to_string(), 255 => "?".to_string(), c => c.to_string() }))
                .collect();
            out.push(format!("  {}", parts.join(", ")));
            let v = colouring_check(&g, &col);
            out.push(match &v {
                Ok(()) => format!("colouring :{} with {} colours: PROPER (checked edge by edge, not by energy)", name, k),
                Err(e) => format!("colouring :{} with {} colours: NOT PROPER: {}", name, k, e),
            });
            v.is_ok()
        }
        "maxcut" => {
            let g = parse_edges(&words[1]).ok()?;
            let side = bits_of(&s[start..start + g.nodes.len()]);
            let cut = cut_value(&g, &side);
            let a: Vec<&str> = g.nodes.iter().zip(&side).filter(|x| *x.1).map(|x| x.0.as_str()).collect();
            let b: Vec<&str> = g.nodes.iter().zip(&side).filter(|x| !*x.1).map(|x| x.0.as_str()).collect();
            out.push(format!("  side yes: {}   side no: {}", a.join(" "), b.join(" ")));
            let target = nums[1];
            let best = if g.nodes.len() <= 22 { Some(maxcut_exact(&g)) } else { None };
            let mut line = format!("maxcut :{}: cut {}", name, cut);
            let mut ok = true;
            if let Some(bst) = best {
                ok = cut >= bst;
                line.push_str(&format!(
                    ", exact best {} by brute force: {}",
                    bst,
                    if ok { "OPTIMAL".to_string() } else { format!("SHORT by {}", bst - cut) }
                ));
            }
            if !target.is_nan() {
                let hit = cut >= target;
                ok = hit;
                line.push_str(&format!(", target {}: {}", target, if hit { "REACHED" } else { "NOT REACHED" }));
            }
            out.push(line);
            ok
        }
        "factor" => {
            let n = nums[1] as u64;
            let lay = factor_layout(nums[2] as usize, nums[3] as usize);
            let (p, q) = factor_decode(&lay, &bits_of(&s[start..start + lay.nvars]));
            let v = factor_check(n, p, q);
            out.push(match &v {
                Ok(()) => format!("factor :{}: {} = {} x {}: VALID (checked by multiplying)", name, n, p.min(q), p.max(q)),
                Err(e) => format!("factor :{}: NOT VALID for {}: {}", name, n, e),
            });
            v.is_ok()
        }
        "nonogram" => {
            let (h, w) = (nums[1] as usize, nums[2] as usize);
            let rows = parse_clues(&words[1]).ok()?;
            let cols = parse_clues(&words[2]).ok()?;
            let grid = bits_of(&s[start..start + h * w]);
            for r in 0..h {
                out.push(format!("  {}", grid[r * w..(r + 1) * w].iter().map(|&v| if v { '#' } else { '.' }).collect::<String>()));
            }
            let v = nonogram_check(&rows, &cols, &grid);
            out.push(match &v {
                Ok(()) => format!("nonogram :{}: VALID (every row and column read back against its clue, not by energy)", name),
                Err(e) => format!("nonogram :{}: NOT VALID: {}", name, e),
            });
            v.is_ok()
        }
        _ => return None,
    };
    Some((ok, out))
}

fn anneal_each_stmt(m: &mut Model, st: &mut State, sweeps: usize, rest: &[Tok], ln: usize, ctx: &mut Ctx) -> Result<(), SettleError> {
    let rest = if rest.first() == Some(&Tok::Comma) { &rest[1..] } else { rest };
    let kv = kwargs(rest, ln)?;
    only(&kv, &["temperature", "seed", "update"], "anneal_each", ln)?;
    // `update:` as on the core `settle` and `anneal`: :metro (the default since 2026-10-06) or :gibbs
    match kw(&kv, "update") {
        None => {}
        Some(Tok::Sym(s)) if crate::words::core::update_of(s).is_some() => st.update = crate::words::core::update_of(s).unwrap(),
        Some(_) => return err(ln, "`update:` takes :gibbs or :metro"),
    }
    if let Some(v) = kw(&kv, "temperature") {
        st.temp = num(v, ln)?;
        if st.temp <= 0.0 {
            return err(ln, "temperature must be above zero");
        }
    }
    if let Some(v) = kw(&kv, "seed") {
        st.rng = crate::rng::Rng::new(num(v, ln)? as u64);
    }
    let parts = anneal_each(m, st, sweeps);
    let e = st.best.as_ref().map(|b| b.1).unwrap_or(f64::NAN);
    let each: Vec<String> = parts.iter().map(|(n, pe)| format!(":{} {:.3}", n, pe)).collect();
    ctx.say(format!("annealed each: {} sweeps, calmest energy found {:.3}; per puzzle {}", sweeps, e, each.join(", ")));
    Ok(())
}

impl Ext for Zoo {
    fn name(&self) -> &'static str {
        "zoo"
    }

    fn statements(&self) -> &'static [&'static str] {
        &[
            "model: sudoku :s, size: 4, given: \"1... .4.. ..4. ...1\", by: 1, given_by: 4",
            "model: colouring :g, colours: 3, edges: \"a-b b-c c-a\"",
            "model: maxcut :m, edges: \"a-b b-c:2 c-a\", target: 3",
            "model: factor :f, number: 10403   /   factor :f, number: 143, encoding: :rosenberg, penalty: 128",
            "model: nonogram :n, rows: \"1 1/5/5/3/1\", cols: \"2/4/4/4/2\"",
            "run: anneal_each 10_000, seed: 1, update: :metro|:gibbs   (each puzzle keeps its own calmest arrangement)",
            "run: s.solution   (after anneal: decode the calmest arrangement and check it by the rules)",
        ]
    }

    fn model_stmt(&self, m: &mut Model, t: &[Tok], ln: usize, _ctx: &mut Ctx) -> Claim {
        match t {
            [Tok::Ident(k), Tok::Sym(name), rest @ ..] if ["sudoku", "colouring", "maxcut", "factor", "nonogram"].contains(&k.as_str()) => {
                let rest = if rest.first() == Some(&Tok::Comma) { &rest[1..] } else { rest };
                Some(match k.as_str() {
                    "sudoku" => declare_sudoku(m, name, rest, ln),
                    "colouring" => declare_colouring(m, name, rest, ln),
                    "maxcut" => declare_maxcut(m, name, rest, ln),
                    "nonogram" => declare_nonogram(m, name, rest, ln),
                    _ => declare_factor(m, name, rest, ln),
                })
            }
            _ => None,
        }
    }

    fn run_stmt(&self, m: &mut Model, st: &mut State, t: &[Tok], ln: usize, ctx: &mut Ctx) -> Claim {
        match t {
            [Tok::Ident(name), Tok::Dot, Tok::Ident(v)] if v == "solution" && m.notes.contains_key(&note_key(name)) => {
                Some(match judged_arrangement(m, st, name) {
                    None => err(ln, "solution needs an anneal first"),
                    Some((s, own)) => {
                        let (_, lines) = judge(m, name, &s).expect("a declared puzzle always judges");
                        if own {
                            ctx.say(format!("  (:{} judged on its own calmest arrangement from anneal_each)", name));
                        }
                        for l in lines {
                            ctx.say(l);
                        }
                        Ok(())
                    }
                })
            }
            [Tok::Ident(k), Tok::Num(nv), rest @ ..] if k == "anneal_each" => {
                Some(whole(*nv, 0.0, f64::INFINITY, "anneal_each", ln).and_then(|sweeps| anneal_each_stmt(m, st, sweeps, rest, ln, ctx)))
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interp::Interp;
    use crate::rng::Rng;

    fn model_of(src: &str) -> Model {
        let mut it = Interp::default();
        it.exec(src).unwrap_or_else(|e| panic!("{}", e));
        it.models.values().next().unwrap().clone()
    }

    #[test]
    fn qubo_to_springs_keeps_the_energy_up_to_a_constant() {
        let mut r = Rng::new(9);
        let mut q = Qubo::new(6);
        for i in 0..6 {
            q.lin[i] = r.signed() * 3.0;
            for j in i + 1..6 {
                q.add_quad(i, j, r.signed() * 2.0);
            }
        }
        q.exactly_one(&[0, 2, 4], 1.5);
        let mut m = Model::default();
        for i in 0..6 {
            m.add(&format!("x{}", i));
        }
        let off = q.apply(&mut m, 0);
        for bits in 0u32..64 {
            let y: Vec<bool> = (0..6).map(|k| (bits >> k) & 1 == 1).collect();
            let s: Vec<f64> = y.iter().map(|&b| if b { 1.0 } else { -1.0 }).collect();
            assert!((m.energy(&s) + off - q.energy(&y)).abs() < 1e-9);
        }
    }

    const SOLVED4: &str = "1234 3412 2143 4321";

    #[test]
    fn the_checker_accepts_a_real_sudoku_and_names_each_broken_rule() {
        let good = parse_givens(SOLVED4, 4).unwrap();
        assert!(sudoku_check(4, &good, &good).is_ok());
        let mut bad = good.clone();
        bad.swap(0, 1); // 2134 in row 1: columns 1 and 2 now repeat
        assert!(sudoku_check(4, &parse_givens(".... .... .... ....", 4).unwrap(), &bad).unwrap_err().contains("column"));
        assert!(sudoku_check(4, &good, &bad).unwrap_err().contains("was given"));
        let mut empty = good.clone();
        empty[5] = 0;
        assert!(sudoku_check(4, &good, &empty).unwrap_err().contains("no digit"));
    }

    #[test]
    fn a_solved_grid_is_the_calmest_arrangement_of_the_sudoku_springs() {
        // Every single flip away from the solution raises the energy, and the QUBO energy of the solution is
        // exactly -A x (number of givens), the lowest any arrangement can reach.
        let givens = parse_givens("1... .4.. ..4. ...1", 4).unwrap();
        let q = sudoku_qubo(4, &givens, 1.0, 1.0);
        let sol = parse_givens(SOLVED4, 4).unwrap();
        let y: Vec<bool> = (0..64).map(|v| sol[v / 4] as usize == v % 4 + 1).collect();
        let e0 = q.energy(&y);
        assert_eq!(e0, -4.0);
        for k in 0..64 {
            let mut y2 = y.clone();
            y2[k] = !y2[k];
            assert!(q.energy(&y2) > e0);
        }
    }

    #[test]
    fn annealing_solves_a_4x4_sudoku_and_the_answer_passes_the_rules() {
        let out = Interp::default()
            .exec("model :p do\n  sudoku :s, size: 4, given: \"1... .4.. ..4. ...1\"\nend\nrun :p do\n  anneal 3_000, seed: 1\n  s.solution\nend")
            .unwrap();
        assert!(out.last().unwrap().ends_with("VALID (checked rule by rule, not by energy)"), "{:?}", out);
        assert!(!out.last().unwrap().contains("NOT"));
    }

    #[test]
    fn a_contradictory_sudoku_is_never_reported_valid() {
        // negative control: two 1s given in row 1 cannot be kept by any valid grid
        let m = model_of("model :p do\n  sudoku :s, size: 4, given: \"1.1. .... .... ....\"\nend");
        for seed in 0..10 {
            let mut st = State::new(seed);
            st.anneal(&m, 2_000);
            let (ok, lines) = judge(&m, "s", &st.best.as_ref().unwrap().0).unwrap();
            assert!(!ok && lines.last().unwrap().contains("NOT VALID"), "{:?}", lines);
        }
    }

    #[test]
    fn a_triangle_cannot_take_two_colours_and_the_springs_do_not_pretend_it_can() {
        let g = parse_edges("a-b b-c c-a").unwrap();
        assert!(!colouring_exists(&g, 2) && colouring_exists(&g, 3));
        let two = model_of("model :p do\n  colouring :g, colours: 2, edges: \"a-b b-c c-a\"\nend");
        let three = model_of("model :p do\n  colouring :g, colours: 3, edges: \"a-b b-c c-a\"\nend");
        for seed in 0..10 {
            let mut st = State::new(seed);
            st.anneal(&two, 500);
            assert!(!judge(&two, "g", &st.best.as_ref().unwrap().0).unwrap().0);
            let mut st = State::new(seed);
            st.anneal(&three, 500);
            assert!(judge(&three, "g", &st.best.as_ref().unwrap().0).unwrap().0, "vacuity control: 3 colours work");
        }
    }

    #[test]
    fn maxcut_reports_the_brute_force_optimum_and_a_target_it_cannot_reach() {
        let out = Interp::default()
            .exec("model :p do\n  maxcut :m, edges: \"a-b b-c c-d d-a a-c\"\n  maxcut :t, edges: \"a-b b-c c-a\", target: 3\nend\nrun :p do\n  anneal 1_000, seed: 2\n  m.solution\n  t.solution\nend")
            .unwrap();
        assert!(out.iter().any(|l| l.contains("maxcut :m: cut 4, exact best 4 by brute force: OPTIMAL")), "{:?}", out);
        // negative control: a triangle's best cut is 2, so a target of 3 is never reached
        assert!(out.iter().any(|l| l.contains("maxcut :t: cut 2") && l.contains("target 3: NOT REACHED")), "{:?}", out);
    }

    #[test]
    fn the_factor_springs_have_their_calmest_arrangement_at_the_factors() {
        for (n, want) in [(15u64, Some((3, 5))), (21, Some((3, 7))), (35, Some((5, 7))), (13, None), (31, None)] {
            let (pb, qb) = factor_widths(n);
            let lay = factor_layout(pb, qb);
            let q = factor_qubo(n, &lay, (1u64 << (pb + qb - 2)) as f64);
            assert!(lay.nvars <= 20);
            let mut best = (f64::INFINITY, 0u64, 0u64);
            for bits in 0u64..(1 << lay.nvars) {
                let y: Vec<bool> = (0..lay.nvars).map(|k| (bits >> k) & 1 == 1).collect();
                let e = q.energy(&y);
                if e < best.0 {
                    let (p, qq) = factor_decode(&lay, &y);
                    best = (e, p, qq);
                }
            }
            match want {
                Some((p, qq)) => assert_eq!((best.0, best.1.min(best.2), best.1.max(best.2)), (0.0, p, qq), "N={}", n),
                None => assert!(best.0 > 0.0, "a prime {} must have no zero-energy arrangement", n),
            }
        }
    }

    #[test]
    fn factor_15_is_found_and_a_prime_is_never_called_factored() {
        let m = model_of("model :p do\n  factor :f, number: 15\n  factor :g, number: 13\nend");
        let mut found = 0;
        for seed in 0..10 {
            let mut st = State::new(seed);
            st.temp = 5.0;
            st.anneal(&m, 1_000);
            let s = &st.best.as_ref().unwrap().0;
            found += judge(&m, "f", s).unwrap().0 as usize;
            assert!(!judge(&m, "g", s).unwrap().0, "negative control: 13 is prime");
        }
        assert!(found >= 5, "15 found in only {} of 10", found);
    }

    #[test]
    fn the_zoo_examples_give_their_documented_verdicts() {
        // every example pairs a solvable puzzle with an impossible one; the impossible one must never pass
        let want: [(&str, &[&str]); 4] = [
            ("sudoku", &[": VALID", ": VALID", "NOT VALID: row 1 has two 1s"]),
            ("colouring", &[": PROPER", "NOT PROPER"]),
            ("maxcut", &["OPTIMAL", "target 3: NOT REACHED"]),
            ("factor", &["143 = 11 x 13: VALID", "NOT VALID for 127"]),
        ];
        for (ex, verdicts) in want {
            let path = format!("{}/examples/{}.settle", env!("CARGO_MANIFEST_DIR"), ex);
            let out = Interp::default().exec(&std::fs::read_to_string(&path).unwrap()).unwrap();
            let got: Vec<&String> = out.iter().filter(|l| l.starts_with("sudoku :") || l.starts_with("colouring :") || l.starts_with("maxcut :") || l.starts_with("factor :")).collect();
            assert_eq!(got.len(), verdicts.len(), "{}: {:?}", ex, out);
            for (g, w) in got.iter().zip(verdicts) {
                assert!(g.contains(w), "{}: {:?} should contain {:?}", ex, g, w);
            }
        }
    }

    #[test]
    fn anneal_each_walks_the_same_path_as_anneal() {
        let m = model_of("model :p do\n  factor :f, number: 143\n  factor :g, number: 127\nend");
        for seed in 0..5 {
            let mut a = State::new(seed);
            a.temp = 115.8;
            a.anneal(&m, 2_000);
            let mut m2 = m.clone();
            let mut b = State::new(seed);
            b.temp = 115.8;
            let parts = anneal_each(&mut m2, &mut b, 2_000);
            assert_eq!(a.best.as_ref().unwrap().0, b.best.as_ref().unwrap().0);
            assert_eq!(a.last, b.last);
            assert_eq!(parts.len(), 2);
            // each puzzle's own best is at least as calm, for its own energy, as its block of the shared best
            let s = &b.best.as_ref().unwrap().0;
            for (nm, pe) in parts {
                let (st, l) = span(&m2, &nm).unwrap();
                assert!(pe <= part_energy(&m2, s, st, l) + 1e-9);
            }
        }
    }

    #[test]
    fn a_stale_per_puzzle_best_is_never_used() {
        let src = "model :p do\n  factor :f, number: 143\nend\nrun :p do\n  anneal_each 2_000, temperature: 1158, seed: 1\n  f.solution\n  anneal 2_000, temperature: 1158, seed: 2\n  f.solution\nend";
        let out = Interp::default().exec(src).unwrap();
        let own: Vec<usize> = out.iter().enumerate().filter(|(_, l)| l.contains("judged on its own calmest")).map(|(i, _)| i).collect();
        assert_eq!(own.len(), 1, "{:?}", out);
        assert!(own[0] < 3, "only the solution right after anneal_each uses the per-puzzle best: {:?}", out);
    }

    #[test]
    fn the_column_encoding_factors_143_and_never_a_prime() {
        let m = model_of("model :p do\n  factor :f, number: 143, encoding: :columns\n  factor :g, number: 127, encoding: :columns\nend");
        let mut found = 0;
        for seed in 0..10 {
            let mut st = State::new(seed);
            st.temp = 3.0;
            let mut m2 = m.clone();
            anneal_each(&mut m2, &mut st, 5_000);
            let (sf, _) = judged_arrangement(&m2, &st, "f").unwrap();
            let (sg, _) = judged_arrangement(&m2, &st, "g").unwrap();
            found += judge(&m2, "f", &sf).unwrap().0 as usize;
            assert!(!judge(&m2, "g", &sg).unwrap().0, "negative control: 127 is prime");
        }
        assert!(found >= 3, "143 found in only {} of 10", found);
    }

    #[test]
    fn zoo_errors_name_their_line() {
        let cases = [
            ("model :m do\n  sudoku :s, size: 4, given: \"12\"\nend", "line 2: a 4x4 sudoku needs 16 cells"),
            ("model :m do\n  sudoku :s, size: 5\nend", "line 2: sudoku size is 4 or 9"),
            ("model :m do\n  colouring :g, edges: \"a-a\"\nend", "line 2: 'a-a' joins a node to itself"),
            ("model :m do\n  maxcut :g\nend", "line 2: needs edges:"),
            ("model :m do\n  factor :f, number: 16\nend", "line 2: factor takes an odd"),
            ("model :m do\n  factor :f, number: 15, encoding: :wires\nend", "line 2: encoding is :columns"),
            ("model :m do\n  maxcut :g, edges: \"a-b\"\nend\nrun :m do\n  g.solution\nend", "line 5: solution needs an anneal first"),
        ];
        for (src, want) in cases {
            let e = Interp::default().exec(src).err().map(|e| e.0).unwrap_or_default();
            assert!(e.starts_with(want), "{:?} gave {:?}", src, e);
        }
    }
    // ── NONOGRAM (PUZZLEFEATURE) ──

    fn grid_of(rows: &[&str]) -> Vec<bool> {
        rows.iter().flat_map(|r| r.chars().map(|c| c == '#')).collect()
    }

    fn clues_of(rows: &[&str]) -> (Vec<Vec<usize>>, Vec<Vec<usize>>) {
        let (h, w) = (rows.len(), rows[0].len());
        let g = grid_of(rows);
        let r = (0..h).map(|i| line_clue(&g[i * w..(i + 1) * w])).collect();
        let c = (0..w).map(|j| line_clue(&(0..h).map(|i| g[i * w + j]).collect::<Vec<_>>())).collect();
        (r, c)
    }

    #[test]
    fn nonogram_cost_is_zero_exactly_at_the_pictures_that_fit_every_clue() {
        // brute force over every arrangement of a 3x4 puzzle: the least cost is 0, every zero-cost arrangement
        // decodes to a grid that passes the plain check, and every grid that passes it has a zero-cost arrangement
        let pic = ["#.##", ".##.", "##.#"];
        let (rows, cols) = clues_of(&pic);
        let (q, lay) = nonogram_qubo(&rows, &cols, 1.0, 1.0, 1.0).unwrap();
        assert!(lay.nvars <= 24, "{} variables", lay.nvars);
        let mut zero = 0;
        let mut least = f64::INFINITY;
        for bits in 0u32..(1u32 << lay.nvars) {
            let y: Vec<bool> = (0..lay.nvars).map(|k| (bits >> k) & 1 == 1).collect();
            let e = q.energy(&y);
            least = least.min(e);
            if e.abs() < 1e-9 {
                zero += 1;
                assert!(nonogram_check(&rows, &cols, &y[..12]).is_ok());
            }
            assert!(e > -1e-9, "a cost below zero");
        }
        assert_eq!(least, 0.0);
        let mut valid = 0;
        for g in 0u32..(1 << 12) {
            let grid: Vec<bool> = (0..12).map(|k| (g >> k) & 1 == 1).collect();
            if nonogram_check(&rows, &cols, &grid).is_ok() {
                valid += 1;
                let y = nonogram_assign(&lay, &grid).expect("a valid grid has walls");
                assert_eq!(q.energy(&y), 0.0);
            }
        }
        assert_eq!(zero, valid, "one zero-cost arrangement per valid picture");
        // negative control: clues no picture satisfies (rows fill 3 cells, columns 2) never reach zero
        let (q2, lay2) = nonogram_qubo(&parse_clues("1/1/1").unwrap(), &parse_clues("1/1/0").unwrap(), 1.0, 1.0, 1.0).unwrap();
        let least2 = (0u32..(1 << lay2.nvars))
            .map(|b| q2.energy(&(0..lay2.nvars).map(|k| (b >> k) & 1 == 1).collect::<Vec<_>>()))
            .fold(f64::INFINITY, f64::min);
        assert!(least2 >= 1.0 - 1e-9, "least {}", least2);
    }

    #[test]
    fn annealing_draws_the_heart_and_an_impossible_nonogram_is_never_valid() {
        let out = Interp::default()
            .exec("model :p do\n  nonogram :n, rows: \"1 1/5/5/3/1\", cols: \"2/4/4/4/2\"\nend\nrun :p do\n  anneal 2_000, seed: 1\n  n.solution\nend")
            .unwrap();
        assert!(out.last().unwrap().contains("nonogram :n: VALID"), "{:?}", out);
        assert!(out.iter().any(|l| l.trim() == ".#.#."), "{:?}", out);
        let m = model_of("model :p do\n  nonogram :n, rows: \"1/1/1\", cols: \"1/1/0\"\nend");
        for seed in 0..10 {
            let mut st = State::new(seed);
            st.anneal(&m, 1_000);
            let (ok, lines) = judge(&m, "n", &st.best.as_ref().unwrap().0).unwrap();
            assert!(!ok && lines.last().unwrap().contains("NOT VALID"), "{:?}", lines);
        }
    }

    #[test]
    fn the_rust_and_browser_nonogram_encodings_have_the_same_size() {
        // SETTLE/settle-site/tests/nonogram.test.mjs pins the same counts for the duck: 175 things
        let duck = ["...###....", "..#####...", "..##.###..", ".....####.", "..######..", ".########.", "##########", "##########", ".########.", "..######.."];
        let (rows, cols) = clues_of(&duck);
        let (q, lay) = nonogram_qubo(&rows, &cols, 1.0, 1.0, 1.0).unwrap();
        assert_eq!(lay.nvars, 175);
        assert_eq!(q.energy(&nonogram_assign(&lay, &grid_of(&duck)).unwrap()), 0.0);
        assert_eq!(parse_clues("1 1/0//3").unwrap(), vec![vec![1, 1], vec![], vec![], vec![3]]);
        assert!(nonogram_layout(&parse_clues("3").unwrap(), &parse_clues("1/1").unwrap()).is_err());
    }
}

/// Success-rate measurements over 50 seeds. Slow; run by hand:
/// `cargo test --release measure_ -- --ignored --nocapture --test-threads 1`
#[cfg(test)]
mod measure {
    use super::*;
    use crate::interp::Interp;
    use crate::rng::Rng;
    use std::time::Instant;

    const SEEDS: u64 = 50;

    fn build(src: &str) -> Model {
        let mut it = Interp::default();
        it.exec(src).unwrap_or_else(|e| panic!("{}", e));
        it.models.values().next().unwrap().clone()
    }

    /// Fraction of seeds whose annealed answer passes the checker, with wall time. Seeds run in parallel.
    fn rate(m: &Model, name: &str, sweeps: usize, temp: f64) -> (usize, f64) {
        let t0 = Instant::now();
        let ok = std::thread::scope(|sc| {
            let hs: Vec<_> = (0..SEEDS)
                .map(|seed| {
                    sc.spawn(move || {
                        let mut st = State::new(1_000 + seed);
                        st.temp = temp;
                        st.anneal(m, sweeps);
                        judge(m, name, &st.best.as_ref().unwrap().0).unwrap().0 as usize
                    })
                })
                .collect();
            hs.into_iter().map(|h| h.join().unwrap()).sum::<usize>()
        });
        (ok, t0.elapsed().as_secs_f64())
    }

    fn pct(ok: usize) -> String {
        format!("{:.0}%", 100.0 * ok as f64 / SEEDS as f64)
    }

    fn sudoku_src(n: usize, g: &str, extra: &str) -> String {
        format!("model :p do\n  sudoku :s, size: {}, given: \"{}\"{}\nend", n, g, extra)
    }

    const EASY9: &str = "53..7.... 6..195... .98....6. 8...6...3 4..8.3..1 7...2...6 .6....28. ...419..5 ....8..79";
    const EASY9_SOLVED: &str = "534678912 672195348 198342567 859761423 426853791 713924856 961537284 287419635 345286179";
    const BAD9: &str = "55..7.... 6..195... .98....6. 8...6...3 4..8.3..1 7...2...6 .6....28. ...419..5 ....8..79";

    #[test]
    #[ignore]
    fn measure_sudoku() {
        let g9 = parse_givens(EASY9, 9).unwrap();
        assert!(sudoku_check(9, &g9, &parse_givens(EASY9_SOLVED, 9).unwrap()).is_ok(), "reference solution is valid");
        println!("\n| puzzle | given lean | sweeps | valid of 50 | seconds |\n|---|---|---|---|---|");
        let four = sudoku_src(4, "1... .4.. ..4. ...1", "");
        let m = build(&four);
        for sw in [100, 500, 2_000] {
            let (ok, t) = rate(&m, "s", sw, 1.0);
            println!("| 4x4, 4 givens | 4A | {} | {} | {:.2} |", sw, pct(ok), t);
        }
        let m1 = build(&sudoku_src(4, "1... .4.. ..4. ...1", ", given_by: 1"));
        let (ok, t) = rate(&m1, "s", 2_000, 1.0);
        println!("| 4x4, 4 givens | A | 2000 | {} | {:.2} |", pct(ok), t);
        let m9 = build(&sudoku_src(9, EASY9, ""));
        for sw in [2_000, 10_000, 50_000] {
            let (ok, t) = rate(&m9, "s", sw, 1.0);
            println!("| 9x9 easy, 30 givens | 4A | {} | {} | {:.1} |", sw, pct(ok), t);
        }
        // negative controls
        let bad4 = build(&sudoku_src(4, "1.1. .... .... ....", ""));
        let (ok, _) = rate(&bad4, "s", 2_000, 1.0);
        println!("| NEG 4x4, two 1s in row 1 | 4A | 2000 | {} | |", pct(ok));
        let bad9 = build(&sudoku_src(9, BAD9, ""));
        let (ok, _) = rate(&bad9, "s", 10_000, 1.0);
        println!("| NEG 9x9, two 5s in row 1 | 4A | 10000 | {} | |", pct(ok));
    }

    /// A planted 3-colourable graph: hidden colours, then `edges` distinct edges between different colours.
    fn planted(n: usize, edges: usize, seed: u64) -> String {
        let mut r = Rng::new(seed);
        let col: Vec<usize> = (0..n).map(|_| r.below(3)).collect();
        let mut set = std::collections::BTreeSet::new();
        while set.len() < edges {
            let (a, b) = (r.below(n), r.below(n));
            if a != b && col[a] != col[b] {
                set.insert((a.min(b), a.max(b)));
            }
        }
        set.iter().map(|(a, b)| format!("v{}-v{}", a, b)).collect::<Vec<_>>().join(" ")
    }

    #[test]
    #[ignore]
    fn measure_colouring() {
        println!("\n| nodes | edges | sweeps | proper of 50 | seconds |\n|---|---|---|---|---|");
        for n in [10, 20, 40, 80] {
            let e = planted(n, 2 * n, 77 + n as u64);
            let g = parse_edges(&e).unwrap();
            if g.nodes.len() <= 20 {
                assert!(colouring_exists(&g, 3), "planted graph is 3-colourable by brute force");
            }
            let m = build(&format!("model :p do\n  colouring :g, colours: 3, edges: \"{}\"\nend", e));
            for sw in [500, 2_000, 10_000] {
                let (ok, t) = rate(&m, "g", sw, 1.0);
                println!("| {} ({} used) | {} | {} | {} | {:.2} |", n, g.nodes.len(), g.edges.len(), sw, pct(ok), t);
            }
        }
        for (label, e, k) in [("NEG triangle, 2 colours", "a-b b-c c-a", 2), ("NEG K4, 3 colours", "a-b a-c a-d b-c b-d c-d", 3)] {
            assert!(!colouring_exists(&parse_edges(e).unwrap(), k));
            let m = build(&format!("model :p do\n  colouring :g, colours: {}, edges: \"{}\"\nend", k, e));
            let (ok, _) = rate(&m, "g", 2_000, 1.0);
            println!("| {} | | 2000 | {} | |", label, pct(ok));
        }
    }

    fn gnp(n: usize, seed: u64) -> String {
        let mut r = Rng::new(seed);
        let mut out = Vec::new();
        for a in 0..n {
            for b in a + 1..n {
                if r.unit() < 0.5 {
                    out.push(format!("v{}-v{}", a, b));
                }
            }
        }
        out.join(" ")
    }

    #[test]
    #[ignore]
    fn measure_maxcut() {
        println!("\n| nodes | edges | exact best cut | sweeps | optimal of 50 | mean cut / best | seconds |\n|---|---|---|---|---|---|---|");
        for n in [8, 12, 16, 20] {
            let e = gnp(n, 500 + n as u64);
            let g = parse_edges(&e).unwrap();
            let best = maxcut_exact(&g);
            let m = build(&format!("model :p do\n  maxcut :g, edges: \"{}\"\nend", e));
            let start = m.notes["zoo:g"].0[0] as usize;
            for sw in [200, 1_000, 5_000] {
                let (ok, t) = rate(&m, "g", sw, 1.0);
                let mut sum = 0.0;
                for seed in 0..SEEDS {
                    let mut st = State::new(1_000 + seed);
                    st.anneal(&m, sw);
                    sum += cut_value(&g, &bits_of(&st.best.as_ref().unwrap().0[start..start + g.nodes.len()]));
                }
                println!("| {} ({} used) | {} | {} | {} | {} | {:.3} | {:.2} |", n, g.nodes.len(), g.edges.len(), best, sw, pct(ok), sum / SEEDS as f64 / best, t);
            }
        }
        let m = build("model :p do\n  maxcut :t, edges: \"a-b b-c c-a\", target: 3\nend");
        let (ok, _) = rate(&m, "t", 1_000, 1.0);
        println!("| NEG triangle, target 3 | 3 | 2 | 1000 | {} | | |", pct(ok));
    }

    /// Plain baseline: draw `samples` uniformly random arrangements, keep the calmest, check it.
    /// Same number of arrangements as the anneal's sweeps; no settling at all.
    fn random_rate(m: &Model, name: &str, samples: usize) -> usize {
        std::thread::scope(|sc| {
            let hs: Vec<_> = (0..SEEDS)
                .map(|seed| {
                    sc.spawn(move || {
                        let mut r = Rng::new(9_000 + seed);
                        let mut best = (Vec::new(), f64::INFINITY);
                        for _ in 0..samples {
                            let s: Vec<f64> = (0..m.len()).map(|_| if r.unit() < 0.5 { -1.0 } else { 1.0 }).collect();
                            let e = m.energy(&s);
                            if e < best.1 {
                                best = (s, e);
                            }
                        }
                        judge(m, name, &best.0).unwrap().0 as usize
                    })
                })
                .collect();
            hs.into_iter().map(|h| h.join().unwrap()).sum::<usize>()
        })
    }

    /// Post-hoc (not sealed): the random-guess baseline at the largest sweep count of every sealed row,
    /// and a sparser max-cut family because G(n, 1/2) turned out easy.
    #[test]
    #[ignore]
    fn measure_baseline() {
        println!("\n| puzzle | arrangements | anneal (from sealed runs) | random, keep calmest |\n|---|---|---|---|");
        let four = build(&sudoku_src(4, "1... .4.. ..4. ...1", ""));
        println!("| sudoku 4x4 | 2000 | see table | {} |", pct(random_rate(&four, "s", 2_000)));
        let nine = build(&sudoku_src(9, EASY9, ""));
        println!("| sudoku 9x9 easy | 50000 | see table | {} |", pct(random_rate(&nine, "s", 50_000)));
        for n in [10, 20, 40, 80] {
            let m = build(&format!("model :p do\n  colouring :g, colours: 3, edges: \"{}\"\nend", planted(n, 2 * n, 77 + n as u64)));
            println!("| colouring n={} | 10000 | see table | {} |", n, pct(random_rate(&m, "g", 10_000)));
        }
        for n in [8, 12, 16, 20] {
            let m = build(&format!("model :p do\n  maxcut :g, edges: \"{}\"\nend", gnp(n, 500 + n as u64)));
            println!("| maxcut G(n,1/2) n={} | 200 | see table | {} |", n, pct(random_rate(&m, "g", 200)));
        }
        for n in [15u64, 21, 35, 77, 143] {
            let m = build(&format!("model :p do\n  factor :f, number: {}\nend", n));
            println!("| factor {} | 1000 | see table | {} |", n, pct(random_rate(&m, "f", 1_000)));
            println!("| factor {} | 10000 | see table | {} |", n, pct(random_rate(&m, "f", 10_000)));
        }
        println!("\n| sparse maxcut, avg degree 3 | edges | exact best | sweeps | anneal optimal of 50 | random optimal of 50 |\n|---|---|---|---|---|---|");
        for n in [16, 20, 22] {
            let mut r = Rng::new(700 + n as u64);
            let mut e = Vec::new();
            for a in 0..n {
                for b in a + 1..n {
                    if r.unit() < 3.0 / (n - 1) as f64 {
                        e.push(format!("v{}-v{}", a, b));
                    }
                }
            }
            let e = e.join(" ");
            let g = parse_edges(&e).unwrap();
            let best = maxcut_exact(&g);
            let m = build(&format!("model :p do\n  maxcut :g, edges: \"{}\"\nend", e));
            for sw in [50, 200, 1_000] {
                let (ok, _) = rate(&m, "g", sw, 1.0);
                println!("| n={} ({} used) | {} | {} | {} | {} | {} |", n, g.nodes.len(), g.edges.len(), best, sw, pct(ok), pct(random_rate(&m, "g", sw)));
            }
        }
    }

    /// Post-hoc (not sealed): puzzles sharing one model share the anneal's "calmest so far", so a puzzle
    /// is judged on the arrangement that is calmest for the SUM. How much does a neighbour cost?
    #[test]
    #[ignore]
    fn measure_shared_model() {
        println!("\n| model | T | sweeps | factor 143 valid of 50 |\n|---|---|---|---|");
        // T = 1158.4 is the sealed max/10 rule for 143 alone; T = 115.8 is ten times colder.
        let alone = build("model :p do\n  factor :f, number: 143\nend");
        let pair = build("model :p do\n  factor :f, number: 143\n  factor :g, number: 127\nend");
        for t in [1158.4, 115.8] {
            for (label, m) in [("143 alone", &alone), ("143 beside prime 127", &pair)] {
                let (ok, _) = rate(m, "f", 10_000, t);
                println!("| {} | {} | 10000 | {} |", label, t, pct(ok));
            }
        }
    }

    /// Chance that uniformly random free bits decode to a valid factor pair.
    fn chance(n: u64) -> f64 {
        let (pb, qb) = factor_widths(n);
        let (mut ok, mut all) = (0u64, 0u64);
        for pf in 0u64..(1 << (pb - 1)) {
            for qf in 0u64..(1 << (qb - 1)) {
                all += 1;
                ok += factor_check(n, (pf << 1) | 1, (qf << 1) | 1).is_ok() as u64;
            }
        }
        ok as f64 / all as f64
    }

    #[test]
    #[ignore]
    fn measure_factor() {
        println!("\n| N | things | chance | T | sweeps | valid of 50 | seconds |\n|---|---|---|---|---|---|---|");
        for n in [15u64, 21, 35, 77, 143, 31, 97, 127] {
            let m = build(&format!("model :p do\n  factor :f, number: {}\nend", n));
            let big = (0..m.len())
                .map(|i| m.h[i].abs().max(m.adj[i].iter().map(|e| e.1.abs()).fold(0.0, f64::max)))
                .fold(0.0, f64::max);
            let auto = big / 10.0;
            let tag = if [31, 97, 127].contains(&n) { "NEG prime " } else { "" };
            for (tl, t, sw) in [("max/10", auto, 1_000), ("max/10", auto, 10_000), ("5", 5.0, 10_000)] {
                let (ok, secs) = rate(&m, "f", sw, t);
                println!("| {}{} | {} | {:.1}% | {} = {:.1} | {} | {} | {:.2} |", tag, n, m.len(), 100.0 * chance(n), tl, t, sw, pct(ok), secs);
            }
        }
    }
}

