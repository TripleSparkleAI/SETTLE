//! SDM: Kanerva's sparse distributed memory (1988), with its bit-counters living in SETTLE pulls.
//!
//! ```text
//! model :mind do
//!   sdm :s, word-size: 256, hard-locations: 2000          # 256 data things, 2000 hidden hard location things
//!   s.write :cat                                 # a random pattern named :cat
//!   s.write :note, "meet at nine"                # text, masked by its name
//!   s.write :diary, "private", key: "secret"     # text turned by a key (see memory.rs)
//! end
//! run :mind do
//!   s.read read-address: :cat, address-noise: 0.2                # read from a 20%-scrambled cat, iterate to a fixed point
//!   s.read read-address: :cat, address-noise: 0.2, via: :pulls   # the same read done by the pulls alone
//!   s.read key: "secret"
//!   s.read                                       # from pure noise
//! end
//! ```
//!
//! Things: data `s_0..s_{n-1}` and hidden hard locations `s_loc_0..s_loc_{M-1}`. Every hard location i has a fixed random
//! ADDRESS a_i (drawn from the name and seed, never stored) and a row of COUNTERS C[i][0..n]. The bit-counters are
//! not stored anywhere else: they ARE the pulls between hard location i and each data thing, pull = C * gain / 2
//! with gain 4/n. Each data thing also leans by the sum of its pulls, and each hard location leans against waking.
//!
//! Write p: every hard location whose address is within `radius` (Hamming) of p gets C[i] += p (a Hebbian nudge).
//!
//! Two HARD reads (no temperature; a soft read is the SOFTSDM lane's):
//! - `via: :addresses` (default) is Kanerva's read: awake = { i : hamming(a_i, z) <= activation radius },
//!   z[j] <- sign( sum over awake i of C[i][j] ), a zero sum keeps the old bit, repeat to a fixed point.
//!   This is NOT a settling process. It decodes with the address matrix and reads with the counter matrix,
//!   two different matrices; one symmetric set of pulls cannot hold both, so the addresses sit outside the
//!   pulls and only the bit-counters are pulls.
//! - `via: :pulls` is the zero-temperature limit of the energy the pulls define: hard location i wakes when its
//!   input (gain/2)(C[i].z - 0.4 n) is positive, then data thing j takes the sign of its input
//!   gain * sum over awake i of C[i][j]; layered (all hard locations, then all data) to a fixed point. Addresses are
//!   used only when writing. It is a settling process (each layer step lowers the energy) but a different
//!   memory: hard locations are woken by what they hold, not where they sit.
//!
//! Unlike the Hopfield memory, SDM has no mirror images: the flipped pattern wakes other hard locations.
//!
//! The algorithms are KANERVA's: the named address matrix and the Hamming-ball activation
//! (`kanerva::address::Addresses`), the iterated address read (`kanerva::address::iterated_read`), the
//! activation radius rule (`kanerva::theory::radius_for`) and the codes and keys. This file keeps what only SETTLE
//! has: the bit-counters as pulls, the leans, and the zero-temperature settle of the pulls.

use crate::ext::{Claim, Ctx, Ext};
use crate::lex::{err, kw, kwargs, num, only, text, SettleError, Tok};
use crate::model::{Model, State};
use crate::rng::Rng;
use kanerva::address::{iterated_read, Addresses};
use kanerva::codes::{bits_text, code, overlap, pattern};
use kanerva::keys::{keyed_capacity, keyed_read_address, keyed_pattern, keyed_read};

pub use kanerva::store::WAKE;
pub use kanerva::theory::radius_for;

pub struct Sdm;

/// One SDM on a model: where its things are, its addresses (regenerated from the name and seed), and its
/// settings.
pub struct View {
    pub name: String,
    pub data: usize,
    pub n: usize,
    pub loc: usize,
    pub m_loc: usize,
    pub radius: usize,
    pub seed: u64,
    pub fade: f64,
    addr: Addresses,
    /// (name, tag): tag "" for a code, "=text" for masked text, "#" for keyed text (not held).
    pub stored: Vec<(String, String)>,
}

impl View {
    fn build(name: &str, data: usize, n: usize, loc: usize, m_loc: usize, radius: usize, seed: u64, fade: f64) -> View {
        let addr = Addresses::named(name, seed, n, m_loc);
        View { name: name.to_string(), data, n, loc, m_loc, radius, seed, fade, addr, stored: Vec::new() }
    }

