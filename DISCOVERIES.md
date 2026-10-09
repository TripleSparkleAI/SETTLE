# SETTLE discoveries

This file maps what the SETTLE campaign measured, and what the upgrade of 2026-10-06 measured, to what each
finding became in the language. Each row names the data file the number comes from. Where a report's prose
disagrees with its own data file (`../runs/README.md`, section "Number checks"), the number here is the data
file's.

A finding becomes one of four things:

- **DEFAULT**: the language does it unless a program says otherwise.
- **OPTION**: a keyword or a statement offers it, and the default is unchanged.
- **DOCS**: a page states it with its number, and there is no mechanism.
- **NOT ADOPTED**: the reason is given.

## 1. Measured in this upgrade

### A survey printed different text on different runs (fixed)

- `examples/survey.settle` printed 3 different texts in 6 runs of one binary.
- Cause: `survey` sorted its valleys by count, then energy. A valley and its mirror image often tie on both, and
  the tied valleys kept the order of a hash map, which changes from run to run.
- **DEFAULT**: the sort now ends on the arrangement itself. Six runs now print one text.
- The test `a_survey_prints_the_same_lines_on_every_run` (`src/valleys.rs`) fails without the tiebreak and passes
  with it.
- No documented example used a survey with ties, so no recorded output changed.

### Metropolised Gibbs settles closer to the exact answer than Gibbs (now an option)

- Instrument: `examples/core_update_measure.rs`. The prediction was sealed in its header and committed
  (`7f3d29554`) before the first run. Output: `../runs/settleperfect/core_update_measure_out.txt`.
- Setup: four small models, each checked against exact enumeration, 300 seeds each, at 100 and 1,000 sweeps.
- The squared error of the yes-rates under `:metro` was **0.146 to 0.465 times** the error under Gibbs.

| model | 100 sweeps | 1,000 sweeps |
|---|---|---|
| weather | 0.194 | 0.146 |
| ring | 0.465 | 0.460 |
| chain | 0.461 | 0.448 |
| dense | 0.463 | 0.423 |

- The sealed predictions, scored:
  - P1, never worse: **held 8 of 8**.
  - P2, ratio between 0.3 and 1.0: **missed 2 of 8**. The weather model did better than predicted.
  - P3, time per update within 30%: **missed 2 of 8**. `:metro` was 32% to 35% faster on the weather model.
- The times were taken at load 43 to 92 on 18 cores, so they measure the machine as much as the code. The errors
  come from fixed seeds and are not affected by load.
- **OPTION**: `settle ..., update: :metro` and `anneal ..., update: :metro`, and `.update("metro")` in the
  builder. The documented example is `core-update`.
- `:gibbs` stays the **default**, so every program prints what it printed before. Making `:metro` the default is
  a decision for the navigator (section 3).
- This agrees with FILMSHARP, which measured the same rule on the film grid (`../runs/filmsharp/`).

### A flat neighbour layout did not speed the sampler (not adopted)

- The experiment: the sampler's pulls stored in three flat arrays instead of one list per thing. The terms were
  added in the same order, so the samples were identical.
- Instrument: `examples/core_bench.rs`, run as an interleaved A/B, minimum of 7 runs each.
- Result: within the noise on all four shapes. Weather was 190 ms against 186 ms, chain 83 against 80, grid 39
  against 39, dense 76 against 83.
- **NOT ADOPTED**: the change added code for no measured gain. It was reverted.
- The benchmark stays. Its yes-count checksums are identical before and after this upgrade
  (`../runs/settleperfect/core_bench_out.txt`).

### SETTLE runs without KANERVA

- `tests/standalone.rs` builds SETTLE with `--no-default-features` and runs all 136 programs (`docs/examples/` and
  `examples/`):
  - 103 programs print the same as the full build;
  - 33 programs use an sdm-family statement, and each is refused by name.
- SETTLE's own copies of the generator and the word codes are held to KANERVA's, draw for draw, by
  `settle_and_kanerva_draw_the_same_stream` and `settle_and_kanerva_give_the_same_codes`.
- The test is red-proven: a one-bit change to the standalone generator makes it fail on
  `docs/examples/colour-channels.settle`.

### Counts were truncated silently (now refused)

- Twenty places read a count with `as usize`:
  - `show: 2.5` showed 2 valleys;
  - `drift 100.5` drifted 100 steps;
  - a negative count became 0.
- **DEFAULT**: one helper, `lex::whole`, now reads every one of them. It is the denoise family's existing rule,
  so its messages did not change.
- `tests/counts.rs` covers seven families and is red-proven.

## 2. The campaign harvest

These are the campaign's measured findings about settling, sampling, schedules and the statement families, with
the status each has now. The SDM memory lanes belong to KANERVA's own `DISCOVERIES.md`.

