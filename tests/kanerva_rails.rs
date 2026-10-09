//! SETTLE's sdm-family statements and KANERVA's Rails face speak the same words and give the same answers.
//!
//! <claudes_code_comments>
//! ** Function List **
//! said(statement)                               - the keywords SETTLE says a statement takes, from its own refusal
//! settle_accepts_exactly_kanervas_word_list     - one list (no sdm file checks keywords itself) and both ways:
//!                                                 what SETTLE says each statement takes equals kanerva::words
//! the_parity_check_can_fail                     - the control: a list one word longer or shorter differs
//! settle_takes_every_listed_keyword_and_refuses_one_that_is_not - the behaviour: a line with every keyword parses,
//!                                                 a line with one invented keyword is refused by name
//! the_retired_words_are_one_list                - settle::lex::RETIRED equals kanerva::words::RETIRED
//! the_rails_example_prints_what_settle_prints   - kanerva/examples/rails.rs against the SETTLE programs it mirrors
//! the_comparison_can_fail                       - the control: one changed line is caught
//! spelling(family, name)                        - the spelling SETTLE takes today (an alias, before ONEPARSER lands)
//!
//! ** Technical Review **
//! - KANERVA may not depend on SETTLE, so the test that needs both lives here. The word list's home is
//!   kanerva::words (five families, thirteen statements; `memory` is SETTLE's own and is not in it). SETTLE
//!   reads every sdm-family line through THE PLUG (src/plug.rs), which hands it to kanerva::lang, so the first test
//!   checks two things: no sdm-family file keeps a keyword list of its own (`only(&kv, &[...])` outside its tests;
//!   memory.rs, which still does, is the scan's positive control), and, for each statement that takes keywords,
//!   the list SETTLE prints when it refuses an invented keyword equals the list's words, both ways. Names compare
//!   after `words::canonical`, so `write_samples` is `write-samples`.
//! - The behaviour check builds one line per statement and verb carrying every keyword at a valid value, and
//!   asserts SETTLE does not refuse any keyword (a later refusal, like "read-address: or key:, not both", is not a
//!   keyword refusal). The control adds `zzinvented:` and asserts SETTLE names it.
//! - The answer check runs settle-rs/examples/{sdm,softsdm}.settle and a program for sdmscale,
//!   refusal and contenttrack, and compares every line with kanerva's rails example, which reads the same
//!   memories through the builder. They must be byte-identical.
//!
//! </claudes_code_comments>

use kanerva::words::{canonical, statement, Args, DefaultValue, Head, Place, Statement, Value, ALIASES, STATEMENTS};
use settle::interp::Interp;
use std::collections::BTreeSet;

#[path = "../../kanerva/examples/rails.rs"]
#[allow(dead_code)]
mod rails;

/// The five sdm-family files, which hold SETTLE's engine bridges and no keyword list of their own.
const BRIDGES: &[(&str, &str)] = &[
    ("sdm.rs", include_str!("../src/sdm.rs")),
    ("softsdm.rs", include_str!("../src/softsdm.rs")),
    ("sdmscale.rs", include_str!("../src/sdmscale.rs")),
    ("sdmrefuse.rs", include_str!("../src/sdmrefuse.rs")),
    ("sdmtrack.rs", include_str!("../src/sdmtrack.rs")),
];

/// The control for the scan below: memory.rs is SETTLE's own statement and still checks its keywords itself.
const MEMORY: &str = include_str!("../src/memory.rs");

fn keyword_checks(src: &str) -> usize {
    src.split("#[cfg(test)]\nmod tests").next().unwrap().matches("only(&kv, &[").count()
}

/// The keywords SETTLE says a statement takes, read from its refusal of an invented keyword: "<statement> does
/// not take `zzinvented:`; it takes `a:`, `b:` and `c:`". A statement with one keyword says "it takes `a:`".
fn said(s: &Statement) -> BTreeSet<String> {
    let bad = program(s, ", zzinvented: 1");
    let e = Interp::default().exec(&bad).expect_err(&bad).0;
    let first = e.lines().next().unwrap();
    let list = first.split("; it takes ").nth(1).unwrap_or_else(|| panic!("{} {}: no list in {:?}", s.family, s.verb, first));
    list.replace(" and ", ", ").split(", ").map(|w| canonical(s.family, w.trim().trim_matches('`').trim_end_matches(':')).to_string()).collect()
}

