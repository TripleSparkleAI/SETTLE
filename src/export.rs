//! EXPORT: write a SETTLE model in formats other samplers and chip toolkits read, and read it back.
//!
//! ```text
//! model :weather do
//!   thing :rain, leans: :no, by: 1
//!   ...
//!   export "weather.json", as: :ising     # plain Ising data (the default form)
//!   export "weather.qubo.json", as: :qubo # the 0/1 form, with its energy constant
//! end
//! model :copy do
//!   import "weather.json"                 # things, leans and pulls, bit for bit
//! end
//! run :copy do
//!   import "weather.json"                 # held things and temperature; refuses if the model differs
//!   settle 40_000, seed: 1
//!   export "rates.json", as: :moments     # yes-rates and pair averages, for a checker
//!   anneal 2_000, seed: 1
//!   export "best.json", as: :best         # the calmest arrangement found and its energy
//! end
//! ```
//!
//! The forms (schema and equations, each with a plain reading: `runs/backends/REPORT_SETTLEBACKENDS.md`):
//! - `:ising` (`settle-ising` v1). Things are s_i in {-1, +1}. Energy E(s) = -Σ_i h_i s_i - Σ_{i<j} J_ij s_i s_j,
//!   and a settled machine visits s with chance proportional to exp(-E(s)/T). Pulls are a sparse edge list
//!   [i, j, J] with i < j, sorted. Held things are [i, ±1]. Numbers are written in Rust's shortest form that
//!   parses back to the same f64, so an export then import is exact.
//! - `:qubo` (`settle-qubo` v1). Bits x_i in {0, 1} with x_i = 1 meaning yes, so s_i = 2 x_i - 1. Then
//!   E(s) = Σ_i a_i x_i + Σ_{i<j} Q_ij x_i x_j + c with a_i = -2 h_i + 2 Σ_j J_ij, Q_ij = -4 J_ij and
//!   c = Σ_i h_i - Σ_{i<j} J_ij. The file carries a, Q and c ("offset"), so a QUBO solver's energy plus c is
//!   exactly the SETTLE energy.
//! - `:gset` and `:dimacs`, for max-cut only (every lean zero). A max-cut model pushes each edge's ends apart,
//!   J_ij = -w_ij, so E = Σ w_ij s_i s_j and cut = (W - E)/2 with W the total weight. Gset: a line "n m", then
//!   "i j w" per edge, 1-based. DIMACS: "p edge n m", then "e i j w" per edge (the common weighted extension).
//! - `:moments` (`settle-moments` v1): the last settle's yes-rates and, when the samples were kept, the average
//!   of s_i s_j for every pair (every pair up to 64 things, else the pulled pairs only).
//! - `:best` (`settle-best` v1): the calmest arrangement the last anneal found, and its energy.
//!
//! Only the yes/no part of a model is exported. Real-valued `number` things (kept in the model's notes) are not.

use crate::ext::{Claim, Ctx, Ext};
use crate::lex::{err, kw, kwargs, only, SettleError, Tok};
use crate::model::{Model, State};
use crate::doors::json::{number as fnum, string as fstr};
pub use crate::doors::json::{parse_json, Json};
use std::collections::HashMap;

pub struct Export;

/// A model as plain Ising data.
#[derive(Clone, Debug, PartialEq)]
pub struct Ising {
    pub names: Vec<String>,
    pub h: Vec<f64>,
    /// (i, j, J) with i < j, sorted, no zero pulls.
    pub edges: Vec<(usize, usize, f64)>,
    pub temperature: f64,
    /// (i, ±1), sorted by i.
    pub held: Vec<(usize, f64)>,
}

/// The 0/1 form: E = Σ linear_i x_i + Σ quad (i, j, Q) x_i x_j + offset.
#[derive(Clone, Debug)]
pub struct Qubo {
    pub linear: Vec<f64>,
    pub quad: Vec<(usize, usize, f64)>,
    pub offset: f64,
}