    pub fn load(m: &Model, name: &str, ln: usize) -> Result<View, SettleError> {
        let (nums, words) = match m.notes.get(&format!("sdm:{}", name)) {
            Some(x) => x,
            None => return err(ln, format!("no sdm :{} (declare it with: sdm :{}, word-size: 256, hard-locations: 2000)", name, name)),
        };
        let mut v = View::build(name, nums[0] as usize, nums[1] as usize, nums[2] as usize, nums[3] as usize, nums[4] as usize, nums[5] as u64, nums[6]);
        v.stored = words.chunks(2).map(|w| (w[0].clone(), w[1].clone())).collect();
        Ok(v)
    }

    fn keep(&self, m: &mut Model) {
        let nums = vec![self.data as f64, self.n as f64, self.loc as f64, self.m_loc as f64, self.radius as f64, self.seed as f64, self.fade];
        let words = self.stored.iter().flat_map(|(a, b)| [a.clone(), b.clone()]).collect();
        m.notes.insert(format!("sdm:{}", self.name), (nums, words));
    }

    /// Declare the things and lay out the pulls: data row j holds hard locations 0..M in order at positions 0..M,
    /// hard location row i holds data 0..n in order at positions 0..n. All pulls start at zero.
    pub fn declare(m: &mut Model, name: &str, n: usize, m_loc: usize, radius: usize, seed: u64, fade: f64) -> View {
        let data = m.len();
        for j in 0..n {
            m.add(&format!("{}_{}", name, j));
        }
        let loc = m.len();
        for i in 0..m_loc {
            m.add(&format!("{}_loc_{}", name, i));
        }
        for j in 0..n {
            m.adj[data + j].extend((0..m_loc).map(|i| (loc + i, 0.0)));
        }
        for i in 0..m_loc {
            m.adj[loc + i].extend((0..n).map(|j| (data + j, 0.0)));
            m.h[loc + i] = -(4.0 / n as f64) * WAKE * n as f64 / 2.0;
        }
        let v = View::build(name, data, n, loc, m_loc, radius, seed, fade);
        v.keep(m);
        v
    }

    pub fn gain(&self) -> f64 {
        4.0 / self.n as f64
    }

    /// Counter C[i][j], read back from the pull.
    pub fn counter(&self, m: &Model, i: usize, j: usize) -> f64 {
        m.adj[self.loc + i][j].1 * 2.0 / self.gain()
    }

    /// Hard locations whose address is within the activation radius of z.
    pub fn awake(&self, z: &[f64]) -> Vec<usize> {
        self.addr.awake(z, self.radius)
    }

    /// Write p: fade every counter, then C[i] += p for each hard location within the activation radius of p. Keeps each data
    /// thing's lean equal to the sum of its pulls. Returns how many hard locations took the write.
    pub fn write(&self, m: &mut Model, p: &[f64]) -> usize {
        let half = self.gain() / 2.0;
        if self.fade < 1.0 {
            for j in 0..self.n {
                let row = &mut m.adj[self.data + j];
                let mut sum = 0.0;
                for e in row[..self.m_loc].iter_mut() {
                    sum += e.1;
                    e.1 *= self.fade;
                }
                m.h[self.data + j] += (self.fade - 1.0) * sum;
            }
            for i in 0..self.m_loc {
                for e in m.adj[self.loc + i][..self.n].iter_mut() {
                    e.1 *= self.fade;
                }
            }
        }
        let act = self.awake(p);
        for &i in &act {
            for j in 0..self.n {
                let d = half * p[j];
                m.adj[self.loc + i][j].1 += d;
                m.adj[self.data + j][i].1 += d;
                m.h[self.data + j] += d;
            }
        }
        act.len()
    }

    /// Kanerva's read, iterated to a fixed point (at most `iters` reads). Returns (z, reads done, awake at the end).
    /// The bit-counters are read straight from the pulls of each awake hard location.
    pub fn read_addresses(&self, m: &Model, cue: &[f64], iters: usize) -> (Vec<f64>, usize, usize) {
        iterated_read(&self.addr, self.radius, cue, iters, |i, sum| {
            for (j, e) in m.adj[self.loc + i][..self.n].iter().enumerate() {
                sum[j] += e.1;
            }
        })
    }

