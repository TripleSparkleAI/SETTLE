//! The five defaults of 2026-10-06 (lane NEWDEFAULTS, the navigator's ruling on DISCOVERIES.md section 3): a
//! program that names nothing gets the measured-better setting, and the old default is still there as an option.
//!
//! <claudes_code_comments>
//! ** Function List **
//! untimed(lines)                              - the printed lines with wall-clock timings replaced by `<time>`
//! scratch(tag)                                - a fresh scratch folder with an empty f/ inside
//! frames(dir, n)                              - write n small 24x16 PGM frames into dir
//! play_prog(extra)                            - a one-grid model and a `play` with extra keywords appended
//! run_in(dir, src)                            - run a program with its base directory set to dir
//! core_settle_defaults_to_metro               - `settle` and `anneal` with no `update:` equal `update: :metro`
//! play_defaults_to_checkerboard_metropolised  - `play` with no `update:` equals `update: :metro_checker`
//! correct_defaults_to_tap                     - `play` and `lean_from` with no `correct:` equal `correct: :tap`
//! warm_from_defaults_to_the_correction        - a warm fit with no `warm_from:` equals `warm_from: :correction`
//! factor_defaults_to_the_column_encoding      - `factor` with no `encoding:` equals `encoding: :columns`
//!
//! ** Technical Review **
//! - Each test runs one program three ways: with the keyword left out, with the new default named, and with the
//!   old default named. The first two must print the same lines (timings masked); the third must print something
//!   else, which proves the comparison can see a difference (the vacuity control).
//! - The printed run line names a rule only when it is not the default: `play` prints ", gibbs" and ", mean" for
//!   the old defaults and nothing for :metro_checker and :tap.
//! - Each test was red-proven against the old default by hand (see SETTLE/runs/newdefaults/REDPROOF.txt).
//! </claudes_code_comments>

use settle::grid::{write_pgm, Pgm};
use settle::interp::Interp;
use std::path::{Path, PathBuf};

/// The lines with every number that is a timing removed: anything followed by ` ms`, ` frames/s` or ` s fitting`.
fn untimed(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .map(|l| {
            let mut s = l.clone();
            for unit in [" ms", " frames/s", " s fitting"] {
                while let Some(p) = s.find(unit) {
                    let start = s[..p].rfind(' ').map(|i| i + 1).unwrap_or(0);
                    s.replace_range(start..p + unit.len(), "<time>");
                }
            }
            s
        })
        .collect()
}

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("settle-newdefaults-{}-{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("f")).unwrap();
    d
}

fn frames(dir: &Path, n: usize) {
    let (w, h) = (24, 16);
    for k in 0..n {
        let px = (0..w * h)
            .map(|i| {
                let (x, y) = ((i % w) as f64 / w as f64, (i / w) as f64 / h as f64);
                let disc = ((x - 0.35 - 0.03 * k as f64).powi(2) + (y - 0.5).powi(2)).sqrt() < 0.22;
                if disc {
                    0.12
                } else {
                    0.3 + 0.4 * x
                }
            })
            .collect();
        write_pgm(&dir.join(format!("f{}.pgm", k)), &Pgm { w, h, px }).unwrap();
    }
}

fn run_in(dir: &Path, src: &str) -> Vec<String> {
    let mut it = Interp::in_dir(dir.to_path_buf());
    untimed(&it.exec(src).unwrap_or_else(|e| panic!("{}\n{}", src, e)))
}

fn play_prog(extra: &str) -> String {
    format!("model :f do\n  grid :img, width: 24, height: 16, smooth: 0.2\nend\nrun :f do\n  play :img, frames: \"f/\", sweeps: 40, read: :soft, seed: 3{}\nend", extra)
}

#[test]
fn core_settle_defaults_to_metro() {
    let p = |verb: &str, extra: &str| {
        format!("model :w do\n  thing :rain, leans: :no, by: 1\n  thing :sprinkler, leans: :no, by: 0.5\n  thing :wet\n  rain.pushes :sprinkler, by: 0.5\n  rain.pulls :wet, by: 1.5\n  sprinkler.pulls :wet, by: 1\nend\nrun :w do\n  hold :wet, :yes\n  {} 2_000, seed: 1{}\n  {}\nend", verb, extra, if verb == "settle" { "ask :rain" } else { "best" })
    };
    for verb in ["settle", "anneal"] {
        let none = Interp::default().exec(&p(verb, "")).unwrap();
        let metro = Interp::default().exec(&p(verb, ", update: :metro")).unwrap();
        let gibbs = Interp::default().exec(&p(verb, ", update: :gibbs")).unwrap();
        assert_eq!(none, metro, "{}: no update: must equal update: :metro", verb);
        if verb == "settle" {
            assert_ne!(none, gibbs, "control: update: :gibbs must print something else");
        }
    }
}

