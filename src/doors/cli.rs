//! The `settle` command: run a .settle program and print what it says.
//!
//! <claudes_code_comments>
//! ** Function List **
//! usage()          - the help text: the usage lines, every statement by family, some example programs
//! main(args)       - reads the arguments (--help, --version, --json, one program path), runs it; the exit status
//! run(path, json)  - one program: read it, run it, print its lines or its error (plain or JSON)
//! report(src, msg) - an error as printed: the message, the program line with a caret at the column, the rest
//!
//! ** Technical Review **
//! - `settle <program.settle>` runs one program. Output is collected and printed only when the whole program
//!   succeeds; on an error the message goes to standard error with the program line it points at and a caret
//!   under the place (`report`, from `lex::locate`), and the exit status is 2.
//! - `settle --json <program.settle>` prints one JSON object on standard output either way:
//!   `{"settle": version, "ok": true, "lines": [...]}` or `{"settle": version, "ok": false, "error":
//!   {"message", "line", "column", "width"}}` (`doors::json`). The exit status is the same as without `--json`.
//! - `--help`, `-h` or no argument prints the usage; `--version` or `-V` prints `settle <version>` from Cargo.toml.
//!   Any other argument starting with `-` is refused with exit status 2, so a mistyped flag is never read as a file.
//! - Relative paths inside a program resolve against the program's own folder (`Interp::base_dir`).
//!
//! </claudes_code_comments>

use crate::doors::json::{answer_error, answer_ok};
use crate::words::interp::Interp;
use crate::words::lex::locate;
use std::path::Path;

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn usage() {
    println!("settle <program.settle>\nsettle --json <program.settle>\nsettle --version\n\nSETTLE: declare things and how they pull on each other, then settle and ask.");
    println!("\nStatements, by family:");
    for e in Interp::default().exts {
        for s in e.statements() {
            println!("  [{}] {}", e.name(), s);
        }
    }
    println!("\nExamples: examples/weather.settle · examples/party.settle · examples/memory.settle · examples/learn.settle · examples/denoise.settle");
}

/// The command, given its arguments (the program name first). Returns the exit status: 0 on success, 2 on any
/// error (an unknown option, an unreadable file, or an error in the program). `--json`, when given, comes first.
pub fn main(args: &[String]) -> i32 {
    let json = args.get(1).map(String::as_str) == Some("--json");
    let args: Vec<&str> = args.iter().map(String::as_str).enumerate().filter(|&(i, _)| !(json && i == 1)).map(|(_, a)| a).collect();
    match args.get(1).copied() {
        None if json => {
            eprintln!("settle: --json needs a program (see settle --help)");
            return 2;
        }
        None | Some("--help") | Some("-h") => {
            usage();
            return 0;
        }
        Some("--version") | Some("-V") => {
            println!("settle {}", VERSION);
            return 0;
        }
        Some(f) if f.starts_with('-') => {
            eprintln!("settle: unknown option {} (see settle --help)", f);
            return 2;
        }
        _ => {}
    }
    if args.len() > 2 {
        eprintln!("settle: one program at a time; got {} arguments (see settle --help)", args.len() - 1);
        return 2;
    }
    run(args[1], json)
}

/// Run one program file; print its lines, or its error.
pub fn run(path: &str, json: bool) -> i32 {
    let src = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            let msg = format!("cannot read {}: {}", path, e);
            if json {
                println!("{}", answer_error(VERSION, &msg, None));
            } else {
                eprintln!("settle: {}", msg);
            }
            return 2;
        }
    };
    let mut it = Interp::in_dir(Path::new(path).parent().map(|p| p.to_path_buf()).unwrap_or_default());
    match it.exec(&src) {
        Ok(lines) if json => {
            println!("{}", answer_ok(VERSION, &lines));
            0
        }
        Ok(lines) => {
            lines.iter().for_each(|l| println!("{}", l));
            0
        }
        Err(e) if json => {
            println!("{}", answer_error(VERSION, &e.0, locate(&src, &e.0)));
            2
        }
        Err(e) => {
            eprint!("{}", report(&src, &e.0));
            2
        }
    }
}

/// An error as the command prints it: the message's first line, the program line it points at with a caret under
/// the place (`lex::locate`), then any further lines of the message.
pub fn report(src: &str, msg: &str) -> String {
    let mut lines = msg.lines();
    let mut out = format!("settle: {}\n", lines.next().unwrap_or(""));
    if let Some((ln, col, width)) = locate(src, msg) {
        let code = src.lines().nth(ln - 1).unwrap_or("");
        let gutter = ln.to_string().len().max(4);
        out += &format!("{:>g$} | {}\n", ln, code.trim_end(), g = gutter);
        out += &format!("{:>g$} | {}{} column {}\n", "", " ".repeat(col - 1), "^".repeat(width), col, g = gutter);
    }
    for l in lines {
        out += l;
        out += "\n";
    }
    out
}
