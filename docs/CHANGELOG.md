# Changelog

What each change to the SETTLE interpreter added, in order, taken from the git history of
`experiments/thermosim/settle-rs/`. The crate's version has stayed at `0.1.0` (`Cargo.toml`) throughout; entries
are grouped by date and name the commit that landed them. Lane names in capitals (GRIDPLAYER, SETTLEZOO, ...) are
the campaign lanes in `experiments/thermosim/SETTLE_CAMPAIGN_2026-09-30.md`; each has a report in
`experiments/thermosim/runs/`.

## Before the Rust interpreter

`experiments/thermosim/settle.py` was the first SETTLE interpreter, in Python, with a line-per-statement syntax
(`thing rain, sprinkler`, `rain leans no by 1.5`, `rain pulls wet_grass by 2`). Its example is
`experiments/thermosim/examples/rain.settle`. The Rust interpreter replaced that syntax with the Rails-style one
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
  one word. Sources and quotes: `experiments/thermosim/kanerva/KANERVA_TERMS.md`.
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
