//! No program or source in the repository uses a keyword retired for Kanerva's terms.
//!
//! <claudes_code_comments>
//! ** Function List **
//! retired_uses(text)        - every (line, retired keyword) a SETTLE line in `text` uses
//! uses_on_line(line)        - the retired keywords one line uses as keywords
//! keyword_at(line, word)    - true when `word:` appears there as a keyword with a SETTLE value after it
//! sdm_line(line)            - true when the line carries an SDM statement in SETTLE form (`sdm :s`)
//! walk(dir, out)            - every scanned file under a folder, skipping build output and dependencies
//! history(path)             - true for append-only records, which keep the words they were written with
//! the_scanner_sees_a_retired_keyword - the positive control
//! no_file_uses_a_retired_keyword     - the scan
//!
//! ** Technical Review **
//! - The retired words are settle-rs's `lex::RETIRED` (SETTLE/kanerva/KANERVA_TERMS.md). `cue`,
//!   `damage`, `iterations` and `tolerate` are retired everywhere; `locations`, `radius`, `fire` and `size` only on
//!   the SDM statements (`sdm`, `sdmscale`, `softsdm`, `contenttrack`, `refusal`), because the Hopfield `memory`,
//!   `sudoku` and `landscape` keep `size:`. A line counts as an SDM line only when it carries an SDM statement in
//!   SETTLE's own form, the statement word then a space then a symbol (`sdm :s, word-size: 64`); a JavaScript
//!   variable named `sdm` (`const sdm = makeSdm(7, { radius: 112 })`) is not one.
//! - `cue:` named the read-address, which takes only a symbol (`read-address: :cat`), so it counts only with a
//!   symbol after it; a JavaScript option `cue: 0.5` is not a use.
//! - A use is `word:` with no letter, digit, `_` or `-` before it, then spaces, then a SETTLE value: a symbol, a
//!   number or a minus sign. So `read-address:` and a Rust type annotation `cue: Vec<f64>` are not uses.
//! - The scan covers the SETTLE programs and every source that carries one: settle-rs and kanerva (.settle, .rs,
//!   docs .md), the site (src, tests, tools) and BRAND (.settle, .js, .html, .py, .md). Append-only records (the
//!   channel, the campaign, lane reports, changelogs, the term table itself) keep their words and are skipped.
//! - A program quoted in a string keeps its line breaks as a literal backslash-n, so each piece is a line.
//! - A line that carries the words `not a SETTLE line` (a JavaScript option object, say) is not a use.
//! - The scan must read at least 100 files and 40 SETTLE programs, so a wrong root cannot pass by finding nothing.
//! - Standalone: in SETTLE's own repository (no research repository around it) the scan covers this crate alone.
//!
//! </claudes_code_comments>

use settle::lex::RETIRED;
use std::fs;
use std::path::{Path, PathBuf};

const SDM_STATEMENTS: &[&str] = &["sdm", "sdmscale", "softsdm", "contenttrack", "refusal"];
const SDM_ONLY: &[&str] = &["locations", "radius", "fire", "size"];

fn keyword_at(line: &str, word: &str) -> bool {
    let b = line.as_bytes();
    let pat = format!("{}:", word);
    let mut from = 0;
    while let Some(i) = line[from..].find(&pat) {
        let at = from + i;
        let before_ok = at == 0 || !(b[at - 1].is_ascii_alphanumeric() || b[at - 1] == b'_' || b[at - 1] == b'-');
        let mut j = at + pat.len();
        let spaced = j < b.len() && b[j] == b' ';
        while j < b.len() && b[j] == b' ' {
            j += 1;
        }
        // `cue:` named the read-address, and a read-address is always a symbol
        let value_ok = j < b.len()
            && if word == "cue" { b[j] == b':' } else { b[j] == b':' || b[j] == b'-' || b[j] == b'.' || b[j].is_ascii_digit() };
        if before_ok && spaced && value_ok {
            return true;
        }
        from = at + pat.len();
    }
    false
}

fn sdm_line(line: &str) -> bool {
    // the statement word, standing alone, then ` :` and a letter: `sdm :s`, `softsdm :m`
    let b = line.as_bytes();
    SDM_STATEMENTS.iter().any(|st| {
        let pat = format!("{} :", st);
        line.match_indices(&pat).any(|(at, _)| {
            let before_ok = at == 0 || !(b[at - 1].is_ascii_alphanumeric() || b[at - 1] == b'_' || b[at - 1] == b'-');
            let after = at + pat.len();
            before_ok && after < b.len() && b[after].is_ascii_alphabetic()
        })
    })
}

