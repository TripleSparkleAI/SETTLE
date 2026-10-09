# SETTLE release checklist

What is ready in this crate for a first release, how each item was checked, and what the owner still decides.
Checked on 2026-10-05 on an M5 MacBook Pro with rustc 1.96.0 and cargo 1.96.0. Re-run the commands before a
release rather than trusting the dates here.

## Ready

| Item | How it was checked |
|---|---|
| Builds | `cargo build --release`: no compiler warnings. |
| Tests pass, one red owed to another lane | Re-checked 2026-10-06 (lane SETTLEPERFECT), after merging ONEPARSER: `cargo test --release --no-fail-fast`: 260 unit tests pass and 9 are ignored (long measurements); `tests/oneparser_parity.rs` (7) holds KANERVA's parser and runner equal to SETTLE's; `tests/kanerva_rails.rs` passes 4 of 5, and its scan of SETTLE's `only(...)` lists finds none since the plug replaced them (lane KANERVARAILSFIX is repointing it); `tests/docs_examples.rs` (3) runs every documented example against its recorded output; `tests/docs_complete.rs` (3) finds every accepted keyword and listed statement on its family's page; `tests/kanerva_terms.rs` (2); `tests/two_faces.rs` (10) holds the builder and the program files equal; `tests/standalone.rs` (1) builds without KANERVA and runs all 136 programs; `tests/counts.rs` (2); 5 doc-tests. |
| Builds without KANERVA | `cargo build --release --no-default-features` and `cargo test --release --no-default-features --lib`: green, no warnings. |
| Lints | Re-checked 2026-10-06: `cargo clippy --release --all-targets` gives no warnings, with or without `--no-default-features`. `needless_range_loop` is allowed crate-wide with its reason in `src/lib.rs`, and `too_many_arguments` on the film fitting loops. |
| Every example program runs | All 17 `examples/*.settle` exit 0 when run with `./target/release/settle examples/<name>.settle`, including `horse.settle`, whose frames ship in `examples/horse/frames/`. |
| The command line | `settle --help` lists 20 statement families; `settle --version` prints `settle 0.1.0`; `settle --json <program>` answers with one JSON object; an unknown option, a second argument and a missing file each print one line and exit 2. A program error prints the line it points at with a caret and its column, and a misspelt keyword, thing or statement gets a `did you mean`. |
| Documentation | `docs/` covers install, a tour, syntax, semantics, all 20 families, errors, extending, a cookbook, examples and the science. Every program in it is a file in `docs/examples/` checked by the tests. |
| Metadata | `Cargo.toml`: name, version 0.1.0, edition 2021, description, repository, readme, keywords, categories, `publish = false`. |
| Changelog | `docs/CHANGELOG.md`, by date, with the 2026-10-05 release preparation as its last section. |
| Third-party material | The Muybridge frames are public domain (`examples/horse/frames/SOURCE.txt`); the two texts in `data/` are public-domain works with sources and checksums in `data/PROVENANCE.txt`. |
| The standalone repository | `SETTLE/tools/export_settle_repos.sh --only KANERVA --only SETTLE` in the research repository writes the SETTLE repository from tracked files only, rewrites the KANERVA dependency to its git URL, adds `.cargo/config.toml`, and scans for secrets and research-only names. The export was built and tested (`cargo test --release`, all green) against a local KANERVA export on 2026-10-05, with KANERVA patched in by path because the exported KANERVA commit was not pushed. |

## The owner decides

1. **The licence.** DECIDED: MIT (the navigator, 2026-10-09). `LICENSE` holds the text, `Cargo.toml` says
   `license = "MIT"`, and the site's `src/pkgrefs.js` names it. KANERVA carries the same licence.
2. **Making the repository public.** DECIDED: public at launch (the navigator, 2026-10-09). `SETTLE/launch.sh`'s
   public step (step 7, or `--public` alone) makes all eight SETTLE repositories public, KANERVA with SETTLE, just
   before the deploying push; the site's `src/repo.js` already says public.
3. **Publishing to crates.io.** `publish = false` stops an accidental `cargo publish`. **The name `settle` is
   already taken on crates.io** by an unrelated crate (a Zettelkasten command-line tool, version 0.40.1, checked
   2026-10-05). Publishing needs a different crate name, and KANERVA would have to be published first, because
   crates.io does not accept git dependencies. The name `kanerva` was free on 2026-10-05.
4. **A minimum Rust version.** None is declared (`rust-version`). The crate is tested only on rustc 1.96.0.
   Two floors are known: the crate uses `Option::is_none_or` (stable since Rust 1.82), and lane PKGKANERVA
   reports moving KANERVA to edition 2024 with `rust-version = "1.87"` (not yet on the main line on 2026-10-05). Declare one only after building on that version.
5. **The version number.** The crate has been 0.1.0 since it began. Decide whether the first release is 0.1.0
   or a new number, and add a dated section to `docs/CHANGELOG.md` with it.

## Not covered by this checklist

- The measurement programs in `examples/*.rs` build with the tests, but several need data that is not in the
  repository (MNIST, film frames, lane outputs); their headers say what each one needs. They are research
  instruments, not part of the language.
- Timings printed by programs (frames per second, milliseconds) depend on the machine and its load, so the
  documentation shows them as `<time>` and the tests do not compare them.
