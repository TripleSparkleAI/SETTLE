# Changelog

What each change to the SETTLE interpreter added, in order, taken from the git history of the interpreter's
folder in the SETTLE research repository (`experiments/thermosim/settle-rs/` until 2026-10-05, then
`SETTLE/settle-rs/`). The crate's version has stayed at `0.1.0` (`Cargo.toml`) throughout; entries are grouped
by date and name the commit that landed them.

**The commit names before 2026-10-04 do not resolve any more.** The research repository's history was restarted
on 2026-10-04 as one commit holding every tracked file, so the short hashes below are a record of that older
history, not links into the current one. The SETTLE repository has its own history of export commits, each
naming the research commit it came from. Lane names in capitals (GRIDPLAYER, SETTLEZOO, ...) are
the campaign lanes in `SETTLE/SETTLE_CAMPAIGN_2026-09-30.md`; each has a report in
`SETTLE/runs/`.

## Before the Rust interpreter

`SETTLE/origins/settle-python-toy/settle.py` was the first SETTLE interpreter, in Python, with a line-per-statement syntax
(`thing rain, sprinkler`, `rain leans no by 1.5`, `rain pulls wet_grass by 2`). Its example is
`SETTLE/origins/settle-python-toy/examples/rain.settle`. The Rust interpreter replaced that syntax with the Rails-style one
documented here.

## 2026-09-30

- `7e0d2d97e` **core.** SETTLE in Rust with Rails-style syntax: `model` and `run` blocks, `thing`, `pulls`,
  `pushes`, `hold`, `settle`, `anneal`, `show`, `best`, `ask`.
- `e88efdf4a` **The plug-in slot and memory.** The interpreter split into modules, with the `Ext` trait and
  `registry()` so each statement family lives in its own file; sparse pull storage; the **memory** family
  (`memory`, `remember`, `save`, `recall`).
- `3afc8a149` **grid.** One thing per pixel, PGM in and out, `play` for a folder of frames. GRIDPLAYER added
  its measurements and the horse example (`3267a3011`).
- `cbc3e4df3` **zoo.** Sudoku, graph colouring, max-cut and factoring as settling puzzles, with `solution`.
  SETTLEZOO added its harness and four examples (`5d139d734`).
- `058a8fc29` **learn.** `examples`, `hidden`, `learn`, `classify`, `shuffle`: a Boltzmann machine learner and a
  settled classifier (BOLTZLEARN; results and example `5f0875b78`).
- `5a4a6ce6f` **valleys.** `landscape`, `valleys`, `survey`: exact enumeration and random surveys of the valleys
  (VALLEYMAP; driver and example `7ec6a13e6`).
- `4d14f6d6d` **numbers.** Real-valued numbers on springs settled by Langevin drift: `number`, `springs`,
  `opposes`, `leans_to`, `drift`, `means`, `spread`, `solve` (SMOOTHNUMBERS; measurements `ce1e0ea9c`,
  `f5bb8396e`).
- `11887fe7c` **Keyed memory.** `save ..., key:` and `recall key:` in the memory family (SDMKEYS).
- `9146fb873` **sdm.** Kanerva hard locations with counters in the pulls: `sdm`, `write`, `read` (SDMKEYS; examples
  `7f04c8d82`, instrument and results `437fbc1ea`, `e4facff63`).
- `6670e1522` **softsdm.** Sparse distributed memory built from p-bits with a soft cut-off: `softsdm`, `write`,
  `read`, `attend` (SOFTSDM; baseline and examples `f2fc75fd5`, `818b05efc`). `6fd9ad6d7` let the sdm and
  softsdm families share the registry.
- `b82d3b20e` **export.** `export` as Ising, QUBO, Gset and DIMACS, and `import` of an Ising file (SETTLEBACKENDS).
- `79d98bc3d` **grid: TAP.** `correct: :tap`, `copies:`, and a shared PNM reader.
- `6a3586cdc` **colour.** Three grids per colour picture, PPM in and out, `play_colour` (GRIDPLAYER-2;
  measurements `0118b9984`).
- `b7a2d3430` **sdmscale.** Sparse distributed memory at scale, with the S-map theory (SDMSCALE; measurements from
  2,000 to 1,000,000 locations `9d8bea03a`).
- `ed5228045` **coded.** Compress, error-code, mask and store: `save_coded` and `recall_coded` (SDMCODED;
  measurements and example `963756c27`).
