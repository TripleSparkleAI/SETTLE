//! The `--json` answers the docs show are what `settle --json` prints.
//!
//! <claudes_code_comments>
//! ** Function List **
//! json_examples()                       - every docs/examples/NAME.json, with its NAME.settle beside it
//! every_documented_json_answer_matches  - runs the release command on each and compares, byte for byte
//! a_changed_answer_is_caught            - the control: one changed character, and the comparison sees it
//!
//! ** Technical Review **
//! - `docs/examples/NAME.json` holds the exact line `settle --json NAME.settle` prints, run from inside
//!   docs/examples, which is how the docs tell a reader to run it. The Markdown shows the file through a
//!   ```text file=NAME.json fence, and tests/docs_examples.rs holds that fence equal to the file; this test holds
//!   the file equal to the command. So a JSON answer on a page is the command's answer, never a typed one.
//! - It runs the real binary (`CARGO_BIN_EXE_settle`), so the argument handling, the JSON door and the exit status
//!   are all exercised: status 0 for `"ok": true`, status 2 for `"ok": false`, as 01-install-and-run.md says.
//! - The examples used this way write no files, so the command runs in place.
//! - `SETTLE_DOCS_BLESS=1 cargo test --release --test docs_json` rewrites each NAME.json from the command.
//! </claudes_code_comments>

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn examples() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("docs").join("examples")
}

fn json_examples() -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(examples())
        .unwrap()
        .filter_map(|e| {
            let p = e.unwrap().path();
            (p.extension().map(|x| x == "json").unwrap_or(false)).then(|| p.file_stem().unwrap().to_string_lossy().into_owned())
        })
        .collect();
    names.sort();
    names
}

fn run_json(name: &str) -> (String, i32) {
    let out = Command::new(env!("CARGO_BIN_EXE_settle"))
        .args(["--json", &format!("{}.settle", name)])
        .current_dir(examples())
        .output()
        .unwrap();
    (String::from_utf8(out.stdout).unwrap().trim_end().to_string(), out.status.code().unwrap_or(-1))
}

#[test]
fn every_documented_json_answer_matches() {
    let names = json_examples();
    assert!(names.len() >= 2, "the docs show a success and an error, found {:?}", names);
    let bless = std::env::var("SETTLE_DOCS_BLESS").map(|v| v == "1").unwrap_or(false);
    for n in &names {
        assert!(examples().join(format!("{}.settle", n)).exists(), "{}.json has no {}.settle beside it", n, n);
        let (got, status) = run_json(n);
        let path = examples().join(format!("{}.json", n));
        if bless {
            fs::write(&path, format!("{}\n", got)).unwrap();
            continue;
        }
        let want = fs::read_to_string(&path).unwrap().trim_end().to_string();
        assert_eq!(got, want, "{}: settle --json prints something else", n);
        let ok = want.contains("\"ok\": true");
        assert_eq!(status, if ok { 0 } else { 2 }, "{}: exit status", n);
    }
    // one success and one error, so the page shows both shapes
    let texts: Vec<String> = names.iter().map(|n| fs::read_to_string(examples().join(format!("{}.json", n))).unwrap()).collect();
    assert!(texts.iter().any(|t| t.contains("\"ok\": true")), "a success");
    assert!(texts.iter().any(|t| t.contains("\"ok\": false") && t.contains("\"column\"")), "an error with its column");
}

#[test]
fn a_changed_answer_is_caught() {
    let n = json_examples().into_iter().next().unwrap();
    let (got, _) = run_json(&n);
    let want = fs::read_to_string(examples().join(format!("{}.json", n))).unwrap();
    let changed = want.replacen("settle", "sett1e", 1);
    assert_ne!(got, changed.trim_end(), "a one-character change compared equal");
}
