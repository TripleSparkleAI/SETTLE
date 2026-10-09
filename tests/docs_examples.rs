//! Every example in the SETTLE docs runs, and prints what the docs say it prints.
//!
//! <claudes_code_comments>
//! ** Function List **
//! mask_times(s)            - replace wall-clock timings in output with `<time>`
//! md_files(dir, out)       - every Markdown file under docs/, recursively
//! fences(md)               - the fenced code blocks of one Markdown file, with their info strings
//! rewrite_fences(md, ex)   - bless mode: refresh output/error/file fences from the files
//! the_extension_example_runs - the example family of 07-extending.md compiles and runs
//! copy_dir(src, dst)       - copy docs/examples (and its data/) into a scratch folder
//! run_example(dir, name)   - run one example with the interpreter, in the scratch folder
//! every_docs_example_runs_and_matches  - the test
//!
//! ** Technical Review **
//! - The examples live in `docs/examples/<name>.settle`. Beside each is `<name>.out` (the exact printed lines)
//!   or `<name>.err` (the exact error message, for the examples that show an error).
//! - Each example runs in a scratch copy of docs/examples, with the process working directory set to that copy
//!   and the program's base directory empty. That is exactly what `cd docs/examples && settle <name>.settle`
//!   does, so files an example writes land in the scratch copy, never in the repository, and paths print the
//!   same way in both.
//! - Timings (`12.34 ms`, `5.0 frames/s`, `0.52 s fitting`, `0.52s;`, a trailing `0.52s`) change from run to run, so
//!   both this test and docs/examples/run.sh replace the number with `<time>`.
//! - The Markdown is checked too: a fence opened with ```settle example=NAME must hold exactly
//!   examples/NAME.settle, ```text output=NAME exactly NAME.out and ```text error=NAME exactly NAME.err, and
//!   every example file must be shown by at least one fence. So the docs cannot drift from what the
//!   interpreter prints.
//! - `SETTLE_DOCS_BLESS=1 cargo test --release --test docs_examples` rewrites the .out and .err files from the
//!   current interpreter instead of comparing, then rewrites every output=, error= and file= fence in the
//!   Markdown from those files. It never rewrites an example= fence: the program text is edited by hand.
//! - One test function, so changing the working directory cannot race another test in this binary.
//!
//! </claudes_code_comments>

use settle::interp::Interp;

#[path = "../docs/examples/ext_chain.rs"]
mod ext_chain;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

fn is_digit(c: u8) -> bool {
    c.is_ascii_digit()
}

/// Replace a decimal number followed by ` ms`, ` frames/s`, ` s fitting`, `s;`, or by `s` at the end of the line,
/// with `<time>`. Must agree with the sed expression in docs/examples/run.sh.
fn mask_times(line: &str) -> String {
    let b = line.as_bytes();
    let mut out = String::new();
    let mut i = 0;
    while i < b.len() {
        if is_digit(b[i]) && (i == 0 || !(is_digit(b[i - 1]) || b[i - 1] == b'.')) {
            let s = i;
            while i < b.len() && is_digit(b[i]) {
                i += 1;
            }
            if i + 1 < b.len() && b[i] == b'.' && is_digit(b[i + 1]) {
                i += 1;
                while i < b.len() && is_digit(b[i]) {
                    i += 1;
                }
                let rest = &line[i..];
                if rest.starts_with(" ms") || rest.starts_with(" frames/s") || rest.starts_with(" s fitting") || rest == "s" || rest.starts_with("s;") {
                    out.push_str("<time>");
                    continue;
                }
            }
            out.push_str(&line[s..i]);
            continue;
        }
        let ch = line[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn md_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<_> = fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            md_files(&p, out);
        } else if p.extension().map(|e| e == "md").unwrap_or(false) {
            out.push(p);
        }
    }
}

/// (info string, body) of every fenced block opened with three backticks.
fn fences(md: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut cur: Option<(String, Vec<&str>)> = None;
    for line in md.lines() {
        match &mut cur {
            None => {
                if let Some(info) = line.strip_prefix("```") {
                    cur = Some((info.trim().to_string(), Vec::new()));
                }
            }
            Some((info, body)) => {
                if line.trim_end() == "```" {
                    out.push((info.clone(), body.join("\n")));
                    cur = None;
                } else {
                    body.push(line);
                }
            }
        }
    }
    out
}

