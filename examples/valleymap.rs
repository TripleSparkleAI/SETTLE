//! VALLEYMAP measurement driver: every table in runs/valleymap/REPORT_VALLEYMAP.md comes from here.
//!
//!   cargo run --release --example valleymap -- <part> [threads]
//!
//! Parts: random · shuffled · controls · grid · hopfield · code · info · files · optimise · all
//! Each part prints plain text tables. Seeds are fixed, so a rerun prints the same numbers.

use settle::interp::Interp;
use settle::engine::codes::code;
use settle::model::{Model, State};
use settle::rng::Rng;
use settle::valleys::*;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

fn par_map<T: Send, F: Fn(u64) -> T + Sync>(items: &[u64], threads: usize, f: F) -> Vec<T> {
    let next = AtomicUsize::new(0);
    let out: Mutex<Vec<(usize, T)>> = Mutex::new(Vec::new());
    std::thread::scope(|sc| {
        for _ in 0..threads.max(1) {
            sc.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::SeqCst);
                if i >= items.len() {
                    break;
                }
                let r = f(items[i]);
                out.lock().unwrap().push((i, r));
            });
        }
    });
    let mut v = out.into_inner().unwrap();
    v.sort_by_key(|x| x.0);
    v.into_iter().map(|x| x.1).collect()
}

fn sk(n: usize, seed: u64) -> (Model, Dense) {
    let mut m = Model::default();
    random_landscape(&mut m, n, seed, 1.0, 0.0);
    let d = Dense::of(&m, &HashMap::new());
    (m, d)
}

fn mean(x: &[f64]) -> f64 {
    x.iter().sum::<f64>() / x.len().max(1) as f64
}
fn sd(x: &[f64]) -> f64 {
    let m = mean(x);
    (x.iter().map(|v| (v - m) * (v - m)).sum::<f64>() / (x.len().max(2) - 1) as f64).sqrt()
}
fn median(mut x: Vec<f64>) -> f64 {
    x.sort_by(|a, b| a.partial_cmp(b).unwrap());
    if x.is_empty() {
        0.0
    } else {
        x[x.len() / 2]
    }
}
fn ranks(x: &[f64]) -> Vec<f64> {
    let mut idx: Vec<usize> = (0..x.len()).collect();
    idx.sort_by(|&a, &b| x[a].partial_cmp(&x[b]).unwrap());
    let mut r = vec![0.0; x.len()];
    let mut i = 0;
    while i < idx.len() {
        let mut j = i;
        while j + 1 < idx.len() && x[idx[j + 1]] == x[idx[i]] {
            j += 1;
        }
        for k in i..=j {
            r[idx[k]] = (i + j) as f64 / 2.0;
        }
        i = j + 1;
    }
    r
}
fn spearman(a: &[f64], b: &[f64]) -> f64 {
    let (ra, rb) = (ranks(a), ranks(b));
    let (ma, mb) = (mean(&ra), mean(&rb));
    let cov: f64 = ra.iter().zip(&rb).map(|(x, y)| (x - ma) * (y - mb)).sum();
    let va: f64 = ra.iter().map(|x| (x - ma).powi(2)).sum();
    let vb: f64 = rb.iter().map(|y| (y - mb).powi(2)).sum();
    if va == 0.0 || vb == 0.0 {
        0.0
    } else {
        cov / (va * vb).sqrt()
    }
}
fn slope(x: &[f64], y: &[f64]) -> f64 {
    let (mx, my) = (mean(x), mean(y));
    x.iter().zip(y).map(|(a, b)| (a - mx) * (b - my)).sum::<f64>() / x.iter().map(|a| (a - mx).powi(2)).sum::<f64>()
}
fn bits_to_state(c: u32, n: usize) -> Vec<f64> {
    (0..n).map(|i| if (c >> i) & 1 == 1 { 1.0 } else { -1.0 }).collect()
}

// ---------------------------------------------------------------------------------------------------------

struct SkRow {
    v: f64,
    ground_share: f64,
    median_share: f64,
    tiny_frac: f64,
    rho: f64,
    mean_absq: f64,
    shares: Vec<f64>,
}

