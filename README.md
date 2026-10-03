# settle-rs

The SETTLE interpreter: a toy language where a program is springs and running it means letting it
settle. Run a program with `cargo run --release -- examples/weather.settle`; list every statement with
`cargo run --release -- --help`. The full documentation is in `docs/` (start at `docs/README.md`).

## Get it and build it

```bash
git clone https://github.com/triplesparkle/SETTLE
cd SETTLE
cargo build --release
./target/release/settle docs/examples/tour-first.settle
```

The repository is private for now, so a clone needs access until it is made public.

## Sparse distributed memory lives in KANERVA

Every SDM algorithm the statements use (`memory`, `sdm`, `softsdm`, `sdmscale`, `refusal`,
`contenttrack`) is in the KANERVA crate, a standalone library at `github.com/triplesparkle/KANERVA`.
Cargo fetches it by its git URL when you build (in the research repository the two crates sit side by side and the dependency is a path, `../kanerva`). The files in `src/` keep the statements, the pull layouts
and their tests, and re-export KANERVA's items under their old names. Read KANERVA's README for the
toolbox, its equations and its results.