    /// The zero-temperature settle of the pulls: hard locations then data, by the sign of each thing's input from
    /// the model (so any other pulls on these things count too). Returns (z, layer rounds, awake at the end).
    pub fn read_pulls(&self, m: &Model, st: &mut State, cue: &[f64], iters: usize) -> (Vec<f64>, usize, usize) {
        let (mut s, _) = st.start(m);
        s[self.data..self.data + self.n].copy_from_slice(cue);
        let hard = |x: f64, old: f64| if x > 0.0 { 1.0 } else if x < 0.0 { -1.0 } else { old };
        let mut rounds = iters.max(1);
        for t in 0..iters.max(1) {
            for i in self.loc..self.loc + self.m_loc {
                if !st.held.contains_key(&i) {
                    s[i] = hard(m.input(i, &s), -1.0);
                }
            }
            let before = s[self.data..self.data + self.n].to_vec();
            for j in self.data..self.data + self.n {
                if !st.held.contains_key(&j) {
                    s[j] = hard(m.input(j, &s), s[j]);
                }
            }
            if s[self.data..self.data + self.n] == before[..] {
                rounds = t + 1;
                break;
            }
        }
        let awake = s[self.loc..self.loc + self.m_loc].iter().filter(|&&v| v > 0.0).count();
        let z = s[self.data..self.data + self.n].to_vec();
        st.last = s;
        (z, rounds, awake)
    }

    /// The public pattern of a stored name (None for keyed text or an unknown name).
    pub fn public(&self, name: &str) -> Option<Vec<f64>> {
        let tag = &self.stored.iter().find(|(n, _)| n == name)?.1;
        match tag.chars().next() {
            Some('#') => None,
            Some('=') => Some(pattern(name, Some(&tag[1..]), self.n)),
            _ => Some(code(name, self.n)),
        }
    }
}

fn declare(m: &mut Model, rest: &[Tok], name: &str, ln: usize) -> Result<(), SettleError> {
    if m.notes.contains_key(&format!("sdm:{}", name)) {
        return err(ln, format!("sdm :{} is already declared", name));
    }
    let kv = kwargs(rest, ln)?;
    only(&kv, &["word-size", "hard-locations", "activation-radius", "seed", "fade"], "sdm", ln)?;
    let get = |k: &str, d: f64| kw(&kv, k).map(|v| num(v, ln)).transpose().map(|x| x.unwrap_or(d));
    let n = get("word-size", 256.0)? as usize;
    let m_loc = get("hard-locations", 2000.0)? as usize;
    let seed = get("seed", 1.0)? as u64;
    let fade = get("fade", 1.0)?;
    if !(16..=4096).contains(&n) {
        return err(ln, "sdm word-size must be between 16 and 4096");
    }
    if !(1..=100_000).contains(&m_loc) || n * m_loc > 20_000_000 {
        return err(ln, "sdm hard-locations must be at least 1, and word-size x hard-locations at most 20 million");
    }
    if !(0.0..=1.0).contains(&fade) || fade == 0.0 {
        return err(ln, "fade must be above 0 and at most 1");
    }
    let radius = get("activation-radius", radius_for(n, 0.02) as f64)? as usize;
    if radius > n {
        return err(ln, "activation-radius cannot be larger than word-size");
    }
    View::declare(m, name, n, m_loc, radius, seed, fade);
    Ok(())
}

fn write(m: &mut Model, name: &str, what: &str, txt: Option<String>, key: Option<String>, ln: usize, ctx: &mut Ctx) -> Result<(), SettleError> {
    let mut v = View::load(m, name, ln)?;
    if v.stored.iter().any(|(n, _)| n == what) {
        return err(ln, format!(":{} is already written in :{}", what, name));
    }
    let (p, tag) = match (&txt, &key) {
        (Some(t), Some(k)) => {
            if t.len() > keyed_capacity(v.n) {
                return err(ln, format!("keyed text of {} bytes is too long; sdm :{} holds at most {}", t.len(), name, keyed_capacity(v.n)));
            }
            (keyed_pattern(k, t, v.n), "#".to_string())
        }
        (Some(t), None) => {
            if t.len() * 8 > v.n {
                return err(ln, format!("{} bytes of text need {} things; sdm :{} has {}", t.len(), t.len() * 8, name, v.n));
            }
            (pattern(what, Some(t), v.n), format!("={}", t))
        }
        (None, _) => (code(what, v.n), String::new()),
    };
    let took = v.write(m, &p);
    if took == 0 {
        ctx.say(format!("warning: no hard location of :{} is within activation-radius {} of :{}, so nothing was written", name, v.radius, what));
    }
    v.stored.push((what.to_string(), tag));
    v.keep(m);
    Ok(())
}


