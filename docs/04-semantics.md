# Semantics

This page states what the interpreter does when it runs a program. It covers the core model and the core
sampler, which every family builds on. Families that add their own state (grids, memories, learned machines,
real-valued numbers) describe it on their own pages. The source for this page is `src/interp.rs`,
`src/model.rs` and `src/core.rs`.

## Execution order

The interpreter executes the file from top to bottom, one line at a time, in a single pass.

- A model statement changes the model as soon as it is read.
- A run statement acts on the model as it stands at that point in the file.
- A model block that appears after a run block changes the model for later run blocks only.
- Some run statements change the model (for example `learn` fits pulls, and `m.save` stores a memory). Those
  changes persist: later run blocks see the changed model.

Output lines are collected in order and printed when the whole file has run without error. If any line fails,
nothing is printed except the error message (see [Install and run](01-install-and-run.md)).

## The model

A model is created the first time `model :name do` appears. It holds:

- **Things**, in the order they were declared. Each has a name and a value that is either yes (+1) or no (-1)
  in any given arrangement. `thing :a` declares `a`; declaring an existing thing again does not create a second
  one.
- A **lean** for each thing, a real number `h`. `leans: :yes, by: b` adds `+b` to `h`; `leans: :no, by: b` adds
  `-b`. Leans accumulate: two declarations with leans add up. A thing declared without a lean has `h = 0`.
- A **pull** for each pair of things, a real number `J`. `a.pulls :b, by: w` adds `+w` to the pull between `a`
  and `b`, and `a.pushes :b, by: w` adds `-w`. Pulls are symmetric (`a.pulls :b` and `b.pulls :a` change the same
  number) and accumulate. A thing cannot pull itself. Pairs never mentioned have a pull of zero, and the model
  stores only the non-zero pairs, so a model can hold tens of thousands of things.
- **Notes**: named storage that statement families use to keep their own structures on the model (a grid's
  size, a memory's patterns, a learned machine). Notes do not affect sampling directly; the families that write
  them also write the leans and pulls they need.

Other families declare things in bulk. For example `grid :img, width: 64, height: 48` declares one thing per
pixel. They are ordinary things with names the family chooses.

## Energy and probability

An arrangement `s` gives every thing a value `s_i` in {+1, -1}. Its energy is

```text
E(s) = - sum_i h_i s_i  -  sum_{i<k} J_ik s_i s_k
```

Reading: each thing lowers the energy by its lean when it points the way it leans, and each pair lowers the
energy by its pull when it agrees (or raises it, for a positive pull, when it disagrees).

At temperature `T`, the distribution the sampler draws from gives each arrangement the probability

```text
P(s) = exp(-E(s) / T) / Z,      Z = sum over all arrangements of exp(-E(s) / T)
```

Reading: calmer arrangements are exponentially more likely, and the temperature sets how strongly. At a low
temperature almost all the probability sits on the calmest arrangements; at a high temperature every arrangement
is nearly equally likely. [The science](10-the-science.md) names this distribution and gives references.

## A run

`run :name do` creates a fresh **run state** for the model. The run state holds:

| Part | Starts as | Changed by |
|---|---|---|
| Held things and their values | none | `hold` |
| Temperature | 1 | `temperature:` on `settle` or `anneal` |
| Random number generator | seeded with 24301 (hexadecimal `5eed`) | `seed:` on `settle` or `anneal`, and on many family statements |
| Recorded samples and yes-counts | none | `settle` (replaces them) |
| Calmest arrangement found | none | `anneal` |

The run state lasts until the block's `end`. The next run block, even for the same model, starts again from the
table above. The model itself is not reset.

Consequences:

- Every run block begins with the same seed, so a program without `seed:` arguments prints the same numbers
  every time it is run, and two identical run blocks print the same numbers.
- `temperature:` and `seed:` persist within a run block. After `settle 1_000, temperature: 2`, a later
  `settle 1_000` in the same block also samples at temperature 2.
- `seed: n` replaces the generator with a new one seeded with `n` (converted to a whole number; a negative
  seed becomes 0). Later statements continue from that generator's state.
- `hold` lasts for the rest of the run block. There is no statement to release a hold; start a new run block
  instead.

```settle example=sem-seed
# Each run block starts from the same default seed, so these two runs print the same numbers.
model :coin do
  thing :c
end

run :coin do
  settle 1_000
  ask :c
end

run :coin do
  settle 1_000
  ask :c
  settle 1_000, seed: 2            # a different seed gives a different sample
  ask :c
end
```

```text output=sem-seed
settled: 1000 samples of 1 things at temperature 1
ask :c: yes 50.7% of 1000 samples
settled: 1000 samples of 1 things at temperature 1
ask :c: yes 50.7% of 1000 samples
settled: 1000 samples of 1 things at temperature 1
ask :c: yes 48.9% of 1000 samples
```

## Settling

`settle N` draws samples by Gibbs sampling (also called Glauber dynamics, or the p-bit rule). Precisely:

1. **Start.** Every thing that is not held gets a random value, yes or no with equal chance. Held things get
   their held value.
2. **Sweep.** A sweep visits every free thing once, in a fresh random order (a uniformly random permutation per
   sweep). When thing `i` is visited, its input from the current arrangement is

   ```text
   I_i = h_i + sum_k J_ik s_k
   ```

   and the thing is set to yes with probability

   ```text
   P(s_i = yes) = (1 + tanh(I_i / T)) / 2
   ```

   Reading: the stronger the combined push of its lean and its neighbours, the more surely a thing follows it.
   This is the exact conditional probability of `s_i` given all the other things under `P(s)`, so repeated
   sweeps leave `P(s)` unchanged. The implementation draws `r` uniformly from [-1, 1) and sets yes when
   `tanh(I_i / T) > r`, which has that probability.
3. **Burn in.** The first `max(1, N / 10)` sweeps (whole-number division) are discarded, so the recorded samples
   do not depend much on the random start.
4. **Record.** Then `N` more sweeps are run, and the arrangement after each is recorded as one sample.

`settle` replaces any samples recorded by an earlier `settle` in the same block and prints

```text
settled: <N> samples of <things> things at temperature <T>
```

Consecutive samples come from consecutive sweeps, so they are correlated. An estimate from `N` samples is
therefore less precise than one from `N` independent draws; how much less depends on the model. Strong pulls
and low temperatures slow the sampler down, and a model with several deep valleys may stay in one of them for
the whole run. The [valleys](05-statements/valleys.md) family measures this.

### The sample budget

If `(number of things) x N` is more than 20,000,000, `settle` keeps only a count of how often each thing was yes,
not the full arrangements. `show` still works; `ask` refuses with
`this settle was too large to keep every sample; ask needs a smaller one`.

## Annealing

`anneal N` searches for the calmest arrangement instead of sampling the distribution.

1. It starts from a random arrangement, as `settle` does, with held things held.
2. It runs `N` sweeps. Sweep `k` (counting from 0) uses the temperature

   ```text
   T_k = 10 T x 0.005^( k / (N - 1) )
   ```

   so the temperature falls geometrically from 10 times the run's temperature to one twentieth of it (for
   `N = 1` the single sweep uses `10 T`).
