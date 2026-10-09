//! THE TWO FACES ARE ONE LANGUAGE: the core family written as a program file and written with the Rust builder
//! (`Model::build()`) build the same model and print the same lines, and both are held to one word list
//! (`settle::words::vocab::CORE`).
//!
//! <claudes_code_comments>
//! ** Function List **
//! file(name)                    - a documented example run through the interpreter: its lines
//! file_model(name, model)       - the model a documented example declares
//! same_model(a, b)              - two models equal in names, leans and pulls
//! each documented core example  - rebuilt with the builder, line for line and model for model
//! the_documented_builder_prints_the_programs_lines - docs/examples/builder-alarm.rs, compiled as written, prints the alarm program's lines
//! every_word_is_a_builder_word  - the word list against the builder's method names, both directions
//! every_word_is_written_on_the_core_page - the word list against docs/05-statements/core.md
//!
//! ** Technical Review **
//! - The examples are the documented ones in `docs/examples/`, so the builder is held to the outputs a reader
//!   of the manual sees. The model check compares names, leans and every pull, so two faces that print the
//!   same lines by luck still fail if they built different models.
//! - The word-list checks read `src/words/builder.rs` as text: every word of `CORE` is a `pub fn` there or one
//!   of the parameter words (`by`, `yes`, `no`, `gibbs`, `metro`), and every `pub fn` there is a word of `CORE` or one of the
//!   named plumbing methods (`build`, `run`, `model`, `lines`, `statements`, `names`).
//!
//! </claudes_code_comments>

use settle::engine::model::Model;
use settle::interp::Interp;
use settle::words::builder::ModelBuilder;
use settle::words::vocab::{BY, CORE, GIBBS, METRO, NO, YES};
use std::path::{Path, PathBuf};

fn ex(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("docs").join("examples").join(format!("{}.settle", name))
}

fn file(name: &str) -> Vec<String> {
    let src = std::fs::read_to_string(ex(name)).unwrap();
    Interp::default().exec(&src).unwrap()
}

fn file_model(name: &str, model: &str) -> Model {
    let src = std::fs::read_to_string(ex(name)).unwrap();
    let mut it = Interp::default();
    it.exec(&src).unwrap();
    it.models[model].clone()
}

fn same_model(a: &Model, b: &Model) {
    assert_eq!(a.names, b.names);
    assert_eq!(a.h, b.h);
    for i in 0..a.len() {
        let mut x = a.adj[i].clone();
        let mut y = b.adj[i].clone();
        x.sort_by_key(|p| p.0);
        y.sort_by_key(|p| p.0);
        assert_eq!(x, y, "the pulls of {}", a.names[i]);
    }
}

fn both(name: &str, model: &str, built: &ModelBuilder, runs: &[Vec<String>]) {
    same_model(&built.model().unwrap(), &file_model(name, model));
    let joined: Vec<String> = runs.iter().flatten().cloned().collect();
    assert_eq!(joined, file(name), "{}: the builder prints differently", name);
}

#[test]
fn core_weather() {
    let m = Model::build()
        .thing("rain")
        .leans("no", 1.0)
        .thing("sprinkler")
        .leans("no", 0.5)
        .thing("wet_grass")
        .pushes("rain", "sprinkler", 0.5)
        .pulls("rain", "wet_grass", 1.5)
        .pulls("sprinkler", "wet_grass", 1.0);
    let lines = m
        .clone()
        .run()
        .hold("wet_grass", "yes")
        .settle(20_000)
        .temperature(1.0)
        .seed(1)
        .show()
        .ask("rain")
        .ask("rain")
        .and("sprinkler")
        .ask("rain")
        .or("sprinkler")
        .ask("sprinkler")
        .and_not("rain")
        .lines()
        .unwrap();
    both("core-weather", "weather", &m, &[lines]);
}

#[test]
fn core_update() {
    let m = Model::build()
        .thing("rain")
        .leans("no", 1.0)
        .thing("sprinkler")
        .leans("no", 0.5)
        .thing("wet_grass")
        .pushes("rain", "sprinkler", 0.5)
        .pulls("rain", "wet_grass", 1.5)
        .pulls("sprinkler", "wet_grass", 1.0);
    // the first settle names no rule and gets Metropolised Gibbs, the default since 2026-10-06; the second asks for Gibbs
    let lines = m.clone().run().hold("wet_grass", "yes").settle(20_000).seed(1).ask("rain").settle(20_000).seed(1).update("gibbs").ask("rain").lines().unwrap();
    both("core-update", "weather", &m, &[lines]);
    // the control: the same chain left on the default prints a different second answer, so the rule really reached the run
    let metro = m.run().hold("wet_grass", "yes").settle(20_000).seed(1).ask("rain").settle(20_000).seed(1).update("metro").ask("rain").lines().unwrap();
    assert_ne!(metro[3], file("core-update")[3]);
}

#[test]
fn core_leans_and_pulls() {
    let m = Model::build()
        .thing(["a", "b", "c"])
        .leans("yes", 0.5)
        .thing("a")
        .leans("no", 1.0)
        .pulls("a", "b", 2.0)
        .pushes("b", "a", 1.5)
        .pushes("b", "c", 1.0);
    let lines = m.clone().run().settle(20_000).seed(2).show().lines().unwrap();
    both("core-leans-and-pulls", "m", &m, &[lines]);
}