fn read(m: &Model, st: &mut State, name: &str, rest: &[Tok], ln: usize, ctx: &mut Ctx) -> Result<(), SettleError> {
    let v = View::load(m, name, ln)?;
    let kv = kwargs(rest, ln)?;
    only(&kv, &["read-address", "key", "address-noise", "iterated-reads", "via", "seed"], "read", ln)?;
    if let Some(x) = kw(&kv, "seed") {
        st.rng = Rng::new(num(x, ln)? as u64);
    }
    let iters = kw(&kv, "iterated-reads").map(|x| num(x, ln)).transpose()?.unwrap_or(10.0) as usize;
    let via = match kw(&kv, "via") {
        None => "addresses".to_string(),
        Some(Tok::Sym(s)) if s == "addresses" || s == "pulls" => s.clone(),
        Some(_) => return err(ln, "via: takes :addresses or :pulls"),
    };
    let key = kw(&kv, "key").map(|t| text(t, ln)).transpose()?;
    let (start, from) = match (kw(&kv, "read-address"), &key) {
        (Some(_), Some(_)) => return err(ln, "read takes read-address: or key:, not both"),
        (Some(Tok::Sym(c)), None) => {
            let damage = kw(&kv, "address-noise").map(|x| num(x, ln)).transpose()?.unwrap_or(0.3);
            let p = v.public(c).unwrap_or_else(|| code(c, v.n));
            let z: Vec<f64> = p.iter().map(|&b| if st.rng.unit() < damage { -b } else { b }).collect();
            (z, format!("read-address :{} with {:.0}% address-noise", c, 100.0 * damage))
        }
        (Some(_), None) => return err(ln, "read-address: takes a symbol, like read-address: :cat"),
        (None, Some(k)) => {
            let damage = kw(&kv, "address-noise").map(|x| num(x, ln)).transpose()?.unwrap_or(0.0);
            let p = keyed_read_address(k, v.n);
            (p.iter().map(|&b| if st.rng.unit() < damage { -b } else { b }).collect(), "a key".to_string())
        }
        (None, None) => ((0..v.n).map(|_| if st.rng.unit() < 0.5 { -1.0 } else { 1.0 }).collect(), "pure noise".to_string()),
    };
    let (got, rounds, awake) = if via == "pulls" { v.read_pulls(m, st, &start, iters) } else { v.read_addresses(m, &start, iters) };
    let head = format!("read :{} from {} via {} ({} iterated reads, {} of {} hard locations activated)", name, from, via, rounds, awake, v.m_loc);
    if let Some(k) = &key {
        match keyed_read(k, &got) {
            Some(t) => ctx.say(format!("{}: text \"{}\"", head, t)),
            None => ctx.say(format!("{}: nothing readable", head)),
        }
        return Ok(());
    }
    let mut scores: Vec<(String, f64)> = v.stored.iter().filter_map(|(n, _)| v.public(n).map(|p| (n.clone(), overlap(&got, &p)))).collect();
    scores.sort_by(|x, y| y.1.abs().partial_cmp(&x.1.abs()).unwrap());
    let top: Vec<String> = scores.iter().take(3).map(|(n, o)| format!(":{} {:+.2}", n, o)).collect();
    let verdict = match scores.first() {
        Some((n, o)) if *o >= 0.9 => format!("-> :{}", n),
        Some((n, o)) => format!("-> nothing clear (closest :{} at {:+.2})", n, o),
        None => "-> nothing is written".to_string(),
    };
    ctx.say(format!("{}: {}  {}", head, top.join("  "), verdict));
    if let Some((n, o)) = scores.first() {
        if *o >= 0.9 {
            if let Some((_, tag)) = v.stored.iter().find(|(x, _)| x == n) {
                if let Some(t) = tag.strip_prefix('=') {
                    let mask = code(n, v.n);
                    let bits: Vec<f64> = got.iter().zip(&mask).map(|(a, b)| a * b).collect();
                    ctx.say(format!("  text: \"{}\"", bits_text(&bits, t.len())));
                }
            }
        }
    }
    Ok(())
}

