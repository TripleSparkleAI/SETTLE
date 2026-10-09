# Cookbook

Short recipes for common tasks. Each recipe is a complete program with its exact output; the explanation says
which statements do the work and links to their reference pages. Relative file paths in the recipes (`data/...`)
refer to files in `docs/examples/`, where every recipe is stored.

## Condition on evidence

Hold what you observed, settle, and ask about the rest. Each observation gets its own run block here, so that
each starts from the same seed and only the holds differ.

```settle example=cook-explaining-away
# The alarm rang. Was it a burglary? Then we learn there was an earthquake.
model :alarm do
  thing :burglary,   leans: :no, by: 1      # burglaries are unusual
  thing :earthquake, leans: :no, by: 1      # so are earthquakes
  thing :alarm,      leans: :no, by: 1      # the alarm is usually quiet
  burglary.pulls   :alarm, by: 1.5          # either cause sets it off
  earthquake.pulls :alarm, by: 1.5
  burglary.pushes  :earthquake, by: 1       # one cause is enough: the two compete to explain the alarm
end

run :alarm do
  settle 40_000, seed: 1
  ask :burglary                             # before we hear anything
end

run :alarm do
  hold :alarm, :yes                         # we hear the alarm
  settle 40_000, seed: 1
  ask :burglary
end

run :alarm do
  hold :alarm, :yes
  hold :earthquake, :yes                    # and then the radio reports an earthquake
  settle 40_000, seed: 1
  ask :burglary                             # the earthquake explains the alarm away
end
```

```text output=cook-explaining-away
settled: 40000 samples of 3 things at temperature 1
ask :burglary: yes 5.4% of 40000 samples
settled: 40000 samples of 3 things at temperature 1
ask :burglary: yes 56.0% of 40000 samples
settled: 40000 samples of 3 things at temperature 1
ask :burglary: yes 26.8% of 40000 samples
```

Hearing the alarm raises the chance of a burglary from 5% to 57%. Learning of the earthquake lowers it again to
27%: the earthquake explains the alarm away.

