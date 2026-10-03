# The softsdm family

The softsdm family builds Kanerva's sparse distributed memory out of p-bits, with a soft cut-off. In the
[sdm](sdm.md) family a hard location either wakes or does not, depending on whether the read-address is within a fixed
activation radius of its address. Here every hard location is a p-bit that is activated with a probability that falls smoothly
with that distance. One dial, `softness:`, sets how smooth the fall is. At softness 0 the machine is exactly the
hard SDM; as softness rises the edge of the ball blurs. The family also computes, outside the sampler, the read
this machine approaches with infinitely many samples, with infinitely many hard locations, and as softmax attention.

The source is `src/softsdm.rs`. The lane report is `experiments/thermosim/runs/softsdm/REPORT_SOFTSDM.md`. Its
main results: at a activated fraction of 0.01 the machine holds about 80, 40 and 10 patterns at 10%, 20% and 30%
address-noise (half of the read-addresses recalled) for softness up to 0.25, recall collapses from softness 0.5 up, and the settle read (below) does not beat
the one-way pass.

| Statement | Block | Summary |
|---|---|---|
| [`softsdm`](#softsdm) | model | declare a soft sparse distributed memory |
| [`name.write`](#namewrite) | both | write a random pattern or a text |
| [`name.read`](#nameread) | run | read from a noisy read-address or noise by sampling, and report what came back |
| [`name.attend`](#nameattend) | run | compute three limit reads outside the sampler and compare them |

## The machine

A softsdm with size `n` and `M` hard locations adds three blocks of things to the model:

- **address things** `s_addr_0` to `s_addr_<n - 1>`. During a read they hold the read-address.
- **hard location things** `s_loc_0` to `s_loc_<M - 1>`. Each is a p-bit with a fixed random address `x_m`, a
  pattern of `n` values of +1 or -1.
- **data things** `s_data_0` to `s_data_<n - 1>`. They are free during a read and take the vote of the
  hard locations that are activated.

Let `d` be the Hamming distance from the read-address to a hard location's address. The hard location is activated with probability:

```text
phi(d) = 1 / (1 + exp(-(t - d) / w)),    w = softness * sqrt(n) / 2
```

A hard location close to the read-address is activated almost surely, a far one almost never, and `w` sets how wide the band between
them is. `w` is measured in units of the spread of a random read-address's distance, `sqrt(n) / 2` bits.

At softness 0, `phi(d)` is 1 when `d` is at most the activation radius `r` and 0 otherwise: classic hard SDM.

**The activation radius and the threshold.** `activation-probability:` asks for a fraction of hard locations. The activation radius `r` is the smallest
activation radius with `P[Binomial(n, 1/2) <= r] >= activation-probability`, and that probability is the actual expected activated fraction
`f`. For word-size 128 and `activation-probability: 0.05` the activation radius is 55 and `f` is 0.066. For word-size 256 it is 115 and 0.059. At
softness 0 the threshold is `t = r + 0.5`. At any other softness, `t` is found by bisection so that the expected
activated fraction for a random read-address stays `f`. The softness dial therefore changes the shape of the cut-off, never
how many hard locations are activated on average.

**Writing.** To write a pattern `p`, the address things are held at `p` and each hard location is activated or not
`write_samples` times. Location `m` adds `f_m * p` to its row of bit-counters `J_m`, where `f_m` is the fraction of
those tries in which it was activated:

```text
J_m <- J_m + f_m * p
```

With `write_samples: 0`, `f_m` is the exact activated probability `phi(d)` instead of a sampled fraction.

**The model's pulls.** The machine is laid out as ordinary things, leans and pulls, so the energy of the model is
the energy of the machine:

```text
pull(s_addr_j, s_loc_m) = x_mj / (4w)          lean(s_loc_m) = (2t - n) / (4w)
pull(s_loc_m, s_data_k) = g * J_mk / 4         lean(s_data_k) = g * sum_m J_mk / 4
g = gain / (f * M)
```

With the address things at a read-address, a hard location's input is `(t - d) / (2w)`, which gives exactly the activated
probability `phi(d)` under SETTLE's update rule. `g` scales the bit-counters so that one clean stored pattern gives a
data field `g * F` (below) of about `gain`. At softness 0 the pulls use `w = 1e-9`, so every hard location input is
saturated.

The addresses are drawn from `seed:` and are not stored. The bit-counters are stored twice: in the pulls, and as
numbers in the model's notes under `softsdm:<name>`, together with the settings and the list of what was
written. The reads use the copy in the notes.

## `softsdm`

**Block:** model.

**Form:**

```text
softsdm :name, word-size: 256, hard-locations: 2000, activation-probability: 0.05, softness: 0.3, gain: 64, seed: 1, write_samples: 16
```

**Arguments:**

| Argument | Type | Default | Meaning |
|---|---|---|---|
| `:name` | symbol | required | the memory's name, used as `name.write`, `name.read` and `name.attend` |
| `word-size:` | whole number, 8 to 4096 | `256` | the length of every pattern: the number of address things and of data things |
| `hard-locations:` | whole number, at least 1 | `2000` | how many hard location things |
| `activation-probability:` | number, above 0 and below 1 | `0.05` | the fraction of hard locations that should be activated for a random read-address |
| `softness:` | number, at least 0 | `0.3` | the width of the activated edge, in units of `sqrt(size) / 2` bits; 0 is hard SDM |
| `gain:` | number above 0 | `64` | the data input one clean stored pattern gives |
| `seed:` | whole number | `1` | the seed of the random addresses |
| `write_samples:` | whole number | `16` | how many activated tries each write samples per hard location; 0 uses the exact probability |

`hard-locations` times `word-size` must be at most 4,000,000. The comma after `:name` is optional. Numbers given for
`word-size:`, `hard-locations:`, `seed:` and `write_samples:` are cut to whole numbers.

**What it does:** computes the activation radius and threshold (above), draws the addresses from a generator seeded by
`seed` mixed with a fixed constant, and adds the three blocks of things at the end of the model in the order
address, hard location, data. It sets each hard location's lean, a pull from each address thing to each hard location, and a
zero pull from each hard location to each data thing. It records the machine in the notes under `softsdm:<name>`.
Nothing is written yet.

**Output:** none.

**Example:** see [`name.read`](#nameread).

**Errors:**

- `softsdm :<name> is already declared`
- `` softsdm does not take `<key>:` ``
- `softsdm word-size must be between 8 and 4096`
- `softsdm needs at least 1 hard location and at most 4,000,000 location-bits (hard-locations x word-size)`
- `activation-probability is a fraction of hard locations, above 0 and below 1`
- `softness must be at least 0 and gain above 0`
- `a thing :<name>_addr_0 already exists; pick another softsdm name` (also for `_loc_0` and `_data_0`)
- `a number was expected`

## `name.write`

**Block:** both. In a run block it changes the model, and the change stays for later run blocks.

**Form:**

```text
name.write :cat
name.write :note, "some text"
```

**Arguments:**

| Argument | Type | Default | Meaning |
|---|---|---|---|
| `name` | the name of a declared softsdm, written without a colon | required | which memory to write to |
| `:cat` | symbol | required | the name of what is written |
| `"some text"` | string | none | text to write |

**What it does:** builds a pattern of `word-size` values and writes it with the rule above, then adds the change to
the pulls between hard locations and data things and to the data things' leans.

- **A symbol only.** The name's random pattern, from a generator seeded by the FNV-1a hash of the name.
- **Text.** The text's bytes become bits, eight per byte, most significant bit first, with 1 as +1 and 0 as -1.
  Each bit is multiplied by the matching value of the name's random pattern, and the positions after the text
  keep that pattern's values. The text needs `8 * bytes` positions, at most `word-size`, and is kept in the notes so
  that `name.read` can decode it.

There is no keyed write in this family. For keyed text use the [sdm](sdm.md) or [memory](memory.md) family.

The activated tries use a random generator. In a model block every `name.write` statement starts a fresh generator
with the same fixed seed, so a model block's writes do not depend on the statements around them. In a run block
the write draws from the run's generator, so it depends on the run's seed and on earlier statements.

The line is claimed by this family only when `name` is a declared softsdm.

**Output:** none.

**Example:** see [`name.read`](#nameread). A keyed write is refused:

```settle example=softsdm-no-key
# softsdm has no keyed write; a trailing key: is refused.
model :mind do
  softsdm :s, word-size: 64, hard-locations: 200
  s.write :diary, "secret", key: "blue heron"
end
```

```text error=softsdm-no-key
line 4: write takes a symbol, and optionally text: s.write :note, "some text"
```

**Errors:**

- `:<what> is already written in :<name>`
- `<bytes> bytes of text need <bits> bits; softsdm :<name> has word-size <word-size>`
- `write takes a symbol, and optionally text: s.write :note, "some text"`
- `the pulls of softsdm :<name> were changed by hand; its layout is fixed`
- `a "quoted" string was expected`

The layout check counts the neighbours of the first hard location thing and the first data thing. It refuses a write
when a pull was added to either of them by hand. Pulls added to other things of the memory are not detected.

## `name.read`

**Block:** run.

**Form:**

```text
name.read read-address: :cat, address-noise: 0.3, rounds: 3, samples: 16, mode: :pass, burn: 10, seed: 1
name.read rounds: 3, samples: 16, mode: :pass, burn: 10, seed: 1
```

**Arguments:**

| Argument | Type | Default | Meaning |
|---|---|---|---|
| `name` | the name of a declared softsdm, written without a colon | required | which memory to read |
| `read-address:` | symbol | none | start from the pattern written under this name |
| `address-noise:` | number from 0 to 1 | `0.3` | the chance that each value of the read-address is flipped |
| `rounds:` | whole number | `3` | how many reads, each fed the last read's result; 0 counts as 1 |
| `samples:` | whole number | `16` | how many samples each read takes; 0 counts as 1 |
| `mode:` | one of `:pass` / `:settle` | `:pass` | the one-way pass, or a joint settle with feedback |
| `burn:` | whole number | `10` | sweeps discarded before the samples are counted; used only by `:settle` |
| `seed:` | whole number | the run's generator | replace the run's random generator with one seeded by this number |

Without `read-address:` the read starts from random values, which the output calls "pure noise". `address-noise:` is checked
even then, but has no effect. A `read-address:` name that was never written gives that name's random pattern.

**What it does:**

1. If `seed:` is given, the run's generator is replaced by one seeded with it, and kept for the rest of the run
   block.
2. It builds the read-address: the pattern written under `read-address:` (text patterns included), with each value flipped with
   chance `damage`, or `word-size` random values. Each uses one draw per value.
3. It reads `rounds` times. Each read's result becomes the next read's read-address.

**`mode: :pass`** is faithful to Kanerva's one-way read. The address things hold the read-address. In each of `samples`
samples, every hard location is activated or not with probability `phi(d)`, the rows of the activated hard locations are summed into
a field `F`, and each data thing is drawn with:

```text
P(data_k = +1) = 1 / (1 + exp(-g * F_k))
```

Each data thing leans toward the sign of the summed bit-counters of the activated hard locations.

The result of the read is, for each position, the sign of the mean over the samples. A mean of exactly zero keeps
the read-address's value.

**`mode: :settle`** Gibbs-samples the hard location and data things together with the address things held at the read-address.
Hard locations and data start at random values, and each sweep updates all of them once in a fresh random order. The
data things now pull back on the hard locations through the same symmetric pulls:

```text
input(location m) = (t - d_m) / (2w) + (g / 4) * sum_k J_mk z_k
```

A hard location's input adds the data's agreement with its bit-counters to the address term.

The first `burn` sweeps are discarded, then `samples` sweeps are counted, and the result is the sign of the mean
data value, a zero mean keeping the read-address's value. This feedback is the difference between SDM and a settle
machine: SDM's read is a one-way pass. The lane report measured the feedback term at about 40 against an address
term of about 1, and at softness 0.25 and a activated fraction of 0.05 the settle read recalled 0 of 420 read-addresses where
the pass recalled 76.

Both modes use the machine in the notes and the run's generator. They do not read held values, and they do not
see pulls you have added to the memory's things.

4. It reports. The result is compared with every written pattern by the overlap:

   ```text
   q = (1 / size) * sum_k z_k p_k
   ```

   `q` is +1 when the result equals the pattern and near 0 when they are unrelated.

   The patterns are sorted by overlap, largest first (sign included). After each round the largest overlap is
   recorded. The verdict names the first pattern when its overlap is at least +0.9. If that pattern is a text, a
   second line decodes the text from the result.
5. It replaces the run's last arrangement with a list that holds the result on the address things and on the
   data things, and 0 on every other thing of the model, the hard location things included.

The read does not change the model.

**Output:**

```text
read :<name> from <source> (<pass|settle>, softness <softness>, <rounds> rounds, best overlap by round <o_1> <o_2> ...): <top three>  <verdict>
```

- `<source>` is `read-address :<read-address> with <address-noise x 100>% address-noise` (rounded to a whole percent) or `pure noise`.
- `<softness>` is the declared softness, printed as written (`0`, `0.25`, `2`).
- `<o_1> <o_2> ...` is the largest overlap after each round, signed with two decimals; empty when nothing is
  written.
- `<top three>` is up to three entries `:<pattern> <overlap>`, separated by two spaces.
- `<verdict>` is `-> :<pattern>`, `-> nothing clear (closest :<pattern> at <overlap>)`, or
  `-> nothing is written`.

When the verdict names a text, a second line follows:

```text
  text: "<text>"
```

**Example:** a pattern, a text, a settle read and a read from noise.

```settle example=softsdm-read
# A soft Kanerva memory: 128 address things, 1,000 hard location p-bits, 128 data things.
model :mind do
  softsdm :s, word-size: 128, hard-locations: 1_000, activation-probability: 0.05, softness: 0.25   # about 5% of hard locations are activated
  s.write :cat                                   # a random pattern
  s.write :dog
  s.write :owl
  s.write :note, "at nine"                       # 7 bytes of text, masked by the name
end

run :mind do
  s.read read-address: :cat, address-noise: 0.2, seed: 1                 # the one-way pass, three rounds
  s.read read-address: :note, address-noise: 0.2, seed: 2                # text comes back letter for letter
  s.read read-address: :dog, address-noise: 0.2, mode: :settle, seed: 3  # the joint settle with feedback
  s.read seed: 4                                         # from pure noise
end
```

Output:

```text output=softsdm-read
read :s from read-address :cat with 20% address-noise (pass, softness 0.25, 3 rounds, best overlap by round +1.00 +1.00 +1.00): :cat +1.00  :note +0.06  :dog -0.08  -> :cat
read :s from read-address :note with 20% address-noise (pass, softness 0.25, 3 rounds, best overlap by round +1.00 +1.00 +1.00): :note +1.00  :cat +0.06  :dog -0.11  -> :note
  text: "at nine"
read :s from read-address :dog with 20% address-noise (settle, softness 0.25, 3 rounds, best overlap by round +0.83 +0.83 +0.83): :dog +0.83  :owl +0.19  :cat +0.09  -> nothing clear (closest :dog at +0.83)
read :s from pure noise (pass, softness 0.25, 3 rounds, best overlap by round +0.55 +0.55 +0.55): :dog +0.55  :owl +0.47  :cat +0.38  -> nothing clear (closest :dog at +0.55)
```

The settle read settles to a blend near `:dog` rather than to `:dog` itself, and the read from noise settles to a
blend of the stored patterns.

**Example:** the same six patterns at softness 0 and softness 2.

```settle example=softsdm-hard
# Softness 0 is classic hard SDM: a hard location is activated exactly when the read-address is within the activation radius.
# Softness 2 blurs the edge of that ball, so far hard locations are activated too and the vote gets noisy.
model :hard do
  softsdm :s, word-size: 128, hard-locations: 1_000, softness: 0
  s.write :cat
  s.write :dog
  s.write :owl
  s.write :fox
  s.write :bee
  s.write :elk
end

model :blurred do
  softsdm :s, word-size: 128, hard-locations: 1_000, softness: 2   # same activated count on average, softer edge
  s.write :cat
  s.write :dog
  s.write :owl
  s.write :fox
  s.write :bee
  s.write :elk
end

run :hard do
  s.read read-address: :cat, address-noise: 0.2, seed: 1
end

run :blurred do
  s.read read-address: :cat, address-noise: 0.2, seed: 1
end
```

Output:

```text output=softsdm-hard
read :s from read-address :cat with 20% address-noise (pass, softness 0, 3 rounds, best overlap by round +1.00 +1.00 +1.00): :cat +1.00  :bee -0.02  :fox -0.06  -> :cat
read :s from read-address :cat with 20% address-noise (pass, softness 2, 3 rounds, best overlap by round +0.39 +0.47 +0.44): :fox +0.44  :bee +0.33  :cat +0.31  -> nothing clear (closest :fox at +0.44)
```

**Errors:**

- `address-noise is a fraction between 0 and 1`
- `read-address: takes a symbol, like read-address: :cat`
- `mode: is :pass or :settle`
- `` read does not take `<key>:` ``
- `a number was expected`

## `name.attend`

**Block:** run.

**Form:**

```text
name.attend read-address: :cat, address-noise: 0.3, rounds: 3, seed: 1
```

**Arguments:**

| Argument | Type | Default | Meaning |
|---|---|---|---|
| `name` | the name of a declared softsdm, written without a colon | required | which memory to use |
| `read-address:` | symbol | none | start from the pattern written under this name; without it, from random values |
| `address-noise:` | number from 0 to 1 | `0.3` | the chance that each value of the read-address is flipped |
| `rounds:` | whole number | `3` | how many times each read is applied, each fed its own last result; 0 counts as 1 |
| `seed:` | whole number | the run's generator | replace the run's random generator with one seeded by this number |

**What it does:** builds one read-address as `name.read` does, then runs three reads from it, each iterated `rounds`
times. None of them samples. They show what the sampled read approaches in three limits.

- **Mean-field read of this machine.** The read with infinitely many samples and data at zero temperature:
  each position takes the sign of `sum_m phi(d_m) J_mk`, the bit-counters weighted by each hard location's activated
  probability. It uses this machine's own addresses and bit-counters.
- **Kernel read, infinitely many hard locations.** The bit-counters of infinitely many random hard locations sum to a vote of
  the stored patterns, each weighted by a kernel of its distance from the read-address:

  ```text
  z_k = sign( sum over stored p of K(d(c, p)) * p_k )
  K(d) = sum over a, b of Bin(a; n - d, 1/2) Bin(b; d, 1/2) phi(a + b) phi(a + d - b)
  ```

  `K(d)` is the chance that a random hard location is activated for both of two points `d` bits apart. Patterns nearer the
  read-address share more activated hard locations and get more weight.
- **Softmax attention.** The same vote with weights `exp(beta * cos(c, p))`, where `cos` is the overlap. `beta`
  is fitted to the kernel: a least-squares line `ln K(d) = a - c * d` over the distances from 0 to `n / 2` where
  `K(d)` is above a thousandth of `K(0)`, then `beta = c * n / 2`, because `d = n (1 - cos) / 2`. If fewer than
  two distances qualify, `beta` is infinite.

In every read a sum of exactly zero keeps the read-address's value. The kernel reads use the stored patterns, not the
bit-counters, so they ignore the randomness of the writes.

`name.attend` does not change the model or the run's last arrangement. It draws from the run's generator only
to build the read-address.

**Output:** a header, then one line per read:

```text
attend :<name> from <source> (softness <softness>, fitted softmax inverse temperature <beta> on cosine):
  <read label, padded to 40 characters> <top two>  <verdict>
```

`<beta>` has one decimal (`inf` when infinite). The labels are `mean-field read of this machine`,
`kernel read, infinitely many locations` and `softmax attention`. `<top two>` and `<verdict>` are formatted as in
`name.read`, with two patterns instead of three.

**Example:**

```settle example=softsdm-attend
# Three reads computed outside the sampler: the mean field of this machine,
# the kernel for infinitely many hard locations, and softmax attention fitted to that kernel.
model :mind do
  softsdm :s, word-size: 128, hard-locations: 1_000, softness: 0.5
  s.write :cat
  s.write :dog
  s.write :owl
end

run :mind do
  s.attend read-address: :cat, address-noise: 0.2, seed: 1   # three rounds of each read from one scrambled cat
end
```

Output:

```text output=softsdm-attend
attend :s from read-address :cat with 20% address-noise (softness 0.5, fitted softmax inverse temperature 1.5 on cosine):
  mean-field read of this machine          :cat +1.00  :dog -0.08  -> :cat
  kernel read, infinitely many locations   :cat +1.00  :dog -0.08  -> :cat
  softmax attention                        :cat +1.00  :dog -0.08  -> :cat
```

**Errors:**

- `address-noise is a fraction between 0 and 1`
- `read-address: takes a symbol, like read-address: :cat`
- `` attend does not take `<key>:` ``
- `a number was expected`

## Notes

- The model's notes hold every counter as a number, `locations x word-size` of them, beside the same bit-counters in
  the pulls. Every statement that uses the machine rebuilds it from the notes, addresses included.
- The kernel in `name.attend` is an exact double sum for every distance from 0 to `word-size`, so it costs about
  `size^3 / 6` activated-probability evaluations. At size 4096 that is slow.
- `name.read` sets the hard location things to 0 in the run's last arrangement. A statement that reads the last
  arrangement after a softsdm read sees 0 there, which is neither yes nor no.
