//! MEMORY: store patterns in the pulls between free things, and get them back by shaking.
//!
//! ```text
//! model :mind do
//!   memory :m, size: 512, fade: 0.9     # 512 free things; each new memory weakens the older ones by 10%
//!   m.remember :cat                     # a random pattern named :cat, stored in the pulls
//!   m.save :note, "meet at the harbour at nine"
//! end
//! run :mind do
//!   m.recall read-address: :cat, address-noise: 0.3     # start from :cat with 30% of it scrambled, then shake
//!   m.recall read-address: :note, address-noise: 0.25   # the text comes back letter for letter
//!   m.recall                            # start from pure noise and see which memory the shaking finds
//! end
//! ```
//!
//! Nothing is held. A memory lives only in how strongly each pair of things pulls or pushes (Hebbian storage:
//! pairs that agree in the pattern are pulled together, pairs that disagree pushed apart, by 1/size each).
//! Each stored pattern becomes a calm point of the springs, so shaking at a low temperature rolls a noisy
//! read-address down into it. A pattern and its mirror image (every bit flipped) are equally calm; recall reports which.
//!
//! Saved text is XORed with a random mask made from its name before storing. That makes it look like random
//! bits, which is what this memory stores best (patterns that resemble each other interfere), and the mask is
//! removed again on recall. Compression would do the same job and also save space.
//!
//! KEYED text (`m.save :note, "text", key: "secret"`, then `m.recall key: "secret"`): the key names a turn of
//! the cube of arrangements, a sign flip on every thing (a mask) plus a shuffle of which thing holds which bit
//! (a permutation). Sign flips and shuffles are exactly the rotations of the cube, so the key is "directions
//! to move in that space". The stored pattern is the turned payload `[length byte][text][padding of +1s]`.
//! With the key you know the padding, so the key both FINDS the memory (the turned all-+1 payload is a read-address
//! that agrees with the pattern on every padding bit) and READS it (turn back, unmask). The text is never
//! written into the model's notes. This is NOT cryptography: the key is hashed to 64 bits by FNV-1a, the
//! mask comes from a xorshift generator, the landscape is public, and a guessed key can be checked offline
//! against the padding. The codes, the masks and the keyed notes are KANERVA's (`kanerva::codes`,
//! `kanerva::keys`), re-exported here so `memory::code` and friends keep their paths; `sdm.rs` uses them too.
//! The Hebbian store and the shake that recalls are this file's own: they live in the model's pulls.

use crate::ext::{Claim, Ctx, Ext};
use crate::lex::{err, kw, kwargs, num, only, text, SettleError, Tok};
use crate::model::{Model, State};
use crate::rng::Rng;
use kanerva::codes::overlap;

pub use kanerva::codes::{bits_text, code, pattern, seed_of};
pub use kanerva::keys::{key_turn, keyed_capacity, keyed_read_address, keyed_pattern, keyed_payload, keyed_read, KeyTurn};

pub struct Memory;

#[derive(Clone)]
enum Kind {
    Code,
    Text(String),
    /// Keyed text: the text is not written down anywhere, only its turned pattern lives in the pulls.
    Keyed,
}

struct Mem {
    start: usize,
    size: usize,
    fade: f64,
    stored: Vec<(String, Kind)>,
}

impl Mem {
    /// The public pattern of a stored name; keyed patterns are not public, so None.
    fn public(&self, name: &str) -> Option<Vec<f64>> {
        match self.stored.iter().find(|(n, _)| n == name).map(|(_, k)| k) {
            Some(Kind::Keyed) => None,
            Some(Kind::Text(t)) => Some(pattern(name, Some(t), self.size)),
            _ => Some(pattern(name, None, self.size)),
        }
    }
}

fn load(m: &Model, name: &str, ln: usize) -> Result<Mem, SettleError> {
    let (nums, words) = match m.notes.get(&format!("memory:{}", name)) {
        Some(x) => x,
        None => return err(ln, format!("no memory :{} (declare it with: memory :{}, size: 256)", name, name)),
    };
    let stored = words
        .chunks(2)
        .map(|w| {
            let kind = match w[1].chars().next() {
                Some('=') => Kind::Text(w[1][1..].to_string()),
                Some('#') => Kind::Keyed,
                _ => Kind::Code,
            };
            (w[0].clone(), kind)
        })
        .collect();
    Ok(Mem { start: nums[0] as usize, size: nums[1] as usize, fade: nums[2], stored })
}

