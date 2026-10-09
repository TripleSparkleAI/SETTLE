//! The `settle` command. All of it lives in the doors floor (`settle::doors::cli`); this is the shim Cargo builds.

fn main() {
    let args: Vec<String> = std::env::args().collect();
    std::process::exit(settle::doors::cli::main(&args));
}
