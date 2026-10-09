# Install and run

Three steps: get the source, build it, run a program. Then the command line, Rust, and the tests.

## Get the source

SETTLE has its own repository, <https://github.com/triplesparkle/SETTLE>. It is private for now, so a clone needs
access:

```bash
git clone https://github.com/triplesparkle/SETTLE
cd SETTLE
```

The commands on this page run from the root of that repository. It is an export of `SETTLE/settle-rs/` in the
SETTLE research repository; the commands run the same from that folder.

## Requirements

- Rust with Cargo, stable channel. The crate uses the 2021 edition and was last built and tested with rustc 1.96.0.
- The crate has one dependency, KANERVA, the sparse distributed memory library. In the SETTLE repository Cargo
  fetches it with `git` from <https://github.com/triplesparkle/KANERVA>, so the first build needs the network and
  the same access as the clone. In the research repository it is the sibling folder `../kanerva`, and the crate
  builds offline.
- KANERVA comes in only with the sdm-family statements (`memory`, `sdm`, `softsdm`, `sdmscale`, `refusal`,
  `contenttrack`, and `coded`'s `save_coded` and `recall_coded`), through the `sdm` feature, on by default.
  `cargo build --release --no-default-features` builds SETTLE without it: every other statement prints exactly
  what the full build prints, and an sdm-family line is refused with an error naming the feature
  (`tests/standalone.rs` checks both halves).

## Build

```bash
cargo build --release
```

The interpreter is `target/release/settle`. Always use the release build: the sampler flips millions of coins,
and the debug build is many times slower.

## Your first program

Save this as `first.settle`:

```settle example=tour-first
# Two things that tend to agree, and one that leans towards yes.
model :pair do
  thing :a, leans: :yes, by: 0.5   # a prefers yes a little
  thing :b                         # b has no preference of its own
  a.pulls :b, by: 1                # a and b prefer to agree
end

run :pair do
  settle 20_000, seed: 1           # draw 20,000 arrangements
  show                             # how often each thing was yes
  ask :b                           # the chance that b is yes
  ask :a, and: :b                  # the chance that both are yes
end
```

Run it with `target/release/settle first.settle`. It prints:

```text output=tour-first
settled: 20000 samples of 2 things at temperature 1
  a              ###################### 72.7%
  b              #################### 67.3%
ask :b: yes 67.3% of 20000 samples
ask :a, and: :b: yes 63.9% of 20000 samples
```

The [tour](02-tour.md) explains each line.

## Run a program

A SETTLE program is a text file, by convention with the extension `.settle`:

```bash
target/release/settle path/to/program.settle
```

or, through Cargo, `cargo run --release -- path/to/program.settle`. What happens:

1. The interpreter reads the whole file and runs it line by line (see [Semantics](04-semantics.md)).
2. If every line succeeds, it prints every output line, in order, and exits with status 0.
3. If a line fails, it prints one message to standard error and exits with status 2. The message reads
   `settle: line N: <what went wrong>`, then the program line with a caret under the place the error points at
   ([Errors](06-errors.md#how-an-error-is-reported)). **The lines before the failure print nothing**: output is
   printed only when the whole program has succeeded.
4. If the file cannot be read, it prints `settle: cannot read <path>: <reason>` and exits with status 2.

A relative path inside a program (a picture, an example set, an output folder) is read from the folder that holds
the program file, not from the current directory. An absolute path is used as given.

## The command line

```bash
settle <program.settle>
settle --json <program.settle>
settle --help
settle -h
settle
settle --version
settle -V
```

`--help`, `-h`, or no argument at all print the usage, then every statement grouped by family (one line per
statement form, marked `[family]` and `model:` or `run:`), then a list of example programs. The full argument
lists are in [Statements by family](05-statements/README.md). `--version` or `-V` prints `settle` and the crate's
version, for example `settle 0.1.0`.

`--json` before the program path prints one JSON object instead of the lines. It is meant for a program that
drives `settle`, such as an editor or an MCP server. `settle --json first.settle` prints:

```text file=tour-first.json
{"settle": "0.1.0", "ok": true, "lines": ["settled: 20000 samples of 2 things at temperature 1", "  a              ###################### 72.7%", "  b              #################### 67.3%", "ask :b: yes 67.3% of 20000 samples", "ask :a, and: :b: yes 63.9% of 20000 samples"]}
```

A program that fails gives `"ok": false` and the error, with the `line`, `column` and `width` the caret would
mark, counted from 1. This one pulls on a thing it never declared
([Errors](06-errors.md#argument-errors-shared-by-every-family)):

```text file=err-unknown-thing.json
{"settle": "0.1.0", "ok": false, "error": {"message": "line 3: unknown thing :b (declare it with: thing :b)", "line": 3, "column": 11, "width": 2}}
```

The exit status is the same as without `--json`. An error that names no program line (a file that cannot be read)
has no `line`, `column` or `width`.

There are no other options. Any other argument that starts with `-` stops with
`settle: unknown option <arg> (see settle --help)` and status 2, and two program paths stop with
`settle: one program at a time; got <n> arguments (see settle --help)`. Everything a program needs, seeds,
temperatures and file paths included, is written in the program.

## Use it from Rust

The crate is also a library, `settle`. `Interp` runs SETTLE source and returns the printed lines:

```rust
let mut it = settle::interp::Interp::in_dir("models/"); // relative paths resolve against models/
let lines: Vec<String> = it.exec(source_text)?;          // the printed lines, or a SettleError
```

The core statements also have a builder, `settle::engine::model::Model::build()`, which writes a program in Rust
and prints the same lines ([Extending SETTLE](07-extending.md#the-builder-face)).

## Run the tests

```bash
cargo test --release
```

This runs the unit tests in each source file and the integration tests in `tests/`, among them:

- `tests/docs_examples.rs`: every example in this documentation runs and prints what the page says.
- `tests/docs_json.rs`: every `--json` answer on these pages is what `settle --json` prints.
- `tests/docs_complete.rs`: every keyword and statement the interpreter accepts is written on its family's page.
- `tests/two_faces.rs`: the builder and the program files build the same models and print the same lines.
- `tests/standalone.rs`: the build without KANERVA. `tests/counts.rs`: every family refuses a count that is not a
  whole number.

[Examples](09-examples.md) says how to record the output of a new example.

## Measurement programs

`examples/*.rs` are the Rust programs the experiments used for their measurements. Each one names its parts in
its header:

```bash
cargo run --release --example valleymap -- <part>
```

They are research instruments, not language examples; [Examples](09-examples.md) lists them.
