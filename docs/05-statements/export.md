# The export family

The export family writes a SETTLE model as plain data that other samplers, annealers and chip toolkits can read,
and reads the Ising form back. `export` writes one of six forms: the model as Ising data or as a QUBO, a max-cut
graph in the Gset or DIMACS text format, the rates and pair averages of the last settle, or the calmest
arrangement of the last anneal. `import` reads an Ising file: in a model block it adds the file's things, leans
and pulls to the model; in a run block it restores the file's held things and temperature.

The source is `src/export.rs`. The forms, the schema and a cross-check of SETTLE's answers against other samplers
are in `experiments/thermosim/runs/backends/REPORT_SETTLEBACKENDS.md`.

Only the yes/no part of a model is exported. Real-valued `number` things, which live in the model's notes, are
not.

| Statement | Block | Summary |
|---|---|---|
| [`export`](#export) | both | write the model, or the last settle or anneal, to a file |
| [`import`](#import) | both | read a `settle-ising` file: things, leans and pulls (model), or held things and temperature (run) |

## `export`

**Block:** both. The forms `:moments` and `:best` work only in a run block.

**Form:**

```text
export "path", as: :ising
```

**Arguments:**

| Argument | Type | Default | Meaning |
|---|---|---|---|
| `"path"` | path string | required | the file to write; a relative path is taken from the program file's folder |
| `as:` | one of `:ising` / `:qubo` / `:gset` / `:dimacs` / `:moments` / `:best` | `:ising` | the form to write |

**What it does:** builds the Ising view of the model (below), turns it into the chosen form, and writes the file.
Folders in the path that do not exist are created. An existing file is replaced.

In a model block the Ising view has no held things and temperature 1. In a run block it carries the run's held
things and the run's current temperature (the one the last `settle` set, or 1).

The Ising view lists the pulls as edges `[i, j, J]` with `i < j`, sorted by `i` then `j`, with every zero pull left
out. Things are numbered from 0 in the model's order.

Numbers are written in Rust's shortest form that reads back to the same 64-bit float, so `1` is written `1.0`.
Exporting then importing an Ising file gives back the same leans and pulls, bit for bit.

The export statement does not change the model or the run.

**Output:**

```text
exported :<form> (<things> things, <pulls> pulls) to <path>
```

`<pulls>` is the number of non-zero pulls in the model, whatever the form. `<path>` is the path as it was
resolved. If the model has real-valued `number` things, the line ends with
` (real-valued number things are not exported)`.

**Example:** the Ising and QUBO forms, and a round trip through `import`.

```settle example=export-forms
# write one model as Ising and as QUBO data, then read the Ising file back into a second model
model :weather do
  thing :rain,      leans: :no, by: 1
  thing :sprinkler, leans: :no, by: 0.5
  thing :wet_grass
  rain.pushes :sprinkler, by: 0.5
  rain.pulls  :wet_grass, by: 1.5
  sprinkler.pulls :wet_grass, by: 1
  export "out/weather.json"                        # as: :ising is the default
  export "out/weather.qubo.json", as: :qubo        # the 0/1 form with its energy constant
end

model :copy do
  import "out/weather.json"                        # the same things, leans and pulls
end

run :weather do
  anneal 500, seed: 2
  best
end

run :copy do
  anneal 500, seed: 2                              # the same model and seed give the same result
  best
end
```

Output:

```text output=export-forms
exported :ising (3 things, 3 pulls) to out/weather.json
exported :qubo (3 things, 3 pulls) to out/weather.qubo.json
imported 3 things and 3 pulls from out/weather.json
annealed: 500 sweeps, calmest energy found -3.500
best (energy -3.500): rain no, sprinkler no, wet_grass no
annealed: 500 sweeps, calmest energy found -3.500
best (energy -3.500): rain no, sprinkler no, wet_grass no
```

The model `:copy` is built from the file alone, and the same anneal with the same seed finds the same calmest
arrangement in both models.

**Errors:**

- `` export does not take `<key>:` ``
- `as: takes :ising, :qubo, :gset, :dimacs, :moments or :best`
- `unknown form :<form> (known: :ising, :qubo, :gset, :dimacs, :moments, :best)`
- `as: :moments is a run statement (it reads the last settle or anneal)` (and the same for `:best`)
- `export as: :moments needs a settle first`
- `export as: :best needs an anneal first`
- `max-cut formats carry no leans, but :<thing> leans by <lean>`
- `<number> cannot be written as JSON` (a lean, pull or temperature that is infinite or not a number)
- `cannot make <folder>: <reason>`
- `cannot write <path>: <reason>`

### The form `:ising`

A JSON object with `"format": "settle-ising"` and `"version": 1`. The energy it describes is:

```text
E(s) = - sum_i h_i s_i - sum_{i<j} J_ij s_i s_j,     s_i in {-1, +1}, +1 = yes
```

An arrangement that agrees with its leans and pulls has a low energy. A settled machine at temperature `T` visits
an arrangement `s` with chance proportional to `exp(-E(s) / T)`.

| Key | Contents |
|---|---|
| `format` | `"settle-ising"` |
| `version` | `1` |
| `convention` | the energy formula above, as a sentence |
| `n` | the number of things |
| `names` | the things' names, in index order |
| `h` | the lean of each thing |
| `edges` | `[i, j, J]` for every non-zero pull; a positive `J` pulls the two things to agree, a negative one pushes them apart |
| `temperature` | the run's temperature, or 1 from a model block |
| `held` | `[i, 1]` or `[i, -1]` for each held thing, sorted by `i`; empty from a model block |

This is the file `out/w.json` that the example under [`import`](#import) writes from a run in which `:wet_grass` is
held at yes and the temperature is 0.8:

```json
{
  "format": "settle-ising",
  "version": 1,
  "convention": "E(s) = -sum_i h_i s_i - sum_{i<j} J_ij s_i s_j, s_i in {-1,+1} (+1 = yes); p(s) proportional to exp(-E(s)/T)",
  "n": 3,
  "names": ["rain", "sprinkler", "wet_grass"],
  "h": [-1.0, -0.5, 0.0],
  "edges": [
    [0, 1, -0.5],
    [0, 2, 1.5],
    [1, 2, 1.0]
  ],
  "temperature": 0.8,
  "held": [[2, 1]]
}
```

### The form `:qubo`

A JSON object with `"format": "settle-qubo"` and `"version": 1`. It describes the same energy over 0/1 bits
`x_i`, with `x_i = 1` meaning yes, so `s_i = 2 x_i - 1`:

```text
E = sum_i a_i x_i + sum_{i<j} Q_ij x_i x_j + c
a_i = -2 h_i + 2 sum_j J_ij        Q_ij = -4 J_ij        c = sum_i h_i - sum_{i<j} J_ij
```

For every arrangement, `E` equals the Ising energy exactly. A QUBO solver's energy plus the constant `c` is the
SETTLE energy.

| Key | Contents |
|---|---|
| `format` | `"settle-qubo"` |
| `version` | `1` |
| `convention` | the formula above, as a sentence |
| `n` | the number of things |
| `names` | the things' names, in index order |
| `linear` | `a_i` for each thing |
| `quadratic` | `[i, j, Q_ij]` for every non-zero pull, in the same order as the Ising edges |
| `offset` | the constant `c` |
| `temperature` | as in `:ising` |
| `held` | `[i, 1]` or `[i, 0]` for each held thing: held values are written as bits, not as +1/-1 |

This is `out/weather.qubo.json` from the example above:

```json
{
  "format": "settle-qubo",
  "version": 1,
  "convention": "E = sum_i linear_i x_i + sum_{i<j} Q_ij x_i x_j + offset, x_i in {0,1} (1 = yes), s_i = 2 x_i - 1; E equals the Ising energy",
  "n": 3,
  "names": ["rain", "sprinkler", "wet_grass"],
  "linear": [4.0, 2.0, 5.0],
  "quadratic": [
    [0, 1, 2.0],
    [0, 2, -6.0],
    [1, 2, -4.0]
  ],
  "offset": -3.5,
  "temperature": 1.0,
  "held": []
}
```

`import` cannot read this form.

### The forms `:gset` and `:dimacs`

Two plain-text formats for a max-cut graph. They accept only a model in which no thing leans; any non-zero lean
is refused. Each non-zero pull becomes an edge of weight `w = -J`, so a push of 1 is an edge of weight 1. With
`W` the total weight of all edges, the size of the cut of an arrangement is:

```text
cut(s) = (W - E(s)) / 2
```

So the calmest arrangement is the largest cut. Node numbers in both formats start at 1. Held things and the
temperature are not written.

Gset: a first line `<nodes> <edges>`, then one line `<i> <j> <w>` per edge. This is `out/square.gset` from the
example below:

```text
4 5
1 2 1.0
1 3 2.0
1 4 1.0
2 3 1.0
3 4 1.0
```

DIMACS: a comment line explaining the sign rule, one comment line `c node <i> <name>` per thing, a problem line
`p edge <nodes> <edges>`, then one line `e <i> <j> <w>` per edge (the common weighted extension of the DIMACS
edge format). This is `out/square.dimacs`:

```text
c SETTLE max-cut export: an edge i-j of weight w is the pull J_ij = -w
c node 1 a
c node 2 b
c node 3 c
c node 4 d
p edge 4 5
e 1 2 1.0
e 1 3 2.0
e 1 4 1.0
e 2 3 1.0
e 3 4 1.0
```

**Example:** a max-cut graph written in both formats.

```settle example=export-maxcut
# a max-cut graph: every edge pushes its two ends apart, and no thing leans
model :square do
  thing :a, :b, :c, :d
  a.pushes :b, by: 1
  b.pushes :c, by: 1
  c.pushes :d, by: 1
  d.pushes :a, by: 1
  a.pushes :c, by: 2          # a heavier diagonal
  export "out/square.gset", as: :gset
  export "out/square.dimacs", as: :dimacs
end
```

Output:

```text output=export-maxcut
exported :gset (4 things, 5 pulls) to out/square.gset
exported :dimacs (4 things, 5 pulls) to out/square.dimacs
```

### The form `:moments`

Run block only. A JSON object with `"format": "settle-moments"` and `"version": 1` that describes the last
`settle` of the run.

| Key | Contents |
|---|---|
| `names` | the things' names, in index order |
| `samples` | how many arrangements the settle recorded |
| `temperature` | the run's temperature |
| `held` | `[i, 1]` or `[i, -1]` for each held thing |
| `yes_rate` | for each thing, the share of recorded arrangements in which it was yes; a thing held at yes has 1.0 |
| `pair_mean` | `[i, j, m]`: the average of `s_i s_j` over the recorded arrangements |

`pair_mean` lists every pair `i < j` when the model has at most 64 things, and otherwise only the pairs that have
a pull entry. It is empty when the settle kept only per-thing counts, which happens when `things * sweeps` is
above 20,000,000 (see `settle` in [the core family](core.md)). An `anneal` does not replace these numbers: they
always come from the last `settle`.

This is `out/rates.json` from the example under [`import`](#import):

```json
{
  "format": "settle-moments",
  "version": 1,
  "names": ["rain", "sprinkler", "wet_grass"],
  "samples": 2000,
  "temperature": 0.8,
  "held": [[2, 1]],
  "yes_rate": [0.6375, 0.6505, 1.0],
  "pair_mean": [
    [0, 1, -0.344],
    [0, 2, 0.275],
    [1, 2, 0.301]
  ]
}
```

### The form `:best`

Run block only. A JSON object with `"format": "settle-best"` and `"version": 1` that holds the calmest arrangement
the last `anneal` found.

| Key | Contents |
|---|---|
| `names` | the things' names, in index order |
| `s` | the arrangement, 1 for yes and -1 for no, in index order |
| `energy` | its energy, as the anneal computed it |

This is `out/best.json` from the example under [`import`](#import):

```json
{
  "format": "settle-best",
  "version": 1,
  "names": ["rain", "sprinkler", "wet_grass"],
  "s": [1, 1, 1],
  "energy": -0.5
}
```

## `import`

**Block:** both, with a different meaning in each.

**Form:**

```text
import "path"
```

**Arguments:**

| Argument | Type | Default | Meaning |
|---|---|---|---|
| `"path"` | path string | required | a `settle-ising` file; a relative path is taken from the program file's folder |

`import` takes exactly one quoted path and no keyword arguments. It reads only the `:ising` form.

**What it reads:** the file must be JSON with `"format": "settle-ising"` and `"version": 1`. It must have `names`
(non-empty, unique strings), `h` (one number per name) and `edges` (each `[i, j, J]`, where `i` and `j` are
different whole numbers below the number of names, and no pair appears twice in either order). `n`, if present,
must equal the number of names. `temperature` is optional, defaults to 1 and must be above zero. `held` is
optional; each entry is `[i, 1]` or `[i, -1]`. Other keys, such as `convention`, are ignored. Edges with a pull of
zero are dropped, and edges are put in `i < j` order.

**What it does in a model block:** adds the file's things to the model, then adds each lean to its thing's lean
and each pull to the pull between its two things. A thing whose name already exists in the model is reused, so
its lean and pulls add to the ones it has, exactly as repeated `thing ... by:` and `pulls` statements add. The
file's temperature and held things are ignored.

**What it does in a run block:** checks that the file describes exactly this run's model: the same names in the
same order, the same leans and the same pulls, compared bit for bit. If they match, the run's held things are
replaced by the file's held things (any earlier holds are dropped), and the run's temperature is set to the file's
temperature. The model is not changed.

**Output:** in a model block:

```text
imported <things> things and <pulls> pulls from <path>
```

In a run block:

```text
imported <held> held things and temperature <temperature> from <path>
```

`<path>` is printed as written in the program.

**Example:** export a run's held things and temperature, and its rates and calmest arrangement; then rebuild the
model and restore the run from the file.

```settle example=export-run
# export a run's held things and temperature, then restore them in another run of an identical model
model :weather do
  thing :rain,      leans: :no, by: 1
  thing :sprinkler, leans: :no, by: 0.5
  thing :wet_grass
  rain.pushes :sprinkler, by: 0.5
  rain.pulls  :wet_grass, by: 1.5
  sprinkler.pulls :wet_grass, by: 1
end

run :weather do
  hold :wet_grass, :yes
  settle 2_000, temperature: 0.8, seed: 1
  export "out/w.json"                     # carries held :wet_grass and temperature 0.8
  export "out/rates.json", as: :moments   # yes-rates and pair averages of this settle
  anneal 500, seed: 2
  export "out/best.json", as: :best       # the calmest arrangement of this anneal
end

model :copy do
  import "out/w.json"                     # model import: things, leans and pulls only
end

run :copy do
  import "out/w.json"                     # run import: held things and temperature
  settle 2_000, seed: 1                   # the same run as above, so the same rates
  show
end
```

Output:

```text output=export-run
settled: 2000 samples of 3 things at temperature 0.8
exported :ising (3 things, 3 pulls) to out/w.json
exported :moments (3 things, 3 pulls) to out/rates.json
annealed: 500 sweeps, calmest energy found -0.500
exported :best (3 things, 3 pulls) to out/best.json
imported 3 things and 3 pulls from out/w.json
imported 1 held things and temperature 0.8 from out/w.json
settled: 2000 samples of 3 things at temperature 0.8
  rain           ################### 63.7%
  sprinkler      #################### 65.0%
  wet_grass      ############################## 100.0%  (held)
```

The files this example writes are shown above, under [`:ising`](#the-form-ising),
[`:moments`](#the-form-moments) and [`:best`](#the-form-best).

**Example:** a run import refuses a file written for a different model.

```settle example=export-different-model
# a run import refuses a file written for a model with different pulls
model :one do
  thing :a, :b
  a.pulls :b, by: 1
  export "out/one.json"
end

model :two do
  thing :a, :b
  a.pushes :b, by: 1
end

run :two do
  import "out/one.json"
end
```

```text error=export-different-model
line 14: out/one.json describes a different model than this run's (names, leans or pulls differ)
```

**Errors:** the statement itself:

- `import takes one "quoted" path`
- `cannot read <path>: <reason>`
- `<path> describes a different model than this run's (names, leans or pulls differ)` (run block only)

A file that is not a readable `settle-ising` file gives `<path>: <problem>`, where `<problem>` is one of:

- `JSON: <what> at byte <offset>`, where `<what>` is `unexpected end`, `unknown word`, `object key must be a
  string`, `expected ':'`, `expected ',' or '}'`, `expected ',' or ']'`, `unclosed string`, `bad escape`,
  `'<text>' is not a number` or `trailing text`; also `JSON: bad \u escape` and `JSON: not UTF-8`
- `not a settle-ising file ("format": "settle-ising" is missing)`
- `only settle-ising version 1 is known`
- `missing names`, `missing h`, `missing edges`
- `names must be a list`, `h must be a list`, `edges must be a list`, `an edge must be a list`, `held must be a
  list`, `a held entry must be a list`
- `every name must be a non-empty string`
- `the name <name> appears twice`
- `n says <n> but there are <count> names`
- `<count> leans for <count> names`
- `an edge is [i, j, J]`
- `edge end <x> is not a thing index below <n>`
- `edge <i>-<j> joins a thing to itself`
- `edge <i>-<j> appears twice`
- `a lean must be a number`, `a pull must be a number`, `edge end must be a number`, `n must be a number`,
  `temperature must be a number`, `a held value must be a number`, `held thing must be a number`
- `temperature must be above zero`
- `a held entry is [i, 1 or -1]`
- `a held value is 1 or -1, not <value>`
- `held thing <x> is not a thing index below <n>`

## Notes

- **What each form carries.** `:ising` and `:qubo` carry the whole yes/no model, plus the held things and
  temperature of a run. `:gset` and `:dimacs` carry only the pulls, and refuse a model with leans. `:moments` and
  `:best` carry results, not the model: `import` cannot rebuild a model from them.
- **Round trips.** Only `:ising` can be imported. A model rebuilt from it has the same things in the same order and
  the same leans and pulls bit for bit, so a run on the copy with the same seed gives the same result. The run
  import compares the file with the model bit for bit, so a model that differs only by rounding is refused.
