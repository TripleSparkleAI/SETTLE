# The coded family

The coded family stores short text in a memory after two steps that make it safer to read back. First the text
is compressed, so it takes fewer of the memory's things. Then the compressed bits are protected by an
error-correcting code, so a few wrong things after a recall can be repaired. The coded bits are multiplied by a
random mask made from the note's name and stored as a pattern in a Hopfield `memory` or a Kanerva `sdm`. A recall
runs the steps backwards and checks the result against a 16-bit check value. If any step fails, the recall is
refused and prints no text. It never prints a guess.

The source is `src/coded.rs`. The static English model used by the compressors is built from
`data/coded_train_austen.txt` (provenance in `data/PROVENANCE.txt`). The family was measured in
`experiments/thermosim/runs/sdmcoded/REPORT_SDMCODED.md`. The memories it stores into are documented in
[memory](memory.md) and [sdm](sdm.md).

| Statement | Block | Summary |
|---|---|---|
| [`name.save_coded`](#namesave_coded) | both | compress, error-code, mask and store a text note in a memory or an sdm |
| [`name.recall_coded`](#namerecall_coded) | run | shake or read from a read-address, then decode, check and print the text or refuse |

## The pipeline

A note goes through the same steps in both directions:

```text
save:    text -> compress -> frame -> code -> mask -> store
recall:  cue -> shake or read -> unmask (both signs) -> decode -> unframe -> decompress -> check -> text or refuse
```

### The frame

The compressed bits are put in a frame of exactly `K` bits, where `K` is the number of information bits the
code carries (see [the codes](#the-codes)):

```text
[length: 8 bits][check: 16 bits][compressed payload][zeros up to K bits]
```

- The length is the text's length in bytes, so a text is at most 255 bytes.
- The check is CRC-16/CCITT-FALSE (polynomial 0x1021, start value 0xFFFF) over the original text.
- Every number is written most significant bit first.
- The frame is always padded with zeros to fill `K`, so the codeword fills the memory.

The frame fits when:

```text
24 + (payload bits) <= K
```

The 24 header bits and the payload must fit in the code's information bits. If they do not, `save_coded` stops
with an error that gives both numbers.

### The compressors (`compress:`)

The compressors use a fixed model of English built from byte counts of the training text. The model belongs to
the codec, like a dictionary. It is never stored in a memory.

- **`:none`**: 8 bits per byte, most significant bit first.
- **`:ac`** (the default): arithmetic coding (Witten, Neal and Cleary, 32-bit integer form) under an order-1 model.
  Each byte is coded with the odds of the bytes that followed the previous byte in the training text. The first
  byte uses a space as its previous byte. The odds are blended with the plain byte frequencies:

  ```text
  p(s | c) = (n(c, s) + 8 p0(s)) / (n(c) + 8),    p0(s) = (n(s) + 0.5) / (N + 128)
  ```

  The chance of byte `s` after byte `c` is its count after `c` in the training text, softened toward its overall
  frequency. `n(c, s)` counts `s` after `c`, `n(c)` counts `c`, `n(s)` counts `s` and `N` is the length of the
  training text. The probabilities are scaled to integer frequencies with a total of at most 2^16, and every
  byte gets a frequency of at least 1, so any byte can be coded. The code length is close to the text's surprise
  under the model plus about two bits. The decoder reads exactly the number of bytes the length field names;
  bits past the end of the frame read as 0.
- **`:lz`**: LZSS, a small DEFLATE. At each position the coder looks back up to 255 bytes for the longest earlier
  match of 3 to 18 bytes (the nearest one wins a tie). A match is written as a flag bit 1, an 8-bit distance
  and a 4-bit length (the length minus 3). Otherwise it writes a flag bit 0 and the byte in a canonical Huffman
  code built from the training text's byte frequencies.

One wrong payload bit ruins `:ac` and `:lz` text from that point on, while `:none` loses one byte. This is why
the codes below matter. The report measures it: about 3.7 bits per character for `:ac` and about 5.3 for `:lz`
on 61-byte passages of a held-out book.

### The codes

`n` is the memory's size (its number of things). `K` is the number of frame bits the code carries.

| `code:` | `K` | Codeword | Decoding |
|---|---|---|---|
| `:none` | `n` | the frame itself | none |
| `:rep3` | `floor(n / 3)` | each frame bit three times in a row | majority of the three copies |
| `:hamming74` (the default) | `4 * floor(n / 7)` | each 4 frame bits `d1 d2 d3 d4` become 7 bits `p1 p2 d1 p3 d2 d3 d4` | syndrome decoding; corrects one wrong bit in each block of 7 |
| `:ldpc` with `rate: R` | `floor(R * n)` | a systematic low-density parity-check codeword of length `n` | belief propagation, at most 50 rounds |

In the Hamming code, `p1 = d1 xor d2 xor d4`, `p2 = d1 xor d3 xor d4` and `p3 = d2 xor d3 xor d4`.

The LDPC code has `n - K` parity checks. Every code bit sits in three checks, placed in random rows (each placement
takes the least used of up to four random rows, which keeps the check sizes even). The matrix is put in reduced
row echelon form: the pivot columns carry parity, the first `K` other columns carry the frame bits, and any
remaining columns are fixed at 0. The matrix is drawn from a seed made from `n`, `K` and a codebook seed, which is
1 when a note is saved. The decoder is sum-product belief propagation in log-likelihood form. It treats each
recalled bit as a hard bit through a channel that flips 2% of bits (a log-likelihood ratio of 3.89 toward the
recalled value) and pins the fixed columns at 0. It stops as soon as every check holds. If no codeword is found in
50 rounds, the decoder reports that it did not converge. The rate must be above 0.05 and below 0.99; it is kept to
three decimal places, so the code's name reads like `ldpc0.500`.

When the codeword is shorter than `n` (`:rep3` and `:hamming74` usually leave a few things over), the unused
things are stored as if they held a 1 bit, and decoding ignores them.

### The mask and the stored pattern

The note's mask `m` is a random pattern of +1 and -1 values, one per thing, made from the name `coded:<note>`. It is
different from the mask the memory family gives the same name. Code bit `c_i` is stored as:

```text
x_i = (2 c_i - 1) m_i
```

A 1 bit is stored as the mask value and a 0 bit as its opposite. The result looks random whatever the text is,
which is what these memories store best. In a `memory` the pattern is stored by the memory's Hebbian rule (with
its fade); in an `sdm` it is written to every hard location near it.

### Decoding and refusal

A recall ends in a state `s` of the memory's things. The decoder reads a bit `c_i = 1` where `s_i m_i > 0`, decodes
the code, reads the frame's length and check, decompresses that many bytes and computes the check of the result.
A memory can settle on the mirror image of a pattern as easily as on the pattern, so the decoder tries the state
as it is and then its mirror image (`-s`), and accepts the first one whose check passes. If both fail, the recall is
refused, with one reason for each sign:

- `the code did not converge`: the LDPC decoder found no codeword.
- `the payload does not decode`: the compressed stream is invalid (an `:lz` distance that points before the start,
  a bit string that is no Huffman code, or a `:none` payload too short for the length field).
- `the check failed`: the text decoded, but its check does not match the stored check.

A random frame passes the 16-bit check with a chance of about 1 in 65,536, and the decoder tries two frames.

### What the read-address knows (`knows:`)

- **`knows: :name`** (the default): the read-address is the pipeline applied to an all-zero frame. Every code here maps an
  all-zero frame to an all-zero codeword, so this read-address is `-m_i` on every codeword thing (and `m_i` on the unused
  things). It is right on every stored bit that does not depend on the text: the zero padding, anything the code
  computes from the padding alone, and the unused things. It is a coin flip on the rest. It needs only the note's
  name and settings, not the text or its length. A code widens the part of the pattern that is fixed by the
  padding; compression shrinks the part the text controls.
- **`knows: :all`**: the read-address is the stored pattern itself, rebuilt from the saved text with the settings and codebook
  it was saved with. This is what `name.recall read-address: :note` does in the memory family.

The saved text is kept in the model's notes (as the memory family keeps saved text) so that `knows: :all` can
rebuild the pattern. Decoding never reads it.

## `name.save_coded`

**Block:** both. A save in a run block changes the model, so later run blocks see the note too.

**Form:**

```text
name.save_coded :note, "text", compress: :ac, code: :hamming74, rate: 0.5
```

**Arguments:**

| Argument | Type | Default | Meaning |
|---|---|---|---|
| `name` | identifier naming a `memory` or an `sdm` | required | The store to write into. |
| `:note` | symbol | required | The note's name. It makes the mask and is what a recall asks for. |
| `"text"` | string | required | The text to store, at most 255 bytes, and short enough for the frame to fit. |
| `compress:` | one of `:none` / `:ac` / `:lz` | `:ac` | The compressor. |
| `code:` | one of `:none` / `:rep3` / `:hamming74` / `:ldpc` | `:hamming74` | The error-correcting code. |
| `rate:` | number, above 0.05 and below 0.99 | 0.5 | The LDPC code's rate. Ignored for the other codes. |

**What it does:** finds the store (a memory first, then an sdm), builds the frame, codes it with the store's size
as `n`, masks it and stores the pattern. In a memory, every pull inside the memory is first weakened by the
memory's fade and then the pattern's pulls are added. In an sdm, the pattern is written to every hard location within
the sdm's activation radius of it. The note's name is added to the store's own list with the tag `#`. That keeps it out of
the memory's plain `recall` scoreboard, as a keyed note is, and it stops the same name being stored twice. The
text, the compressor and the code are recorded in the model's notes under `coded:<name>` for `recall_coded`. No
randomness is used; the mask comes from the note's name and the LDPC matrix from a fixed seed.

**Output:**

```text
save_coded :<note> in :<name>: <bytes> bytes -> <payload> payload bits (<compress>) + 24 header -> code <code> (rate <K/n>) over <n> things
```

`<code>` is `none`, `rep3`, `hamming74` or `ldpc` followed by the rate to three places (for example `ldpc0.750`).
`<K/n>` is the fraction of the store's things that carry frame bits, to three places. When an sdm write reaches no
hard location, it also prints:

```text
warning: no location of :<name> is near :<note>, so nothing was written
```

**Example:** the same text through each compressor and each code.

```settle example=coded-choices
# The same text through each compressor and each code, one memory per choice.
model :mind do
  memory :a, size: 256
  memory :b, size: 256
  memory :c, size: 512                  # rep3 keeps only a third of its things for the frame
  memory :d, size: 256
  memory :e, size: 256
  a.save_coded :note, "under the third stone", compress: :none, code: :none
  b.save_coded :note, "under the third stone", compress: :lz, code: :hamming74
  c.save_coded :note, "under the third stone", compress: :ac, code: :rep3
  d.save_coded :note, "under the third stone", compress: :ac, code: :ldpc          # rate 0.5
  e.save_coded :note, "under the third stone", compress: :ac, code: :ldpc, rate: 0.75
end
run :mind do
  # each read-address is the stored pattern with 5% of its things flipped
  a.recall_coded read-address: :note, address-noise: 0.05, knows: :all, seed: 1
  b.recall_coded read-address: :note, address-noise: 0.05, knows: :all, seed: 1
  c.recall_coded read-address: :note, address-noise: 0.05, knows: :all, seed: 1
  d.recall_coded read-address: :note, address-noise: 0.05, knows: :all, seed: 1
  e.recall_coded read-address: :note, address-noise: 0.05, knows: :all, seed: 1
end
```

Output:

```text output=coded-choices
save_coded :note in :a: 21 bytes -> 168 payload bits (none) + 24 header -> code none (rate 1.000) over 256 things
save_coded :note in :b: 21 bytes -> 104 payload bits (lz) + 24 header -> code hamming74 (rate 0.562) over 256 things
save_coded :note in :c: 21 bytes -> 66 payload bits (ac) + 24 header -> code rep3 (rate 0.332) over 512 things
save_coded :note in :d: 21 bytes -> 66 payload bits (ac) + 24 header -> code ldpc0.500 (rate 0.500) over 256 things
save_coded :note in :e: 21 bytes -> 66 payload bits (ac) + 24 header -> code ldpc0.750 (rate 0.750) over 256 things
recall_coded :a read-address :note (knows the whole pattern, 5% address-noise) after 30 sweeps: text "under the third stone"
recall_coded :b read-address :note (knows the whole pattern, 5% address-noise) after 30 sweeps: text "under the third stone"
recall_coded :c read-address :note (knows the whole pattern, 5% address-noise) after 30 sweeps: text "under the third stone"
recall_coded :d read-address :note (knows the whole pattern, 5% address-noise) after 30 sweeps: text "under the third stone"
recall_coded :e read-address :note (knows the whole pattern, 5% address-noise) after 30 sweeps: text "under the third stone"
```

**Errors:**

- `no memory or sdm named :<name>`
- `save_coded takes :name, "text", then options` (the statement does not start with a symbol, a comma and a value)
- `a "quoted" string was expected`
- `save_coded does not take `<key>:``
- `compress: takes a symbol` (and the same for `code:`)
- `compress: takes :none, :ac or :lz`
- `code: takes :none, :rep3, :hamming74 or :ldpc (with rate: between 0.05 and 0.99)`
- `a number was expected` (the value of `rate:`)
- `:<note> is already stored in :<name>`
- `"<note>" does not fit in :<name>: <n> bytes is more than the 255 a length byte can name`
- `"<note>" does not fit in :<name>: the frame needs <bits> bits (<payload> of them the <compress> payload) and the code carries <K>`

The last error is the one most often met. A small memory with a code leaves little room:

```settle example=coded-does-not-fit
# A 64-thing memory with a Hamming code carries 4 x 9 = 36 frame bits: 24 header bits and 12 payload bits.
model :mind do
  memory :m, size: 64
  m.save_coded :note, "meet at the harbour at nine"
end
```

```text error=coded-does-not-fit
line 4: "note" does not fit in :m: the frame needs 114 bits (90 of them the ac payload) and the code carries 36
```

## `name.recall_coded`

**Block:** run.

**Form:**

```text
name.recall_coded read-address: :note, address-noise: 0, knows: :name, seed: 24301
name.recall_coded read-address: :note, compress: :ac, code: :ldpc, rate: 0.5, codebook_seed: 1
name.recall_coded read-address: :note, sweeps: 30, temperature: 0.1              # a memory
name.recall_coded read-address: :note, iterated-reads: 10, via: :addresses           # an sdm
```

**Arguments:**

| Argument | Type | Default | Meaning |
|---|---|---|---|
| `name` | identifier naming a `memory` or an `sdm` | required | The store to read. |
| `read-address:` | symbol | required | The note to recall. It need not have been saved (then the recall is normally refused). |
| `address-noise:` | number | 0 | The chance that each read-address value is flipped before the recall starts. |
| `knows:` | `:name` or `:all` | `:name` | Which read-address to start from: see [What the read-address knows](#what-the-read-address-knows-knows). |
| `seed:` | whole number | the run's current generator | Replaces the run's random generator with one seeded by this number. |
| `compress:` | one of `:none` / `:ac` / `:lz` | the note's saved compressor (`:ac` if never saved) | The compressor to decode with. |
| `code:` | one of `:none` / `:rep3` / `:hamming74` / `:ldpc` | the note's saved code (`:hamming74` if never saved) | The code to decode with. |
| `rate:` | number, above 0.05 and below 0.99 | 0.5 | The LDPC rate to decode with, used only when `code: :ldpc` is given here. |
| `codebook_seed:` | whole number | 1 | The seed of the LDPC matrix to decode with. |
| `sweeps:` | whole number | 30 | A memory only: Gibbs sweeps of the whole model. |
| `temperature:` | number above zero | 0.1 | A memory only: the temperature of those sweeps. |
| `iterated-reads:` | whole number | 10 | An sdm only: the most rounds of the read. |
| `via:` | `:addresses` or `:pulls` | `:addresses` | An sdm only: Kanerva's address read, or the zero-temperature settle of the pulls. |

A memory accepts `iterated-reads:` and `via:` and ignores them; an sdm accepts `sweeps:` and `temperature:` and ignores
them.

**What it does:** builds the read-address (see `knows:`), then flips each read-address value with probability `address-noise:`, drawing from
the run's random generator. In a memory it starts every free thing of the model at a random value, sets the
memory's things to the read-address, and runs `sweeps:` Gibbs sweeps of the whole model at `temperature:`, as the memory
family's `recall` does. In an sdm it runs the chosen read from the read-address, up to `iterated-reads:` rounds. Then it decodes
the memory's final state as described in [Decoding and refusal](#decoding-and-refusal).

The compressor, code and codebook used for decoding can be changed with `compress:`, `code:`, `rate:` and
`codebook_seed:`. The `knows: :all` read-address is always rebuilt with the settings the note was saved with, so these
options test what happens when the reader does not have the writer's settings. The note's saved text is never used
to decide the answer.

The recall uses the run's random generator for the address-noise and, in a memory, for the start and the sweeps. The
`seed:` option replaces that generator first. A memory recall and an sdm's pulls read replace the run's last
arrangement, as the memory and sdm recalls do. The model is not changed.

**Output:** one line, either:

```text
recall_coded :<name> read-address :<note> (knows <what>, <address-noise>% address-noise) after <how>: text "<text>"
recall_coded :<name> read-address :<note> (knows <what>, <address-noise>% address-noise) after <how>: text "<text>" (from its mirror image)
recall_coded :<name> read-address :<note> (knows <what>, <address-noise>% address-noise) after <how>: refused (as stored: <reason>; as its mirror image: <reason>)
```

`<what>` is `only the name` or `the whole pattern`. `<address-noise>` is a whole percentage. `<how>` is `<sweeps> sweeps`
for a memory, and `the address read` or `the pulls read` for an sdm. Bytes of the text that are not valid UTF-8 are
printed as the replacement character.

**Example:** a note recalled from its name, from its whole pattern, and a name that was never saved.

```settle example=coded-save-recall
# Store a compressed, error-coded note in a Hopfield memory, then read it back.
model :mind do
  memory :m, size: 256                  # 256 things
  m.remember :cat                       # a plain random pattern shares the memory
  m.save_coded :note, "meet at the harbour at nine"   # defaults: compress: :ac, code: :hamming74
end
run :mind do
  m.recall_coded read-address: :note, address-noise: 0.1, seed: 1              # the read-address knows only the name
  m.recall_coded read-address: :note, address-noise: 0.3, knows: :all, seed: 2 # the read-address is the stored pattern, 30% flipped
  m.recall_coded read-address: :ghost, seed: 3                          # never saved: refused
end
```

Output:

```text output=coded-save-recall
save_coded :note in :m: 27 bytes -> 90 payload bits (ac) + 24 header -> code hamming74 (rate 0.562) over 256 things
recall_coded :m read-address :note (knows only the name, 10% address-noise) after 30 sweeps: text "meet at the harbour at nine"
recall_coded :m read-address :note (knows the whole pattern, 30% address-noise) after 30 sweeps: text "meet at the harbour at nine"
recall_coded :m read-address :ghost (knows only the name, 0% address-noise) after 30 sweeps: refused (as stored: the check failed; as its mirror image: the check failed)
```

**Example:** a coded note in an sdm, read by both of the sdm's reads.

```settle example=coded-sdm
# A coded note in a Kanerva sdm, read by both of the sdm's reads.
model :mind do
  sdm :s, word-size: 256, hard-locations: 600
  s.write :cat
  s.save_coded :memo, "bring the red lantern", code: :ldpc, rate: 0.75
end
run :mind do
  s.recall_coded read-address: :memo, address-noise: 0.05, knows: :all, seed: 1                # the address read (default)
  s.recall_coded read-address: :memo, address-noise: 0.05, knows: :all, via: :pulls, seed: 1   # the pulls read
end
```

Output:

```text output=coded-sdm
save_coded :memo in :s: 21 bytes -> 70 payload bits (ac) + 24 header -> code ldpc0.750 (rate 0.750) over 256 things
recall_coded :s read-address :memo (knows the whole pattern, 5% address-noise) after the address read: text "bring the red lantern"
recall_coded :s read-address :memo (knows the whole pattern, 5% address-noise) after the pulls read: text "bring the red lantern"
```

**Example:** a save inside a run block, and the options each store takes.

```settle example=coded-options
# save_coded also works in a run block, and each store has its own read options.
model :mind do
  memory :m, size: 256
  sdm :s, word-size: 256, hard-locations: 600
end
run :mind do
  m.save_coded :note, "meet at nine", code: :rep3     # stored in the model, like a model-block save
  m.recall_coded read-address: :note, address-noise: 0.1, sweeps: 10, temperature: 0.05, seed: 1   # a memory takes sweeps: and temperature:
  s.save_coded :memo, "meet at nine", compress: :lz
  s.recall_coded read-address: :memo, address-noise: 0.1, iterated-reads: 3, seed: 1                   # an sdm takes iterated-reads: and via:
end
run :mind do
  m.recall_coded read-address: :note, address-noise: 0.1, seed: 2     # the note saved in the first run block is still there
end
```

Output:

```text output=coded-options
save_coded :note in :m: 12 bytes -> 44 payload bits (ac) + 24 header -> code rep3 (rate 0.332) over 256 things
recall_coded :m read-address :note (knows only the name, 10% address-noise) after 10 sweeps: text "meet at nine"
save_coded :memo in :s: 12 bytes -> 57 payload bits (lz) + 24 header -> code hamming74 (rate 0.562) over 256 things
recall_coded :s read-address :memo (knows only the name, 10% address-noise) after the address read: text "meet at nine"
recall_coded :m read-address :note (knows only the name, 10% address-noise) after 30 sweeps: text "meet at nine"
```

**Example:** decoding with the wrong settings is refused. A read-address with 97% address-noise is almost the mirror image of the
pattern, and the decoder reads it as such.

```settle example=coded-controls
# Recalls that decode with the wrong settings are refused, never printed as text.
model :mind do
  memory :m, size: 256
  m.save_coded :note, "meet at nine", code: :ldpc
end
run :mind do
  m.recall_coded read-address: :note, knows: :all, seed: 1                      # the settings it was saved with
  m.recall_coded read-address: :note, knows: :all, codebook_seed: 2, seed: 1    # a different LDPC matrix
  m.recall_coded read-address: :note, knows: :all, code: :hamming74, seed: 1    # a different code
  m.recall_coded read-address: :note, knows: :all, compress: :lz, seed: 1       # a different compressor
  m.recall_coded read-address: :note, knows: :all, address-noise: 0.97, seed: 1        # a read-address near the mirror image
end
```

Output:

```text output=coded-controls
save_coded :note in :m: 12 bytes -> 44 payload bits (ac) + 24 header -> code ldpc0.500 (rate 0.500) over 256 things
recall_coded :m read-address :note (knows the whole pattern, 0% address-noise) after 30 sweeps: text "meet at nine"
recall_coded :m read-address :note (knows the whole pattern, 0% address-noise) after 30 sweeps: refused (as stored: the code did not converge; as its mirror image: the code did not converge)
recall_coded :m read-address :note (knows the whole pattern, 0% address-noise) after 30 sweeps: refused (as stored: the check failed; as its mirror image: the check failed)
recall_coded :m read-address :note (knows the whole pattern, 0% address-noise) after 30 sweeps: refused (as stored: the payload does not decode; as its mirror image: the code did not converge)
recall_coded :m read-address :note (knows the whole pattern, 97% address-noise) after 30 sweeps: text "meet at nine" (from its mirror image)
```

**Errors:**

- `no memory or sdm named :<name>`
- `recall_coded does not take `<key>:``
- `recall_coded needs read-address: :name (the name of a coded note)`
- `a number was expected`
- `knows: takes a symbol` (and the same for `compress:`, `code:` and `via:`)
- `compress: takes :none, :ac or :lz`
- `code: takes :none, :rep3, :hamming74 or :ldpc (with rate: between 0.05 and 0.99)`
- `knows: :all needs a saved note, and :<note> was never saved in :<name>`
- `knows: takes :name or :all`
- `temperature must be above zero` (a memory only)
- `via: takes :addresses or :pulls` (an sdm only)

## Notes

- `address-noise:` defaults to 0 here, where the memory family's `recall` defaults to 0.3.
- A refusal is an answer, not an error: the program goes on. A wrong text can only be printed if a wrong frame
  passes the 16-bit check.
- The report measures how much text comes back exactly for each choice of compressor and code, in a Hopfield memory
  and in an sdm, at several loads and address-noise levels: `experiments/thermosim/runs/sdmcoded/REPORT_SDMCODED.md`.