#[test]
fn settle_accepts_exactly_kanervas_word_list() {
    // one list: no sdm-family file keeps a keyword list of its own, so every sdm line is checked by the plug
    for (file, src) in BRIDGES {
        assert_eq!(keyword_checks(src), 0, "src/{} checks keywords itself; the sdm family's one list is kanerva::words", file);
    }
    assert!(keyword_checks(MEMORY) >= 2, "control: the scan sees memory.rs's own checks");
    // both ways, through the plug: what SETTLE says each statement takes is exactly what the list holds
    let mut checked = 0;
    for s in STATEMENTS.iter().filter(|s| !s.words.is_empty()) {
        let listed: BTreeSet<String> = s.words.iter().map(|w| w.name.to_string()).collect();
        let got = said(s);
        let extra: Vec<&String> = got.difference(&listed).collect();
        let missing: Vec<&String> = listed.difference(&got).collect();
        assert!(extra.is_empty(), "SETTLE's {} {} takes {:?}, which the word list lacks", s.family, s.verb, extra);
        assert!(missing.is_empty(), "the word list's {} {} has {:?}, which SETTLE does not take", s.family, s.verb, missing);
        checked += 1;
    }
    assert_eq!(checked, 10, "the ten statements that take keywords");
}

#[test]
fn the_parity_check_can_fail() {
    // the control for said(): a list with one word more or one word less than SETTLE's is caught
    let s = statement("refusal", "refusal").unwrap();
    let got = said(s);
    let mut more = got.clone();
    more.insert("zzlisted".to_string());
    let mut less = got.clone();
    less.remove("level");
    assert_ne!(more, got);
    assert_ne!(less, got);
    assert_eq!(got, ["level", "load", "word-size"].iter().map(|w| w.to_string()).collect());
}

/// The spelling SETTLE takes today: an alias where the list's spelling of record is newer than the parser.
fn spelling(family: &str, name: &str) -> String {
    ALIASES.iter().find(|(f, _, new)| *f == family && *new == name).map(|(_, old, _)| old.to_string()).unwrap_or_else(|| name.to_string())
}

/// Small settings, so a line with every keyword runs in milliseconds.
const SMALL: &[(&str, &str)] = &[("word-size", "64"), ("hard-locations", "200"), ("samples", "4"), ("load", "20"), ("iterated-reads", "2"), ("rounds", "1"), ("seed", "1")];

fn value(family: &str, w: &kanerva::words::Word) -> String {
    if let Some((_, v)) = SMALL.iter().find(|(k, _)| *k == w.name) {
        return v.to_string();
    }
    match (w.value, w.default) {
        (Value::Symbol(cs), _) if !cs.is_empty() => format!(":{}", cs[cs.len() - 1]),
        (Value::Symbol(_), _) => ":cat".to_string(),
        (Value::Text, _) => "\"k\"".to_string(),
        (_, DefaultValue::Num(x)) => format!("{}", x),
        _ => match (family, w.name) {
            (_, "activation-radius") => "20".to_string(),
            (_, "tolerate-noise") => "0.3".to_string(),
            _ => "0.2".to_string(),
        },
    }
}

fn line(s: &Statement, only_small: bool) -> String {
    s.words
        .iter()
        .filter(|w| !only_small || SMALL.iter().any(|(k, _)| *k == w.name))
        .map(|w| format!("{}: {}", spelling(s.family, w.name), value(s.family, w)))
        .collect::<Vec<_>>()
        .join(", ")
}

