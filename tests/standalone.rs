//! SETTLE STANDS ALONE: built with `--no-default-features` (no `sdm` feature, so no KANERVA), the `settle` command
//! prints exactly what the full build prints for every program that uses no sdm-family statement, and refuses
//! every program that does, naming the feature.
//!
//! <claudes_code_comments>
//! ** Function List **
//! standalone_binary()   - build `settle` with --no-default-features into its own target dir; its path
//! uses_sdm(src)         - does a program open an sdm-family statement (`vocab::SDM_*_HEADS`, what `sdmoff` claims)
//! mask(s)               - wall-clock timings replaced with `<time>`, as docs/examples/run.sh does
//! scratch_copy(dir)     - the programs and their data in a scratch folder, so written files stay out of the repo
//! run(bin, dir, file)   - one program run by one binary: (exit status, stdout, stderr)
//! the_standalone_build_prints_what_the_full_build_prints - the test
//!
//! ** Technical Review **
//! - The test runs under the default features and builds the second binary itself with `cargo build --release
//!   --no-default-features --target-dir target/standalone` (a separate target dir, so it cannot wait on this
//!   test's own build lock). Cargo's own `CARGO` variable names the cargo to run.
//! - Every program in `docs/examples/` and `examples/` is run by both binaries, each in its own scratch copy.
//!   A program with no sdm-family line must give the same exit status, the same stdout and the same error
//!   message and caret (`head`), timings masked; only the list of known statements after an error may differ. A program with one must exit 2 under the standalone build with a message naming
//!   `--features sdm`.
//! - A control: the count of programs in each class is printed and both classes must be non-empty, so the test
//!   cannot pass by skipping everything.
//!
//! </claudes_code_comments>

#![cfg(feature = "sdm")]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn manifest() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn standalone_binary() -> PathBuf {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let target = manifest().join("target").join("standalone");
    let st = Command::new(cargo)
        .args(["build", "--release", "--no-default-features", "--bin", "settle", "--target-dir"])
        .arg(&target)
        .current_dir(manifest())
        .status()
        .expect("run cargo");
    assert!(st.success(), "the standalone build failed");
    target.join("release").join("settle")
}

use settle::words::vocab::{SDM_MODEL_HEADS as MODEL_HEADS, SDM_RUN_HEADS as RUN_HEADS};

fn uses_sdm(src: &str) -> bool {
    src.lines().any(|l| {
        let first = l.split('#').next().unwrap_or("").split_whitespace().next().unwrap_or("");
        MODEL_HEADS.contains(&first) || RUN_HEADS.contains(&first)
    })
}

fn mask(s: &str) -> String {
    let mut out = String::new();
    for line in s.lines() {
        let b = line.as_bytes();
        let mut i = 0;
        let mut o = String::new();
        while i < b.len() {
            if b[i].is_ascii_digit() && (i == 0 || !(b[i - 1].is_ascii_digit() || b[i - 1] == b'.')) {
                let st = i;
                while i < b.len() && b[i].is_ascii_digit() {
                    i += 1;
                }
                if i + 1 < b.len() && b[i] == b'.' && b[i + 1].is_ascii_digit() {
                    i += 1;
                    while i < b.len() && b[i].is_ascii_digit() {
                        i += 1;
                    }
                    let rest = &line[i..];
                    if rest.starts_with(" ms") || rest.starts_with(" frames/s") || rest.starts_with(" s fitting") || rest == "s" || rest.starts_with("s;") {
                        o.push_str("<time>");
                        continue;
                    }
                }
                o.push_str(&line[st..i]);
                continue;
            }
            let ch = line[i..].chars().next().unwrap();
            o.push(ch);
            i += ch.len_utf8();
        }
        out.push_str(&o);
        out.push('\n');
    }
    out
}

fn copy_dir(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).unwrap();
    for e in fs::read_dir(src).unwrap() {
        let p = e.unwrap().path();
        let to = dst.join(p.file_name().unwrap());
        if p.is_dir() {
            copy_dir(&p, &to);
        } else {
            fs::copy(&p, &to).unwrap();
        }
    }
}

fn scratch_copy(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("settle-standalone-{}-{}", std::process::id(), tag));
    let _ = fs::remove_dir_all(&root);
    copy_dir(&manifest().join("docs").join("examples"), &root.join("docs").join("examples"));
    copy_dir(&manifest().join("examples"), &root.join("examples"));
    root
}

fn run(bin: &Path, root: &Path, rel: &str) -> (i32, String, String) {
    let p = Path::new(rel);
    let o = Command::new(bin).arg(p.file_name().unwrap()).current_dir(root.join(p.parent().unwrap())).output().expect("run settle");
    (o.status.code().unwrap_or(-1), mask(&String::from_utf8_lossy(&o.stdout)), mask(&String::from_utf8_lossy(&o.stderr)))
}

/// An error as printed, without the list of known statements that may follow it: the first line and the
/// program line with its caret (the lines with a ` | ` gutter).
fn head(err: &str) -> String {
    let gutter = |l: &str| l.trim_start().trim_start_matches(|c: char| c.is_ascii_digit()).starts_with(" | ") || l.trim_start().starts_with("| ");
    err.lines().enumerate().filter(|(i, l)| *i == 0 || gutter(l)).map(|(_, l)| format!("{}\n", l)).collect()
}

#[test]
fn the_standalone_build_prints_what_the_full_build_prints() {
    let full = PathBuf::from(env!("CARGO_BIN_EXE_settle"));
    let alone = standalone_binary();
    let (a, b) = (scratch_copy("full"), scratch_copy("alone"));
    let mut programs: Vec<String> = Vec::new();
    for dir in ["docs/examples", "examples"] {
        let mut names: Vec<String> =
            fs::read_dir(manifest().join(dir)).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).filter(|n| n.ends_with(".settle")).collect();
        names.sort();
        programs.extend(names.into_iter().map(|n| format!("{}/{}", dir, n)));
    }
    let (mut same, mut refused) = (0, 0);
    for rel in &programs {
        let src = fs::read_to_string(manifest().join(rel)).unwrap();
        if uses_sdm(&src) {
            let (code, out, err) = run(&alone, &b, rel);
            assert_eq!(code, 2, "{}: the standalone build should refuse it", rel);
            assert!(out.is_empty(), "{}: printed output before refusing", rel);
            assert!(err.contains("--features sdm"), "{}: the refusal does not name the feature: {}", rel, err);
            refused += 1;
        } else {
            let want = run(&full, &a, rel);
            let got = run(&alone, &b, rel);
            assert_eq!((got.0, &got.1), (want.0, &want.1), "{}: the standalone build prints differently", rel);
            // an error's message and its caret must match; the list of known statements after them names what
            // each build knows, so it rightly differs
            assert_eq!(head(&got.2), head(&want.2), "{}: the standalone build's error differs", rel);
            same += 1;
        }
    }
    eprintln!("standalone: {} programs identical, {} sdm-family programs refused by name", same, refused);
    assert!(same > 50 && refused > 10, "too few programs in a class: {} identical, {} refused", same, refused);
    let _ = fs::remove_dir_all(&a);
    let _ = fs::remove_dir_all(&b);
}