fn part_random(threads: usize) {
    println!("== RANDOM PULLS, exact enumeration: J_ij ~ N(0,1)/sqrt(n), no leans ==");
    println!("{:>3} {:>6} {:>9} {:>8} {:>9} {:>8} {:>10} {:>10} {:>9} {:>8} {:>8}", "n", "seeds", "mean V", "sd V", "ln meanV", "meanlnV", "ground %", "median %", "<1% frac", "rho", "mean|q|");
    let plan: [(usize, u64); 8] = [(10, 200), (12, 200), (14, 200), (16, 200), (18, 100), (20, 50), (22, 20), (24, 10)];
    let (mut xs, mut ys) = (Vec::new(), Vec::new());
    let mut pooled20 = Vec::new();
    for (n, seeds) in plan {
        let ids: Vec<u64> = (1..=seeds).collect();
        let rows = par_map(&ids, if n >= 22 { threads.min(4) } else { threads }, |seed| {
            let (_, d) = sk(n, seed);
            let c = enumerate(&d);
            let t = c.total() as f64;
            let shares: Vec<f64> = c.valleys.iter().map(|v| v.basin as f64 / t).collect();
            let energies: Vec<f64> = c.valleys.iter().map(|v| v.energy).collect();
            let g = c.valleys[0].energy;
            let ground_share = c.valleys.iter().filter(|v| (v.energy - g).abs() < 1e-9).map(|v| v.basin as f64 / t).sum();
            let states: Vec<Vec<f64>> = c.valleys.iter().map(|v| bits_to_state(v.rep, n)).collect();
            let mut qs = Vec::new();
            for a in 0..states.len() {
                for b in (a + 1)..states.len() {
                    let q = overlap(&states[a], &states[b]).abs();
                    if q < 0.999 {
                        qs.push(q);
                    }
                }
            }
            SkRow {
                v: c.valleys.len() as f64,
                ground_share,
                median_share: median(shares.clone()),
                tiny_frac: shares.iter().filter(|&&s| s < 0.01).count() as f64 / shares.len() as f64,
                rho: spearman(&shares, &energies),
                mean_absq: mean(&qs),
                shares,
            }
        });
        let v: Vec<f64> = rows.iter().map(|r| r.v).collect();
        let lnv: Vec<f64> = v.iter().map(|x| x.ln()).collect();
        let rhos: Vec<f64> = rows.iter().filter(|r| r.v > 2.0).map(|r| r.rho).collect();
        println!(
            "{:>3} {:>6} {:>9.2} {:>8.2} {:>9.3} {:>8.3} {:>10.1} {:>10.2} {:>9.2} {:>8.2} {:>8.3}",
            n,
            seeds,
            mean(&v),
            sd(&v),
            mean(&v).ln(),
            mean(&lnv),
            100.0 * mean(&rows.iter().map(|r| r.ground_share).collect::<Vec<_>>()),
            100.0 * mean(&rows.iter().map(|r| r.median_share).collect::<Vec<_>>()),
            mean(&rows.iter().map(|r| r.tiny_frac).collect::<Vec<_>>()),
            mean(&rhos),
            mean(&rows.iter().map(|r| r.mean_absq).collect::<Vec<_>>()),
        );
        xs.push(n as f64);
        ys.push(mean(&v).ln());
        if n == 20 {
            for r in rows {
                pooled20.extend(r.shares);
            }
        }
    }
    println!("fitted slope of ln(mean V) against n, n = 10..24: {:.4} per thing ({:.4} bits)", slope(&xs, &ys), slope(&xs, &ys) / 2f64.ln());
    let (xl, yl): (Vec<f64>, Vec<f64>) = xs.iter().zip(&ys).filter(|(x, _)| **x >= 16.0).map(|(a, b)| (*a, *b)).unzip();
    println!("fitted slope over n = 16..24 only: {:.4} per thing", slope(&xl, &yl));
    println!("basin-share histogram, n = 20, all valleys of 50 landscapes pooled ({} valleys):", pooled20.len());
    let edges = [0.0, 0.0001, 0.001, 0.01, 0.05, 0.1, 0.2, 0.3, 0.5, 1.01];
    for w in edges.windows(2) {
        let c = pooled20.iter().filter(|&&s| s >= w[0] && s < w[1]).count();
        println!("  {:>7.2}% to {:>7.2}%  {:>5}  {}", 100.0 * w[0], 100.0 * w[1].min(1.0), c, "#".repeat((c as f64 / 4.0).ceil() as usize));
    }
}

