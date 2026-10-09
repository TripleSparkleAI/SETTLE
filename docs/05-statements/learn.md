# The learn family

The learn family fits a model's leans and pulls to examples, then uses the fitted model as a classifier. A set
of examples is a list of rows, and each row gives a yes or no value to every thing the set covers. `learn`
changes the leans and pulls so that the model, when it settles, produces arrangements like the rows (the
Boltzmann machine learning rule). `classify` holds the input things of each row, settles the model, and reads
which label thing comes out yes. Optional hidden things turn the model into a restricted Boltzmann machine.

The source is `src/learn.rs`. The measurements are in
`SETTLE/runs/boltzlearn/REPORT_BOLTZLEARN.md`: on binarised 8x8 handwritten digits, a machine
with 64 hidden things classified 93.8% of the test digits correctly on average over three splits, against
91.2% for logistic regression. The denoise family builds on this family; its report is
`SETTLE/runs/boltzlearn2/REPORT_BOLTZLEARN2.md`.

| Statement | Block | Summary |
|---|---|---|
| [`examples`](#examples) | both | declare a set of examples from a file or inline, or add rows to one |
| [`hidden`](#hidden) | model | add hidden things, making a restricted machine |
| [`learn`](#learn) | run | fit leans and pulls to a set of examples |
| [`classify`](#classify) | run | hold the inputs of each row, settle, and read the label |
| [`shuffle`](#shuffle) | both | scramble which labels go with which row |

## The machine being learned

Things are yes (+1) or no (-1). With leans `h` and pulls `W`, the energy of an arrangement `s` is:

```text
E(s) = - sum_i h_i s_i - sum_{i<j} W_ij s_i s_j
P(s) = exp(-E(s) / T) / Z
```

An arrangement is likely when things agree with their leans and pulled pairs agree with each other. `T` is the
run's temperature (1 unless the run changed it), and `beta = 1 / T`.

`learn` works on a part of the model: the things the examples cover (the visible things) and the model's
hidden things, if it has any. It ignores every other thing of the model. It reads the current leans and pulls
of that part from the model, so a second `learn` continues from where the first stopped.

- **Without hidden things** (a fully visible machine), the learnable pulls are every pair of visible things.
- **With hidden things** (a restricted machine), the learnable pulls are every pair of one visible thing and
  one hidden thing. Pulls between visible things are not learned, but any that exist stay in the energy.
  Pulls between hidden things are not allowed.

In both cases the leans of every visible and hidden thing are learned.

A hidden thing is pulled only by visible things, so it can be summed out exactly. For a visible arrangement
`v`, the input to hidden thing `k` is `x_k(v) = h_k + sum_i W_ik v_i`, and:

```text
-beta F(v) = beta * (sum_i h_i v_i + sum_{i<j visible} W_ij v_i v_j) + sum_k ln(2 cosh(beta x_k(v)))
E[z_k | v] = tanh(beta x_k(v))
```

`-beta F(v)` is the log of the unnormalised chance of the visible arrangement `v`, with the hidden things
summed out. The second line is the expected value of hidden thing `k` given `v`.

## The examples file format

A file of examples is plain text, read in full when the `examples` statement runs.

- On each line, everything from the first `#` onward is a comment and is removed. The rest of the line is
  trimmed. Lines that are then empty are skipped.
- The first remaining line names the things. Names are separated by spaces, tabs or commas. A name is written
  without a colon.
- Every later line is one row. Spaces, tabs and commas in a row are ignored. Every other character is one
  cell:
  - `1`, `+`, `y` or `Y` means yes;
  - `0`, `-`, `n` or `N` means no;
  - any other character is an error.
- Every row must have exactly one cell per name, in the order of the names.
- A file with no names line is an error. A file with a names line and no rows gives an empty set.
- Errors inside the file report the line of the `examples` statement, not the line of the file.

This is the file the examples below use, `docs/examples/data/learn-same.txt`:

```text
# Two input bits and a one-hot label: l_same is on when the bits agree,
# l_diff when they differ. The first line that is not a comment names the things.
x1 x2 l_same l_diff
0 0 1 0
0 1 0 1
1 0 0 1
1 1 1 0   # a comment may follow a row
```

The inline form uses the same cells. In `rows:`, rows are separated by spaces or commas, so each row is written
as one word with no spaces inside it, like `"0010 0101"`.

## `examples`

**Block:** both.

**Form:**

```text
examples :name, "path/to/file.txt"
examples :name, over: "a b c", rows: "011 101"
examples :name, rows: "110"
```

**Arguments:**

| Argument | Type | Default | Meaning |
|---|---|---|---|
| `:name` | symbol | required | the name of the set |
| file | path string | none | a file in the format above; a relative path is read from the program file's folder |
| `over:` | string | required for a new inline set | the names of the things the set covers, separated by spaces or commas |
| `rows:` | string | no rows | rows to add, separated by spaces or commas, each one word of cells |

The file form takes nothing after the path. The inline form takes only `over:` and `rows:`.

**What it does:**

- **File form:** reads the file and creates the set. The set must not exist yet.
- **Inline form with `over:`:** creates a new set covering the named things, with the rows of `rows:`. The set
  must not exist yet. `over:` alone creates a set with no rows.
- **Inline form without `over:`:** adds the rows of `rows:` to an existing set.

In every form, each name the set covers becomes a thing of the model, declared if it does not exist yet. A
name may not appear twice in one set, and `yes` and `no` cannot be thing names.

The set is kept in the model's notes under the key `examples:<name>`: the rows' cells (+1 or -1) one row after
another, with the names as the words. The notes belong to the model, so rows added in a run block stay for
later run blocks of the same model.

**Output:**

```text
examples :<name> from <path>, <rows> in all, over <n> things
examples :<name> inline: <added> rows added, <rows> in all, over <n> things
```

`<path>` is printed as written in the program. `<rows>` counts every row of the set after the statement.

**Examples:** an inline set, with two more rows added in the run.

```settle example=learn-inline
# Inline examples: name the things with over:, give each row as a string of 1 and 0.
model :pairs do
  examples :d, over: "a b c", rows: "110 110 001 001"
end

run :pairs do
  examples :d, rows: "111 000"                                     # add two more rows in the run
  learn :d, rounds: 200, rate: 0.1, sweeps: 1, batch: 2, seed: 1   # contrastive divergence, the default
end
```

Output:

```text output=learn-inline
examples :d inline: 4 rows added, 4 in all, over 3 things
examples :d inline: 2 rows added, 6 in all, over 3 things
learned :d by contrastive divergence over 200 rounds of 6 rows: 3 visible, 0 hidden, 3 pulls, <time>s; exact log-likelihood per example -1.4288
```

A row with the wrong number of cells is the error a user is most likely to meet:

```settle example=learn-bad-row
# Every row must have one cell per thing named in over:.
model :m do
  examples :d, over: "a b c", rows: "110 01"
end
```

```text error=learn-bad-row
line 3: a row has 2 cells but the examples cover 3 things
```

The file form is used in the example under [`classify`](#classify).

**Errors:**

- `examples from a file take nothing after the file name`
- `examples :<name> already exist` (file form)
- `examples :<name> already exist; add rows with `rows:` alone` (inline form with `over:`)
- `new examples :<name> need `over:` naming their things`
- `` `over:` names no things ``
- `cannot read <path>: <reason>`
- `the examples file is empty`
- `a row has <k> cells but the examples cover <n> things`
- `'<c>' is not a yes/no cell (use 1/0, +/-, y/n)`
- `:<thing> is named twice in examples :<name>`
- `:yes cannot be a thing name` and `:no cannot be a thing name`
- `` examples does not take `<key>:` ``
- `a "quoted" string was expected` (for `over:` or `rows:`)

## `hidden`

**Block:** model.

**Form:**

```text
hidden N
```

**Arguments:**

| Argument | Type | Default | Meaning |
|---|---|---|---|
| `N` | whole number from 1 to 4096 | required | how many hidden things to add |

**What it does:** declares `N` things named `hidden_0` to `hidden_<N-1>` and records them as the model's
hidden things, in the note `learn:hidden` (the index of the first one and the count). A model has at most one
set of hidden things. The hidden things are ordinary things of the model: `settle`, `show` and the other
statements see them. Every `learn` and `classify` in the model uses them, whichever set of examples it names.

With hidden things, `learn` learns the pulls between visible and hidden things only. When all of those pulls
are zero at the start of a `learn`, it first sets each one to `0.1` times a standard normal draw from the
run's random generator. At zero every hidden thing would look alike, and the gradient could not tell them
apart.

**Output:** none.

The example under [`classify`](#classify) uses `hidden`.

**Errors:**

- `hidden things are already declared in this model`
- `hidden takes a whole number from 1 to 4096`

## `learn`

**Block:** run.

**Form:**

```text
learn :name, rounds: 100, rate: 0.05, method: :contrastive, sweeps: 1, batch: 50, decay: 0, seed: 24301
```

**Arguments:**

| Argument | Type | Default | Meaning |
|---|---|---|---|
| `:name` | symbol | required | the set of examples to fit |
| `rounds:` | whole number | `100` | how many passes over the rows |
| `rate:` | number above 0 | `0.05` | the starting learning rate `eta` |
| `method:` | one of `:exact` / `:contrastive` / `:persistent` / `:pseudo` | `:contrastive` | how the machine's own averages are estimated |
| `sweeps:` | whole number, at least 1 | `1` | settling sweeps per estimate, for `:contrastive` and `:persistent` |
| `batch:` | whole number | `50` | rows per update; clamped to between 1 and the number of rows; ignored by `:exact` |
| `decay:` | number, 0 or above | `0` | weight decay `lambda` on the learnable pulls |
| `seed:` | whole number | the run's current generator (seeded 24301 at the start of a run block) | replaces the run's random generator with one seeded by this number |

**What it does:** fits the leans and learnable pulls to the rows of the set, then writes them back into the
model. The learned values stay in the model for later statements and later run blocks.

Each round shuffles the rows and cuts them into batches of `batch:` rows (the last batch may be smaller). For
each batch `B` it computes, for every lean and every learnable pull, a data term and a model term, and then
steps:

```text
eta_t = eta * (1 - 0.9 * t / rounds)
h_i  <- h_i  + (eta_t / |B|) * (data_i  - model_i)
W_ij <- W_ij + (eta_t / |B|) * (data_ij - model_ij) - eta_t * lambda * W_ij
```

The rate falls linearly from `eta` in round `t = 0` toward a tenth of `eta`. A pull rises when the rows agree
on its pair more often than the machine does, and falls when they agree less often. The fit is done when the
machine's own averages equal the rows' averages. The decay applies to the learnable pulls only, not to the
leans.

The data term sums, over the rows of the batch, the value of each thing (`data_i`) and the product of each
learnable pair (`data_ij`). A hidden thing has no value in a row, so its expected value `tanh(beta x_k(v))`
stands in for it.

The model term depends on `method:`:

| Method | Model term |
|---|---|
| `:exact` | `|B|` times the machine's exact averages, found by enumerating every arrangement of the visible things with the hidden things summed out. No sampling noise. At most 20 visible things. The batch is always every row. |
| `:contrastive` | Contrastive divergence. For each row: start at the row, draw each hidden thing given the row, then run `sweeps:` settling sweeps, and add the resulting arrangement (hidden things by their expected values). |
| `:persistent` | Persistent contrastive divergence. `batch` settling chains start at random arrangements and run through the whole fit. For each batch every chain runs `sweeps:` sweeps, and their average arrangement, scaled to the batch size, is the model term. |
| `:pseudo` | Pseudo-likelihood: fit each visible thing given all the others. The difference term for thing `i` in row `v` is `v_i - tanh(beta x_i(v))`, and for a pair it is `(v_i - tanh(beta x_i)) v_j + (v_j - tanh(beta x_j)) v_i`. No settling. Not allowed with hidden things. |

One settling sweep in `learn` updates every visible thing once, in a fresh random order, then every hidden
thing once, with the same rule as `settle`. The temperature `T` of the run sets `beta` for every method.

All randomness (the row order, the chains, the sweeps and the small random start of hidden pulls) comes from
the run's random generator. `seed:` replaces that generator before the fit, and the replacement stays in place
for the rest of the run block.

After fitting, `learn` scores the fit:

- with at most 20 visible things: the exact average log-chance of the rows, in natural log units,
  `mean over rows of (-beta F(v) - ln sum_v' exp(-beta F(v')))`;
- with more than 20 visible things and no hidden things: the average pseudo-log-likelihood,
  `mean over rows of sum_i (beta v_i x_i(v) - ln(2 cosh(beta x_i(v))))`;
- otherwise no score.

The exact score enumerates `2^visible` arrangements, whichever method was used.

**Output:**

```text
learned :<name> by <method words> over <rounds> rounds of <rows> rows: <v> visible, <h> hidden, <p> pulls, <seconds>s; <score>
```

`<method words>` is `exact gradients`, `contrastive divergence`, `persistent contrastive divergence` or
`pseudo-likelihood`. `<p>` is the number of learnable pulls: `v (v - 1) / 2` without hidden things, `v * h`
with them. `<seconds>` has two decimals. `<score>` is one of:

```text
exact log-likelihood per example <x>
pseudo-log-likelihood per example <x>
no cheap fit score for a large machine with hidden things
```

The exact score has four decimals and the pseudo score three.

**Example:** the same rows fitted by three methods, one model for each so that each fit starts from zero. The
contrastive method is shown in the example under [`examples`](#examples).

```settle example=learn-methods
# The same four rows fitted three ways, one model for each method, so each fit starts from zero.
model :ex do
  examples :d, over: "a b c", rows: "110 110 001 001"
end
model :ps do
  examples :d, over: "a b c", rows: "110 110 001 001"
end
model :pe do
  examples :d, over: "a b c", rows: "110 110 001 001"
end

run :ex do
  learn :d, method: :exact, rounds: 200, rate: 0.1, seed: 1
end
run :ps do
  learn :d, method: :pseudo, rounds: 200, rate: 0.1, seed: 1
end
run :pe do
  learn :d, method: :persistent, rounds: 200, rate: 0.1, batch: 4, sweeps: 2, seed: 1
end
```

Output:

```text output=learn-methods
examples :d inline: 4 rows added, 4 in all, over 3 things
examples :d inline: 4 rows added, 4 in all, over 3 things
examples :d inline: 4 rows added, 4 in all, over 3 things
learned :d by exact gradients over 200 rounds of 4 rows: 3 visible, 0 hidden, 3 pulls, <time>s; exact log-likelihood per example -0.7112
learned :d by pseudo-likelihood over 200 rounds of 4 rows: 3 visible, 0 hidden, 3 pulls, <time>s; exact log-likelihood per example -0.7101
learned :d by persistent contrastive divergence over 200 rounds of 4 rows: 3 visible, 0 hidden, 3 pulls, <time>s; exact log-likelihood per example -0.7876
```

**Errors:**

- `no examples :<name> (declare them with: examples :<name>, "file.txt")`
- `examples :<name> have no rows`
- `method: is :exact, :contrastive, :persistent or :pseudo`
- `method: takes a symbol, like method: :contrastive`
- `rate must be above zero, sweeps at least 1, decay not negative`
- `method: :pseudo needs every thing visible; this model has hidden things`
- `method: :exact enumerates the visible things and allows at most 20, not <n>`
- `hidden things may not pull each other (this is a restricted machine)`
- `` learn does not take `<key>:` ``
- `a number was expected`

## `classify`

**Block:** run.

**Form:**

```text
classify :name, labels: "d*", sweeps: 100, seed: 24301, temperature: 1
```

**Arguments:**

| Argument | Type | Default | Meaning |
|---|---|---|---|
| `:name` | symbol | required | the set of examples to classify |
| `labels:` | string | required | which things of the set are labels |
| `sweeps:` | whole number | `100` | settling sweeps per row after the burn-in |
| `seed:` | whole number | the run's current generator | replaces the run's random generator with one seeded by this number |
| `temperature:` | number above 0 | the run's temperature | sets the run's temperature |

`labels:` holds names separated by spaces or commas. A name ending in `*` matches every thing of the set whose
name starts with the part before the `*`, so `"d*"` matches `d0`, `d1` and so on. Each word must match at
least one thing of the set. The labels are ordered word by word, and within a `*` word in the order of the
set, with repeats dropped. There must be at least two labels and at least one other thing. The things of the
set that are not labels are the inputs.

**What it does:** scores every row that has exactly one label on. The label that is on is the row's true
answer. Rows with no label on or with more than one are skipped and counted. For each scored row:

1. **Settled readout.** Hold every input thing at the row's value, on top of anything the run already holds.
   Start from a random arrangement, run `sweeps / 10` burn-in sweeps (at least one), then run `sweeps:` sweeps
   and count, for each label, how many sweeps ended with it at yes. The label with the largest count is the
   answer; a tie goes to the first label in order. The labels and every thing outside the set, hidden things
   included, settle freely.
2. **Exact readout.** For each label `c`, set the labels one-hot with `c` on and compute `-beta F(v)` for the
   row, using the things of the set and the hidden things. The label with the largest value is the answer; a
   tie goes to the first label. Things outside the set and their pulls are not part of this readout.

The held things are restored afterwards. `classify` does not record samples and does not change the leans or
pulls. `seed:` and `temperature:` change the run's state and stay in force for the rest of the run block. A
`sweeps:` of 0 runs one sweep but prints 0.

**Output:**

```text
classify :<name> by settling <sweeps> sweeps with <inputs> inputs held: <right> of <rows> right (<a>%); exact one-hot readout <b>%; chance <c>%
```

`<rows>` counts the scored rows. `<a>` and `<b>` are the percentages of scored rows that the settled and the
exact readout got right, and `<c>` is `100 / number of labels`, all with one decimal. When rows were skipped
the line ends with `; <k> rows skipped (not exactly one label on)`.

**Examples:** "do the two bits agree?" learned from a file, with four hidden things, then classified.

```settle example=learn-file
# Learn "do the two bits agree?" from a file, with four hidden things, then classify.
model :agree do
  examples :train, "data/learn-same.txt"   # reads the header line and four rows
  hidden 4                                 # hidden_0 .. hidden_3, pulled only by visible things
end

run :agree do
  learn :train, method: :exact, rounds: 400, rate: 0.5, seed: 1
  classify :train, labels: "l_*", sweeps: 200, seed: 2   # hold x1 and x2, settle, read the labels
end
```

Output:

```text output=learn-file
examples :train from data/learn-same.txt, 4 in all, over 4 things
learned :train by exact gradients over 400 rounds of 4 rows: 4 visible, 4 hidden, 16 pulls, <time>s; exact log-likelihood per example -1.4313
classify :train by settling 200 sweeps with 2 inputs held: 3 of 4 right (75.0%); exact one-hot readout 100.0%; chance 50.0%
```

A test set with a row that has both labels on, `decay:` in the fit, and `temperature:` in `classify`:

```settle example=learn-classify
# classify on a test set with a row that has both labels on. That row is skipped.
# decay: shrinks the pulls a little each step; temperature: sets the run's temperature.
model :agree do
  examples :train, "data/learn-same.txt"
  examples :test, over: "x1 x2 l_same l_diff", rows: "0010 0101 1001 1110 1111"
  hidden 4
end

run :agree do
  learn :train, method: :exact, rounds: 400, rate: 0.5, decay: 0.001, seed: 1
  classify :test, labels: "l_same l_diff", sweeps: 200, temperature: 1.5, seed: 2
end
```

Output:

```text output=learn-classify
examples :train from data/learn-same.txt, 4 in all, over 4 things
examples :test inline: 5 rows added, 5 in all, over 4 things
learned :train by exact gradients over 400 rounds of 4 rows: 4 visible, 4 hidden, 16 pulls, <time>s; exact log-likelihood per example -1.4363
classify :test by settling 200 sweeps with 2 inputs held: 3 of 4 right (75.0%); exact one-hot readout 100.0%; chance 50.0%; 1 rows skipped (not exactly one label on)
```

**Errors:**

- `no examples :<name> (declare them with: examples :<name>, "file.txt")`
- `classify needs `labels:`, like labels: "d0 d1 d2" or labels: "d*"`
- `` no thing in these examples matches label `<word>` ``
- `labels: needs at least two label things and at least one input thing`
- `temperature must be above zero`
- `hidden things may not pull each other (this is a restricted machine)`
- `` classify does not take `<key>:` ``
- `a number was expected`
- `a "quoted" string was expected` (for `labels:`)

## `shuffle`

**Block:** both.

**Form:**

```text
shuffle :name, labels: "d*", seed: 24301
```

**Arguments:**

| Argument | Type | Default | Meaning |
|---|---|---|---|
| `:name` | symbol | required | the set of examples to change |
| `labels:` | string | required | the label things, written as for `classify` |
| `seed:` | whole number | see below | seed of the shuffle's own random generator |

**What it does:** draws a random permutation of the rows and gives each row the label cells of another row,
leaving its input cells in place. The set in the notes is replaced by the shuffled rows. This is the negative
control for `classify`: a model trained on shuffled labels should classify no better than chance.

The shuffle uses its own random generator. With `seed:`, it is seeded by that number. Without `seed:`, in a
run block its seed is one draw from the run's random generator, which advances that generator; in a model
block its seed is one draw from a fresh generator seeded 24301, so it is the same every time. The labels
follow the same rules as in `classify`, so at least two labels and one input are required.

**Output:**

```text
shuffled the <labels> label columns of :<name> across <rows> rows
```

**Example:** learn from shuffled training rows, then classify the unshuffled rows.

```settle example=learn-shuffle
# The negative control: scramble which labels go with which row of the training set,
# learn from the scrambled rows, then classify the unscrambled rows.
model :agree do
  examples :train, "data/learn-same.txt"
  examples :test, "data/learn-same.txt"
  hidden 4
end

run :agree do
  shuffle :train, labels: "l_same l_diff", seed: 2   # moves the label pairs between rows
  learn :train, method: :exact, rounds: 400, rate: 0.5, seed: 1
  classify :test, labels: "l_*", sweeps: 200, seed: 2
end
```

Output:

```text output=learn-shuffle
examples :train from data/learn-same.txt, 4 in all, over 4 things
examples :test from data/learn-same.txt, 4 in all, over 4 things
shuffled the 2 label columns of :train across 4 rows
learned :train by exact gradients over 400 rounds of 4 rows: 4 visible, 4 hidden, 16 pulls, <time>s; exact log-likelihood per example -1.3877
classify :test by settling 200 sweeps with 2 inputs held: 2 of 4 right (50.0%); exact one-hot readout 50.0%; chance 50.0%
```

**Errors:**

- `no examples :<name> (declare them with: examples :<name>, "file.txt")`
- `shuffle needs `labels:` naming the columns to scramble`
- `` no thing in these examples matches label `<word>` ``
- `labels: needs at least two label things and at least one input thing`
- `` shuffle does not take `<key>:` ``
- `a number was expected`
- `a "quoted" string was expected` (for `labels:`)

## Notes

- **Cost of `:exact` and of the score.** Both enumerate every arrangement of the visible things, `2^v` of them,
  so they grow fast: at 20 visible things that is about a million arrangements per round for `:exact`.
- **The timing in the output.** The seconds in the `learned` line are measured, so they vary between runs and
  machines.
- **Stalls.** A restricted machine can stall at the point where every hidden thing sees no net signal. The
  small random start of the hidden pulls usually escapes it, but not for every seed.
