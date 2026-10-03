# The descend family

The descend family settles continuous **parameters** on a **loss**. The loss is the energy, the temperature is
the noise, and the settling step is a gradient step with a random kick. At temperature 0 that step is plain
gradient descent (or stochastic gradient descent, with minibatches). Above temperature 0 it is Langevin dynamics,
and the cloud of positions the walkers visit is the Bayesian posterior over the parameters. Cooling the
temperature during a run is the bridge between the two: the walkers explore the hill first, then descend it.

It is the [numbers family](numbers.md)'s springs generalised from one fixed quadratic energy to any
differentiable loss. On the numbers' own springs, `descend` is `drift`, number for number.

The source is `src/descend.rs`. The measurements are in
`experiments/thermosim/runs/gradsettle/REPORT_GRADSETTLE.md`.

| Statement | Block | Summary |
|---|---|---|
| [`data`](#data) | model | the examples a loss is fitted to: a CSV file, or MNIST |
| [`test`](#test) | model | held-out examples that `score` predicts |
| [`loss`](#loss) | model | the loss piece: springs, least squares, logistic, or a one-hidden-layer net |
| [`descend`](#descend) | run | settle the parameters: Langevin above temperature 0, gradient descent at 0, or Adam |
| [`ask`](#ask) | run | the parameters' means and spreads, beside the exact posterior when there is one |
| [`score`](#score) | run | accuracy, log-likelihood, calibration and doubt on the test examples |

## The energy, the step and the cloud

A loss piece and its data make an energy over the parameters `th`:

```text
U(th) = sum over examples of loss_i(th) + |th|^2 / (2 * prior^2)
```

The first term is how badly the parameters fit the examples. The second is a Gaussian prior with standard
deviation `prior` on every parameter (`prior: 0` drops it). The posterior over the parameters is proportional to
`exp(-U)`: the lower the energy, the more likely the parameters.

`descend` runs this step, from `th = 0` (a net starts at `init * N(0, 1)`, because 0 is a saddle of a net):

```text
th <- th - h * grad U + sqrt(2 * T * h) * xi
```

`h` is the step, `T` the temperature, and `xi` holds one standard normal draw per parameter per step. Every
parameter moves downhill by the step times its gradient, then takes a random kick. With `batch: B` of `N`
examples, the gradient of the loss term is estimated from `B` examples and multiplied by `N / B` (stochastic
gradient Langevin dynamics, Welling and Teh 2011).

- **T = 0** is gradient descent. With every example in each step it converges to the bottom of the valley.
- **T = 1** samples the posterior `exp(-U)`. The mean of the cloud is the posterior mean; its spread is the
  posterior's uncertainty.
- **T > 1** samples a flattened posterior `exp(-U / T)`, wider than the evidence supports. Below 1 it is
  sharpened (a "cold" posterior).
- **`cool_to:`** lowers the temperature along a straight line during the run: explore, then descend.

For a quadratic energy (springs, least squares) the long-run statistics of the step are exact:

```text
mean = A^-1 c              (the posterior mean, for any step)
covariance = T * A^-1 * (I - h*A/2)^-1
```

`ask` removes the step's inflation `(I - h*A/2)^-1`, as the numbers family's `spread` does, and compares the
cloud with the exact posterior `A^-1`.

A cloud predicts by averaging the predictions of its snapshots, not by averaging its parameters. Where the
snapshots disagree, the cloud has **doubt**: the mutual information between the prediction and the parameters,

```text
doubt = H[mean of the snapshots' predictions] - mean over snapshots of H[prediction]
```

in nats, where `H` is entropy. One parameter vector has no doubt about itself: its doubt is exactly 0.

## `data`

**Block:** model.

**Form:**

```text
data "points.csv"
data :mnist, dir: "data", rows: 1_000, from: 0, split: "train"
```

**What it does:** reads the examples the loss is fitted to. A CSV file holds one example per row, numbers
separated by spaces or commas, the target in the last column. Empty lines and lines starting with `#` are
skipped, and a first line that is not all numbers is a header. `:mnist` reads the MNIST IDX files from `dir:`
(the four unzipped files; see `experiments/thermosim/runs/mnist/PROVENANCE.md`): 784 features, each grey value
divided by 255, and the digit as the target. `split:` defaults to `"train"`.

**Output:** `data: <n> examples of <p> features`.

**Errors:** `cannot read <path>: ...` · `row <k> has <a> columns, the first row has <b>` · ``data :mnist needs `dir:` `` · `data has <p> features but the test rows have <q>`.

## `test`

**Block:** model.

**Form:** as `data`; `split:` defaults to `"test"` for `:mnist`.

**What it does:** reads held-out examples. `score` predicts them; without `test`, `score` uses the training data.

**Output:** `test: <n> test examples of <p> features`.

## `loss`

**Block:** model.

**Form:**

```text
loss :springs
loss :least_squares, noise: 0.5, prior: 10
loss :logistic, classes: 10, prior: 10
loss :net, hidden: 8, classes: 2, prior: 3, init: 0.5
loss :net, hidden: 8, noise: 0.5
```

| Piece | Parameters | Loss of one example |
|---|---|---|
| `:springs` | the model's numbers | none; the energy is the springs' `x.A.x/2 - b.x` (no prior) |
| `:least_squares` | `w1 .. wp`, `bias` | `(y - w.x - bias)^2 / (2 noise^2)` |
| `:logistic`, 2 classes | `w1 .. wp`, `bias` | `log(1 + e^z) - y z`, `z = w.x + bias`, `y` 0 or 1 |
| `:logistic`, k classes | `w<c>_<i>`, `bias<c>` | `log sum_c e^(z_c) - z_y` (softmax) |
| `:net` | `h<j>_<i>`, `hbias<j>`, `o<c>_<j>`, `obias<c>` | the same outputs on `tanh` hidden things |

Features are numbered from 1, classes from 0 and hidden things from 1. `:logistic` takes its classes from the
targets (whole numbers from 0) unless `classes:` says. `:net` predicts classes when `classes:` is given and a
value with Gaussian noise of sd `noise:` otherwise. `prior:` defaults to 10, `noise:` to 1, `init:` to 0.5.

**Errors:** `no loss piece :<name>` · ``loss :net needs `hidden:` `` · `classes must be at least 2` · `noise must
be above zero` · `loss :springs needs numbers and springs`.

## `descend`

**Block:** run.

**Form:**

```text
descend 20_000, step: 0.01, temperature: 1, cool_to: 0, step_to: 0.001, batch: 100, walkers: 4,
        seed: 1, burn: 2_000, every: 1, keep: 50, method: :langevin
```

| Argument | Default | Meaning |
|---|---|---|
| steps (first) | required, at least 20 | how many steps each walker takes |
| `step:` | 0.01 | the step `h` (for Adam, its rate) |
| `step_to:` | none | the step falls geometrically to this over the run |
| `temperature:` | 0 | `T`; 0 is gradient descent |
| `cool_to:` | none | the temperature falls along a straight line to this over the run |
| `batch:` | every example | examples per gradient |
| `walkers:` | 1 | independent walkers, each with its own random stream (and its own start, for a net) |
| `seed:` | 1 | the random stream |
| `burn:` | steps / 10 | steps discarded before the cloud is gathered |
| `every:` | 1 | gather every `every`-th position after the burn |
| `keep:` | 50 (0 for springs and least squares) | snapshots kept per walker, for `score` |
| `method:` | `:langevin` | `:adam` runs Adam (0.9, 0.999, 1e-8) at temperature 0 |

**What it does:** runs every walker, keeps running sums of the positions after the burn (the mean, the variance,
the covariance when there are at most 64 parameters, and standard errors from 20 batch means), and keeps
snapshots evenly spaced through the gathered positions. A walker that leaves the finite range (or passes
`10^12`) stops the run, and `ask` and `score` then refuse to report it.

**Output:** one line naming the steps, parameters, piece, method, step, temperature and batch, then the loss at
the end (per example) and the energy.

**Errors:** `descend needs a loss` · `descend needs data for this loss` · `step must be above zero` ·
`temperature cannot be below zero` · `refused: Adam has no temperature` · `did not settle: walker <w> blew up at
step <s> (step <h> is too large)`, with the safe bound `2 / stiffness` when the energy is quadratic.

**Example:** springs. At temperature 0 `descend` is gradient descent and lands on the exact answer; at
temperature 1 it is the walk `drift` runs (`tests`: the means and covariance are equal bit for bit).

```settle example=descend-springs
# Two numbers on springs, settled by descend instead of drift. At temperature 0 descend is gradient descent and
# lands on the exact answer; at temperature 1 it is the same noisy walk drift runs, number for number.
model :bowl do
  number :x, :y
  x.springs :y, by: 0.5
  x.leans_to 2.0, by: 1
  y.leans_to -1.0, by: 0.5
  loss :springs
end
run :bowl do
  descend 2_000, step: 0.1
  ask
  descend 100_000, step: 0.02, temperature: 1, seed: 5
  ask
end
```

Output:

```text output=descend-springs
descended: 2000 steps of 2 parameters (springs), gradient descent, step 0.1, temperature 0, every row; kept 1800 (<time> ms)
  energy at the end -1.3500
  x          mean     1.4000 ± 0.0000   spread 0.0000   posterior mean     1.4000  spread 0.8944
  y          mean     0.2000 ± 0.0000   spread 0.0000   posterior mean     0.2000  spread 1.0954
ask: 2 parameters; largest error 3.48e-9 against the exact answer; temperature 0, so the cloud has no spread
descended: 100000 steps of 2 parameters (springs), Langevin, step 0.02, temperature 1, every row; kept 90000 (<time> ms)
  energy at the end -1.1006
  x          mean     1.4207 ± 0.0270   spread 0.8825   posterior mean     1.4000  spread 0.8944
  y          mean     0.2267 ± 0.0425   spread 1.0829   posterior mean     0.2000  spread 1.0954
ask: 2 parameters; largest mean error 0.0267 (0.8 standard errors) against the exact posterior mean; spreads are step-corrected, and the cloud's covariance is 2.72% off the posterior's (temperature 1)
```

**Example:** gradient descent on a line, to the exact least-squares answer.

```settle example=descend-gd
# Temperature 0: plain gradient descent. With every row in each step it reaches the exact least-squares answer.
model :line do
  data "data/descend-line.csv"
  loss :least_squares, noise: 0.5, prior: 10
end
run :line do
  descend 3_000, step: 0.004
  ask :w1, :bias
end
```

Output:

```text output=descend-gd
data: 30 examples of 1 feature
descended: 3000 steps of 2 parameters (least squares), gradient descent, step 0.004, temperature 0, every row; kept 2700 (<time> ms)
  loss at the end 0.5579 per example, energy 16.7504
  w1         mean     1.5042 ± 0.0000   spread 0.0000   posterior mean     1.5042  spread 0.0795
  bias       mean     0.4118 ± 0.0000   spread 0.0000   posterior mean     0.4118  spread 0.0950
ask: 2 parameters; largest error 4.15e-14 against the exact answer; temperature 0, so the cloud has no spread
```

## `ask`

**Block:** run.

**Form:** `ask` (the first 16 parameters) or `ask :w1, :bias`.

**What it does:** prints each parameter's mean with its standard error and its spread (standard deviation over
the cloud). When the energy is quadratic it prints the exact posterior mean and spread beside them, and the
largest error. When the run had one temperature and every example in each step, spreads are step-corrected and
the cloud's whole covariance is compared with the posterior's.

**Example:** the line at temperature 1. The cloud is the Bayesian posterior over `w1` and `bias`.

```settle example=descend-line
# Fit a line y = w1 x + bias to 30 noisy points. The loss is the energy, so at temperature 1 the cloud of
# positions is the Bayesian posterior over (w1, bias); ask prints it beside the exact posterior.
model :line do
  data "data/descend-line.csv"
  test "data/descend-line-test.csv"
  loss :least_squares, noise: 0.5, prior: 10
end
run :line do
  descend 200_000, step: 0.004, temperature: 1, seed: 1, keep: 200
  ask
  score
end
```

Output:

```text output=descend-line
data: 30 examples of 1 feature
test: 200 test examples of 1 feature
descended: 200000 steps of 2 parameters (least squares), Langevin, step 0.004, temperature 1, every row; kept 180000 (<time> ms)
  loss at the end 0.6344 per example, energy 19.0459
  w1         mean     1.5048 ± 0.0003   spread 0.0794   posterior mean     1.5042  spread 0.0795
  bias       mean     0.4123 ± 0.0005   spread 0.0952   posterior mean     0.4118  spread 0.0950
ask: 2 parameters; largest mean error 0.0005 (2.0 standard errors) against the exact posterior mean; spreads are step-corrected, and the cloud's covariance is 0.49% off the posterior's (temperature 1)
score on 200 test rows:
  last   rmse 0.5192   nll 0.7650   inside the 95% interval 95.0%
  mean   rmse 0.4978   nll 0.7214   inside the 95% interval 94.5%
  cloud  rmse 0.4987   nll 0.7306   inside the 95% interval 96.0%
score: last = the final position, mean = the averaged parameters, the cloud averages 200 snapshots' predictions; doubt is what the snapshots disagree on (nats)
```

**Example (the negative control):** the same at temperature 2. The mean is unchanged and the cloud is twice the
posterior's variance, so the covariance is 100% off.

```settle example=descend-temperature
# The negative control: the same line at temperature 2. The mean is still the posterior mean, but the cloud is
# twice as wide as the posterior (in variance), and ask says so.
model :line do
  data "data/descend-line.csv"
  loss :least_squares, noise: 0.5, prior: 10
end
run :line do
  descend 200_000, step: 0.004, temperature: 2, seed: 1
  ask
end
```

Output:

```text output=descend-temperature
data: 30 examples of 1 feature
descended: 200000 steps of 2 parameters (least squares), Langevin, step 0.004, temperature 2, every row; kept 180000 (<time> ms)
  loss at the end 0.7109 per example, energy 21.3415
  w1         mean     1.5050 ± 0.0004   spread 0.1123   posterior mean     1.5042  spread 0.0795
  bias       mean     0.4125 ± 0.0007   spread 0.1347   posterior mean     0.4118  spread 0.0950
ask: 2 parameters; largest mean error 0.0007 (2.0 standard errors) against the exact posterior mean; spreads are step-corrected, and the cloud's covariance is 100.39% off the posterior's (temperature 2)
```

## `score`

**Block:** run.

**Form:** `score`.

**What it does:** predicts every test example (or every training example when there is no `test`) three ways:
`last`, the final position of the first walker; `mean`, the averaged parameters; and `cloud`, the average of the
kept snapshots' predictions. For classes it prints accuracy, the mean negative log-likelihood (nll), the
expected calibration error over 15 confidence bins, and the mean doubt. For values it prints the root mean
squared error, the nll, and the share of targets inside the predictive 95% interval.

**Example:** two rings of points. The logistic piece cannot split them; the net, annealed, can.

```settle example=descend-rings
# Two rings of points: a linear piece cannot split them, a net with 8 hidden things can. The net is settled
# hot and cooled to 0 (annealing), in batches of 20 rows.
model :rings do
  data "data/descend-rings.csv"
  test "data/descend-rings-test.csv"
  loss :logistic
end
model :ringnet do
  data "data/descend-rings.csv"
  test "data/descend-rings-test.csv"
  loss :net, hidden: 8, classes: 2, prior: 3
end
run :rings do
  descend 5_000, step: 0.01
  score
end
run :ringnet do
  descend 20_000, step: 0.005, temperature: 0.05, cool_to: 0, batch: 20, seed: 2
  score
end
```

Output:

```text output=descend-rings
data: 200 examples of 2 features
test: 400 test examples of 2 features
data: 200 examples of 2 features
test: 400 test examples of 2 features
descended: 5000 steps of 3 parameters (logistic, 2 classes), gradient descent, step 0.01, temperature 0, every row; kept 4500 (<time> ms)
  loss at the end 0.6931 per example, energy 138.6151
score on 400 test rows:
  last   accuracy 48.25%   nll 0.6928   calibration error 0.0223   doubt 0.0000
  mean   accuracy 48.25%   nll 0.6928   calibration error 0.0223   doubt 0.0000
  cloud  accuracy 48.25%   nll 0.6928   calibration error 0.0223   doubt 0.0000
score: last = the final position, mean = the averaged parameters, the cloud is 50 snapshots of the path at temperature 0, not a posterior
descended: 20000 steps of 33 parameters (net, 8 hidden, 2 classes), Langevin, step 0.005, temperature 0.05 cooling to 0, batches of 20; kept 18000 (<time> ms)
  loss at the end 0.0299 per example, energy 16.9513
score on 400 test rows:
  last   accuracy 99.50%   nll 0.0357   calibration error 0.0264   doubt 0.0000
  mean   accuracy 99.75%   nll 0.0337   calibration error 0.0263   doubt 0.0000
  cloud  accuracy 99.50%   nll 0.0360   calibration error 0.0286   doubt 0.0025
score: last = the final position, mean = the averaged parameters, the cloud averages 50 snapshots' predictions; doubt is what the snapshots disagree on (nats)
```

**Example:** four walkers at temperature 1. Their mean parameters are useless (50%), because a net's walkers
settle in different valleys; their averaged predictions are the best of the three rows, with doubt.

```settle example=descend-cloud
# The same net at temperature 1 with four walkers: the cloud of 4 x 50 snapshots predicts by averaging, and
# its doubt (how much the snapshots disagree) is what one walker at temperature 0 cannot give.
model :ringnet do
  data "data/descend-rings.csv"
  test "data/descend-rings-test.csv"
  loss :net, hidden: 8, classes: 2, prior: 3
end
run :ringnet do
  descend 10_000, step: 0.002, temperature: 1, walkers: 4, keep: 50, seed: 3
  score
end
```

Output:

```text output=descend-cloud
data: 200 examples of 2 features
test: 400 test examples of 2 features
descended: 10000 steps of 33 parameters (net, 8 hidden, 2 classes), Langevin, step 0.002, temperature 1, every row, 4 walkers; kept 36000 (<time> ms)
  loss at the end 0.0459 per example, energy 27.1953
score on 400 test rows:
  last   accuracy 99.00%   nll 0.0502   calibration error 0.0318   doubt 0.0000
  mean   accuracy 50.00%   nll 4.1967   calibration error 0.4979   doubt 0.0000
  cloud  accuracy 99.75%   nll 0.0561   calibration error 0.0464   doubt 0.0369
score: last = the final position, mean = the averaged parameters, the cloud averages 200 snapshots' predictions; doubt is what the snapshots disagree on (nats)
  the 4 walkers of a net settle in different valleys (its hidden things can swap places), so averaging their parameters mixes valleys; average predictions, as the cloud row does
```

**Example:** Adam, a point with no cloud of its own.

```settle example=descend-adam
# Adam on the rings net, minibatches of 20: a point, no cloud of its own.
model :ringnet do
  data "data/descend-rings.csv"
  test "data/descend-rings-test.csv"
  loss :net, hidden: 8, classes: 2, prior: 3
end
run :ringnet do
  descend 4_000, step: 0.01, method: :adam, batch: 20, keep: 0
  score
end
```

Output:

```text output=descend-adam
data: 200 examples of 2 features
test: 400 test examples of 2 features
descended: 4000 steps of 33 parameters (net, 8 hidden, 2 classes), Adam, step 0.01, temperature 0, batches of 20; kept 3600 (<time> ms)
  loss at the end 0.0442 per example, energy 17.9443
score on 400 test rows:
  last   accuracy 98.25%   nll 0.0497   calibration error 0.0262   doubt 0.0000
  mean   accuracy 99.25%   nll 0.0570   calibration error 0.0430   doubt 0.0000
score: last = the final position, mean = the averaged parameters, no snapshots were kept (keep: 0), so there is no cloud row
```

## Errors

```settle example=descend-adam-hot
# Adam has no temperature, so asking for one is refused.
model :line do
  data "data/descend-line.csv"
  loss :least_squares
end
run :line do
  descend 1_000, method: :adam, temperature: 1
end
```

Error:

```text error=descend-adam-hot
line 7: refused: Adam has no temperature. Settling with noise is method :langevin; Adam is for temperature 0
```

```settle example=descend-no-data
# A loss with examples needs data.
model :fit do
  loss :logistic
end
run :fit do
  descend 1_000
end
```

Error:

```text error=descend-no-data
line 6: descend needs data for this loss; declare it in the model, like: data "points.csv"
```

## What was measured

`experiments/thermosim/runs/gradsettle/REPORT_GRADSETTLE.md`: MNIST fitted by Settling beside SGD and Adam at
the same budget, with sealed predictions.
