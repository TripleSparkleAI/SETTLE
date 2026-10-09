# The denoise family

The denoise family builds a chain of small restricted machines that turns coin noise into examples one small
step at a time, in the style of a denoising thermodynamic model (Jelinčič et al., arXiv 2510.23972). Each machine
removes a little noise from a picture while the noisier picture is held. The family also has two baselines to
compare such a chain with: `sample` settles the model's own things directly, and `coins` draws every pixel alone
at its rate in the examples.

The pictures are rows of yes/no pixels declared with the learn family's `examples` statement. Every statement
here reports how close its samples come to the nearest training row, and counts the samples that copy a training
row exactly.

The source is `src/denoise.rs`. The measurements are in
`SETTLE/runs/boltzlearn2/REPORT_BOLTZLEARN2.md`.

| Statement | Block | Summary |
|---|---|---|
| [`denoiser`](#denoiser) | model | declare a chain of restricted machines |
| [`d.train`](#dtrain) | run | fit every machine of the chain to an examples set |
| [`d.generate`](#dgenerate) | run | draw samples by settling down the chain from coin noise |
| [`sample`](#sample) | run | draw samples by settling the model's own things |
| [`coins`](#coins) | run | draw samples with every pixel an independent coin |

## The chain

Pixels are +1 (yes) or -1 (no). A chain of `T` steps has `T + 1` levels. Level 0 is the data and level `T` is
pure coin noise. Level `t` keeps a correlation with the clean example that falls in a straight line:

```text
rho_t = 1 - t / T
```

A pixel at level `t` agrees with the clean pixel with chance `(1 + rho_t) / 2`.

```text
q_t = (1 - rho_t / rho_(t-1)) / 2
```

Forward step `t` flips each bit of level `t - 1` with chance `q_t`. Flip chances compose, so level `t` has the
correlation `rho_t`.

Machine `t` (for `t` from 1 to `T`) maps level `t` to level `t - 1`. It has `nv` visible things `v` (the less
noisy picture), `nh` hidden things `z`, and `nv` held inputs `x` (the noisier picture). Its energy is:

```text
E_t(v, z; x) = -a.v - b.z - v.W.z - x.C.z - gamma_t v.x,   gamma_t = atanh(rho_t / rho_(t-1))
```

An arrangement is calmer when things agree with their leans `a` and `b`, when visible and hidden things agree
through `W`, when hidden things agree with the held picture through `C`, and when each pixel agrees with the
held pixel it came from. `gamma_t` is fixed: it is the forward step's own likelihood, so `a`, `b`, `W` and `C`
only have to learn the model of level `t - 1`. At `t = T` it is 0, because the last machine sees pure noise.

The machine is sampled in blocks, which is exact Gibbs sampling because each layer only pulls on the other:

```text
P(z_k = +1 | v, x) = (1 + tanh(b_k + sum_i W_ik v_i + sum_i C_ik x_i)) / 2
P(v_i = +1 | z, x) = (1 + tanh(a_i + sum_k W_ik z_k + gamma_t x_i)) / 2
```

One sweep updates all hidden things, then all visible things. The run's temperature does not apply to the
chain.

The family stores two entries in the model's `notes`: `denoise:<d>` holds the declared settings and the pixel
names, and `denoise:<d>:stack` holds every machine's parameters and the name of the last training set.

## `denoiser`

**Block:** model.

**Form:**

```text
denoiser :d, steps: 4, hidden: 32, over: :train, width: 0, seed: 7
```

**Arguments:**

| Argument | Type | Default | Meaning |
|---|---|---|---|
| `:d` | symbol | required | the chain's name, used as `d.train` and `d.generate` |
| `steps:` | whole number, 1 to 64 | `4` | the number of steps `T`, one machine each |
| `hidden:` | whole number, 1 to 4096 | `32` | hidden things per machine |
| `over:` | symbol naming an examples set | none | size the chain now, from this set's pixels |
| `width:` | whole number | `0` (automatic) | the picture's width in pixels, used when `d.generate` draws a contact sheet |
| `seed:` | number | `7` | the seed of the machines' starting parameters |

**What it does:** records the settings. With `over:`, it also builds the chain now, over the pixels of that
examples set: every machine starts with `a = 0`, `b = 0`, and each entry of `W` and `C` drawn as `0.1` times a
standard normal number from a generator seeded with `seed:`. Without `over:`, the chain is built by its first
`d.train`, over that set's pixels.

The denoiser adds no things to the model, and its machines are not part of the model's leans and pulls. Its name
must not be the name of a thing.

**Output:**

```text
denoiser :<d>: <T> steps, <nh> hidden things per step[, over <nv> pixels]
```

**Example:** see [`d.train`](#dtrain).

**Errors:**

- `denoiser :<d> is already declared`
- `:<d> is already a thing in this model; name the denoiser something else`
- `` denoiser does not take `<key>:` ``
- `steps: takes a whole number from 1 to 64`
- `hidden: takes a whole number from 1 to 4096`
- `over: takes an examples set, like over: :train`
- `no examples :<set> (declare them with: examples :<set>, "file.txt")`
- `a number was expected`

## `d.train`

**Block:** run.

**Form:**

```text
d.train :train, rounds: 100, rate: 0.05, sweeps: 1, batch: 50, decay: 0, seed: 1, leans: :data
```

**Arguments:**

| Argument | Type | Default | Meaning |
|---|---|---|---|
| `d` | a denoiser's name | required | the chain to train |
| `:train` | symbol naming an examples set | required | the training rows |
| `rounds:` | whole number, 1 to 1,000,000 | `100` | passes over the rows |
| `rate:` | number above 0 | `0.05` | the starting learning rate |
| `sweeps:` | whole number, 1 to 10,000 | `1` | Gibbs sweeps in each contrastive-divergence step |
| `batch:` | whole number, 1 to 1,000,000 | `50` | rows per update; at most the number of rows |
| `decay:` | number, 0 or above | `0` | weight decay on `W` and `C` |
| `seed:` | number | `1` | the seed of the training noise |
| `leans:` | one of `:data` / `:zero` | `:data` | where each visible lean starts on the first training |

**What it does:** fits every machine by contrastive divergence conditioned on the held picture, each machine on
its own thread. Machine `t` uses its own random number generator, seeded from `seed:` and `t`, so the result
does not depend on the threads. For each of the `rounds:` rounds the rows are shuffled and split into batches.
For each row in a batch the machine draws fresh noise:

1. `v+`: the row with each bit flipped with chance `(1 - rho_(t-1)) / 2`, a picture at level `t - 1`;
2. `x`: `v+` with each bit flipped with chance `q_t`, the held picture at level `t`;
3. `v-`: the result of `sweeps:` sweeps started from `v+` with `x` held.

With `z+` and `z-` the average hidden states (`tanh` of the hidden inputs) given `v+` and given `v-`, the batch
adds to the parameters:

```text
a += s * sum (v+ - v-)          b += s * sum (z+ - z-)
W += s * sum (v+ z+ - v- z-)    C += s * sum x (z+ - z-)
```

where `s` is the round's rate divided by the batch size and the sums run over the batch. `W` and `C` also shrink
by the round's rate times `decay:` times their value. The rate falls in a straight line from `rate` in the first
round toward `0.1 * rate` in the last.

On the first training of a chain, `leans: :data` sets each visible lean of machine `t` to
`atanh(rho_(t-1) * mean)`, clamped to `atanh(-0.9)` to `atanh(0.9)`, where `mean` is the pixel's mean value
(+1 and -1) in the rows. This is the pixel's rate at level `t - 1`. `leans: :zero` keeps the leans at 0. A second
`d.train` continues from the trained parameters and does not reset the leans.

The chain must cover the same pixel names as the examples set. A chain declared without `over:` is built here,
from the set's pixels, and then keeps them.

**Output:**

```text
trained :<d> on :<set> (<rows> rows, <nv> pixels): <T> machines of <nh> hidden, <rounds> rounds, noise per step <q_1>/<q_2>/.../<q_T>, <secs>s
```

`<q_t>` is each step's flip chance to three decimals. `<secs>` is the training time; the documentation's
examples print it as `<time>`.

**Example:** a four-step chain over 4 x 4 glyphs, generated before and after training.

```settle example=denoise-chain
# A four-step denoising chain over 4 x 4 glyphs: generate before and after training.
model :glyphs do
  examples :train, "data/denoise-glyphs.txt"           # 24 rows over the 16 things p0 .. p15
  denoiser :d, steps: 4, hidden: 8, over: :train, width: 4
end
run :glyphs do
  d.generate 8, sweeps: 20, seed: 1                    # untrained: random pulls, the negative control
  d.train :train, rounds: 100, rate: 0.05, batch: 8, seed: 1
  d.generate 8, sweeps: 20, seed: 2, out: "denoise-chain-samples.pgm", chain: "denoise-chain-levels.pgm", cols: 4
end
```

Output:

```text output=denoise-chain
examples :train from data/denoise-glyphs.txt, 24 in all, over 16 things
denoiser :d: 4 steps, 8 hidden things per step, over 16 pixels
generated from :d (untrained: random pulls, 4 steps x 20 sweeps): 8 samples, yes-rate 0.477
trained :d on :train (24 rows, 16 pixels): 4 machines of 8 hidden, 100 rounds, noise per step 0.125/0.167/0.250/0.500, <time>s
generated from :d (trained, 4 steps x 20 sweeps): 8 samples, yes-rate 0.367 (:train 0.333); nearest :train row: median 2 of 16 pixels differ, 2 exact copies; wrote denoise-chain-samples.pgm
```

**Errors:**

- `examples :<set> have no rows`
- `no examples :<set> (declare them with: examples :<set>, "file.txt")`
- `denoiser :<d> covers other things than examples :<set>`
- `` train does not take `<key>:` ``
- `rounds: takes a whole number from 1 to 1000000`
- `sweeps: takes a whole number from 1 to 10000`
- `batch: takes a whole number from 1 to 1000000`
- `leans: is :data (start at the data's rates) or :zero`
- `rate must be above zero and decay not negative`
- `a number was expected`

## `d.generate`

**Block:** run.

**Form:**

```text
d.generate 16, sweeps: 100, seed: 2, out: "samples.pgm", rows: "samples.txt", chain: "chain.pgm",
               cols: 4, scale: 4, width: 4
```

A statement is one line; the form is split here only for reading.

**Arguments:**

| Argument | Type | Default | Meaning |
|---|---|---|---|
| `d` | a denoiser's name | required | the chain to sample |
| count | whole number, 1 to 1,000,000 | required | the number of samples |
| `sweeps:` | whole number, 1 to 1,000,000 | `100` | sweeps of each machine per sample |
| `seed:` | number | `2` | the seed of this statement's own random number generator |
| `out:` | path string | none | write a contact sheet of the samples (PGM) |
| `rows:` | path string | none | write the samples as a rows file in the examples format |
| `chain:` | path string | none | write a contact sheet of every level of the first 8 samples (PGM) |
| `cols:` | whole number | `ceil(sqrt(count))` | samples per row on the `out:` sheet |
| `scale:` | whole number | `4` | sheet cells per pixel (at least 1) |
| `width:` | whole number, 1 to the pixel count | see below | the picture's width in pixels |

**What it does:** draws each sample from its own chain. Level `T` is a coin flip for every pixel. Then, for `t`
from `T` down to 1, machine `t` holds the current picture as `x`, starts `v` as a copy of it, runs `sweeps:`
sweeps, and hands its last `v` down as the next picture. The last picture (level 0) is the sample. All samples
draw from one generator seeded with `seed:`; the run's generator is not used.

An untrained chain still generates, from its random starting pulls. That is the negative control.

`width:` sets the tile width of the sheets. When it is absent the `denoiser` declaration's `width:` is used; when
that is 0 too, the width is the square root of the pixel count if the count is a perfect square, else the pixel
count (one row).

`chain:` draws one row per sample (at most 8) and one tile per level, from the coin noise at level `T` on the
left to the sample on the right.

**Output:**

```text
generated from :<d> (<trained|untrained: random pulls>, <T> steps x <sweeps> sweeps): <n> samples, yes-rate <r>[ (:<set> <rs>); nearest :<set> row: median <m> of <nv> pixels differ, <c> exact copies][; wrote <rows file>][; wrote <out file>]
```

`<r>` is the share of yes pixels over all samples. The part in the first brackets appears once the chain has
been trained: `<set>` is the last training set, `<rs>` its share of yes pixels, `<m>` the median over samples of
the number of pixels in which a sample differs from its nearest training row, and `<c>` the number of samples
identical to a training row. The `wrote` parts name the written files by file name only. The `chain:` sheet is
written but not named in the line.

**Example:** a chain declared without `over:`, sized by its first training set, with its samples written as a
rows file and read back as examples.

```settle example=denoise-rows
# Samples written as a rows file can be read back as examples.
model :glyphs do
  examples :train, "data/denoise-glyphs.txt"
  denoiser :d, steps: 2, hidden: 8
end
run :glyphs do
  d.train :train, rounds: 60, batch: 8, seed: 1, leans: :zero   # the stack is sized by the examples here
  d.generate 6, sweeps: 20, seed: 3, rows: "denoise-rows-samples.txt"
  examples :made, "denoise-rows-samples.txt"            # the rows file uses the examples format
end
```

Output:

```text output=denoise-rows
examples :train from data/denoise-glyphs.txt, 24 in all, over 16 things
denoiser :d: 2 steps, 8 hidden things per step
trained :d on :train (24 rows, 16 pixels): 2 machines of 8 hidden, 60 rounds, noise per step 0.250/0.500, <time>s
generated from :d (trained, 2 steps x 20 sweeps): 6 samples, yes-rate 0.438 (:train 0.333); nearest :train row: median 1.5 of 16 pixels differ, 1 exact copies; wrote denoise-rows-samples.txt
examples :made from denoise-rows-samples.txt, 6 in all, over 16 things
```

The error a program meets when it generates from a chain that has neither `over:` nor a training:

```settle example=denoise-untrained
# A denoiser declared without over: learns its pixels from its first training set.
model :glyphs do
  examples :train, "data/denoise-glyphs.txt"
  denoiser :d, steps: 2, hidden: 8
end
run :glyphs do
  d.generate 4
end
```

```text error=denoise-untrained
line 7: denoiser :d does not know its pixels yet: train it, or declare it with over: :examples
```

**Errors:**

- `denoiser :<d> does not know its pixels yet: train it, or declare it with over: :examples`
- `generate takes a whole number from 1 to 1000000`
- `` generate does not take `<key>:` ``
- `sweeps: takes a whole number from 1 to 1000000`
- `width: takes a whole number from 1 to <nv>`
- `cannot write <path>: <reason>`
- `a number was expected` or `a "quoted" string was expected`

## `sample`

**Block:** run.

**Form:**

```text
sample :train, 16, sweeps: 800, seed: 3, out: "direct.pgm", rows: "direct.txt", cols: 4, scale: 4, width: 4
```

**Arguments:**

| Argument | Type | Default | Meaning |
|---|---|---|---|
| `:train` | symbol naming an examples set | required | the things to read, and the rows to compare with |
| count | whole number, 1 to 1,000,000 | required | the number of samples |
| `sweeps:` | whole number, 1 to 10,000,000 | `800` | sweeps per sample |
| `seed:` | number | the run's generator | replaces the run's random number generator with one seeded with this number |
| `out:`, `rows:`, `cols:`, `scale:` | | | as for [`d.generate`](#dgenerate) |
| `width:` | whole number, 1 to the pixel count | automatic | as for `d.generate`, without a declared width |

**What it does:** settles the model's own things, with their leans and pulls, and reads the things the examples
set names. For each sample it starts from coin flips (held things keep their values), runs `sweeps:` sweeps of
every free thing in the model at the run's temperature, and reads the set's things from the last arrangement.
This samples a machine the learn family has fitted (for example after `hidden 64` and `learn`) directly, with no
chain. It uses the run's random number generator, so `seed:` persists for the rest of the run block.

**Output:**

```text
sampled the model directly over :<set> (<sweeps> sweeps from a random start, <things> things settling): <n> samples, yes-rate <r> (:<set> <rs>); nearest :<set> row: median <m> of <nv> pixels differ, <c> exact copies[; wrote <rows file>][; wrote <out file>]
```

`<things>` is the number of things in the model. The other fields are as for `d.generate`, with `<set>` the named
set.

**Example:** both baselines on a model with a few hand-set pulls.

```settle example=denoise-baselines
# Two baselines for a denoiser: sample the model's own things directly, and draw independent coins.
model :glyphs do
  examples :train, "data/denoise-glyphs.txt"          # declares the things p0 .. p15
  p0.pulls :p1, by: 1                                  # hand-set pulls along the top row, so the model
  p1.pulls :p2, by: 1                                  # has some structure for `sample` to settle into
  p2.pulls :p3, by: 1
end
run :glyphs do
  sample :train, 8, sweeps: 200, seed: 3, out: "denoise-baselines-direct.pgm"
  coins :train, 8, seed: 4, out: "denoise-baselines-coins.pgm", rows: "denoise-baselines-coins.txt"
end
```

Output:

```text output=denoise-baselines
examples :train from data/denoise-glyphs.txt, 24 in all, over 16 things
sampled the model directly over :train (200 sweeps from a random start, 16 things settling): 8 samples, yes-rate 0.516 (:train 0.333); nearest :train row: median 6 of 16 pixels differ, 0 exact copies; wrote denoise-baselines-direct.pgm
coins at the pixel rates of :train: 8 samples, yes-rate 0.297 (:train 0.333); nearest :train row: median 4 of 16 pixels differ, 0 exact copies; wrote denoise-baselines-coins.txt; wrote denoise-baselines-coins.pgm
```

**Errors:**

- `no examples :<set> (declare them with: examples :<set>, "file.txt")`
- `sample takes a whole number from 1 to 1000000`
- `` sample does not take `<key>:` ``
- `sweeps: takes a whole number from 1 to 10000000`
- `unknown thing :<name> (declare it with: thing :<name>)`
- `width: takes a whole number from 1 to <nv>`
- `cannot write <path>: <reason>`
- `a number was expected` or `a "quoted" string was expected`

## `coins`

**Block:** run.

**Form:**

```text
coins :train, 16, seed: 4, out: "coins.pgm", rows: "coins.txt", cols: 4, scale: 4, width: 4
```

**Arguments:**

| Argument | Type | Default | Meaning |
|---|---|---|---|
| `:train` | symbol naming an examples set | required | the rows whose pixel rates are used |
| count | whole number, 1 to 1,000,000 | required | the number of samples |
| `seed:` | number | `4` | the seed of this statement's own random number generator |
| `out:`, `rows:`, `cols:`, `scale:` | | | as for [`d.generate`](#dgenerate) |
| `width:` | whole number, 1 to the pixel count | automatic | as for `d.generate`, without a declared width |

**What it does:** computes each pixel's yes-rate in the set's rows, then draws every pixel of every sample as an
independent coin with that chance. It does not touch the model or the run's state.

**Output:**

```text
coins at the pixel rates of :<set>: <n> samples, yes-rate <r> (:<set> <rs>); nearest :<set> row: median <m> of <nv> pixels differ, <c> exact copies[; wrote <rows file>][; wrote <out file>]
```

**Example:** see [`sample`](#sample).

**Errors:**

- `no examples :<set> (declare them with: examples :<set>, "file.txt")`
- `examples :<set> have no rows`
- `coins takes a whole number from 1 to 1000000`
- `` coins does not take `<key>:` ``
- `width: takes a whole number from 1 to <nv>`
- `cannot write <path>: <reason>`
- `a number was expected` or `a "quoted" string was expected`

## Output files

**Rows files.** `rows:` writes the samples in the examples format: the first line is the pixel names separated by
spaces, then one line per sample with `1` for yes and `0` for no. The learn family's `examples` statement reads
such a file back.

**Contact sheets.** `out:` and `chain:` write an 8-bit binary PGM (see
[Picture files](grid.md#picture-files)). Each sample is a tile `width` pixels wide and `ceil(nv / width)` rows
high, each pixel drawn as a `scale` x `scale` square: black for yes, white for no. Tiles are separated by a grey
(0.6) line one pixel wide, and the sheet has a grey border of the same width.

## Notes

- `d.train` and `d.generate` are claimed by this family only when `d` is a declared denoiser. Otherwise the line
  goes on to the other families and, if none claims it, fails with the interpreter's unknown-statement error.
- Machines are sized by the pixel count and the hidden count: each holds `2 * nv * nh + nv + nh` parameters. The
  report used 8 x 8 digits (64 pixels).
- On 8 x 8 binarised digits, an 8-step chain reached an MMD (x1000) of about 1.5 against held-out digits
  whether its leans started at the data's rates or at zero. The learn family's machine sampled directly for 800
  sweeps reached 51.5 and ran to a yes-rate of 0.455 against the data's 0.323
  (`SETTLE/runs/boltzlearn2/REPORT_BOLTZLEARN2.md`, "Results").
