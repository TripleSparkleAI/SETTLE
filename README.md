<!-- settle-banner -->
```text
 ████  █████  █████  █████  █      █████
█      █        █      █    █      █
 ███   ████     █      █    █      ████
    █  █        █      █    █      █
████   █████    █      █    █████  █████
✦ a small language for settling machines
```

# SETTLE

SETTLE is a small programming language for probabilistic models made of yes/no things. A program declares
things, how strongly each one leans towards yes or no, and how pairs of things pull towards agreement or push
towards disagreement. Running the program is Settling: the interpreter draws many random arrangements of the
things, each with a probability set by those leans and pulls, and answers questions by counting.

The syntax follows Ruby on Rails conventions (`do ... end` blocks, `:symbols`, `key: value` arguments). This
crate is the interpreter, written in Rust, as a command (`settle`) and a library (`settle::interp::Interp`).
Version 0.1.0 (`Cargo.toml`).

## Get it and build it

You need a Rust toolchain with Cargo (stable; it was last built and tested with rustc 1.96.0).

```bash
git clone https://github.com/TripleSparkleAI/SETTLE
cd SETTLE
cargo build --release
./target/release/settle docs/examples/tour-first.settle
```

The repository is public, so anyone can clone it. The first build fetches KANERVA, SETTLE's one dependency, from
its public repository `https://github.com/TripleSparkleAI/KANERVA` with the `git` command (`.cargo/config.toml`).

The last command runs the first program of the documentation and prints:

```text
settled: 20000 samples of 2 things at temperature 1
  a              ###################### 73.5%
  b              #################### 68.0%
ask :b: yes 68.0% of 20000 samples
ask :a, and: :b: yes 64.9% of 20000 samples
```

## A program

`docs/examples/core-weather.settle`, one of the documented examples:

```settle
# the grass is wet: was it the rain or the sprinkler?
model :weather do
  thing :rain,      leans: :no, by: 1       # rain is usually not happening
  thing :sprinkler, leans: :no, by: 0.5     # the sprinkler is usually off
  thing :wet_grass                          # no lean of its own

  rain.pushes :sprinkler, by: 0.5           # nobody waters in the rain
  rain.pulls  :wet_grass, by: 1.5           # rain wets the grass
  sprinkler.pulls :wet_grass, by: 1         # so does the sprinkler
end

run :weather do
  hold :wet_grass, :yes                     # we saw it: the grass is wet
  settle 20_000, temperature: 1, seed: 1    # 20,000 kept samples
  show                                      # yes-rate of every thing
  ask :rain                                 # how often was it raining?
  ask :rain, and: :sprinkler                # both at once
  ask :rain, or: :sprinkler                 # at least one of them
  ask :sprinkler, and_not: :rain            # the sprinkler alone
end
```

`./target/release/settle docs/examples/core-weather.settle` prints (the test `tests/docs_examples.rs` checks
it on every run):

```text
settled: 20000 samples of 3 things at temperature 1
  rain           ################### 64.2%
  sprinkler      ################### 63.0%
  wet_grass      ############################## 100.0%  (held)
ask :rain: yes 64.2% of 20000 samples
ask :rain, and: :sprinkler: yes 31.6% of 20000 samples
ask :rain, or: :sprinkler: yes 95.6% of 20000 samples
ask :sprinkler, and_not: :rain: yes 31.4% of 20000 samples
```

`docs/02-tour.md` explains the language line by line, and `examples/weather.settle` is the same model with
40,000 samples.

## The command line

```text
settle <program.settle>     run one program
settle --json <program>     run one program; answer with one JSON object (its lines, or its error and where it is)
settle --help               the usage, every statement by family, some example programs (also -h, or no argument)
settle --version            settle 0.1.0 (also -V)
```

A program's output is printed only when the whole program succeeds. On an error `settle` prints
`settle: line N: <what went wrong>` to standard error, then that program line with a caret under the place, and
exits with status 2. A misspelt keyword, thing or statement gets a `did you mean` suggestion. Relative paths inside a program
resolve against the program's own folder. `docs/01-install-and-run.md` has the details.