fn program(s: &Statement, extra: &str) -> String {
    match s.head {
        Head::Command => format!("model :m do\nend\nrun :m do\n  {} {}{}\nend\n", s.verb, line(s, false), extra),
        Head::Declare => format!("model :m do\n  {} :s, {}{}\nend\n", s.family, line(s, false), extra),
        Head::Method => {
            let decl = statement(s.family, s.family).unwrap();
            let args = match s.args {
                Args::None => String::new(),
                Args::Name => " :cat".to_string(),
                Args::NameText => " :cat, \"hi\"".to_string(),
                Args::Count => " 5".to_string(),
            };
            let sep = if s.args == Args::None || s.words.is_empty() { "" } else { "," };
            let call = format!("s.{}{}{} {}{}", s.verb, args, sep, line(s, false), extra);
            let store = if s.family == "sdmscale" { "s.put :cat" } else { "s.write :cat" };
            let head = format!("model :m do\n  {} :s, {}\n  {}\n", s.family, line(decl, true), store);
            if s.places == [Place::Run] { format!("{}end\nrun :m do\n  {}\nend\n", head, call) } else { format!("{}  {}\nend\n", head, call.replace("s.write :cat, \"hi\"", "s.write :dog, \"hi\"")) }
        }
    }
}

#[test]
fn settle_takes_every_listed_keyword_and_refuses_one_that_is_not() {
    let mut checked = 0;
    for s in STATEMENTS.iter().filter(|s| !s.words.is_empty()) {
        let ok = program(s, "");
        if let Err(e) = Interp::default().exec(&ok) {
            assert!(!e.0.contains("does not take"), "{} {}: SETTLE refused a listed keyword: {}\n{}", s.family, s.verb, e.0, ok);
        }
        let bad = program(s, ", zzinvented: 1");
        let e = Interp::default().exec(&bad).expect_err(&bad);
        assert!(e.0.contains("does not take `zzinvented:`"), "{} {}: {}\n{}", s.family, s.verb, e.0, bad);
        checked += 1;
    }
    assert_eq!(checked, 10, "the ten statements that take keywords (13 less write, put and fill)");
}

#[test]
fn the_retired_words_are_one_list() {
    assert_eq!(settle::lex::RETIRED, kanerva::words::RETIRED);
}

fn settle_output() -> String {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/examples/");
    let mut out = String::new();
    for name in ["sdm", "softsdm"] {
        out.push_str(&format!("== {}\n", name));
        let src = std::fs::read_to_string(format!("{}{}.settle", dir, name)).unwrap();
        for l in Interp::default().exec(&src).unwrap() {
            out.push_str(&l);
            out.push('\n');
        }
    }
    let scale = "model :mind do
  sdmscale :k, word-size: 256, hard-locations: 20000, activation-probability: 0.01
  k.put :cat
  k.fill 200
end
run :mind do
  k.read read-address: :cat, address-noise: 0.2, seed: 1
  k.read read-address: :cat, address-noise: 0.2, via: :pulls, wake: :top, seed: 1
  k.read read-address: :owl, address-noise: 0.0, seed: 2
  refusal word-size: 256, load: 3000, level: 0.01
  contenttrack word-size: 256, hard-locations: 20000, load: 300, address-noise: 0.3, samples: 50
end
";
    let lines = Interp::default().exec(scale).unwrap();
    out.push_str("== sdmscale\n");
    for (i, l) in lines.iter().enumerate() {
        if i == 3 {
            out.push_str("== refusal\n");
        }
        if i == 4 {
            out.push_str("== contenttrack\n");
        }
        out.push_str(l);
        out.push('\n');
    }
    out
}

fn first_difference(a: &str, b: &str) -> Option<String> {
    if a == b {
        return None;
    }
    let i = a.lines().zip(b.lines()).position(|(x, y)| x != y).unwrap_or(a.lines().count().min(b.lines().count()));
    Some(format!("line {}:\n  SETTLE: {:?}\n  rails:  {:?}", i + 1, a.lines().nth(i), b.lines().nth(i)))
}

#[test]
fn the_rails_example_prints_what_settle_prints() {
    let (s, r) = (settle_output(), rails::run());
    assert!(s.lines().count() >= 25, "{}", s);
    if let Some(d) = first_difference(&s, &r) {
        panic!("{}", d);
    }
}

#[test]
fn the_comparison_can_fail() {
    let s = settle_output();
    let changed = s.replacen("+1.00", "+0.99", 1);
    assert!(first_difference(&s, &changed).is_some());
}
