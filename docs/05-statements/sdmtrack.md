# The sdmtrack family

The sdmtrack family predicts how often a sparse distributed memory recalls a noisy pattern when its read wakes
hard locations by what they hold rather than by their addresses. These are the two content reads of the
[sdmscale](sdmscale.md#nameread) store, `via: :pulls` with `wake: :top` (the top-k read) and `wake: :density`
(the block read). The predictor is called TRACK-C. It does not build a memory. It follows every stored pattern
through the read as a count: how many woken hard locations hold that pattern. It samples those counts from their
predicted distribution and lets them vote. The family's one statement, `contenttrack`, is a calculator that prints
TRACK-C's predicted recall for a memory size, a load and a address-noise.

The source is `src/sdmtrack.rs`. Most of the file is measurement code with no statement of its own: a reference
predictor on a random membership graph, a predictor that also places the hard locations' addresses, a traced top-k
read of a real store, and combined refusal rules. The measurement program `examples/sdmtrack_measure.rs` uses them.
The family was measured in `experiments/thermosim/runs/sdmtrack/REPORT_SDMTRACK.md`. There, TRACK-C matched the
real store's failure rate to about 0.02 where `p T < 2` (light loads). Above `p T` of about 2 it predicted
failure far earlier than the real store (mean absolute error about 0.3). The family continues the
[sdmrefuse](sdmrefuse.md) family, whose predictor covers the address read.

| Statement | Block | Summary |
|---|---|---|
| [`contenttrack`](#contenttrack) | run | print TRACK-C's predicted recall for a content read, a size, a load and a address-noise |

## `contenttrack`

**Block:** run.

**Form:**

```text
contenttrack word-size: 256, hard-locations: 100_000, load: 1000, address-noise: 0.3, block: 0, samples: 200
```

**Arguments:**

| Argument | Type | Default | Meaning |
|---|---|---|---|
| `word-size:` | whole number, 64 to 1024 | 256 | `n`: the number of bits in a pattern and in a hard location's address. |
| `hard-locations:` | whole number, at least 100 | 100000 | `M`: the number of hard locations in the memory. |
| `load:` | whole number, at least 1 | 1000 | `T`: the number of random patterns the memory holds. |
| `address-noise:` | number, at least 0 and below 0.5 | 0.3 | `D`: the chance that each bit of the read-address is flipped. |
| `block:` | number | 0 | `0` predicts the top-k read; any other number predicts the block read. |
| `samples:` | whole number, at least 1 | 200 | `S`: the number of sampled reads for each of the two predictions. |

The help line shows `load: 3000` as an example value; the default is 1000. `block:` takes a number, so
`block: :yes` is refused with `a number was expected`.

**What it does:** builds no memory and does not read or change the model or the run.

1. **The activation radius.** `r` is the smallest activation radius with

   ```text
   p = P[Bin(n, 1/2) <= r] >= (M^2 / 10)^(-1/3)
   ```

   where `p` is the fraction of all addresses within `r` bits of a given point. A write then reaches about `p M`
   hard locations. This is the activation radius rule of the [sdmscale](sdmscale.md) window.
2. **The wake rule.**
   - **Top-k** wakes the `k = max(1, round(p M))` filled hard locations whose match `C_i . z` with the state is largest.
   - **Block** wakes every hard location whose match exceeds a threshold `theta`. The threshold is the one the
     sdmscale store's `wake: :density` sets, computed from the store's expected shape instead of a real store:

     ```text
     lambda = p T,   L = max(1, lambda / (1 - e^-lambda))
     theta = max(k_row, k_pat) * sqrt(n L)
     k_row = max(1, Phi^-1(1 - min(0.5, 0.1 p M / (M (1 - e^-lambda)))))
     k_pat = max(1, Phi^-1(1 - min(0.5, 0.01 / (T - 1))))
     ```

     `L` is the mean number of patterns in a filled hard location. `Phi^-1` is the inverse of the standard normal
     distribution function. `k_pat` uses `T - 1` of at least 1.
3. **One sampled read.** It draws `T` random `n`-bit patterns. The target is the first. The read-address is the target with
   each bit flipped with probability `D`. The state starts at the read-address. Each round, for every pattern `mu`:
   - `u_mu = n/2 - d(x_mu, z)` is its overlap with the state, in half units.
   - The match of a hard location holding `mu` is `u_mu` plus the rest: the sum of the overlaps of the hard location's other
     patterns. TRACK-C takes the rest to be a compound Poisson sum: a Poisson(`p (T - 1)`) number of overlaps
     drawn from the current overlaps of all patterns. It computes that distribution with a fast Fourier transform.
     When `T` is 64 or less, each pattern's own overlap is left out of its own rest.
   - The cut is where the wake rule stops. For top-k it is the match value `x` with exactly `k` filled hard locations
     above it, a fraction of those at `x` being woken to make the count exact:

     ```text
     M (P[match > x] - e^-lambda 1[x < 0]) = k
     ```

     Empty hard locations have a match of 0 and never wake. If the expected number of filled hard locations is `k` or
     fewer, every hard location wakes. For block the cut is `theta`.
   - `q_mu = P[u_mu + rest > cut]` is the chance that a hard location holding `mu` wakes.
   - The number of woken hard locations holding `mu` is `k_mu`. **FRESH** draws it anew each round,
     `k_mu ~ Poisson(p M q_mu)`. **PERSIST** keeps each hard location's rank from round to round: when `q_mu` rises it
     adds `Poisson(p M (q_mu - q_mu_before))` hard locations, and when it falls it keeps each woken hard location with
     probability `q_mu / q_mu_before`. A Poisson draw with a mean above 30 uses a rounded normal draw with the
     same mean and variance.
   - The vote on bit `j` is `sum_mu k_mu x_mu,j` with bits as +1 and -1. Each bit of the state takes the sign of
     its vote; a vote of 0 keeps the bit.

   The read stops when the state no longer changes, or after 20 rounds. It succeeds if the final state's overlap
   with the target is at least 0.95, that is, if it differs from the target in at most `0.025 n` bits.
4. **The two predictions.** It runs `S` sampled reads with FRESH counts, then `S` with PERSIST counts, and prints
   the fraction of each that succeeded.

The random draws come from a generator with a fixed seed, started afresh for each of the two predictions. The run's
generator and `seed:` are not used, and the statement takes no `seed:`. The same arguments always print the same
line.

**Output:** one line:

```text
contenttrack <top-k|block> read, <M> locations of <n> bits, radius <r>, <T> patterns, <D x 100>% damage: TRACK-C predicts recall <fresh> (fresh) <persist> (persist) over <S> sampled reads
```

The address-noise is rounded to a whole percent. The two recall fractions have three decimal places.

**Example:**

```settle example=sdmtrack-predict
# TRACK-C's predicted recall for a memory of 20,000 hard locations of 256 bits. No memory is built.
model :m do
end
run :m do
  contenttrack hard-locations: 20_000, load: 300, samples: 20                # a light load at 30% address-noise
  contenttrack hard-locations: 20_000, load: 300, address-noise: 0.4, samples: 20   # more address-noise in the read-address
  contenttrack hard-locations: 20_000, load: 2_000, samples: 10              # a heavier load
  contenttrack hard-locations: 20_000, load: 2_000, samples: 10, block: 1    # the same load, block read
end
```

Output:

```text output=sdmtrack-predict
contenttrack top-k read, 20000 hard locations of 256 bits, activation-radius 106, 300 patterns, 30% address-noise: TRACK-C predicts recall 1.000 (fresh) 1.000 (persist) over 20 sampled reads
contenttrack top-k read, 20000 hard locations of 256 bits, activation-radius 106, 300 patterns, 40% address-noise: TRACK-C predicts recall 0.650 (fresh) 0.500 (persist) over 20 sampled reads
contenttrack top-k read, 20000 hard locations of 256 bits, activation-radius 106, 2000 patterns, 30% address-noise: TRACK-C predicts recall 0.900 (fresh) 0.800 (persist) over 10 sampled reads
contenttrack block read, 20000 hard locations of 256 bits, activation-radius 106, 2000 patterns, 30% address-noise: TRACK-C predicts recall 0.300 (fresh) 0.500 (persist) over 10 sampled reads
```

With 20,000 hard locations the activation radius is 106, so `p M` is about 71 and `p T` is about 1.1 at 300 patterns and 7.1 at
2,000. The last two lines are therefore in the range where the report found TRACK-C too pessimistic. The example
uses few samples to stay fast; each fraction then moves in steps of 0.05 or 0.1.

**Errors:**

- `` contenttrack does not take `<key>:` ``
- `a number was expected`
- `contenttrack needs word-size 64..1024, hard-locations >= 100, load >= 1, address-noise in [0, 0.5), samples >= 1`

## Notes

- Each round of each sampled read costs Fourier transforms of 8,192 points, so the time grows with `samples:` and
  with the number of rounds a read takes. Heavy loads take more rounds. With the defaults the statement took about
  0.7 seconds on the machine used for this page. Loads of 64 or less are slower per read than somewhat larger
  loads, because each distinct overlap needs its own transform.
- `word-size:`, `hard-locations:`, `load:` and `samples:` are cut to whole numbers, so `load: 0.5` is `load: 0` and is
  refused.
- TRACK-C treats every pattern's count as independent. The report's hypothesis for its pessimism at heavy load is
  that in a real store the other patterns in a woken target hard location lean toward the target, which a count model
  cannot express. The report marks this as not measured.
