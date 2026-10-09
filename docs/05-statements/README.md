# Statements by family

Every SETTLE statement belongs to a **family**: one Rust file in `src/` that implements the `Ext` trait (see
[Extending SETTLE](../07-extending.md)). The interpreter offers each line to the families in the order below,
and the first family whose pattern matches the line runs it. Each page lists the family's statements with their
arguments, defaults, exact output and error messages, and at least one tested example.

`settle --help` prints a one-line reminder of each statement form. Those lines are short and leave out many
keyword arguments; the pages below are complete.

| Order | Family | Source | What it adds |
|---|---|---|---|
| 1 | [core](core.md) | `src/words/core.rs` | Things, leans and pulls; `hold`, `settle`, `anneal`, `show`, `best`, `ask`. |
| 2 | [memory](memory.md) | `src/memory.rs` | A Hopfield memory: store patterns and text in the pulls, recall them from a damaged cue, keyed notes. |
| 3 | [grid](grid.md) | `src/grid.rs` | One thing per pixel: read and write PGM pictures, play a folder of frames. |
| 4 | [zoo](zoo.md) | `src/zoo.rs` | Puzzles as models: sudoku, graph colouring, max-cut, factoring, nonograms; checking a solution. |
| 5 | [learn](learn.md) | `src/learn.rs` | Example sets, hidden things, Boltzmann machine learning, classification by settling. |
| 6 | [valleys](valleys.md) | `src/valleys.rs` | Test landscapes; exact enumeration and random surveys of the calm arrangements. |
| 7 | [numbers](numbers.md) | `src/numbers.rs` | Real-valued things on springs, Langevin drift, solving linear systems. |
| 8 | [sdm](sdm.md) | `src/sdm.rs` | Kanerva sparse distributed memory with hard locations and counters. |
| 9 | [softsdm](softsdm.md) | `src/softsdm.rs` | Sparse distributed memory built from p-bits, with a soft cut-off. |
| 10 | [export](export.md) | `src/export.rs` | Write a model as Ising, QUBO, Gset or DIMACS; read an Ising file back. |
| 11 | [colour](colour.md) | `src/colour.rs` | Three grids per colour picture: PPM in and out, colour playback. |
| 12 | [sdmscale](sdmscale.md) | `src/sdmscale.rs` | Sparse distributed memory at scale, up to a million locations. |
| 13 | [coded](coded.md) | `src/coded.rs` | Compress and error-code text before storing it in a memory; decode on recall. |
| 14 | [denoise](denoise.md) | `src/denoise.rs` | A chain of small machines trained to turn coin noise into examples. |
| 15 | [ldpcsettle](ldpcsettle.md) | `src/ldpcsettle.rs` | LDPC error-correcting codes as springs; decode by settling or belief propagation. |
| 16 | [ldpcmoves](ldpcmoves.md) | `src/ldpcmoves.rs` | Multi-bit moves and Nishimori-temperature decoding for those codes. |
| 17 | [sdmrefuse](sdmrefuse.md) | `src/sdmrefuse.rs` | When a sparse distributed memory read should refuse to answer. |
| 18 | [zootemp](zootemp.md) | `src/zootemp.rs` | An anneal with a chosen schedule and restarts; judging a puzzle on the end state with `x.final`. |
| 19 | [sdmtrack](sdmtrack.md) | `src/sdmtrack.rs` | Predicted recall of a sparse distributed memory's content reads, without building the memory. |
| 20 | [descend](descend.md) | `src/descend.rs` | Gradient descent as Settling: parameters on any differentiable loss, Langevin above temperature 0, the posterior cloud. |

## Modules without statements of their own

Five source files add no statements. They implement options or data that other families' statements use:

| Module | Used by | What it provides |
|---|---|---|
| `src/filmsharp.rs` | grid, colour | The Bethe inversion (`correct: :bethe`), fitted leans (`fit:`), the Rao-Blackwellised read (`read: :rb`), and the update rules (`update:`). |
| `src/filmwarm.rs` | grid | Warm-started lean fits from frame to frame (`warm_fit:` and related options). |
| `src/zoohard.rs` | zoo | Hard puzzle generators and exact deciders used by the measurement program `examples/zoohard_measure.rs`. |
| `src/sdmradius.rs` | sdmscale | Choosing a radius for the damage a cue carries, and density-scaled read thresholds. |
| `src/mnist.rs` | descend | Reading the MNIST IDX files for `data :mnist` and `test :mnist`, and the restricted machines the measurement program `examples/mnist_measure.rs` trains. |

The options are documented on the pages of the statements that accept them.

## The KANERVA boundary

Every sparse distributed memory algorithm the statements use lives in KANERVA, the crate SETTLE depends on
(`Cargo.toml`; [Install and run](../01-install-and-run.md#requirements) says where Cargo finds it). The rule at
the seam: KANERVA holds the algorithms, their equations and their own tests; a SETTLE file holds the statements,
the way a memory is laid out as things and pulls, and the tests of that layout. A SETTLE file reaches KANERVA
only through `use kanerva::...` at its top, and re-exports KANERVA's items under the names they had before the
split (`pub use`), so `settle::sdmscale::Store` and `kanerva::store::Store` are the same type. Nothing in
KANERVA knows about SETTLE.

| SETTLE file | KANERVA modules it uses |
|---|---|
| `src/engine/rng.rs` | `rng`, with the `sdm` feature on: one random generator both crates draw from (off, SETTLE's own byte-identical copy) |
| `src/engine/codes.rs` | `codes`, in its test only: SETTLE's own copies of seed_of and code are held equal to KANERVA's |
| `src/plug.rs` | `lang`, `words`: KANERVA's parser of the five sdm-family statements, mounted in the registry |
| `src/memory.rs` | `codes`, `keys` |
| `src/sdm.rs` | `address`, `codes`, `keys`, `lang`, `store`, `theory` |
| `src/softsdm.rs` | `soft`, `codes`, `lang`, `theory` |
| `src/sdmscale.rs` | `bits`, `hopfield`, `lang`, `smap`, `store`, `theory` |
| `src/sdmradius.rs` | `smap`, `store`, `theory` |
| `src/sdmrefuse.rs` | `bits`, `lang`, `refuse`, `store`, `theory` |
| `src/sdmtrack.rs` | `track`, `bits`, `lang`, `store`, `theory` |

Recount it with `grep -rn "kanerva::" src`. The `valleys` and `coded` statements use `src/engine/codes.rs`, not
KANERVA, however the crate is built. The KANERVA modules and their equations are in KANERVA's own
README.