fn part_shuffled(threads: usize) {
    println!("== SHUFFLED-ENERGY CONTROL: same energies, placed at random over the arrangements ==");
    println!("{:>3} {:>6} {:>12} {:>10} {:>14} {:>12} {:>12}", "n", "seeds", "shuffled V", "sd", "2^n/(n+1)", "random V", "ratio");
    for (n, seeds) in [(10usize, 40u64), (12, 40), (14, 20), (16, 20), (18, 10), (20, 5)] {
        let ids: Vec<u64> = (1..=seeds).collect();
        let rows = par_map(&ids, threads.min(6), |seed| {
            let (_, d) = sk(n, seed);
            let straight = enumerate(&d).valleys.len() as f64;
            let mut e = energy_table(&d);
            let mut r = Rng::new(seed ^ 0xabcdef);
            for k in (1..e.len()).rev() {
                let j = r.below(k + 1);
                e.swap(k, j);
            }
            (enumerate_table(n, &e).valleys.len() as f64, straight)
        });
        let sh: Vec<f64> = rows.iter().map(|r| r.0).collect();
        let st: Vec<f64> = rows.iter().map(|r| r.1).collect();
        let want = (1u64 << n) as f64 / (n as f64 + 1.0);
        println!("{:>3} {:>6} {:>12.1} {:>10.1} {:>14.1} {:>12.2} {:>12.5}", n, seeds, mean(&sh), sd(&sh), want, mean(&st), mean(&st) / mean(&sh));
    }
}

fn describe(c: &Census) -> String {
    let t = c.total() as f64;
    let parts: Vec<String> = c.valleys.iter().map(|v| format!("{:.3}:{:.2}%{}", v.energy, 100.0 * v.basin as f64 / t, if v.floor > 1 { format!("(floor {})", v.floor) } else { String::new() })).collect();
    format!("V = {}, saddle {:.2}%  [{}]", c.valleys.len(), 100.0 * c.saddle as f64 / t, parts.join(", "))
}

fn part_controls() {
    println!("== CONTROLS AND REGULAR LANDSCAPES, exact ==");
    let mut m = Model::default();
    let mut r = Rng::new(7);
    for i in 0..16 {
        m.add(&format!("t{}", i));
        m.h[i] = r.normal();
    }
    let c = enumerate(&Dense::of(&m, &HashMap::new()));
    let want = (0..16).fold(0u32, |a, i| if m.h[i] > 0.0 { a | (1 << i) } else { a });
    println!("zero pulls, 16 random leans: V = {}, at the lean signs: {}, basin {:.1}%", c.valleys.len(), c.valleys[0].rep == want, 100.0 * c.valleys[0].basin as f64 / c.total() as f64);
    let mut m = Model::default();
    for i in 0..12 {
        m.add(&format!("t{}", i));
    }
    let c = enumerate(&Dense::of(&m, &HashMap::new()));
    println!("zero pulls, zero leans, 12 things: V = {}, flat floor of {} arrangements", c.valleys.len(), c.valleys[0].floor);
    for n in [8, 12, 16, 20] {
        let mut m = Model::default();
        ring(&mut m, n);
        println!("ring of {}: {}", n, describe(&enumerate(&Dense::of(&m, &HashMap::new()))));
    }
    for (w, h, wrap) in [(4, 4, true), (4, 5, true), (3, 6, true), (4, 4, false), (4, 5, false)] {
        let mut m = Model::default();
        grid(&mut m, w, h, wrap);
        let c = enumerate(&Dense::of(&m, &HashMap::new()));
        let t = c.total() as f64;
        let (mut ground, mut stripes, mut other) = ((0, 0.0), (0, 0.0), (0, 0.0));
        for v in &c.valleys {
            let s = bits_to_state(v.rep, w * h);
            let share = v.basin as f64 / t;
            let rows_same = (0..h).all(|y| (0..w).all(|x| s[y * w + x] == s[y * w]));
            let cols_same = (0..w).all(|x| (0..h).all(|y| s[y * w + x] == s[x]));
            let all_same = s.iter().all(|&q| q == s[0]);
            let slot = if all_same { &mut ground } else if rows_same || cols_same { &mut stripes } else { &mut other };
            slot.0 += 1;
            slot.1 += share;
        }
        println!(
            "grid {}x{} {}: V = {} (uniform {} holding {:.1}%, straight stripes {} holding {:.1}%, other {} holding {:.1}%), flat saddles {:.1}%",
            w,
            h,
            if wrap { "wrapped" } else { "open" },
            c.valleys.len(),
            ground.0,
            100.0 * ground.1,
            stripes.0,
            100.0 * stripes.1,
            other.0,
            100.0 * other.1,
            100.0 * c.saddle as f64 / t
        );
    }
}

