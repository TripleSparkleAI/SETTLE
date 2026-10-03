# The valleys family

The valleys family measures the shape of a model's energy landscape: how many valleys it has, how large each
valley's basin is, and which valleys are mirror images of each other. `valleys` does this exactly, by visiting
every arrangement of up to 24 free things. `survey` does it by sampling: it shakes from many random starts,
rolls each one down to the bottom, and counts where they end. The family also adds `landscape`, which builds four
kinds of test landscape into a model: random pulls, a grid, a ring and a parity code.

When a model has a memory from [the memory family](memory.md), or a code landscape, both statements label each
valley: a stored pattern, a mirror image of one, a fake, a codeword, or not a codeword.

The source is `src/valleys.rs`. The measurements are in `experiments/thermosim/runs/valleymap/REPORT_VALLEYMAP.md`.
For example, a landscape of 20 things with random pulls has about 58 valleys on average, where a table of the same
energies shuffled at random has about 49,900.

| Statement | Block | Summary |
|---|---|---|
| [`landscape`](#landscape) | model | build a random, grid, ring or code landscape |
| [`valleys`](#valleys) | run | list every valley and its exact basin (up to 24 free things) |
| [`survey`](#survey) | run | estimate the valleys by shaking and quenching from many random starts |

## Valleys, floors, saddles and basins

The energy of an arrangement `s` is `E(s) = - sum_i h_i s_i - sum_{i<j} J_ij s_i s_j`. Flipping thing `i` alone
changes it by:

```text
dE_i = 2 s_i (h_i + sum_j J_ij s_j)
```

A flip with `dE_i < 0` goes downhill.

- A **valley** is an arrangement from which no single flip goes downhill. If some flips leave the energy
  unchanged, the valley is the whole **flat floor**: every arrangement reachable from it by such flips. A flat
  floor counts as a valley only if none of its members has a downhill flip.
- A flat floor that has a way down from one of its members is a **saddle**, not a valley.
- **Steepest descent** from an arrangement takes the flip with the most negative `dE_i` until none is negative.
  When two flips are equally steep, the thing with the lower index is flipped.
- The **basin** of a valley is the share of all `2^n` arrangements whose steepest descent ends in it.
- A **mirror image** of an arrangement has every free thing flipped. Without leans, a landscape's valleys come in
  mirror pairs with equal energies and equal basins.

"No change" is judged with a tolerance: a flip whose `|dE_i|` is at most `1e-9` times the largest size of any
lean or pull (or `1e-9`, if every lean and pull is smaller than 1) counts as flat, and a flip must be steeper by
more than that tolerance to beat another.

The overlap of two arrangements `a` and `b` of `n` things is `q = (1 / n) sum_i a_i b_i`: +1 when they are the
same, -1 when one is the mirror image of the other, near 0 when they are unrelated.

## `landscape`

**Block:** model.

**Form:**

```text
landscape :random, size: 16, seed: 1, scale: 1, field: 0
landscape :grid, width: 8, height: 8, wrap: :yes
landscape :ring, size: 12
landscape :code, bits: 12, checks: 6, seed: 1, strength: 1
```

**Arguments:**

| Argument | Type | Default | Meaning |
|---|---|---|---|
| `:kind` | one of `:random` / `:grid` / `:ring` / `:code` | required | which landscape to build |
| `size:` (random) | whole number, 2 to 100000 | `16` | the number of things |
| `seed:` (random) | whole number | `1` | seeds the random pulls and leans |
| `scale:` (random) | number | `1` | the size of the pulls |
| `field:` (random) | number | `0` | the size of the random leans, relative to `scale` |
| `width:` (grid) | whole number | `8` | things per row |
| `height:` (grid) | whole number | `8` | number of rows |
| `wrap:` (grid) | `:yes` / `:no` | `:yes` | join the last column to the first and the last row to the first |
| `size:` (ring) | whole number, at least 3 | `12` | the number of things |
| `bits:` (code) | whole number, 3 to 64 | `12` | the number of data things |
| `checks:` (code) | whole number, at least `bits / 3` | `6` | the number of parity checks, each with one helper thing |
| `seed:` (code) | whole number | `1` | seeds which bits each check covers |
| `strength:` (code) | number | `1` | the penalty for a broken check |

Each kind accepts only its own keyword arguments. Numbers given for whole-number arguments are cut to whole
numbers. The comma after `:kind` is optional.

**What it does:**

- `:random` adds `size` things named `v0` to `v<size - 1>`. Every pair is pulled or pushed by a random amount, and
  when `field` is not 0 every thing also gets a random lean:

  ```text
  J_ik = scale * z_ik / sqrt(size)        h_i = scale * field * z_i
  ```

  where each `z` is a standard normal number from a generator seeded by `seed:`. The pulls are drawn first, then
  the leans.
- `:grid` adds `width * height` things named `g0` onward, row by row: the thing in row `r` and column `c` is
  `g<r * width + c>`. Every thing pulls its right and lower neighbour by 1. With `wrap: :yes` the last column
  also pulls the first column and the last row pulls the first row, which makes a torus. A wrapped grid needs a
  width and height of at least 3.
- `:ring` adds `size` things named `r0` onward. Each pulls the next by 1, and the last pulls the first.
- `:code` adds `bits` data things named `d0` onward and `checks` helper things named `x0` onward. Each check
  covers 3 data bits. The first checks take the bits of a random order three at a time, so that every bit is in
  at least one check; checks beyond that take random bits. Check `k` with bits `a`, `b`, `c` and helper `x`
  (all read as 0 or 1) adds the penalty

  ```text
  strength * (a + b + c - 2 x)^2
  ```

  as leans and pulls. It is 0 when the check is even and the helper is set to half its sum, and at least
  `strength` when the check is odd. So the calmest arrangements are the codewords: the data arrangements with
  every check even, each with its helpers set. The leans and pulls differ from this penalty only by a constant.
  The checks are recorded in the model's notes under `valleys:code`, which `valleys` and `survey` use to label
  codewords.

A model can hold one landscape of each kind. All of them add to the model's existing things.

**Output:** none.

**Example:** see [`valleys`](#valleys).

**Errors:**

- `no landscape :<kind> (try :random, :grid, :ring or :code)`
- `` landscape :<kind> does not take `<key>:` ``
- `this model already has a :<kind> landscape`
- `size must be between 2 and 100000`
- `a wrapped grid needs width and height of at least 3`
- `a ring needs at least 3 things`
- `a code needs between 3 and 64 bits`
- `<checks> checks of 3 bits cannot cover <bits> bits; an unchecked bit makes a flat valley`
- `thing :<name> already exists; one landscape of this kind per model` (code landscape)
- `a number was expected`
- `expected :yes or :no`

## `valleys`

**Block:** run.

**Form:**

```text
valleys show: 10
```

**Arguments:**

| Argument | Type | Default | Meaning |
|---|---|---|---|
| `show:` | whole number | `10` | how many valleys to list, largest basin first |

**What it does:** works on the run's free things. Each held thing is fixed at its held value and folded into the
leans of the free things it pulls. With `n` free things, `valleys` visits all `2^n` arrangements, finds every
valley and flat saddle, and follows steepest descent from every arrangement to get each valley's exact basin.
It refuses more than 24 free things. At 24 it keeps about 150 MB of tables and takes a few seconds.

The listed energy of a valley counts only terms that involve a free thing. It leaves out the leans of held things
and the pulls between two held things, so when things are held it can differ from the model's energy by a
constant.

`valleys` uses no randomness and changes nothing in the model or the run.

If the model has a memory, each listed valley is compared with the patterns of the memory whose name comes first
in alphabetical order: an overlap of at least 0.9 with a pattern labels it `stored`, an overlap of at most -0.9
labels it `mirror of`, and anything else labels it `fake`, with the overlap of largest size. If the model has no
memory but has a code landscape, each valley is labelled `codeword` or `not a codeword`.

**Output:**

```text
valleys (exact, all <2^n> arrangements of <n> free things): <count> valleys, <m> of them in mirror pairs, calmest energy <energy>
  energy <energy>  basin <percent>% <bar><bits><floor><label>
  ... <k> more valleys
  steepest descent stops on a flat saddle from <percent>% of arrangements
```

- `<m>` counts the valleys that are single arrangements and whose mirror image is also a valley.
- One `energy` line is printed for each of the `show` valleys with the largest basins. Valleys with equal basins
  are listed calmest first.
- `<bar>` is one `#` per thirtieth of all arrangements in the basin, padded to 30 characters.
- `<bits>` is a space followed by the whole arrangement of the model, `1` for yes and `0` for no, in index order,
  held things included. It is left out when the model has more than 64 things.
- `<floor>` is `  flat floor of <k>` when the valley is a flat floor of `k` arrangements.
- `<label>` is two spaces followed by `stored :<pattern>`, `mirror of :<pattern>`, `fake (best overlap <q>)`,
  `codeword` or `not a codeword (<k> checks broken)`, or nothing.
- The `... more valleys` line appears only when there are more valleys than `show`. The saddle line appears only
  when some steepest descents stop on a saddle.

**Example:** a ring has two valleys, all yes and all no.

```settle example=valleys-ring
# a ring of 8 things, each pulling the next: the only valleys are all-yes and all-no
model :ring do
  landscape :ring, size: 8        # things r0 .. r7
end

run :ring do
  valleys                         # exact: visits all 2^8 arrangements
end
```

Output:

```text output=valleys-ring
valleys (exact, all 256 arrangements of 8 free things): 2 valleys, 2 of them in mirror pairs, calmest energy -8.000
  energy    -8.000  basin  16.41% #####                          00000000
  energy    -8.000  basin  16.41% #####                          11111111
  steepest descent stops on a flat saddle from 67.19% of arrangements
```

**Example:** a grid, open and wrapped.

```settle example=valleys-grid
# a 4 x 3 grid where every thing pulls its neighbours by 1, open and wrapped into a torus
model :open do
  landscape :grid, width: 4, height: 3, wrap: :no   # things g0 .. g11, row by row
end

model :torus do
  landscape :grid, width: 4, height: 3              # wrap: :yes is the default
end

run :open do
  valleys show: 4
end

run :torus do
  valleys show: 4
end
```

Output:

```text output=valleys-grid
valleys (exact, all 4096 arrangements of 12 free things): 4 valleys, 4 of them in mirror pairs, calmest energy -17.000
  energy   -17.000  basin  31.45% #########                      000000000000
  energy   -17.000  basin  31.45% #########                      111111111111
  energy   -11.000  basin   4.71% #                              110011001100
  energy   -11.000  basin   4.71% #                              001100110011
  steepest descent stops on a flat saddle from 27.69% of arrangements
valleys (exact, all 4096 arrangements of 12 free things): 6 valleys, 6 of them in mirror pairs, calmest energy -24.000
  energy   -24.000  basin  29.88% #########                      000000000000
  energy   -24.000  basin  29.88% #########                      111111111111
  energy   -12.000  basin   2.86% #                              110011001100
  energy   -12.000  basin   2.86% #                              001100110011
  ... 2 more valleys
  steepest descent stops on a flat saddle from 29.10% of arrangements
```

**Example:** a random landscape, listed exactly and then surveyed.

```settle example=valleys-random
# a random landscape of 12 things, measured exactly and then by sampling
model :glass do
  landscape :random, size: 12, seed: 1, field: 0.3   # random pulls, and small random leans
end

run :glass do
  valleys show: 4                                     # the 4 valleys with the largest basins
  survey starts: 300, sweeps: 20, temperature: 0.05, seed: 1, show: 3
end
```

Output:

```text output=valleys-random
valleys (exact, all 4096 arrangements of 12 free things): 14 valleys, 6 of them in mirror pairs, calmest energy -9.657
  energy    -9.657  basin  24.49% #######                        111001010110
  energy    -7.328  basin  18.14% #####                          011111111010
  energy    -8.128  basin  14.87% ####                           000110101001
  energy    -7.289  basin  11.82% ####                           100000010100
  ... 10 more valleys
survey: 300 starts, 20 sweeps at temperature 0.05 then a quench: 11 valleys found (1 seen once; Chao1 estimate 11)
  calmest found: energy -9.657, reached from 23.0% of starts
  #1   energy    -9.657  basin  23.00% #######                        111001010110  mirror of #4
  #2   energy    -7.328  basin  20.67% ######                         011111111010
  #3   energy    -7.289  basin  16.67% #####                          100000010100
  overlaps among the top valleys (+1 same, -1 mirror, 0 unrelated):
    +1.00 +0.00 +0.33
    +0.00 +1.00 -0.67
    +0.33 -0.67 +1.00
```

**Example:** in a code landscape every codeword is a valley at the lowest energy.

```settle example=valleys-code
# a code landscape: 8 data bits, 4 parity checks of 3 bits, one helper thing per check
model :code do
  landscape :code, bits: 8, checks: 4, seed: 1   # things d0 .. d7 and x0 .. x3
end

run :code do
  valleys show: 16                               # each valley is labelled codeword or not
end
```

Output:

```text output=valleys-code
valleys (exact, all 4096 arrangements of 12 free things): 16 valleys, 0 of them in mirror pairs, calmest energy -8.000
  energy    -8.000  basin   7.20% ##                             000000000000  codeword
  energy    -8.000  basin   6.01% ##                             010000100100  codeword
  energy    -8.000  basin   3.78% #                              100100011010  codeword
  energy    -8.000  basin   3.20% #                              110100111110  codeword
  energy    -8.000  basin   2.95% #                              001101011011  codeword
  energy    -8.000  basin   2.83% #                              010111001101  codeword
  energy    -8.000  basin   2.83% #                              000111101101  codeword
  energy    -8.000  basin   2.69% #                              011010010111  codeword
  energy    -8.000  basin   2.66% #                              011101111111  codeword
  energy    -8.000  basin   2.47% #                              101001001011  codeword
  energy    -8.000  basin   2.39% #                              001010110111  codeword
  energy    -8.000  basin   2.39% #                              111110001111  codeword
  energy    -8.000  basin   2.17% #                              101110101111  codeword
  energy    -8.000  basin   2.12% #                              111001101111  codeword
  energy    -8.000  basin   2.03% #                              110011011111  codeword
  energy    -8.000  basin   1.93% #                              100011111111  codeword
  steepest descent stops on a flat saddle from 50.34% of arrangements
```

**Example:** held things are folded into the leans of the free things, so fewer arrangements are visited.

```settle example=valleys-held
# a held thing is folded into its neighbours' leans; valleys and survey then vary only the free things
model :weather do
  thing :rain,      leans: :no, by: 1
  thing :sprinkler, leans: :no, by: 0.5
  thing :wet_grass
  rain.pushes :sprinkler, by: 0.5
  rain.pulls  :wet_grass, by: 1.5
  sprinkler.pulls :wet_grass, by: 1
end

run :weather do
  valleys                          # all 3 things free
  hold :wet_grass, :yes
  valleys                          # 2 free things
end
```

Output:

```text output=valleys-held
valleys (exact, all 8 arrangements of 3 free things): 1 valleys, 0 of them in mirror pairs, calmest energy -3.500
  energy    -3.500  basin  75.00% #######################        000
  steepest descent stops on a flat saddle from 25.00% of arrangements
valleys (exact, all 4 arrangements of 2 free things): 1 valleys, 0 of them in mirror pairs, calmest energy -0.500
  energy    -0.500  basin 100.00% ############################## 101  flat floor of 3
```

With `:wet_grass` held at yes, three of the four arrangements of the other two things have the same energy and
form one flat floor.

**Example:** more than 24 free things.

```settle example=valleys-too-large
# valleys refuses more than 24 free things; survey has no limit
model :big do
  landscape :grid, width: 5, height: 5    # 25 things, wrapped into a torus
end

run :big do
  valleys
end
```

```text error=valleys-too-large
line 7: valleys is exact and visits every arrangement: 25 free things is above 24; use survey
```

**Errors:**

- `valleys is exact and visits every arrangement: <n> free things is above 24; use survey`
- `` valleys does not take `<key>:` ``
- `a number was expected`

## `survey`

**Block:** run.

**Form:**

```text
survey starts: 1000, sweeps: 50, temperature: 0.05, seed: 1, show: 8
```

**Arguments:**

| Argument | Type | Default | Meaning |
|---|---|---|---|
| `starts:` | whole number, at least 1 | `1000` | how many random starts |
| `sweeps:` | whole number | `50` | how many sweeps to shake each start for |
| `temperature:` | number above 0 | `0.05` | the temperature of the shaking |
| `seed:` | whole number | the run's generator | replace the run's random generator with one seeded by this number |
| `show:` | whole number | `8` | how many valleys to list, most often reached first |

**What it does:** if `seed:` is given, the run's random generator is replaced and kept for the rest of the run.
Then, for each start:

1. Every free thing gets a random value, and every held thing its held value.
2. The model is swept `sweeps` times at `temperature`, with the same update rule as `settle`.
3. The arrangement is quenched: in a fresh random order each pass, every free thing whose flip lowers the energy
   by more than the tolerance is flipped, until a whole pass flips nothing (at most 100,000 passes). The result is
   a valley bottom: no single flip goes downhill.
4. The result is counted by its exact arrangement. It is marked flat if some flip leaves the energy unchanged
   within the tolerance, which means it sits on a flat floor or a saddle.

`survey` has no size limit. Its "basin" of a valley is the share of starts that ended there, which estimates the
true basin under shaking and quenching (not under steepest descent, which `valleys` uses). The listed energy is
the model's full energy. The survey does not change the model, the run's samples or its last arrangement.

The number of valleys the starts missed is estimated with the Chao1 estimator:

```text
V = V_obs + f1^2 / (2 f2)            (or V_obs + f1 (f1 - 1) / 2 when f2 is 0)
```

`V_obs` is the number of valleys found, `f1` the number found by exactly one start, and `f2` the number found by
exactly two. Many valleys seen only once means many are still unseen.

**Output:**

```text
survey: <starts> starts, <sweeps> sweeps at temperature <t> then a quench: <count> valleys found (<f1> seen once; Chao1 estimate <V>)
  calmest found: energy <energy>, reached from <percent>% of starts
  #<rank> energy <energy>  basin <percent>% <bar><bits><flat><mirror><label>
  overlaps among the top valleys (+1 same, -1 mirror, 0 unrelated):
    <q> <q> ...
  <percent>% of starts stopped on a flat floor or saddle
  kinds: <kind> <percent>% of starts (<k> valleys) · ...
```

- The `calmest found` share counts every start that ended at the lowest energy found.
- One `#` line is printed for each of the `show` valleys reached most often. Valleys reached equally often are
  listed calmest first. `<bar>`, `<bits>` and `<label>` are as in `valleys`. `<flat>` is `  (flat)` for a flat
  result. `<mirror>` is `  mirror of #<j>` when the mirror image of the whole arrangement was also found.
- The overlap table covers the first `show` valleys, at most 5, and appears only when there are at least two.
- The flat line appears only when some start stopped on a flat floor or saddle.
- The `kinds` line appears only when the model has a memory or a code landscape. It sums the starts and valleys of
  each label, in alphabetical order, separated by ` · `.

**Example:** the valleys of a small memory, exactly and by survey.

```settle example=valleys-memory
# the valleys of a small memory: stored patterns, their mirror images, and fakes
model :mind do
  memory :m, size: 16                 # things m_0 .. m_15
  m.remember :cat
  m.remember :dog
  m.remember :owl
  thing :m_0, leans: :yes, by: 0.05   # a tiny lean, so no two valleys have exactly the same energy
end

run :mind do
  valleys show: 8                     # exact
  survey starts: 400, seed: 1, show: 4
end
```

Output:

```text output=valleys-memory
valleys (exact, all 65536 arrangements of 16 free things): 8 valleys, 8 of them in mirror pairs, calmest energy -7.800
  energy    -7.675  basin  19.05% ######                         1101010001111000  stored :owl
  energy    -7.575  basin  17.19% #####                          0010101110000111  mirror of :owl
  energy    -7.800  basin  15.78% #####                          1100011001000000  mirror of :cat
  energy    -7.700  basin  15.56% #####                          0011100110111111  stored :cat
  energy    -6.675  basin  13.06% ####                           1111001010010001  stored :dog
  energy    -6.575  basin  10.94% ###                            0000110101101110  mirror of :dog
  energy    -7.300  basin   4.23% #                              1101011001010000  fake (best overlap -0.75)
  energy    -7.200  basin   4.19% #                              0010100110101111  fake (best overlap +0.75)
survey: 400 starts, 50 sweeps at temperature 0.05 then a quench: 6 valleys found (0 seen once; Chao1 estimate 6)
  calmest found: energy -7.800, reached from 15.8% of starts
  #1   energy    -7.700  basin  19.75% ######                         0011100110111111  mirror of #4  stored :cat
  #2   energy    -7.675  basin  18.50% ######                         1101010001111000  mirror of #3  stored :owl
  #3   energy    -7.575  basin  17.75% #####                          0010101110000111  mirror of #2  mirror of :owl
  #4   energy    -7.800  basin  15.75% #####                          1100011001000000  mirror of #1  mirror of :cat
  overlaps among the top valleys (+1 same, -1 mirror, 0 unrelated):
    +1.00 -0.38 +0.38 -1.00
    -0.38 +1.00 -1.00 +0.38
    +0.38 -1.00 +1.00 -0.38
    -1.00 +0.38 -0.38 +1.00
  kinds: mirror 47.5% of starts (3 valleys) · stored 52.5% of starts (3 valleys)
```

The tiny lean on `m_0` gives every valley a different energy. Without it, a pattern and its mirror image have the
same energy, and the order of two valleys reached equally often could change from one run to the next (see Notes).

**Errors:**

- `temperature must be above zero`
- `survey needs at least one start`
- `` survey does not take `<key>:` ``
- `a number was expected`

## Notes

- **Survey order.** `survey` sorts its valleys by how often they were reached, then by energy. When two valleys
  were reached equally often and have the same energy, their order is not fixed and can differ between runs of
  the same program with the same seed. Every other number the survey prints is the same from run to run.
- **Keyed memories are labelled as fakes.** The labels use the public pattern of each stored name. A note saved
  with `key:` is stored as a turned pattern that the valleys family does not know, so its valleys are labelled
  `fake`.
- **Thing names.** The landscapes add things named `v`, `g`, `r`, `d` and `x` followed by a number. Build a
  landscape before declaring any thing with one of those names. A `:random`, `:grid` or `:ring` landscape whose
  first name (`v0`, `g0` or `r0`) is free but whose later names are taken stops the interpreter with an internal
  error instead of a SETTLE error.