fn uses_on_line(line: &str) -> Vec<&'static str> {
    if line.contains("not a SETTLE line") {
        return Vec::new();
    }
    RETIRED
        .iter()
        .map(|(old, _, _)| *old)
        .filter(|old| (!SDM_ONLY.contains(old) || sdm_line(line)) && keyword_at(line, old))
        .collect()
}

fn retired_uses(text: &str) -> Vec<(usize, &'static str)> {
    // a program quoted inside a string carries its line breaks as a literal backslash-n: each piece is its own line
    text.lines()
        .enumerate()
        .flat_map(|(n, l)| l.split("\\n").flat_map(uses_on_line).map(move |w| (n + 1, w)).collect::<Vec<_>>())
        .collect()
}

fn history(p: &Path) -> bool {
    let s = p.to_string_lossy();
    let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    name.starts_with("SETTLE_CHANNEL")
        || name.starts_with("SETTLE_CAMPAIGN")
        || name.starts_with("HERMES_TAKEOVER")
        || name.starts_with("REPORT_")
        || name.starts_with("LANE_")
        || name == "CHANGELOG.md"
        || name == "KANERVA_TERMS.md"
        || s.contains("/runs/")
        // the retired table itself, and the tests that plant a retired keyword on purpose
        || s.ends_with("src/words/lex.rs")
        || s.ends_with("tests/kanerva_terms.rs")
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().to_string();
        if p.is_dir() {
            if !["target", "node_modules", "dist", ".git", "public"].contains(&name.as_str()) {
                walk(&p, out);
            }
        } else if ["settle", "rs", "md", "js", "jsx", "mjs", "html", "py"].iter().any(|x| name.ends_with(&format!(".{}", x))) {
            out.push(p);
        }
    }
}

#[test]
fn the_scanner_sees_a_retired_keyword() {
    let old = concat!("  s.read cu", "e: :cat, dam", "age: 0.2, seed: 1\n  sdm :s, si", "ze: 64, loca", "tions: 10\n");
    let got: Vec<&str> = retired_uses(old).into_iter().map(|(_, w)| w).collect();
    assert_eq!(got, vec!["cue", "damage", "locations", "size"]);
    // the new words, the Hopfield memory's size:, and a Rust annotation are not uses
    let new = "  s.read read-address: :cat, address-noise: 0.2\n  memory :m, size: 64\n  let cue: Vec<f64> = x;\n  sdm :s, word-size: 64, hard-locations: 10\n";
    assert!(retired_uses(new).is_empty(), "{:?}", retired_uses(new));
    // a one-line blob holding two programs: the memory's size: is fine, the sdm's is not
    let blob = concat!(r#""memory :m, size: 64\nend", "sdm :s, si"#, r#"ze: 64\nend""#);
    assert_eq!(retired_uses(blob), vec![(1, "size")]);
    // JavaScript option objects in the site's hero art: a variable named sdm, a numeric cue, a result field
    let js = concat!(
        "  const sdm = makeSdm(5503, { m: 2000, radi", "us: 112 });\n",
        "export const PMEM = { cols: 10, rows: 5, cu", "e: 0.5, frames: 16 };\n",
        "  return { P, woken: sdm.woken(P.at(-1)).length, locati", "ons: 1500 };\n",
    );
    assert!(retired_uses(js).is_empty(), "{:?}", retired_uses(js));
}

#[test]
fn no_file_uses_a_retired_keyword() {
    let here = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mono = here.join("../..");
    // inside the research repository the scan covers every package; in SETTLE's own repository (a clone of
    // github.com/triplesparkle/SETTLE, where this crate is the root) it covers this crate alone
    let (root, dirs): (PathBuf, Vec<&str>) = if mono.join("SETTLE/settle-rs").is_dir() {
        (mono, vec!["SETTLE/settle-rs", "SETTLE/kanerva", "SETTLE/settle-mcp", "SETTLE/settle-site/src", "SETTLE/settle-site/tests", "SETTLE/settle-site/tools", "BRAND"])
    } else {
        (here.to_path_buf(), vec!["."])
    };
    let mut files = Vec::new();
    for d in dirs {
        walk(&root.join(d), &mut files);
    }
    let programs = files.iter().filter(|p| p.extension().map(|e| e == "settle").unwrap_or(false)).count();
    assert!(files.len() >= 100 && programs >= 40, "the scan read {} files and {} programs", files.len(), programs);
    let mut bad = Vec::new();
    for p in files.iter().filter(|p| !history(p)) {
        let Ok(text) = fs::read_to_string(p) else { continue };
        for (n, w) in retired_uses(&text) {
            bad.push(format!("{}:{}: `{}:`", p.strip_prefix(&root).unwrap_or(p).display(), n, w));
        }
    }
    assert!(bad.is_empty(), "retired keywords still in use ({}):\n{}", bad.len(), bad.join("\n"));
}