/// Bless mode only: rewrite the body of every output=, error= and file= fence from its file on disk.
fn rewrite_fences(md: &str, ex: &Path) -> String {
    let mut out = Vec::new();
    let mut lines = md.lines();
    while let Some(line) = lines.next() {
        out.push(line.to_string());
        let Some(info) = line.strip_prefix("```") else { continue };
        let words: Vec<&str> = info.split_whitespace().collect();
        let target = words.iter().find_map(|w| {
            w.strip_prefix("output=")
                .map(|n| format!("{}.out", n))
                .or_else(|| w.strip_prefix("error=").map(|n| format!("{}.err", n)))
                .or_else(|| w.strip_prefix("file=").map(|n| n.to_string()))
        });
        let mut body = Vec::new();
        for l in lines.by_ref() {
            if l.trim_end() == "```" {
                break;
            }
            body.push(l.to_string());
        }
        match target.and_then(|t| read_trim(&ex.join(t))) {
            Some(fresh) => out.extend(fresh.lines().map(|l| l.to_string())),
            None => out.extend(body),
        }
        out.push("```".to_string());
    }
    let mut s = out.join("\n");
    if md.ends_with('\n') {
        s.push('\n');
    }
    s
}

fn copy_dir(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).unwrap();
    for e in fs::read_dir(src).unwrap() {
        let p = e.unwrap().path();
        let q = dst.join(p.file_name().unwrap());
        if p.is_dir() {
            copy_dir(&p, &q);
        } else {
            fs::copy(&p, &q).unwrap();
        }
    }
}

/// Ok(printed lines) or Err(error message), timings masked.
fn run_example(dir: &Path, name: &str) -> Result<String, String> {
    let src = fs::read_to_string(dir.join(format!("{}.settle", name))).unwrap();
    let mut it = Interp::in_dir(PathBuf::new());
    match it.exec(&src) {
        Ok(lines) => Ok(lines.iter().map(|l| mask_times(l)).collect::<Vec<_>>().join("\n")),
        Err(e) => Err(mask_times(&e.0)),
    }
}

fn read_trim(p: &Path) -> Option<String> {
    fs::read_to_string(p).ok().map(|s| s.trim_end_matches('\n').to_string())
}

/// The example family in docs/07-extending.md compiles, registers, and prints what the page says.
#[test]
fn the_extension_example_runs() {
    let ex = Path::new(env!("CARGO_MANIFEST_DIR")).join("docs").join("examples");
    let src = fs::read_to_string(ex.join("ext_chain.program")).unwrap();
    let mut it = Interp::default();
    it.exts.push(Box::new(ext_chain::Chain));
    let out = it.exec(&src).map(|l| l.join("\n")).unwrap_or_else(|e| e.0);
    let out_p = ex.join("ext_chain.program.out");
    if std::env::var("SETTLE_DOCS_BLESS").map(|v| v == "1").unwrap_or(false) {
        fs::write(&out_p, format!("{}\n", out)).unwrap();
    } else {
        assert_eq!(read_trim(&out_p).unwrap_or_default(), out);
    }
}

#[test]
fn mask_times_agrees_with_run_sh() {
    assert_eq!(mask_times("  f.pgm  PSNR 21.50 dB  3.14 ms"), "  f.pgm  PSNR 21.50 dB  <time> ms");
    assert_eq!(mask_times("x, 12.5 frames/s settling, 3.0 frames/s with"), "x, <time> frames/s settling, <time> frames/s with");
    assert_eq!(mask_times("residual 0.1 -> 0.2, 0.52 s fitting"), "residual 0.1 -> 0.2, <time> s fitting");
    assert_eq!(mask_times("rounds 200, noise 0.100, 1.25s"), "rounds 200, noise 0.100, <time>s");
    // numbers that are not timings stay
    assert_eq!(mask_times("yes 42.5% of 1000 samples, 3 sweeps"), "yes 42.5% of 1000 samples, 3 sweeps");
    assert_eq!(mask_times("energy -3.000s1"), "energy -3.000s1");
    assert_eq!(mask_times("12 pulls, 0.03s; exact"), "12 pulls, <time>s; exact");
}

