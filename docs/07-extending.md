# Extending SETTLE

SETTLE's statements are grouped into **families**. Each family is one Rust file that implements the `Ext` trait
from `src/ext.rs`. The interpreter knows nothing about any particular statement: it offers every line to each
family in turn, and the first family that recognises the line runs it. Adding a family therefore needs no change
to the lexer or the interpreter. This page describes that interface and walks through a complete small family.

## The interface

```text
pub trait Ext {
    fn name(&self) -> &'static str;
    fn statements(&self) -> &'static [&'static str];
    fn model_stmt(&self, m: &mut Model, t: &[Tok], ln: usize, ctx: &mut Ctx) -> Claim { None }
    fn run_stmt(&self, m: &mut Model, st: &mut State, t: &[Tok], ln: usize, ctx: &mut Ctx) -> Claim { None }
}

pub type Claim = Option<Result<(), SettleError>>;
```

- `name` is the short family name shown in `settle --help` as `[name]`.
- `statements` returns one line per statement form, each starting with `model: ` or `run: ` (or another word,
  such as `model/run: `, for a statement allowed in both). The help listing prints these lines, and the error
  for an unrecognised line lists every `model: ` or `run: ` line with that prefix removed. Write them as short,
  correct examples of the statement.
- `model_stmt` is called for each line inside a `model` block. `run_stmt` is called for each line inside a
  `run` block. Both receive the line as a token slice `t` (see [Syntax](03-syntax.md)) and its line number `ln`.
- The return value is the **claim**:
  - `None`: this line is not mine. The interpreter offers it to the next family.
  - `Some(Ok(()))`: I recognised and ran the line.
  - `Some(Err(e))`: I recognised the line, and it is wrong. The program stops with this error.

  Return `None` only when the line's shape is not yours. Once the shape is yours (the verb matches), return
  `Some`, so a mistake in the arguments is reported by your family rather than as "no statement family knows
  this line".

### What a statement can see

| Parameter | Type | What it is |
|---|---|---|
| `m` | `&mut Model` | The model named by the block: things (`names`, `idx`), leans (`h`), pulls (`adj`), and `notes`. Mutable in both kinds of block, so a run statement may change the model (learning does). |
| `st` | `&mut State` | Run blocks only: held things, temperature, the random generator, recorded samples, yes-counts, the last arrangement and the calmest one found. |
| `t` | `&[Tok]` | The line's tokens. |
| `ln` | `usize` | The line number, for error messages. |
| `ctx` | `&mut Ctx` | `ctx.say(line)` adds an output line. `ctx.path(p)` resolves a path from the program against the program file's folder. |

### Useful helpers

From `src/lex.rs`: `err(ln, msg)` builds an error that prints as `line N: msg`; `kwargs(tokens, ln)` parses
`key: value` pairs; `kw(&kv, "key")` looks one up; `only(&kv, &[...], "statement", ln)` refuses unknown keys;
`num`, `text` and `yes_no` convert a value token or fail with the standard message.

From `src/model.rs`: `Model::add(name)` declares a thing (or returns the existing one), `Model::couple(i, k, w)`
adds to a pull, `Model::need(name, ln)` finds a thing or fails with the standard message, `Model::energy(s)`
and `Model::input(i, s)` compute the energy and one thing's input. `State::settle(m, n)` and
`State::anneal(m, n)` run the core sampler, and `State::rates()` gives the yes-rates.

### Keeping state between statements

A family that builds a structure (a grid's size, a memory's patterns, a trained machine) stores it in the
model's `notes`, a map from a string key to a pair of a number vector and a word vector. Use a key that starts
with your family's name, so families never collide. Notes live on the model, so they persist across run
blocks; the run `State` does not.

## Adding a family

1. Write `src/<family>.rs` with a unit struct and an `impl Ext` for it.
2. Add `pub mod <family>;` to `src/lib.rs`.
3. Add one line, `Box::new(crate::<family>::<Struct>),`, to `registry()` in `src/ext.rs`. Order matters: the
   first family whose pattern matches a line wins, and `core` stays first.
4. Add tests in the file's `#[cfg(test)]` module, run `cargo test --release`, and document the family: a page
   in `docs/05-statements/`, with examples in `docs/examples/`.

A program can also register a family without editing the registry, by pushing it onto `Interp::exts` before
calling `exec`. The example below is registered that way, from `tests/docs_examples.rs`.

## A complete example: the `chain` family

This family adds two statements. `chain :x, length: 5, by: 1` in a model declares things `x1` to `x5` with each
pulling the next. `x.ends` in a run prints how often the two ends agreed in the last `settle`.

