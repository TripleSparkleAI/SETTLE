# A tour of SETTLE

This page walks through the language with three short programs. It takes about ten minutes. Each program is
followed by its exact output.

## Things, leans and pulls

A SETTLE program has two kinds of block.

- A **model block**, `model :name do ... end`, describes a probability distribution. It declares things (yes/no
  variables), how each thing leans, and how pairs of things pull on each other.
- A **run block**, `run :name do ... end`, works with the model of the same name. It can fix some things, draw
  samples from the distribution, and ask questions about the samples.

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

```text output=tour-first
settled: 20000 samples of 2 things at temperature 1
  a              ###################### 73.5%
  b              #################### 68.0%
ask :b: yes 68.0% of 20000 samples
ask :a, and: :b: yes 64.9% of 20000 samples
```

Line by line:

- `thing :a, leans: :yes, by: 0.5` declares the thing `a` and gives it a lean of +0.5 towards yes. A thing
  declared without `leans:` has no lean.
- `a.pulls :b, by: 1` adds a pull of strength 1 between `a` and `b`. A pull makes arrangements where the two
  things agree more likely. `a.pushes :b, by: 1` would do the opposite. The receiver (`a`) is written without a
  colon; the argument (`:b`) is a symbol.
- `settle 20_000, seed: 1` draws 20,000 arrangements of every thing, using a random number generator seeded
  with 1. The line it prints reports the number of samples, things and the temperature.
- `show` prints the fraction of samples in which each thing was yes, as a bar of up to 30 `#` characters and a
  percentage.
- `ask :b` prints the fraction of samples in which `b` was yes. `ask :a, and: :b` prints the fraction in which
  both were yes.

The numbers come from sampling, so they are estimates. With the same seed they are the same every time the
program runs.

## Why the numbers are what they are

Every arrangement of the things has an **energy**. Each thing's lean lowers the energy when the thing points the
way it leans, and each pull lowers the energy when the pair agrees:

```text
energy = - (sum of lean x value over things) - (sum of pull x value x value over pulled pairs)
```

where a thing's value is +1 for yes and -1 for no. Settling draws arrangements with probability proportional to
`exp(-energy / temperature)`. Low-energy (calm) arrangements come up often and high-energy ones rarely. Here
`a` leans yes and pulls `b` along with it, so both are yes more often than not. The exact rules, including how
the drawing works, are in [Semantics](04-semantics.md).

## Holding a thing

`hold` fixes a thing at yes or no for the rest of the run block. The other things are sampled with that thing
fixed. This is how you condition on an observation.

```settle example=tour-hold
# Holding a thing fixes it at one value; the others respond.
model :pair do
  thing :a
  thing :b
  a.pulls :b, by: 1
end

run :pair do
  hold :a, :no                     # a is fixed at no
  settle 20_000, seed: 1
  show
end
```

```text output=tour-hold
settled: 20000 samples of 2 things at temperature 1
  a               0.0%  (held)
  b              #### 12.1%
```

With `a` held at no, `b` is pulled towards no and is yes only about 12% of the time. The exact value is
`1 / (1 + e^2)`, about 11.9%.

## Finding the calmest arrangement

`settle` answers "how likely is each arrangement?". `anneal` answers a different question: "which arrangement is
calmest?". It samples while lowering the temperature step by step, and keeps the lowest-energy arrangement it
visits. `best` prints that arrangement.

```settle example=tour-anneal
# Three things that each want to disagree with the other two cannot all get their way.
model :triangle do
  thing :x, :y, :z
  x.pushes :y, by: 1
  y.pushes :z, by: 1
  z.pushes :x, by: 1
  thing :x, leans: :yes, by: 0.1   # a small lean breaks the tie
end

run :triangle do
  anneal 2_000, seed: 3            # cool slowly and keep the calmest arrangement
  best                             # print it with its energy
end
```

```text output=tour-anneal
annealed: 2000 sweeps, calmest energy found -1.100
best (energy -1.100): x yes, y no, z yes
```

Declaring `:x` a second time does not create a new thing. It adds the new lean to the existing one.

## Beyond the core

The statements above belong to the **core** family. Every other family adds statements that build larger
models or read results in new ways, but they all rest on the same things, leans, pulls and sampler:

| Family | What it adds |
|---|---|
| [memory](05-statements/memory.md) | Store patterns and text in the pulls; recall them from a noisy read-address. |
| [grid](05-statements/grid.md) | One thing per pixel; read PGM pictures, write them, play a folder of frames. |
| [colour](05-statements/colour.md) | Three grids per colour picture; PPM in and out. |
| [zoo](05-statements/zoo.md), [zootemp](05-statements/zootemp.md) | Puzzles as models: sudoku, graph colouring, max-cut, factoring; annealing schedules for them. |
| [learn](05-statements/learn.md) | Fit leans and pulls from examples (Boltzmann machine learning); classify by Settling. |
| [denoise](05-statements/denoise.md) | A chain of small machines that learns to turn noise into examples. |
| [valleys](05-statements/valleys.md) | Map the landscape of calm arrangements, exactly or by survey. |
| [numbers](05-statements/numbers.md) | Real-valued things on springs; solve linear systems by Settling. |
| [sdm](05-statements/sdm.md), [softsdm](05-statements/softsdm.md), [sdmscale](05-statements/sdmscale.md), [sdmrefuse](05-statements/sdmrefuse.md), [sdmtrack](05-statements/sdmtrack.md) | Kanerva sparse distributed memory in several forms, and predictions of how well it recalls. |
| [coded](05-statements/coded.md) | Compress and error-code text before storing it in a memory. |
| [ldpcsettle](05-statements/ldpcsettle.md), [ldpcmoves](05-statements/ldpcmoves.md) | Decode LDPC error-correcting codes by Settling. |
| [export](05-statements/export.md) | Write a model for other solvers (Ising, QUBO, Gset, DIMACS) and read one back. |

## Where to go next

- [Syntax](03-syntax.md) for the exact rules of what can be written.
- [Semantics](04-semantics.md) for what the interpreter does with it.
- [Cookbook](08-cookbook.md) for recipes that use the other families.