pub fn to_ising(m: &Model, held: &HashMap<usize, f64>, temperature: f64) -> Ising {
    let mut edges = Vec::new();
    for i in 0..m.len() {
        for &(k, w) in &m.adj[i] {
            if k > i && w != 0.0 {
                edges.push((i, k, w));
            }
        }
    }
    edges.sort_by_key(|a| (a.0, a.1));
    let mut hl: Vec<(usize, f64)> = held.iter().map(|(&i, &v)| (i, v)).collect();
    hl.sort_by_key(|a| a.0);
    Ising { names: m.names.clone(), h: m.h.clone(), edges, temperature, held: hl }
}

/// A fresh model with exactly these things, leans and pulls.
pub fn into_model(d: &Ising) -> Model {
    let mut m = Model::default();
    merge_into(&mut m, d);
    m
}

/// Add these things, leans and pulls to a model (leans and pulls add, as `thing ... by:` and `pulls` do).
pub fn merge_into(m: &mut Model, d: &Ising) {
    let ids: Vec<usize> = d.names.iter().map(|n| m.add(n)).collect();
    for (k, &i) in ids.iter().enumerate() {
        m.h[i] += d.h[k];
    }
    for &(i, j, w) in &d.edges {
        m.couple(ids[i], ids[j], w);
    }
}

pub fn energy(d: &Ising, s: &[f64]) -> f64 {
    let mut e = 0.0;
    for (i, &h) in d.h.iter().enumerate() {
        e -= h * s[i];
    }
    for &(i, j, w) in &d.edges {
        e -= w * s[i] * s[j];
    }
    e
}

pub fn to_qubo(d: &Ising) -> Qubo {
    let mut linear: Vec<f64> = d.h.iter().map(|h| -2.0 * h).collect();
    let mut quad = Vec::new();
    let mut offset: f64 = d.h.iter().sum();
    for &(i, j, w) in &d.edges {
        linear[i] += 2.0 * w;
        linear[j] += 2.0 * w;
        quad.push((i, j, -4.0 * w));
        offset -= w;
    }
    Qubo { linear, quad, offset }
}

/// QUBO energy of 0/1 bits, without the offset.
pub fn qubo_energy(q: &Qubo, x: &[bool]) -> f64 {
    let mut e = 0.0;
    for (i, &a) in q.linear.iter().enumerate() {
        if x[i] {
            e += a;
        }
    }
    for &(i, j, w) in &q.quad {
        if x[i] && x[j] {
            e += w;
        }
    }
    e
}

// -------------------------------------------------------------------------------------------------------
// writing
// -------------------------------------------------------------------------------------------------------

fn names_json(names: &[String]) -> String {
    format!("[{}]", names.iter().map(|n| fstr(n)).collect::<Vec<_>>().join(", "))
}

fn nums_json(v: &[f64]) -> Result<String, String> {
    Ok(format!("[{}]", v.iter().map(|&x| fnum(x)).collect::<Result<Vec<_>, _>>()?.join(", ")))
}

fn triples_json(v: &[(usize, usize, f64)]) -> Result<String, String> {
    let mut rows = Vec::new();
    for &(i, j, w) in v {
        rows.push(format!("    [{}, {}, {}]", i, j, fnum(w)?));
    }
    Ok(if rows.is_empty() { "[]".into() } else { format!("[\n{}\n  ]", rows.join(",\n")) })
}

fn held_json(held: &[(usize, f64)]) -> String {
    format!("[{}]", held.iter().map(|&(i, v)| format!("[{}, {}]", i, if v > 0.0 { 1 } else { -1 })).collect::<Vec<_>>().join(", "))
}

pub const ISING_CONVENTION: &str =
    "E(s) = -sum_i h_i s_i - sum_{i<j} J_ij s_i s_j, s_i in {-1,+1} (+1 = yes); p(s) proportional to exp(-E(s)/T)";
pub const QUBO_CONVENTION: &str =
    "E = sum_i linear_i x_i + sum_{i<j} Q_ij x_i x_j + offset, x_i in {0,1} (1 = yes), s_i = 2 x_i - 1; E equals the Ising energy";