| Finding (data file) | Status | What it became, or why not |
|---|---|---|
| Column factoring beats Rosenberg. 899 at 100,000 sweeps: 100% against 14%. 3,599: 74% against 0%. 50 seeds (`runs/zoohard/measure_factor.txt`) | DOCS; default flip owed | `zoo.md` now gives the measurement and says to write `encoding: :columns` above a few hundred. The default is unchanged because a flip changes the output of existing programs (section 3). |
| Annealing passes the answer and leaves it. 899 at 50,000 sweeps: 90% of walks visited the answer, 9% ended in it (`runs/zoohard/measure_dwave.txt`) | DOCS (was contradicted) | `10-the-science.md` said annealing "settles into a deep one". It now gives this measurement and says why `anneal` keeps the best arrangement visited and `x.final` reports the end. |
| Nishimori averaging at finite strength. Block error 67.5% against 18.0% for the average over codewords only (`runs/ldpcmoves/exact.txt`, 400 blocks) | DOCS (was contradicted) | `10-the-science.md` said the average at T = 1 is the bitwise best decision. That is true only at infinite strength, and the page now says so. |
| The TAP picture breaks down at stronger pulls: 21.15 dB at 0.35 falls to 7.22 dB at 0.40. TAP beats mean-field on 20 of 20 exact 4x4 targets at every pull (`runs/gridplayer2/tap_out.txt`, `tap_exact_out.txt`) | DOCS; default flip owed | `grid.md` now records both. `correct:` still defaults to `:mean` (section 3). |
| Metropolised Gibbs on a checkerboard mixes fastest on the film. J 0.2, bits at 80 sweeps: 29.68 against 26.71 dB (`runs/filmsharp/mixing_out.txt`) | OPTION; default flip owed | `play`/`play_colour` offer `update: :metro_checker`. The core family now offers `update: :metro` (section 1). |
| Warm fits from the correction are never worse than from the leans. One iteration gains 2.7 to 6.5 dB (`runs/filmwarm/budget_j4*_out.txt`) | OPTION; default flip owed | `warm_from: :correction` exists. The default is still `:leans` (section 3). |
| Factoring needs the right temperature. 143 at 10,000 sweeps: 88% at T = largest pull / 10, against 18% at T = 5 (`runs/settlezoo/measure_factor.txt`) | NOT ADOPTED | A `temperature: :auto` rule changes what an anneal does, and it was measured on one family only. It is owed as an option with its own measurement. |
| Puzzles sharing one model hurt each other. 143 scores 88% alone and 46% beside a prime; `anneal_each` gives 78% (`runs/zoohard/measure_shared_model.txt`) | OPTION (`anneal_each`) | A warning would add a line to existing outputs, so it is left to the navigator (section 3). |
| Many short restarts beat one long anneal at equal sweeps. 10,403: 19/20 at 1,000 x 1,000 against 3/20 at 1 x 1,000,000 (`runs/zootemp/measure_restarts_1e6.txt`) | OPTION (`anneal_schedule restarts:`) | Unchanged. The best walk length grows with N, so no single default fits. |
| The default ldpc strength 2 at flip 0.03 is kappa 0.58. Kappa 0.5 gives 100% block error, kappa 1 gives 87% (`runs/ldpcsettle/kappa.txt`) | DOCS (`ldpcsettle.md`) | A warning or a kappa-scaled strength is owed. Either changes the output of existing programs. |
| Samples are correlated: tau about 28 sweeps on mc16 (`runs/backends/diag_mc16.json`) | DOCS (`04-semantics.md`) | An effective sample count on `ask` would change every `ask` line. It is owed as an opt-in keyword. |
| Burn-in bias is 57% to 93% of the error at kappa 1000 (`runs/smoothnumbers/bench_bias.txt`) | DOCS (`numbers.md`) | `solve` already factors the matrix and could warn. Not built here. |
| Domain-wall nonograms 50/50 against one-start-per-block 0/50 (`runs/puzzlefeature/survey_results.json`) | DEFAULT | none needed |
| Penalty 2 is best for column factoring (`runs/zootemp/measure_lambda.txt`) | DEFAULT | none needed |
| The chain gadget improves with sweeps; the sum gadget stays frozen (`runs/ldpcsettle/diag.txt`) | DEFAULT (`gadget: :chain`) | none needed |
| Block moves of size 4 for LDPC (`runs/ldpcmoves/grid400.txt`) | DEFAULT | none needed |
| Contrastive divergence is the best learning rule tried (`runs/boltzlearn/results.tsv`) | DEFAULT | none needed |

The four `zootemp` and `zoohard` sentences that "Number checks" flags never reached `docs/`. Each was searched for.