impl Ext for Sdm {
    fn name(&self) -> &'static str {
        "sdm"
    }

    fn statements(&self) -> &'static [&'static str] {
        &[
            "model: sdm :s, word-size: 256, hard-locations: 2000, activation-radius: 112, seed: 1, fade: 1",
            "model: s.write :cat   /   s.write :note, \"text\"   /   s.write :diary, \"text\", key: \"secret\"",
            "run: s.write ... (as in a model)",
            "run: s.read read-address: :cat, address-noise: 0.3, iterated-reads: 10, via: :addresses, seed: 1   (via: :pulls too)",
            "run: s.read key: \"secret\"   /   s.read   (from noise)",
        ]
    }

    fn model_stmt(&self, m: &mut Model, t: &[Tok], ln: usize, ctx: &mut Ctx) -> Claim {
        match t {
            [Tok::Ident(k), Tok::Sym(name), rest @ ..] if k == "sdm" => {
                let rest = if rest.first() == Some(&Tok::Comma) { &rest[1..] } else { rest };
                Some(declare(m, rest, name, ln))
            }
            _ => write_stmt(m, t, ln, ctx),
        }
    }

    fn run_stmt(&self, m: &mut Model, st: &mut State, t: &[Tok], ln: usize, ctx: &mut Ctx) -> Claim {
        match t {
            [Tok::Ident(name), Tok::Dot, Tok::Ident(v), rest @ ..] if v == "read" && m.notes.contains_key(&format!("sdm:{}", name)) => {
                Some(read(m, st, name, rest, ln, ctx))
            }
            _ => write_stmt(m, t, ln, ctx),
        }
    }
}