#[test]
fn core_ask() {
    let m = Model::build().thing("a").leans("yes", 0.5).thing("b").leans("no", 0.5).thing("c").pulls("a", "c", 1.0);
    let lines = m
        .clone()
        .run()
        .settle(5_000)
        .seed(3)
        .ask("a")
        .ask("a")
        .and("b")
        .ask("a")
        .or("b")
        .ask("a")
        .and_not("b")
        .ask("a")
        .or_not("b")
        .ask("a")
        .and("c")
        .or("b")
        .ask("a")
        .and("b")
        .and("c")
        .lines()
        .unwrap();
    both("core-ask", "m", &m, &[lines]);
}

#[test]
fn core_seed_two_runs() {
    let m = Model::build().thing(["a", "b"]).pulls("a", "b", 1.0);
    let first = m
        .clone()
        .run()
        .settle(1_000)
        .ask("a")
        .and("b")
        .settle(1_000)
        .ask("a")
        .and("b")
        .settle(1_000)
        .seed(24301)
        .ask("a")
        .and("b")
        .settle(1_000)
        .temperature(3.0)
        .ask("a")
        .and("b")
        .settle(1_000)
        .lines()
        .unwrap();
    let second = m.clone().run().settle(1_000).ask("a").and("b").lines().unwrap();
    both("core-seed", "pair", &m, &[first, second]);
}

#[test]
fn core_anneal_and_tour_anneal() {
    let ring = Model::build()
        .thing(["a", "b", "c", "d", "e"])
        .thing("b")
        .leans("yes", 0.2)
        .pushes("a", "b", 1.0)
        .pushes("b", "c", 1.0)
        .pushes("c", "d", 1.0)
        .pushes("d", "e", 1.0)
        .pushes("e", "a", 1.0)
        .pulls("a", "c", 0.3);
    let lines = ring.clone().run().anneal(4_000).seed(5).best().lines().unwrap();
    both("core-anneal", "ring", &ring, &[lines]);

    let tri = Model::build()
        .thing(["x", "y", "z"])
        .pushes("x", "y", 1.0)
        .pushes("y", "z", 1.0)
        .pushes("z", "x", 1.0)
        .thing("x")
        .leans("yes", 0.1);
    let lines = tri.clone().run().anneal(2_000).seed(3).best().lines().unwrap();
    both("tour-anneal", "triangle", &tri, &[lines]);
}

#[test]
fn tour_hold_and_explaining_away() {
    let pair = Model::build().thing("a").thing("b").pulls("a", "b", 1.0);
    let lines = pair.clone().run().hold("a", "no").settle(20_000).seed(1).show().lines().unwrap();
    both("tour-hold", "pair", &pair, &[lines]);

    let alarm = Model::build()
        .thing("burglary")
        .leans("no", 1.0)
        .thing("earthquake")
        .leans("no", 1.0)
        .thing("alarm")
        .leans("no", 1.0)
        .pulls("burglary", "alarm", 1.5)
        .pulls("earthquake", "alarm", 1.5)
        .pushes("burglary", "earthquake", 1.0);
    let lines = alarm.clone().run().hold("alarm", "yes").hold("earthquake", "yes").settle(40_000).seed(1).ask("burglary").lines().unwrap();
    both("cook-explaining-away-short", "alarm", &alarm, &[lines]);
}

// the builder program the docs show (07-extending.md and #/settle, lane SETTLEPAGE), included as written, so the page's
// Rust is compiled and run here and prints exactly the alarm program's recorded lines
mod doc_builder {
    include!("../docs/examples/builder-alarm.rs");
    pub fn lines() -> Vec<String> {
        alarm().unwrap()
    }
}

#[test]
fn the_documented_builder_prints_the_programs_lines() {
    assert_eq!(doc_builder::lines(), file("cook-explaining-away-short"));
}

#[test]
fn a_different_build_is_caught() {
    // the control: one pull changed, and the comparison sees it
    let m = Model::build().thing("a").leans("yes", 0.5).thing("b").pulls("a", "b", 1.5);
    let built = m.model().unwrap();
    let r = std::panic::catch_unwind(|| same_model(&built, &file_model("tour-first", "pair")));
    assert!(r.is_err(), "a model with a different pull compared equal");
}

fn builder_source() -> String {
    std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join("words").join("builder.rs")).unwrap()
}

fn builder_methods() -> Vec<String> {
    builder_source()
        .lines()
        .filter_map(|l| l.trim_start().strip_prefix("pub fn "))
        .map(|rest| rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect())
        .collect()
}

#[test]
fn every_word_is_a_builder_word() {
    let methods = builder_methods();
    let params = [BY, YES, NO, GIBBS, METRO];
    for w in CORE {
        assert!(methods.iter().any(|m| m == w) || params.contains(w), "`{}` is in the word list but not in the builder", w);
    }
    let plumbing = ["build", "run", "model", "lines", "statements", "names"];
    for m in &methods {
        assert!(CORE.contains(&m.as_str()) || plumbing.contains(&m.as_str()), "the builder method `{}` is not in the word list", m);
    }
}

#[test]
fn every_word_is_written_on_the_core_page() {
    let page = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("docs").join("05-statements").join("core.md")).unwrap();
    for w in CORE {
        assert!(page.contains(w), "`{}` is not on docs/05-statements/core.md", w);
    }
}
