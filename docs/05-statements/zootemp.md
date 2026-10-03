# The zootemp family

The zootemp family adds two statements to the puzzles of the [zoo](zoo.md) family. `anneal_schedule` is the
core [`anneal`](core.md#anneal) with its schedule made adjustable: the hot and cold ends of the cooling become
arguments, and the sweeps can be split into several independent walks (restarts). `x.final` judges a puzzle on
the arrangement the last walk came to rest in, where [`x.solution`](zoo.md#xsolution) judges it on the calmest
arrangement the walk visited. With both statements a program can say which of the two it measured.

The source is `src/zootemp.rs`. The family was measured in `experiments/thermosim/runs/zootemp/REPORT_ZOOTEMP.md`.
There, on the column encoding for factoring, a long anneal often visited the answer and then cooled away from
it. At an equal total number of sweeps, many short walks judged by their end states landed on the answer more
often than one long walk (19 of 20 seeds against 3 of 20 at 10,403).

| Statement | Block | Summary |
|---|---|---|
| [`anneal_schedule`](#anneal_schedule) | run | anneal with chosen hot and cold ends, in one or several walks |
| [`x.final`](#xfinal) | run | decode a puzzle from the run's last arrangement and check it |

## `anneal_schedule`

**Block:** run.

**Form:**

```text
anneal_schedule S, temperature: 1, hot: 10, cold: 0.05, restarts: 1, seed: 24301
```

A comma after `S` is allowed: `anneal_schedule 1_000, seed: 1` and `anneal_schedule 1_000 seed: 1` are the same.

**Arguments:**

| Argument | Type | Default | Meaning |
|---|---|---|---|
| `S` | whole number | required | The total number of sweeps over all walks. A fractional value is cut to a whole number. |
| `temperature:` | number above zero | the run's current temperature (1 at the start of a run) | Sets the run's base temperature `T`. It stays set for later statements in the run. |
| `hot:` | number above zero | 10 | The schedule starts at `hot x T`. |
| `cold:` | number above zero | 0.05 | The schedule ends at `cold x T`. |
| `restarts:` | whole number from 1 to `S` | 1 | `R`: the number of independent walks the sweeps are split into. |
| `seed:` | whole number | the run's current generator | Replaces the run's random generator before the first walk. |

**What it does:** runs `R` walks one after another, each of `L = S / R` sweeps (rounded down, at least 1). Each
walk starts from a fresh random arrangement, with held things held, and cools geometrically. Sweep `k` of a
walk (counting from 0) runs at

```text
T_k = T * hot * (cold / hot)^(k / (L - 1))
```

The temperature starts at `hot` times the base temperature and falls by the same factor every sweep until it
reaches `cold` times the base temperature at the last sweep. For a walk of one sweep, `L - 1` is taken as 1.

A sweep updates every free thing once in a random order, with the same rule as [`settle`](core.md#settle). With
`hot: 10`, `cold: 0.05` and `restarts: 1` the statement is the core `anneal`: it draws the same random numbers,
visits the same arrangements and keeps the same best, to the last bit. The source tests this on sudoku, graph
colouring and both factoring encodings.

After the walks, the run keeps two arrangements:

- **best:** the calmest arrangement any walk visited, including each walk's starting arrangement. `best` and
  `x.solution` read it. Each candidate is re-measured exactly before it is compared, so the choice does not
  depend on the rounding of the walk's running energy.
- **last:** the calmest of the `R` end states; on a tie, the first walk's. With one walk this is the walk's end
  state. `x.final` reads it.

The walks share the run's random stream: walk 2 continues where walk 1 stopped. The statement does not record
samples, so `show` and `ask` are not affected. It does not change the model. An earlier `anneal_each` leaves
per-puzzle blocks in the notes, stamped with that anneal's best; `anneal_schedule` sets a new best, so
`x.solution` no longer uses those blocks.

The temperature rule of the zoo's measurements is a tenth of the largest lean or pull in the model. The statement
does not apply it; give `temperature:` yourself.

**Output:**

```text
annealed on a schedule: <S> sweeps in <R> walk(s), temperature <start> to <end>; calmest visited <B>, calmest end state <F>
```

`<start>` is `hot x T` and `<end>` is `cold x T`, printed in the shortest form that reads back as the same
number (for example `10`, `0.05`, `0.0325`). `<B>` is the energy of the best arrangement and `<F>` the energy of
the last one, each with three decimals. `<S>` is the number given, even when `R` does not divide it (see Notes).

**Example:** the default schedule reproduces `anneal`, and a custom schedule in four walks finds the same best.

```settle example=zootemp-schedule
# Five things in a ring that all push their neighbours. Each run block starts a fresh run.
model :ring do
  thing :a, :b, :c, :d, :e
  a.pushes :b, by: 1
  b.pushes :c, by: 1
  c.pushes :d, by: 1
  d.pushes :e, by: 1
  e.pushes :a, by: 1
end

run :ring do
  anneal 400, seed: 5             # the core anneal
  best
end

run :ring do
  anneal_schedule 400, seed: 5    # the default schedule: the same walk, draw for draw
  best
end

run :ring do
  # from 4 x 0.5 = 2 down to 0.1 x 0.5 = 0.05, in 4 walks of 100 sweeps
  anneal_schedule 400, temperature: 0.5, hot: 4, cold: 0.1, restarts: 4, seed: 5
  best
end
```

Output:

```text output=zootemp-schedule
annealed: 400 sweeps, calmest energy found -3.000
best (energy -3.000): a yes, b yes, c no, d yes, e no
annealed on a schedule: 400 sweeps in 1 walk(s), temperature 10 to 0.05; calmest visited -3.000, calmest end state -3.000
best (energy -3.000): a yes, b yes, c no, d yes, e no
annealed on a schedule: 400 sweeps in 4 walk(s), temperature 2 to 0.05; calmest visited -3.000, calmest end state -3.000
best (energy -3.000): a yes, b yes, c no, d yes, e no
```

The example for [`x.final`](#xfinal) below shows restarts on a factoring puzzle.

**Errors:**

- `` anneal_schedule does not take `<key>:` ``
- `a number was expected`
- `temperature must be above zero`
- `hot and cold must be above zero`
- `restarts is a whole number from 1 to the number of sweeps`

## `x.final`

**Block:** run.

**Form:**

```text
name.final
```

**Arguments:** none. `name` is a puzzle declared with [`sudoku`](zoo.md#sudoku), [`colouring`](zoo.md#colouring),
[`maxcut`](zoo.md#maxcut) or [`factor`](zoo.md#factor). A line `x.final` where `x` is not a declared puzzle is
not claimed by this family.

**What it does:** takes the run's last arrangement, decodes the puzzle's things from it and checks the result
with the same plain-code rules as [`x.solution`](zoo.md#xsolution). The energy is never used for the verdict.

The last arrangement is the one the most recent statement that walks the model left behind. For the statements
used with puzzles:

- after `anneal_schedule`, the calmest end state of its walks;
- after the core `anneal` or `anneal_each`, the arrangement at the final sweep;
- after `settle`, the arrangement at the final recorded sweep.

It does not use the per-puzzle blocks that `anneal_each` stores. It changes nothing in the run.

**Output:** first the line

```text
  (:<name> judged on the end state, not on the best visited)
```

then exactly the lines `x.solution` prints for the same puzzle and arrangement: the grid or the colour list, and
the verdict line. See [`x.solution`](zoo.md#xsolution) for every form.

**Example:** one walk of 20,000 sweeps visits the factorisation of 899 and then cools away from it. Twenty walks
of 1,000 sweeps each spend the same budget, and the calmest of their end states is the answer.

```settle example=zootemp-restarts
# Factor 899 = 29 x 31 with the column encoding, on one budget of 20,000 sweeps, two ways.
model :p do
  factor :f, number: 899, encoding: :columns
end

run :p do
  anneal_schedule 20_000, temperature: 0.65, seed: 5                # one long walk
  f.solution   # judged on the calmest arrangement the walk visited
  f.final      # judged on where the walk came to rest
  anneal_schedule 20_000, temperature: 0.65, restarts: 20, seed: 5  # 20 walks of 1,000 sweeps
  f.solution   # the calmest arrangement any of the 20 walks visited
  f.final      # the calmest of the 20 end states
end
```

Output:

```text output=zootemp-restarts
annealed on a schedule: 20000 sweeps in 1 walk(s), temperature 6.5 to 0.0325; calmest visited -70.500, calmest end state -69.500
factor :f: 899 = 29 x 31: VALID (checked by multiplying)
  (:f judged on the end state, not on the best visited)
factor :f: NOT VALID for 899: 21 x 55 = 1155, not 899
annealed on a schedule: 20000 sweeps in 20 walk(s), temperature 6.5 to 0.0325; calmest visited -70.500, calmest end state -70.500
factor :f: 899 = 29 x 31: VALID (checked by multiplying)
  (:f judged on the end state, not on the best visited)
factor :f: 899 = 29 x 31: VALID (checked by multiplying)
```

This is one seed. The report measures the rates over many seeds.

This example shows the error a program meets when it asks for the end state before anything has run:

```settle example=zootemp-final-first
# x.final reads the run's last arrangement, so something must have run first.
model :p do
  factor :f, number: 15
end

run :p do
  f.final   # no anneal or settle has run yet
end
```

```text error=zootemp-final-first
line 7: final needs an anneal first
```

**Errors:**

- `final needs an anneal first` (the run has no last arrangement yet; a `settle` also provides one)

## Notes

- The printed sweep count is `S` as given, but the walks run `R x floor(S / R)` sweeps in total. For example,
  `anneal_schedule 10, restarts: 3` runs three walks of 3 sweeps and prints `10 sweeps`.
- `anneal_schedule 0` runs one sweep and prints `0 sweeps`.
- A puzzle's best-so-far and its end state agree on easy puzzles. They differ most where the energy has many
  valleys close to the lowest one: near-threshold colourings, 9x9 sudoku with few givens, dense max-cut graphs and
  both factoring encodings (`runs/zootemp/REPORT_ZOOTEMP.md`, section 6).
