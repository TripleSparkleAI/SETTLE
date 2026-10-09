# The numbers family

The numbers family adds real-valued numbers to a model. Numbers are joined by springs, and a run settles them
by noisy drift: each number slides downhill in the springs' energy and is shaken by random kicks. The average
position of the numbers solves a linear system `A x = b`, and the spread of the positions gives the inverse
matrix `A^-1`. Every result is printed beside an exact answer from Gaussian elimination, so a reader can see
how close the drift came.

Numbers are separate from things. A number is not a thing, it has no lean or pull in the yes/no sense, and the
numbers statements never read or change the run's yes/no state (its held things, temperature, random generator
or samples). A number and a thing may share a name.

The source is `src/numbers.rs`. The measurements are in
`SETTLE/runs/smoothnumbers/REPORT_SMOOTHNUMBERS.md`.

| Statement | Block | Summary |
|---|---|---|
| [`number`](#number) | model | declare real-valued numbers |
| [`x.springs`](#xsprings) | model | a spring pulling two numbers together |
| [`x.opposes`](#xopposes) | model | a spring pulling one number toward minus another |
| [`x.leans_to`](#xleans_to) | model | a spring tying a number to a fixed value |
| [`drift`](#drift) | run | settle the model's springs by noisy drift |
| [`means`](#means) | run | the settled averages against an exact solve |
| [`spread`](#spread) | run | the covariance and the inverse matrix it gives |
| [`solve`](#solve) | run | build springs for `A x = b` from a matrix, settle them and check the answer |

## The springs, the energy and the drift

The springs of a model make a symmetric stiffness matrix `A` and a push vector `b`, one row and one entry for
each number, in the order the numbers were declared. The springs' energy is:

```text
U(x) = x.A.x / 2 - b.x
```

The energy is lowest where `A x = b`, when `A` is positive definite.

`drift` and `solve` run the overdamped Langevin update (an Ornstein-Uhlenbeck process, stepped by
Euler-Maruyama) from `x = 0`:

```text
x <- x - h * (A x - b) + sqrt(2 * T * h) * xi
```

Each step moves every number downhill by the step `h` times its force, then adds a normal random kick. `T` is
the drift's temperature and `xi` holds one standard normal draw per number per step.

After enough steps the numbers wander around the solution. Their long-run statistics are:

```text
mean = A^-1 b
covariance = T * A^-1 * (I - h*A/2)^-1
```

The average is the exact solution for any step. The covariance is the inverse matrix times the temperature,
inflated along the stiff directions by the finite step. `spread` removes that inflation.

The update is stable only when the step is below `2 / lambda_max`, where `lambda_max` is the largest stiffness
(the largest eigenvalue of `A`). At temperature 0 the update is plain gradient descent and converges to the
exact solution.

### How a drift ends

A drift runs `steps` steps. It stops early if any number becomes non-finite or larger than `10^12` in size (the
springs have blown up). The first `burn` steps are discarded. The average, the covariance and the standard
errors use the remaining steps.

- The covariance divides by the number of kept steps.
- The standard error of each average uses batch means: the kept steps are cut into 20 batches of
  `floor(kept / 20)` steps, and the standard error is the standard deviation of the batch averages divided by
  `sqrt(20)`. Steps left over after the 20th batch count toward the average but not toward the standard error.

The drift then checks the matrix with a Cholesky factorisation. The factorisation certifies that `A` is
positive definite, which means the springs have a valley. A pivot at or below `10^-10` times the largest
diagonal entry fails the certificate. The drift prints one of three lines:

- The certificate fails: the springs have no valley. The line names the number whose row failed and says what
  the drift did.
- The certificate passes but the drift blew up: the step was too large. The line gives the largest stiffness,
  estimated by 200 rounds of power iteration, and the safe bound `2 / stiffness`.
- The certificate passes and the drift stayed finite: the drift settled.

A drift that did not settle is recorded as such. `means` and `spread` then refuse to report it, so a drift
with no valley is never reported as solved.

## `number`

**Block:** model.

**Form:**

```text
number :x, :y, :z
```

**Arguments:**

| Argument | Type | Default | Meaning |
|---|---|---|---|
| `:x, :y, ...` | symbols | required, at least one | the numbers to declare |

**What it does:** declares each named number. A new number adds a zero row and a zero column to `A` and a zero
entry to `b`. A name that is already a number is left as it is, so declaring a number twice is not an error.
The springs are kept in the model's notes under the key `numbers`: the entries of `A` row by row, then `b`,
with the names as the words.

**Output:** none.

**Errors:**

- `` unexpected `<token>` in number (write: number :x, :y) `` (anything other than symbols and commas; the token
  is quoted as written, like `` `x` ``)
- `number needs at least one name, like: number :x`

## `x.springs`

**Block:** model.

**Form:**

```text
x.springs :y, by: 0.5
```

**Arguments:**

| Argument | Type | Default | Meaning |
|---|---|---|---|
| `x` | name of a number | required | one end of the spring, written before the dot |
| `:y` | symbol | required | the other end |
| `by:` | number | required | the spring's strength `k` |

**What it does:** adds a spring with energy `k/2 * (x - y)^2`, which pulls `x` and `y` toward the same value.
It adds `k` to `A[x][x]` and to `A[y][y]`, and adds `-k` to `A[x][y]` and to `A[y][x]`. Springs add up: two
springs between the same pair give one spring with the summed strength. Both numbers must be declared first.
The strength is not checked, so a negative `by:` gives a spring that pushes apart and can remove the valley.

**Output:** none.

The example under [`drift`](#drift) uses `x.springs`.

**Errors:**

- `unknown number :<name> (declare it with: number :<name>)`
- `a number cannot spring to itself; use leans_to for a spring to a fixed value`
- `` springs needs `by:` ``
- `` springs does not take `<key>:` ``
- `a number was expected` (for `by:`)
- `expected `key: value`, found <token>`

## `x.opposes`

**Block:** model.

**Form:**

```text
x.opposes :y, by: 0.25
```

**Arguments:**

| Argument | Type | Default | Meaning |
|---|---|---|---|
| `x` | name of a number | required | one end of the spring, written before the dot |
| `:y` | symbol | required | the other end |
| `by:` | number | required | the spring's strength `k` |

**What it does:** adds a spring with energy `k/2 * (x + y)^2`, which pulls `x` toward `-y`. It adds `k` to
`A[x][x]` and to `A[y][y]`, and adds `+k` to `A[x][y]` and to `A[y][x]`. Otherwise it behaves like
`x.springs`.

**Output:** none.

**Example:** at temperature 0 the drift is plain relaxation and lands on the exact answer.

```settle example=numbers-opposes
# opposes pulls one number toward minus the other. At temperature 0 the drift
# is plain relaxation and lands on the exact answer.
model :pair do
  number :u, :v
  u.opposes :v, by: 1        # energy 1/2 (u + v)^2
  u.leans_to 2.0, by: 1      # energy 1/2 (u - 2)^2
end

run :pair do
  drift 20_000, step: 0.1, temperature: 0
  means
end
```

Output:

```text output=numbers-opposes
drifted: 20000 steps of 2 numbers, step 0.1, time 2000, temperature 0; averaged the last 18000 (<time> ms)
  u          mean     2.0000 ± 0.0000   exact     2.0000   off -0.0000
  v          mean    -2.0000 ± 0.0000   exact    -2.0000   off +0.0000
means: largest error 0.0000, relative error 0.000% (exact by Gaussian elimination), largest error 8.7 standard errors
```

At temperature 0 the standard errors are close to zero, so the count of standard errors in the `means` line
compares two rounding errors and carries no meaning.

**Errors:** the same as for `x.springs`, with `opposes` in place of `springs`. The self-spring refusal also
applies: `x.opposes :x` is refused.

## `x.leans_to`

**Block:** model.

**Form:**

```text
x.leans_to 2.0, by: 1
```

**Arguments:**

| Argument | Type | Default | Meaning |
|---|---|---|---|
| `x` | name of a number | required | the number, written before the dot |
| value | number | required | the value `v` the spring ties `x` to; written as a number literal, and may be negative |
| `by:` | number | required | the spring's strength `k` |

**What it does:** adds a spring with energy `k/2 * (x - v)^2`, which ties `x` to the value `v`. It adds `k` to
`A[x][x]` and adds `k * v` to `b[x]`. This is the only statement that makes `b` non-zero.

**Output:** none.

The example under [`drift`](#drift) uses `x.leans_to`.

**Errors:**

- `unknown number :<name> (declare it with: number :<name>)`
- `` leans_to needs `by:` ``
- `` leans_to does not take `<key>:` ``
- `a number was expected` (for `by:`)

## `drift`

**Block:** run.

**Form:**

```text
drift STEPS, step: 0.01, temperature: 1, seed: 1, burn: STEPS / 10
```

**Arguments:**

| Argument | Type | Default | Meaning |
|---|---|---|---|
| `STEPS` | whole number, at least 20 | required | how many update steps to run |
| `step:` | number above 0 | `0.01` | the step `h` |
| `temperature:` | number, 0 or above | `1` | the temperature `T`; 0 means no kicks (plain relaxation) |
| `seed:` | whole number | `1` | the seed of the drift's own random generator |
| `burn:` | whole number | `STEPS / 10`, rounded down | how many early steps to discard; capped at `STEPS - 20` |

**What it does:** runs the update described in
[The springs, the energy and the drift](#the-springs-the-energy-and-the-drift) on every number of the model,
from `x = 0`, and prints how it ended. It checks first that `A` is symmetric. The springs statements always
build a symmetric matrix, so this check fails only if the notes were changed some other way.

The drift uses its own random generator, seeded by `seed:` (1 when absent). It does not use or advance the
run's random generator, and the run's `temperature` does not apply: `temperature:` here is the drift's own.
Two drifts with the same arguments give the same result.

The result is kept in the model's notes under the key `numbers:drift`: the number of steps, the step, the
temperature, whether the drift settled, `A`, `b`, the averages, their standard errors and the covariance. The
notes belong to the model, so a later run block of the same model can still report them with `means` or
`spread`. Each `drift` or `solve` replaces them.

**Output:** one of these lines:

```text
drifted: <steps> steps of <n> numbers, step <h>, time <steps*h>, temperature <T>; averaged the last <kept> (<time> ms)
did not settle: the springs have no valley (not positive definite; the stiffness fails at :<name>); the drift blew up at step <s>
did not settle: the springs have no valley (not positive definite; the stiffness fails at :<name>); the drift ended with its largest number at <x>
did not settle: step <h> is too large for the stiffest spring (stiffness <lambda>); the drift blew up at step <s>; keep step below <2/lambda>
```

The step, the time and the temperature are printed in the shortest form that reads back as the same number,
so `1.0` prints as `1`. `<x>` is printed in scientific form with three decimals, like `1.000e3`. The stiffness
and the bound have four decimals.

**Example:** two numbers with three springs, drifted, then reported with `means` and `spread`.

```settle example=numbers-drift
# Two numbers joined by springs. Their energy is lowest where A x = b with
# A = [[3, -1], [-1, 2]] and b = [2, -1].
model :bowl do
  number :x, :y
  x.springs :y, by: 1        # energy 1/2 (x - y)^2
  x.leans_to 1.0, by: 2      # energy 2/2 (x - 1)^2
  y.leans_to -1.0, by: 1     # energy 1/2 (y + 1)^2
end

run :bowl do
  drift 200_000, step: 0.05, seed: 1  # noisy settling from x = y = 0
  means                               # the averages against an exact solve
  spread                              # the covariance and the inverse matrix it gives
end
```

Output:

```text output=numbers-drift
drifted: 200000 steps of 2 numbers, step 0.05, time 10000, temperature 1; averaged the last 180000 (<time> ms)
  x          mean     0.6101 ± 0.0049   exact     0.6000   off +0.0101
  y          mean    -0.1900 ± 0.0090   exact    -0.2000   off +0.0100
means: largest error 0.0101, relative error 2.245% (exact by Gaussian elimination), largest error 2.1 standard errors
  covariance of the settled numbers:
  x            0.4280   0.1983
  y            0.1983   0.6325
  inverse from the spread (covariance / temperature, step-corrected)  |  exact inverse:
  x            0.4008   0.1991  |    0.4000   0.2000
  y            0.1992   0.6058  |    0.2000   0.6000
spread: inverse from the spread against the exact inverse, relative error 0.77% step-corrected, 5.54% raw
```

**Errors:**

- `drift needs numbers; declare them in the model with: number :x`
- `drift needs at least 20 steps`
- `step must be above zero`
- `temperature cannot be below zero (zero means no shaking: plain relaxation)`
- `the springs between :<a> and :<b> are not symmetric`
- `` drift does not take `<key>:` ``
- `a number was expected`

`drift` reads the step count only from its position. It does not accept `steps:`.

## `means`

**Block:** run.

**Form:**

```text
means
```

**Arguments:** none.

**What it does:** reads the most recent `drift` or `solve` of the model from its notes and prints each average
beside the exact solution of `A x = b`, which it computes by Gaussian elimination with partial pivoting. It
changes nothing.

For each number, `off` is the average minus the exact value. The summary line reports:

- the largest error: the largest `|off|` over all numbers;
- the relative error: `|mean - exact| / |exact|`, with both vectors measured by their Euclidean length, as a
  percentage;
- the largest error in standard errors: the largest `|off| / standard error` over all numbers.

**Output:** one line per number when there are at most 16 numbers, then the summary line:

```text
  <name> mean <average> ± <standard error>   exact <exact value>   off <average - exact>
means: largest error <e>, relative error <r>% (exact by Gaussian elimination), largest error <z> standard errors
```

The name is padded to 10 characters. The average and the exact value have four decimals and are right-aligned
in 10 characters; the standard error has four decimals; `off` has four decimals and always a sign. The
relative error has three decimals and `<z>` has one.

The example under [`drift`](#drift) uses `means`.

**Errors:**

- `means needs a drift or a solve first`
- `means: the last drift did not settle, so there is nothing to report`

## `spread`

**Block:** run.

**Form:**

```text
spread
```

**Arguments:** none.

**What it does:** reads the most recent `drift` or `solve` of the model and estimates the inverse matrix from
the covariance `C` of the kept steps. With step `h` and temperature `T`:

```text
inverse from the spread = (C / T) * (I - h*A/2)
```

The raw covariance divided by the temperature is `A^-1 (I - h*A/2)^-1`; multiplying by `(I - h*A/2)` removes
the step's bias. `spread` compares both the corrected estimate and the raw `C / T` with the exact inverse,
computed column by column by Gaussian elimination. The error is the relative Frobenius distance
`|estimate - exact| / |exact|`, where `|M|` is the square root of the sum of the squared entries. It changes
nothing.

**Output:** when there are at most 6 numbers, the covariance and the two inverses, one row per number:

```text
  covariance of the settled numbers:
  <name> <c1> <c2> ...
  inverse from the spread (covariance / temperature, step-corrected)  |  exact inverse:
  <name> <e1> <e2> ...  | <x1> <x2> ...
```

Every entry has four decimals and is right-aligned in 9 characters; the name is padded to 10 characters. Then,
for any number of numbers, the summary line:

```text
spread: inverse from the spread against the exact inverse, relative error <p>% step-corrected, <q>% raw
```

The example under [`drift`](#drift) uses `spread`.

**Errors:**

- `spread needs a drift or a solve first`
- `spread: the last drift did not settle, so there is nothing to report`
- `spread needs a drift above temperature zero; at zero the numbers do not shake`

## `solve`

**Block:** run.

**Form:**

```text
solve :x, :y, matrix: "2 1; 1 3", target: "1 2", steps: 100_000, step: 0.01, temperature: 1, seed: 1, burn: steps / 10
```

**Arguments:**

| Argument | Type | Default | Meaning |
|---|---|---|---|
| `:x, :y, ...` | symbols | required, at least one | the `d` numbers to solve for, in the order of the matrix rows |
| `matrix:` | string | required | the `d` by `d` matrix `A` |
| `target:` | string | required | the `d` entries of `b` |
| `steps:` | whole number, at least 20 | `100_000` | how many update steps to run |
| `step:` | number above 0 | `0.01` | the step `h` |
| `temperature:` | number, 0 or above | `1` | the temperature `T` |
| `seed:` | whole number | `1` | the seed of the drift's own random generator |
| `burn:` | whole number | `steps / 10`, rounded down | how many early steps to discard; capped at `steps - 20` |

The symbols come first, then the keyword arguments. The matrix string holds rows separated by `;` or by a
newline, and each row holds numbers separated by spaces or commas. `_` inside a number is ignored. Empty rows
are skipped. The target string is read the same way and all its rows are joined, so `"1 2"` and `"1; 2"` are
the same target.

**What it does:**

1. Checks the sizes and refuses a matrix that is not symmetric. Springs pull both ways equally, so only a
   symmetric matrix is a set of springs. Two entries count as equal when they differ by at most `10^-12` times
   one plus the larger of their sizes.
2. Declares any of the named numbers that are not declared yet.
3. Cuts every spring between a named number and a number outside the list: the off-diagonal entries of `A`
   between them are set to zero. The other numbers keep their own diagonal entries and pushes, so a later
   `drift` of the whole model still feels the stiffness those springs added to them.
4. Writes the matrix into `A` among the named numbers and the target into `b` for them. These springs stay in
   the model, so a later `drift` uses them.
5. Drifts the named numbers alone (the `d` by `d` system) with the given options and prints how the drift
   ended, as `drift` does. The result is kept in the notes as the most recent drift, so `means` and `spread`
   afterwards report the solved system.
6. If the drift settled, prints each settled average beside the exact solution, and a summary line. The
   summary adds two timings: the drift run again with the same options and no covariance, and the mean time
   of 200 Gaussian eliminations of the same system.

A general, non-symmetric `A` can be solved through the normal equations `A^T A x = A^T b`, whose matrix is
symmetric.

**Output:**

```text
solve: cut <k> springs between the solved numbers and the others
solve: <d> numbers by drifting <steps> steps (time <steps*h>) at temperature <T>
<one of the drift lines>
  <name> settled <average> ± <standard error>   exact <exact value>   off <average - exact>
solved: largest error <e>, relative error <r>%, largest error <z> standard errors; drift <time> ms against exact elimination <time> ms
```

The first line appears only when springs were cut. The per-number lines appear only when the drift settled and
there are at most 16 numbers. The `solved:` line appears only when the drift settled. The fields are formatted
as in [`means`](#means), and the two timings have two and four decimals.

**Examples:** a 2 by 2 system solved by drifting.

```settle example=numbers-solve
# Solve the 2x2 system [[2, 1], [1, 3]] x = [1, 2] by drifting springs.
model :lin do
  number :a, :b
end

run :lin do
  solve :a, :b, matrix: "2 1; 1 3", target: "1 2", steps: 200_000, step: 0.05, seed: 1
end
```

Output:

```text output=numbers-solve
solve: 2 numbers by drifting 200000 steps (time 10000) at temperature 1
drifted: 200000 steps of 2 numbers, step 0.05, time 10000, temperature 1; averaged the last 180000 (<time> ms)
  a          settled     0.2101 ± 0.0099   exact     0.2000   off +0.0101
  b          settled     0.6000 ± 0.0081   exact     0.6000   off +0.0000
solved: largest error 0.0101, relative error 1.596%, largest error 1.0 standard errors; drift <time> ms against exact elimination <time> ms
```

Solving numbers that already have springs to another number cuts those springs, and `spread` afterwards
reports the solved pair:

```settle example=numbers-cut
# solve on numbers that already have springs to another number. The springs
# between the solved numbers and the others are cut, and means and spread then
# report the solved system.
model :mix do
  number :x, :y, :z
  x.springs :z, by: 1        # a spring from x to z
  y.opposes :z, by: 0.5      # and one from y to z
  z.leans_to 3.0, by: 1
end

run :mix do
  solve :x, :y, matrix: "2 1; 1 3", target: "1 2", steps: 50_000, step: 0.05, burn: 1_000, seed: 3
  spread                     # the covariance of the solved pair and the inverse it gives
end
```

Output:

```text output=numbers-cut
solve: cut 2 springs between the solved numbers and the others
solve: 2 numbers by drifting 50000 steps (time 2500) at temperature 1
drifted: 50000 steps of 2 numbers, step 0.05, time 2500, temperature 1; averaged the last 49000 (<time> ms)
  x          settled     0.1584 ± 0.0196   exact     0.2000   off -0.0416
  y          settled     0.6109 ± 0.0140   exact     0.6000   off +0.0109
solved: largest error 0.0416, relative error 6.802%, largest error 2.1 standard errors; drift <time> ms against exact elimination <time> ms
  covariance of the settled numbers:
  x            0.6163  -0.1881
  y           -0.1881   0.4176
  inverse from the spread (covariance / temperature, step-corrected)  |  exact inverse:
  x            0.5902  -0.1894  |    0.6000  -0.2000
  y           -0.1892   0.3910  |   -0.2000   0.4000
spread: inverse from the spread against the exact inverse, relative error 2.60% step-corrected, 3.79% raw
```

A symmetric matrix that is not positive definite has no valley. The drift still runs, and the result is never
reported as solved:

```settle example=numbers-no-valley
# A symmetric matrix that is not positive definite has no valley. The drift runs,
# and the result is reported as not settled.
model :lin do
  number :a, :b
end

run :lin do
  solve :a, :b, matrix: "1 2; 2 1", target: "1 1", steps: 2_000, step: 0.05
end
```

Output:

```text output=numbers-no-valley
solve: 2 numbers by drifting 2000 steps (time 100) at temperature 1
did not settle: the springs have no valley (not positive definite; the stiffness fails at :b); the drift blew up at step 565
```

A step too large for the stiffest spring blows the drift up, and the line names a safe step:

```settle example=numbers-step
# A step too large for the stiffest spring makes the drift overshoot and blow up.
# solve names the largest safe step.
model :stiff do
  number :p, :q
end

run :stiff do
  solve :p, :q, matrix: "40 1; 1 30", target: "1 1", step: 0.1, steps: 1_000
end
```

Output:

```text output=numbers-step
solve: 2 numbers by drifting 1000 steps (time 100) at temperature 1
did not settle: step 0.1 is too large for the stiffest spring (stiffness 40.0990); the drift blew up at step 28; keep step below 0.0499
```

A matrix that is not symmetric is refused before any drift:

```settle example=numbers-refused
# A matrix that is not symmetric is not a set of springs, so solve refuses it.
model :lin do
  number :a, :b
end

run :lin do
  solve :a, :b, matrix: "2 1; 0 3", target: "1 2"
end
```

```text error=numbers-refused
line 7: refused: the matrix is not symmetric (row 1 column 2 is 1, row 2 column 1 is 0). Springs pull both ways equally, so only a symmetric matrix is a set of springs; for a general A, solve A^T A x = A^T b instead
```

**Errors:**

- `solve needs the numbers to solve for, like: solve :x, :y, matrix: "2 1; 1 3", target: "1 2"`
- `` unexpected `<token>` in solve `` (anything other than symbols and commas before the first keyword)
- `` solve needs `matrix:` ``
- `` solve needs `target:` ``
- `matrix must be <d> by <d> for <d> numbers`
- `target must have <d> entries`
- `'<text>' is not a number` (a cell of the matrix or target)
- `refused: the matrix is not symmetric (row <i> column <j> is <a>, row <j> column <i> is <b>). Springs pull both ways equally, so only a symmetric matrix is a set of springs; for a general A, solve A^T A x = A^T b instead`
- `drift needs at least 20 steps` (the refusal names `drift` although the statement is `solve`)
- `step must be above zero`
- `temperature cannot be below zero (zero means no shaking: plain relaxation)`
- `` solve does not take `<key>:` ``
- `a number was expected`
- `a "quoted" string was expected` (for `matrix:` or `target:`)

## Notes

- **Accuracy.** For a random target the error of the average falls as `1 / sqrt(time)` once the burn-in is
  over, and sits in the soft directions of `A`. The softest spring needs about `condition number` units of time
  to arrive, so an ill-conditioned system needs a longer `burn:`. The measurements put the solve error at 0.4
  to 0.8% at time 100,000 for sizes 4 to 64 and condition numbers 10 and 1000; plain relaxation at temperature
  0 is about 10^8 times more accurate for the solve alone, and the noise is what makes `spread` possible. See
  `SETTLE/runs/smoothnumbers/REPORT_SMOOTHNUMBERS.md`.
- **Cost.** One step costs `d^2` multiply-adds, and the covariance costs about `d^2 / 2` more per kept step.
- **Temperature 0.** `means` still works, but the standard errors are close to zero, so the count of standard
  errors compares rounding noise. `spread` refuses a drift at temperature 0.