fn keep(m: &mut Model, name: &str, mem: &Mem) {
    let words = mem
        .stored
        .iter()
        .flat_map(|(n, k)| {
            let tag = match k {
                Kind::Code => String::new(),
                Kind::Text(t) => format!("={}", t),
                Kind::Keyed => "#".to_string(),
            };
            [n.clone(), tag]
        })
        .collect();
    m.notes.insert(format!("memory:{}", name), (vec![mem.start as f64, mem.size as f64, mem.fade], words));
}

fn declare(m: &mut Model, rest: &[Tok], name: &str, ln: usize) -> Result<(), SettleError> {
    if m.notes.contains_key(&format!("memory:{}", name)) {
        return err(ln, format!("memory :{} is already declared", name));
    }
    let kv = kwargs(rest, ln)?;
    only(&kv, &["size", "fade"], "memory", ln)?;
    let size = kw(&kv, "size").map(|v| num(v, ln)).transpose()?.unwrap_or(256.0) as usize;
    let fade = kw(&kv, "fade").map(|v| num(v, ln)).transpose()?.unwrap_or(1.0);
    if !(8..=4096).contains(&size) {
        return err(ln, "memory size must be between 8 and 4096");
    }
    if !(0.0..=1.0).contains(&fade) || fade == 0.0 {
        return err(ln, "fade must be above 0 and at most 1 (1 means memories never fade)");
    }
    let start = m.len();
    for i in 0..size {
        m.add(&format!("{}_{}", name, i));
    }
    keep(m, name, &Mem { start, size, fade, stored: Vec::new() });
    Ok(())
}

/// Hebbian storage of one pattern into the block of `size` things starting at `a`: every existing pull inside
/// the block is first weakened by `fade`, then pair (i, k) gains p[i] p[k] / size. Cost O(size^2): each row's
/// in-block entries are located once per store, and missing pairs are added in place (same arithmetic as
/// calling `Model::couple` for each pair, which was O(size^3)).
pub fn store_pattern(m: &mut Model, a: usize, size: usize, fade: f64, p: &[f64]) {
    let w = 1.0 / size as f64;
    let mut pos = vec![usize::MAX; size];
    for i in 0..size {
        pos.iter_mut().for_each(|q| *q = usize::MAX);
        for (q, e) in m.adj[a + i].iter().enumerate() {
            if e.0 >= a && e.0 < a + size {
                pos[e.0 - a] = q;
            }
        }
        for k in 0..size {
            if k == i {
                continue;
            }
            if pos[k] == usize::MAX {
                m.adj[a + i].push((a + k, 0.0));
                pos[k] = m.adj[a + i].len() - 1;
            }
            let e = &mut m.adj[a + i][pos[k]];
            if fade < 1.0 {
                e.1 *= fade;
            }
            e.1 += w * p[i] * p[k];
        }
    }
}

/// Weaken every existing pull inside the memory by `fade`, then add the new pattern's pulls.
fn store(m: &mut Model, name: &str, what: &str, saved: Option<String>, key: Option<String>, ln: usize) -> Result<(), SettleError> {
    let mut mem = load(m, name, ln)?;
    if let Some(t) = &saved {
        if key.is_some() {
            if t.len() > keyed_capacity(mem.size) {
                return err(ln, format!("keyed text of {} bytes is too long; memory :{} holds at most {}", t.len(), name, keyed_capacity(mem.size)));
            }
        } else if t.len() * 8 > mem.size {
            return err(ln, format!("{} bytes of text need {} things; memory :{} has {}", t.len(), t.len() * 8, name, mem.size));
        }
    }
    if mem.stored.iter().any(|(n, _)| n == what) {
        return err(ln, format!(":{} is already stored in :{}", what, name));
    }
    let (p, kind) = match (saved, key) {
        (Some(t), Some(k)) => (keyed_pattern(&k, &t, mem.size), Kind::Keyed),
        (Some(t), None) => (pattern(what, Some(&t), mem.size), Kind::Text(t)),
        (None, _) => (pattern(what, None, mem.size), Kind::Code),
    };
    store_pattern(m, mem.start, mem.size, mem.fade, &p);
    mem.stored.push((what.to_string(), kind));
    keep(m, name, &mem);
    Ok(())
}