fn write_stmt(m: &mut Model, t: &[Tok], ln: usize, ctx: &mut Ctx) -> Claim {
    // `write` is shared with the softsdm family: leave a line alone when its name is a declared softsdm.
    if let [Tok::Ident(name), ..] = t {
        if m.notes.contains_key(&format!("softsdm:{}", name)) {
            return None;
        }
    }
    match t {
        [Tok::Ident(name), Tok::Dot, Tok::Ident(v), Tok::Sym(what)] if v == "write" => Some(write(m, name, what, None, None, ln, ctx)),
        [Tok::Ident(name), Tok::Dot, Tok::Ident(v), Tok::Sym(what), Tok::Comma, s, rest @ ..] if v == "write" => Some((|| {
            let txt = text(s, ln)?;
            let kv = kwargs(rest, ln)?;
            only(&kv, &["key"], "write", ln)?;
            let key = kw(&kv, "key").map(|k| text(k, ln)).transpose()?;
            write(m, name, what, Some(txt), key, ln, ctx)
        })()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interp::Interp;

    fn run(src: &str) -> Vec<String> {
        Interp::default().exec(src).unwrap_or_else(|e| panic!("{}", e))
    }

    const SMALL: &str = "model :mind do
  sdm :s, word-size: 256, hard-locations: 2000
  s.write :cat
  s.write :dog
  s.write :owl
  s.write :note, \"meet at nine\"
  s.write :diary, \"under the mat\", key: \"blue heron\"
end
";

    #[test]
    fn default_radius_wakes_about_two_percent() {
        assert_eq!(radius_for(256, 0.02), 112); // P[Bin(256,1/2) <= 111] is just under 0.02
        assert_eq!(radius_for(1000, 0.001), 451); // Kanerva's own example (wiki 01, section 3)
    }

    #[test]
    fn counters_live_in_the_pulls_and_leans_match_them() {
        let mut m = Model::default();
        let v = View::declare(&mut m, "s", 64, 300, radius_for(64, 0.05), 1, 1.0);
        let p = code("x", 64);
        let act = v.write(&mut m, &p);
        assert!(act > 0);
        for &i in &act_of(&v, &p) {
            for j in 0..64 {
                assert_eq!(v.counter(&m, i, j), p[j]);
                assert_eq!(m.coupling(v.loc + i, v.data + j), m.coupling(v.data + j, v.loc + i));
            }
        }
        for j in 0..64 {
            let sum: f64 = (0..300).map(|i| m.adj[v.data + j][i].1).sum();
            assert!((m.h[v.data + j] - sum).abs() < 1e-12);
        }
    }

    fn act_of(v: &View, p: &[f64]) -> Vec<usize> {
        v.awake(p)
    }

    #[test]
    fn damaged_cues_come_back_by_both_reads() {
        let out = run(&format!(
            "{}run :mind do
  s.read read-address: :cat, address-noise: 0.15, seed: 1
  s.read read-address: :dog, address-noise: 0.15, seed: 2
  s.read read-address: :owl, address-noise: 0.15, via: :pulls, seed: 3
  s.read read-address: :note, address-noise: 0.15, via: :pulls, seed: 4
end",
            SMALL
        ));
        assert!(out[0].ends_with("-> :cat"), "{}", out[0]);
        assert!(out[1].ends_with("-> :dog"), "{}", out[1]);
        assert!(out[2].ends_with("-> :owl"), "{}", out[2]);
        assert!(out[3].ends_with("-> :note"), "{}", out[3]);
        assert_eq!(out[4], "  text: \"meet at nine\"");
    }

    #[test]
    fn a_never_written_cue_is_not_recalled() {
        let out = run(&format!("{}run :mind do\n  s.read read-address: :zebra, address-noise: 0.0, seed: 5\nend", SMALL));
        assert!(!out[0].contains("-> :zebra") && out[0].contains("nothing clear"), "{}", out[0]);
    }

    #[test]
    fn the_key_reads_the_diary_and_a_wrong_key_does_not() {
        let out = run(&format!("{}run :mind do\n  s.read key: \"blue heron\"\n  s.read key: \"red heron\"\nend", SMALL));
        assert!(out[0].ends_with("text \"under the mat\""), "{}", out[0]);
        assert!(!out[1].contains("under the mat"), "{}", out[1]);
    }

    #[test]
    fn a_mirror_cue_is_not_a_valley_in_sdm() {
        // unlike Hopfield, the flipped pattern wakes other hard locations and does not come back
        let mut it = Interp::default();
        it.exec(SMALL).unwrap();
        let m = &it.models["mind"];
        let v = View::load(m, "s", 0).unwrap();
        let p = code("cat", 256);
        let flipped: Vec<f64> = p.iter().map(|x| -x).collect();
        let (z, _, _) = v.read_addresses(m, &flipped, 10);
        assert!(overlap(&z, &p).abs() < 0.9, "{}", overlap(&z, &p));
    }

    #[test]
    fn shuffled_counters_recall_nothing() {
        // negative control: the same bit-counters dealt to the wrong hard locations
        let mut it = Interp::default();
        it.exec(SMALL).unwrap();
        let mut m = it.models["mind"].clone();
        let v = View::load(&m, "s", 0).unwrap();
        let rows: Vec<Vec<f64>> = (0..v.m_loc).map(|i| m.adj[v.loc + i][..v.n].iter().map(|e| e.1).collect()).collect();
        let mut r = Rng::new(9);
        for i in 0..v.m_loc {
            let src = &rows[r.below(v.m_loc)];
            for j in 0..v.n {
                m.adj[v.loc + i][j].1 = src[j];
            }
        }
        let p = code("cat", 256);
        let mut rr = Rng::new(3);
        let cue: Vec<f64> = p.iter().map(|&b| if rr.unit() < 0.15 { -b } else { b }).collect();
        let (z, _, _) = v.read_addresses(&m, &cue, 10);
        assert!(overlap(&z, &p) < 0.9, "{}", overlap(&z, &p));
    }

    #[test]
    fn errors_name_their_line() {
        let e = Interp::default().exec("model :m do\n  sdm :s, word-size: 8\nend").err().unwrap().0;
        assert!(e.starts_with("line 2: sdm word-size"), "{}", e);
        // a program written before Kanerva's terms stops with the new word, never silently
        let old = concat!("model :m do\n  sdm :s, word-size: 64, hard-locations: 10\nend\nrun :m do\n  s.read ", "cu", "e: :cat\nend");
        let e = Interp::default().exec(old).err().unwrap().0;
        assert_eq!(e, "line 5: `cue:` is now `read-address:` (Kanerva's retrieval address)");
        let old = concat!("model :m do\n  sdm :s, si", "ze: 64\nend");
        assert!(Interp::default().exec(old).err().unwrap().0.contains("is now `word-size:`"));
        let e = Interp::default().exec("model :m do\n  sdm :s, word-size: 64, hard-locations: 10\nend\nrun :m do\n  s.read via: :soft\nend").err().unwrap().0;
        assert!(e.starts_with("line 5: via:"), "{}", e);
    }
}