```rust file=ext_chain.rs
//! An example statement family, `chain`, used by docs/07-extending.md and compiled by tests/docs_examples.rs.
//!
//! model:  chain :x, length: 5, by: 1    declares x1 .. x5, each pulling the next by 1
//! run:    x.ends                         prints how often the two ends of the chain agreed in the last settle

use settle::ext::{Claim, Ctx, Ext};
use settle::lex::{err, kw, kwargs, num, only, SettleError, Tok};
use settle::model::{Model, State};

pub struct Chain;

/// Declare the links and their pulls, and remember the chain's length in the model's notes.
fn declare(m: &mut Model, name: &str, rest: &[Tok], ln: usize) -> Result<(), SettleError> {
    let kv = kwargs(rest, ln)?;
    only(&kv, &["length", "by"], "chain", ln)?;
    let length = kw(&kv, "length").map(|v| num(v, ln)).transpose()?.unwrap_or(5.0) as usize;
    let by = kw(&kv, "by").map(|v| num(v, ln)).transpose()?.unwrap_or(1.0);
    if length < 2 {
        return err(ln, "a chain needs at least 2 links");
    }
    let ids: Vec<usize> = (1..=length).map(|k| m.add(&format!("{}{}", name, k))).collect();
    for w in ids.windows(2) {
        m.couple(w[0], w[1], by);
    }
    m.notes.insert(format!("chain:{}", name), (vec![length as f64], Vec::new()));
    Ok(())
}

/// The fraction of recorded samples in which the first and last links agree.
fn ends(m: &Model, st: &State, name: &str, ln: usize, ctx: &mut Ctx) -> Result<(), SettleError> {
    let length = match m.notes.get(&format!("chain:{}", name)) {
        Some((v, _)) => v[0] as usize,
        None => return err(ln, format!("no chain :{} (declare it with: chain :{})", name, name)),
    };
    if st.samples.is_empty() {
        return err(ln, "ends needs a settle first");
    }
    let (a, b) = (m.need(&format!("{}1", name), ln)?, m.need(&format!("{}{}", name, length), ln)?);
    let agree = st.samples.iter().filter(|s| s[a] == s[b]).count();
    ctx.say(format!("chain :{}: ends agree in {:.1}% of {} samples", name, 100.0 * agree as f64 / st.samples.len() as f64, st.samples.len()));
    Ok(())
}

impl Ext for Chain {
    fn name(&self) -> &'static str {
        "chain"
    }

    fn statements(&self) -> &'static [&'static str] {
        &["model: chain :x, length: 5, by: 1", "run: x.ends"]
    }

    fn model_stmt(&self, m: &mut Model, t: &[Tok], ln: usize, _ctx: &mut Ctx) -> Claim {
        match t {
            [Tok::Ident(k), Tok::Sym(name), rest @ ..] if k == "chain" => {
                let rest = rest.strip_prefix(&[Tok::Comma]).unwrap_or(rest);
                Some(declare(m, name, rest, ln))
            }
            _ => None,
        }
    }

    fn run_stmt(&self, m: &mut Model, st: &mut State, t: &[Tok], ln: usize, ctx: &mut Ctx) -> Claim {
        match t {
            [Tok::Ident(name), Tok::Dot, Tok::Ident(verb)] if verb == "ends" => Some(ends(m, st, name, ln, ctx)),
            _ => None,
        }
    }
}
```

The test registers it and runs this program:

```settle file=ext_chain.program
model :m do
  chain :x, length: 6, by: 0.8     # x1 .. x6, each pulling the next by 0.8
end

run :m do
  settle 20_000, seed: 1
  x.ends                           # how often x1 and x6 agree
end
```

```text file=ext_chain.program.out
settled: 20000 samples of 6 things at temperature 1
chain :x: ends agree in 56.9% of 20000 samples
```

For a chain of pulls `J` at temperature 1 with no leans, the two ends agree with probability
`(1 + tanh(J)^(L-1)) / 2`, which is 56.5% for `J = 0.8` and `L = 6`; the sampled 56.9% is within sampling error.

The file is `docs/examples/ext_chain.rs`, and the test `the_extension_example_runs` compiles it, so this page's
code cannot drift from code that builds.

## Conventions the existing families follow

- Refuse unknown keywords with `only`, and give every keyword a default where a sensible one exists.
- Print one summary line per statement, starting with the statement's subject, in plain words.
- Report a refusal (a result that cannot be trusted) as an output line that says so, and an impossible request
  as an error.
- Take a `seed:` keyword wherever the statement uses randomness, and use the run's generator otherwise.
- Keep examples fast and add them to `docs/examples/` so the doctest keeps them honest.