## Documentation

`docs/` is the whole reference; start at `docs/README.md`:

1. Install and run · 2. A tour of SETTLE · 3. Syntax · 4. Semantics · 5. Statements by family (20 families, one
   page each) · 6. Errors · 7. Extending SETTLE · 8. Cookbook · 9. Examples · 10. The science.

Every program shown in the documentation is a file in `docs/examples/` with its recorded output beside it, and
the test `tests/docs_examples.rs` runs each one on every `cargo test --release`.

## Tests

```bash
cargo test --release
```

This runs the unit tests in each source file and the integration tests in `tests/`: `docs_examples.rs` (every
documented example, against its recorded output), `docs_complete.rs` (every keyword and statement the
interpreter accepts is written on its family's page), `kanerva_terms.rs` (no program uses a keyword retired
for Kanerva's own terms), `two_faces.rs` (the Rust builder and the program files build the same models and print
the same lines), `standalone.rs` (the build without KANERVA prints the same for every program that needs no
memory) and `counts.rs` (every family refuses a count that is not a whole number).

## Examples

- `examples/*.settle`: 17 programs, each runnable from a clean clone with
  `./target/release/settle examples/<name>.settle` (weather, party, memory, learn, denoise, sudoku, colouring,
  max-cut, factoring, keyed memory, sdm, softsdm, coded text, LDPC codes, solving linear systems, a survey of
  valleys, and Muybridge's horse played on 15,000 p-bits, whose frames ship in `examples/horse/frames/`).
- `examples/*.rs`: the measurement programs the research lanes used. `docs/09-examples.md` lists them. Some
  need data that is not in this repository (MNIST, film frames); each file's header says what it needs.

## Using SETTLE from Rust

The crate is built in three floors, the same three KANERVA has: `engine` (the model, the sampler, the anneal
schedule, energy and answers, with no parsing and no printing), `words` (the language: the lexer, the blocks, the
registry every statement family plugs into, and the core family) and `doors` (the `settle` command and JSON). Run a
program with `settle::interp::Interp`, or write the core statements in Rust with the builder:

```rust
use settle::engine::model::Model;

let lines = Model::build()
    .thing("rain").leans("no", 1.0)
    .thing("wet_grass")
    .pulls("rain", "wet_grass", 1.5)
    .run()
    .hold("wet_grass", "yes")
    .settle(20_000).seed(1)
    .ask("rain")
    .lines()?;
```

Its method names are the words of a program, and it runs through the same code as a program does, so it prints the
same lines. `docs/07-extending.md` describes the floors, the registry and the builder.

## Sparse distributed memory lives in KANERVA

Every SDM algorithm the statements use (`memory`, `sdm`, `softsdm`, `sdmscale`, `refusal`, `contenttrack`) is in
the KANERVA crate, a standalone library at `https://github.com/TripleSparkleAI/KANERVA`. The
files in `src/` keep the statements, the pull layouts and their tests, and re-export KANERVA's items under their
old names. Read KANERVA's README for the toolbox, its equations and its results.

KANERVA is optional. The `sdm` feature, on by default, brings it in. `cargo build --release --no-default-features`
builds SETTLE without it: every other statement prints exactly what the full build prints, and an sdm-family
statement is refused with an error that names the feature.

## Where this repository comes from

The SETTLE repository is an export of `SETTLE/settle-rs/` in the SETTLE research repository, where the work
happens. `SETTLE/tools/export_settle_repos.sh` there makes it: it copies only tracked files, turns the
`../kanerva` path dependency into KANERVA's git URL, scans for secrets, and commits with a line naming the
research commit it came from.

## Licence

SETTLE is under the MIT licence; the text is in `LICENSE`. Third-party material keeps its own terms: the Muybridge
frames are public domain (`examples/horse/frames/SOURCE.txt`) and the texts in `data/` are public-domain works
(`data/PROVENANCE.txt`).

## Changes

`docs/CHANGELOG.md` lists what each statement family added, and when. `RELEASE_CHECKLIST.md` lists what is
ready for release and what is still the owner's to decide.
