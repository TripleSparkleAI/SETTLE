# The core family

The core family declares things, leans and pulls, and runs the sampler. Every other family builds on the model
these statements create. Source: `src/core.rs`, with the model and the sampler in `src/model.rs`. The meaning of
energy, temperature, sweeps and samples is set out in [Semantics](../04-semantics.md); this page gives each
statement's exact form.

| Statement | Block | Summary |
|---|---|---|
| [`thing`](#thing) | model | Declare things, optionally with a lean. |
| [`a.pulls` / `a.pushes`](#apulls-and-apushes) | model | Add a pull or a push between two things. |
| [`hold`](#hold) | run | Fix a thing at yes or no for the rest of the run block. |
| [`settle`](#settle) | run | Draw samples by Gibbs sampling. |
| [`anneal`](#anneal) | run | Search for the calmest arrangement by cooling. |
| [`show`](#show) | run | Print each thing's yes-rate over the last `settle`. |
| [`best`](#best) | run | Print the calmest arrangement the last `anneal` found. |
| [`ask`](#ask) | run | Print the fraction of samples in which a combination holds. |

## `thing`

**Block:** model.

**Form:**

```text
thing :a
thing :a, :b, :c
thing :a, :b, leans: :yes, by: 1
```

**Arguments:**

| Argument | Type | Default | Meaning |
|---|---|---|---|
| names | one or more symbols, separated by commas | required | The things to declare. |
| `leans:` | `:yes` or `:no` | none | The direction of the lean. |
| `by:` | number | none | The strength of the lean. |

**What it does:** declares each named thing that does not exist yet, in order, with a lean of zero. A thing that
already exists is not declared again. If `leans:` and `by:` are given, it adds `+by` (for `:yes`) or `-by` (for
`:no`) to the lean of every named thing, new or existing. Leans accumulate across declarations. `leans:` and
`by:` must be given together.

A thing's name is any symbol except `:yes` and `:no`. To use a thing as the receiver of a statement such as
`a.pulls`, its name must also be a valid identifier (start with a letter or an underscore).

**Output:** none.

**Example:**

```settle example=core-leans-and-pulls
# leans and pulls add up when they are declared more than once
model :m do
  thing :a, :b, :c, leans: :yes, by: 0.5   # one lean given to three things at once
  thing :a, leans: :no, by: 1              # a's lean is now 0.5 - 1 = -0.5
  a.pulls :b, by: 2
  b.pushes :a, by: 1.5                     # the same pair: the pull is now 2 - 1.5 = 0.5
  b.pushes :c, by: 1
end

run :m do
  settle 20_000, seed: 2
  show
end
```

Output:

```text output=core-leans-and-pulls
settled: 20000 samples of 3 things at temperature 1
  a              ######### 29.4%
  b              ############## 45.3%
  c              ################### 64.5%
```

**Errors:**

- `:yes cannot be a thing name` (and the same for `:no`)
- `unexpected <token> in thing` (something other than a symbol or a comma before the keywords)
- `` thing does not take `<key>:` ``
- `use `leans:` and `by:` together`
- `expected :yes or :no` (the value of `leans:`)
- `a number was expected` (the value of `by:`)

```settle example=core-lean-needs-by
model :m do
  thing :a, leans: :yes          # a lean needs a strength
end
```

```text error=core-lean-needs-by
line 2: use `leans:` and `by:` together
```

## `a.pulls` and `a.pushes`

**Block:** model.

**Form:**

```text
a.pulls :b, by: W
a.pushes :b, by: W
```

**Arguments:**

| Argument | Type | Default | Meaning |
|---|---|---|---|
| `a` | identifier naming a thing | required | The first thing. |
| `:b` | symbol naming a thing | required | The second thing. |
| `by:` | number | required | The strength. |

**What it does:** `pulls` adds `+W` to the pull between `a` and `b`; `pushes` adds `-W`. The pull is symmetric,
so `a.pulls :b` and `b.pulls :a` change the same number. Repeated statements on the same pair add up. Both
things must already be declared, and they must be different. A negative `W` is allowed: `a.pulls :b, by: -1`
is the same as `a.pushes :b, by: 1`.

**Output:** none.

**Example:** the `core-leans-and-pulls` example above, and `core-weather` below.

**Errors:**

- `unknown thing :<name> (declare it with: thing :<name>)`
- `a thing cannot pull itself`
- `` pulls does not take `<key>:` `` (or `pushes ...`)
- `` pulls needs `by:` `` (or `` pushes needs `by:` ``)
- `a number was expected`

## `hold`

**Block:** run.

**Form:**

```text
hold :a, :yes
hold :a, :no
```

**Arguments:**

| Argument | Type | Default | Meaning |
|---|---|---|---|
| `:a` | symbol naming a thing | required | The thing to fix. |
| value | `:yes` or `:no` | required | The value to fix it at. |

**What it does:** fixes the thing at the given value for the rest of the run block. `settle` and `anneal` start
held things at their held value and never update them, so the other things are sampled conditioned on the hold.
Holding a thing again replaces its held value. There is no statement that releases a hold; a new run block
starts with no holds. The statement must have exactly this shape; any other shape is not recognised.

**Output:** none. `show` marks held things with `(held)`.

**Example:** `core-weather` below.

**Errors:**

- `unknown thing :<name> (declare it with: thing :<name>)`
- `expected :yes or :no`

## `settle`

**Block:** run.

**Form:**

```text
settle N, temperature: 1, seed: 24301
```

**Arguments:**

| Argument | Type | Default | Meaning |
|---|---|---|---|
| `N` | whole number | required | Number of samples to record (one per sweep). |
| `temperature:` | number above zero | the run's current temperature (1 at the start of a run block) | Sets the run's temperature, for this and later statements. |
| `seed:` | whole number | the run's current generator (seeded 24301 at the start of a run block) | Replaces the run's random generator with one seeded by this number. |

**What it does:** starts every free thing at a random value, runs `max(1, N / 10)` burn-in sweeps that are not
recorded, then runs `N` sweeps and records the arrangement after each. Each sweep updates every free thing once
in a fresh random order with the Gibbs rule. The recorded samples replace those of any earlier `settle` in the
block. If `(things) x N` is more than 20,000,000, only per-thing yes-counts are kept, so `ask` refuses
afterwards. Full detail: [Semantics](../04-semantics.md#settling).

`temperature:` and `seed:` change the run state, so they also apply to later statements in the same run block.

**Output:**

```text
settled: <N> samples of <things> things at temperature <T>
```

**Example:**

```settle example=core-weather
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

Output:

```text output=core-weather
settled: 20000 samples of 3 things at temperature 1
  rain           ################### 64.2%
  sprinkler      ################### 63.0%
  wet_grass      ############################## 100.0%  (held)
ask :rain: yes 64.2% of 20000 samples
ask :rain, and: :sprinkler: yes 31.6% of 20000 samples
ask :rain, or: :sprinkler: yes 95.6% of 20000 samples
ask :sprinkler, and_not: :rain: yes 31.4% of 20000 samples
```

The next example shows what `seed:` and `temperature:` keep for later statements:

```settle example=core-seed
# what seed: and temperature: keep for later statements in the same run block
model :pair do
  thing :a, :b
  a.pulls :b, by: 1
end

run :pair do
  settle 1_000                  # seed 0x5eed (24301) and temperature 1, the defaults
  ask :a, and: :b
  settle 1_000                  # the generator continues, so this differs
  ask :a, and: :b
  settle 1_000, seed: 24301     # a fresh generator with the default seed repeats the first settle
  ask :a, and: :b
  settle 1_000, temperature: 3  # temperature 3 stays for the rest of this block
  ask :a, and: :b
  settle 1_000                  # still at temperature 3
end

run :pair do
  settle 1_000                  # a new run block starts again from seed 0x5eed and temperature 1
  ask :a, and: :b
end
```

Output:

```text output=core-seed
settled: 1000 samples of 2 things at temperature 1
ask :a, and: :b: yes 39.9% of 1000 samples
settled: 1000 samples of 2 things at temperature 1
ask :a, and: :b: yes 42.1% of 1000 samples
settled: 1000 samples of 2 things at temperature 1
ask :a, and: :b: yes 39.9% of 1000 samples
settled: 1000 samples of 2 things at temperature 3
ask :a, and: :b: yes 34.3% of 1000 samples
settled: 1000 samples of 2 things at temperature 3
settled: 1000 samples of 2 things at temperature 1
ask :a, and: :b: yes 39.9% of 1000 samples
```

**Errors:**

- `` settle does not take `<key>:` ``
- `temperature must be above zero`
- `a number was expected`

## `anneal`

**Block:** run.

**Form:**

```text
anneal N, temperature: 1, seed: 24301
```

**Arguments:**

| Argument | Type | Default | Meaning |
|---|---|---|---|
| `N` | whole number | required | Number of sweeps. |
| `temperature:` | number above zero | the run's current temperature | Sets the run's temperature `T`; the schedule runs from `10 T` to `T / 20`. |
| `seed:` | whole number | the run's current generator | Replaces the run's random generator. |

**What it does:** starts from a random arrangement (held things held) and runs `N` sweeps, sweep `k` at
temperature `10 T x 0.005^(k / (N - 1))`. After every sweep it computes the energy and keeps the lowest-energy
arrangement seen, including the start. It stores that arrangement for `best`. It does not record samples, so
`show` and `ask` are not affected. Annealing is a search heuristic; it does not guarantee the global minimum.

**Output:**

```text
annealed: <N> sweeps, calmest energy found <E>
```

with `<E>` to three decimals.

**Example:**

```settle example=core-anneal
# five things in a ring that all push their neighbours: no arrangement satisfies every push
model :ring do
  thing :a, :b, :c, :d, :e
  thing :b, leans: :yes, by: 0.2      # declaring :b again only adds a lean
  a.pushes :b, by: 1
  b.pushes :c, by: 1
  c.pushes :d, by: 1
  d.pushes :e, by: 1
  e.pushes :a, by: 1
  a.pulls :c, by: 0.3
end

run :ring do
  anneal 4_000, seed: 5               # cool from 10x to 1/20 of the temperature
  best                                # the calmest arrangement visited
end
```

Output:

```text output=core-anneal
annealed: 4000 sweeps, calmest energy found -3.500
best (energy -3.500): a no, b yes, c no, d yes, e no
```

**Errors:** as for `settle`: `` anneal does not take `<key>:` ``, `temperature must be above zero`,
`a number was expected`.

## `show`

**Block:** run.

**Form:**

```text
show
```

**What it does:** prints one line per thing, in declaration order, from the counts of the last `settle` in this
run block. Each line has the name padded to 14 characters, `round(30 p)` `#` characters, the percentage `100 p`
to one decimal, and `(held)` for a held thing, where `p` is the fraction of samples in which the thing was yes.
It works after a `settle` of any size, including one too large for `ask`.

**Output:**

```text
  <name padded to 14> <bar> <percent>%[  (held)]
```

**Example:** `core-weather` above. `show` after only an `anneal` is an error, because an anneal records no
samples:

```settle example=core-show-before-settle
# show reads the samples of the last settle; an anneal does not make any
model :m do
  thing :a
end

run :m do
  anneal 100
  show
end
```

```text error=core-show-before-settle
line 8: show needs a settle first
```

**Errors:** `show needs a settle first`.

## `best`

**Block:** run.

**Form:**

```text
best
```

**What it does:** prints the calmest arrangement found by the last `anneal` in this run block, with its energy.

**Output:**

```text
best (energy <E>): <name> yes|no, <name> yes|no, ...
```

**Example:** `core-anneal` above.

**Errors:** `best needs an anneal first`.

## `ask`

**Block:** run.

**Form:**

```text
ask :a
ask :a, and: :b, or: :c, and_not: :d, or_not: :e
```

**Arguments:**

| Argument | Type | Default | Meaning |
|---|---|---|---|
| `:a` | symbol naming a thing | required | The first term: the samples in which `a` is yes. |
| `and:` | symbol naming a thing | none | Keep samples where the answer so far is true and this thing is yes. |
| `or:` | symbol naming a thing | none | Keep samples where the answer so far is true or this thing is yes. |
| `and_not:` | symbol naming a thing | none | Keep samples where the answer so far is true and this thing is no. |
| `or_not:` | symbol naming a thing | none | Keep samples where the answer so far is true or this thing is no. |

**What it does:** evaluates the expression on every recorded sample of the last `settle`, combining terms
strictly left to right with no precedence, and prints the fraction of samples in which it is true. Unlike most
keyword arguments, these may repeat and their order matters: `ask :a, and: :c, or: :b` is `(a and c) or b`.
The answer estimates a probability under the model with any held things fixed.

**Output:**

```text
ask :<a>[, <key>: :<thing> ...]: yes <percent>% of <samples> samples
```

**Example:**

```settle example=core-ask
# the four ask combinators, applied left to right
model :m do
  thing :a, leans: :yes, by: 0.5
  thing :b, leans: :no,  by: 0.5
  thing :c
  a.pulls :c, by: 1
end

run :m do
  settle 5_000, seed: 3
  ask :a                          # a
  ask :a, and: :b                 # a and b
  ask :a, or: :b                  # a or b
  ask :a, and_not: :b             # a and not b
  ask :a, or_not: :b              # a or not b
  ask :a, and: :c, or: :b         # (a and c) or b
  ask :a, and: :b, and: :c        # a key may repeat
end
```

Output:

```text output=core-ask
settled: 5000 samples of 3 things at temperature 1
ask :a: yes 71.3% of 5000 samples
ask :a, and: :b: yes 19.0% of 5000 samples
ask :a, or: :b: yes 79.3% of 5000 samples
ask :a, and_not: :b: yes 52.4% of 5000 samples
ask :a, or_not: :b: yes 92.1% of 5000 samples
ask :a, and: :c, or: :b: yes 73.1% of 5000 samples
ask :a, and: :b, and: :c: yes 16.8% of 5000 samples
```

**Errors:**

- `ask needs a settle first`
- `this settle was too large to keep every sample; ask needs a smaller one`
- `unknown thing :<name> (declare it with: thing :<name>)`
- `` ask terms are symbols, like `and: :sprinkler` ``
- `` ask takes and: / or: / and_not: / or_not:, not `<key>:` ``

## Notes

- The core family is first in the registry, so its patterns are tried before any other family's. A line that
  looks like a core statement but has the wrong shape (for example `hold :a` with no value) falls through to
  the other families and, if none claims it, is reported as `no statement family knows this line`.
- Counts are converted from numbers by truncation: `settle 2.9` records 2 samples, and a negative count is 0.
