# SETTLE documentation

SETTLE is a small programming language for probabilistic models made of yes/no things. A program declares
things, how strongly each one leans towards yes or no, and how pairs of things pull towards agreement or push
towards disagreement. Running the program is Settling: it draws many random arrangements of the things, each
with a probability set by those leans and pulls, and answers questions by counting.

The syntax follows Ruby on Rails conventions: `do ... end` blocks, `:symbols` and `key: value` arguments. The
interpreter is written in Rust; its one dependency is KANERVA, the crate that holds the memory algorithms. Every
example on these pages is a file in `docs/examples/`, and `cargo test --release` runs each one and compares its
output with the output printed here.

New to SETTLE? Read [Install and run](01-install-and-run.md), then [the tour](02-tour.md). A picture-first
introduction is on the demo site's WHAT? page (`#/what`).

## Contents

### Getting started

1. [Install and run](01-install-and-run.md): build the interpreter, run a program, the command line.
2. [A tour of SETTLE](02-tour.md): the language in ten minutes.

### The language

3. [Syntax](03-syntax.md): tokens, comments, blocks, statement shapes, and a grammar.
4. [Semantics](04-semantics.md): what a model is, how a run samples, temperature, seeds, and what `ask` means.
5. [Statements by family](05-statements/README.md): every statement, its arguments, defaults, output and errors.
6. [Errors](06-errors.md): the messages the interpreter produces and what causes them.
7. [Extending SETTLE](07-extending.md): how to add a statement family.

### Using it

8. [Cookbook](08-cookbook.md): short recipes for common tasks.
9. [Examples](09-examples.md): every example program in the repository, and where it lives.
10. [The science](10-the-science.md): the physics and statistics the interpreter implements, with references.

[Changelog](CHANGELOG.md): what each statement family added, and when.

## Conventions on these pages

- `model` block and `run` block name the two kinds of block a program contains.
- A code fence marked `settle` is a SETTLE program. The fence below it marked `text` is the exact output the
  release interpreter prints for it. Timings in output vary between runs, so they are shown as `<time>`. A fence
  marked `bash` holds commands you type.
- "Thing" means one yes/no variable. "Lean" means a thing's own bias. "Pull" means a coupling between two
  things; a positive pull favours agreement and a negative one (written `pushes`) favours disagreement.

## Source repository

SETTLE has its own repository, <https://github.com/triplesparkle/SETTLE>, private for now.
[Install and run](01-install-and-run.md#get-the-source) says how to get it.
