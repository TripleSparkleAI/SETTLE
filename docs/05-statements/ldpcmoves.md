# The ldpcmoves family

The ldpcmoves family adds two decoders for the codes of the [ldpcsettle](ldpcsettle.md) family. The ldpcsettle
decoder updates one thing at a time, so a wrong code bit can only be fixed by walking the change along its checks'
helper things one flip at a time. This family moves a code bit together with the helpers of every check it sits
in, and it can move several bits of one check at once. It also adds a decoder that stays at temperature 1 and
takes each bit's majority value, instead of annealing.

The source is `src/ldpcmoves.rs`. The statements run on a code declared with `ldpc` and sent with `c.transmit`;
they have no model statements of their own. The family was measured in
`SETTLE/runs/ldpcmoves/REPORT_LDPCMOVES.md`: with these moves, a 400-sweep anneal decodes better
than the one-thing ldpcsettle decoder does at 10,000 sweeps, while belief propagation still wins at higher noise.

| Statement | Block | Summary |
|---|---|---|
| [`c.decode_moves`](#cdecode_moves) | run | anneal from the received word with moves that carry each bit's checks with it |
| [`c.decode_nishimori`](#cdecode_nishimori) | run | stay at temperature 1, average each bit, take the majority |

## How the helpers are summed out

Each check's helper things are pulled only by that check's code bits. So once the code bits are fixed, the
helpers of each check can be summed over exactly, one check at a time. For check `r`, at inverse temperature
`beta = 1 / T` and strength `lambda`:

```text
Z_r(y) = sum over the helper values x of check r of exp(-beta * lambda * P_r(y, x))
```

`Z_r` counts the ways the check's helpers can sit for the current code bits `y`, each way weighted by its cost.

- For the `:sum` gadget, with `c` ones in the check, `Z_r = sum over z = 0 .. 2^K - 1 of exp(-beta lambda (c - 2z)^2)`.
  It depends only on the check's weight and `c`, so it is a table lookup.
- For the `:chain` gadget, the partial parities are summed by a forward pass along the chain that carries two
  numbers (the partial parity at 0 and at 1). Each three-bit check adds the factor
  `g(n) = exp(-beta lambda n^2) + exp(-beta lambda (n - 2)^2)`, where `n` counts the ones among its three bits.

With the helpers summed out, the code bits alone follow:

```text
pi(y) proportional to exp(beta * sum over i of h_i s_i) * product over checks r of Z_r(y)
```

This is the distribution of the ldpcsettle model over its code bits, with every helper summed out. `h_i` is the
channel lean of the last `transmit` and `s_i` is +1 for bit 1 and -1 for bit 0. The unit tests check it against the
full model on three small codes, both gadgets and two temperatures. A move that changes some code bits and draws
their values from `pi` is the same as changing those bits and redrawing all the touched checks' helpers from their
exact conditional distribution.

## The movers

Every move is a heat-bath move: it draws new values for the chosen bits with probability proportional to `pi`,
with every other bit held. The unit tests build each mover's full transition matrix on small codes and check that
it leaves `pi` unchanged. The code bits the matrix fixes at 0 are never moved.

- **`:single`**: one code bit. It is set to 1 with probability

  ```text
  P(y_i = 1 | the rest) = 1 / (1 + exp(-D)),
  D = 2 beta h_i + sum over the checks r that hold bit i of [ ln Z_r(y_i = 1) - ln Z_r(y_i = 0) ]
  ```

  `D` is how much more likely bit 1 is than bit 0: the channel's pull plus the change in each of its checks.
  A sweep visits every free code bit once, in a fresh random order.
- **`:block`** with `block: b` (the default mover, with `b = 4`): pick one check, uniformly among the checks that
  hold at least one free bit. Pick `min(b, its free bits)` of its free bits uniformly, and draw their `2^b` joint
  values from `pi`. A pair of flips that keeps the check even is one move. A sweep makes `ceil(free bits / b)`
  block moves, so each free bit is visited about once. `block: 1` is a heat-bath move on one bit of a randomly
  chosen check, which is not the same schedule as `:single`.

A block move evaluates `2^b` joint values, so its cost doubles with each step of `block:`.

## `c.decode_moves`

**Block:** run.

**Form:**

```text
c.decode_moves mover: :block, block: 4, sweeps: 400, seed: 24301
c.decode_moves mover: :single, sweeps: 400
```

**Arguments:**

| Argument | Type | Default | Meaning |
|---|---|---|---|
| `c` | identifier naming a code | required | A code declared with `ldpc`, after a `transmit`. |
| `mover:` | `:block` or `:single` | `:block` | The move. |
| `block:` | whole number, 1 to 10 | 4 | The bits per block move. It is checked even when `mover: :single` is given, and ignored then. |
| `sweeps:` | whole number, at least 2 | 400 | The number of annealing sweeps. |
| `seed:` | whole number | the run's current generator | Replaces the run's random generator with one seeded by this number. |

**What it does:** builds the code with its helpers summed out, using the code's own `gadget:` and `strength:`
and the channel leans of the last `transmit`. It starts at the received word and anneals: sweep `t` runs at

```text
T(t) = 1 * (0.05 / 1)^(t / (sweeps - 1)),    t = 0 .. sweeps - 1
```

so the first sweep runs at temperature 1 and the last at 0.05. These temperatures are fixed; the statement has no
`hot:` or `cold:`. After each sweep it scores the code bits by their energy at temperature zero:

```text
E0(y) = - sum over i of h_i s_i + lambda * (the number of broken checks)
```

At temperature zero every broken check's calmest helper arrangement costs exactly `lambda`, for both gadgets.
The decoder keeps the code bits with the lowest `E0` seen at the start or after any sweep. If they satisfy every
check, it reports a codeword and compares it with the sent codeword. Otherwise it refuses.

The decoder draws one number from the run's random generator and seeds its own generator with it, so `seed:` fixes
the whole decode. It does not use the model's things, the run's holds or the run's temperature. Apart from that
one draw from the run's generator, it changes nothing in the model or the run.

**Output:** one of:

```text
<c> settled with <mover> moves: CODEWORD, the sent codeword; 0 bits differ from what was sent
<c> settled with <mover> moves: CODEWORD, a DIFFERENT codeword (miscorrection); <bits> bits differ from what was sent
<c> settled with <mover> moves: REFUSED, <checks> checks still broken
```

`<mover>` is `single` or `block<b>`, for example `block4`.

**Example:** one received word decoded by the one-thing settle of the ldpcsettle family and by each mover.

```settle example=ldpcmoves-movers
# One received word, decoded by the one-thing settle and by the moves that carry a bit's checks with it.
model :line do
  ldpc :c, bits: 128, rate: 0.5, strength: 4.6
end
run :line do
  c.transmit flip: 0.08, seed: 5
  c.decode seed: 2                         # ldpcsettle: one thing at a time, 400 sweeps
  c.decode_moves seed: 2                   # blocks of 4 bits from one check (the default)
  c.decode_moves block: 2, seed: 2         # blocks of 2
  c.decode_moves mover: :single, seed: 2   # one code bit with its checks' helpers
  c.decode_nishimori seed: 3               # stay at temperature 1 and average each bit
  c.decode_nishimori sweeps: 600, seed: 3
end
```

Output:

```text output=ldpcmoves-movers
c: sent 128 bits (64 message bits), the channel flipped 7
c settled from received: REFUSED, 5 checks still broken
c settled with block4 moves: CODEWORD, the sent codeword; 0 bits differ from what was sent
c settled with block2 moves: CODEWORD, the sent codeword; 0 bits differ from what was sent
c settled with single moves: REFUSED, 2 checks still broken
c averaged at T 1 (block4): CODEWORD, the sent codeword; 0 bits differ from what was sent
c averaged at T 1 (block4): CODEWORD, the sent codeword; 0 bits differ from what was sent
```

**Errors:**

- `<c>.decode_moves needs <c>.transmit first`
- `decode_moves does not take `<key>:``
- `a number was expected`
- `block must be between 1 and 10`
- `mover: takes a symbol like :block`
- `mover is :single or :block, not :<name>`
- `sweeps must be at least 2`

## `c.decode_nishimori`

**Block:** run.

**Form:**

```text
c.decode_nishimori mover: :block, block: 4, sweeps: 2000, seed: 24301
```

**Arguments:**

| Argument | Type | Default | Meaning |
|---|---|---|---|
| `c` | identifier naming a code | required | A code declared with `ldpc`, after a `transmit`. |
| `mover:` | `:block` or `:single` | `:block` | The move. |
| `block:` | whole number, 1 to 10 | 4 | The bits per block move (checked, and ignored with `mover: :single`). |
| `sweeps:` | whole number, at least 2 | 2000 | The number of sweeps. |
| `seed:` | whole number | the run's current generator | Replaces the run's random generator with one seeded by this number. |

**What it does:** starts at the received word and runs `sweeps:` sweeps of the chosen mover at temperature 1. It
discards the first `floor(sweeps / 2)` sweeps. For each code bit it takes the fraction of the remaining sweeps in
which the bit was 1, and sets the bit to 1 when that fraction is above one half and to 0 when it is below. A bit
whose fraction is exactly one half keeps its received value; a bit the matrix fixes at 0 is 0. If the result
satisfies every check, it reports a codeword; otherwise it refuses.

Temperature 1 is the Nishimori temperature of this channel: the channel leans `atanh(1 - 2p)` are the true log-odds
of the received bits there. With the checks infinitely stiff, the distribution at temperature 1 is the exact
posterior over codewords given the received word, and the majority of each bit is the decision that makes the
fewest expected bit errors. At a finite `strength:` the checks are soft and the distribution is a relaxed version of
that posterior, so the majority bits are often not a codeword. The report measures this: at `kappa` 1 the decoder
refuses 93% of blocks at rate 0.5 and `p = 0.03`, and a stiffer penalty helps only up to a point.

The randomness and seeding are as in `c.decode_moves`.

**Output:** one of:

```text
<c> averaged at T 1 (<mover>): CODEWORD, the sent codeword; 0 bits differ from what was sent
<c> averaged at T 1 (<mover>): CODEWORD, a DIFFERENT codeword (miscorrection); <bits> bits differ from what was sent
<c> averaged at T 1 (<mover>): REFUSED, <checks> checks still broken
```

**Example:** `ldpcmoves-movers` above uses `strength: 4.6`, where the average is a codeword. At the default
`strength: 2`, the same kind of read refuses while the anneal, which ends at temperature 0.05, decodes:

```settle example=ldpcmoves-soft-checks
# At the default strength, broken checks are cheap at temperature 1, so the bitwise average is not a codeword.
model :line do
  ldpc :c, bits: 64, rate: 0.5            # strength: 2
end
run :line do
  c.transmit flip: 0.05, seed: 1
  c.decode_nishimori seed: 3              # refused: the majority breaks 3 checks
  c.decode_moves seed: 2                  # the anneal cools to 0.05, where the checks are stiff
end
```

Output:

```text output=ldpcmoves-soft-checks
c: sent 64 bits (32 message bits), the channel flipped 1
c averaged at T 1 (block4): REFUSED, 3 checks still broken
c settled with block4 moves: CODEWORD, the sent codeword; 0 bits differ from what was sent
```

**Errors:**

- `<c>.decode_nishimori needs <c>.transmit first`
- `decode_nishimori does not take `<key>:``
- `a number was expected`
- `block must be between 1 and 10`
- `mover: takes a symbol like :block`
- `mover is :single or :block, not :<name>`
- `sweeps must be at least 2`

## Notes

- Both statements are recognised only on a code declared with `ldpc`. On any other receiver the line is not
  recognised, and the interpreter reports that no statement family knows it.
- Checks of weight above 64 are not supported; the codes built by `ldpc` have much lighter checks.