- `d74b61bc0` **denoise.** A chain of conditional restricted machines: `denoiser`, `train`, `generate`,
  `sample`, `coins` (BOLTZLEARN-2; the `leans:` option and example `8d653a166`).
- `ba0ff31af` **ldpcsettle.** LDPC codes built as SETTLE springs with sum and chain parity gadgets (LDPCSETTLE).

## 2026-10-01

- **Kanerva's terms (KANERVATERMS).** The SDM keywords take Kanerva's own words, hyphenated: `cue:` is
  `read-address:`, `damage:` is `address-noise:`, `locations:` is `hard-locations:`, `radius:` is
  `activation-radius:`, `fire:` is `activation-probability:`, `iterations:` is `iterated-reads:`, `tolerate:` is
  `tolerate-noise:`, and `size:` is `word-size:` on the sdm, sdmscale, softsdm, contenttrack and refusal statements.
  A retired keyword stops the program with the new word. The lexer reads a hyphen between letters as part of
  one word. Sources and quotes: `SETTLE/kanerva/KANERVA_TERMS.md`.
- `29dcf6a0f`, `e3d78a7c6`, `909215f37` **ldpcsettle** measurements, results and the example program.
- `c331d5247` **zoo: harder instances.** `factor ..., encoding: :columns` (column-with-carries factoring) and
  `anneal_each`; `src/zoohard.rs` generators and exact deciders (ZOOHARD; measurements `d7f5b5ee3`, `895701594`,
  `d54a0bc4b`; header `a3c43aa0b`).
- `5a8660f78` **sdmscale: radius per damage.** `src/sdmradius.rs`: a radius chosen for the damage a cue carries,
  the Poisson-conditioned S-map, and a density-scaled pulls read (SDMRADIUS; follow-ups `a691d0386`,
  `4c5b7a6ea`, `829de11a8`).
- `fa479f343` **grid and colour: FILMSHARP.** `src/filmsharp.rs`: fitted leans (`fit:`), the Bethe inversion
  (`correct: :bethe`), the Rao-Blackwellised read (`read: :rb`) and faster-mixing updates (`update:`); then
  `play ..., against:` and the exact cluster law (`98e2f4dde`); runs `2bcd3ed8c`.
- `fd141e0dc` **sdmrefuse.** When a sparse distributed memory read should refuse to answer, and the `refusal`
  statement (SDMREFUSE; follow-ups `d868d4cd1`).
- `a7c61cc59` **ldpcmoves.** Helpers summed out, multi-bit check-block moves, and Nishimori averaging:
  `decode_moves`, `decode_nishimori` (LDPCMOVES; seals `8f7c74050`, `e1681760a`, `128451645`).
- `5a2be89b4` **grid: FILMWARM.** `src/filmwarm.rs`: warm-started lean fits for `play` (`warm_fit:` and related
  options), the tau example, frames and controls; `ba0892a39` added `warm_step:`.