#[test]
fn every_docs_example_runs_and_matches() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("docs");
    let ex = root.join("examples");
    let bless = std::env::var("SETTLE_DOCS_BLESS").map(|v| v == "1").unwrap_or(false);

    let mut names: Vec<String> = fs::read_dir(&ex)
        .unwrap()
        .filter_map(|e| {
            let p = e.unwrap().path();
            (p.extension().map(|x| x == "settle").unwrap_or(false)).then(|| p.file_stem().unwrap().to_string_lossy().into_owned())
        })
        .collect();
    names.sort();
    assert!(!names.is_empty(), "no examples found in {}", ex.display());

    let scratch = std::env::temp_dir().join(format!("settle-docs-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    copy_dir(&ex, &scratch);
    let here = std::env::current_dir().unwrap();
    std::env::set_current_dir(&scratch).unwrap();

    let mut got: BTreeMap<String, Result<String, String>> = BTreeMap::new();
    let mut problems = Vec::new();
    for n in &names {
        let t = std::time::Instant::now();
        let r = run_example(&scratch, n);
        let secs = t.elapsed().as_secs_f64();
        if secs > 20.0 {
            problems.push(format!("{}: took {:.1} s; docs examples must stay small", n, secs));
        }
        got.insert(n.clone(), r);
    }
    std::env::set_current_dir(&here).unwrap();
    let _ = fs::remove_dir_all(&scratch);

    for (n, r) in &got {
        let (out_p, err_p) = (ex.join(format!("{}.out", n)), ex.join(format!("{}.err", n)));
        if bless {
            match r {
                Ok(o) => {
                    fs::write(&out_p, format!("{}\n", o)).unwrap();
                    let _ = fs::remove_file(&err_p);
                }
                Err(e) => {
                    fs::write(&err_p, format!("{}\n", e)).unwrap();
                    let _ = fs::remove_file(&out_p);
                }
            }
            continue;
        }
        match (r, read_trim(&out_p), read_trim(&err_p)) {
            (Ok(o), Some(want), _) if *o == want => {}
            (Err(e), _, Some(want)) if *e == want => {}
            (Ok(o), Some(want), _) => problems.push(format!("{}: output differs\n--- recorded\n{}\n--- printed\n{}", n, want, o)),
            (Err(e), _, Some(want)) => problems.push(format!("{}: error differs\n--- recorded\n{}\n--- printed\n{}", n, want, e)),
            (Ok(o), None, _) => problems.push(format!("{}: ran, but has no {}.out; printed:\n{}", n, n, o)),
            (Err(e), _, None) => problems.push(format!("{}: failed, and has no {}.err: {}", n, n, e)),
        }
    }

    // the Markdown shows each example verbatim, and shows every one of them
    let mut mds = Vec::new();
    md_files(&root, &mut mds);
    if bless {
        for md in &mds {
            let text = fs::read_to_string(md).unwrap();
            let fresh = rewrite_fences(&text, &ex);
            if fresh != text {
                fs::write(md, fresh).unwrap();
            }
        }
    }
    let mut shown: BTreeMap<String, usize> = BTreeMap::new();
    for md in &mds {
        let text = fs::read_to_string(md).unwrap();
        let rel = md.strip_prefix(&root).unwrap().display().to_string();
        for (info, body) in fences(&text) {
            let words: Vec<&str> = info.split_whitespace().collect();
            let attr = |k: &str| words.iter().find_map(|w| w.strip_prefix(&format!("{}=", k)).map(|v| v.to_string()));
            if let Some(f) = attr("file") {
                match read_trim(&ex.join(&f)) {
                    None => problems.push(format!("{}: fence `{}` names examples/{}, which does not exist", rel, info, f)),
                    Some(want) if body != want => problems.push(format!("{}: fence `{}` differs from examples/{}", rel, info, f)),
                    _ => {}
                }
                continue;
            }
            let (file, name) = if let Some(n) = attr("example") {
                (ex.join(format!("{}.settle", n)), n)
            } else if let Some(n) = attr("output") {
                (ex.join(format!("{}.out", n)), n)
            } else if let Some(n) = attr("error") {
                (ex.join(format!("{}.err", n)), n)
            } else {
                continue;
            };
            if attr("example").is_some() {
                *shown.entry(name.clone()).or_default() += 1;
            }
            match read_trim(&file) {
                None => problems.push(format!("{}: fence `{}` names {}, which does not exist", rel, info, file.display())),
                Some(want) if body != want => {
                    problems.push(format!("{}: fence `{}` differs from {}\n--- file\n{}\n--- fence\n{}", rel, info, file.display(), want, body))
                }
                _ => {}
            }
        }
    }
    for n in &names {
        if !shown.contains_key(n) {
            problems.push(format!("examples/{}.settle is not shown by any ```settle example={} fence", n, n));
        }
    }
    assert!(problems.is_empty(), "{} problem(s):\n\n{}", problems.len(), problems.join("\n\n"));
}