The line `burglary.pushes :earthquake` is what makes that happen. SETTLE models only pairs of things, so it
cannot express "the alarm needs one cause or the other" as a single three-way rule. Without the push, burglary
and earthquake are independent once the alarm is held, and the report of an earthquake would leave the chance of
a burglary unchanged. The push says the two causes compete. See [`hold`](05-statements/core.md#hold) and
[Semantics](04-semantics.md#energy-and-probability).

### The last question on its own

The model and one run block make a complete program. Every run block starts from the seed it names, so the last of
the three runs above prints the same number on its own.

```settle example=cook-explaining-away-short
# The alarm rang. Then an earthquake was reported.
model :alarm do
  thing :burglary,   leans: :no, by: 1   # rare
  thing :earthquake, leans: :no, by: 1   # rare
  thing :alarm,      leans: :no, by: 1   # quiet
  burglary.pulls   :alarm, by: 1.5   # sets it off
  earthquake.pulls :alarm, by: 1.5   # this too
  burglary.pushes  :earthquake, by: 1  # competes
end

run :alarm do
  hold :alarm, :yes        # we hear the alarm
  hold :earthquake, :yes   # the radio reports one
  settle 40_000, seed: 1
  ask :burglary            # was it a burglary?
end
```

```text output=cook-explaining-away-short
settled: 40000 samples of 3 things at temperature 1
ask :burglary: yes 26.8% of 40000 samples
```

## Ask several questions of one sample

`ask` counts over the samples the last `settle` recorded, so one `settle` can answer many questions without
resampling. Terms combine left to right.

```settle example=cook-several-questions
# One settle answers many questions: each ask counts over the same samples.
model :weather do
  thing :rain,      leans: :no, by: 1
  thing :sprinkler, leans: :no, by: 0.5
  thing :wet_grass
  rain.pushes :sprinkler, by: 0.5
  rain.pulls  :wet_grass, by: 1.5
  sprinkler.pulls :wet_grass, by: 1
end

run :weather do
  settle 40_000, seed: 1
  ask :wet_grass                             # P(wet grass)
  ask :rain, and: :wet_grass                 # P(rain and wet grass)
  ask :rain, and_not: :wet_grass             # P(rain and dry grass)
  ask :rain, or: :sprinkler, and: :wet_grass # P((rain or sprinkler) and wet grass)
end
```

```text output=cook-several-questions
settled: 40000 samples of 3 things at temperature 1
ask :wet_grass: yes 11.7% of 40000 samples
ask :rain, and: :wet_grass: yes 7.5% of 40000 samples
ask :rain, and_not: :wet_grass: yes 1.4% of 40000 samples
ask :rain, or: :sprinkler, and: :wet_grass: yes 11.2% of 40000 samples
```

See [`ask`](05-statements/core.md#ask).

## Find the best arrangement

Use `anneal` and `best` when you want the single calmest arrangement rather than probabilities. Here, rivals
push apart and friends pull together, and the calmest arrangement is a seating plan.

```settle example=cook-seating
# Seat six guests at two tables (yes = table A, no = table B).
# Rivals push apart, friends pull together; anneal finds the calmest seating.
model :party do
  thing :ada, :bo, :cy, :dee, :eli, :fay
  ada.pushes :bo,  by: 3     # old rivals
  cy.pushes  :dee, by: 3
  eli.pushes :fay, by: 3
  ada.pulls  :cy,  by: 1     # friends
  bo.pulls   :eli, by: 1
  dee.pulls  :fay, by: 1
  ada.pushes :fay, by: 1     # a small grudge
  thing :ada, leans: :yes, by: 0.1   # put ada at table A, to pick one of two mirror seatings
end

run :party do
  anneal 5_000, seed: 3
  best
end
```

```text output=cook-seating
annealed: 5000 sweeps, calmest energy found -11.100
best (energy -11.100): ada yes, bo no, cy yes, dee no, eli yes, fay no
```

A model whose leans are all zero has two calmest arrangements, each the mirror image of the other (every yes
swapped for no). The small lean on `ada` picks one. See [`anneal`](05-statements/core.md#anneal).

## See how temperature changes an answer

Run several settles in one run block with different temperatures. `temperature:` stays in force for later
statements in the block, so give it on every `settle` that should differ.

```settle example=cook-temperature-sweep
# How strongly a chain of six things agrees, at four temperatures.
model :chain do
  thing :a, :b, :c, :d, :e, :f
  a.pulls :b, by: 1
  b.pulls :c, by: 1
  c.pulls :d, by: 1
  d.pulls :e, by: 1
  e.pulls :f, by: 1
end

run :chain do
  hold :a, :yes                      # pin one end
  settle 20_000, temperature: 0.5, seed: 1
  ask :f                             # how often the far end follows
  settle 20_000, temperature: 1, seed: 1
  ask :f
  settle 20_000, temperature: 2, seed: 1
  ask :f
  settle 20_000, temperature: 4, seed: 1
  ask :f
end
```

```text output=cook-temperature-sweep
settled: 20000 samples of 6 things at temperature 0.5
ask :f: yes 92.0% of 20000 samples
settled: 20000 samples of 6 things at temperature 1
ask :f: yes 61.8% of 20000 samples
settled: 20000 samples of 6 things at temperature 2
ask :f: yes 50.6% of 20000 samples
settled: 20000 samples of 6 things at temperature 4
ask :f: yes 50.0% of 20000 samples
```

At a low temperature the pulls dominate and the far end of the chain follows the held end; at a high
temperature each thing is close to a fair coin. For a chain of pulls `J` at temperature `T` with no leans, the
far end agrees with the held end with probability `(1 + tanh(J/T)^5) / 2` five links away, which is 91.6%,
62.8%, 51.1% and 50.0% for these four temperatures; the sampled values are within sampling error of these.

## Solve a puzzle

The zoo family builds the things and pulls for a puzzle from a short description, and `solution` decodes the
calmest arrangement and checks it against the puzzle's rules.

```settle example=zoo-sudoku
# A 4x4 sudoku with four givens. Rows are separated by spaces; '.' is an empty cell.
model :p do
  sudoku :s, size: 4, given: "1... .4.. ..4. ...1"
end

run :p do
  anneal 2_000, seed: 1   # look for the calmest arrangement
  s.solution              # decode it into a grid and check every rule
end
```

```text output=zoo-sudoku
annealed: 2000 sweeps, calmest energy found -136.000
  1 2 | 3 4
  3 4 | 1 2
  ---------
  2 1 | 4 3
  4 3 | 2 1
sudoku :s: VALID (checked rule by rule, not by energy)
```

See [the zoo family](05-statements/zoo.md) for colouring, max-cut, factoring and nonograms.

## Store text and recall it from a noisy read-address

A memory stores patterns in the pulls between its things. `save` stores text, and `recall` starts from a
scrambled copy of the stored pattern and settles back to it.

```settle example=memory-text
# text saved in a memory comes back letter for letter from a noisy read-address
model :mind do
  memory :m, size: 512
  m.remember :cat
  m.save :note, "meet at the harbour at nine"   # 27 bytes use 216 of the 512 things
  m.remember :dog
end

run :mind do
  m.recall read-address: :note, address-noise: 0.25, seed: 7
end
```

```text output=memory-text
recall :m from read-address :note with 25% address-noise after 30 sweeps: :note +1.00  :dog +0.03  :cat +0.02  -> :note
  text: "meet at the harbour at nine"
```

See [the memory family](05-statements/memory.md), and the sparse distributed memory families
([sdm](05-statements/sdm.md), [softsdm](05-statements/softsdm.md), [sdmscale](05-statements/sdmscale.md)) for
larger stores.

## Settle a picture

A grid declares one thing per pixel, and `lean_from` sets each pixel's lean from a grey picture. After a
`settle`, `show_as` writes each pixel's yes-rate as a grey picture.

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

```text output=grid-still
settled: 400 samples of 384 things at temperature 1
show_as :img -> grid-still-rate.pgm (rate), PSNR 35.77 dB against the leans' picture
show_as :img -> grid-still-last.pgm (last), PSNR 6.88 dB against the leans' picture
```

See [the grid family](05-statements/grid.md) for playing a folder of frames, and [colour](05-statements/colour.md)
for colour pictures.

## Learn from examples and classify

`examples` loads rows of yes/no values, `learn` fits leans and pulls so that the model reproduces them, and
`classify` holds each test row's inputs and settles to read the label.

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

```text output=learn-classify
examples :train from data/learn-same.txt, 4 in all, over 4 things
examples :test inline: 5 rows added, 5 in all, over 4 things
learned :train by exact gradients over 400 rounds of 4 rows: 4 visible, 4 hidden, 16 pulls, <time>s; exact log-likelihood per example -1.4363
classify :test by settling 200 sweeps with 2 inputs held: 3 of 4 right (75.0%); exact one-hot readout 100.0%; chance 50.0%; 1 rows skipped (not exactly one label on)
```

See [the learn family](05-statements/learn.md) and, for generating new examples, [denoise](05-statements/denoise.md).

## Solve a linear system

The numbers family has real-valued things joined by springs. Its `solve` statement turns a symmetric positive
definite system `A x = b` into springs; the average position of the shaken numbers is the solution.

```settle example=numbers-solve
# Solve the 2x2 system [[2, 1], [1, 3]] x = [1, 2] by drifting springs.
model :lin do
  number :a, :b
end

run :lin do
  solve :a, :b, matrix: "2 1; 1 3", target: "1 2", steps: 200_000, step: 0.05, seed: 1
end
```

```text output=numbers-solve
solve: 2 numbers by drifting 200000 steps (time 10000) at temperature 1
drifted: 200000 steps of 2 numbers, step 0.05, time 10000, temperature 1; averaged the last 180000 (<time> ms)
  a          settled     0.2101 ± 0.0099   exact     0.2000   off +0.0101
  b          settled     0.6000 ± 0.0081   exact     0.6000   off +0.0000
solved: largest error 0.0101, relative error 1.596%, largest error 1.0 standard errors; drift <time> ms against exact elimination <time> ms
```

See [the numbers family](05-statements/numbers.md).

## Map the calm arrangements

For a model of up to 24 free things, `valleys` enumerates every arrangement and reports each local minimum
(valley) with the fraction of arrangements that roll into it.

```settle example=valleys-ring
# a ring of 8 things, each pulling the next: the only valleys are all-yes and all-no
model :ring do
  landscape :ring, size: 8        # things r0 .. r7
end

run :ring do
  valleys                         # exact: visits all 2^8 arrangements
end
```

```text output=valleys-ring
valleys (exact, all 256 arrangements of 8 free things): 2 valleys, 2 of them in mirror pairs, calmest energy -8.000
  energy    -8.000  basin  16.41% #####                          00000000
  energy    -8.000  basin  16.41% #####                          11111111
  steepest descent stops on a flat saddle from 67.19% of arrangements
```

See [the valleys family](05-statements/valleys.md), including `survey` for larger models.

## Hand a model to another solver

`export` writes the model in a standard format that other Ising and QUBO solvers read.

```settle example=export-maxcut
# a max-cut graph: every edge pushes its two ends apart, and no thing leans
model :square do
  thing :a, :b, :c, :d
  a.pushes :b, by: 1
  b.pushes :c, by: 1
  c.pushes :d, by: 1
  d.pushes :a, by: 1
  a.pushes :c, by: 2          # a heavier diagonal
  export "out/square.gset", as: :gset
  export "out/square.dimacs", as: :dimacs
end
```

```text output=export-maxcut
exported :gset (4 things, 5 pulls) to out/square.gset
exported :dimacs (4 things, 5 pulls) to out/square.dimacs
```

See [the export family](05-statements/export.md) for the formats and for reading a model back.
