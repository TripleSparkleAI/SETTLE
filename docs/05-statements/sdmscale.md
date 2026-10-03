# The sdmscale family

The sdmscale family runs Kanerva's sparse distributed memory at Kanerva's own scale: up to 2,000,000 hard
locations. The [sdm](sdm.md) family keeps every counter as a SETTLE pull, which caps it at 20 million bit-counters.
A million hard locations at word-size 256 needs 256 million. This family keeps the bit-counters outside the model: one byte
each in a flat array, with the addresses packed into bits, and a read is a popcount scan. The arithmetic is the
same as the sdm family's (a unit test proves the bit-counters and both reads bit for bit equal to the sdm family's
on the same addresses), so what is true of one is true of the other. The model keeps only the memory's settings
and the list of what was written, and every read rebuilds the store from that list.

The family also chooses the activation radius for a given read-address address-noise (`tolerate-noise:`) and offers three rules for which
hard locations wake in the pulls read (`wake:`). Those two parts come from the SDMRADIUS lane.

The source is `src/sdmscale.rs`. The activation radius choice and the wake thresholds are in `src/sdmradius.rs`. The lane
reports are `experiments/thermosim/runs/sdmscale/REPORT_SDMSCALE.md` and
`experiments/thermosim/runs/sdmradius/REPORT_SDMRADIUS.md`. SDMSCALE measured, at word-size 256, that the address read
at 10% address-noise holds about 0.02 to 0.03 times the number of hard locations, from 2,000 to 1,000,000 hard locations, while at
30% and 40% address-noise capacity does not grow with the number of hard locations under the default activation radius. SDMRADIUS found
that the activation radius chosen for 30% address-noise makes 30% capacity grow with the number of hard locations, at a price at 10%.