fn grid_kind(s: &[f64], w: usize, h: usize) -> &'static str {
    if s.iter().all(|&q| q == s[0]) {
        return "uniform";
    }
    let rows_same = (0..h).all(|y| (0..w).all(|x| s[y * w + x] == s[y * w]));
    let cols_same = (0..w).all(|x| (0..h).all(|y| s[y * w + x] == s[x]));
    if rows_same || cols_same {
        "straight stripes"
    } else {
        "other"
    }
}

fn part_grid(threads: usize) {
    println!("== WRAPPED GRIDS, survey (sampled) ==");
    for (l, starts, sweeps) in [(8usize, 400usize, 1000usize), (16, 400, 2000)] {
        let chunks: Vec<u64> = (0..8).collect();
        let res = par_map(&chunks, threads, |ch| {
            let mut m = Model::default();
            grid(&mut m, l, l, true);
            let mut st = State::new(1000 + ch);
            st.temp = 0.05;
            survey(&m, &mut st, starts / 8, sweeps, 0.05)
        });
        let mut kinds: HashMap<&str, (usize, u64)> = HashMap::new();
        let mut distinct: HashSet<Vec<u64>> = HashSet::new();
        let mut flat = 0u64;
        for f in res.iter().flatten() {
            let e = kinds.entry(grid_kind(&f.state, l, l)).or_default();
            e.1 += f.count;
            if distinct.insert(key(&f.state)) {
                e.0 += 1;
            }
            if f.flat {
                flat += f.count;
            }
        }
        let mut ks: Vec<_> = kinds.into_iter().collect();
        ks.sort();
        let parts: Vec<String> = ks.iter().map(|(k, (v, c))| format!("{} {:.1}% of starts ({} distinct)", k, 100.0 * *c as f64 / starts as f64, v)).collect();
        println!("grid {}x{} ({} things), {} starts x {} sweeps at T 0.05: {} distinct valleys; {}; flat stops {:.1}%", l, l, l * l, starts, sweeps, distinct.len(), parts.join(" · "), 100.0 * flat as f64 / starts as f64);
    }
}

fn hopfield(n: usize, p: usize, set: u64) -> Model {
    let mut src = format!("model :h do\n  memory :m, size: {}\n", n);
    for k in 0..p {
        src.push_str(&format!("  m.remember :s{}_p{}\n", set, k));
    }
    src.push_str("end\n");
    let mut it = Interp::default();
    it.exec(&src).unwrap();
    it.models.remove("h").unwrap()
}