/// Shake the block of `size` things starting at `a` from `init` for `sweeps` sweeps at `temp`; returns the block.
/// Every free thing of the model is updated (exactly what `recall` does). Used by `recall` and by measurements.
pub fn shake(m: &Model, st: &mut State, a: usize, init: &[f64], sweeps: usize, temp: f64) -> Vec<f64> {
    let (mut s, mut free) = st.start(m);
    s[a..a + init.len()].copy_from_slice(init);
    for _ in 0..sweeps {
        st.sweep(m, &mut s, &mut free, 1.0 / temp);
    }
    let got = s[a..a + init.len()].to_vec();
    st.last = s;
    got
}


fn recall(m: &Model, st: &mut State, name: &str, rest: &[Tok], ln: usize, ctx: &mut Ctx) -> Result<(), SettleError> {
    let mem = load(m, name, ln)?;
    let kv = kwargs(rest, ln)?;
    only(&kv, &["read-address", "key", "address-noise", "sweeps", "temperature", "seed"], "recall", ln)?;
    if let Some(v) = kw(&kv, "seed") {
        st.rng = Rng::new(num(v, ln)? as u64);
    }
    let damage = kw(&kv, "address-noise").map(|v| num(v, ln)).transpose()?.unwrap_or(0.3);
    let sweeps = kw(&kv, "sweeps").map(|v| num(v, ln)).transpose()?.unwrap_or(30.0) as usize;
    let temp = kw(&kv, "temperature").map(|v| num(v, ln)).transpose()?.unwrap_or(0.1);
    if temp <= 0.0 {
        return err(ln, "temperature must be above zero");
    }
    let key = match kw(&kv, "key") {
        Some(t) => Some(text(t, ln)?),
        None => None,
    };
    if key.is_some() && kw(&kv, "read-address").is_some() {
        return err(ln, "recall takes read-address: or key:, not both");
    }
    // Keyed patterns are not public, so the public scoreboard lists only public memories.
    let patterns: Vec<(String, Vec<f64>)> =
        mem.stored.iter().filter_map(|(n, _)| mem.public(n).map(|p| (n.clone(), p))).collect();
    let (mut s, mut free) = st.start(m);
    let a = mem.start;
    let from = match (kw(&kv, "read-address"), &key) {
        (Some(Tok::Sym(c)), _) => {
            let p = mem.public(c).unwrap_or_else(|| pattern(c, None, mem.size));
            for i in 0..mem.size {
                s[a + i] = if st.rng.unit() < damage { -p[i] } else { p[i] };
            }
            format!("read-address :{} with {:.0}% address-noise", c, 100.0 * damage)
        }
        (Some(_), _) => return err(ln, "read-address: takes a symbol, like read-address: :cat"),
        (None, Some(k)) => {
            let damage = kw(&kv, "address-noise").map(|v| num(v, ln)).transpose()?.unwrap_or(0.0);
            let p = keyed_read_address(k, mem.size);
            for i in 0..mem.size {
                s[a + i] = if st.rng.unit() < damage { -p[i] } else { p[i] };
            }
            "a key".to_string()
        }
        (None, None) => "pure noise".to_string(),
    };
    for _ in 0..sweeps {
        st.sweep(m, &mut s, &mut free, 1.0 / temp);
    }
    let got = &s[a..a + mem.size];
    if let Some(k) = &key {
        let agree = overlap(got, &keyed_read_address(k, mem.size));
        match keyed_read(k, got) {
            Some(t) => ctx.say(format!(
                "recall :{} with a key after {} sweeps (agreement with the key's read-address {:+.2}): text \"{}\"",
                name, sweeps, agree, t
            )),
            None => ctx.say(format!(
                "recall :{} with a key after {} sweeps (agreement with the key's read-address {:+.2}): nothing readable",
                name, sweeps, agree
            )),
        }
        st.last = s;
        return Ok(());
    }
    let mut scores: Vec<(String, f64)> = patterns.iter().map(|(n, p)| (n.clone(), overlap(got, p))).collect();
    scores.sort_by(|x, y| y.1.abs().partial_cmp(&x.1.abs()).unwrap());
    let top: Vec<String> = scores.iter().take(3).map(|(n, o)| format!(":{} {:+.2}", n, o)).collect();
    let verdict = match scores.first() {
        Some((n, o)) if o.abs() >= 0.9 => {
            let mirror = if *o < 0.0 { " (its mirror image)" } else { "" };
            format!("-> :{}{}", n, mirror)
        }
        Some((n, o)) => format!("-> nothing clear (closest :{} at {:+.2})", n, o),
        None => "-> nothing is stored".to_string(),
    };
    ctx.say(format!("recall :{} from {} after {} sweeps: {}  {}", name, from, sweeps, top.join("  "), verdict));
    if let Some((n, o)) = scores.first() {
        if let Some((_, Kind::Text(t))) = mem.stored.iter().find(|(x, _)| x == n) {
            if o.abs() >= 0.9 {
                let mask = code(n, mem.size);
                let sign = if *o < 0.0 { -1.0 } else { 1.0 };
                let bits: Vec<f64> = got.iter().zip(&mask).map(|(v, k)| sign * v * k).collect();
                ctx.say(format!("  text: \"{}\"", bits_text(&bits, t.len())));
            }
        }
    }
    st.last = s;
    Ok(())
}