## 3. Owed to the navigator: defaults the measurements favour

**Ruled and done on 2026-10-06: all five are the defaults now (section 5).**

Each of these would change the printed output of existing programs and the docs examples. The site's JavaScript
film player mirrors `play`'s defaults, so a flip there must move both together. That is why none of them was made
here.

1. **`play` and `play_colour`: `update: :metro_checker`.** On the film it is ahead of Gibbs at every pull measured
   (`runs/filmsharp/`).
2. **`correct: :tap`.** It wins 20 of 20 exact targets at every pull. On the horse at J 0.3 it gives 24.50 dB
   against 12.67 dB for mean-field (`runs/gridplayer2/`).
3. **`factor encoding: :columns`.** 899: 100% against 14% (`runs/zoohard/`).
4. **`warm_from: :correction`.** It is never worse than `:leans`, and up to 6.5 dB better (`runs/filmwarm/`).
5. **Core `update: :metro`.** Its yes-rate error is 0.15 to 0.47 times Gibbs's (section 1).

## 4. The floors, family by family

The ruling splits each package into an engine, words and doors. This upgrade did it for the core family and for
the crate's spine:

- **Split:** `core`. Its engine is in `src/engine/`: the model, the sampler, the anneal schedule, energy and
  answers. Its words are in `src/words/core.rs`, as statements parsed into data and run by one executor. The
  builder face is in `src/words/builder.rs`, over the word list in `src/words/vocab.rs`.
- **Moved:** JSON reading and writing, from `export.rs` to `src/doors/json.rs`.
- **Not split yet:** the other 19 families keep engine and words in one file each: `memory`, `grid`, `zoo`,
  `learn`, `valleys`, `numbers`, `sdm`, `softsdm`, `export`, `colour`, `sdmscale`, `coded`, `denoise`,
  `ldpcsettle`, `ldpcmoves`, `sdmrefuse`, `zootemp`, `sdmtrack`, `descend`.
- The sdm-family files are ONEPARSER's, through the plug. For each of the others, the next step follows the same
  pattern: statements as data, one executor, and a builder over the family's own words.

## 5. The defaults flipped (2026-10-06, lane NEWDEFAULTS)

The navigator ruled on section 3: "Yes, all five, re-record everything." All five are now **DEFAULT**, and the
old default of each is the option it became. Sections 1 to 3 above are kept as written on the day of the upgrade.

| Section 3 row | Now the default | The old default, as an option |
|---|---|---|
| 1 | `play` / `play_colour` `update: :metro_checker` | `update: :gibbs` |
| 2 | `correct: :tap` (play, play_colour, lean_from) | `correct: :mean` |
| 3 | `factor encoding: :columns` | `encoding: :rosenberg` |
| 4 | `warm_from: :correction` | `warm_from: :leans` |
| 5 | core `update: :metro` (settle, anneal, and now `anneal_each` and `anneal_schedule`) | `update: :gibbs` |

Measured in this landing:

- **The anneal under the new core default.** Factoring 899 (column encoding, `anneal_schedule 20_000,
  temperature: 0.65`), seeds 1 to 40, the prediction sealed first (`../runs/newdefaults/PREDICTION_restarts_2026-10-06.md`).
  Output: `../runs/newdefaults/restarts_899_out.txt`.

  | judged on | `:gibbs` | `:metro` |
  |---|---|---|
  | one walk, calmest visited | 19 of 40 | 28 of 40 |
  | one walk, end state | 2 of 40 | 7 of 40 |
  | 20 restarts, calmest visited | 30 of 40 | 32 of 40 |
  | 20 restarts, end state | 26 of 40 | 21 of 40 |

  P1 (calmest visited with restarts, `:metro` at least as good within 4 seeds) held. P2 (end state with restarts,
  `:metro` worse) held: a cold `:metro` walk always flips a thing with zero input, so it comes to rest away from the
  answer more often. P3 (restarts' end states beat one walk's) held for both rules.
- **A free thing with no input.** Under `:metro` it flips at every visit, so it reads exactly 50.0% on every seed
  (the `sem-seed` example's coin did; it now leans a little).
- **A valleys survey near zero temperature** reaches the rarest valleys less often under `:metro`: on a 12-thing
  landscape with 16 valleys, 3,000 starts found 14, 15 and 15 on seeds 2, 3 and 4, against 16, 15 and 14 under
  `:gibbs`; 30,000 starts found 16, 16 and 15, against 16 on all three.
- **A defect the flip uncovered:** `play ..., update: :gibbs` took the core sweep, which follows the run's core
  rule; with `:metro` the default it silently played Metropolised. Fixed, and the coin-noise test catches it.
