# The colour family

The colour family plays colour pictures. A colour picture becomes three [grids](grid.md), one for each channel
(red, green and blue), with no pull between the channels. Each channel is played exactly as `play` plays a
greyscale grid: a channel value is a yes-rate, and each pixel's lean aims its yes-rate at that value. The family
scores each channel and the whole picture.

The source is `src/colour.rs`. It uses the grid family's player and options (`src/grid.rs`) and the options
that `src/filmsharp.rs` adds to them. The measurements are in
`experiments/thermosim/runs/gridplayer2/REPORT_GRIDPLAYER2.md` (section 3, where the family was built) and
`experiments/thermosim/runs/filmsharp/REPORT_FILMSHARP.md` (section 5), both on a clip of Tears of Steel.

| Statement | Block | Summary |
|---|---|---|
| [`colour`](#colour) | model | declare three grids, one per channel |
| [`play_colour`](#play_colour) | run | reproduce every frame of a folder of colour pictures |

## `colour`

**Block:** model.

**Form:**

```text
colour :film, width: 160, height: 67, smooth: 0
```

**Arguments:**

| Argument | Type | Default | Meaning |
|---|---|---|---|
| `:film` | symbol | required | the picture's name |
| `width:` | whole number | required | pixels per row |
| `height:` | whole number | required | rows |
| `smooth:` | number | `0` | the pull between each pixel and each of its four neighbours within one channel |

**What it does:** declares three grids named `<film>_r`, `<film>_g` and `<film>_b`, in that order, each exactly
as `grid :<film>_<c>, width:, height:, smooth:` would. So the things are `<film>_r_<x>_<y>` and so on, and the
model gains `3 * width * height` things. No pull joins two channels. The family records the picture in the
model's `notes` under `colour:<film>`.

Each channel is an ordinary grid, so the grid statements work on it by name: `film_g.lean_from`,
`film_g.show_as`, and `play :film_g`.

**Output:** none.

**Example:**

```settle example=colour-channels
# Each channel is an ordinary grid: its leans can be set and its yes-rates written like any grid's.
model :film do
  colour :film, width: 24, height: 16, smooth: 0.1
  film_g.lean_from "data/grid-frames/f0.pgm"   # the green channel leans toward a greyscale picture
end
run :film do
  settle 200, seed: 3
  film_g.show_as "colour-channels-green.pgm"
end
```

Output:

```text output=colour-channels
settled: 200 samples of 1152 things at temperature 1
show_as :film_g -> colour-channels-green.pgm (rate), PSNR 28.97 dB against the leans' picture
```

**Errors:**

- `colour :<name> is already declared`
- `grid :<name>_r is already declared` (a grid of that name exists already)
- `` grid does not take `<key>:` `` (the arguments are checked by the grid declaration)
- `` grid needs `width:` and `height:` ``
- `a grid needs between 1 and 4,000,000 pixels` (per channel)
- `a number was expected`

## `play_colour`

**Block:** run.

**Form:**

```text
play_colour :film, frames: "dir/", sweeps: 20, out: "dir2/", against: "other/", warm: :yes, read: :bits,
                   keep: 1, by: 1, correct: :mean, copies: 1, update: :gibbs, fit: 0, fit_sweeps: 200,
                   fit_update: :gibbs, temperature: 1, seed: 1, quiet: :no
```

A statement is one line; the form is split here only for reading.

**Arguments:**

| Argument | Type | Default | Meaning |
|---|---|---|---|
| `:film` | symbol | required | the colour picture to play on |
| `frames:` | path string | required | a folder of `.ppm` frames, each of the picture's size |
| `sweeps:` | whole number | required | sweeps per frame, per channel and per copy; at least 1 |
| `out:` | path string | none | a folder to write each reproduced frame to, as an 8-bit PPM under the frame's file name; created if missing |
| `against:` | path string | none | score frame `k` against the `k`-th `.ppm` in this folder instead of against its own target |
| `warm:` | `:yes` / `:no` | `:yes` | start each frame from the state the previous frame ended in, per channel |
| `read:` | one of `:bits` / `:soft` / `:rb` | `:bits` | how a frame is read from the sweeps |
| `keep:` | number above 0, at most 1 | `1` | the share of the last sweeps that is counted |
| `by:` | number | `1` | multiplies the `atanh(m)` part of each lean |
| `correct:` | one of `:mean` / `:yes` / `:tap` / `:bethe` / `:no` | `:mean` | the inversion from value to lean |
| `copies:` | whole number, 1 to 4096 | `1` | independent copies of each channel whose counts are pooled |
| `update:` | one of `:gibbs` / `:checker` / `:metro` / `:metro_checker` / `:cluster` | `:gibbs` | how each sweep updates the pixels |
| `fit:` | whole number, 0 to 1000 | `0` | iterations of the lean fit run for each channel of each frame |
| `fit_sweeps:` | whole number, at least 4 | `200` | sweeps per fit iteration |
| `fit_update:` | as `update:` | the value of `update:` | the update rule of the fit's own chain |
| `temperature:` | number above 0 | the run's temperature | sets the run's temperature, for this play and for later statements |
| `seed:` | number | the run's generator | replaces the run's random number generator with one seeded with this number |
| `quiet:` | `:yes` / `:no` | `:no` | `:yes` prints only the summary line |

Each option means exactly what it means for the grid family's [`play`](grid.md#play), where the reads, the
update rules, the inversions and the fit are described. `play_colour` does not take the warm-fit options
(`warm_fit:`, `warm_fit_sweeps:`, `warm_from:`, `warm_step:`, `cut:`).

**What it does:** lists the `.ppm` files in `frames:` (the extension in any case), sorts them by path, and plays
them in that order. For each frame it plays the red channel, then green, then blue, each as one grid `play`
frame: set the channel grid's leans from that channel's values, fit them if `fit:` is above 0, run the sweeps,
and read the channel. The three channels share the run's random number generator, and each channel keeps its own
warm state from frame to frame. The three read channels form the output picture, which is scored and, with
`out:`, written.

Files are read and written as described in [Picture files](grid.md#picture-files). A frame must be a binary PPM
(`P6`).

The scores, with values in `[0, 1]` and `n = width * height` pixels:

```text
MSE_c = mean_i (out_c,i - target_c,i)^2
PSNR_c = 10 log10(1 / MSE_c)
overall PSNR = 10 log10(1 / ((MSE_r + MSE_g + MSE_b) / 3))
```

Each channel's PSNR uses that channel's squared error. The overall PSNR pools the squared errors of all
`3 * n` values, which is the usual colour PSNR. A PSNR is reported as 99 when its mean squared error is at most
`1e-12`.

For the bits read, the summary also gives the coin-noise law at no pulls for each channel. With
`S = copies * round(sweeps * keep)` samples per pixel (at least `copies`):

```text
law_c = 10 log10(S / mean_i g_c,i (1 - g_c,i))
```

This is the PSNR the bits read has when no pixel pulls another, because each sample is then an independent coin
with chance `g`. The line gives the median of `law_c` over frames, computed from the played frames' values. It
is a reference number: it is printed whatever `smooth:` is.

**State after a play.** Each channel grid's leans are its last frame's leans. The run's yes-counts hold the
**blue** channel's last-frame counts and zero for every other thing, because each channel's frame replaces the
counts and blue is played last. So after a `play_colour`, `film_b.show_as` writes the last frame's blue channel,
while `film_r.show_as` and `film_g.show_as` write black pictures. The last arrangement is blue's too.
`temperature:` and `seed:` persist for the rest of the run block.

**Output:** one line per frame unless `quiet: :yes`:

```text
  <file>  R <r> G <g> B <b> overall <o> dB
```

then the summary:

```text
play_colour :<film>: <frames> frames, [<K> copies x ]<sweeps> sweeps, <warm|cold>, <read>[<correct>][<update>][, scored against another shot]: median PSNR R <r> G <g> B <b> overall <o> dB (worst overall <w>)[ (coin-noise law at no pulls: R <r> G <g> B <b>)], <rate> frames/s settling
```

The markers `<read>`, `<correct>` and `<update>` are those of `play`'s summary. The medians are taken over frames.
The coin-noise part appears only for `read: :bits`. `<rate>` counts settling time only. The line does not report
the fit, `by:` or `temperature:`, even when they are used.

**Example:** a play with the default options, writing the reproduced frames.

```settle example=colour-play
# Play three 24 x 16 colour frames on three grids of p-bits, one per channel.
model :film do
  colour :film, width: 24, height: 16, smooth: 0.1   # grids film_r, film_g, film_b, 384 things each
end
run :film do
  play_colour :film, frames: "data/colour-frames/", out: "out/colour-play/", sweeps: 20, seed: 1
end
```

Output:

```text output=colour-play
  f0.ppm  R 19.25 G 18.80 B 21.88 overall 19.78 dB
  f1.ppm  R 19.10 G 19.04 B 22.20 overall 19.89 dB
  f2.ppm  R 19.58 G 19.33 B 22.27 overall 20.20 dB
play_colour :film: 3 frames, 20 sweeps, warm, bits: median PSNR R 19.25 G 19.04 B 22.20 overall 19.89 dB (worst overall 19.78) (coin-noise law at no pulls: R 19.59 G 19.59 B 22.14), <time> frames/s settling
```

**Example:** the soft read with TAP leans, then the negative control.

```settle example=colour-soft
# The soft read with TAP leans, then the same play scored against another shot.
model :film do
  colour :film, width: 24, height: 16, smooth: 0.1
end
run :film do
  play_colour :film, frames: "data/colour-frames/", sweeps: 20, read: :soft, correct: :tap, seed: 2, quiet: :yes
  play_colour :film, frames: "data/colour-frames/", sweeps: 20, read: :soft, correct: :tap, seed: 2, quiet: :yes, against: "data/colour-other/"
end
```

Output:

```text output=colour-soft
play_colour :film: 3 frames, 20 sweeps, warm, soft, tap: median PSNR R 33.78 G 34.31 B 41.34 overall 35.48 dB (worst overall 34.87), <time> frames/s settling
play_colour :film: 3 frames, 20 sweeps, warm, soft, tap, scored against another shot: median PSNR R 7.65 G 11.19 B 7.08 overall 8.31 dB (worst overall 8.17), <time> frames/s settling
```

**Example:** copies, `keep:`, a cold start, another update rule and a fit.

```settle example=colour-options
# The grid options work per channel: copies, keep, a cold start, another update rule, a fit.
model :film do
  colour :film, width: 24, height: 16, smooth: 0.1
end
run :film do
  # 2 copies of 10 sweeps, counting the last half of each copy's sweeps: 2 x 5 = 10 samples per pixel
  play_colour :film, frames: "data/colour-frames/", sweeps: 10, copies: 2, keep: 0.5, warm: :no, update: :checker, seed: 3, quiet: :yes
  # Bethe leans refined by a 2-iteration fit per channel and frame, read Rao-Blackwellised
  play_colour :film, frames: "data/colour-frames/", sweeps: 20, read: :rb, correct: :bethe, fit: 2, fit_sweeps: 20, seed: 3, quiet: :yes
end
```

Output:

```text output=colour-options
play_colour :film: 3 frames, 2 copies x 10 sweeps, cold, bits, checker: median PSNR R 16.22 G 16.08 B 19.26 overall 16.98 dB (worst overall 16.81) (coin-noise law at no pulls: R 16.58 G 16.58 B 19.13), <time> frames/s settling
play_colour :film: 3 frames, 20 sweeps, warm, rb, bethe: median PSNR R 30.30 G 29.92 B 37.48 overall 31.50 dB (worst overall 31.22), <time> frames/s settling
```

**Example:** the warm-fit options belong to `play` only.

```settle example=colour-no-warm-fit
# play_colour takes the fit options but not the warm-fit options of play.
model :film do
  colour :film, width: 24, height: 16, smooth: 0.1
end
run :film do
  play_colour :film, frames: "data/colour-frames/", sweeps: 20, fit: 2, warm_fit: 1
end
```

```text error=colour-no-warm-fit
line 6: play_colour does not take `warm_fit:`
```

**Errors:**

- `no colour :<name> (declare it with: colour :<name>, width: 160, height: 67)`
- `` play_colour does not take `<key>:` ``
- `play_colour needs `frames:` (a folder of .ppm pictures)`
- `` play_colour needs `sweeps:` ``
- `sweeps must be at least 1`
- `keep must be above 0 and at most 1`
- `read: takes :bits (yes-rate), :soft (average of tanh of each input) or :rb (Rao-Blackwellised, at each draw)`
- `correct: takes :yes or :mean (mean-field), :tap (TAP), :bethe (Bethe) or :no`
- `copies must be a whole number from 1 to 4096`
- `update: takes :gibbs, :checker, :metro, :metro_checker or :cluster` (also for a bad `fit_update:`)
- `fit must be a whole number from 0 to 1000`
- `fit_sweeps must be a whole number of at least 4`
- `temperature must be above zero`
- `expected :yes or :no` (for `warm:` or `quiet:`)
- `a number was expected` or `a "quoted" string was expected` (a value of the wrong type)
- `cannot read frames folder <dir>: <reason>`
- `no .ppm frames in <dir>`
- `against: <dir> has <n> frames, fewer than the <m> played`
- `cannot make <dir>: <reason>`
- `<path> is <w>x<h> but colour :<name> is <w>x<h>` (for a frame or an `against:` frame)
- `cannot write <path>: <reason>`
- the picture errors listed under [Picture files](grid.md#picture-files), with `PPM` in place of `PGM`

## Notes

- The frames-per-second rate depends on the machine and its load. The documentation's examples print it as
  `<time>`.
- The per-frame lines do not mark an `against:` play; only the summary does.