fn part_hopfield(threads: usize) {
    println!("== HOPFIELD MEMORIES, exact at N = 20 (10 pattern sets per p) ==");
    println!("{:>2} {:>7} {:>13} {:>14} {:>11} {:>13} {:>16}", "p", "mean V", "stored+mirror", "their basin %", "fake V", "fake basin %", "sets losing one");
    for p in 1..=8usize {
        let sets: Vec<u64> = (1..=10).collect();
        let rows = par_map(&sets, threads, |set| {
            let m = hopfield(20, p, set);
            let c = enumerate(&Dense::of(&m, &HashMap::new()));
            let t = c.total() as f64;
            let (mut sv, mut sb, mut fv, mut fb) = (0.0, 0.0, 0.0, 0.0);
            let mut have: HashSet<String> = HashSet::new();
            for v in &c.valleys {
                match classify(&m, &bits_to_state(v.rep, 20)) {
                    Kind::Stored(n) | Kind::Mirror(n) => {
                        sv += 1.0;
                        sb += v.basin as f64 / t;
                        have.insert(n);
                    }
                    _ => {
                        fv += 1.0;
                        fb += v.basin as f64 / t;
                    }
                }
            }
            (c.valleys.len() as f64, sv, sb, fv, fb, (have.len() < p) as u8 as f64)
        });
        type Row6 = (f64, f64, f64, f64, f64, f64);
        let col = |f: &dyn Fn(&Row6) -> f64| mean(&rows.iter().map(f).collect::<Vec<_>>());
        println!(
            "{:>2} {:>7.1} {:>13.1} {:>14.1} {:>11.1} {:>13.1} {:>13}/10",
            p,
            col(&|r| r.0),
            col(&|r| r.1),
            100.0 * col(&|r| r.2),
            col(&|r| r.3),
            100.0 * col(&|r| r.4),
            rows.iter().filter(|r| r.5 > 0.0).count()
        );
    }
    println!("== HOPFIELD MEMORIES, survey at N = 200 (400 random starts, 30 sweeps at T 0.05) ==");
    println!("{:>3} {:>6} {:>16} {:>9} {:>10} {:>8}", "p", "alpha", "stored+mirror %", "fake %", "valleys", "Chao1");
    let ps: Vec<u64> = vec![1, 2, 5, 10, 15, 20, 25, 30, 40];
    let rows = par_map(&ps, threads, |p| {
        let m = hopfield(200, p as usize, 1);
        let mut st = State::new(p);
        let f = survey(&m, &mut st, 400, 30, 0.05);
        let mut good = 0;
        for x in &f {
            if matches!(classify(&m, &x.state), Kind::Stored(_) | Kind::Mirror(_)) {
                good += x.count;
            }
        }
        (p, good as f64 / 400.0, f.len(), chao1(&f))
    });
    for (p, g, v, ch) in rows {
        println!("{:>3} {:>6.3} {:>16.1} {:>9.1} {:>10} {:>8.0}", p, p as f64 / 200.0, 100.0 * g, 100.0 * (1.0 - g), v, ch);
    }
    println!("== HOPFIELD RECALL FROM DAMAGE vs NEAREST-PATTERN LOOKUP (N = 200, 300 trials each) ==");
    println!("{:>3} {:>7} {:>16} {:>18}", "p", "damage", "landscape recall", "nearest lookup");
    let cases: Vec<u64> = vec![10_20, 10_30, 10_40, 20_20, 20_30, 25_20, 30_20];
    let rows = par_map(&cases, threads, |cs| {
        let (p, dmg) = ((cs / 100) as usize, (cs % 100) as f64 / 100.0);
        let m = hopfield(200, p, 1);
        let pats = &stored_patterns(&m)[0].patterns;
        let mut st = State::new(cs);
        let (mut ok, mut near) = (0, 0);
        for t in 0..300 {
            let target = &pats[t % p].1;
            let (mut s, mut free) = st.start(&m);
            for i in 0..200 {
                s[i] = if st.rng.unit() < dmg { -target[i] } else { target[i] };
            }
            let cue = s.clone();
            let best = pats.iter().enumerate().max_by(|a, b| overlap(&cue, &a.1 .1).abs().partial_cmp(&overlap(&cue, &b.1 .1).abs()).unwrap()).unwrap().0;
            if best == t % p && overlap(&cue, target) > 0.0 {
                near += 1;
            }
            for _ in 0..30 {
                st.sweep(&m, &mut s, &mut free, 1.0 / 0.05);
            }
            quench(&m, &mut st, &mut s, &mut free);
            if overlap(&s, target) >= 0.9 {
                ok += 1;
            }
        }
        (p, dmg, ok as f64 / 300.0, near as f64 / 300.0)
    });
    for (p, d, ok, near) in rows {
        println!("{:>3} {:>7.2} {:>15.1}% {:>17.1}%", p, d, 100.0 * ok, 100.0 * near);
    }
    println!("== CLUSTERING: does a random input roll to its nearest pattern? (N = 200, 1000 inputs) ==");
    println!("{:>3} {:>10} {:>16} {:>18} {:>8}", "p", "dynamics", "agree nearest %", "stored/mirror %", "fake %");
    let cases: Vec<u64> = vec![20, 21, 50, 51, 100, 101]; // p * 10 + (1 if shaken)
    let rows = par_map(&cases, threads, |cs| {
        let (p, shake) = ((cs / 10) as usize, cs % 10 == 1);
        let m = hopfield(200, p, 1);
        let pats = &stored_patterns(&m)[0].patterns;
        let mut st = State::new(cs + 77);
        let (mut agree, mut good) = (0, 0);
        for _ in 0..1000 {
            let (mut s, mut free) = st.start(&m);
            let near = pats.iter().enumerate().max_by(|a, b| overlap(&s, &a.1 .1).abs().partial_cmp(&overlap(&s, &b.1 .1).abs()).unwrap()).unwrap().0;
            if shake {
                for _ in 0..30 {
                    st.sweep(&m, &mut s, &mut free, 1.0 / 0.05);
                }
            }
            quench(&m, &mut st, &mut s, &mut free);
            if let Kind::Stored(n) | Kind::Mirror(n) = classify(&m, &s) {
                good += 1;
                if n == pats[near].0 {
                    agree += 1;
                }
            }
        }
        (p, shake, agree as f64 / 1000.0, good as f64 / 1000.0)
    });
    for (p, shake, a, g) in rows {
        println!("{:>3} {:>10} {:>16.1} {:>18.1} {:>8.1}", p, if shake { "shake+quench" } else { "quench" }, 100.0 * a, 100.0 * g, 100.0 * (1.0 - g));
    }
}

