//! The statement pages are complete: every keyword a statement accepts, and every statement a family lists in
//! `settle --help`, is written on that family's page in docs/05-statements/.
//!
//! <claudes_code_comments>
//! ** Function List **
//! keyword_lists(src)        - every `only(&kv, &[...], "statement", ln)` call in a source file: (statement, keywords)
//! undocumented(src, page)   - the keywords of a source file that its page never writes as `keyword:`
//! verbs_of(statement)       - the verbs of one help line: `settle` in "run: settle 10_000", `read` in "s.read ..."
//! the_keyword_scan_sees_a_missing_keyword - the positive control on a two-line fixture
//! every_accepted_keyword_is_on_its_family_page - the scan over src/
//! every_listed_statement_is_on_its_family_page - the help lines against the pages
//!
//! ** Technical Review **
//! - A statement refuses unknown keywords with `lex::only(&kv, &[allowed...], "name", ln)`, so the allowed list in
//!   the source IS the statement's keyword set. The page for `src/<family>.rs` is `docs/05-statements/<family>.md`;
//!   a keyword counts as documented when the page holds `keyword:` anywhere (its arguments table, its form, or a
//!   note). `src/words/lex.rs` is skipped: its `only` calls are its own tests.
//! - The help side reads `Ext::statements()` from the live registry, so a family added to `registry()` without a
//!   page, or a statement added to a family without a word on its page, fails here.
//! - Floors (at least 150 keywords and 70 verbs found) keep a broken parser from passing by finding nothing.
//!
//! </claudes_code_comments>

use settle::interp::Interp;
use std::fs;
use std::path::Path;

fn keyword_lists(src: &str) -> Vec<(String, Vec<String>)> {
    let mut out = Vec::new();
    let mut rest = src;
    while let Some(i) = rest.find("only(&kv, &[") {
        let after = &rest[i + "only(&kv, &[".len()..];
        let Some(close) = after.find(']') else { break };
        let list = &after[..close];
        let words: Vec<String> = list.split('"').skip(1).step_by(2).map(String::from).collect();
        // the statement's name is the next quoted string after the list, when there is one on the same call
        let tail = &after[close..];
        let call_end = tail.find(';').unwrap_or(tail.len());
        let name = tail[..call_end].split('"').nth(1).unwrap_or("?").to_string();
        out.push((name, words));
        rest = &after[close..];
    }
    out
}

fn undocumented(src: &str, page: &str) -> Vec<String> {
    let mut miss = Vec::new();
    for (stmt, words) in keyword_lists(src) {
        for w in words {
            if !page.contains(&format!("{}:", w)) {
                miss.push(format!("{} `{}:`", stmt, w));
            }
        }
    }
    miss
}

fn verbs_of(statement: &str) -> Vec<String> {
    let Some((_, body)) = statement.split_once(": ") else { return Vec::new() };
    body.split("   /   ")
        .filter_map(|form| {
            let first = form.split_whitespace().next()?;
            let word = first.rsplit('.').next().unwrap_or(first);
            let word: String = word.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
            (!word.is_empty()).then_some(word)
        })
        .collect()
}

#[test]
fn the_keyword_scan_sees_a_missing_keyword() {
    let src = r#"    only(&kv, &["seed", "temperature"], "settle", ln)?;"#;
    assert_eq!(keyword_lists(src), vec![("settle".to_string(), vec!["seed".to_string(), "temperature".to_string()])]);
    assert_eq!(undocumented(src, "| `seed:` | number |"), vec!["settle `temperature:`".to_string()]);
    assert!(undocumented(src, "`seed:` and `temperature:`").is_empty());
    assert_eq!(verbs_of("run: s.read read-address: :cat   /   s.read   (from noise)"), vec!["read", "read"]);
}

#[test]
fn every_accepted_keyword_is_on_its_family_page() {
    let here = Path::new(env!("CARGO_MANIFEST_DIR"));
    let (mut missing, mut found) = (Vec::new(), 0usize);
    let mut files: Vec<_> = fs::read_dir(here.join("src")).unwrap().flatten().map(|e| e.path()).collect();
    files.sort();
    for p in files {
        let stem = p.file_stem().unwrap().to_string_lossy().to_string();
        if p.extension().is_none_or(|e| e != "rs") || stem == "lex" {
            continue;
        }
        let src = fs::read_to_string(&p).unwrap();
        let lists = keyword_lists(&src);
        if lists.iter().all(|(_, w)| w.is_empty()) {
            continue;
        }
        found += lists.iter().map(|(_, w)| w.len()).sum::<usize>();
        let page_path = here.join("docs/05-statements").join(format!("{}.md", stem));
        match fs::read_to_string(&page_path) {
            Ok(page) => missing.extend(undocumented(&src, &page).into_iter().map(|m| format!("src/{}.rs: {}", stem, m))),
            Err(_) => missing.push(format!("src/{}.rs takes keywords but docs/05-statements/{}.md does not exist", stem, stem)),
        }
    }
    assert!(found >= 150, "the scan found only {} keywords; the parser is broken", found);
    assert!(missing.is_empty(), "keywords a statement takes but its page never writes ({}):\n{}", missing.len(), missing.join("\n"));
}

#[test]
fn every_listed_statement_is_on_its_family_page() {
    let here = Path::new(env!("CARGO_MANIFEST_DIR"));
    let (mut missing, mut found) = (Vec::new(), 0usize);
    for e in Interp::default().exts {
        let page_path = here.join("docs/05-statements").join(format!("{}.md", e.name()));
        let Ok(page) = fs::read_to_string(&page_path) else {
            missing.push(format!("family [{}] has no page docs/05-statements/{}.md", e.name(), e.name()));
            continue;
        };
        for s in e.statements() {
            for v in verbs_of(s) {
                found += 1;
                if !page.contains(&format!("`{}", v)) && !page.contains(&format!(".{}", v)) {
                    missing.push(format!("[{}] `{}` (from: {})", e.name(), v, s));
                }
            }
        }
    }
    assert!(found >= 70, "the help gave only {} verbs; the parser is broken", found);
    assert!(missing.is_empty(), "statements in `settle --help` that their page never names ({}):\n{}", missing.len(), missing.join("\n"));
}
