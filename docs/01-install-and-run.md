# Install and run

## Get the source

The SETTLE source is moving to its own repository:

- <https://github.com/triplesparkle/SETTLE> (a private repository; you need to be given access)

Until that move is complete, the interpreter lives in the dwarfstar repository at
`experiments/thermosim/settle-rs/`. The commands on this page are run from that folder, or from the root of
the SETTLE repository once it exists.

## Requirements

- A Rust toolchain with Cargo, stable channel. The crate uses the 2021 edition.
- Nothing from the network. The crate has one dependency, KANERVA, the sibling crate at `../kanerva` (a path
  dependency in `Cargo.toml`), so it builds offline.

## Build

```text
cargo build --release
```

This produces the interpreter at `target/release/settle`. Use the release build: the sampler does millions of
coin flips and the debug build is many times slower.

The crate is both a library (`settle`, in `src/lib.rs`) and a binary (`settle`, in `src/main.rs`). The library
exposes the interpreter, `settle::interp::Interp`, so a Rust program can run SETTLE source directly:

```text
let mut it = settle::interp::Interp::default();
let lines: Vec<String> = it.exec(source_text)?;   // the printed lines, or a SettleError
```

## Run a program

A SETTLE program is a text file, by convention with the extension `.settle`.

```text
target/release/settle path/to/program.settle
```

or, through Cargo:

```text
cargo run --release -- path/to/program.settle
```

What happens:

1. The interpreter reads the whole file.
2. It executes the file line by line (see [Semantics](04-semantics.md)).
3. If every line succeeds, it prints every output line, in order, and exits with status 0.
4. If a line fails, it prints one message to standard error and exits with status 2. The message has the form
   `settle: line N: <what went wrong>`. **Output from the lines before the failure is not printed**: the
   interpreter collects output and prints it only when the whole program has succeeded.
5. If the file cannot be read, it prints `settle: cannot read <path>: <reason>` and exits with status 2.

Relative paths inside a program (picture files, example sets, output folders) are resolved against the folder
that contains the program file, not against the current working directory. Absolute paths are used as given.

## The command line

```text
settle <program.settle>
settle --help
settle -h
settle
```

With `--help`, `-h`, or no argument at all, `settle` prints a one-line usage, then every statement grouped by
family (one line per statement form, prefixed with `[family]` and `model:` or `run:`), then a list of example
programs. The help lines are short reminders; the full argument lists are in
[Statements by family](05-statements/README.md).

There are no other options. Everything a program needs, including seeds, temperatures and file paths, is
written in the program.

## Run the tests

```text
cargo test --release
```

This runs the unit tests inside each source file (`#[cfg(test)]` modules) and the integration tests in
`tests/`. One of those, `tests/docs_examples.rs`, runs every example in this documentation and checks its
output against the recorded output. See [Examples](09-examples.md) for how to record the output of a new
example.

## Measurement programs

`examples/*.rs` are Rust programs that the campaign lanes used to take their measurements. Each one names its
parts in its header. Run one with, for example:

```text
cargo run --release --example valleymap -- <part>
```

These are research instruments rather than language examples; [Examples](09-examples.md) lists them.

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
  a              ###################### 73.5%
  b              #################### 68.0%
ask :b: yes 68.0% of 20000 samples
ask :a, and: :b: yes 64.9% of 20000 samples
```

The [tour](02-tour.md) explains each line.