fn part_code(threads: usize) {
    println!("== CODE LANDSCAPES, exact: 12 data bits, 8 random 3-bit checks, 8 helpers (20 things) ==");
    println!("{:>4} {:>4} {:>10} {:>9} {:>11} {:>12} {:>13} {:>12} {:>10}", "seed", "V", "codewords", "cw V", "cw basin%", "cw min-max%", "fake V", "fake basin%", "saddle%");
    let seeds: Vec<u64> = (1..=10).collect();
    let rows = par_map(&seeds, threads, |seed| {
        let mut m = Model::default();
        let (_, words) = code_landscape(&mut m, 12, 8, seed, 1.0, 0).unwrap();
        let d = Dense::of(&m, &HashMap::new());
        let c = enumerate(&d);
        let t = c.total() as f64;
        let (mut cw, mut cwb, mut fk, mut fkb) = (0, 0.0, 0, 0.0);
        let mut cwb_each = Vec::new();
        for v in &c.valleys {
            if classify(&m, &bits_to_state(v.rep, d.n)) == Kind::Codeword {
                cw += 1;
                cwb += v.basin as f64 / t;
                cwb_each.push(v.basin as f64 / t);
            } else {
                fk += 1;
                fkb += v.basin as f64 / t;
            }
        }
        let mn = cwb_each.iter().cloned().fold(f64::INFINITY, f64::min);
        let mx = cwb_each.iter().cloned().fold(0.0, f64::max);
        (seed, c.valleys.len(), words, cw, cwb, mn, mx, fk, fkb, c.saddle as f64 / t)
    });
    for r in &rows {
        println!(
            "{:>4} {:>4} {:>10} {:>9} {:>11.1} {:>5.2}-{:<6.2} {:>13} {:>12.1} {:>10.1}",
            r.0, r.1, r.2, r.3, 100.0 * r.4, 100.0 * r.5, 100.0 * r.6, r.7, 100.0 * r.8, 100.0 * r.9
        );
    }
    println!("== CODE ERROR CORRECTION: flip k data bits of a codeword, quench, compare with nearest-codeword decoding ==");
    println!("{:>2} {:>18} {:>22}", "k", "landscape recovers", "nearest codeword (ML)");
    for k in 1..=3usize {
        let rows = par_map(&seeds, threads, |seed| {
            let mut m = Model::default();
            let (start, _) = code_landscape(&mut m, 12, 8, seed, 1.0, 0).unwrap();
            let d = Dense::of(&m, &HashMap::new());
            let c = enumerate(&d);
            let words: Vec<u32> = c.valleys.iter().filter(|v| classify(&m, &bits_to_state(v.rep, d.n)) == Kind::Codeword).map(|v| v.rep).collect();
            let data = |x: u32| x & 0xfff;
            let mut st = State::new(seed * 31 + k as u64);
            let (mut ok, mut ml) = (0.0, 0.0);
            let trials = 300;
            for t in 0..trials {
                let w = words[t % words.len()];
                let mut flip = 0u32;
                while flip.count_ones() < k as u32 {
                    flip |= 1 << st.rng.below(12);
                }
                let mut s = bits_to_state(w ^ flip, d.n);
                let mut free: Vec<usize> = (0..d.n).collect();
                quench(&m, &mut st, &mut s, &mut free);
                let got = s.iter().enumerate().fold(0u32, |a, (i, &v)| if v > 0.0 { a | (1 << i) } else { a });
                if data(got) == data(w) {
                    ok += 1.0;
                }
                let recv = data(w ^ flip);
                let dist: Vec<u32> = words.iter().map(|&x| (data(x) ^ recv).count_ones()).collect();
                let best = *dist.iter().min().unwrap();
                let ties = dist.iter().filter(|&&x| x == best).count() as f64;
                if (data(w) ^ recv).count_ones() == best {
                    ml += 1.0 / ties;
                }
                let _ = start;
            }
            (ok / trials as f64, ml / trials as f64)
        });
        println!("{:>2} {:>17.1}% {:>21.1}%", k, 100.0 * mean(&rows.iter().map(|r| r.0).collect::<Vec<_>>()), 100.0 * mean(&rows.iter().map(|r| r.1).collect::<Vec<_>>()));
    }
}