#[test]
fn play_defaults_to_checkerboard_metropolised() {
    let d = scratch("update");
    frames(&d.join("f"), 2);
    let none = run_in(&d, &play_prog(""));
    let new = run_in(&d, &play_prog(", update: :metro_checker"));
    let old = run_in(&d, &play_prog(", update: :gibbs"));
    assert_eq!(none, new, "no update: must equal update: :metro_checker");
    assert_ne!(none, old, "control: update: :gibbs must play something else");
    let line = old.iter().find(|l| l.starts_with("play :img")).unwrap();
    assert!(line.contains(", gibbs:"), "the old default is named on the run line: {}", line);
    let line = none.iter().find(|l| l.starts_with("play :img")).unwrap();
    assert!(!line.contains("gibbs") && !line.contains("metro"), "the default is not named: {}", line);
}

#[test]
fn correct_defaults_to_tap() {
    let d = scratch("correct");
    frames(&d.join("f"), 2);
    let none = run_in(&d, &play_prog(""));
    let new = run_in(&d, &play_prog(", correct: :tap"));
    let old = run_in(&d, &play_prog(", correct: :mean"));
    assert_eq!(none, new, "no correct: must equal correct: :tap");
    assert_ne!(none, old, "control: correct: :mean must play something else");
    let line = old.iter().find(|l| l.starts_with("play :img")).unwrap();
    assert!(line.contains(", mean"), "the old default is named on the run line: {}", line);
    // lean_from takes the same default: its leans settle to the same rates as :tap's
    let lf = |extra: &str| {
        format!("model :f do\n  grid :img, width: 24, height: 16, smooth: 0.2\n  img.lean_from \"f/f0.pgm\"{}\nend\nrun :f do\n  settle 200, seed: 5\n  img.show_as \"out.pgm\"\nend", extra)
    };
    let read = |extra: &str| {
        run_in(&d, &lf(extra));
        std::fs::read(d.join("out.pgm")).unwrap()
    };
    let (a, b, c) = (read(""), read(", correct: :tap"), read(", correct: :mean"));
    assert_eq!(a, b, "lean_from with no correct: must equal correct: :tap");
    assert_ne!(a, c, "control: lean_from correct: :mean must settle to another picture");
}

#[test]
fn warm_from_defaults_to_the_correction() {
    let d = scratch("warm");
    frames(&d.join("f"), 3);
    let base = ", fit: 3, fit_sweeps: 40, warm_fit: 1, warm_fit_sweeps: 40";
    let none = run_in(&d, &play_prog(base));
    let new = run_in(&d, &play_prog(&format!("{}, warm_from: :correction", base)));
    let old = run_in(&d, &play_prog(&format!("{}, warm_from: :leans", base)));
    assert_eq!(none, new, "no warm_from: must equal warm_from: :correction");
    assert_ne!(none, old, "control: warm_from: :leans must fit something else");
    let line = none.iter().find(|l| l.starts_with("play :img")).unwrap();
    assert!(line.contains("from correction"), "the run line names where the warm fit starts: {}", line);
}

#[test]
fn factor_defaults_to_the_column_encoding() {
    let p = |extra: &str| format!("model :p do\n  factor :f, number: 143{}\nend\nrun :p do\n  anneal 2_000, seed: 4\n  f.solution\nend", extra);
    let none = Interp::default().exec(&p("")).unwrap();
    let new = Interp::default().exec(&p(", encoding: :columns")).unwrap();
    let old = Interp::default().exec(&p(", encoding: :rosenberg")).unwrap();
    assert_eq!(none, new, "no encoding: must equal encoding: :columns");
    assert_ne!(none, old, "control: encoding: :rosenberg must build another model");
    // the column encoding's range is the default's range now: above 1,000,000 needs no keyword
    assert!(Interp::default().exec("model :p do\n  factor :f, number: 1000003\nend").is_ok());
    let e = Interp::default().exec("model :p do\n  factor :f, number: 1000003, encoding: :rosenberg\nend").err().unwrap().0;
    assert!(e.contains("encoding: :rosenberg takes an odd whole number from 9 to 1,000,000"), "{}", e);
}
