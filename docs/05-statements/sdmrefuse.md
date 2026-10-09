# The sdmrefuse family

The sdmrefuse family answers one question about sparse distributed memories: when should a read say "I never
stored that"? A read that settles from a read-address always ends on some pattern. A read-address that was never stored usually ends
on a stored pattern too, and its final state then looks exactly like a correct recall. The memory can still see how
far the read moved: the Hamming distance from the read-address to the answer, called the **travel**. The family's one
statement, `refusal`, is a calculator for the travel rule. For a memory holding a given number of random patterns,
it prints the largest travel an answer may have and still be accepted, how often a never-stored read-address is then
refused, and the best recall any read could reach with that rule.

The source is `src/sdmrefuse.rs`. The words of this family are parsed by KANERVA (`kanerva::lang`, its keywords in `kanerva/src/words.rs`), the same parser the `kanerva` command uses, so a `.kanerva` file of these lines prints the same under `settle` and `kanerva` (see [the plug](#the-plug) below). Most of the file is measurement code with no statement of its own: reads that
return diagnostics, the exact nearest-neighbour oracle, and a predictor of how a read converges. The measurement
program `examples/sdmrefuse_measure.rs` uses them. The family was measured in
`SETTLE/runs/sdmrefuse/REPORT_SDMREFUSE.md`. There, the travel rule followed the exact oracle to
within 0.01 to 0.05 wherever never-stored read-addresses moved, and failed in the most crowded stores, where they did not
move. The stores it was measured on are those of the [sdmscale](sdmscale.md) family.

| Statement | Block | Summary |
|---|---|---|
| [`refusal`](#refusal) | run | print the travel rule's threshold and the nearest-neighbour ceiling for a size and a load |

## `refusal`

**Block:** run.

**Form:**

```text
refusal word-size: 256, load: 1000, level: 0.01
```

**Arguments:**

| Argument | Type | Default | Meaning |
|---|---|---|---|
| `word-size:` | whole number, 16 to 4096 | 256 | `n`: the number of bits in a pattern. |
| `load:` | whole number, at least 1 | 1000 | `T`: the number of random patterns the memory holds. |
| `level:` | number, above 0 and below 1 | 0.01 | `alpha`: the largest allowed chance that a never-stored read-address is accepted. |

The help line shows `load: 3000` as an example value; the default is 1000.

**What it does:** computes, exactly and without randomness, three things for a memory of `T` random `n`-bit
patterns. It builds no memory and does not read or change the model or the run.

1. **The threshold `h`.** A never-stored read-address is a random `n`-bit pattern. The chance that one random pattern lies
   within `h` bits of it is

   ```text
   F(h) = P[Bin(n, 1/2) <= h]
   ```

   the chance that a fair coin tossed `n` times shows at most `h` heads. The chance that none of the `T` stored
   patterns lies that close is

   ```text
   R(h) = (1 - F(h))^T
   ```

   A read that ends on a stored pattern then travels more than `h` bits, so `R(h)` is the refusal rate of any read
   that always ends on a stored pattern, and of the oracle below.

   `h` is the largest value, counting up from 0, with `1 - R(h) <= alpha`. The rule is then: accept a read's
   answer only if it lies within `h` bits of the read-address.
2. **The refusal rate `R(h)`** at that threshold.
3. **The nearest-neighbour ceiling.** A stored read-address is a stored pattern with each bit flipped with probability `D`.
   The oracle sees every stored pattern, answers the nearest one (ties broken uniformly at random), and refuses when
   that nearest pattern is more than `h` bits away. No read can recall more often at the same refusal rate. For
   address-noise `D` at 10%, 20%, 30% and 40%, the statement computes the chance that the oracle names the right pattern
   and accepts it:

   ```text
   recall(D) = sum over d = 0 .. h of P[Bin(n, D) = d] * ((G + g)^T - G^T) / (T g)
   G = P[Bin(n, 1/2) > d],    g = P[Bin(n, 1/2) = d]
   ```

   The read-address lies `d` bits from its own pattern. The second factor is the chance that its pattern beats the `T - 1`
   other patterns, each a random distance away, with ties shared uniformly. It also computes the same sum over every
   `d`, which is the oracle's recall with no refusal at all.

**Output:** one line:

```text
refusal for <T> patterns of <n> bits: accept an answer only if it lies within <h> bits of the cue (a never-stored cue is refused with probability <R>); the nearest-neighbour ceiling then recalls 10%: <r> of <a>, 20%: <r> of <a>, 30%: <r> of <a>, 40%: <r> of <a>
```

`<R>` has four decimal places. Each `<r> of <a>` pair is the oracle's recall with the rule, then its recall with no
refusal, both with three decimal places.

**Example:**

```settle example=sdmrefuse-refusal
# The refusal threshold and the nearest-neighbour ceiling for a few memory sizes and loads. No memory is built.
model :m do
end
run :m do
  refusal                                        # 1000 patterns of 256 bits, level 0.01
  refusal load: 3000                             # more patterns: a tighter threshold
  refusal word-size: 64, load: 10, level: 0.001       # short patterns and a stricter level
  refusal word-size: 1024, load: 100_000              # long patterns hold many more
end
```

Output:

```text output=sdmrefuse-refusal
refusal for 1000 patterns of 256 bits: accept an answer only if it lies within 93 bits of the cue (a never-stored cue is refused with probability 0.9928); the nearest-neighbour ceiling then recalls 10%: 1.000 of 1.000, 20%: 1.000 of 1.000, 30%: 0.988 of 0.999, 40%: 0.128 of 0.490
refusal for 3000 patterns of 256 bits: accept an answer only if it lies within 91 bits of the cue (a never-stored cue is refused with probability 0.9935); the nearest-neighbour ceiling then recalls 10%: 1.000 of 1.000, 20%: 1.000 of 1.000, 30%: 0.976 of 0.998, 40%: 0.081 of 0.375
refusal for 10 patterns of 64 bits: accept an answer only if it lies within 16 bits of the cue (a never-stored cue is refused with probability 0.9996); the nearest-neighbour ceiling then recalls 10%: 1.000 of 1.000, 20%: 0.875 of 0.999, 30%: 0.233 of 0.938, 40%: 0.009 of 0.544
refusal for 100000 patterns of 1024 bits: accept an answer only if it lies within 428 bits of the cue (a never-stored cue is refused with probability 0.9916); the nearest-neighbour ceiling then recalls 10%: 1.000 of 1.000, 20%: 1.000 of 1.000, 30%: 1.000 of 1.000, 40%: 0.885 of 0.976
```

With 1000 patterns of 256 bits, a 40% read-address is recalled by the oracle 49% of the time with no refusal, and 12.8% of
the time once the rule refuses 99.28% of never-stored read-addresses. At 10% to 30% address-noise the rule costs almost nothing.

**Errors:**

- `refusal does not take `<key>:``
- `a number was expected`
- `refusal size must be between 16 and 4096`
- `refusal needs load: at least 1 and level: between 0 and 1`

## Notes

- The threshold search starts at `h = 0` and stops at the first `h` that breaks the level. If even `h = 0` breaks
  it, the statement still prints `within 0 bits`, and the printed refusal rate is below `1 - level`. This happens
  only for short patterns and large loads, for example `word-size: 16, load: 1000`, which prints a refusal rate of 0.9849
  against a level of 0.01.
- `word-size:` and `load:` are cut to whole numbers, so `load: 0.5` is `load: 0` and is refused.
- The rule and the oracle assume the stored patterns and the never-stored read-addresses are random and independent. The
  report measures how real reads compare with them.

## The plug

SETTLE does not parse this family itself. Its registry entry is a mount of KANERVA's family (`src/plug.rs`): the
line goes to `kanerva::lang`, and the typed statement comes back to `src/sdmrefuse.rs`, which runs it on the model.
The `kanerva` command runs the same lines on KANERVA's engine alone, and the test `tests/oneparser_parity.rs` holds
the two equal: every program KANERVA accepts prints the same lines under both, and every error is the same error.
`refusal` runs under both commands.