fn part_info(threads: usize) {
    let n = 16usize;
    let total = 1usize << n;
    let smax = 16_384u64;
    println!("== INFORMATION: can a short seed plus a valley index name any {}-bit file? ==", n);
    let seeds: Vec<u64> = (0..smax).collect();
    let valleys = par_map(&seeds, threads, |seed| {
        let (_, d) = sk(n, 1_000_000 + seed);
        enumerate(&d).valleys.iter().map(|v| v.rep).collect::<Vec<u32>>()
    });
    let mut first = vec![u64::MAX; total];
    let mut vs = Vec::new();
    for (s, list) in valleys.iter().enumerate() {
        vs.push(list.len() as f64);
        for &c in list {
            if first[c as usize] == u64::MAX {
                first[c as usize] = s as u64;
            }
        }
    }
    let vbar = mean(&vs);
    println!("mean valleys per landscape at n = {}: {:.2} (log2 {:.2} bits)", n, vbar, vbar.log2());
    println!("{:>7} {:>11} {:>15} {:>12} {:>12} {:>14}", "seeds S", "seed bits", "pointer bits", "covered %", "predicted %", "covered/2^n x S*V");
    let mut s = 1u64;
    while s <= smax {
        let cov = first.iter().filter(|&&f| f < s).count() as f64 / total as f64;
        let pred = 1.0 - (-(s as f64) * vbar / total as f64).exp();
        let used = (s as f64 * vbar) / total as f64;
        println!("{:>7} {:>11.1} {:>15.2} {:>11.2}% {:>11.2}% {:>14.3}", s, (s as f64).log2(), (s as f64).log2() + vbar.log2(), 100.0 * cov, 100.0 * pred, cov / used.min(1e9));
        s *= 4;
    }
    let covered: Vec<usize> = (0..total).filter(|&c| first[c] < smax).collect();
    let var: Vec<f64> = covered.iter().map(|&c| ((first[c] + 1) as f64).log2() + vs[first[c] as usize].log2()).collect();
    println!(
        "strings covered by the first {} landscapes: {} of {}; for each, name it by its first seed i and its valley index: log2(i+1) + log2(V_i) bits, mean {:.2}, median {:.2}, 10th percentile {:.2}",
        smax,
        covered.len(),
        total,
        mean(&var),
        median(var.clone()),
        {
            let mut v = var.clone();
            v.sort_by(|a, b| a.partial_cmp(b).unwrap());
            v[v.len() / 10]
        }
    );
    println!("(log2(i+1) is a lower bound: a real self-delimiting seed code costs about 2 log2 log2 i more)");
}