| Statement | Block | Summary |
|---|---|---|
| [`sdmscale`](#sdmscale) | model | declare a large sparse distributed memory |
| [`name.put`](#nameput) | model | write a named random pattern |
| [`name.fill`](#namefill) | model | write a number of unnamed random patterns, as load |
| [`name.read`](#nameread) | run | rebuild the store, read from a noisy read-address, and report whether the pattern came back |

## How the store works

The store is the one described on the [sdm](sdm.md) page. In short: each of `M` hard locations has a fixed random
address and a row of `word-size` bit-counters. Writing a pattern `p` adds it to the bit-counters of every hard location whose
address is within the activation radius `r` of `p`. The address read lets the hard locations within `r` of the current state vote
position by position, and repeats until the state stops changing. The pulls read wakes hard locations by the
agreement of their bit-counters with the state instead.

The differences from the sdm family:

- **No things.** `sdmscale` adds no things, leans or pulls to the model. It keeps a note under
  `sdmscale:<name>` with the size, the number of hard locations, the activation radius, the seed, and the list of what was put
  and filled. Other statements cannot see the memory's state.
- **Byte bit-counters.** Each counter is a whole number from -127 to 127. An update that would leave that range is
  clamped. The store counts clamped updates, but no statement prints the count.
- **Rebuilt on every read.** Each `name.read` draws the addresses from the seed and writes every pattern in the
  list again, in the order they were put and filled. A store takes `word-size x locations` bytes of memory while a
  read runs. The wake sets of the writes are found on several threads (`SDMSCALE_THREADS`, default 8); the result
  is the same as writing one pattern at a time.
- **Only random patterns.** No text and no keys.

The addresses come from a generator seeded by `seed` mixed with a fixed constant, 64 bits at a time. They are not
the same addresses the sdm family draws for the same seed.

## `sdmscale`

**Block:** model.

**Form:**

```text
sdmscale :name, word-size: 256, hard-locations: 100000, activation-probability: 0.001, seed: 1
sdmscale :name, word-size: 256, hard-locations: 100000, tolerate-noise: 0.3, seed: 1
sdmscale :name, word-size: 256, hard-locations: 100000, activation-radius: 103, seed: 1
```

**Arguments:**

| Argument | Type | Default | Meaning |
|---|---|---|---|
| `:name` | symbol | required | the memory's name, used as `name.put`, `name.fill` and `name.read` |
| `word-size:` | whole number, 16 to 4096 | `256` | the length of every pattern |
| `hard-locations:` | whole number, 1 to 2,000,000 | `100000` | how many hard locations |
| `activation-probability:` | number, above 0 and below 1 | `0.001` | the fraction of hard locations a random pattern should wake; sets the activation radius |
| `tolerate-noise:` | number, from 0 to below 0.5 | off | choose the activation radius for read-addresses with this fraction of address-noise; overrides `activation-probability:` |
| `activation-radius:` | whole number, at most `word-size` | from `tolerate-noise:` or `activation-probability:` | the activation radius; overrides both |
| `seed:` | whole number | `1` | the seed of the addresses and of the fill patterns |

`word-size` times `hard-locations` must be at most 1,100,000,000 (bytes of bit-counters). The comma after `:name` is
optional. Numbers given for `word-size:`, `hard-locations:`, `activation-radius:` and `seed:` are cut to whole numbers. `activation-probability:` is
checked even when `tolerate-noise:` or `activation-radius:` decides the activation radius. A negative `tolerate-noise:` counts as not given.

**What it does:** checks the arguments, chooses the activation radius, and records the memory in the notes. It builds
nothing: the store exists only while a read runs.

The activation radius is chosen in this order:

1. `activation-radius:` if given.
2. Otherwise, with `tolerate-noise: D`, the activation radius whose predicted capacity for read-addresses `D` noisy is largest (below).
3. Otherwise, the smallest activation radius `r` with `P[Binomial(word-size, 1/2) <= r] >= activation-probability`. For `activation-probability: 0.001` that is 47 at
   word-size 128 and 103 at word-size 256.

**How `tolerate-noise:` chooses the activation radius.** It uses the Bricken-Pehlevan signal-to-noise map (the S-map), which
predicts where an iterated address read goes. Let `I(d)` be the fraction of all addresses within `r` of both of
two points `d` bits apart, computed exactly. For `T` stored patterns and a state `d` bits from its pattern:

```text
SNR(d) = M I(d) / sqrt( M I(d) + (T - 1) (M I_o + (M I_o)^2) ),   I_o = I(size / 2)
d_next = size * Phi( -SNR(d) )
```

The pattern's own shared hard locations are the signal; the other `T - 1` patterns are the noise. `Phi` is the
standard normal distribution function, and `d_next` is the predicted distance after one read.

A activation radius's capacity at address-noise `D` is the largest `T` for which the map, started at `d_0 = round(D * size)`,
reaches a distance of at most `0.025 * size` (an overlap of at least 0.95) within 50 steps. The map stops early,
as a failure, if a step does not reduce the distance. `T` is found by doubling, then bisection.

The radii tried form a window around the activation radius that suits undamaged read-addresses. With
`p = (M^2 / 10)^(-1/3)` and `r_0` the smallest activation radius whose wake fraction is at least `p`, the window is
`r_0 - 4` to the larger of `r_0 + 1` and the activation radius waking 5% of hard locations. The activation radius with the largest capacity
wins; a tie goes to the smaller activation radius. If no activation radius in the window has any capacity, the bottom of the window is
used.

The S-map uses the mean number of shared hard locations only. The SDMRADIUS report compares it with measured
capacities and with sharper predictors; at 30% address-noise it overestimates the measured capacity by about 4 to 13
times, but it picks radii at which 30% capacity grows with the number of hard locations.

**Output:** none. No statement prints the chosen activation radius.

**Errors:**

- `sdmscale :<name> is already declared`
- `` sdmscale does not take `<key>:` ``
- `sdmscale word-size must be between 16 and 4096`
- `sdmscale hard-locations must be 1 to 2,000,000, and word-size x hard-locations at most 1.1 billion bytes`
- `activation-probability must be between 0 and 1`
- `tolerate-noise is an address-noise fraction below 0.5`
- `activation-radius cannot be larger than word-size`
- `a number was expected`

## `name.put`

**Block:** model.

**Form:**

```text
name.put :cat
```

**Arguments:**

| Argument | Type | Default | Meaning |
|---|---|---|---|
| `name` | the name of a declared sdmscale, written without a colon | required | which memory to write to |
| `:cat` | symbol | required | the pattern's name |

**What it does:** adds the name to the memory's list. The pattern is the name's random pattern: `word-size` values of
+1 or -1 from a generator seeded by the FNV-1a hash of the name, the same pattern the sdm family writes for that
name. It is written into the store each time a read rebuilds it.

The line is claimed only when `name` is a declared sdmscale, and only in a model block. In a run block, or for a
name that is not an sdmscale, no family claims it and the line is an error.

**Output:** none.

**Errors:**

- `:<what> is already in :<name>`

## `name.fill`

**Block:** model.

**Form:**

```text
name.fill 500
```

**Arguments:**

| Argument | Type | Default | Meaning |
|---|---|---|---|
| `name` | the name of a declared sdmscale, written without a colon | required | which memory to write to |
| count | whole number | required | how many random patterns to add |

**What it does:** adds `count` unnamed random patterns to the memory's list, as load. They come from one
generator seeded by the FNV-1a hash of `sdmscale-fill:<name>:<seed>`, so the same memory always gets the same
patterns. They are written in the list's order: patterns put before the fill are written before them, patterns
put after are written after. A fill's patterns have no names and cannot be used as a read-address. A number is cut to a
whole number, and a negative count counts as 0.

A memory takes one fill. To load more, give a larger count.

**Output:** none.

**Errors:**

- `:<name> is already filled; one fill per store`

## `name.read`

**Block:** run.

**Form:**

```text
name.read read-address: :cat, address-noise: 0.2, iterated-reads: 20, via: :addresses, seed: 1
name.read read-address: :cat, address-noise: 0.2, iterated-reads: 20, via: :pulls, wake: :fixed, seed: 1
```

**Arguments:**

| Argument | Type | Default | Meaning |
|---|---|---|---|
| `name` | the name of a declared sdmscale, written without a colon | required | which memory to read |
| `read-address:` | symbol | required | the pattern to start from |
| `address-noise:` | number, a chance from 0 to 1 (not checked) | `0.2` | the chance that each value of the read-address is flipped |
| `iterated-reads:` | whole number | `20` | the most rounds to read; 0 counts as 1 |
| `via:` | one of `:addresses` / `:pulls` | `:addresses` | wake hard locations by their addresses, or by their bit-counters |
| `wake:` | one of `:fixed` / `:density` / `:top` | `:fixed` | how the pulls read chooses which hard locations wake; only with `via: :pulls` |
| `seed:` | whole number | the run's generator | replace the run's random generator with one seeded by this number |

**What it does:**

1. If `seed:` is given, the run's generator is replaced by one seeded with it, and kept for the rest of the run
   block.
2. It rebuilds the store from the note: the addresses from the seed, then every put and fill pattern written in
   order.
3. It builds the read-address: the random pattern of the `read-address:` name, whether or not it was put, with each value flipped
   with chance `damage`. This uses one draw of the run's generator per value.
4. It reads, round by round, until a round leaves the state unchanged or `iterations` rounds are done. In every
   round the state becomes the sign of the summed counter rows of the woken hard locations, position by position, and
   a zero sum keeps the old value. The read mode decides which hard locations wake:

   - **`via: :addresses`** wakes every hard location whose address is within the activation radius of the state: Kanerva's
     read. Hard locations that hold no bit-counters wake too but add nothing.
   - **`via: :pulls`** looks only at hard locations that hold at least one non-zero counter, and wakes them by the
     agreement `C_i . z` of their bit-counters with the state. Addresses are not used. `wake:` chooses the rule:

     | `wake:` | a hard location wakes when |
     |---|---|
     | `:fixed` | `C_i . z > 0.4 * size`, the sdm family's pulls read at zero temperature |
     | `:density` | `C_i . z > theta`, a threshold scaled with the store's load (below) |
     | `:top` | it is one of the `k` hard locations with the largest `C_i . z`, `k = max(1, round(p * M))`, ties to the lower index |

     Here `p = P[Binomial(word-size, 1/2) <= r]` is the fraction of hard locations a random pattern wakes, so `p * M` is
     about how many hard locations hold any one pattern.

   **The density threshold.**

   ```text
   L     = mean over filled locations of (sum_j C_ij^2) / size, at least 1
   kappa = max(1, Phi^-1(1 - min(0.5, 0.1 * p * M / M_f)))
   theta = kappa * sqrt(size * L)
   ```

   `L` is the mean load of a filled row (a row holding `L` random patterns has `E[C_ij^2] = L`). A row that does
   not hold the pattern has `C_i . z` spread like a normal with variance `size * L`. `kappa` sets the threshold so
   that the expected number of such rows woken by chance is a tenth of the `p * M` rows that do hold the pattern.
   `M_f` is the number of filled rows.

5. It compares the result with the read-address's pattern (the undamaged one) by the overlap
   `q = (1 / size) * sum_j z_j p_j` and prints one line.

The read does not change the model or the run's last arrangement.

**Output:**

```text
read :<name> from read-address :<read-address> with <address-noise x 100>% address-noise via <addresses|pulls> (<rounds> iterated reads, <activated> of <hard-locations> hard locations activated, <holding> holding bit-counters): <verdict>
```

- `<address-noise x 100>` is rounded to a whole percent.
- `<rounds>` is the number of rounds done, counting the round that changed nothing.
- `<activated>` is the number of hard locations woken in the last round. `<holding>` is how many of them hold a non-zero
  counter. For the pulls read the two are equal.
- `<verdict>` is one of:
  - `-> :<read-address>` when the overlap is at least 0.95 and the read-address's name was put;
  - `-> back to :<read-address> though it was never written (the read did not move it)` when the overlap is at least 0.95
    and the name was never put;
  - `-> nothing clear (overlap with :<read-address> <overlap>)` otherwise, the overlap signed with two decimals.

The `wake:` rule is not named in the output.

**Example:** reads by addresses and by pulls, and a read-address that was never put.

```settle example=sdmscale-read
# A Kanerva memory whose bit-counters live outside the pulls: one byte each, in a flat array.
model :big do
  sdmscale :k, word-size: 128, hard-locations: 20_000   # activation-probability: 0.001 gives activation radius 47
  k.put :cat                                  # a named random pattern
  k.put :dog
  k.fill 300                                  # 300 more random patterns, as load
end

run :big do
  k.read read-address: :cat, address-noise: 0.1, seed: 1                 # Kanerva's read by addresses
  k.read read-address: :dog, address-noise: 0.1, via: :pulls, seed: 2    # hard locations woken by what they hold
  k.read read-address: :zebra, address-noise: 0, seed: 3                 # a name that was never put
end
```

Output:

```text output=sdmscale-read
read :k from read-address :cat with 10% address-noise via addresses (4 iterated reads, 32 of 20000 hard locations activated, 32 holding bit-counters): -> :cat
read :k from read-address :dog with 10% address-noise via pulls (2 iterated reads, 35 of 20000 hard locations activated, 35 holding bit-counters): -> :dog
read :k from read-address :zebra with 0% address-noise via addresses (7 iterated reads, 41 of 20000 hard locations activated, 41 holding bit-counters): -> nothing clear (overlap with :zebra +0.09)
```

**Example:** the three wake rules on one 30%-noisy read-address.

```settle example=sdmscale-wake
# The pulls read wakes a hard location by what it holds: by the agreement of its bit-counters with the state.
# Three rules pick which hard locations wake. At 30% address-noise the fixed rule wakes nothing.
model :big do
  sdmscale :k, word-size: 128, hard-locations: 20_000
  k.put :cat
  k.fill 100
end

run :big do
  k.read read-address: :cat, address-noise: 0.3, via: :pulls, seed: 1                  # wake: :fixed, above 0.4 x size
  k.read read-address: :cat, address-noise: 0.3, via: :pulls, wake: :density, seed: 1  # a threshold scaled with the load
  k.read read-address: :cat, address-noise: 0.3, via: :pulls, wake: :top, seed: 1      # the best p x M rows
end
```

Output:

```text output=sdmscale-wake
read :k from read-address :cat with 30% address-noise via pulls (1 iterated reads, 0 of 20000 hard locations activated, 0 holding bit-counters): -> nothing clear (overlap with :cat +0.27)
read :k from read-address :cat with 30% address-noise via pulls (3 iterated reads, 92 of 20000 hard locations activated, 92 holding bit-counters): -> nothing clear (overlap with :cat +0.44)
read :k from read-address :cat with 30% address-noise via pulls (2 iterated reads, 34 of 20000 hard locations activated, 34 holding bit-counters): -> :cat
```

This read-address's overlap with `:cat` is +0.27, so a row that holds only `:cat` agrees with it by `0.27 * size`, below
the fixed threshold of `0.4 * size`, and no row wakes. A read-address with exactly 30% address-noise would sit exactly on that
threshold. The density threshold wakes 92 rows, too many. The top rule wakes the 34 rows that agree best, about
the number that hold any one pattern (`p * M` is 33.7 here), and reads `:cat` back. The SDMRADIUS report measured the same ordering: top-k held
the most patterns at 30% address-noise, and the density rule failed at loads of 2 to 20 patterns because the rows that
hold one other pattern wake together.

**Example:** the default activation radius and the activation radius chosen for 30% address-noise.

```settle example=sdmscale-tolerate
# tolerate-noise: picks the activation radius for read-addresses with that much address-noise, instead of the activation radius activation-probability: gives.
# Both memories hold the same 31 patterns; each is read from three different cats at 30% address-noise.
model :default do
  sdmscale :k, word-size: 128, hard-locations: 20_000                # activation-probability: 0.001 gives activation radius 47
  k.put :cat
  k.fill 30
end

model :tolerant do
  sdmscale :k, word-size: 128, hard-locations: 20_000, tolerate-noise: 0.3 # the activation radius chosen for 30% address-noise
  k.put :cat
  k.fill 30
end

run :default do
  k.read read-address: :cat, address-noise: 0.3, seed: 1
  k.read read-address: :cat, address-noise: 0.3, seed: 2
  k.read read-address: :cat, address-noise: 0.3, seed: 3
end

run :tolerant do
  k.read read-address: :cat, address-noise: 0.3, seed: 1
  k.read read-address: :cat, address-noise: 0.3, seed: 2
  k.read read-address: :cat, address-noise: 0.3, seed: 3
end
```

Output:

```text output=sdmscale-tolerate
read :k from read-address :cat with 30% address-noise via addresses (2 iterated reads, 28 of 20000 hard locations activated, 28 holding bit-counters): -> nothing clear (overlap with :cat +0.17)
read :k from read-address :cat with 30% address-noise via addresses (4 iterated reads, 51 of 20000 hard locations activated, 51 holding bit-counters): -> nothing clear (overlap with :cat +0.12)
read :k from read-address :cat with 30% address-noise via addresses (2 iterated reads, 32 of 20000 hard locations activated, 32 holding bit-counters): -> :cat
read :k from read-address :cat with 30% address-noise via addresses (6 iterated reads, 169 of 20000 hard locations activated, 169 holding bit-counters): -> nothing clear (overlap with :cat +0.00)
read :k from read-address :cat with 30% address-noise via addresses (3 iterated reads, 168 of 20000 hard locations activated, 168 holding bit-counters): -> :cat
read :k from read-address :cat with 30% address-noise via addresses (4 iterated reads, 168 of 20000 hard locations activated, 168 holding bit-counters): -> :cat
```

The default activation radius recalls one of the three read-addresses and the chosen activation radius two. The chosen activation radius wakes about 168
hard locations, which is what activation radius 50 wakes on average at word-size 128 and 20,000 hard locations. Three read-addresses are far too few
to measure a capacity; the reports use 60 to 120 read-addresses per cell.

**Example:** `wake:` without the pulls read.

```settle example=sdmscale-wake-needs-pulls
# wake: chooses how the pulls read wakes hard locations, so it needs via: :pulls.
model :big do
  sdmscale :k, word-size: 64, hard-locations: 1_000
  k.put :cat
end

run :big do
  k.read read-address: :cat, wake: :top
end
```

```text error=sdmscale-wake-needs-pulls
line 8: wake: applies to via: :pulls
```

`wake: :fixed` is accepted with the address read, because it is the default.

**Errors:**

- `read needs read-address: :name`
- `via: takes :addresses or :pulls`
- `wake: takes :fixed (0.4 n), :density (scaled with the rows' load) or :top (the p M best rows)`
- `wake: applies to via: :pulls`
- `` read does not take `<key>:` ``
- `a number was expected`

## Notes

- Every read rebuilds the store and rewrites every pattern. A program that reads a large store many times pays
  for the writes each time. At 2,000,000 hard locations and word-size 256 a store is 512 MB.
- Counters are clamped to -127 to 127. A hard location holds more than 127 patterns only in very dense stores, and the
  clamping is not reported by any statement.
- `address-noise:` is not checked. A value above 1 flips every value; a negative value flips none.