impl Ext for Memory {
    fn name(&self) -> &'static str {
        "memory"
    }

    fn statements(&self) -> &'static [&'static str] {
        &[
            "model: memory :m, size: 256, fade: 0.9",
            "model: m.remember :cat   /   m.save :note, \"some text\"",
            "run: m.remember :cat   /   m.save :note, \"some text\"",
            "model: m.save :note, \"some text\", key: \"secret\"   (keyed: the text is not held anywhere)",
            "run: m.recall read-address: :cat, address-noise: 0.3, sweeps: 30, temperature: 0.1, seed: 1",
            "run: m.recall key: \"secret\"   (the key finds the memory and reads it)",
        ]
    }

    fn model_stmt(&self, m: &mut Model, t: &[Tok], ln: usize, _ctx: &mut Ctx) -> Claim {
        match t {
            [Tok::Ident(k), Tok::Sym(name), rest @ ..] if k == "memory" => {
                let rest = if rest.first() == Some(&Tok::Comma) { &rest[1..] } else { rest };
                Some(declare(m, rest, name, ln))
            }
            _ => self.store_stmt(m, t, ln),
        }
    }

    fn run_stmt(&self, m: &mut Model, st: &mut State, t: &[Tok], ln: usize, ctx: &mut Ctx) -> Claim {
        match t {
            [Tok::Ident(name), Tok::Dot, Tok::Ident(v), rest @ ..] if v == "recall" => Some(recall(m, st, name, rest, ln, ctx)),
            _ => self.store_stmt(m, t, ln),
        }
    }
}