fn part_files(threads: usize) {
    println!("== WHERE DOES A RANDOM FILE LAND? quench from the file itself ==");
    let seeds: Vec<u64> = (1..=5).collect();
    let rows = par_map(&seeds, threads, |seed| {
        let (m, _) = sk(24, seed);
        let mut st = State::new(seed + 500);
        let mut dists = Vec::new();
        let mut ends = HashSet::new();
        for _ in 0..1000 {
            let (mut s, mut free) = st.start(&m);
            let s0 = s.clone();
            quench(&m, &mut st, &mut s, &mut free);
            ends.insert(key(&s));
            dists.push(s.iter().zip(&s0).filter(|(a, b)| a != b).count() as f64 / 24.0);
        }
        (mean(&dists), ends.len())
    });
    for (i, (d, e)) in rows.iter().enumerate() {
        println!("random pulls n = 24, seed {}: 1000 random files land {:.1}% of their bits away, in {} distinct valleys", i + 1, 100.0 * d, e);
    }
    let mut m = Model::default();
    grid(&mut m, 16, 16, true);
    let mut st = State::new(9);
    let mut dists = Vec::new();
    let mut kinds: HashMap<&str, usize> = HashMap::new();
    for _ in 0..200 {
        let (mut s, mut free) = st.start(&m);
        let s0 = s.clone();
        quench(&m, &mut st, &mut s, &mut free);
        *kinds.entry(grid_kind(&s, 16, 16)).or_default() += 1;
        dists.push(s.iter().zip(&s0).filter(|(a, b)| a != b).count() as f64 / 256.0);
    }
    println!("16x16 wrapped grid: 200 random 256-bit files land {:.1}% of their bits away; end states {:?}", 100.0 * mean(&dists), kinds);
    let mut it = Interp::default();
    it.exec("model :h do\n  memory :m, size: 200\n  m.save :t, \"a file stored on purpose\"\nend").unwrap();
    let m = &it.models["h"];
    let mut st = State::new(3);
    let mut dists = Vec::new();
    for _ in 0..200 {
        let (mut s, mut free) = st.start(m);
        let s0 = s.clone();
        quench(m, &mut st, &mut s, &mut free);
        dists.push(s.iter().zip(&s0).filter(|(a, b)| a != b).count() as f64 / 200.0);
    }
    println!("Hopfield N = 200 holding one saved text: 200 random files land {:.1}% of their bits away (all on the text or its mirror)", 100.0 * mean(&dists));
    let _ = code("x", 1);
}

fn part_optimise(threads: usize) {
    println!("== OPTIMISATION: how often does one survey start find the exact calmest arrangement? n = 24 ==");
    let seeds: Vec<u64> = (1..=10).collect();
    let rows = par_map(&seeds, threads.min(4), |seed| {
        let (m, d) = sk(24, seed);
        let c = enumerate(&d);
        let g = c.valleys[0].energy;
        let ground_basin = c.valleys.iter().filter(|v| (v.energy - g).abs() < 1e-9).map(|v| v.basin).sum::<u64>() as f64 / c.total() as f64;
        let mut st = State::new(seed);
        let f = survey(&m, &mut st, 1000, 50, 0.05);
        let hit = f.iter().filter(|x| (x.energy - g).abs() < 1e-9).map(|x| x.count).sum::<u64>() as f64 / 1000.0;
        let mut st = State::new(seed);
        let f0 = survey(&m, &mut st, 1000, 0, 0.05);
        let hit0 = f0.iter().filter(|x| (x.energy - g).abs() < 1e-9).map(|x| x.count).sum::<u64>() as f64 / 1000.0;
        (seed, c.valleys.len(), ground_basin, hit0, hit)
    });
    println!("{:>4} {:>4} {:>22} {:>20} {:>26}", "seed", "V", "ground steepest basin", "quench only hits", "50 sweeps T 0.05 + quench");
    for r in &rows {
        println!("{:>4} {:>4} {:>21.1}% {:>19.1}% {:>25.1}%", r.0, r.1, 100.0 * r.2, 100.0 * r.3, 100.0 * r.4);
    }
    println!("mean hit rate with shaking {:.1}%, quench only {:.1}%", 100.0 * mean(&rows.iter().map(|r| r.4).collect::<Vec<_>>()), 100.0 * mean(&rows.iter().map(|r| r.3).collect::<Vec<_>>()));
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let part = args.get(1).map(|s| s.as_str()).unwrap_or("all");
    let threads = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(6usize);
    let all = part == "all";
    if all || part == "controls" {
        part_controls();
    }
    if all || part == "random" {
        part_random(threads);
    }
    if all || part == "shuffled" {
        part_shuffled(threads);
    }
    if all || part == "grid" {
        part_grid(threads);
    }
    if all || part == "hopfield" {
        part_hopfield(threads);
    }
    if all || part == "code" {
        part_code(threads);
    }
    if all || part == "info" {
        part_info(threads);
    }
    if all || part == "files" {
        part_files(threads);
    }
    if all || part == "optimise" {
        part_optimise(threads);
    }
}