pub fn ising_json(d: &Ising) -> Result<String, String> {
    Ok(format!(
        "{{\n  \"format\": \"settle-ising\",\n  \"version\": 1,\n  \"convention\": {},\n  \"n\": {},\n  \"names\": {},\n  \"h\": {},\n  \"edges\": {},\n  \"temperature\": {},\n  \"held\": {}\n}}\n",
        fstr(ISING_CONVENTION),
        d.names.len(),
        names_json(&d.names),
        nums_json(&d.h)?,
        triples_json(&d.edges)?,
        fnum(d.temperature)?,
        held_json(&d.held)
    ))
}

pub fn qubo_json(d: &Ising) -> Result<String, String> {
    let q = to_qubo(d);
    let held_bits = format!("[{}]", d.held.iter().map(|&(i, v)| format!("[{}, {}]", i, if v > 0.0 { 1 } else { 0 })).collect::<Vec<_>>().join(", "));
    Ok(format!(
        "{{\n  \"format\": \"settle-qubo\",\n  \"version\": 1,\n  \"convention\": {},\n  \"n\": {},\n  \"names\": {},\n  \"linear\": {},\n  \"quadratic\": {},\n  \"offset\": {},\n  \"temperature\": {},\n  \"held\": {}\n}}\n",
        fstr(QUBO_CONVENTION),
        d.names.len(),
        names_json(&d.names),
        nums_json(&q.linear)?,
        triples_json(&q.quad)?,
        fnum(q.offset)?,
        fnum(d.temperature)?,
        held_bits
    ))
}

/// Max-cut weights w = -J; refuses a model with any lean (a max-cut graph has none).
fn cut_edges(d: &Ising) -> Result<Vec<(usize, usize, f64)>, String> {
    if let Some(i) = d.h.iter().position(|&h| h != 0.0) {
        return Err(format!("max-cut formats carry no leans, but :{} leans by {}", d.names[i], d.h[i]));
    }
    Ok(d.edges.iter().map(|&(i, j, w)| (i, j, -w)).collect())
}

pub fn gset_text(d: &Ising) -> Result<String, String> {
    let e = cut_edges(d)?;
    let mut o = format!("{} {}\n", d.names.len(), e.len());
    for (i, j, w) in e {
        o.push_str(&format!("{} {} {}\n", i + 1, j + 1, fnum(w)?));
    }
    Ok(o)
}

pub fn dimacs_text(d: &Ising) -> Result<String, String> {
    let e = cut_edges(d)?;
    let mut o = String::from("c SETTLE max-cut export: an edge i-j of weight w is the pull J_ij = -w\n");
    for (k, n) in d.names.iter().enumerate() {
        o.push_str(&format!("c node {} {}\n", k + 1, n));
    }
    o.push_str(&format!("p edge {} {}\n", d.names.len(), e.len()));
    for (i, j, w) in e {
        o.push_str(&format!("e {} {} {}\n", i + 1, j + 1, fnum(w)?));
    }
    Ok(o)
}

pub fn moments_json(m: &Model, st: &State) -> Result<String, String> {
    if st.n == 0 {
        return Err("export as: :moments needs a settle first".into());
    }
    let rates = st.rates();
    let mut pairs = Vec::new();
    if !st.samples.is_empty() {
        let want: Vec<(usize, usize)> = if m.len() <= 64 {
            (0..m.len()).flat_map(|i| (i + 1..m.len()).map(move |j| (i, j))).collect()
        } else {
            let mut v: Vec<(usize, usize)> = (0..m.len()).flat_map(|i| m.adj[i].iter().filter(move |e| e.0 > i).map(move |e| (i, e.0))).collect();
            v.sort();
            v
        };
        for (i, j) in want {
            let c = st.samples.iter().map(|s| s[i] * s[j]).sum::<f64>() / st.samples.len() as f64;
            pairs.push((i, j, c));
        }
    }
    let mut held: Vec<(usize, f64)> = st.held.iter().map(|(&i, &v)| (i, v)).collect();
    held.sort_by_key(|a| a.0);
    Ok(format!(
        "{{\n  \"format\": \"settle-moments\",\n  \"version\": 1,\n  \"names\": {},\n  \"samples\": {},\n  \"temperature\": {},\n  \"held\": {},\n  \"yes_rate\": {},\n  \"pair_mean\": {}\n}}\n",
        names_json(&m.names),
        st.n,
        fnum(st.temp)?,
        held_json(&held),
        nums_json(&rates)?,
        triples_json(&pairs)?
    ))
}

