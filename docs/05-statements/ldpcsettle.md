# The ldpcsettle family

The ldpcsettle family writes a low-density parity-check (LDPC) code as things and pulls, sends a random message
through a channel that flips bits, and decodes the received word by settling. A codeword of the code is a calm
arrangement of the model, and the received word adds a lean to every code bit. Annealing looks for the calmest
arrangement, which is the most likely codeword. The family also decodes the same received word by belief
propagation, the standard decoder, for comparison. A decode claims a result only when it has found a codeword;
otherwise it refuses.

The source is `src/ldpcsettle.rs`. The codes are the LDPC codes of the [coded](coded.md) family (`src/coded.rs`),
so a code declared here with `bits: n` and `rate: R` is the same matrix a coded note uses in an `n`-thing memory
with `code: :ldpc, rate: R`. The family was measured in `SETTLE/runs/ldpcsettle/REPORT_LDPCSETTLE.md`.
At 512 bits the one-thing-at-a-time settle loses clearly to belief propagation at 400 sweeps; at 10,000 sweeps it
reaches belief propagation's result at rate 0.5 and a 1% flip rate. In 12,800 settle decodes it never returned a
wrong codeword. The [ldpcmoves](ldpcmoves.md) family adds decoders that move several things at once.

| Statement | Block | Summary |
|---|---|---|
| [`ldpc`](#ldpc) | model | declare an LDPC code as code-bit things, helper things and pulls |
| [`c.transmit`](#ctransmit) | run | send a random message through a random-flip channel |
| [`c.decode`](#cdecode) | run | decode the received word by annealing, and report a codeword or refuse |
| [`c.decode_bp`](#cdecode_bp) | run | decode the received word by belief propagation |

## How a parity check becomes pulls

A binary word `c` is a codeword when every check of the code holds:

```text
sum over j in check r of c_j = 0 (mod 2),    for every check r
```

Each check names a few code bits, and the word must have an even number of ones among them.

The engine has only leans and pairwise pulls, so it cannot state "even" directly. Each check is built from extra
**helper things** and a penalty `P_r` that is zero exactly when the check holds and the helpers take their right
values, and at least 1 otherwise. The code bits and helpers are 0/1 values `y` (a thing at yes is 1). The model's
energy is the sum of all the penalties times the `strength:` `lambda`:

```text
E = lambda * sum over checks r of P_r(y) + constant
```

Each penalty is a square of a weighted sum of 0/1 values. Expanded, a square has only terms in one value and
terms in a pair of values, and those become leans and pulls between the things. Two ways of building a check,
called gadgets, are available:

- **`:sum`** gives a check of weight `w` (the number of code bits in it) `K = bits(floor(w / 2))` helpers
  `x_0 .. x_(K-1)`, which spell a whole number `z` in binary:

  ```text
  P_r = (sum over j in r of y_j - 2 z)^2,    z = sum over k of 2^k x_k
  ```

  The penalty is zero exactly when the number of ones in the check is even and `z` is half of it. The helper
  count stays small, but the pulls grow with the check's weight: the report's census of 512-bit codes finds a
  strongest pull of 4 `lambda` for checks of weight 4 to 7 and 16 `lambda` for weights 10 to 13.
- **`:chain`** (the default) splits a check of weight `w` into a chain of `w - 2` three-bit checks, linked by
  `w - 3` partial-parity things, `a_1 = b_0 xor b_1`, `a_2 = a_1 xor b_2`, and so on. Each three-bit check
  `(u, v, t)` has one helper `x`:

  ```text
  P = (u + v + t - 2 x)^2
  ```

  The three-bit penalty is zero exactly when `u + v + t` is even and `x` is half of it. A check of weight 2 is
  `(b_0 - b_1)^2` with no helpers, and a check of weight 1 is `b_0`. The chain uses more helper things than the
  sum gadget, but its strongest pull is 1 `lambda` at every weight in the same census.

In both gadgets the helper values of a codeword are unique, and every arrangement that breaks a check costs at
least `lambda`. The unit tests prove this by trying every arrangement of every check up to weight 12 (`:sum`) and
9 (`:chain`).

The channel adds a lean on each code bit toward the received value. For a channel that flips each bit with
probability `p`:

```text
h_i = atanh(1 - 2p)  toward the received bit,    so  e^(2 h_i) = (1 - p) / p
```

A bit that agrees with the received word is favoured by exactly the channel's odds. Changing one code bit away from
the received word costs `ln((1 - p) / p)` in energy, and breaking one check costs at least `lambda`. With `lambda`
large and the temperature low, the calmest arrangement is the most likely codeword given the received word. The
report writes `lambda = kappa ln((1 - p) / p)`, so the default `strength: 2` at `p = 0.03` is `kappa` about 0.58.

## `ldpc`

**Block:** model.

**Form:**

```text
ldpc :c, bits: 64, rate: 0.5, gadget: :chain, strength: 2, codebook_seed: 1
```

The comma after the name may be left out.

**Arguments:**

| Argument | Type | Default | Meaning |
|---|---|---|---|
| `:c` | symbol | required | The code's name. It is used as the receiver of the run statements (`c.transmit`). |
| `bits:` | whole number, 8 to 4096 | 64 | The code length `n`: the number of code bits. |
| `rate:` | number, 0.1 to 0.9 | 0.5 | The fraction of code bits that carry the message. `K = floor(rate * n)` message bits. |
| `gadget:` | `:chain` or `:sum` | `:chain` | How each check is built from helper things. |
| `strength:` | number above zero | 2 | `lambda`: the least energy a broken check costs. |
| `codebook_seed:` | whole number | 1 | Picks the parity-check matrix; a different seed is a different code. |

**What it does:** builds the LDPC code of the coded family for `n` bits at this rate and codebook seed (the rate
is first rounded to three decimal places). The code has `n - K` checks, and every code bit sits in three of them.
Some columns of the matrix are fixed at 0 so that exactly `K` columns carry the message; the decoders hold those
bits at 0. See [the codes](coded.md#the-codes) for how the matrix is drawn.

It then adds things to the model, in this order: `<c>_b0` .. `<c>_b(n-1)` for the code bits (yes means bit 1),
then `<c>_x0`, `<c>_x1`, ... for every helper and partial-parity thing. It adds the penalty's leans and pulls,
scaled by `strength:`, to those things. The code is part of the model, so the core `settle` and `anneal` run it
like any other model. The code's settings are kept in the model's notes under `ldpcsettle:<c>`. No randomness is
used and nothing is printed.

**Output:** none.

**Example:** the code's things in a core `anneal`. The `best` line lists the 8 code bits and the 28 helper
things of the chain gadget.

```settle example=ldpcsettle-springs
# The code's things and pulls are part of the model, so the core anneal can run them too.
model :line do
  ldpc :c, bits: 8, rate: 0.5          # 8 code bits c_b0 .. c_b7 and their helper things c_x0 ...
end
run :line do
  c.transmit flip: 0.1, seed: 1        # also puts the channel leans on c_b0 .. c_b7
  anneal 200, seed: 1
  best
end
```

Output:

```text output=ldpcsettle-springs
c: sent 8 bits (4 message bits), the channel flipped 1
annealed: 200 sweeps, calmest energy found -70.592
best (energy -70.592): c_b0 yes, c_b1 no, c_b2 yes, c_b3 yes, c_b4 yes, c_b5 yes, c_b6 no, c_b7 yes, c_x0 yes, c_x1 no, c_x2 yes, c_x3 yes, c_x4 yes, c_x5 yes, c_x6 yes, c_x7 yes, c_x8 no, c_x9 yes, c_x10 yes, c_x11 yes, c_x12 yes, c_x13 yes, c_x14 no, c_x15 yes, c_x16 no, c_x17 yes, c_x18 yes, c_x19 yes, c_x20 yes, c_x21 yes, c_x22 no, c_x23 yes, c_x24 yes, c_x25 yes, c_x26 yes, c_x27 yes
```

**Errors:**

- `code :<c> is already declared`
- `ldpc does not take `<key>:``
- `a number was expected`
- `bits must be between 8 and 4096`
- `rate must be between 0.1 and 0.9`
- `strength must be above zero`
- `gadget: takes a symbol like :sum`
- `gadget is :sum or :chain, not :<name>`

## `c.transmit`

**Block:** run.

**Form:**

```text
c.transmit flip: 0.03, seed: 24301
```

**Arguments:**

| Argument | Type | Default | Meaning |
|---|---|---|---|
| `c` | identifier naming a code | required | The code, declared with `ldpc`. |
| `flip:` | number, at least 0 and below 0.5 | 0.03 | The channel's flip probability `p`. |
| `seed:` | whole number | the run's current generator | Replaces the run's random generator with one seeded by this number. |

**What it does:** draws `K` message bits, each 1 with probability one half, from the run's random generator. It
encodes them into an `n`-bit codeword and flips each code bit with probability `p`, drawing again from the run's
generator. The same seed and the same `flip:` give the same message and the same flips; with the same seed and a
larger `flip:`, the message is the same and the flipped bits include the earlier ones.

The sent codeword, the received word and `p` are stored in the model's notes (`ldpcsettle:<c>:sent`,
`ldpcsettle:<c>:recv`), where the decoders read them. A later `transmit` of the same code replaces them.

The statement also puts the channel leans `h_i = atanh(1 - 2p)` toward each received bit on the model's code-bit
things `<c>_b<i>`, replacing the leans of any earlier `transmit` of the same code. For `flip: 0` the lean uses
`p = 0.000001`, so it stays finite (about 6.9). These leans are a change to the model, so they stay for later run
blocks, and the core `anneal` then decodes the received word too.

**Output:**

```text
<c>: sent <n> bits (<K> message bits), the channel flipped <flips>
```

**Example:** see `ldpcsettle-decode` under [`c.decode`](#cdecode).

**Errors:**

- `transmit does not take `<key>:``
- `a number was expected`
- `flip must be at least 0 and below 0.5`

## `c.decode`

**Block:** run.

**Form:**

```text
c.decode start: :received, sweeps: 400, hot: 1, cold: 0.05, seed: 24301
c.decode start: :random, sweeps: 400, hot: 10, cold: 0.05
```

**Arguments:**

| Argument | Type | Default | Meaning |
|---|---|---|---|
| `c` | identifier naming a code | required | The code, after a `transmit`. |
| `start:` | `:received` or `:random` | `:received` | Start from the received word, or from a random arrangement of every thing. |
| `sweeps:` | whole number | 400 | The number of annealing sweeps. |
| `hot:` | number, at least `cold:` | 1 from `:received`, 10 from `:random` | The first sweep's temperature. |
| `cold:` | number above zero | 0.05 | The last sweep's temperature. |
| `seed:` | whole number | the run's current generator | Replaces the run's random generator with one seeded by this number. |

**What it does:** builds a separate copy of the code as leans and pulls, with the channel leans of the last
`transmit`. It does not use the model's own `<c>_b` and `<c>_x` things, the run's holds or the run's temperature.

- **The start.** From `:received`, the code bits start at the received word and every helper at its calmest value
  for those bits. From `:random`, every thing starts at a random value. The code bits the matrix fixes at 0 are
  held at 0 throughout.
- **The anneal.** Each sweep updates every free thing once, in a fresh random order, with the Gibbs rule (see
  [Semantics](../04-semantics.md)). The temperature falls geometrically:

  ```text
  T(t) = hot * (cold / hot)^(t / (sweeps - 1)),    t = 0 .. sweeps - 1
  ```

  The first sweep runs at `hot:` and the last at `cold:`.
- **The answer.** The decoder keeps the calmest arrangement seen at the start or at the end of any sweep. If its code bits
  satisfy every check, it reports a codeword and compares it with the sent codeword. Otherwise it refuses.

The decoder draws one number from the run's random generator and seeds its own generator with it, so `seed:`
fixes the whole decode.

**Output:** one of:

```text
<c> settled from <start>: CODEWORD, the sent codeword; 0 bits differ from what was sent
<c> settled from <start>: CODEWORD, a DIFFERENT codeword (miscorrection); <bits> bits differ from what was sent
<c> settled from <start>: REFUSED, <checks> checks still broken
```

`<start>` is `received` or `random`. A miscorrection is a valid codeword that is not the one sent; the decoder
cannot tell it from the right one, so it reports it and says so.

**Example:** one code with each gadget, decoded from the received word, from noise and by belief propagation.

```settle example=ldpcsettle-decode
# A 64-bit LDPC code built as springs, sent through a channel that flips 5% of bits, then decoded.
model :line do
  ldpc :c, bits: 64, rate: 0.5                  # the chain gadget (the default), strength 2
  ldpc :s, bits: 64, rate: 0.5, gadget: :sum    # the same code with the sum gadget
end
run :line do
  c.transmit flip: 0.05, seed: 1      # a random message, encoded and sent
  c.decode seed: 2                    # anneal from the received word, temperature 1 down to 0.05
  c.decode start: :random, seed: 3    # anneal from noise, temperature 10 down to 0.05
  c.decode_bp                         # belief propagation on the same received word
  s.transmit flip: 0.05, seed: 1      # the same message and the same flips
  s.decode seed: 2
  s.decode start: :random, seed: 3    # the sum gadget does not find a codeword from noise here
end
```

Output:

```text output=ldpcsettle-decode
c: sent 64 bits (32 message bits), the channel flipped 1
c settled from received: CODEWORD, the sent codeword; 0 bits differ from what was sent
c settled from random: CODEWORD, the sent codeword; 0 bits differ from what was sent
c by belief propagation: CODEWORD, the sent codeword; 0 bits differ from what was sent
s: sent 64 bits (32 message bits), the channel flipped 1
s settled from received: CODEWORD, the sent codeword; 0 bits differ from what was sent
s settled from random: REFUSED, 6 checks still broken
```

**Example:** a stiffer penalty and a longer anneal, a noisier channel, and a penalty that is too weak.

```settle example=ldpcsettle-options
# Longer anneals, stiffer penalties and harder channels. A failed decode is a refusal, never a guess.
model :line do
  ldpc :c, bits: 128, rate: 0.5, strength: 4.6, codebook_seed: 7   # a stiffer penalty, a different matrix
  ldpc :w, bits: 64, rate: 0.5, strength: 0.5                      # a penalty too weak to hold the checks
end
run :line do
  c.transmit flip: 0.06, seed: 2
  c.decode seed: 2                                   # 400 sweeps from the received word: not enough
  c.decode sweeps: 3000, seed: 2                     # a longer anneal finds the sent codeword
  c.decode sweeps: 100, hot: 2, cold: 0.1, seed: 2   # a short anneal on a different schedule
  c.decode_bp
  c.transmit flip: 0.15, seed: 2                     # the same seed: the same message, more flips
  c.decode_bp                                        # belief propagation gives up after 50 rounds
  w.transmit flip: 0.05, seed: 1
  w.decode seed: 2                                   # the received word is calmer than any codeword
  w.decode_bp                                        # belief propagation does not use the strength
end
```

Output:

```text output=ldpcsettle-options
c: sent 128 bits (64 message bits), the channel flipped 6
c settled from received: REFUSED, 3 checks still broken
c settled from received: CODEWORD, the sent codeword; 0 bits differ from what was sent
c settled from received: REFUSED, 8 checks still broken
c by belief propagation: CODEWORD, the sent codeword; 0 bits differ from what was sent
c: sent 128 bits (64 message bits), the channel flipped 15
c by belief propagation: REFUSED, 64 checks still broken
w: sent 64 bits (32 message bits), the channel flipped 1
w settled from received: REFUSED, 3 checks still broken
w by belief propagation: CODEWORD, the sent codeword; 0 bits differ from what was sent
```

With `strength: 0.5` a broken check costs less than one disagreement with the received word (`ln(0.95 / 0.05)`
is about 2.9), so the calmest arrangement stays near the received word and breaks checks. The decode refuses.

**Errors:**

- `<c>.decode needs <c>.transmit first`
- `decode does not take `<key>:``
- `a number was expected`
- `start: takes a symbol like :sum` (the message names `:sum` although `start:` takes `:received` or `:random`)
- `start is :received or :random`
- `need hot >= cold > 0`

```settle example=ldpcsettle-transmit-first
# A decode needs a received word.
model :line do
  ldpc :c, bits: 64
end
run :line do
  c.decode seed: 2
end
```

```text error=ldpcsettle-transmit-first
line 6: c.decode needs c.transmit first
```

## `c.decode_bp`

**Block:** run.

**Form:**

```text
c.decode_bp
```

The statement takes no arguments. A line such as `c.decode_bp seed: 1` is not recognised, and the interpreter
reports that no statement family knows it.

**What it does:** decodes the last `transmit`'s received word with sum-product belief propagation, the decoder the
coded family uses. Each received bit enters as a log-likelihood ratio of `ln((1 - p) / p)` toward its value, using
the `flip:` of the `transmit` (the bits fixed at 0 are pinned there). It runs at most 50 rounds and stops as soon as
every check holds. The `strength:` and `gadget:` of the code play no part. No randomness is used.

**Output:** one of:

```text
<c> by belief propagation: CODEWORD, the sent codeword; 0 bits differ from what was sent
<c> by belief propagation: CODEWORD, a DIFFERENT codeword (miscorrection); <bits> bits differ from what was sent
<c> by belief propagation: REFUSED, <checks> checks still broken
```

On a refusal, `<checks>` is always the code's total number of checks (`n - K`), not the number that were still
broken when belief propagation stopped.

**Example:** see `ldpcsettle-decode` and `ldpcsettle-options` above.

**Errors:**

- `<c>.decode_bp needs <c>.transmit first`, reported as `line 0:` rather than the statement's line.

## Notes

- `ldpc` adds things named `<c>_b<i>` and `<c>_x<j>`. Do not declare things with these names before the `ldpc`
  line: the code then reuses the existing thing for one of its own and the interpreter stops with an internal
  error.
- A decode reads only the last `transmit`. Each `transmit` replaces the sent word, the received word and the
  channel leans.
