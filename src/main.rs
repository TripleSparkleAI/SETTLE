//! The `settle` command: run a .settle program and print what it says.

use settle::interp::Interp;
use std::path::Path;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 || args[1] == "--help" || args[1] == "-h" {
        println!("settle <program.settle>\n\nSETTLE: declare things and how they pull on each other, then settle and ask.");
        println!("\nStatements, by family:");
        for e in Interp::default().exts {
            for s in e.statements() {
                println!("  [{}] {}", e.name(), s);
            }
        }
        println!("\nExamples: examples/weather.settle · examples/party.settle · examples/memory.settle · examples/learn.settle · examples/denoise.settle");
        return;
    }
    let src = match std::fs::read_to_string(&args[1]) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("settle: cannot read {}: {}", args[1], e);
            std::process::exit(2);
        }
    };
    let mut it = Interp::default();
    it.base_dir = Path::new(&args[1]).parent().map(|p| p.to_path_buf()).unwrap_or_default();
    match it.exec(&src) {
        Ok(lines) => lines.iter().for_each(|l| println!("{}", l)),
        Err(e) => {
            eprintln!("settle: {}", e);
            std::process::exit(2);
        }
    }
}
