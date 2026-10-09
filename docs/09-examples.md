# Examples

This page catalogues every SETTLE program in the repository: the examples in this documentation, the example
programs that ship with the interpreter, and the Rust measurement programs. Paths are relative to the root of the
SETTLE repository.

## Documentation examples

Every program shown on these pages is a file in `docs/examples/`, with its exact output beside it in a `.out`
file (or its error in a `.err` file). `tests/docs_examples.rs` runs each one on every `cargo test --release`,
compares what it prints with the recorded file, and checks that each Markdown fence matches its file. The table
below lists them by name, with the page that shows each one.

### Running an example

```text
cd docs/examples
../../target/release/settle core-weather.settle
```

Examples that read pictures or example rows use the small files in `docs/examples/data/`. Examples that write
files write them relative to the example, so run them in a scratch copy of the folder if you want to keep it clean.

### Recording the output of a new example

1. Write `docs/examples/<family>-<name>.settle`.
2. Run `bash docs/examples/run.sh <family>-<name>`. It runs the program twice in a scratch copy, writes
   `<family>-<name>.out` (or `.err`), masks timings as `<time>`, and reports a program whose two runs differ.
3. Show it on a page in a fence opened with ```` ```settle example=<family>-<name> ```` and its output in a fence
   opened with ```` ```text output=<family>-<name> ````, each holding the file's contents exactly.
4. Run `cargo test --release --test docs_examples`. After a change to the interpreter that changes output on
   purpose, `SETTLE_DOCS_BLESS=1 cargo test --release --test docs_examples` rewrites every `.out` and `.err` file and
   every output fence; review the diff before committing it.

### The catalogue