pub fn best_json(m: &Model, st: &State) -> Result<String, String> {
    let (b, e) = st.best.as_ref().ok_or("export as: :best needs an anneal first")?;
    Ok(format!(
        "{{\n  \"format\": \"settle-best\",\n  \"version\": 1,\n  \"names\": {},\n  \"s\": [{}],\n  \"energy\": {}\n}}\n",
        names_json(&m.names),
        b.iter().map(|&v| if v > 0.0 { "1" } else { "-1" }).collect::<Vec<_>>().join(", "),
        fnum(*e)?
    ))
}

// -------------------------------------------------------------------------------------------------------
// reading: a small JSON parser, enough for these files
// -------------------------------------------------------------------------------------------------------

fn as_num(j: &Json, what: &str) -> Result<f64, String> {
    match j {
        Json::Num(x) => Ok(*x),
        _ => Err(format!("{} must be a number", what)),
    }
}

fn as_arr<'a>(j: &'a Json, what: &str) -> Result<&'a [Json], String> {
    match j {
        Json::Arr(v) => Ok(v),
        _ => Err(format!("{} must be a list", what)),
    }
}

fn as_index(j: &Json, n: usize, what: &str) -> Result<usize, String> {
    let x = as_num(j, what)?;
    if x < 0.0 || x.fract() != 0.0 || x as usize >= n {
        return Err(format!("{} {} is not a thing index below {}", what, x, n));
    }
    Ok(x as usize)
}

/// Read a `settle-ising` v1 file. Refuses anything it cannot read exactly.
pub fn parse_ising(src: &str) -> Result<Ising, String> {
    let j = parse_json(src)?;
    match j.get("format") {
        Some(Json::Str(f)) if f == "settle-ising" => {}
        _ => return Err("not a settle-ising file (\"format\": \"settle-ising\" is missing)".into()),
    }
    match j.get("version") {
        Some(Json::Num(v)) if *v == 1.0 => {}
        _ => return Err("only settle-ising version 1 is known".into()),
    }
    let names: Vec<String> = as_arr(j.get("names").ok_or("missing names")?, "names")?
        .iter()
        .map(|x| match x {
            Json::Str(s) if !s.is_empty() => Ok(s.clone()),
            _ => Err("every name must be a non-empty string".to_string()),
        })
        .collect::<Result<_, _>>()?;
    let n = names.len();
    let mut seen = std::collections::HashSet::new();
    for nm in &names {
        if !seen.insert(nm) {
            return Err(format!("the name {} appears twice", nm));
        }
    }
    if let Some(v) = j.get("n") {
        if as_num(v, "n")? != n as f64 {
            return Err(format!("n says {} but there are {} names", as_num(v, "n")?, n));
        }
    }
    let h: Vec<f64> = as_arr(j.get("h").ok_or("missing h")?, "h")?.iter().map(|x| as_num(x, "a lean")).collect::<Result<_, _>>()?;
    if h.len() != n {
        return Err(format!("{} leans for {} names", h.len(), n));
    }
    let mut edges = Vec::new();
    let mut pairs = std::collections::HashSet::new();
    for e in as_arr(j.get("edges").ok_or("missing edges")?, "edges")? {
        let e = as_arr(e, "an edge")?;
        if e.len() != 3 {
            return Err("an edge is [i, j, J]".into());
        }
        let (a, b) = (as_index(&e[0], n, "edge end")?, as_index(&e[1], n, "edge end")?);
        if a == b {
            return Err(format!("edge {}-{} joins a thing to itself", a, b));
        }
        let (i, k) = (a.min(b), a.max(b));
        if !pairs.insert((i, k)) {
            return Err(format!("edge {}-{} appears twice", i, k));
        }
        edges.push((i, k, as_num(&e[2], "a pull")?));
    }
    edges.retain(|e| e.2 != 0.0);
    edges.sort_by_key(|a| (a.0, a.1));
    let temperature = match j.get("temperature") {
        None => 1.0,
        Some(v) => as_num(v, "temperature")?,
    };
    if temperature <= 0.0 {
        return Err("temperature must be above zero".into());
    }
    let mut held = Vec::new();
    if let Some(hv) = j.get("held") {
        for e in as_arr(hv, "held")? {
            let e = as_arr(e, "a held entry")?;
            if e.len() != 2 {
                return Err("a held entry is [i, 1 or -1]".into());
            }
            let v = as_num(&e[1], "a held value")?;
            if v != 1.0 && v != -1.0 {
                return Err(format!("a held value is 1 or -1, not {}", v));
            }
            held.push((as_index(&e[0], n, "held thing")?, v));
        }
    }
    held.sort_by_key(|a| a.0);
    Ok(Ising { names, h, edges, temperature, held })
}

