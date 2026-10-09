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
//!
//! The statements are parsed by KANERVA (`kanerva::lang`), mounted in the registry by `crate::plug`; `exec` runs the
//! typed statement on the model, and the printed lines come from `kanerva::lang::say`.

use crate::ext::Ctx;
use crate::lex::{err, SettleError};
use crate::model::{Model, State};
use crate::rng::Rng;
use kanerva::address::{iterated_read, Addresses};
use kanerva::codes::{code, pattern};
use kanerva::keys::{keyed_capacity, keyed_pattern, keyed_read_address};
use kanerva::lang::{say, From, Stmt, Via};

pub use kanerva::store::WAKE;
pub use kanerva::theory::radius_for;

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
    // the eight values are the memory's own fields, set once; a struct for them would only rename the struct
    #[allow(clippy::too_many_arguments)]
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
        say::public(&self.stored, name, self.n)
    }
}

crate::plug::mount!(Sdm, kanerva::lang::Family::Sdm);

/// Run one parsed sdm statement on this model: declare the things and pulls, write into the pulls, or read.
/// `st` is the run's state (None inside a model block, where no read can stand).
pub fn exec(m: &mut Model, st: Option<&mut State>, s: Stmt, ln: usize, ctx: &mut Ctx) -> Result<(), SettleError> {
    match s {
        Stmt::SdmDeclare { name, word_size, hard_locations, activation_radius, seed, fade } => {
            // A clash would make `Model::add` hand back an existing thing and break the fixed layout (this used to
            // panic): refuse it, in the words softsdm uses and KANERVA's runner uses for the same program.
            let data = (0..word_size).map(|j| format!("{}_{}", name, j));
            let locs = (0..hard_locations).map(|i| format!("{}_loc_{}", name, i));
            if let Some(clash) = data.chain(locs).find(|x| m.idx.contains_key(x)) {
                return err(ln, format!("a thing :{} already exists; pick another sdm name", clash));
            }
            View::declare(m, &name, word_size, hard_locations, activation_radius, seed, fade);
            Ok(())
        }
        Stmt::SdmWrite { name, what, text, key } => write(m, &name, &what, text, key, ln, ctx),
        Stmt::SdmRead { name, seed, iterated_reads, via, from } => match st {
            Some(st) => read(m, st, &name, seed, iterated_reads, via, &from, ln, ctx),
            None => err(ln, "s.read is a run statement; put it inside `run :name do ... end`"),
        },
        _ => err(ln, "not an sdm statement"),
    }
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
        ctx.say(say::sdm_write_warning(name, v.radius, what));
    }
    v.stored.push((what.to_string(), tag));
    v.keep(m);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn read(m: &Model, st: &mut State, name: &str, seed: Option<u64>, iters: usize, via: Via, from: &From, ln: usize, ctx: &mut Ctx) -> Result<(), SettleError> {
    let v = View::load(m, name, ln)?;
    if let Some(s) = seed {
        st.rng = Rng::new(s);
    }
    let start: Vec<f64> = match from {
        From::Address { name: c, noise } => {
            let p = v.public(c).unwrap_or_else(|| code(c, v.n));
            p.iter().map(|&b| if st.rng.unit() < *noise { -b } else { b }).collect()
        }
        From::Key { key, noise } => keyed_read_address(key, v.n).iter().map(|&b| if st.rng.unit() < *noise { -b } else { b }).collect(),
        From::Noise => (0..v.n).map(|_| if st.rng.unit() < 0.5 { -1.0 } else { 1.0 }).collect(),
    };
    let (got, rounds, awake) = match via {
        Via::Pulls => v.read_pulls(m, st, &start, iters),
        Via::Addresses => v.read_addresses(m, &start, iters),
    };
    for line in say::sdm_read(name, from, via, rounds, awake, v.m_loc, &got, &v.stored, v.n) {
        ctx.say(line);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interp::Interp;
    use kanerva::codes::overlap;

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

    #[test]
    fn an_sdm_whose_things_clash_is_refused_not_a_panic() {
        let e = Interp::default().exec("model :m do\n  softsdm :s, word-size: 64, hard-locations: 50\n  sdm :s, word-size: 64, hard-locations: 50\nend").err().unwrap().0;
        assert_eq!(e, "line 3: a thing :s_loc_0 already exists; pick another sdm name");
        // the control: two names that do not clash are both declared
        assert!(Interp::default().exec("model :m do\n  softsdm :f, word-size: 64, hard-locations: 50\n  sdm :s, word-size: 64, hard-locations: 50\nend").is_ok());
    }
}
