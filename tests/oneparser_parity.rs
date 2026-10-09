//! ONE PARSER: SETTLE and KANERVA read the sdm family the same way, and run it to the same printed lines.
//!
//! <claudes_code_comments>
//! ** Function List **
//! settle_files()                         - every .settle in examples/ and docs/examples, and KANERVA's programs
//! corpus()                               - the sdm-family corpus: tests/oneparser_corpus.txt split into programs
//! the_two_lexers_read_every_line_alike   - SETTLE's lexer and KANERVA's agree token for token on every line
//! the_retired_words_are_one_list         - SETTLE's RETIRED table equals KANERVA's
//! the_helpers_agree                      - suggest, edit_distance and locate agree on a set of cases
//! settle_and_kanerva_print_the_same_lines - every program KANERVA runs prints the same under both; every
//!                                          parser error is the same error
//! the_comparison_can_fail                - the negative control: a changed read is seen as a difference
//! every_kanerva_keyword_is_on_its_family_page - the docs keyword check for the five KANERVA families
//! the_stand_in_refuses_exactly_kanervas_heads - SETTLE's feature-off heads are KANERVA's heads plus `memory`
//!
//! ** Technical Review **
//! - SETTLE builds without KANERVA, so it keeps its own lexer; KANERVA keeps its own so it needs no SETTLE. These
//!   tests are what keep the two one syntax: if a lexer or a helper drifts, a line here reds.
//! - The runner comparison takes every program KANERVA accepts and runs it through SETTLE's interpreter: the
//!   printed lines must be equal. Where KANERVA stops with an error, SETTLE must stop with the same message,
//!   except two refusals that are KANERVA's own by design: `via: :pulls` on an sdm (SETTLE's pulls) and a line
//!   that is not an sdm-family statement. Those are counted, and the counts are asserted, so a change that
//!   quietly turns comparisons into refusals reds too.
//! - The corpus holds 118 small programs: every error each statement can give, every option, clashes between
//!   families, and reads of every kind. It was written before the parser moved into KANERVA, and SETTLE's output
//!   on it was recorded then; the parser move changed exactly the `write-samples:` spelling.
//!
//! </claudes_code_comments>

// It compares SETTLE with KANERVA, so it needs KANERVA: built only with the `sdm` feature (the default).
#![cfg(feature = "sdm")]

use settle::interp::Interp;
use settle::lex;
use settle::plug::to_kanerva;
use std::path::{Path, PathBuf};

fn files_in(dir: &Path, ext: &str, out: &mut Vec<PathBuf>) {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()).filter(|p| p.extension().is_some_and(|x| x == ext)).collect();
    v.sort();
    out.extend(v);
}

fn settle_files() -> Vec<PathBuf> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut out = Vec::new();
    files_in(&root.join("examples"), "settle", &mut out);
    files_in(&root.join("docs/examples"), "settle", &mut out);
    files_in(&root.join("../kanerva/programs"), "kanerva", &mut out);
    out
}

fn corpus() -> Vec<String> {
    include_str!("oneparser_corpus.txt").split("\n# ---- next program\n").map(|s| s.to_string()).collect()
}

#[test]
fn the_two_lexers_read_every_line_alike() {
    let mut lines: Vec<String> = Vec::new();
    for f in settle_files() {
        lines.extend(std::fs::read_to_string(&f).unwrap().lines().map(String::from));
    }
    for p in corpus() {
        lines.extend(p.lines().map(String::from));
    }
    // lines that break the lexer, each in its own way
    for odd in ["s.write :n, \"open", "a ; b", "x: :", "k.fill 1_000", "by: -0.5e2", ": nope", "s.read read-address: :cat # a comment", "1.2.3", "-x"] {
        lines.push(odd.to_string());
    }
    let mut compared = 0;
    for (i, line) in lines.iter().enumerate() {
        let a = lex::lex(line, i + 1).map(|t| to_kanerva(&t)).map_err(|e| e.0);
        let b = kanerva::lang::lex(line, i + 1).map_err(|e| e.0);
        assert_eq!(a, b, "the lexers disagree on: {}", line);
        compared += 1;
    }
    assert!(compared > 2000, "only {} lines compared", compared);
}

#[test]
fn the_retired_words_are_one_list() {
    assert_eq!(lex::RETIRED, kanerva::words::RETIRED);
}

#[test]
fn the_helpers_agree() {
    let words = ["seed", "sed", "temprature", "temperature", "word-size", "word_size", "write-samples", "write_samples", "cue", "rian", "rain", "b", "a"];
    for a in words {
        for b in words {
            assert_eq!(lex::edit_distance(a, b), kanerva::lang::lex::edit_distance(a, b), "{} {}", a, b);
        }
        assert_eq!(lex::suggest(a, words), kanerva::lang::suggest(a, words), "{}", a);
    }
    let src = "model :m do\n  sdm :s, sed: 1   # a slip\nend";
    for msg in ["line 2: sdm does not take `sed:`; did you mean `seed:`?", "line 2: sdm :s is already declared", "line 2: oops", "line 9: past", "no line"] {
        assert_eq!(lex::locate(src, msg), kanerva::lang::locate(src, msg), "{}", msg);
    }
}