// -------------------------------------------------------------------------------------------------------
// statements
// -------------------------------------------------------------------------------------------------------

fn write_file(ctx: &Ctx, path: &str, body: &str, ln: usize) -> Result<std::path::PathBuf, SettleError> {
    let p = ctx.path(path);
    if let Some(dir) = p.parent() {
        if !dir.as_os_str().is_empty() {
            std::fs::create_dir_all(dir).or_else(|e| err(ln, format!("cannot make {}: {}", dir.display(), e)))?;
        }
    }
    std::fs::write(&p, body).or_else(|e| err(ln, format!("cannot write {}: {}", p.display(), e)))?;
    Ok(p)
}

fn export(m: &Model, st: Option<&State>, path: &str, rest: &[Tok], ln: usize, ctx: &mut Ctx) -> Result<(), SettleError> {
    let kv = kwargs(rest, ln)?;
    only(&kv, &["as"], "export", ln)?;
    let form = match kw(&kv, "as") {
        None => "ising".to_string(),
        Some(Tok::Sym(s)) => s.clone(),
        Some(_) => return err(ln, "as: takes :ising, :qubo, :gset, :dimacs, :moments or :best"),
    };
    let empty = HashMap::new();
    let (held, temp) = match st {
        Some(s) => (&s.held, s.temp),
        None => (&empty, 1.0),
    };
    let d = to_ising(m, held, temp);
    let body = match form.as_str() {
        "ising" => ising_json(&d),
        "qubo" => qubo_json(&d),
        "gset" => gset_text(&d),
        "dimacs" => dimacs_text(&d),
        "moments" | "best" => match st {
            None => Err(format!("as: :{} is a run statement (it reads the last settle or anneal)", form)),
            Some(s) => {
                if form == "moments" {
                    moments_json(m, s)
                } else {
                    best_json(m, s)
                }
            }
        },
        other => return err(ln, format!("unknown form :{} (known: :ising, :qubo, :gset, :dimacs, :moments, :best)", other)),
    };
    let body = body.or_else(|e| err(ln, e))?;
    let p = write_file(ctx, path, &body, ln)?;
    let mut line = format!("exported :{} ({} things, {} pulls) to {}", form, d.names.len(), d.edges.len(), p.display());
    if m.notes.keys().any(|k| k.starts_with("numbers")) {
        line.push_str(" (real-valued number things are not exported)");
    }
    ctx.say(line);
    Ok(())
}

fn read_ising(path: &str, ln: usize, ctx: &Ctx) -> Result<Ising, SettleError> {
    let p = ctx.path(path);
    let src = std::fs::read_to_string(&p).or_else(|e| err(ln, format!("cannot read {}: {}", p.display(), e)))?;
    parse_ising(&src).or_else(|e| err(ln, format!("{}: {}", p.display(), e)))
}