3. After each sweep it computes the energy, and it keeps the lowest-energy arrangement seen, including the
   starting one.

It prints `annealed: <N> sweeps, calmest energy found <E>` with the energy to three decimals, and `best` then
prints that arrangement. Annealing does not record samples: `show` and `ask` still refer to the last `settle`.
Annealing is a heuristic. It usually finds a calm arrangement, but it does not guarantee the calmest one.

## Temperature

The temperature must be above zero. It divides every lean and pull, so doubling all leans and pulls has the
same effect as halving the temperature.

```settle example=sem-temperature
# The same model sampled at a low and a high temperature.
model :pair do
  thing :a, leans: :yes, by: 1
  thing :b
  a.pulls :b, by: 1
end

run :pair do
  settle 20_000, temperature: 0.25, seed: 1   # cold: the calm arrangement dominates
  ask :a, and: :b
  settle 20_000, temperature: 4, seed: 1      # hot: close to a fair coin for each thing
  ask :a, and: :b
end
```

```text output=sem-temperature
settled: 20000 samples of 2 things at temperature 0.25
ask :a, and: :b: yes 100.0% of 20000 samples
settled: 20000 samples of 2 things at temperature 4
ask :a, and: :b: yes 39.3% of 20000 samples
```

## What `show` and `ask` report

`show` prints one line per thing, in declaration order: the name padded to 14 characters, a bar of
`round(30 p)` `#` characters, the percentage `100 p` to one decimal, and `(held)` for a held thing. Here `p` is the
fraction of recorded samples in which the thing was yes.

`ask :a` reports the fraction of recorded samples in which `a` was yes. Further terms combine another thing with
the answer so far, left to right:

| Term | Keeps a sample when |
|---|---|
| `and: :b` | the answer so far is true and `b` is yes |
| `or: :b` | the answer so far is true or `b` is yes |
| `and_not: :b` | the answer so far is true and `b` is no |
| `or_not: :b` | the answer so far is true or `b` is no |

So `ask :a, or: :b, and: :c` is `(a or b) and c`. The answer is a fraction of samples, which estimates the
probability of that event under `P(s)` with any held things fixed; it is a conditional probability given the
holds. `ask` prints `ask <terms>: yes <percent>% of <samples> samples`.

## Re-opening a model

A model block may be opened again later in the file. Its statements add to the same model.

```settle example=sem-reopen
# A model block may be opened again; its statements add to the same model.
model :m do
  thing :a
end

model :m do
  thing :b
  a.pulls :b, by: 2
end

run :m do
  hold :a, :yes
  settle 10_000, seed: 1
  show
end
```

```text output=sem-reopen
settled: 10000 samples of 2 things at temperature 1
  a              ############################## 100.0%  (held)
  b              ############################# 98.0%
```

## Determinism

Given the same program, the same input files and the same interpreter build, the output is the same on every
run: all randomness comes from the run state's generator (an xorshift64* generator in `src/rng.rs`), and every
run block starts from a fixed seed. Families that print wall-clock timings are the one exception; those numbers
vary. Results may differ in the last printed digit between machines whose maths libraries compute `tanh` or
`exp` differently.