- `d8c04dc4e` **zootemp.** `anneal_schedule` (a schedule dial with hot and cold ends and restarts) and
  `f.final` (judge the state the walk ended in, beside `solution`'s best-so-far) for the zoo's puzzles (ZOOTEMP).
- `1a9ee8922` **sdmtrack.** `contenttrack`, the TRACK-C predictor of recall for content reads of a sparse
  distributed memory; no memory is built (SDMTRACK; seals and runs `a89d0bf30`, `88a720097`, `9fa80524a`,
  `31cf35665`, `f3c9b10fd`).
- `c449a01d1` **descend.** Gradient descent as Settling: `data`, `test`, `loss` (springs, least squares, logistic,
  a one-hidden-layer net), `descend` (Langevin above temperature 0, gradient descent at 0, Adam, minibatches,
  walkers, temperature and step schedules), `ask`, `score` (GRADSETTLE).
- **Documentation.** This `docs/` folder: the language reference, one page per family, and
  `tests/docs_examples.rs`, which runs every documented example and checks its output.

## 2026-10-04 to 2026-10-05

- The SETTLE site's page for the language moved to `#/settle` (the old address redirects there), and
  `docs/09-examples.md` links to it.
- The research repository moved `experiments/thermosim/` to `SETTLE/` (lane MOVESETTLE); the interpreter is now
  `SETTLE/settle-rs/`.

## Unreleased: release preparation (2026-10-05)

- **The command line.** `settle --version` (and `-V`) prints `settle` and the crate version. An unknown option
  (any other argument starting with `-`) and a second program path stop with a message and exit status 2,
  instead of being read as a file name or ignored.
- **Errors point and suggest.** The `settle` command prints the program line an error points at with a caret
  under the place and its column (`lex::locate`). An unknown keyword names the nearest keyword the statement
  takes (``did you mean `seed:`?``) or lists every keyword it takes; an unknown thing names the nearest declared
  thing; a line no family knows names the nearest verb, or says that the verb belongs in the other kind of block.
  The first line of every message is unchanged in form, `line N: <message>`, so code that reads it still works.
- **The nonogram statement is documented.** `nonogram` (lane PUZZLEFEATURE) was in `settle --help` and in the
  code but on no page; the zoo page now has its section and a tested example, `zoo-nonogram`.
- **Docs completeness is a test.** `tests/docs_complete.rs` fails when a keyword a statement accepts, or a
  statement a family lists in `settle --help`, is not written on that family's page.
- **The KANERVA boundary is written down** in Statements by family: which SETTLE file uses which KANERVA module.
- **The horse example runs from a clean clone.** Its 15 Muybridge frames (public domain, 150 x 100 PGM) ship in
  `examples/horse/frames/` with their source in `SOURCE.txt`; `examples/horse.settle` reads them there and writes
  `examples/horse/still.pgm` and `examples/horse/out/`, which git ignores. It used to read frames from a
  research folder that is not in git.
- **The package.** `Cargo.toml` names the repository, the README, keywords and categories, and
  `publish = false`; `LICENSE.md` says no licence is chosen yet; `RELEASE_CHECKLIST.md` lists what is ready and
  what the owner decides. The README covers getting the source, building, the command line, the docs, the tests
  and the examples.
- **Docs.** Install and run says where Cargo finds KANERVA in each repository (a git URL in the SETTLE
  repository, the sibling folder in the research repository) instead of claiming an offline build everywhere.
  Statements by family lists `src/mnist.rs` among the modules without statements; Examples lists the
  `gradsettle_measure` and `mnist_measure` programs.
- **Warnings.** Three compiler warnings (an unused import, an unneeded `mut`, an unused closure argument) and
  two lint findings (an operator precedence that read ambiguously, and an `if` whose two branches were the same)
  are fixed, with the behaviour unchanged.

## 2026-10-06 (lane SETTLEPERFECT)

- **Three floors.** The crate is split into `src/engine/` (the model, the sampler, the anneal schedule, energy,
  answers, the random generator, the word codes), `src/words/` (the lexer, the blocks, the registry at
  `words/registry.rs`, the core family, its word list `vocab.rs`, the builder) and `src/doors/` (the command and
  JSON). Every old path (`settle::model`, `lex`, `ext`, `interp`, `core`, `rng`) still resolves.
- **The builder face.** `Model::build()` writes the core statements in Rust over the same word list as a program
  and runs them through the same code, so it prints the same lines. `tests/two_faces.rs` holds the two faces equal
  on the documented core examples and holds the builder's methods to the word list.
- **KANERVA is optional.** The `sdm` feature (on by default) brings it in; without it SETTLE builds and runs every
  other statement, prints the same, and refuses an sdm-family line by name. SETTLE owns its own generator and word
  codes, held equal to KANERVA's by tests. `tests/standalone.rs` checks all 136 programs.
- **`update: :metro`.** `settle` and `anneal` take `update: :gibbs` (the default) or `update: :metro`
  (Metropolised Gibbs). Measured against exact enumeration on four small models, 300 seeds each, its yes-rate
  error was 0.15 to 0.47 times Gibbs's at the same sweeps (`runs/settleperfect/`). New example `core-update`.
- **`settle --json`.** One JSON object out: the printed lines, or the error with its line and column.
- **Counts.** Every family reads a count with one helper, `lex::whole`; a fraction or a negative count is refused
  instead of truncated. `settle` and `anneal` refuse a count below 1.
- **Errors.** Tokens in error messages are quoted as written (`` `3` ``), not in their internal form (`Num(3.0)`).
- **A survey prints the same on every run.** Valleys tied on count and energy were ordered by a hash map, so
  `examples/survey.settle` printed three different texts in six runs; an arrangement tiebreak fixes the order.
- **Docs.** The science page says where Nishimori averaging and annealing fall short in the measurements; the grid
  page records where the TAP picture breaks down; the zoo page gives the measured case for `encoding: :columns`.
- **Lint.** Clippy's findings fell from 155 to 3, all in the sdm files, which lane ONEPARSER then cleared; every
  documented output is unchanged.

### 2026-10-06, lane ONEPARSER: one parser for the sdm family

- **KANERVA parses the sdm family.** `sdm`, `softsdm`, `sdmscale`, `refusal` and `contenttrack` lines go to
  `kanerva::lang` (KANERVA's file face), mounted in the registry by `src/plug.rs` under the same five names in the
  same five places; the typed statement comes back to the family file's `exec`, which runs it on the model. The
  keywords live in `kanerva/src/words.rs`, the one word list. `memory` stays SETTLE's.
- **The same lines under two commands.** A `.kanerva` file of these statements runs under `kanerva` (KANERVA
  alone) and under `settle`, and prints the same lines; `tests/oneparser_parity.rs` holds the two lexers equal on
  every line of every example, and the two runners equal on every program KANERVA runs. `via: :pulls` on an sdm is
  SETTLE's alone.
- **`write-samples:`** is softsdm's spelling of record; `write_samples:` still works.
- **An sdm whose things clash is an error, not a panic:** `a thing :s_loc_0 already exists; pick another sdm name`.

### 2026-10-06, lane NEWDEFAULTS: the five measured settings become the defaults

The navigator ruled on DISCOVERIES.md section 3: "Yes, all five, re-record everything." A program that names
none of these options now gets the measured-better setting, and the old default is the option it became.

| statement | new default | old default, now an option | the measurement |
|---|---|---|---|
| `settle`, `anneal`, `anneal_each`, `anneal_schedule` | `update: :metro` | `update: :gibbs` | yes-rate error 0.15 to 0.47 times Gibbs's (`runs/settleperfect/`) |
| `play`, `play_colour` | `update: :metro_checker` | `update: :gibbs` | bits 29.68 against 26.71 dB at J 0.2 (`runs/filmsharp/`) |
| `play`, `play_colour`, `lean_from` | `correct: :tap` | `correct: :mean` (`:yes`) | 20 of 20 exact targets (`runs/gridplayer2/`) |
| `play` warm fit | `warm_from: :correction` | `warm_from: :leans` | never worse, up to 6.5 dB better (`runs/filmwarm/`) |
| `factor` | `encoding: :columns` | `encoding: :rosenberg` | 899: 100% against 14% (`runs/zoohard/`) |

- **Every documented output was re-recorded** with the release interpreter: 42 of the 95 recorded outputs in
  `docs/examples/` moved, and 1 of the 26 recorded errors (the statement list's help lines).
  The run line of `play` and `play_colour` names a rule only when it is not the default, so it now prints
  `, gibbs` and `, mean` for the old defaults and nothing for `:metro_checker` and `:tap`.
- **`anneal_each` and `anneal_schedule` take `update:`.** The schedule's walk follows the run's rule, so it still
  walks the core `anneal`'s path draw for draw under both rules.
- **Fixed:** `play ..., update: :gibbs` took the core sweep, which now follows the run's core rule; it is taken
  only while that rule is Gibbs too.
- Four examples changed their program: `core-update` and `zoo-factor` now show the old default as the option,
  `sem-seed`'s coin leans a little (with no input at all, `:metro` flips at every visit and reads exactly 50.0% on
  every seed), and `zootemp-restarts` names `update: :gibbs`, the rule ZOOTEMP measured. `examples/factor.settle`
  moved to the column encoding at the zoo's temperature rule.
- `tests/new_defaults.rs`: one test per default with a control, each red-proven against the old default.

### 2026-10-07, lane SETTLEPAGE: the first pages, simpler, and every answer on them checked

- **Install and run** goes clone, build, first program, then the details. The `--json` answers it shows are files,
  `docs/examples/tour-first.json` and `err-unknown-thing.json`, and the new test `tests/docs_json.rs` runs
  `settle --json` on each and compares, with the exit status (0 for `"ok": true`, 2 for `"ok": false`).
- **The tour** has a fourth program, `tour-remember`: write three patterns into an sdm, read one back from a noisy
  read-address, and see a never-written one read as `nothing clear`.
- **The builder face** is the alarm program, `docs/examples/builder-alarm.rs`. `tests/two_faces.rs` compiles that
  file as written and checks it prints the alarm program's two lines, so the Rust on the page runs.
- No statement, default or output changed.