impl Ext for Export {
    fn name(&self) -> &'static str {
        "export"
    }

    fn statements(&self) -> &'static [&'static str] {
        &[
            "model: export \"m.json\", as: :ising   (or :qubo, :gset, :dimacs)",
            "model: import \"m.json\"   (things, leans and pulls from a settle-ising file)",
            "run: export \"m.json\", as: :ising   (also :qubo, :gset, :dimacs, :moments, :best)",
            "run: import \"m.json\"   (held things and temperature; refuses if the model differs)",
        ]
    }

    fn model_stmt(&self, m: &mut Model, t: &[Tok], ln: usize, ctx: &mut Ctx) -> Claim {
        match t {
            [Tok::Ident(k), Tok::Str(p), rest @ ..] if k == "export" => Some(export(m, None, p, rest, ln, ctx)),
            [Tok::Ident(k), Tok::Str(p)] if k == "import" => Some(read_ising(p, ln, ctx).map(|d| {
                merge_into(m, &d);
                ctx.say(format!("imported {} things and {} pulls from {}", d.names.len(), d.edges.len(), p));
            })),
            [Tok::Ident(k), ..] if k == "import" => Some(err(ln, "import takes one \"quoted\" path")),
            _ => None,
        }
    }

    fn run_stmt(&self, m: &mut Model, st: &mut State, t: &[Tok], ln: usize, ctx: &mut Ctx) -> Claim {
        match t {
            [Tok::Ident(k), Tok::Str(p), rest @ ..] if k == "export" => Some(export(m, Some(st), p, rest, ln, ctx)),
            [Tok::Ident(k), Tok::Str(p)] if k == "import" => Some(read_ising(p, ln, ctx).and_then(|d| {
                let here = to_ising(m, &HashMap::new(), d.temperature);
                let theirs = Ising { held: Vec::new(), ..d.clone() };
                if here != theirs {
                    return err(ln, format!("{} describes a different model than this run's (names, leans or pulls differ)", p));
                }
                st.held = d.held.iter().copied().collect();
                st.temp = d.temperature;
                ctx.say(format!("imported {} held things and temperature {} from {}", d.held.len(), d.temperature, p));
                Ok(())
            })),
            [Tok::Ident(k), ..] if k == "import" => Some(err(ln, "import takes one \"quoted\" path")),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interp::Interp;
    use crate::rng::Rng;

    const WEATHER: &str = "model :weather do
  thing :rain,      leans: :no, by: 1
  thing :sprinkler, leans: :no, by: 0.5
  thing :wet_grass
  rain.pushes :sprinkler, by: 0.5
  rain.pulls  :wet_grass, by: 1.5
  sprinkler.pulls :wet_grass, by: 1
end";

    fn random_model(n: usize, seed: u64, density: f64) -> Model {
        let mut r = Rng::new(seed);
        let mut m = Model::default();
        for i in 0..n {
            m.add(&format!("x{}", i));
            m.h[i] = r.normal() * 0.7 + 1e-13 * r.signed(); // awkward decimals on purpose
        }
        for i in 0..n {
            for j in i + 1..n {
                if r.unit() < density {
                    m.couple(i, j, r.normal() / 3.0);
                }
            }
        }
        m
    }

    fn arrangements(n: usize) -> impl Iterator<Item = Vec<f64>> {
        (0u64..(1 << n)).map(move |b| (0..n).map(|k| if (b >> k) & 1 == 1 { 1.0 } else { -1.0 }).collect())
    }

    fn tmp(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("settle_export_test_{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d.join(name)
    }

    #[test]
    fn ising_round_trip_is_exact_bit_for_bit() {
        for seed in 1..6 {
            let m = random_model(17, seed, 0.4);
            let mut held = HashMap::new();
            held.insert(3, 1.0);
            held.insert(11, -1.0);
            let d = to_ising(&m, &held, 0.731_234_567_891_234_5);
            let back = parse_ising(&ising_json(&d).unwrap()).unwrap();
            assert_eq!(d, back);
            let m2 = into_model(&back);
            assert_eq!(m.names, m2.names);
            for i in 0..m.len() {
                assert_eq!(m.h[i].to_bits(), m2.h[i].to_bits());
                for k in 0..m.len() {
                    if i != k {
                        assert_eq!(m.coupling(i, k).to_bits(), m2.coupling(i, k).to_bits(), "pull {}-{}", i, k);
                    }
                }
            }
            for s in arrangements(10).take(64) {
                let mut full = vec![1.0; 17];
                full[..10].copy_from_slice(&s);
                assert_eq!(m.energy(&full).to_bits(), m2.energy(&full).to_bits());
            }
        }
    }

    #[test]
    fn qubo_energy_plus_offset_equals_ising_energy_on_every_arrangement() {
        for seed in 1..4 {
            let m = random_model(12, seed, 0.5);
            let d = to_ising(&m, &HashMap::new(), 1.0);
            let q = to_qubo(&d);
            let scale = 1.0 + d.h.iter().map(|x| x.abs()).sum::<f64>() + d.edges.iter().map(|e| e.2.abs()).sum::<f64>();
            let mut worst: f64 = 0.0;
            for s in arrangements(12) {
                let x: Vec<bool> = s.iter().map(|&v| v > 0.0).collect();
                let diff = (qubo_energy(&q, &x) + q.offset - m.energy(&s)).abs();
                worst = worst.max(diff);
            }
            assert!(worst < 1e-12 * scale, "worst gap {}", worst);
            // negative control: dropping the constant is visible
            let x = vec![false; 12];
            if q.offset.abs() > 1e-9 {
                assert!((qubo_energy(&q, &x) - m.energy(&[-1.0; 12])).abs() > 1e-9);
            }
        }
    }

    #[test]
    fn qubo_all_zero_bits_is_minus_the_offset_identity() {
        // all x = 0 is all s = -1, where E = Σ h_i - Σ J_ij, which is exactly the offset
        let m = random_model(9, 7, 0.6);
        let d = to_ising(&m, &HashMap::new(), 1.0);
        let q = to_qubo(&d);
        assert!((q.offset - m.energy(&[-1.0; 9])).abs() < 1e-12);
    }

    #[test]
    fn statements_export_then_import_reproduce_the_model_and_the_run() {
        let dir = tmp("w");
        let src = format!(
            "{}\nrun :weather do\n  hold :wet_grass, :yes\n  settle 2_000, temperature: 0.8, seed: 1\n  export \"{p}/w.json\", as: :ising\n  export \"{p}/w.qubo.json\", as: :qubo\n  export \"{p}/m.json\", as: :moments\nend\nmodel :copy do\n  import \"{p}/w.json\"\nend\nrun :copy do\n  import \"{p}/w.json\"\n  settle 2_000, seed: 1\n  export \"{p}/m2.json\", as: :moments\nend",
            WEATHER,
            p = dir.display()
        );
        let mut it = Interp::default();
        let out = it.exec(&src).unwrap();
        assert!(out.iter().any(|l| l.starts_with("exported :ising (3 things, 3 pulls)")), "{:?}", out);
        let a = &it.models["weather"];
        let b = &it.models["copy"];
        assert_eq!(to_ising(a, &HashMap::new(), 1.0), to_ising(b, &HashMap::new(), 1.0));
        // same model, same held, same temperature, same seed: the moment files are identical
        let m1 = std::fs::read_to_string(dir.join("m.json")).unwrap();
        let m2 = std::fs::read_to_string(dir.join("m2.json")).unwrap();
        assert_eq!(m1, m2);
        let j = parse_json(&m1).unwrap();
        assert_eq!(j.get("temperature"), Some(&Json::Num(0.8)));
        let q = parse_json(&std::fs::read_to_string(dir.join("w.qubo.json")).unwrap()).unwrap();
        // offset for weather: Σh - ΣJ = (-1 - 0.5 + 0) - (-0.5 + 1.5 + 1) = -3.5
        assert_eq!(q.get("offset"), Some(&Json::Num(-3.5)));
    }

    #[test]
    fn run_import_refuses_a_file_for_a_different_model() {
        let dir = tmp("x");
        let src = format!(
            "{}\nrun :weather do\n  export \"{p}/w.json\"\nend",
            WEATHER,
            p = dir.display()
        );
        Interp::default().exec(&src).unwrap();
        // flip one pull's sign in the file: the run import must notice (control for the model check)
        let body = std::fs::read_to_string(dir.join("w.json")).unwrap().replace("[0, 2, 1.5]", "[0, 2, -1.5]");
        std::fs::write(dir.join("bad.json"), body).unwrap();
        let src2 = format!("{}\nrun :weather do\n  import \"{p}/bad.json\"\nend", WEATHER, p = dir.display());
        let e = Interp::default().exec(&src2).err().unwrap().0;
        assert!(e.contains("describes a different model"), "{}", e);
    }

    #[test]
    fn parse_refuses_bad_files() {
        let good = ising_json(&to_ising(&random_model(4, 1, 1.0), &HashMap::new(), 1.0)).unwrap();
        assert!(parse_ising(&good).is_ok());
        let cases = [
            (good.replace("settle-ising", "other"), "not a settle-ising"),
            (good.replace("\"n\": 4", "\"n\": 5"), "n says"),
            (good.replace("\"x1\"", "\"x0\""), "appears twice"),
            (good.replace("\"held\": []", "\"held\": [[9, 1]]"), "not a thing index"),
            (good.replace("\"held\": []", "\"held\": [[1, 0]]"), "held value"),
            (good.replace("\"temperature\": 1.0", "\"temperature\": 0"), "temperature"),
            (good.replace("}\n", "} extra"), "trailing"),
        ];
        for (src, want) in cases {
            let e = parse_ising(&src).err().unwrap_or_default();
            assert!(e.contains(want), "wanted {:?}, got {:?}", want, e);
        }
    }

    #[test]
    fn maxcut_exports_to_gset_and_dimacs_with_cut_equal_to_half_w_minus_e() {
        let dir = tmp("g");
        let src = format!(
            "model :ring do\n  maxcut :g, edges: \"a-b b-c c-d d-e e-f f-a a-d:2\"\n  export \"{p}/ring.gset\", as: :gset\n  export \"{p}/ring.dimacs\", as: :dimacs\nend",
            p = dir.display()
        );
        let mut it = Interp::default();
        it.exec(&src).unwrap();
        let g = std::fs::read_to_string(dir.join("ring.gset")).unwrap();
        assert_eq!(g.lines().next(), Some("6 7"));
        assert!(g.lines().any(|l| l == "1 4 2.0"), "{}", g);
        let dm = std::fs::read_to_string(dir.join("ring.dimacs")).unwrap();
        assert!(dm.contains("p edge 6 7") && dm.contains("e 1 4 2.0"));
        let m = &it.models["ring"];
        let d = to_ising(m, &HashMap::new(), 1.0);
        let w_total: f64 = d.edges.iter().map(|e| -e.2).sum();
        for s in arrangements(6) {
            let cut: f64 = d.edges.iter().filter(|e| s[e.0] != s[e.1]).map(|e| -e.2).sum();
            assert!((cut - (w_total - m.energy(&s)) / 2.0).abs() < 1e-12);
        }
        // a model with a lean is refused
        let e = Interp::default().exec(&format!("model :m do\n  thing :a, leans: :yes, by: 1\n  export \"{}/x.gset\", as: :gset\nend", dir.display())).err().unwrap().0;
        assert!(e.contains("carry no leans"), "{}", e);
    }

    #[test]
    fn best_export_carries_the_anneal_energy() {
        let dir = tmp("b");
        let src = format!("{}\nrun :weather do\n  anneal 500, seed: 2\n  export \"{p}/b.json\", as: :best\nend", WEATHER, p = dir.display());
        let mut it = Interp::default();
        it.exec(&src).unwrap();
        let j = parse_json(&std::fs::read_to_string(dir.join("b.json")).unwrap()).unwrap();
        let s: Vec<f64> = as_arr(j.get("s").unwrap(), "s").unwrap().iter().map(|x| as_num(x, "s").unwrap()).collect();
        let e = as_num(j.get("energy").unwrap(), "energy").unwrap();
        assert_eq!(it.models["weather"].energy(&s), e);
    }

    #[test]
    fn json_parser_reads_escapes_and_numbers() {
        let j = parse_json(r#"{"a": [1, -2.5e-3, true, null], "b": "q\"\\\u0041\n"}"#).unwrap();
        assert_eq!(j.get("b"), Some(&Json::Str("q\"\\A\n".into())));
        assert_eq!(as_arr(j.get("a").unwrap(), "a").unwrap()[1], Json::Num(-2.5e-3));
        assert!(parse_json("[1, 2").is_err());
        assert_eq!(fstr("a\"b\\c\n"), "\"a\\\"b\\\\c\\u000a\"");
    }
}