impl Memory {
    fn store_stmt(&self, m: &mut Model, t: &[Tok], ln: usize) -> Claim {
        match t {
            [Tok::Ident(name), Tok::Dot, Tok::Ident(v), Tok::Sym(what)] if v == "remember" => Some(store(m, name, what, None, None, ln)),
            [Tok::Ident(name), Tok::Dot, Tok::Ident(v), Tok::Sym(what), Tok::Comma, s] if v == "save" => {
                Some(text(s, ln).and_then(|txt| store(m, name, what, Some(txt), None, ln)))
            }
            [Tok::Ident(name), Tok::Dot, Tok::Ident(v), Tok::Sym(what), Tok::Comma, s, rest @ ..] if v == "save" => Some((|| {
                let txt = text(s, ln)?;
                let kv = kwargs(rest, ln)?;
                only(&kv, &["key"], "save", ln)?;
                let key = match kw(&kv, "key") {
                    Some(k) => text(k, ln)?,
                    None => return err(ln, "save with a trailing option needs `key: \"...\"`"),
                };
                store(m, name, what, Some(txt), Some(key), ln)
            })()),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::interp::Interp;

    fn run(src: &str) -> Vec<String> {
        Interp::default().exec(src).unwrap_or_else(|e| panic!("{}", e))
    }

    #[test]
    fn damaged_cues_come_back_whole() {
        let out = run("model :mind do
  memory :m, size: 256
  m.remember :cat
  m.remember :dog
  m.remember :owl
end
run :mind do
  m.recall read-address: :cat, address-noise: 0.3, seed: 1
  m.recall read-address: :dog, address-noise: 0.3, seed: 2
  m.recall read-address: :owl, address-noise: 0.3, seed: 3
end");
        for (line, want) in out.iter().zip(["-> :cat", "-> :dog", "-> :owl"]) {
            assert!(line.ends_with(want), "{}", line);
        }
    }

    #[test]
    fn a_never_stored_cue_is_not_recalled() {
        // negative control: :zebra was never stored, so shaking must not settle on :zebra's pattern
        let out = run("model :mind do
  memory :m, size: 256
  m.remember :cat
  m.remember :dog
end
run :mind do
  m.recall read-address: :zebra, address-noise: 0.0, seed: 4
end");
        assert!(!out[0].contains("-> :zebra"), "{}", out[0]);
    }

    #[test]
    fn saved_text_comes_back_letter_for_letter() {
        let out = run("model :mind do
  memory :m, size: 512
  m.remember :cat
  m.save :note, \"meet at the harbour at nine\"
  m.remember :dog
end
run :mind do
  m.recall read-address: :note, address-noise: 0.25, seed: 7
end");
        assert!(out[1] == "  text: \"meet at the harbour at nine\"", "{:?}", out);
    }

    #[test]
    fn with_fade_the_newest_memory_is_found_and_the_oldest_is_lost() {
        let mut src = String::from("model :mind do\n  memory :m, size: 256, fade: 0.6\n");
        for i in 0..12 {
            src.push_str(&format!("  m.remember :p{}\n", i));
        }
        src.push_str("end\nrun :mind do\n  m.recall read-address: :p11, address-noise: 0.3, seed: 1\n  m.recall read-address: :p0, address-noise: 0.3, seed: 1\nend");
        let out = run(&src);
        assert!(out[0].ends_with("-> :p11"), "{}", out[0]);
        assert!(!out[1].ends_with("-> :p0"), "{}", out[1]);
    }

    #[test]
    fn without_fade_the_oldest_of_the_same_twelve_is_still_found() {
        // vacuity control for the fade test: the same twelve memories with no fade keep :p0
        let mut src = String::from("model :mind do\n  memory :m, size: 256\n");
        for i in 0..12 {
            src.push_str(&format!("  m.remember :p{}\n", i));
        }
        src.push_str("end\nrun :mind do\n  m.recall read-address: :p0, address-noise: 0.3, seed: 1\nend");
        let out = run(&src);
        assert!(out[0].ends_with("-> :p0"), "{}", out[0]);
    }

    const KEYED: &str = "model :mind do
  memory :m, size: 512
  m.remember :cat
  m.save :note, \"meet at the harbour at nine\", key: \"blue heron\"
  m.remember :dog
end
";

    #[test]
    fn a_keyed_note_comes_back_with_its_key_and_the_text_is_held_nowhere() {
        let mut it = Interp::default();
        let out = it.exec(&format!("{}run :mind do\n  m.recall key: \"blue heron\", seed: 3\nend", KEYED)).unwrap();
        assert!(out[0].ends_with("text \"meet at the harbour at nine\""), "{:?}", out);
        let notes = &it.models["mind"].notes["memory:m"];
        assert!(notes.1.iter().all(|w| !w.contains("harbour")), "the keyed text leaked into the notes: {:?}", notes.1);
    }

    #[test]
    fn a_wrong_key_reads_no_letter_of_the_note() {
        // negative control for the key: same landscape, different key
        let out = run(&format!("{}run :mind do\n  m.recall key: \"red heron\", seed: 3\nend", KEYED));
        assert!(!out[0].contains("harbour"), "{:?}", out);
    }

    #[test]
    fn the_key_turn_is_a_rotation_of_the_cube() {
        // a turn is invertible and keeps distances: undo(apply(x)) = x, and overlaps are preserved
        let t = super::key_turn("k", 64);
        let x = super::code("x", 64);
        let y = super::code("y", 64);
        assert_eq!(t.undo(&t.apply(&x)), x);
        let dot = |a: &[f64], b: &[f64]| a.iter().zip(b).map(|(p, q)| p * q).sum::<f64>();
        assert_eq!(dot(&t.apply(&x), &t.apply(&y)), dot(&x, &y));
        assert_ne!(t.apply(&x), x);
    }
}