| Example | Result | Shown on | What it does |
|---|---|---|---|
| `coded-choices` | output | [The coded family](05-statements/coded.md) | The same text through each compressor and each code, one memory per choice. |
| `coded-controls` | output | [The coded family](05-statements/coded.md) | Recalls that decode with the wrong settings are refused, never printed as text. |
| `coded-does-not-fit` | error | [The coded family](05-statements/coded.md) | A 64-thing memory with a Hamming code carries 4 x 9 = 36 frame bits: 24 header bits and 12 payload bits. |
| `coded-options` | output | [The coded family](05-statements/coded.md) | save_coded also works in a run block, and each store has its own read options. |
| `coded-save-recall` | output | [The coded family](05-statements/coded.md) | Store a compressed, error-coded note in a Hopfield memory, then read it back. |
| `coded-sdm` | output | [The coded family](05-statements/coded.md) | A coded note in a Kanerva sdm, read by both of the sdm's reads. |
| `colour-channels` | output | [The colour family](05-statements/colour.md) | Each channel is an ordinary grid: its leans can be set and its yes-rates written like any grid's. |
| `colour-no-warm-fit` | error | [The colour family](05-statements/colour.md) | play_colour takes the fit options but not the warm-fit options of play. |
| `colour-options` | output | [The colour family](05-statements/colour.md) | The grid options work per channel: copies, keep, a cold start, another update rule, a fit. |
| `colour-play` | output | [The colour family](05-statements/colour.md) | Play three 24 x 16 colour frames on three grids of p-bits, one per channel. |
| `colour-soft` | output | [The colour family](05-statements/colour.md) | The soft read with TAP leans, then the same play scored against another shot. |
| `cook-explaining-away` | output | [Cookbook](08-cookbook.md) | The alarm rang. Was it a burglary? Then we learn there was an earthquake. |
| `cook-seating` | output | [Cookbook](08-cookbook.md) | Seat six guests at two tables (yes = table A, no = table B). |
| `cook-several-questions` | output | [Cookbook](08-cookbook.md) | One settle answers many questions: each ask counts over the same samples. |
| `cook-temperature-sweep` | output | [Cookbook](08-cookbook.md) | How strongly a chain of six things agrees, at four temperatures. |
| `core-anneal` | output | [The core family](05-statements/core.md) | five things in a ring that all push their neighbours: no arrangement satisfies every push |
| `core-ask` | output | [The core family](05-statements/core.md) | the four ask combinators, applied left to right |
| `core-lean-needs-by` | error | [The core family](05-statements/core.md) | A lean given without a strength. |
| `core-leans-and-pulls` | output | [The core family](05-statements/core.md) | leans and pulls add up when they are declared more than once |
| `core-seed` | output | [The core family](05-statements/core.md) | what seed: and temperature: keep for later statements in the same run block |
| `core-update` | output | [The core family](05-statements/core.md) | Gibbs and Metropolised Gibbs settle to the same yes-rates |
| `core-show-before-settle` | error | [The core family](05-statements/core.md) | show reads the samples of the last settle; an anneal does not make any |
| `core-weather` | output | [The core family](05-statements/core.md) | the grass is wet: was it the rain or the sprinkler? |
| `denoise-baselines` | output | [The denoise family](05-statements/denoise.md) | Two baselines for a denoiser: sample the model's own things directly, and draw independent coins. |
| `denoise-chain` | output | [The denoise family](05-statements/denoise.md) | A four-step denoising chain over 4 x 4 glyphs: generate before and after training. |
| `denoise-rows` | output | [The denoise family](05-statements/denoise.md) | Samples written as a rows file can be read back as examples. |
| `denoise-untrained` | error | [The denoise family](05-statements/denoise.md) | A denoiser declared without over: learns its pixels from its first training set. |
| `descend-adam` | output | [The descend family](05-statements/descend.md) | Adam on the rings net: a point with no cloud of its own. |
| `descend-adam-hot` | error | [The descend family](05-statements/descend.md) | Adam has no temperature, so asking for one is refused. |
| `descend-cloud` | output | [The descend family](05-statements/descend.md) | Four net walkers at temperature 1: averaged parameters fail, averaged predictions win, with doubt. |
| `descend-gd` | output | [The descend family](05-statements/descend.md) | Temperature 0 is gradient descent: the exact least-squares line. |
| `descend-line` | output | [The descend family](05-statements/descend.md) | A line fitted at temperature 1: the cloud is the Bayesian posterior, beside the exact one. |
| `descend-no-data` | error | [The descend family](05-statements/descend.md) | A loss with examples needs data. |
| `descend-rings` | output | [The descend family](05-statements/descend.md) | Two rings: the logistic piece cannot split them, an annealed net can. |
| `descend-springs` | output | [The descend family](05-statements/descend.md) | descend on springs: gradient descent at 0, drift's own walk at 1. |
| `descend-temperature` | output | [The descend family](05-statements/descend.md) | The negative control: temperature 2 doubles the posterior's variance. |
| `err-ask-before-settle` | error | [Errors](06-errors.md) | `ask` before any `settle` in the run block. |
| `err-bad-keyword` | error | [Errors](06-errors.md) | A keyword argument the statement does not accept. |
| `err-nested` | error | [Errors](06-errors.md) | A block opened inside another block. |
| `err-string` | error | [Errors](06-errors.md) | A string with no closing quote. |
| `err-unclosed` | error | [Errors](06-errors.md) | A block never closed with `end`. |
| `err-unknown-statement` | error | [Errors](06-errors.md) | A line no statement family recognises. |
| `err-unknown-thing` | error | [Errors](06-errors.md) | A pull on a thing that was never declared. |
| `export-different-model` | error | [The export family](05-statements/export.md) | a run import refuses a file written for a model with different pulls |
| `export-forms` | output | [The export family](05-statements/export.md) | write one model as Ising and as QUBO data, then read the Ising file back into a second model |
| `export-maxcut` | output | [The export family](05-statements/export.md), [Cookbook](08-cookbook.md) | a max-cut graph: every edge pushes its two ends apart, and no thing leans |
| `export-run` | output | [The export family](05-statements/export.md) | export a run's held things and temperature, then restore them in another run of an identical model |
| `grid-against` | output | [The grid family](05-statements/grid.md) | The negative control: score the output against frames it was never given. |
| `grid-fit` | output | [The grid family](05-statements/grid.md) | Fit the leans by settling the grid itself before each frame is played. |
| `grid-lean-run` | output | [The grid family](05-statements/grid.md) | lean_from inside a run changes the leans for the rest of that run. |
| `grid-options` | output | [The grid family](05-statements/grid.md) | Other inversions, update rules, copies, keep, and a cold start. |
| `grid-play` | output | [The grid family](05-statements/grid.md) | Play a folder of three 24 x 16 frames with the default options. |
| `grid-read` | output | [The grid family](05-statements/grid.md) | The same play read three ways. The leans use the TAP inversion. |
| `grid-still` | output | [The grid family](05-statements/grid.md), [Cookbook](08-cookbook.md) | One still picture on a 24 x 16 grid of p-bits. |
| `grid-temperature` | output | [The grid family](05-statements/grid.md) | Temperature and sharpening in play, then show_as reads the counts play leaves behind. |
| `grid-warm-fit` | output | [The grid family](05-statements/grid.md) | Fit the first frame's leans cold, then fit each later frame warm from the frame before. |
| `grid-wrong-size` | error | [The grid family](05-statements/grid.md) | A picture must have the grid's width and height. |
| `ldpcmoves-movers` | output | [The ldpcmoves family](05-statements/ldpcmoves.md) | One received word, decoded by the one-thing settle and by the moves that carry a bit's checks with it. |
| `ldpcmoves-soft-checks` | output | [The ldpcmoves family](05-statements/ldpcmoves.md) | At the default strength, broken checks are cheap at temperature 1, so the bitwise average is not a codeword. |
| `ldpcsettle-decode` | output | [The ldpcsettle family](05-statements/ldpcsettle.md) | A 64-bit LDPC code built as springs, sent through a channel that flips 5% of bits, then decoded. |
| `ldpcsettle-options` | output | [The ldpcsettle family](05-statements/ldpcsettle.md) | Longer anneals, stiffer penalties and harder channels. A failed decode is a refusal, never a guess. |
| `ldpcsettle-springs` | output | [The ldpcsettle family](05-statements/ldpcsettle.md) | The code's things and pulls are part of the model, so the core anneal can run them too. |
| `ldpcsettle-transmit-first` | error | [The ldpcsettle family](05-statements/ldpcsettle.md) | A decode needs a received word. |
| `learn-bad-row` | error | [The learn family](05-statements/learn.md) | Every row must have one cell per thing named in over:. |
| `learn-classify` | output | [The learn family](05-statements/learn.md), [Cookbook](08-cookbook.md) | classify on a test set with a row that has both labels on. That row is skipped. |
| `learn-file` | output | [The learn family](05-statements/learn.md) | Learn "do the two bits agree?" from a file, with four hidden things, then classify. |
| `learn-inline` | output | [The learn family](05-statements/learn.md) | Inline examples: name the things with over:, give each row as a string of 1 and 0. |
| `learn-methods` | output | [The learn family](05-statements/learn.md) | The same four rows fitted three ways, one model for each method, so each fit starts from zero. |
| `learn-shuffle` | output | [The learn family](05-statements/learn.md) | The negative control: scramble which labels go with which row of the training set, |
| `memory-fade` | output | [The memory family](05-statements/memory.md) | with fade, each new memory weakens the older ones, so the oldest of twelve is lost |
| `memory-keyed` | output | [The memory family](05-statements/memory.md) | keyed text: the key both finds the memory and reads it; the text is not kept in the model |
| `memory-options` | output | [The memory family](05-statements/memory.md) | the recall options: how long to shake, how hot, and where the randomness starts |
| `memory-recall` | output | [The memory family](05-statements/memory.md) | three random patterns stored in the pulls of 256 things, then recalled from noisy read-addresses |
| `memory-text-too-long` | error | [The memory family](05-statements/memory.md) | a plain text note needs 8 things per byte; 9 bytes do not fit in 64 things |
| `memory-text` | output | [The memory family](05-statements/memory.md), [Cookbook](08-cookbook.md) | text saved in a memory comes back letter for letter from a noisy read-address |
| `numbers-cut` | output | [The numbers family](05-statements/numbers.md) | solve on numbers that already have springs to another number. The springs |
| `numbers-drift` | output | [The numbers family](05-statements/numbers.md) | Two numbers joined by springs. Their energy is lowest where A x = b with |
| `numbers-no-valley` | output | [The numbers family](05-statements/numbers.md) | A symmetric matrix that is not positive definite has no valley. The drift runs, |
| `numbers-opposes` | output | [The numbers family](05-statements/numbers.md) | opposes pulls one number toward minus the other. At temperature 0 the drift |
| `numbers-refused` | error | [The numbers family](05-statements/numbers.md) | A matrix that is not symmetric is not a set of springs, so solve refuses it. |
| `numbers-solve` | output | [The numbers family](05-statements/numbers.md), [Cookbook](08-cookbook.md) | Solve the 2x2 system [[2, 1], [1, 3]] x = [1, 2] by drifting springs. |
| `numbers-step` | output | [The numbers family](05-statements/numbers.md) | A step too large for the stiffest spring makes the drift overshoot and blow up. |
| `sdm-already-written` | error | [The sdm family](05-statements/sdm.md) | A name can be written only once in a memory. |
| `sdm-fade` | output | [The sdm family](05-statements/sdm.md) | Fade: every write first multiplies all bit-counters by the fade, so older patterns weaken. |
| `sdm-read` | output | [The sdm family](05-statements/sdm.md) | A Kanerva memory of 128 data things and 1,000 hard locations. |
| `sdm-text` | output | [The sdm family](05-statements/sdm.md) | Text in the open and text under a key, in one Kanerva memory. |
| `sdmrefuse-refusal` | output | [The sdmrefuse family](05-statements/sdmrefuse.md) | The refusal threshold and the nearest-neighbour ceiling for a few memory sizes and loads. No memory is built. |
| `sdmscale-read` | output | [The sdmscale family](05-statements/sdmscale.md) | A Kanerva memory whose bit-counters live outside the pulls: one byte each, in a flat array. |
| `sdmscale-tolerate` | output | [The sdmscale family](05-statements/sdmscale.md) | tolerate-noise: picks the activation radius for read-addresses with that much address-noise, instead of the activation radius activation-probability: gives. |
| `sdmscale-wake-needs-pulls` | error | [The sdmscale family](05-statements/sdmscale.md) | wake: chooses how the pulls read activates hard locations, so it needs via: :pulls. |
| `sdmscale-wake` | output | [The sdmscale family](05-statements/sdmscale.md) | The pulls read wakes a hard location by what it holds: by the agreement of its bit-counters with the state. |
| `sdmtrack-predict` | output | [The sdmtrack family](05-statements/sdmtrack.md) | TRACK-C's predicted recall for a memory of 20,000 hard locations of 256 bits. No memory is built. |
| `sem-reopen` | output | [Semantics](04-semantics.md) | A model block may be opened again; its statements add to the same model. |
| `sem-seed` | output | [Semantics](04-semantics.md) | Each run block starts from the same default seed, so these two runs print the same numbers. |
| `sem-temperature` | output | [Semantics](04-semantics.md) | The same model sampled at a low and a high temperature. |
| `softsdm-attend` | output | [The softsdm family](05-statements/softsdm.md) | Three reads computed outside the sampler: the mean field of this machine, |
| `softsdm-hard` | output | [The softsdm family](05-statements/softsdm.md) | Softness 0 is classic hard SDM: a hard location is activated exactly when the read-address is within the activation radius. |
| `softsdm-no-key` | error | [The softsdm family](05-statements/softsdm.md) | softsdm has no keyed write; a trailing key: is refused. |
| `softsdm-read` | output | [The softsdm family](05-statements/softsdm.md) | A soft Kanerva memory: 128 address things, 1,000 hard location p-bits, 128 data things. |
| `syntax-lexical` | output | [Syntax](03-syntax.md) | A comment runs from # to the end of the line. |
| `tour-anneal` | output | [A tour of SETTLE](02-tour.md) | Three things that each want to disagree with the other two cannot all get their way. |
| `tour-first` | output | [Install and run](01-install-and-run.md), [A tour of SETTLE](02-tour.md) | Two things that tend to agree, and one that leans towards yes. |
| `tour-hold` | output | [A tour of SETTLE](02-tour.md) | Holding a thing fixes it at one value; the others respond. |
| `tour-remember` | output | [A tour of SETTLE](02-tour.md) | Write three patterns. Read one back from a noisy copy. |
| `valleys-code` | output | [The valleys family](05-statements/valleys.md) | a code landscape: 8 data bits, 4 parity checks of 3 bits, one helper thing per check |
| `valleys-grid` | output | [The valleys family](05-statements/valleys.md) | a 4 x 3 grid where every thing pulls its neighbours by 1, open and wrapped into a torus |
| `valleys-held` | output | [The valleys family](05-statements/valleys.md) | a held thing is folded into its neighbours' leans; valleys and survey then vary only the free things |
| `valleys-memory` | output | [The valleys family](05-statements/valleys.md) | the valleys of a small memory: stored patterns, their mirror images, and fakes |
| `valleys-random` | output | [The valleys family](05-statements/valleys.md) | a random landscape of 12 things, measured exactly and then by sampling |
| `valleys-ring` | output | [The valleys family](05-statements/valleys.md), [Cookbook](08-cookbook.md) | a ring of 8 things, each pulling the next: the only valleys are all-yes and all-no |
| `valleys-too-large` | error | [The valleys family](05-statements/valleys.md) | valleys refuses more than 24 free things; survey has no limit |
| `zoo-colouring` | output | [The zoo family](05-statements/zoo.md) | A triangle needs three colours. Two colourings of the same triangle share one model. |
| `zoo-factor` | output | [The zoo family](05-statements/zoo.md) | Factor 15 with the default encoding, and 21 with the column encoding. |
| `zoo-maxcut` | output | [The zoo family](05-statements/zoo.md) | A square with one weighted diagonal, and a triangle with a target it cannot reach. |
| `zoo-solution-first` | error | [The zoo family](05-statements/zoo.md) | x.solution reads the calmest arrangement of an anneal, so it needs one first. |
| `zoo-nonogram` | output | [The zoo family](05-statements/zoo.md) | A 5x5 nonogram that draws a heart, and a 3x3 one whose clues no picture satisfies. |
| `zoo-sudoku` | output | [The zoo family](05-statements/zoo.md), [Cookbook](08-cookbook.md) | A 4x4 sudoku with four givens. Rows are separated by spaces; '.' is an empty cell. |
| `zootemp-final-first` | error | [The zootemp family](05-statements/zootemp.md) | x.final reads the run's last arrangement, so something must have run first. |
| `zootemp-restarts` | output | [The zootemp family](05-statements/zootemp.md) | Factor 899 = 29 x 31 with the column encoding, on one budget of 20,000 sweeps, two ways. |
| `zootemp-schedule` | output | [The zootemp family](05-statements/zootemp.md) | Five things in a ring that all push their neighbours. Each run block starts a fresh run. |

