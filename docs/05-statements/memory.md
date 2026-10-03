# The memory family

The memory family stores patterns in the pulls between a block of free things and gets them back by shaking.
A memory holds no values. Each stored pattern changes how strongly every pair of the memory's things pulls or
pushes, so that the pattern and its mirror image (every bit flipped) become calm arrangements of the block. To
recall, a run starts the block from a noisy read-address and shakes it at a low temperature, and the block rolls down
into the nearest calm arrangement. This is a Hopfield network with Hebbian storage. A memory can also hold short
text, either in the open or under a key.

The source is `src/memory.rs`. The family has no lane report of its own. The Hopfield store in this file is
measured against Kanerva's sparse distributed memory, and the keyed save and recall are measured, in
`experiments/thermosim/runs/sdmkeys/REPORT_SDMKEYS.md`.

| Statement | Block | Summary |
|---|---|---|
| [`memory`](#memory) | model | declare a memory: a block of free things |
| [`name.remember`](#nameremember) | both | store a random pattern named by a symbol |
| [`name.save`](#namesave) | both | store text, in the open or under a key |
| [`name.recall`](#namerecall) | run | shake from a read-address, a key or noise, and report what the memory settled on |

## How a pattern is stored

A pattern `p` is a list of `size` values, each +1 (yes) or -1 (no). Storing it changes the pulls inside the
memory's block of things:

```text
J_ik <- f * J_ik + p_i * p_k / n        for every pair i != k inside the block
```

Every existing pull between two of the memory's things is first weakened by the fade `f`, then the pair gains
`p_i p_k / n`, where `n` is the memory's size. Pairs that agree in the pattern are pulled together and pairs that
disagree are pushed apart.

With `f = 1` (the default) nothing fades and the pulls are the sum over all stored patterns. With `f` below 1,
every new pattern weakens all older ones, so the oldest memories are lost first. The fade applies to every pull
inside the block, including pulls you set yourself between two of the memory's things. Pulls between a memory
thing and a thing outside the block, and all leans, are never changed.

A pattern and its mirror image have the same energy, so recall can land on either. `name.recall` reports which.

How close an arrangement `s` of the block is to a pattern `p` is measured by the overlap:

```text
q = (1 / n) * sum_i s_i p_i
```

It is +1 when `s` equals `p`, -1 when `s` is the mirror image of `p`, and near 0 when they are unrelated.

## Where a memory keeps its state

A memory records itself in the model's notes under the key `memory:<name>`: the index of its first thing, its
size and its fade, and one entry per stored pattern. For a plain text save the entry holds the text itself. For a
keyed save it holds only a marker, so keyed text is never written anywhere in the model. The `valleys` family
reads these notes to label valleys as stored patterns or mirrors (see [valleys](valleys.md)).

## `memory`

**Block:** model.

**Form:**

```text
memory :name, size: 256, fade: 1
```

**Arguments:**

| Argument | Type | Default | Meaning |
|---|---|---|---|
| `:name` | symbol | required | the memory's name, used as `name.remember`, `name.save` and `name.recall` |
| `size:` | whole number, 8 to 4096 | `256` | how many things the memory has |
| `fade:` | number, above 0 and at most 1 | `1` | the factor every older pull is multiplied by when a new pattern is stored; 1 means nothing fades |

**What it does:** adds `size` free things named `name_0` to `name_<size - 1>`, with no leans and no pulls, at the
end of the model. It records the memory in the notes under `memory:<name>`. Nothing is stored yet.

A number given for `size:` is cut to a whole number. The comma after `:name` is optional.

**Output:** none.

**Example:** see [`name.remember`](#nameremember).

**Errors:**

- `memory :<name> is already declared`
- `memory size must be between 8 and 4096`
- `fade must be above 0 and at most 1 (1 means memories never fade)`
- `` memory does not take `<key>:` ``
- `a number was expected`

## `name.remember`

**Block:** both. In a run block it changes the model, so later runs of the model see the new pattern too.

**Form:**

```text
name.remember :pattern
```

**Arguments:**

| Argument | Type | Default | Meaning |
|---|---|---|---|
| `name` | the name of a declared memory, written without a colon | required | which memory to store in |
| `:pattern` | symbol | required | the pattern's name |

**What it does:** stores the random pattern that belongs to the symbol `:pattern`, using the rule above. The
pattern is made from the name alone: the name is hashed with 64-bit FNV-1a, and that hash seeds a xorshift
generator that draws each value as +1 or -1 with equal chance. The same name therefore gives the same pattern in
every program and every run, and the run's seed does not affect it.

**Output:** none.

**Example:** three patterns in 256 things, recalled from read-addresses with different address-noise.

```settle example=memory-recall
# three random patterns stored in the pulls of 256 things, then recalled from noisy read-addresses
model :mind do
  memory :m, size: 256          # adds 256 free things m_0 .. m_255
  m.remember :cat               # a random pattern named :cat
  m.remember :dog
  m.remember :owl
end

run :mind do
  m.recall read-address: :cat, address-noise: 0.3, seed: 1    # 30% of :cat's bits flipped, then 30 sweeps at temperature 0.1
  m.recall read-address: :dog, address-noise: 0.45, seed: 2   # a heavier scramble: this read-address rolls into :owl
  m.recall read-address: :zebra, address-noise: 0, seed: 3    # never stored: its pattern is not a valley
  m.recall seed: 4                            # start from pure noise
end
```

Output:

```text output=memory-recall
recall :m from read-address :cat with 30% address-noise after 30 sweeps: :cat +1.00  :owl -0.09  :dog -0.07  -> :cat
recall :m from read-address :dog with 45% address-noise after 30 sweeps: :owl +1.00  :cat -0.09  :dog +0.02  -> :owl
recall :m from read-address :zebra with 0% address-noise after 30 sweeps: :cat -1.00  :owl +0.09  :dog +0.07  -> :cat (its mirror image)
recall :m from pure noise after 30 sweeps: :dog +0.52  :owl +0.50  :cat +0.41  -> nothing clear (closest :dog at +0.52)
```

The second read-address, with 45% of its bits flipped, lands on `:owl` instead of `:dog`. The third read-address, `:zebra`, was
never stored: its pattern is not a calm arrangement, so the block rolls away from it into the mirror image of
`:cat`. The last recall starts from noise and finds no single memory.

**Example:** with a fade, each new pattern weakens the older ones.

```settle example=memory-fade
# with fade, each new memory weakens the older ones, so the oldest of twelve is lost
model :fading do
  memory :m, size: 256, fade: 0.6
  m.remember :p0
  m.remember :p1
  m.remember :p2
  m.remember :p3
  m.remember :p4
  m.remember :p5
  m.remember :p6
  m.remember :p7
  m.remember :p8
  m.remember :p9
  m.remember :p10
  m.remember :p11
end

run :fading do
  m.recall read-address: :p11, address-noise: 0.3, seed: 1   # the newest is found
  m.recall read-address: :p0, address-noise: 0.3, seed: 1    # the oldest is not
  m.remember :p12                            # storing inside a run changes the model too
  m.recall read-address: :p12, address-noise: 0.3, seed: 1
end
```

Output:

```text output=memory-fade
recall :m from read-address :p11 with 30% address-noise after 30 sweeps: :p11 +1.00  :p2 -0.09  :p6 +0.05  -> :p11
recall :m from read-address :p0 with 30% address-noise after 30 sweeps: :p11 -1.00  :p2 +0.09  :p6 -0.05  -> :p11 (its mirror image)
recall :m from read-address :p12 with 30% address-noise after 30 sweeps: :p12 +1.00  :p9 +0.15  :p1 -0.09  -> :p12
```

**Errors:**

- `no memory :<name> (declare it with: memory :<name>, size: 256)`
- `:<pattern> is already stored in :<name>`

## `name.save`

**Block:** both. In a run block it changes the model.

**Form:**

```text
name.save :note, "text"
name.save :note, "text", key: "secret"
```

**Arguments:**

| Argument | Type | Default | Meaning |
|---|---|---|---|
| `name` | the name of a declared memory, written without a colon | required | which memory to store in |
| `:note` | symbol | required | the note's name |
| `"text"` | string | required | the text to store |
| `key:` | string | no key | store the text under this key |

**What it does without a key:** the text's bytes (UTF-8) become bits, eight per byte, most significant bit first,
with 1 as +1 and 0 as -1. Each bit is multiplied by the matching value of the note name's random pattern (the
same pattern `name.remember :note` would store), and the things after the text keep that random pattern's values.
The result is stored with the rule above. Multiplying by the random pattern makes the text look like random bits,
which this memory stores best, because patterns that resemble each other interfere. The text needs `8 * bytes`
things, at most the memory's size.

The text is also written into the model's notes, so `name.recall` can report its length and decode it.

**What it does with a key:** the stored pattern is built from three parts in this order: one byte holding the
text's length, the text's bytes, and +1 on every remaining thing. That list is then turned by the key: every value
is multiplied by a sign from a mask, and the values are moved to other things by a shuffle. The mask is the random
pattern of the name `mask:<key>`. The shuffle is a Fisher-Yates shuffle driven by a xorshift generator seeded
with the FNV-1a hash of `turn:<key>`. The turned pattern is stored with the rule above.

The text is not written into the notes. Only a marker is kept, which records that `:note` is a keyed save. A keyed
text can hold at most `size / 8 - 1` bytes, and never more than 255.

A key is not encryption. It is hashed to 64 bits, the landscape of pulls is visible to anyone who holds the model,
and a guessed key can be checked offline against the padding. The measurements of what a key protects and what it
does not are in `experiments/thermosim/runs/sdmkeys/REPORT_SDMKEYS.md`.

**Output:** none.

**Example:** plain text comes back letter for letter from a noisy read-address.

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

Output:

```text output=memory-text
recall :m from read-address :note with 25% address-noise after 30 sweeps: :note +1.00  :dog +0.03  :cat +0.02  -> :note
  text: "meet at the harbour at nine"
```

**Example:** a text that does not fit.

```settle example=memory-text-too-long
# a plain text note needs 8 things per byte; 9 bytes do not fit in 64 things
model :mind do
  memory :m, size: 64
  m.save :note, "nine byte"
end
```

```text error=memory-text-too-long
line 4: 9 bytes of text need 72 things; memory :m has 64
```

**Errors:**

- `<bytes> bytes of text need <things> things; memory :<name> has <size>`
- `keyed text of <bytes> bytes is too long; memory :<name> holds at most <capacity>`
- `:<note> is already stored in :<name>`
- `` save does not take `<key>:` ``
- `` save with a trailing option needs `key: "..."` ``
- `no memory :<name> (declare it with: memory :<name>, size: 256)`
- `a "quoted" string was expected`

## `name.recall`

**Block:** run.

**Form:**

```text
name.recall read-address: :pattern, address-noise: 0.3, sweeps: 30, temperature: 0.1, seed: 1
name.recall key: "secret", address-noise: 0, sweeps: 30, temperature: 0.1, seed: 1
name.recall sweeps: 30, temperature: 0.1, seed: 1
```

**Arguments:**

| Argument | Type | Default | Meaning |
|---|---|---|---|
| `name` | the name of a declared memory, written without a colon | required | which memory to recall from |
| `read-address:` | symbol | none | start the block from this pattern |
| `key:` | string | none | start the block from the key's read-address, and read keyed text with this key |
| `address-noise:` | number, a chance from 0 to 1 (not checked) | `0.3` with `read-address:`, `0` with `key:` | the chance that each bit of the starting pattern is flipped |
| `sweeps:` | whole number | `30` | how many sweeps to shake for |
| `temperature:` | number above 0 | `0.1` | the temperature of the shaking |
| `seed:` | whole number | the run's generator | replace the run's random generator with one seeded by this number |

`read-address:` and `key:` cannot be given together. With neither, the block starts from random values, which the output
calls "pure noise". `address-noise:` has no effect without `read-address:` or `key:`.

**What it does:**

1. If `seed:` is given, the run's random generator is replaced, as `settle ... seed:` does. The new generator is
   kept for the rest of the run.
2. Every free thing of the model gets a random value, and every held thing its held value.
3. The memory's block is overwritten with the starting pattern, and each of its bits is flipped with chance
   `damage`:
   - with `read-address: :pattern`, the pattern stored under that name. For a keyed note, or for a name that was never
     stored, this is the name's plain random pattern.
   - with `key:`, the key's read-address: the all-+1 list turned by the key. It agrees with the stored keyed pattern on
     every padding bit, so it rolls into that pattern.
4. The whole model is swept `sweeps` times at the given temperature, with the same update rule as `settle`. Every
   free thing moves, not only the memory's things.
5. The final arrangement becomes the run's last arrangement. Samples, yes counts and the best arrangement of the
   run are not changed.

With `read-address:` or neither, recall then compares the block with every stored pattern that is not keyed. It sorts
them by the size of the overlap, prints the three largest, and gives a verdict: the name of the closest pattern
when the size of its overlap is at least 0.9, with "(its mirror image)" when the overlap is negative, and
"nothing clear" otherwise. If the closest pattern is a plain text note with an overlap of size at least 0.9, a
second line decodes the text from the block.

With `key:`, recall prints no scoreboard. It turns the block back with the key and tries both signs. For each
sign it reads the length byte, then checks the padding that follows the text: at least 32 padding bits must
remain and at least 85% of them must read +1. If a sign passes, it prints the text. A wrong key turns the block
into noise, so about half of its padding reads +1 and the check fails.

**Output:** with `read-address:` or neither:

```text
recall :<name> from <start> after <sweeps> sweeps: :<p1> <q1>  :<p2> <q2>  :<p3> <q3>  <verdict>
  text: "<text>"
```

`<start>` is `read-address :<pattern> with <address-noise>% address-noise` or `pure noise`. Each `<q>` is an overlap with a sign and two
decimals. `<verdict>` is one of:

```text
-> :<pattern>
-> :<pattern> (its mirror image)
-> nothing clear (closest :<pattern> at <q>)
-> nothing is stored
```

The `text:` line appears only when the verdict names a plain text note. Characters that do not decode, and
control characters, print as `?`.

With `key:`:

```text
recall :<name> with a key after <sweeps> sweeps (agreement with the key's cue <q>): text "<text>"
recall :<name> with a key after <sweeps> sweeps (agreement with the key's cue <q>): nothing readable
```

`<q>` is the overlap between the block and the key's read-address.

**Example:** keyed text. The right key finds the note and reads it. A wrong key reads nothing. A read-address with the
note's name does not find it, because a keyed pattern is not the name's public pattern.

```settle example=memory-keyed
# keyed text: the key both finds the memory and reads it; the text is not kept in the model
model :mind do
  memory :m, size: 512
  m.remember :cat
  m.save :note, "meet at the harbour at nine", key: "blue heron"
  m.remember :dog
end

run :mind do
  m.recall key: "blue heron", seed: 3   # the right key
  m.recall key: "red heron", seed: 3    # a wrong key reads nothing
  m.recall read-address: :note, seed: 3          # a keyed pattern is not public, so :note is not on the scoreboard
end
```

Output:

```text output=memory-keyed
recall :m with a key after 30 sweeps (agreement with the key's read-address +0.50): text "meet at the harbour at nine"
recall :m with a key after 30 sweeps (agreement with the key's read-address +0.01): nothing readable
recall :m from read-address :note with 30% address-noise after 30 sweeps: :dog -0.12  :cat +0.05  -> nothing clear (closest :dog at -0.12)
```

**Example:** the options that control the shaking.

```settle example=memory-options
# the recall options: how long to shake, how hot, and where the randomness starts
model :mind do
  memory :m, size: 128                                    # 128 free things m_0 .. m_127
  m.remember :cat
end

run :mind do
  m.remember :dog                                         # a run block can store too; this changes the model
  m.recall read-address: :cat, address-noise: 0.2, sweeps: 0, seed: 5     # no shaking: the noisy read-address as it is
  m.recall read-address: :cat, address-noise: 0.2, sweeps: 5, seed: 5     # 5 sweeps at the default temperature 0.1
  m.recall read-address: :cat, address-noise: 0.2, temperature: 5, seed: 5   # too hot: the pattern melts
end
```

Output:

```text output=memory-options
recall :m from read-address :cat with 20% address-noise after 0 sweeps: :cat +0.56  :dog -0.02  -> nothing clear (closest :cat at +0.56)
recall :m from read-address :cat with 20% address-noise after 5 sweeps: :cat +1.00  :dog -0.08  -> :cat
recall :m from read-address :cat with 20% address-noise after 30 sweeps: :cat +0.05  :dog +0.00  -> nothing clear (closest :cat at +0.05)
```

With no sweeps, recall reports the noisy read-address itself, which is 20% away from `:cat`. Five sweeps at the default
temperature are enough to repair it. At temperature 5 the pulls are too weak against the noise, and the block ends
up unrelated to any memory.

**Errors:**

- `no memory :<name> (declare it with: memory :<name>, size: 256)`
- `temperature must be above zero`
- `recall takes read-address: or key:, not both`
- `read-address: takes a symbol, like read-address: :cat`
- `` recall does not take `<key>:` ``
- `a number was expected`
- `a "quoted" string was expected`

## Notes

- **Capacity.** The usual estimate for a Hopfield memory of `n` things is about `0.138 n` random patterns
  (35 at `n = 256`), as quoted in `REPORT_SDMKEYS.md`.
- **A keyed note that fills the memory cannot be read.** `name.save` accepts keyed text up to `size / 8 - 1`
  bytes, but `name.recall key:` needs at least 32 padding bits after the text. A keyed text of `b` bytes is
  readable only if `size - 8 * (b + 1)` is at least 32. For example, a 7-byte keyed note in a 64-thing memory is
  stored without an error and always recalls as `nothing readable`.
- **Thing names.** `memory :m` adds things named `m_0`, `m_1`, and so on. Declare the memory before any thing
  with one of those names. If such a thing already exists, the memory's block is not laid out as the code expects
  and a later statement can stop the interpreter with an index error.
- **Held memory things.** `name.recall` writes the read-address into every thing of the block, held or not. A held thing
  in the block therefore keeps the read-address's value during the shaking, not its held value.
