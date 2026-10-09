# The grid family

The grid family turns a greyscale picture into a block of things, one thing per pixel. Each pixel pulls its
four neighbours, and each pixel's lean is set so that its yes-rate aims at the picture's grey level at that
pixel. A settle then reproduces the picture as yes-rates. The `play` statement does this for every frame in a
folder of pictures and scores each reproduced frame against its target.

The source is `src/grid.rs`. Two library files add options to `play` and have no statements of their own:
`src/filmsharp.rs` (the Bethe inversion, the fitted leans, the Rao-Blackwellised read and the other update rules)
and `src/filmwarm.rs` (warm-started fits). The measurements are in these experiment reports, all under
`SETTLE/runs/`:

- `gridplayer/REPORT_GRIDPLAYER.md`: the mapping, the bits and soft reads, warm and cold starts.
- `gridplayer2/REPORT_GRIDPLAYER2.md`: the TAP inversion and `copies:`.
- `filmsharp/REPORT_FILMSHARP.md`: `correct: :bethe`, `fit:`, `read: :rb` and `update:`.
- `filmwarm/REPORT_FILMWARM.md`: `warm_fit:` and the options that go with it.

| Statement | Block | Summary |
|---|---|---|
| [`grid`](#grid) | model | declare a width x height block of pixel things with neighbour pulls |
| [`img.lean_from`](#imglean_from) | both | set every pixel's lean from a greyscale picture |
| [`img.show_as`](#imgshow_as) | run | write the pixels' yes-rates or last states as a picture |
| [`play`](#play) | run | reproduce every frame of a folder of pictures, one after another |

## How a grey becomes a lean

A thing is only ever yes or no, so a grey level cannot be a thing's state. A grey `g` in `[0, 1]` is a rate
instead: a pixel of grey `g` should be yes a fraction `g` of the time. The family works with the pixel's
magnetisation `m = 2g - 1`, which runs from -1 (black) to +1 (white). Greys are clamped to `[0.001, 0.999]`
before this step so that every lean below stays finite.

```text
P(s_i = yes) = (1 + tanh(I_i / T)) / 2,   I_i = h_i + sum_j J_ij s_j
```

A pixel is yes with a chance set by its lean `h_i` plus the pull `J_ij` of each neighbour's state `s_j` (+1 or
-1), at temperature `T`.

A lone pixel with lean `atanh(m)` at temperature 1 has exactly the yes-rate `g`. Neighbour pulls add to the input,
so each inversion below subtracts what the neighbours add. The option `correct:` chooses the inversion:

```text
correct: :no      h_i = by * atanh(m_i)
correct: :mean    h_i = by * atanh(m_i) - sum_j J_ij m_j
correct: :tap     h_i = by * atanh(m_i) - sum_j J_ij m_j + m_i * sum_j J_ij^2 (1 - m_j^2)
```

`:no` aims at the grey and ignores the neighbours. `:mean` (also written `:yes`) subtracts the neighbours'
average pull; it was the default until 2026-10-06. `:tap`, the default since then, adds back the Onsager term: part of a neighbour's pull is the pixel's own
influence echoed back, so the mean-field form subtracts too much. On a uniform grey region the mean-field lean
changes sign above a pull of 0.25 and the picture comes out as its mirror image; the TAP lean does not. The TAP
picture still breaks down at stronger pulls: on the horse, bits after 80 sweeps, it falls from 21.15 dB at a
pull of 0.35 to 7.22 dB at 0.40 (`runs/gridplayer2/tap_out.txt`). On 20 targets solved exactly on a 4x4 grid,
`:tap` was closer than `:mean` at every pull from 0.05 to 0.5 (`runs/gridplayer2/tap_exact_out.txt`), which is
why it became the default on 2026-10-06. Write `correct: :mean` for the old default.

```text
correct: :bethe   h_i = atanh(m_i) - sum_j atanh(t_ij * mu_j\i) + (by - 1) * atanh(m_i),   t_ij = tanh(J_ij)
```

For each edge, the cavity magnetisations `mu_i\j` and `mu_j\i` solve the pair equations
`m_i (1 + t a b) = a + t b` and `m_j (1 + t a b) = b + t a` (with `a = mu_i\j`, `b = mu_j\i`), found by
bisection. The lean subtracts what each neighbour passes on. This is exact on a tree.

All four inversions use only pulls between pixels of the same grid. They assume temperature 1: at another
temperature every input is divided by `T`, and the yes-rates move away from the target greys.

The family stores two entries in the model's `notes`: `grid:<name>` holds the first thing's index, the width,
the height and the `smooth:` pull; `gridtarget:<name>` holds the picture most recently given to `lean_from`.

## `grid`

**Block:** model.

**Form:**

```text
grid :img, width: 64, height: 48, smooth: 0
```

**Arguments:**

| Argument | Type | Default | Meaning |
|---|---|---|---|
| `:img` | symbol | required | the grid's name; it prefixes every pixel's thing name |
| `width:` | whole number | required | pixels per row |
| `height:` | whole number | required | rows |
| `smooth:` | number | `0` | the pull between each pixel and each of its four neighbours; negative values push |

**What it does:** declares `width * height` things named `<img>_<x>_<y>`, with `x` from 0 to `width - 1` and
`y` from 0 to `height - 1`, added row by row. Every lean starts at 0. When `smooth:` is not 0 it adds a pull of
`smooth` between each pixel and its right neighbour and between each pixel and the pixel below it. The edges
do not wrap, so a corner pixel has two neighbours and an edge pixel three. `width:` and `height:` are truncated
to whole numbers.

**Output:** none.

**Example:** the example under [`img.show_as`](#imgshow_as) declares a 24 x 16 grid and sets its leans.

**Errors:**

- `grid :<name> is already declared`
- `` grid does not take `<key>:` ``
- `` grid needs `width:` and `height:` ``
- `a grid needs between 1 and 4,000,000 pixels`
- `a number was expected` (for `width:`, `height:` or `smooth:`)

## `img.lean_from`

**Block:** both.

**Form:**

```text
img.lean_from "frame.pgm", by: 1, correct: :mean
```

**Arguments:**

| Argument | Type | Default | Meaning |
|---|---|---|---|
| `img` | a grid's name | required | the grid whose leans are set |
| path | path string | required | a binary PGM (P5) picture of exactly the grid's width and height; relative to the program file |
| `by:` | number | `1` | multiplies the `atanh(m)` part of each lean; above 1 sharpens the picture |
| `correct:` | one of `:tap` / `:mean` / `:yes` / `:bethe` / `:no` | `:tap` (`:mean` until 2026-10-06) | the inversion (see [How a grey becomes a lean](#how-a-grey-becomes-a-lean)) |

**What it does:** reads the picture and replaces the lean of every pixel of the grid with the lean the chosen
inversion gives. Leans of other things are not changed. It stores the picture's greys in `notes` as
`gridtarget:<img>`, where `show_as` finds them. In a run block the new leans are written into the model, so
they stay after the block ends.

**Output:** none.

**Example:**

```settle example=grid-lean-run
# lean_from inside a run changes the leans for the rest of that run.
model :film do
  grid :img, width: 24, height: 16, smooth: 0.2
end
run :film do
  img.lean_from "data/grid-frames/f1.pgm", correct: :no   # leans without the neighbour term
  settle 400, seed: 7
  img.show_as "grid-lean-run-a.pgm"
  img.lean_from "data/grid-frames/f1.pgm", by: 1.5        # mean-field leans, sharpened by 1.5
  settle 400, seed: 7
  img.show_as "grid-lean-run-b.pgm"
end
```

Output:

```text output=grid-lean-run
settled: 400 samples of 384 things at temperature 1
show_as :img -> grid-lean-run-a.pgm (rate), PSNR 17.36 dB against the leans' picture
settled: 400 samples of 384 things at temperature 1
show_as :img -> grid-lean-run-b.pgm (rate), PSNR 18.88 dB against the leans' picture
```

**Errors:**

- `no grid :<name> (declare it with: grid :<name>, width: 64, height: 48)`
- `` lean_from does not take `<key>:` ``
- `correct: takes :yes or :mean (mean-field), :tap (TAP), :bethe (Bethe) or :no`
- `<path> is <w>x<h> but grid :<name> is <w>x<h>`
- the picture errors listed under [Picture files](#picture-files)

The error a program most often meets is a picture of the wrong size:

```settle example=grid-wrong-size
# A picture must have the grid's width and height.
model :film do
  grid :img, width: 24, height: 16
  img.lean_from "data/grid-small.pgm"
end
```

```text error=grid-wrong-size
line 4: data/grid-small.pgm is 8x8 but grid :img is 24x16
```

## `img.show_as`

**Block:** run.

**Form:**

```text
img.show_as "out.pgm", from: :rate
```

**Arguments:**

| Argument | Type | Default | Meaning |
|---|---|---|---|
| `img` | a grid's name | required | the grid to write |
| path | path string | required | the PGM file to write; relative to the program file |
| `from:` | one of `:rate` / `:last` | `:rate` | what each pixel's grey shows |

**What it does:** writes an 8-bit binary PGM of the grid's size. With `from: :rate` each pixel's grey is its
yes-rate: its yes-count divided by the number of samples, from the last `settle` or `play` in the run. With
`from: :last` each pixel is black (no) or white (yes), from the last arrangement of the last `settle`,
`anneal` or `play`. The folder of the path must already exist.

**Output:**

```text
show_as :<img> -> <path> (<rate|last>)
show_as :<img> -> <path> (<rate|last>), PSNR <p> dB against the leans' picture
```

The second form appears when `lean_from` has given the grid a picture. The PSNR compares the written greys with
that picture (see [`play`](#play) for the formula).

**Example:**

```settle example=grid-still
# One still picture on a 24 x 16 grid of p-bits.
model :film do
  grid :img, width: 24, height: 16, smooth: 0.2   # 384 things img_0_0 .. img_23_15, each pulls its neighbours by 0.2
  img.lean_from "data/grid-frames/f0.pgm"          # leans aim each pixel's yes-rate at the picture's grey
end
run :film do
  settle 400, seed: 1                   # sample the grid with the core settle statement
  img.show_as "grid-still-rate.pgm"     # write each pixel's yes-rate as a grey level
  img.show_as "grid-still-last.pgm", from: :last   # write the last arrangement: black or white only
end
```

Output:

```text output=grid-still
settled: 400 samples of 384 things at temperature 1
show_as :img -> grid-still-rate.pgm (rate), PSNR 35.77 dB against the leans' picture
show_as :img -> grid-still-last.pgm (last), PSNR 6.88 dB against the leans' picture
```

**Errors:**

- `no grid :<name> (declare it with: grid :<name>, width: 64, height: 48)`
- `` show_as does not take `<key>:` ``
- `show_as takes from: :rate (yes-rate of the last settle or play) or from: :last`
- `show_as needs a settle or a play first (or use from: :last)`
- `show_as from: :last needs a settle, anneal or play first`
- `cannot write <path>: <reason>`

## `play`

**Block:** run.

**Form:**

```text
play :img, frames: "dir/", sweeps: 10, out: "dir2/", against: "other/", warm: :yes, read: :bits, keep: 1,
           by: 1, correct: :tap, copies: 1, update: :metro_checker, fit: 0, fit_sweeps: 200,
           fit_update: :metro_checker, warm_fit: 0, warm_fit_sweeps: 200, warm_from: :correction, warm_step: 1, cut: 0,
           temperature: 1, seed: 1, quiet: :no
```

A statement is one line; the form is split here only for reading.

**Arguments:**

| Argument | Type | Default | Meaning |
|---|---|---|---|
| `:img` | symbol | required | the grid to play on |
| `frames:` | path string | required | a folder of `.pgm` frames, each of the grid's size |
| `sweeps:` | whole number | required | sweeps per frame (per copy); at least 1 |
| `out:` | path string | none | a folder to write each reproduced frame to, under the frame's file name; created if missing |
| `against:` | path string | none | score frame `k` against the `k`-th `.pgm` in this folder instead of against its own target |
| `warm:` | `:yes` / `:no` | `:yes` | start each frame from the state the previous frame ended in (`:yes`) or from coin flips (`:no`) |
| `read:` | one of `:bits` / `:soft` / `:rb` | `:bits` | how a frame is read from the sweeps (see below) |
| `keep:` | number above 0, at most 1 | `1` | the share of the last sweeps that is counted |
| `by:` | number | `1` | as for `lean_from` |
| `correct:` | one of `:tap` / `:mean` / `:yes` / `:bethe` / `:no` | `:tap` (`:mean` until 2026-10-06) | as for `lean_from` |
| `copies:` | whole number, 1 to 4096 | `1` | independent copies of the grid whose counts are pooled |
| `update:` | one of `:metro_checker` / `:gibbs` / `:checker` / `:metro` / `:cluster` | `:metro_checker` (`:gibbs` until 2026-10-06) | how each sweep updates the pixels (see below) |
| `fit:` | whole number, 0 to 1000 | `0` | iterations of the lean fit run before each frame; 0 uses the closed-form leans |
| `fit_sweeps:` | whole number, at least 4 | `200` | sweeps per fit iteration |
| `fit_update:` | as `update:` | the value of `update:` | the update rule of the fit's own chain |
| `warm_fit:` | whole number, 0 to 1000 | `0` (off) | fit iterations for every frame after the first, started from the previous frame's fit; needs `fit:` |
| `warm_fit_sweeps:` | whole number, at least 4 | the value of `fit_sweeps:` | sweeps per warm fit iteration |
| `warm_from:` | one of `:correction` / `:leans` | `:correction` (`:leans` until 2026-10-06) | where a warm fit starts (see below) |
| `warm_step:` | number, 0 to 1 | `1` | the first step size of each warm fit |
| `cut:` | number, 0 to 1 | `0` (off) | fit a frame cold when its RMS grey change from the previous frame is above this |
| `temperature:` | number above 0 | the run's temperature | sets the run's temperature, for this play and for later statements |
| `seed:` | number | the run's generator | replaces the run's random number generator with one seeded with this number |
| `quiet:` | `:yes` / `:no` | `:no` | `:yes` prints only the summary line |

`warm_fit_sweeps:`, `warm_from:`, `warm_step:` and `cut:` are refused unless `warm_fit:` is 1 or more.

**What it does:** lists the `.pgm` files in `frames:` (the extension in any case), sorts them by path, and plays
them in that order. For each frame it:

1. reads the picture and sets the grid's leans with the inversion `correct:` and the factor `by:`;
2. if `fit:` is above 0, refines those leans with the fit described below;
3. runs `sweeps:` sweeps over the grid's pixels for each of the `copies:` copies, one copy after another;
4. reads each pixel's grey from the last `round(sweeps * keep)` sweeps of each copy (at least 1), pooled over
   the copies;
5. scores the grey picture against the target (or the `against:` frame) and, with `out:`, writes it as an
   8-bit PGM.

Only the grid's pixels are updated. Held pixels keep their held values. Things outside the grid keep the values
they had in the starting arrangement. The work per frame is `copies * sweeps` sweeps; there is no burn-in.

A warm play keeps each copy's arrangement from one frame to the next, so frame 2 starts where frame 1 ended.
The first frame of every `play` statement starts from coin flips. A cold play (`warm: :no`) starts every frame
from coin flips.

**The reads.** With `S = copies * round(sweeps * keep)` counted sweeps per pixel:

```text
read: :bits   g_i = (number of counted sweeps that end with pixel i at yes) / S
read: :soft   g_i = (1 + (1/S) * sum tanh(I_i / T)) / 2,   I_i taken at the end of each counted sweep
read: :rb     g_i = (1 + (1/S) * sum tanh(I_i / T)) / 2,   I_i taken just before pixel i draws its coin
```

`:bits` counts the pixel's yes states. `:soft` and `:rb` average the chance of yes given the neighbours, which
has far less noise than the 0/1 bit. `:rb` is the Rao-Blackwellised form: it reads the neighbours at the moment
the pixel is updated. With `smooth: 0` the soft read returns the target after one sweep, so it measures only the
mapping there.

**The update rules.**

- `:gibbs`: every pixel once per sweep in a fresh random order, each drawing yes with the chance above. This is
  the core `settle` rule `update: :gibbs` and uses the same random numbers. It was the default until 2026-10-06.
- `:checker`: all pixels with even `x + y`, then all with odd `x + y`, in a fixed order.
- `:metro`: random order; each pixel proposes the other state and accepts with chance
  `min(1, exp(-2 s_i I_i / T))` (Metropolised Gibbs).
- `:metro_checker`: the `:metro` rule in the `:checker` order. The default since 2026-10-06:
  on the film it was ahead of Gibbs at every pull measured, 29.68 against 26.71 dB for bits at J 0.2 and 80
  sweeps (`runs/filmsharp/mixing_out.txt`). Its successive draws of a pixel are anti-correlated, so with no pulls
  its bits read beats the coin-noise law, which is exact only for `:gibbs`.
- `:cluster`: one Swendsen-Wang step per sweep. A satisfied pull bonds its two pixels with chance
  `1 - exp(-2 |J| / T)`; a pixel that agrees with its lean plus any pulls from outside the grid bonds to a ghost
  with chance `1 - exp(-2 |field| / T)`; held pixels are tied to the ghost; every cluster not joined to the ghost
  flips with chance 1/2. This is not a single-pixel rule. With `read: :rb` it reads the inputs after the step.

**The fit.** With `fit: N`, each frame's leans start from the closed-form leans and take `N` steps:

```text
h <- h + eta * P (m* - m_hat)
```

`m*` is the target magnetisation and `m_hat` is measured by settling the grid itself: `fit_sweeps` sweeps of
the fit's own chain with rule `fit_update:`, a quarter of them burn-in, read as `:rb`. `P` is the TAP inverse
response at the target, with each row's diagonal raised until it exceeds the sum of that row's off-diagonal
sizes by 0.05, so `P` is positive definite. `eta` starts at 1 and halves (to no less than 1/64) whenever the
residual grows. Each lean moves by at most 2 per step. The fitted leans are the average of the leans after
the last `N - floor(N/2)` steps. The residual reported for each step is the RMS grey error `sqrt(mean((m* - m_hat)^2 / 4))`,
measured before that step.

The fit's chain starts from coin flips seeded by one number drawn from the run's generator, and the fit's
sweeps use a generator seeded with that number, not the run's. The fit's sweeps are extra work: they are not
part of `sweeps:` and their time is not part of the settling rate.

**The warm fit.** With `warm_fit: W` (and `fit: N`), the first frame is fitted cold as above. Every later frame
takes `W` iterations of `warm_fit_sweeps:` sweeps, continuing the previous frame's fit chain instead of starting
a fresh one, from these leans:

```text
warm_from: :leans        h0 = h_fit(previous frame)
warm_from: :correction   h0 = h_closed(this frame) + h_fit(previous frame) - h_closed(previous frame)
```

`:leans` starts from the previous frame's fitted leans. `:correction` starts from this frame's closed-form
leans (the ones `correct:` gives) plus the correction the fit added to the previous frame. `:correction` is the
default since 2026-10-06: it was never worse than `:leans` and up to 6.5 dB better after one
iteration (`runs/filmwarm/budget_j4*_out.txt`). Write `warm_from: :leans` for the old default. The first step of a
warm fit has size `warm_step:` instead of 1. With `cut: X` above 0, a frame whose target greys differ from the
previous frame's by more than `X` in RMS is fitted cold instead, with the `fit:` budget and a fresh chain. The
warm state lasts for one `play` statement.

**State after a play.** The grid's leans are the last frame's leans (fitted, if a fit ran); they stay in the
model after the run block ends. The run's yes-counts hold the last frame's counts for the grid's pixels (all
copies pooled) and zero for every other thing, the sample count is `S`, and the last arrangement is copy 1's
final arrangement. So `show_as` after a `play` writes the last frame. `temperature:` and `seed:` persist for
the rest of the run block.

**Output:** one line per frame unless `quiet: :yes`, then one summary line. Without `warm_fit:`:

```text
  <file>  PSNR <p> dB  <t> ms
```

With `warm_fit:`:

```text
  <file>  PSNR <p> dB  <t> ms  fit <warm|cold> <n> sweeps, residual <first> -> <last>, target change <c>
```

`<t>` is the time spent settling that frame. `<n>` is the fit's sweeps for the frame (iterations times sweeps).
`<first>` and `<last>` are the residuals measured at the fit's first and last iterations; with one iteration
they are the same number. `<c>` is the RMS grey change of the target from the previous frame (0 on the first).

The summary line:

```text
play :<img>: <frames> frames, [<K> copies x ]<sweeps> sweeps, <warm|cold>, <read>[<correct>][<update>]: median PSNR <p> dB (worst <w>), <r1> frames/s settling, <r2> frames/s with file work[<fit>]
```

`<read>` is `bits`, `soft` or `rb`. `<correct>` is empty for `:tap` (the default), else `, mean`, `, bethe` or
`, uncorrected`. `<update>` is empty for `:metro_checker` (the default), else `, gibbs`, `, checker`, `, metro` or
`, cluster`. `<r1>` counts only settling time; `<r2>` counts the whole statement, including reading and writing
files and any fit. `by:` and `temperature:` do not appear in the line. `<fit>` is empty without `fit:`; with
`fit:` it is:

```text
; fit <N> x <fit_sweeps> sweeps[<fit_update>], median grey residual <first> -> <last>, <s> s fitting
```

and with `warm_fit:` it is:

```text
; fit <N> x <fit_sweeps> sweeps[<fit_update>] cold on the first frame, then warm <W> x <warm_fit_sweeps> from <leans|correction>[, cut above <X>][, step <e>]: <a> warm and <b> cold frames, fit sweeps <total> in all, <avg> per frame after the first; median PSNR after the first frame <p> dB; median grey residual <first> -> <last>, <s> s fitting
```

The medians are taken over frames. With `against:` the summary is followed by
`  (scored against another folder: the negative control)`.

PSNR is computed over the grid's pixels, with greys in `[0, 1]`:

```text
PSNR = 10 log10(1 / MSE),   MSE = mean_i (g_i - target_i)^2
```

A higher PSNR is a closer picture. When the two pictures match to within `MSE <= 1e-12` the PSNR is reported as
99.

**Example:** a play with the default options, writing the reproduced frames.

```settle example=grid-play
# Play a folder of three 24 x 16 frames with the default options.
model :film do
  grid :img, width: 24, height: 16, smooth: 0.2
end
run :film do
  play :img, frames: "data/grid-frames/", out: "out/grid-play/", sweeps: 20, seed: 2
end
```

Output:

```text output=grid-play
  f0.pgm  PSNR 22.59 dB  <time> ms
  f1.pgm  PSNR 22.45 dB  <time> ms
  f2.pgm  PSNR 23.02 dB  <time> ms
play :img: 3 frames, 20 sweeps, warm, bits: median PSNR 22.59 dB (worst 22.45), <time> frames/s settling, <time> frames/s with file work
```

**Example:** the three reads on the same chain.

```settle example=grid-read
# The same play read three ways. The leans use the TAP inversion.
model :film do
  grid :img, width: 24, height: 16, smooth: 0.2
end
run :film do
  play :img, frames: "data/grid-frames/", sweeps: 20, correct: :tap, read: :bits, seed: 3, quiet: :yes
  play :img, frames: "data/grid-frames/", sweeps: 20, correct: :tap, read: :soft, seed: 3, quiet: :yes
  play :img, frames: "data/grid-frames/", sweeps: 20, correct: :tap, read: :rb, seed: 3, quiet: :yes
end
```

Output:

```text output=grid-read
play :img: 3 frames, 20 sweeps, warm, bits: median PSNR 22.78 dB (worst 22.77), <time> frames/s settling, <time> frames/s with file work
play :img: 3 frames, 20 sweeps, warm, soft: median PSNR 30.96 dB (worst 30.55), <time> frames/s settling, <time> frames/s with file work
play :img: 3 frames, 20 sweeps, warm, rb: median PSNR 30.76 dB (worst 30.42), <time> frames/s settling, <time> frames/s with file work
```

**Example:** other inversions and update rules, copies, `keep:` and a cold start.

```settle example=grid-options
# Other inversions, update rules, copies, keep, and a cold start.
model :film do
  grid :img, width: 24, height: 16, smooth: 0.3
end
run :film do
  play :img, frames: "data/grid-frames/", sweeps: 20, correct: :bethe, read: :rb, seed: 4, quiet: :yes
  play :img, frames: "data/grid-frames/", sweeps: 20, correct: :tap, update: :checker, read: :rb, seed: 4, quiet: :yes
  play :img, frames: "data/grid-frames/", sweeps: 20, correct: :tap, update: :metro, read: :rb, seed: 4, quiet: :yes
  play :img, frames: "data/grid-frames/", sweeps: 20, correct: :tap, update: :cluster, read: :rb, seed: 4, quiet: :yes
  play :img, frames: "data/grid-frames/", sweeps: 10, copies: 4, keep: 0.5, warm: :no, correct: :tap, seed: 4, quiet: :yes
end
```

Output:

```text output=grid-options
play :img: 3 frames, 20 sweeps, warm, rb, bethe: median PSNR 23.84 dB (worst 23.21), <time> frames/s settling, <time> frames/s with file work
play :img: 3 frames, 20 sweeps, warm, rb, checker: median PSNR 19.72 dB (worst 19.20), <time> frames/s settling, <time> frames/s with file work
play :img: 3 frames, 20 sweeps, warm, rb, metro: median PSNR 21.40 dB (worst 20.98), <time> frames/s settling, <time> frames/s with file work
play :img: 3 frames, 20 sweeps, warm, rb, cluster: median PSNR 21.26 dB (worst 20.52), <time> frames/s settling, <time> frames/s with file work
play :img: 3 frames, 4 copies x 10 sweeps, cold, bits: median PSNR 20.74 dB (worst 20.22), <time> frames/s settling, <time> frames/s with file work
```

**Example:** `temperature:` and `by:`, then `show_as` after a play.

```settle example=grid-temperature
# Temperature and sharpening in play, then show_as reads the counts play leaves behind.
model :film do
  grid :img, width: 24, height: 16, smooth: 0.2
end
run :film do
  play :img, frames: "data/grid-frames/", sweeps: 40, correct: :tap, seed: 8, quiet: :yes
  # at temperature 2 every input is halved, so the greys wash toward 0.5; by: 2 doubles the atanh part, which partly compensates
  play :img, frames: "data/grid-frames/", sweeps: 40, correct: :tap, temperature: 2, seed: 8, quiet: :yes
  play :img, frames: "data/grid-frames/", sweeps: 40, correct: :tap, temperature: 2, by: 2, seed: 8, quiet: :yes
  img.show_as "grid-temperature-last-frame.pgm"   # the last frame's yes-rates (no lean_from, so no PSNR)
end
```

Output:

```text output=grid-temperature
play :img: 3 frames, 40 sweeps, warm, bits: median PSNR 26.18 dB (worst 25.75), <time> frames/s settling, <time> frames/s with file work
play :img: 3 frames, 40 sweeps, warm, bits: median PSNR 18.64 dB (worst 17.88), <time> frames/s settling, <time> frames/s with file work
play :img: 3 frames, 40 sweeps, warm, bits: median PSNR 27.47 dB (worst 27.44), <time> frames/s settling, <time> frames/s with file work
show_as :img -> grid-temperature-last-frame.pgm (rate)
```

**Example:** the negative control. The output is scored against a different folder of frames.

```settle example=grid-against
# The negative control: score the output against frames it was never given.
model :film do
  grid :img, width: 24, height: 16, smooth: 0.1
end
run :film do
  play :img, frames: "data/grid-frames/", sweeps: 40, read: :soft, correct: :tap, seed: 6, quiet: :yes
  play :img, frames: "data/grid-frames/", sweeps: 40, read: :soft, correct: :tap, seed: 6, against: "data/grid-other/"
end
```

Output:

```text output=grid-against
play :img: 3 frames, 40 sweeps, warm, soft: median PSNR 43.67 dB (worst 42.69), <time> frames/s settling, <time> frames/s with file work
  f0.pgm  PSNR 8.91 dB  <time> ms
  f1.pgm  PSNR 8.82 dB  <time> ms
  f2.pgm  PSNR 8.64 dB  <time> ms
play :img: 3 frames, 40 sweeps, warm, soft: median PSNR 8.82 dB (worst 8.64), <time> frames/s settling, <time> frames/s with file work
  (scored against another folder: the negative control)
```

**Example:** a fit of the leans before each frame, at a strong pull.

```settle example=grid-fit
# Fit the leans by settling the grid itself before each frame is played.
model :film do
  grid :img, width: 24, height: 16, smooth: 0.42   # a strong pull, where the TAP leans overshoot
end
run :film do
  play :img, frames: "data/grid-frames/", sweeps: 80, correct: :tap, read: :rb, update: :cluster, seed: 5, quiet: :yes
  # the same play with 8 fit iterations of 80 sweeps each; the fit chain uses update: :cluster too
  play :img, frames: "data/grid-frames/", sweeps: 80, correct: :tap, read: :rb, update: :cluster, fit: 8, fit_sweeps: 80, seed: 5, quiet: :yes
end
```

Output:

```text output=grid-fit
play :img: 3 frames, 80 sweeps, warm, rb, cluster: median PSNR 13.64 dB (worst 12.06), <time> frames/s settling, <time> frames/s with file work
play :img: 3 frames, 80 sweeps, warm, rb, cluster: median PSNR 18.54 dB (worst 15.20), <time> frames/s settling, <time> frames/s with file work; fit 8 x 80 sweeps, cluster, median grey residual 0.15422 -> 0.14304, <time> s fitting
```

**Example:** a warm fit. In the second play the cut detector fits the third frame cold, because its target
changed by 0.1872 in RMS grey, more than the 0.16 allowed.

```settle example=grid-warm-fit
# Fit the first frame's leans cold, then fit each later frame warm from the frame before.
model :film do
  grid :img, width: 24, height: 16, smooth: 0.42   # a strong pull, where the closed-form leans fall short
end
run :film do
  # cold fit on frame 1 (4 x 60 sweeps), then 1 warm iteration of 60 sweeps per frame, carrying the correction
  play :img, frames: "data/grid-frames/", sweeps: 60, correct: :tap, read: :soft, fit: 4, fit_sweeps: 60, warm_fit: 1, warm_from: :correction, seed: 5
  # the same with a smaller first warm step and a cut detector that refits cold when a frame changes a lot
  play :img, frames: "data/grid-frames/", sweeps: 60, correct: :tap, read: :soft, fit: 4, fit_sweeps: 60, warm_fit: 2, warm_fit_sweeps: 30, warm_step: 0.5, cut: 0.16, seed: 5
end
```

Output:

```text output=grid-warm-fit
  f0.pgm  PSNR 16.95 dB  <time> ms  fit cold 240 sweeps, residual 0.32257 -> 0.22687, target change 0.0000
  f1.pgm  PSNR 17.12 dB  <time> ms  fit warm 60 sweeps, residual 0.18828 -> 0.18828, target change 0.1437
  f2.pgm  PSNR 17.59 dB  <time> ms  fit warm 60 sweeps, residual 0.18831 -> 0.18831, target change 0.1872
play :img: 3 frames, 60 sweeps, warm, soft: median PSNR 17.12 dB (worst 16.95), <time> frames/s settling, <time> frames/s with file work; fit 4 x 60 sweeps cold on the first frame, then warm 1 x 60 from correction: 2 warm and 1 cold frames, fit sweeps 360 in all, 60.0 per frame after the first; median PSNR after the first frame 17.35 dB; median grey residual 0.18831 -> 0.18831, <time> s fitting
  f0.pgm  PSNR 16.95 dB  <time> ms  fit cold 240 sweeps, residual 0.32257 -> 0.22687, target change 0.0000
  f1.pgm  PSNR 18.38 dB  <time> ms  fit warm 60 sweeps, residual 0.19669 -> 0.16683, target change 0.1437
  f2.pgm  PSNR 18.02 dB  <time> ms  fit cold 240 sweeps, residual 0.20480 -> 0.09783, target change 0.1872
play :img: 3 frames, 60 sweeps, warm, soft: median PSNR 18.02 dB (worst 16.95), <time> frames/s settling, <time> frames/s with file work; fit 4 x 60 sweeps cold on the first frame, then warm 2 x 30 from correction, cut above 0.16, step 0.5: 1 warm and 2 cold frames, fit sweeps 540 in all, 150.0 per frame after the first; median PSNR after the first frame 18.20 dB; median grey residual 0.20480 -> 0.16683, <time> s fitting
```

**Errors:**

- `no grid :<name> (declare it with: grid :<name>, width: 64, height: 48)`
- `` play does not take `<key>:` ``
- `play needs `frames:` (a folder of .pgm pictures)`
- `` play needs `sweeps:` ``
- `sweeps must be at least 1`
- `keep must be above 0 and at most 1`
- `read: takes :bits (yes-rate), :soft (average of tanh of each input) or :rb (Rao-Blackwellised, at each draw)`
- `correct: takes :yes or :mean (mean-field), :tap (TAP), :bethe (Bethe) or :no`
- `copies must be a whole number from 1 to 4096`
- `update: takes :gibbs, :checker, :metro, :metro_checker or :cluster` (also for a bad `fit_update:`)
- `fit must be a whole number from 0 to 1000`
- `fit_sweeps must be a whole number of at least 4`
- `warm_fit must be a whole number from 0 to 1000`
- `warm_fit_sweeps must be a whole number of at least 4`
- `warm_from: takes :leans or :correction`
- `warm_step must be from 0 (keep the previous leans) to 1`
- `cut must be an RMS grey change from 0 (off) to 1`
- `warm_fit_sweeps:, warm_from:, warm_step: and cut: need warm_fit: 1 or more`
- `warm_fit: needs fit: (the first frame's cold fit)`
- `temperature must be above zero`
- `expected :yes or :no` (for `warm:` or `quiet:`)
- `a number was expected` or `a "quoted" string was expected` (a value of the wrong type)
- `cannot read frames folder <dir>: <reason>`
- `no .pgm frames in <dir>`
- `against: <dir> has <n> frames, fewer than the <m> played`
- `cannot make <dir>: <reason>`
- `<path> is <w>x<h> but grid :<name> is <w>x<h>` (for a frame or an `against:` frame)
- `cannot write <path>: <reason>`
- the picture errors listed under [Picture files](#picture-files)

## Picture files

The family reads and writes the binary forms of the Netpbm formats: PGM (magic `P5`, one grey plane) here, and
PPM (magic `P6`, three planes red, green and blue) in the [colour family](colour.md). The same reader serves
both.

**Reading.** The header is four fields separated by whitespace: the magic, the width, the height and the
maximum value. A `#` at the start of a field begins a comment that runs to the end of the line. Exactly one
whitespace byte follows the maximum value, then the pixel data. The maximum value may be from 1 to 65535. Below
256 each sample is one byte; from 256 up each sample is two bytes, most significant first. Each sample is
divided by the maximum value, so greys are in `[0, 1]`. Bytes after the pixel data are ignored. The plain text
forms (`P2`, `P3`) are refused.

**Writing.** `show_as` and `play` write an 8-bit PGM with the header `P5\n<w> <h>\n255\n`, each grey clamped to
`[0, 1]` and rounded to the nearest of 0 to 255. `play_colour` writes an 8-bit PPM the same way, with the header
`P6\n<w> <h>\n255\n` and the three samples of each pixel in the order red, green, blue.

The reader's errors, each printed after `line N: `:

- `cannot read <path>: <reason>`
- `<path>: the PGM header is cut short` (`PPM` for a colour frame)
- `<path>: only binary PGM (P5) is read, this file starts "<magic>"` (`PPM (P6)` for a colour frame)
- `<path>: bad header number "<text>"`
- `<path>: bad size <w>x<h> or maxval <v>`
- `<path>: <n> pixel bytes, expected <m>`

## Notes

- The per-frame times and the frames-per-second rates depend on the machine and its load. The documentation's
  examples print them as `<time>`.
- The names `<img>_<x>_<y>` must not already be in use in the model. `grid` does not check for this, and a grid
  whose names collide with earlier things does not get the pixel block it expects.
- A `play` with `warm_fit:` over a folder that holds only one frame stops with an internal error in this
  version (the median over the frames after the first is taken over no frames). Use two frames or more.
- `warm_step: 0` keeps the start leans only for a one-iteration warm fit. With two or more iterations the step
  size is raised to 1/64 whenever the residual grows, so the leans can move.
- Measured numbers for the options are in the reports listed at the top. For example, on the Muybridge horse at
  150 x 100 with a pull of 0.2 and TAP leans, `read: :rb` scored 35.44 dB against 26.71 dB for `read: :bits`
  at 80 sweeps (`filmsharp/REPORT_FILMSHARP.md`, section 3).
