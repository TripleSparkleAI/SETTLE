# The sdm family

The sdm family adds Kanerva's sparse distributed memory (SDM, 1988). An SDM stores patterns of +1 and -1 values
in a large set of hidden "hard locations", and gets a pattern back from a noisy copy of it. In this family the
memory's bit-counters are SETTLE pulls, so the memory is part of the model: its bit-counters can be inspected, exported
and read by a settle of the pulls.

The source is `src/sdm.rs`. The keyed text helpers are shared with the memory family and live in
`src/memory.rs`. The family was measured against the Hopfield memory, with fake valleys, fade and keys, in
`experiments/thermosim/runs/sdmkeys/REPORT_SDMKEYS.md`. At 2,000 hard locations and word-size 256 that report found the
address read holds 40 patterns at 10% address-noise (90% recall) and the pulls read 100. For much larger stores see the
[sdmscale](sdmscale.md) family, and for a soft cut-off see [softsdm](softsdm.md).

| Statement | Block | Summary |
|---|---|---|
| [`sdm`](#sdm) | model | declare a sparse distributed memory |
| [`name.write`](#namewrite) | both | write a random pattern, a text, or a text under a key |
| [`name.read`](#nameread) | run | read from a noisy read-address, a key or noise, and report what came back |

## How a sparse distributed memory works

A pattern is a list of `word-size` values, each +1 or -1. The memory has `hard-locations` hard locations. Each hard location
has two parts:

- an **address**: a fixed random pattern of `word-size` values, drawn once when the memory is declared and never
  changed;
- a row of **bit-counters**: one number for each of the `word-size` positions, all zero at the start.

The distance between two patterns is the Hamming distance: the number of positions where they differ. A
hard location is **activated** for a pattern when its address is within the **activation radius** of that pattern:

```text
awake(z) = { i : hamming(a_i, z) <= radius }
```

The hard locations whose address is at most `activation-radius` bits away from `z` wake.

For a random pattern, the expected fraction of hard locations that wake is `P[Binomial(word-size, 1/2) <= radius]`. The
default activation radius is the smallest one whose expected fraction is at least 2%. At word-size 128 it is 52 (2.1% wake), and at
word-size 256 it is 112 (2.6% wake).

**Writing** a pattern `p` adds it to the bit-counters of every hard location activated by `p`:

```text
C_i <- C_i + p        for every i in awake(p)
```

Each activated hard location's bit-counters move one step toward the pattern, position by position.

**Reading** from a read-address `z` lets the hard locations activated by `z` vote, position by position:

```text
z_j <- sign( sum over i in awake(z) of C_ij )        a zero sum keeps the old z_j
```

Each position takes the sign of the summed bit-counters of the activated hard locations.

A read-address near a stored pattern wakes many of the same hard locations that the pattern woke when it was written, so their
bit-counters carry that pattern and outvote the rest. The read is **iterated**: the result becomes the next read-address, and
the read repeats until the result stops changing (a fixed point) or the iteration limit is reached. Each round
brings the state closer to the stored pattern, as long as the first read-address was close enough.

A hard location's bit-counters can hold several patterns. The patterns that share a hard location add up there, and the other
patterns act as noise in the vote. That noise is what limits how many patterns a memory holds.

Unlike the Hopfield memory of the [memory](memory.md) family, an SDM has no mirror images: the flipped pattern
wakes different hard locations, so it does not come back.

## Where the memory keeps its state

`sdm :s` adds two blocks of things at the end of the model:

- data things `s_0` to `s_<word-size - 1>`,
- hard location things `s_loc_0` to `s_loc_<hard-locations - 1>`.

Every hard location thing is joined to every data thing by a pull. The bit-counters are kept in these pulls and nowhere
else:

```text
pull(s_loc_i, s_j) = C_ij * gain / 2,    gain = 4 / size
```

The pull between hard location `i` and data thing `j` is the counter times `2 / size`.

Each data thing leans by the sum of its pulls to the hard locations, and each hard location thing leans by
`-0.8` (that is `-gain * 0.4 * size / 2`). These leans make the pulls read below work.

The addresses are not stored. They are drawn again from the memory's name and seed each time they are needed,
with a generator seeded by the FNV-1a hash of `sdm-addresses:<name>:<seed>`. The memory's settings and the list
of what was written are kept in the model's notes under the key `sdm:<name>`. For plain text the note holds the
text. For keyed text it holds only a marker, so the text itself is never written anywhere in the model.

## `sdm`

**Block:** model.

**Form:**

```text
sdm :name, word-size: 256, hard-locations: 2000, activation-radius: 112, seed: 1, fade: 1
```

**Arguments:**

| Argument | Type | Default | Meaning |
|---|---|---|---|
| `:name` | symbol | required | the memory's name, used as `name.write` and `name.read` |
| `word-size:` | whole number, 16 to 4096 | `256` | how many data things, the length of every pattern |
| `hard-locations:` | whole number, 1 to 100,000 | `2000` | how many hard locations |
| `activation-radius:` | whole number, at most `word-size` | the activation radius that wakes about 2% of hard locations (112 at word-size 256) | the activation radius for waking a hard location |
| `seed:` | whole number | `1` | the seed of the random addresses |
| `fade:` | number, above 0 and at most 1 | `1` | every counter is multiplied by this before each write |

`word-size` times `hard-locations` must be at most 20,000,000, because every counter is a pull. The comma after `:name`
is optional. Numbers given for `word-size:`, `hard-locations:`, `activation-radius:` and `seed:` are cut to whole numbers.

**What it does:** adds `word-size` data things and `hard-locations` hard location things to the model, with a zero pull between
every hard location and every data thing, a lean of zero on each data thing and a lean of `-0.8` on each hard location. It
records the memory in the notes under `sdm:<name>`. Nothing is written yet.

The default activation radius is the smallest `r` with `P[Binomial(word-size, 1/2) <= r] >= 0.02`. The `activation-radius:` shown in
`settle --help` (112) is this default at word-size 256 only.

With `fade:` below 1, every write first multiplies every counter of the memory by the fade, and lowers each data
thing's lean to match. Each write therefore weakens all older patterns, and the oldest are lost first.

**Output:** none.

**Example:** see [`name.read`](#nameread) and the fade example below.

```settle example=sdm-fade
# Fade: every write first multiplies all bit-counters by the fade, so older patterns weaken.
# Two memories hold the same five patterns, one with a gentle fade and one with a strong fade.
model :gentle do
  sdm :s, word-size: 128, hard-locations: 1_000, fade: 0.9   # the oldest pattern keeps 0.9^4 = 0.66 of its weight
  s.write :first
  s.write :second
  s.write :third
  s.write :fourth
  s.write :fifth
end

model :strong do
  sdm :s, word-size: 128, hard-locations: 1_000, fade: 0.5   # the oldest pattern keeps 0.5^4 = 0.06 of its weight
  s.write :first
  s.write :second
  s.write :third
  s.write :fourth
  s.write :fifth
end

run :gentle do
  s.read read-address: :first, address-noise: 0.1, seed: 1   # the oldest pattern still comes back
end

run :strong do
  s.read read-address: :first, address-noise: 0.1, seed: 1   # the newest pattern outvotes the oldest
end
```

Output:

```text output=sdm-fade
read :s from read-address :first with 10% address-noise via addresses (2 iterated reads, 22 of 1000 hard locations activated): :first +1.00  :fifth +0.16  :fourth +0.14  -> :first
read :s from read-address :first with 10% address-noise via addresses (2 iterated reads, 32 of 1000 hard locations activated): :fifth +1.00  :first +0.16  :second -0.09  -> :fifth
```

**Errors:**

- `sdm :<name> is already declared`
- `` sdm does not take `<key>:` ``
- `sdm word-size must be between 16 and 4096`
- `sdm hard-locations must be at least 1, and word-size x hard-locations at most 20 million`
- `fade must be above 0 and at most 1`
- `activation-radius cannot be larger than word-size`
- `a number was expected`

## `name.write`

**Block:** both. In a run block it changes the model, and the change stays for later run blocks.

**Form:**

```text
name.write :cat
name.write :note, "text"
name.write :diary, "text", key: "secret"
```

**Arguments:**

| Argument | Type | Default | Meaning |
|---|---|---|---|
| `name` | the name of a declared sdm, written without a colon | required | which memory to write to |
| `:cat` | symbol | required | the name of what is written |
| `"text"` | string | none | text to write |
| `key:` | string | no key | write the text under this key; needs `"text"` |

**What it does:** builds a pattern of `word-size` values and writes it with the rule above: if `fade:` is below 1,
every counter is first multiplied by the fade; then every hard location activated by the pattern adds the pattern to its
bit-counters. Each data thing's lean is kept equal to the sum of its pulls.

The pattern depends on the form:

- **A symbol only.** The name's random pattern: `word-size` values of +1 or -1 from a generator seeded by the FNV-1a
  hash of the name. The same name always gives the same pattern, in every memory of that size.
- **Text.** The text's bytes (UTF-8) become bits, eight per byte, most significant bit first, with 1 as +1 and 0
  as -1. Each bit is multiplied by the matching value of the name's random pattern, and the positions after the
  text keep the random pattern's values. The text needs `8 * bytes` data things, at most `word-size`. The text is kept
  in the notes so that `name.read` can decode it.
- **Text with a key.** One byte holding the text's length, then the text's bytes, then +1 on every remaining
  position. That list is turned by the key: each value is multiplied by a sign from the random pattern of
  `mask:<key>`, then the values are moved by a shuffle seeded by the hash of `turn:<key>`. This is the same keyed
  pattern the memory family's `name.save ..., key:` builds. The text can hold at most `size / 8 - 1` bytes, and
  never more than 255. Only a marker is kept in the notes; the text is not.

A key is not encryption. It is hashed to 64 bits and the bit-counters are visible to anyone who holds the model. The
measurements of what a key protects are in `experiments/thermosim/runs/sdmkeys/REPORT_SDMKEYS.md`.

The line is claimed by this family whenever `name` is not a declared [softsdm](softsdm.md), so writing to a name
that is neither gives the error `no sdm :<name> (...)`.

**Output:** nothing, unless no hard location is activated by the pattern. Then nothing is written, the name is still
recorded as written, and the statement prints:

```text
warning: no location of :<name> is within radius <radius> of :<what>, so nothing was written
```

**Example:** see [`name.read`](#nameread) for writes of all three forms. A name can be written only once:

```settle example=sdm-already-written
# A name can be written only once in a memory.
model :mind do
  sdm :s, word-size: 64, hard-locations: 200
  s.write :cat
  s.write :cat   # refused: :cat is already in :s
end
```

```text error=sdm-already-written
line 5: :cat is already written in :s
```

**Errors:**

- `no sdm :<name> (declare it with: sdm :<name>, word-size: 256, hard-locations: 2000)`
- `:<what> is already written in :<name>`
- `<bytes> bytes of text need <bits> things; sdm :<name> has <word-size>`
- `keyed text of <bytes> bytes is too long; sdm :<name> holds at most <capacity>`
- `` write does not take `<key>:` ``
- `a "quoted" string was expected`

## `name.read`

**Block:** run.

**Form:**

```text
name.read read-address: :cat, address-noise: 0.3, iterated-reads: 10, via: :addresses, seed: 1
name.read key: "secret", address-noise: 0, iterated-reads: 10, via: :addresses, seed: 1
name.read iterated-reads: 10, via: :addresses, seed: 1
```

**Arguments:**

| Argument | Type | Default | Meaning |
|---|---|---|---|
| `name` | the name of a declared sdm, written without a colon | required | which memory to read |
| `read-address:` | symbol | none | start from the pattern written under this name |
| `key:` | string | none | start from the key's read-address, and read keyed text with this key |
| `address-noise:` | number, a chance from 0 to 1 (not checked) | `0.3` with `read-address:`, `0` with `key:` | the chance that each value of the starting pattern is flipped |
| `iterated-reads:` | whole number | `10` | the most rounds to read; 0 counts as 1 |
| `via:` | one of `:addresses` / `:pulls` | `:addresses` | how the read decides which hard locations wake |
| `seed:` | whole number | the run's generator | replace the run's random generator with one seeded by this number |

`read-address:` and `key:` cannot be given together. With neither, the read starts from random values, which the output
calls "pure noise". `address-noise:` has no effect without `read-address:` or `key:`.

**What it does:**

1. If `seed:` is given, the run's random generator is replaced by one seeded with it, and kept for the rest of
   the run block.
2. It builds the starting pattern:
   - with `read-address: :cat`, the pattern written under `:cat`. For a keyed name, or a name that was never written, it
     is the name's plain random pattern. Each value is then flipped with chance `damage`.
   - with `key:`, the key's read-address: the all-+1 list turned by the key. It agrees with the stored keyed pattern on
     every padding position. Each value is flipped with chance `damage`.
   - with neither, `word-size` random values.

   Each of these uses one draw of the run's generator per value.
3. It reads, in one of two ways.

**`via: :addresses`** is Kanerva's read, described above. In each round the hard locations activated by the current
state vote, and each position takes the sign of the sum of their bit-counters; a zero sum keeps the old value. It
repeats until a round changes nothing, or `iterations` rounds are done. A round that changes nothing is counted.
This read is not a settling process. It picks hard locations with the address matrix and reads with the counter
matrix, and one symmetric set of pulls cannot hold two matrices. So the addresses stay outside the pulls, and
only the bit-counters are pulls.

**`via: :pulls`** is the zero-temperature settle of the energy the pulls define. The addresses are not used
(they are used only to write). A hard location wakes when its bit-counters agree with the current state by more than
`0.4 * size`:

```text
location i wakes  when  sum_j C_ij z_j > 0.4 * size
z_j <- sign( sum over woken i of C_ij )        a zero sum keeps the old z_j
```

A hard location wakes because of what it holds, not where its address sits. Then the data take the vote.

In the model these are the signs of each thing's input from its lean and pulls. The read first gives every
free thing of the model a random value and every held thing its held value (one draw per thing), then puts the
starting pattern on the data things. Each round sets every free hard location thing to the sign of its input (a zero
input gives -1), then every free data thing to the sign of its input (a zero input keeps the old value). It stops
when a round leaves the data unchanged, or after `iterations` rounds. Pulls from other things of the model to
these things count too. A held hard location thing is not updated. A held data thing keeps its value from the starting
pattern, not its held value. The final arrangement of the whole model becomes the run's last arrangement. The
address read does not change the last arrangement.

This is a different memory from the address read: it lowers the energy at each step, but it wakes hard locations by
content. In the sdmkeys measurements it held more patterns at 10% and 20% address-noise and none at 30%.

4. It reports. With `read-address:` or no read-address, it compares the result with every written pattern that is not keyed, by the
   overlap:

   ```text
   q = (1 / size) * sum_j z_j p_j
   ```

   `q` is +1 when the result equals the pattern, and near 0 when they are unrelated.

   The patterns are sorted by the size of the overlap, ignoring its sign, and the first three are printed. The
   verdict names the first pattern when its overlap is at least +0.9. If that pattern is a plain text, a second
   line decodes the text from the result.

   With `key:`, the result is turned back with the key and read. For each sign (all values as read, then all
   flipped), the first byte gives the text's length, and the padding after the text is checked: at least 32
   padding positions must remain and at least 85% of them must read +1. If one sign passes, the text is printed.
   A wrong key turns the result into noise, whose padding reads +1 only about half the time, so it fails.

The read does not change the model.

**Output:** with `read-address:` or no read-address:

```text
read :<name> from <source> via <addresses|pulls> (<rounds> iterated reads, <activated> of <hard-locations> hard locations activated): <top three>  <verdict>
```

- `<source>` is `read-address :<read-address> with <address-noise x 100>% address-noise` (rounded to a whole percent) or `pure noise`.
- `<rounds>` is the number of iterated reads done. `<activated>` is the number of hard locations activated in the last round.
- `<top three>` is up to three entries `:<pattern> <overlap>`, the overlap signed with two decimals, separated
  by two spaces.
- `<verdict>` is one of `-> :<pattern>`, `-> nothing clear (closest :<pattern> at <overlap>)`, or
  `-> nothing is written` when no plain pattern has been written (the top three is then empty).

When the verdict names a plain text, a second line follows:

```text
  text: "<text>"
```

Control characters in the decoded text are printed as `?`.

With `key:`:

```text
read :<name> from a key via <addresses|pulls> (<rounds> iterated reads, <activated> of <hard-locations> hard locations activated): text "<text>"
read :<name> from a key via <addresses|pulls> (<rounds> iterated reads, <activated> of <hard-locations> hard locations activated): nothing readable
```

**Example:** random patterns, read by addresses, by pulls, from a read-address never written, and from noise.

```settle example=sdm-read
# A Kanerva memory of 128 data things and 1,000 hard locations.
model :mind do
  sdm :s, word-size: 128, hard-locations: 1_000   # the activation radius defaults to the one that wakes about 2% of hard locations
  s.write :cat                          # three random patterns, each named by a symbol
  s.write :dog
  s.write :owl
end

run :mind do
  s.read read-address: :cat, address-noise: 0.1, seed: 1                # Kanerva's read by addresses, from cat at 10% address-noise
  s.read read-address: :dog, address-noise: 0.1, via: :pulls, seed: 2   # the zero-temperature read of the pulls
  s.read read-address: :zebra, address-noise: 0, seed: 3                # a pattern that was never written
  s.read seed: 4                                        # start from pure noise
end
```

Output:

```text output=sdm-read
read :s from read-address :cat with 10% address-noise via addresses (2 iterated reads, 22 of 1000 hard locations activated): :cat +1.00  :owl -0.16  :dog -0.08  -> :cat
read :s from read-address :dog with 10% address-noise via pulls (2 iterated reads, 26 of 1000 hard locations activated): :dog +1.00  :cat -0.08  :owl +0.02  -> :dog
read :s from read-address :zebra with 0% address-noise via addresses (1 iterated reads, 13 of 1000 hard locations activated): :cat -0.08  :dog -0.06  :owl -0.02  -> nothing clear (closest :cat at -0.08)
read :s from pure noise via addresses (2 iterated reads, 26 of 1000 hard locations activated): :owl +1.00  :cat -0.16  :dog +0.02  -> :owl
```

The `:zebra` read stops after one round: the 13 hard locations it wakes hold no bit-counters, every sum is zero, and every
value is kept. The read from noise happened to fall into `:owl`.

**Example:** text in the open and text under a key.

```settle example=sdm-text
# Text in the open and text under a key, in one Kanerva memory.
model :mind do
  sdm :s, word-size: 128, hard-locations: 1_000
  s.write :cat                                     # a random pattern
  s.write :note, "at nine"                         # 7 bytes = 56 bits, masked by the name :note
  s.write :diary, "the mat", key: "blue heron"     # turned by the key; the text is not kept in the model
end

run :mind do
  s.read read-address: :note, address-noise: 0.1, seed: 1   # recall the note and print its text
  s.read key: "blue heron"                  # start from the key's read-address and read the keyed text
  s.read key: "red heron"                   # a wrong key reads nothing
end
```

Output:

```text output=sdm-text
read :s from read-address :note with 10% address-noise via addresses (2 iterated reads, 14 of 1000 hard locations activated): :note +1.00  :cat +0.06  -> :note
  text: "at nine"
read :s from a key via addresses (2 iterated reads, 29 of 1000 hard locations activated): text "the mat"
read :s from a key via addresses (3 iterated reads, 22 of 1000 hard locations activated): nothing readable
```

**Errors:**

- `read takes read-address: or key:, not both`
- `read-address: takes a symbol, like read-address: :cat`
- `via: takes :addresses or :pulls`
- `` read does not take `<key>:` ``
- `a number was expected`
- `a "quoted" string was expected`

## Notes

- A keyed text is accepted by `name.write` up to `size / 8 - 1` bytes, but `name.read key:` can read it only
  when at least 32 padding positions remain after it: `8 * (bytes + 1) + 32 <= word-size`. At word-size 128 that is 11
  bytes, while the write accepts 15.
- `sdm :s` adds things named `s_<j>` and `s_loc_<i>` without checking whether things of those names already
  exist. Declare the memory before any thing whose name could collide.
- The pulls read follows every pull on the memory's things, so pulls you add by hand between a memory's things
  and other things change it. The address read uses only the bit-counters.