`docs/examples/ext_chain.rs` and `ext_chain.program` are the example statement family of
[Extending SETTLE](07-extending.md); the test `the_extension_example_runs` compiles and runs them.

`docs/examples/builder-alarm.rs` is the builder program of [Extending SETTLE](07-extending.md#the-builder-face);
`tests/two_faces.rs` compiles it and checks it prints the alarm program's lines. `tour-first.json` and
`err-unknown-thing.json` are the `settle --json` answers shown in [Install and run](01-install-and-run.md#the-command-line);
`tests/docs_json.rs` runs the command and checks them.

## Example programs that ship with the interpreter

`examples/*.settle` are the programs listed by `settle --help`. The SETTLE site runs each one with the release
binary when it builds its data and shows the output at `#/settle/run-examples`. Every one runs from a clean clone
with `./target/release/settle examples/<name>.settle`. The largest is the horse example, which plays 15 frames of
150 x 100 pixels that ship beside it in `examples/horse/frames/` (public domain; `SOURCE.txt` there says where
they come from) and writes its outputs to `examples/horse/still.pgm` and `examples/horse/out/`, which git
ignores.

| Program | What it does |
|---|---|
| `examples/coded.settle` | Compress, then error-code, then mask, then store; read back by shaking and the reverse pipeline. |
| `examples/colouring.settle` | Graph colouring as settling: one thing per (node, colour). Each node is pushed toward exactly one colour, |
| `examples/denoise.settle` | Denoising from coins: three clean 3x3 glyphs (plus, cross, ring), ten copies each. A chain of four small |
| `examples/factor.settle` | Factoring as settling: bits of p and q, one helper thing per product of two bits (held to that product by a |
| `examples/horse.settle` | Muybridge's horse (1878, public domain) played on a grid of 15,000 p-bits, one per pixel. |
| `examples/keyed.settle` | A key names a turn of the cube: a sign flip on every thing plus a shuffle of which thing holds |
| `examples/ldpcsettle.settle` | An LDPC code written as springs: parity checks are helper things and penalties in the pulls. |
| `examples/learn.settle` | Learning from examples: three 3x3 glyphs (plus, cross, ring), 12% of pixels flipped, 90 to learn from |
| `examples/maxcut.settle` | Max-cut as settling: one thing per node (yes side or no side); every edge pushes its two ends apart, |
| `examples/memory.settle` | Memory with nothing held: every pattern lives in the pulls between 512 free things. |
| `examples/party.settle` | seat six guests at two tables (yes = table A, no = table B); rivals push apart, friends pull together |
| `examples/sdm.settle` | Kanerva's sparse distributed memory: 256 data things and 2000 hidden hard locations. |
| `examples/softsdm.settle` | Kanerva's sparse distributed memory, built out of p-bits, with a soft cut-off. |
| `examples/solve.settle` | SOLVE: real-valued numbers on springs. Shake them, and their average position solves a linear system; |
| `examples/sudoku.settle` | Sudoku as settling: one thing per (cell, digit). Every cell, and every digit in every row, column and box, |
| `examples/survey.settle` | The shape of a settle landscape: how many valleys, how wide, how regular. |
| `examples/weather.settle` | the grass is wet: was it the rain or the sprinkler? |

## Measurement programs

`examples/*.rs` are Rust programs the experiments used to take their measurements. Run one with
`cargo run --release --example <name> -- <part>`; each header names its parts.

| Program | What it measures |
|---|---|
| `examples/core_bench.rs` | SETTLEPERFECT: how fast the core sampler settles on four model shapes, stamped with the machine's load and power mode, with a yes-count checksum so a faster build that changed the samples shows it. |
| `examples/core_update_measure.rs` | SETTLEPERFECT: Gibbs against Metropolised Gibbs (`update: :metro`) on four small models, against exact enumeration; the prediction was sealed in its header before it was run. Output: `runs/settleperfect/`. |
| `examples/filmsharp_exact.rs` | FILMSHARP: every new inversion against the exact answer on a 4x4 open grid (enumeration of all 2^16 states). |
| `examples/filmsharp_fitprobe.rs` | FILMSHARP scratch probe: the fit's residual trajectory on a synthetic picture (development, not a sealed arm). |
| `examples/filmsharp_laws.rs` | FILMSHARP: the exact answers for a grid with no pulls, per budget, on a folder of PGM frames. |
| `examples/filmsharp_tau.rs` | FILMSHARP: how correlated in time is the chain? Pooled autocorrelation of the bits and of the Rao-Blackwellised value tanh(I) on real frames, and the variance-inflation factor 2 tau each carries into a K-sweep average. |
| `examples/filmwarm_tau.rs` | FILMWARM: the time correlation of each update rule with the SAME fitted leans near the critical pull, and the 80-sweep read it predicts, bias included. |
| `examples/gridplayer2_newton_exact.rs` | GRIDPLAYER-2 post-hoc (not sealed): the exact inversion by Newton's method on a 4x4 grid. |
| `examples/gridplayer2_tap_exact.rs` | GRIDPLAYER-2 small-grid control: how close do mean-field and TAP leans land to the target greys, measured against the EXACT marginals of a 4x4 grid (every one of the 65,536 arrangements enumerated)? |
| `examples/gradsettle_measure.rs` | GRADSETTLE: fit MNIST by Settling (Langevin at temperature 1), beside SGD and Adam at the same budget of gradient rows; accuracy, NLL, calibration and the cloud's own doubt. Needs the MNIST files (see the `data` statement of [the descend family](05-statements/descend.md)). |
| `examples/ldpcmoves_measure.rs` | LDPCMOVES measurements: moves that change several things at once, decoding at the Nishimori temperature, and the sealed 10,000-sweep rerun of LDPCSETTLE's settle decoder, on SDMCODED's LDPC codes (n 512, codebook seed 1). Run: `cargo run --release --example ldpcmoves_measure <part> [args]`, part one of pilot | grid <blocks> <sweeps> | long <blocks> | controls <blocks> | exact <blocks> Predictions were sealed in the campaign ledger (`#/results/ledger`) before any measuring part ran. `pilot` prints only wall-clock time per decode, never an error count. Every result is seeded; timings are never claimed. Threads: LDPCMOVES_THREADS (default 6). |
| `examples/ldpcsettle_measure.rs` | LDPCSETTLE measurements: decode SDMCODED's LDPC codes by settling, against its belief propagation. Run: `cargo run --release --example ldpcsettle_measure <part> [args]`, part one of census | pilot | bsc [blocks] | kappa [blocks] | controls | soft <sum|chain> | diag [blocks] Predictions were sealed in the campaign ledger (`#/results/ledger`) before the measuring parts were run. `pilot` prints only wall-clock time per decode (to pick the sweep budget) and never an error count. Everything is seeded. |
| `examples/mnist_measure.rs` | MNIST: restricted settling machines on the 60,000 + 10,000 handwritten digits (`src/mnist.rs`), with nearest-centroid and logistic-regression baselines. Needs the MNIST files. |
| `examples/numbers_bench.rs` | SMOOTHNUMBERS measurement bench: error of the settled solution against settling time, the inverse from the spread, wall clock against Gaussian elimination, and the refusal controls. |
| `examples/sdmcoded_measure.rs` | SDMCODED measurements: compress, then error-code, then mask, then store in a memory; read back end to end. Run: `cargo run --release --example sdmcoded_measure <part> [args]`, part one of ratio | passages <dir> | fragility | bsc | grid <hop|sdm> | controls Predictions were sealed in the campaign ledger (`#/results/ledger`) before this instrument was run. Everything is seeded; the only timings printed are wall-clock totals, stamped by the caller with the machine load. |
| `examples/sdmkeys_measure.rs` | SDMKEYS measurements: Hopfield (memory.rs) against Kanerva SDM (sdm.rs), fake valleys, fade, controls, and what a key protects. Run: `cargo run --release --example sdmkeys_measure [part]`, part one of capacity | noise | fade | controls | keys | all (default all). Predictions P1..P11 were sealed in the campaign ledger (`#/results/ledger`) before this file existed. Everything is seeded; no timing is measured. |
| `examples/sdmradius_measure.rs` | SDMRADIUS measurements: a activation radius chosen per address-noise level, a density-scaled pulls read, the trade-off frontier across radii, and Hopfield at equal memory (`src/sdmradius.rs` on `src/sdmscale.rs`'s store). |
| `examples/sdmrefuse_measure.rs` | SDMREFUSE measurements (`src/sdmrefuse.rs` on `src/sdmscale.rs`'s store). |
| `examples/sdmscale_measure.rs` | SDMSCALE measurements: Kanerva SDM from 2,000 to 1,000,000 hard locations (`src/sdmscale.rs`). |
| `examples/sdmtrack_measure.rs` | SDMTRACK measurements (`src/sdmtrack.rs` on `src/sdmscale.rs`'s store, SDMREFUSE's diagnostic reads). |
| `examples/valleymap.rs` | VALLEYMAP measurement driver: every table in `runs/valleymap/REPORT_VALLEYMAP.md` comes from here. |
| `examples/zoohard_measure.rs` | ZOOHARD measurements (`src/zoohard.rs`, `src/zoo.rs`). Report: `runs/zoohard/REPORT_ZOOHARD.md`. |
| `examples/zootemp_measure.rs` | ZOOTEMP measurements (`src/zootemp.rs`). Report: `runs/zootemp/REPORT_ZOOTEMP.md`. |

## Programs the experiments ran

The experiments that measured these families also ran SETTLE programs of their own, on frames and data that are
not in this repository, and many of them take minutes to hours. They are records of what was run rather than
examples to copy. Each family's page gives the measurements, and the experiments with a card on the SETTLE site's
results page (`#/results`) have their full report there.

## The first interpreter

SETTLE began as a small Python interpreter with an older syntax (`thing rain, sprinkler`, `rain leans no by 1.5`).
Those programs do not run on this interpreter.