/// Both runs of one program: KANERVA's result, SETTLE's result.
fn both(src: &str) -> (Result<Vec<String>, String>, Result<Vec<String>, String>) {
    let k = kanerva::lang::run(src).map_err(|e| e.0);
    let s = Interp::default().exec(src).map_err(|e| e.0);
    (k, s)
}

fn kanervas_own_refusal(msg: &str) -> bool {
    msg.contains("so run this program with settle") || msg.contains("no statement kanerva knows")
}

#[test]
fn settle_and_kanerva_print_the_same_lines() {
    let (mut same_lines, mut same_errors, mut refused) = (0, 0, 0);
    let mut programs: Vec<(String, String)> = settle_files().iter().map(|f| (f.display().to_string(), std::fs::read_to_string(f).unwrap())).collect();
    programs.extend(corpus().into_iter().enumerate().map(|(i, p)| (format!("corpus program {}", i + 1), p)));
    for (name, src) in &programs {
        let k = kanerva::lang::run(src).map_err(|e| e.0);
        match k {
            Err(e) if kanervas_own_refusal(&e) => refused += 1,
            Err(e) => {
                let s = Interp::default().exec(src).map_err(|e| e.0);
                assert_eq!(Err(e), s, "{}: the two stop differently", name);
                same_errors += 1;
            }
            Ok(lines) => {
                let s = Interp::default().exec(src).map_err(|e| e.0);
                assert_eq!(Ok(lines), s, "{}: the two print differently", name);
                same_lines += 1;
            }
        }
    }
    // measured 2026-10-06 on 258 programs: 32 print the same lines, 88 stop with the same error, and 138 are
    // KANERVA's own refusals (mostly the docs examples of SETTLE's other families). The floors catch a change that
    // quietly turns comparisons into refusals.
    assert!(same_lines >= 32, "only {} programs compared by their lines", same_lines);
    assert!(same_errors >= 88, "only {} programs compared by their errors", same_errors);
    assert_eq!(same_lines + same_errors + refused, programs.len());
}

#[test]
fn the_comparison_can_fail() {
    let src = std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../kanerva/programs/sdm.kanerva")).unwrap();
    let (k, s) = both(&src);
    assert_eq!(k, s);
    // the same program with its first read pointed at another stored word: the comparison must see it
    let (k2, _) = both(&src.replacen("read-address: :cat", "read-address: :dog", 1));
    assert_ne!(k2, s);
}

/// The keyword half of tests/docs_complete.rs, for the families whose keywords now live in KANERVA's word list
/// rather than in `only(...)` calls in src/: every keyword a statement takes is written as `keyword:` on its
/// family's page.
#[test]
fn every_kanerva_keyword_is_on_its_family_page() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut checked = 0;
    let mut missing = Vec::new();
    for f in kanerva::lang::Family::ALL {
        let page = std::fs::read_to_string(root.join(format!("docs/05-statements/{}.md", f.ext_name()))).unwrap();
        for s in kanerva::words::STATEMENTS.iter().filter(|s| s.family == f.word()) {
            for w in s.words {
                checked += 1;
                if !page.contains(&format!("{}:", w.name)) {
                    missing.push(format!("{}.md lacks `{}:` ({} {})", f.ext_name(), w.name, s.family, s.verb));
                }
            }
        }
    }
    assert!(missing.is_empty(), "{}", missing.join("\n"));
    assert!(checked >= 40, "only {} keywords checked", checked);
}

/// With the feature off, SETTLE's stand-in refuses the lines that open the sdm family. Its head list lives on
/// SETTLE's words floor (it must compile with no KANERVA), so it is held here to KANERVA's: the same heads, plus
/// `memory`, which is SETTLE's own statement but needs KANERVA's keys.
#[test]
fn the_stand_in_refuses_exactly_kanervas_heads() {
    use kanerva::words::Place;
    use settle::words::vocab::{SDM_MODEL_HEADS, SDM_RUN_HEADS};
    let mut model: Vec<&str> = SDM_MODEL_HEADS.iter().copied().filter(|h| *h != "memory").collect();
    let mut run: Vec<&str> = SDM_RUN_HEADS.to_vec();
    let mut k_model: Vec<&str> = kanerva::lang::HEADS.iter().filter(|(p, _)| *p == Place::Model).map(|(_, h)| *h).collect();
    let mut k_run: Vec<&str> = kanerva::lang::HEADS.iter().filter(|(p, _)| *p == Place::Run).map(|(_, h)| *h).collect();
    for v in [&mut model, &mut run, &mut k_model, &mut k_run] {
        v.sort();
    }
    assert_eq!(model, k_model);
    assert_eq!(run, k_run);
    assert!(SDM_MODEL_HEADS.contains(&"memory"));
}
